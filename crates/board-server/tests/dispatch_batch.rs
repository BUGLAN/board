//! 批量派发（default/main#737）：`POST /api/dispatches/batch` 一次请求派整批，按机器「一个会话最多做几张」排成几个会话；
//! 校验不过的卡记在 failed，其余照常派，返回格式同 `POST /api/cards/batch`。

mod common;

use board_server::auth::Auth;
use common::{send, Server, UI};
use serde_json::json;

fn new_card(s: &Server, title: &str) -> i64 {
    let r = send("POST", &format!("{}/api/cmd", s.url), &[UI], Some(json!({ "board": "default/main", "op": "new", "title": title })));
    assert_eq!(r.status, 200, "{}", r.text);
    r.body["id"].as_i64().expect("card.id")
}

#[test]
fn dispatch_batch_groups_cards_into_sessions_in_one_request() {
    let s = Server::start(Auth::Local);
    let beat = json!({
        "name": "mbp",
        "instance": "i1",
        "os": "macOS 15.4",
        "version": env!("CARGO_PKG_VERSION"),
        "agents": [{ "id": "claude", "name": "Claude Code", "version": "2.4.1", "found": true }],
        "workspace": { "path": "/tmp" },
        "selftest": true,
    });
    let r = send("POST", &format!("{}/api/hosts/mbp/beat", s.url), &[UI], Some(beat));
    assert_eq!(r.status, 200, "{}", r.text);
    let r = send("PATCH", &format!("{}/api/hosts/mbp", s.url), &[UI], Some(json!({ "max": 2, "perSession": 2 })));
    assert_eq!(r.status, 200, "{}", r.text);
    let cards: Vec<i64> = (0..4).map(|i| new_card(&s, &format!("卡 {i}"))).collect();
    let rev = || send("GET", &format!("{}/api/workspace", s.url), &[], None).body["rev"].as_i64().unwrap();
    let before = rev();
    let r = send(
        "POST",
        &format!("{}/api/dispatches/batch", s.url),
        &[UI],
        Some(json!({ "board": "default/main", "cards": [cards[0], cards[1], 99, cards[2], cards[3]], "host": "mbp", "agent": "claude" })),
    );
    assert_eq!(r.status, 200, "{}", r.text);
    let ok = r.body["ok"].as_array().unwrap();
    assert_eq!(ok.iter().map(|d| d["card"].as_i64().unwrap()).collect::<Vec<_>>(), cards);
    assert!(ok.iter().all(|d| d["total"] == 2), "{}", r.text);
    assert_eq!(ok[1]["session"], ok[0]["id"]);
    assert_eq!(ok[3]["session"], ok[2]["id"]);
    assert_eq!((r.body["failed"][0]["id"].as_i64(), r.body["failed"][0]["code"].as_str()), (Some(99), Some("NOT_FOUND")));
    assert_eq!(r.body["rev"], before + 1, "一次写入");
    // board host 只拿到两个会话的第 1 张
    let w = send("GET", &format!("{}/api/hosts/mbp/work", s.url), &[UI], None);
    let queued: Vec<i64> = w.body["queued"].as_array().unwrap().iter().map(|d| d["card"].as_i64().unwrap()).collect();
    assert_eq!(queued, vec![cards[0], cards[2]], "{}", w.text);
}
