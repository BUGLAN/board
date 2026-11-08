//! CLI 层面的验收标准（PRD §13）：身份与白板定位、退出码、--json、连接配置。
//! 所有命令都经测试服务端执行（tests/common）。

mod common;

use common::{start, Server};

const ALPHA: &str = "test/alpha";

#[test]
fn ac11_json_and_exit_codes() {
    let s = start();
    for args in [
        vec!["new", "x", "--json"],
        vec!["ls", "--json"],
        vec!["show", "1", "--json"],
        vec!["claim", "--json"],
        vec!["comment", "1", "进展", "--json"],
        vec!["edit", "1", "--body", "b", "--json"],
        vec!["move", "1", "review", "--json"],
        vec!["boards", "--json"],
        vec!["labels", "--json"],
    ] {
        let r = s.run(ALPHA, &args);
        assert_eq!(r.code, 0, "{args:?}: {}{}", r.out, r.err);
        r.json();
    }
    // 被拒绝：退出码 1，JSON 带错误码
    let r = s.run(ALPHA, &["move", "1", "doing", "--json"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.error_code(), "FORBIDDEN_COLUMN");
    let r = s.run(ALPHA, &["show", "99", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_FOUND"));
    let r = s.run(ALPHA, &["frobnicate", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = s.run(ALPHA, &["move", "1", "nowhere", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = s.run(ALPHA, &["comment", "1", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    // 文本模式：错误码写在 stderr
    let r = s.run(ALPHA, &["move", "1", "doing"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("FORBIDDEN_COLUMN"), "{}", r.err);
    // help 不需要身份
    assert_eq!(s.raw(&[], &["help"]).code, 0);
}

#[test]
fn ac1_claim_json_returns_full_card() {
    let s = start();
    s.run(ALPHA, &["new", "一"]);
    s.run(ALPHA, &["new", "二", "--body", "gh#140"]);
    let c = s.run(ALPHA, &["claim", "--json"]).json();
    assert_eq!(c["id"], 1);
    assert_eq!(c["column"], "doing");
    assert_eq!(c["holder"], "test/alpha");
    assert!(c["claimed_at"].is_string());
    assert!(c["run_started_at"].is_null(), "第一次心跳才开始计时");
    // 心跳：开始第 1 次
    let r = s.run(ALPHA, &["beat", "1", "--json"]);
    assert!(r.json()["run_started_at"].is_string());
    assert_eq!(r.json()["work"]["runs"], 1);
    assert!(c["activity"].as_array().unwrap().len() >= 2);
    // --search 在描述里查找
    let r = s.run(ALPHA, &["ls", "--search", "GH#140", "--json"]);
    let ids: Vec<i64> = r.json().as_array().unwrap().iter().map(|c| c["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn ac12_identity_and_board_required() {
    let s = start();
    let with = |extra: &[(&'static str, &str)]| {
        let mut env = s.env();
        env.extend(extra.iter().map(|(k, v)| (*k, v.to_string())));
        env
    };
    let r = s.raw(&with(&[]), &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NO_IDENTITY"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("board check --as"));
    let r = s.raw(&with(&[("BOARD_AS", "test/alpha")]), &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NO_BOARD"));
    // --as / --board 与环境变量等价
    let r = s.raw(&with(&[]), &["ls", "--as", "test/alpha", "--board", "default/main", "--json"]);
    assert_eq!(r.code, 0, "{}", r.out);
    // boards 只需要身份
    let r = s.raw(&with(&[("BOARD_AS", "test/alpha")]), &["boards", "--json"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.json()[0]["board"], "default/main");
    let r = s.raw(&with(&[("BOARD_AS", "test/alpha"), ("BOARD", "nope")]), &["ls", "--json"]);
    assert_eq!(r.error_code(), "BOARD_NOT_FOUND");
}

#[test]
fn ac23_24_labels() {
    let s = start();
    s.run(ALPHA, &["new", "a"]);
    s.create_label("bug");
    s.run(ALPHA, &["new", "b"]);
    let r = s.run(ALPHA, &["label", "1", "+bug", "--json"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(r.json()["labels"][0], "bug");
    let r = s.run(ALPHA, &["label", "1", "+不存在的标签", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("bug"), "原因里列出可用的标签");
    let r = s.run(ALPHA, &["ls", "--label", "BUG", "--json"]);
    let ids: Vec<i64> = r.json().as_array().unwrap().iter().map(|c| c["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![1]);
    let r = s.run(ALPHA, &["labels", "--json"]);
    assert_eq!(r.json()[0]["cards"], 1);
    let r = s.run(ALPHA, &["label", "1", "-bug", "--json"]);
    assert!(r.json()["labels"].as_array().unwrap().is_empty());
}

/// 附件：上传成一条评论、show 列出、files 下载到本地
#[test]
fn attach_and_download_files() {
    let s = start();
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("截图.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
    let log = dir.path().join("run.log");
    std::fs::write(&log, "ok\n").unwrap();
    s.run(ALPHA, &["new", "带附件"]);

    let r = s.run(ALPHA, &["attach", "1", png.to_str().unwrap(), log.to_str().unwrap(), "--comment", "复现截图和日志", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let v = r.json();
    assert_eq!(v["attachments"][0]["name"], "截图.png");
    assert_eq!(v["attachments"][0]["image"], true);
    let comment = v["card"]["activity"].as_array().unwrap().last().unwrap()["text"].as_str().unwrap().to_string();
    assert!(comment.starts_with("复现截图和日志\n\n![截图.png](/api/files/1)"), "{comment}");
    assert!(comment.ends_with("[run.log](/api/files/2)"), "{comment}");

    // show 列出附件，提示怎么下载
    let r = s.run(ALPHA, &["show", "1"]);
    assert!(r.out.contains("附件  board files 1 下载到本地查看"), "{}", r.out);
    assert!(r.out.contains("截图.png"));
    assert_eq!(s.run(ALPHA, &["show", "1", "--json"]).json()["attachments"][1]["name"], "run.log");

    // files 下载到指定目录
    let out = dir.path().join("dl");
    let r = s.run(ALPHA, &["files", "1", "--out", out.to_str().unwrap(), "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json().as_array().unwrap().len(), 2);
    assert_eq!(std::fs::read(out.join("2-run.log")).unwrap(), b"ok\n");
    assert!(out.join("1-截图.png").exists());

    // 错误：不支持的类型、文件不存在、卡片不存在、没有附件
    let svg = dir.path().join("x.svg");
    std::fs::write(&svg, "<svg/>").unwrap();
    let r = s.run(ALPHA, &["attach", "1", svg.to_str().unwrap(), "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = s.run(ALPHA, &["attach", "1", "nope.png", "--json"]);
    assert_eq!(r.error_code(), "USAGE");
    let r = s.run(ALPHA, &["attach", "9", png.to_str().unwrap(), "--json"]);
    assert_eq!(r.error_code(), "NOT_FOUND");
    s.run(ALPHA, &["new", "没附件"]);
    assert!(s.run(ALPHA, &["files", "2"]).out.contains("#2 没有附件"));
}

/// 没有配置服务端：直接失败，提示运行 board login；不会在 HOME 下建本地数据库
#[test]
fn requires_login() {
    let s: Server = start();
    let who = [("BOARD_AS", "test/alpha".to_string()), ("BOARD", "default".to_string())];

    // 环境变量和配置文件都没有
    for args in [vec!["ls", "--json"], vec!["boards", "--json"], vec!["claim", "--json"], vec!["check", "--json"], vec!["beat", "--json"]] {
        let r = s.raw(&who, &args);
        assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_LOGGED_IN"), "{args:?}");
        let msg = r.json()["error"]["message"].as_str().unwrap().to_string();
        assert!(msg.contains("board login") && msg.contains("设置 → Agent 接入"), "{msg}");
    }
    let r = s.raw(&who, &["ls"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("NOT_LOGGED_IN") && r.err.contains("board login"), "{}", r.err);
    // 即使给了 BOARD_DB 也不再读写本地数据库
    let db = s.home.join("x.db");
    let mut env = who.to_vec();
    env.push(("BOARD_DB", db.to_string_lossy().into()));
    assert_eq!(s.raw(&env, &["ls", "--json"]).error_code(), "NOT_LOGGED_IN");
    assert!(!db.exists());

    // 有地址但没有令牌
    let mut env = who.to_vec();
    env.push(("BOARD_URL", s.url.clone()));
    let r = s.raw(&env, &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_LOGGED_IN"));
    let msg = r.json()["error"]["message"].as_str().unwrap().to_string();
    assert!(msg.contains("board login") && msg.contains("设置 → Agent 接入"), "{msg}");

    // help 照常可用，并说明怎么配置
    let r = s.raw(&[], &["help"]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("board login") && r.out.contains("设置 → Agent 接入") && !r.out.contains("本地数据库"), "{}", r.out);

    assert!(!s.home.join(".board").exists(), "不应该在 HOME 下创建 ~/.board");
}
