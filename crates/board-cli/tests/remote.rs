//! 连接服务端（PRD §9）：CLI 通过 BOARD_URL + BOARD_TOKEN 读写白板，规则在服务端执行，
//! 被拒绝时错误码、退出码原样还原。测试服务端见 tests/common。

mod common;

use board_server::auth;
use common::{board, start, Run};

#[test]
fn remote_rules_and_exit_codes() {
    let s = start();
    for args in [
        vec!["new", "一", "--json"],
        vec!["new", "二", "--body", "gh#140", "--json"],
        vec!["ls", "--json"],
        vec!["show", "1", "--json"],
        vec!["flow", "--json"],
        vec!["boards", "--json"],
        vec!["labels", "--json"],
    ] {
        let r = s.run("alpha", &args);
        assert_eq!(r.code, 0, "{args:?}: {}{}", r.out, r.err);
        r.json();
    }
    // 验收 1：领取最上面的卡片，进入「进行中」
    let c = s.run("alpha", &["claim", "--json"]).json();
    assert_eq!((c["id"].as_i64(), c["column"].as_str(), c["holder"].as_str()), (Some(1), Some("doing"), Some("alpha")));
    assert!(c["claimed_at"].is_string());
    let c = s.run("alpha", &["beat", "1", "--json"]).json();
    assert_eq!(c["work"]["runs"], 1);
    // 验收 3
    assert_eq!(s.run("alpha", &["claim", "--json"]).error_code(), "ALREADY_HOLDING");
    // 验收 4
    assert_eq!(s.run("beta", &["edit", "1", "--title", "x", "--json"]).error_code(), "NOT_HOLDER");
    assert_eq!(s.run("beta", &["comment", "1", "看看", "--json"]).code, 0);
    // 验收 7
    let r = s.run("alpha", &["move", "1", "done", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "FORBIDDEN_COLUMN"));
    let r = s.run("alpha", &["move", "1", "review", "--json"]);
    assert_eq!(r.code, 0);
    assert!(r.json()["holder"].is_null());
    // 验收 8
    assert_eq!(s.run("alpha", &["move", "2", "doing", "--json"]).error_code(), "FORBIDDEN_COLUMN");
    // 其他拒绝
    assert_eq!(s.run("alpha", &["show", "99", "--json"]).error_code(), "NOT_FOUND");
    assert_eq!(s.run("alpha", &["move", "1", "nowhere", "--json"]).error_code(), "USAGE");
    assert_eq!(s.run("alpha", &["label", "1", "+不存在", "--json"]).error_code(), "USAGE");
    // 文本模式：错误码写在 stderr，退出码 1
    let r = s.run("alpha", &["move", "2", "doing"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("FORBIDDEN_COLUMN"), "{}", r.err);
    // --search
    let ids: Vec<i64> = s.run("alpha", &["ls", "--search", "GH#140", "--json"]).json().as_array().unwrap().iter().map(|c| c["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![2]);
    // 白板定位（验收 12、13）
    let r = board(&s.home, &[("BOARD_URL", s.url.clone()), ("BOARD_TOKEN", s.token.clone()), ("BOARD_AS", "a".into())], &["ls", "--json"]);
    assert_eq!(r.error_code(), "NO_BOARD");
    let r = board(&s.home, &[("BOARD_URL", s.url.clone()), ("BOARD_TOKEN", s.token.clone()), ("BOARD_AS", "a".into()), ("BOARD", "nope".into())], &["ls", "--json"]);
    assert_eq!(r.error_code(), "BOARD_NOT_FOUND");
    // 动态里记的是 agent 名字和 CLI
    let show = s.run("alpha", &["show", "1", "--json"]).json();
    let acts = show["activity"].as_array().unwrap();
    assert!(acts.iter().any(|a| a["actor"] == "beta" && a["type"] == "comment"));
    // 不碰 HOME 下的本地数据库
    assert!(!s.home.join(".board").exists(), "不应该创建 ~/.board");
}

#[test]
fn remote_concurrent_claim_only_one_wins() {
    let s = start();
    s.run("alpha", &["new", "x"]);
    let handles: Vec<_> = ["alpha", "beta", "gamma", "delta"]
        .into_iter()
        .map(|who| {
            let mut env = s.env();
            env.push(("BOARD_AS", who.into()));
            env.push(("BOARD", "default".into()));
            let home = s.home.clone();
            std::thread::spawn(move || board(&home, &env, &["claim", "1", "--json"]))
        })
        .collect();
    let runs: Vec<Run> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let ok = runs.iter().filter(|r| r.code == 0).count();
    assert_eq!(ok, 1, "{:?}", runs.iter().map(|r| &r.out).collect::<Vec<_>>());
    assert!(runs.iter().filter(|r| r.code != 0).all(|r| r.error_code() == "NOT_IN_TODO"));
}

#[test]
fn remote_auth_errors() {
    let s = start();
    let base = |extra: Vec<(&'static str, String)>| {
        let mut env = vec![("BOARD_AS", "alpha".to_string()), ("BOARD", "default".to_string())];
        env.extend(extra);
        env
    };
    // 设置了 BOARD_URL 但没有令牌
    let r = board(&s.home, &base(vec![("BOARD_URL", s.url.clone())]), &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_LOGGED_IN"));
    // 令牌无效
    let r = board(&s.home, &base(vec![("BOARD_URL", s.url.clone()), ("BOARD_TOKEN", "bd_nope".into())]), &["claim", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "INVALID_TOKEN"));
    // 令牌被吊销
    {
        let store = board_core::Store::open(&s.db).unwrap();
        let id = auth::list_tokens(store.conn()).unwrap()[0].id;
        auth::revoke_token(store.conn(), id).unwrap();
    }
    let r = s.run("alpha", &["ls", "--json"]);
    assert_eq!(r.error_code(), "INVALID_TOKEN");
    // 连不上服务端
    let r = board(&s.home, &base(vec![("BOARD_URL", "http://127.0.0.1:9".into()), ("BOARD_TOKEN", "x".into())]), &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "UNREACHABLE"));
    let r = board(&s.home, &base(vec![("BOARD_URL", "http://127.0.0.1:9".into()), ("BOARD_TOKEN", "x".into())]), &["ls"]);
    assert!(r.err.contains("UNREACHABLE"), "{}", r.err);
    // 身份仍然先检查
    let r = board(&s.home, &[("BOARD_URL", s.url.clone())], &["ls", "--json"]);
    assert_eq!(r.error_code(), "NO_IDENTITY");
}

/// 附件经服务端上传、读取
#[test]
fn remote_attachments() {
    let s = start();
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("shot.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
    s.run("alpha", &["new", "带图"]);
    let r = s.run("alpha", &["attach", "1", png.to_str().unwrap(), "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let v = r.json();
    assert_eq!(v["attachments"][0]["created_by"], "alpha");
    assert_eq!(v["card"]["attachments"][0]["url"], "/api/files/1");
    let out = dir.path().join("dl");
    let r = s.run("beta", &["files", "1", "--out", out.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("已下载 1 个附件"), "{}", r.out);
    assert_eq!(std::fs::read(out.join("1-shot.png")).unwrap(), std::fs::read(&png).unwrap());
    // 服务端的拒绝原样还原
    let txt = dir.path().join("x.exe");
    std::fs::write(&txt, "MZ").unwrap();
    let r = s.run("alpha", &["attach", "1", txt.to_str().unwrap(), "--json"]);
    assert_eq!(r.error_code(), "USAGE");
    assert!(!s.home.join(".board").exists(), "不应该创建 ~/.board");
}
