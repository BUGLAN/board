//! CLI 层面的验收标准（PRD §13）：身份与白板定位、退出码、--json。

use serde_json::Value;
use std::path::Path;
use std::process::Command;

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.out).unwrap_or_else(|e| panic!("不是合法 JSON：{e}\n{}", self.out))
    }
    fn error_code(&self) -> String {
        self.json()["error"]["code"].as_str().unwrap_or_default().to_string()
    }
}

fn board(db: &Path, env: &[(&str, &str)], args: &[&str]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_board"));
    // 只用临时数据库：去掉开发者环境里连接线上服务端的变量，免得测试写到真实白板上
    cmd.args(args)
        .env("BOARD_DB", db)
        .env_remove("BOARD")
        .env_remove("BOARD_AS")
        .env_remove("BOARD_URL")
        .env_remove("BOARD_TOKEN")
        .env("NO_COLOR", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().unwrap();
    Run {
        code: o.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&o.stdout).into(),
        err: String::from_utf8_lossy(&o.stderr).into(),
    }
}

const ALPHA: &[(&str, &str)] = &[("BOARD_AS", "alpha"), ("BOARD", "default")];

#[test]
fn ac11_json_and_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
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
        let r = board(&db, ALPHA, &args);
        assert_eq!(r.code, 0, "{args:?}: {}{}", r.out, r.err);
        r.json();
    }
    // 被拒绝：退出码 1，JSON 带错误码
    let r = board(&db, ALPHA, &["move", "1", "doing", "--json"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.error_code(), "FORBIDDEN_COLUMN");
    let r = board(&db, ALPHA, &["show", "99", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_FOUND"));
    let r = board(&db, ALPHA, &["frobnicate", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = board(&db, ALPHA, &["move", "1", "nowhere", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = board(&db, ALPHA, &["comment", "1", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    // 文本模式：错误码写在 stderr
    let r = board(&db, ALPHA, &["move", "1", "doing"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("FORBIDDEN_COLUMN"), "{}", r.err);
    // help 不需要身份
    assert_eq!(board(&db, &[], &["help"]).code, 0);
}

#[test]
fn ac1_claim_json_returns_full_card() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
    board(&db, ALPHA, &["new", "一"]);
    board(&db, ALPHA, &["new", "二", "--body", "gh#140"]);
    let r = board(&db, ALPHA, &["claim", "--json"]);
    let c = r.json();
    assert_eq!(c["id"], 1);
    assert_eq!(c["column"], "doing");
    assert_eq!(c["holder"], "alpha");
    assert!(c["expires_at"].is_string());
    assert!(c["activity"].as_array().unwrap().len() >= 2);
    // --search 在描述里查找
    let r = board(&db, ALPHA, &["ls", "--search", "GH#140", "--json"]);
    let ids: Vec<i64> = r.json().as_array().unwrap().iter().map(|c| c["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn ac12_identity_and_board_required() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
    let r = board(&db, &[], &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NO_IDENTITY"));
    let r = board(&db, &[("BOARD_AS", "alpha")], &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NO_BOARD"));
    // --as / --board 与环境变量等价
    let r = board(&db, &[], &["ls", "--as", "alpha", "--board", "default/main", "--json"]);
    assert_eq!(r.code, 0, "{}", r.out);
    // boards 只需要身份
    let r = board(&db, &[("BOARD_AS", "alpha")], &["boards", "--json"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.json()[0]["board"], "default/main");
    let r = board(&db, &[("BOARD_AS", "alpha"), ("BOARD", "nope")], &["ls", "--json"]);
    assert_eq!(r.error_code(), "BOARD_NOT_FOUND");
}

#[test]
fn ac23_24_labels() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
    board(&db, ALPHA, &["new", "a"]);
    // 标签只能在界面上新建；这里直接写库
    {
        let mut s = board_core::Store::open(&db).unwrap();
        s.write(0, board_core::Origin::ui(None), |w| w.create_label("default", "bug", None).map(|_| ())).unwrap();
    }
    board(&db, ALPHA, &["new", "b"]);
    let r = board(&db, ALPHA, &["label", "1", "+bug", "--json"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(r.json()["labels"][0], "bug");
    let r = board(&db, ALPHA, &["label", "1", "+不存在的标签", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("bug"), "原因里列出可用的标签");
    let r = board(&db, ALPHA, &["ls", "--label", "BUG", "--json"]);
    let ids: Vec<i64> = r.json().as_array().unwrap().iter().map(|c| c["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![1]);
    let r = board(&db, ALPHA, &["labels", "--json"]);
    assert_eq!(r.json()[0]["cards"], 1);
    let r = board(&db, ALPHA, &["label", "1", "-bug", "--json"]);
    assert!(r.json()["labels"].as_array().unwrap().is_empty());
}

/// 附件：上传成一条评论、show 列出、files 下载到本地（本地数据库模式）
#[test]
fn attach_and_download_files_locally() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
    let png = dir.path().join("截图.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
    let log = dir.path().join("run.log");
    std::fs::write(&log, "ok\n").unwrap();
    board(&db, ALPHA, &["new", "带附件"]);

    let r = board(&db, ALPHA, &["attach", "1", png.to_str().unwrap(), log.to_str().unwrap(), "--comment", "复现截图和日志", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let v = r.json();
    assert_eq!(v["attachments"][0]["name"], "截图.png");
    assert_eq!(v["attachments"][0]["image"], true);
    let comment = v["card"]["activity"].as_array().unwrap().last().unwrap()["text"].as_str().unwrap().to_string();
    assert!(comment.starts_with("复现截图和日志\n\n![截图.png](/api/files/1)"), "{comment}");
    assert!(comment.ends_with("[run.log](/api/files/2)"), "{comment}");

    // show 列出附件，提示怎么下载
    let r = board(&db, ALPHA, &["show", "1"]);
    assert!(r.out.contains("附件  board files 1 下载到本地查看"), "{}", r.out);
    assert!(r.out.contains("截图.png"));
    assert_eq!(board(&db, ALPHA, &["show", "1", "--json"]).json()["attachments"][1]["name"], "run.log");

    // files 下载到指定目录
    let out = dir.path().join("dl");
    let r = board(&db, ALPHA, &["files", "1", "--out", out.to_str().unwrap(), "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let saved = r.json();
    assert_eq!(saved.as_array().unwrap().len(), 2);
    assert_eq!(std::fs::read(out.join("2-run.log")).unwrap(), b"ok\n");
    assert!(out.join("1-截图.png").exists());

    // 错误：不支持的类型、文件不存在、卡片不存在、没有附件
    let svg = dir.path().join("x.svg");
    std::fs::write(&svg, "<svg/>").unwrap();
    let r = board(&db, ALPHA, &["attach", "1", svg.to_str().unwrap(), "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = board(&db, ALPHA, &["attach", "1", "nope.png", "--json"]);
    assert_eq!(r.error_code(), "USAGE");
    let r = board(&db, ALPHA, &["attach", "9", png.to_str().unwrap(), "--json"]);
    assert_eq!(r.error_code(), "NOT_FOUND");
    board(&db, ALPHA, &["new", "没附件"]);
    assert!(board(&db, ALPHA, &["files", "2"]).out.contains("#2 没有附件"));
}
