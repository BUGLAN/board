//! 部署后的健康检查：API 的模式/版本/提交，以及首页确实返回 Board 的 HTML。

use board_server::healthcheck::{self, Expect};
use serde_json::{json, Value};

fn start(health: Value, html: &'static str) -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.set_nonblocking(true).unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let app = axum::Router::new()
        .route(
            "/api/health",
            axum::routing::get(move || {
                let health = health.clone();
                async move { axum::Json(health) }
            }),
        )
        .fallback(axum::routing::get(move || async move { axum::response::Html(html) }));
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let l = tokio::net::TcpListener::from_std(l).unwrap();
            axum::serve(l, app).await.unwrap();
        });
    });
    url
}

fn health(mode: &str, version: &str, commit: &str, ui: bool) -> Value {
    json!({ "ok": true, "mode": mode, "version": version, "commit": commit, "ui": ui })
}

#[test]
fn checks_mode_version_commit_and_html() {
    let url = start(health("github", "1.2.3", "abcdef0", true), "<!doctype html><html><div id=\"root\"></div></html>");
    let expect = Expect {
        mode: Some("github".into()),
        version: Some("1.2.3".into()),
        commit: Some("abcdef0".into()),
    };
    let got = healthcheck::wait(&url, &expect, 2).unwrap();
    assert_eq!((got.mode.as_str(), got.version.as_str(), got.commit.as_str()), ("github", "1.2.3", "abcdef0"));
}

#[test]
fn reports_the_mode_and_ui_separately() {
    let url = start(health("local", "1.2.3", "abcdef0", false), "界面还没有构建");
    let expect = Expect { mode: Some("github".into()), ..Expect::default() };
    let errors = healthcheck::wait(&url, &expect, 0).unwrap_err();
    let text = errors.join("\n");
    assert!(text.contains("模式是 \"local\""), "{text}");
    assert!(text.contains("ui=false"), "{text}");
    assert!(text.contains("不是 Board 的 index.html"), "{text}");
}
