//! 部署脚本的运行检查。只验证脚本本身，不需要 Docker 或线上服务。

#[cfg(unix)]
#[test]
fn backup_sh_supports_launchd_layout() {
    use std::os::unix::fs::PermissionsExt;

    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::create_dir_all(root.join("deploy")).unwrap();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(root.join("data/backups")).unwrap();
    std::fs::create_dir_all(root.join("tools")).unwrap();
    std::fs::copy(repo.join("deploy/backup.sh"), root.join("deploy/backup.sh")).unwrap();

    let fake_server = root.join("bin/board-server");
    std::fs::write(
        &fake_server,
        r#"#!/bin/sh
set -eu
[ "$#" -eq 4 ] || exit 11
[ "$1" = "--db" ] || exit 12
[ "$2" = "data/board.db" ] || exit 13
[ "$3" = "backup" ] || exit 14
mkdir -p "$(dirname "$4")"
printf 'sqlite backup\n' > "$4"
printf '%s\n' "$*" > "${BOARD_TEST_ARGS:?}"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&fake_server, std::fs::Permissions::from_mode(0o755)).unwrap();

    let fake_docker = root.join("tools/docker");
    std::fs::write(&fake_docker, "#!/bin/sh\nexit 99\n").unwrap();
    std::fs::set_permissions(&fake_docker, std::fs::Permissions::from_mode(0o755)).unwrap();

    let old = root.join("data/backups/board-20000101-000000.db");
    std::fs::write(&old, "old\n").unwrap();
    let status = std::process::Command::new("touch").args(["-t", "200001010000"]).arg(&old).status().unwrap();
    assert!(status.success());

    let args = root.join("args.txt");
    let path = format!("{}:{}", root.join("tools").display(), std::env::var("PATH").unwrap_or_default());
    let out = std::process::Command::new("sh")
        .arg(root.join("deploy/backup.sh"))
        .current_dir(root)
        .env("PATH", path)
        .env("BOARD_TEST_ARGS", &args)
        .env_remove("BOARD_BACKUP_REMOTE")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let args = std::fs::read_to_string(&args).unwrap();
    assert!(args.starts_with("--db data/board.db backup data/backups/board-"), "{args}");
    assert!(args.trim_end().ends_with(".db"), "{args}");
    assert!(!old.exists(), "超过 14 天的备份没有清理");
    let backups: Vec<_> = std::fs::read_dir(root.join("data/backups"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("board-"))
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), "sqlite backup\n");
}

#[cfg(unix)]
#[test]
fn release_script_restores_worktree_when_cargo_update_fails() {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Board Test")
            .env("GIT_AUTHOR_EMAIL", "board-test@example.com")
            .env("GIT_COMMITTER_NAME", "Board Test")
            .env("GIT_COMMITTER_EMAIL", "board-test@example.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} 失败\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let repo = root.join("repo");
    let remote = root.join("origin.git");
    let fake_bin = root.join("bin");
    std::fs::create_dir_all(repo.join("scripts")).unwrap();
    std::fs::create_dir_all(repo.join("web")).unwrap();
    std::fs::create_dir_all(repo.join("src-tauri")).unwrap();
    std::fs::create_dir_all(&fake_bin).unwrap();

    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/release.mjs");
    std::fs::copy(source, repo.join("scripts/release.mjs")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("Cargo.lock"), "# lock\n").unwrap();
    std::fs::write(repo.join("package.json"), "{\"version\":\"0.1.0\"}\n").unwrap();
    std::fs::write(repo.join("package-lock.json"), "{\"version\":\"0.1.0\"}\n").unwrap();
    std::fs::write(repo.join("web/package.json"), "{\"version\":\"0.1.0\"}\n").unwrap();
    std::fs::write(repo.join("web/package-lock.json"), "{\"version\":\"0.1.0\"}\n").unwrap();
    std::fs::write(repo.join("src-tauri/tauri.conf.json"), "{\n  \"version\": \"0.1.0\"\n}\n").unwrap();
    std::fs::write(repo.join("CHANGELOG.md"), "# 更新日志\n\n## v0.1.0\n").unwrap();
    std::fs::write(repo.join("README.md"), "first\n").unwrap();

    git(root, &["init", "-q", "--bare", "origin.git"]);
    git(root, &["init", "-q", "-b", "main", "repo"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "feat: 初始版本"]);
    git(&repo, &["tag", "-a", "v0.1.0", "-m", "v0.1.0"]);
    std::fs::write(repo.join("README.md"), "second\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "fix: 发版前的新提交"]);
    git(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&repo, &["push", "-q", "-u", "origin", "main", "--tags"]);
    let before = git(&repo, &["rev-parse", "HEAD"]);

    let cargo = fake_bin.join("cargo");
    std::fs::write(&cargo, "#!/bin/sh\necho 'cargo update failed' >&2\nexit 42\n").unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap_or_default());
    let out = Command::new("node")
        .args(["scripts/release.mjs", "0.1.1"])
        .current_dir(&repo)
        .env("PATH", path)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "发版脚本应失败\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stderr.contains("已还原工作区和本地 tag"), "stderr:\n{stderr}");
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        std::fs::read_to_string(repo.join("Cargo.toml")).unwrap(),
        "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\n"
    );
    let tag = Command::new("git")
        .args(["rev-parse", "-q", "--verify", "refs/tags/v0.1.1"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(!tag.status.success(), "失败后不应留下 v0.1.1 tag");
}
