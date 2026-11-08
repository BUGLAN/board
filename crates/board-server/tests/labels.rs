//! 标签顺序（#58）：`PUT /api/labels/order` 保存拖拽后的标签顺序。
//!
//! 起一个真实的 board-server（后台线程、随机端口），用 HTTP 访问；
//! 本机模式（Auth::Local）不需要登录，写请求只要带 x-board-client。

use board_server::auth::Auth;
use board_server::{bind, serve_in_thread, AppState, Config};
use serde_json::{json, Value};

struct Res {
    status: u16,
    body: Value,
    text: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).build().into()
}

/// 本机模式下写请求要带 x-board-client（防跨站），这里统一带上
fn call(method: &str, url: &str, body: Option<Value>) -> Res {
    let a = agent();
    let res = match method {
        "GET" => a.get(url).call(),
        "POST" => a.post(url).header("x-board-client", "test-window").send_json(body.unwrap_or(json!({}))),
        "PUT" => a.put(url).header("x-board-client", "test-window").send_json(body.unwrap_or(json!({}))),
        "PATCH" => a.patch(url).header("x-board-client", "test-window").send_json(body.unwrap_or(json!({}))),
        _ => unreachable!(),
    }
    .unwrap_or_else(|e| panic!("{method} {url}: {e}"));
    let mut res = res;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Res { status, body, text }
}

fn code(r: &Res) -> String {
    r.body["error"]["code"].as_str().unwrap_or_default().to_string()
}

struct Server {
    url: String,
    _dir: tempfile::TempDir,
}

fn local() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
    let cfg = Config { db, demo: false, serve_web: false, auth: Auth::Local };
    let state = AppState::open(&cfg).unwrap();
    let l = bind(([127, 0, 0, 1], 0).into()).unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    serve_in_thread(state, cfg, l);
    Server { url, _dir: dir }
}

fn label(s: &Server, project: &str, name: &str) -> i64 {
    let r = call("POST", &format!("{}/api/labels", s.url), Some(json!({ "project": project, "name": name })));
    assert_eq!(r.status, 200, "{}", r.text);
    r.body["id"].as_i64().expect("label.id")
}

/// `board labels --json` 读的是 workspace 里的 labels（按 sort 排）
fn names(s: &Server) -> Vec<String> {
    let r = call("GET", &format!("{}/api/workspace", s.url), None);
    assert_eq!(r.status, 200, "{}", r.text);
    let p = r.body["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .find(|p| p["key"] == "default")
        .expect("默认项目");
    p["labels"].as_array().expect("labels").iter().map(|l| l["name"].as_str().unwrap_or_default().to_string()).collect()
}

fn order(s: &Server, ids: &[i64]) -> Res {
    call("PUT", &format!("{}/api/labels/order", s.url), Some(json!({ "project": "default", "ids": ids })))
}

#[test]
fn label_order_is_saved_and_new_labels_go_last() {
    let s = local();
    let a = label(&s, "default", "甲");
    let b = label(&s, "default", "乙");
    let c = label(&s, "default", "丙");
    assert_eq!(names(&s), ["甲", "乙", "丙"]);

    // 拖拽后保存：只给出部分顺序，没提到的按原顺序补在后面
    assert_eq!(order(&s, &[c, a]).status, 200);
    assert_eq!(names(&s), ["丙", "甲", "乙"]);

    // 新建的标签排在最后，不会插到中间
    let d = label(&s, "default", "丁");
    assert_eq!(names(&s), ["丙", "甲", "乙", "丁"]);

    // 再读一次（相当于刷新页面/换客户端）：顺序一样
    assert_eq!(names(&s), ["丙", "甲", "乙", "丁"]);

    // 全部重排：以传入的 ids 为准
    assert_eq!(order(&s, &[b, c, a, d]).status, 200);
    assert_eq!(names(&s), ["乙", "丙", "甲", "丁"]);
    // 原有的 PATCH /api/labels/{id} 没被新路由挡住
    let r = call("PATCH", &format!("{}/api/labels/{a}", s.url), Some(json!({ "project": "default", "name": "甲甲" })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(names(&s), ["乙", "丙", "甲甲", "丁"]);
}

#[test]
fn label_order_rejects_illegal_ids_and_keeps_order() {
    let s = local();
    let a = label(&s, "default", "甲");
    let b = label(&s, "default", "乙");
    // 另一个项目 + 它自己的标签
    let r = call(
        "POST",
        &format!("{}/api/projects", s.url),
        Some(json!({ "name": "乙项目", "key": "b", "board": { "name": "主白板", "key": "main" } })),
    );
    assert_eq!(r.status, 200, "{}", r.text);
    let other = label(&s, "b", "别处");

    // 混入别的项目的标签
    let r = order(&s, &[b, other]);
    assert_eq!((r.status, code(&r)), (400, "USAGE".to_string()), "{}", r.text);
    // id 不存在
    let r = order(&s, &[a, 9999]);
    assert_eq!((r.status, code(&r)), (400, "USAGE".to_string()), "{}", r.text);
    // id 重复
    let r = order(&s, &[a, a]);
    assert_eq!((r.status, code(&r)), (400, "USAGE".to_string()), "{}", r.text);
    // 报错时顺序不能变
    assert_eq!(names(&s), ["甲", "乙"]);
    // 项目不存在：按既有约定
    let r = call("PUT", &format!("{}/api/labels/order", s.url), Some(json!({ "project": "nope", "ids": [] })));
    assert_eq!((r.status, code(&r)), (404, "BOARD_NOT_FOUND".to_string()), "{}", r.text);
}
