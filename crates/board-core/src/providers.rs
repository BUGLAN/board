//! 模型供应商共用配置（PRD §10.16「共用配置」）：服务端存供应商的地址、API key、模型，各台机器的 board host 按设置改写本机 agent 的配置文件。
//!
//! 和 MCP 共用配置（[`crate::mcp`]）是同一套做法：
//! - 每个供应商：名称、接口（Anthropic Messages、OpenAI Chat Completions、OpenAI Responses 三种里它提供的，每种一个地址）、
//!   API key、模型（一行一个）。API key 只存在服务端：列表接口只给末尾 4 位（[`Provider::masked`]），
//!   完整的值只经 board host 的令牌接口（`GET /api/hosts/{名字}/providers`，只给这台机器上报时用的令牌）下发。
//! - 地址保存时就规范化（[`normalize_url`]）：去掉结尾的 `/`，Anthropic 的再去掉结尾的 `/v1`（Claude Code 自己会接
//!   `/v1/messages`）。按角色派发和「获取模型」（[`models_url`]）都按规范化后的地址。
//! - 每个模型可以标支持哪几档思考强度（`efforts`）；没标的模型在角色里不能选强度（[`writable_efforts`]）。
//! - 版本号是全部内容（含密钥）的哈希前 12 位：密钥改了也会重新同步，哈希不泄露原文。
//! - 每个内置 agent（claude、codex、dsh）用哪个供应商、哪个模型、什么强度、哪种接口：在每台机器上选「不改 / 指定」
//!   （机器设置的 `run`，见 hosts/model.rs 的 [`crate::hosts::AgentRun`]）；服务器上不再存一份给所有机器跟的。
//! - 改了算配置有变化：服务端给受影响的机器下发「同步配置」请求（hosts/sync.rs 的 `request_config_sync`）。

use crate::store::{R, W};
use crate::{BoardError, ErrorCode, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(crate) const SCHEMA: &str = r#"
-- 模型供应商共用配置
CREATE TABLE IF NOT EXISTS providers (
  name       TEXT PRIMARY KEY,
  spec       TEXT NOT NULL,             -- JSON：Provider（含 API key 原文）
  version    TEXT NOT NULL,             -- 内容的哈希
  updated_at INTEGER NOT NULL,
  by_name    TEXT NOT NULL,
  source     TEXT NOT NULL
);
"#;

/// 三种接口的标识和名称
pub const INTERFACES: [(&str, &str); 3] = [
    ("anthropic", "Anthropic Messages"),
    ("openai-chat", "OpenAI Chat Completions"),
    ("openai-responses", "OpenAI Responses"),
];

/// 所有档位，从低到高：供应商里给模型标的强度只能是这几个
pub const ALL_EFFORTS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// 发给旧版 board host 的「不改」：它把这个供应商名当成「本机自己管」。供应商名称不会以 `@` 开头
pub const PROVIDER_LOCAL: &str = "@local";

/// 规范化一个接口的地址：去掉首尾空白和结尾的 `/`；Anthropic 的再去掉结尾的 `/v1`
/// （Claude Code、dsh 自己会接 `/v1/messages`，地址里带着会变成 `/v1/v1/messages`，#344）
pub fn normalize_url(interface: &str, url: &str) -> String {
    let mut u = url.trim().trim_end_matches('/').to_string();
    if interface == "anthropic" {
        while let Some(rest) = u.strip_suffix("/v1") {
            u = rest.trim_end_matches('/').to_string();
        }
    }
    u
}

/// 「获取模型」问哪个地址、是不是 Anthropic 的写法（要带 `x-api-key` 和 `anthropic-version`）：
/// OpenAI 兼容的接口优先（大多数供应商的 Anthropic 兼容地址不提供模型列表），只有 Anthropic 接口时问 `<地址>/v1/models`
pub fn models_url(endpoints: &[ProviderEndpoint]) -> Option<(String, bool)> {
    let url_of = |id: &str| endpoints.iter().find(|e| e.interface == id && !e.url.trim().is_empty()).map(|e| normalize_url(id, &e.url));
    if let Some(base) = url_of("openai-chat").or_else(|| url_of("openai-responses")) {
        return Some((format!("{base}/models"), false));
    }
    let base = url_of("anthropic")?;
    Some((format!("{base}/v1/models"), true))
}

/// 一个内置 agent 的说明（界面用，随 `GET /api/providers` 下发）：名称、认哪些接口、角色里能选哪几档强度、
/// 读用户级 skill 的目录、支不支持 MCP 同步、主色。都来自 [`crate::agentdef`]；旧版服务端只有前四项
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMeta {
    pub id: String,
    pub name: String,
    pub protocols: Vec<String>,
    pub efforts: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub mcp: bool,
    #[serde(default)]
    pub color: String,
}

/// 内置 agent 的说明，按默认 agent 的顺序
pub fn agent_meta() -> Vec<AgentMeta> {
    let strs = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    crate::agentdef::AGENT_DEFS
        .iter()
        .map(|d| AgentMeta {
            id: d.id.to_string(),
            name: d.name.to_string(),
            protocols: strs(d.protocols),
            efforts: strs(d.efforts),
            skills: strs(d.skills),
            mcp: d.mcp.is_some(),
            color: d.color.to_string(),
        })
        .collect()
}

/// 这个 agent 用这家供应商的这个模型时，角色里能选哪几档强度：模型标了的、这个 agent 也能设的（按从低到高）。
/// 模型没标强度时为空：不能选，也不写
pub fn writable_efforts(agent: &str, p: &Provider, model: &str) -> Vec<String> {
    let marked = p.efforts.get(model).cloned().unwrap_or_default();
    efforts_of(agent).iter().filter(|e| marked.iter().any(|m| m == *e)).map(|e| e.to_string()).collect()
}

pub fn interface_name(id: &str) -> &str {
    INTERFACES.iter().find(|(k, _)| *k == id).map(|(_, n)| *n).unwrap_or(id)
}

pub fn agent_name(id: &str) -> &str {
    crate::agentdef::find(id).map_or(id, |d| d.name)
}

/// 这个 agent 认哪些接口；配置文件里新加的 agent 不在这里（`[]`）
pub fn protocols_of(agent: &str) -> &'static [&'static str] {
    crate::agentdef::find(agent).map_or(&[], |d| d.protocols)
}

/// 这个 agent 能设的思考强度（从低到高）
pub fn efforts_of(agent: &str) -> &'static [&'static str] {
    crate::agentdef::find(agent).map_or(&[], |d| d.efforts)
}

/// 供应商的一个接口和它的地址
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEndpoint {
    /// anthropic | openai-chat | openai-responses
    pub interface: String,
    pub url: String,
}

/// 一个模型供应商（PRD §10.16「共用配置」）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub name: String,
    #[serde(default)]
    pub endpoints: Vec<ProviderEndpoint>,
    /// API key；列表接口里是空的（看 tail），保存时留空表示保留原来的值
    #[serde(default)]
    pub api_key: Option<String>,
    /// 密钥的末尾 4 位（只在列表接口里有）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<String>,
    /// 这个供应商的模型（一行一个）
    #[serde(default)]
    pub models: Vec<String>,
    /// 每个模型支持的思考强度（按从低到高）；没标的模型不能选强度
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub efforts: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub source: String,
}

/// 任务里带的索引：board host 据此知道要不要重新取完整内容
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderIndex {
    pub name: String,
    pub version: String,
}

/// 发给 board host 的供应商共用配置：索引（取完整内容用：按角色派发时换供应商、自检时问连不连得上）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderWork {
    #[serde(default)]
    pub providers: Vec<ProviderIndex>,
}

fn tail(v: &str) -> String {
    let c: Vec<char> = v.chars().collect();
    c[c.len().saturating_sub(4)..].iter().collect()
}

/// 名称：字母、数字、`_`、`-`，以字母或数字开头，最长 64 个字符（和 MCP 服务器一样；`@` 开头留给「本机自己管」）
pub fn check_name(name: &str) -> Result<()> {
    crate::mcp::check_name(name).map_err(|_| BoardError::usage(format!("供应商名称「{name}」不对：只能用字母、数字、_、-，以字母或数字开头，最长 64 个字符")))
}

impl Provider {
    /// 给界面、命令行看的：API key 只留末尾 4 位
    pub fn masked(&self) -> Provider {
        Provider { api_key: None, tail: self.api_key.as_deref().map(tail), ..self.clone() }
    }

    /// 接口对上：这个 agent 认的接口里，这家供应商提供了至少一种
    pub fn fits(&self, agent: &str) -> bool {
        let want = protocols_of(agent);
        self.endpoints.iter().any(|e| want.contains(&e.interface.as_str()))
    }

    /// 和这个 agent 对得上的接口（按供应商里的顺序）
    pub fn endpoints_for(&self, agent: &str) -> impl Iterator<Item = &ProviderEndpoint> {
        let want = protocols_of(agent);
        self.endpoints.iter().filter(move |e| want.contains(&e.interface.as_str()))
    }

    /// 这个 agent 用哪个接口：机器上指定了、供应商给着、agent 也认就是它；没指定或已经对不上时用第一个对得上的；都没有时空
    pub fn endpoint_for(&self, agent: &str, protocol: &str) -> Option<&ProviderEndpoint> {
        self.endpoints_for(agent).find(|e| e.interface == protocol).or_else(|| self.endpoints_for(agent).next())
    }

    /// 内容的版本号（含密钥原文，不含版本号和元数据本身）
    pub fn content_version(&self) -> String {
        let body = serde_json::json!({
            "name": self.name,
            "endpoints": self.endpoints,
            "apiKey": self.api_key,
            "models": self.models,
            "efforts": self.efforts,
        });
        Sha256::digest(body.to_string().as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
    }

    /// 校验：名称、至少一个接口、地址、不重复、每个接口一种、有密钥和模型
    fn check(&self) -> Result<()> {
        check_name(&self.name)?;
        if self.endpoints.is_empty() {
            return Err(BoardError::usage("供应商至少要有一个接口和地址"));
        }
        let mut seen: Vec<&str> = vec![];
        for e in &self.endpoints {
            if !INTERFACES.iter().any(|(k, _)| *k == e.interface) {
                return Err(BoardError::usage(format!("不认识的接口「{}」：只能是 {}", e.interface, INTERFACES.map(|(k, _)| k).join("、"))));
            }
            if seen.contains(&e.interface.as_str()) {
                return Err(BoardError::usage(format!("接口「{}」写了两次", interface_name(&e.interface))));
            }
            seen.push(&e.interface);
            let url = e.url.trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(BoardError::usage(format!("{} 的地址要以 http:// 或 https:// 开头", interface_name(&e.interface))));
            }
        }
        if self.api_key.as_deref().map(str::trim).unwrap_or_default().is_empty() {
            return Err(BoardError::usage("供应商要填 API key"));
        }
        if self.models.is_empty() {
            return Err(BoardError::usage("供应商至少要填一个模型（一行一个）"));
        }
        for m in &self.models {
            crate::hosts::check_model(m).map_err(BoardError::usage)?;
        }
        for (m, list) in &self.efforts {
            if let Some(bad) = list.iter().find(|e| !ALL_EFFORTS.contains(&e.as_str())) {
                return Err(BoardError::usage(format!("模型 {m} 的强度「{bad}」不对：只能是 {}", ALL_EFFORTS.join("、"))));
            }
        }
        Ok(())
    }
}

fn provider_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Provider> {
    let spec: String = r.get(1)?;
    let mut p: Provider = serde_json::from_str(&spec).unwrap_or_default();
    p.name = r.get(0)?;
    p.version = r.get(2)?;
    p.updated_at = r.get(3)?;
    p.by = r.get(4)?;
    p.source = r.get(5)?;
    Ok(p)
}

impl R<'_> {
    /// 全部（含 API key 原文：只给这台机器用的令牌接口，见 [`R::host_uses_token`]；界面用 [`Provider::masked`]）
    pub fn providers(&self) -> Result<Vec<Provider>> {
        let mut st = self.conn.prepare("SELECT name, spec, version, updated_at, by_name, source FROM providers ORDER BY name")?;
        let rows = st.query_map([], provider_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn provider_index(&self) -> Result<Vec<ProviderIndex>> {
        Ok(self.providers()?.into_iter().map(|p| ProviderIndex { name: p.name, version: p.version }).collect())
    }

    pub fn provider_work(&self) -> Result<ProviderWork> {
        Ok(ProviderWork { providers: self.provider_index()? })
    }

    pub(crate) fn provider(&self, name: &str) -> Result<Option<Provider>> {
        Ok(self.conn.query_row("SELECT name, spec, version, updated_at, by_name, source FROM providers WHERE name = ?1", [name], provider_row).optional()?)
    }
}

impl W<'_> {
    /// 新增或修改一项。API key 留空时保留原来的值。返回打过码的
    pub fn provider_put(&mut self, mut p: Provider, by: &str, source: &str) -> Result<Provider> {
        p.name = p.name.trim().to_string();
        for e in &mut p.endpoints {
            e.interface = e.interface.trim().to_string();
            e.url = e.url.trim().to_string();
        }
        for e in &mut p.endpoints {
            e.url = normalize_url(&e.interface, &e.url);
        }
        let mut seen: Vec<String> = vec![];
        p.endpoints.retain(|e| {
            let keep = !seen.contains(&e.interface);
            if keep {
                seen.push(e.interface.clone());
            }
            keep
        });
        p.models = p.models.iter().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
        // 强度：只留列表里有的模型，档位按从低到高排好、去重；空的不存
        let models = p.models.clone();
        p.efforts = std::mem::take(&mut p.efforts)
            .into_iter()
            .map(|(m, list)| (m.trim().to_string(), list))
            .filter(|(m, _)| models.contains(m))
            .map(|(m, list)| {
                let list: Vec<String> = list.iter().map(|e| e.trim().to_string()).collect();
                let mut out: Vec<String> = ALL_EFFORTS.iter().filter(|e| list.iter().any(|x| x == *e)).map(|e| e.to_string()).collect();
                out.extend(list.into_iter().filter(|e| !e.is_empty() && !ALL_EFFORTS.contains(&e.as_str())));
                (m, out)
            })
            .filter(|(_, l)| !l.is_empty())
            .collect();
        let old = self.read().provider(&p.name)?;
        p.tail = None;
        if p.api_key.as_deref().map(str::trim).unwrap_or_default().is_empty() {
            p.api_key = old.as_ref().and_then(|o| o.api_key.clone());
        }
        p.check()?;
        p.version = p.content_version();
        p.updated_at = self.now;
        p.by = if by.trim().is_empty() { "人".into() } else { by.trim().into() };
        p.source = if source.trim().is_empty() { "界面".into() } else { source.trim().into() };
        if old.as_ref().is_some_and(|o| o.version == p.version) {
            return Ok(old.expect("上面看过").masked());
        }
        self.tx.execute(
            "INSERT INTO providers (name, spec, version, updated_at, by_name, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name) DO UPDATE SET spec = ?2, version = ?3, updated_at = ?4, by_name = ?5, source = ?6",
            params![p.name, serde_json::to_string(&p).unwrap_or_default(), p.version, p.updated_at, p.by, p.source],
        )?;
        self.hosts_changed = true;
        self.config_changed = true;
        self.config_by = p.by.clone();
        self.changed = true;
        Ok(p.masked())
    }

    /// 删掉一个供应商：机器上指定成它的 agent 下次同步时写入失败（界面上提示换一个）
    pub fn provider_rm(&mut self, name: &str) -> Result<()> {
        // 还有角色在用它：先改掉这些角色（换供应商或本机配置）
        let users: Vec<String> = self.read().profiles()?.into_iter().filter(|p| p.provider == name).map(|p| format!("「{}」", p.name)).collect();
        if !users.is_empty() {
            return Err(BoardError::usage(format!("角色{}还在用「{name}」：先把它们改成别的供应商或本机配置", users.join(""))));
        }
        if self.tx.execute("DELETE FROM providers WHERE name = ?1", [name])? == 0 {
            return Err(BoardError::new(ErrorCode::NotFound, format!("没有模型供应商「{name}」")));
        }
        self.hosts_changed = true;
        self.config_changed = true;
        self.changed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Origin, Store};

    fn provider(name: &str) -> Provider {
        Provider {
            name: name.into(),
            endpoints: vec![ProviderEndpoint { interface: "anthropic".into(), url: "https://api.example.com".into() }],
            api_key: Some("sk-123456789".into()),
            models: vec!["model-a".into(), "model-b".into()],
            ..Default::default()
        }
    }

    #[test]
    fn secrets_are_masked_kept_and_versioned() {
        let mut s = Store::open_in_memory().unwrap();
        let put = |s: &mut Store, p: Provider| s.write(1000, Origin::ui(None), |w| w.provider_put(p, "octocat", ""));
        let out = put(&mut s, provider("deepseek")).unwrap();
        // 返回给界面的打过码：只有末尾 4 位
        assert_eq!((out.api_key.as_deref(), out.tail.as_deref()), (None, Some("6789")));
        assert!(!serde_json::to_string(&out).unwrap().contains("sk-123456789"));
        // board host 拿到原文
        let full = s.read(|r| r.providers()).unwrap();
        assert_eq!(full[0].api_key.as_deref(), Some("sk-123456789"));
        let v1 = full[0].version.clone();
        // 改别的、密钥留空：保留原来的值，版本变了
        let out = put(&mut s, Provider { endpoints: vec![ProviderEndpoint { interface: "openai-chat".into(), url: "https://api.example.com/v1".into() }], api_key: None, ..provider("deepseek") }).unwrap();
        assert_ne!(out.version, v1);
        assert_eq!(s.read(|r| r.providers()).unwrap()[0].api_key.as_deref(), Some("sk-123456789"));
        // 只改密钥：版本也变，但不是原文
        let v2 = out.version.clone();
        let v3 = put(&mut s, Provider { api_key: Some("sk-new".into()), ..provider("deepseek") }).unwrap().version;
        assert_ne!(v2, v3);
        assert!(!v3.contains("sk-new"));
        // 新加的没有 key、没有模型、没有接口：拒绝
        assert!(put(&mut s, Provider { api_key: None, ..provider("x") }).unwrap_err().message.contains("API key"));
        assert!(put(&mut s, Provider { models: vec![], ..provider("x") }).unwrap_err().message.contains("模型"));
        assert!(put(&mut s, Provider { endpoints: vec![], ..provider("x") }).unwrap_err().message.contains("接口"));
        assert!(put(&mut s, Provider { endpoints: vec![ProviderEndpoint { interface: "anthropic".into(), url: "ftp://x".into() }], ..provider("x") }).unwrap_err().message.contains("http"));
    }

    #[test]
    fn urls_are_normalized_and_models_url_shares_it() {
        assert_eq!(normalize_url("anthropic", " https://relay.example.com/v1/ "), "https://relay.example.com");
        assert_eq!(normalize_url("anthropic", "https://api.deepseek.com/anthropic"), "https://api.deepseek.com/anthropic");
        assert_eq!(normalize_url("openai-chat", "https://api.deepseek.com/v1/"), "https://api.deepseek.com/v1", "OpenAI 的 /v1 照填的留着");
        let ep = |i: &str, u: &str| ProviderEndpoint { interface: i.into(), url: u.into() };
        let both = [ep("anthropic", "https://api.deepseek.com/anthropic"), ep("openai-chat", "https://api.deepseek.com/v1/")];
        assert_eq!(models_url(&both), Some(("https://api.deepseek.com/v1/models".into(), false)));
        assert_eq!(models_url(&[ep("openai-responses", "https://x/v1")]), Some(("https://x/v1/models".into(), false)));
        assert_eq!(models_url(&[ep("anthropic", "https://api.anthropic.com")]), Some(("https://api.anthropic.com/v1/models".into(), true)));
        assert_eq!(models_url(&[ep("anthropic", "https://relay/v1")]), Some(("https://relay/v1/models".into(), true)), "带 /v1 的不会问成 /v1/v1/models");
        assert_eq!(models_url(&[ep("openai-chat", "  ")]), None);
        // 保存时就去掉
        let mut s = Store::open_in_memory().unwrap();
        s.write(1000, Origin::ui(None), |w| w.provider_put(Provider { endpoints: vec![ep("anthropic", "https://relay/v1/")], ..provider("r") }, "", "")).unwrap();
        assert_eq!(s.read(|r| r.providers()).unwrap()[0].endpoints[0].url, "https://relay");
    }

    #[test]
    fn model_efforts_are_tidied_and_limit_what_agents_can_pick() {
        let mut s = Store::open_in_memory().unwrap();
        let efforts: BTreeMap<String, Vec<String>> =
            [("model-a".to_string(), vec!["max".to_string(), "low".into(), "low".into(), "minimal".into()]), ("gone".to_string(), vec!["high".to_string()]), ("model-b".to_string(), vec![])].into();
        let out = s.write(1000, Origin::ui(None), |w| w.provider_put(Provider { efforts, ..provider("p1") }, "", "")).unwrap();
        // 按从低到高、去重；列表里没有的模型、空的去掉
        assert_eq!(out.efforts, [("model-a".to_string(), vec!["minimal".to_string(), "low".into(), "max".into()])].into());
        let p = s.read(|r| r.providers()).unwrap().remove(0);
        // 交集：claude 写不了 minimal，codex 能写 minimal、max
        assert_eq!(writable_efforts("claude", &p, "model-a"), vec!["low", "max"]);
        assert_eq!(writable_efforts("codex", &p, "model-a"), vec!["minimal", "low", "max"]);
        assert!(writable_efforts("claude", &p, "model-b").is_empty(), "没标的不能选");
        // 不认识的档位
        let bad: BTreeMap<String, Vec<String>> = [("model-a".to_string(), vec!["ultra".to_string()])].into();
        assert!(s.write(3000, Origin::ui(None), |w| w.provider_put(Provider { efforts: bad, ..provider("p1") }, "", "")).unwrap_err().message.contains("ultra"));
        // 改了强度，版本跟着变（board host 要重新写）
        let v1 = s.read(|r| r.providers()).unwrap()[0].version.clone();
        let fewer: BTreeMap<String, Vec<String>> = [("model-a".to_string(), vec!["low".to_string()])].into();
        let v2 = s.write(3000, Origin::ui(None), |w| w.provider_put(Provider { efforts: fewer, ..provider("p1") }, "", "")).unwrap().version;
        assert_ne!(v1, v2);
    }

    #[test]
    fn endpoint_follows_the_chosen_interface_and_falls_back_to_the_first() {
        let ep = |i: &str, u: &str| ProviderEndpoint { interface: i.into(), url: u.into() };
        let p = Provider { endpoints: vec![ep("openai-chat", "https://x/v1"), ep("anthropic", "https://x"), ep("openai-responses", "https://x/v1")], ..provider("p") };
        let pick = |agent: &str, protocol: &str| p.endpoint_for(agent, protocol).map(|e| e.interface.clone());
        // 没指定：供应商里第一个这个 agent 认的
        assert_eq!(pick("dsh", ""), Some("openai-chat".into()));
        assert_eq!(pick("claude", ""), Some("anthropic".into()));
        // 指定了、供应商给着、agent 也认
        assert_eq!(pick("dsh", "openai-responses"), Some("openai-responses".into()));
        // agent 不认指定的那种：回到第一个对得上的
        assert_eq!(pick("claude", "openai-chat"), Some("anthropic".into()));
        // 对不上任何一种
        let chat = Provider { endpoints: vec![ep("openai-chat", "https://x/v1")], ..provider("c") };
        assert_eq!(chat.endpoint_for("codex", ""), None);
        assert!(!chat.fits("codex") && chat.fits("dsh"));
    }

    #[test]
    fn removing_a_provider_twice_is_not_found() {
        let mut s = Store::open_in_memory().unwrap();
        s.write(1000, Origin::ui(None), |w| w.provider_put(provider("p1"), "", "")).unwrap();
        s.write(3000, Origin::ui(None), |w| w.provider_rm("p1")).unwrap();
        assert!(s.read(|r| r.providers()).unwrap().is_empty());
        assert_eq!(s.write(3000, Origin::ui(None), |w| w.provider_rm("p1")).unwrap_err().code, ErrorCode::NotFound);
    }
}
