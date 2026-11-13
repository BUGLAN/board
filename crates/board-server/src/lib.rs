//! Board 服务：界面和远程模式的 CLI 都通过它读写白板。
//!
//! - 身份（`auth`）：所有 `/api/*` 都要登录（GitHub 会话 Cookie）或带访问令牌，只有 `/api/health` 例外
//!   （它返回版本和提交号，部署后用来核对线上版本）；
//!   本机模式（`--local`）只接受本机访问，不需要登录
//! - `GET /api/workspace`、`/api/board`、`/api/card`：快照；`GET /api/events`：按游标向前翻页取动态
//! - `POST /api/cmd`：卡片命令（与 CLI 走同一个规则引擎）。带令牌的请求按 agent 执行，名字取请求里的 `as`，
//!   名字必须在线（先 `POST /api/agents/check`，之后每 10 秒 `POST /api/agents/beat`），否则 `NAME_OFFLINE`
//! - `POST /api/agents/check`、`/api/agents/beat`、`GET /api/agents`：agent 名字与 agent 级心跳（`agents`）
//! - `POST /api/files`、`GET /api/files/{id}`：卡片附件（上传、读取）
//! - `GET /api/me`、`POST /api/auth/logout`、`/api/tokens`：账号与访问令牌
//! - `/auth/github`、`/auth/github/callback`：GitHub 登录
//! - `GET /install.sh`、`/install.ps1`：命令行版 board 的安装脚本，不需要登录（`install`）
//! - `/api/skills`：skills 共用库（列出、取内容、上传、删除；`skills`）
//! - `/api/mcp`：MCP 服务器共用配置（列表打码、增删改只在界面上做；`/api/hosts/{名字}/mcp` 给 board host 原文，只给这台机器上报时用的令牌；`mcp`）
//! - `/api/providers`：模型供应商共用配置（列表打码、增删改只在界面上做；`/api/hosts/{名字}/providers` 给 board host 原文，只给这台机器上报时用的令牌；`providers`）
//! - 项目 / 白板 / 标签（增删改、拖拽排序 `PUT /api/labels/order`）/ 画布位置的增删改
//! - 工作流：`/api/flow/{项目}/{白板}`（新增状态、调整顺序）、`…/layout`（状态块布局）、`…/layout/saved`、`…/layout/apply`（白板设置：保存、应用布局）、`…/order`（白板设置：保存顺序）、`…/{状态}`（修改、删除）
//! - 任务下发（`hosts`）：board host 上报、开始 / 结束派发、上传日志；机器列表与设置；创建、取消派发
//! - `GET /api/stream`：SSE。任何一处（其他窗口、CLI、超时释放）写入后 ≤ 300ms 推送 `{rev, hosts, events}`；
//!   `hosts` 是机器和派发的版本，board host 看到它变了就重新取自己的任务
//!
//! 服务端每 250ms 检查一次数据库的 rev，发现变化就推送给已打开的界面（PRD §11）。
//! 服务端每秒扫描一次到期的领取（规则 7）和心跳中断的名字（PRD §3），没有命令执行时界面上也能看到超时释放、agent 下线。

pub mod agents;
pub mod auth;
pub mod hosts;
pub mod install;
pub mod mcp;
pub mod providers;
pub mod skills;
pub mod templates;

use auth::{Auth, Caller};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
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
use tower_http::trace::TraceLayer;

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

struct Inner {
    store: Mutex<Store>,
    tx: broadcast::Sender<Arc<str>>,
    poke: Notify,
    db: PathBuf,
    auth: Auth,
    /// 有会话被删除（退出登录）或令牌被吊销时加一：实时推送据此检查自己的登录态还在不在
    sessions: watch::Sender<u64>,
    /// 测试用：下一次 `/api/dispatches/{id}/end` 返回 502（Cargo feature `test-hooks`）
    #[cfg(feature = "test-hooks")]
    fail_end: std::sync::atomic::AtomicBool,
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
        // 服务端重启：停机时间不计入心跳超时，否则第一次扫描就会把所有领取释放（#95）
        store.resume_after_downtime(now)?;
        auth::setup(store.conn())?;
        let (tx, _) = broadcast::channel(256);
        Ok(AppState(Arc::new(Inner {
            store: Mutex::new(store),
            tx,
            poke: Notify::new(),
            db: cfg.db.clone(),
            auth: cfg.auth.clone(),
            sessions: watch::channel(0).0,
            #[cfg(feature = "test-hooks")]
            fail_end: std::sync::atomic::AtomicBool::new(false),
        })))
    }

    pub fn auth(&self) -> &Auth {
        &self.0.auth
    }

    /// 有写入：马上检查 rev，把变更推给界面（不用等下一次轮询）
    pub(crate) fn poke(&self) {
        self.0.poke.notify_one();
    }

    pub(crate) fn sessions_changed(&self) {
        self.0.sessions.send_modify(|v| *v += 1);
    }

    /// 测试用：让下一次 `POST /api/dispatches/{id}/end` 返回 502（只生效一次）
    #[cfg(feature = "test-hooks")]
    pub fn fail_next_end(&self) {
        self.0.fail_end.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// 测试用：取走一次 502 标记
    #[cfg(feature = "test-hooks")]
    pub(crate) fn take_fail_end(&self) -> bool {
        self.0.fail_end.swap(false, std::sync::atomic::Ordering::SeqCst)
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

/// 成功的心跳、日志上传不刷屏：给它们的响应打个标记，请求日志降到 DEBUG
#[derive(Clone, Copy)]
struct Quiet;

async fn quiet(mut res: Response) -> Response {
    res.extensions_mut().insert(Quiet);
    res
}

/// 成功的心跳、日志上传用 DEBUG，其他请求（含所有失败）用 INFO
fn response_level(response: &Response) -> tracing::Level {
    if response.status().is_success() && response.extensions().get::<Quiet>().is_some() {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // 500 的原因也要进日志；请求的 span 里带着方法、路径和 who
        if self.0.code == ErrorCode::Internal {
            tracing::error!(error = %self.0.message, code = self.0.code.as_str(), "请求失败：内部错误");
        }
        let status = match self.0.code {
            ErrorCode::Usage | ErrorCode::EmptyTitle | ErrorCode::NoBoard | ErrorCode::NoIdentity => StatusCode::BAD_REQUEST,
            ErrorCode::NotFound | ErrorCode::BoardNotFound | ErrorCode::NoSuchHost => StatusCode::NOT_FOUND,
            ErrorCode::NotLoggedIn | ErrorCode::InvalidToken => StatusCode::UNAUTHORIZED,
            ErrorCode::NotAuthor => StatusCode::FORBIDDEN,
            ErrorCode::InTrash => StatusCode::GONE,
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

pub(crate) fn client_of(h: &HeaderMap) -> Option<String> {
    h.get("x-board-client").and_then(|v| v.to_str().ok()).map(|s| s.chars().take(64).collect())
}

/* ------------------------------ 路由 ------------------------------ */

pub fn router(state: AppState, cfg: &Config) -> Router {
    let api = Router::new()
        .route("/info", get(info))
        .route("/workspace", get(workspace))
        .route("/board", get(board))
        .route("/card", get(card))
        .route("/trash", get(trash))
        .route("/events", get(events))
        .route("/stats", get(stats))
        .route("/search", get(search))
        .route("/milestones", get(milestones))
        .route("/releases", post(sync_release))
        .route("/cmd", post(cmd))
        .route("/agents", get(agents::list))
        .route("/agents/check", post(agents::check))
        .route("/agents/beat", post(agents::beat).layer(axum::middleware::map_response(quiet)))
        .route("/agents/next", post(agents::next))
        // 附件：上限之外留一点余量，略超的文件由 board-core 给出清楚的提示
        .route("/files", post(upload).layer(DefaultBodyLimit::max(files::MAX_BYTES + 1024 * 1024)))
        .route("/files/{id}", get(file))
        .route("/projects", post(create_project))
        .route("/projects/{key}", patch(update_project).delete(delete_project))
        .route("/boards", post(create_board))
        .route("/boards/{project}/{board}", patch(update_board).delete(delete_board))
        .route("/labels", post(create_label))
        .route("/labels/order", put(reorder_labels))
        .route("/labels/{id}", patch(update_label))
        .route("/labels/{project}/{id}", delete(delete_label))
        .route("/layout", put(layout))
        .route("/card-crons/{project}/{board}", post(create_card_cron))
        .route("/card-crons/{project}/{board}/{id}", patch(update_card_cron).delete(delete_card_cron))
        .route("/card-crons/{project}/{board}/{id}/run", post(run_card_cron))
        .route("/flow/{project}/{board}", post(create_status).put(reorder_statuses))
        .route("/flow/{project}/{board}/layout", put(flow_layout))
        .route("/flow/{project}/{board}/layout/saved", put(save_flow_layout))
        .route("/flow/{project}/{board}/layout/apply", post(apply_saved_layout))
        .route("/flow/{project}/{board}/order", put(keep_sorted))
        .route("/flow/{project}/{board}/edges", put(flow_edges))
        .route("/flow/{project}/{board}/{key}", patch(update_status).delete(delete_status))
        .route("/hosts", get(hosts::list))
        .route("/hosts/pause-all", post(hosts::pause_all))
        .route("/hosts/stop-all", post(hosts::stop_all))
        .route("/hosts/{name}", patch(hosts::update).delete(hosts::remove))
        .route("/hosts/{name}/rename", post(hosts::rename))
        .route("/hosts/{name}/ask", post(hosts::ask))
        .route("/hosts/{name}/selftest", post(hosts::selftest))
        .route("/hosts/{name}/selftested", post(hosts::selftest_done))
        .route("/hosts/{name}/skills/import", post(hosts::skills_import))
        .route("/hosts/{name}/skills/imported", post(hosts::skills_imported))
        .route("/hosts/{name}/copy", post(hosts::copy))
        .route("/hosts/{name}/dirs/clone", post(hosts::clone_dir))
        .route("/hosts/{name}/beat", post(hosts::beat).layer(axum::middleware::map_response(quiet)))
        .route("/hosts/{name}/work", get(hosts::work))
        .route("/hosts/{name}/begin", post(hosts::begin))
        .route("/hosts/{name}/pull", post(hosts::pull))
        .route("/hosts/{name}/round", post(hosts::round))
        .route("/hosts/{name}/recent", get(hosts::recent))
        .route("/dispatches", post(hosts::create).get(hosts::card_dispatches))
        .route("/sessions/{id}", get(hosts::session))
        .route("/dispatches/cancel", post(hosts::cancel))
        .route("/dispatches/{id}/end", post(hosts::end))
        .route("/dispatches/{id}/log", get(hosts::log).post(hosts::append_log).layer(axum::middleware::map_response(quiet)))
        .route("/dispatches/{id}/events", post(hosts::append_events).layer(axum::middleware::map_response(quiet)))
        .route("/stream", get(stream))
        .route("/me", get(auth::me))
        .route("/auth/logout", post(auth::logout))
        .route("/skills", get(skills::list).post(skills::put).layer(DefaultBodyLimit::max(12 * 1024 * 1024)))
        .route("/skills/{name}", get(skills::get).delete(skills::remove))
        .route("/mcp", get(mcp::list).post(mcp::put))
        .route("/mcp/{name}", delete(mcp::remove))
        .route("/providers", get(providers::list).post(providers::put))
        .route("/providers/agents", post(providers::assign))
        .route("/providers/models", post(providers::models))
        .route("/providers/{name}", delete(providers::remove))
        .route("/agent-templates", get(templates::list).post(templates::put))
        .route("/agent-templates/{id}", delete(templates::remove))
        .route("/hosts/{name}/mcp", get(mcp::for_host))
        .route("/hosts/{name}/providers", get(providers::for_host))
        .route("/tokens", get(auth::tokens).post(auth::new_token))
        .route("/tokens/{id}", delete(auth::revoke))
        .route("/health", get(health))
        .fallback(|| async { ApiError(BoardError::usage("未知的接口")) })
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth::guard));

    let mut app = Router::new()
        .nest("/api", api)
        .route("/auth/github", get(auth::login))
        .route("/auth/github/callback", get(auth::callback))
        .route("/auth/desktop", get(auth::desktop))
        // 命令行版 board 的安装脚本：新机器上还没有令牌，不需要登录
        .route("/install.sh", get(install::sh))
        .route("/install.ps1", get(install::ps1));
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
            // 成功的心跳、日志上传是 DEBUG；其他请求（含所有失败）是 INFO
            .on_response(|response: &Response, latency: Duration, _span: &tracing::Span| {
                if response_level(response) == tracing::Level::DEBUG {
                    tracing::debug!(status = response.status().as_u16(), ?latency, "finished processing request");
                } else {
                    tracing::info!(status = response.status().as_u16(), ?latency, "finished processing request");
                }
            }),
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
    // 规则 7：每秒释放一次到期的领取；每 10 分钟清理一次附件（没归到卡片的、不再被引用的文件），
    // 并把回收站里删了超过 30 天的卡片永久删除（附件一起删，文件由紧接着的附件清理去掉）
    let s = state.clone();
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        // 定时建卡（PRD §4）：每进入新的一分钟按这一分钟到点一次；从启动的那一分钟之后算起，停机期间错过的不补建。
        // 一次最多补 5 分钟（服务端卡住、睡眠唤醒后不一口气补很多次）
        let mut minute = now_ms().div_euclid(60_000);
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            match s.db(|st| st.sweep(now_ms())).await {
                Ok(n) if n > 0 => s.0.poke.notify_one(),
                Ok(_) => {}
                Err(e) => tracing::warn!("超时扫描失败：{e}"),
            }
            let cur = now_ms().div_euclid(60_000);
            if cur > minute {
                for m in (minute + 1).max(cur - 4)..=cur {
                    match s.db(move |st| st.write(now_ms(), Origin::system(), |w| w.card_crons_due(m * 60_000))).await {
                        Ok(n) if n > 0 => s.0.poke.notify_one(),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("定时建卡失败：{e}"),
                    }
                }
                minute = cur;
            }
            tick += 1;
            if tick % 600 == 0 {
                match s.db(|st| board_core::purge_trash(st, now_ms())).await {
                    Ok(n) if n > 0 => {
                        tracing::info!("回收站清理：永久删除了 {n} 张卡片");
                        s.0.poke.notify_one();
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("回收站清理失败：{e}"),
                }
                // 结束超过 30 天的派发只留结果事件和最后 50 条
                match s.db(|st| st.write(now_ms(), Origin::system(), |w| w.prune_run_events())).await {
                    Ok(n) if n > 0 => tracing::info!("清理派发事件：删了 {n} 条"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("清理派发事件失败：{e}"),
                }
                match s.db(|st| st.gc_files(now_ms())).await {
                    Ok((r, f)) if r + f > 0 => tracing::info!("清理附件：{r} 条记录，{f} 个文件"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("清理附件失败：{e}"),
                }
            }
        }
    });
    // 变更推送：rev 变化时把新事件广播给所有界面和 board host。
    // agent 的结构化事件（会话「过程」）不算变更：只有 runs 变了时推 { rev（不变）, runs, ran: [有新事件的派发] }，
    // 界面不重新取工作区，打开着这些派发的会话增量取
    tokio::spawn(async move {
        let (mut rev, mut last, mut runs) = state
            .db(|st| Ok((st.rev()?, st.read(|r| r.last_event_id())?, st.read(|r| r.runs_rev())?)))
            .await
            .unwrap_or((0, 0, 0));
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                _ = state.0.poke.notified() => {}
            }
            let r = state
                .db(move |st| {
                    let now = st.rev()?;
                    let now_runs = st.read(|r| r.runs_rev())?;
                    if now == rev && now_runs == runs {
                        return Ok(None);
                    }
                    let events = if now == rev { vec![] } else { st.read(|r| r.events_since(last, 200))? };
                    let ran = if now_runs == runs { vec![] } else { st.read(|r| r.dispatches_with_events_since(runs))? };
                    Ok(Some((now, st.hosts_rev()?, events, now_runs, ran)))
                })
                .await;
            match r {
                Ok(Some((now, hosts, events, now_runs, ran))) => {
                    rev = now;
                    runs = now_runs;
                    if let Some(e) = events.last() {
                        last = e.id;
                    }
                    let msg = json!({ "rev": rev, "hosts": hosts, "events": events, "runs": runs, "ran": ran }).to_string();
                    let _ = state.0.tx.send(Arc::from(msg));
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("读取变更失败：{e}"),
            }
        }
    });
}

/* ------------------------------ 读 ------------------------------ */

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
/// 编译时的提交号，见 build.rs
pub const COMMIT: &str = env!("BOARD_COMMIT");

async fn info(State(s): State<AppState>) -> Json<Value> {
    Json(json!({ "name": "board", "version": VERSION, "commit": COMMIT, "db": s.0.db.display().to_string() }))
}

/// `GET /api/health`：不需要登录。带上版本和提交号，部署后用来核对线上是不是这次的版本（GitHub Actions 部署）
async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "version": VERSION, "commit": COMMIT }))
}

async fn workspace(State(s): State<AppState>) -> ApiResult<Workspace> {
    Ok(Json(s.db(|st| st.read(|r| r.workspace(now_ms()))).await?))
}

#[derive(Deserialize)]
struct BoardQuery {
    path: String,
    #[serde(default)]
    id: Option<i64>,
    /// GET /api/board?full=1：终点状态也带全部卡片（默认只带最近的 FINAL_RECENT 张）
    #[serde(default)]
    full: Option<String>,
}

async fn board(State(s): State<AppState>, Query(q): Query<BoardQuery>) -> ApiResult<BoardSnapshot> {
    Ok(Json(
        s.db(move |st| {
            st.read(|r| {
                let b = r.resolve(&q.path)?;
                let full = matches!(q.full.as_deref(), Some("1" | "true"));
                r.board_snapshot(&b, now_ms(), full)
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

/// `GET /api/trash?path=<项目>/<白板>`：这块白板回收站里的卡片（按删除时间，最近删的在最后）
async fn trash(State(s): State<AppState>, Query(q): Query<BoardQuery>) -> ApiResult<Vec<board_core::TrashCard>> {
    Ok(Json(
        s.db(move |st| {
            st.read(|r| {
                let b = r.resolve(&q.path)?;
                r.trash(&b)
            })
        })
        .await?,
    ))
}

/// 动态一页默认多少条、最多多少条（界面滚到底时按 cursor 往回翻页）
const EVENTS_PAGE_DEFAULT: i64 = 20;
const EVENTS_PAGE_MAX: i64 = 100;

#[derive(Deserialize)]
struct EventsQuery {
    path: String,
    /// 上一页最后一条的 id；不传表示从最新一条开始。越界（比所有 id 都小）时返回空数组
    #[serde(default)]
    before: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
}

/// `GET /api/events?path=<项目>/<白板>&before=<id>&limit=<n>`：按 id 倒序分页取动态。
///
/// 新的在前；把这一页最后一条的 id 当 `before` 再请求就是更早的一页，两页之间不重不漏。
/// 没有更早的了就返回 `[]`，界面据此停止加载。
async fn events(State(s): State<AppState>, Query(q): Query<EventsQuery>) -> ApiResult<Vec<Event>> {
    let limit = q.limit.unwrap_or(EVENTS_PAGE_DEFAULT);
    if !(1..=EVENTS_PAGE_MAX).contains(&limit) {
        return Err(BoardError::usage(format!("limit 必须是 1 到 {EVENTS_PAGE_MAX} 之间的整数")).into());
    }
    Ok(Json(
        s.db(move |st| {
            st.read(|r| {
                let b = r.resolve(&q.path)?;
                r.events_page(&b, q.before, limit)
            })
        })
        .await?,
    ))
}

#[derive(Deserialize)]
struct StatsQueryRaw {
    scope: String,
    #[serde(default)]
    path: Option<String>,
    from: i64,
    to: i64,
    tz: String,
}

/// `GET /api/stats?scope=board|project|all&path=<项目/白板 或 项目>&from=<ms>&to=<ms>&tz=<IANA 时区>`：大盘的全部数据
async fn stats(State(s): State<AppState>, Query(q): Query<StatsQueryRaw>) -> ApiResult<board_core::stats::Stats> {
    use board_core::stats::{Scope, StatsQuery};
    let scope = Scope::parse(&q.scope).ok_or_else(|| BoardError::usage("scope 只能是 board、project 或 all"))?;
    if scope != Scope::All && q.path.as_deref().is_none_or(|p| p.trim().is_empty()) {
        return Err(BoardError::usage("缺少 path").into());
    }
    let q = StatsQuery { scope, path: q.path, from: q.from, to: q.to, tz: q.tz };
    Ok(Json(s.db(move |st| st.read(|r| r.stats(&q, now_ms()))).await?))
}

#[derive(Deserialize)]
struct MilestonesQuery {
    project: String,
}

/// `GET /api/milestones?project=<项目>`：里程碑页（PRD §10.15）——项目里的版本、带版本标签的卡片、未发版和没有版本的卡片
async fn milestones(State(s): State<AppState>, Query(q): Query<MilestonesQuery>) -> ApiResult<board_core::releases::Milestones> {
    Ok(Json(s.db(move |st| st.read(|r| r.milestones(q.project.trim()))).await?))
}

/// `POST /api/releases`：写入一个版本的记录（`board versions sync`，PRD §9）：
/// 版本标签不存在时新建，提交里连到的卡片带上这个标签，记录按项目和名称覆盖
async fn sync_release(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> ApiResult<board_core::releases::SyncResult> {
    let req: board_core::releases::ReleaseSync = parse(v)?;
    let origin = match caller {
        Caller::Human { .. } => Origin::ui(client_of(&headers)),
        Caller::Agent { .. } => Origin::cli(),
    };
    let out = s.db(move |st| st.write(now_ms(), origin, |w| w.sync_release(req))).await?;
    s.0.poke.notify_one();
    Ok(Json(out))
}

/* ------------------------------ 卡片命令 ------------------------------ */

const SEARCH_LIMIT_DEFAULT: usize = 20;

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /api/search?q=<文字>&limit=<n>`：跨白板搜卡片（PRD §9）。
///
/// `q` 可以是普通文字（标题、描述、评论），也可以是 `#12`、`oss/main#3`、`label:bug`。
/// 不需要白板：一次在所有白板（含「未整理」）里找，结果带上白板路径。最多 50 条。
async fn search(State(s): State<AppState>, Query(q): Query<SearchQuery>) -> ApiResult<Vec<board_core::SearchHit>> {
    let limit = q.limit.unwrap_or(SEARCH_LIMIT_DEFAULT);
    Ok(Json(s.db(move |st| st.read(|r| r.search(&q.q, limit))).await?))
}

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
            let name = agent_name(body.agent.as_deref())?;
            tracing::Span::current().record("who", format!("{name}（令牌#{token}）"));
            (Actor::agent(name), Origin::cli())
        }
    };
    let out = s
        .db(move |st| {
            st.write(now_ms(), origin, |w| {
                // 严格模式（PRD §3）：agent 的名字必须在线
                if actor.is_agent() {
                    w.read().require_online(&actor.name, w.now())?;
                }
                let b = w.resolve(&body.board)?;
                // 在回收站里的卡片：读不到（NOT_FOUND），但不是要恢复、永久删除、删除时，原因写成「在回收站里」
                if let Some(id) = card_id_of(&body.cmd) {
                    if !matches!(body.cmd, Cmd::Restore { .. } | Cmd::Purge { .. } | Cmd::Delete { .. })
                        && w.read().in_trash(&b, id)?
                    {
                        return Err(BoardError::new(ErrorCode::InTrash, format!("#{id} 在回收站里：先恢复，才能编辑、移动或评论"))
                            .with_detail(json!({ "card": id })));
                    }
                }
                w.exec(&b, &actor, body.cmd)
            })
        })
        .await?;
    s.0.poke.notify_one();
    Ok(Json(out))
}

/// 卡片命令里的卡片 id（新建、领取不带 id 时为 None）
fn card_id_of(cmd: &Cmd) -> Option<i64> {
    match cmd {
        Cmd::New { .. } | Cmd::Claim { id: None } => None,
        Cmd::Edit { id, .. }
        | Cmd::Move { id, .. }
        | Cmd::Comment { id, .. }
        | Cmd::Reply { id, .. }
        | Cmd::EditComment { id, .. }
        | Cmd::DeleteComment { id, .. }
        | Cmd::Label { id, .. }
        | Cmd::Beat { id, .. }
        | Cmd::Release { id }
        | Cmd::Delete { id }
        | Cmd::Restore { id }
        | Cmd::Purge { id }
        | Cmd::Merge { id, .. }
        | Cmd::Seen { id: Some(id) }
        | Cmd::Claim { id: Some(id) } => Some(*id),
        // 全部标为已读没有卡片 id
        Cmd::Seen { id: None } => None,
    }
}

/// 带令牌的请求里 agent 的名字：不能为空（是否在线在写库前检查）
fn agent_name(raw: Option<&str>) -> std::result::Result<String, ApiError> {
    let name = raw.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        return Err(BoardError::code(ErrorCode::NoIdentity).into());
    }
    Ok(name.to_string())
}

/* ------------------------------ 附件 ------------------------------ */

#[derive(Deserialize)]
struct UploadQuery {
    board: String,
    /// 不传：先不归到卡片（界面新建卡片时）
    #[serde(default)]
    card: Option<i64>,
    name: String,
    /// 带令牌时 agent 的名字
    #[serde(default, rename = "as")]
    agent: Option<String>,
}

/// `POST /api/files?board=<白板>&card=<ID>&name=<文件名>`：请求体就是文件内容
async fn upload(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    Query(q): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> ApiResult<Attachment> {
    let actor = match caller {
        Caller::Human { .. } => Actor::human(),
        Caller::Agent { token } => {
            let name = agent_name(q.agent.as_deref())?;
            tracing::Span::current().record("who", format!("{name}（令牌#{token}）"));
            Actor::agent(name)
        }
    };
    let out = s
        .db(move |st| {
            let now = now_ms();
            let b = st.read(|r| {
                // 严格模式（PRD §3）：agent 的名字必须在线
                if actor.is_agent() {
                    r.require_online(&actor.name, now)?;
                }
                r.resolve(&q.board)
            })?;
            st.attach(now, &b, q.card, &actor, &q.name, &body)
        })
        .await?;
    Ok(Json(out))
}

/// RFC 5987：Content-Disposition 里的 UTF-8 文件名
fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `GET /api/files/{id}`：图片直接显示，其他文件下载。内容不会变，可以长期缓存；
/// nosniff + sandbox，任何附件都不会被当成网页执行
async fn file(State(s): State<AppState>, Path(id): Path<i64>) -> std::result::Result<Response, ApiError> {
    let not_found = || ApiError(BoardError::new(ErrorCode::NotFound, format!("附件 {id} 不存在")));
    let (a, _, path) = s.db(move |st| st.attachment(id)).await?.ok_or_else(not_found)?;
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .ok()
        .and_then(|r| r.ok())
        .ok_or_else(not_found)?;
    let disposition = format!("{}; filename*=UTF-8''{}", if a.image { "inline" } else { "attachment" }, pct(&a.name));
    Ok((
        [
            (header::CONTENT_TYPE, a.mime),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, "private, max-age=31536000, immutable".into()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox".into()),
        ],
        bytes,
    )
        .into_response())
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
    /// 白板说明（Markdown）
    #[serde(default)]
    readme: Option<String>,
    /// 白板的环境检查：所有系统 / 只给 Windows 的一行命令（空字符串表示不检查）
    #[serde(default, rename = "readyCheck")]
    ready_check: Option<String>,
    #[serde(default, rename = "readyCheckWin")]
    ready_check_win: Option<String>,
    /// 白板的代码仓库地址、集成分支（空字符串表示清空）
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    project: Option<String>,
    /// 标签：仅人可打
    #[serde(default, rename = "humanOnly")]
    human_only: Option<bool>,
    /// 标签：优先（带这个标签的卡片先被领取）
    #[serde(default)]
    priority: Option<bool>,
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
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path((project, board)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let p: PatchBody = parse(v)?;
    // 代码仓库地址和集成分支决定 agent 往哪推、界面把提交号链到哪：只有人能改（PRD §4）
    if p.repo.is_some() || p.branch.is_some() {
        if let Caller::Agent { .. } = caller {
            return Err(ApiError(BoardError::code(ErrorCode::NotLoggedIn)));
        }
    }
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| {
        w.update_board(&path, p.name.as_deref(), p.desc.as_deref(), p.readme.as_deref())?;
        if p.ready_check.is_some() || p.ready_check_win.is_some() {
            w.set_ready_check(&path, p.ready_check.as_deref(), p.ready_check_win.as_deref())?;
        }
        if p.repo.is_some() || p.branch.is_some() {
            w.set_repo_branch(&path, p.repo.as_deref(), p.branch.as_deref())?;
        }
        Ok(())
    })
    .await?;
    Ok(Json(json!({ "ok": true })))
}

/* ---------------- 定时建卡（PRD §4「白板设置」）：只有人能改 ---------------- */

async fn create_card_cron(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path((project, board)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<board_core::card_crons::CardCron> {
    auth::human(&caller)?;
    let p: board_core::card_crons::CardCronPatch = parse(v)?;
    let path = format!("{project}/{board}");
    Ok(Json(write(&s, &h, move |w| w.card_cron_create(&path, &p)).await?))
}

async fn update_card_cron(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path((project, board, id)): Path<(String, String, i64)>,
    Json(v): Json<Value>,
) -> ApiResult<board_core::card_crons::CardCron> {
    auth::human(&caller)?;
    let p: board_core::card_crons::CardCronPatch = parse(v)?;
    let path = format!("{project}/{board}");
    Ok(Json(write(&s, &h, move |w| w.card_cron_update(&path, id, &p)).await?))
}

async fn delete_card_cron(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path((project, board, id)): Path<(String, String, i64)>,
) -> ApiResult<Value> {
    auth::human(&caller)?;
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.card_cron_delete(&path, id)).await?;
    Ok(Json(json!({ "ok": true })))
}

/// 「现在建一张」：同样受跳过规则约束，返回这条（带这一次的结果）
async fn run_card_cron(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path((project, board, id)): Path<(String, String, i64)>,
) -> ApiResult<board_core::card_crons::CardCron> {
    auth::human(&caller)?;
    let path = format!("{project}/{board}");
    Ok(Json(write(&s, &h, move |w| w.card_cron_run(&path, id)).await?))
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
    let l = write(&s, &h, move |w| w.create_label(&project, &name, p.color.as_deref(), p.human_only.unwrap_or(false))).await?;
    Ok(Json(l))
}

async fn update_label(State(s): State<AppState>, h: HeaderMap, Path(id): Path<i64>, Json(v): Json<Value>) -> ApiResult<Value> {
    let p: PatchBody = parse(v)?;
    let project = p.project.ok_or_else(|| BoardError::usage("缺少 project"))?;
    write(&s, &h, move |w| w.update_label(&project, id, p.name.as_deref(), p.color.as_deref(), p.human_only, p.priority)).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_label(State(s): State<AppState>, h: HeaderMap, Path((project, id)): Path<(String, i64)>) -> ApiResult<Value> {
    write(&s, &h, move |w| w.delete_label(&project, id)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct LabelOrderBody {
    project: String,
    /// 拖拽后的标签 id 顺序；没出现在里面的标签按原顺序补在后面
    ids: Vec<i64>,
}

/// `PUT /api/labels/order`：保存拖拽排序后的标签顺序。
///
/// ids 里的标签必须都属于这个项目、不能重复；任何一个不合法都整体拒绝、顺序不变。
async fn reorder_labels(State(s): State<AppState>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Value> {
    let b: LabelOrderBody = parse(v)?;
    write(&s, &h, move |w| w.reorder_labels(&b.project, &b.ids)).await?;
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

/// 白板设置「保存布局」：body 和复原 / 拖动布局一样是 {tiles}
async fn save_flow_layout(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((project, board)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let b: FlowLayoutBody = parse(v)?;
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.save_flow_layout(&path, &b.tiles)).await?;
    Ok(Json(json!({ "ok": true })))
}

/// 白板设置「应用布局」
async fn apply_saved_layout(State(s): State<AppState>, h: HeaderMap, Path((project, board)): Path<(String, String)>) -> ApiResult<Value> {
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.apply_saved_layout(&path)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct KeepSortedBody {
    #[serde(rename = "keepSorted")]
    keep_sorted: bool,
}

/// 白板设置「保留顺序」开关
async fn keep_sorted(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((project, board)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let b: KeepSortedBody = parse(v)?;
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.set_keep_sorted(&path, b.keep_sorted)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct EdgesBody {
    /// 要加上的连线 [起点, 终点]
    #[serde(default)]
    add: Vec<(String, String)>,
    /// 要去掉的连线
    #[serde(default)]
    remove: Vec<(String, String)>,
}

async fn flow_edges(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((project, board)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let b: EdgesBody = parse(v)?;
    let path = format!("{project}/{board}");
    write(&s, &h, move |w| w.update_edges(&path, &b.add, &b.remove)).await?;
    Ok(Json(json!({ "ok": true })))
}

/* ------------------------------ 实时推送 ------------------------------ */

async fn stream(State(s): State<AppState>, Extension(caller): Extension<Caller>) -> impl IntoResponse {
    let rx = s.0.tx.subscribe();
    let (rev, hosts) = s.db(|st| Ok((st.rev()?, st.hosts_rev()?))).await.unwrap_or((0, 0));
    let first = SseEvent::default().data(json!({ "rev": rev, "hosts": hosts, "events": [] }).to_string());
    let (session, token) = match caller {
        Caller::Human { session, .. } => (session, None),
        Caller::Agent { token } => (None, Some(token)),
    };
    let watch = s.0.sessions.subscribe();
    let rest = futures_util::stream::unfold((rx, watch, s, session, token), |(mut rx, mut watch, s, session, token)| async move {
        loop {
            let watching = session.is_some() || token.is_some();
            tokio::select! {
                r = rx.recv() => {
                    let ev = match r {
                        Ok(msg) => SseEvent::default().data(&*msg),
                        // 落后太多：让界面整体刷新一次
                        Err(broadcast::error::RecvError::Lagged(_)) => SseEvent::default().data(json!({ "rev": -1, "hosts": -1, "events": [] }).to_string()),
                        Err(broadcast::error::RecvError::Closed) => return None,
                    };
                    return Some((Ok::<_, Infallible>(ev), (rx, watch, s, session, token)));
                }
                // 有人退出登录或吊销令牌：自己的会话 / 令牌没了就断开（重连时会得到 401，回到登录页）
                changed = watch.changed(), if watching => {
                    if changed.is_err() {
                        return None;
                    }
                    let alive = if let Some(h) = session.clone() {
                        s.db(move |st| auth::session_alive(st.conn(), &h, now_ms())).await.unwrap_or(false)
                    } else if let Some(t) = token {
                        s.db(move |st| auth::token_alive(st.conn(), t)).await.unwrap_or(false)
                    } else {
                        true
                    };
                    if !alive {
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
    let mime = file.metadata.mimetype().to_string();
    let mut res = ([(header::CONTENT_TYPE, mime.clone()), (header::CACHE_CONTROL, cache.to_string())], file.data).into_response();
    if mime.starts_with("text/html") {
        res.headers_mut().insert(header::CONTENT_SECURITY_POLICY, header::HeaderValue::from_static(CSP));
    }
    res
}

/// 界面页面的内容安全策略（CSP）。卡片描述、评论来自 agent，Markdown 清理（web/src/lib/markdown.ts）之外的第二道屏障：
/// 就算漏进了脚本也执行不了，也不能把数据发到外站。
/// - script-src 只有同源文件：index.html 里不放内联脚本（首帧主题在 web/public/theme.js）
/// - style-src 允许内联：React 的 style 属性、index.html 首帧底色
/// - img-src：附件、data:/blob:（粘贴截图的预览）、GitHub 头像
/// - connect-src：同源接口和推送；ipc: / http://ipc.localhost 是桌面端（Tauri）调用本机命令的通道
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data: blob: https://avatars.githubusercontent.com; font-src 'self' data:; \
    connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

#[cfg(test)]
mod tests {
    use super::{response_level, ApiError, Quiet, CSP};
    use axum::body::Body;
    use axum::response::IntoResponse;
    use board_core::{BoardError, ErrorCode};

    /// 测试用的日志收集器：把 tracing 的输出写进内存
    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    struct SinkWriter(Sink);

    impl std::io::Write for SinkWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn capture(f: impl FnOnce()) -> String {
        let sink = Sink::default();
        let sub = tracing_subscriber::fmt()
            .with_writer({
                let sink = sink.clone();
                move || SinkWriter(sink.clone())
            })
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(sub, f);
        let out = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        out
    }

    #[test]
    fn internal_errors_are_logged_with_reason_path_and_caller() {
        let text = capture(|| {
            let span = tracing::info_span!("http", method = "POST", path = "/api/files", who = "claude/opus-5.5");
            let _guard = span.enter();
            let res = ApiError(BoardError::new(ErrorCode::Internal, "文件错误：磁盘已满")).into_response();
            assert_eq!(res.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        });
        assert!(text.contains("ERROR"), "没有 ERROR：{text}");
        assert!(text.contains("文件错误：磁盘已满"), "没有原因：{text}");
        assert!(text.contains("path=\"/api/files\"") || text.contains("path=/api/files"), "没有路径：{text}");
        assert!(
            text.contains("who=\"claude/opus-5.5\"") || text.contains("who=claude/opus-5.5"),
            "没有调用者：{text}"
        );
        // 普通错误（例如参数不对）不写 ERROR，只有响应体
        let text = capture(|| {
            let _ = ApiError(BoardError::usage("参数不对")).into_response();
        });
        assert!(!text.contains("ERROR"), "普通错误不该写 ERROR：{text}");
    }

    #[test]
    fn quiet_success_is_debug_but_quiet_failures_stay_info() {
        let mut ok = axum::response::Response::new(Body::empty());
        ok.extensions_mut().insert(Quiet);
        assert_eq!(response_level(&ok), tracing::Level::DEBUG);
        let mut failed = axum::response::Response::new(Body::empty());
        failed.extensions_mut().insert(Quiet);
        *failed.status_mut() = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        assert_eq!(response_level(&failed), tracing::Level::INFO, "失败的心跳仍要记 INFO");
        let plain = axum::response::Response::new(Body::empty());
        assert_eq!(response_level(&plain), tracing::Level::INFO, "其他请求保持 INFO");
    }

    /// 页面的 CSP：脚本只能来自同源文件，不能 eval、不能内联；图片和请求不能发到外站（GitHub 头像、桌面端 IPC 除外）
    #[test]
    fn csp_directives() {
        let d: Vec<(&str, Vec<&str>)> = CSP
            .split(';')
            .map(|x| {
                let mut it = x.split_whitespace();
                (it.next().unwrap_or_default(), it.collect())
            })
            .collect();
        let get = |k: &str| d.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone()).unwrap_or_else(|| panic!("缺少 {k}"));
        assert_eq!(get("default-src"), ["'self'"]);
        assert_eq!(get("script-src"), ["'self'"]);
        assert_eq!(get("img-src"), ["'self'", "data:", "blob:", "https://avatars.githubusercontent.com"]);
        assert_eq!(get("connect-src"), ["'self'", "ipc:", "http://ipc.localhost"]);
        assert_eq!(get("object-src"), ["'none'"]);
        assert_eq!(get("base-uri"), ["'none'"]);
        assert_eq!(get("frame-ancestors"), ["'none'"]);
        assert!(!CSP.contains("unsafe-eval"));
        // index.html 不能有内联脚本（script-src 不允许），首帧主题在 public/theme.js
        let html = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/index.html")).unwrap();
        for tag in html.split("<script").skip(1) {
            let head = &tag[..tag.find('>').unwrap()];
            assert!(head.contains("src="), "index.html 里有内联脚本：<script{head}>");
        }
    }
}
