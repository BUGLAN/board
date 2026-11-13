//! 模型供应商的「获取模型」（PRD §10.12「共用配置」）：服务端用填的地址和 API key 问供应商有哪些模型。
//!
//! 起一个模拟的供应商（OpenAI 兼容的 /v1/models 要 Bearer key，Anthropic 的 /v1/models 要 x-api-key），
//! 再起一个本机模式的 board-server，从界面的接口去问。

mod common;

use axum::http::HeaderMap;
use axum::{routing::get, Json, Router};
use board_server::auth::Auth;
use common::{send, Server, UI};
use serde_json::{json, Value};

fn serve_router(app: Router) -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    l.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let l = tokio::net::TcpListener::from_std(l).unwrap();
            axum::serve(l, app).await.unwrap();
        });
    });
    format!("http://{addr}")
}

/// 模拟的供应商：key 是 sk-good 才给列表
fn mock_vendor() -> String {
    let app = Router::new()
        .route(
            "/v1/models",
            get(|h: HeaderMap| async move {
                let bearer = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                let x = h.get("x-api-key").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                if bearer == "Bearer sk-good" {
                    return (axum::http::StatusCode::OK, Json(json!({ "object": "list", "data": [{ "id": "m-chat" }, { "id": "m-reasoner" }] })));
                }
                if x == "sk-good" && h.get("anthropic-version").is_some() {
                    return (axum::http::StatusCode::OK, Json(json!({ "data": [{ "id": "claude-x", "type": "model" }] })));
                }
                (axum::http::StatusCode::UNAUTHORIZED, Json(json!({ "error": { "message": "invalid key" } })))
            }),
        );
    serve_router(app)
}

fn ask(s: &Server, body: Value) -> common::Res {
    send("POST", &format!("{}/api/providers/models", s.url), &[UI], Some(body))
}

#[test]
fn fetch_models_from_openai_compatible_and_anthropic() {
    let vendor = mock_vendor();
    let s = Server::start(Auth::Local);
    let r = ask(&s, json!({ "endpoints": [{ "interface": "openai-chat", "url": format!("{vendor}/v1/") }], "apiKey": "sk-good" }));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["models"], json!(["m-chat", "m-reasoner"]));
    assert_eq!(r.body["from"], json!(format!("{vendor}/v1/models")));

    // 只有 Anthropic 接口：问 <地址>/v1/models，带 x-api-key
    let r = ask(&s, json!({ "endpoints": [{ "interface": "anthropic", "url": vendor }], "apiKey": "sk-good" }));
    assert_eq!(r.body["models"], json!(["claude-x"]), "{}", r.text);

    // key 不对：写明原因
    let r = ask(&s, json!({ "endpoints": [{ "interface": "openai-chat", "url": format!("{vendor}/v1") }], "apiKey": "sk-bad" }));
    assert_eq!(r.code(), "USAGE");
    assert!(r.body["error"]["message"].as_str().unwrap().contains("401"), "{}", r.text);

    // 没填 key、也没有存着的：先填 key
    let r = ask(&s, json!({ "endpoints": [{ "interface": "openai-chat", "url": format!("{vendor}/v1") }] }));
    assert!(r.body["error"]["message"].as_str().unwrap().contains("API key"), "{}", r.text);
}

#[test]
fn fetch_models_uses_stored_key_when_editing() {
    let vendor = mock_vendor();
    let s = Server::start(Auth::Local);
    let saved = send(
        "POST",
        &format!("{}/api/providers", s.url),
        &[UI],
        Some(json!({ "name": "relay", "endpoints": [{ "interface": "openai-chat", "url": format!("{vendor}/v1") }], "apiKey": "sk-good", "models": ["m-chat"] })),
    );
    assert_eq!(saved.status, 200, "{}", saved.text);
    // 修改时 key 留空：用 relay 存着的
    let r = ask(&s, json!({ "name": "relay", "endpoints": [{ "interface": "openai-chat", "url": format!("{vendor}/v1") }], "apiKey": "" }));
    assert_eq!(r.body["models"], json!(["m-chat", "m-reasoner"]), "{}", r.text);
}

/// board host 用自己的令牌取供应商原文：`GET /api/hosts/{名字}/providers` 要放行给 agent 令牌，
/// 别的令牌拿不到；`GET /api/providers` 的列表（API key 打码）访问令牌能读（PRD §10.12「共用配置」）
#[test]
fn host_token_can_read_provider_config() {
    let s = Server::start(Auth::Local);
    let store = board_core::Store::open(&s.db).unwrap();
    let tok = board_server::auth::create_token(store.conn(), "win 的令牌", board_core::now_ms()).unwrap().1;
    let other = board_server::auth::create_token(store.conn(), "别的令牌", board_core::now_ms()).unwrap().1;
    let bearer = format!("Bearer {tok}");
    let headers = [("authorization", bearer.as_str())];
    // board host 上报：它用的令牌记在这台机器上
    let r = send("POST", &format!("{}/api/hosts/win/beat", s.url), &headers, Some(json!({ "name": "win", "instance": "i1" })));
    assert_eq!(r.status, 200, "{}", r.text);
    // 这台机器的令牌能取原文；别的令牌不行
    let r = send("GET", &format!("{}/api/hosts/win/providers", s.url), &headers, None);
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(r.body["providers"].is_array(), "{}", r.text);
    let other_bearer = format!("Bearer {other}");
    let r = send("GET", &format!("{}/api/hosts/win/providers", s.url), &[("authorization", other_bearer.as_str())], None);
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{}", r.text);
    // 列表（打码）访问令牌能读
    let r = send("GET", &format!("{}/api/providers", s.url), &headers, None);
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(r.body["providers"].is_array(), "{}", r.text);
}
