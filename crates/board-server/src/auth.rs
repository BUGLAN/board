//! 登录与访问令牌：只在服务端入口这一层，规则引擎不知道它的存在（PRD §3）。
//!
//! - 人：用 GitHub 登录，得到会话 Cookie（`board_session`）。只有 `BOARD_GITHUB_USER` 指定的账号能登录。
//!   桌面端在系统默认浏览器里登录（用上浏览器里已有的 GitHub 登录状态），登录结果经本机回环地址交回桌面端，
//!   见 [`login`]、[`desktop`]。
//! - agent：`Authorization: Bearer bd_…` 访问令牌，只能调用 `board` 命令行用到的接口。
//! - 本机模式（`--local`）：只接受本机地址访问，不需要登录；带令牌的请求仍按 agent 处理。
//!
//! 会话和令牌在数据库里只存 SHA-256 哈希。

use crate::{ApiError, AppState};
use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use board_core::{BoardError, ErrorCode, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const SESSION_COOKIE: &str = "board_session";
const STATE_COOKIE: &str = "board_oauth";
/// 会话有效期 30 天；使用中每过一天续期一次
const SESSION_MS: i64 = 30 * 24 * 3600 * 1000;
const RENEW_MS: i64 = 24 * 3600 * 1000;
/// 桌面端登录：一次性登录码的有效期
const HANDOFF_MS: i64 = 2 * 60 * 1000;
/// 令牌「最近使用时间」最多每分钟更新一次
const TOUCH_MS: i64 = 60 * 1000;
pub const TOKEN_PREFIX: &str = "bd_";

/// 登录方式
#[derive(Debug, Clone)]
pub enum Auth {
    /// 本机模式：只监听回环地址，不需要登录
    Local,
    GitHub(GitHub),
}

#[derive(Debug, Clone)]
pub struct GitHub {
    pub client_id: String,
    pub client_secret: String,
    /// 允许登录的 GitHub 账号（不区分大小写）
    pub user: String,
    /// 对外地址，例如 https://board.buglan.com
    pub public_url: String,
    /// GitHub 授权地址与 API 地址（默认 https://github.com、https://api.github.com；测试时指向模拟服务）
    pub oauth_url: String,
    pub api_url: String,
}

impl GitHub {
    fn redirect_uri(&self) -> String {
        format!("{}/auth/github/callback", self.public_url.trim_end_matches('/'))
    }
    fn secure(&self) -> bool {
        self.public_url.starts_with("https://")
    }
}

/// 发起请求的是谁：带令牌的是 agent，其余是人
#[derive(Debug, Clone)]
pub enum Caller {
    /// session：登录会话的哈希（本机模式没有）。会话被删除后，它打开的实时推送随之关闭
    Human { login: Option<String>, session: Option<String> },
    Agent { token: i64 },
}

/// 会话是否还在（实时推送用来判断要不要断开）
pub(crate) fn session_alive(conn: &Connection, session_hash: &str, now: i64) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM auth_sessions WHERE hash = ?1 AND expires_at > ?2",
        params![session_hash, now],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/* ------------------------------ 存储 ------------------------------ */

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS auth_sessions (
  hash       TEXT PRIMARY KEY,
  login      TEXT NOT NULL,
  name       TEXT NOT NULL DEFAULT '',
  avatar     TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
-- 桌面端登录：浏览器里登录成功后发给桌面端的一次性登录码（只存哈希），桌面端用它换会话
CREATE TABLE IF NOT EXISTS auth_handoffs (
  hash       TEXT PRIMARY KEY,
  login      TEXT NOT NULL,
  name       TEXT NOT NULL DEFAULT '',
  avatar     TEXT NOT NULL DEFAULT '',
  expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS auth_tokens (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  name         TEXT NOT NULL,
  hash         TEXT NOT NULL UNIQUE,
  hint         TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  last_used_at INTEGER
);
"#;

pub fn setup(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

fn hash(raw: &str) -> String {
    hex(&Sha256::digest(raw.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("系统随机数不可用");
    hex(&buf)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfo {
    pub id: i64,
    pub name: String,
    /// 令牌开头几位，便于辨认
    pub hint: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// 新建访问令牌。返回令牌信息和令牌原文（只在这里出现一次）
pub fn create_token(conn: &Connection, name: &str, now: i64) -> Result<(TokenInfo, String)> {
    let name: String = name.trim().chars().take(40).collect();
    if name.is_empty() {
        return Err(BoardError::usage("令牌名称不能为空"));
    }
    let raw = format!("{TOKEN_PREFIX}{}", random_hex(24));
    let hint = format!("{}…", &raw[..TOKEN_PREFIX.len() + 6]);
    conn.execute(
        "INSERT INTO auth_tokens (name, hash, hint, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![name, hash(&raw), hint, now],
    )?;
    let id = conn.last_insert_rowid();
    Ok((TokenInfo { id, name, hint, created_at: now, last_used_at: None }, raw))
}

pub fn list_tokens(conn: &Connection) -> Result<Vec<TokenInfo>> {
    let mut st = conn.prepare("SELECT id, name, hint, created_at, last_used_at FROM auth_tokens ORDER BY id")?;
    let rows = st.query_map([], |r| {
        Ok(TokenInfo { id: r.get(0)?, name: r.get(1)?, hint: r.get(2)?, created_at: r.get(3)?, last_used_at: r.get(4)? })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// 吊销：直接删除，之后用它的请求得到 INVALID_TOKEN
pub fn revoke_token(conn: &Connection, id: i64) -> Result<()> {
    if conn.execute("DELETE FROM auth_tokens WHERE id = ?1", [id])? == 0 {
        return Err(BoardError::usage(format!("令牌 #{id} 不存在")));
    }
    Ok(())
}

/// 校验令牌，返回令牌 ID
fn check_token(conn: &Connection, raw: &str, now: i64) -> Result<Option<i64>> {
    let row: Option<(i64, Option<i64>)> = conn
        .query_row("SELECT id, last_used_at FROM auth_tokens WHERE hash = ?1", [hash(raw)], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((id, last)) = row else { return Ok(None) };
    if last.is_none_or(|t| now - t >= TOUCH_MS) {
        conn.execute("UPDATE auth_tokens SET last_used_at = ?2 WHERE id = ?1", params![id, now])?;
    }
    Ok(Some(id))
}

#[derive(Debug, Clone, Deserialize)]
struct GhUser {
    login: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
}

fn create_session(conn: &Connection, u: &GhUser, now: i64) -> Result<String> {
    conn.execute("DELETE FROM auth_sessions WHERE expires_at <= ?1", [now])?;
    let raw = random_hex(32);
    conn.execute(
        "INSERT INTO auth_sessions (hash, login, name, avatar, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            hash(&raw),
            u.login,
            u.name.clone().unwrap_or_default(),
            u.avatar_url.clone().unwrap_or_default(),
            now,
            now + SESSION_MS
        ],
    )?;
    Ok(raw)
}

/// 桌面端登录：为已通过校验的 GitHub 用户发一个一次性登录码
fn create_handoff(conn: &Connection, u: &GhUser, now: i64) -> Result<String> {
    conn.execute("DELETE FROM auth_handoffs WHERE expires_at <= ?1", [now])?;
    let raw = random_hex(32);
    conn.execute(
        "INSERT INTO auth_handoffs (hash, login, name, avatar, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            hash(&raw),
            u.login,
            u.name.clone().unwrap_or_default(),
            u.avatar_url.clone().unwrap_or_default(),
            now + HANDOFF_MS
        ],
    )?;
    Ok(raw)
}

/// 用一次性登录码换会话：码用过即删；过期、不存在或账号已不允许登录时返回 None
fn redeem_handoff(conn: &Connection, raw: &str, allowed: &str, now: i64) -> Result<Option<String>> {
    let h = hash(raw);
    let row: Option<(String, String, String, i64)> = conn
        .query_row("SELECT login, name, avatar, expires_at FROM auth_handoffs WHERE hash = ?1", [&h], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    conn.execute("DELETE FROM auth_handoffs WHERE hash = ?1", [&h])?;
    let Some((login, name, avatar, expires)) = row else { return Ok(None) };
    if expires <= now || !login.eq_ignore_ascii_case(allowed) {
        return Ok(None);
    }
    let u = GhUser { login, name: Some(name), avatar_url: Some(avatar) };
    create_session(conn, &u, now).map(Some)
}

#[derive(Debug, Clone)]
struct Session {
    login: String,
    /// 这次请求顺带续期了：响应里要重新下发 Cookie
    renewed: bool,
}

fn check_session(conn: &Connection, raw: &str, allowed: &str, now: i64) -> Result<Option<Session>> {
    let h = hash(raw);
    let row: Option<(String, i64)> = conn
        .query_row("SELECT login, expires_at FROM auth_sessions WHERE hash = ?1", [&h], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((login, expires)) = row else { return Ok(None) };
    // 过期，或允许登录的账号已经换了
    if expires <= now || !login.eq_ignore_ascii_case(allowed) {
        conn.execute("DELETE FROM auth_sessions WHERE hash = ?1", [&h])?;
        return Ok(None);
    }
    let renewed = now + SESSION_MS - expires >= RENEW_MS;
    if renewed {
        conn.execute("UPDATE auth_sessions SET expires_at = ?2 WHERE hash = ?1", params![h, now + SESSION_MS])?;
    }
    Ok(Some(Session { login, renewed }))
}

/* ------------------------------ Cookie ------------------------------ */

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

fn set_cookie(name: &str, value: &str, max_age_s: i64, path: &str, secure: bool) -> HeaderValue {
    let s = format!(
        "{name}={value}; Path={path}; Max-Age={max_age_s}; HttpOnly; SameSite=Lax{}",
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&s).expect("cookie")
}

fn session_cookie(raw: &str, secure: bool) -> HeaderValue {
    set_cookie(SESSION_COOKIE, raw, SESSION_MS / 1000, "/", secure)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let t = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))?.trim();
    (!t.is_empty()).then_some(t)
}

/// 本机模式只接受本机地址（防止其他网页借 DNS 重绑定访问本机服务）
fn local_host(headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else { return false };
    let name = if host.starts_with('[') {
        host.split(']').next().map(|h| format!("{h}]")).unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or_default().to_string()
    };
    matches!(name.as_str(), "127.0.0.1" | "localhost" | "[::1]")
}

/* ------------------------------ 入口 ------------------------------ */

/// `/api/*` 的守卫：确认身份，限制令牌能用的接口，写请求要带 x-board-client（防跨站请求）
pub(crate) async fn guard(State(s): State<AppState>, mut req: Request, next: Next) -> Response {
    let full = req.uri().path().to_string();
    let path = full.strip_prefix("/api").unwrap_or(&full).to_string();
    if path == "/health" {
        return next.run(req).await;
    }
    let auth = s.auth().clone();
    if matches!(auth, Auth::Local) && !local_host(req.headers()) {
        return ApiError(BoardError::usage("本机模式只接受 127.0.0.1 或 localhost 访问")).into_response();
    }
    let now = board_core::now_ms();
    let mut renew: Option<(String, bool)> = None;
    let caller = if let Some(tok) = bearer(req.headers()).map(str::to_string) {
        match s.db(move |st| check_token(st.conn(), &tok, now)).await {
            Ok(Some(id)) => Caller::Agent { token: id },
            Ok(None) => return ApiError(BoardError::code(ErrorCode::InvalidToken)).into_response(),
            Err(e) => return ApiError(e).into_response(),
        }
    } else {
        match &auth {
            Auth::Local => Caller::Human { login: None, session: None },
            Auth::GitHub(gh) => {
                let Some(sid) = cookie(req.headers(), SESSION_COOKIE).map(str::to_string) else {
                    return ApiError(BoardError::code(ErrorCode::NotLoggedIn)).into_response();
                };
                let allowed = gh.user.clone();
                let sid2 = sid.clone();
                match s.db(move |st| check_session(st.conn(), &sid2, &allowed, now)).await {
                    Ok(Some(sess)) => {
                        let session = Some(hash(&sid));
                        if sess.renewed {
                            renew = Some((sid, gh.secure()));
                        }
                        Caller::Human { login: Some(sess.login), session }
                    }
                    Ok(None) => return ApiError(BoardError::code(ErrorCode::NotLoggedIn)).into_response(),
                    Err(e) => return ApiError(e).into_response(),
                }
            }
        }
    };

    match &caller {
        Caller::Agent { token } => {
            // 令牌只能用于命令行：读工作区、白板、卡片，执行卡片命令，读取和上传附件；
            // agent 接口：check 名字、agent 级心跳、在线列表（PRD §3）；
            // 任务下发：board host 的上报、取任务、开始 / 结束派发、上传和读取日志、订阅实时推送，board dispatch 派发和取消
            let m = req.method().clone();
            let cli = (m == Method::GET
                && (matches!(path.as_str(), "/workspace" | "/board" | "/card" | "/agents" | "/stream")
                    || path.starts_with("/files/")
                    || host_path(&path, &["work"])
                    || dispatch_path(&path, "log")))
                || (m == Method::POST
                    && (matches!(path.as_str(), "/cmd" | "/files" | "/agents/check" | "/agents/beat" | "/dispatches" | "/dispatches/cancel")
                        || host_path(&path, &["beat", "begin", "pull", "round"])
                        || dispatch_path(&path, "end")
                        || dispatch_path(&path, "log")));
            if !cli {
                return ApiError(BoardError::new(
                    ErrorCode::NotLoggedIn,
                    "访问令牌只能用于 board 命令行；这个操作需要在界面上登录后进行",
                ))
                .into_response();
            }
            // 带 agent 名字的请求由处理函数记录名字
            if !matches!(path.as_str(), "/cmd" | "/files" | "/agents/check" | "/agents/beat") {
                tracing::Span::current().record("who", format!("令牌#{token}"));
            }
        }
        Caller::Human { login, .. } => {
            if req.method() != Method::GET && !req.headers().contains_key("x-board-client") {
                return ApiError(BoardError::usage("缺少 x-board-client 请求头")).into_response();
            }
            tracing::Span::current().record("who", login.as_deref().unwrap_or("本机"));
        }
    }

    req.extensions_mut().insert(caller);
    let mut res = next.run(req).await;
    if let Some((sid, secure)) = renew {
        res.headers_mut().append(header::SET_COOKIE, session_cookie(&sid, secure));
    }
    res
}

/// `/hosts/{名字}/<后缀>`，后缀是给定的几个之一
fn host_path(path: &str, tails: &[&str]) -> bool {
    let mut it = path.strip_prefix("/hosts/").unwrap_or_default().split('/');
    matches!((it.next(), it.next(), it.next()), (Some(n), Some(t), None) if !n.is_empty() && tails.contains(&t))
}

/// `/dispatches/{ID}/<后缀>`
fn dispatch_path(path: &str, tail: &str) -> bool {
    let mut it = path.strip_prefix("/dispatches/").unwrap_or_default().split('/');
    matches!((it.next(), it.next(), it.next()), (Some(id), Some(t), None) if id.parse::<i64>().is_ok() && t == tail)
}

/// 令牌的名称（派发的发起人没有 agent 名字时用）
pub(crate) fn token_name(conn: &Connection, id: i64) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT name FROM auth_tokens WHERE id = ?1", [id], |r| r.get(0)).optional()?)
}

/* ------------------------------ GitHub 登录 ------------------------------ */

fn redirect(to: &str) -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, to.to_string())]).into_response()
}

/// 桌面端登录：桌面端在本机回环地址上监听的端口，和它用来核对回调的随机串
#[derive(Debug, Clone, PartialEq)]
struct Desktop {
    port: u16,
    nonce: String,
}

impl Desktop {
    fn parse(port: &str, nonce: &str) -> Option<Self> {
        let port: u16 = port.parse().ok().filter(|p| *p != 0)?;
        let ok = (16..=64).contains(&nonce.len()) && nonce.bytes().all(|b| b.is_ascii_hexdigit());
        ok.then(|| Desktop { port, nonce: nonce.to_string() })
    }
    /// 登录结果交回桌面端：只会跳到本机回环地址
    fn redirect(&self, query: &[(&str, &str)]) -> Response {
        let mut q = vec![("state", self.nonce.as_str())];
        q.extend_from_slice(query);
        let q = serde_urlencoded::to_string(q).unwrap_or_default();
        redirect(&format!("http://127.0.0.1:{}/callback?{q}", self.port))
    }
}

/// state Cookie：网页端是 `<state>`；桌面端是 `<state>.<端口>.<随机串>`
fn state_cookie_value(state: &str, desktop: Option<&Desktop>) -> String {
    match desktop {
        Some(d) => format!("{state}.{}.{}", d.port, d.nonce),
        None => state.to_string(),
    }
}

fn parse_state_cookie(v: &str) -> (&str, Option<Desktop>) {
    let mut it = v.splitn(3, '.');
    let state = it.next().unwrap_or_default();
    let desktop = match (it.next(), it.next()) {
        (Some(port), Some(nonce)) => Desktop::parse(port, nonce),
        _ => None,
    };
    (state, desktop)
}

#[derive(Deserialize)]
pub(crate) struct LoginQuery {
    #[serde(default)]
    desktop: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
}

/// `GET /auth/github`：跳到 GitHub 授权页
///
/// 桌面端带上 `?desktop=<端口>&nonce=<随机串>`，在系统默认浏览器里打开这个地址：登录成功后浏览器跳到
/// `http://127.0.0.1:<端口>/callback?state=<随机串>&code=<一次性登录码>`（失败时是 `&error=<原因>`），
/// 桌面端收到后在自己的窗口里打开 `/auth/desktop?code=…` 换取会话。登录码只会送到本机回环地址，
/// 别人诱导你打开带 desktop 参数的链接也拿不到它。
pub(crate) async fn login(State(s): State<AppState>, Query(q): Query<LoginQuery>) -> Response {
    let Auth::GitHub(gh) = s.auth() else { return redirect("/") };
    let desktop = match (q.desktop.as_deref(), q.nonce.as_deref()) {
        (None, _) => None,
        (Some(port), nonce) => match Desktop::parse(port, nonce.unwrap_or_default()) {
            Some(d) => Some(d),
            None => return (StatusCode::BAD_REQUEST, "桌面端登录参数不正确").into_response(),
        },
    };
    let state = random_hex(16);
    let q = serde_urlencoded::to_string([
        ("client_id", gh.client_id.as_str()),
        ("redirect_uri", gh.redirect_uri().as_str()),
        ("state", state.as_str()),
        ("allow_signup", "false"),
    ])
    .unwrap_or_default();
    let mut res = redirect(&format!("{}/login/oauth/authorize?{q}", gh.oauth_url.trim_end_matches('/')));
    let v = state_cookie_value(&state, desktop.as_ref());
    res.headers_mut().append(header::SET_COOKIE, set_cookie(STATE_COOKIE, &v, 600, "/auth", gh.secure()));
    res
}

#[derive(Deserialize)]
pub(crate) struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// `GET /auth/github/callback`：校验 state，换取 GitHub 用户，只放行指定账号
pub(crate) async fn callback(State(s): State<AppState>, headers: HeaderMap, Query(q): Query<CallbackQuery>) -> Response {
    let Auth::GitHub(gh) = s.auth().clone() else { return redirect("/") };
    let secure = gh.secure();
    let (expected, desktop) = match cookie(&headers, STATE_COOKIE) {
        Some(v) => {
            let (st, d) = parse_state_cookie(v);
            (Some(st.to_string()), d)
        }
        None => (None, None),
    };
    let clear_state = set_cookie(STATE_COOKIE, "", 0, "/auth", secure);
    let fail = |reason: &str, login: Option<&str>| {
        // GitHub 账号名只含字母、数字和 -
        let login: Option<String> = login.map(|l| l.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect());
        let mut res = match &desktop {
            Some(d) => {
                let mut q = vec![("error", reason)];
                if let Some(l) = &login {
                    q.push(("login", l.as_str()));
                }
                d.redirect(&q)
            }
            None => {
                let mut to = format!("/?login_error={reason}");
                if let Some(l) = &login {
                    to += &format!("&login={l}");
                }
                redirect(&to)
            }
        };
        res.headers_mut().append(header::SET_COOKIE, clear_state.clone());
        res
    };
    if q.error.is_some() {
        return fail("cancelled", None);
    }
    let (Some(code), Some(state)) = (q.code, q.state) else { return fail("state", None) };
    if expected.as_deref() != Some(state.as_str()) {
        return fail("state", None);
    }
    let gh2 = gh.clone();
    let user = tokio::task::spawn_blocking(move || github_user(&gh2, &code))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    let user = match user {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("GitHub 登录失败：{e}");
            return fail("github", None);
        }
    };
    if !user.login.eq_ignore_ascii_case(&gh.user) {
        tracing::warn!(login = %user.login, "拒绝登录：不是允许的 GitHub 账号");
        return fail("denied", Some(&user.login));
    }
    let now = board_core::now_ms();
    let u = user.clone();
    // 桌面端：浏览器这边不登录，只把一次性登录码交给桌面端
    if let Some(d) = &desktop {
        let code = match s.db(move |st| create_handoff(st.conn(), &u, now)).await {
            Ok(c) => c,
            Err(e) => return ApiError(e).into_response(),
        };
        tracing::info!(login = %user.login, "GitHub 登录成功（桌面端）");
        let mut res = d.redirect(&[("code", code.as_str())]);
        res.headers_mut().append(header::SET_COOKIE, clear_state);
        return res;
    }
    let sid = match s.db(move |st| create_session(st.conn(), &u, now)).await {
        Ok(sid) => sid,
        Err(e) => return ApiError(e).into_response(),
    };
    tracing::info!(login = %user.login, "GitHub 登录成功");
    let mut res = redirect("/");
    res.headers_mut().append(header::SET_COOKIE, clear_state);
    res.headers_mut().append(header::SET_COOKIE, session_cookie(&sid, secure));
    res
}

#[derive(Deserialize)]
pub(crate) struct DesktopQuery {
    #[serde(default)]
    code: Option<String>,
}

/// `GET /auth/desktop?code=…`：桌面端用一次性登录码换会话 Cookie，然后打开白板
pub(crate) async fn desktop(State(s): State<AppState>, Query(q): Query<DesktopQuery>) -> Response {
    let Auth::GitHub(gh) = s.auth().clone() else { return redirect("/") };
    let Some(code) = q.code.filter(|c| !c.is_empty()) else { return redirect("/?login_error=state") };
    let now = board_core::now_ms();
    let allowed = gh.user.clone();
    match s.db(move |st| redeem_handoff(st.conn(), &code, &allowed, now)).await {
        Ok(Some(sid)) => {
            let mut res = redirect("/");
            res.headers_mut().append(header::SET_COOKIE, session_cookie(&sid, gh.secure()));
            res
        }
        Ok(None) => redirect("/?login_error=state"),
        Err(e) => ApiError(e).into_response(),
    }
}

fn github_user(gh: &GitHub, code: &str) -> std::result::Result<GhUser, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut res = agent
        .post(format!("{}/login/oauth/access_token", gh.oauth_url.trim_end_matches('/')))
        .header("accept", "application/json")
        .send_form([
            ("client_id", gh.client_id.as_str()),
            ("client_secret", gh.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", gh.redirect_uri().as_str()),
        ])
        .map_err(|e| format!("换取访问令牌失败：{e}"))?;
    let tok: Value = res.body_mut().read_json().map_err(|e| format!("换取访问令牌失败：{e}"))?;
    let Some(access) = tok["access_token"].as_str() else {
        return Err(format!("GitHub 没有返回访问令牌：{}", tok["error_description"].as_str().or(tok["error"].as_str()).unwrap_or("未知原因")));
    };
    let mut res = agent
        .get(format!("{}/user", gh.api_url.trim_end_matches('/')))
        .header("authorization", format!("Bearer {access}"))
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "board")
        .call()
        .map_err(|e| format!("读取 GitHub 用户失败：{e}"))?;
    if !res.status().is_success() {
        return Err(format!("读取 GitHub 用户失败：HTTP {}", res.status()));
    }
    res.body_mut().read_json::<GhUser>().map_err(|e| format!("读取 GitHub 用户失败：{e}"))
}

/* ------------------------------ 账号与令牌接口 ------------------------------ */

/// `GET /api/me`：当前登录的是谁
pub(crate) async fn me(State(s): State<AppState>, Extension(c): Extension<Caller>) -> std::result::Result<Json<Value>, ApiError> {
    let Caller::Human { login, .. } = c else { return Err(ApiError(BoardError::code(ErrorCode::NotLoggedIn))) };
    let Some(login) = login else { return Ok(Json(json!({ "mode": "local" }))) };
    let l = login.clone();
    let (name, avatar): (String, String) = s
        .db(move |st| {
            Ok(st
                .conn()
                .query_row(
                    "SELECT name, avatar FROM auth_sessions WHERE login = ?1 ORDER BY created_at DESC LIMIT 1",
                    [&l],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .unwrap_or_default())
        })
        .await?;
    Ok(Json(json!({ "mode": "github", "login": login, "name": name, "avatar": avatar })))
}

/// `POST /api/auth/logout`
pub(crate) async fn logout(State(s): State<AppState>, headers: HeaderMap) -> Response {
    let secure = matches!(s.auth(), Auth::GitHub(gh) if gh.secure());
    if let Some(sid) = cookie(&headers, SESSION_COOKIE).map(str::to_string) {
        let _ = s.db(move |st| Ok(st.conn().execute("DELETE FROM auth_sessions WHERE hash = ?1", [hash(&sid)])?)).await;
        // 这个会话打开的实时推送随之关闭
        s.sessions_changed();
    }
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut().append(header::SET_COOKIE, set_cookie(SESSION_COOKIE, "", 0, "/", secure));
    res
}

pub(crate) async fn tokens(State(s): State<AppState>) -> std::result::Result<Json<Vec<TokenInfo>>, ApiError> {
    Ok(Json(s.db(|st| list_tokens(st.conn())).await?))
}

#[derive(Deserialize)]
pub(crate) struct NewToken {
    name: String,
}

pub(crate) async fn new_token(State(s): State<AppState>, Json(v): Json<Value>) -> std::result::Result<Json<Value>, ApiError> {
    let b: NewToken = crate::parse(v)?;
    let now = board_core::now_ms();
    let (info, raw) = s.db(move |st| create_token(st.conn(), &b.name, now)).await?;
    Ok(Json(json!({ "token": raw, "info": info })))
}

pub(crate) async fn revoke(State(s): State<AppState>, Path(id): Path<i64>) -> std::result::Result<Json<Value>, ApiError> {
    s.db(move |st| revoke_token(st.conn(), id)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_check() {
        let h = |v: &str| {
            let mut m = HeaderMap::new();
            m.insert(header::HOST, v.parse().unwrap());
            local_host(&m)
        };
        assert!(h("127.0.0.1:7420"));
        assert!(h("localhost:1420"));
        assert!(h("[::1]:7420"));
        assert!(!h("evil.example.com"));
        assert!(!h("127.0.0.1.evil.com:7420"));
    }

    #[test]
    fn state_cookie_roundtrip() {
        assert_eq!(parse_state_cookie("abc"), ("abc", None));
        let d = Desktop::parse("51234", "0123456789abcdef").unwrap();
        let v = state_cookie_value("abc", Some(&d));
        assert_eq!(parse_state_cookie(&v), ("abc", Some(d)));
        // 端口、随机串不合法：当作网页端
        assert_eq!(parse_state_cookie("abc.0.0123456789abcdef").1, None);
        assert_eq!(parse_state_cookie("abc.80.short").1, None);
        assert_eq!(parse_state_cookie("abc.80.0123456789abcdeg").1, None);
        assert!(Desktop::parse("70000", "0123456789abcdef").is_none());
    }

    #[test]
    fn cookie_parse() {
        let mut m = HeaderMap::new();
        m.insert(header::COOKIE, "a=1; board_session=abc; b=2".parse().unwrap());
        assert_eq!(cookie(&m, SESSION_COOKIE), Some("abc"));
        assert_eq!(cookie(&m, "nope"), None);
    }
}
