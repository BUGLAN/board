//! 任务下发接口（任务下发需求 §十）。规则在 board-core 的 `hosts` 里。
//!
//! board host 用访问令牌调用：
//! - `POST /api/hosts/{名字}/beat`：上报（启动时、之后每 10 秒），返回这台机器的任务
//! - `GET /api/hosts/{名字}/work`：只取任务（收到实时推送里 `hosts` 变化时）
//! - `POST /api/hosts/{名字}/begin {instance, id}`：开始一次排队中的派发（领取卡片）
//! - `POST /api/hosts/{名字}/pull {instance}`：定时领取的下一张；这一轮结束时返回 null
//! - `POST /api/hosts/{名字}/round`：开始一轮领取（定时到点；界面上的「现在领一轮」也是它）
//! - `POST /api/hosts/{名字}/rename {name}`：机器改名（桌面端「设置 → 本机」保存机器名时调用）
//! - `POST /api/dispatches/{ID}/end`、`POST /api/dispatches/{ID}/log`：派发结束、上传日志
//!
//! 界面（登录）：`GET /api/hosts`、`PATCH|DELETE /api/hosts/{名字}`、`GET /api/hosts/{名字}/recent`、`GET /api/dispatches/{ID}/log`。
//! 登录或令牌：`POST /api/dispatches`（派发）、`POST /api/dispatches/cancel`（取消排队 / 停止在跑）。

use axum::extract::{Path, State};
use axum::http::HeaderMap;
#[cfg(feature = "test-hooks")]
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use board_core::hosts::EndHow;
use board_core::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::Caller;
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
    Ok(Json(out))
}

pub(crate) async fn work(State(s): State<AppState>, Path(name): Path<String>) -> ApiResult<HostWork> {
    Ok(Json(s.db(move |st| st.read(|r| r.host_work(&name, now_ms()))).await?))
}

#[derive(Deserialize)]
pub(crate) struct BeginBody {
    instance: String,
    #[serde(default)]
    id: Option<i64>,
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
    let out = s.db(move |st| st.write(now_ms(), Origin::cli(), |w| w.dispatch_begin(&name, &b.instance, id, token))).await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn pull(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Option<Dispatch>> {
    let b: BeginBody = parse(v)?;
    let token = token_of(&caller);
    let out = s.db(move |st| st.write(now_ms(), Origin::cli(), |w| w.dispatch_pull(&name, &b.instance, token))).await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn round(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Path(name): Path<String>) -> ApiResult<HostInfo> {
    let origin = origin_of(&caller, &h);
    let out = s.db(move |st| st.write(now_ms(), origin, |w| w.host_round(&name))).await?;
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
}

fn exit() -> EndHow {
    EndHow::Exit
}

pub(crate) async fn end(State(s): State<AppState>, Path(id): Path<i64>, Json(v): Json<Value>) -> Response {
    // 集成测试用：模拟「服务端已经收到、响应丢了」之前的失败响应
    #[cfg(feature = "test-hooks")]
    if s.take_fail_end() {
        return (StatusCode::BAD_GATEWAY, "bad gateway").into_response();
    }
    match end_inner(s, id, v).await {
        Ok(j) => j.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn end_inner(s: AppState, id: i64, v: Value) -> ApiResult<Dispatch> {
    let b: EndBody = parse(v)?;
    let out = s.db(move |st| st.write(now_ms(), Origin::cli(), |w| w.dispatch_end(id, b.exit_code, &b.tail, b.how))).await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct LogBody {
    #[serde(default)]
    lines: Vec<String>,
}

pub(crate) async fn append_log(State(s): State<AppState>, Path(id): Path<i64>, Json(v): Json<Value>) -> ApiResult<Value> {
    let b: LogBody = parse(v)?;
    s.db(move |st| st.write(now_ms(), Origin::cli(), |w| w.dispatch_log_append(id, &b.lines))).await?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn log(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Value> {
    let lines = s.db(move |st| st.read(|r| r.dispatch_log(id))).await?;
    Ok(Json(json!({ "lines": lines })))
}

/* ------------------------------ 界面 ------------------------------ */

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
    let client = client_of(&h);
    let by = match caller {
        Caller::Human { login, .. } => login.unwrap_or_else(|| "人".into()),
        Caller::Agent { .. } => return Err(ApiError(BoardError::code(ErrorCode::NotLoggedIn))),
    };
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
    let by = match caller {
        Caller::Human { login, .. } => login.unwrap_or_else(|| "人".into()),
        Caller::Agent { .. } => return Err(ApiError(BoardError::code(ErrorCode::NotLoggedIn))),
    };
    let out = s
        .db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.host_clone_request(&name, &b.board, &b.url, &by)))
        .await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn remove(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Path(name): Path<String>) -> ApiResult<Value> {
    let (actor, by) = who(&s, &caller, None).await?;
    let client = client_of(&h);
    s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.host_remove(&name, &actor, &by))).await?;
    s.poke();
    Ok(Json(json!({ "ok": true })))
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
    let out = s.db(move |st| st.write(now_ms(), origin, |w| w.host_rename(&name, &b.name))).await?;
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
    host: String,
    #[serde(default)]
    agent: Option<String>,
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
                w.dispatch_create(&board, b.card, b.host.trim(), b.agent.as_deref(), &actor)
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
