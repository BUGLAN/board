//! 定时建卡（PRD §4「白板设置」）的接口：新建、修改、删除、「现在建一张」，workspace 里带上每块白板的定时建卡和下次时间。
//! 本机模式（Auth::Local）不需要登录，写请求只要带 x-board-client；令牌调用被拒绝的用例在 auth.rs。

mod common;

use board_server::auth::Auth;
use common::{send, Server, UI};
use serde_json::{json, Value};

fn crons(s: &Server) -> Vec<Value> {
    let r = send("GET", &format!("{}/api/workspace", s.url), &[], None);
    let p = r.body["projects"].as_array().unwrap().iter().find(|p| p["key"] == "default").unwrap().clone();
    let b = p["boards"].as_array().unwrap().iter().find(|b| b["key"] == "main").unwrap().clone();
    b["cardCrons"].as_array().cloned().unwrap_or_default()
}

#[test]
fn card_crons_crud_and_run_now() {
    let s = Server::start(Auth::Local);
    let base = format!("{}/api/card-crons/default/main", s.url);
    // 不合法的 cron：USAGE，写明哪一段不对
    let r = send("POST", &base, &[UI], Some(json!({ "name": "清理死代码", "title": "清理死代码", "cron": "0 25 * * *", "tz": "Asia/Shanghai" })));
    assert_eq!((r.status, r.code().as_str()), (400, "USAGE"), "{}", r.text);
    assert!(r.text.contains("第 2 段"), "{}", r.text);
    // 派给不存在的机器：USAGE
    let r = send("POST", &base, &[UI], Some(json!({ "name": "更新代码", "title": "更新代码", "cron": "0 3 * * *", "tz": "Asia/Shanghai", "host": "nope" })));
    assert_eq!((r.status, r.code().as_str()), (400, "USAGE"), "{}", r.text);
    assert!(r.text.contains("没有叫「nope」的机器"), "{}", r.text);
    // host 是空字符串：不派发
    let r = send("POST", &base, &[UI], Some(json!({ "name": "清理死代码", "title": "清理死代码", "body": "找出没人用的代码", "cron": "0 9 * * 1", "tz": "Asia/Shanghai", "host": "" })));
    assert_eq!(r.status, 200, "{}", r.text);
    let id = r.body["id"].as_i64().unwrap();
    assert_eq!((r.body["on"].as_bool(), r.body["place"].as_str(), r.body["host"].is_null()), (Some(true), Some("home"), true));
    let list = crons(&s);
    assert_eq!(list.len(), 1);
    assert!(list[0]["nextAt"].is_i64(), "workspace 里带下次时间：{:?}", list[0]);
    // 修改：关掉开关后没有下次
    let r = send("PATCH", &format!("{base}/{id}"), &[UI], Some(json!({ "on": false })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(crons(&s)[0]["nextAt"].is_null());
    // 现在建一张：待办最下面多一张，标题带日期
    let r = send("POST", &format!("{base}/{id}/run"), &[UI], None);
    assert_eq!(r.status, 200, "{}", r.text);
    let card = r.body["ranCard"].as_i64().expect("建了一张");
    let c = send("GET", &format!("{}/api/card?path=default/main&id={card}", s.url), &[], None);
    assert!(c.body["title"].as_str().unwrap().starts_with("清理死代码（"), "{}", c.text);
    assert_eq!(c.body["createdBy"], json!({ "kind": "system", "name": "定时建卡" }));
    assert_eq!(c.body["activity"][0]["text"], "定时建卡「清理死代码」建了这张卡");
    // 再点一次：上一张还在待办，跳过
    let r = send("POST", &format!("{base}/{id}/run"), &[UI], None);
    assert_eq!((r.body["ranCard"].is_null(), r.body["ranSkip"].as_i64()), (true, Some(card)));
    // 删除
    assert_eq!(send("DELETE", &format!("{base}/{id}"), &[UI], None).status, 200);
    assert!(crons(&s).is_empty());
    assert_eq!(send("DELETE", &format!("{base}/{id}"), &[UI], None).code(), "NOT_FOUND");
}
