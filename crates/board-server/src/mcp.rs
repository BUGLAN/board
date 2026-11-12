//! MCP 服务器共用配置的接口（PRD §10.12「共用配置」）：
//!
//! - `GET /api/mcp`：列表，密钥只给末尾 4 位（界面、访问令牌都能读）
//! - `POST /api/mcp {name, kind, command, args, env, url, headers, agents}`：新增或修改（只能在登录后的界面上做）；密钥留空保留原来的值
//! - `DELETE /api/mcp/{名称}`：删掉（只能在界面上做）；各台机器上同步加的条目随后删掉
//! - `GET /api/hosts/{名字}/mcp`：完整内容（含密钥原文），只给 board host 的访问令牌

use crate::auth::{human, Caller};
use crate::{client_of, parse, ApiError, ApiResult, AppState};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::{Extension, Json};
use board_core::mcp::McpServer;
use board_core::*;
use serde_json::{json, Value};

pub(crate) async fn list(State(s): State<AppState>) -> ApiResult<Vec<McpServer>> {
    let all = s.db(|st| st.read(|r| r.mcp_servers())).await?;
    Ok(Json(all.iter().map(McpServer::masked).collect()))
}

pub(crate) async fn put(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<McpServer> {
    let by = human(&caller).map_err(ApiError)?;
    let server: McpServer = parse(v)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.mcp_put(server, &by, "界面"))).await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn remove(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Path(name): Path<String>) -> ApiResult<Value> {
    human(&caller).map_err(ApiError)?;
    let client = client_of(&h);
    s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.mcp_rm(&name))).await?;
    s.poke();
    Ok(Json(json!({ "ok": true })))
}

/// board host 取完整内容（含密钥原文）：只给访问令牌；登录的界面拿不到原文
pub(crate) async fn for_host(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(_name): Path<String>) -> ApiResult<Vec<McpServer>> {
    if !matches!(caller, Caller::Agent { .. }) {
        return Err(ApiError(BoardError::usage("密钥原文只发给 board host：界面上用 GET /api/mcp")));
    }
    Ok(Json(s.db(|st| st.read(|r| r.mcp_servers())).await?))
}
