//! 模型供应商共用配置在本机这一侧（PRD §10.12「共用配置」）：board host 按设置把供应商的地址、API key、模型写进本机 agent 的用户级配置文件。
//!
//! - Claude Code：`~/.claude/settings.json` 的 `env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_AUTH_TOKEN`、`model`
//!   （有强度时再加 `env.CLAUDE_CODE_EFFORT_LEVEL`），文件里别的配置、顺序不变
//! - Codex：`$CODEX_HOME/config.toml`（默认 `~/.codex`）的 `model_provider`、`[model_providers.board-<供应商>]`
//!   （`name`、`base_url`、`wire_api = "responses"`、`experimental_bearer_token`）、`model`、`model_reasoning_effort`，
//!   保留别的配置、注释和顺序
//! - pi：`~/.pi/agent/models.json` 的 `providers.board-<供应商>`（`api`、`apiKey`、`baseUrl`、`models`）和
//!   `~/.pi/agent/settings.json` 的 `defaultProvider`、`defaultModel`（有强度时还有 `defaultThinkingLevel`）
//! - dsh：`~/.dsh/profiles/headless/cordis.patch.yml` 的 `llm-pi-ai` 里加一条 `board-<供应商>` 路由、
//!   `agent-default-model` 指向它；key 写 `~/.dsh/.env` 的 `BOARD_PROVIDER_<名称>_API_KEY`
//! - 配置文件里新加的 agent、WSL 里的 agent：不支持，状态写「不支持」
//!
//! 只改 Board 管的那几项。第一次改某个 agent 之前，把这几项原来的值备份到
//! `~/.board/host/agent-config/<agent>.json`；改成「不管」「本机自己管」、供应商被删除、接口对不上时按备份恢复
//! （原来没有的项删掉），然后删掉备份。先写临时文件再整体换上；文件格式解析不了时不覆盖，上报「失败」和原因。
//! API key 只写进 agent 自己的配置文件，不打印、不写日志（输出里只有供应商和模型名称）。

use crate::envs::{self, Env};
use board_core::providers::{self, AgentProvider, Provider, ProviderState, PROVIDER_LOCAL};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 一个 agent 要改的配置文件
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Files {
    Claude { settings: PathBuf },
    Codex { config: PathBuf },
    Pi { settings: PathBuf, models: PathBuf },
    Dsh { patch: PathBuf, env: PathBuf },
}

/// 一个要按共用配置改写的 agent
#[derive(Debug, Clone)]
pub struct Target {
    pub agent: String,
    /// 机器设置里的供应商：没有 = 跟随共用配置；`@local` = 本机自己管；其余是供应商名称
    pub run_provider: Option<String>,
    pub files: Files,
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
    #[serde(default)]
    pub agents: Vec<AgentProvider>,
}

/// 一个 agent 的配置文件位置；WSL 里的 agent（返回 None）和配置文件里新加的 agent 不支持
pub fn files_of(agent: &str, env: &Env) -> Option<Files> {
    // WSL 里改文件要经 \\wsl.localhost\…，这一版先不做
    if env.wsl().is_some() || !providers::AGENTS.iter().any(|(k, _, _)| *k == agent) {
        return None;
    }
    Some(match agent {
        "claude" => Files::Claude { settings: envs::expand("~/.claude/settings.json") },
        "codex" => Files::Codex { config: envs::expand("$CODEX_HOME/config.toml") },
        "pi" => Files::Pi { settings: envs::expand("~/.pi/agent/settings.json"), models: envs::expand("~/.pi/agent/models.json") },
        _ => Files::Dsh { patch: envs::expand("~/.dsh/profiles/headless/cordis.patch.yml"), env: envs::expand("~/.dsh/.env") },
    })
}

fn state(agent: &str, state: &str, provider: &str, model: &str, why: Option<String>) -> ProviderState {
    ProviderState { agent: agent.into(), state: state.into(), provider: provider.into(), model: model.into(), why }
}

/// 按共用配置同步这些 agent。`unsupported`：本机找到、但不能改写的 agent（写「不支持」，why 是原因）；
/// `backup_dir`：备份放在这里（每个 agent 一个文件）
pub fn sync(lib: &[Provider], agents: &[AgentProvider], targets: &[Target], unsupported: &[(String, String)], backup_dir: &Path) -> Synced {
    let mut out = Synced::default();
    for t in targets {
        let assigned = agents.iter().find(|a| a.agent == t.agent);
        let choice = match t.run_provider.as_deref() {
            Some(PROVIDER_LOCAL) => None,
            Some(name) => Some(name.to_string()),
            None => assigned.map(|a| a.provider.clone()).filter(|p| !p.is_empty()),
        };
        let Some(name) = choice else {
            let why = if t.run_provider.as_deref() == Some(PROVIDER_LOCAL) {
                "这台机器设置为本机自己管"
            } else {
                "共用配置里没有给这个 agent 指定供应商"
            };
            match restore(&t.agent, backup_dir) {
                Ok(true) => out.changes.push(format!("恢复 {} 原来的配置（{why}）", t.agent)),
                Ok(false) => {}
                Err(e) => {
                    out.states.push(state(&t.agent, "failed", "", "", Some(format!("恢复原来的配置没成：{e}"))));
                    continue;
                }
            }
            out.states.push(state(&t.agent, "local", "", "", Some(why.into())));
            continue;
        };
        let Some(p) = lib.iter().find(|p| p.name == name) else {
            let why = format!("共用配置里没有供应商「{name}」");
            if let Err(e) = restore(&t.agent, backup_dir) {
                out.states.push(state(&t.agent, "failed", "", "", Some(format!("{why}，恢复原来的配置也没成：{e}"))));
            } else {
                out.states.push(state(&t.agent, "failed", "", "", Some(why)));
            }
            continue;
        };
        let Some(endpoint) = p.endpoint_for(&t.agent) else {
            let why = format!(
                "{} 用不了供应商 {}：它没有 {} 接口",
                providers::agent_name(&t.agent),
                p.name,
                providers::protocols_of(&t.agent).iter().map(|x| providers::interface_name(x)).collect::<Vec<_>>().join("、")
            );
            match restore(&t.agent, backup_dir) {
                Ok(true) => out.changes.push(format!("恢复 {} 原来的配置（接口对不上）", t.agent)),
                Ok(false) => {}
                Err(e) => {
                    out.states.push(state(&t.agent, "failed", "", "", Some(format!("{why}；恢复原来的配置也没成：{e}"))));
                    continue;
                }
            }
            out.states.push(state(&t.agent, "unsupported", "", "", Some(why)));
            continue;
        };
        // 配置里写的模型：共用配置给这个供应商选的那个；机器设置里的模型只覆盖 Board 派发的这一次
        let model = assigned
            .filter(|a| a.provider == p.name)
            .map(|a| a.model.clone())
            .filter(|m| p.models.iter().any(|x| x == m))
            .or_else(|| p.models.first().cloned())
            .unwrap_or_default();
        let effort = assigned.filter(|a| a.provider == p.name).map(|a| a.effort.clone()).unwrap_or_default();
        let key = p.api_key.clone().unwrap_or_default();
        match apply(&t.agent, &t.files, p, endpoint, &model, &effort, &key, backup_dir) {
            Ok(true) => {
                out.changes.push(format!("把供应商 {} 写进 {}（模型 {model}）", p.name, t.agent));
                out.states.push(state(&t.agent, "synced", &p.name, &model, None));
            }
            Ok(false) => out.states.push(state(&t.agent, "synced", &p.name, &model, None)),
            Err(e) => out.states.push(state(&t.agent, "failed", &p.name, &model, Some(e))),
        }
    }
    for (agent, why) in unsupported {
        out.states.push(state(agent, "unsupported", "", "", Some(why.clone())));
    }
    out
}

/* ------------------------------ 备份 ------------------------------ */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "agent", rename_all = "camelCase")]
enum Backup {
    Claude {
        settings: PathBuf,
        file_existed: bool,
        env_existed: bool,
        /// `env.ANTHROPIC_BASE_URL` 这样的路径 → 原来的值
        values: BTreeMap<String, Option<Value>>,
    },
    Codex {
        config: PathBuf,
        file_existed: bool,
        /// 顶层项 → 原来的值
        values: BTreeMap<String, Option<String>>,
        /// `model_providers.board-x` 原来的表（原来没有时是 None）
        table: Option<BTreeMap<String, String>>,
        table_name: String,
    },
    Pi {
        settings: PathBuf,
        settings_existed: bool,
        settings_values: BTreeMap<String, Option<Value>>,
        models: PathBuf,
        models_existed: bool,
        /// 原来 `providers.board-<名字>` 那一项
        provider_name: Option<String>,
        provider: Option<Value>,
    },
    Dsh {
        patch: PathBuf,
        patch_existed: bool,
        env: PathBuf,
        env_existed: bool,
        /// Board 写过的环境变量 → 原来的值（None = 原来没有）
        env_vars: BTreeMap<String, Option<String>>,
        /// 原来 `agent-default-model` 的三项（原文，含引号）；None = 原来没有这一条
        default_entry: Option<DshDefault>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DshDefault {
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
}

fn backup_path(dir: &Path, agent: &str) -> PathBuf {
    dir.join(format!("{agent}.json"))
}

fn load_backup(dir: &Path, agent: &str) -> Option<Backup> {
    let text = std::fs::read_to_string(backup_path(dir, agent)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_backup(dir: &Path, agent: &str, b: &Backup) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("建不了 {}：{e}", dir.display()))?;
    let path = backup_path(dir, agent);
    std::fs::write(&path, serde_json::to_string_pretty(b).unwrap_or_default()).map_err(|e| format!("写不了 {}：{e}", path.display()))
}

fn drop_backup(dir: &Path, agent: &str) {
    let _ = std::fs::remove_file(backup_path(dir, agent));
}

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

/* ------------------------------ Claude Code ------------------------------ */

fn apply_claude(settings: &Path, endpoint: &providers::ProviderEndpoint, model: &str, effort: &str, key: &str, dir: &Path) -> Result<bool, String> {
    let text = read(settings);
    let mut v = parse_json(&text, "settings.json")?;
    let env_existed = get_path(&v, &["env"]).is_some();
    let mut values = BTreeMap::new();
    let keys: [(&str, Option<String>); 4] = [
        ("env.ANTHROPIC_BASE_URL", Some(endpoint.url.clone())),
        ("env.ANTHROPIC_AUTH_TOKEN", Some(key.to_string())),
        ("model", Some(model.to_string())),
        ("env.CLAUDE_CODE_EFFORT_LEVEL", (!effort.is_empty()).then(|| effort.to_string())),
    ];
    for (k, val) in keys {
        let path: Vec<&str> = k.split('.').collect();
        values.insert(k.to_string(), get_path(&v, &path).cloned());
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
    if load_backup(dir, "claude").is_none() {
        save_backup(dir, "claude", &Backup::Claude { settings: settings.into(), file_existed: settings.exists(), env_existed, values })?;
    }
    write(settings, &out)?;
    Ok(true)
}

fn restore_claude(b: &Backup, dir: &Path) -> Result<bool, String> {
    let Backup::Claude { settings, file_existed, env_existed, values } = b else { return Ok(false) };
    let mut changed = false;
    if !file_existed {
        if settings.exists() {
            std::fs::remove_file(settings).map_err(|e| format!("删不掉 {}：{e}", settings.display()))?;
            changed = true;
        }
    } else {
        let text = read(settings);
        let mut v = parse_json(&text, "settings.json")?;
        for (k, val) in values {
            let path: Vec<&str> = k.split('.').collect();
            match val {
                Some(x) => set_path(&mut v, &path, x.clone()),
                None => {
                    remove_path(&mut v, &path);
                }
            }
        }
        if !env_existed && get_path(&v, &["env"]).and_then(Value::as_object).is_some_and(|m: &Map<String, Value>| m.is_empty()) {
            remove_path(&mut v, &["env"]);
        }
        let out = json_text(&v);
        if out != text {
            write(settings, &out)?;
            changed = true;
        }
    }
    drop_backup(dir, "claude");
    Ok(changed)
}

/* ------------------------------ Codex ------------------------------ */

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

fn codex_table(doc: &toml_edit::DocumentMut, name: &str) -> Option<BTreeMap<String, String>> {
    let t = doc.get("model_providers")?.get(name)?.as_table_like()?;
    let mut out = BTreeMap::new();
    for (k, v) in t.iter() {
        if let Some(s) = v.as_str() {
            out.insert(k.to_string(), s.to_string());
        }
    }
    Some(out)
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

fn apply_codex(config: &Path, p: &Provider, endpoint: &providers::ProviderEndpoint, model: &str, effort: &str, key: &str, dir: &Path) -> Result<bool, String> {
    let text = read(config);
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("config.toml 解析不了：{e}"))?;
    let before_backup = load_backup(dir, "codex").is_none();
    let table_name = format!("board-{}", p.name);
    let old_table = if before_backup { codex_table(&doc, &table_name) } else { None };
    let mut values = BTreeMap::new();
    for (k, val) in [("model_provider", Some(table_name.clone())), ("model", Some(model.to_string())), ("model_reasoning_effort", (!effort.is_empty()).then(|| effort.to_string()))] {
        values.insert(k.to_string(), codex_top(&doc, k));
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
    if before_backup {
        save_backup(dir, "codex", &Backup::Codex { config: config.into(), file_existed: config.exists(), values, table: old_table, table_name })?;
    }
    write(config, &out)?;
    Ok(true)
}

fn restore_codex(b: &Backup, dir: &Path) -> Result<bool, String> {
    let Backup::Codex { config, file_existed, values, table, table_name } = b else { return Ok(false) };
    let mut changed = false;
    if !file_existed {
        if config.exists() {
            std::fs::remove_file(config).map_err(|e| format!("删不掉 {}：{e}", config.display()))?;
            changed = true;
        }
    } else {
        let text = read(config);
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("config.toml 解析不了：{e}"))?;
        for (k, val) in values {
            match val {
                Some(x) => codex_set_top(&mut doc, k, &x),
                None => {
                    doc.remove(k);
                }
            }
        }
        codex_strip_tables(&mut doc);
        if let Some(old) = table.clone() {
            let entry = doc.entry("model_providers").or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
            if let Some(t) = entry.as_table_like_mut() {
                let mut ours = toml_edit::Table::new();
                for (k, v) in old {
                    ours.insert(&k, toml_edit::value(v));
                }
                t.insert(table_name.as_str(), toml_edit::Item::Table(ours));
            }
        }
        let out = doc.to_string();
        if out != text {
            write(config, &out)?;
            changed = true;
        }
    }
    drop_backup(dir, "codex");
    Ok(changed)
}

/* ------------------------------ pi ------------------------------ */

fn pi_api(interface: &str) -> &'static str {
    match interface {
        "anthropic" => "anthropic-messages",
        "openai-responses" => "openai-responses",
        _ => "openai-completions",
    }
}

fn apply_pi(settings: &Path, models: &Path, p: &Provider, endpoint: &providers::ProviderEndpoint, model: &str, effort: &str, key: &str, dir: &Path) -> Result<bool, String> {
    let models_text = read(models);
    let mut mv = parse_json(&models_text, "models.json")?;
    let settings_text = read(settings);
    let mut sv = parse_json(&settings_text, "settings.json")?;
    let before_backup = load_backup(dir, "pi").is_none();
    let provider_key = format!("board-{}", p.name);
    let old_provider = if before_backup { get_path(&mv, &["providers", &provider_key]).cloned() } else { None };
    // 供应商换了名字：删掉以前 Board 写的那一项
    if let Some(t) = mv.get_mut("providers").and_then(Value::as_object_mut) {
        let ours: Vec<String> = t.keys().filter(|k| k.starts_with("board-")).cloned().collect();
        for k in ours {
            t.remove(&k);
        }
    }
    set_path(
        &mut mv,
        &["providers", &provider_key],
        json!({
            "api": pi_api(&endpoint.interface),
            "apiKey": key,
            "baseUrl": endpoint.url,
            "models": p.models.iter().map(|m| json!({ "id": m })).collect::<Vec<_>>(),
        }),
    );
    let mut values = BTreeMap::new();
    let keys: [(&str, Option<String>); 3] = [
        ("defaultProvider", Some(provider_key.clone())),
        ("defaultModel", Some(model.to_string())),
        ("defaultThinkingLevel", (!effort.is_empty()).then(|| effort.to_string())),
    ];
    for (k, val) in keys {
        values.insert(k.to_string(), get_path(&sv, &[k]).cloned());
        match val {
            Some(x) => set_path(&mut sv, &[k], Value::String(x)),
            None => {
                remove_path(&mut sv, &[k]);
            }
        }
    }
    let models_out = json_text(&mv);
    let settings_out = json_text(&sv);
    if before_backup && (models_out != models_text || settings_out != settings_text) {
        save_backup(
            dir,
            "pi",
            &Backup::Pi {
                settings: settings.into(),
                settings_existed: settings.exists(),
                settings_values: values,
                models: models.into(),
                models_existed: models.exists(),
                provider_name: old_provider.is_some().then_some(provider_key),
                provider: old_provider,
            },
        )?;
    }
    let mut changed = false;
    if models_out != models_text {
        write(models, &models_out)?;
        changed = true;
    }
    if settings_out != settings_text {
        write(settings, &settings_out)?;
        changed = true;
    }
    Ok(changed)
}

fn restore_pi(b: &Backup, dir: &Path) -> Result<bool, String> {
    let Backup::Pi { settings, settings_existed, settings_values, models, models_existed, provider_name, provider } = b else { return Ok(false) };
    let mut changed = false;
    if !models_existed {
        if models.exists() {
            std::fs::remove_file(models).map_err(|e| format!("删不掉 {}：{e}", models.display()))?;
            changed = true;
        }
    } else {
        let text = read(models);
        let mut v = parse_json(&text, "models.json")?;
        if let Some(t) = v.get_mut("providers").and_then(Value::as_object_mut) {
            let ours: Vec<String> = t.keys().filter(|k| k.starts_with("board-")).cloned().collect();
            for k in ours {
                t.remove(&k);
            }
            if t.is_empty() && provider.is_none() {
                v.as_object_mut().expect("上面是对象").remove("providers");
            }
        }
        if let (Some(name), Some(old)) = (provider_name, provider) {
            set_path(&mut v, &["providers", name], old.clone());
        }
        let out = json_text(&v);
        if out != text {
            write(models, &out)?;
            changed = true;
        }
    }
    if !settings_existed {
        if settings.exists() {
            std::fs::remove_file(settings).map_err(|e| format!("删不掉 {}：{e}", settings.display()))?;
            changed = true;
        }
    } else {
        let text = read(settings);
        let mut v = parse_json(&text, "settings.json")?;
        for (k, val) in settings_values {
            match val {
                Some(x) => set_path(&mut v, &[k], x.clone()),
                None => {
                    remove_path(&mut v, &[k]);
                }
            }
        }
        let out = json_text(&v);
        if out != text {
            write(settings, &out)?;
            changed = true;
        }
    }
    drop_backup(dir, "pi");
    Ok(changed)
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

fn apply_dsh(patch: &Path, env_file: &Path, p: &Provider, endpoint: &providers::ProviderEndpoint, model: &str, effort: &str, key: &str, dir: &Path) -> Result<bool, String> {
    let text = read(patch);
    let env_text = read(env_file);
    let before_backup = load_backup(dir, "dsh").is_none();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // 原来的 agent-default-model（第一次写时记进备份）
    let default_before = yaml_entry(&lines, "agent-default-model").map(|(s, e)| {
        let ci = yaml_child(&lines, s, e, "config", indent_of(&lines[s]) + 2).map(|(i, _)| indent_of(&lines[i]));
        let get = |k: &str| ci.and_then(|i| yaml_child(&lines, s, e, k, i + 2)).map(|(i, _)| yaml_raw(&lines[i], k));
        DshDefault { provider: get("provider"), model: get("model"), effort: get("reasoningEffort") }
    });
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
        format!("{}api: {}", " ".repeat(pind + 2), pi_api(&endpoint.interface)),
        format!("{}baseURL: {}", " ".repeat(pind + 2), yaml_str(&endpoint.url)),
        format!("{}models:", " ".repeat(pind + 2)),
    ];
    for m in &p.models {
        block.push(format!("{}- id: {}", " ".repeat(pind + 4), yaml_str(m)));
    }
    block.push(format!("{}streamIdleTimeoutMs: 600000", " ".repeat(pind + 2)));
    lines.splice(pe..pe, block);

    if yaml_entry(&lines, "agent-default-model").is_none() {
        lines.push("- id: agent-default-model # board".into());
        lines.push("  config:".into());
        lines.push("    provider:".into());
    }
    let route_line = route.clone();
    dsh_set_default(&mut lines, "reasoningEffort", (!effort.is_empty()).then(|| effort.to_string()));
    dsh_set_default(&mut lines, "model", Some(yaml_str(model)));
    dsh_set_default(&mut lines, "provider", Some(route_line));

    // 换供应商时把以前写的环境变量清掉（原来有值的恢复）
    let mut env_vars = match load_backup(dir, "dsh") {
        Some(Backup::Dsh { env_vars, .. }) => env_vars,
        _ => BTreeMap::new(),
    };
    let mut env_out = env_text.clone();
    for (k, old) in env_vars.iter().filter(|(k, _)| **k != env_key) {
        env_out = match old {
            Some(v) => set_env_line(&env_out, k, v),
            None => remove_env_line(&env_out, k),
        };
    }
    env_out = set_env_line(&env_out, &env_key, key);
    env_vars.entry(env_key).or_insert_with(|| env_value(&env_text, &format!("BOARD_PROVIDER_{}_API_KEY", p.name.to_uppercase().replace(['-', '.'], "_"))));

    let patch_out = lines.join("\n") + "\n";
    let changed = patch_out != text || env_out != env_text;
    if changed {
        let backup = match load_backup(dir, "dsh") {
            Some(Backup::Dsh { patch: bp, patch_existed, env: be, env_existed, default_entry, .. }) => {
                Backup::Dsh { patch: bp, patch_existed, env: be, env_existed, env_vars: env_vars.clone(), default_entry }
            }
            _ => Backup::Dsh {
                patch: patch.into(),
                patch_existed: patch.exists(),
                env: env_file.into(),
                env_existed: env_file.exists(),
                env_vars: env_vars.clone(),
                default_entry: default_before,
            },
        };
        save_backup(dir, "dsh", &backup)?;
    } else if before_backup {
        // 内容没变也不用备份
    }
    if patch_out != text {
        write(patch, &patch_out)?;
    }
    if env_out != env_text {
        write(env_file, &env_out)?;
    }
    Ok(changed)
}

fn restore_dsh(b: &Backup, dir: &Path) -> Result<bool, String> {
    let Backup::Dsh { patch, patch_existed, env, env_existed, env_vars, default_entry } = b else { return Ok(false) };
    let mut changed = false;
    if !patch_existed {
        if patch.exists() {
            std::fs::remove_file(patch).map_err(|e| format!("删不掉 {}：{e}", patch.display()))?;
            changed = true;
        }
    } else {
        let text = read(patch);
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        dsh_strip_routes(&mut lines);
        match default_entry {
            None => {
                // 这一条是 Board 加的：整条删掉
                if let Some((s, e)) = yaml_entry(&lines, "agent-default-model").filter(|(s, _)| lines[*s].contains("# board")) {
                    lines.drain(s..e);
                }
            }
            Some(old) => {
                dsh_set_default(&mut lines, "reasoningEffort", old.effort.clone());
                dsh_set_default(&mut lines, "model", old.model.clone());
                dsh_set_default(&mut lines, "provider", old.provider.clone());
            }
        }
        // Board 加的 llm-pi-ai 这一条：路由删完只剩空壳时整条删掉
        if let Some((s, e)) = yaml_entry(&lines, "llm-pi-ai").filter(|(s, _)| lines[*s].contains("# board")) {
            let skeleton = lines[s + 1..e].iter().all(|l| l.trim().is_empty() || matches!(l.trim(), "name: \"@deepseek-ai/dsh-llm-pi-ai\"" | "config:" | "providers:"));
            if skeleton {
                lines.drain(s..e);
            }
        }
        let out = lines.join("\n") + "\n";
        if out != text {
            write(patch, &out)?;
            changed = true;
        }
    }
    if !env_existed {
        if env.exists() {
            std::fs::remove_file(env).map_err(|e| format!("删不掉 {}：{e}", env.display()))?;
            changed = true;
        }
    } else {
        let text = read(env);
        let mut out = text.clone();
        for (k, old) in env_vars {
            out = match old {
                Some(v) => set_env_line(&out, k, v),
                None => remove_env_line(&out, k),
            };
        }
        if out != text {
            write(env, &out)?;
            changed = true;
        }
    }
    drop_backup(dir, "dsh");
    Ok(changed)
}

/// `.env` 里这一项原来的值
fn env_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| l.trim().strip_prefix(&format!("{key}=")).map(|v| v.to_string()))
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

/* ------------------------------ 入口 ------------------------------ */

fn apply(agent: &str, files: &Files, p: &Provider, endpoint: &providers::ProviderEndpoint, model: &str, effort: &str, key: &str, dir: &Path) -> Result<bool, String> {
    match files {
        Files::Claude { settings } => apply_claude(settings, endpoint, model, effort, key, dir),
        Files::Codex { config } => apply_codex(config, p, endpoint, model, effort, key, dir),
        Files::Pi { settings, models } => apply_pi(settings, models, p, endpoint, model, effort, key, dir),
        Files::Dsh { patch, env } => apply_dsh(patch, env, p, endpoint, model, effort, key, dir),
    }
    .map_err(|e| format!("{}：{e}", providers::agent_name(agent)))
}

/// 按备份恢复这个 agent 原来的这几项；没有备份时什么也不做。返回有没有改动文件
fn restore(agent: &str, dir: &Path) -> Result<bool, String> {
    let Some(b) = load_backup(dir, agent) else { return Ok(false) };
    match &b {
        Backup::Claude { .. } => restore_claude(&b, dir),
        Backup::Codex { .. } => restore_codex(&b, dir),
        Backup::Pi { .. } => restore_pi(&b, dir),
        Backup::Dsh { .. } => restore_dsh(&b, dir),
    }
    .map_err(|e| format!("{}：{e}", providers::agent_name(agent)))
}

/* ------------------------------ 测试 ------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;
    use board_core::providers::ProviderEndpoint;

    fn provider(name: &str) -> Provider {
        Provider {
            name: name.into(),
            endpoints: vec![
                ProviderEndpoint { interface: "anthropic".into(), url: "https://api.example.com".into() },
                ProviderEndpoint { interface: "openai-chat".into(), url: "https://api.example.com/v1".into() },
                ProviderEndpoint { interface: "openai-responses".into(), url: "https://api.example.com/v1".into() },
            ],
            api_key: Some("sk-secret-1234".into()),
            models: vec!["model-a".into(), "model-b".into()],
            ..Default::default()
        }
    }

    fn agents(agent: &str, provider: &str, model: &str, effort: &str) -> Vec<AgentProvider> {
        vec![AgentProvider { agent: agent.into(), provider: provider.into(), model: model.into(), effort: effort.into(), ..Default::default() }]
    }

    fn read_json(p: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn claude_writes_env_model_and_restores() {
        let d = tempfile::tempdir().unwrap();
        let settings = d.path().join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let original = "{\n  \"permissions\": {\"allow\": [\"Bash\"]},\n  \"model\": \"old-model\",\n  \"env\": {\"FOO\": \"1\"}\n}\n";
        std::fs::write(&settings, original).unwrap();
        let bak = d.path().join("backup");
        let target = Target { agent: "claude".into(), run_provider: None, files: Files::Claude { settings: settings.clone() } };
        let out = sync(&[provider("deepseek")], &agents("claude", "deepseek", "model-b", "high"), &[target.clone()], &[], &bak);
        assert_eq!((out.states[0].state.as_str(), out.states[0].model.as_str()), ("synced", "model-b"));
        let v = read_json(&settings);
        assert_eq!(v["model"], "model-b");
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://api.example.com");
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-secret-1234");
        assert_eq!(v["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "high");
        assert_eq!(v["permissions"]["allow"][0], "Bash");
        assert!(bak.join("claude.json").exists());
        let out = sync(&[], &[], &[target], &[], &bak);
        assert_eq!(out.states[0].state, "local");
        let v = read_json(&settings);
        assert_eq!(v["model"], "old-model");
        assert_eq!(v["env"]["FOO"], "1");
        assert!(v["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
        assert!(!bak.join("claude.json").exists());
    }

    #[test]
    fn claude_parse_failure_does_not_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let settings = d.path().join("settings.json");
        std::fs::write(&settings, "{ not json").unwrap();
        let bak = d.path().join("backup");
        let target = Target { agent: "claude".into(), run_provider: None, files: Files::Claude { settings: settings.clone() } };
        let out = sync(&[provider("p")], &agents("claude", "p", "", ""), &[target], &[], &bak);
        assert_eq!(out.states[0].state, "failed");
        assert!(out.states[0].why.as_deref().unwrap().contains("JSON"));
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ not json");
        assert!(!bak.join("claude.json").exists());
    }

    #[test]
    fn codex_keeps_comments_and_restores() {
        let d = tempfile::tempdir().unwrap();
        let config = d.path().join(".codex/config.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        let original = "# 我的配置\nmodel = \"old-model\"\n\n[model_providers.other]\nname = \"other\"\nbase_url = \"https://other\"\n";
        std::fs::write(&config, original).unwrap();
        let bak = d.path().join("backup");
        let target = Target { agent: "codex".into(), run_provider: None, files: Files::Codex { config: config.clone() } };
        let out = sync(&[provider("p")], &agents("codex", "p", "model-a", "medium"), &[target.clone()], &[], &bak);
        assert_eq!(out.states[0].state, "synced", "{:?}", out.states[0]);
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(text.contains("# 我的配置"), "{text}");
        assert!(text.contains("[model_providers.other]"), "{text}");
        assert!(text.contains("model_provider = \"board-p\""), "{text}");
        assert!(text.contains("model_reasoning_effort = \"medium\""), "{text}");
        assert!(text.contains("experimental_bearer_token = \"sk-secret-1234\""), "{text}");
        assert!(text.contains("wire_api = \"responses\""), "{text}");
        let out = sync(&[], &[], &[target], &[], &bak);
        assert_eq!(out.states[0].state, "local");
        let text = std::fs::read_to_string(&config).unwrap();
        assert_eq!(text, original);
        assert!(!bak.join("codex.json").exists());
    }

    #[test]
    fn codex_parse_failure_does_not_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let config = d.path().join("config.toml");
        std::fs::write(&config, "model = [").unwrap();
        let bak = d.path().join("backup");
        let target = Target { agent: "codex".into(), run_provider: None, files: Files::Codex { config: config.clone() } };
        let out = sync(&[provider("p")], &agents("codex", "p", "", ""), &[target], &[], &bak);
        assert_eq!(out.states[0].state, "failed");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "model = [");
    }

    #[test]
    fn pi_writes_models_and_defaults_then_restores() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".pi/agent");
        std::fs::create_dir_all(&dir).unwrap();
        let models = dir.join("models.json");
        let settings = dir.join("settings.json");
        std::fs::write(&models, "{\"providers\":{\"mine\":{\"api\":\"openai-completions\",\"apiKey\":\"k\",\"models\":[{\"id\":\"x\"}]}}}\n").unwrap();
        std::fs::write(&settings, "{\"theme\":\"dark\",\"defaultProvider\":\"mine\"}\n").unwrap();
        let bak = d.path().join("backup");
        let target = Target { agent: "pi".into(), run_provider: None, files: Files::Pi { settings: settings.clone(), models: models.clone() } };
        let out = sync(&[provider("p")], &agents("pi", "p", "", "low"), &[target.clone()], &[], &bak);
        assert_eq!(out.states[0].state, "synced", "{:?}", out.states[0]);
        let v = read_json(&models);
        assert_eq!(v["providers"]["board-p"]["api"], "anthropic-messages");
        assert_eq!(v["providers"]["board-p"]["models"][1]["id"], "model-b");
        assert_eq!(v["providers"]["mine"]["apiKey"], "k");
        let s = read_json(&settings);
        assert_eq!((s["defaultProvider"].as_str(), s["defaultModel"].as_str()), (Some("board-p"), Some("model-a")));
        assert_eq!(s["defaultThinkingLevel"], "low");
        assert_eq!(s["theme"], "dark");
        let out = sync(&[], &[], &[target], &[], &bak);
        assert_eq!(out.states[0].state, "local");
        let v = read_json(&models);
        assert!(v["providers"].get("board-p").is_none());
        assert_eq!(v["providers"]["mine"]["apiKey"], "k");
        let s = read_json(&settings);
        assert_eq!((s["defaultProvider"].as_str(), s["theme"].as_str()), (Some("mine"), Some("dark")));
        assert!(s.get("defaultModel").is_none());
        assert!(!bak.join("pi.json").exists());
    }

    #[test]
    fn dsh_writes_route_and_env_then_restores() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".dsh");
        std::fs::create_dir_all(dir.join("profiles/headless")).unwrap();
        let patch = dir.join("profiles/headless/cordis.patch.yml");
        let env = dir.join(".env");
        let original = "# 我的补丁\n- id: llm-pi-ai # magpie\n  name: \"@deepseek-ai/dsh-llm-pi-ai\"\n  config:\n    providers:\n      magpie:\n        displayName: Magpie\n        apiKeyEnv: MAGPIE_KEY\n        api: openai-completions\n        baseURL: http://127.0.0.1:3425/v1\n        models:\n          - id: group/x\n        streamIdleTimeoutMs: 600000\n- id: agent-default-model # magpie\n  config:\n    provider: magpie\n    model: \"group/x\"\n";
        std::fs::write(&patch, original).unwrap();
        std::fs::write(&env, "MAGPIE_KEY=abc\n").unwrap();
        let bak = d.path().join("backup");
        let target = Target { agent: "dsh".into(), run_provider: None, files: Files::Dsh { patch: patch.clone(), env: env.clone() } };
        let out = sync(&[provider("p")], &agents("dsh", "p", "", "max"), &[target.clone()], &[], &bak);
        assert_eq!(out.states[0].state, "synced", "{:?}", out.states[0]);
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("board-p:"), "{text}");
        assert!(text.contains("apiKeyEnv: BOARD_PROVIDER_P_API_KEY"), "{text}");
        assert!(text.contains("provider: board-p"), "{text}");
        assert!(text.contains("reasoningEffort: max"), "{text}");
        assert!(text.contains("magpie:"), "{text}");
        assert_eq!(std::fs::read_to_string(&env).unwrap(), "MAGPIE_KEY=abc\nBOARD_PROVIDER_P_API_KEY=sk-secret-1234\n");
        // 换供应商：旧路由和旧环境变量清掉
        let out = sync(&[provider("q")], &agents("dsh", "q", "", ""), &[target.clone()], &[], &bak);
        assert_eq!(out.states[0].state, "synced", "{:?}", out.states[0]);
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(!text.contains("board-p:"), "{text}");
        assert!(text.contains("board-q:"), "{text}");
        assert!(!text.contains("reasoningEffort:"), "{text}");
        let e = std::fs::read_to_string(&env).unwrap();
        assert!(!e.contains("BOARD_PROVIDER_P_API_KEY"), "{e}");
        assert!(e.contains("BOARD_PROVIDER_Q_API_KEY=sk-secret-1234"), "{e}");
        // 本机自己管：恢复成原来的样子
        let out = sync(&[], &[], &[Target { run_provider: Some(PROVIDER_LOCAL.into()), ..target.clone() }], &[], &bak);
        assert_eq!(out.states[0].state, "local");
        assert_eq!(std::fs::read_to_string(&patch).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&env).unwrap(), "MAGPIE_KEY=abc\n");
        assert!(!bak.join("dsh.json").exists());
    }

    #[test]
    fn dsh_creates_entries_when_missing_and_removes_them() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".dsh");
        std::fs::create_dir_all(dir.join("profiles/headless")).unwrap();
        let patch = dir.join("profiles/headless/cordis.patch.yml");
        let env = dir.join(".env");
        std::fs::write(&patch, "").unwrap();
        let bak = d.path().join("backup");
        let target = Target { agent: "dsh".into(), run_provider: None, files: Files::Dsh { patch: patch.clone(), env: env.clone() } };
        let out = sync(&[provider("p")], &agents("dsh", "p", "", ""), &[target.clone()], &[], &bak);
        assert_eq!(out.states[0].state, "synced", "{:?}", out.states[0]);
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("- id: llm-pi-ai # board"), "{text}");
        assert!(text.contains("- id: agent-default-model # board"), "{text}");
        assert!(env.exists());
        let out = sync(&[], &[], &[target], &[], &bak);
        assert_eq!(out.states[0].state, "local");
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(!text.contains("llm-pi-ai"), "{text}");
        assert!(!text.contains("agent-default-model"), "{text}");
        // 原来没有 .env：恢复时删掉
        assert!(!env.exists());
    }

    #[test]
    fn unsupported_agents_and_interface_mismatch_are_reported() {
        let d = tempfile::tempdir().unwrap();
        let bak = d.path().join("backup");
        let out = sync(&[], &[], &[], &[("mycli".into(), "配置文件里新加的 agent 不支持按共用配置改写".into())], &bak);
        assert_eq!((out.states[0].state.as_str(), out.states[0].agent.as_str()), ("unsupported", "mycli"));
        // 只有 OpenAI Chat 接口的供应商给 Codex（要 Responses）
        let p = Provider { endpoints: vec![ProviderEndpoint { interface: "openai-chat".into(), url: "https://x".into() }], ..provider("chat") };
        let config = d.path().join("config.toml");
        std::fs::write(&config, "").unwrap();
        let target = Target { agent: "codex".into(), run_provider: None, files: Files::Codex { config: config.clone() } };
        let out = sync(&[p], &agents("codex", "chat", "", ""), &[target], &[], &bak);
        assert_eq!(out.states[0].state, "unsupported");
        assert!(out.states[0].why.as_deref().unwrap().contains("Responses"));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "");
    }

    #[test]
    fn wsl_agents_have_no_files() {
        let env = Env { id: "wsl:Ubuntu".into(), name: "Ubuntu".into(), kind: crate::envs::Kind::Wsl("Ubuntu".into()), prog: PathBuf::new() };
        assert!(files_of("claude", &env).is_none());
        // 配置文件里新加的 agent 也不支持
        assert!(files_of("mycli", &Env::direct()).is_none());
    }
}
