//! Board 服务：界面和远程模式的 CLI 都通过它读写白板。
//!
//! - 身份（`auth`）：所有 `/api/*` 都要登录（GitHub 会话 Cookie）或带访问令牌，只有 `/api/health` 例外；
//!   本机模式（`--local`）只接受本机访问，不需要登录
//! - `GET /api/workspace`、`/api/board`、`/api/card`：快照
//! - `POST /api/cmd`：卡片命令（与 CLI 走同一个规则引擎）。带令牌的请求按 agent 执行，名字取请求里的 `as`
//! - `GET /api/me`、`POST /api/auth/logout`、`/api/tokens`：账号与访问令牌
//! - `/auth/github`、`/auth/github/callback`：GitHub 登录
//! - 项目 / 白板 / 标签 / 画布位置的增删改
//! - 工作流：`/api/flow/{项目}/{白板}`（新增状态、调整顺序）、`…/layout`（状态块布局）、`…/{状态}`（修改、删除）
//! - `GET /api/stream`：SSE。任何一处（其他窗口、CLI、超时释放）写入后 ≤ 300ms 推送 `{rev, events}`
//!
//! 本地模式的 CLI 直接写数据库，服务端每 250ms 检查一次 rev，发现变化就推送给已打开的界面（PRD §11）。
//! 服务端每秒扫描一次到期的领取（规则 7），没有命令执行时界面上也能看到超时释放。

pub mod auth;

use auth::{Auth, Caller};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, Request, StatusCode, Uri},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, patch, post, put},
    Extension, Json, Router,
};
use board_core::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    convert::Infallible,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{broadcast, watch, Notify};
use tower_http::trace::{DefaultOnResponse, TraceLayer};

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct WebAssets;

#[derive(Debug, Clone)]
pub struct Config {
    pub db: PathBuf,
    /// 空数据库时写入示例工作区
    pub demo: bool,
    /// 是否托管界面
    pub serve_web: bool,
    /// 登录方式：GitHub，或本机模式（不需要登录）
    pub auth: Auth,
}

impl Config {
    pub fn new(db: PathBuf, auth: Auth) -> Self {
        Config { db, demo: false, serve_web: true, auth }
    }
}

struct Inner {
    store: Mutex<Store>,
    tx: broadcast::Sender<Arc<str>>,
    poke: Notify,
    db: PathBuf,
    auth: Auth,
    /// 有会话被删除（退出登录）时加一：实时推送据此检查自己的会话还在不在
    sessions: watch::Sender<u64>,
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl AppState {
    pub fn open(cfg: &Config) -> Result<AppState> {
        let mut store = Store::open(&cfg.db)?;
        let now = now_ms();
        if cfg.demo {
            board_core::seed::seed_demo(&mut store, now)?;
        }
        store.ensure_default(now)?;
        auth::setup(store.conn())?;
        let (tx, _) = broadcast::channel(256);
        Ok(AppState(Arc::new(Inner {
            store: Mutex::new(store),
            tx,
            poke: Notify::new(),
            db: cfg.db.clone(),
            auth: cfg.auth.clone(),
            sessions: watch::channel(0).0,
        })))
    }

    pub fn auth(&self) -> &Auth {
        &self.0.auth
    }

    pub(crate) fn sessions_changed(&self) {
        self.0.sessions.send_modify(|v| *v += 1);
    }

    /// 在阻塞线程里使用数据库
    pub(crate) async fn db<T: Send + 'static>(&self, f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static) -> Result<T> {
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut s = inner.store.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut s)
        })
        .await
        .map_err(|e| BoardError::new(ErrorCode::Internal, e.to_string()))?
    }
}

/* ------------------------------ 错误 ------------------------------ */

pub(crate) struct ApiError(pub(crate) BoardError);

impl From<BoardError> for ApiError {
    fn from(e: BoardError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::Usage | ErrorCode::EmptyTitle | ErrorCode::NoBoard | ErrorCode::NoIdentity => StatusCode::BAD_REQUEST,
            ErrorCode::NotFound | ErrorCode::BoardNotFound => StatusCode::NOT_FOUND,
            ErrorCode::NotLoggedIn | ErrorCode::InvalidToken => StatusCode::UNAUTHORIZED,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::CONFLICT,
        };
        (status, Json(json!({ "error": self.0 }))).into_response()
    }
}

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

pub(crate) fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| BoardError::usage(format!("请求格式不正确：{e}")))
}

fn client_of(h: &HeaderMap) -> Option<String> {
    h.get("x-board-client").and_then(|v| v.to_str().ok()).map(|s| s.chars().take(64).collect())
}

/* ------------------------------ 路由 ------------------------------ */

pub fn router(state: AppState, cfg: &Config) -> Router {
    let api = Router::new()
        .route("/info", get(info))
        .route("/workspace", get(workspace))
        .route("/board", get(board))
        .route("/card", get(card))
        .route("/cmd", post(cmd))
        .route("/projects", post(create_project))
        .route("/projects/{key}", patch(update_project).delete(delete_project))
        .route("/boards", post(create_board))
        .route("/boards/{project}/{board}", patch(update_board).delete(delete_board))
        .route("/labels", post(create_label))
        .route("/labels/{id}", patch(update_label))
        .route("/labels/{project}/{id}", delete(delete_label))
        .route("/layout", put(layout))
        .route("/flow/{project}/{board}", post(create_status).put(reorder_statuses))
        .route("/flow/{project}/{board}/layout", put(flow_layout))
        .route("/flow/{project}/{board}/{key}", patch(update_status).delete(delete_status))
        .route("/stream", get(stream))
        .route("/me", get(auth::me))
        .route("/auth/logout", post(auth::logout))
        .route("/tokens", get(auth::tokens).post(auth::new_token))
        .route("/tokens/{id}", delete(auth::revoke))
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        .fallback(|| async { ApiError(BoardError::usage("未知的接口")) })
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth::guard));

    let mut app = Router::new()
        .nest("/api", api)
        .route("/auth/github", get(auth::login))
        .route("/auth/github/callback", get(auth::callback));
    if cfg.serve_web {
        app = app.fallback(get(static_file));
    }
    // 请求日志：方法、路径（不含查询参数）、状态、耗时、是谁。不记录令牌和 Cookie
    app.layer(
        TraceLayer::new_for_http()
            .make_span_with(|req: &Request<_>| {
                tracing::info_span!("http", method = %req.method(), path = %req.uri().path(), who = tracing::field::Empty)
            })
            .on_request(())
            .on_response(DefaultOnResponse::new().level(tracing::Level::INFO).latency_unit(tower_http::LatencyUnit::Millis)),
    )
    .with_state(state)
}

/// 启动后台任务（超时扫描、变更推送）并开始服务。不会返回，除非监听失败
pub async fn serve(state: AppState, cfg: Config, listener: std::net::TcpListener) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    spawn_background(state.clone());
    let app = router(state, &cfg);
    axum::serve(listener, app).await
}

pub fn bind(addr: SocketAddr) -> std::io::Result<std::net::TcpListener> {
    std::net::TcpListener::bind(addr)
}

/// 在后台线程里运行服务（测试用）。返回后即可连接
pub fn serve_in_thread(state: AppState, cfg: Config, listener: std::net::TcpListener) {
    std::thread::Builder::new()
        .name("board-server".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio");
            rt.block_on(async move {
                if let Err(e) = serve(state, cfg, listener).await {
                    eprintln!("Board 服务异常退出：{e}");
                }
            });
        })
        .expect("线程");
}

fn spawn_background(state: AppState) {
    // 规则 7：每秒释放一次到期的领取
    let s = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            match s.db(|st| st.sweep(now_ms())).await {
                Ok(n) if n > 0 => s.0.poke.notify_one(),
                Ok(_) => {}
                Err(e) => tracing::warn!("超时扫描失败：{e}"),
            }
        }
    });
    // 变更推送：rev 变化时把新事件广播给所有界面
    tokio::spawn(async move {
        let (mut rev, mut last) = state
            .db(|st| Ok((st.rev()?, st.read(|r| r.last_event_id())?)))
            .await
            .unwrap_or((0, 0));
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                _ = state.0.poke.notified() => {}
            }
            let r = state
                .db(move |st| {
                    let now = st.rev()?;
                    if now == rev {
                        return Ok(None);
                    }
                    let events = st.read(|r| r.events_since(last, 200))?;
                    Ok(Some((now, events)))
                })
                .await;
            match r {
                Ok(Some((now, events))) => {
                    rev = now;
                    if let Some(e) = events.last() {
                        last = e.id;
                    }
                    let msg = json!({ "rev": rev, "events": events }).to_string();
                    let _ = state.0.tx.send(Arc::from(msg));
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("读取变更失败：{e}"),
            }
        }
    });
}

/* ------------------------------ 读 ------------------------------ */

async fn info(State(s): State<AppState>) -> Json<Value> {
    Json(json!({ "name": "board", "version": env!("CARGO_PKG_VERSION"), "db": s.0.db.display().to_string() }))
}

async fn workspace(State(s): State<AppState>) -> ApiResult<Workspace> {
    Ok(Json(s.db(|st| st.read(|r| r.workspace(now_ms()))).await?))
}

#[derive(Deserialize)]
struct BoardQuery {
    path: String,
    #[serde(default)]
    id: Option<i64>,
}

async fn board(State(s): State<AppState>, Query(q): Query<BoardQuery>) -> ApiResult<BoardSnapshot> {
    Ok(Json(
        s.db(move |st| {
            st.read(|r| {
                let b = r.resolve(&q.path)?;
                r.board_snapshot(&b, now_ms())
            })
        })
        .await?,
    ))
}

async fn card(State(s): State<AppState>, Query(q): Query<BoardQuery>) -> ApiResult<CardDetail> {
    let id = q.id.ok_or_else(|| BoardError::usage("缺少 id"))?;
    Ok(Json(
        s.db(move |st| {
            st.read(|r| {
                let b = r.resolve(&q.path)?;
                r.card(&b, id)
            })
        })
        .await?,
    ))
}

/* ------------------------------ 卡片命令 ------------------------------ */

#[derive(Deserialize)]
struct CmdBody {
    board: String,
    /// agent 的名字（`--as` / `BOARD_AS`），只有带令牌的请求使用
    #[serde(default, rename = "as")]
    agent: Option<String>,
    #[serde(flatten)]
    cmd: Cmd,
}

async fn cmd(State(s): State<AppState>, Extension(caller): Extension<Caller>, headers: HeaderMap, Json(v): Json<Value>) -> ApiResult<CardDetail> {
    let body: CmdBody = parse(v)?;
    // 界面上是人；带令牌的是 agent，名字由 CLI 给出（PRD §3）
    let (actor, origin) = match caller {
        Caller::Human { .. } => (Actor::human(), Origin::ui(client_of(&headers))),
        Caller::Agent { token } => {
            let name: String = body.agent.as_deref().map(str::trim).unwrap_or_default().chars().take(64).collect();
            if name.is_empty() {
                return Err(BoardError::code(ErrorCode::NoIdentity).into());
            }
            tracing::Span::current().record("who", format!("{name}（令牌#{token}）"));
            (Actor::agent(name), Origin::cli())
        }
    };
    let out = s
        .db(move |st| {
            st.write(now_ms(), origin, |w| {
                let b = w.resolve(&body.board)?;
                w.exec(&b, &actor, body.cmd)
            })
        })
        .await?;
    s.0.poke.notify_one();
    Ok(Json(out))
}

/* ------------------------------ 项目 / 白板 / 标签 ------------------------------ */

async fn write<T: Send + 'static>(
    s: &AppState,
    headers: &HeaderMap,
    f: impl FnOnce(&mut W) -> Result<T> + Send + 'static,
) -> std::result::Result<T, ApiError> {
    let client = client_of(headers);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), f)).await?;
    s.0.poke.notify_one();
    Ok(out)
}

#[derive(Deserialize)]
struct NewBoardBody {
    name: String,
    key: String,
    #[serde(default)]
    desc: String,
    #[serde(default)]
    pos: Option<Pos>,
}

#[derive(Deserialize)]
struct NewProjectBody {
    name: String,
    key: String,
    #[serde(default)]
    desc: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    pos: Option<Pos>,
    board: NewBoardBody,
}

async fn create_project(State(s): State<AppState>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Value> {
    let b: NewProjectBody = parse(v)?;
    let key = write(&s, &h, move |w| {
        w.create_project(
            &b.name,
            &b.key,
            &b.desc,
            b.color.as_deref(),
            b.pos,
            NewBoard { name: b.board.name, key: b.board.key, desc: b.board.desc, pos: b.board.pos },
        )
    })
    .await?;
    Ok(Json(json!({ "key": key })))
}

#[derive(Deserialize)]
struct PatchBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    project: Option<String>,
}

async fn update_project(State(s): State<AppState>, h: HeaderMap, Path(key): Path<String>, Json(v): Json<Value>) -> ApiResult<Value> {
    let p: PatchBody = parse(v)?;
    write(&s, &h, move |w| w.update_project(&key, p.name.as_deref(), p.desc.as_deref(), p.color.as_deref())).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_project(State(s): State<AppState>, h: HeaderMap, Path(key): Path<String>) -> ApiResult<Value> {
    write(&s, &h, move |w| w.delete_project(&key)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct CreateBoardBody {
    project: String,
    #[serde(flatten)]
    board: NewBoardBody,
}

async fn create_board(State(s): State<AppState>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Value> {
    let b: CreateBoardBody = parse(v)?;
    let path = write(&s, &h, move |w| w.create_board(&b.project, &b.board.name, &b.board.key, &b.board.desc, b.board.pos)).await?;
    Ok(Json(json!({ "path": path })))
}

async fn update_board(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((project, board)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let p: PatchBody = parse(v)?;
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.update_board(&path, p.name.as_deref(), p.desc.as_deref())).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_board(State(s): State<AppState>, h: HeaderMap, Path((project, board)): Path<(String, String)>) -> ApiResult<Value> {
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.delete_board(&path)).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn create_label(State(s): State<AppState>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Label> {
    let p: PatchBody = parse(v)?;
    let project = p.project.ok_or_else(|| BoardError::usage("缺少 project"))?;
    let name = p.name.unwrap_or_default();
    let l = write(&s, &h, move |w| w.create_label(&project, &name, p.color.as_deref())).await?;
    Ok(Json(l))
}

async fn update_label(State(s): State<AppState>, h: HeaderMap, Path(id): Path<i64>, Json(v): Json<Value>) -> ApiResult<Value> {
    let p: PatchBody = parse(v)?;
    let project = p.project.ok_or_else(|| BoardError::usage("缺少 project"))?;
    write(&s, &h, move |w| w.update_label(&project, id, p.name.as_deref(), p.color.as_deref())).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_label(State(s): State<AppState>, h: HeaderMap, Path((project, id)): Path<(String, i64)>) -> ApiResult<Value> {
    write(&s, &h, move |w| w.delete_label(&project, id)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct LayoutBody {
    kind: String,
    id: String,
    x: f64,
    y: f64,
}

async fn layout(State(s): State<AppState>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Value> {
    let b: LayoutBody = parse(v)?;
    write(&s, &h, move |w| w.set_pos(&b.kind, &b.id, Pos { x: b.x, y: b.y })).await?;
    Ok(Json(json!({ "ok": true })))
}

/* ------------------------------ 工作流 ------------------------------ */

#[derive(Deserialize)]
struct NewStatusBody {
    name: String,
    #[serde(default)]
    color: Option<String>,
}

async fn create_status(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((project, board)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Status> {
    let b: NewStatusBody = parse(v)?;
    let path = format!("{project}/{board}");
    Ok(Json(write(&s, &h, move |w| w.create_status(&path, &b.name, b.color.as_deref())).await?))
}

#[derive(Deserialize)]
struct OrderBody {
    keys: Vec<String>,
}

async fn reorder_statuses(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((project, board)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let b: OrderBody = parse(v)?;
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.reorder_statuses(&path, &b.keys)).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn update_status(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((project, board, key)): Path<(String, String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let p: StatusPatch = parse(v)?;
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.update_status(&path, &key, p)).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_status(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((project, board, key)): Path<(String, String, String)>,
) -> ApiResult<Value> {
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.delete_status(&path, &key)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct FlowLayoutBody {
    /// 状态标识 → 位置；空对象表示复原默认布局
    tiles: std::collections::BTreeMap<String, Tile>,
}

async fn flow_layout(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((project, board)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let b: FlowLayoutBody = parse(v)?;
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.set_flow_layout(&path, &b.tiles)).await?;
    Ok(Json(json!({ "ok": true })))
}

/* ------------------------------ 实时推送 ------------------------------ */

async fn stream(State(s): State<AppState>, Extension(caller): Extension<Caller>) -> impl IntoResponse {
    let rx = s.0.tx.subscribe();
    let rev = s.db(|st| st.rev()).await.unwrap_or(0);
    let first = SseEvent::default().data(json!({ "rev": rev, "events": [] }).to_string());
    let session = match caller {
        Caller::Human { session, .. } => session,
        Caller::Agent { .. } => None,
    };
    let watch = s.0.sessions.subscribe();
    let rest = futures_util::stream::unfold((rx, watch, s, session), |(mut rx, mut watch, s, session)| async move {
        loop {
            tokio::select! {
                r = rx.recv() => {
                    let ev = match r {
                        Ok(msg) => SseEvent::default().data(&*msg),
                        // 落后太多：让界面整体刷新一次
                        Err(broadcast::error::RecvError::Lagged(_)) => SseEvent::default().data(json!({ "rev": -1, "events": [] }).to_string()),
                        Err(broadcast::error::RecvError::Closed) => return None,
                    };
                    return Some((Ok::<_, Infallible>(ev), (rx, watch, s, session)));
                }
                // 有人退出登录：自己的会话没了就断开（界面重连时会得到 401，回到登录页）
                changed = watch.changed(), if session.is_some() => {
                    if changed.is_err() {
                        return None;
                    }
                    let h = session.clone().unwrap_or_default();
                    if !s.db(move |st| auth::session_alive(st.conn(), &h, now_ms())).await.unwrap_or(false) {
                        return None;
                    }
                }
            }
        }
    });
    let stream = futures_util::StreamExt::chain(futures_util::stream::once(async move { Ok(first) }), rest);
    // X-Accel-Buffering：让 nginx 等反向代理不要缓冲推送
    (
        [("x-accel-buffering", "no")],
        Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))),
    )
}

/* ------------------------------ 界面 ------------------------------ */

async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (file, name) = match WebAssets::get(path) {
        Some(f) => (f, path),
        // 单页应用：其他路径都返回 index.html
        None => match WebAssets::get("index.html") {
            Some(f) => (f, "index.html"),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    "界面还没有构建：先在仓库根目录执行 npm run build:web，再重新编译 board-server",
                )
                    .into_response()
            }
        },
    };
    let cache = if name.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    (
        [(header::CONTENT_TYPE, file.metadata.mimetype().to_string()), (header::CACHE_CONTROL, cache.to_string())],
        file.data,
    )
        .into_response()
}
