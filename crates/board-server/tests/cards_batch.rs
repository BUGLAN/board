//! 通用批量卡片接口（default/main#595）：一次请求完成移动、稍后再做和回收站操作，
//! 单卡失败不影响其他卡片；访问令牌不能调用界面批量接口。

mod common;

use board_server::auth::Auth;
use common::{send, Res, Server, UI};
use serde_json::json;

fn new_card(s: &Server, title: &str) -> i64 {
    let r = send("POST", &format!("{}/api/cmd", s.url), &[UI], Some(json!({ "board": "default/main", "op": "new", "title": title })));
    assert_eq!(r.status, 200, "{}", r.text);
    r.body["id"].as_i64().expect("card.id")
}

fn batch(s: &Server, body: serde_json::Value) -> Res {
    send("POST", &format!("{}/api/cards/batch", s.url), &[UI], Some(body))
}

#[test]
fn card_batch_updates_many_cards_in_one_request() {
    let s = Server::start(Auth::Local);
    for title in ["一", "二", "三"] {
        new_card(&s, title);
    }
    let del = send("POST", &format!("{}/api/cmd", s.url), &[UI], Some(json!({ "board": "default/main", "op": "delete", "id": 3 })));
    assert_eq!(del.status, 200, "{}", del.text);
    let rev = || send("GET", &format!("{}/api/workspace", s.url), &[], None).body["rev"].as_i64().unwrap();

    let before = rev();
    let r = batch(&s, json!({ "board": "default/main", "op": "move", "column": "review", "ids": [2, 1, 2, 3, 99] }));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["ok"], json!([2, 1]));
    assert_eq!(r.body["failed"][0]["id"], 3);
    assert_eq!(r.body["failed"][0]["code"], "IN_TRASH");
    assert_eq!(r.body["failed"][1]["id"], 99);
    assert_eq!(r.body["failed"][1]["code"], "NOT_FOUND");
    assert_eq!(r.body["rev"], before + 1);
    assert_eq!(rev(), before + 1, "一次批量写入只让 rev 加一");

    let b = send("GET", &format!("{}/api/board?path=default/main", s.url), &[], None);
    assert_eq!(b.body["cols"]["review"], json!([2, 1]));
    assert_eq!(b.body["trash"], 1);
}

#[test]
fn access_tokens_cannot_use_card_batch() {
    let s = Server::start(Auth::Local);
    new_card(&s, "一");
    let store = board_core::Store::open(&s.db).unwrap();
    let token = board_server::auth::create_token_for(store.conn(), "命令行", None, board_core::now_ms()).unwrap().1;
    let bearer = format!("Bearer {token}");
    let r = send(
        "POST",
        &format!("{}/api/cards/batch", s.url),
        &[("authorization", bearer.as_str())],
        Some(json!({ "board": "default/main", "op": "delete", "ids": [1] })),
    );
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{}", r.text);
}
