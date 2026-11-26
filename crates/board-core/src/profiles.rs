//! 角色（PRD §9.1「分工」）：把「谁来做、用什么模型」从机器上拿出来，服务端存一份，所有机器共用。
//!
//! - 一个角色 = agent + 模型来源 + 强度，可选附加说明（派发时接在提示词后面）、可选「只在这些机器上跑」。
//!   模型来源二选一：共用配置里的「供应商 · 模型」（`provider` 不为空），或「本机登录」（`provider` 为空：用机器上
//!   agent 自己登录的账号，会话里切到 `model`；`model` 也为空时用 agent 自己的默认模型）。
//! - 领取规则是「条件 → 角色」（`sched.rules` 每条的 `profile`），派发（手动、任意机器、改派）也选角色。
//!   board host 每次派发按角色给这一个 agent 进程带上供应商、模型和强度（ACP `providers/set` + 会话配置项；
//!   dsh 写进这次派发的 `--patch`），只对这个进程生效，不改 agent 的全局配置文件。
//! - 一台机器能跑一个角色（[`fit`]）：装了这个 agent（找得到命令、没缺 ACP 入口要的命令），没被这个角色排除
//!   （`only` 不为空时只在这些机器上）；本机登录的角色：最近一次自检里这个 agent 没有因为没登录而失败；用供应商的角色：
//!   这台机器的 board host 会按次换供应商（[`crate::hosts::HostAgent::switch`]，旧版不会），最近一次自检里连得上这个供应商。
//!   登录和供应商的结果来自自检（机器详情「自检」），一直算到下一次自检。
//! - 删掉一个角色时，交给它的规则一起删。
//!
//! 升级（[`migrate`]）：规则、兜底、「由这些机器做」里用到的每个 agent 生成一个同名角色（本机登录、不指定模型，等于
//! 以前机器上的「不改」），规则指向它；「由这些机器做」里排除的机器换成这个角色的「只在这些机器上跑」。
//! 兜底（规则都没命中的卡交给谁）去掉了：规则都没命中的卡不自动领，可以手动派发。

use crate::hosts::{ClaimRule, HostRow};
use crate::store::{R, W};
use crate::{BoardError, ErrorCode, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub(crate) const SCHEMA: &str = r#"
-- 角色：agent + 模型来源 + 强度（PRD §9.1「分工」）
CREATE TABLE IF NOT EXISTS profiles (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  agent      TEXT NOT NULL,
  provider   TEXT NOT NULL DEFAULT '',  -- 空 = 本机登录
  model      TEXT NOT NULL DEFAULT '',  -- 空 = agent 自己的默认（只有本机登录时可以空）
  effort     TEXT NOT NULL DEFAULT '',  -- 空 = 不设
  note       TEXT NOT NULL DEFAULT '',  -- 附加说明：派发时接在提示词后面
  only       TEXT NOT NULL DEFAULT '[]', -- 只在这些机器上跑（机器 id）；空数组 = 所有能跑的
  sort       INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  by_name    TEXT NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX IF NOT EXISTS profiles_name ON profiles (name COLLATE NOCASE);
"#;

/// 名字最长几个字
pub const NAME_MAX: usize = 20;
/// 附加说明最长几个字
pub const NOTE_MAX: usize = 2000;
/// 最多几个角色
pub const PROFILES_LIMIT: usize = 40;

/// 一个角色
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// 标识：新建时服务端生成，之后不变（改名字不影响规则）
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// agent 标识：claude、codex、dsh …
    pub agent: String,
    /// 共用配置里的供应商名称；空 = 本机登录
    #[serde(default)]
    pub provider: String,
    /// 模型；空 = agent 自己的默认（只有本机登录时可以空）
    #[serde(default)]
    pub model: String,
    /// 思考强度；空 = 不设
    #[serde(default)]
    pub effort: String,
    /// 附加说明：派发时接在提示词后面
    #[serde(default)]
    pub note: String,
    /// 只在这些机器上跑（机器 id）；空 = 所有能跑的
    #[serde(default)]
    pub only: Vec<i64>,
    #[serde(default)]
    pub sort: i64,
    #[serde(default)]
    pub created_at: i64,
}

impl Profile {
    /// 「Claude Code · 本机登录 · sonnet · high」：动态、派发记录里写
    pub fn recipe(&self) -> String {
        let mut parts = vec![crate::providers::agent_name(&self.agent).to_string()];
        parts.push(if self.provider.is_empty() { "本机登录".into() } else { self.provider.clone() });
        if !self.model.is_empty() {
            parts.push(self.model.clone());
        }
        if !self.effort.is_empty() {
            parts.push(self.effort.clone());
        }
        parts.join(" · ")
    }
}

/// 一台机器能不能跑一个角色（发给界面：机器详情、分工页、派发对话框）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileFit {
    pub profile: String,
    pub ok: bool,
    /// 跑不了的原因（`ok` 时为空）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: r.get(0)?,
        name: r.get(1)?,
        agent: r.get(2)?,
        provider: r.get(3)?,
        model: r.get(4)?,
        effort: r.get(5)?,
        note: r.get(6)?,
        only: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
        sort: r.get(8)?,
        created_at: r.get(9)?,
    })
}

/// 自检里 agent 一项因为没登录没通过时，原因的开头（board host 的 `selftest::judge_agent`）
pub const LOGIN_FAILED: &str = "没有登录";

const COLS: &str = "id, name, agent, provider, model, effort, note, only, sort, created_at";

/// 一台机器能不能跑这个角色：只看角色和机器，不看在不在线、有没有空位、在不在接单时段（那些是「现在能不能开始」）。
/// 跑不了时返回原因。登录、连不连得上供应商看最近一次自检（没自检过、旧版 board host 不报时当能跑）；本机登录的角色
/// 还看这台机器上这个 agent 的额度用没用完（暂停到 `quota_until`，`now` 过了就自动恢复，#668）
pub(crate) fn fit(p: &Profile, h: &HostRow, now: i64) -> std::result::Result<(), String> {
    if !p.only.is_empty() && !p.only.contains(&h.id) {
        return Err(format!("不在「{}」的机器里", p.name));
    }
    let all = h.resolved();
    let name = crate::providers::agent_name(&p.agent);
    let a = match all.iter().find(|a| a.id == p.agent) {
        None => return Err(format!("没装 {name}")),
        Some(a) if a.missing.is_some() => return Err(format!("缺 {}，{name} 跑不起来", a.missing.as_deref().unwrap_or_default())),
        Some(a) if !a.found => return Err(format!("没找到 {name} 的命令")),
        Some(a) => a,
    };
    let failed = |kind: &str, key: &str| h.selftest.items.iter().find(|i| i.kind == kind && i.key == key && !i.ok);
    if p.provider.is_empty() {
        if failed("agent", &p.agent).is_some_and(|i| i.why.as_deref().is_some_and(|w| w.starts_with(LOGIN_FAILED))) {
            return Err(format!("{name} 没登录（上次自检）：登录后重新自检"));
        }
        if let Some(until) = h.run.get(&p.agent).and_then(|r| r.quota_until).filter(|t| *t > now) {
            return Err(format!("额度用完，{} 恢复", crate::store::when_words(until, now)));
        }
        return Ok(());
    }
    if !crate::agentdef::is_builtin(&p.agent) {
        return Err(format!("{name} 是配置文件里加的 agent，不能换供应商"));
    }
    if !a.switch {
        return Err("board 版本太旧，升级后才能换供应商".into());
    }
    if let Some(i) = failed("provider", &p.provider) {
        return Err(format!("供应商 {} 没通过上次自检：{}", p.provider, i.why.as_deref().unwrap_or("没有原因")));
    }
    Ok(())
}

impl R<'_> {
    /// 所有角色，按排列顺序
    pub fn profiles(&self) -> Result<Vec<Profile>> {
        let mut st = self.conn.prepare(&format!("SELECT {COLS} FROM profiles ORDER BY sort, created_at, id"))?;
        let rows = st.query_map([], row)?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn profile(&self, id: &str) -> Result<Option<Profile>> {
        Ok(self.conn.query_row(&format!("SELECT {COLS} FROM profiles WHERE id = ?1"), [id], row).optional()?)
    }

    /// 按标识或名字（不区分大小写）找角色：`board dispatch --profile` 用
    pub fn profile_by(&self, key: &str) -> Result<Option<Profile>> {
        let key = key.trim();
        if let Some(p) = self.profile(key)? {
            return Ok(Some(p));
        }
        Ok(self.conn.query_row(&format!("SELECT {COLS} FROM profiles WHERE name = ?1 COLLATE NOCASE"), [key], row).optional()?)
    }
}

/// 新角色的标识：`p` + 时间和序号（足够区分，不用随机数）
fn new_id(conn: &rusqlite::Connection, now: i64) -> rusqlite::Result<String> {
    let mut n = 0i64;
    loop {
        let id = format!("p{}", radix36((now as u64).wrapping_add(n as u64)));
        let taken: bool = conn.query_row("SELECT COUNT(*) FROM profiles WHERE id = ?1", [&id], |r| r.get::<_, i64>(0))? > 0;
        if !taken {
            return Ok(id);
        }
        n += 1;
    }
}

fn radix36(mut v: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = vec![];
    loop {
        out.push(D[(v % 36) as usize]);
        v /= 36;
        if v == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

impl W<'_> {
    /// 新建（`id` 为空）或修改一个角色。校验：名字不空、不重复；agent 是内置的或某台机器上报过的；用供应商时供应商存在、
    /// 这个 agent 认它的某种接口、模型在它的模型列表里；强度是这个 agent 能设的一档；「只在这些机器上跑」只留存在的机器
    pub fn profile_put(&mut self, p: &Profile, by: &str) -> Result<Profile> {
        let name = p.name.trim().to_string();
        if name.is_empty() {
            return Err(BoardError::usage("角色要有名字"));
        }
        if name.chars().count() > NAME_MAX {
            return Err(BoardError::usage(format!("角色的名字最长 {NAME_MAX} 个字")));
        }
        let existing = self.read().profiles()?;
        if existing.iter().any(|x| x.id != p.id && x.name.to_lowercase() == name.to_lowercase()) {
            return Err(BoardError::usage(format!("已经有叫「{name}」的角色了")));
        }
        let old = if p.id.is_empty() { None } else { Some(self.read().profile(&p.id)?.ok_or_else(|| no_such_profile(&p.id))?) };
        if old.is_none() && existing.len() >= PROFILES_LIMIT {
            return Err(BoardError::usage(format!("角色最多 {PROFILES_LIMIT} 个")));
        }
        let agent = p.agent.trim().to_string();
        let hosts = self.read().host_rows()?;
        let known = crate::agentdef::is_builtin(&agent) || hosts.iter().any(|h| h.resolved().iter().any(|a| a.id == agent));
        if agent.is_empty() || !known {
            let builtin: Vec<&str> = crate::agentdef::ids().collect();
            return Err(BoardError::usage(format!("没有 agent「{agent}」：内置的有 {}，配置文件里加的要先有机器上报", builtin.join("、"))));
        }
        let provider = p.provider.trim().to_string();
        let model = p.model.trim().to_string();
        let effort = p.effort.trim().to_string();
        let aname = crate::providers::agent_name(&agent);
        if !provider.is_empty() {
            if !crate::agentdef::is_builtin(&agent) {
                return Err(BoardError::usage(format!("{aname} 是配置文件里加的 agent，不能换供应商：选「本机登录」")));
            }
            let prov = self.read().provider(&provider)?.ok_or_else(|| BoardError::usage(format!("没有叫「{provider}」的供应商")))?;
            if prov.endpoint_for(&agent, "").is_none() {
                let want: Vec<&str> = crate::providers::protocols_of(&agent).iter().map(|i| crate::providers::interface_name(i)).collect();
                return Err(BoardError::usage(format!("供应商「{provider}」没有 {aname} 认的接口（{}）", want.join("、"))));
            }
            if model.is_empty() {
                return Err(BoardError::usage("用供应商时要选模型"));
            }
            if !prov.models.iter().any(|m| m == &model) {
                return Err(BoardError::usage(format!("供应商「{provider}」没有模型「{model}」")));
            }
        }
        if !model.is_empty() {
            crate::hosts::check_model(&model).map_err(BoardError::usage)?;
        }
        if !effort.is_empty() {
            let can = crate::providers::efforts_of(&agent);
            if crate::agentdef::is_builtin(&agent) && !can.contains(&effort.as_str()) {
                return Err(BoardError::usage(format!("{aname} 的强度只能是 {}", can.join("、"))));
            }
            if !crate::providers::ALL_EFFORTS.contains(&effort.as_str()) {
                return Err(BoardError::usage(format!("强度只能是 {}", crate::providers::ALL_EFFORTS.join("、"))));
            }
        }
        let note = p.note.trim().to_string();
        if note.chars().count() > NOTE_MAX {
            return Err(BoardError::usage(format!("附加说明最长 {NOTE_MAX} 个字")));
        }
        // 勾了几台就存几台，等于现在的全部机器也照存（以后新接的机器不算进来，#664）；空 = 所有能跑的机器，只由调用方明确给
        let mut only: Vec<i64> = p.only.iter().copied().filter(|id| hosts.iter().any(|h| h.id == *id)).collect();
        only.sort_unstable();
        only.dedup();
        let only_raw = serde_json::to_string(&only).unwrap_or_else(|_| "[]".into());
        let id = match old {
            Some(o) => {
                self.tx.execute(
                    "UPDATE profiles SET name = ?2, agent = ?3, provider = ?4, model = ?5, effort = ?6, note = ?7, only = ?8, by_name = ?9 WHERE id = ?1",
                    params![o.id, name, agent, provider, model, effort, note, only_raw, by],
                )?;
                o.id
            }
            None => {
                let id = new_id(self.tx, self.now)?;
                let sort: i64 = self.tx.query_row("SELECT COALESCE(MAX(sort), 0) + 1 FROM profiles", [], |r| r.get(0))?;
                self.tx.execute(
                    "INSERT INTO profiles (id, name, agent, provider, model, effort, note, only, sort, created_at, by_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![id, name, agent, provider, model, effort, note, only_raw, sort, self.now, by],
                )?;
                id
            }
        };
        self.touch_hosts();
        // 任意机器的排队派发：能去的机器可能变了
        self.assign_any()?;
        self.read().profile(&id)?.ok_or_else(|| no_such_profile(&id))
    }

    /// 删掉一个角色：交给它的规则一起删，选了它的定时建卡回到「按规则」。返回删掉了几条规则
    pub fn profile_delete(&mut self, id: &str) -> Result<usize> {
        self.read().profile(id)?.ok_or_else(|| no_such_profile(id))?;
        let (rules, _) = self.read().sched_rules()?;
        let keep: Vec<ClaimRule> = rules.iter().filter(|r| r.profile != id).cloned().collect();
        let dropped = rules.len() - keep.len();
        if dropped > 0 {
            let raw = serde_json::to_string(&keep.iter().map(ClaimRule::stored).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
            self.tx.execute("UPDATE sched SET rules = ?1 WHERE id = 1", [raw])?;
        }
        self.tx.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        self.tx.execute("UPDATE card_crons SET profile = NULL WHERE profile = ?1", [id])?;
        self.touch_hosts();
        Ok(dropped)
    }
}

/// 给 agent 自动建角色时的名字：agent 的名称，重名时后面加标识
pub(crate) fn free_name(profiles: &[Profile], agent: &str) -> String {
    let name = crate::providers::agent_name(agent).to_string();
    if profiles.iter().any(|p| p.name.to_lowercase() == name.to_lowercase()) {
        format!("{name}（{agent}）")
    } else {
        name
    }
}

pub(crate) fn no_such_profile(id: &str) -> BoardError {
    BoardError::new(ErrorCode::NotFound, format!("没有角色「{id}」：Agent → 自动领取 → 分工里能看到所有角色"))
}

/// 一个 agent 的角色（升级、旧版界面只给 agent 时用）：已经有这个 agent 的角色就用排在最前面的那个，没有就建一个——名字是
/// agent 的名称（重名时加标识），本机登录、不指定模型、不设强度（等于以前机器上的「不改」），`only` 是「只在这些机器上跑」
pub(crate) fn agent_profile(conn: &rusqlite::Connection, agent: &str, only: &[i64], now: i64) -> rusqlite::Result<String> {
    let have: Option<String> =
        conn.query_row("SELECT id FROM profiles WHERE agent = ?1 ORDER BY sort, created_at, id LIMIT 1", [agent], |r| r.get(0)).optional()?;
    if let Some(id) = have {
        return Ok(id);
    }
    let existing: Vec<Profile> = {
        let mut st = conn.prepare(&format!("SELECT {COLS} FROM profiles"))?;
        let v = st.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        v
    };
    let name = free_name(&existing, agent);
    let id = new_id(conn, now)?;
    let sort: i64 = conn.query_row("SELECT COALESCE(MAX(sort), 0) + 1 FROM profiles", [], |r| r.get(0))?;
    conn.execute(
        "INSERT INTO profiles (id, name, agent, only, sort, created_at, by_name) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '升级')",
        params![id, name, agent, serde_json::to_string(only).unwrap_or_else(|_| "[]".into()), sort, now],
    )?;
    Ok(id)
}

/// 升级：旧的「条件 → agent」规则换成「条件 → 角色」。每次打开都看一遍（只在有没换过的规则、或者还有兜底时改，
/// 改过之后什么也不做），这样哪一步之后断掉、或者旧版又写回了旧的规则都能接上。
/// 规则、兜底、「由这些机器做」里用到的每个 agent：已经有这个 agent 的角色就用排在最前面的那个，没有就建一个——名字是
/// agent 的名称（重名时加标识），本机登录、不指定模型、不设强度，等于以前机器上的「不改」；「由这些机器做」里排除了
/// 机器的，换成「只在没排除的机器上跑」。兜底去掉（规则都没命中的卡不自动领）
pub(crate) fn migrate(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let got: Option<(String, Option<String>, String)> =
        conn.query_row("SELECT rules, fallback, picks FROM sched WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
    let Some((raw, fallback, picks_raw)) = got else { return Ok(()) };
    let mut rules: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
    let old = |r: &serde_json::Value| {
        r.get("profile").and_then(|p| p.as_str()).is_none_or(|p| p.trim().is_empty())
            && r.get("agent").and_then(|a| a.as_str()).is_some_and(|a| !a.trim().is_empty())
    };
    let fallback = fallback.filter(|f| !f.trim().is_empty());
    if !rules.iter().any(old) && fallback.is_none() {
        return Ok(());
    }
    let picks = crate::sched::parse_picks(&picks_raw);
    let mut agents: Vec<String> = vec![];
    for r in rules.iter().filter(|r| old(r)) {
        let a = r.get("agent").and_then(|a| a.as_str()).unwrap_or_default().to_string();
        if !agents.contains(&a) {
            agents.push(a);
        }
    }
    let host_ids: Vec<i64> = {
        let mut st = conn.prepare("SELECT id FROM hosts ORDER BY id")?;
        let v = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        v
    };
    let now = crate::store::now_ms();
    let mut ids: Vec<(String, String)> = vec![];
    for (i, agent) in agents.iter().enumerate() {
        let only: Vec<i64> = match picks.get(agent.as_str()) {
            Some(off) if !off.is_empty() => host_ids.iter().copied().filter(|h| !off.contains(h)).collect(),
            _ => vec![],
        };
        ids.push((agent.clone(), agent_profile(conn, agent, &only, now + i as i64)?));
    }
    for r in rules.iter_mut().filter(|r| old(r)) {
        let agent = r.get("agent").and_then(|a| a.as_str()).unwrap_or_default().to_string();
        if let Some((_, id)) = ids.iter().find(|(a, _)| *a == agent) {
            let op = r.get("op").cloned().unwrap_or_else(|| json!("in"));
            let labels = r.get("labels").cloned().unwrap_or_else(|| json!([]));
            let label = r.get("label").cloned();
            let mut out = json!({ "op": op, "labels": labels, "profile": id });
            if let Some(l) = label.filter(|l| l.is_string()) {
                out["label"] = l;
            }
            *r = out;
        }
    }
    conn.execute(
        "UPDATE sched SET rules = ?1, fallback = NULL, picks = '{}' WHERE id = 1",
        [serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into())],
    )?;
    Ok(())
}
