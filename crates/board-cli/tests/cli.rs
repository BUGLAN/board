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
    cmd.args(args).env("BOARD_DB", db).env_remove("BOARD").env_remove("BOARD_AS").env("NO_COLOR", "1");
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
