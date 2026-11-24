//! 任务下发接口（任务下发需求 §十）。规则在 board-core 的 `hosts` 里。
//!
//! board host 用访问令牌调用：
//! - `POST /api/hosts/{名字}/beat`：上报（启动时、之后每 10 秒），返回这台机器的任务
//! - `GET /api/hosts/{名字}/work`：只取任务（收到实时推送里 `hosts` 变化时）
//! - `POST /api/hosts/{名字}/begin {instance, id}`：开始一次排队中的派发（领取卡片）
//! - `POST /api/hosts/{名字}/pull {instance}`：自动领取的下一张；现在没有可领的、或机器不在自动接单时返回 null
//! - `POST /api/hosts/{名字}/round`：开始一轮领取（定时到点；界面上的「现在领一轮」也是它）
//! - `POST /api/hosts/{名字}/rename {name}`：机器改名（桌面端「设置 → 本机」保存机器名时调用）
//! - `POST /api/hosts/{名字}/synced {instance, result}`：同步配置做完（启动时写的一次、收到请求写的一次），回报结果
//! - `POST /api/hosts/{名字}/ls/{ID} {path, chain, entries, error?}`：列目录的结果
//! - `POST /api/dispatches/{ID}/end`、`POST /api/dispatches/{ID}/log`：派发结束、上传日志
//! - `POST /api/dispatches/{ID}/events {fmt, events}`：上传结构化事件（会话「过程」，见 board-core runlog.rs）
//!
//! 用令牌调用时，机器得归这个令牌（它上报时用的就是这个令牌），派发得在这样的机器上，否则 NO_SUCH_HOST / NOT_FOUND
//! （board-core 的 `R::host_owned_by`、`R::dispatch_owned_by`，和写操作在同一个事务里查）；上报本身不能接管绑着别的
//! 有效令牌的机器（`W::host_beat`）。登录的界面、本机模式不受限。
//!
//! 界面（登录）：`GET /api/hosts`、`PATCH|DELETE /api/hosts/{名字}`（`DELETE …?revoke=1` 同时吊销它用的令牌）、
//! `POST /api/hosts/{名字}/ask {kind, now}`（远程管理：重新检查、重启、升级、排空、取消）、
//! `POST /api/hosts/{名字}/sync`（同步配置写入失败后「重试」：重新下发同步请求）、
//! `POST /api/hosts/{名字}/ls {path}`（列那台机器上一个文件夹的子文件夹：服务端把请求放进它的任务，等 board host 回传，最多 20 秒）、
//! `POST /api/hosts/{名字}/selftest {agent?}`（自检）、`POST /api/hosts/{名字}/selftested {instance, id, items}`（board host 回报自检结果）、`GET /api/hosts/{名字}/recent`、
//! `GET /api/dispatches/{ID}/log`、`GET /api/dispatches?board=<白板>&card=<ID>`（一张卡的全部派发）、
//! `GET /api/sessions/{派发 ID}[?since=<runs>]`（一个会话的全部派发、输出和事件；带 since 时事件只取之后新增、改过的）。这些用令牌调用时 NOT_LOGGED_IN。
//! 登录或令牌：`POST /api/dispatches`（派发；不带 `host` 是任意机器）、`POST /api/dispatches/move`（改派排队中的派发）、
//! `POST /api/dispatches/cancel`（取消排队 / 停止在跑）。

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
#[cfg(feature = "test-hooks")]
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use board_core::hosts::EndHow;
use board_core::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{human, Caller};
use crate::{client_of, parse, ApiError, AppState};

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

fn token_of(c: &Caller) -> Option<i64> {
    match c {
        Caller::Agent { token } => Some(*token),
        Caller::Human { .. } => None,
    }
}

fn origin_of(c: &Caller, h: &HeaderMap) -> Origin {
    match c {
        Caller::Human { .. } => Origin::ui(client_of(h)),
        Caller::Agent { .. } => Origin::cli(),
    }
}

/// 发起人：人（动态里写「人」，停止评论里写登录名）；令牌带名字时是那个 agent，不带时写令牌的名称
async fn who(s: &AppState, c: &Caller, name: Option<&str>) -> Result<(Actor, String)> {
    match c {
        Caller::Human { login, .. } => Ok((Actor::human(), login.clone().unwrap_or_else(|| "人".into()))),
        Caller::Agent { token } => {
            if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
                return Ok((Actor::agent(n), n.to_string()));
            }
            let id = *token;
            let t = s.db(move |st| crate::auth::token_name(st.conn(), id)).await?;
            let n = format!("命令行（令牌「{}」）", t.unwrap_or_else(|| format!("#{id}")));
            Ok((Actor::agent(n.clone()), n))
        }
    }
}

/* ------------------------------ board host ------------------------------ */

pub(crate) async fn beat(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<HostWork> {
    let mut rep: HostReport = parse(v)?;
    rep.name = name;
    let token = token_of(&caller);
    tracing::Span::current().record("who", format!("{}（机器上报）", rep.name));
    let out = s.db(move |st| st.write(now_ms(), Origin::cli(), |w| w.host_beat(&rep, token))).await?;
    s.poke();
    Ok(Json(finish(&s, out)))
}

/// 任务里的 skills 库补上随服务端版本的 board-skills（MCP 的索引 board-core 已经带上），再补上内存里等它列的目录
fn finish(s: &AppState, mut w: HostWork) -> HostWork {
    w.skills = w.skills.take().map(crate::skills::index);
    w.ls = s.ls_asks(&w.host.name);
    w
}

pub(crate) async fn work(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(name): Path<String>) -> ApiResult<HostWork> {
    let token = token_of(&caller);
    let out = s
        .db(move |st| {
            st.read(|r| {
                r.host_owned_by(&name, token)?;
                r.host_work(&name, now_ms())
            })
        })
        .await?;
    Ok(Json(finish(&s, out)))
}

/* ------------------------------ 同步配置、列目录 ------------------------------ */

#[derive(Deserialize)]
pub(crate) struct SyncedBody {
    instance: String,
    result: SyncResult,
}

/// board host 回报一次同步配置的结果
pub(crate) async fn synced(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(name): Path<String>, Json(v): Json<Value>) -> ApiResult<HostInfo> {
    let b: SyncedBody = parse(v)?;
    let token = token_of(&caller);
    let out = s
        .db(move |st| {
            st.write(now_ms(), Origin::cli(), |w| {
                w.read().host_owned_by(&name, token)?;
                w.host_synced(&name, &b.instance, b.result)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct CopyBody {
    from: String,
    #[serde(default)]
    dry: bool,
}

/// 照另一台机器配（hostcopy.rs）：`dry` 只列出要变哪些，确认后再不带 `dry` 调一次写进去。只能在登录后的界面上做
pub(crate) async fn copy(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<board_core::hostcopy::HostCopy> {
    let by = crate::auth::human(&caller).map_err(ApiError)?;
    let b: CopyBody = parse(v)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.host_copy(&name, &b.from, &by, b.dry))).await?;
    if out.done {
        s.poke();
    }
    Ok(Json(out))
}

/// 界面上「重试」：重新下发同步请求
pub(crate) async fn sync(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Path(name): Path<String>) -> ApiResult<HostInfo> {
    let by = crate::auth::human(&caller).map_err(ApiError)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.host_sync_retry(&name, &by))).await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct LsBody {
    #[serde(default)]
    path: String,
}

/// 等 board host 回传列目录的结果最多多久
const LS_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// 界面选工作空间：列那台机器上一个文件夹的子文件夹。请求放在内存里，让它的 board host 重新取任务，等它回传
pub(crate) async fn list_dir(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(name): Path<String>, Json(v): Json<Value>) -> ApiResult<DirList> {
    crate::auth::human(&caller).map_err(ApiError)?;
    let b: LsBody = parse(v)?;
    let host = name.clone();
    let h = s.db(move |st| st.read(|r| r.host(&host, now_ms()))).await?;
    if !h.online {
        return Err(ApiError(BoardError::usage(format!("{name} 离线：上线后才能看它的文件夹"))));
    }
    if !h.sync.able {
        return Err(ApiError(BoardError::usage(format!("{name} 的 board 是旧版（{}），不会列文件夹：升级后再选，或者直接填路径", h.version))));
    }
    let (id, rx) = s.ls_ask(&name, b.path.trim());
    s.db(|st| {
        st.write(now_ms(), Origin::system(), |w| {
            w.hosts_poke();
            Ok(())
        })
    })
    .await?;
    s.poke();
    match tokio::time::timeout(LS_WAIT, rx).await {
        Ok(Ok(list)) => Ok(Json(list)),
        _ => {
            s.ls_drop(id);
            Err(ApiError(BoardError::new(ErrorCode::Unreachable, format!("{name} 的 board host 没回话：稍后再试"))))
        }
    }
}

/// board host 回传列目录的结果
pub(crate) async fn listed(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path((name, id)): Path<(String, i64)>, Json(v): Json<Value>) -> ApiResult<Value> {
    let list: DirList = parse(v)?;
    let token = token_of(&caller);
    let host = name.clone();
    s.db(move |st| st.read(|r| r.host_owned_by(&host, token))).await?;
    s.ls_done(&name, id, list);
    Ok(Json(json!({})))
}

/// `GET /api/hosts/mine`：这个令牌是哪台在线机器的 board host 用的那个（`board check --agent <标识>` 在旧服务端上
/// 自己拼名字时用它）；没有时返回 null。本机模式（不用令牌）返回这台机器的记录。登录的界面也是 null
pub(crate) async fn mine(State(s): State<AppState>, Extension(caller): Extension<Caller>) -> ApiResult<Option<HostInfo>> {
    let token = token_of(&caller);
    let out = s.db(move |st| st.read(|r| r.my_host(token, now_ms()))).await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct BeginBody {
    instance: String,
    #[serde(default)]
    id: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct PullBody {
    instance: String,
}

pub(crate) async fn begin(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Dispatch> {
    let b: BeginBody = parse(v)?;
    let id = b.id.ok_or_else(|| BoardError::usage("缺少 id"))?;
    let token = token_of(&caller);
    let out = s
        .db(move |st| {
            st.write(now_ms(), Origin::cli(), |w| {
                w.read().host_owned_by_proc(&name, &b.instance, token)?;
                w.dispatch_begin(&name, &b.instance, id, token)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn pull(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Option<Dispatch>> {
    let b: PullBody = parse(v)?;
    let token = token_of(&caller);
    let out = s
        .db(move |st| {
            st.write(now_ms(), Origin::cli(), |w| {
                w.read().host_owned_by_proc(&name, &b.instance, token)?;
                w.dispatch_pull(&name, &b.instance, token)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn round(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Path(name): Path<String>) -> ApiResult<HostInfo> {
    let origin = origin_of(&caller, &h);
    let token = token_of(&caller);
    let out = s
        .db(move |st| {
            st.write(now_ms(), origin, |w| {
                w.read().host_owned_by(&name, token)?;
                w.host_round(&name)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EndBody {
    #[serde(default)]
    exit_code: Option<i64>,
    #[serde(default)]
    tail: Vec<String>,
    #[serde(default = "exit")]
    how: EndHow,
    /// how = timeout 时给人看的原因（「运行超过 3 小时…」）
    #[serde(default)]
    why: Option<String>,
    /// 最后 10 个事件的文字（异常退出评论里用）；旧版 board host 没有
    #[serde(default)]
    events: Vec<String>,
}

fn exit() -> EndHow {
    EndHow::Exit
}

pub(crate) async fn end(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(id): Path<i64>, Json(v): Json<Value>) -> Response {
    // 集成测试用：模拟「服务端已经收到、响应丢了」之前的失败响应
    #[cfg(feature = "test-hooks")]
    if s.take_fail_end() {
        return (StatusCode::BAD_GATEWAY, "bad gateway").into_response();
    }
    match end_inner(s, token_of(&caller), id, v).await {
        Ok(j) => j.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn end_inner(s: AppState, token: Option<i64>, id: i64, v: Value) -> ApiResult<Dispatch> {
    let b: EndBody = parse(v)?;
    let out = s
        .db(move |st| {
            st.write(now_ms(), Origin::cli(), |w| {
                w.read().dispatch_owned_by(id, token)?;
                w.dispatch_end_with(id, b.exit_code, &b.tail, &b.events, b.how, b.why.as_deref())
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

/// `POST /api/dispatches/{ID}/wip {branch, how, changes, tail}`：board host 在派发结束时把没推的改动保存成了 wip 分支，
/// 在卡片上评论（半成品交接，PRD §9.1）
pub(crate) async fn wip(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(id): Path<i64>, Json(v): Json<Value>) -> ApiResult<Value> {
    #[derive(Deserialize)]
    struct Body {
        branch: String,
        #[serde(default)]
        how: String,
        #[serde(default)]
        changes: String,
        #[serde(default)]
        tail: Vec<String>,
    }
    let b: Body = parse(v)?;
    let token = token_of(&caller);
    s.db(move |st| {
        st.write(now_ms(), Origin::cli(), |w| {
            w.read().dispatch_owned_by(id, token)?;
            w.dispatch_wip(id, &b.branch, &b.how, &b.changes, &b.tail)
        })
    })
    .await?;
    s.poke();
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct LogBody {
    #[serde(default)]
    lines: Vec<String>,
}

pub(crate) async fn append_log(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(id): Path<i64>, Json(v): Json<Value>) -> ApiResult<Value> {
    let b: LogBody = parse(v)?;
    let token = token_of(&caller);
    s.db(move |st| {
        st.write(now_ms(), Origin::cli(), |w| {
            w.read().dispatch_owned_by(id, token)?;
            w.dispatch_log_append(id, &b.lines)
        })
    })
    .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct EventsBody {
    /// agent 的输出格式（claude、codex、dsh）；空字符串 = 只有原始输出
    #[serde(default)]
    fmt: Option<String>,
    #[serde(default)]
    events: Vec<RunEvent>,
}

/// 一次最多收多少条（host 每 2 秒传一次，正常远到不了）
const EVENTS_BATCH: usize = 2000;

pub(crate) async fn append_events(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(id): Path<i64>, Json(v): Json<Value>) -> ApiResult<Value> {
    let mut b: EventsBody = parse(v)?;
    if b.events.len() > EVENTS_BATCH {
        b.events.drain(..b.events.len() - EVENTS_BATCH);
    }
    let token = token_of(&caller);
    let n = s
        .db(move |st| {
            st.write(now_ms(), Origin::cli(), |w| {
                w.read().dispatch_owned_by(id, token)?;
                w.dispatch_events_append(id, b.fmt.as_deref(), &b.events)
            })
        })
        .await?;
    if n > 0 {
        s.poke();
    }
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn log(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(id): Path<i64>) -> ApiResult<Value> {
    let token = token_of(&caller);
    let lines = s
        .db(move |st| {
            st.read(|r| {
                r.dispatch_owned_by(id, token)?;
                r.dispatch_log(id)
            })
        })
        .await?;
    Ok(Json(json!({ "lines": lines })))
}

/* ------------------------------ 界面 ------------------------------ */

#[derive(Deserialize)]
pub(crate) struct CardDispatchesQuery {
    /// 白板路径
    board: String,
    card: i64,
}

/// 一张卡片的全部派发，新的在前（卡片详情「派发记录」）
pub(crate) async fn card_dispatches(State(s): State<AppState>, Query(q): Query<CardDispatchesQuery>) -> ApiResult<Vec<Dispatch>> {
    Ok(Json(
        s.db(move |st| {
            st.read(|r| {
                let b = r.resolve(&q.board)?;
                r.card_dispatches(&b, q.card)
            })
        })
        .await?,
    ))
}

#[derive(Deserialize)]
pub(crate) struct SessionQuery {
    /// 上次取到的事件版本（`runs`）：只取之后新增、改过的事件
    #[serde(default)]
    since: i64,
}

/// 一个会话的全部派发和各自的输出、事件（「会话」弹窗）；id 是会话里任一次派发
pub(crate) async fn session(State(s): State<AppState>, Path(id): Path<i64>, Query(q): Query<SessionQuery>) -> ApiResult<SessionView> {
    Ok(Json(s.db(move |st| st.read(|r| r.session_since(id, q.since.max(0)))).await?))
}

pub(crate) async fn list(State(s): State<AppState>) -> ApiResult<Value> {
    let (hosts, dispatches) = s
        .db(|st| {
            st.read(|r| {
                let now = now_ms();
                Ok((r.hosts(now)?, r.active_dispatches()?))
            })
        })
        .await?;
    Ok(Json(json!({ "hosts": hosts, "dispatches": dispatches })))
}

pub(crate) async fn update(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<HostInfo> {
    let p: HostPatch = parse(v)?;
    // 旧版界面提交的是「白板与目录」（`dirs`）：请求头里带着工作空间时按新模型收
    let mut p = p;
    if p.workspace.is_none() {
        if let Some(ws) = h.get("x-board-workspace").and_then(|v| v.to_str().ok()).map(str::trim).filter(|w| !w.is_empty()) {
            p.workspace = Some(HostWorkspace { path: ws.to_string(), ..Default::default() });
        }
    }
    let client = client_of(&h);
    let by = human(&caller).map_err(ApiError)?;
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.host_update_as(&name, &p, &by))).await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct CloneDirBody {
    board: String,
    url: String,
}

/// 在那台机器上克隆一块白板的目录。访问令牌不能调用（守卫只放行界面登录）
pub(crate) async fn clone_dir(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<HostInfo> {
    let b: CloneDirBody = parse(v)?;
    let client = client_of(&h);
    let by = human(&caller).map_err(ApiError)?;
    let out = s
        .db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.host_clone_request(&name, &b.board, &b.url, &by)))
        .await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize, Default)]
pub(crate) struct RemoveQuery {
    /// 1：同时吊销这台机器用的令牌
    #[serde(default)]
    revoke: Option<String>,
}

pub(crate) async fn remove(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<RemoveQuery>,
) -> ApiResult<Value> {
    let revoke = q.revoke.as_deref().is_some_and(|v| v == "1" || v == "true");
    if revoke && matches!(caller, Caller::Agent { .. }) {
        return Err(ApiError(BoardError::code(ErrorCode::NotLoggedIn)));
    }
    let (actor, by) = who(&s, &caller, None).await?;
    let client = client_of(&h);
    let revoked = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.host_remove_revoke(&name, &actor, &by, revoke))).await?;
    s.poke();
    if revoked.is_some() {
        // 用这个令牌打开的实时推送随之关闭
        s.sessions_changed();
    }
    Ok(Json(json!({ "ok": true, "revoked": revoked })))
}

#[derive(Deserialize)]
pub(crate) struct AskBody {
    /// recheck | restart | upgrade | drain | cancel | agent-install | agent-update | rescan-skills
    kind: String,
    /// 升级：不等在跑的派发结束
    #[serde(default)]
    now: bool,
    /// agent-install / agent-update：哪个 agent
    #[serde(default)]
    agent: Option<String>,
    /// agent-install / agent-update：目标版本
    #[serde(default)]
    to: Option<String>,
}

/// 远程管理一台机器（重新检查、重启、升级、排空、取消请求）。只能在登录后的界面上做，访问令牌调用时 NOT_LOGGED_IN
pub(crate) async fn ask(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<HostInfo> {
    let b: AskBody = parse(v)?;
    let client = client_of(&h);
    let by = human(&caller).map_err(ApiError)?;
    let out = s.db(move |st| {
        st.write(now_ms(), Origin::ui(client), |w| match (&*b.kind, b.agent.as_deref()) {
            ("agent-install" | "agent-update", Some(agent)) => w.host_agent_ask(&name, &b.kind, agent, b.to.as_deref(), &by),
            ("agent-install" | "agent-update", None) => Err(board_core::BoardError::usage("agent 操作要写是哪个 agent")),
            _ => w.host_ask(&name, &b.kind, b.now, &by),
        })
    })
    .await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct SelfTestBody {
    /// 只检查这个 agent；不给是整台机器
    #[serde(default)]
    agent: Option<String>,
}

/// 自检一台机器（PRD §10.12「自检」）。只能在登录后的界面上做，访问令牌调用时 NOT_LOGGED_IN
pub(crate) async fn selftest(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<HostInfo> {
    let b: SelfTestBody = parse(v)?;
    let client = client_of(&h);
    let by = human(&caller).map_err(ApiError)?;
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.host_selftest(&name, b.agent.as_deref(), &by))).await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct SelfTestDoneBody {
    instance: String,
    id: i64,
    #[serde(default)]
    items: Vec<SelfTestItem>,
}

/// board host 回报一次自检的结果（只收这台机器的令牌）
pub(crate) async fn selftest_done(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(name): Path<String>, Json(v): Json<Value>) -> ApiResult<HostInfo> {
    let b: SelfTestDoneBody = parse(v)?;
    let token = token_of(&caller);
    let out = s
        .db(move |st| {
            st.write(now_ms(), Origin::cli(), |w| {
                w.read().host_owned_by(&name, token)?;
                w.host_selftest_done(&name, &b.instance, b.id, b.items)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct SkillImportBody {
    #[serde(default)]
    picks: Vec<SkillImportPick>,
}

/// 从这台机器导入 skills（只能在登录后的界面上做）：记下要导入的，board host 取到任务时传进共用库
pub(crate) async fn skills_import(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<HostInfo> {
    let b: SkillImportBody = parse(v)?;
    let client = client_of(&h);
    let by = human(&caller).map_err(ApiError)?;
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.host_skill_import(&name, b.picks, &by))).await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct SkillImportedBody {
    instance: String,
    id: i64,
    #[serde(default)]
    ok: Vec<String>,
    #[serde(default)]
    failed: Vec<SkillImportFail>,
}

/// board host 做完了导入，回报结果
pub(crate) async fn skills_imported(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(name): Path<String>, Json(v): Json<Value>) -> ApiResult<HostInfo> {
    let b: SkillImportedBody = parse(v)?;
    let token = token_of(&caller);
    let out = s
        .db(move |st| {
            st.write(now_ms(), Origin::cli(), |w| {
                w.read().host_owned_by(&name, token)?;
                w.host_skill_imported(&name, &b.instance, b.id, b.ok, b.failed)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

/// 所有机器一起暂停接单（`paused: false` 是全部恢复）
pub(crate) async fn pause_all(State(s): State<AppState>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Vec<HostInfo>> {
    #[derive(Deserialize)]
    struct Body {
        paused: bool,
    }
    let b: Body = parse(v)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.hosts_pause_all(b.paused))).await?;
    s.poke();
    Ok(Json(out))
}

/// `PATCH /api/sched {rules?, fallback?, autoUpgrade?}`：改全局的领取规则、兜底和自动升级的开关（只有人能改；访问令牌调不到这个接口）
pub(crate) async fn sched_update(State(s): State<AppState>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<board_core::sched::Sched> {
    #[derive(Deserialize)]
    struct Body {
        /// 领取规则（整个列表替换）
        #[serde(default)]
        rules: Option<Vec<board_core::hosts::ClaimRule>>,
        /// 兜底的 agent；空字符串表示取消兜底
        #[serde(default)]
        fallback: Option<String>,
        /// 自动升级的开关
        #[serde(default, rename = "autoUpgrade")]
        auto_upgrade: Option<bool>,
        /// 单次最长运行（分钟，0 = 不限），所有机器一样
        #[serde(default, rename = "maxRun")]
        max_run: Option<i64>,
        /// 没输出多久算卡住（分钟，0 = 不限），所有机器一样
        #[serde(default, rename = "idleRun")]
        idle_run: Option<i64>,
    }
    let b: Body = parse(v)?;
    let client = client_of(&h);
    let out = s
        .db(move |st| {
            st.write(now_ms(), Origin::ui(client), |w| {
                if let Some(on) = b.auto_upgrade {
                    w.set_auto_upgrade(on)?;
                }
                if b.max_run.is_some() || b.idle_run.is_some() {
                    w.set_run_limits(b.max_run, b.idle_run)?;
                }
                w.sched_update(b.rules.as_deref(), b.fallback.as_deref())
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

/// 停止所有正在跑的派发，同时全部暂停
pub(crate) async fn stop_all(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap) -> ApiResult<Vec<HostInfo>> {
    let (actor, by) = who(&s, &caller, None).await?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.hosts_stop_all(&actor, &by))).await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct RenameBody {
    /// 新机器名
    name: String,
}

/// 机器改名（桌面端「设置 → 本机」保存机器名时调用）：机器记录原地改名，设置和历史派发记录保留。
/// 新名字被在线的机器占用时 HOST_NAME_TAKEN，被离线的机器占用时 USAGE
pub(crate) async fn rename(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<HostInfo> {
    let b: RenameBody = parse(v)?;
    let origin = origin_of(&caller, &h);
    let token = token_of(&caller);
    let out = s
        .db(move |st| {
            st.write(now_ms(), origin, |w| {
                w.read().host_owned_by(&name, token)?;
                w.host_rename(&name, &b.name)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

/// 这台机器最近 50 个结束的派发
pub(crate) async fn recent(State(s): State<AppState>, Path(name): Path<String>) -> ApiResult<Vec<Dispatch>> {
    Ok(Json(
        s.db(move |st| {
            st.read(|r| {
                let h = r.host(&name, now_ms())?;
                r.host_recent(h.id, 50)
            })
        })
        .await?,
    ))
}

/* ------------------------------ 派发 ------------------------------ */

#[derive(Deserialize)]
pub(crate) struct CreateBody {
    board: String,
    card: i64,
    /// 机器名；不带或是空的 = 任意机器（服务端挑）
    #[serde(default)]
    host: String,
    #[serde(default)]
    agent: Option<String>,
    /// 角色（标识或名字）；不带、也不带 agent 时用按规则会交给的角色
    #[serde(default)]
    profile: Option<String>,
    /// 令牌带的 agent 名字（发起人）
    #[serde(default, rename = "as")]
    who: Option<String>,
}

pub(crate) async fn create(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Dispatch> {
    let b: CreateBody = parse(v)?;
    let (actor, _) = who(&s, &caller, b.who.as_deref()).await?;
    let origin = origin_of(&caller, &h);
    let out = s
        .db(move |st| {
            st.write(now_ms(), origin, |w| {
                let board = w.resolve(&b.board)?;
                match b.host.trim() {
                    "" => w.dispatch_create_any(&board, b.card, b.agent.as_deref(), b.profile.as_deref(), &actor),
                    host => w.dispatch_create(&board, b.card, host, b.agent.as_deref(), b.profile.as_deref(), &actor),
                }
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct MoveBody {
    /// 派发 ID；不给时按白板和卡片找它排队的派发
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    board: Option<String>,
    #[serde(default)]
    card: Option<i64>,
    /// 换到哪台机器；不带或是空的 = 交给任意机器
    #[serde(default)]
    host: String,
    #[serde(default)]
    agent: Option<String>,
    /// 换成哪个角色；不带时沿用原来的
    #[serde(default)]
    profile: Option<String>,
    #[serde(default, rename = "as")]
    who: Option<String>,
}

/// `POST /api/dispatches/move`：改派一条还在排队的派发（换一台机器，或交给任意机器）
pub(crate) async fn move_to(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Dispatch> {
    let b: MoveBody = parse(v)?;
    let (actor, _) = who(&s, &caller, b.who.as_deref()).await?;
    let origin = origin_of(&caller, &h);
    let out = s
        .db(move |st| {
            st.write(now_ms(), origin, |w| {
                let id = match (b.id, b.board.as_deref(), b.card) {
                    (Some(id), _, _) => id,
                    (None, Some(board), Some(card)) => {
                        let board = w.resolve(board)?;
                        w.card_dispatch_id(&board, card)?
                    }
                    _ => return Err(BoardError::usage("缺少 id，或 board 和 card")),
                };
                let host = Some(b.host.trim()).filter(|h| !h.is_empty());
                w.dispatch_move(id, host, b.agent.as_deref(), b.profile.as_deref(), &actor)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct CancelBody {
    /// 派发 ID；不给时按白板和卡片找它排队或正在跑的派发
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    board: Option<String>,
    #[serde(default)]
    card: Option<i64>,
    #[serde(default, rename = "as")]
    who: Option<String>,
}

pub(crate) async fn cancel(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Dispatch> {
    let b: CancelBody = parse(v)?;
    let (actor, by) = who(&s, &caller, b.who.as_deref()).await?;
    let origin = origin_of(&caller, &h);
    let out = s
        .db(move |st| {
            st.write(now_ms(), origin, |w| {
                let id = match (b.id, b.board.as_deref(), b.card) {
                    (Some(id), _, _) => id,
                    (None, Some(board), Some(card)) => {
                        let board = w.resolve(board)?;
                        w.card_dispatch_id(&board, card)?
                    }
                    _ => return Err(BoardError::usage("缺少 id，或 board 和 card")),
                };
                w.dispatch_cancel(id, &actor, &by)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

/// `POST /api/profiles`：新建（不带 `id`）或修改一个角色（只有人能改）
pub(crate) async fn profile_put(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<board_core::profiles::Profile> {
    let by = human(&caller).map_err(ApiError)?;
    let p: board_core::profiles::Profile = parse(v)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.profile_put(&p, &by))).await?;
    s.poke();
    Ok(Json(out))
}

/// `DELETE /api/profiles/{id}`：删掉一个角色，交给它的规则一起删；返回删掉了几条规则
pub(crate) async fn profile_remove(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Path(id): Path<String>) -> ApiResult<Value> {
    human(&caller).map_err(ApiError)?;
    let client = client_of(&h);
    let rules = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.profile_delete(&id))).await?;
    s.poke();
    Ok(Json(json!({ "ok": true, "rules": rules })))
}
