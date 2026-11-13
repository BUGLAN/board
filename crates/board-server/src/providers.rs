//! 模型供应商共用配置的接口（PRD §10.12「共用配置」）：
//!
//! - `GET /api/providers`：列表和每个内置 agent 用哪个，API key 只给末尾 4 位（界面、访问令牌都能读）
//! - `POST /api/providers {name, endpoints, apiKey, models}`：新增或修改（只能在登录后的界面上做）；API key 留空保留原来的值
//! - `POST /api/providers/agents {agent, provider, model, effort}`：一个内置 agent 用哪个供应商（只能在登录后的界面上做）
//! - `DELETE /api/providers/{名称}`：删掉（只能在界面上做）；用着它的 agent 回到「不管」，各台机器恢复原来的配置
//! - `GET /api/hosts/{名字}/providers`：完整内容（含 API key 原文）和每个 agent 用哪个，只给这台机器用的访问令牌；
//!   界面、其他机器的令牌都是 `NOT_LOGGED_IN`
//! - `POST /api/providers/models {name?, endpoints, apiKey?}`：获取模型——服务端用这些地址和 API key 问供应商有哪些模型
//!   （只能在登录后的界面上做）；API key 留空时用名称对应的供应商存着的那个。OpenAI 兼容的接口优先（`<地址>/models`），
//!   只有 Anthropic 接口时问 `<地址>/v1/models`；12 秒超时，失败时写明原因（key 不对、没有模型列表接口、连不上）

use crate::auth::{human, Caller};
use crate::{client_of, parse, ApiError, ApiResult, AppState};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::{Extension, Json};
use board_core::providers::{AgentProvider, Provider, ProviderEndpoint};
use board_core::*;
use serde::{Deserialize, Serialize};
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

/* ------------------------------ 获取模型 ------------------------------ */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelsReq {
    /// 修改时的供应商名称：API key 留空时用它存着的
    #[serde(default)]
    name: String,
    #[serde(default)]
    endpoints: Vec<ProviderEndpoint>,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelsOut {
    /// 模型名，按字母排、去重
    pub(crate) models: Vec<String>,
    /// 问的是哪个地址
    pub(crate) from: String,
}

pub(crate) async fn models(State(s): State<AppState>, Extension(caller): Extension<Caller>, Json(v): Json<Value>) -> ApiResult<ModelsOut> {
    human(&caller).map_err(ApiError)?;
    let req: ModelsReq = parse(v)?;
    let typed = req.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string);
    let key = match typed {
        Some(k) => k,
        None => {
            let name = req.name.trim().to_string();
            let stored = if name.is_empty() {
                None
            } else {
                s.db(move |st| st.read(|r| Ok(r.providers()?.into_iter().find(|p| p.name == name).and_then(|p| p.api_key)))).await?
            };
            stored.filter(|k| !k.is_empty()).ok_or_else(|| ApiError(BoardError::usage("先填 API key")))?
        }
    };
    let endpoints = req.endpoints;
    let out = tokio::task::spawn_blocking(move || fetch_models(&endpoints, &key))
        .await
        .map_err(|e| ApiError(BoardError::new(ErrorCode::Internal, format!("获取模型的线程出错：{e}"))))?
        .map_err(|e| ApiError(BoardError::usage(e)))?;
    Ok(Json(out))
}

/// 问哪个地址：OpenAI 兼容的接口优先（大多数供应商的 Anthropic 兼容地址不提供模型列表），只有 Anthropic 接口时问 /v1/models
pub(crate) fn models_url(endpoints: &[ProviderEndpoint]) -> Option<(String, bool)> {
    let url_of = |id: &str| endpoints.iter().find(|e| e.interface == id && !e.url.trim().is_empty()).map(|e| e.url.trim().trim_end_matches('/').to_string());
    if let Some(base) = url_of("openai-chat").or_else(|| url_of("openai-responses")) {
        return Some((format!("{base}/models"), false));
    }
    let base = url_of("anthropic")?;
    Some((if base.ends_with("/v1") { format!("{base}/models") } else { format!("{base}/v1/models") }, true))
}

/// 模型列表的几种写法：`{data: [{id}]}`（OpenAI、Anthropic）、`{models: [...]}`、直接一个数组；元素是字符串或带 id / name 的对象
pub(crate) fn parse_models(text: &str) -> std::result::Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "返回的不是模型列表（不是 JSON）".to_string())?;
    let list = v["data"].as_array().or_else(|| v["models"].as_array()).or_else(|| v.as_array()).ok_or_else(|| "返回里没有模型列表".to_string())?;
    let mut out: Vec<String> = list
        .iter()
        .filter_map(|m| m.as_str().or_else(|| m["id"].as_str()).or_else(|| m["name"].as_str()))
        .map(|s| s.trim().trim_start_matches("models/").to_string())
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    if out.is_empty() {
        return Err("供应商返回的模型列表是空的".into());
    }
    Ok(out)
}

fn fetch_models(endpoints: &[ProviderEndpoint], key: &str) -> std::result::Result<ModelsOut, String> {
    let (url, anthropic) = models_url(endpoints).ok_or_else(|| "先打开一个接口、填上地址".to_string())?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("地址要以 http:// 或 https:// 开头".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(12)))
        .http_status_as_error(false)
        .build()
        .into();
    let req = agent.get(&url).header("accept", "application/json");
    let req = if anthropic { req.header("x-api-key", key).header("anthropic-version", "2023-06-01") } else { req.header("authorization", format!("Bearer {key}")) };
    let host = url.split('/').nth(2).unwrap_or(&url).to_string();
    let mut res = req.call().map_err(|e| format!("连不上 {host}：{e}"))?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    match status {
        200..=299 => parse_models(&text).map(|models| ModelsOut { models, from: url }),
        401 | 403 => Err(format!("供应商返回 {status}：API key 不对，或没有权限看模型列表")),
        404 => Err(format!("{url} 返回 404：这个地址没有模型列表接口，可以手动填模型")),
        _ => {
            let brief: String = text.chars().take(160).collect();
            Err(format!("供应商返回 {status}{}", if brief.trim().is_empty() { String::new() } else { format!("：{}", brief.trim()) }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(interface: &str, url: &str) -> ProviderEndpoint {
        ProviderEndpoint { interface: interface.into(), url: url.into() }
    }

    #[test]
    fn models_url_prefers_openai_compatible() {
        let both = [ep("anthropic", "https://api.deepseek.com/anthropic"), ep("openai-chat", "https://api.deepseek.com/v1/")];
        assert_eq!(models_url(&both), Some(("https://api.deepseek.com/v1/models".into(), false)));
        assert_eq!(models_url(&[ep("openai-responses", "https://x/v1")]), Some(("https://x/v1/models".into(), false)));
        assert_eq!(models_url(&[ep("anthropic", "https://api.anthropic.com")]), Some(("https://api.anthropic.com/v1/models".into(), true)));
        assert_eq!(models_url(&[ep("anthropic", "https://relay/v1")]), Some(("https://relay/v1/models".into(), true)));
        assert_eq!(models_url(&[ep("openai-chat", "  ")]), None);
    }

    #[test]
    fn parse_models_accepts_common_shapes() {
        assert_eq!(parse_models(r#"{"data":[{"id":"b"},{"id":"a"},{"id":"b"}]}"#).unwrap(), vec!["a", "b"]);
        assert_eq!(parse_models(r#"{"models":[{"name":"models/gemini-3-pro"}]}"#).unwrap(), vec!["gemini-3-pro"]);
        assert_eq!(parse_models(r#"["x","y"]"#).unwrap(), vec!["x", "y"]);
        assert!(parse_models("<html>").unwrap_err().contains("不是 JSON"));
        assert!(parse_models(r#"{"data":[]}"#).unwrap_err().contains("空"));
    }
}
