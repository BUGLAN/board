//! MCP 服务器共用配置（PRD §10.12「共用配置」）：服务端存一份 MCP 服务器列表，各台机器的 board host 写进本机 agent 的用户级配置。
//!
//! - 每一项：名称、类型（本地命令 stdio：命令、参数、环境变量；远程 HTTP：地址、请求头）、同步给哪些 agent（空 = 所有支持 MCP 的内置 agent）。
//! - 环境变量、请求头的值可以标成「密钥」：只存在服务端；列表接口只给末尾 4 位（[`McpServer::masked`]），完整的值只经
//!   board host 的令牌接口（`GET /api/hosts/{名字}/mcp`）下发；改一项时密钥没填就保留原来的值。
//! - 版本号是全部内容（含密钥）的哈希前 12 位：密钥改了也会重新同步，哈希不泄露原文。
//! - 列表变了算机器有变化，在线的 board host 马上重新取任务。

use crate::store::{R, W};
use crate::{BoardError, ErrorCode, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const SCHEMA: &str = r#"
-- MCP 服务器共用配置
CREATE TABLE IF NOT EXISTS mcp_servers (
  name       TEXT PRIMARY KEY,
  spec       TEXT NOT NULL,             -- JSON：McpServer（含密钥原文）
  version    TEXT NOT NULL,             -- 内容的哈希
  updated_at INTEGER NOT NULL,
  by_name    TEXT NOT NULL,
  source     TEXT NOT NULL
);
"#;

/// 支持写 MCP 的内置 agent（默认同步给它们）
pub const AGENTS: [&str; 3] = ["claude", "codex", "pi"];

/// 一个环境变量或请求头
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpVar {
    pub key: String,
    /// 值；列表接口里密钥的值是空的（看 tail）；保存时密钥留空表示保留原来的值
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub secret: bool,
    /// 密钥的末尾 4 位（只在列表接口里有）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<String>,
}

/// 一个 MCP 服务器
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    /// stdio | http
    pub kind: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<McpVar>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: Vec<McpVar>,
    /// 同步给哪些 agent；空 = 所有支持的（[`AGENTS`]）
    #[serde(default)]
    pub agents: Vec<String>,
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
pub struct McpIndex {
    pub name: String,
    pub version: String,
}

/// 一台机器上一个 agent 里一个 MCP 服务器的同步状态（board host 上报）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpState {
    pub server: String,
    pub agent: String,
    /// synced 已同步 | local 有同名、不是同步加的，没有覆盖 | unsupported 不支持 | failed 失败
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// 名称：字母、数字、`_`、`-`，以字母或数字开头，最长 64 个字符（pi 只认这些字符）
pub fn check_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(BoardError::usage(format!("MCP 服务器名称「{name}」不对：只能用字母、数字、_、-，以字母或数字开头，最长 64 个字符")))
    }
}

fn tail(v: &str) -> String {
    let c: Vec<char> = v.chars().collect();
    c[c.len().saturating_sub(4)..].iter().collect()
}

impl McpServer {
    /// 给界面、命令行看的：密钥只留末尾 4 位
    pub fn masked(&self) -> McpServer {
        let mask = |vs: &[McpVar]| -> Vec<McpVar> {
            vs.iter()
                .map(|v| if v.secret { McpVar { key: v.key.clone(), value: None, secret: true, tail: v.value.as_deref().map(tail) } } else { McpVar { tail: None, ..v.clone() } })
                .collect()
        };
        McpServer { env: mask(&self.env), headers: mask(&self.headers), ..self.clone() }
    }

    /// 这个 agent 要不要同步
    pub fn for_agent(&self, agent: &str) -> bool {
        if self.agents.is_empty() {
            AGENTS.contains(&agent)
        } else {
            self.agents.iter().any(|a| a == agent)
        }
    }

    /// 内容的版本号（含密钥原文，不含版本号和元数据本身）
    pub fn content_version(&self) -> String {
        let body = serde_json::json!({ "kind": self.kind, "command": self.command, "args": self.args, "env": vars_raw(&self.env), "url": self.url, "headers": vars_raw(&self.headers), "agents": self.agents });
        Sha256::digest(body.to_string().as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
    }

    /// 校验：名称、类型、stdio 要命令、http 要地址、键不重复、值都有
    fn check(&self) -> Result<()> {
        check_name(&self.name)?;
        match self.kind.as_str() {
            "stdio" => {
                if self.command.trim().is_empty() {
                    return Err(BoardError::usage("本地命令（stdio）要填命令"));
                }
            }
            "http" => {
                if !(self.url.starts_with("https://") || self.url.starts_with("http://")) {
                    return Err(BoardError::usage("远程 HTTP 要填以 http:// 或 https:// 开头的地址"));
                }
            }
            k => return Err(BoardError::usage(format!("不认识的类型「{k}」：只能是 stdio、http"))),
        }
        for (what, vs) in [("环境变量", &self.env), ("请求头", &self.headers)] {
            let mut seen: Vec<&str> = vec![];
            for v in vs {
                let k = v.key.trim();
                if k.is_empty() || k.contains(['=', '\n', '"']) {
                    return Err(BoardError::usage(format!("{what}的名字不对：「{k}」")));
                }
                if seen.contains(&k) {
                    return Err(BoardError::usage(format!("{what}「{k}」写了两次")));
                }
                seen.push(k);
                if v.value.is_none() {
                    return Err(BoardError::usage(format!("{what}「{k}」还没有值")));
                }
            }
        }
        for a in &self.agents {
            if !AGENTS.contains(&a.as_str()) {
                return Err(BoardError::usage(format!("{a} 不支持同步 MCP：只能选 {}", AGENTS.join("、"))));
            }
        }
        Ok(())
    }
}

fn vars_raw(vs: &[McpVar]) -> Vec<(String, String, bool)> {
    vs.iter().map(|v| (v.key.clone(), v.value.clone().unwrap_or_default(), v.secret)).collect()
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<McpServer> {
    let spec: String = r.get(1)?;
    let mut s: McpServer = serde_json::from_str(&spec).unwrap_or(McpServer {
        name: String::new(),
        kind: "stdio".into(),
        command: String::new(),
        args: vec![],
        env: vec![],
        url: String::new(),
        headers: vec![],
        agents: vec![],
        version: String::new(),
        updated_at: 0,
        by: String::new(),
        source: String::new(),
    });
    s.name = r.get(0)?;
    s.version = r.get(2)?;
    s.updated_at = r.get(3)?;
    s.by = r.get(4)?;
    s.source = r.get(5)?;
    Ok(s)
}

impl R<'_> {
    /// 全部（含密钥原文：只给 board host 的令牌接口；界面用 [`McpServer::masked`]）
    pub fn mcp_servers(&self) -> Result<Vec<McpServer>> {
        let mut st = self.conn.prepare("SELECT name, spec, version, updated_at, by_name, source FROM mcp_servers ORDER BY name")?;
        let rows = st.query_map([], row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn mcp_index(&self) -> Result<Vec<McpIndex>> {
        Ok(self.mcp_servers()?.into_iter().map(|s| McpIndex { name: s.name, version: s.version }).collect())
    }

    fn mcp_server(&self, name: &str) -> Result<Option<McpServer>> {
        Ok(self.conn.query_row("SELECT name, spec, version, updated_at, by_name, source FROM mcp_servers WHERE name = ?1", [name], row).optional()?)
    }
}

impl W<'_> {
    /// 新增或修改一项。密钥留空（value 为空）时保留原来同名密钥的值。返回打过码的
    pub fn mcp_put(&mut self, mut s: McpServer, by: &str, source: &str) -> Result<McpServer> {
        s.name = s.name.trim().to_string();
        s.command = s.command.trim().to_string();
        s.url = s.url.trim().to_string();
        let old = self.read().mcp_server(&s.name)?;
        let keep = |vs: &mut Vec<McpVar>, olds: Option<&Vec<McpVar>>| {
            for v in vs.iter_mut() {
                v.key = v.key.trim().to_string();
                v.tail = None;
                if v.secret && v.value.as_deref().is_none_or(str::is_empty) {
                    v.value = olds.and_then(|o| o.iter().find(|x| x.key == v.key && x.secret)).and_then(|x| x.value.clone());
                }
            }
        };
        keep(&mut s.env, old.as_ref().map(|o| &o.env));
        keep(&mut s.headers, old.as_ref().map(|o| &o.headers));
        if s.kind == "stdio" {
            s.url.clear();
            s.headers.clear();
        } else {
            s.command.clear();
            s.args.clear();
            s.env.clear();
        }
        s.check()?;
        s.version = s.content_version();
        s.updated_at = self.now;
        s.by = if by.trim().is_empty() { "人".into() } else { by.trim().into() };
        s.source = if source.trim().is_empty() { "界面".into() } else { source.trim().into() };
        if old.as_ref().is_some_and(|o| o.version == s.version) {
            return Ok(old.expect("上面看过").masked());
        }
        self.tx.execute(
            "INSERT INTO mcp_servers (name, spec, version, updated_at, by_name, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name) DO UPDATE SET spec = ?2, version = ?3, updated_at = ?4, by_name = ?5, source = ?6",
            params![s.name, serde_json::to_string(&s).unwrap_or_default(), s.version, s.updated_at, s.by, s.source],
        )?;
        self.hosts_changed = true;
        self.changed = true;
        Ok(s.masked())
    }

    pub fn mcp_rm(&mut self, name: &str) -> Result<()> {
        if self.tx.execute("DELETE FROM mcp_servers WHERE name = ?1", [name])? == 0 {
            return Err(BoardError::new(ErrorCode::NotFound, format!("没有 MCP 服务器「{name}」")));
        }
        self.hosts_changed = true;
        self.changed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Origin, Store};

    fn var(k: &str, v: Option<&str>, secret: bool) -> McpVar {
        McpVar { key: k.into(), value: v.map(str::to_string), secret, tail: None }
    }

    fn stdio(env: Vec<McpVar>) -> McpServer {
        McpServer {
            name: "db".into(),
            kind: "stdio".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "db-mcp".into()],
            env,
            url: String::new(),
            headers: vec![],
            agents: vec![],
            version: String::new(),
            updated_at: 0,
            by: String::new(),
            source: String::new(),
        }
    }

    #[test]
    fn secrets_are_masked_kept_and_versioned() {
        let mut s = Store::open_in_memory().unwrap();
        let put = |s: &mut Store, x: McpServer| s.write(1000, Origin::ui(None), |w| w.mcp_put(x, "octocat", ""));
        let out = put(&mut s, stdio(vec![var("DB_URL", Some("postgres://x"), false), var("DB_TOKEN", Some("sk-123456789"), true)])).unwrap();
        // 返回给界面的打过码：密钥只有末尾 4 位
        let t = out.env.iter().find(|v| v.key == "DB_TOKEN").unwrap();
        assert_eq!((t.value.as_deref(), t.tail.as_deref()), (None, Some("6789")));
        assert_eq!(out.env[0].value.as_deref(), Some("postgres://x"));
        assert!(!serde_json::to_string(&out).unwrap().contains("sk-123456789"));
        // board host 拿到原文
        let full = s.read(|r| r.mcp_servers()).unwrap();
        assert_eq!(full[0].env[1].value.as_deref(), Some("sk-123456789"));
        let v1 = full[0].version.clone();
        // 改别的、密钥留空：保留原来的值，版本变了
        let out = put(&mut s, stdio(vec![var("DB_URL", Some("postgres://y"), false), var("DB_TOKEN", None, true)])).unwrap();
        assert_ne!(out.version, v1);
        assert_eq!(s.read(|r| r.mcp_servers()).unwrap()[0].env[1].value.as_deref(), Some("sk-123456789"));
        // 只改密钥：版本也变（哈希里有密钥，但不是原文）
        let v2 = out.version.clone();
        let v3 = put(&mut s, stdio(vec![var("DB_URL", Some("postgres://y"), false), var("DB_TOKEN", Some("sk-new"), true)])).unwrap().version;
        assert_ne!(v2, v3);
        assert!(!v3.contains("sk-new"));
        // 新加的密钥没有值：拒绝
        assert!(put(&mut s, stdio(vec![var("OTHER", None, true)])).unwrap_err().message.contains("还没有值"));
        // 删掉
        s.write(2000, Origin::ui(None), |w| w.mcp_rm("db")).unwrap();
        assert!(s.read(|r| r.mcp_servers()).unwrap().is_empty());
    }

    #[test]
    fn checks_and_agent_selection() {
        let mut s = Store::open_in_memory().unwrap();
        let put = |s: &mut Store, x: McpServer| s.write(1000, Origin::ui(None), |w| w.mcp_put(x, "", ""));
        assert!(put(&mut s, McpServer { name: "bad name".into(), ..stdio(vec![]) }).is_err());
        assert!(put(&mut s, McpServer { command: " ".into(), ..stdio(vec![]) }).is_err());
        assert!(put(&mut s, McpServer { kind: "sse".into(), ..stdio(vec![]) }).is_err());
        assert!(put(&mut s, McpServer { agents: vec!["dsh".into()], ..stdio(vec![]) }).is_err());
        let http = McpServer { kind: "http".into(), url: "https://example.com/mcp".into(), headers: vec![var("Authorization", Some("Bearer abc"), true)], ..stdio(vec![var("X", Some("1"), false)]) };
        let out = put(&mut s, http).unwrap();
        assert!(out.command.is_empty() && out.env.is_empty(), "http 不留 stdio 的字段");
        assert_eq!(out.headers[0].tail.as_deref(), Some(" abc"));
        assert!(out.for_agent("claude") && out.for_agent("pi") && !out.for_agent("dsh"));
        let only = McpServer { agents: vec!["codex".into()], ..stdio(vec![]) };
        assert!(only.for_agent("codex") && !only.for_agent("claude"));
    }
}

/// 各 agent 的 MCP 配置文件（PRD §10.12「共用配置」）：读出已有的条目（判断同名、桌面端「从本机导入」），改写 Codex、pi 的文件。
/// Claude Code 的 `~/.claude.json` 它自己经常改写，只读不写：写用 `claude mcp add-json / remove --scope user`（board-cli）。
/// - Claude Code：`~/.claude.json` 顶层 `mcpServers`：`{command, args, env}` 或 `{type: "http", url, headers}`
/// - Codex：`$CODEX_HOME/config.toml`（默认 `~/.codex`）的 `[mcp_servers.<名称>]`：`command`、`args`、`env` 或 `url`、`http_headers`
/// - pi：`~/.pi/agent/mcp.json` 的 `mcpServers`：`{command, args, env}` 或 `{url, headers}`
#[cfg(feature = "config")]
pub mod files {
    use super::{McpServer, McpVar};
    use crate::{BoardError, Result};
    use serde_json::{json, Map, Value};

    /// 导入时按名字猜哪些值是密钥
    pub fn looks_secret(key: &str) -> bool {
        let k = key.to_ascii_uppercase();
        ["TOKEN", "KEY", "SECRET", "PASSWORD", "PASSWD", "AUTH", "CREDENTIAL", "COOKIE", "SESSION"].iter().any(|m| k.contains(m))
    }

    fn vars(m: Option<&Map<String, Value>>) -> Vec<McpVar> {
        m.map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| McpVar { key: k.clone(), value: Some(s.to_string()), secret: looks_secret(k), tail: None }))
                .collect()
        })
        .unwrap_or_default()
    }

    fn blank(name: &str) -> McpServer {
        McpServer {
            name: name.into(),
            kind: "stdio".into(),
            command: String::new(),
            args: vec![],
            env: vec![],
            url: String::new(),
            headers: vec![],
            agents: vec![],
            version: String::new(),
            updated_at: 0,
            by: String::new(),
            source: String::new(),
        }
    }

    /// JSON 的 `mcpServers`（Claude Code、pi 一样的写法）；认不出的（sse、ws）跳过
    fn read_json_servers(v: &Value) -> Vec<McpServer> {
        let Some(m) = v.get("mcpServers").and_then(Value::as_object) else { return vec![] };
        let mut out = vec![];
        for (name, e) in m {
            let ty = e.get("type").and_then(Value::as_str).unwrap_or("");
            let mut s = blank(name);
            if let Some(cmd) = e.get("command").and_then(Value::as_str).filter(|_| ty.is_empty() || ty == "stdio") {
                s.command = cmd.into();
                s.args = e.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                s.env = vars(e.get("env").and_then(Value::as_object));
            } else if let Some(url) = e.get("url").and_then(Value::as_str).filter(|_| matches!(ty, "" | "http" | "streamable-http")) {
                s.kind = "http".into();
                s.url = url.into();
                s.headers = vars(e.get("headers").and_then(Value::as_object));
                for h in &mut s.headers {
                    h.secret = h.secret || h.key.eq_ignore_ascii_case("authorization");
                }
            } else {
                continue;
            }
            out.push(s);
        }
        out
    }

    /// Claude Code 的 `~/.claude.json`、pi 的 `~/.pi/agent/mcp.json`：已有的条目
    pub fn read_json(text: &str) -> Vec<McpServer> {
        serde_json::from_str::<Value>(text).map(|v| read_json_servers(&v)).unwrap_or_default()
    }

    /// Codex 的 `config.toml`：已有的条目
    pub fn read_codex(text: &str) -> Vec<McpServer> {
        let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else { return vec![] };
        let Some(t) = doc.get("mcp_servers").and_then(|x| x.as_table_like()) else { return vec![] };
        let strs = |i: Option<&toml_edit::Item>| -> Vec<String> { i.and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let map = |i: Option<&toml_edit::Item>| -> Vec<McpVar> {
            i.and_then(|x| x.as_table_like())
                .map(|t| t.iter().filter_map(|(k, v)| v.as_str().map(|s| McpVar { key: k.to_string(), value: Some(s.to_string()), secret: looks_secret(k), tail: None })).collect())
                .unwrap_or_default()
        };
        let mut out = vec![];
        for (name, e) in t.iter() {
            let Some(e) = e.as_table_like() else { continue };
            let mut s = blank(name);
            if let Some(cmd) = e.get("command").and_then(|x| x.as_str()) {
                s.command = cmd.into();
                s.args = strs(e.get("args"));
                s.env = map(e.get("env"));
            } else if let Some(url) = e.get("url").and_then(|x| x.as_str()) {
                s.kind = "http".into();
                s.url = url.into();
                s.headers = map(e.get("http_headers"));
            } else {
                continue;
            }
            out.push(s);
        }
        out
    }

    fn obj(vs: &[McpVar]) -> Value {
        Value::Object(vs.iter().map(|v| (v.key.clone(), Value::String(v.value.clone().unwrap_or_default()))).collect())
    }

    /// Claude Code、pi 的一项（`claude mcp add-json` 的 JSON、`mcp.json` 里的值）
    pub fn json_entry(s: &McpServer, typed: bool) -> Value {
        if s.kind == "http" {
            let mut v = json!({ "url": s.url });
            if typed {
                v["type"] = json!("http");
            }
            if !s.headers.is_empty() {
                v["headers"] = obj(&s.headers);
            }
            v
        } else {
            let mut v = json!({ "command": s.command, "args": s.args });
            if typed {
                v["type"] = json!("stdio");
            }
            if !s.env.is_empty() {
                v["env"] = obj(&s.env);
            }
            v
        }
    }

    /// pi 的 `mcp.json`：加上或换掉一项，保留别的
    pub fn pi_upsert(text: &str, s: &McpServer) -> Result<String> {
        let mut v: Value = if text.trim().is_empty() { json!({}) } else { serde_json::from_str(text).map_err(|e| BoardError::usage(format!("mcp.json 不是 JSON：{e}")))? };
        if !v.is_object() {
            return Err(BoardError::usage("mcp.json 不是 JSON 对象"));
        }
        if !v.get("mcpServers").is_some_and(Value::is_object) {
            v["mcpServers"] = json!({});
        }
        v["mcpServers"][&s.name] = json_entry(s, false);
        Ok(serde_json::to_string_pretty(&v).unwrap_or_default() + "\n")
    }

    /// pi 的 `mcp.json`：删掉一项
    pub fn pi_remove(text: &str, name: &str) -> Result<String> {
        let mut v: Value = serde_json::from_str(text).map_err(|e| BoardError::usage(format!("mcp.json 不是 JSON：{e}")))?;
        if let Some(m) = v.get_mut("mcpServers").and_then(Value::as_object_mut) {
            m.remove(name);
        }
        Ok(serde_json::to_string_pretty(&v).unwrap_or_default() + "\n")
    }

    /// Codex 的 `config.toml`：加上或换掉 `[mcp_servers.<名称>]`，保留文件里的其他配置和注释
    pub fn codex_upsert(text: &str, s: &McpServer) -> Result<String> {
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| BoardError::usage(format!("config.toml 解析不了：{e}")))?;
        if !doc.get("mcp_servers").is_some_and(|x| x.is_table()) {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            doc["mcp_servers"] = toml_edit::Item::Table(t);
        }
        let mut t = toml_edit::Table::new();
        let inline = |vs: &[McpVar]| {
            let mut it = toml_edit::InlineTable::new();
            for v in vs {
                it.insert(&v.key, v.value.clone().unwrap_or_default().into());
            }
            toml_edit::value(it)
        };
        if s.kind == "http" {
            t.insert("url", toml_edit::value(s.url.clone()));
            if !s.headers.is_empty() {
                t.insert("http_headers", inline(&s.headers));
            }
        } else {
            t.insert("command", toml_edit::value(s.command.clone()));
            let mut a = toml_edit::Array::new();
            for x in &s.args {
                a.push(x.as_str());
            }
            t.insert("args", toml_edit::value(a));
            if !s.env.is_empty() {
                t.insert("env", inline(&s.env));
            }
        }
        doc["mcp_servers"][&s.name] = toml_edit::Item::Table(t);
        Ok(doc.to_string())
    }

    /// Codex 的 `config.toml`：删掉 `[mcp_servers.<名称>]`
    pub fn codex_remove(text: &str, name: &str) -> Result<String> {
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| BoardError::usage(format!("config.toml 解析不了：{e}")))?;
        if let Some(t) = doc.get_mut("mcp_servers").and_then(|x| x.as_table_like_mut()) {
            t.remove(name);
        }
        Ok(doc.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn server(kind: &str) -> McpServer {
            let mut s = blank("db");
            s.kind = kind.into();
            if kind == "http" {
                s.url = "https://example.com/mcp".into();
                s.headers = vec![McpVar { key: "Authorization".into(), value: Some("Bearer t0k3n".into()), secret: true, tail: None }];
            } else {
                s.command = "npx".into();
                s.args = vec!["-y".into(), "db-mcp".into()];
                s.env = vec![McpVar { key: "DB_TOKEN".into(), value: Some("sk-1".into()), secret: true, tail: None }];
            }
            s
        }

        #[test]
        fn codex_keeps_other_config_and_round_trips() {
            let text = "# 我的配置\nmodel = \"gpt-5\"\n\n[mcp_servers.mine]\ncommand = \"my-mcp\"\n";
            let out = codex_upsert(text, &server("stdio")).unwrap();
            assert!(out.starts_with("# 我的配置\nmodel = \"gpt-5\""), "{out}");
            assert!(out.contains("[mcp_servers.db]\ncommand = \"npx\"\nargs = [\"-y\", \"db-mcp\"]\nenv = { DB_TOKEN = \"sk-1\" }"), "{out}");
            let back = read_codex(&out);
            assert_eq!(back.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["mine", "db"]);
            assert_eq!(back[1].args, ["-y", "db-mcp"]);
            assert!(back[1].env[0].secret, "名字像密钥的标成密钥");
            // 换成 http：旧的 command 不留
            let out = codex_upsert(&out, &server("http")).unwrap();
            assert!(out.contains("[mcp_servers.db]\nurl = \"https://example.com/mcp\"\nhttp_headers = { Authorization = \"Bearer t0k3n\" }"), "{out}");
            assert_eq!(read_codex(&out)[1].kind, "http");
            let out = codex_remove(&out, "db").unwrap();
            assert!(!out.contains("db") && out.contains("[mcp_servers.mine]"), "{out}");
            // 空文件也能写
            assert!(codex_upsert("", &server("stdio")).unwrap().contains("[mcp_servers.db]"));
        }

        #[test]
        fn json_configs_read_write() {
            let text = r#"{"theme":"dark","mcpServers":{"mine":{"command":"x"},"ws":{"type":"ws","url":"wss://a"},"remote":{"type":"http","url":"https://r","headers":{"Authorization":"Bearer a"}}}}"#;
            let got = read_json(text);
            assert_eq!(got.iter().map(|s| (s.name.as_str(), s.kind.as_str())).collect::<Vec<_>>(), [("mine", "stdio"), ("remote", "http")]);
            assert!(got[1].headers[0].secret);
            let out = pi_upsert(text, &server("stdio")).unwrap();
            let v: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(v["theme"], "dark");
            assert_eq!(v["mcpServers"]["db"], json!({ "command": "npx", "args": ["-y", "db-mcp"], "env": { "DB_TOKEN": "sk-1" } }));
            assert_eq!(v["mcpServers"]["mine"]["command"], "x");
            let out = pi_remove(&out, "db").unwrap();
            assert!(serde_json::from_str::<Value>(&out).unwrap()["mcpServers"].get("db").is_none());
            assert!(pi_upsert("", &server("http")).unwrap().contains("\"url\": \"https://example.com/mcp\""));
            // Claude Code 的 add-json 带 type
            assert_eq!(json_entry(&server("http"), true)["type"], "http");
            assert_eq!(json_entry(&server("stdio"), true)["type"], "stdio");
            assert!(pi_upsert("[1]", &server("stdio")).is_err());
        }
    }
}
