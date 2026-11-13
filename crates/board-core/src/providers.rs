//! 模型供应商共用配置（PRD §10.12「共用配置」）：服务端存供应商的地址、API key、模型，各台机器的 board host 按设置改写本机 agent 的配置文件。
//!
//! 和 MCP 共用配置（[`crate::mcp`]）是同一套做法：
//! - 每个供应商：名称、接口（Anthropic Messages、OpenAI Chat Completions、OpenAI Responses 三种里它提供的，每种一个地址）、
//!   API key、模型（一行一个）。API key 只存在服务端：列表接口只给末尾 4 位（[`Provider::masked`]），
//!   完整的值只经 board host 的令牌接口（`GET /api/hosts/{名字}/providers`，只给这台机器上报时用的令牌）下发。
//! - 版本号是全部内容（含密钥）的哈希前 12 位：密钥改了也会重新同步，哈希不泄露原文。
//! - 每个内置 agent（claude、codex、pi、dsh）用哪个供应商、哪个模型、什么强度：不放这里，在 [`AgentProvider`] 里；
//!   机器详情可以改成「本机自己管」或单独指定一个供应商（存在机器设置的 `run` 里）。
//! - 列表变了算机器有变化，在线的 board host 马上重新取任务。

use crate::store::{R, W};
use crate::{BoardError, ErrorCode, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

-- 每个内置 agent 用哪个供应商、哪个模型、什么强度
CREATE TABLE IF NOT EXISTS provider_agents (
  agent      TEXT PRIMARY KEY,
  spec       TEXT NOT NULL,             -- JSON：AgentProvider
  updated_at INTEGER NOT NULL,
  by_name    TEXT NOT NULL
);
"#;

/// 三种接口的标识和名称
pub const INTERFACES: [(&str, &str); 3] = [
    ("anthropic", "Anthropic Messages"),
    ("openai-chat", "OpenAI Chat Completions"),
    ("openai-responses", "OpenAI Responses"),
];

/// 内置 agent：标识、名称、认哪些接口
pub const AGENTS: [(&str, &str, &[&str]); 4] = [
    ("claude", "Claude Code", &["anthropic"]),
    // Codex 的 model_providers 走 Responses 接口（wire_api = "responses"）
    ("codex", "Codex", &["openai-responses"]),
    ("pi", "pi", &["anthropic", "openai-chat", "openai-responses"]),
    ("dsh", "DeepSeek Harness", &["anthropic", "openai-chat", "openai-responses"]),
];

/// 一个 agent 在「模型供应商」里能选的思考强度，从低到高（配置文件的写法按各自 agent 核实过）
pub const EFFORTS: [(&str, &[&str]); 4] = [
    ("claude", &["low", "medium", "high", "xhigh", "max"]),
    ("codex", &["minimal", "low", "medium", "high"]),
    ("pi", &["off", "minimal", "low", "medium", "high", "xhigh", "max"]),
    ("dsh", &["off", "minimal", "low", "medium", "high", "xhigh", "max"]),
];

/// 机器设置里「本机自己管」的写法：不改这个 agent 的配置文件。供应商名称不会以 `@` 开头
pub const PROVIDER_LOCAL: &str = "@local";

pub fn interface_name(id: &str) -> &str {
    INTERFACES.iter().find(|(k, _)| *k == id).map(|(_, n)| *n).unwrap_or(id)
}

pub fn agent_name(id: &str) -> &str {
    AGENTS.iter().find(|(k, _, _)| *k == id).map(|(_, n, _)| *n).unwrap_or(id)
}

/// 这个 agent 认哪些接口；配置文件里新加的 agent 不在这里（`[]`）
pub fn protocols_of(agent: &str) -> &'static [&'static str] {
    AGENTS.iter().find(|(k, _, _)| *k == agent).map(|(_, _, p)| *p).unwrap_or(&[])
}

/// 这个 agent 能选的思考强度
pub fn efforts_of(agent: &str) -> &'static [&'static str] {
    EFFORTS.iter().find(|(k, _)| *k == agent).map(|(_, e)| *e).unwrap_or(&[])
}

/// 供应商的一个接口和它的地址
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEndpoint {
    /// anthropic | openai-chat | openai-responses
    pub interface: String,
    pub url: String,
}

/// 一个模型供应商（PRD §10.12「共用配置」）
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

/// 一个内置 agent 用哪个供应商、哪个模型、什么强度（共用配置）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    /// 内置 agent 标识：claude、codex、pi、dsh
    pub agent: String,
    /// 供应商名称；空 = 不管（不改这个 agent 的配置文件）
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// 思考强度；空 = agent 自己的默认
    #[serde(default)]
    pub effort: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub by: String,
}

/// 一台机器上一个 agent 的配置文件写入状态（board host 上报）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderState {
    pub agent: String,
    /// synced 已写入 | local 本机自己管 | unsupported 不支持 | failed 失败
    pub state: String,
    /// 写进去的供应商、模型（synced 时）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// 发给 board host 的供应商共用配置：索引（取完整内容用）和每个 agent 用哪个
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderWork {
    #[serde(default)]
    pub providers: Vec<ProviderIndex>,
    #[serde(default)]
    pub agents: Vec<AgentProvider>,
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

    /// 和这个 agent 对得上的接口（按接口顺序）；没有时空
    pub fn endpoint_for(&self, agent: &str) -> Option<&ProviderEndpoint> {
        let want = protocols_of(agent);
        self.endpoints.iter().find(|e| want.contains(&e.interface.as_str()))
    }

    /// 内容的版本号（含密钥原文，不含版本号和元数据本身）
    pub fn content_version(&self) -> String {
        let body = serde_json::json!({
            "name": self.name,
            "endpoints": self.endpoints,
            "apiKey": self.api_key,
            "models": self.models,
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

fn agent_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentProvider> {
    let spec: String = r.get(1)?;
    let mut a: AgentProvider = serde_json::from_str(&spec).unwrap_or_default();
    a.agent = r.get(0)?;
    a.updated_at = r.get(2)?;
    a.by = r.get(3)?;
    Ok(a)
}

impl R<'_> {
    /// 全部（含 API key 原文：只给这台机器用的令牌接口，见 [`R::host_uses_token`]；界面用 [`Provider::masked`]）
    pub fn providers(&self) -> Result<Vec<Provider>> {
        let mut st = self.conn.prepare("SELECT name, spec, version, updated_at, by_name, source FROM providers ORDER BY name")?;
        let rows = st.query_map([], provider_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// 每个内置 agent 用哪个供应商
    pub fn provider_agents(&self) -> Result<Vec<AgentProvider>> {
        let mut st = self.conn.prepare("SELECT agent, spec, updated_at, by_name FROM provider_agents ORDER BY agent")?;
        let rows = st.query_map([], agent_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn provider_index(&self) -> Result<Vec<ProviderIndex>> {
        Ok(self.providers()?.into_iter().map(|p| ProviderIndex { name: p.name, version: p.version }).collect())
    }

    pub fn provider_work(&self) -> Result<ProviderWork> {
        Ok(ProviderWork { providers: self.provider_index()?, agents: self.provider_agents()? })
    }

    fn provider(&self, name: &str) -> Result<Option<Provider>> {
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
        let mut seen: Vec<String> = vec![];
        p.endpoints.retain(|e| {
            let keep = !seen.contains(&e.interface);
            if keep {
                seen.push(e.interface.clone());
            }
            keep
        });
        p.models = p.models.iter().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
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
        self.changed = true;
        Ok(p.masked())
    }

    /// 删掉一个供应商：用着它的 agent 回到「不管」，各台机器恢复原来的配置
    pub fn provider_rm(&mut self, name: &str) -> Result<()> {
        if self.tx.execute("DELETE FROM providers WHERE name = ?1", [name])? == 0 {
            return Err(BoardError::new(ErrorCode::NotFound, format!("没有模型供应商「{name}」")));
        }
        let all = self.read().provider_agents()?;
        for a in all.iter().filter(|a| a.provider == name) {
            let cleared = AgentProvider { agent: a.agent.clone(), updated_at: self.now, by: a.by.clone(), ..Default::default() };
            self.tx.execute(
                "UPDATE provider_agents SET spec = ?2, updated_at = ?3 WHERE agent = ?1",
                params![a.agent, serde_json::to_string(&cleared).unwrap_or_default(), self.now],
            )?;
        }
        self.hosts_changed = true;
        self.changed = true;
        Ok(())
    }

    /// 一个内置 agent 用哪个供应商、哪个模型、什么强度。供应商为空 = 不管
    pub fn provider_agent_put(&mut self, mut a: AgentProvider, by: &str) -> Result<AgentProvider> {
        a.agent = a.agent.trim().to_string();
        a.provider = a.provider.trim().to_string();
        a.model = a.model.trim().to_string();
        a.effort = a.effort.trim().to_string();
        if !AGENTS.iter().any(|(k, _, _)| *k == a.agent) {
            return Err(BoardError::usage(format!("不认识的 agent「{}」：只能是 {}", a.agent, AGENTS.map(|(k, _, _)| k).join("、"))));
        }
        if a.provider.is_empty() {
            a.model.clear();
            a.effort.clear();
        } else {
            let p = self
                .read()
                .provider(&a.provider)?
                .ok_or_else(|| BoardError::usage(format!("没有模型供应商「{}」", a.provider)))?;
            let fits = protocols_of(&a.agent);
            if !p.endpoints.iter().any(|e| fits.contains(&e.interface.as_str())) {
                return Err(BoardError::usage(format!(
                    "{} 用不了供应商 {}：它没有 {} 接口",
                    agent_name(&a.agent),
                    a.provider,
                    fits.iter().map(|x| interface_name(x)).collect::<Vec<_>>().join("、")
                )));
            }
            if a.model.is_empty() {
                a.model = p.models.first().cloned().unwrap_or_default();
            }
            if !p.models.iter().any(|m| m == &a.model) {
                return Err(BoardError::usage(format!("供应商 {} 里没有模型「{}」：只能是 {}", a.provider, a.model, p.models.join("、"))));
            }
            let efforts = efforts_of(&a.agent);
            if !a.effort.is_empty() && !efforts.contains(&a.effort.as_str()) {
                return Err(BoardError::usage(if efforts.is_empty() {
                    format!("{} 不能选思考强度", agent_name(&a.agent))
                } else {
                    format!("{} 不支持思考强度「{}」：只能是 {}", agent_name(&a.agent), a.effort, efforts.join("、"))
                }));
            }
        }
        a.updated_at = self.now;
        a.by = if by.trim().is_empty() { "人".into() } else { by.trim().into() };
        let old = self.read().provider_agents()?.into_iter().find(|x| x.agent == a.agent);
        if old.as_ref().is_some_and(|o| o.provider == a.provider && o.model == a.model && o.effort == a.effort) {
            return Ok(old.expect("上面看过"));
        }
        self.tx.execute(
            "INSERT INTO provider_agents (agent, spec, updated_at, by_name) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent) DO UPDATE SET spec = ?2, updated_at = ?3, by_name = ?4",
            params![a.agent, serde_json::to_string(&a).unwrap_or_default(), a.updated_at, a.by],
        )?;
        self.hosts_changed = true;
        self.changed = true;
        Ok(a)
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
    fn agent_assignments_are_checked_and_cleared_on_delete() {
        let mut s = Store::open_in_memory().unwrap();
        s.write(1000, Origin::ui(None), |w| w.provider_put(provider("p1"), "", "")).unwrap();
        s.write(1000, Origin::ui(None), |w| w.provider_put(Provider { name: "chat".into(), endpoints: vec![ProviderEndpoint { interface: "openai-chat".into(), url: "https://x".into() }], ..provider("chat") }, "", "")).unwrap();
        let set = |s: &mut Store, a: AgentProvider| s.write(2000, Origin::ui(None), |w| w.provider_agent_put(a, "octocat"));
        // 接口对不上
        let err = set(&mut s, AgentProvider { agent: "codex".into(), provider: "p1".into(), ..Default::default() }).unwrap_err();
        assert!(err.message.contains("接口"), "{}", err.message);
        // 正常的：模型留空时用第一个
        let out = set(&mut s, AgentProvider { agent: "claude".into(), provider: "p1".into(), ..Default::default() }).unwrap();
        assert_eq!((out.model.as_str(), out.effort.as_str()), ("model-a", ""));
        // 模型不在供应商里
        assert!(set(&mut s, AgentProvider { agent: "claude".into(), provider: "p1".into(), model: "nope".into(), ..Default::default() }).unwrap_err().message.contains("没有模型"));
        // 强度不对
        assert!(set(&mut s, AgentProvider { agent: "claude".into(), provider: "p1".into(), model: "model-a".into(), effort: "ultra".into(), ..Default::default() }).unwrap_err().message.contains("思考强度"));
        // 不认识的内置 agent
        assert!(set(&mut s, AgentProvider { agent: "mycli".into(), provider: "p1".into(), ..Default::default() }).unwrap_err().message.contains("不认识的 agent"));
        // 删掉供应商：用着它的 agent 回到不管
        s.write(3000, Origin::ui(None), |w| w.provider_rm("p1")).unwrap();
        let a = s.read(|r| r.provider_agents()).unwrap();
        assert_eq!((a[0].provider.as_str(), a[0].model.as_str()), ("", ""));
        assert!(s.read(|r| r.providers()).unwrap().iter().all(|p| p.name != "p1"));
        assert_eq!(s.write(3000, Origin::ui(None), |w| w.provider_rm("p1")).unwrap_err().code, ErrorCode::NotFound);
    }
}
