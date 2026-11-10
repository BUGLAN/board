//! agent 接口（PRD §3）：check 名字、agent 级心跳、在线列表。访问令牌可以调用。
//!
//! - `POST /api/agents/check {name}`：名字正被在线 agent 使用时 `NAME_TAKEN`；否则标为在线
//! - `POST /api/agents/beat {name, final}`：agent 级心跳，同时维持名字在线和它持有的卡片；`final` 让名字立刻离线
//! - `GET /api/agents`：在线的 agent，以及它们持有的卡片
//!
//! 规则在 board-core 的 `agents` 里。上线、下线会推送给界面，只更新心跳时间的心跳不推送。

use axum::extract::State;
use axum::{Extension, Json};
use board_core::{now_ms, AgentInfo, BoardError, CardDetail, ErrorCode, Origin};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::Caller;
use crate::{ApiError, AppState};

#[derive(Deserialize)]
pub(crate) struct CheckBody {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
pub(crate) struct BeatBody {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "final")]
    done: bool,
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
    let out = s.db(move |st| st.write(now_ms(), Origin::cli(), |w| w.agent_check(&name, token))).await?;
    s.poke();
    Ok(Json(out))
}

/// `POST /api/agents/beat {name, final}`
pub(crate) async fn beat(State(s): State<AppState>, Json(v): Json<Value>) -> Result<Json<AgentInfo>, ApiError> {
    let b: BeatBody = crate::parse(v)?;
    let name = name_of(&b.name)?;
    tracing::Span::current().record("who", format!("{name}（心跳{}）", if b.done { " final" } else { "" }));
    let out = s.db(move |st| st.write(now_ms(), Origin::cli(), |w| w.agent_beat(&name, b.done))).await?;
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
    let out = s.db(move |st| st.write(now_ms(), Origin::cli(), |w| w.dispatch_next(&name))).await?;
    s.poke();
    Ok(Json(out))
}

/// `GET /api/agents`：在线的 agent
pub(crate) async fn list(State(s): State<AppState>) -> Result<Json<Vec<AgentInfo>>, ApiError> {
    Ok(Json(s.db(|st| st.read(|r| r.agents(now_ms()))).await?))
}
