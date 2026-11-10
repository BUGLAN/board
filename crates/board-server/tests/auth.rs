//! 鉴权（PRD §3、§12、§13）：未登录被拒、GitHub 登录（模拟的 GitHub）、访问令牌的权限、本机模式。
//!
//! 每个测试起一个真实的 board-server（后台线程、随机端口），用 HTTP 访问。

use axum::{routing::get, routing::post, Json, Router};
use board_server::auth::{self, Auth, GitHub};
use board_server::{bind, serve_in_thread, AppState, Config};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;

struct Res {
    status: u16,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Res {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or_default()
    }
    fn header(&self, k: &str) -> Vec<&str> {
        self.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str()).collect()
    }
    fn location(&self) -> String {
        self.header("location").first().copied().unwrap_or_default().to_string()
    }
    /// Set-Cookie 里某个 Cookie 的值（空字符串表示被清除）
    fn cookie(&self, name: &str) -> Option<String> {
        self.header("set-cookie")
            .into_iter()
            .find_map(|c| c.split(';').next()?.trim().strip_prefix(&format!("{name}=")).map(str::to_string))
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).build().into()
}

fn send(method: &str, url: &str, headers: &[(&str, &str)], body: Option<Value>) -> Res {
    let a = agent();
    let mut res = match (method, body) {
        ("GET", _) => {
            let mut r = a.get(url);
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.call()
        }
        ("DELETE", _) => {
            let mut r = a.delete(url);
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
    let headers = res
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
        .collect();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Res { status, headers, body }
}

/// 在后台线程里跑一个 axum 服务，返回地址
fn serve_router(app: Router) -> SocketAddr {
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
    addr
}

/// 模拟的 GitHub：任何授权码都换成访问令牌，用户名由授权码决定（code = 用户名）
fn mock_github() -> String {
    let app = Router::new()
        .route(
            "/login/oauth/access_token",
            post(|body: String| async move {
                let form: Vec<(String, String)> = serde_urlencoded::from_str(&body).unwrap_or_default();
                let code = form.iter().find(|(k, _)| k == "code").map(|(_, v)| v.clone()).unwrap_or_default();
                if code == "bad" {
                    return Json(json!({ "error": "bad_verification_code" }));
                }
                Json(json!({ "access_token": format!("gho_{code}"), "token_type": "bearer" }))
            }),
        )
        .route(
            "/user",
            get(|h: axum::http::HeaderMap| async move {
                let tok = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default();
                let login = tok.trim_start_matches("Bearer gho_").to_string();
                Json(json!({ "login": login, "name": format!("{login} 本人"), "avatar_url": "https://example.com/a.png" }))
            }),
        );
    format!("http://{}", serve_router(app))
}

struct Server {
    url: String,
    db: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn start(auth: Auth) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
    let cfg = Config { db: db.clone(), demo: false, serve_web: false, auth };
    let state = AppState::open(&cfg).unwrap();
    let l = bind(([127, 0, 0, 1], 0).into()).unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    serve_in_thread(state, cfg, l);
    Server { url, db, _dir: dir }
}

fn github(user: &str) -> Server {
    let gh = mock_github();
    start(Auth::GitHub(GitHub {
        client_id: "cid".into(),
        client_secret: "secret".into(),
        user: user.into(),
        public_url: "http://board.test".into(),
        oauth_url: gh.clone(),
        api_url: gh,
    }))
}

fn new_token(db: &Path, name: &str) -> String {
    let store = board_core::Store::open(db).unwrap();
    auth::create_token(store.conn(), name, board_core::now_ms()).unwrap().1
}

/// 走一遍 GitHub 登录，返回会话 Cookie
fn login(s: &Server, as_user: &str) -> Res {
    let r = send("GET", &format!("{}/auth/github", s.url), &[], None);
    assert_eq!(r.status, 303);
    let loc = r.location();
    assert!(loc.contains("/login/oauth/authorize?"), "{loc}");
    assert!(loc.contains("client_id=cid"));
    assert!(loc.contains("redirect_uri=http%3A%2F%2Fboard.test%2Fauth%2Fgithub%2Fcallback"), "{loc}");
    let state = r.cookie("board_oauth").expect("state cookie");
    assert!(loc.contains(&format!("state={state}")));
    send(
        "GET",
        &format!("{}/auth/github/callback?code={as_user}&state={state}", s.url),
        &[("cookie", &format!("board_oauth={state}"))],
        None,
    )
}

fn session(s: &Server) -> String {
    let r = login(s, "buglan");
    assert_eq!(r.status, 303);
    assert_eq!(r.location(), "/");
    let sid = r.cookie("board_session").expect("会话 Cookie");
    let raw = r.header("set-cookie").into_iter().find(|c| c.starts_with("board_session=")).unwrap().to_string();
    assert!(raw.contains("HttpOnly") && raw.contains("SameSite=Lax"), "{raw}");
    format!("board_session={sid}")
}

const UI: (&str, &str) = ("x-board-client", "test-window");

#[test]
fn unauthenticated_requests_are_rejected() {
    let s = github("buglan");
    let routes: &[(&str, &str)] = &[
        ("GET", "/api/info"),
        ("GET", "/api/workspace"),
        ("GET", "/api/board?path=default"),
        ("GET", "/api/card?path=default&id=1"),
        ("POST", "/api/cmd"),
        ("POST", "/api/projects"),
        ("PATCH", "/api/projects/default"),
        ("DELETE", "/api/projects/default"),
        ("POST", "/api/boards"),
        ("PATCH", "/api/boards/default/main"),
        ("DELETE", "/api/boards/default/main"),
        ("POST", "/api/labels"),
        ("PATCH", "/api/labels/1"),
        ("DELETE", "/api/labels/default/1"),
        ("PUT", "/api/layout"),
        ("POST", "/api/flow/default/main"),
        ("PUT", "/api/flow/default/main"),
        ("PUT", "/api/flow/default/main/layout"),
        ("PATCH", "/api/flow/default/main/todo"),
        ("DELETE", "/api/flow/default/main/todo"),
        ("GET", "/api/stream"),
        ("GET", "/api/me"),
        ("POST", "/api/auth/logout"),
        ("GET", "/api/tokens"),
        ("POST", "/api/tokens"),
        ("DELETE", "/api/tokens/1"),
        ("GET", "/api/nope"),
    ];
    for (m, p) in routes {
        let r = send(m, &format!("{}{p}", s.url), &[UI], None);
        assert_eq!(r.status, 401, "{m} {p}");
        assert_eq!(r.code(), "NOT_LOGGED_IN", "{m} {p}");
    }
    // 伪造的会话
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("cookie", "board_session=forged")], None);
    assert_eq!((r.status, r.code()), (401, "NOT_LOGGED_IN"));
    // 错误的令牌
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("authorization", "Bearer bd_nope")], None);
    assert_eq!((r.status, r.code()), (401, "INVALID_TOKEN"));
    // 健康检查不需要登录，带版本和提交号（部署后用来核对线上版本）
    let r = send("GET", &format!("{}/api/health", s.url), &[], None);
    assert_eq!(r.status, 200);
    assert_eq!(r.body["ok"], true);
    assert_eq!(r.body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(r.body["commit"], board_server::COMMIT);
    assert!(!board_server::COMMIT.is_empty());
}

#[test]
fn info_reports_version_and_commit() {
    // 设置页底部显示服务端版本：登录后 GET /api/info 返回编译时的版本和提交号
    let s = github("buglan");
    let cookie = session(&s);
    let r = send("GET", &format!("{}/api/info", s.url), &[("cookie", &cookie)], None);
    assert_eq!(r.status, 200);
    assert_eq!(r.body["name"], "board");
    assert_eq!(r.body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(r.body["commit"], board_server::COMMIT);
    assert!(!board_server::COMMIT.is_empty());
}

#[test]
fn github_login_session_and_logout() {
    let s = github("BugLan");
    let cookie = session(&s);
    let r = send("GET", &format!("{}/api/me", s.url), &[("cookie", &cookie)], None);
    assert_eq!(r.status, 200);
    assert_eq!(r.body["login"], "buglan");
    assert_eq!(r.body["mode"], "github");
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("cookie", &cookie)], None);
    assert_eq!(r.status, 200);
    // 界面上新建卡片：身份是人
    let r = send(
        "POST",
        &format!("{}/api/cmd", s.url),
        &[("cookie", &cookie), UI],
        Some(json!({ "board": "default", "op": "new", "title": "人建的" })),
    );
    assert_eq!(r.status, 200, "{:?}", r.body);
    assert_eq!(r.body["createdBy"]["kind"], "human");
    // 写请求必须带 x-board-client（防跨站请求）
    let r = send("POST", &format!("{}/api/cmd", s.url), &[("cookie", &cookie)], Some(json!({ "board": "default", "op": "new", "title": "x" })));
    assert_eq!((r.status, r.code()), (400, "USAGE"));
    // 退出后会话失效
    let r = send("POST", &format!("{}/api/auth/logout", s.url), &[("cookie", &cookie), UI], None);
    assert_eq!(r.status, 200);
    assert_eq!(r.cookie("board_session").as_deref(), Some(""));
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("cookie", &cookie)], None);
    assert_eq!((r.status, r.code()), (401, "NOT_LOGGED_IN"));
}

#[test]
fn github_login_rejections() {
    let s = github("buglan");
    // 不是允许的账号
    let r = login(&s, "someone-else");
    assert_eq!(r.status, 303);
    assert_eq!(r.location(), "/?login_error=denied&login=someone-else");
    assert!(r.cookie("board_session").is_none());
    // state 对不上
    let r = send(
        "GET",
        &format!("{}/auth/github/callback?code=buglan&state=aaa", s.url),
        &[("cookie", "board_oauth=bbb")],
        None,
    );
    assert_eq!(r.location(), "/?login_error=state");
    assert!(r.cookie("board_session").is_none());
    // 没有 state Cookie
    let r = send("GET", &format!("{}/auth/github/callback?code=buglan&state=aaa", s.url), &[], None);
    assert_eq!(r.location(), "/?login_error=state");
    // GitHub 拒绝授权码
    let st = send("GET", &format!("{}/auth/github", s.url), &[], None).cookie("board_oauth").unwrap();
    let r = send(
        "GET",
        &format!("{}/auth/github/callback?code=bad&state={st}", s.url),
        &[("cookie", &format!("board_oauth={st}"))],
        None,
    );
    assert_eq!(r.location(), "/?login_error=github");
    // 用户在 GitHub 上取消
    let r = send("GET", &format!("{}/auth/github/callback?error=access_denied", s.url), &[], None);
    assert_eq!(r.location(), "/?login_error=cancelled");
}

#[test]
fn tokens_are_for_cli_only() {
    let s = github("buglan");
    let cookie = session(&s);
    // 在界面上新建令牌：原文只返回这一次
    let r = send("POST", &format!("{}/api/tokens", s.url), &[("cookie", &cookie), UI], Some(json!({ "name": "macbook" })));
    assert_eq!(r.status, 200, "{:?}", r.body);
    let tok = r.body["token"].as_str().unwrap().to_string();
    assert!(tok.starts_with("bd_"));
    let id = r.body["info"]["id"].as_i64().unwrap();
    let list = send("GET", &format!("{}/api/tokens", s.url), &[("cookie", &cookie)], None);
    assert_eq!(list.body[0]["name"], "macbook");
    assert!(!list.body.to_string().contains(&tok), "列表里不能出现令牌原文");

    let bearer = format!("Bearer {tok}");
    let a = [("authorization", bearer.as_str())];
    // CLI 用到的接口
    assert_eq!(send("GET", &format!("{}/api/workspace", s.url), &a, None).status, 200);
    assert_eq!(send("GET", &format!("{}/api/board?path=default", s.url), &a, None).status, 200);
    // 严格模式：名字没有 check 过，卡片命令被拒绝；check 之后可以
    let r = send("POST", &format!("{}/api/cmd", s.url), &a, Some(json!({ "board": "default", "as": NAME, "op": "new", "title": "agent 建的" })));
    assert_eq!((r.status, r.code()), (409, "NAME_OFFLINE"));
    assert!(r.body["error"]["message"].as_str().unwrap().contains(&format!("board check --as {NAME}")), "{:?}", r.body);
    let r = send("POST", &format!("{}/api/agents/check", s.url), &a, Some(json!({ "name": NAME })));
    assert_eq!(r.status, 200, "{:?}", r.body);
    let r = send("POST", &format!("{}/api/cmd", s.url), &a, Some(json!({ "board": "default", "as": NAME, "op": "new", "title": "agent 建的" })));
    assert_eq!(r.status, 200, "{:?}", r.body);
    assert_eq!(r.body["createdBy"], json!({ "kind": "agent", "name": NAME }));
    let r = send("POST", &format!("{}/api/cmd", s.url), &a, Some(json!({ "board": "default", "as": NAME, "op": "claim" })));
    assert_eq!(r.body["holder"], NAME);
    // 没有名字
    let r = send("POST", &format!("{}/api/cmd", s.url), &a, Some(json!({ "board": "default", "op": "claim" })));
    assert_eq!((r.status, r.code()), (400, "NO_IDENTITY"));
    // agent 接口：心跳、在线列表
    let r = send("POST", &format!("{}/api/agents/beat", s.url), &a, Some(json!({ "name": NAME })));
    assert_eq!(r.status, 200, "{:?}", r.body);
    let r = send("GET", &format!("{}/api/agents", s.url), &a, None);
    assert_eq!(r.body[0]["name"], NAME);
    // 规则仍由规则引擎判定：agent 不能删除、不能强制释放
    let r = send("POST", &format!("{}/api/cmd", s.url), &a, Some(json!({ "board": "default", "as": NAME, "op": "delete", "id": 1 })));
    assert_eq!(r.code(), "USAGE");
    // 评论只能由作者（人）修改：直接调接口改别人的评论，得到 403 NOT_AUTHOR
    let cmd = |body: serde_json::Value, h: &[(&str, &str)]| send("POST", &format!("{}/api/cmd", s.url), h, Some(body));
    let ui = [("cookie", cookie.as_str()), UI];
    let r = cmd(json!({ "board": "default", "op": "comment", "id": 1, "text": "人写的" }), &ui);
    let mine = r.body["activity"].as_array().unwrap().last().unwrap()["id"].as_i64().unwrap();
    let r = cmd(json!({ "board": "default", "as": NAME, "op": "comment", "id": 1, "text": "agent 写的" }), &a);
    let theirs = r.body["activity"].as_array().unwrap().last().unwrap()["id"].as_i64().unwrap();
    let r = cmd(json!({ "board": "default", "as": NAME, "op": "edit_comment", "id": 1, "activity": mine, "text": "x" }), &a);
    assert_eq!((r.status, r.code()), (403, "NOT_AUTHOR"));
    let r = cmd(json!({ "board": "default", "op": "edit_comment", "id": 1, "activity": theirs, "text": "x" }), &ui);
    assert_eq!((r.status, r.code()), (403, "NOT_AUTHOR"));
    let r = cmd(json!({ "board": "default", "op": "edit_comment", "id": 1, "activity": mine, "text": "人改过的" }), &ui);
    assert_eq!(r.status, 200, "{:?}", r.body);
    let c = r.body["activity"].as_array().unwrap().iter().find(|x| x["id"] == mine).unwrap().clone();
    assert_eq!(c["text"], "人改过的");
    assert!(c["edited_at"].is_i64(), "{c:?}");
    // 只有人能做的操作、机器列表与设置、令牌管理：令牌不能用
    for (m, p, body) in [
        ("POST", "/api/projects", Some(json!({ "name": "x", "key": "x", "board": { "name": "m", "key": "main" } }))),
        ("DELETE", "/api/boards/default/main", None),
        ("POST", "/api/flow/default/main", Some(json!({ "name": "新状态" }))),
        ("GET", "/api/hosts", None),
        ("PATCH", "/api/hosts/mbp", Some(json!({ "max": 2 }))),
        ("DELETE", "/api/hosts/mbp", None),
        ("GET", "/api/hosts/mbp/recent", None),
        ("GET", "/api/tokens", None),
        ("POST", "/api/tokens", Some(json!({ "name": "y" }))),
        ("GET", "/api/me", None),
    ] {
        let r = send(m, &format!("{}{p}", s.url), &a, body);
        assert_eq!((r.status, r.code()), (401, "NOT_LOGGED_IN"), "{m} {p}");
    }
    // 实时推送：board host 用令牌订阅，第一条就带上机器和派发的版本（hosts）
    {
        use std::io::BufRead;
        let res = agent().get(format!("{}/api/stream", s.url)).header("authorization", &bearer).call().unwrap();
        assert_eq!(res.status().as_u16(), 200);
        let mut first = String::new();
        let mut r = std::io::BufReader::new(res.into_body().into_reader());
        while !first.starts_with("data:") {
            first.clear();
            r.read_line(&mut first).unwrap();
        }
        let v: Value = serde_json::from_str(first.trim_start_matches("data:").trim()).unwrap();
        assert!(v["hosts"].is_i64(), "{first}");
    }
    // 吊销后：INVALID_TOKEN
    let r = send("DELETE", &format!("{}/api/tokens/{id}", s.url), &[("cookie", &cookie), UI], None);
    assert_eq!(r.status, 200);
    let r = send("GET", &format!("{}/api/workspace", s.url), &a, None);
    assert_eq!((r.status, r.code()), (401, "INVALID_TOKEN"));
}

#[test]
fn server_side_token_command_works() {
    let s = github("buglan");
    let tok = new_token(&s.db, "ci");
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("authorization", &format!("Bearer {tok}"))], None);
    assert_eq!(r.status, 200);
}

#[test]
fn local_mode_needs_no_login_but_only_local_hosts() {
    let s = start(Auth::Local);
    let r = send("GET", &format!("{}/api/workspace", s.url), &[], None);
    assert_eq!(r.status, 200);
    assert_eq!(send("GET", &format!("{}/api/me", s.url), &[], None).body["mode"], "local");
    // 其他网页借 DNS 重绑定访问：Host 不是本机
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("host", "evil.example.com")], None);
    assert_eq!(r.status, 400);
    // 本机模式也认令牌：带令牌就是 agent
    let tok = new_token(&s.db, "local");
    let bearer = format!("Bearer {tok}");
    let r = send("POST", &format!("{}/api/agents/check", s.url), &[("authorization", &bearer)], Some(json!({ "name": "codex/gpt-5" })));
    assert_eq!(r.status, 200, "{:?}", r.body);
    let r = send(
        "POST",
        &format!("{}/api/cmd", s.url),
        &[("authorization", &bearer)],
        Some(json!({ "board": "default", "as": "codex/gpt-5", "op": "new", "title": "x" })),
    );
    assert_eq!(r.body["createdBy"]["kind"], "agent");
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("authorization", "Bearer bd_nope")], None);
    assert_eq!(r.code(), "INVALID_TOKEN");
}

/// 打开实时推送，返回一个通道：推送连接断开时收到消息
fn open_stream(url: &str, cookie: &str) -> std::sync::mpsc::Receiver<()> {
    let res = agent().get(format!("{url}/api/stream")).header("cookie", cookie).call().unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut r = res.into_body().into_reader();
        let _ = std::io::copy(&mut r, &mut std::io::sink());
        let _ = tx.send(());
    });
    rx
}

/// 用访问令牌打开实时推送，返回一个通道：推送连接断开时收到消息
fn open_token_stream(url: &str, token: &str) -> std::sync::mpsc::Receiver<()> {
    let bearer = format!("Bearer {token}");
    let res = agent().get(format!("{url}/api/stream")).header("authorization", &bearer).call().unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut r = res.into_body().into_reader();
        let _ = std::io::copy(&mut r, &mut std::io::sink());
        let _ = tx.send(());
    });
    rx
}

#[test]
fn revoke_closes_that_tokens_stream() {
    let s = github("buglan");
    let cookie = session(&s);
    let ui = [("cookie", cookie.as_str()), UI];
    // 在界面上建两个令牌，拿到它们的原文和编号
    let mut tokens = Vec::new();
    for name in ["first", "second"] {
        let r = send("POST", &format!("{}/api/tokens", s.url), &ui, Some(json!({ "name": name })));
        assert_eq!(r.status, 200, "{:?}", r.body);
        tokens.push((r.body["token"].as_str().unwrap().to_string(), r.body["info"]["id"].as_i64().unwrap()));
    }
    let (ta, ia) = &tokens[0];
    let (tb, _) = &tokens[1];
    let sa = open_token_stream(&s.url, ta);
    let sb = open_token_stream(&s.url, tb);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let r = send("DELETE", &format!("{}/api/tokens/{ia}", s.url), &ui, None);
    assert_eq!(r.status, 200);
    sa.recv_timeout(std::time::Duration::from_secs(1)).expect("被吊销的令牌：推送应当断开");
    assert!(sb.recv_timeout(std::time::Duration::from_millis(800)).is_err(), "另一个令牌的推送不受影响");
    // 重新连接被拒绝
    let bearer = format!("Bearer {ta}");
    let r = send("GET", &format!("{}/api/stream", s.url), &[("authorization", &bearer)], None);
    assert_eq!((r.status, r.code()), (401, "INVALID_TOKEN"));
}

#[test]
fn logout_closes_that_sessions_stream() {
    let s = github("buglan");
    let a = session(&s);
    let b = session(&s);
    let sa = open_stream(&s.url, &a);
    let sb = open_stream(&s.url, &b);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let r = send("POST", &format!("{}/api/auth/logout", s.url), &[("cookie", &a), UI], None);
    assert_eq!(r.status, 200);
    sa.recv_timeout(std::time::Duration::from_secs(3)).expect("退出的会话：推送应当断开");
    assert!(sb.recv_timeout(std::time::Duration::from_millis(800)).is_err(), "另一个会话的推送不受影响");
    // 重新连接被拒绝
    let r = send("GET", &format!("{}/api/stream", s.url), &[("cookie", &a)], None);
    assert_eq!((r.status, r.code()), (401, "NOT_LOGGED_IN"));
}

/// 桌面端登录：在浏览器里走 GitHub 登录，返回浏览器最后被送到的地址（本机回环地址）
fn desktop_login(s: &Server, as_user: &str, port: u16, nonce: &str) -> Res {
    let r = send("GET", &format!("{}/auth/github?desktop={port}&nonce={nonce}", s.url), &[], None);
    assert_eq!(r.status, 303);
    assert!(r.location().contains("/login/oauth/authorize?"));
    let cookie = r.cookie("board_oauth").expect("state cookie");
    let state = cookie.split('.').next().unwrap().to_string();
    assert!(r.location().contains(&format!("state={state}&")), "授权页拿到的 state 不含端口和随机串");
    send(
        "GET",
        &format!("{}/auth/github/callback?code={as_user}&state={state}", s.url),
        &[("cookie", &format!("board_oauth={cookie}"))],
        None,
    )
}

#[test]
fn desktop_login_hands_session_to_the_app() {
    let s = github("buglan");
    let nonce = "00112233445566778899aabbccddeeff";
    let r = desktop_login(&s, "buglan", 51234, nonce);
    assert_eq!(r.status, 303);
    // 浏览器只被送到本机回环地址，并且浏览器这边不登录
    let loc = r.location();
    let prefix = format!("http://127.0.0.1:51234/callback?state={nonce}&code=");
    assert!(loc.starts_with(&prefix), "{loc}");
    assert!(r.cookie("board_session").is_none());
    assert_eq!(r.cookie("board_oauth").as_deref(), Some(""));
    let code = loc[prefix.len()..].to_string();

    // 桌面端窗口用登录码换会话
    let r = send("GET", &format!("{}/auth/desktop?code={code}", s.url), &[], None);
    assert_eq!(r.status, 303);
    assert_eq!(r.location(), "/");
    let sid = r.cookie("board_session").expect("会话 Cookie");
    let cookie = format!("board_session={sid}");
    let r = send("GET", &format!("{}/api/me", s.url), &[("cookie", &cookie)], None);
    assert_eq!(r.status, 200);
    assert_eq!(r.body["login"], "buglan");

    // 登录码只能用一次
    let r = send("GET", &format!("{}/auth/desktop?code={code}", s.url), &[], None);
    assert_eq!(r.location(), "/?login_error=state");
    assert!(r.cookie("board_session").is_none());
    // 伪造的、空的登录码
    assert_eq!(send("GET", &format!("{}/auth/desktop?code=nope", s.url), &[], None).location(), "/?login_error=state");
    assert_eq!(send("GET", &format!("{}/auth/desktop", s.url), &[], None).location(), "/?login_error=state");
}

#[test]
fn desktop_login_errors_go_back_to_the_app() {
    let s = github("buglan");
    let nonce = "ffeeddccbbaa99887766554433221100";
    // 不是允许的账号：原因交回桌面端，由桌面端窗口显示
    let r = desktop_login(&s, "someone-else", 40000, nonce);
    assert_eq!(r.location(), format!("http://127.0.0.1:40000/callback?state={nonce}&error=denied&login=someone-else"));
    assert!(r.cookie("board_session").is_none());
    // 在 GitHub 上取消
    let cookie = format!("board_oauth=abc.40000.{nonce}");
    let r = send("GET", &format!("{}/auth/github/callback?error=access_denied", s.url), &[("cookie", &cookie)], None);
    assert_eq!(r.location(), format!("http://127.0.0.1:40000/callback?state={nonce}&error=cancelled"));
    // state 对不上
    let r = send("GET", &format!("{}/auth/github/callback?code=buglan&state=zzz", s.url), &[("cookie", &cookie)], None);
    assert_eq!(r.location(), format!("http://127.0.0.1:40000/callback?state={nonce}&error=state"));
    // 参数不合法：不发起登录
    for q in ["desktop=0&nonce=00112233445566778899aabbccddeeff", "desktop=80&nonce=short", "desktop=80", "desktop=x&nonce=00112233445566778899aabbccddeeff"] {
        let r = send("GET", &format!("{}/auth/github?{q}", s.url), &[], None);
        assert_eq!(r.status, 400, "{q}");
        assert!(r.cookie("board_oauth").is_none(), "{q}");
    }
}

/// 上传附件：请求体是文件内容
fn upload(url: &str, headers: &[(&str, &str)], bytes: &[u8]) -> Res {
    let mut r = agent().post(url);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let mut res = r.send(bytes).unwrap_or_else(|e| panic!("POST {url}: {e}"));
    let status = res.status().as_u16();
    let headers = res.headers().iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string())).collect();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    Res { status, headers, body: serde_json::from_str(&text).unwrap_or(Value::Null) }
}

/// 读取附件，返回状态、响应头和内容
fn download(url: &str, headers: &[(&str, &str)]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut r = agent().get(url);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let mut res = r.call().unwrap();
    let status = res.status().as_u16();
    let h = res.headers().iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string())).collect();
    (status, h, res.body_mut().read_to_vec().unwrap_or_default())
}

const NAME: &str = "claude/opus-5.5";

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

#[test]
fn attachments_need_login_and_work_for_agents() {
    let s = github("buglan");
    let cookie = session(&s);
    let tok = new_token(&s.db, "ci");
    let bearer = format!("Bearer {tok}");
    let card = send("POST", &format!("{}/api/cmd", s.url), &[("cookie", &cookie), UI], Some(json!({ "board": "default", "op": "new", "title": "截图" })));
    let id = card.body["id"].as_i64().unwrap();

    // 未登录不能上传、不能读
    let r = upload(&format!("{}/api/files?board=default&name=a.png", s.url), &[], PNG);
    assert_eq!((r.status, r.code()), (401, "NOT_LOGGED_IN"));

    // 人在新建弹窗里先上传（不带卡片），新建时描述引用到就归到卡片
    let r = upload(&format!("{}/api/files?board=default&name=%E6%88%AA%E5%9B%BE.png", s.url), &[("cookie", &cookie), UI], PNG);
    assert_eq!(r.status, 200, "{:?}", r.body);
    assert_eq!(r.body["name"], "截图.png");
    assert_eq!(r.body["image"], true);
    assert!(r.body["cardId"].is_null());
    let url = r.body["url"].as_str().unwrap().to_string();
    let md = r.body["markdown"].as_str().unwrap().to_string();
    let c = send(
        "POST",
        &format!("{}/api/cmd", s.url),
        &[("cookie", &cookie), UI],
        Some(json!({ "board": "default", "op": "new", "title": "带图", "body": format!("如图 {md}") })),
    );
    assert_eq!(c.body["attachments"][0]["url"], url.as_str());

    // 读取：要登录；图片直接显示，nosniff + sandbox
    let (st, _, _) = download(&format!("{}{url}", s.url), &[]);
    assert_eq!(st, 401);
    let (st, h, body) = download(&format!("{}{url}", s.url), &[("cookie", &cookie)]);
    assert_eq!((st, body.as_slice()), (200, PNG));
    let get = |k: &str| h.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone()).unwrap_or_default();
    assert_eq!(get("content-type"), "image/png");
    assert!(get("content-disposition").starts_with("inline; filename*=UTF-8''%E6%88%AA%E5%9B%BE.png"));
    assert_eq!(get("x-content-type-options"), "nosniff");
    assert!(get("content-security-policy").contains("sandbox"));

    // agent 用令牌上传到卡片（要带名字）、读取；卡片详情里列出附件
    let r = upload(&format!("{}/api/files?board=default&card={id}&name=run.log", s.url), &[("authorization", &bearer)], b"ok\n");
    assert_eq!((r.status, r.code()), (400, "NO_IDENTITY"));
    let up = format!("{}/api/files?board=default&card={id}&name=run.log&as=claude%2Fopus-5.5", s.url);
    let r = upload(&up, &[("authorization", &bearer)], b"ok\n");
    assert_eq!((r.status, r.code()), (409, "NAME_OFFLINE"), "名字要先 check");
    send("POST", &format!("{}/api/agents/check", s.url), &[("authorization", &bearer)], Some(json!({ "name": NAME })));
    let r = upload(&up, &[("authorization", &bearer)], b"ok\n");
    assert_eq!(r.status, 200, "{:?}", r.body);
    assert_eq!(r.body["createdBy"], json!({ "kind": "agent", "name": NAME }));
    let log = r.body["url"].as_str().unwrap().to_string();
    let (st, h, body) = download(&format!("{}{log}", s.url), &[("authorization", &bearer)]);
    assert_eq!((st, body.as_slice()), (200, b"ok\n".as_slice()));
    assert!(h.iter().any(|(k, v)| k == "content-disposition" && v.starts_with("attachment;")), "其他文件是下载");
    let d = send("GET", &format!("{}/api/card?path=default&id={id}", s.url), &[("authorization", &bearer)], None);
    assert_eq!(d.body["attachments"][0]["name"], "run.log");

    // 类型、大小、卡片不存在
    let r = upload(&format!("{}/api/files?board=default&name=x.svg", s.url), &[("cookie", &cookie), UI], b"<svg/>");
    assert_eq!(r.code(), "USAGE");
    let big = vec![b'a'; 10 * 1024 * 1024 + 1];
    let r = upload(&format!("{}/api/files?board=default&name=big.txt", s.url), &[("cookie", &cookie), UI], &big);
    assert_eq!(r.code(), "USAGE", "{:?}", r.body);
    assert!(r.body["error"]["message"].as_str().unwrap().contains("10 MB"));
    let r = upload(&format!("{}/api/files?board=default&card=999&name=a.png", s.url), &[("cookie", &cookie), UI], PNG);
    assert_eq!(r.code(), "NOT_FOUND");
    let (st, _, _) = download(&format!("{}/api/files/999", s.url), &[("cookie", &cookie)]);
    assert_eq!(st, 404);
}

/// agent 名字（PRD §3）：check 查重与占用是原子的；工作区快照带在线 agent 和它持有的卡片；
/// 只更新心跳时间的心跳不改 rev（不推送），上线、下线改 rev
#[test]
fn agent_names_check_beat_and_online_list() {
    let s = github("buglan");
    let tok = new_token(&s.db, "ci");
    let bearer = format!("Bearer {tok}");
    let a = [("authorization", bearer.as_str())];
    let url = s.url.clone();
    let post = move |p: &str, body: Value| send("POST", &format!("{url}{p}"), &[("authorization", &format!("Bearer {tok}"))], Some(body));

    // 格式不对：USAGE，并给出格式说明
    for bad in ["claude", "Claude/opus", "a/b/c"] {
        let r = post("/api/agents/check", json!({ "name": bad }));
        assert_eq!((r.status, r.code()), (400, "USAGE"), "{bad}");
        assert!(r.body["error"]["message"].as_str().unwrap().contains("<运行环境>/<模型>"));
    }
    let r = post("/api/agents/check", json!({ "name": "" }));
    assert_eq!(r.code(), "NO_IDENTITY");

    // 同时 check 同一个名字：只有一个成功
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let post = post.clone();
            std::thread::spawn(move || post("/api/agents/check", json!({ "name": NAME })))
        })
        .collect();
    let res: Vec<Res> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(res.iter().filter(|r| r.status == 200).count(), 1);
    for r in res.iter().filter(|r| r.status != 200) {
        assert_eq!((r.status, r.code()), (409, "NAME_TAKEN"));
        assert_eq!(r.body["error"]["detail"]["name"], NAME);
    }

    // 工作区快照：在线 agent，以及它持有的卡片（白板和 ID）
    let ws = send("GET", &format!("{}/api/workspace", s.url), &a, None);
    assert_eq!(ws.body["agents"], json!([{ "name": NAME, "checkedAt": ws.body["agents"][0]["checkedAt"], "beatAt": ws.body["agents"][0]["beatAt"], "cards": [] }]));
    post("/api/cmd", json!({ "board": "default", "as": NAME, "op": "new", "title": "一" }));
    post("/api/cmd", json!({ "board": "default", "as": NAME, "op": "claim" }));
    let r = post("/api/agents/beat", json!({ "name": NAME }));
    assert_eq!(r.body["cards"][0]["board"], "default/main");
    assert_eq!(r.body["cards"][0]["id"], 1);
    assert_eq!(r.body["cards"][0]["runs"], 1, "第一次心跳开始第 1 次");
    let ws = send("GET", &format!("{}/api/workspace", s.url), &a, None);
    assert_eq!(ws.body["agents"][0]["cards"][0]["board"], "default/main");
    assert_eq!(ws.body["agents"][0]["cards"][0]["holder"], NAME);

    // 只更新心跳时间：rev 不变
    let rev = |s: &Server| send("GET", &format!("{}/api/workspace", s.url), &a, None).body["rev"].as_i64().unwrap();
    let r0 = rev(&s);
    for _ in 0..3 {
        assert_eq!(post("/api/agents/beat", json!({ "name": NAME })).status, 200);
    }
    assert_eq!(rev(&s), r0, "只更新心跳时间的心跳不推送");

    // final：立刻离线，rev 变化；之后的心跳和卡片命令都是 NAME_OFFLINE
    let r = post("/api/agents/beat", json!({ "name": NAME, "final": true }));
    assert_eq!(r.status, 200, "{:?}", r.body);
    assert!(rev(&s) > r0, "下线要推送");
    assert_eq!(send("GET", &format!("{}/api/agents", s.url), &a, None).body, json!([]));
    let r = post("/api/agents/beat", json!({ "name": NAME }));
    assert_eq!((r.status, r.code()), (409, "NAME_OFFLINE"));
    let r = post("/api/cmd", json!({ "board": "default", "as": NAME, "op": "comment", "id": 1, "text": "x" }));
    assert_eq!(r.code(), "NAME_OFFLINE");
    // 读接口不检查名字
    assert_eq!(send("GET", &format!("{}/api/card?path=default&id=1", s.url), &a, None).status, 200);
    // 离线后可以重新 check
    assert_eq!(post("/api/agents/check", json!({ "name": NAME })).status, 200);
}
