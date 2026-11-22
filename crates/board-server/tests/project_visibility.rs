//! 项目在左侧项目面板的显示开关（default/main#610）：`PATCH /api/projects/{项目} {hidden}`。

mod common;

use board_server::auth::Auth;
use common::{send, Server};
use serde_json::{json, Value};

fn project(s: &Server, key: &str) -> Value {
    let r = send("GET", &format!("{}/api/workspace", s.url), &[], None);
    assert_eq!(r.status, 200, "{}", r.text);
    r.body["projects"].as_array().unwrap().iter().find(|p| p["key"] == key).cloned().expect("项目")
}

fn patch(s: &Server, body: Value) -> common::Res {
    send("PATCH", &format!("{}/api/projects/default", s.url), &[common::UI], Some(body))
}

#[test]
fn project_hidden_is_saved_and_defaults_to_visible() {
    let s = Server::start(Auth::Local);
    assert_eq!(project(&s, "default")["hidden"], false);

    let r = patch(&s, json!({ "hidden": true }));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(project(&s, "default")["hidden"], true);

    let r = patch(&s, json!({ "name": "改名" }));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(project(&s, "default")["hidden"], true, "只改别的字段不动开关");

    let r = patch(&s, json!({ "hidden": false }));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(project(&s, "default")["hidden"], false);
}
