//! `board versions sync`（PRD §9、§10.15）：在代码仓库里从 git tag 和提交反推版本，卡片带上版本标签。

mod common;

use common::start;
use std::path::Path;
use std::process::Command;

const ALPHA: &str = "test/alpha";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env_remove("GIT_DIR")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}：{}", String::from_utf8_lossy(&out.stderr));
}

fn commit(dir: &Path, msg: &str) {
    git(dir, &["commit", "--allow-empty", "-q", "-m", msg]);
}

#[test]
fn versions_sync_labels_cards_from_git_history() {
    let s = start();
    let a = s.run(ALPHA, &["new", "第一版的需求", "--json"]).json()["id"].as_i64().unwrap();
    let b = s.run(ALPHA, &["new", "第二版的需求", "--json"]).json()["id"].as_i64().unwrap();
    let c = s.run(ALPHA, &["new", "还没发版的", "--json"]).json()["id"].as_i64().unwrap();

    // 仓库：v0.1.0 有 #a 和一个没写卡片行的提交，v0.2.0 有 #b，之后还有 #c（没打 tag）
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    git(dir, &["init", "-q"]);
    commit(dir, &format!("feat: 第一版\n\n卡片：default/main#{a}"));
    commit(dir, "docs: 没写卡片行");
    commit(dir, "chore: 发版 v0.1.0");
    git(dir, &["tag", "v0.1.0"]);
    commit(dir, &format!("fix: 第二版\n\n卡片：default/main#{b}\n卡片：default/main#999"));
    git(dir, &["tag", "-a", "v0.2.0", "-m", "v0.2.0"]);
    git(dir, &["tag", "not-a-version"]);
    commit(dir, &format!("feat: 之后的\n\n卡片：default/main#{c}"));

    let d = dir.to_str().unwrap();
    // --dry-run：只列出来，不写入
    let r = s.raw(&s.env(), &["versions", "sync", "--dir", d, "--dry-run", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let j = r.json();
    assert_eq!(j["project"], "default");
    let names: Vec<&str> = j["versions"].as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["v0.2.0", "v0.1.0"]);
    // 发版提交不算：v0.1.0 是两个提交，一个没写卡片行
    assert_eq!(j["versions"][1]["commits"], 2);
    assert_eq!(j["versions"][1]["loose"], 1);
    // 没有远端：未发版的提交不同步，写明原因
    assert!(j["unreleased"]["skipped"].is_string(), "{j}");
    assert!(s.run(ALPHA, &["labels", "--json"]).json().as_array().unwrap().is_empty());

    // 同步：不需要 --as、--board
    let r = s.raw(&s.env(), &["versions", "sync", "--dir", d, "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let j = r.json();
    assert_eq!(j["versions"][0]["added"], serde_json::json!([format!("main#{b}")]));
    assert_eq!(j["versions"][0]["missing"], serde_json::json!(["main#999"]));
    assert_eq!(j["versions"][1]["added"], serde_json::json!([format!("main#{a}")]));
    // board show --json 里的 labels 是标签名
    let names = |id: i64| -> Vec<String> {
        let j = s.run(ALPHA, &["show", &id.to_string(), "--json"]).json();
        j["labels"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_string()).collect()
    };
    assert_eq!(names(a), vec!["v0.1.0"]);
    assert_eq!(names(b), vec!["v0.2.0"]);
    assert!(names(c).is_empty());

    // 再同步一次：结果一样，不重复打
    let r = s.raw(&s.env(), &["versions", "sync", "v0.2.0", "--dir", d, "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let j = r.json();
    assert_eq!(j["versions"].as_array().unwrap().len(), 1);
    assert_eq!(j["versions"][0]["added"], serde_json::json!([]));
    assert_eq!(j["versions"][0]["already"], 1);

    // 没有这个 tag、不在仓库里：USAGE
    let r = s.raw(&s.env(), &["versions", "sync", "v9.9.9", "--dir", d, "--json"]);
    assert_eq!(r.error_code(), "USAGE");
    let empty = tempfile::tempdir().unwrap();
    git(empty.path(), &["init", "-q"]);
    let r = s.raw(&s.env(), &["versions", "sync", "--dir", empty.path().to_str().unwrap(), "--json"]);
    assert_eq!(r.error_code(), "USAGE");
}

/// 未发版的提交（§10.4）：最新的 tag 之后、已经推上 origin 集成分支的提交顺带同步；本地没推的不算；发版之后归进新版本
#[test]
fn versions_sync_also_syncs_unreleased_commits() {
    let s = start();
    let a = s.run(ALPHA, &["new", "第一版的需求", "--json"]).json()["id"].as_i64().unwrap();
    let b = s.run(ALPHA, &["new", "还没发版的", "--json"]).json()["id"].as_i64().unwrap();

    let root = tempfile::tempdir().unwrap();
    let up = root.path().join("up.git");
    std::fs::create_dir_all(&up).unwrap();
    git(&up, &["init", "-q", "--bare", "-b", "main"]);
    let dir = root.path().join("work");
    git(root.path(), &["clone", "-q", up.to_str().unwrap(), dir.to_str().unwrap()]);
    git(&dir, &["switch", "-q", "-c", "main"]);
    commit(&dir, &format!("feat: 第一版\n\n卡片：default/main#{a}"));
    commit(&dir, "chore: 发版 v0.1.0");
    git(&dir, &["tag", "v0.1.0"]);
    commit(&dir, &format!("feat: 之后的\n\n卡片：default/main#{b}"));
    commit(&dir, "docs: 没写卡片行");
    git(&dir, &["push", "-q", "-u", "origin", "main", "--tags"]);
    commit(&dir, "feat: 本地还没推");
    let d = dir.to_str().unwrap();
    let store = || board_core::Store::open(&s.db).unwrap();
    let unreleased = || store().read(|r| r.milestones("default")).unwrap().unreleased;

    let r = s.raw(&s.env(), &["versions", "sync", "--dir", d, "--dry-run", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let j = r.json();
    assert_eq!((j["unreleased"]["base"].as_str(), j["unreleased"]["branch"].as_str()), (Some("v0.1.0"), Some("main")), "{j}");
    assert_eq!((j["unreleased"]["commits"].as_i64(), j["unreleased"]["cards"].as_i64()), (Some(2), Some(1)), "{j}");
    assert!(unreleased().is_empty(), "--dry-run 不写入");

    let r = s.raw(&s.env(), &["versions", "sync", "--dir", d, "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!((r.json()["unreleased"]["added"].as_i64(), r.json()["unreleased"]["total"].as_i64()), (Some(2), Some(2)));
    let u = unreleased();
    assert_eq!(u.iter().map(|c| c.s.as_str()).collect::<Vec<_>>(), vec!["docs: 没写卡片行", "feat: 之后的"]);
    assert_eq!(u[1].cards, vec![format!("main#{b}")]);
    // 文本输出里有「未发版」一行
    let r = s.raw(&s.env(), &["versions", "sync", "--dir", d]);
    assert!(r.out.contains("未发版") && r.out.contains("2 个提交"), "{}", r.out);

    // 发 v0.2.0：只同步这个 tag，之前的未发版提交归进它，未发版清空
    git(&dir, &["reset", "-q", "--hard", "origin/main"]);
    commit(&dir, "chore: 发版 v0.2.0");
    git(&dir, &["tag", "v0.2.0"]);
    let r = s.raw(&s.env(), &["versions", "sync", "v0.2.0", "--dir", d, "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["unreleased"]["total"].as_i64(), Some(0), "{}", r.out);
    assert!(unreleased().is_empty());
    let m = store().read(|r| r.milestones("default")).unwrap();
    assert_eq!(m.versions[0].name, "v0.2.0");
    assert_eq!(m.versions[0].commits.len(), 2);
}

/// 无网络时 `versions sync` 也要给出可读的错误并以退出码 1 结束，不能 panic
#[test]
fn sync_offline_reports_unreachable_without_panicking() {
    let s = start();
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    git(dir, &["init", "-q"]);
    commit(dir, "feat: 第一版\n\n卡片：default/main#1");
    git(dir, &["tag", "v0.1.0"]);

    let env = vec![
        ("BOARD_URL", "http://127.0.0.1:9".to_string()),
        ("BOARD_TOKEN", "x".to_string()),
    ];
    let d = dir.to_str().unwrap();
    let r = s.raw(&env, &["versions", "sync", "--dir", d, "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "UNREACHABLE"), "{}{}", r.out, r.err);
    assert!(r.out.contains("连不上"), "错误信息要可读：{}", r.out);
    assert!(!r.out.contains("panicked"), "不能 panic：{}", r.out);

    // 文本模式也同样：错误码写在 stderr，退出码 1
    let r = s.raw(&env, &["versions", "sync", "--dir", d]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    assert!(r.err.contains("UNREACHABLE") && r.err.contains("连不上"), "{}", r.err);
    assert!(!r.err.contains("panicked"), "不能 panic：{}", r.err);
}
