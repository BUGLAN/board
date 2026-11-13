//! 动态分页接口（#55）：`GET /api/events?path=<项目>/<白板>&before=<id>&limit=<n>`。
//!
//! 每个测试起一个真实的 board-server（后台线程、随机端口），用 HTTP 访问；
//! 本机模式（Auth::Local）不需要登录，请求按「人」处理。

mod common;

use board_server::auth::{Auth, GitHub};
use common::{send, Server, UI};
use serde_json::{json, Value};

fn local() -> Server {
    Server::start(Auth::Local)
}

fn github() -> Server {
    Server::start(Auth::GitHub(GitHub {
        client_id: "cid".into(),
        client_secret: "secret".into(),
        user: "buglan".into(),
        public_url: "http://board.test".into(),
        // 鉴权测试不发往 GitHub，随便填
        oauth_url: "http://127.0.0.1:1".into(),
        api_url: "http://127.0.0.1:1".into(),
    }))
}

fn new_card(s: &Server, title: &str) {
    let r = send(
        "POST",
        &format!("{}/api/cmd", s.url),
        &[UI],
        Some(json!({ "board": "default/main", "op": "new", "title": title })),
    );
    assert_eq!(r.status, 200, "{}", r.text);
}

fn page_at(s: &Server, before: Option<i64>, limit: i64) -> Vec<Value> {
    let mut url = format!("{}/api/events?path=default/main&limit={limit}", s.url);
    if let Some(before) = before {
        url.push_str(&format!("&before={before}"));
    }
    let r = send("GET", &url, &[], None);
    assert_eq!(r.status, 200, "{url} {}", r.text);
    r.body.as_array().cloned().expect("返回数组")
}

fn ids(page: &[Value]) -> Vec<i64> {
    page.iter().map(|e| e["id"].as_i64().expect("event.id")).collect()
}

#[test]
fn events_pages_backwards_without_gaps_or_duplicates() {
    let s = local();
    for i in 0..12 {
        new_card(&s, &format!("卡{i}"));
    }
    // 不带 before：最新的一页，按 id 倒序
    let all = page_at(&s, None, 100);
    assert!(all.len() >= 12, "12 张卡片至少有 12 条动态：{}", all.len());
    let all_ids = ids(&all);
    assert!(all_ids.windows(2).all(|w| w[0] > w[1]), "新的在前：{all_ids:?}");
    assert_eq!(all[0]["board"], "default/main");
    assert!(all[0]["title"].is_string());

    // 每页 5 条往前翻：拼起来正好是全部（不重不漏），最后一页为空
    let mut got: Vec<i64> = vec![];
    let mut before = None;
    loop {
        let page = page_at(&s, before, 5);
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 5);
        let page_ids = ids(&page);
        assert!(page_ids.windows(2).all(|w| w[0] > w[1]));
        before = page_ids.last().copied();
        got.extend(page_ids);
        assert!(got.len() <= all_ids.len(), "翻页应当终止");
    }
    assert_eq!(got, all_ids);
}

#[test]
fn events_limit_is_capped() {
    let s = local();
    new_card(&s, "一");
    // 不传 limit：用默认值
    let r = send("GET", &format!("{}/api/events?path=default/main", s.url), &[], None);
    assert_eq!(r.status, 200, "{}", r.text);
    let n = r.body.as_array().expect("返回数组").len();
    assert!((1..=20).contains(&n), "默认一页 20 条以内：{n}");
    // 上限 100 仍然可用
    assert_eq!(send("GET", &format!("{}/api/events?path=default/main&limit=100", s.url), &[], None).status, 200);
    // 超出范围：按既有用法错误
    for q in ["limit=0", "limit=101", "limit=-1"] {
        let r = send("GET", &format!("{}/api/events?path=default/main&{q}", s.url), &[], None);
        assert_eq!((r.status, r.code().as_str()), (400, "USAGE"), "{q} {}", r.text);
    }
    // 不是整数：参数解析直接拒绝
    assert_eq!(send("GET", &format!("{}/api/events?path=default/main&limit=abc", s.url), &[], None).status, 400);
    // 白板不存在：按既有约定
    let r = send("GET", &format!("{}/api/events?path=default/nope", s.url), &[], None);
    assert_eq!((r.status, r.code().as_str()), (404, "BOARD_NOT_FOUND"), "{}", r.text);
}

#[test]
fn events_before_out_of_range() {
    let s = local();
    new_card(&s, "一");
    new_card(&s, "二");
    let all = page_at(&s, None, 100);
    let all_ids = ids(&all);
    let newest = all_ids[0];
    let oldest = *all_ids.last().unwrap();
    // 比所有 id 都大：当作从最新一条开始
    let page = page_at(&s, Some(newest + 1000), 3);
    assert_eq!(ids(&page), all_ids[..all_ids.len().min(3)].to_vec());
    // 比所有 id 都小（或 0）：翻到底，返回空数组而不是报错
    for before in [oldest - 1, oldest, 0] {
        let r = send("GET", &format!("{}/api/events?path=default/main&before={before}", s.url), &[], None);
        assert_eq!(r.status, 200, "{}", r.text);
        assert_eq!(r.body, json!([]), "before={before}");
    }
    // before 不是整数：参数解析直接拒绝
    assert_eq!(send("GET", &format!("{}/api/events?path=default/main&before=abc", s.url), &[], None).status, 400);
}

#[test]
fn events_require_login() {
    let s = github();
    let r = send("GET", &format!("{}/api/events?path=default/main", s.url), &[], None);
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{}", r.text);
    let r = send("GET", &format!("{}/api/events?path=default/main", s.url), &[("authorization", "Bearer bd_nope")], None);
    assert_eq!((r.status, r.code().as_str()), (401, "INVALID_TOKEN"), "{}", r.text);
}
