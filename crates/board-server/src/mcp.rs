//! MCP 服务器共用配置的接口（PRD §10.12「共用配置」）：
//!
//! - `GET /api/mcp`：列表，密钥只给末尾 4 位（界面、访问令牌都能读）
//! - `POST /api/mcp {name, kind, command, args, env, url, headers, agents}`：新增或修改（只能在登录后的界面上做）；密钥留空保留原来的值
//! - `DELETE /api/mcp/{名称}`：删掉（只能在界面上做）；各台机器上同步加的条目随后删掉
//! - `GET /api/hosts/{名字}/mcp`：完整内容（含密钥原文），只给这台机器用的访问令牌（board host 上报时记下的那个）；
//!   界面、其他机器的令牌都是 `NOT_LOGGED_IN`

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

/// board host 取完整内容（含密钥原文）：只给这台机器用的访问令牌，别的令牌（界面登录、其他机器、没绑机器的新令牌）
/// 和登录的界面都拿不到原文，用打码的 `GET /api/mcp`
pub(crate) async fn for_host(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(name): Path<String>) -> ApiResult<Vec<McpServer>> {
    let Caller::Agent { token } = caller else {
        return Err(ApiError(BoardError::new(ErrorCode::NotLoggedIn, "密钥原文只发给 board host：界面上用 GET /api/mcp")));
    };
    let name = name.trim().to_string();
    let now = now_ms();
    let own = s.db(move |st| st.read(|r| r.host_uses_token(&name, token, now))).await?;
    if !own {
        return Err(ApiError(BoardError::new(
            ErrorCode::NotLoggedIn,
            "密钥原文只发给这台机器用的令牌：机器不在这台机器上，或它的令牌不是你用的这个；界面上用 GET /api/mcp（密钥打码）",
        )));
    }
    Ok(Json(s.db(|st| st.read(|r| r.mcp_servers())).await?))
}
