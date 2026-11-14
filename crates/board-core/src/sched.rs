//! 调度（PRD §9.1「自动领取」）：哪些卡自动交给哪个 agent，在服务端存一份，所有机器共用。
//!
//! - **领取规则**（`sched` 表，只有一行）：有顺序的「条件 → agent」和一条兜底。以前每台机器各存一份
//!   （`hosts.rules`、`hosts.fallback`），几台机器要靠「复制设置」保持一致；现在读机器记录时直接带上这一份
//!   （`hosts.rs` 的 `HOST_COLS`），按规则挑卡片的代码不用分机器的还是全局的。
//! - **白板参不参与**（`boards.auto`，默认参与）：不参与的白板不会被自动领取，手动派发照常。
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

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sched (
  id       INTEGER PRIMARY KEY CHECK (id = 1),
  rules    TEXT NOT NULL DEFAULT '[]',   -- 领取规则，有顺序：[{op, labels, agent}]
  fallback TEXT                           -- 兜底的 agent；空表示规则没命中的卡片不领
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

/// 旧的定时领取表达式是不是「全天每隔几分钟领一轮」（`* * * * *`、`*/5 * * * *`）：这种迁移成全天接单
pub fn all_day_cron(expr: &str) -> bool {
    let p: Vec<&str> = expr.split_whitespace().collect();
    p.len() == 5 && (p[0] == "*" || p[0].strip_prefix("*/").is_some_and(|n| n.parse::<u32>().is_ok())) && p[1..].iter().all(|x| *x == "*")
}

fn same_rule(a: &ClaimRule, b: &ClaimRule) -> bool {
    let low = |v: &[String]| {
        let mut v: Vec<String> = v.iter().map(|l| l.to_lowercase()).collect();
        v.sort();
        v
    };
    a.op == b.op && a.agent == b.agent && low(&a.labels) == low(&b.labels)
}

/// 升级：
/// - 加上 `hosts.auto`、`hosts.windows`、`boards.auto` 三列；
/// - 第一次（`sched` 表还是空的）把各台机器上的领取规则并成全局的一份：按机器建立的先后依次接上，一模一样的只留一条；
///   兜底取第一台设了的。几台机器的规则不一样时合并后的顺序可能不是想要的，由人在「调度」里调；
/// - 机器的自动接单：原来开着定时领取、有规则，并且时间表是「全天每隔几分钟」的，迁移成开着、全天；
///   时间表是每天某几个时间点的没法换成时段，迁移成**关着**（免得白天也开始接单），在这台机器的操作记录里写明要重新设
pub(crate) fn migrate(conn: &rusqlite::Connection, now: i64) -> rusqlite::Result<()> {
    let has = |table: &str, col: &str| -> rusqlite::Result<bool> {
        conn.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"), [col], |r| r.get::<_, i64>(0)).map(|n| n > 0)
    };
    for (table, col, def) in [
        ("hosts", "auto", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "windows", "TEXT NOT NULL DEFAULT '[[0,1440]]'"),
        ("boards", "auto", "INTEGER NOT NULL DEFAULT 1"),
    ] {
        if !has(table, col)? {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"))?;
        }
    }
    let done: i64 = conn.query_row("SELECT COUNT(*) FROM sched", [], |r| r.get(0))?;
    if done > 0 {
        return Ok(());
    }
    let mut st = conn.prepare("SELECT id, rules, fallback, cron_on, cron FROM hosts ORDER BY id")?;
    let hosts = st
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, i64>(3)?, r.get::<_, String>(4)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut rules: Vec<ClaimRule> = vec![];
    let mut fallback: Option<String> = None;
    for (id, raw, fb, cron_on, cron) in &hosts {
        let mine: Vec<ClaimRule> = serde_json::from_str::<Vec<ClaimRule>>(raw).unwrap_or_default().into_iter().map(ClaimRule::upgraded).collect();
        for r in &mine {
            if rules.len() < RULES_LIMIT && !rules.iter().any(|x| same_rule(x, r)) {
                rules.push(r.clone());
            }
        }
        if fallback.is_none() {
            fallback = fb.clone().map(|f| f.trim().to_string()).filter(|f| !f.is_empty());
        }
        let crons: Vec<String> = serde_json::from_str(cron).unwrap_or_default();
        let was_on = *cron_on != 0 && !mine.is_empty() && !crons.is_empty();
        let auto = was_on && crons.iter().all(|c| all_day_cron(c));
        conn.execute("UPDATE hosts SET auto = ?2 WHERE id = ?1", params![id, auto as i64])?;
        if was_on && !auto {
            let text = format!(
                "升级：定时领取换成了接单时段。这台机器原来的时间表（{}）是每天的时间点，换不成时段，自动接单先关着：在「机器 → 共用配置 → 调度」里拖出它的接单时段，再打开自动接单",
                crons.join("、")
            );
            conn.execute("INSERT INTO host_ops (host_id, at, by_name, text) VALUES (?1, ?2, 'board', ?3)", params![id, now, text])?;
        }
    }
    let raw = serde_json::to_string(&rules.iter().map(ClaimRule::stored).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
    conn.execute("INSERT INTO sched (id, rules, fallback) VALUES (1, ?1, ?2)", params![raw, fallback])?;
    Ok(())
}

/// 调度的全局设置（随工作区数据发给界面）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sched {
    /// 领取规则，有顺序；`active` 是「至少有一台机器上找得到这个 agent」，`missing` 是所有项目里都没有的标签
    pub rules: Vec<ClaimRule>,
    /// 兜底的 agent；没有时规则没命中的卡片不领
    #[serde(default)]
    pub fallback: Option<String>,
    /// 至少有一台机器上找得到兜底的 agent
    #[serde(default)]
    pub fallback_active: bool,
    /// 服务端的时区（环境变量 `TZ`，没设时为空）：界面上写在「服务器时间」后面
    #[serde(default)]
    pub tz: String,
    /// 现在是服务端时区一天里的第几分钟：界面上画「现在」那条线、判断机器在不在时段里
    pub minute: i64,
}

impl R<'_> {
    /// 存着的领取规则和兜底
    pub(crate) fn sched_rules(&self) -> Result<(Vec<ClaimRule>, Option<String>)> {
        let row: Option<(String, Option<String>)> = self.conn.query_row("SELECT rules, fallback FROM sched WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let (raw, fallback) = row.unwrap_or_else(|| ("[]".into(), None));
        let rules = serde_json::from_str::<Vec<ClaimRule>>(&raw).unwrap_or_default().into_iter().map(ClaimRule::upgraded).collect();
        Ok((rules, fallback))
    }

    /// 所有项目里的标签名（小写）
    pub(crate) fn all_labels(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT DISTINCT lower(name) FROM labels")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 调度的全局设置：规则带上生不生效、哪些标签已经不存在
    pub fn sched(&self, now: i64) -> Result<Sched> {
        let (rules, fallback) = self.sched_rules()?;
        let hosts = self.host_rows()?;
        let known = self.all_labels()?;
        let found = |agent: &str| hosts.iter().any(|h| h.has_agent(agent));
        Ok(Sched {
            rules: rules
                .into_iter()
                .map(|r| ClaimRule { active: found(&r.agent), missing: r.labels.iter().filter(|l| !known.contains(&l.to_lowercase())).cloned().collect(), ..r })
                .collect(),
            fallback_active: fallback.as_deref().is_some_and(found),
            fallback,
            tz: std::env::var("TZ").unwrap_or_default().trim().to_string(),
            minute: server_minute(now),
        })
    }
}

impl W<'_> {
    /// 改领取规则（整个列表替换）和兜底（空字符串表示取消兜底）；`None` 表示不改。
    /// 新加或改过的规则：标签要在某个项目里有，agent 要至少在一台机器上找得到；原样保留的规则不拦（标签之后被删了、
    /// agent 之后找不到了照样存着，界面上标出来）
    pub fn sched_update(&mut self, rules: Option<&[ClaimRule]>, fallback: Option<&str>) -> Result<Sched> {
        let (old, old_fallback) = self.read().sched_rules()?;
        let hosts = self.read().host_rows()?;
        let found = |agent: &str| hosts.iter().any(|h| h.has_agent(agent));
        let all: Vec<String> = {
            let mut v: Vec<String> = hosts.iter().flat_map(|h| h.resolved().into_iter().filter(|a| a.found).map(|a| a.id)).collect();
            v.sort();
            v.dedup();
            v
        };
        let options = if all.is_empty() { "（还没有机器上报 agent）".to_string() } else { all.join("、") };
        if let Some(rules) = rules {
            if rules.len() > RULES_LIMIT {
                return Err(BoardError::usage(format!("领取规则最多 {RULES_LIMIT} 条")));
            }
            let known = self.read().all_labels()?;
            let mut out: Vec<ClaimRule> = vec![];
            for r in rules {
                let r = r.clone().upgraded();
                if !RULE_OPS.contains(&r.op.as_str()) {
                    return Err(BoardError::usage(format!("领取规则的条件「{}」不对：只能是 {}", r.op, RULE_OPS.join("、"))));
                }
                if r.op != "none" && r.labels.is_empty() {
                    return Err(BoardError::usage("领取规则至少要选一个标签"));
                }
                if !old.iter().any(|x| same_rule(x, &r)) {
                    if let Some(l) = r.labels.iter().find(|l| !known.contains(&l.to_lowercase())) {
                        return Err(BoardError::usage(format!("没有哪个项目里有标签「{l}」")));
                    }
                    if !found(&r.agent) {
                        return Err(BoardError::usage(format!("没有机器上找得到 agent「{}」，可选：{options}", r.agent)));
                    }
                }
                out.push(r);
            }
            let raw = serde_json::to_string(&out.iter().map(ClaimRule::stored).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
            self.tx.execute("INSERT INTO sched (id, rules) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET rules = ?1", [raw])?;
        }
        if let Some(f) = fallback {
            let f = f.trim();
            if f.is_empty() {
                self.tx.execute("INSERT INTO sched (id, fallback) VALUES (1, NULL) ON CONFLICT(id) DO UPDATE SET fallback = NULL", [])?;
            } else {
                if old_fallback.as_deref() != Some(f) && !found(f) {
                    return Err(BoardError::usage(format!("没有机器上找得到 agent「{f}」，可选：{options}")));
                }
                self.tx.execute("INSERT INTO sched (id, fallback) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET fallback = ?1", [f])?;
            }
        }
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

    #[test]
    fn only_every_n_minutes_all_day_counts_as_all_day() {
        for (expr, want) in [("* * * * *", true), ("*/5 * * * *", true), ("*/30  *  * * *", true), ("0 23 * * *", false), ("*/5 9-18 * * *", false), ("0 * * * *", false), ("*/5 * * * 1-5", false), ("bad", false)] {
            assert_eq!(all_day_cron(expr), want, "{expr}");
        }
    }
}
