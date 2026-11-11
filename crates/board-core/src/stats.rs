//! 大盘统计（PRD §10.13、§11 `GET /api/stats`）。
//!
//! 一次调用拿到一个范围（一块白板、一个项目、全部项目）在一段时间里的全部数据，全部在服务端算。
//! 项目和全部项目是范围内各白板的合计：比例和分位数用合并后的样本重新算，不是各白板取平均。
//!
//! 数据来源：`cards`（新建时间、现在的状态）、`moves`（每次换状态）、`card_runs`（每次领取）、
//! `activity`（评论数、热力图）、`dispatches`（派发启动的名字合并成「agent 标识/机器名」）。
//!
//! 状态的角色按工作流规则和连线判断（与 board-skills 认状态的规则一致）：
//! 可领取（`claim`）、终点（`final`）、需要人处理（锁定状态连出去、名称里带「人工」之类关键词，以及错误节点）、
//! 交审（锁定状态连出去、不是可领取 / 锁定 / 终点 / 错误节点、也不是需要人处理的状态）。

use crate::error::{BoardError, ErrorCode, Result};
use crate::model::{Flow, INBOX, INBOX_NAME};
use crate::store::{flow_of, R};
use jiff::{tz::TimeZone, RoundMode, Timestamp, ToSpan, Unit, Zoned, ZonedRound};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;
/// 空转：单段满 15 分钟才计入
const IDLE_MIN: i64 = 15 * MIN;
/// 区间不超过 3 天按小时分桶，否则按天
const HOURLY_MAX: i64 = 3 * DAY;
/// 区间最长 400 天
const SPAN_MAX: i64 = 400 * DAY;
/// 交付周期分布：7 档的上界（最后一档「3 天以上」没有上界）
const LEAD_BINS: [i64; 6] = [HOUR, 2 * HOUR, 4 * HOUR, 8 * HOUR, DAY, 3 * DAY];
/// 日历热力图看最近多少周
const CAL_WEEKS: i64 = 26;
/// 卡片流转明细最多几张
const LIVES_MAX: usize = 40;
/// 「正在卡着的卡」几张
const STUCK_MAX: usize = 6;
/// 附注里评论最多几个字
const NOTE_MAX: usize = 40;

/* ============================== 请求 ============================== */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Board,
    Project,
    All,
}

impl Scope {
    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "board" => Some(Scope::Board),
            "project" => Some(Scope::Project),
            "all" => Some(Scope::All),
            _ => None,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Scope::Board => "board",
            Scope::Project => "project",
            Scope::All => "all",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatsQuery {
    pub scope: Scope,
    /// 白板（项目/白板）或项目标识；全部项目时为空
    pub path: Option<String>,
    /// 区间 [from, to)，毫秒
    pub from: i64,
    pub to: i64,
    /// IANA 时区（浏览器的），按它分桶
    pub tz: String,
}

/* ============================== 返回 ============================== */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub scope: &'static str,
    pub path: Option<String>,
    pub from: i64,
    pub to: i64,
    pub now: i64,
    pub tz: String,
    /// hour | day
    pub bucket: &'static str,
    /// 范围内的白板（项目/白板）
    pub boards_in_scope: Vec<String>,
    /// 区间内的动态条数；为 0 时界面显示「这段时间没有动态」
    pub activity: i64,
    pub cur: Period,
    /// 上一周期 [from - (to - from), from)
    pub prev: Period,
    pub series: Vec<Bucket>,
    /// 区间内的空转时段（到现在为止）
    pub idle: Vec<IdleSpan>,
    pub agents: Vec<AgentRow>,
    pub dwell: Vec<Dwell>,
    /// 7 × 24：周一..周日 × 0..23 点，值是动态条数
    pub hour_heat: Vec<Vec<i64>>,
    /// 最近 26 周每天的动态条数（周一开始，不受 from / to 限制）
    pub day_heat: Vec<HeatDay>,
    /// 区间内有交集的每次领取
    pub runs: Vec<RunRow>,
    /// 区间内完成的卡片，按完成时间倒序，最多 40 张
    pub lives: Vec<Life>,
    /// 现在（不受时间范围影响）
    pub present: Present,
    /// 创建以来（白板和项目范围）
    pub life: Option<Lifetime>,
    /// 侧栏和对比表：每块白板、每个项目、全部项目的几个数
    pub boards: Vec<Brief>,
    pub projects: Vec<Brief>,
    pub all: Brief,
}

/// 一段时间的总量
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Period {
    pub created: i64,
    pub claims: i64,
    pub done: i64,
    pub rework: i64,
    pub comments: i64,
    pub beats: i64,
    /// 累计耗时（毫秒）：开始时间在区间内、已结束的领取
    pub spent: i64,
    /// 交付周期（毫秒）：中位、P85；没有完成的卡片时为空
    pub lead_p50: Option<i64>,
    pub lead_p85: Option<i64>,
    /// 排队（新建 → 第一次领取）、处理（第一次领取 → 完成）的中位
    pub wait_p50: Option<i64>,
    pub work_p50: Option<i64>,
    /// 交付周期分布：7 档的张数
    pub lead_hist: Vec<i64>,
    /// 一次通过率 0..1；没有完成的卡片时为空
    pub first_pass: Option<f64>,
    /// 空转（毫秒）
    pub idle: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    /// 桶的开始时间
    pub t: i64,
    pub created: i64,
    pub claims: i64,
    pub done: i64,
    pub rework: i64,
    pub comments: i64,
    pub beats: i64,
    pub spent: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleSpan {
    pub board: String,
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRow {
    /// 派发启动的是「agent 标识/机器名」，其他是名字
    pub agent: String,
    pub claims: i64,
    pub spent: i64,
    /// 它最后一次领取后完成的卡片里，没有返工的比例；没有这样的卡片时为空
    pub first_pass: Option<f64>,
    pub lost: i64,
    pub gaveup: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dwell {
    pub key: String,
    pub name: String,
    pub color: String,
    pub median: i64,
    pub mean: i64,
    pub n: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatDay {
    pub t: i64,
    pub v: i64,
    /// 还没到的日子
    pub future: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRow {
    /// 合并后的名字（「agent 标识/机器名」或名字）
    pub agent: String,
    /// 领取时用的名字
    pub name: String,
    pub board: String,
    pub card: i64,
    pub title: String,
    pub start: i64,
    /// 还在进行中时为空
    pub end: Option<i64>,
    /// review 交审 | rework 被打回 | lost 异常退出 | gaveup 做不下去 | released 领取被释放 | running 进行中
    pub result: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Life {
    pub board: String,
    pub card: i64,
    pub title: String,
    pub created: i64,
    pub done: i64,
    pub reworks: i64,
    pub segs: Vec<Seg>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seg {
    pub key: Option<String>,
    pub name: String,
    /// 状态现在的颜色；状态已经不在时为空
    pub color: Option<String>,
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Present {
    /// 每个状态现在的卡片数（含「未整理」，按工作流顺序）
    pub statuses: Vec<StatusCount>,
    pub review: Waiting,
    pub todo: Waiting,
    pub doing: Doing,
    pub stuck: Vec<Stuck>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusCount {
    pub key: String,
    pub name: String,
    pub color: String,
    pub n: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub n: i64,
    /// 等得最久的一张等了多久（毫秒）；没有卡片时为空
    pub oldest: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Doing {
    pub n: i64,
    pub holders: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stuck {
    pub board: String,
    pub card: i64,
    pub title: String,
    pub key: String,
    pub name: String,
    pub color: String,
    /// 进入现在这个状态的时间
    pub since: i64,
    pub holder: Option<String>,
    /// 没有持有者时：最近一条评论的第一行（最多 40 字）
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lifetime {
    /// 白板或项目的创建时间
    pub since: i64,
    pub days: i64,
    pub cards: i64,
    /// 现在在终点状态的张数
    pub finals: i64,
    pub reworks: i64,
    pub agents: i64,
    /// agent 工时（毫秒）：全部已结束领取的耗时之和
    pub spent: i64,
    /// 平均每天完成：终点状态张数 ÷ 创建以来的天数，保留一位小数
    pub per_day: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Brief {
    /// 白板是「项目/白板」，项目是项目标识，全部项目为空
    pub key: String,
    /// 现在：待复盘、待办的张数，有没有 agent 在干活
    pub review: i64,
    pub todo: i64,
    pub busy: bool,
    /// 这段时间
    pub done: i64,
    pub series: Vec<i64>,
    pub lead_p50: Option<i64>,
    pub first_pass: Option<f64>,
    pub rework: i64,
}

/* ============================== 读进来的数据 ============================== */

struct BoardD {
    id: i64,
    path: String,
    project: String,
    created_at: i64,
    flow: Flow,
    roles: Roles,
}

/// 状态的角色（统计用）
#[derive(Default)]
struct Roles {
    claim: HashSet<String>,
    fin: HashSet<String>,
    lock: HashSet<String>,
    review: HashSet<String>,
    human: HashSet<String>,
}

impl Roles {
    fn of(flow: &Flow) -> Roles {
        let mut r = Roles::default();
        for s in &flow.0 {
            let k = s.key.clone();
            let from_lock = flow.0.iter().any(|l| l.lock && l.to.iter().any(|t| *t == s.key));
            if s.claim {
                r.claim.insert(k.clone());
            }
            if s.fin {
                r.fin.insert(k.clone());
            }
            if s.lock {
                r.lock.insert(k.clone());
            }
            if s.err || flow.is_human(&s.key) {
                r.human.insert(k);
            } else if from_lock && !s.claim && !s.fin && !s.lock {
                r.review.insert(k);
            }
        }
        r
    }
    fn is(set: &HashSet<String>, key: Option<&str>) -> bool {
        key.is_some_and(|k| set.contains(k))
    }
}

struct Mv {
    at: i64,
    kind: String,
    from: Option<String>,
    from_name: Option<String>,
    to: Option<String>,
    to_name: String,
    /// agent 移动时是它的名字
    agent: Option<String>,
}

struct Run {
    n: i64,
    agent: String,
    start: i64,
    end: Option<i64>,
    how: Option<String>,
    beats: i64,
}

/// 在一个状态里的一段
struct SegD {
    key: Option<String>,
    name: String,
    from: i64,
    to: Option<i64>,
}

struct CardD {
    b: usize,
    id: i64,
    title: String,
    col: String,
    holder: Option<String>,
    created: i64,
    moves: Vec<Mv>,
    runs: Vec<Run>,
    /// 移入终点状态的时间
    done_at: Vec<i64>,
    /// 返工：移入可领取状态、且此前已有领取记录
    rework_at: Vec<i64>,
    first_claim: Option<i64>,
    segs: Vec<SegD>,
}

impl CardD {
    fn derive(&mut self, b: &BoardD) {
        let r = &b.roles;
        self.done_at = self.moves.iter().filter(|m| Roles::is(&r.fin, m.to.as_deref())).map(|m| m.at).collect();
        let claims: Vec<i64> = self.moves.iter().filter(|m| m.kind == "claim").map(|m| m.at).collect();
        self.first_claim = claims.iter().copied().chain(self.runs.iter().map(|x| x.start)).min();
        self.rework_at = self
            .moves
            .iter()
            .filter(|m| m.kind == "move" && Roles::is(&r.claim, m.to.as_deref()))
            .filter(|m| claims.iter().any(|t| *t < m.at) || self.runs.iter().any(|x| x.start < m.at))
            .map(|m| m.at)
            .collect();
        // 新建时在哪：第一次移动的出发状态；没有移动过就是现在的状态
        let (k0, n0) = match self.moves.first() {
            Some(m) => (m.from.clone(), m.from_name.clone()),
            None => (Some(self.col.clone()), None),
        };
        let name_of = |k: &Option<String>, n: Option<String>| n.unwrap_or_else(|| k.as_deref().map(|k| b.flow.name(k).to_string()).unwrap_or_default());
        let mut cur = SegD { name: name_of(&k0, n0), key: k0, from: self.created, to: None };
        let mut segs = vec![];
        for m in &self.moves {
            let next = SegD { key: m.to.clone(), name: m.to_name.clone(), from: m.at, to: None };
            cur.to = Some(m.at);
            segs.push(std::mem::replace(&mut cur, next));
        }
        segs.push(cur);
        self.segs = segs;
    }

    /// 进入现在这个状态的时间
    fn entered(&self) -> i64 {
        self.moves.last().map(|m| m.at).unwrap_or(self.created)
    }

    /// 区间内最后一次完成的时间
    fn done_in(&self, a: i64, b: i64) -> Option<i64> {
        self.done_at.iter().copied().filter(|t| *t >= a && *t < b).max()
    }
}

fn quantile(xs: &[i64], q: f64) -> Option<i64> {
    if xs.is_empty() {
        return None;
    }
    let mut v = xs.to_vec();
    v.sort_unstable();
    let i = ((q * v.len() as f64).floor() as usize).min(v.len() - 1);
    Some(v[i])
}

fn ratio(a: i64, b: i64) -> Option<f64> {
    (b > 0).then(|| a as f64 / b as f64)
}

/// 交付周期样本：完成张数、返工次数、交付周期 / 排队 / 处理、一次通过的张数
#[derive(Default)]
struct Outcome {
    done: i64,
    rework: i64,
    leads: Vec<i64>,
    waits: Vec<i64>,
    works: Vec<i64>,
    pass: i64,
}

impl Outcome {
    fn of<'a>(cards: impl Iterator<Item = &'a CardD>, a: i64, b: i64) -> Outcome {
        let mut o = Outcome::default();
        for c in cards {
            o.rework += c.rework_at.iter().filter(|t| **t >= a && **t < b).count() as i64;
            let Some(t) = c.done_in(a, b) else { continue };
            o.done += 1;
            o.leads.push(t - c.created);
            if let Some(fc) = c.first_claim.filter(|fc| *fc <= t) {
                o.waits.push(fc - c.created);
                o.works.push(t - fc);
            }
            if !c.rework_at.iter().any(|r| *r <= t) {
                o.pass += 1;
            }
        }
        o
    }
    fn first_pass(&self) -> Option<f64> {
        ratio(self.pass, self.done)
    }
}

/* ============================== 时区与分桶 ============================== */

fn zoned(ms: i64, tz: &TimeZone) -> Result<Zoned> {
    Ok(Timestamp::from_millisecond(ms).map_err(time_err)?.to_zoned(tz.clone()))
}

fn ms_of(z: &Zoned) -> i64 {
    z.timestamp().as_millisecond()
}

fn time_err(e: jiff::Error) -> BoardError {
    BoardError::usage(format!("时间超出范围：{e}"))
}

/// 分桶的开始时间：小时桶从 from 所在的那个小时开始，天桶从 from 那天的零点开始（都按 tz）
fn bucket_starts(from: i64, to: i64, hourly: bool, tz: &TimeZone) -> Result<Vec<i64>> {
    let z = zoned(from, tz)?;
    let mut z = if hourly {
        z.round(ZonedRound::new().smallest(Unit::Hour).mode(RoundMode::Trunc)).map_err(time_err)?
    } else {
        z.start_of_day().map_err(time_err)?
    };
    let mut out = vec![];
    while ms_of(&z) < to {
        out.push(ms_of(&z));
        z = if hourly { z.checked_add(1.hour()) } else { z.tomorrow().and_then(|d| d.start_of_day()) }.map_err(time_err)?;
    }
    Ok(out)
}

/// t 落在第几个桶（t 要在 [from, to) 里）
fn bucket_of(starts: &[i64], t: i64) -> Option<usize> {
    let i = starts.partition_point(|s| *s <= t);
    (i > 0).then(|| i - 1)
}

/* ============================== 计算 ============================== */

struct Data {
    boards: Vec<BoardD>,
    cards: Vec<CardD>,
    /// 每块白板的卡片下标
    by_board: Vec<Vec<usize>>,
    /// 派发启动的名字 → 「agent 标识/机器名」
    merge: HashMap<String, String>,
    projects: Vec<(String, i64)>,
}

impl Data {
    fn label(&self, name: &str) -> String {
        self.merge.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    fn color(&self, b: usize, key: &str) -> Option<String> {
        if key == INBOX {
            return Some("inbox".into());
        }
        self.boards[b].flow.get(key).map(|s| s.color.clone())
    }

    fn cards_of<'a>(&'a self, boards: &'a [usize]) -> impl Iterator<Item = &'a CardD> + 'a {
        boards.iter().flat_map(move |b| self.by_board[*b].iter().map(move |i| &self.cards[*i]))
    }

    /// 一块白板在 [a, end) 里的空转时段：可领取状态里至少有一张卡、而白板上没有任何进行中的领取，单段满 15 分钟
    fn idle_spans(&self, b: usize, a: i64, end: i64, now: i64) -> Vec<(i64, i64)> {
        if end <= a {
            return vec![];
        }
        let r = &self.boards[b].roles;
        let mut ev: Vec<(i64, i32, i32)> = vec![];
        for i in &self.by_board[b] {
            let c = &self.cards[*i];
            for s in &c.segs {
                let to = s.to.unwrap_or(i64::MAX);
                if Roles::is(&r.claim, s.key.as_deref()) {
                    ev.push((s.from, 1, 0));
                    ev.push((to, -1, 0));
                }
                if Roles::is(&r.lock, s.key.as_deref()) {
                    ev.push((s.from, 0, 1));
                    ev.push((to, 0, -1));
                }
            }
            for x in &c.runs {
                ev.push((x.start, 0, 1));
                ev.push((x.end.unwrap_or(now.max(x.start)), 0, -1));
            }
        }
        ev.sort_unstable_by_key(|e| e.0);
        let (mut q, mut busy) = (0, 0);
        let mut i = 0;
        while i < ev.len() && ev[i].0 <= a {
            q += ev[i].1;
            busy += ev[i].2;
            i += 1;
        }
        let idle = |q: i32, busy: i32| q > 0 && busy <= 0;
        let mut open = idle(q, busy).then_some(a);
        let mut out = vec![];
        while i < ev.len() && ev[i].0 < end {
            let t = ev[i].0;
            while i < ev.len() && ev[i].0 == t {
                q += ev[i].1;
                busy += ev[i].2;
                i += 1;
            }
            match (open, idle(q, busy)) {
                (None, true) => open = Some(t),
                (Some(s), false) => {
                    out.push((s, t));
                    open = None;
                }
                _ => {}
            }
        }
        if let Some(s) = open {
            out.push((s, end));
        }
        out.retain(|(s, e)| e - s >= IDLE_MIN);
        out
    }

    /// 一次领取的结果
    fn result(&self, c: &CardD, x: &Run) -> &'static str {
        let r = &self.boards[c.b].roles;
        if x.end.is_none() {
            return "running";
        }
        match x.how.as_deref() {
            Some("lost") => return "lost",
            Some("release") => return "released",
            _ => {}
        }
        // 这次领取之后，卡片第一次离开锁定状态
        let Some(exit) = c.moves.iter().find(|m| m.at >= x.start && m.kind != "claim" && !Roles::is(&r.lock, m.to.as_deref())) else {
            return "review";
        };
        if Roles::is(&r.human, exit.to.as_deref()) && exit.agent.as_deref() == Some(x.agent.as_str()) {
            return "gaveup";
        }
        if Roles::is(&r.review, exit.to.as_deref()) {
            let next = c.runs.iter().filter(|y| y.n > x.n).map(|y| y.start).min().unwrap_or(i64::MAX);
            let back = c.moves.iter().any(|m| m.at > exit.at && m.at < next && m.kind == "move" && Roles::is(&r.claim, m.to.as_deref()));
            return if back { "rework" } else { "review" };
        }
        if Roles::is(&r.claim, exit.to.as_deref()) {
            return "released";
        }
        "review"
    }
}

impl R<'_> {
    pub fn stats(&self, q: &StatsQuery, now: i64) -> Result<Stats> {
        let (from, to) = (q.from, q.to);
        if to <= from {
            return Err(BoardError::usage("时间范围不对：to 要大于 from"));
        }
        if to - from > SPAN_MAX {
            return Err(BoardError::usage(format!("时间范围最长 {} 天", SPAN_MAX / DAY)));
        }
        let tz = TimeZone::get(q.tz.trim()).map_err(|_| BoardError::usage(format!("不认识的时区「{}」，要用 IANA 时区，例如 Asia/Shanghai", q.tz)))?;
        let span = to - from;
        let (pfrom, pto) = (from - span, from);
        let d = self.stats_data()?;

        // 范围内的白板
        let scope: Vec<usize> = match q.scope {
            Scope::All => (0..d.boards.len()).collect(),
            Scope::Project => {
                let pk = q.path.as_deref().unwrap_or("").trim();
                if !d.projects.iter().any(|(k, _)| k == pk) {
                    return Err(BoardError::new(ErrorCode::BoardNotFound, format!("项目「{pk}」不存在")));
                }
                (0..d.boards.len()).filter(|i| d.boards[*i].project == pk).collect()
            }
            Scope::Board => {
                let b = self.resolve(q.path.as_deref().unwrap_or(""))?;
                vec![d.boards.iter().position(|x| x.id == b.id).ok_or_else(|| BoardError::code(ErrorCode::BoardNotFound))?]
            }
        };

        // 范围内的动态：评论数、热力图
        let today = zoned(now, &tz)?.start_of_day().map_err(time_err)?;
        let cal0 = today
            .checked_sub((today.weekday().to_monday_zero_offset() as i64).days())
            .and_then(|z| z.checked_sub(((CAL_WEEKS - 1) * 7).days()))
            .and_then(|z| z.start_of_day())
            .map_err(time_err)?;
        let since = pfrom.min(ms_of(&cal0));
        let mut acts: Vec<(i64, bool)> = vec![];
        {
            let mut st = self.conn.prepare_cached("SELECT at, action = 'comment' FROM activity WHERE board = ?1 AND at >= ?2")?;
            for b in &scope {
                let rows = st.query_map(params![d.boards[*b].id, since], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, bool>(1)?)))?;
                for row in rows {
                    acts.push(row?);
                }
            }
        }

        let hourly = span <= HOURLY_MAX;
        let starts = bucket_starts(from, to, hourly, &tz)?;
        let inr = |t: i64| t >= from && t < to;

        let period = |a: i64, b: i64| -> Period {
            let cards: Vec<&CardD> = d.cards_of(&scope).collect();
            let o = Outcome::of(cards.iter().copied(), a, b);
            let mut p = Period { done: o.done, rework: o.rework, ..Period::default() };
            for c in &cards {
                if c.created >= a && c.created < b {
                    p.created += 1;
                }
                for x in c.runs.iter().filter(|x| x.start >= a && x.start < b) {
                    p.claims += 1;
                    p.beats += x.beats;
                    p.spent += x.end.map(|e| e - x.start).unwrap_or(0);
                }
            }
            p.comments = acts.iter().filter(|(t, c)| *c && *t >= a && *t < b).count() as i64;
            p.lead_p50 = quantile(&o.leads, 0.5);
            p.lead_p85 = quantile(&o.leads, 0.85);
            p.wait_p50 = quantile(&o.waits, 0.5);
            p.work_p50 = quantile(&o.works, 0.5);
            p.lead_hist = vec![0; LEAD_BINS.len() + 1];
            for l in &o.leads {
                p.lead_hist[LEAD_BINS.iter().position(|m| l < m).unwrap_or(LEAD_BINS.len())] += 1;
            }
            p.first_pass = o.first_pass();
            let end = b.min(now);
            p.idle = scope.iter().flat_map(|bi| d.idle_spans(*bi, a, end, now)).map(|(s, e)| e - s).sum();
            p
        };
        let cur = period(from, to);
        let prev = period(pfrom, pto);

        // 按桶的时间序列
        let mut series: Vec<Bucket> = starts.iter().map(|t| Bucket { t: *t, ..Bucket::default() }).collect();
        for c in d.cards_of(&scope) {
            if inr(c.created) {
                if let Some(i) = bucket_of(&starts, c.created) {
                    series[i].created += 1;
                }
            }
            for x in c.runs.iter().filter(|x| inr(x.start)) {
                if let Some(i) = bucket_of(&starts, x.start) {
                    series[i].claims += 1;
                    series[i].beats += x.beats;
                    series[i].spent += x.end.map(|e| e - x.start).unwrap_or(0);
                }
            }
            let mut seen = HashSet::new();
            for t in c.done_at.iter().filter(|t| inr(**t)) {
                if let Some(i) = bucket_of(&starts, *t) {
                    if seen.insert(i) {
                        series[i].done += 1;
                    }
                }
            }
            for t in c.rework_at.iter().filter(|t| inr(**t)) {
                if let Some(i) = bucket_of(&starts, *t) {
                    series[i].rework += 1;
                }
            }
        }
        for (t, comment) in &acts {
            if *comment && inr(*t) {
                if let Some(i) = bucket_of(&starts, *t) {
                    series[i].comments += 1;
                }
            }
        }

        // 热力图
        let mut hour_heat = vec![vec![0i64; 24]; 7];
        let mut activity = 0;
        let mut days: Vec<i64> = vec![];
        let mut z = cal0.clone();
        for _ in 0..CAL_WEEKS * 7 {
            days.push(ms_of(&z));
            z = z.tomorrow().and_then(|x| x.start_of_day()).map_err(time_err)?;
        }
        let cal_end = ms_of(&z);
        let mut day_v = vec![0i64; days.len()];
        for (t, _) in &acts {
            if inr(*t) {
                activity += 1;
                let at = zoned(*t, &tz)?;
                hour_heat[at.weekday().to_monday_zero_offset() as usize][at.hour() as usize] += 1;
            }
            if *t >= days[0] && *t < cal_end {
                if let Some(i) = bucket_of(&days, *t) {
                    day_v[i] += 1;
                }
            }
        }
        let today_ms = ms_of(&today);
        let day_heat = days.iter().zip(day_v).map(|(t, v)| HeatDay { t: *t, v, future: *t > today_ms }).collect();

        // 空转时段
        let mut idle = vec![];
        for b in &scope {
            for (s, e) in d.idle_spans(*b, from, to.min(now), now) {
                idle.push(IdleSpan { board: d.boards[*b].path.clone(), from: s, to: e });
            }
        }
        idle.sort_by_key(|x| x.from);

        // 每次领取、按 agent
        let mut runs: Vec<RunRow> = vec![];
        let mut agents: Vec<AgentRow> = vec![];
        let mut fp: HashMap<String, (i64, i64)> = HashMap::new();
        for c in d.cards_of(&scope) {
            for x in &c.runs {
                let result = d.result(c, x);
                let label = d.label(&x.agent);
                if x.start < to && x.end.unwrap_or(now) > from {
                    runs.push(RunRow {
                        agent: label.clone(),
                        name: x.agent.clone(),
                        board: d.boards[c.b].path.clone(),
                        card: c.id,
                        title: c.title.clone(),
                        start: x.start,
                        end: x.end,
                        result,
                    });
                }
                if inr(x.start) {
                    let row = match agents.iter().position(|a| a.agent == label) {
                        Some(i) => &mut agents[i],
                        None => {
                            agents.push(AgentRow { agent: label.clone(), claims: 0, spent: 0, first_pass: None, lost: 0, gaveup: 0 });
                            agents.last_mut().unwrap()
                        }
                    };
                    row.claims += 1;
                    row.spent += x.end.map(|e| e - x.start).unwrap_or(0);
                    row.lost += (result == "lost") as i64;
                    row.gaveup += (result == "gaveup") as i64;
                }
            }
            // 完成的卡片记在它最后一次领取的 agent 名下
            if let Some(t) = c.done_in(from, to) {
                if let Some(last) = c.runs.iter().filter(|x| x.start <= t).max_by_key(|x| x.start) {
                    let e = fp.entry(d.label(&last.agent)).or_default();
                    e.1 += 1;
                    e.0 += (!c.rework_at.iter().any(|r| *r <= t)) as i64;
                }
            }
        }
        for a in agents.iter_mut() {
            a.first_pass = fp.get(&a.agent).and_then(|(p, n)| ratio(*p, *n));
        }
        agents.sort_by(|a, b| b.claims.cmp(&a.claims).then_with(|| a.agent.cmp(&b.agent)));
        runs.sort_by_key(|x| (x.start, x.card));

        // 状态停留：离开时间落在区间内的停留
        let mut dwell_v: HashMap<String, Vec<i64>> = HashMap::new();
        for c in d.cards_of(&scope) {
            let r = &d.boards[c.b].roles;
            for s in &c.segs {
                let (Some(k), Some(e)) = (s.key.as_deref(), s.to) else { continue };
                if inr(e) && !r.fin.contains(k) {
                    dwell_v.entry(k.to_string()).or_default().push(e - s.from);
                }
            }
        }
        let mut dwell = vec![];
        for b in &scope {
            let keys = std::iter::once(INBOX.to_string()).chain(d.boards[*b].flow.0.iter().map(|s| s.key.clone()));
            for k in keys {
                let Some(v) = dwell_v.remove(&k) else { continue };
                let n = v.len() as i64;
                dwell.push(Dwell {
                    name: d.boards[*b].flow.name(&k).to_string(),
                    color: d.color(*b, &k).unwrap_or_default(),
                    median: quantile(&v, 0.5).unwrap_or(0),
                    mean: v.iter().sum::<i64>() / n.max(1),
                    n,
                    key: k,
                });
            }
        }

        // 卡片流转明细
        let mut done_cards: Vec<(&CardD, i64)> = d.cards_of(&scope).filter_map(|c| c.done_in(from, to).map(|t| (c, t))).collect();
        done_cards.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.id.cmp(&a.0.id)));
        let lives = done_cards
            .iter()
            .take(LIVES_MAX)
            .map(|(c, t)| Life {
                board: d.boards[c.b].path.clone(),
                card: c.id,
                title: c.title.clone(),
                created: c.created,
                done: *t,
                reworks: c.rework_at.iter().filter(|r| **r <= *t).count() as i64,
                segs: c
                    .segs
                    .iter()
                    .filter(|s| s.from < *t)
                    .map(|s| Seg {
                        key: s.key.clone(),
                        name: s.name.clone(),
                        color: s.key.as_deref().and_then(|k| d.color(c.b, k)),
                        from: s.from,
                        to: s.to.unwrap_or(*t).min(*t),
                    })
                    .filter(|s| s.to > s.from)
                    .collect(),
            })
            .collect();

        let present = self.present(&d, &scope, now)?;
        let life = match q.scope {
            Scope::All => None,
            _ => {
                let since = match q.scope {
                    Scope::Board => d.boards[scope[0]].created_at,
                    _ => d.projects.iter().find(|(k, _)| Some(k.as_str()) == q.path.as_deref().map(str::trim)).map(|p| p.1).unwrap_or(now),
                };
                let cards: Vec<&CardD> = d.cards_of(&scope).collect();
                let finals = cards.iter().filter(|c| d.boards[c.b].roles.fin.contains(&c.col)).count() as i64;
                let names: HashSet<String> = cards.iter().flat_map(|c| c.runs.iter().map(|x| d.label(&x.agent))).collect();
                let days = ((now - since) as f64 / DAY as f64).round().max(1.0) as i64;
                Some(Lifetime {
                    since,
                    days,
                    cards: cards.len() as i64,
                    finals,
                    reworks: cards.iter().map(|c| c.rework_at.len() as i64).sum(),
                    agents: names.len() as i64,
                    spent: cards.iter().flat_map(|c| c.runs.iter()).filter_map(|x| x.end.map(|e| e - x.start)).sum(),
                    per_day: (finals as f64 / days as f64 * 10.0).round() / 10.0,
                })
            }
        };

        // 侧栏、对比表
        let brief = |key: String, bs: &[usize]| -> Brief {
            let o = Outcome::of(d.cards_of(bs), from, to);
            let mut series = vec![0; starts.len()];
            let (mut review, mut todo, mut busy) = (0, 0, false);
            for c in d.cards_of(bs) {
                let r = &d.boards[c.b].roles;
                review += r.review.contains(&c.col) as i64;
                todo += r.claim.contains(&c.col) as i64;
                busy |= c.holder.is_some();
                let mut seen = HashSet::new();
                for t in c.done_at.iter().filter(|t| inr(**t)) {
                    if let Some(i) = bucket_of(&starts, *t) {
                        if seen.insert(i) {
                            series[i] += 1;
                        }
                    }
                }
            }
            Brief { key, review, todo, busy, done: o.done, series, lead_p50: quantile(&o.leads, 0.5), first_pass: o.first_pass(), rework: o.rework }
        };
        let boards = (0..d.boards.len()).map(|i| brief(d.boards[i].path.clone(), &[i])).collect();
        let projects = d
            .projects
            .iter()
            .map(|(pk, _)| {
                let bs: Vec<usize> = (0..d.boards.len()).filter(|i| d.boards[*i].project == *pk).collect();
                brief(pk.clone(), &bs)
            })
            .collect();
        let all_boards: Vec<usize> = (0..d.boards.len()).collect();
        let all = brief(String::new(), &all_boards);

        Ok(Stats {
            scope: q.scope.as_str(),
            path: q.path.clone().filter(|_| q.scope != Scope::All),
            from,
            to,
            now,
            tz: q.tz.trim().to_string(),
            bucket: if hourly { "hour" } else { "day" },
            boards_in_scope: scope.iter().map(|b| d.boards[*b].path.clone()).collect(),
            activity,
            cur,
            prev,
            series,
            idle,
            agents,
            dwell,
            hour_heat,
            day_heat,
            runs,
            lives,
            present,
            life,
            boards,
            projects,
            all,
        })
    }

    fn present(&self, d: &Data, scope: &[usize], now: i64) -> Result<Present> {
        // 各状态张数：按第一块白板的工作流顺序，同一个 key 合在一起（「未整理」在最前）
        let mut statuses: Vec<StatusCount> = vec![];
        for b in scope {
            let keys = std::iter::once(INBOX.to_string()).chain(d.boards[*b].flow.0.iter().map(|s| s.key.clone()));
            for k in keys {
                if !statuses.iter().any(|s| s.key == k) {
                    statuses.push(StatusCount {
                        name: if k == INBOX { INBOX_NAME.to_string() } else { d.boards[*b].flow.name(&k).to_string() },
                        color: d.color(*b, &k).unwrap_or_default(),
                        n: 0,
                        key: k,
                    });
                }
            }
        }
        let (mut review, mut todo) = (Waiting { n: 0, oldest: None }, Waiting { n: 0, oldest: None });
        let mut doing = Doing { n: 0, holders: vec![] };
        let mut open: Vec<&CardD> = vec![];
        for c in d.cards_of(scope) {
            let r = &d.boards[c.b].roles;
            if let Some(s) = statuses.iter_mut().find(|s| s.key == c.col) {
                s.n += 1;
            }
            let wait = now - c.entered();
            let bump = |w: &mut Waiting| {
                w.n += 1;
                w.oldest = Some(w.oldest.map_or(wait, |o| o.max(wait)));
            };
            if r.review.contains(&c.col) {
                bump(&mut review);
            }
            if r.claim.contains(&c.col) {
                bump(&mut todo);
            }
            if let Some(h) = &c.holder {
                doing.n += 1;
                if !doing.holders.contains(h) {
                    doing.holders.push(h.clone());
                }
            }
            if c.col != INBOX && !r.fin.contains(&c.col) {
                open.push(c);
            }
        }
        open.sort_by_key(|c| (c.entered(), c.id));
        let mut stuck = vec![];
        let mut last = self.conn.prepare_cached(
            "SELECT text FROM activity WHERE board = ?1 AND card = ?2 AND action = 'comment' ORDER BY id DESC LIMIT 1",
        )?;
        for c in open.into_iter().take(STUCK_MAX) {
            let b = &d.boards[c.b];
            let note = match &c.holder {
                Some(_) => None,
                None => last
                    .query_row(params![b.id, c.id], |r| r.get::<_, String>(0))
                    .optional()?
                    .and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| l.chars().take(NOTE_MAX).collect())),
            };
            stuck.push(Stuck {
                board: b.path.clone(),
                card: c.id,
                title: c.title.clone(),
                key: c.col.clone(),
                name: b.flow.name(&c.col).to_string(),
                color: d.color(c.b, &c.col).unwrap_or_default(),
                since: c.entered(),
                holder: c.holder.clone(),
                note,
            });
        }
        Ok(Present { statuses, review, todo, doing, stuck })
    }

    /// 读出全部白板的卡片、流转、领取（统计都在内存里算）
    fn stats_data(&self) -> Result<Data> {
        let conn = self.conn;
        let projects: Vec<(String, i64)> = conn
            .prepare("SELECT key, created_at FROM projects ORDER BY sort, created_at, key")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut boards: Vec<BoardD> = vec![];
        {
            let mut st = conn.prepare("SELECT id, key, created_at FROM boards WHERE project = ?1 ORDER BY sort, created_at, key")?;
            for (pk, _) in &projects {
                let rows: Vec<(i64, String, i64)> =
                    st.query_map([pk], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
                for (id, key, created_at) in rows {
                    let flow = flow_of(conn, id)?;
                    let roles = Roles::of(&flow);
                    boards.push(BoardD { id, path: format!("{pk}/{key}"), project: pk.clone(), created_at, flow, roles });
                }
            }
        }
        let index: HashMap<i64, usize> = boards.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
        let mut cards: Vec<CardD> = vec![];
        let mut at: HashMap<(i64, i64), usize> = HashMap::new();
        {
            let mut st = conn.prepare("SELECT board, id, title, col, holder, created_at FROM cards ORDER BY board, id")?;
            let rows = st.query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, i64>(5)?))
            })?;
            for row in rows {
                let (board, id, title, col, holder, created) = row?;
                let Some(b) = index.get(&board) else { continue };
                at.insert((board, id), cards.len());
                cards.push(CardD {
                    b: *b,
                    id,
                    title,
                    col,
                    holder,
                    created,
                    moves: vec![],
                    runs: vec![],
                    done_at: vec![],
                    rework_at: vec![],
                    first_claim: None,
                    segs: vec![],
                });
            }
        }
        {
            let mut st = conn.prepare(
                "SELECT board, card, at, kind, from_key, from_name, to_key, to_name, actor_kind, actor_name FROM moves ORDER BY board, card, at, id",
            )?;
            let rows = st.query_map([], |r| {
                let agent = (r.get::<_, String>(8)? == "agent").then(|| r.get::<_, String>(9)).transpose()?;
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    Mv { at: r.get(2)?, kind: r.get(3)?, from: r.get(4)?, from_name: r.get(5)?, to: r.get(6)?, to_name: r.get(7)?, agent },
                ))
            })?;
            for row in rows {
                let (b, c, m) = row?;
                if let Some(i) = at.get(&(b, c)) {
                    cards[*i].moves.push(m);
                }
            }
        }
        {
            let mut st = conn.prepare("SELECT board, card, n, agent, started_at, ended_at, how, beats FROM card_runs ORDER BY board, card, n")?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    Run { n: r.get(2)?, agent: r.get(3)?, start: r.get(4)?, end: r.get(5)?, how: r.get(6)?, beats: r.get(7)? },
                ))
            })?;
            for row in rows {
                let (b, c, x) = row?;
                if let Some(i) = at.get(&(b, c)) {
                    cards[*i].runs.push(x);
                }
            }
        }
        let mut by_board = vec![vec![]; boards.len()];
        for (i, c) in cards.iter_mut().enumerate() {
            c.derive(&boards[c.b]);
            by_board[c.b].push(i);
        }
        let mut merge = HashMap::new();
        {
            let mut st = conn.prepare("SELECT name, agent, host FROM dispatches WHERE name IS NOT NULL ORDER BY id")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            for row in rows {
                let (name, agent, host) = row?;
                merge.insert(name, format!("{agent}/{host}"));
            }
        }
        Ok(Data { boards, cards, by_board, merge, projects })
    }
}
