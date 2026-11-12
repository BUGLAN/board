//! 「共用配置 → MCP 服务器」的「从本机导入」（PRD §10.12「共用配置」）：读出这台电脑上各个 agent 已经配好的 MCP 服务器，
//! 页面上勾选后加进共用列表。读的是用户级配置（和 board host 同步写的是同一处）：
//! Claude Code `~/.claude.json` 顶层 `mcpServers`、Codex `$CODEX_HOME/config.toml`（默认 `~/.codex`）的 `mcp_servers`、
//! pi `~/.pi/agent/mcp.json` 的 `mcpServers`。名字像密钥的环境变量、请求头（TOKEN、KEY、SECRET、Authorization……）标成密钥，值一起带上。
//! 只读不写；同步加进去的条目（`~/.board/host/mcp-sync.json` 里记着的）不列，免得把共用配置自己再导一遍。

use board_core::mcp::{files, McpServer};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct Found {
    pub agent: String,
    pub server: McpServer,
}

fn home() -> Option<PathBuf> {
    board_core::config::home()
}

/// 本机各 agent 的 MCP 服务器（跳过同步加的）
pub fn local() -> Vec<Found> {
    let Some(h) = home() else { return vec![] };
    let codex = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| h.join(".codex"));
    let owned: serde_json::Value = std::fs::read_to_string(h.join(".board/host/mcp-sync.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut out = vec![];
    for (agent, path) in [("claude", h.join(".claude.json")), ("codex", codex.join("config.toml")), ("pi", h.join(".pi/agent/mcp.json"))] {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let list = if agent == "codex" { files::read_codex(&text) } else { files::read_json(&text) };
        for s in list {
            if owned[agent].get(&s.name).is_some() {
                continue;
            }
            out.push(Found { agent: agent.into(), server: s });
        }
    }
    out
}

/// 页面调用：本机 agent 已经配好的 MCP 服务器
#[tauri::command]
pub async fn mcp_local() -> Vec<Found> {
    tauri::async_runtime::spawn_blocking(local).await.unwrap_or_default()
}
