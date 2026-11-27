//! MCP 服务器共用配置在本机这一侧（PRD §10.16「共用配置」）：board host 把服务端的 MCP 服务器写进本机 agent 的用户级配置。
//!
//! - Claude Code：直接改 `~/.claude.json` 的 `mcpServers.<名称>`（它自己经常改写：读完、改完、写完后再重读确认，
//!   被它同时改写时重试一次；密钥不经过命令行，`ps` 里看不到）
//! - Codex：改 `$CODEX_HOME/config.toml`（默认 `~/.codex`）的 `[mcp_servers.<名称>]`，保留别的配置和注释
//! - dsh 和配置文件里的 agent：不支持，状态写「不支持」
//!
//! 机器上 MCP 选了「用服务器的」才同步（board host 启动时、收到「同步配置」请求时）；选「不改」时不碰，之前加的也不删。
//! 同步加过哪些记在 `~/.board/host/mcp-sync.json`（agent → 名称 → 版本）。配置里已有同名、不是同步加的：
//! 原来的值存进 `~/.board/host/mcp-backup.json`（agent → 名称 → 原来的条目），换成库里的，之后归 Board 管；
//! 共用配置里删掉的（或不再同步给这个 agent 的），同步加的那条删掉。
//! WSL 里的 agent：文件经 `\\wsl.localhost\…` 改。
//! 密钥只写进 agent 自己的配置，不打印、不写日志（输出里只有名称）。

use crate::envs::{self, Env};
use board_core::mcp::{files, McpServer, McpState};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 同步加过的条目：agent → 名称 → 版本
type Owned = BTreeMap<String, BTreeMap<String, String>>;

/// 一个要写 MCP 的 agent
pub struct Target {
    pub agent: String,
    /// 它的 MCP 配置文件（本机能读写的路径）和显示用的写法（见 [`file_of`]）
    pub file: PathBuf,
    pub shown: String,
}

/// 一次同步的结果
#[derive(Default)]
pub struct Synced {
    pub states: Vec<McpState>,
    pub changes: Vec<String>,
    /// 换掉了的本机同名条目（原来的值存进了备份文件）
    pub replaced: Vec<String>,
}

/// 原来的同名条目：agent → 名称 → 原来的值（读出来的样子）
type Backup = BTreeMap<String, BTreeMap<String, McpServer>>;

/// 下面按 agent 写配置文件的实现（[`put`]、[`remove`]）认得的 agent：`agentdef.rs` 里标了支持 MCP 的都要在这里
pub fn implemented(agent: &str) -> bool {
    matches!(agent, "claude" | "codex")
}

/// 这次从服务端取到的 MCP 密钥原文（标了「密钥」的环境变量和请求头）：日志上传前要按原文脱敏
pub fn secret_values(lib: &[McpServer]) -> Vec<String> {
    lib.iter()
        .flat_map(|s| s.env.iter().chain(&s.headers))
        .filter(|v| v.secret)
        .filter_map(|v| v.value.as_ref())
        .filter(|v| !v.trim().is_empty())
        .cloned()
        .collect()
}

/// 一个 agent 的 MCP 配置文件（`agentdef.rs` 里标的）：支持的 agent 才有；WSL 里的换成 Windows 上能访问的路径（取不到时 None）
pub fn file_of(agent: &str, env: &Env) -> Option<(PathBuf, String)> {
    let rel = board_core::agentdef::find(agent)?.mcp?;
    if !implemented(agent) {
        return None;
    }
    let p = if env.wsl().is_some() {
        // wsl_unc 认目录：取上一级再接文件名
        let (dir, name) = rel.rsplit_once('/').unwrap_or((rel, ""));
        envs::wsl_unc(env, dir)?.join(name)
    } else {
        envs::expand(rel)
    };
    Some((p, rel.to_string()))
}

/// 已有的条目
fn existing(t: &Target, path: &Path) -> Vec<McpServer> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if t.agent == "codex" {
        files::read_codex(&text)
    } else {
        files::read_json(&text)
    }
}

fn contents(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("读不了 {}：{e}", path.display())),
    }
}

fn temp_path(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "board".into());
    path.with_file_name(format!(".{name}.board-sync-{}-{seq}.tmp", std::process::id()))
}

/// 写临时文件；Unix 上从创建那一刻就是 `mode`（不会先按 umask 露出 0644）
fn write_temp(path: &Path, text: &str, mode: u32) -> Result<PathBuf, String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("建不了 {}：{e}", d.display()))?;
    }
    use std::io::Write;
    for _ in 0..8 {
        let tmp = temp_path(path);
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(mode);
        }
        let mut f = match o.open(&tmp) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("写不了 {}：{e}", tmp.display())),
        };
        let res = f.write_all(text.as_bytes()).and_then(|_| f.sync_all());
        drop(f);
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("写不了 {}：{e}", tmp.display()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode)) {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("设不了 {} 的权限：{e}", tmp.display()));
            }
        }
        return Ok(tmp);
    }
    Err(format!("建不了 {} 的临时文件", path.display()))
}

fn install(temp: &Path, path: &Path) -> Result<(), String> {
    std::fs::rename(temp, path).map_err(|e| {
        let _ = std::fs::remove_file(temp);
        format!("换不上 {}：{e}", path.display())
    })
}

fn mode_of(path: &Path) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).map(|m| m.permissions().mode() & 0o7777).unwrap_or(0o600)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0o600
    }
}

/// 原子地换掉一个文件，保留原文件的权限；没有原文件时新建的权限是 0600
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = write_temp(path, text, mode_of(path))?;
    install(&tmp, path)
}

/// 原子地写一个只有自己能读的文件（备份也要 0600，不能按 umask 变成 0644）
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    let tmp = write_temp(path, text, 0o600)?;
    install(&tmp, path)
}

/// 改一个文件：读出来、改、先写临时文件再换上
fn rewrite(path: &Path, f: impl FnOnce(&str) -> board_core::Result<String>) -> Result<(), String> {
    let text = contents(path)?;
    let out = f(&text).map_err(|e| e.message)?;
    write_atomic(path, &out)
}

/// Claude Code 会同时改 `~/.claude.json`：写完重读确认，不对就拿最新的再改一次
fn rewrite_verified(
    path: &Path,
    f: impl Fn(&str) -> board_core::Result<String>,
    okay: impl Fn(&str) -> bool,
) -> Result<(), String> {
    let mut text = contents(path)?;
    for attempt in 0..2 {
        let out = f(&text).map_err(|e| e.message)?;
        write_atomic(path, &out)?;
        let after = contents(path)?;
        if okay(&after) {
            return Ok(());
        }
        if attempt == 0 {
            text = after;
        }
    }
    Err(format!("写完后重读 {} 还是不对（可能被 Claude Code 同时改写了）", path.display()))
}

fn put(t: &Target, path: &Path, srv: &McpServer) -> Result<(), String> {
    match t.agent.as_str() {
        "codex" => rewrite(path, |x| files::codex_upsert(x, srv)),
        "claude" => rewrite_verified(
            path,
            |x| files::json_upsert(x, srv),
            |x| {
                serde_json::from_str::<serde_json::Value>(x)
                    .ok()
                    .and_then(|v| v.get("mcpServers").and_then(|s| s.get(&srv.name)).cloned())
                    .is_some_and(|got| got == files::json_entry(srv))
            },
        ),
        other => Err(format!("{other} 还没有写 MCP 配置的实现")),
    }
}

fn remove(t: &Target, path: &Path, name: &str) -> Result<(), String> {
    match t.agent.as_str() {
        "codex" => rewrite(path, |x| files::codex_remove(x, name)),
        "claude" => rewrite_verified(
            path,
            |x| files::json_remove(x, name),
            |x| serde_json::from_str::<serde_json::Value>(x).ok().is_some_and(|v| v.get("mcpServers").and_then(|s| s.get(name)).is_none()),
        ),
        other => Err(format!("{other} 还没有写 MCP 配置的实现")),
    }
}

/// 按共用配置同步这些 agent。`unsupported`：本机找到、但不能同步 MCP 的 agent（写「不支持」）；`owned_path`：同步加过哪些；
/// `backup_path`：换掉的同名条目原来的值存在这里
pub fn sync(lib: &[McpServer], targets: &[Target], unsupported: &[String], owned_path: &Path, backup_path: &Path) -> Synced {
    let mut owned: Owned = std::fs::read_to_string(owned_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut backup: Backup = std::fs::read_to_string(backup_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut backed = false;
    let mut states = vec![];
    let mut changes = vec![];
    let mut replaced: Vec<String> = vec![];
    for t in targets {
        let want: Vec<&McpServer> = lib.iter().filter(|x| x.for_agent(&t.agent)).collect();
        let (path, shown) = (&t.file, &t.shown);
        let have = existing(t, path);
        let names: Vec<String> = have.iter().map(|x| x.name.clone()).collect();
        let mine = owned.entry(t.agent.clone()).or_default();
        for srv in &want {
            let state = |state: &str, why: Option<String>| McpState { server: srv.name.clone(), agent: t.agent.clone(), state: state.into(), why };
            let has = names.contains(&srv.name);
            let ours = mine.get(&srv.name).cloned();
            if has && ours.as_deref() == Some(srv.version.as_str()) {
                states.push(state("synced", None));
                continue;
            }
            // 同名、不是同步加的：原来的值存进备份，再换成库里的
            let theirs = has && ours.is_none();
            if theirs {
                if let Some(old) = have.iter().find(|x| x.name == srv.name) {
                    backup.entry(t.agent.clone()).or_default().insert(srv.name.clone(), old.clone());
                    backed = true;
                }
            }
            match put(t, path, srv) {
                Ok(()) => {
                    if theirs {
                        changes.push(format!("{shown} 里已有同名的 {}（不是同步加的）：原来的存进了备份，换成共用配置里的", srv.name));
                        if !replaced.contains(&srv.name) {
                            replaced.push(srv.name.clone());
                        }
                    } else {
                        changes.push(format!("{} {} 的 MCP 服务器 {}", if has { "更新" } else { "加上" }, t.agent, srv.name));
                    }
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
            states.push(McpState { server: srv.name.clone(), agent: a.clone(), state: "unsupported".into(), why: Some(format!("{a} 还不能同步 MCP（支持 {}）", board_core::mcp::agents().join("、"))) });
        }
    }
    if let Some(d) = owned_path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = write_private(owned_path, &serde_json::to_string_pretty(&owned).unwrap_or_default());
    if backed {
        let _ = write_private(backup_path, &serde_json::to_string_pretty(&backup).unwrap_or_default());
    }
    Synced { states, changes, replaced }
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

    /// Codex 直接改文件：加上、更新、删掉；用户自己的不动，同名不是同步加的换成共用配置里的、原来的存进备份
    #[cfg(unix)]
    /// `agentdef.rs` 里标了支持 MCP 的 agent，这里都有写配置文件的实现、找得到配置文件；没标的都没有
    #[test]
    fn every_mcp_agent_in_the_description_has_a_writer() {
        for d in &board_core::agentdef::AGENT_DEFS {
            let file = file_of(d.id, &Env::direct());
            match d.mcp {
                Some(shown) => {
                    assert!(implemented(d.id), "{} 标了支持 MCP，但 mcpsync 里没有它的写法", d.id);
                    assert_eq!(file.map(|(_, s)| s).as_deref(), Some(shown), "{}", d.id);
                }
                None => assert!(file.is_none(), "{} 没标 MCP", d.id),
            }
        }
        assert!(file_of("nope", &Env::direct()).is_none());
    }

    #[test]
    fn codex_and_pi_files() {
        let d = tempfile::tempdir().unwrap();
        let codex_home = d.path().join("codex");
        std::fs::create_dir_all(&codex_home).unwrap();
        std::fs::write(codex_home.join("config.toml"), "model = \"m\"\n[mcp_servers.mine]\ncommand = \"mine\"\n[mcp_servers.taken]\ncommand = \"user\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(codex_home.join("config.toml"), std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let t = vec![Target { agent: "codex".into(), file: codex_home.join("config.toml"), shown: "$CODEX_HOME/config.toml".into() }];
        let owned = d.path().join("owned.json");
        let bk = d.path().join("mcp-backup.json");
        let out = sync(&[srv("db", "npx", "sk-1"), srv("taken", "x", "s")], &t, &["dsh".into()], &owned, &bk);
        let text = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
        assert!(text.contains("[mcp_servers.db]") && text.contains("TOKEN = \"sk-1\"") && text.contains("model = \"m\""), "{text}");
        assert!(!text.contains("command = \"user\"") && text.contains("[mcp_servers.taken]"), "同名的换成共用配置里的：{text}");
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&bk).unwrap()).unwrap();
        assert_eq!(saved["codex"]["taken"]["command"], "user", "原来的值存进了备份");
        assert_eq!(out.replaced, vec!["taken".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(codex_home.join("config.toml")).unwrap().permissions().mode() & 0o777, 0o600, "写配置不放大权限");
            assert_eq!(std::fs::metadata(&bk).unwrap().permissions().mode() & 0o777, 0o600, "备份只有自己能读");
        }
        let st = |out: &Synced, s: &str, a: &str| out.states.iter().find(|x| x.server == s && x.agent == a).map(|x| x.state.clone()).unwrap();
        assert_eq!((st(&out, "db", "codex"), st(&out, "taken", "codex")), ("synced".into(), "synced".into()));
        assert_eq!(st(&out, "db", "dsh"), "unsupported");
        assert!(out.changes.iter().all(|c| !c.contains("sk-1")), "输出里没有密钥");
        // 没变：什么都不写
        let out = sync(&[srv("db", "npx", "sk-1"), srv("taken", "x", "s")], &t, &[], &owned, &bk);
        assert!(out.changes.is_empty() && out.replaced.is_empty(), "换过一次就归 Board 管了");
        // 更新
        sync(&[srv("db", "npx", "sk-2"), srv("taken", "x", "s")], &t, &[], &owned, &bk);
        assert!(std::fs::read_to_string(codex_home.join("config.toml")).unwrap().contains("TOKEN = \"sk-2\""));
        // 删掉：同步加的删掉（包括换过来的 taken），用户自己的 mine 留着
        let out = sync(&[], &t, &[], &owned, &bk);
        let text = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
        assert!(!text.contains("[mcp_servers.db]") && text.contains("[mcp_servers.mine]") && !text.contains("[mcp_servers.taken]"), "{text}");
        assert_eq!(out.changes.len(), 2);
    }

    #[test]
    fn claude_is_written_directly_with_the_original_mode() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join(".claude.json");
        let t = vec![Target { agent: "claude".into(), file: file.clone(), shown: "~/.claude.json".into() }];
        let owned = d.path().join("owned.json");
        let bk = d.path().join("mcp-backup.json");
        let out = sync(&[srv("db", "npx", "sk-1")], &t, &[], &owned, &bk);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["db"], files::json_entry(&srv("db", "npx", "sk-1")));
        assert_eq!(out.states[0].state, "synced");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600, "新文件只有自己能读");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
            sync(&[srv("db", "npx", "sk-2")], &t, &[], &owned, &bk);
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o640, "保留原来的权限");
        }
        sync(&[], &t, &[], &owned, &bk);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert!(v["mcpServers"].get("db").is_none());
    }

    #[test]
    fn secret_values_are_the_marked_mcp_values() {
        let mut s = srv("db", "npx", "top-secret");
        s.env.push(McpVar { key: "PUBLIC".into(), value: Some("not-secret".into()), secret: false, tail: None });
        s.headers.push(McpVar { key: "Authorization".into(), value: Some("Bearer header-secret".into()), secret: true, tail: None });
        assert_eq!(secret_values(&[s]), ["top-secret", "Bearer header-secret"]);
    }
}
