//! agent 模板共用配置的接口（PRD §10.12「共用配置」，规则在 board-core 的 templates.rs）。只能在登录后的界面上用：
//!
//! - `GET /api/agent-templates`：全部模板
//! - `POST /api/agent-templates {id, name, cmd, model, modelArgs, effortArgs, efforts, win}`：新增或修改；在线的 board host 马上取到、重新检查本机 agent
//! - `DELETE /api/agent-templates/{标识}`：删掉
//!
//! board host 不调这些：模板随它的任务（`HostWork.templates`）下发。

use crate::auth::{human, Caller};
use crate::{client_of, parse, ApiError, ApiResult, AppState};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::{Extension, Json};
use board_core::*;
use serde_json::{json, Value};

pub(crate) async fn list(State(s): State<AppState>) -> ApiResult<Vec<AgentTemplate>> {
    Ok(Json(s.db(|st| st.read(|r| r.agent_templates())).await?))
}

pub(crate) async fn put(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<AgentTemplate> {
    let by = human(&caller).map_err(ApiError)?;
    let t: AgentTemplate = parse(v)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.template_put(t, &by))).await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn remove(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Path(id): Path<String>) -> ApiResult<Value> {
    human(&caller).map_err(ApiError)?;
    let client = client_of(&h);
    s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.template_rm(&id))).await?;
    s.poke();
    Ok(Json(json!({ "ok": true })))
}
