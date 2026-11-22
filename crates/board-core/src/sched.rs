//! 调度（PRD §9.1「自动领取」）：哪些卡自动交给哪个 agent，在服务端存一份，所有机器共用。
//!
//! - **领取规则**（`sched` 表，只有一行）：有顺序的「条件 → agent」和一条兜底。以前每台机器各存一份
//!   （`hosts.rules`、`hosts.fallback`），几台机器要靠「复制设置」保持一致；现在读机器记录时直接带上这一份
//!   （`hosts/read.rs` 的 `HOST_COLS`），按规则挑卡片的代码不用分机器的还是全局的。
//! - **白板参不参与**（`boards.auto`，默认参与）：不参与的白板不会被自动领取，手动派发照常。
//! - **由哪些机器做**（`sched.picks`，按 agent 记）：界面「领取规则 → 由这些机器做」里勾掉的机器不自动领这个 agent
//!   的卡（即使它有空的会话位），别的 agent、别的机器不受影响；手动派发、`board claim` 照常。存的是明确排除的机器，
//!   所以没设置过的 agent、之后才装上它的机器默认都参与，见 [`crate::hosts::HostRow::routes`]。
//! - **机器的接单时段**（`hosts.auto`、`hosts.windows`）：机器开了自动接单、现在在它的时段里，就持续领符合规则的卡，
//!   不再是「到点领一轮」。时段是一天里的几段 `[起, 止)`，单位分钟，每天一样，按服务端所在的时区算
//!   （[`server_minute`]）——机器分布在哪个时区都按同一个钟。
//!
//! 升级时的迁移见 [`migrate`]。

use crate::hosts::{ClaimRule, RULES_LIMIT, RULE_OPS};
use crate::{BoardError, Result, R, W};
use chrono::{TimeZone as _, Timelike as _};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sched (
  id       INTEGER PRIMARY KEY CHECK (id = 1),
  rules    TEXT NOT NULL DEFAULT '[]',   -- 领取规则，有顺序：[{op, labels, agent}]
  fallback TEXT,                          -- 兜底的 agent；空表示规则没命中的卡片不领
  max_run  INTEGER NOT NULL DEFAULT 180,   -- 单次最长运行（分钟，0 = 不限），所有机器一样
  idle_run INTEGER NOT NULL DEFAULT 20,    -- 多久没有输出算卡住（分钟，0 = 不限），所有机器一样
  picks    TEXT NOT NULL DEFAULT '{}'      -- 每个 agent 明确不参与的机器 id：{\"claude\":[1,2]}；没有一项 = 全都参与
);
";

/// 一天多少分钟
pub const DAY: i64 = 1440;
/// 一台机器最多几段接单时段
pub const WINDOWS_LIMIT: usize = 12;
/// 一段接单时段：一天里的 `[起, 止)`，分钟
pub type Window = [i64; 2];

/// 全天
pub fn all_day() -> Vec<Window> {
    vec![[0, DAY]]
}

/// 校验并整理接单时段：每段 `0 ≤ 起 < 止 ≤ 1440`，按先后排好，挨着、重叠的并成一段。一段都没有 = 不接单
pub fn tidy_windows(list: &[Window]) -> std::result::Result<Vec<Window>, String> {
    if list.len() > WINDOWS_LIMIT {
        return Err(format!("接单时段最多 {WINDOWS_LIMIT} 段"));
    }
    let mut v: Vec<Window> = vec![];
    for [a, b] in list {
        if !(0 <= *a && a < b && *b <= DAY) {
            return Err(format!("接单时段要写成一天里的起止分钟（0 ≤ 起 < 止 ≤ {DAY}）：[{a}, {b}]"));
        }
        v.push([*a, *b]);
    }
    v.sort();
    let mut out: Vec<Window> = vec![];
    for [a, b] in v {
        match out.last_mut() {
            Some(prev) if a <= prev[1] => prev[1] = prev[1].max(b),
            _ => out.push([a, b]),
        }
    }
    Ok(out)
}

/// 一天里的第 `minute` 分钟在不在时段里
pub fn in_windows(list: &[Window], minute: i64) -> bool {
    list.iter().any(|[a, b]| *a <= minute && minute < *b)
}

/// 现在是服务端所在时区一天里的第几分钟（接单时段按它算）
pub fn server_minute(now_ms: i64) -> i64 {
    match chrono::Local.timestamp_millis_opt(now_ms) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => (t.hour() * 60 + t.minute()) as i64,
        chrono::LocalResult::None => 0,
    }
}

/// 一条规则和另一条是不是同一个条件（不看「由哪些机器做」）：升级时合并各台机器的规则用
fn same_cond(a: &ClaimRule, b: &ClaimRule) -> bool {
    let low = |v: &[String]| {
        let mut v: Vec<String> = v.iter().map(|l| l.to_lowercase()).collect();
        v.sort();
        v
    };
    a.op == b.op && a.profile == b.profile && low(&a.labels) == low(&b.labels)
}

/// 每个 agent 明确不参与的机器 id。按 agent 记，同一个 agent 的几条规则（和它的兜底）共用一组；
/// 不在表里、或列表为空 = 所有装了它的机器都参与。以后才装上这个 agent 的机器也默认参与
pub(crate) type Picks = BTreeMap<String, Vec<i64>>;

/// 读存下来的「由哪些机器做」：去掉无效值、排序去重，空列表不保留
pub(crate) fn tidy_picks(mut picks: Picks) -> Picks {
    picks.retain(|agent, off| {
        off.retain(|h| *h > 0);
        off.sort_unstable();
        off.dedup();
        !agent.trim().is_empty() && !off.is_empty()
    });
    picks
}

pub(crate) fn parse_picks(raw: &str) -> Picks {
    tidy_picks(serde_json::from_str(raw).unwrap_or_default())
}

/// 升级：
/// - 加上 `hosts.auto`、`hosts.windows`、`boards.auto` 三列；
/// - 第一次（`sched` 表还是空的）把各台机器上的领取规则并成全局的一份：按机器建立的先后依次接上，一模一样的只留一条；
///   兜底取第一台设了的。几台机器的规则不一样时合并后的顺序可能不是想要的，由人在「调度」里调；
/// - 旧的定时领取（开关和 cron 时间表）**不迁移**：到点领一轮换不成接单时段，由人在「调度」里重新设。
///   升级后每台机器的自动接单都是关着的、接单时段是全天，打开之前不会自己领卡
pub(crate) fn migrate(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let has = |table: &str, col: &str| -> rusqlite::Result<bool> {
        conn.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"), [col], |r| r.get::<_, i64>(0)).map(|n| n > 0)
    };
    for (table, col, def) in [
        ("hosts", "auto", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "windows", "TEXT NOT NULL DEFAULT '[[0,1440]]'"),
        ("boards", "auto", "INTEGER NOT NULL DEFAULT 1"),
        // 自动升级：全局开关，和每台机器已经替它试过升级到哪个版本、试了几次、什么时候再试（hosts/manage.rs auto_upgrade_host）
        ("sched", "auto_upgrade", "INTEGER NOT NULL DEFAULT 0"),
        // 「由这些机器做」：每个 agent 明确不参与的机器 id（JSON）
        ("sched", "picks", "TEXT NOT NULL DEFAULT '{}'"),
        ("hosts", "up_tried", "TEXT"),
        ("hosts", "up_tries", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "up_retry", "INTEGER"),
    ] {
        if !has(table, col)? {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"))?;
        }
    }
    // 跑多久停以前每台机器各设一次：并成所有机器一个值，取机器里最常见的（一样多时取先建的那台的），没有机器用默认值
    // 逐列补（旧版只加了 max_run 就断掉的库也补上 idle_run，#509）；补上了哪一列都按第一次迁移处理
    if !crate::store::add_cols(conn, "sched", &[("max_run", "INTEGER NOT NULL DEFAULT 180"), ("idle_run", "INTEGER NOT NULL DEFAULT 20")])?.is_empty() {
        let common = |col: &str| -> rusqlite::Result<Option<i64>> {
            conn.query_row(&format!("SELECT {col} FROM hosts GROUP BY {col} ORDER BY COUNT(*) DESC, MIN(id) LIMIT 1"), [], |r| r.get(0)).optional()
        };
        if has("hosts", "max_run")? {
            let (m, i) = (common("max_run")?, common("idle_run")?);
            conn.execute("UPDATE sched SET max_run = COALESCE(?1, max_run), idle_run = COALESCE(?2, idle_run)", params![m, i])?;
            // sched 还没有那一行（下面第一次迁移时建）：记下来，建行时带上
            if conn.query_row("SELECT COUNT(*) FROM sched", [], |r| r.get::<_, i64>(0))? == 0 {
                conn.execute(
                    "INSERT INTO sched (id, rules, fallback, max_run, idle_run) VALUES (1, '[]', NULL, COALESCE(?1, 180), COALESCE(?2, 20))",
                    params![m, i],
                )?;
                return rules_from_hosts(conn, true);
            }
        }
    }
    let done: i64 = conn.query_row("SELECT COUNT(*) FROM sched", [], |r| r.get(0))?;
    if done > 0 {
        return Ok(());
    }
    rules_from_hosts(conn, false)
}

/// 第一次：把各台机器上的领取规则并成全局的一份（`update` 时那一行已经建好，只改规则和兜底）
fn rules_from_hosts(conn: &rusqlite::Connection, update: bool) -> rusqlite::Result<()> {
    let mut st = conn.prepare("SELECT rules, fallback FROM hosts ORDER BY id")?;
    let hosts = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut rules: Vec<ClaimRule> = vec![];
    let mut fallback: Option<String> = None;
    for (raw, fb) in &hosts {
        for r in serde_json::from_str::<Vec<ClaimRule>>(raw).unwrap_or_default().into_iter().map(ClaimRule::upgraded) {
            if rules.len() < RULES_LIMIT && !rules.iter().any(|x| same_cond(x, &r)) {
                rules.push(r);
            }
        }
        if fallback.is_none() {
            fallback = fb.clone().map(|f| f.trim().to_string()).filter(|f| !f.is_empty());
        }
    }
    let raw = serde_json::to_string(&rules.iter().map(ClaimRule::stored).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
    if update {
        conn.execute("UPDATE sched SET rules = ?1, fallback = ?2 WHERE id = 1", params![raw, fallback])?;
    } else {
        conn.execute("INSERT INTO sched (id, rules, fallback) VALUES (1, ?1, ?2)", params![raw, fallback])?;
    }
    Ok(())
}

/// 调度的全局设置（随工作区数据发给界面）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sched {
    /// 领取规则，有顺序；`active` 是「至少有一台机器上找得到这个 agent」，`missing` 是所有项目里都没有的标签
    pub rules: Vec<ClaimRule>,
    /// 服务端的时区（环境变量 `TZ`，没设时为空）：界面上写在「服务器时间」后面
    #[serde(default)]
    pub tz: String,
    /// 现在是服务端时区一天里的第几分钟：界面上画「现在」那条线、判断机器在不在时段里
    pub minute: i64,
    /// 自动升级（PRD §9.1「自动升级」）：服务端的版本比机器新时，机器等手上的派发做完自己升级
    #[serde(default)]
    pub auto_upgrade: bool,
    /// 单次最长运行（分钟，0 = 不限），所有机器一样
    #[serde(default)]
    pub max_run: i64,
    /// 多久没有输出算卡住（分钟，0 = 不限），所有机器一样
    #[serde(default)]
    pub idle_run: i64,
    /// 角色（按排列顺序）：规则交给角色，派发选角色（[`crate::profiles`]）
    #[serde(default)]
    pub profiles: Vec<crate::profiles::Profile>,
}

impl R<'_> {
    /// 存着的领取规则和兜底
    pub(crate) fn sched_rules(&self) -> Result<(Vec<ClaimRule>, Option<String>)> {
        let row: Option<(String, Option<String>)> = self.conn.query_row("SELECT rules, fallback FROM sched WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let (raw, fallback) = row.unwrap_or_else(|| ("[]".into(), None));
        let rules = serde_json::from_str::<Vec<ClaimRule>>(&raw).unwrap_or_default().into_iter().map(ClaimRule::upgraded).collect();
        Ok((rules, fallback))
    }

    /// 每个 agent 明确不参与的机器 id
    pub(crate) fn sched_picks(&self) -> Result<Picks> {
        let raw: Option<String> = self.conn.query_row("SELECT picks FROM sched WHERE id = 1", [], |r| r.get(0)).optional()?;
        Ok(parse_picks(raw.as_deref().unwrap_or("{}")))
    }

    /// 跑多久停：单次最长运行、没输出多久算卡住（分钟）
    pub(crate) fn run_limits(&self) -> Result<(i64, i64)> {
        Ok(self
            .conn
            .query_row("SELECT max_run, idle_run FROM sched WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .unwrap_or((crate::hosts::MAX_RUN_DEFAULT, crate::hosts::IDLE_RUN_DEFAULT)))
    }

    /// 自动升级开没开
    pub(crate) fn auto_upgrade(&self) -> Result<bool> {
        Ok(self.conn.query_row("SELECT auto_upgrade FROM sched WHERE id = 1", [], |r| r.get::<_, i64>(0)).optional()?.unwrap_or(0) != 0)
    }

    /// 所有项目里的标签名（小写）
    pub(crate) fn all_labels(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT DISTINCT lower(name) FROM labels")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 机器被移除之后，把它从各条规则的「由这些机器做」里清掉。
    /// 存的是机器 id；清完某个 agent 的排除列表后，这个 agent 回到「所有装了它的机器都参与」
    pub(crate) fn drop_host_picks(&self, host: i64) -> Result<()> {
        let mut picks = self.sched_picks()?;
        let mut changed = false;
        picks.retain(|_, off| {
            let before = off.len();
            off.retain(|h| *h != host);
            changed |= off.len() != before;
            !off.is_empty()
        });
        if changed {
            let raw = serde_json::to_string(&picks).unwrap_or_else(|_| "{}".into());
            self.conn.execute("UPDATE sched SET picks = ?1 WHERE id = 1", [raw])?;
        }
        Ok(())
    }

    /// 调度的全局设置：规则带上生不生效、哪些标签已经不存在
    pub fn sched(&self, now: i64) -> Result<Sched> {
        let (rules, _) = self.sched_rules()?;
        let hosts = self.host_rows()?;
        let known = self.all_labels()?;
        let profiles = self.profiles()?;
        Ok(Sched {
            rules: rules
                .into_iter()
                .map(|r| ClaimRule {
                    active: hosts.iter().any(|h| h.can_run(&r.profile)),
                    agent: profiles.iter().find(|p| p.id == r.profile).map(|p| p.agent.clone()).unwrap_or_default(),
                    missing: r.labels.iter().filter(|l| !known.contains(&l.to_lowercase())).cloned().collect(),
                    ..r
                })
                .collect(),
            profiles,
            tz: std::env::var("TZ").unwrap_or_default().trim().to_string(),
            minute: server_minute(now),
            auto_upgrade: self.auto_upgrade()?,
            max_run: self.run_limits()?.0,
            idle_run: self.run_limits()?.1,
        })
    }
}

impl W<'_> {
    /// 改领取规则（整个列表替换）和兜底（空字符串表示取消兜底）；`None` 表示不改。
    /// 新加或改过的规则：标签要在某个项目里有，agent 要至少在一台机器上找得到；原样保留的规则不拦（标签之后被删了、
    /// agent 之后找不到了照样存着，界面上标出来）。
    /// 还没有哪台机器上报找得到的 agent 时（没接机器，先在 Agent tab 配好规则），内置的 agent 都能选，和界面的下拉一致；
    /// 这样存下的规则在有机器上报它之前不生效
    pub fn sched_update(&mut self, rules: Option<&[ClaimRule]>, fallback: Option<&str>) -> Result<Sched> {
        let (old, _) = self.read().sched_rules()?;
        let mut profiles = self.read().profiles()?;
        let hosts = self.read().host_rows()?;
        let all: Vec<String> = {
            let mut v: Vec<String> = hosts.iter().flat_map(|h| h.resolved().into_iter().filter(|a| a.found).map(|a| a.id)).collect();
            v.sort();
            v.dedup();
            v
        };
        let builtin: Vec<&str> = crate::agentdef::ids().collect();
        let found = |agent: &str| if all.is_empty() { builtin.contains(&agent) } else { hosts.iter().any(|h| h.has_agent(agent)) };
        let options = if all.is_empty() { format!("{}（还没有机器上报 agent，先选内置的）", builtin.join("、")) } else { all.join("、") };
        if let Some(rules) = rules {
            if rules.len() > RULES_LIMIT {
                return Err(BoardError::usage(format!("领取规则最多 {RULES_LIMIT} 条")));
            }
            let known = self.read().all_labels()?;
            let mut out: Vec<ClaimRule> = vec![];
            let mut seen: Vec<ClaimRule> = vec![];
            for r in rules {
                let mut r = r.clone().upgraded();
                if !RULE_OPS.contains(&r.op.as_str()) {
                    return Err(BoardError::usage(format!("领取规则的条件「{}」不对：只能是 {}", r.op, RULE_OPS.join("、"))));
                }
                if r.op != "none" && r.labels.is_empty() {
                    return Err(BoardError::usage("领取规则至少要选一个标签"));
                }
                // 交给的角色：按标识或名字找。旧版界面（和 `board` 命令行）只给 agent 时，交给这个 agent 的第一个角色；
                // 还没有时建一个同名的（本机登录、不指定模型，和升级时一样），agent 要至少在一台机器上找得到
                let agent = r.agent.trim().to_string();
                if r.profile.trim().is_empty() && !agent.is_empty() && !profiles.iter().any(|p| p.agent == agent) {
                    if !old.iter().any(|x| x.agent == agent) && !found(&agent) {
                        return Err(BoardError::usage(format!("没有机器上找得到 agent「{agent}」，可选：{options}")));
                    }
                    let made = self.profile_put(&crate::profiles::Profile { name: crate::profiles::free_name(&profiles, &agent), agent: agent.clone(), ..Default::default() }, "领取规则")?;
                    profiles.push(made);
                }
                let p = if !r.profile.trim().is_empty() {
                    profiles.iter().find(|p| p.id == r.profile.trim() || p.name.to_lowercase() == r.profile.trim().to_lowercase())
                } else {
                    profiles.iter().find(|p| !agent.is_empty() && p.agent == agent)
                };
                let Some(p) = p else {
                    let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
                    let names = if names.is_empty() { "（还没有角色，先新建一个）".to_string() } else { names.join("、") };
                    return Err(BoardError::usage(format!("领取规则要交给一个角色，可选：{names}")));
                };
                r.profile = p.id.clone();
                if !old.iter().any(|x| same_cond(x, &r)) {
                    if let Some(l) = r.labels.iter().find(|l| !known.contains(&l.to_lowercase())) {
                        return Err(BoardError::usage(format!("没有哪个项目里有标签「{l}」")));
                    }
                }
                if seen.iter().any(|x| x.op == r.op && {
                    let low = |v: &[String]| {
                        let mut v: Vec<String> = v.iter().map(|l| l.to_lowercase()).collect();
                        v.sort();
                        v
                    };
                    low(&x.labels) == low(&r.labels)
                }) {
                    return Err(BoardError::usage("领取规则里有重复的条件"));
                }
                seen.push(r.clone());
                out.push(r);
            }
            let raw = serde_json::to_string(&out.iter().map(ClaimRule::stored).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
            self.tx.execute("INSERT INTO sched (id, rules) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET rules = ?1", [raw])?;
        }
        // 兜底去掉了（规则都没命中的卡不自动领）：旧版界面发来的兜底只认「取消」
        if let Some(f) = fallback {
            if !f.trim().is_empty() {
                return Err(BoardError::usage("没有兜底了：规则都没命中的卡不自动领。要自动领就加一条规则交给一个角色"));
            }
            self.tx.execute("UPDATE sched SET fallback = NULL WHERE id = 1", [])?;
        }
        self.changed = true;
        self.hosts_changed = true;
        self.read().sched(self.now)
    }


    /// 跑多久停（分钟，0 = 不限）：所有机器一个值；`None` 表示不改
    pub fn set_run_limits(&mut self, max_run: Option<i64>, idle_run: Option<i64>) -> Result<Sched> {
        use crate::hosts::{IDLE_RUN_LIMIT, MAX_RUN_LIMIT};
        if let Some(n) = max_run {
            if !(0..=MAX_RUN_LIMIT).contains(&n) {
                return Err(BoardError::usage(format!("单次最长运行要在 1 到 {MAX_RUN_LIMIT} 分钟之间，或者不限（0）")));
            }
            self.tx.execute("INSERT INTO sched (id, max_run) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET max_run = ?1", [n])?;
        }
        if let Some(n) = idle_run {
            if !(0..=IDLE_RUN_LIMIT).contains(&n) {
                return Err(BoardError::usage(format!("没输出多久算卡住要在 1 到 {IDLE_RUN_LIMIT} 分钟之间，或者不限（0）")));
            }
            self.tx.execute("INSERT INTO sched (id, idle_run) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET idle_run = ?1", [n])?;
        }
        self.changed = true;
        self.hosts_changed = true;
        self.read().sched(self.now)
    }

    /// 开、关自动升级。打开后各台机器下一次上报时就会按服务端的版本检查一遍
    pub fn set_auto_upgrade(&mut self, on: bool) -> Result<Sched> {
        self.tx.execute("INSERT INTO sched (id, auto_upgrade) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET auto_upgrade = ?1", [on as i64])?;
        self.changed = true;
        self.hosts_changed = true;
        self.read().sched(self.now)
    }

    /// 这块白板参不参与自动领取（不参与的只是不被自动领取，手动派发照常）
    pub fn set_board_auto(&mut self, path: &str, on: bool) -> Result<()> {
        let b = self.resolve_exact(path)?;
        self.tx.execute("UPDATE boards SET auto = ?1 WHERE id = ?2", params![on as i64, b.id])?;
        self.changed = true;
        self.hosts_changed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_are_validated_sorted_and_merged() {
        assert_eq!(tidy_windows(&[[1320, 1440], [0, 480]]).unwrap(), vec![[0, 480], [1320, 1440]]);
        assert_eq!(tidy_windows(&[[540, 720], [720, 1080], [600, 660]]).unwrap(), vec![[540, 1080]], "挨着、重叠的并成一段");
        assert_eq!(tidy_windows(&[]).unwrap(), Vec::<Window>::new(), "一段都没有 = 不接单");
        for bad in [[-1, 10], [10, 10], [20, 10], [0, 1441]] {
            assert!(tidy_windows(&[bad]).is_err(), "{bad:?}");
        }
        assert!(tidy_windows(&vec![[0, 1]; WINDOWS_LIMIT + 1]).is_err());
    }

    #[test]
    fn minute_is_inside_half_open_windows() {
        let night = [[0, 480], [1320, 1440]];
        for (m, want) in [(0, true), (479, true), (480, false), (860, false), (1320, true), (1439, true)] {
            assert_eq!(in_windows(&night, m), want, "{m}");
        }
        assert!(in_windows(&all_day(), 0) && in_windows(&all_day(), 1439));
        assert!(!in_windows(&[], 600));
    }
}
