//! 项目目录（default/main#488）：`PATCH /api/projects/{项目} {dir}`、`POST /api/projects {dir}`。
//!
//! 起一个真实的 board-server（本机模式，后台线程、随机端口），用 HTTP 访问。

mod common;

use board_server::auth::Auth;
use common::{send, Server};
use serde_json::{json, Value};

fn project(s: &Server, key: &str) -> Value {
    let r = send("GET", &format!("{}/api/workspace", s.url), &[], None);
    assert_eq!(r.status, 200, "{}", r.text);
    r.body["projects"].as_array().unwrap().iter().find(|p| p["key"] == key).cloned().expect("项目")
}

fn patch(s: &Server, key: &str, dir: &str) -> common::Res {
    send("PATCH", &format!("{}/api/projects/{key}", s.url), &[common::UI], Some(json!({ "dir": dir })))
}

#[test]
fn project_dir_is_validated_and_unique() {
    let s = Server::start(Auth::Local);
    assert_eq!(project(&s, "default")["dir"], "", "没填");
    // 合法的目录名：存下（去掉首尾空白）
    let r = patch(&s, "default", " board ");
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(project(&s, "default")["dir"], "board");
    // 不是一个目录名：USAGE，写明原因，不改
    for bad in ["../x", "a/b", "C:\\repo", "~", "..", "con", "AUX", "nul.txt"] {
        let r = patch(&s, "default", bad);
        assert_eq!((r.status, r.code().as_str()), (400, "USAGE"), "{bad}: {}", r.text);
        assert!(r.body["error"]["message"].as_str().unwrap().contains("项目目录"), "{}", r.text);
    }
    assert_eq!(project(&s, "default")["dir"], "board");
    // 新建项目可以带上；和别的项目的目录重名的不建
    let new = |key: &str, dir: Option<&str>| {
        let mut b = json!({ "name": key, "key": key, "board": { "name": "主", "key": "main" } });
        if let Some(d) = dir {
            b["dir"] = json!(d);
        }
        send("POST", &format!("{}/api/projects", s.url), &[common::UI], Some(b))
    };
    let r = new("site", Some("web"));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(project(&s, "site")["dir"], "web");
    let r = new("api", Some("BOARD"));
    assert_eq!((r.status, r.code().as_str()), (400, "USAGE"), "{}", r.text);
    let r = new("board", None);
    assert_eq!((r.status, r.code().as_str()), (400, "USAGE"), "项目标识和别的项目填的目录重名：{}", r.text);
    for bad in ["con", "AUX", "nul.txt"] {
        let r = new(bad, None);
        assert_eq!((r.status, r.code().as_str()), (400, "USAGE"), "{bad}: {}", r.text);
        assert!(r.body["error"]["message"].as_str().unwrap().contains("保留设备名"), "{}", r.text);
    }
    let r = send("GET", &format!("{}/api/workspace", s.url), &[], None);
    assert!(
        !r.body["projects"].as_array().unwrap().iter().any(|p| p["key"] == "api" || p["key"] == "board" || p["key"] == "con" || p["key"] == "AUX" || p["key"] == "nul.txt"),
        "没建成的项目不留下"
    );
    // 清空：回到项目标识
    let r = patch(&s, "default", "");
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(project(&s, "default")["dir"], "");
}
