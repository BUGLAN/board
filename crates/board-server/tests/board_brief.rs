//! `GET /api/board?brief=1`：不带卡片和事件的快照。命令行的大多数命令只用得到工作流和每个状态的张数，
//! 不用每次传整块白板的卡片（终点状态攒了几百张时有几百 KB）。
//! `labels=1`：带上所在项目的标签，命令行不用为了标签名再取整个工作区。
//!
//! 起一个真实的 board-server（后台线程、随机端口），本机模式（Auth::Local）不需要登录。

mod common;

use board_server::auth::Auth;
use common::{send, Server};
use serde_json::json;

fn new_card(s: &Server, title: &str) -> i64 {
    let r = send("POST", &format!("{}/api/cmd", s.url), &[common::UI], Some(json!({ "board": "default/main", "op": "new", "title": title })));
    assert_eq!(r.status, 200, "{}", r.text);
    r.body["id"].as_i64().expect("card.id")
}

#[test]
fn brief_snapshot_has_flow_and_ids_but_no_cards() {
    let s = Server::start(Auth::Local);
    let a = new_card(&s, "一");
    let b = new_card(&s, "二");

    let full = send("GET", &format!("{}/api/board?path=default/main", s.url), &[], None);
    assert_eq!(full.status, 200, "{}", full.text);
    assert_eq!(full.body["cards"].as_object().unwrap().len(), 2);
    assert!(!full.body["log"].as_array().unwrap().is_empty());

    let brief = send("GET", &format!("{}/api/board?path=default/main&brief=1", s.url), &[], None);
    assert_eq!(brief.status, 200, "{}", brief.text);
    // 卡片和事件不带
    assert!(brief.body["cards"].as_object().unwrap().is_empty(), "{}", brief.text);
    assert!(brief.body["log"].as_array().unwrap().is_empty(), "{}", brief.text);
    // 其余和普通快照一样：工作流、每个状态里的卡片编号、白板路径
    assert_eq!(brief.body["statuses"], full.body["statuses"]);
    assert_eq!(brief.body["cols"], full.body["cols"]);
    assert_eq!(brief.body["cols"]["todo"], json!([a, b]));
    assert_eq!(brief.body["path"], "default/main");
    assert_eq!(brief.body["rev"], full.body["rev"]);
}

#[test]
fn labels_come_with_the_snapshot_only_when_asked() {
    let s = Server::start(Auth::Local);
    let r = send("POST", &format!("{}/api/labels", s.url), &[common::UI], Some(json!({ "project": "default", "name": "bug" })));
    assert_eq!(r.status, 200, "{}", r.text);

    // 界面不带这个参数：快照里没有 labels 字段
    let plain = send("GET", &format!("{}/api/board?path=default/main", s.url), &[], None);
    assert!(plain.body.get("labels").is_none(), "{}", plain.text);

    // 带 labels=1：和工作区里这个项目的标签一样（含使用张数），可以和 brief 一起用
    let ws = send("GET", &format!("{}/api/workspace", s.url), &[], None);
    let want = ws.body["projects"].as_array().unwrap().iter().find(|p| p["key"] == "default").unwrap()["labels"].clone();
    assert!(want.as_array().unwrap().iter().any(|l| l["name"] == "bug"), "{}", ws.text);
    for q in ["labels=1", "brief=1&labels=1", "full=1&labels=1"] {
        let r = send("GET", &format!("{}/api/board?path=default/main&{q}", s.url), &[], None);
        assert_eq!(r.status, 200, "{q}: {}", r.text);
        assert_eq!(r.body["labels"], want, "{q}");
    }
}
