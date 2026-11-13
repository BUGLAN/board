//! MCP 服务器共用配置在本机这一侧（PRD §10.12「共用配置」）：board host 把服务端的 MCP 服务器写进本机 agent 的用户级配置。
//!
//! - Claude Code：`claude mcp add-json --scope user <名称> <JSON>`、`claude mcp remove --scope user <名称>`（在它的启动环境里执行；
//!   `~/.claude.json` 它自己经常改写，只读它判断已有哪些）
//! - Codex：改 `$CODEX_HOME/config.toml`（默认 `~/.codex`）的 `[mcp_servers.<名称>]`，保留别的配置和注释
//! - pi：改 `~/.pi/agent/mcp.json` 的 `mcpServers`
//! - dsh 和配置文件里的 agent：不支持，状态写「不支持」
//!
//! 只动同步加进去的条目：加过哪些记在 `~/.board/host/mcp-sync.json`（agent → 名称 → 版本）。配置里已有同名、不是同步加的，
//! 不覆盖，状态写「有同名，没有覆盖」；共用配置里删掉的（或不再同步给这个 agent 的），同步加的那条删掉。
//! WSL 里的 agent：文件经 `\\wsl.localhost\…` 改，Claude Code 的命令在 WSL 里执行。
//! 密钥只写进 agent 自己的配置，不打印、不写日志（输出里只有名称）。

use crate::envs::{self, Env};
use board_core::mcp::{files, McpServer, McpState, AGENTS};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 同步加过的条目：agent → 名称 → 版本
type Owned = BTreeMap<String, BTreeMap<String, String>>;

/// 一个要写 MCP 的 agent
pub struct Target {
    pub agent: String,
    pub env: Env,
    /// Claude Code 的命令（在这个启动环境里找到的）
    pub prog: Option<String>,
    /// 它的 MCP 配置文件（本机能读写的路径）和显示用的写法（见 [`file_of`]）
    pub file: PathBuf,
    pub shown: String,
}

/// 一次同步的结果
pub struct Synced {
    pub states: Vec<McpState>,
    pub changes: Vec<String>,
}

/// 一个 agent 的 MCP 配置文件：支持的 agent 才有；WSL 里的换成 Windows 上能访问的路径（取不到时 None）
pub fn file_of(agent: &str, env: &Env) -> Option<(PathBuf, String)> {
    let rel = match agent {
        "claude" => "~/.claude.json",
        "codex" => "$CODEX_HOME/config.toml",
        "pi" => "~/.pi/agent/mcp.json",
        _ => return None,
    };
    let p = if env.wsl().is_some() {
        // wsl_unc 认目录：取上一级再接文件名
        let (dir, name) = rel.rsplit_once('/').unwrap_or((rel, ""));
        envs::wsl_unc(env, dir)?.join(name)
    } else {
        envs::expand(rel)
    };
    Some((p, rel.to_string()))
}

/// 已有的条目名称
fn existing(t: &Target, path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let list = if t.agent == "codex" { files::read_codex(&text) } else { files::read_json(&text) };
    list.into_iter().map(|s| s.name).collect()
}

/// 改一个文件：读出来、改、先写临时文件再换上
fn rewrite(path: &Path, f: impl FnOnce(&str) -> board_core::Result<String>) -> Result<(), String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let out = f(&text).map_err(|e| e.message)?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("建不了 {}：{e}", d.display()))?;
    }
    let tmp = path.with_extension(format!("board-sync-{}", std::process::id()));
    std::fs::write(&tmp, out).map_err(|e| format!("写不了 {}：{e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("换不上 {}：{e}", path.display())
    })
}

/// 在 agent 的启动环境里执行 `claude mcp …`
fn claude(t: &Target, args: &[String]) -> Result<(), String> {
    let prog = t.prog.as_deref().ok_or("在它的启动环境里没找到 claude 命令")?;
    let mut c = t.env.command(prog, args, None, false);
    c.env_remove("CLAUDECODE");
    match envs::run_out(c, Duration::from_secs(60)) {
        Some(o) if o.ok => Ok(()),
        Some(o) => {
            let msg = [o.stderr.trim(), o.stdout.trim()].into_iter().find(|s| !s.is_empty()).unwrap_or("没有输出").lines().last().unwrap_or_default().to_string();
            Err(format!("claude mcp {} 失败：{msg}", args.get(1).map(String::as_str).unwrap_or("")))
        }
        None => Err("claude mcp 超过 60 秒没做完（或启动不了）".into()),
    }
}

fn s(v: &str) -> String {
    v.to_string()
}

fn put(t: &Target, path: &Path, srv: &McpServer, replace: bool) -> Result<(), String> {
    match t.agent.as_str() {
        "codex" => rewrite(path, |x| files::codex_upsert(x, srv)),
        "pi" => rewrite(path, |x| files::pi_upsert(x, srv)),
        _ => {
            if replace {
                let _ = claude(t, &[s("mcp"), s("remove"), s("--scope"), s("user"), srv.name.clone()]);
            }
            let json = files::json_entry(srv, true).to_string();
            claude(t, &[s("mcp"), s("add-json"), s("--scope"), s("user"), srv.name.clone(), json])
        }
    }
}

fn remove(t: &Target, path: &Path, name: &str) -> Result<(), String> {
    match t.agent.as_str() {
        "codex" => rewrite(path, |x| files::codex_remove(x, name)),
        "pi" => rewrite(path, |x| files::pi_remove(x, name)),
        _ => claude(t, &[s("mcp"), s("remove"), s("--scope"), s("user"), s(name)]),
    }
}

/// 按共用配置同步这些 agent。`unsupported`：本机找到、但不能同步 MCP 的 agent（写「不支持」）；`owned_path`：同步加过哪些
pub fn sync(lib: &[McpServer], targets: &[Target], unsupported: &[String], owned_path: &Path) -> Synced {
    let mut owned: Owned = std::fs::read_to_string(owned_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut states = vec![];
    let mut changes = vec![];
    for t in targets {
        let want: Vec<&McpServer> = lib.iter().filter(|x| x.for_agent(&t.agent)).collect();
        let (path, shown) = (&t.file, &t.shown);
        let names = existing(t, path);
        let mine = owned.entry(t.agent.clone()).or_default();
        for srv in &want {
            let state = |state: &str, why: Option<String>| McpState { server: srv.name.clone(), agent: t.agent.clone(), state: state.into(), why };
            let has = names.contains(&srv.name);
            let ours = mine.get(&srv.name).cloned();
            if has && ours.is_none() {
                states.push(state("local", Some(format!("{shown} 里已有同名的条目（不是同步加的），没有覆盖"))));
                continue;
            }
            if has && ours.as_deref() == Some(srv.version.as_str()) {
                states.push(state("synced", None));
                continue;
            }
            match put(t, path, srv, has) {
                Ok(()) => {
                    changes.push(format!("{} {} 的 MCP 服务器 {}", if has { "更新" } else { "加上" }, t.agent, srv.name));
                    mine.insert(srv.name.clone(), srv.version.clone());
                    states.push(state("synced", None));
                }
                Err(why) => states.push(state("failed", Some(why))),
            }
        }
        // 共用配置里删掉的、不再同步给这个 agent 的：同步加的那条删掉
        let gone: Vec<String> = mine.keys().filter(|n| !want.iter().any(|x| &&x.name == n)).cloned().collect();
        for name in gone {
            if names.contains(&name) {
                match remove(t, path, &name) {
                    Ok(()) => changes.push(format!("删掉 {} 的 MCP 服务器 {name}（共用配置里已经没有）", t.agent)),
                    Err(why) => {
                        states.push(McpState { server: name.clone(), agent: t.agent.clone(), state: "failed".into(), why: Some(format!("删不掉：{why}")) });
                        continue;
                    }
                }
            }
            mine.remove(&name);
        }
    }
    for a in unsupported {
        for srv in lib.iter().filter(|x| x.agents.is_empty()) {
            states.push(McpState { server: srv.name.clone(), agent: a.clone(), state: "unsupported".into(), why: Some(format!("{a} 还不能同步 MCP（支持 {}）", AGENTS.join("、"))) });
        }
    }
    if let Some(d) = owned_path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(owned_path, serde_json::to_string_pretty(&owned).unwrap_or_default());
    Synced { states, changes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use board_core::mcp::McpVar;

    fn srv(name: &str, cmd: &str, secret: &str) -> McpServer {
        McpServer {
            name: name.into(),
            kind: "stdio".into(),
            command: cmd.into(),
            args: vec!["--x".into()],
            env: vec![McpVar { key: "TOKEN".into(), value: Some(secret.into()), secret: true, tail: None }],
            url: String::new(),
            headers: vec![],
            agents: vec![],
            version: format!("v-{cmd}-{secret}"),
            updated_at: 0,
            by: String::new(),
            source: String::new(),
        }
    }

    /// Codex、pi 直接改文件：加上、更新、删掉；用户自己的、同名不是同步加的不动。HOME 指到临时目录（只在这个测试进程里）
    #[cfg(unix)]
    #[test]
    fn codex_and_pi_files() {
        let d = tempfile::tempdir().unwrap();
        let codex_home = d.path().join("codex");
        std::fs::create_dir_all(&codex_home).unwrap();
        std::fs::write(codex_home.join("config.toml"), "model = \"m\"\n[mcp_servers.mine]\ncommand = \"mine\"\n[mcp_servers.taken]\ncommand = \"user\"\n").unwrap();
        let t = vec![Target { agent: "codex".into(), env: Env::direct(), prog: None, file: codex_home.join("config.toml"), shown: "$CODEX_HOME/config.toml".into() }];
        let owned = d.path().join("owned.json");
        let out = sync(&[srv("db", "npx", "sk-1"), srv("taken", "x", "s")], &t, &["dsh".into()], &owned);
        let text = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
        assert!(text.contains("[mcp_servers.db]") && text.contains("TOKEN = \"sk-1\"") && text.contains("model = \"m\""), "{text}");
        assert!(text.contains("[mcp_servers.taken]\ncommand = \"user\""), "同名的不覆盖：{text}");
        let st = |out: &Synced, s: &str, a: &str| out.states.iter().find(|x| x.server == s && x.agent == a).map(|x| x.state.clone()).unwrap();
        assert_eq!((st(&out, "db", "codex"), st(&out, "taken", "codex")), ("synced".into(), "local".into()));
        assert_eq!(st(&out, "db", "dsh"), "unsupported");
        assert!(out.changes.iter().all(|c| !c.contains("sk-1")), "输出里没有密钥");
        // 没变：什么都不写
        let out = sync(&[srv("db", "npx", "sk-1"), srv("taken", "x", "s")], &t, &[], &owned);
        assert!(out.changes.is_empty());
        // 更新
        sync(&[srv("db", "npx", "sk-2")], &t, &[], &owned);
        assert!(std::fs::read_to_string(codex_home.join("config.toml")).unwrap().contains("TOKEN = \"sk-2\""));
        // 删掉：同步加的删掉，用户自己的 mine、taken 留着
        let out = sync(&[], &t, &[], &owned);
        let text = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
        assert!(!text.contains("[mcp_servers.db]") && text.contains("[mcp_servers.mine]") && text.contains("[mcp_servers.taken]"), "{text}");
        assert_eq!(out.changes.len(), 1);
        // pi 的 mcp.json 一样
        let pi = d.path().join("pi").join("mcp.json");
        let t = vec![Target { agent: "pi".into(), env: Env::direct(), prog: None, file: pi.clone(), shown: "~/.pi/agent/mcp.json".into() }];
        sync(&[srv("db", "npx", "sk-1")], &t, &[], &owned);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&pi).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["db"]["env"]["TOKEN"], "sk-1");
        sync(&[], &t, &[], &owned);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&pi).unwrap()).unwrap();
        assert!(v["mcpServers"].get("db").is_none());
    }
}
