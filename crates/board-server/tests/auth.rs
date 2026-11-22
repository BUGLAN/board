//! 鉴权（PRD §3、§12、§13）：未登录被拒、GitHub 登录（模拟的 GitHub）、访问令牌的权限、本机模式。
//!
//! 每个测试起一个真实的 board-server（后台线程、随机端口），用 HTTP 访问。

mod common;

use axum::{routing::get, routing::post, Json, Router};
use board_server::auth::{self, Auth, GitHub};
use common::{agent, send, Res, Server, UI};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;

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

fn github(user: &str) -> Server {
    let gh = mock_github();
    Server::start(Auth::GitHub(GitHub {
        client_id: "cid".into(),
        client_secret: "secret".into(),
        user: user.into(),
        public_url: "http://board.test".into(),
        oauth_url: gh.clone(),
        api_url: gh,
    }))
}

/// 同 [`github`]，实时推送每 `check` 查一次登录态（默认 20 秒，测试里缩短）
fn github_checking(user: &str, check: std::time::Duration) -> Server {
    let gh = mock_github();
    Server::start_with(
        Auth::GitHub(GitHub {
            client_id: "cid".into(),
            client_secret: "secret".into(),
            user: user.into(),
            public_url: "http://board.test".into(),
            oauth_url: gh.clone(),
            api_url: gh,
        }),
        |st| st.set_stream_check(check),
    )
}

fn local_mode() -> Server {
    Server::start(Auth::Local)
}

fn new_token(db: &Path, name: &str) -> String {
    let store = board_core::Store::open(db).unwrap();
    auth::create_token(store.conn(), name, board_core::now_ms()).unwrap().1
}

/// 「添加机器」新建的机器令牌：绑定机器名 `host`，只有它能登记这台新机器（#507）
fn new_host_token(db: &Path, name: &str, host: &str) -> String {
    let store = board_core::Store::open(db).unwrap();
    auth::create_token_for(store.conn(), name, Some(host), board_core::now_ms()).unwrap().1
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
        assert_eq!(r.code().as_str(), "NOT_LOGGED_IN", "{m} {p}");
    }
    // 伪造的会话
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("cookie", "board_session=forged")], None);
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"));
    // 错误的令牌
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("authorization", "Bearer bd_nope")], None);
    assert_eq!((r.status, r.code().as_str()), (401, "INVALID_TOKEN"));
    // 健康检查不需要登录，带版本和提交号（部署后用来核对线上版本）
    let r = send("GET", &format!("{}/api/health", s.url), &[], None);
    assert_eq!(r.status, 200);
    assert_eq!(r.body["ok"], true);
    assert_eq!(r.body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(r.body["commit"], board_server::COMMIT);
    assert_eq!(r.body["mode"], "github");
    assert!(r.body["ui"].is_boolean());
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
    assert_eq!((r.status, r.code().as_str()), (400, "USAGE"));
    // 退出后会话失效
    let r = send("POST", &format!("{}/api/auth/logout", s.url), &[("cookie", &cookie), UI], None);
    assert_eq!(r.status, 200);
    assert_eq!(r.cookie("board_session").as_deref(), Some(""));
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("cookie", &cookie)], None);
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"));
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
    assert_eq!((r.status, r.code().as_str()), (409, "NAME_OFFLINE"));
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
    assert_eq!((r.status, r.code().as_str()), (400, "NO_IDENTITY"));
    // agent 接口：心跳、在线列表
    let r = send("POST", &format!("{}/api/agents/beat", s.url), &a, Some(json!({ "name": NAME })));
    assert_eq!(r.status, 200, "{:?}", r.body);
    let r = send("GET", &format!("{}/api/agents", s.url), &a, None);
    assert_eq!(r.body[0]["name"], NAME);
    // 批量打标签只给界面用（default/main#531）：agent 走单卡 board label
    let r = send("POST", &format!("{}/api/labels/batch", s.url), &a, Some(json!({ "board": "default", "ids": [1], "add": [] })));
    assert_eq!(r.code().as_str(), "NOT_LOGGED_IN", "{:?}", r.body);
    // 规则仍由规则引擎判定：agent 不能删除、不能强制释放
    let r = send("POST", &format!("{}/api/cmd", s.url), &a, Some(json!({ "board": "default", "as": NAME, "op": "delete", "id": 1 })));
    assert_eq!(r.code().as_str(), "USAGE");
    // 评论只能由作者（人）修改：直接调接口改别人的评论，得到 403 NOT_AUTHOR
    let cmd = |body: serde_json::Value, h: &[(&str, &str)]| send("POST", &format!("{}/api/cmd", s.url), h, Some(body));
    let ui = [("cookie", cookie.as_str()), UI];
    let r = cmd(json!({ "board": "default", "op": "comment", "id": 1, "text": "人写的" }), &ui);
    let mine = r.body["activity"].as_array().unwrap().last().unwrap()["id"].as_i64().unwrap();
    let r = cmd(json!({ "board": "default", "as": NAME, "op": "comment", "id": 1, "text": "agent 写的" }), &a);
    let theirs = r.body["activity"].as_array().unwrap().last().unwrap()["id"].as_i64().unwrap();
    let r = cmd(json!({ "board": "default", "as": NAME, "op": "edit_comment", "id": 1, "activity": mine, "text": "x" }), &a);
    assert_eq!((r.status, r.code().as_str()), (403, "NOT_AUTHOR"));
    let r = cmd(json!({ "board": "default", "op": "edit_comment", "id": 1, "activity": theirs, "text": "x" }), &ui);
    assert_eq!((r.status, r.code().as_str()), (403, "NOT_AUTHOR"));
    let r = cmd(json!({ "board": "default", "op": "edit_comment", "id": 1, "activity": mine, "text": "人改过的" }), &ui);
    assert_eq!(r.status, 200, "{:?}", r.body);
    let c = r.body["activity"].as_array().unwrap().iter().find(|x| x["id"] == mine).unwrap().clone();
    assert_eq!(c["text"], "人改过的");
    assert!(c["edited_at"].is_i64(), "{c:?}");
    // 删除评论同样只能由作者（人）删：agent 删、人删 agent 的评论都是 403 NOT_AUTHOR；人删自己的评论成功
    let r = cmd(json!({ "board": "default", "as": NAME, "op": "delete_comment", "id": 1, "activity": mine }), &a);
    assert_eq!((r.status, r.code().as_str()), (403, "NOT_AUTHOR"));
    let r = cmd(json!({ "board": "default", "op": "delete_comment", "id": 1, "activity": theirs }), &ui);
    assert_eq!((r.status, r.code().as_str()), (403, "NOT_AUTHOR"));
    let r = cmd(json!({ "board": "default", "op": "delete_comment", "id": 1, "activity": mine }), &ui);
    assert_eq!(r.status, 200, "{:?}", r.body);
    assert!(!r.body["activity"].as_array().unwrap().iter().any(|x| x["id"] == mine));
    // 只有人能做的操作、机器列表与设置、令牌管理：令牌不能用
    for (m, p, body) in [
        ("POST", "/api/projects", Some(json!({ "name": "x", "key": "x", "board": { "name": "m", "key": "main" } }))),
        ("DELETE", "/api/boards/default/main", None),
        // 代码仓库地址、集成分支在项目上（按白板改的写到它所在的项目）：只有人能改
        ("PATCH", "/api/projects/default", Some(json!({ "repo": "git@github.com:o/r.git" }))),
        // 项目目录决定机器上的目录：同样只有人能改（#488）
        ("PATCH", "/api/projects/default", Some(json!({ "dir": "board" }))),
        ("PATCH", "/api/boards/default/main", Some(json!({ "branch": "dev" }))),
        ("POST", "/api/flow/default/main", Some(json!({ "name": "新状态" }))),
        ("POST", "/api/card-crons/default/main", Some(json!({ "name": "清理", "title": "清理", "cron": "0 9 * * *", "tz": "UTC" }))),
        ("PATCH", "/api/card-crons/default/main/1", Some(json!({ "on": false }))),
        ("DELETE", "/api/card-crons/default/main/1", None),
        ("POST", "/api/card-crons/default/main/1/run", None),
        ("GET", "/api/hosts", None),
        ("PATCH", "/api/hosts/mbp", Some(json!({ "max": 2 }))),
        ("POST", "/api/hosts/mbp/dirs/clone", Some(json!({ "board": "default/main", "url": "https://example.com/repo.git" }))),
        ("DELETE", "/api/hosts/mbp", None),
        ("DELETE", "/api/hosts/mbp?revoke=1", None),
        ("POST", "/api/hosts/mbp/ask", Some(json!({ "kind": "recheck" }))),
        ("POST", "/api/hosts/mbp/ask", Some(json!({ "kind": "upgrade", "now": true }))),
        ("GET", "/api/hosts/mbp/recent", None),
        ("GET", "/api/tokens", None),
        ("POST", "/api/tokens", Some(json!({ "name": "y" }))),
        ("GET", "/api/me", None),
    ] {
        let r = send(m, &format!("{}{p}", s.url), &a, body);
        assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{m} {p}");
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
    assert_eq!((r.status, r.code().as_str()), (401, "INVALID_TOKEN"));
}

/// board host、派发的接口只认机器上报时用的令牌（卡片 #379）：别的令牌读不到它的任务、领不了一轮、改不了名、
/// 回报不了结果，结束、写不了它的派发；离线后也上报接管不了它（接管后就能读走只发给它的密钥）。界面（人）不受影响
#[test]
fn host_and_dispatch_routes_only_take_that_hosts_token() {
    let s = local_mode();
    let (ta, tb) = (new_host_token(&s.db, "a 的令牌", "host-a"), new_token(&s.db, "b 的令牌"));
    let (ba, bb) = (format!("Bearer {ta}"), format!("Bearer {tb}"));
    let a = [("authorization", ba.as_str())];
    let b = [("authorization", bb.as_str())];
    let url = |p: &str| format!("{}/api{p}", s.url);
    let report = |inst: &str| {
        json!({
            "name": "host-a", "instance": inst, "os": "macOS 15.4", "version": env!("CARGO_PKG_VERSION"),
            "agents": [{ "id": "claude", "name": "Claude Code", "version": "2.4.1", "found": true }],
            "workspace": { "path": "/tmp/ws" },
            "dirs": [{ "board": "default/main", "path": "/tmp/ws/default" }],
        })
    };
    let r = send("POST", &url("/hosts/host-a/beat"), &a, Some(report("a1")));
    assert_eq!(r.status, 200, "{}", r.text);
    // 派一张卡给 host-a，它的 board host 开始
    let card = send("POST", &url("/cmd"), &[UI], Some(json!({ "board": "default/main", "op": "new", "title": "派给 host-a" })));
    assert_eq!(card.status, 200, "{}", card.text);
    let d = send("POST", &url("/dispatches"), &[UI], Some(json!({ "board": "default/main", "card": card.body["id"], "host": "host-a" })));
    assert_eq!(d.status, 200, "{}", d.text);
    let d = d.body["id"].as_i64().unwrap();
    let r = send("POST", &url("/hosts/host-a/begin"), &a, Some(json!({ "instance": "a1", "id": d })));
    assert_eq!(r.status, 200, "{}", r.text);

    // 令牌 B：知道机器名、进程标识、派发 ID 也不行，机器的接口像不存在一样（NO_SUCH_HOST），派发的是 NOT_FOUND
    for (m, p, body) in [
        ("GET", "/hosts/host-a/work".to_string(), None),
        ("POST", "/hosts/host-a/round".to_string(), Some(json!({}))),
        ("POST", "/hosts/host-a/rename".to_string(), Some(json!({ "name": "host-a-renamed" }))),
        ("POST", "/hosts/host-a/begin".to_string(), Some(json!({ "instance": "a1", "id": d }))),
        ("POST", "/hosts/host-a/pull".to_string(), Some(json!({ "instance": "a1" }))),
        ("POST", "/hosts/host-a/synced".to_string(), Some(json!({ "instance": "a1", "result": {} }))),
        ("POST", "/hosts/host-a/selftested".to_string(), Some(json!({ "instance": "a1", "id": 1 }))),
        ("POST", "/hosts/host-a/skills/imported".to_string(), Some(json!({ "instance": "a1", "id": 1 }))),
        ("POST", "/hosts/host-a/ls/1".to_string(), Some(json!({}))),
    ] {
        let r = send(m, &url(&p), &b, body);
        assert_eq!((r.status, r.code().as_str()), (404, "NO_SUCH_HOST"), "{m} {p}: {}", r.text);
    }
    for (m, p, body) in [
        ("POST", format!("/dispatches/{d}/end"), Some(json!({ "exitCode": 0 }))),
        ("POST", format!("/dispatches/{d}/wip"), Some(json!({ "branch": "wip/main/1" }))),
        ("POST", format!("/dispatches/{d}/log"), Some(json!({ "lines": ["伪造的日志"] }))),
        ("GET", format!("/dispatches/{d}/log"), None),
        ("POST", format!("/dispatches/{d}/events"), Some(json!({ "events": [] }))),
    ] {
        let r = send(m, &url(&p), &b, body);
        assert_eq!((r.status, r.code().as_str()), (404, "NOT_FOUND"), "{m} {p}: {}", r.text);
    }
    // 上面都没生效：机器名没改，派发还在跑，日志里没有伪造的那行
    let w = send("GET", &url("/hosts/host-a/work"), &a, None);
    assert_eq!(w.status, 200, "{}", w.text);
    assert_eq!(w.body["host"]["name"], "host-a");
    assert_eq!(w.body["running"][0]["id"], json!(d), "{}", w.text);
    let log = send("GET", &url(&format!("/dispatches/{d}/log")), &a, None);
    assert!(!log.text.contains("伪造"), "{}", log.text);

    // 令牌 B 冒充 host-a 上报：在线时不行；host-a 下线后也不行，令牌和密钥原文仍只给 A
    let r = send("POST", &url("/hosts/host-a/beat"), &b, Some(report("b1")));
    assert_eq!((r.status, r.code().as_str()), (409, "HOST_NAME_TAKEN"), "{}", r.text);
    let mut bye = report("a1");
    bye["final"] = json!(true);
    assert_eq!(send("POST", &url("/hosts/host-a/beat"), &a, Some(bye)).status, 200);
    let r = send("POST", &url("/hosts/host-a/beat"), &b, Some(report("b1")));
    assert_eq!((r.status, r.code().as_str()), (409, "HOST_NAME_TAKEN"), "{}", r.text);
    assert!(r.body["error"]["message"].as_str().unwrap().contains("a 的令牌"), "{}", r.text);
    let r = send("GET", &url("/hosts/host-a/mcp"), &b, None);
    assert_eq!(r.code(), "NOT_LOGGED_IN", "{}", r.text);

    // A 自己照常：重新上线、写日志、结束派发
    let mut back = report("a2");
    back["resume"] = json!("a1");
    assert_eq!(send("POST", &url("/hosts/host-a/beat"), &a, Some(back)).status, 200);
    assert_eq!(send("GET", &url("/hosts/host-a/mcp"), &a, None).status, 200);
    assert_eq!(send("POST", &url(&format!("/dispatches/{d}/log")), &a, Some(json!({ "lines": ["真的日志"] }))).status, 200);
    let log = send("GET", &url(&format!("/dispatches/{d}/log")), &a, None);
    assert!(log.text.contains("真的日志"), "{}", log.text);
    let r = send("POST", &url(&format!("/dispatches/{d}/end")), &a, Some(json!({ "exitCode": 0 })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(send("POST", &url("/hosts/host-a/round"), &a, Some(json!({}))).status, 200);

    // 界面（人）不受影响：取任务、领一轮、读日志、改名
    assert_eq!(send("GET", &url("/hosts/host-a/work"), &[UI], None).status, 200);
    assert_eq!(send("POST", &url("/hosts/host-a/round"), &[UI], Some(json!({}))).status, 200);
    assert_eq!(send("GET", &url(&format!("/dispatches/{d}/log")), &[UI], None).status, 200);
    let r = send("POST", &url("/hosts/host-a/rename"), &[UI], Some(json!({ "name": "host-a2" })));
    assert_eq!(r.status, 200, "{}", r.text);
    // 改名后 A 用新名字照常
    assert_eq!(send("GET", &url("/hosts/host-a2/work"), &a, None).status, 200);
}

/// 机器对话（PRD §9.1「机器对话」）：开对话、发消息、停止这一轮、读对话只能在界面上（令牌 NOT_LOGGED_IN，工作区里也看不到）；
/// board host 用这台机器的令牌取到原样的消息、回报，别的令牌回报像机器不存在一样
#[test]
fn machine_chats_are_ui_only_and_reported_by_that_host() {
    let s = local_mode();
    let (ta, tb) = (new_host_token(&s.db, "a 的令牌", "host-a"), new_token(&s.db, "b 的令牌"));
    let (ba, bb) = (format!("Bearer {ta}"), format!("Bearer {tb}"));
    let a = [("authorization", ba.as_str())];
    let b = [("authorization", bb.as_str())];
    let url = |p: &str| format!("{}/api{p}", s.url);
    let report = json!({
        "name": "host-a", "instance": "a1", "os": "macOS 15.4", "version": env!("CARGO_PKG_VERSION"),
        "agents": [{ "id": "claude", "name": "Claude Code", "version": "2.4.1", "found": true }],
        "workspace": { "path": "/tmp/ws" }, "dirs": [], "chats": [],
    });
    assert_eq!(send("POST", &url("/hosts/host-a/beat"), &a, Some(report)).status, 200);
    let text = "看看主目录下哪些文件夹最大";
    let c = send("POST", &url("/hosts/host-a/chats"), &[UI], Some(json!({ "agent": "claude", "text": text })));
    assert_eq!(c.status, 200, "{}", c.text);
    assert_eq!(c.body["turn"], "busy");
    let id = c.body["id"].as_i64().unwrap();
    for (m, p, body) in [
        ("POST", "/hosts/host-a/chats".to_string(), Some(json!({ "agent": "claude", "text": "x" }))),
        ("GET", format!("/chats/{id}"), None),
        ("POST", format!("/chats/{id}/prompt"), Some(json!({ "text": "x" }))),
        ("POST", format!("/chats/{id}/cancel"), Some(json!({}))),
    ] {
        let r = send(m, &url(&p), &a, body);
        assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{m} {p}: {}", r.text);
    }
    let ws = send("GET", &url("/workspace"), &a, None);
    assert_eq!(ws.body["chats"], json!([]), "令牌读工作区看不到对话");
    assert_eq!(send("GET", &url("/workspace"), &[UI], None).body["chats"][0]["id"], json!(id));
    // board host 取到的消息就是人写的那句
    let w = send("GET", &url("/hosts/host-a/work"), &a, None);
    assert_eq!(w.body["chats"][0]["prompts"][0]["text"], text, "{}", w.text);
    // 别的令牌回报：像机器不存在
    let r = send("POST", &url(&format!("/hosts/host-a/chats/{id}")), &b, Some(json!({ "took": 1 })));
    assert_eq!((r.status, r.code().as_str()), (404, "NO_SUCH_HOST"), "{}", r.text);
    // 这台的令牌回报：收下、回答、这一轮结束
    let r = send(
        "POST",
        &url(&format!("/hosts/host-a/chats/{id}")),
        &a,
        Some(json!({ "took": 1, "acp": "s1", "idle": "end_turn", "fmt": "acp", "events": [{ "kind": "message", "text": "最大的是 ~/Library" }] })),
    );
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["turn"], "idle");
    let v = send("GET", &url(&format!("/chats/{id}")), &[UI], None);
    assert_eq!(v.status, 200, "{}", v.text);
    let kinds: Vec<&str> = v.body["events"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["prompt", "message"]);
    // 接着发
    let r = send("POST", &url(&format!("/chats/{id}/prompt")), &[UI], Some(json!({ "text": "清一下 ~/.cache" })));
    assert_eq!((r.status, r.body["turn"].as_str()), (200, Some("busy")), "{}", r.text);
}

/// 登记新机器只认「添加机器」新建的机器令牌（#507）：普通访问令牌用一个没人用过的名字上报，不建机器、读不到
/// MCP / 供应商 / Git 的原文；机器令牌名字一致才能登记，A 的令牌读不了 B 的
#[test]
fn only_machine_tokens_register_new_hosts() {
    let s = local_mode();
    let url = |p: &str| format!("{}/api{p}", s.url);
    let report = |name: &str, inst: &str| json!({ "name": name, "instance": inst, "os": "macOS 15.4", "version": env!("CARGO_PKG_VERSION") });
    let plain = format!("Bearer {}", new_token(&s.db, "ci"));
    let plain = [("authorization", plain.as_str())];

    // 普通访问令牌自注册：409，不建机器，密钥接口照旧拒绝
    let r = send("POST", &url("/hosts/rnd-7f3a/beat"), &plain, Some(report("rnd-7f3a", "x1")));
    assert_eq!((r.status, r.code().as_str()), (409, "HOST_NAME_TAKEN"), "{}", r.text);
    assert!(r.body["error"]["message"].as_str().unwrap().contains("机器 → 添加机器"), "{}", r.text);
    let hosts = send("GET", &url("/hosts"), &[UI], None);
    assert_eq!(hosts.body["hosts"].as_array().map(Vec::len), Some(0), "{}", hosts.text);
    for p in ["mcp", "providers", "git"] {
        let r = send("GET", &url(&format!("/hosts/rnd-7f3a/{p}")), &plain, None);
        assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{p}: {}", r.text);
    }

    // 界面「添加机器」：新建绑定机器名的令牌，列表里看得到绑定的名字
    let r = send("POST", &url("/tokens"), &[UI], Some(json!({ "name": "devbox", "host": "devbox" })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["info"]["host"], "devbox");
    let dev = format!("Bearer {}", r.body["token"].as_str().unwrap());
    let dev = [("authorization", dev.as_str())];
    let r = send("POST", &url("/tokens"), &[UI], Some(json!({ "name": "坏名字", "host": "Bad Name" })));
    assert_eq!(r.code(), "USAGE", "{}", r.text);
    // 名字不一致：拒绝；一致：登记，能取自己的配置
    let r = send("POST", &url("/hosts/devbox-x/beat"), &dev, Some(report("devbox-x", "d1")));
    assert_eq!((r.status, r.code().as_str()), (409, "HOST_NAME_TAKEN"), "{}", r.text);
    let r = send("POST", &url("/hosts/devbox/beat"), &dev, Some(report("devbox", "d1")));
    assert_eq!(r.status, 200, "{}", r.text);
    for p in ["mcp", "providers", "git"] {
        assert_eq!(send("GET", &url(&format!("/hosts/devbox/{p}")), &dev, None).status, 200, "{p}");
    }
    // A 的令牌读不了 B 的密钥
    let other = format!("Bearer {}", new_host_token(&s.db, "other", "other"));
    let other = [("authorization", other.as_str())];
    assert_eq!(send("POST", &url("/hosts/other/beat"), &other, Some(report("other", "o1"))).status, 200);
    for p in ["mcp", "providers", "git"] {
        let r = send("GET", &url(&format!("/hosts/other/{p}")), &dev, None);
        assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{p}: {}", r.text);
    }
    let list = send("GET", &url("/tokens"), &[UI], None);
    let by_name = |n: &str| list.body.as_array().unwrap().iter().find(|t| t["name"] == n).unwrap()["host"].clone();
    assert_eq!((by_name("ci"), by_name("devbox")), (Value::Null, json!("devbox")));
}

/// 升级前的数据库：令牌表没有 host 列，启动时补上；已有的令牌都是普通访问令牌，已经绑着它们的机器照常上报（#507）
#[test]
fn old_token_table_gets_host_column_and_bound_hosts_keep_working() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("old.db");
    let mut store = board_core::Store::open(&db).unwrap();
    store
        .conn()
        .execute_batch(
            "CREATE TABLE auth_tokens (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, hash TEXT NOT NULL UNIQUE, hint TEXT NOT NULL, created_at INTEGER NOT NULL, last_used_at INTEGER);
             INSERT INTO auth_tokens (name, hash, hint, created_at) VALUES ('mbp 的旧令牌', 'h', 'bd_1', 0);",
        )
        .unwrap();
    // 升级前用这个令牌连上来的 mbp（旧版服务端不查绑定）
    let rep = |name: &str| serde_json::from_value::<board_core::HostReport>(json!({ "name": name, "instance": format!("{name}-1") })).unwrap();
    store.conn().execute("ALTER TABLE auth_tokens RENAME TO t_old", []).unwrap();
    store.write(board_core::now_ms(), board_core::Origin::cli(), |w| w.host_beat(&rep("mbp"), Some(1))).unwrap();
    store.conn().execute("ALTER TABLE t_old RENAME TO auth_tokens", []).unwrap();

    auth::setup(store.conn()).unwrap();
    auth::setup(store.conn()).unwrap();
    let list = auth::list_tokens(store.conn()).unwrap();
    assert_eq!((list.len(), list[0].name.as_str(), list[0].host.as_deref()), (1, "mbp 的旧令牌", None));
    let now = board_core::now_ms();
    store.write(now, board_core::Origin::cli(), |w| w.host_beat(&rep("mbp"), Some(1))).unwrap();
    assert!(store.read(|r| r.host_uses_token("mbp", 1, now)).unwrap());
    let e = store.write(now, board_core::Origin::cli(), |w| w.host_beat(&rep("new-one"), Some(1))).unwrap_err();
    assert_eq!(e.code, board_core::ErrorCode::HostNameTaken);
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
    let s = local_mode();
    let r = send("GET", &format!("{}/api/workspace", s.url), &[], None);
    assert_eq!(r.status, 200);
    assert_eq!(send("GET", &format!("{}/api/me", s.url), &[], None).body["mode"], "local");
    let health = send("GET", &format!("{}/api/health", s.url), &[], None);
    assert_eq!(health.status, 200);
    assert_eq!(health.body["mode"], "local");
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
    assert_eq!(r.code().as_str(), "INVALID_TOKEN");
}

/// 一条打开的实时推送。`ready` 在读到服务端发的第一帧后收到消息——那一帧是在订阅注册好之后才发的，
/// 所以测试用它等订阅建立，代替「先 sleep 200 毫秒」：慢机器上 sleep 到了订阅也可能还没注册，测试会误判。
/// `closed` 在推送连接断开时收到消息
struct Stream {
    ready: std::sync::mpsc::Receiver<()>,
    closed: std::sync::mpsc::Receiver<()>,
}

impl Stream {
    /// 等到服务端发出第一帧：订阅已经注册，之后的吊销 / 退出才会断开这条推送
    fn wait_ready(&self) {
        self.ready.recv_timeout(std::time::Duration::from_secs(10)).expect("推送应当在 10 秒内发来第一帧（那时订阅已经建立）");
    }
}

/// 把一条 200 的推送响应挂到后台线程：先读第一帧报就绪，然后一直读到断开
fn open(res: ureq::http::Response<ureq::Body>) -> Stream {
    assert_eq!(res.status().as_u16(), 200);
    let (ready_tx, ready) = std::sync::mpsc::channel();
    let (closed_tx, closed) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut r = std::io::BufReader::new(res.into_body().into_reader());
        let mut first = String::new();
        // 读到第一帧（`data: {...}`）说明订阅已经在服务端注册好；读不到（已经断开）就不报就绪
        if std::io::BufRead::read_line(&mut r, &mut first).unwrap_or(0) > 0 {
            let _ = ready_tx.send(());
        }
        let _ = std::io::copy(&mut r, &mut std::io::sink());
        let _ = closed_tx.send(());
    });
    Stream { ready, closed }
}

/// 用会话 Cookie 打开实时推送
fn open_stream(url: &str, cookie: &str) -> Stream {
    open(agent().get(format!("{url}/api/stream")).header("cookie", cookie).call().unwrap())
}

/// 用访问令牌打开实时推送
fn open_token_stream(url: &str, token: &str) -> Stream {
    let bearer = format!("Bearer {token}");
    open(agent().get(format!("{url}/api/stream")).header("authorization", &bearer).call().unwrap())
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
    sa.wait_ready();
    sb.wait_ready();
    let r = send("DELETE", &format!("{}/api/tokens/{ia}", s.url), &ui, None);
    assert_eq!(r.status, 200);
    sa.closed.recv_timeout(std::time::Duration::from_secs(3)).expect("被吊销的令牌：推送应当断开");
    assert!(sb.closed.recv_timeout(std::time::Duration::from_millis(800)).is_err(), "另一个令牌的推送不受影响");
    // 重新连接被拒绝
    let bearer = format!("Bearer {ta}");
    let r = send("GET", &format!("{}/api/stream", s.url), &[("authorization", &bearer)], None);
    assert_eq!((r.status, r.code().as_str()), (401, "INVALID_TOKEN"));
}

#[test]
fn logout_closes_that_sessions_stream() {
    let s = github("buglan");
    let a = session(&s);
    let b = session(&s);
    let sa = open_stream(&s.url, &a);
    let sb = open_stream(&s.url, &b);
    sa.wait_ready();
    sb.wait_ready();
    let r = send("POST", &format!("{}/api/auth/logout", s.url), &[("cookie", &a), UI], None);
    assert_eq!(r.status, 200);
    sa.closed.recv_timeout(std::time::Duration::from_secs(3)).expect("退出的会话：推送应当断开");
    assert!(sb.closed.recv_timeout(std::time::Duration::from_millis(800)).is_err(), "另一个会话的推送不受影响");
    // 重新连接被拒绝
    let r = send("GET", &format!("{}/api/stream", s.url), &[("cookie", &a)], None);
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"));
}

/// 实时推送定时查登录态（#510）：另一个进程（`board-server token revoke`）吊销的令牌、自然过期的会话不会通知运行中的
/// 服务端，推送在下一次定时检查时断开、之后收不到事件；别的令牌、没过期的会话不受影响
#[test]
fn streams_close_when_token_is_revoked_elsewhere_or_session_expires() {
    use std::time::Duration;
    let s = github_checking("buglan", Duration::from_millis(300));
    let (gone, kept) = (new_token(&s.db, "要吊销的"), new_token(&s.db, "留着的"));
    let gone_id: i64 = {
        let store = board_core::Store::open(&s.db).unwrap();
        auth::list_tokens(store.conn()).unwrap().into_iter().find(|t| t.name == "要吊销的").unwrap().id
    };
    let (a, b) = (session(&s), session(&s));
    let sg = open_token_stream(&s.url, &gone);
    let sk = open_token_stream(&s.url, &kept);
    let sa = open_stream(&s.url, &a);
    let sb = open_stream(&s.url, &b);
    for x in [&sg, &sk, &sa, &sb] {
        x.wait_ready();
    }
    // 另一个进程吊销令牌：服务端没收到通知
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_board-server"))
        .args(["token", "revoke", &gone_id.to_string(), "--db"])
        .arg(&s.db)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    sg.closed.recv_timeout(Duration::from_secs(5)).expect("命令行吊销的令牌：推送应当在下一次定时检查时断开");
    // 会话 a 自然过期（把它的过期时间调到过去；b 不动）
    {
        let store = board_core::Store::open(&s.db).unwrap();
        let mut all: Vec<(String, i64)> = store
            .conn()
            .prepare("SELECT hash, created_at FROM auth_sessions ORDER BY created_at, rowid")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(all.len(), 2);
        let (first, _) = all.remove(0);
        store.conn().execute("UPDATE auth_sessions SET expires_at = 0 WHERE hash = ?1", [&first]).unwrap();
    }
    sa.closed.recv_timeout(Duration::from_secs(5)).expect("过期的会话：推送应当断开");
    // 留着的令牌、没过期的会话：照常
    assert!(sk.closed.recv_timeout(Duration::from_millis(1200)).is_err(), "另一个令牌的推送不受影响");
    assert!(sb.closed.try_recv().is_err(), "没过期的会话不受影响");
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("cookie", &b)], None);
    assert_eq!(r.status, 200);
    let r = send("GET", &format!("{}/api/workspace", s.url), &[("cookie", &a)], None);
    assert_eq!(r.status, 401);
    let bearer = format!("Bearer {gone}");
    let r = send("GET", &format!("{}/api/stream", s.url), &[("authorization", &bearer)], None);
    assert_eq!((r.status, r.code().as_str()), (401, "INVALID_TOKEN"));
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
    Res { status, headers, body: serde_json::from_str(&text).unwrap_or(Value::Null), text }
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
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"));

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
    assert_eq!((r.status, r.code().as_str()), (400, "NO_IDENTITY"));
    let up = format!("{}/api/files?board=default&card={id}&name=run.log&as=claude%2Fopus-5.5", s.url);
    let r = upload(&up, &[("authorization", &bearer)], b"ok\n");
    assert_eq!((r.status, r.code().as_str()), (409, "NAME_OFFLINE"), "名字要先 check");
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
    assert_eq!(r.code().as_str(), "USAGE");
    let big = vec![b'a'; 10 * 1024 * 1024 + 1];
    let r = upload(&format!("{}/api/files?board=default&name=big.txt", s.url), &[("cookie", &cookie), UI], &big);
    assert_eq!(r.code().as_str(), "USAGE", "{:?}", r.body);
    assert!(r.body["error"]["message"].as_str().unwrap().contains("10 MB"));
    let r = upload(&format!("{}/api/files?board=default&card=999&name=a.png", s.url), &[("cookie", &cookie), UI], PNG);
    assert_eq!(r.code().as_str(), "NOT_FOUND");
    let (st, _, _) = download(&format!("{}/api/files/999", s.url), &[("cookie", &cookie)]);
    assert_eq!(st, 404);
}

/// agent 名字（PRD §3）：check 查重与占用是原子的；工作区快照带在线 agent 和它持有的卡片；
/// 只更新心跳时间的心跳不改 rev（不推送），上线、下线改 rev
/// 上传附件走写事务（卡片 #380）：`rev` 加一，开着的其他窗口据此刷新；卡片不存在时 NOT_FOUND，rev 不变、不留记录；
/// agent 的名字不在线时也一样被拒、rev 不变
#[test]
fn upload_bumps_rev_and_rejects_without_side_effects() {
    let s = local_mode();
    let rev = || send("GET", &format!("{}/api/workspace", s.url), &[], None).body["rev"].as_i64().unwrap();
    let c = send("POST", &format!("{}/api/cmd", s.url), &[UI], Some(json!({ "board": "default/main", "op": "new", "title": "要传附件" })));
    let id = c.body["id"].as_i64().unwrap();
    let attachments = || send("GET", &format!("{}/api/card?path=default/main&id={id}", s.url), &[], None).body["attachments"].as_array().unwrap().len();
    let r0 = rev();
    // 推送：开着的窗口（订阅实时推送）收到新的 rev
    let res = agent().get(format!("{}/api/stream", s.url)).call().unwrap();
    let mut lines = std::io::BufReader::new(res.into_body().into_reader());
    let mut next_rev = || {
        use std::io::BufRead;
        let mut l = String::new();
        loop {
            l.clear();
            lines.read_line(&mut l).unwrap();
            if let Some(d) = l.strip_prefix("data:") {
                return serde_json::from_str::<Value>(d.trim()).unwrap()["rev"].as_i64().unwrap();
            }
        }
    };
    assert_eq!(next_rev(), r0, "第一帧是现在的 rev");
    let r = upload(&format!("{}/api/files?board=default/main&card={id}&name=a.txt", s.url), &[UI], b"hello");
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(rev(), r0 + 1);
    assert_eq!(next_rev(), r0 + 1, "其他窗口收到刷新");
    assert_eq!(attachments(), 1);
    // 同样的内容再传一次：照样各记一条
    let r = upload(&format!("{}/api/files?board=default/main&card={id}&name=b.txt", s.url), &[UI], b"hello");
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!((rev(), attachments()), (r0 + 2, 2));
    // 卡片不存在：NOT_FOUND，rev 不变
    let r = upload(&format!("{}/api/files?board=default/main&card=999&name=c.txt", s.url), &[UI], b"nope");
    assert_eq!((r.status, r.code().as_str()), (404, "NOT_FOUND"), "{}", r.text);
    assert_eq!(rev(), r0 + 2);
    // agent 名字不在线：NAME_OFFLINE，rev 不变、不留记录
    let tok = new_token(&s.db, "t");
    let bearer = format!("Bearer {tok}");
    let r = upload(&format!("{}/api/files?board=default/main&card={id}&name=d.txt&as={NAME}", s.url), &[("authorization", &bearer)], b"x");
    assert_eq!(r.code(), "NAME_OFFLINE", "{}", r.text);
    assert_eq!((rev(), attachments()), (r0 + 2, 2));
}

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
        assert_eq!((r.status, r.code().as_str()), (400, "USAGE"), "{bad}");
        assert!(r.body["error"]["message"].as_str().unwrap().contains("<运行环境>/<模型>"));
    }
    let r = post("/api/agents/check", json!({ "name": "" }));
    assert_eq!(r.code().as_str(), "NO_IDENTITY");

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
        assert_eq!((r.status, r.code().as_str()), (409, "NAME_TAKEN"));
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
    assert_eq!((r.status, r.code().as_str()), (409, "NAME_OFFLINE"));
    let r = post("/api/cmd", json!({ "board": "default", "as": NAME, "op": "comment", "id": 1, "text": "x" }));
    assert_eq!(r.code().as_str(), "NAME_OFFLINE");
    // 读接口不检查名字
    assert_eq!(send("GET", &format!("{}/api/card?path=default&id=1", s.url), &a, None).status, 200);
    // 离线后可以重新 check
    assert_eq!(post("/api/agents/check", json!({ "name": NAME })).status, 200);
}

/// `board check --agent <标识>`（PRD §3.2）：不带名字，服务端按这台机器、这个 agent 的设置生成名字——
/// 认机器的顺序是请求里的 `host` → 令牌是它的 board host 上报用的那个；查不到机器时退回 `<标识>/<模型>`
#[test]
fn agent_check_generates_the_name_from_machine_settings() {
    use board_core::{HostAgent, HostDir, HostReport, HostWorkspace, Origin};
    let s = github("buglan");
    let agent_token = new_token(&s.db, "ci");
    // 这台机器的 board host 用的令牌：上报时带上它的 ID
    let (host_info, host_token) = {
        let store = board_core::Store::open(&s.db).unwrap();
        auth::create_token_for(store.conn(), "win2", Some("win"), board_core::now_ms()).unwrap()
    };
    let agent = |id: &str, model: &str| HostAgent { id: id.into(), name: id.into(), version: "1.0".into(), found: true, model: model.into(), ..Default::default() };
    let mut claude = agent("claude", "opus-5.5");
    claude.effort = "high".into();
    let rep = HostReport {
        name: "win".into(),
        instance: "i1".into(),
        resume: None,
        os: "Windows 11".into(),
        version: "0.1.24".into(),
        agents: vec![claude, agent("dsh", "")],
        envs: vec![],
        workspace: Some(HostWorkspace { path: "C:\\repo".into(), ..Default::default() }),
        dirs: vec![HostDir { board: "default/main".into(), path: "C:\\repo\\default\\main".into(), ..Default::default() }],
        dirs_rev: 0,
        running: vec![],
        chats: None,
        done: false,
        asked: None,
        ask_error: None,
        selftest: false,
        config_sync: true,
        rescan_skills: false,
        local_skills: None,
        res: None,
    };
    board_core::Store::open(&s.db).unwrap().write(board_core::now_ms(), Origin::cli(), |w| w.host_beat(&rep, Some(host_info.id))).unwrap();
    let check = |token: &str, body: Value| send("POST", &format!("{}/api/agents/check", s.url), &[("authorization", &format!("Bearer {token}"))], Some(body));
    // 带机器名：claude 配了 opus-5.5；名字和派发一致，占用时自动加序号
    let r = check(&agent_token, json!({ "agent": "claude", "host": "win", "session": true }));
    assert_eq!((r.status, r.body["name"].as_str()), (200, Some("win/opus-5.5-high")), "{:?}", r.body);
    assert_eq!(r.body["agent"], "claude", "名字里不再带 agent 标识，返回里单独带上");
    assert!(r.body["session"].is_string());
    let r = check(&agent_token, json!({ "agent": "claude", "host": "win", "session": true }));
    assert_eq!(r.body["name"], "win/opus-5.5-high-1", "{:?}", r.body);
    // 没配模型：win/dsh
    assert_eq!(check(&agent_token, json!({ "agent": "dsh", "host": "win" })).body["name"], "win/dsh");
    // 不带机器名、令牌不是任何机器的：退回 <标识>/<标识>
    let r = check(&agent_token, json!({ "agent": "codex" }));
    assert_eq!((r.status, r.body["name"].as_str(), r.body["modeled"].as_bool()), (200, Some("codex/codex"), Some(false)));
    // 令牌是 win 的 board host 上报用的：认得出这台机器（win/dsh 已经被上一个会话占了，加序号）
    assert_eq!(check(&host_token, json!({ "agent": "dsh", "session": true })).body["name"], "win/dsh-1");
    // 机器名不合法：USAGE；agent 标识不合法：USAGE
    assert_eq!(check(&agent_token, json!({ "agent": "dsh", "host": "Win 1" })).code().as_str(), "USAGE");
    assert_eq!(check(&agent_token, json!({ "agent": "Deep Seek", "host": "win" })).code().as_str(), "USAGE");
    // GET /api/hosts/mine：令牌是它的机器（登录的界面没有）
    let mine = send("GET", &format!("{}/api/hosts/mine", s.url), &[("authorization", &format!("Bearer {host_token}"))], None);
    assert_eq!((mine.status, mine.body["name"].as_str()), (200, Some("win")));
    let other = send("GET", &format!("{}/api/hosts/mine", s.url), &[("authorization", &format!("Bearer {agent_token}"))], None);
    assert_eq!((200, other.body), (200, json!(null)));
}
