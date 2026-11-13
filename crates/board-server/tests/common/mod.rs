//! 集成测试的公共部分：每个测试起一个真实的 board-server（后台线程、随机端口），用 HTTP 访问。
//! 鉴权的用例（auth）自己带模拟的 GitHub，不在这里。
#![allow(dead_code)]

use board_server::auth::Auth;
use board_server::{bind, serve_in_thread, AppState, Config};
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct Server {
    pub url: String,
    /// 服务端的数据库（测试里直接写，例如新建令牌）
    pub db: PathBuf,
    _dir: tempfile::TempDir,
}

impl Server {
    /// 建库、起服务端；`auth` 决定要不要登录（本机模式下请求就是「人」）
    pub fn start(auth: Auth) -> Server {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("b.db");
        let cfg = Config { db: db.clone(), demo: false, serve_web: false, auth };
        let state = AppState::open(&cfg).unwrap();
        let l = bind(([127, 0, 0, 1], 0).into()).unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        serve_in_thread(state, cfg, l);
        Server { url, db, _dir: dir }
    }
}

/// 响应：状态码、响应头、JSON 和原文一起留着，断言报错时能打出来
pub struct Res {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Value,
    pub text: String,
}

impl Res {
    /// 错误码（错误响应里 `error.code`）
    pub fn code(&self) -> String {
        self.body["error"]["code"].as_str().unwrap_or_default().to_string()
    }
    /// 某个响应头的全部值
    pub fn header(&self, k: &str) -> Vec<&str> {
        self.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str()).collect()
    }
    pub fn location(&self) -> String {
        self.header("location").first().copied().unwrap_or_default().to_string()
    }
    /// Set-Cookie 里某个 Cookie 的值（空字符串表示被清除）
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.header("set-cookie")
            .into_iter()
            .find_map(|c| c.split(';').next()?.trim().strip_prefix(&format!("{name}=")).map(str::to_string))
    }
}

/// 不做跳转、状态码不当错误：被拒绝（400、401 之类）时也要拿到响应体里的错误码
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).build().into()
}

/// 发一个 JSON 请求；GET / DELETE 不带请求体
pub fn send(method: &str, url: &str, headers: &[(&str, &str)], body: Option<Value>) -> Res {
    let a = agent();
    let mut res = match (method, body) {
        ("GET", _) | ("DELETE", _) => {
            let mut r = if method == "GET" { a.get(url) } else { a.delete(url) };
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.call()
        }
        (m, body) => {
            let mut r = match m {
                "POST" => a.post(url),
                "PATCH" => a.patch(url),
                "PUT" => a.put(url),
                _ => unreachable!(),
            };
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.send_json(body.unwrap_or(json!({})))
        }
    }
    .unwrap_or_else(|e| panic!("{method} {url}: {e}"));
    let status = res.status().as_u16();
    let headers = res.headers().iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string())).collect();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Res { status, headers, body, text }
}

/// 写请求要带 x-board-client（防跨站请求），这里统一带上
pub const UI: (&str, &str) = ("x-board-client", "test-window");
