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
