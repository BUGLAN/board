//! 本机 agent 配置文件里的模型，以及按角色派发时给这一个进程换供应商（PRD §9.1「分工」）。
//!
//! - [`current`] 读出这个 agent 配置文件里现在的模型和强度（Claude Code `~/.claude/settings.json` 的 `model`、
//!   `env.CLAUDE_CODE_EFFORT_LEVEL`；Codex `$CODEX_HOME/config.toml` 的 `model`、`model_reasoning_effort`；dsh
//!   `~/.dsh/profiles/acp/cordis.patch.yml` 里 `agent-default-model` 的 `model`、`reasoningEffort`）：board host 上报，
//!   派发名字、派发记录和本机登录、没填模型的角色用它。
//! - [`route_for`] 按角色派发时怎么给这一个 agent 进程换上供应商（ACP `providers/set`，dsh 写这次的 `--patch`），
//!   不改 agent 的全局配置文件。
//!
//! 以前 board host 还按机器上「指定」的供应商、模型把地址和 key 写进这些配置文件（#576 去掉了）：已经写进去的不恢复，
//! 本机登录、没填模型的角色照旧用它；要换就在 agent 自己的配置里改，或者建一个用供应商的角色。
//! 配置文件里新加的 agent、WSL 里的 agent 不读（WSL 里的文件要经 `\\wsl.localhost\…`）。

use crate::envs::{self, Env};
use board_core::providers::{self, Provider};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 一个 agent 的配置文件
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Files {
    Claude { settings: PathBuf },
    Codex { config: PathBuf },
    Dsh { patch: PathBuf },
}

/// 令牌接口取到的完整内容（含 API key 原文，只给这台机器用的令牌）
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Lib {
    #[serde(default)]
    pub providers: Vec<Provider>,
}

/// 一个 agent 的配置文件位置；WSL 里的 agent（返回 None）和配置文件里新加的 agent 不读
pub fn files_of(agent: &str, env: &Env) -> Option<Files> {
    // WSL 里改文件要经 \\wsl.localhost\…，这一版先不做
    if env.wsl().is_some() || !board_core::agentdef::is_builtin(agent) {
        return None;
    }
    Some(match agent {
        "claude" => Files::Claude { settings: envs::expand("~/.claude/settings.json") },
        "codex" => Files::Codex { config: envs::expand("$CODEX_HOME/config.toml") },
        _ => Files::Dsh { patch: envs::expand("~/.dsh/profiles/acp/cordis.patch.yml") },
    })
}

/// 这个 agent 配置文件里现在的模型和强度（读不到、没写模型时为 None）
pub fn current(files: &Files) -> Option<(String, String)> {
    let s = |v: Option<&Value>| v.and_then(Value::as_str).map(|x| x.trim().to_string()).unwrap_or_default();
    let (model, effort) = match files {
        Files::Claude { settings } => {
            let v: Value = serde_json::from_str(&read(settings)).ok()?;
            (s(v.get("model")), s(get_path(&v, &["env", "CLAUDE_CODE_EFFORT_LEVEL"])))
        }
        Files::Codex { config } => {
            let doc: toml_edit::DocumentMut = read(config).parse().ok()?;
            (codex_top(&doc, "model").unwrap_or_default(), codex_top(&doc, "model_reasoning_effort").unwrap_or_default())
        }
        Files::Dsh { patch } => {
            let lines: Vec<String> = read(patch).lines().map(str::to_string).collect();
            let (st, e) = yaml_entry(&lines, "agent-default-model")?;
            let ci = yaml_child(&lines, st, e, "config", indent_of(&lines[st]) + 2).map(|(i, _)| indent_of(&lines[i]))?;
            let get = |k: &str| yaml_child(&lines, st, e, k, ci + 2).map(|(i, _)| yaml_raw(&lines[i], k).split(" #").next().unwrap_or_default().trim().trim_matches('"').to_string()).unwrap_or_default();
            (get("model"), get("reasoningEffort"))
        }
    };
    (!model.is_empty()).then_some((model, effort))
}

/* ------------------------------ 文件 ------------------------------ */

/// 读一个文件；不存在时是空
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/* ------------------------------ 读配置的几个小工具 ------------------------------ */

fn get_path<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |x, k| x.get(*k))
}

fn codex_top(doc: &toml_edit::DocumentMut, key: &str) -> Option<String> {
    doc.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

/// dsh 的 `llm-pi-ai` 路由里接口的写法（它用的 pi-ai 库的名字）
fn pi_ai_api(interface: &str) -> &'static str {
    match interface {
        "anthropic" => "anthropic-messages",
        "openai-responses" => "openai-responses",
        _ => "openai-completions",
    }
}


fn yaml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// 顶层 `- id: <id>` 那一条的范围（不含下一条；`# board` 标记也算同一条）
fn yaml_entry(lines: &[String], id: &str) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| indent_of(l) == 0 && l.trim_start().starts_with(&format!("- id: {id}")))?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| !l.trim().is_empty() && indent_of(l) == 0 && l.trim_start().starts_with("- "))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());
    Some((start, end))
}

/// 一个块里 `key:` 那一行和它的范围（到下一个缩进不比它深的非空行为止）
fn yaml_child(lines: &[String], from: usize, to: usize, key: &str, indent: usize) -> Option<(usize, usize)> {
    let start = (from..to).find(|i| indent_of(&lines[*i]) == indent && lines[*i].trim_start().starts_with(&format!("{key}:")))?;
    let end = (start + 1..to)
        .find(|i| !lines[*i].trim().is_empty() && indent_of(&lines[*i]) <= indent)
        .unwrap_or(to);
    Some((start, end))
}

/// 这一行 `key:` 后面的原文（含引号、注释）
fn yaml_raw(line: &str, key: &str) -> String {
    line.trim_start().strip_prefix(&format!("{key}:")).unwrap_or_default().trim().to_string()
}

/* ------------------------------ 按角色派发：只对这一个进程 ------------------------------ */

/// dsh 按角色派发时，供应商的 key 放在这个环境变量里（只给这一个进程）
pub const ROLE_KEY_ENV: &str = "BOARD_ROLE_API_KEY";

/// 按角色派发时给这一个 agent 进程换上供应商的办法（不改 agent 的全局配置文件）
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// ACP `providers/set` 的参数（Claude Code、Codex）：含 key，board host 写进 0600 的文件交给 `board acp`，读完就删
    Acp(Value),
    /// dsh：接在这次派发的 `--patch` 后面的几条（`llm-pi-ai` 里加一条路由，`acp` 用它和角色的模型），key 放在
    /// [`ROLE_KEY_ENV`] 里
    Dsh { patch: String, key: String },
}

/// 角色用的供应商在这个 agent 上怎么换：找这个 agent 认的接口；配置文件里新加的 agent、供应商没有它认的接口时报错
pub fn route_for(agent: &str, p: &Provider, model: &str, effort: &str) -> Result<Route, String> {
    let e = p.endpoint_for(agent, "").ok_or_else(|| format!("供应商「{}」没有 {} 认的接口", p.name, providers::agent_name(agent)))?;
    let url = providers::normalize_url(&e.interface, &e.url);
    let key = p.api_key.clone().unwrap_or_default();
    let bearer = json!({ "Authorization": format!("Bearer {key}") });
    match agent {
        // claude-agent-acp：providerId main、apiType anthropic；地址不带 /v1（Claude Code 自己接 /v1/messages）
        "claude" => Ok(Route::Acp(json!({ "providerId": "main", "apiType": "anthropic", "baseUrl": url, "headers": bearer }))),
        // codex-acp：providerId openai、apiType openai（wire_api = responses）
        "codex" => Ok(Route::Acp(json!({ "providerId": "openai", "apiType": "openai", "baseUrl": url, "headers": bearer }))),
        "dsh" => {
            let mut lines: Vec<String> = vec![
                "# Board：这次派发的角色用的供应商（只对这一个进程）".into(),
                "- id: llm-pi-ai".into(),
                "  name: \"@deepseek-ai/dsh-llm-pi-ai\"".into(),
                "  config:".into(),
                "    providers:".into(),
                "      board-role:".into(),
                format!("        displayName: {}", yaml_str(&p.name)),
                format!("        apiKeyEnv: {ROLE_KEY_ENV}"),
                format!("        api: {}", pi_ai_api(&e.interface)),
                format!("        baseURL: {}", yaml_str(&url)),
                "        models:".into(),
                format!("          - id: {}", yaml_str(model)),
            ];
            let ok = providers::writable_efforts("dsh", p, model);
            if ok.iter().any(|e| e != "off") {
                lines.push("            reasoningEfforts:".into());
                for e in &ok {
                    lines.push(if e == "off" { "              \"off\":".into() } else { format!("              {}: {}", yaml_str(e), yaml_str(e)) });
                }
            }
            lines.push("        streamIdleTimeoutMs: 600000".into());
            lines.push("- id: acp".into());
            lines.push("  config:".into());
            lines.push("    provider: board-role".into());
            lines.push(format!("    model: {}", yaml_str(model)));
            if !effort.is_empty() {
                lines.push(format!("    reasoningEffort: {}", yaml_str(effort)));
            }
            Ok(Route::Dsh { patch: lines.join("\n") + "\n", key })
        }
        _ => Err(format!("{} 是配置文件里加的 agent，不能按角色换供应商", providers::agent_name(agent))),
    }
}

/* ------------------------------ 测试 ------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;
    use board_core::providers::ProviderEndpoint;
    use std::collections::BTreeMap;

    fn provider(name: &str) -> Provider {
        Provider {
            name: name.into(),
            endpoints: vec![
                // 存着的地址带 /v1（旧数据）：写的时候也去掉
                ProviderEndpoint { interface: "anthropic".into(), url: "https://api.example.com/v1".into() },
                ProviderEndpoint { interface: "openai-chat".into(), url: "https://api.example.com/v1".into() },
                ProviderEndpoint { interface: "openai-responses".into(), url: "https://api.example.com/v1".into() },
            ],
            api_key: Some("sk-secret-1234".into()),
            models: vec!["model-a".into(), "model-b".into()],
            efforts: BTreeMap::from([("model-b".to_string(), vec!["low".to_string(), "high".into(), "max".into()])]),
            ..Default::default()
        }
    }

    /// 读回各 agent 配置文件里现在的模型和强度；没写模型、文件解析不了时为空
    #[test]
    fn current_reads_model_and_effort() {
        let d = tempfile::tempdir().unwrap();
        let settings = d.path().join("settings.json");
        std::fs::write(&settings, "{\n  \"model\": \"opus\",\n  \"env\": {\"CLAUDE_CODE_EFFORT_LEVEL\": \"high\"}\n}\n").unwrap();
        assert_eq!(current(&Files::Claude { settings: settings.clone() }), Some(("opus".into(), "high".into())));
        std::fs::write(&settings, "{ not json").unwrap();
        assert_eq!(current(&Files::Claude { settings }), None);
        let config = d.path().join("config.toml");
        std::fs::write(&config, "# 我的配置\nmodel = \"gpt-x\"\nmodel_reasoning_effort = \"medium\"\n").unwrap();
        assert_eq!(current(&Files::Codex { config }), Some(("gpt-x".into(), "medium".into())));
        let patch = d.path().join("cordis.patch.yml");
        std::fs::write(&patch, "- id: agent-default-model\n  config:\n    model: \"model-b\"\n    reasoningEffort: max\n").unwrap();
        assert_eq!(current(&Files::Dsh { patch: patch.clone() }), Some(("model-b".into(), "max".into())));
        std::fs::write(&patch, "[]\n").unwrap();
        assert_eq!(current(&Files::Dsh { patch }), None);
    }

    /// 按角色派发：Claude Code、Codex 走 ACP providers/set（地址规范化、key 在请求头），dsh 写这次的补丁、key 在环境变量
    #[test]
    fn role_routes_for_each_agent() {
        let p = provider("relay");
        let Route::Acp(c) = route_for("claude", &p, "model-a", "").unwrap() else { panic!() };
        assert_eq!(c, json!({ "providerId": "main", "apiType": "anthropic", "baseUrl": "https://api.example.com", "headers": { "Authorization": "Bearer sk-secret-1234" } }));
        let Route::Acp(x) = route_for("codex", &p, "model-a", "").unwrap() else { panic!() };
        assert_eq!((x["providerId"].as_str(), x["apiType"].as_str(), x["baseUrl"].as_str()), (Some("openai"), Some("openai"), Some("https://api.example.com/v1")));
        let Route::Dsh { patch, key } = route_for("dsh", &p, "model-b", "high").unwrap() else { panic!() };
        assert_eq!(key, "sk-secret-1234");
        assert!(patch.contains("apiKeyEnv: BOARD_ROLE_API_KEY") && patch.contains("- id: \"model-b\"") && patch.contains("reasoningEffort: \"high\""), "{patch}");
        assert!(!patch.contains("sk-secret"), "key 不进补丁文件");
        assert!(route_for("mycli", &p, "m", "").is_err());
        let only_chat = Provider { endpoints: vec![ProviderEndpoint { interface: "openai-chat".into(), url: "https://x/v1".into() }], ..provider("chat") };
        assert!(route_for("claude", &only_chat, "m", "").unwrap_err().contains("没有 Claude Code 认的接口"));
    }
}
