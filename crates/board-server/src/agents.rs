//! agent 接口（PRD §3）：check 名字、agent 级心跳、在线列表。访问令牌可以调用。
//!
//! - `POST /api/agents/check {name, session}`：名字正被在线 agent 使用时 `NAME_TAKEN`；否则标为在线。`session: true`（新版 CLI）
//!   时发一个随机的会话标识，名字记成属于这个会话，返回里带上 `session`
//! - `POST /api/agents/beat {name, final, session}`：agent 级心跳，同时维持名字在线和它持有的卡片；`final` 让名字立刻离线。
//!   名字属于某个会话时，`session` 对不上（或没带）就 `NAME_TAKEN`
//! - `GET /api/agents`：在线的 agent，以及它们持有的卡片
//!
//! 规则在 board-core 的 `agents` 里。上线、下线会推送给界面，只更新心跳时间的心跳不推送。

use axum::extract::State;
use axum::{Extension, Json};
use board_core::{now_ms, AgentInfo, BeatBy, BoardError, CardDetail, ErrorCode, NameOwner, Origin};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::Caller;
use crate::{ApiError, AppState};

#[derive(Deserialize)]
pub(crate) struct CheckBody {
    #[serde(default)]
    name: String,
    /// 新版 CLI：要一个会话标识（旧版 CLI 不带，名字没有标识，规则和以前一样）
    #[serde(default)]
    session: bool,
}

#[derive(Deserialize)]
pub(crate) struct BeatBody {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "final")]
    done: bool,
    /// check 时拿到的会话标识（旧版 CLI 不带）
    #[serde(default)]
    session: Option<String>,
}

/// 名字不能为空（格式由 board-core 检查）
fn name_of(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(BoardError::code(ErrorCode::NoIdentity).into());
    }
    Ok(name.to_string())
}

/// `POST /api/agents/check {name}`
pub(crate) async fn check(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(v): Json<Value>,
) -> Result<Json<AgentInfo>, ApiError> {
    let b: CheckBody = crate::parse(v)?;
    let name = name_of(&b.name)?;
    let token = match caller {
        Caller::Agent { token } => Some(token),
        Caller::Human { .. } => None,
    };
    tracing::Span::current().record("who", format!("{name}（check）"));
    let session = b.session.then(|| crate::auth::random_hex(16));
    let out = s
        .db(move |st| {
            let owner = session.as_deref().map_or(NameOwner::Anyone, NameOwner::Session);
            st.write(now_ms(), Origin::cli(), |w| w.agent_check_owned(&name, token, owner))
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

/// `POST /api/agents/beat {name, final}`
pub(crate) async fn beat(State(s): State<AppState>, Json(v): Json<Value>) -> Result<Json<AgentInfo>, ApiError> {
    let b: BeatBody = crate::parse(v)?;
    let name = name_of(&b.name)?;
    tracing::Span::current().record("who", format!("{name}（心跳{}）", if b.done { " final" } else { "" }));
    let session = b.session.filter(|x| !x.trim().is_empty());
    let out = s.db(move |st| st.write(now_ms(), Origin::cli(), |w| w.agent_beat_by(&name, b.done, BeatBy::Cli(session.as_deref())))).await?;
    s.poke();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub(crate) struct NextBody {
    #[serde(default)]
    name: String,
}

/// `POST /api/agents/next {name}`：board host 启动的会话接着做下一张（board next）。返回领到的卡片，和 claim 一样
pub(crate) async fn next(State(s): State<AppState>, Json(v): Json<Value>) -> Result<Json<CardDetail>, ApiError> {
    let b: NextBody = crate::parse(v)?;
    let name = name_of(&b.name)?;
    tracing::Span::current().record("who", format!("{name}（next）"));
    let out = s.db(move |st| st.next(now_ms(), Origin::cli(), &name)).await?;
    s.poke();
    Ok(Json(out))
}

/// `GET /api/agents`：在线的 agent
pub(crate) async fn list(State(s): State<AppState>) -> Result<Json<Vec<AgentInfo>>, ApiError> {
    Ok(Json(s.db(|st| st.read(|r| r.agents(now_ms()))).await?))
}
