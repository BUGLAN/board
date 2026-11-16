//! 照另一台机器配（default/main#398）：把来源机器的设置一次复制到这台机器上，之后两台各改各的，不保持同步。
//!
//! - 复制：每个 agent 的模型（不改 / 指定的供应商、模型、强度、接口）和启动环境、默认 agent、skills / MCP / Git 凭据的
//!   同步开关、自动更新 agent、最多几个会话、一个会话几张、接单时段
//! - 不复制：工作空间、机器名、接单三态（自动领 / 只做派给它的 / 暂停：新机器保持「只做派给它的」，人看过再打开）
//! - 启动环境只在两台是同一种系统、这台机器上这个 agent 支持并找得到那个环境时复制，否则保持原来的并写明
//! - 来源机器上有、这台机器上没找到的 agent：它的设置照样存下（装上并重新检查后生效），写明
//! - 界面上先预览（`dry`：逐项列出从什么变成什么，没有变化的不列），确认后一次写进去，记一条操作记录；
//!   改了会写进 agent 配置的项照常下发同步请求
//! - 添加机器时可以选「上线后照 <机器> 配」：记在生成命令时新建的令牌上（`host_presets`），
//!   用这个令牌的机器第一次上线时照来源机器当时的设置配
use crate::error::{BoardError, Result};
use crate::hosts::{no_such_host, AgentRun, HostRow, MODE_OFF, MODE_OWN, SYNC_KINDS};
use crate::sched::{Window, DAY};
use crate::store::{R, W};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 照另一台机器配的结果（预览时 `done` 为 false）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCopy {
    /// 来源机器
    pub from: String,
    /// 这台机器
    pub to: String,
    /// 要变的项，按界面上的顺序；没有变化的不列。空 = 已经一样
    pub changes: Vec<CopyChange>,
    /// 没照着改、要说一声的：系统不同启动环境没复制、这台机器上还没找到的 agent、供应商已经不在了
    pub notes: Vec<String>,
    /// 写进去了
    pub done: bool,
}

/// 一项变化：从什么变成什么（都是给人看的话）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyChange {
    /// 标识：`model:<agent>`、`env:<agent>`、`agent`、`sync:<种类>`、`agentAuto`、`max`、`perSession`、`windows`
    pub key: String,
    pub label: String,
    pub from: String,
    pub to: String,
}

/// 照着配之后这台机器的设置（只有要复制的那几项）
struct Plan {
    run: BTreeMap<String, AgentRun>,
    agent: Option<String>,
    sync: Vec<(&'static str, bool)>,
    agent_auto: bool,
    max: i64,
    per_session: i64,
    windows: Vec<Window>,
    out: HostCopy,
}

/// 系统大类：启动环境（直接启动、登录 shell、PowerShell、WSL……）只在同一类系统之间通用；认不出时为空
fn os_family(os: &str) -> &'static str {
    let o = os.to_lowercase();
    if o.contains("windows") {
        "Windows"
    } else if o.contains("mac") || o.contains("darwin") {
        "macOS"
    } else if o.trim().is_empty() {
        ""
    } else {
        "Linux"
    }
}

/// 一天里的分钟写成 HH:MM（1440 是 24:00）
fn hhmm(m: i64) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// 接单时段：全天 / 不接单 / 09:00–18:00、20:00–24:00
fn windows_words(w: &[Window]) -> String {
    match w {
        [] => "不接单".into(),
        [[0, DAY]] => "全天".into(),
        _ => w.iter().map(|[a, b]| format!("{}–{}", hhmm(*a), hhmm(*b))).collect::<Vec<_>>().join("、"),
    }
}

/// agent 的名称：内置的用固定名称，别的用机器上报的
fn agent_label(hs: [&HostRow; 2], id: &str) -> String {
    if crate::providers::AGENTS.iter().any(|(k, _, _)| *k == id) {
        return crate::providers::agent_name(id).to_string();
    }
    hs.iter().flat_map(|h| h.agents.iter()).find(|a| a.id == id).map(|a| a.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.to_string())
}

/// 一个 agent 的模型怎么配，写成一句话：不改 / 供应商 · 模型 · 强度 · 接口
fn model_words(h: &HostRow, id: &str) -> String {
    if h.mode_of(id) != MODE_OWN {
        return "不改".into();
    }
    let r = h.run.get(id).cloned().unwrap_or_default();
    let mut parts = vec![r.provider.unwrap_or_default(), r.model.unwrap_or_else(|| "供应商的第一个模型".into())];
    parts.push(r.effort.map_or_else(|| "强度默认".into(), |e| format!("强度 {e}")));
    if let Some(p) = r.protocol {
        parts.push(crate::providers::interface_name(&p).to_string());
    }
    parts.join(" · ")
}

/// 模型那几项（不改时都清空）
fn model_of(h: &HostRow, id: &str) -> AgentRun {
    let r = h.run.get(id).cloned().unwrap_or_default();
    if h.mode_of(id) != MODE_OWN {
        return AgentRun { mode: Some(MODE_OFF.into()), ..Default::default() };
    }
    AgentRun { mode: Some(MODE_OWN.into()), provider: r.provider, model: r.model, effort: r.effort, protocol: r.protocol, ..Default::default() }
}

fn on_off(on: bool) -> String {
    if on { "用服务器的" } else { "不改" }.into()
}

/// 照来源机器 `s` 配这台机器 `t`：要变成什么样、逐项的变化和要说一声的
fn plan(r: &R, t: &HostRow, s: &HostRow) -> Result<Plan> {
    let mut out = HostCopy { from: s.name.clone(), to: t.name.clone(), ..Default::default() };
    let change = |out: &mut HostCopy, key: String, label: String, from: String, to: String| out.changes.push(CopyChange { key, label, from, to });
    let (fs, ft) = (os_family(&s.os), os_family(&t.os));
    let same_os = !fs.is_empty() && fs == ft;

    // 每个 agent：来源机器上报的，加上它设过的
    let mut ids: Vec<String> = s.agents.iter().map(|a| a.id.clone()).collect();
    for k in s.run.keys() {
        if !ids.contains(k) {
            ids.push(k.clone());
        }
    }
    let mut run = t.run.clone();
    let mut env_skipped = vec![];
    for id in &ids {
        let name = agent_label([s, t], id);
        let found = t.has_agent(id);
        let mut stored = false;
        // 模型（只有内置 agent 能按服务器上的供应商写）
        if crate::providers::AGENTS.iter().any(|(k, _, _)| *k == id) {
            let (from, to) = (model_words(t, id), model_words(s, id));
            if from != to {
                let want = model_of(s, id);
                // 来源机器设的时候校验过；之后供应商可能删了、改了
                let gone = match want.provider.as_deref().filter(|_| want.mode.as_deref() == Some(MODE_OWN)) {
                    Some(p) => match r.provider(p)? {
                        None => Some(format!("供应商 {p} 已经不在了")),
                        Some(pv) if !pv.fits(id) => Some(format!("{name} 用不了供应商 {p}")),
                        Some(_) => None,
                    },
                    None => None,
                };
                match gone {
                    Some(why) => out.notes.push(format!("{name} 的模型没复制：{why}")),
                    None => {
                        let cur = run.entry(id.clone()).or_default();
                        cur.mode = want.mode;
                        cur.provider = want.provider;
                        cur.model = want.model;
                        cur.effort = want.effort;
                        cur.protocol = want.protocol;
                        change(&mut out, format!("model:{id}"), format!("{name} 的模型"), from, to);
                        stored = true;
                    }
                }
            }
        }
        // 启动环境：同一类系统、这台机器上找得到才复制
        let (se, te) = (s.run.get(id).and_then(|x| x.env.clone()), t.run.get(id).and_then(|x| x.env.clone()));
        let env_words = |h: &HostRow, e: &Option<String>| e.as_deref().map_or_else(|| "默认".to_string(), |e| h.env_name(e));
        if se != te {
            if !same_os {
                env_skipped.push(name.clone());
            } else if !found {
                out.notes.push(format!("{name} 的启动环境没复制：这台机器上还没找到 {name}"));
            } else {
                let ok = match &se {
                    Some(e) => t.env_check(id, e).map(|_| ()),
                    None => Ok(()),
                };
                match ok {
                    Err(e) => out.notes.push(format!("{name} 的启动环境没复制：{}", e.message)),
                    Ok(()) => {
                        let (from, to) = (env_words(t, &te), env_words(s, &se));
                        run.entry(id.clone()).or_default().env = se;
                        change(&mut out, format!("env:{id}"), format!("{name} 的启动环境"), from, to);
                    }
                }
            }
        }
        if stored && !found {
            out.notes.push(format!("这台机器上还没找到 {name}：它的设置先存下，装上并重新检查后生效"));
        }
    }
    if !env_skipped.is_empty() {
        let fam = |f: &'static str| if f.is_empty() { "系统不明" } else { f };
        out.notes.push(format!(
            "启动环境没复制：{} 是 {}、{} 是 {}，两边的启动环境不通用（{}）",
            s.name,
            fam(fs),
            t.name,
            fam(ft),
            env_skipped.join("、")
        ));
    }

    // 默认 agent：这台机器上找得到才改
    let mut agent = t.agent.clone();
    if s.agent != t.agent {
        let words = |a: &Option<String>| a.as_deref().map_or_else(|| "第一个找到的".to_string(), |a| agent_label([s, t], a));
        match &s.agent {
            Some(a) if !t.has_agent(a) => out.notes.push(format!("默认 agent 没改：这台机器上还没找到 {}", agent_label([s, t], a))),
            _ => {
                change(&mut out, "agent".into(), "默认 agent".into(), words(&t.agent), words(&s.agent));
                agent = s.agent.clone();
            }
        }
    }

    // skills、MCP、Git 凭据的同步开关
    let mut sync = vec![];
    for k in &SYNC_KINDS {
        let (from, to) = ((k.on)(t), (k.on)(s));
        if from != to {
            change(&mut out, format!("sync:{}", k.key), k.name.into(), on_off(from), on_off(to));
            sync.push((k.col, to));
        }
    }
    if s.agent_auto != t.agent_auto {
        let w = |on: bool| if on { "自动更新" } else { "关闭" }.to_string();
        change(&mut out, "agentAuto".into(), "自动更新 agent".into(), w(t.agent_auto), w(s.agent_auto));
    }
    if s.max != t.max {
        change(&mut out, "max".into(), "最多几个会话".into(), t.max.to_string(), s.max.to_string());
    }
    if s.per_session != t.per_session {
        change(&mut out, "perSession".into(), "一个会话几张".into(), t.per_session.to_string(), s.per_session.to_string());
    }
    if s.windows != t.windows {
        change(&mut out, "windows".into(), "接单时段".into(), windows_words(&t.windows), windows_words(&s.windows));
    }
    Ok(Plan { run, agent, sync, agent_auto: s.agent_auto, max: s.max, per_session: s.per_session, windows: s.windows.clone(), out })
}

impl W<'_> {
    /// 照机器 `from` 配机器 `name`（只能在登录后的界面上做）。`dry` 只算要变哪些、不写
    pub fn host_copy(&mut self, name: &str, from: &str, by: &str, dry: bool) -> Result<HostCopy> {
        let t = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let from = from.trim();
        if from.is_empty() {
            return Err(BoardError::usage("选一台机器来照着配"));
        }
        let s = self.host_row(from)?.ok_or_else(|| no_such_host(from))?;
        if s.id == t.id {
            return Err(BoardError::usage("不能照自己配：选另一台机器"));
        }
        let p = plan(&self.read(), &t, &s)?;
        if dry || p.out.changes.is_empty() {
            return Ok(p.out);
        }
        self.copy_apply(&t, p, by, "照")
    }

    /// 添加机器时选了「上线后照 <机器> 配」：记在这个令牌上，用它的机器第一次上线时照着配
    pub fn host_preset(&mut self, token: i64, from: &str, by: &str) -> Result<()> {
        let from = from.trim();
        self.host_row(from)?.ok_or_else(|| no_such_host(from))?;
        self.tx.execute("INSERT OR REPLACE INTO host_presets (token, src, by, at) VALUES (?1, ?2, ?3, ?4)", params![token, from, by, self.now])?;
        Ok(())
    }

    /// 一台新机器第一次上线（`host_beat` 新建了它的记录）：它用的令牌带着「上线后照 <机器> 配」就照来源机器现在的设置配，用过就删
    pub(crate) fn host_preset_apply(&mut self, name: &str, token: Option<i64>) -> Result<()> {
        let Some(token) = token else { return Ok(()) };
        let got: Option<(String, String)> =
            self.tx.query_row("SELECT src, by FROM host_presets WHERE token = ?1", [token], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let Some((src, by)) = got else { return Ok(()) };
        self.tx.execute("DELETE FROM host_presets WHERE token = ?1", [token])?;
        let t = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        match self.host_row(&src)? {
            Some(s) if s.id != t.id => {
                let p = plan(&self.read(), &t, &s)?;
                if p.out.changes.is_empty() {
                    self.host_op(t.id, &by, &format!("上线后照 {src} 配：默认设置已经和它一样"))?;
                } else {
                    self.copy_apply(&t, p, &by, "上线后照")?;
                }
            }
            _ => self.host_op(t.id, &by, &format!("上线后照 {src} 配：{src} 已经不在了，按默认设置建立"))?,
        }
        Ok(())
    }

    fn copy_apply(&mut self, t: &HostRow, mut p: Plan, by: &str, how: &str) -> Result<HostCopy> {
        self.tx.execute(
            "UPDATE hosts SET run = ?2, agent = ?3, agent_auto = ?4, max = ?5, per_session = ?6, windows = ?7 WHERE id = ?1",
            params![
                t.id,
                serde_json::to_string(&p.run).unwrap_or_default(),
                p.agent,
                p.agent_auto as i64,
                p.max,
                p.per_session,
                serde_json::to_string(&p.windows).unwrap_or_default()
            ],
        )?;
        for (col, on) in &p.sync {
            self.tx.execute(&format!("UPDATE hosts SET {col} = ?2 WHERE id = ?1"), params![t.id, *on as i64])?;
        }
        // 模型、启动环境、同步开关会写进 agent 配置：要写成的样子变了就下发同步请求
        self.config_by = by.to_string();
        self.config_changed = true;
        let labels: Vec<&str> = p.out.changes.iter().map(|c| c.label.as_str()).collect();
        let mut text = format!("{how} {} 配：改了 {}", p.out.from, labels.join("、"));
        if !p.out.notes.is_empty() {
            text.push_str(&format!("；{}", p.out.notes.join("；")));
        }
        self.host_op(t.id, by, &text)?;
        self.touch_hosts();
        // 会话上限变了：等着的「任意机器」派发重新挑一遍
        self.assign_any()?;
        p.out.done = true;
        Ok(p.out)
    }
}
