//! 同步配置里「模型」这一块在本机这一侧（机器详情与配置同步重构 §三）：board host 按机器上每个 agent 的选择
//! （不改 / 指定）把供应商的地址、API key、模型、强度写进本机 agent 的用户级配置文件。地址按指定的接口取
//! （agent 认几种、供应商也给了几种时机器上可以选；没选或对不上时用供应商里第一个这个 agent 认的）。
//!
//! - Claude Code：`~/.claude/settings.json` 的 `env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_AUTH_TOKEN`、`model`
//!   （有强度时再加 `env.CLAUDE_CODE_EFFORT_LEVEL`），文件里别的配置、顺序不变
//! - Codex：`$CODEX_HOME/config.toml`（默认 `~/.codex`）的 `model_provider`、`[model_providers.board-<供应商>]`
//!   （`name`、`base_url`、`wire_api = "responses"`、`experimental_bearer_token`）、`model`、`model_reasoning_effort`，
//!   保留别的配置、注释和顺序
//! - dsh：`~/.dsh/profiles/headless/cordis.patch.yml` 的 `llm-pi-ai` 里加一条 `board-<供应商>` 路由（标了强度的模型按
//!   DSH 的「档位: 线上写法」对象写上 `reasoningEfforts`）、`agent-default-model` 指向它；key 写 `~/.dsh/.env` 的 `BOARD_PROVIDER_<名称>_API_KEY`
//! - 配置文件里新加的 agent、WSL 里的 agent：不支持，状态写「不支持」
//!
//! 写之前检查：强度不在这个模型标的档位里（或这个 agent 写不了）就不写强度，结果里记下没写的那一档（`dropped`）；
//! 地址按服务端规范化后的用（Anthropic 的不带 `/v1`）。只改 Board 管的那几项，先写临时文件再整体换上；
//! 文件格式解析不了时不覆盖，上报「写入失败」和原因。选了「不改」的 agent 不碰；从「指定」改成「不改」时
//! 之前写进去的不恢复。API key 只写进 agent 自己的配置文件，不打印、不写日志（输出里只有供应商和模型名称）。
//!
//! [`current`] 读出这个 agent 配置文件里现在的模型和强度：board host 上报，派发名字和派发记录用它。

use crate::envs::{self, Env};
use board_core::hosts::{MODE_OFF, MODE_OWN};
use board_core::providers::{self, Provider, ProviderState};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 一个 agent 要改的配置文件
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Files {
    Claude { settings: PathBuf },
    Codex { config: PathBuf },
    Dsh { patch: PathBuf, env: PathBuf },
}

/// 一个要按服务器上的配置写的 agent
#[derive(Debug, Clone)]
pub struct Target {
    pub agent: String,
    /// `own` 指定 | `off` 不改（别的也当不改：旧版服务端发来的「用服务器的」不再认）
    pub mode: String,
    pub own: Own,
    pub files: Files,
}

/// 机器上给一个 agent 指定的那一组
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Own {
    pub provider: String,
    /// 空 = 供应商的第一个
    pub model: String,
    /// 空 = 不写
    pub effort: String,
    /// 用哪种接口；空 = 供应商里第一个这个 agent 认的
    pub protocol: String,
}

/// 一次同步的结果
#[derive(Debug, Default)]
pub struct Synced {
    pub states: Vec<ProviderState>,
    pub changes: Vec<String>,
}

/// 令牌接口取到的完整内容（含 API key 原文，只给这台机器用的令牌）
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Lib {
    #[serde(default)]
    pub providers: Vec<Provider>,
}

/// 一个 agent 的配置文件位置；WSL 里的 agent（返回 None）和配置文件里新加的 agent 不支持
pub fn files_of(agent: &str, env: &Env) -> Option<Files> {
    // WSL 里改文件要经 \\wsl.localhost\…，这一版先不做
    if env.wsl().is_some() || !board_core::agentdef::is_builtin(agent) {
        return None;
    }
    Some(match agent {
        "claude" => Files::Claude { settings: envs::expand("~/.claude/settings.json") },
        "codex" => Files::Codex { config: envs::expand("$CODEX_HOME/config.toml") },
        _ => Files::Dsh { patch: envs::expand("~/.dsh/profiles/headless/cordis.patch.yml"), env: envs::expand("~/.dsh/.env") },
    })
}

fn state(agent: &str, state: &str) -> ProviderState {
    ProviderState { agent: agent.into(), state: state.into(), ..Default::default() }
}

fn failed(agent: &str, why: String) -> ProviderState {
    ProviderState { why: Some(why), ..state(agent, "failed") }
}

/// 按机器上的选择写这些 agent。`unsupported`：本机找到、但不能改写的 agent（写「不支持」，why 是原因；选了「不改」的写「不改」）
pub fn sync(lib: &[Provider], targets: &[Target], unsupported: &[(String, String, String)]) -> Synced {
    let mut out = Synced::default();
    for t in targets {
        if t.mode != MODE_OWN {
            out.states.push(state(&t.agent, MODE_OFF));
            continue;
        }
        let Own { provider: name, model, effort, protocol } = t.own.clone();
        let Some(p) = lib.iter().find(|p| p.name == name) else {
            out.states.push(failed(&t.agent, format!("服务器上没有供应商「{name}」")));
            continue;
        };
        let Some(endpoint) = p.endpoint_for(&t.agent, &protocol) else {
            let want = providers::protocols_of(&t.agent).iter().map(|x| providers::interface_name(x)).collect::<Vec<_>>().join("、");
            out.states.push(failed(&t.agent, format!("{} 用不了供应商 {}：它没有 {want} 接口", providers::agent_name(&t.agent), p.name)));
            continue;
        };
        let endpoint = providers::ProviderEndpoint { interface: endpoint.interface.clone(), url: providers::normalize_url(&endpoint.interface, &endpoint.url) };
        let model = Some(model).filter(|m| p.models.contains(m)).or_else(|| p.models.first().cloned()).unwrap_or_default();
        // 强度不在这个模型标的档位里（或这个 agent 写不了）：不写
        let ok = providers::writable_efforts(&t.agent, p, &model);
        let (write_effort, dropped) = if effort.is_empty() || ok.contains(&effort) { (effort.clone(), String::new()) } else { (String::new(), effort.clone()) };
        let key = p.api_key.clone().unwrap_or_default();
        let done = ProviderState { agent: t.agent.clone(), state: "synced".into(), provider: p.name.clone(), model: model.clone(), effort: write_effort.clone(), dropped, why: None };
        match apply(&t.agent, &t.files, p, &endpoint, &model, &write_effort, &key) {
            Ok(true) => {
                out.changes.push(format!("把供应商 {} 写进 {}（模型 {model}{}）", p.name, t.agent, if write_effort.is_empty() { String::new() } else { format!("，强度 {write_effort}") }));
                out.states.push(done);
            }
            Ok(false) => out.states.push(done),
            Err(e) => out.states.push(ProviderState { state: "failed".into(), why: Some(e), ..done }),
        }
    }
    for (agent, mode, why) in unsupported {
        out.states.push(if mode == MODE_OFF { state(agent, MODE_OFF) } else { ProviderState { why: Some(why.clone()), ..state(agent, "unsupported") } });
    }
    out
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
        Files::Dsh { patch, .. } => {
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

/// 改一个文件：先写临时文件再整体换上
fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("建不了 {}：{e}", d.display()))?;
    }
    let tmp = path.with_extension(format!("board-provider-{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("写不了 {}：{e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("换不上 {}：{e}", path.display())
    })
}


/* ------------------------------ JSON 的几个小工具 ------------------------------ */

fn get_path<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |x, k| x.get(*k))
}

fn set_path(v: &mut Value, path: &[&str], val: Value) {
    let mut cur = v;
    for k in &path[..path.len() - 1] {
        if !cur.is_object() {
            *cur = json!({});
        }
        cur = cur.as_object_mut().expect("上面换成了对象").entry((*k).to_string()).or_insert_with(|| json!({}));
    }
    if !cur.is_object() {
        *cur = json!({});
    }
    cur.as_object_mut().expect("上面换成了对象").insert(path[path.len() - 1].to_string(), val);
}

fn remove_path(v: &mut Value, path: &[&str]) -> Option<Value> {
    let mut cur = v;
    for k in &path[..path.len() - 1] {
        cur = cur.get_mut(*k)?;
    }
    cur.as_object_mut()?.remove(path[path.len() - 1])
}

fn parse_json(text: &str, what: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{what} 不是 JSON：{e}"))?;
    if !v.is_object() {
        return Err(format!("{what} 不是 JSON 对象"));
    }
    Ok(v)
}

fn json_text(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default() + "\n"
}


fn codex_top(doc: &toml_edit::DocumentMut, key: &str) -> Option<String> {
    doc.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

/// 换掉顶层一项的值，保留它前面的注释和空行（toml_edit 换整个 Item 会把它们丢掉）
fn codex_set_top(doc: &mut toml_edit::DocumentMut, key: &str, val: &str) {
    match doc.get_mut(key) {
        Some(item) => {
            let decor = item.as_value().map(|v| v.decor().clone());
            *item = toml_edit::value(val);
            if let (Some(d), Some(v)) = (decor, item.as_value_mut()) {
                *v.decor_mut() = d;
            }
        }
        None => {
            doc.insert(key, toml_edit::value(val));
        }
    }
}

/// 把 `model_providers` 里 Board 加的表全删掉（供应商换了名字时）
fn codex_strip_tables(doc: &mut toml_edit::DocumentMut) {
    let Some(mp) = doc.get_mut("model_providers").and_then(|x| x.as_table_like_mut()) else { return };
    let ours: Vec<String> = mp.iter().map(|(k, _)| k.to_string()).filter(|k| k.starts_with("board-")).collect();
    for k in ours {
        mp.remove(&k);
    }
    if mp.is_empty() {
        doc.remove("model_providers");
    }
}

/// dsh 的 `llm-pi-ai` 路由里接口的写法（它用的 pi-ai 库的名字）
fn pi_ai_api(interface: &str) -> &'static str {
    match interface {
        "anthropic" => "anthropic-messages",
        "openai-responses" => "openai-responses",
        _ => "openai-completions",
    }
}


/* ------------------------------ dsh ------------------------------ */

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

/// `llm-pi-ai` 里 Board 加的路由（`board-*`）
fn dsh_strip_routes(lines: &mut Vec<String>) {
    loop {
        let Some((s, e)) = yaml_entry(lines, "llm-pi-ai") else { return };
        let Some((cs, _)) = yaml_child(lines, s, e, "config", indent_of(&lines[s]) + 2) else { return };
        let Some((ps, pe)) = yaml_child(lines, cs, e, "providers", indent_of(&lines[cs]) + 2) else { return };
        let pi = indent_of(&lines[ps]) + 2;
        let start = (ps + 1..pe).find(|i| indent_of(&lines[*i]) == pi && lines[*i].trim_start().starts_with("board-") && lines[*i].trim_end().ends_with(':'));
        let Some(start) = start else { return };
        let end = (start + 1..pe).find(|i| !lines[*i].trim().is_empty() && indent_of(&lines[*i]) <= pi).unwrap_or(pe);
        lines.drain(start..end);
    }
}

/// `agent-default-model` 的 `config` 下面把 `key` 设成 `val`（None = 删掉）；没有 config 时先补一个
fn dsh_set_default(lines: &mut Vec<String>, key: &str, val: Option<String>) {
    let Some((s, e)) = yaml_entry(lines, "agent-default-model") else { return };
    let ci = match yaml_child(lines, s, e, "config", indent_of(&lines[s]) + 2) {
        Some((i, _)) => i,
        None => {
            if val.is_none() {
                return;
            }
            lines.insert(e, "  config:".into());
            e
        }
    };
    let Some((_, e2)) = yaml_entry(lines, "agent-default-model") else { return };
    let cind = indent_of(&lines[ci]) + 2;
    match (yaml_child(lines, ci, e2, key, cind), val) {
        (Some((i, j)), Some(v)) => {
            lines.splice(i..j, [format!("{}{key}: {v}", " ".repeat(cind))]);
        }
        (Some((i, j)), None) => {
            lines.drain(i..j);
        }
        (None, Some(v)) => {
            lines.insert(ci + 1, format!("{}{key}: {v}", " ".repeat(cind)));
        }
        (None, None) => {}
    }
}


fn set_env_line(text: &str, key: &str, val: &str) -> String {
    let mut out: Vec<String> = vec![];
    let mut done = false;
    for l in text.lines() {
        if l.trim_start().starts_with(&format!("{key}=")) {
            out.push(format!("{key}={val}"));
            done = true;
        } else {
            out.push(l.to_string());
        }
    }
    if !done {
        out.push(format!("{key}={val}"));
    }
    out.join("\n") + "\n"
}

fn remove_env_line(text: &str, key: &str) -> String {
    if key.is_empty() {
        return text.to_string();
    }
    let out: Vec<String> = text.lines().filter(|l| !l.trim_start().starts_with(&format!("{key}="))).map(str::to_string).collect();
    if out.is_empty() {
        String::new()
    } else {
        out.join("\n") + "\n"
    }
}


/* ------------------------------ Claude Code ------------------------------ */

fn apply_claude(settings: &Path, endpoint: &providers::ProviderEndpoint, model: &str, effort: &str, key: &str) -> Result<bool, String> {
    let text = read(settings);
    let mut v = parse_json(&text, "settings.json")?;
    let keys: [(&str, Option<String>); 4] = [
        ("env.ANTHROPIC_BASE_URL", Some(endpoint.url.clone())),
        ("env.ANTHROPIC_AUTH_TOKEN", Some(key.to_string())),
        ("model", Some(model.to_string())),
        ("env.CLAUDE_CODE_EFFORT_LEVEL", (!effort.is_empty()).then(|| effort.to_string())),
    ];
    for (k, val) in keys {
        let path: Vec<&str> = k.split('.').collect();
        match val {
            Some(x) => set_path(&mut v, &path, Value::String(x)),
            None => {
                remove_path(&mut v, &path);
            }
        }
    }
    let out = json_text(&v);
    if out == text {
        return Ok(false);
    }
    write(settings, &out)?;
    Ok(true)
}

/* ------------------------------ Codex ------------------------------ */

fn apply_codex(config: &Path, p: &Provider, endpoint: &providers::ProviderEndpoint, model: &str, effort: &str, key: &str) -> Result<bool, String> {
    let text = read(config);
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("config.toml 解析不了：{e}"))?;
    let table_name = format!("board-{}", p.name);
    for (k, val) in [("model_provider", Some(table_name.clone())), ("model", Some(model.to_string())), ("model_reasoning_effort", (!effort.is_empty()).then(|| effort.to_string()))] {
        match val {
            Some(x) => codex_set_top(&mut doc, k, &x),
            None => {
                doc.remove(k);
            }
        }
    }
    codex_strip_tables(&mut doc);
    let table = doc.entry("model_providers").or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    if let Some(t) = table.as_table_like_mut() {
        let mut ours = toml_edit::Table::new();
        ours.insert("name", toml_edit::value(&p.name));
        ours.insert("base_url", toml_edit::value(&endpoint.url));
        ours.insert("wire_api", toml_edit::value("responses"));
        ours.insert("experimental_bearer_token", toml_edit::value(key));
        t.insert(&table_name, toml_edit::Item::Table(ours));
    }
    let out = doc.to_string();
    if out == text {
        return Ok(false);
    }
    write(config, &out)?;
    Ok(true)
}

/* ------------------------------ dsh ------------------------------ */

fn apply_dsh(patch: &Path, env_file: &Path, p: &Provider, endpoint: &providers::ProviderEndpoint, model: &str, effort: &str, key: &str) -> Result<bool, String> {
    let text = read(patch);
    let env_text = read(env_file);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let env_key = format!("BOARD_PROVIDER_{}_API_KEY", p.name.to_uppercase().replace(['-', '.'], "_"));

    dsh_strip_routes(&mut lines);
    if yaml_entry(&lines, "llm-pi-ai").is_none() {
        lines.push("- id: llm-pi-ai # board".into());
        lines.push("  name: \"@deepseek-ai/dsh-llm-pi-ai\"".into());
        lines.push("  config:".into());
        lines.push("    providers:".into());
    }
    let (s, e) = yaml_entry(&lines, "llm-pi-ai").expect("上面加了");
    let base = indent_of(&lines[s]);
    let ci = match yaml_child(&lines, s, e, "config", base + 2) {
        Some((i, _)) => i,
        None => {
            lines.insert(e, "  config:".into());
            e
        }
    };
    let Some((_, e)) = yaml_entry(&lines, "llm-pi-ai") else { return Err("llm-pi-ai 那一条不见了".into()) };
    let cind = indent_of(&lines[ci]);
    let pi = match yaml_child(&lines, ci, e, "providers", cind + 2) {
        Some((i, _)) => i,
        None => {
            lines.insert(ci + 1, format!("{}providers:", " ".repeat(cind + 2)));
            ci + 1
        }
    };
    let Some((_, e)) = yaml_entry(&lines, "llm-pi-ai") else { return Err("llm-pi-ai 那一条不见了".into()) };
    let pe = yaml_child(&lines, ci, e, "providers", indent_of(&lines[pi])).map(|(_, e)| e).unwrap_or(e);
    let pind = indent_of(&lines[pi]) + 2;
    let route = format!("board-{}", p.name);
    let mut block: Vec<String> = vec![
        format!("{}{}:", " ".repeat(pind), route),
        format!("{}displayName: {}", " ".repeat(pind + 2), yaml_str(&p.name)),
        format!("{}apiKeyEnv: {env_key}", " ".repeat(pind + 2)),
        format!("{}api: {}", " ".repeat(pind + 2), pi_ai_api(&endpoint.interface)),
        format!("{}baseURL: {}", " ".repeat(pind + 2), yaml_str(&endpoint.url)),
        format!("{}models:", " ".repeat(pind + 2)),
    ];
    for m in &p.models {
        block.push(format!("{}- id: {}", " ".repeat(pind + 4), yaml_str(m)));
        // 模型标了的强度、dsh 能写的：写上 reasoningEfforts（没标的不写，dsh 就不传强度）。
        // dsh 的 reasoningEfforts 是「档位: 线上写法」的对象，不是数组；off 留空表示不传参数。
        let ok = providers::writable_efforts("dsh", p, m);
        // 只有 off 时 dsh 会认为「没有可用的思考档位」而拒绝整条配置：干脆不写，按非推理模型处理。
        if ok.iter().any(|e| e != "off") {
            block.push(format!("{}reasoningEfforts:", " ".repeat(pind + 6)));
            for e in &ok {
                if e == "off" {
                    block.push(format!("{}\"off\":", " ".repeat(pind + 8)));
                } else {
                    block.push(format!("{}{}: {}", " ".repeat(pind + 8), yaml_str(e), yaml_str(e)));
                }
            }
        }
    }
    block.push(format!("{}streamIdleTimeoutMs: 600000", " ".repeat(pind + 2)));
    lines.splice(pe..pe, block);

    if yaml_entry(&lines, "agent-default-model").is_none() {
        lines.push("- id: agent-default-model # board".into());
        lines.push("  config:".into());
        lines.push("    provider:".into());
    }
    dsh_set_default(&mut lines, "reasoningEffort", (!effort.is_empty()).then(|| effort.to_string()));
    dsh_set_default(&mut lines, "model", Some(yaml_str(model)));
    dsh_set_default(&mut lines, "provider", Some(route));

    // .env 里 BOARD_PROVIDER_ 开头的都是 Board 写的：换供应商时别的清掉，只留这一个
    let mut env_out = env_text.clone();
    let stale: Vec<String> = env_text
        .lines()
        .filter_map(|l| l.trim_start().split_once('=').map(|(k, _)| k.trim().to_string()))
        .filter(|k| k.starts_with("BOARD_PROVIDER_") && *k != env_key)
        .collect();
    for k in stale {
        env_out = remove_env_line(&env_out, &k);
    }
    env_out = set_env_line(&env_out, &env_key, key);

    let patch_out = lines.join("\n") + "\n";
    let changed = patch_out != text || env_out != env_text;
    if patch_out != text {
        write(patch, &patch_out)?;
    }
    if env_out != env_text {
        write(env_file, &env_out)?;
    }
    Ok(changed)
}

/* ------------------------------ 入口 ------------------------------ */

fn apply(agent: &str, files: &Files, p: &Provider, endpoint: &providers::ProviderEndpoint, model: &str, effort: &str, key: &str) -> Result<bool, String> {
    match files {
        Files::Claude { settings } => apply_claude(settings, endpoint, model, effort, key),
        Files::Codex { config } => apply_codex(config, p, endpoint, model, effort, key),
        Files::Dsh { patch, env } => apply_dsh(patch, env, p, endpoint, model, effort, key),
    }
    .map_err(|e| format!("{}：{e}", providers::agent_name(agent)))
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

    /// 指定（供应商、模型、强度），接口不选
    fn target(agent: &str, own: (&str, &str, &str), files: Files) -> Target {
        Target { agent: agent.into(), mode: MODE_OWN.into(), own: Own { provider: own.0.into(), model: own.1.into(), effort: own.2.into(), protocol: String::new() }, files }
    }

    fn off(agent: &str, files: Files) -> Target {
        Target { agent: agent.into(), mode: MODE_OFF.into(), own: Own::default(), files }
    }

    fn read_json(p: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn claude_writes_env_model_and_effort_and_reads_back() {
        let d = tempfile::tempdir().unwrap();
        let settings = d.path().join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{\n  \"permissions\": {\"allow\": [\"Bash\"]},\n  \"model\": \"old-model\",\n  \"env\": {\"FOO\": \"1\"}\n}\n").unwrap();
        let files = Files::Claude { settings: settings.clone() };
        assert_eq!(current(&files), Some(("old-model".into(), String::new())));
        let t = target("claude", ("deepseek", "model-b", "high"), files.clone());
        let out = sync(&[provider("deepseek")], &[t.clone()], &[]);
        assert_eq!((out.states[0].state.as_str(), out.states[0].model.as_str(), out.states[0].effort.as_str()), ("synced", "model-b", "high"));
        let v = read_json(&settings);
        assert_eq!(v["model"], "model-b");
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://api.example.com", "Anthropic 的地址不带 /v1（#344）");
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-secret-1234");
        assert_eq!(v["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "high");
        assert_eq!((v["permissions"]["allow"][0].as_str(), v["env"]["FOO"].as_str()), (Some("Bash"), Some("1")));
        assert_eq!(current(&files), Some(("model-b".into(), "high".into())));
        assert!(out.changes.iter().all(|c| !c.contains("sk-secret")), "输出里没有密钥");
        // 模型不支持选的强度：不写强度，记下没写的那一档（#344）
        let out = sync(&[provider("deepseek")], &[target("claude", ("deepseek", "model-a", "high"), files.clone())], &[]);
        assert_eq!((out.states[0].effort.as_str(), out.states[0].dropped.as_str()), ("", "high"));
        assert!(read_json(&settings)["env"].get("CLAUDE_CODE_EFFORT_LEVEL").is_none());
        // 改成「不改」：不碰，之前写进去的留着
        let before = std::fs::read_to_string(&settings).unwrap();
        let out = sync(&[provider("deepseek")], &[off("claude", files.clone())], &[]);
        assert_eq!(out.states[0].state, "off");
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), before);
        // 旧版服务端发来的「用服务器的」：不再认，当不改
        let out = sync(&[provider("deepseek")], &[Target { mode: "server".into(), ..t.clone() }], &[]);
        assert_eq!(out.states[0].state, "off");
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), before);
        // 模型留空用第一个
        let out = sync(&[provider("deepseek")], &[target("claude", ("deepseek", "", ""), files.clone())], &[]);
        assert_eq!((out.states[0].state.as_str(), out.states[0].model.as_str()), ("synced", "model-a"), "模型留空用第一个");
        // 供应商没了
        let out = sync(&[], &[target("claude", ("gone", "", ""), files)], &[]);
        assert!(out.states[0].why.as_deref().unwrap().contains("没有供应商"));
    }

    #[test]
    fn claude_parse_failure_does_not_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let settings = d.path().join("settings.json");
        std::fs::write(&settings, "{ not json").unwrap();
        let t = target("claude", ("p", "", ""), Files::Claude { settings: settings.clone() });
        let out = sync(&[provider("p")], &[t], &[]);
        assert_eq!(out.states[0].state, "failed");
        assert!(out.states[0].why.as_deref().unwrap().contains("JSON"));
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ not json");
    }

    #[test]
    fn codex_keeps_comments_and_reads_back() {
        let d = tempfile::tempdir().unwrap();
        let config = d.path().join(".codex/config.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, "# 我的配置\nmodel = \"old-model\"\n\n[model_providers.other]\nname = \"other\"\nbase_url = \"https://other\"\n").unwrap();
        let files = Files::Codex { config: config.clone() };
        let out = sync(&[provider("p")], &[target("codex", ("p", "model-b", "high"), files.clone())], &[]);
        assert_eq!(out.states[0].state, "synced", "{:?}", out.states[0]);
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(text.contains("# 我的配置") && text.contains("[model_providers.other]"), "{text}");
        assert!(text.contains("model_provider = \"board-p\"") && text.contains("model_reasoning_effort = \"high\""), "{text}");
        assert!(text.contains("base_url = \"https://api.example.com/v1\""), "OpenAI 的地址照存的：{text}");
        assert_eq!(current(&files), Some(("model-b".into(), "high".into())));
    }

    #[test]
    fn dsh_writes_route_with_reasoning_efforts_and_env() {
        let d = tempfile::tempdir().unwrap();
        let (patch, env) = (d.path().join("cordis.patch.yml"), d.path().join(".env"));
        std::fs::write(&env, "OTHER=1\nBOARD_PROVIDER_OLD_API_KEY=sk-old\n").unwrap();
        let files = Files::Dsh { patch: patch.clone(), env: env.clone() };
        let mut p = provider("new-api");
        p.efforts.insert("model-b".into(), vec!["off".into(), "low".into(), "high".into(), "max".into()]);
        let out = sync(&[p.clone()], &[target("dsh", ("new-api", "model-b", "max"), files.clone())], &[]);
        assert_eq!(out.states[0].state, "synced", "{:?}", out.states[0]);
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("board-new-api:") && text.contains("apiKeyEnv: BOARD_PROVIDER_NEW_API_API_KEY"), "{text}");
        assert!(
            text.contains(
                "- id: \"model-b\"\n            reasoningEfforts:\n              \"off\":\n              \"low\": \"low\"\n              \"high\": \"high\"\n              \"max\": \"max\""
            ),
            "标了强度的模型写成档位到线上写法的对象：{text}"
        );
        assert!(!text.contains("- id: \"model-a\"\n            reasoningEfforts"), "没标的不写：{text}");
        assert!(text.contains("reasoningEffort: max") && text.contains("model: \"model-b\""), "{text}");
        let e = std::fs::read_to_string(&env).unwrap();
        assert!(e.contains("OTHER=1") && e.contains("BOARD_PROVIDER_NEW_API_API_KEY=sk-secret-1234") && !e.contains("sk-old"), "{e}");
        assert_eq!(current(&files), Some(("model-b".into(), "max".into())));
        // 再写一次：没有变化
        let out = sync(&[p], &[target("dsh", ("new-api", "model-b", "max"), files)], &[]);
        assert!(out.changes.is_empty(), "{:?}", out.changes);
    }

    #[test]
    fn unsupported_agents_report_off_or_unsupported() {
        let out = sync(&[], &[], &[("dsh".into(), "own".into(), "WSL 里的 agent 还不能改写配置文件".into()), ("codex".into(), "off".into(), String::new())]);
        assert_eq!((out.states[0].state.as_str(), out.states[1].state.as_str()), ("unsupported", "off"));
    }
}
