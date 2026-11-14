//! 定时建卡（PRD §4「白板设置」、§8「卡片在状态里的位置」）：每块白板可以设几条「到点按模板建一张卡」，
//! 例如每周「清理死代码」、每天「找找可以改进的地方」，建出来的卡由定时领取或人派发去做。
//!
//! - 每条：名称（白板内唯一，同时是建卡标题）、描述、标签、放到哪（首个可领取状态 / 未整理）、派给哪台机器（可选：
//!   指定一台，或每次在能接这块白板的机器里随机选一台；和用哪个 agent）、cron 表达式（和定时领取同一套，见 cron.rs）、
//!   时区（IANA 名称，保存时取浏览器的）、开关。每块白板最多 [`CARD_CRON_MAX`] 条。
//! - **服务端自己计时**：服务端每分钟调用一次 [`W::card_crons_due`]，按每条的时区算出这一分钟的本地时间，对上 cron 就建卡；
//!   不依赖任何机器在线。服务端停机期间错过的不补建（只看重新启动之后的分钟）。
//! - 建出来的卡：标题后面加日期「（MM-DD）」（按这条的时区），建卡人是系统身份「定时建卡」，动态第一条写
//!   「定时建卡「<名称>」建了这张卡」，放到「放到哪」指定的位置最下面。模板描述里引用的附件每次复制一份归到新卡，
//!   模板自己的那份留着（附件清理时按模板描述的引用保留）。
//! - **跳过**：这条上一次建的卡还没进终点状态、也没被删除时，这一次不建，结果记「跳过：上一张 #N 还没完成」。
//! - **派给机器**：选了一台（[`PICK_HOST`]）时，建出卡后马上派发到那台机器（和手动派发一样排队、占用卡片，来源记「定时建卡」，
//!   机器离线、暂停接单时排队等它）；选了随机（[`PICK_RANDOM`]）时，到点在能接这块白板的机器里随机挑一台（候选见 [`W::cron_host_candidates`]）。
//!   派不出去（机器被移除、没有这块白板的目录、环境检查没通过、agent 没找到……）时卡片留在首个可领取状态，
//!   评论写明原因，这一次的结果也记下原因。只能和「放到首个可领取状态」一起用。机器改名时跟着改（hosts.rs `host_rename`）。
//! - 「现在建一张」（[`W::card_cron_run`]）：立即按这条建一张，同样受跳过规则约束。
//! - 删除白板时它的定时建卡一起删除（外键级联）。

use crate::cron::Cron;
use crate::engine::Cmd;
use crate::error::{BoardError, ErrorCode, Result};
use crate::hosts::{workspace_of, Readiness, SOURCE_SCHEDULE};
use crate::model::*;
use crate::store::{flow_of, R, W};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA: &str = r#"
-- 定时建卡：到点按模板在白板上建一张卡（PRD §4）。删除白板时一起删除
CREATE TABLE IF NOT EXISTS card_crons (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  board      INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name       TEXT NOT NULL,
  title      TEXT NOT NULL,              -- 旧版的「卡片标题」：现在标题就是名称，这一列只为旧库留着（不再写、不读）
  body       TEXT NOT NULL DEFAULT '',
  labels     TEXT NOT NULL DEFAULT '[]', -- 标签 ID（JSON 数组）；建卡时只带项目里还在的
  place      TEXT NOT NULL DEFAULT 'home', -- home 首个可领取状态 | inbox 未整理
  cron       TEXT NOT NULL,
  tz         TEXT NOT NULL,             -- IANA 时区名，保存时取浏览器的
  enabled    INTEGER NOT NULL DEFAULT 1,
  last_card  INTEGER,                   -- 上一次建的卡（跳过规则看它）
  ran_at     INTEGER,                   -- 最近一次到点（或「现在建一张」）的时间
  ran_card   INTEGER,                   -- 那一次建的卡；跳过时为空
  ran_skip   INTEGER,                   -- 那一次跳过：因为上一张还没完成
  host       TEXT,                      -- 建出卡后派给哪台机器（机器名）；空表示不派发
  agent      TEXT,                      -- 用哪个 agent；空表示那台机器的默认 agent
  pick       TEXT,                      -- 选哪台机器：host 这台 | random 能接这块白板的机器里随机一台；空同 host（旧库）
  ran_to     TEXT,                      -- 那一次派发到了哪：「<机器> · <agent>」
  ran_err    TEXT,                      -- 那一次没派发出去的原因
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS card_crons_board ON card_crons (board, id);
"#;

/// v0.1.22 建的表没有派发的几列
pub(crate) fn migrate(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    for col in ["host", "agent", "ran_to", "ran_err", "pick"] {
        let has: bool = conn.query_row("SELECT COUNT(*) > 0 FROM pragma_table_info('card_crons') WHERE name = ?1", [col], |r| r.get(0))?;
        if !has {
            conn.execute_batch(&format!("ALTER TABLE card_crons ADD COLUMN {col} TEXT"))?;
        }
    }
    Ok(())
}

/// 每块白板最多几条
pub const CARD_CRON_MAX: i64 = 20;
/// 名称最长几个字
pub const CARD_CRON_NAME_MAX: usize = 24;
/// 建卡人：系统身份「定时建卡」（界面、未整理页按来源分组时单独一组）
pub const CARD_CRON_ACTOR: &str = "定时建卡";
/// 派给指定的一台机器（[`CardCron::host`]）
pub const PICK_HOST: &str = "host";
/// 每次在能接这块白板的机器里随机选一台，然后按注册顺序挑一个找到了命令的默认 agent
pub const PICK_RANDOM: &str = "random";

/// 一条定时建卡
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardCron {
    pub id: i64,
    /// 白板内唯一，最长 [`CARD_CRON_NAME_MAX`] 个字；建卡标题也是它（后面加日期）
    pub name: String,
    /// 卡片标题：和 [`Self::name`] 相同，只为旧版客户端留着（旧版按它显示标题）
    pub title: String,
    /// 卡片描述（Markdown）
    pub body: String,
    /// 标签 ID
    pub labels: Vec<i64>,
    /// `home` 首个可领取状态 | `inbox` 未整理
    pub place: String,
    pub cron: String,
    /// IANA 时区名，例如 `Asia/Shanghai`
    pub tz: String,
    pub on: bool,
    /// 建出卡后派给哪台机器（机器名）；不派发时为空
    #[serde(default)]
    pub host: Option<String>,
    /// 用哪个 agent；空表示那台机器（随机选出来那台）的默认 agent
    #[serde(default)]
    pub agent: Option<String>,
    /// 选哪台机器：[`PICK_HOST`] 派给 [`Self::host`]，[`PICK_RANDOM`] 每次随机挑一台
    #[serde(default = "pick_host")]
    pub pick: String,
    /// 下次到点的时间（毫秒）；关着、或一年内都不会到点（例如 2 月 30 号）时为空
    #[serde(default)]
    pub next_at: Option<i64>,
    /// 最近一次到点（或「现在建一张」）的时间；还没到过点时为空
    #[serde(default)]
    pub ran_at: Option<i64>,
    /// 那一次建的卡
    #[serde(default)]
    pub ran_card: Option<i64>,
    /// 那一次跳过了：上一张（这个编号）还没完成
    #[serde(default)]
    pub ran_skip: Option<i64>,
    /// 那一次建的卡派发到了哪：「<机器> · <agent>」
    #[serde(default)]
    pub ran_to: Option<String>,
    /// 那一次建的卡没派发出去的原因
    #[serde(default)]
    pub ran_err: Option<String>,
    /// 上一次建的卡（跳过规则看它）
    #[serde(default)]
    pub last_card: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 新建、修改的内容（修改时没给的项不变）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardCronPatch {
    #[serde(default)]
    pub name: Option<String>,
    /// 旧版的「卡片标题」：现在标题就是名称，传了也不用、不检查
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<i64>>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub on: Option<bool>,
    /// 派给哪台机器；空字符串表示不派发
    #[serde(default)]
    pub host: Option<String>,
    /// 用哪个 agent；空字符串表示那台机器的默认 agent
    #[serde(default)]
    pub agent: Option<String>,
    /// 选哪台机器：`host`（默认）派给 `host`，`random` 每次随机挑一台；空字符串和新建立时一样处理
    #[serde(default)]
    pub pick: Option<String>,
}

const COLS: &str = "id, name, title, body, labels, place, cron, tz, enabled, last_card, ran_at, ran_card, ran_skip, created_at, updated_at, board, host, agent, ran_to, ran_err, pick";

fn pick_host() -> String {
    PICK_HOST.into()
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(CardCron, i64)> {
    let labels: String = r.get(4)?;
    Ok((
        CardCron {
            id: r.get(0)?,
            name: r.get(1)?,
            title: r.get(1)?,
            body: r.get(3)?,
            labels: serde_json::from_str(&labels).unwrap_or_default(),
            place: r.get(5)?,
            cron: r.get(6)?,
            tz: r.get(7)?,
            on: r.get(8)?,
            host: r.get(16)?,
            agent: r.get(17)?,
            // 旧库没有这一列（迁移补上时是空的）：按「派给指定的机器」算
            pick: r.get::<_, Option<String>>(20)?.unwrap_or_else(pick_host),
            next_at: None,
            last_card: r.get(9)?,
            ran_at: r.get(10)?,
            ran_card: r.get(11)?,
            ran_skip: r.get(12)?,
            ran_to: r.get(18)?,
            ran_err: r.get(19)?,
            created_at: r.get(13)?,
            updated_at: r.get(14)?,
        },
        r.get(15)?,
    ))
}

/// 去掉首尾空白，空的算没有
fn opt(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// 选机器的写法：`host`、`random`；空（旧客户端、新建时没给）是 `host`
fn pick_of(v: Option<String>) -> Result<String> {
    match opt(v).as_deref() {
        None | Some(PICK_HOST) => Ok(PICK_HOST.into()),
        Some(PICK_RANDOM) => Ok(PICK_RANDOM.into()),
        Some(other) => Err(BoardError::usage(format!("「派给机器」只有 host（指定一台）和 random（随机挑一台），不认识「{other}」"))),
    }
}

fn tz_of(name: &str) -> Result<TimeZone> {
    TimeZone::get(name).map_err(|_| BoardError::usage(format!("不认识的时区「{name}」：要写 IANA 时区名，例如 Asia/Shanghai")))
}

/// 这一刻在这个时区里是几月几号几点几分
fn local(at: i64, tz: &TimeZone) -> Option<DateTime> {
    Some(Timestamp::from_millisecond(at).ok()?.to_zoned(tz.clone()).datetime())
}

/// 这一分钟（`at` 落在哪一分钟）按这条的时区对得上 cron 吗
pub fn due_at(cron: &Cron, tz: &TimeZone, at: i64) -> bool {
    let Some(t) = local(at, tz) else { return false };
    cron.matches(t.minute() as u32, t.hour() as u32, t.day() as u32, t.month() as u32, t.weekday().to_sunday_zero_offset() as u32)
}

/// `after` 之后（不含 `after` 所在的这一分钟）第一次到点的时间（毫秒）；一年多都不会到点时为空。
/// 按本地时间往后找：这一天对不上就跳到第二天 0 点，这一小时对不上就跳到下一小时
pub fn next_at(cron: &Cron, tz: &TimeZone, after: i64) -> Option<i64> {
    let start = local(after, tz)?;
    let mut t = start.with().second(0).subsec_nanosecond(0).build().ok()?.checked_add(1.minute()).ok()?;
    let limit = start.checked_add(400.days()).ok()?;
    while t <= limit {
        if !cron.matches_day(t.day() as u32, t.month() as u32, t.weekday().to_sunday_zero_offset() as u32) {
            t = t.date().checked_add(1.day()).ok()?.to_datetime(jiff::civil::Time::midnight());
            continue;
        }
        if !cron.matches_hour(t.hour() as u32) {
            t = t.with().minute(0).build().ok()?.checked_add(1.hour()).ok()?;
            continue;
        }
        if !cron.matches_min(t.minute() as u32) {
            t = t.checked_add(1.minute()).ok()?;
            continue;
        }
        // 夏令时跳过的那一段本地时间不存在：按 jiff 的默认规则落到之后；结果早于 after 时（重复的那一段）接着找
        let ms = tz.to_zoned(t).ok()?.timestamp().as_millisecond();
        if ms > after {
            return Some(ms);
        }
        t = t.checked_add(1.minute()).ok()?;
    }
    None
}

/// 「清理死代码（11-12）」：日期按这条的时区
fn dated(title: &str, tz: &TimeZone, at: i64) -> String {
    match local(at, tz) {
        Some(t) => format!("{title}（{:02}-{:02}）", t.month(), t.day()),
        None => title.to_string(),
    }
}

impl<'a> R<'a> {
    /// 一块白板的定时建卡（按创建先后），带上下次到点的时间
    pub fn card_crons(&self, board: i64, now: i64) -> Result<Vec<CardCron>> {
        let mut st = self.conn.prepare(&format!("SELECT {COLS} FROM card_crons WHERE board = ?1 ORDER BY id"))?;
        let rows = st.query_map([board], row)?;
        let mut out = vec![];
        for r in rows {
            let (mut c, _) = r?;
            if c.on {
                if let (Ok(cron), Ok(tz)) = (Cron::parse(&c.cron), TimeZone::get(&c.tz)) {
                    c.next_at = next_at(&cron, &tz, now);
                }
            }
            out.push(c);
        }
        Ok(out)
    }

    /// 模板描述里引用的附件 ID（附件清理时留着它们）
    pub(crate) fn card_cron_files(&self) -> Result<Vec<(i64, String)>> {
        let mut st = self.conn.prepare("SELECT board, body FROM card_crons WHERE body LIKE '%/api/files/%'")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

impl<'a> W<'a> {
    fn card_cron_row(&self, board: i64, id: i64) -> Result<CardCron> {
        self.tx
            .query_row(&format!("SELECT {COLS} FROM card_crons WHERE id = ?1 AND board = ?2"), params![id, board], row)
            .optional()?
            .map(|(c, _)| c)
            .ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("定时建卡 #{id} 不存在")))
    }



    /// 校验一条完整的定时建卡（新建、修改后），返回整理过的值。`target` 时看要派的机器和 agent 合不合适
    /// （新建、改了机器或 agent 时；只开关、改别的时不看，免得机器后来被移除了连开关都改不了）
    fn check_card_cron(&self, board: &BoardRef, id: Option<i64>, mut c: CardCron, target: bool) -> Result<CardCron> {
        c.name = c.name.trim().to_string();
        // 标题就是名称：旧库里和名称不一样的标题以名称修改时给的为准，这里统一成名称
        c.title = c.name.clone();
        c.body = c.body.trim().to_string();
        c.tz = c.tz.trim().to_string();
        if c.name.is_empty() {
            return Err(BoardError::usage("定时建卡的名称不能为空"));
        }
        if c.name.chars().count() > CARD_CRON_NAME_MAX {
            return Err(BoardError::usage(format!("名称最长 {CARD_CRON_NAME_MAX} 个字")));
        }
        let dup: bool = self.tx.query_row(
            "SELECT COUNT(*) > 0 FROM card_crons WHERE board = ?1 AND name = ?2 AND id != ?3",
            params![board.id, c.name, id.unwrap_or(0)],
            |r| r.get(0),
        )?;
        if dup {
            return Err(BoardError::usage(format!("这块白板已经有叫「{}」的定时建卡", c.name)));
        }
        if c.place != "home" && c.place != INBOX {
            return Err(BoardError::usage(format!("「放到哪」只能是 home（首个可领取状态）或 {INBOX}（未整理），不认识「{}」", c.place)));
        }
        c.cron = crate::cron::normalize(&c.cron).map_err(BoardError::usage)?;
        tz_of(&c.tz)?;
        c.pick = pick_of(Some(c.pick))?;
        // 随机挑的没有指定的机器
        if c.pick == PICK_RANDOM {
            c.host = None;
        } else {
            c.host = opt(c.host.take());
        }
        c.agent = if c.pick == PICK_RANDOM || c.host.is_some() { opt(c.agent.take()) } else { None };
        if c.pick == PICK_RANDOM {
            if c.place == INBOX {
                return Err(BoardError::usage(format!("放到「{INBOX_NAME}」的不派发：要在能接这块白板的机器里随机派发，「放到哪」选首个可领取状态")));
            }
            if target {
                self.cron_host_pick_check(board, c.agent.as_deref())?;
            }
        }
        if let Some(h) = &c.host {
            if c.place == INBOX {
                return Err(BoardError::usage(format!("放到「{INBOX_NAME}」的不派发：要派给机器，「放到哪」选首个可领取状态")));
            }
            if target {
                self.dispatch_target_check(board, h, c.agent.as_deref())?;
            }
        }
        let mut labels: Vec<i64> = vec![];
        for l in &c.labels {
            if !labels.contains(l) {
                labels.push(*l);
            }
        }
        self.check_labels(board, &labels)?;
        c.labels = labels;
        Ok(c)
    }

    /// 新建一条定时建卡（只有人能做，接口层把关）
    pub fn card_cron_create(&mut self, path: &str, p: &CardCronPatch) -> Result<CardCron> {
        let board = self.resolve(path)?;
        let n: i64 = self.tx.query_row("SELECT COUNT(*) FROM card_crons WHERE board = ?1", [board.id], |r| r.get(0))?;
        if n >= CARD_CRON_MAX {
            return Err(BoardError::usage(format!("每块白板最多 {CARD_CRON_MAX} 条定时建卡")));
        }
        let c = CardCron {
            id: 0,
            name: p.name.clone().unwrap_or_default(),
            // 标题就是名称：先用空的，后面 check_card_cron 统一成名称；旧客户端传的 title 不用
            title: String::new(),
            body: p.body.clone().unwrap_or_default(),
            labels: p.labels.clone().unwrap_or_default(),
            place: p.place.clone().unwrap_or_else(|| "home".into()),
            cron: p.cron.clone().unwrap_or_default(),
            tz: p.tz.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "UTC".into()),
            on: p.on.unwrap_or(true),
            host: p.host.clone(),
            agent: p.agent.clone(),
            pick: pick_of(p.pick.clone())?,
            next_at: None,
            ran_at: None,
            ran_card: None,
            ran_skip: None,
            ran_to: None,
            ran_err: None,
            last_card: None,
            created_at: self.now,
            updated_at: self.now,
        };
        let c = self.check_card_cron(&board, None, c, true)?;
        self.tx.execute(
            "INSERT INTO card_crons (board, name, title, body, labels, place, cron, tz, enabled, host, agent, pick, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
            params![
                board.id,
                c.name,
                c.title,
                c.body,
                serde_json::to_string(&c.labels).unwrap_or_else(|_| "[]".into()),
                c.place,
                c.cron,
                c.tz,
                c.on,
                c.host,
                c.agent,
                c.pick,
                self.now
            ],
        )?;
        let id = self.tx.last_insert_rowid();
        self.changed = true;
        self.card_cron_view(&board, id)
    }

    /// 修改一条：没给的项不变
    pub fn card_cron_update(&mut self, path: &str, id: i64, p: &CardCronPatch) -> Result<CardCron> {
        let board = self.resolve(path)?;
        let old = self.card_cron_row(board.id, id)?;
        let pick = p.pick.clone().map(|v| if v.trim().is_empty() { PICK_HOST.to_string() } else { v }).unwrap_or(old.pick.clone());
        let c = CardCron {
            name: p.name.clone().unwrap_or(old.name.clone()),
            // 标题跟着名称走：先用空的，后面 check_card_cron 统一成名称（旧库里和名称不一样的标题，打开改一次保存后就统一成名称）
            title: String::new(),
            body: p.body.clone().unwrap_or(old.body.clone()),
            labels: p.labels.clone().unwrap_or(old.labels.clone()),
            place: p.place.clone().unwrap_or(old.place.clone()),
            cron: p.cron.clone().unwrap_or(old.cron.clone()),
            tz: p.tz.clone().filter(|t| !t.trim().is_empty()).unwrap_or(old.tz.clone()),
            on: p.on.unwrap_or(old.on),
            host: p.host.clone().or(old.host.clone()),
            // 换了机器、没说用哪个 agent 时用新机器的默认 agent；换成随机时也一样
            agent: p.agent
                .clone()
                .or_else(|| if p.host.is_some() || (p.pick.is_some() && p.pick.as_deref().map(str::trim) != Some(old.pick.as_str())) { None } else { old.agent.clone() }),
            pick,
            ..old
        };
        // 换了机器、换了 agent 时重新检查（随机的和指定的一起看：有没有能接的机器、agent 找到了没有）。
        // 只换「随机 ↔ 指定」、别的不动时不看，机器后来被移除了也照样能改，和只改开关、模板时一样
        let how_changed = c.pick == PICK_RANDOM && p.pick.is_some();
        let target = p.agent.is_some() || how_changed || ((p.host.is_some()) && opt(c.host.clone()) != old.host);
        let c = self.check_card_cron(&board, Some(id), c, target)?;
        self.tx.execute(
            "UPDATE card_crons SET name = ?3, title = ?4, body = ?5, labels = ?6, place = ?7, cron = ?8, tz = ?9, enabled = ?10, host = ?11, agent = ?12,
                    pick = ?13, updated_at = ?14
             WHERE id = ?1 AND board = ?2",
            params![
                id,
                board.id,
                c.name,
                c.title,
                c.body,
                serde_json::to_string(&c.labels).unwrap_or_else(|_| "[]".into()),
                c.place,
                c.cron,
                c.tz,
                c.on,
                c.host,
                c.agent,
                c.pick,
                self.now
            ],
        )?;
        self.changed = true;
        self.card_cron_view(&board, id)
    }

    pub fn card_cron_delete(&mut self, path: &str, id: i64) -> Result<()> {
        let board = self.resolve(path)?;
        self.card_cron_row(board.id, id)?;
        self.tx.execute("DELETE FROM card_crons WHERE id = ?1 AND board = ?2", params![id, board.id])?;
        self.changed = true;
        Ok(())
    }

    /// 「现在建一张」：立即按这条建一张（开关关着也行），同样受跳过规则约束。返回这条（带这一次的结果）
    pub fn card_cron_run(&mut self, path: &str, id: i64) -> Result<CardCron> {
        let board = self.resolve(path)?;
        let c = self.card_cron_row(board.id, id)?;
        self.fire(&board, &c, self.now)?;
        self.card_cron_view(&board, id)
    }

    /// 服务端每分钟调用一次：`minute` 是这一分钟里的任一时刻（毫秒）。打开着、按自己的时区这一分钟对得上 cron 的都到点
    /// （建卡或跳过）。返回到点的条数
    pub fn card_crons_due(&mut self, minute: i64) -> Result<usize> {
        let all: Vec<(CardCron, i64)> = {
            let mut st = self.tx.prepare(&format!("SELECT {COLS} FROM card_crons WHERE enabled = 1 ORDER BY id"))?;
            let rows = st.query_map([], row)?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        let mut n = 0;
        for (c, b) in all {
            let (Ok(cron), Ok(tz)) = (Cron::parse(&c.cron), TimeZone::get(&c.tz)) else { continue };
            if !due_at(&cron, &tz, minute) {
                continue;
            }
            let board = self.board_ref(b)?;
            self.fire(&board, &c, minute)?;
            n += 1;
        }
        Ok(n)
    }

    fn board_ref(&self, id: i64) -> Result<BoardRef> {
        Ok(self.tx.query_row("SELECT id, project, key FROM boards WHERE id = ?1", [id], |r| {
            Ok(BoardRef { id: r.get(0)?, project: r.get(1)?, key: r.get(2)? })
        })?)
    }

    fn card_cron_view(&self, board: &BoardRef, id: i64) -> Result<CardCron> {
        self.read().card_crons(board.id, self.now)?.into_iter().find(|c| c.id == id).ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("定时建卡 #{id} 不存在")))
    }

    /// 到点：上一张还没完成（不在终点状态、没被删除）就跳过，否则按模板建一张
    fn fire(&mut self, board: &BoardRef, c: &CardCron, at: i64) -> Result<()> {
        if let Some(prev) = c.last_card {
            let col: Option<(String, Option<i64>)> = self
                .tx
                .query_row("SELECT col, deleted_at FROM cards WHERE board = ?1 AND id = ?2", params![board.id, prev], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let flow = flow_of(self.tx, board.id)?;
            if let Some((col, None)) = col {
                if !flow.is_final(&col) {
                    self.tx.execute(
                        "UPDATE card_crons SET ran_at = ?2, ran_card = NULL, ran_skip = ?3, ran_to = NULL, ran_err = NULL WHERE id = ?1",
                        params![c.id, self.now, prev],
                    )?;
                    self.changed = true;
                    return Ok(());
                }
            }
        }
        let tz = tz_of(&c.tz)?;
        // 标签：只带项目里还在的
        let mut labels = vec![];
        for l in &c.labels {
            let ok: bool = self.tx.query_row("SELECT COUNT(*) > 0 FROM labels WHERE id = ?1 AND project = ?2", params![l, board.project], |r| r.get(0))?;
            if ok {
                labels.push(*l);
            }
        }
        let body = self.copy_files(board.id, &c.body)?;
        let actor = Actor { kind: ActorKind::System, name: CARD_CRON_ACTOR.into() };
        let inbox = c.place == INBOX;
        let column = if inbox { Some(INBOX.to_string()) } else { None };
        let d = self.exec(board, &actor, Cmd::New { title: dated(&c.name, &tz, at), body: Some(body), column, top: false, labels, from: None })?;
        let id = d.card.id;
        // 动态第一条写明是哪条定时建卡建的
        let text = format!("定时建卡「{}」建了这张卡", c.name);
        self.tx.execute("UPDATE activity SET text = ?3 WHERE board = ?1 AND card = ?2 AND action = 'create'", params![board.id, id, text])?;
        self.tx.execute("UPDATE events SET text = ?3 WHERE board = ?1 AND card = ?2 AND op = 'create'", params![board.id, id, text])?;
        // 派给哪台：指定的就是它；随机挑的在这里挑一台
        let target: Option<(String, Option<String>)> = match (c.pick.as_str(), inbox) {
            (PICK_RANDOM, false) => Some(self.cron_pick_host(board, c.agent.as_deref())?),
            (PICK_HOST, false) => c.host.clone().map(|h| (h, c.agent.clone())),
            _ => None,
        };
        let (to, err) = match target {
            Some((host, agent)) => match self.send(board, id, &host, agent.as_deref(), &actor)? {
                Ok(to) => (Some(to), None),
                Err(why) => {
                    let home = flow_of(self.tx, board.id)?.home().name.clone();
                    let text = format!("定时建卡「{}」没能把这张卡派发到 {host}：{why}\n\n卡片留在「{home}」，可以手动派发，或者由定时领取去做。", c.name);
                    // 评论只是说明：被规则拒绝也不影响建卡
                    if let Err(e) = self.exec(board, &actor, Cmd::Comment { id, text }) {
                        if e.code == ErrorCode::Internal {
                            return Err(e);
                        }
                    }
                    (None, Some(why))
                }
            },
            None => (None, None),
        };
        self.tx.execute(
            "UPDATE card_crons SET ran_at = ?2, ran_card = ?3, ran_skip = NULL, last_card = ?3, ran_to = ?4, ran_err = ?5 WHERE id = ?1",
            params![c.id, self.now, id, to, err],
        )?;
        self.changed = true;
        Ok(())
    }

    /// 随机挑一台能接这块白板的机器（以及上面用哪个 agent）：和手动派发一样能派到就算能接——
    /// 机器存在、配了工作空间（或这块白板的目录）、在线或离线都算（离线时排队等它上线）、暂停接单也算（排队等它恢复）、
    /// 环境检查没通过的不算（派过去也做不了）；没指定 agent 时这台机器上至少有一个找到了命令。
    /// `agent` 指定了时只留这台机器上有这个 agent 的。返回（机器名，agent 或 None 表示用它默认的）
    fn cron_pick_host(&self, board: &BoardRef, agent: Option<&str>) -> Result<(String, Option<String>)> {
        let list = self.cron_host_candidates(board, agent)?;
        let Some(list) = list else {
            return Err(BoardError::usage(self.cron_host_pick_why(board, agent)));
        };
        Ok(list[self.roll(list.len())].clone())
    }

    /// 能接这块白板的机器，按注册先后：每台（机器名，agent 或 None）；一台都没有时为空
    fn cron_host_candidates(&self, board: &BoardRef, agent: Option<&str>) -> Result<Option<Vec<(String, Option<String>)>>> {
        let path = board.path();
        let names: Vec<String> = self.collect("SELECT name FROM hosts ORDER BY id", [], |r| r.get(0))?;
        let mut out = vec![];
        for name in names {
            let Some(h) = self.host_row(&name)? else { continue };
            if workspace_of(h.workspace_raw.as_deref(), &h.dirs).is_none() && !h.dirs.iter().any(|d| d.board == path) {
                continue;
            }
            if let Readiness::Failed(_) = self.read().dir_readiness(&h, board)? {
                continue;
            }
            let found = |a: &crate::hosts::HostAgent| a.found;
            match agent {
                Some(a) => {
                    if h.resolved().iter().any(|x| x.id == a && found(x)) {
                        out.push((h.name.clone(), Some(a.to_string())));
                    }
                }
                // 没指定：这台机器有默认 agent（默认 agent 设的那个找到了，或第一个找到了的）
                None => {
                    if h.default_agent().is_some() {
                        out.push((h.name.clone(), None));
                    }
                }
            }
        }
        Ok((!out.is_empty()).then_some(out))
    }

    /// 保存一条随机派发的时看现在有没有能接这块白板的机器、agent 有没有找到：没有就 `USAGE`，写明原因
    fn cron_host_pick_check(&self, board: &BoardRef, agent: Option<&str>) -> Result<()> {
        if self.cron_host_candidates(board, agent)?.is_some() {
            return Ok(());
        }
        Err(BoardError::usage(self.cron_host_pick_why(board, agent)))
    }

    /// 没有机器能接时写明为什么
    fn cron_host_pick_why(&self, board: &BoardRef, agent: Option<&str>) -> String {
        let path = board.path();
        let all: Vec<String> = self.collect("SELECT name FROM hosts ORDER BY id", [], |r| r.get(0)).unwrap_or_default();
        if all.is_empty() {
            return "还没有机器：在要干活的机器上运行 board host，它会出现在这里".into();
        }
        let machines = all.join("、");
        match &agent {
            Some(a) => format!("没有能接 {path}、又有 agent「{a}」的机器（要配了工作空间、环境检查通过，那个 agent 也要找到了命令）：机器 {machines}"),
            None => format!("没有能接 {path} 的机器（要配了工作空间、环境检查通过，默认 agent 也要找到了）：机器 {machines}"),
        }
    }

    /// 随机数：`0..n`。每次取一个新的随机状态，再从系统时间、进程号、卡片数、派发记录数混出结果：
    /// 同一毫秒里连着建几张、几台机器一起跑，也都会变（只用时间的低位时，连续几次常常落在同一台）
    fn roll(&self, n: usize) -> usize {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_i64(self.now);
        h.write_u32(std::process::id());
        let cards: i64 = self.tx.query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0)).unwrap_or(0);
        h.write_i64(cards);
        let sends: i64 = self.tx.query_row("SELECT COUNT(*) FROM dispatches", [], |r| r.get(0)).unwrap_or(0);
        h.write_i64(sends);
        (h.finish() % n as u64) as usize
    }

    /// 把刚建的卡派发到这台机器（来源「定时建卡」）。派不出去时撤掉这一步的改动，返回原因；
    /// 存储层故障照常往上抛
    fn send(&mut self, board: &BoardRef, card: i64, host: &str, agent: Option<&str>, by: &Actor) -> Result<std::result::Result<String, String>> {
        self.tx.execute_batch("SAVEPOINT card_cron_send")?;
        match self.dispatch_create_from(board, card, host, agent, by, SOURCE_SCHEDULE) {
            Ok(d) => {
                self.tx.execute_batch("RELEASE card_cron_send")?;
                Ok(Ok(format!("{} · {}", d.host, d.agent_label)))
            }
            Err(e) => {
                self.tx.execute_batch("ROLLBACK TO card_cron_send; RELEASE card_cron_send")?;
                if e.code == ErrorCode::Internal {
                    return Err(e);
                }
                Ok(Err(e.message))
            }
        }
    }

    /// 模板描述里引用的附件：每个复制一份（同一个文件，新的附件记录、还没归属），描述里的引用换成新的，
    /// 建卡时归到新卡；模板自己的那份不动
    fn copy_files(&mut self, board: i64, body: &str) -> Result<String> {
        let mut out = String::with_capacity(body.len());
        let mut rest = body;
        const KEY: &str = "/api/files/";
        while let Some(i) = rest.find(KEY) {
            let after = &rest[i + KEY.len()..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            out.push_str(&rest[..i + KEY.len()]);
            rest = &after[digits.len()..];
            let Ok(old) = digits.parse::<i64>() else {
                out.push_str(&digits);
                continue;
            };
            let n = self.tx.execute(
                "INSERT INTO attachments (board, card, name, mime, size, sha256, created_at, created_by_kind, created_by_name)
                 SELECT board, NULL, name, mime, size, sha256, ?3, 'system', ?4 FROM attachments WHERE id = ?1 AND board = ?2",
                params![old, board, self.now, CARD_CRON_ACTOR],
            )?;
            if n == 1 {
                out.push_str(&self.tx.last_insert_rowid().to_string());
            } else {
                out.push_str(&digits);
            }
        }
        out.push_str(rest);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HostAgent, HostDir, HostReport, HostWorkspace, Origin, Store};

    const B: &str = "default/main";
    /// 2023-11-14 22:13:20 UTC；上海是 11-15 06:13:20
    const T0: i64 = 1_700_000_000_000;
    const MIN: i64 = 60_000;

    fn store() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.ensure_default(T0).unwrap();
        s
    }

    fn patch(name: &str, cron: &str) -> CardCronPatch {
        CardCronPatch { name: Some(name.into()), cron: Some(cron.into()), tz: Some("UTC".into()), ..Default::default() }
    }

    fn create(s: &mut Store, p: CardCronPatch) -> Result<CardCron> {
        s.write(T0, Origin::ui(None), |w| w.card_cron_create(B, &p))
    }

    fn due(s: &mut Store, at: i64) -> usize {
        s.write(at, Origin::system(), |w| w.card_crons_due(at)).unwrap()
    }

    fn list(s: &Store, now: i64) -> Vec<CardCron> {
        s.read(|r| {
            let b = r.resolve(B)?;
            r.card_crons(b.id, now)
        })
        .unwrap()
    }

    fn card(s: &Store, id: i64) -> CardDetail {
        s.read(|r| {
            let b = r.resolve(B)?;
            r.card(&b, id)
        })
        .unwrap()
    }

    fn cols(s: &Store) -> Cols {
        s.read(|r| {
            let b = r.resolve(B)?;
            Ok(r.board_snapshot(&b, T0, false)?.cols)
        })
        .unwrap()
    }

    #[test]
    fn next_and_due_follow_the_time_zone() {
        let cron = Cron::parse("0 9 * * *").unwrap();
        let sh = TimeZone::get("Asia/Shanghai").unwrap();
        // 上海 11-15 06:13 之后的 09:00 是当天 09:00（UTC 01:00）
        let n = next_at(&cron, &sh, T0).unwrap();
        assert_eq!(Timestamp::from_millisecond(n).unwrap().to_string(), "2023-11-15T01:00:00Z");
        assert!(due_at(&cron, &sh, n + 30_000), "这一分钟里的任一时刻都算到点");
        assert!(!due_at(&cron, &TimeZone::UTC, n), "UTC 的 01:00 不是 9 点");
        // 工作日：周五之后是下周一
        let wk = Cron::parse("30 8 * * 1-5").unwrap();
        let fri = Timestamp::from_millisecond(T0).unwrap().checked_add(72.hours()).unwrap().as_millisecond(); // 11-17（周五）22:13 UTC
        assert_eq!(Timestamp::from_millisecond(next_at(&wk, &TimeZone::UTC, fri).unwrap()).unwrap().to_string(), "2023-11-20T08:30:00Z");
        // 每分钟：下一分钟整
        let every = Cron::parse("* * * * *").unwrap();
        assert_eq!(next_at(&every, &TimeZone::UTC, T0), Some(T0 - 20_000 + MIN));
        // 永远到不了的日子：没有下次
        assert_eq!(next_at(&Cron::parse("0 0 30 2 *").unwrap(), &TimeZone::UTC, T0), None);
        assert_eq!(dated("清理死代码", &sh, T0), "清理死代码（11-15）");
    }

    #[test]
    fn create_checks_every_field() {
        let mut s = store();
        let bug = s.write(T0, Origin::ui(None), |w| w.create_label("default", "bug", None, false)).unwrap();
        let c = create(&mut s, CardCronPatch { labels: Some(vec![bug.id, bug.id]), ..patch("清理死代码", "0  9 * * 1") }).unwrap();
        assert_eq!((c.cron.as_str(), c.labels.clone(), c.place.as_str(), c.on), ("0 9 * * 1", vec![bug.id], "home", true), "默认开、放到首个可领取状态");
        assert_eq!((c.name.as_str(), c.title.as_str()), ("清理死代码", "清理死代码"), "标题就是名称");
        assert!(c.next_at.is_some());
        let usage = |s: &mut Store, p: CardCronPatch| {
            let e = create(s, p).unwrap_err();
            assert_eq!(e.code, ErrorCode::Usage, "{}", e.message);
            e.message
        };
        assert!(usage(&mut s, patch("清理死代码", "0 9 * * *")).contains("已经有"), "名称在白板内唯一");
        assert!(usage(&mut s, patch(" ", "0 9 * * *")).contains("名称"));
        assert!(usage(&mut s, patch(&"长".repeat(25), "0 9 * * *")).contains("24"));
        assert!(usage(&mut s, CardCronPatch { title: Some(" ".into()), ..patch(" ", "0 9 * * *") }).contains("名称"), "只传卡片标题（旧客户端）不当成名称");
        let only_title = create(&mut s, CardCronPatch { title: Some("标题".into()), ..patch(" 只传标题 ", "0 9 * * *") }).unwrap();
        assert_eq!((only_title.name.as_str(), only_title.title.as_str()), ("只传标题", "只传标题"));
        let extra = create(&mut s, CardCronPatch { title: Some("别的标题".into()), ..patch("标题没用", "0 9 * * *") }).unwrap();
        assert_eq!(extra.title, "标题没用", "旧的卡片标题不再单独存");
        assert!(usage(&mut s, patch("b", "61 9 * * *")).contains("第 1 段"), "不合法的 cron 写明哪一段不对");
        assert!(usage(&mut s, CardCronPatch { tz: Some("Mars/Base".into()), ..patch("c", "0 9 * * *") }).contains("时区"));
        assert!(usage(&mut s, CardCronPatch { place: Some("done".into()), ..patch("d", "0 9 * * *") }).contains("放到哪"));
        assert!(usage(&mut s, CardCronPatch { labels: Some(vec![9999]), ..patch("e", "0 9 * * *") }).contains("标签"));
        for i in 4..=20 {
            create(&mut s, patch(&format!("第 {i} 条"), "0 9 * * *")).unwrap();
        }
        assert!(usage(&mut s, patch("第 21 条", "0 9 * * *")).contains("最多 20 条"));
        // 修改：没给的项不变；改名撞上别的也拒绝
        let u = s.write(T0, Origin::ui(None), |w| w.card_cron_update(B, c.id, &CardCronPatch { on: Some(false), ..Default::default() })).unwrap();
        assert_eq!((u.on, u.name.as_str(), u.next_at), (false, "清理死代码", None), "关着没有下次");
        let e = s.write(T0, Origin::ui(None), |w| w.card_cron_update(B, c.id, &CardCronPatch { name: Some("标题没用".into()), ..Default::default() })).unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage);
        s.write(T0, Origin::ui(None), |w| w.card_cron_delete(B, c.id)).unwrap();
        assert_eq!(s.write(T0, Origin::ui(None), |w| w.card_cron_delete(B, c.id)).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn due_creates_a_card_and_skips_while_the_last_is_open() {
        let mut s = store();
        let bug = s.write(T0, Origin::ui(None), |w| w.create_label("default", "bug", None, false)).unwrap();
        let c = create(&mut s, CardCronPatch { body: Some("找出没人用的代码".into()), labels: Some(vec![bug.id]), ..patch("清理死代码", "* * * * *") }).unwrap();
        let t1 = T0 + MIN;
        assert_eq!(due(&mut s, t1), 1);
        let todo = cols(&s)["todo"].clone();
        let id = *todo.last().unwrap();
        let d = card(&s, id);
        assert_eq!(d.card.title, "清理死代码（11-14）", "标题后面加日期");
        assert_eq!((d.card.body.as_str(), d.card.labels.clone()), ("找出没人用的代码", vec![bug.id]));
        assert_eq!((d.card.created_by.kind, d.card.created_by.display()), (ActorKind::System, CARD_CRON_ACTOR));
        assert_eq!(d.activity[0].text, "定时建卡「清理死代码」建了这张卡");
        let c1 = list(&s, t1).into_iter().find(|x| x.id == c.id).unwrap();
        assert_eq!((c1.ran_card, c1.ran_skip, c1.last_card, c1.ran_at), (Some(id), None, Some(id), Some(t1)));
        // 那张还在待办：到点不建，记跳过
        assert_eq!(due(&mut s, t1 + MIN), 1);
        assert_eq!(cols(&s)["todo"], todo, "不建新卡");
        let c2 = list(&s, t1).into_iter().find(|x| x.id == c.id).unwrap();
        assert_eq!((c2.ran_card, c2.ran_skip, c2.last_card), (None, Some(id), Some(id)));
        // 「现在建一张」同样跳过
        let r = s.write(t1 + MIN, Origin::ui(None), |w| w.card_cron_run(B, c.id)).unwrap();
        assert_eq!(r.ran_skip, Some(id));
        // 拖进已完成后照常建
        s.write(t1, Origin::ui(None), |w| {
            let b = w.resolve(B)?;
            w.exec(&b, &Actor::human(), Cmd::Move { id, column: "done".into(), top: false, index: None, order: None, before: None, after: None })
        })
        .unwrap();
        let r = s.write(t1 + 2 * MIN, Origin::ui(None), |w| w.card_cron_run(B, c.id)).unwrap();
        let id2 = r.ran_card.expect("上一张完成了，现在建一张");
        assert_eq!(*cols(&s)["todo"].last().unwrap(), id2);
        // 上一张被删除（进回收站）也照常建
        s.write(t1, Origin::ui(None), |w| {
            let b = w.resolve(B)?;
            w.exec(&b, &Actor::human(), Cmd::Delete { id: id2 })
        })
        .unwrap();
        assert_eq!(due(&mut s, t1 + 3 * MIN), 1);
        let c3 = list(&s, t1).into_iter().find(|x| x.id == c.id).unwrap();
        assert!(c3.ran_card.is_some() && c3.ran_card != Some(id2));
        // 关掉开关：到点不建
        s.write(t1, Origin::ui(None), |w| w.card_cron_update(B, c.id, &CardCronPatch { on: Some(false), ..Default::default() })).unwrap();
        assert_eq!(due(&mut s, t1 + 10 * MIN), 0);
    }

    #[test]
    fn inbox_place_time_zone_and_template_files() {
        let mut s = store();
        // 每天上海 9 点，放到未整理；模板里贴了一张图
        let b = s.read(|r| r.resolve(B)).unwrap();
        s.conn()
            .execute(
                "INSERT INTO attachments (id, board, card, name, mime, size, sha256, created_at, created_by_kind, created_by_name)
                 VALUES (77, ?1, NULL, 'a.png', 'image/png', 3, 'abc', ?2, 'human', '')",
                params![b.id, T0],
            )
            .unwrap();
        let c = create(
            &mut s,
            CardCronPatch { place: Some(INBOX.into()), tz: Some("Asia/Shanghai".into()), body: Some("看图 ![a.png](/api/files/77)".into()), ..patch("找改进", "0 9 * * *") },
        )
        .unwrap();
        let at = c.next_at.unwrap();
        assert_eq!(due(&mut s, at - MIN), 0, "上海 8:59 不到点");
        assert_eq!(due(&mut s, at + 5_000), 1);
        let id = *cols(&s)[INBOX].last().unwrap();
        let d = card(&s, id);
        assert_eq!(d.card.title, "找改进（11-15）", "日期按这条的时区");
        assert!(!d.card.body.contains("/api/files/77") && d.card.body.contains("/api/files/"), "引用换成复制出来的那份：{}", d.card.body);
        assert_eq!(d.attachments.len(), 1, "复制的附件归到新卡");
        let left: Option<i64> = s.conn().query_row("SELECT card FROM attachments WHERE id = 77", [], |r| r.get(0)).unwrap();
        assert_eq!(left, None, "模板自己的那份不动");
        // 附件清理不删模板引用着的
        s.gc_files(T0 + 3 * 86_400_000).unwrap();
        let n: i64 = s.conn().query_row("SELECT COUNT(*) FROM attachments WHERE id = 77", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        // 删除白板时一起删除（整个工作区至少留一块白板：先建一块别的）
        s.write(T0, Origin::ui(None), |w| w.create_board("default", "另一块", "other", "", None).map(|_| ())).unwrap();
        s.write(T0, Origin::ui(None), |w| w.delete_board(B)).unwrap();
        let n: i64 = s.conn().query_row("SELECT COUNT(*) FROM card_crons", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "白板删掉了，定时建卡也没了");
    }

    /// 一台机器上报：有 claude（找到了）和 pi（没找到）；`dirs` 非空时配了工作空间（`/tmp`），为空时没配
    fn host_up(s: &mut Store, name: &str, dirs: &[&str]) {
        let agent = |id: &str, label: &str, found: bool| HostAgent { id: id.into(), name: label.into(), found, model: "m".into(), ..Default::default() };
        let rep = HostReport {
            name: name.into(),
            instance: format!("{name}-1"),
            resume: None,
            os: "macOS".into(),
            version: "0.1.24".into(),
            agents: vec![agent("claude", "Claude Code", true), agent("pi", "pi", false)],
            envs: vec![],
            workspace: (!dirs.is_empty()).then(|| HostWorkspace { path: "/tmp".into(), ..Default::default() }),
            dirs: dirs.iter().map(|b| HostDir { board: b.to_string(), path: format!("/tmp/{b}"), ..Default::default() }).collect(),
            dirs_rev: 0,
            running: vec![],
            done: false,
            asked: None,
            ask_error: None,
            selftest: true,
            skills: None,
            mcp: None,
            providers: None,
            local_skills: None,
        };
        s.write(T0, Origin::cli(), |w| w.host_beat(&rep, Some(1))).unwrap();
    }

    fn update(s: &mut Store, id: i64, p: CardCronPatch) -> Result<CardCron> {
        s.write(T0, Origin::ui(None), |w| w.card_cron_update(B, id, &p))
    }

    /// v0.1.22 建的表没有派发的几列：打开时补上，原来的条目不派发
    #[test]
    fn old_table_gets_dispatch_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let mut s = Store::open(&path).unwrap();
            s.ensure_default(T0).unwrap();
            create(&mut s, patch("清理死代码", "0 9 * * 1")).unwrap();
        }
        {
            let c = rusqlite::Connection::open(&path).unwrap();
            c.execute_batch(
                "ALTER TABLE card_crons DROP COLUMN host; ALTER TABLE card_crons DROP COLUMN agent;
                 ALTER TABLE card_crons DROP COLUMN ran_to; ALTER TABLE card_crons DROP COLUMN ran_err;",
            )
            .unwrap();
        }
        let mut s = Store::open(&path).expect("旧库要能打开");
        let c = list(&s, T0).pop().unwrap();
        assert_eq!((c.name.as_str(), c.host, c.ran_err), ("清理死代码", None, None));
        assert_eq!(due(&mut s, c.next_at.unwrap()), 1);
    }

    /// 旧库里名称和卡片标题不一样（以前是两个输入框）：标题按名称显示；打开改一次保存后统一成名称，到点建的卡用名称
    #[test]
    fn old_title_follows_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        let (id, name) = {
            let mut s = Store::open(&path).unwrap();
            s.ensure_default(T0).unwrap();
            let c = create(&mut s, patch("清道夫", "* * * * *")).unwrap();
            (c.id, c.name.clone())
        };
        // 以前是两个输入框：标题可以写成别的
        {
            let c = rusqlite::Connection::open(&path).unwrap();
            c.execute("UPDATE card_crons SET title = '扫地' WHERE id = ?1", params![id]).unwrap();
        }
        let mut s = Store::open(&path).expect("旧库要能打开");
        let c = list(&s, T0).pop().unwrap();
        assert_eq!((c.name.as_str(), c.title.as_str()), ("清道夫", "清道夫"), "标题就是名称");
        // 到点建的卡用名称，不用旧的标题
        let r = s.write(T0 + MIN, Origin::ui(None), |w| w.card_cron_run(B, id)).unwrap();
        let d = card(&s, r.ran_card.expect("建了卡"));
        assert_eq!(d.card.title, "清道夫（11-14）");
        // 打开编辑时只填名称，保存（不带 title）后旧标题也换成名称
        let c = update(&mut s, id, CardCronPatch { body: Some("找出没人用的代码".into()), ..Default::default() }).unwrap();
        assert_eq!((c.name, c.title), (name.clone(), name.clone()));
        let old: String = s.conn().query_row("SELECT title FROM card_crons WHERE id = ?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(old, name, "旧列也存成名称");
    }

    #[test]
    fn host_is_checked_when_saving() {
        let mut s = store();
        host_up(&mut s, "mbp", &[B]);
        host_up(&mut s, "win", &[]);
        let usage = |r: Result<CardCron>| {
            let e = r.unwrap_err();
            assert_eq!(e.code, ErrorCode::Usage, "{}", e.message);
            e.message
        };
        let on = |h: &str| CardCronPatch { host: Some(h.into()), ..patch("更新代码", "0 3 * * *") };
        assert!(usage(create(&mut s, on("nope"))).contains("没有叫「nope」的机器"));
        assert!(usage(create(&mut s, on("win"))).contains("还没有配工作空间"));
        assert!(usage(create(&mut s, CardCronPatch { agent: Some("codex".into()), ..on("mbp") })).contains("没有 agent「codex」"));
        assert!(usage(create(&mut s, CardCronPatch { place: Some(INBOX.into()), ..on("mbp") })).contains("不派发"));
        // 没找到命令的 agent 也能先选上（到点时再看）
        let c = create(&mut s, CardCronPatch { agent: Some("pi".into()), ..on("mbp") }).unwrap();
        assert_eq!((c.host.as_deref(), c.agent.as_deref()), (Some("mbp"), Some("pi")));
        // 换成随机：机器清掉、agent 跟着清（到点按挑中那台的默认 agent）；再换回指定的机器要重新选
        let c = update(&mut s, c.id, CardCronPatch { pick: Some(PICK_RANDOM.into()), ..Default::default() }).unwrap();
        assert_eq!((c.pick.as_str(), c.host.as_deref(), c.agent), (PICK_RANDOM, None, None));
        let c = update(&mut s, c.id, CardCronPatch { pick: Some("host".into()), host: Some("mbp".into()), ..Default::default() }).unwrap();
        assert_eq!((c.pick.as_str(), c.host.as_deref()), (PICK_HOST, Some("mbp")));
        // 换机器、没说 agent：用新机器的默认 agent；空字符串表示不派发，agent 一起清掉
        host_up(&mut s, "mini", &[B]);
        let c = update(&mut s, c.id, CardCronPatch { host: Some("mini".into()), ..Default::default() }).unwrap();
        assert_eq!((c.host.as_deref(), c.agent), (Some("mini"), None));
        let c = update(&mut s, c.id, CardCronPatch { host: Some(" ".into()), agent: Some("claude".into()), ..Default::default() }).unwrap();
        assert_eq!((c.host, c.agent), (None, None));
        // 机器改名：跟着改
        let c = update(&mut s, c.id, CardCronPatch { host: Some("mbp".into()), ..Default::default() }).unwrap();
        s.write(T0, Origin::ui(None), |w| w.host_rename("mbp", "mbp2").map(|_| ())).unwrap();
        assert_eq!(list(&s, T0)[0].host.as_deref(), Some("mbp2"));
        // 机器被移除后：只开关、改别的照样能保存，再选它才拒绝
        s.write(T0, Origin::ui(None), |w| w.host_remove("mbp2", &Actor::human(), "人")).unwrap();
        let c = update(&mut s, c.id, CardCronPatch { on: Some(false), name: Some("拉代码".into()), ..Default::default() }).unwrap();
        assert_eq!((c.name.as_str(), c.title.as_str()), ("拉代码", "拉代码"), "改名称，标题跟着变");
        assert_eq!((c.on, c.host.as_deref()), (false, Some("mbp2")));
        assert!(usage(update(&mut s, c.id, CardCronPatch { host: Some("mbp".into()), ..Default::default() })).contains("没有叫「mbp」的机器"));
    }

    #[test]
    fn due_dispatches_to_the_host() {
        let mut s = store();
        host_up(&mut s, "mbp", &[B]);
        let c = create(&mut s, CardCronPatch { host: Some("mbp".into()), body: Some("git pull 之后重新编译".into()), ..patch("更新代码", "* * * * *") }).unwrap();
        let t1 = T0 + MIN;
        assert_eq!(due(&mut s, t1), 1);
        let c1 = list(&s, t1).into_iter().find(|x| x.id == c.id).unwrap();
        let id = c1.ran_card.expect("建了卡");
        assert_eq!((c1.ran_to.as_deref(), c1.ran_err.as_deref()), (Some("mbp · Claude Code"), None), "默认 agent");
        // 和手动派发一样排队、占用卡片；来源是定时建卡，发起人是定时建卡
        let d = s
            .read(|r| {
                let b = r.resolve(B)?;
                r.card_dispatch(&b, id)
            })
            .unwrap()
            .expect("派发了");
        assert_eq!((d.host.as_str(), d.agent.as_str(), d.state.as_str(), d.source.as_str()), ("mbp", "claude", "queued", SOURCE_SCHEDULE));
        assert_eq!(d.by.display(), CARD_CRON_ACTOR);
        assert!(cols(&s)["doing"].contains(&id), "排队时卡片进了「进行中」");
        let dc = card(&s, id);
        assert_eq!(dc.activity[0].text, "定时建卡「更新代码」建了这张卡");
        assert!(dc.activity.iter().any(|a| a.text.contains("派发到 mbp · Claude Code（定时建卡）")), "{:?}", dc.activity.iter().map(|a| &a.text).collect::<Vec<_>>());
        // 还在跑（排队）时到点：跳过
        assert_eq!(due(&mut s, t1 + MIN), 1);
        let c2 = list(&s, t1).into_iter().find(|x| x.id == c.id).unwrap();
        assert_eq!((c2.ran_skip, c2.ran_to), (Some(id), None));
        // 指定的 agent 没找到命令：卡照样建，留在待办，评论和结果写明原因
        s.write(t1, Origin::ui(None), |w| {
            let b = w.resolve(B)?;
            w.exec(&b, &Actor::human(), Cmd::Delete { id })
        })
        .unwrap();
        update(&mut s, c.id, CardCronPatch { agent: Some("pi".into()), ..Default::default() }).unwrap();
        let r = s.write(t1 + 2 * MIN, Origin::ui(None), |w| w.card_cron_run(B, c.id)).unwrap();
        let id2 = r.ran_card.expect("派不出去也建卡");
        let err = r.ran_err.expect("记下原因");
        assert!(err.contains("pi"), "{err}");
        assert_eq!(r.ran_to, None);
        assert_eq!(*cols(&s)["todo"].last().unwrap(), id2, "留在待办");
        let none = s
            .read(|r| {
                let b = r.resolve(B)?;
                r.card_dispatch(&b, id2)
            })
            .unwrap();
        assert!(none.is_none(), "没有留下半截派发");
        let last = card(&s, id2).activity.into_iter().last().unwrap();
        assert!(last.text.starts_with("定时建卡「更新代码」没能把这张卡派发到 mbp："), "{}", last.text);
        assert!(last.text.contains("卡片留在「待办」"), "{}", last.text);
    }

    /// 随机派发：不指定机器，每次在能接这块白板的机器里挑一台，按那台的默认 agent 派
    #[test]
    fn random_dispatches_to_any_host_that_can_take_the_board() {
        let mut s = store();
        host_up(&mut s, "mbp", &[B]);
        host_up(&mut s, "win", &[]);
        let usage = |r: Result<CardCron>| {
            let e = r.unwrap_err();
            assert_eq!(e.code, ErrorCode::Usage, "{}", e.message);
            e.message
        };
        // 只有 mbp 能接：随机也只挑得到它；指定的机器跟着随机一起清掉
        let c = create(&mut s, CardCronPatch { pick: Some(PICK_RANDOM.into()), host: Some("win".into()), ..patch("更新代码", "* * * * *") }).unwrap();
        assert_eq!((c.pick.as_str(), c.host, c.agent.as_deref()), (PICK_RANDOM, None, None), "随机的没有指定的机器");
        assert!(usage(create(&mut s, CardCronPatch { pick: Some(PICK_RANDOM.into()), agent: Some("codex".into()), ..patch("x1", "0 9 * * *") })).contains("codex"));
        assert!(usage(create(&mut s, CardCronPatch { pick: Some("any".into()), ..patch("x2", "0 9 * * *") })).contains("random"));
        assert!(usage(create(&mut s, CardCronPatch { pick: Some(PICK_RANDOM.into()), place: Some(INBOX.into()), ..patch("x3", "0 9 * * *") })).contains(INBOX_NAME));
        // 到点：派给能接的那台
        let t1 = T0 + MIN;
        assert_eq!(due(&mut s, t1), 1);
        let r = list(&s, t1).into_iter().find(|x| x.id == c.id).unwrap();
        assert_eq!((r.ran_to.as_deref(), r.ran_err.as_deref()), (Some("mbp · Claude Code"), None));
        let id = r.ran_card.expect("建了卡");
        let d = s
            .read(|read| {
                let b = read.resolve(B)?;
                read.card_dispatch(&b, id)
            })
            .unwrap()
            .expect("派发了");
        assert_eq!((d.host.as_str(), d.source.as_str()), ("mbp", SOURCE_SCHEDULE));
        s.write(t1, Origin::ui(None), |w| {
            let b = w.resolve(B)?;
            w.exec(&b, &Actor::human(), Cmd::Delete { id })
        })
        .unwrap();
        // 再来一台暂停接单的也能接（离线、暂停接单的排队等它）：两台能接的都会挑到
        host_up(&mut s, "mini", &[B]);
        s.write(t1, Origin::ui(None), |w| w.host_update_as("mini", &crate::HostPatch { paused: Some(true), ..Default::default() }, "人").map(|_| ())).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for i in 1..=40 {
            let r = s.write(t1 + i * MIN, Origin::ui(None), |w| w.card_cron_run(B, c.id)).unwrap();
            let id = r.ran_card.expect("建了卡");
            seen.insert(r.ran_to.expect("派发了").split(" · ").next().unwrap().to_string());
            s.write(t1, Origin::ui(None), |w| {
                let b = w.resolve(B)?;
                w.exec(&b, &Actor::human(), Cmd::Delete { id })
            })
            .unwrap();
        }
        assert_eq!(seen, ["mbp", "mini"].map(String::from).into_iter().collect(), "两台能接的都会挑到");
        // 能接的机器都被移除了：随机没有能接的机器，保存被拒绝并写明原因；已经有的照样能开关
        s.write(t1, Origin::ui(None), |w| w.host_remove("mbp", &Actor::human(), "人")).unwrap();
        s.write(t1, Origin::ui(None), |w| w.host_remove("mini", &Actor::human(), "人")).unwrap();
        assert!(usage(create(&mut s, CardCronPatch { pick: Some(PICK_RANDOM.into()), ..patch("随机", "0 9 * * *") })).contains("没有能接"));
        update(&mut s, c.id, CardCronPatch { on: Some(false), ..Default::default() }).unwrap();
    }
}