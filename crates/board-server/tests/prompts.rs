//! 快捷提示词（#572）：`GET /api/prompts` 整列、`PUT /api/prompts` 整列保存；访问令牌用不了，写请求要带 x-board-client。

mod common;

use board_server::auth::Auth;
use common::{send, Server, UI};
use serde_json::json;

#[test]
fn prompts_are_saved_as_a_whole_list_on_the_server() {
    let s = Server::start(Auth::Local);
    let url = format!("{}/api/prompts", s.url);

    let r = send("GET", &url, &[UI], None);
    assert_eq!((r.status, &r.body), (200, &json!({ "prompts": [] })), "{}", r.text);

    let list = json!({ "prompts": [
        { "id": "p-2", "title": " 重启 ", "text": "重启一下\r\n再看日志" },
        { "id": "p-1", "title": "等发版", "text": "等发版" },
    ] });
    let r = send("PUT", &url, &[UI], Some(list));
    assert_eq!(r.status, 200, "{}", r.text);
    let saved = json!({ "prompts": [
        { "id": "p-2", "title": "重启", "text": "重启一下\n再看日志" },
        { "id": "p-1", "title": "等发版", "text": "等发版" },
    ] });
    assert_eq!(r.body, saved);
    // 另一个窗口（没有 x-board-client 也能读）取到同一份
    let r = send("GET", &url, &[], None);
    assert_eq!(r.body, saved, "{}", r.text);

    // 不合规的整列拒绝，原来的不动
    let r = send("PUT", &url, &[UI], Some(json!({ "prompts": [{ "id": "p-1", "title": "", "text": "x" }] })));
    assert_eq!((r.status, r.code().as_str()), (400, "USAGE"), "{}", r.text);
    let r = send("PUT", &url, &[UI], Some(json!({ "prompts": [{ "id": "a", "title": "a", "text": "x" }, { "id": "a", "title": "b", "text": "y" }] })));
    assert_eq!(r.code(), "USAGE", "{}", r.text);
    let r = send("PUT", &url, &[UI], Some(json!({ "list": [] })));
    assert_eq!(r.code(), "USAGE", "{}", r.text);
    // 写请求不带 x-board-client：拒绝（防跨站请求）
    let r = send("PUT", &url, &[], Some(json!({ "prompts": [] })));
    assert_eq!(r.code(), "USAGE", "{}", r.text);
    assert_eq!(send("GET", &url, &[UI], None).body, saved);

    // 清空
    let r = send("PUT", &url, &[UI], Some(json!({ "prompts": [] })));
    assert_eq!(r.body, json!({ "prompts": [] }), "{}", r.text);
}

#[test]
fn access_tokens_cannot_read_or_write_prompts() {
    let s = Server::start(Auth::Local);
    let store = board_core::Store::open(&s.db).unwrap();
    let tok = board_server::auth::create_token_for(store.conn(), "命令行", None, board_core::now_ms()).unwrap().1;
    let bearer = format!("Bearer {tok}");
    let headers = [("authorization", bearer.as_str()), UI];
    let url = format!("{}/api/prompts", s.url);
    let r = send("GET", &url, &headers, None);
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{}", r.text);
    let r = send("PUT", &url, &headers, Some(json!({ "prompts": [] })));
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{}", r.text);
}
