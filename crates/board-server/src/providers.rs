//! 模型供应商共用配置的接口（PRD §10.12「共用配置」）：
//!
//! - `GET /api/providers`：列表和每个内置 agent 用哪个，API key 只给末尾 4 位（界面、访问令牌都能读）
//! - `POST /api/providers {name, endpoints, apiKey, models}`：新增或修改（只能在登录后的界面上做）；API key 留空保留原来的值
//! - `POST /api/providers/agents {agent, provider, model, effort}`：一个内置 agent 用哪个供应商（只能在登录后的界面上做）
//! - `DELETE /api/providers/{名称}`：删掉（只能在界面上做）；用着它的 agent 回到「不管」，各台机器恢复原来的配置
//! - `GET /api/hosts/{名字}/providers`：完整内容（含 API key 原文）和每个 agent 用哪个，只给这台机器用的访问令牌；
//!   界面、其他机器的令牌都是 `NOT_LOGGED_IN`

use crate::auth::{human, Caller};
use crate::{client_of, parse, ApiError, ApiResult, AppState};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::{Extension, Json};
use board_core::providers::{AgentProvider, Provider};
use board_core::*;
use serde::Serialize;
use serde_json::{json, Value};

/// 列表接口：打了码的供应商 + 每个内置 agent 用哪个
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderView {
    providers: Vec<Provider>,
    agents: Vec<AgentProvider>,
}

fn view(providers: Vec<Provider>, agents: Vec<AgentProvider>) -> ProviderView {
    ProviderView { providers: providers.iter().map(Provider::masked).collect(), agents }
}

pub(crate) async fn list(State(s): State<AppState>) -> ApiResult<ProviderView> {
    let (providers, agents) = s.db(|st| st.read(|r| Ok((r.providers()?, r.provider_agents()?)))).await?;
    Ok(Json(view(providers, agents)))
}

pub(crate) async fn put(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<Provider> {
    let by = human(&caller).map_err(ApiError)?;
    let provider: Provider = parse(v)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.provider_put(provider, &by, "界面"))).await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn assign(State(s): State<AppState>, Extension(caller): Extension<Caller>, Json(v): Json<Value>) -> ApiResult<AgentProvider> {
    let by = human(&caller).map_err(ApiError)?;
    let a: AgentProvider = parse(v)?;
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(None), |w| w.provider_agent_put(a, &by))).await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn remove(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Path(name): Path<String>) -> ApiResult<Value> {
    human(&caller).map_err(ApiError)?;
    let client = client_of(&h);
    s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.provider_rm(&name))).await?;
    s.poke();
    Ok(Json(json!({ "ok": true })))
}

/// board host 取到的完整内容：含 API key 原文
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostProviderView {
    providers: Vec<Provider>,
    agents: Vec<AgentProvider>,
}

/// board host 取完整内容（含 API key 原文）：只给这台机器用的访问令牌，别的令牌（界面登录、其他机器、没绑机器的新令牌）
/// 和登录的界面都拿不到原文，用打码的 `GET /api/providers`
pub(crate) async fn for_host(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(name): Path<String>) -> ApiResult<HostProviderView> {
    let Caller::Agent { token } = caller else {
        return Err(ApiError(BoardError::new(ErrorCode::NotLoggedIn, "API key 原文只发给 board host：界面上用 GET /api/providers")));
    };
    let name = name.trim().to_string();
    let now = now_ms();
    let own = s.db(move |st| st.read(|r| r.host_uses_token(&name, token, now))).await?;
    if !own {
        return Err(ApiError(BoardError::new(
            ErrorCode::NotLoggedIn,
            "API key 原文只发给这台机器用的令牌：机器不在这台机器上，或它的令牌不是你用的这个；界面上用 GET /api/providers（API key 打码）",
        )));
    }
    let (providers, agents) = s.db(|st| st.read(|r| Ok((r.providers()?, r.provider_agents()?)))).await?;
    Ok(Json(HostProviderView { providers, agents }))
}
