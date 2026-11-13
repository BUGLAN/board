//! SQLite 存储。
//!
//! - 只有服务端打开数据库文件（`board-server --db`，默认 `~/.board/board.db`，`BOARD_DB` 可覆盖）；网页端、桌面端、CLI 都经服务端访问。
//! - 所有写操作在 `BEGIN IMMEDIATE` 事务里串行执行：多个 agent 同时领取同一张卡片，只有一个成功（规则 2）。
//! - 每次有变更的写入把 `meta.rev` 加一；服务端轮询 rev，把变更推给已打开的界面。

use crate::error::{BoardError, ErrorCode, Result};
use crate::model::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value INTEGER NOT NULL
);
INSERT OR IGNORE INTO meta (key, value) VALUES ('rev', 0), ('schema', 1), ('hosts', 0);

CREATE TABLE IF NOT EXISTS projects (
  key         TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  color       TEXT NOT NULL,
  pos_x       REAL NOT NULL DEFAULT 0,
  pos_y       REAL NOT NULL DEFAULT 0,
  sort        INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS boards (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  project     TEXT NOT NULL REFERENCES projects(key) ON DELETE CASCADE,
  key         TEXT NOT NULL,
  name        TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  pos_x       REAL NOT NULL DEFAULT 0,
  pos_y       REAL NOT NULL DEFAULT 0,
  sort        INTEGER NOT NULL DEFAULT 0,
  seq         INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  -- 白板设置里「保存布局」存下的状态块布局（JSON：状态标识 → {x, w, y}）；NULL 表示没保存过
  saved_layout TEXT,
  -- 不再读写（原来的「保存顺序」开关，已换成 keep_sorted）；保留这一列，免得给旧库做删列迁移
  keep_order  INTEGER NOT NULL DEFAULT 1,
  -- 1 = 保留顺序：界面上每个状态块按自己的排序依据（statuses.order_by）显示，拖动只换状态、不改顺序；0 = 拖到哪就在哪
  keep_sorted INTEGER NOT NULL DEFAULT 0,
  -- 白板说明（Markdown，可以为空）：大盘白板概览里不显示，只在「编辑白板」里改
  readme      TEXT NOT NULL DEFAULT '',
  -- 环境检查（PRD §9.1）：所有系统 / 只给 Windows 的一行命令；都为空时不检查
  ready_check     TEXT NOT NULL DEFAULT '',
  ready_check_win TEXT NOT NULL DEFAULT '',
  -- 代码仓库地址（git 远端，例如 git@github.com:o/r.git）和集成分支（PRD §4）；都为空时按原来的规则
  repo        TEXT NOT NULL DEFAULT '',
  branch      TEXT NOT NULL DEFAULT '',
  UNIQUE (project, key)
);

CREATE TABLE IF NOT EXISTS labels (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES projects(key) ON DELETE CASCADE,
  name    TEXT NOT NULL,
  color   TEXT NOT NULL,
  sort    INTEGER NOT NULL DEFAULT 0,
  -- 1 = 仅人可打：只有人能在界面上给卡片加、去这个标签；0 = agent 可打（人和 agent 都能打）
  human_only INTEGER NOT NULL DEFAULT 0,
  -- 1 = 优先：不带 ID 领取（和定时领取）时，带这个标签的卡片先被领取（PRD §8 规则 1）
  priority   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS cards (
  board           INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  id              INTEGER NOT NULL,
  title           TEXT NOT NULL,
  body            TEXT NOT NULL DEFAULT '',
  col             TEXT NOT NULL,
  pos             INTEGER NOT NULL,
  holder          TEXT,
  claimed_at      INTEGER,
  expires_at      INTEGER, -- 不再读写（原来的 30 分钟领取时长）；保留这一列，免得给旧库做删列迁移
  beat_at         INTEGER,
  final_at        INTEGER, -- 这次领取已经 final（正常结束心跳）的时间；之后的普通心跳被拒绝，重新领取时清空
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  moved_at        INTEGER, -- 移入现在这个状态的时间（W::place 换状态时写）；旧卡片迁移时按动态补
  created_by_kind TEXT NOT NULL,
  created_by_name TEXT NOT NULL,
  -- 人看过「未整理」里的这一条的时间（PRD §10.5）：agent 记下、还没被人看过的条目算「新」
  seen_at         INTEGER,
  -- 拆自：同一块白板上另一张卡的编号（board new --from）。原卡删除、永久删除后这里照样留着编号
  split_from      INTEGER,
  -- 在回收站里：删除的时间；NULL = 正常（PRD §8 规则 10）。回收站里的卡片和它的动态、评论、附件都还在，30 天后由 purge_trash 永久删除
  deleted_at      INTEGER,
  -- 回收站里：谁删的
  deleted_by_kind TEXT,
  deleted_by_name TEXT,
  -- 回收站里：删除前在哪个状态、排第几位，恢复时放回去
  del_col         TEXT,
  del_pos         INTEGER,
  -- 回收站里：删除前领取它的那个可领取状态（被删时还被持有的才有）。删除会释放领取、清空 claimed_from，
  -- 恢复时靠这一列回到「当初领取它的可领取状态」（PRD §8 规则 10）
  del_from        TEXT,
  -- 回收站里：删除前是不是被持有
  del_held        INTEGER,
  PRIMARY KEY (board, id)
);
CREATE INDEX IF NOT EXISTS cards_col ON cards (board, col, pos);
-- 回收站的索引 cards_trash 不在这里建：旧库的 cards 还没有 deleted_at，要等 setup 里补完列再建

-- 每一次领取的耗时：第一次心跳开始，final / 交审 / 释放 / 异常退出（最后一次心跳）结束
-- how：final 正常结束 | move 持有者移出锁定状态 | release 人强制释放 | lost 心跳中断 | timeout board host 超时停止
CREATE TABLE IF NOT EXISTS card_runs (
  board      INTEGER NOT NULL,
  card       INTEGER NOT NULL,
  n          INTEGER NOT NULL,
  agent      TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  ended_at   INTEGER,
  how        TEXT,
  -- 这一次收到的心跳次数（含 final）
  beats      INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (board, card, n),
  FOREIGN KEY (board, card) REFERENCES cards(board, id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS card_labels (
  board INTEGER NOT NULL,
  card  INTEGER NOT NULL,
  label INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  PRIMARY KEY (board, card, label),
  FOREIGN KEY (board, card) REFERENCES cards(board, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS card_labels_label ON card_labels (label);

CREATE TABLE IF NOT EXISTS activity (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  board      INTEGER NOT NULL,
  card       INTEGER NOT NULL,
  at         INTEGER NOT NULL,
  actor_kind TEXT NOT NULL,
  actor_name TEXT NOT NULL,
  kind       TEXT NOT NULL,
  action     TEXT NOT NULL,
  text       TEXT NOT NULL,
  edited_at  INTEGER,
  FOREIGN KEY (board, card) REFERENCES cards(board, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS activity_card ON activity (board, card, id);

CREATE TABLE IF NOT EXISTS events (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  board      INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  card       INTEGER NOT NULL,
  title      TEXT NOT NULL,
  at         INTEGER NOT NULL,
  actor_kind TEXT NOT NULL,
  actor_name TEXT NOT NULL,
  op         TEXT NOT NULL,
  text       TEXT NOT NULL,
  via        TEXT NOT NULL,
  client     TEXT
);
CREATE INDEX IF NOT EXISTS events_board ON events (board, id);

-- 每块白板的工作流（PRD §7）。lx / lw / ly 是状态块的布局，空表示用默认布局；
-- view 是卡片形态 card | line | tile，空表示按规则取（终点用磁贴，其他用卡片）
CREATE TABLE IF NOT EXISTS statuses (
  board INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  key   TEXT NOT NULL,
  name  TEXT NOT NULL,
  color TEXT NOT NULL,
  sort  INTEGER NOT NULL,
  claim INTEGER NOT NULL DEFAULT 0,
  lock  INTEGER NOT NULL DEFAULT 0,
  final INTEGER NOT NULL DEFAULT 0,
  error INTEGER NOT NULL DEFAULT 0,
  view  TEXT,
  -- 保留顺序开着时的排序依据 moved | id | updated | labels | work，空表示按移入时间；order_desc = 1 倒序
  order_by   TEXT,
  order_desc INTEGER NOT NULL DEFAULT 0,
  lx    INTEGER,
  lw    INTEGER,
  ly    REAL,
  PRIMARY KEY (board, key)
);

-- 工作流的连线（PRD §7）：src → dst 表示 agent 可以把卡片从 src 移到 dst
CREATE TABLE IF NOT EXISTS transitions (
  board INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  src   TEXT NOT NULL,
  dst   TEXT NOT NULL,
  PRIMARY KEY (board, src, dst)
);
"#;

/// 需要释放的领取：从领取或最近一次心跳算起，连续 5 次没收到心跳（?1 = 现在，?2 = 允许的间隔）
pub(crate) const DUE: &str = "holder IS NOT NULL AND COALESCE(beat_at, claimed_at, 0) + ?2 <= ?1";

/// 每块白板保留的事件条数（卡片自己的动态不受影响）
const EVENTS_KEEP: i64 = 500;

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 本地时间 HH:MM（动态文字里用）
pub fn hhmm(ms: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.format("%H:%M").to_string(),
        chrono::LocalResult::None => "--:--".into(),
    }
}

/// 标识：小写字母、数字和 `-`，以字母或数字开头，最长 32 个字符
pub fn valid_key(key: &str) -> bool {
    let b = key.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// 白板路径：`<项目>/<白板>`，两段都是标识（[`valid_key`]）
pub fn valid_board(b: &str) -> bool {
    matches!(b.split_once('/'), Some((p, k)) if valid_key(p) && valid_key(k))
}

pub(crate) fn split_path(raw: &str) -> (&str, Option<&str>) {
    match raw.find('/') {
        Some(i) => (&raw[..i], Some(&raw[i + 1..])),
        None => (raw, None),
    }
}

pub struct Store {
    conn: Connection,
    /// 附件目录（数据库旁边的 files/）；内存数据库没有
    files: Option<PathBuf>,
}

/// 新建项目时的第一块白板
#[derive(Debug, Clone)]
pub struct NewBoard {
    pub name: String,
    pub key: String,
    pub desc: String,
    pub pos: Option<Pos>,
}

impl Store {
    /// 默认数据库位置：`BOARD_DB`，否则 `~/.board/board.db`
    pub fn default_path() -> PathBuf {
        if let Some(p) = std::env::var_os("BOARD_DB").filter(|p| !p.is_empty()) {
            return PathBuf::from(p);
        }
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".board").join("board.db")
    }

    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Store::setup(conn, Some(crate::files::dir_for(path)))
    }

    pub fn open_in_memory() -> Result<Store> {
        Store::setup(Connection::open_in_memory()?, None)
    }

    fn setup(conn: Connection, files: Option<PathBuf>) -> Result<Store> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(crate::files::SCHEMA)?;
        conn.execute_batch(crate::agents::SCHEMA)?;
        crate::agents::migrate(&conn)?;
        conn.execute_batch(crate::hosts::SCHEMA)?;
        crate::hosts::migrate(&conn)?;
        conn.execute_batch(crate::moves::SCHEMA)?;
        conn.execute_batch(crate::skills::SCHEMA)?;
        conn.execute_batch(crate::mcp::SCHEMA)?;
        conn.execute_batch(crate::templates::SCHEMA)?;
        conn.execute_batch(crate::card_crons::SCHEMA)?;
        // 旧数据库的 cards 没有心跳列
        let has_beat: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'beat_at'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_beat {
            conn.execute_batch("ALTER TABLE cards ADD COLUMN beat_at INTEGER")?;
        }
        // 旧数据库的 cards 不记录领取来源
        let has_from: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'claimed_from'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_from {
            conn.execute_batch("ALTER TABLE cards ADD COLUMN claimed_from TEXT")?;
        }
        // 旧数据库的 card_runs 不记心跳次数
        let has_beats: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('card_runs') WHERE name = 'beats'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_beats {
            conn.execute_batch("ALTER TABLE card_runs ADD COLUMN beats INTEGER NOT NULL DEFAULT 0")?;
        }
        // 旧数据库的 cards 不记 final
        let has_final: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'final_at'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_final {
            conn.execute_batch("ALTER TABLE cards ADD COLUMN final_at INTEGER")?;
        }
        // 旧数据库的动态不记评论的修改时间
        let has_edited: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('activity') WHERE name = 'edited_at'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_edited {
            conn.execute_batch("ALTER TABLE activity ADD COLUMN edited_at INTEGER")?;
        }
        // 旧数据库的状态没有卡片形态
        let has_view: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('statuses') WHERE name = 'view'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_view {
            conn.execute_batch("ALTER TABLE statuses ADD COLUMN view TEXT")?;
        }
        // 旧数据库的状态没有「错误节点」属性：都不是（不按名称猜，人在状态设置里打开）
        let has_error: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('statuses') WHERE name = 'error'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_error {
            conn.execute_batch("ALTER TABLE statuses ADD COLUMN error INTEGER NOT NULL DEFAULT 0")?;
        }
        // 旧数据库的标签没有类型：都算 agent 可打
        let has_human_only: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('labels') WHERE name = 'human_only'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_human_only {
            conn.execute_batch("ALTER TABLE labels ADD COLUMN human_only INTEGER NOT NULL DEFAULT 0")?;
        }
        // 旧数据库的标签都不是优先
        let has_priority: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('labels') WHERE name = 'priority'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_priority {
            conn.execute_batch("ALTER TABLE labels ADD COLUMN priority INTEGER NOT NULL DEFAULT 0")?;
        }
        // 旧数据库的白板没有白板设置：没有已保存的布局，保存顺序开着（和原来一样能拖动排序）
        let has_saved: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('boards') WHERE name = 'saved_layout'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_saved {
            conn.execute_batch("ALTER TABLE boards ADD COLUMN saved_layout TEXT")?;
        }
        let has_keep: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('boards') WHERE name = 'keep_order'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_keep {
            conn.execute_batch("ALTER TABLE boards ADD COLUMN keep_order INTEGER NOT NULL DEFAULT 1")?;
        }
        // 「保存顺序」换成「保留顺序」：原来关着保存顺序（同一块里不能拖动排序）的白板打开保留顺序（顺序不让拖动改），其余关着（拖到哪就在哪，和原来一样）
        let has_sorted: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('boards') WHERE name = 'keep_sorted'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_sorted {
            conn.execute_batch(
                "ALTER TABLE boards ADD COLUMN keep_sorted INTEGER NOT NULL DEFAULT 0;
                 UPDATE boards SET keep_sorted = 1 WHERE keep_order = 0;",
            )?;
        }
        // 旧数据库的状态没有排序依据：都按移入时间
        let has_order: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('statuses') WHERE name = 'order_by'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_order {
            conn.execute_batch("ALTER TABLE statuses ADD COLUMN order_by TEXT; ALTER TABLE statuses ADD COLUMN order_desc INTEGER NOT NULL DEFAULT 0;")?;
        }
        // 旧数据库的卡片不记移入时间：按最后一次换状态的动态补，没有就用新建时间
        let has_moved: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'moved_at'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_moved {
            conn.execute_batch(
                "ALTER TABLE cards ADD COLUMN moved_at INTEGER;
                 UPDATE cards SET moved_at = COALESCE(
                   (SELECT MAX(a.at) FROM activity a WHERE a.board = cards.board AND a.card = cards.id
                      AND a.action IN ('create', 'move', 'claim', 'release', 'lost', 'timeout')),
                   created_at);",
            )?;
        }
        // 旧数据库的卡片没有回收站：都在正常状态里（不是已删除）
        let has_trash: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'deleted_at'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_trash {
            conn.execute_batch(
                "ALTER TABLE cards ADD COLUMN deleted_at INTEGER;
                 ALTER TABLE cards ADD COLUMN deleted_by_kind TEXT;
                 ALTER TABLE cards ADD COLUMN deleted_by_name TEXT;
                 ALTER TABLE cards ADD COLUMN del_col TEXT;
                 ALTER TABLE cards ADD COLUMN del_pos INTEGER;
                 ALTER TABLE cards ADD COLUMN del_from TEXT;
                 ALTER TABLE cards ADD COLUMN del_held INTEGER;",
            )?;
        }
        // 回收站的索引：新库、旧库都在补完列之后建（放进 SCHEMA 会先于补列执行，旧库打开就报 no such column）
        conn.execute_batch("CREATE INDEX IF NOT EXISTS cards_trash ON cards (board, deleted_at)")?;
        // 旧数据库的回收站不记删除前的领取来源
        let has_del_from: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'del_from'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_del_from {
            conn.execute_batch("ALTER TABLE cards ADD COLUMN del_from TEXT; ALTER TABLE cards ADD COLUMN del_held INTEGER;")?;
        }
        // 旧数据库的白板没有说明
        let has_readme: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('boards') WHERE name = 'readme'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_readme {
            conn.execute_batch("ALTER TABLE boards ADD COLUMN readme TEXT NOT NULL DEFAULT ''")?;
        }
        // 旧数据库的白板没有环境检查：不检查，和原来一样
        let has_check: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('boards') WHERE name = 'ready_check'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_check {
            conn.execute_batch(
                "ALTER TABLE boards ADD COLUMN ready_check TEXT NOT NULL DEFAULT '';
                 ALTER TABLE boards ADD COLUMN ready_check_win TEXT NOT NULL DEFAULT '';",
            )?;
        }
        // 旧数据库的白板没有代码仓库地址和集成分支：都留空，按原来的规则（AGENTS.md / 远端默认分支）
        let has_repo: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('boards') WHERE name = 'repo'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_repo {
            conn.execute_batch(
                "ALTER TABLE boards ADD COLUMN repo TEXT NOT NULL DEFAULT '';
                 ALTER TABLE boards ADD COLUMN branch TEXT NOT NULL DEFAULT '';",
            )?;
        }
        // 旧数据库不记「未整理」的条目有没有被人看过：已经不在「未整理」里的都算看过（那次整理就是看过），
        // 还在「未整理」里的算没看过——和升级前一样，没有点；之后有人打开时照常标成看过
        let has_seen: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'seen_at'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_seen {
            conn.execute_batch(
                "ALTER TABLE cards ADD COLUMN seen_at INTEGER;
                 UPDATE cards SET seen_at = COALESCE(moved_at, created_at) WHERE col != 'inbox' OR created_by_kind = 'human';",
            )?;
        }
        // 旧数据库的卡片没有「拆自」
        let has_split: bool =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'split_from'", [], |r| r.get::<_, i64>(0))? > 0;
        if !has_split {
            conn.execute_batch("ALTER TABLE cards ADD COLUMN split_from INTEGER")?;
        }
        // 旧数据库的工作流没有连线：按原来的顺序给相邻状态依次连线（PRD §7）
        let edged: bool = conn.query_row("SELECT COUNT(*) FROM meta WHERE key = 'edges'", [], |r| r.get::<_, i64>(0))? > 0;
        if !edged {
            let boards: Vec<i64> = conn.prepare("SELECT id FROM boards")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            for b in boards {
                let keys: Vec<String> = conn
                    .prepare("SELECT key FROM statuses WHERE board = ?1 ORDER BY sort, key")?
                    .query_map([b], |r| r.get(0))?
                    .collect::<std::result::Result<_, _>>()?;
                for w in keys.windows(2) {
                    conn.execute("INSERT OR IGNORE INTO transitions (board, src, dst) VALUES (?1, ?2, ?3)", params![b, w[0], w[1]])?;
                }
            }
            conn.execute("INSERT INTO meta (key, value) VALUES ('edges', 1)", [])?;
        }
        // 旧数据库里的白板没有工作流：补上默认的六个状态（DEFAULT_FLOW，见 model.rs）
        let missing: Vec<i64> = conn
            .prepare("SELECT id FROM boards b WHERE NOT EXISTS (SELECT 1 FROM statuses s WHERE s.board = b.id)")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for b in missing {
            insert_default_flow(&conn, b)?;
        }
        // 旧数据库没有流转记录：从动态回填（只跑一次）
        crate::moves::backfill(&conn)?;
        Ok(Store { conn, files })
    }

    /// 工作区为空时建立默认项目（整个工作区至少保留一块白板）
    pub fn ensure_default(&mut self, now: i64) -> Result<()> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM boards", [], |r| r.get(0))?;
        if n > 0 {
            return Ok(());
        }
        self.write(now, Origin::system(), |w| {
            w.create_project(
                "默认项目",
                "default",
                "",
                Some(PROJECT_COLORS[0]),
                None,
                NewBoard { name: "主白板".into(), key: "main".into(), desc: String::new(), pos: None },
            )
        })?;
        Ok(())
    }

    /// 执行一组写操作：同一时刻只有一个写事务；出错则整体回滚
    pub fn write<T>(&mut self, now: i64, origin: Origin, f: impl FnOnce(&mut W) -> Result<T>) -> Result<T> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut w = W { tx: &tx, now, origin, changed: false, hosts_changed: false, dispatch_ok: false };
        let out = f(&mut w)?;
        let (changed, hosts) = (w.changed || w.hosts_changed, w.hosts_changed);
        if changed {
            tx.execute("UPDATE meta SET value = value + 1 WHERE key = 'rev'", [])?;
        }
        // 机器和派发有变化：board host 据此重新取自己的任务（任务下发需求 §十）
        if hosts {
            tx.execute("UPDATE meta SET value = value + 1 WHERE key = 'hosts'", [])?;
        }
        tx.commit()?;
        Ok(out)
    }

    /// `board next`：结算上一张、开始这个会话排着的下一张。不再接着做时（SESSION_DONE）先提交——排着的派发要取消——再返回错误
    pub fn next(&mut self, now: i64, origin: Origin, name: &str) -> Result<CardDetail> {
        self.write(now, origin, |w| w.dispatch_next(name))?
    }

    /// 只读快照：在一个读事务里完成，保证 rev 与数据一致
    pub fn read<T>(&self, f: impl FnOnce(&R) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let out = f(&R { conn: &tx });
        drop(tx);
        out
    }

    pub fn rev(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = 'rev'", [], |r| r.get(0))?)
    }

    /// 机器和派发的版本：有变化时加一（实时推送里带上，board host 据此重新取自己的任务）
    pub fn hosts_rev(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COALESCE((SELECT value FROM meta WHERE key = 'hosts'), 0)", [], |r| r.get(0))?)
    }

    /// 服务端进程启动时调用：把停机前还活着的名字、机器和领取的心跳时间顺延到现在，
    /// 停机的那段时间不计入 50 秒心跳超时（重启后不会把全部领取一次性释放）。
    /// 只由服务进程启动时调用；`Store::open` 和 `token` / `backup` 等子命令都不调用，不会改动心跳时间。
    pub fn resume_after_downtime(&mut self, now: i64) -> Result<usize> {
        self.write(now, Origin::system(), |w| {
            let mut n = w.tx.execute("UPDATE agents SET beat_at = ?1 WHERE off_at IS NULL", [w.now])?;
            n += w.tx.execute("UPDATE hosts SET beat_at = ?1 WHERE off_at IS NULL", [w.now])?;
            // 还没发过第一次心跳的领取：从领取时间重新算；已经发过心跳的：从最近一次心跳重新算
            n += w.tx.execute(
                "UPDATE cards SET claimed_at = ?1 WHERE holder IS NOT NULL AND beat_at IS NULL AND deleted_at IS NULL",
                [w.now],
            )?;
            n += w.tx.execute(
                "UPDATE cards SET beat_at = ?1 WHERE holder IS NOT NULL AND beat_at IS NOT NULL AND deleted_at IS NULL",
                [w.now],
            )?;
            Ok(n)
        })
    }

    /// 规则 7：扫描所有白板，释放心跳中断的领取；心跳中断超过 50 秒的名字标为离线（PRD §3）；
    /// 心跳中断超过 50 秒的机器标为离线，名字已经离线的在跑派发结束（机器掉线，见 hosts.rs）。
    /// 返回释放的张数加上离线的名字、机器个数和结束的派发个数
    pub fn sweep(&mut self, now: i64) -> Result<usize> {
        let gap = BEAT_MS * BEAT_MISSES;
        // 先用只读查询判断，避免没有到期的卡片和名字时也抢写锁
        let due: i64 = self.conn.query_row(
            &format!(
                "SELECT (SELECT COUNT(*) FROM cards WHERE deleted_at IS NULL AND {DUE}) + (SELECT COUNT(*) FROM agents WHERE off_at IS NULL AND beat_at + ?2 <= ?1) + {}",
                crate::hosts::DUE_COUNT
            ),
            params![now, gap],
            |r| r.get(0),
        )?;
        if due == 0 {
            return Ok(0);
        }
        self.write(now, Origin::system(), |w| {
            let boards: Vec<i64> =
                w.collect(&format!("SELECT DISTINCT board FROM cards WHERE deleted_at IS NULL AND {DUE}"), params![now, gap], |r| r.get(0))?;
            let mut n = 0;
            for b in boards {
                n += w.sweep_board(b)?;
            }
            n += w.sweep_agents()?;
            n += w.sweep_hosts()?;
            Ok(n)
        })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// 附件目录；内存数据库没有
    pub fn files_dir(&self) -> Option<&Path> {
        self.files.as_deref()
    }
}

/* ============================== 读 ============================== */

pub struct R<'a> {
    pub(crate) conn: &'a Connection,
}

fn row_actor(kind: String, name: String) -> Actor {
    Actor { kind: ActorKind::parse(&kind), name }
}

impl<'a> R<'a> {
    pub fn rev(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = 'rev'", [], |r| r.get(0))?)
    }

    /// 把 `--board` / `BOARD` 的值解析成白板（PRD §3）
    pub fn resolve(&self, raw: &str) -> Result<BoardRef> {
        resolve(self.conn, raw)
    }

    pub fn workspace(&self, now: i64) -> Result<Workspace> {
        let conn = self.conn;
        let mut projects: Vec<ProjectInfo> = Vec::new();
        {
            let mut st = conn.prepare(
                "SELECT key, name, description, color, pos_x, pos_y, created_at FROM projects ORDER BY sort, created_at, key",
            )?;
            let rows = st.query_map([], |r| {
                Ok(ProjectInfo {
                    key: r.get(0)?,
                    name: r.get(1)?,
                    desc: r.get(2)?,
                    color: r.get(3)?,
                    pos: Pos { x: r.get(4)?, y: r.get(5)? },
                    created_at: r.get(6)?,
                    labels: vec![],
                    boards: vec![],
                })
            })?;
            for p in rows {
                projects.push(p?);
            }
        }
        for p in projects.iter_mut() {
            p.labels = labels_of(conn, &p.key)?;
            let mut st = conn.prepare(
                "SELECT b.id, b.key, b.name, b.description, b.pos_x, b.pos_y, b.created_at,
                        COALESCE((SELECT MAX(at) FROM events e WHERE e.board = b.id), b.created_at), b.readme,
                        b.ready_check, b.ready_check_win, b.repo, b.branch
                 FROM boards b WHERE b.project = ?1 ORDER BY b.sort, b.created_at, b.key",
            )?;
            let rows = st.query_map([&p.key], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    BoardInfo {
                        key: r.get(1)?,
                        path: String::new(),
                        name: r.get(2)?,
                        desc: r.get(3)?,
                        readme: r.get(8)?,
                        ready_check: ReadyCheck { cmd: r.get(9)?, win: r.get(10)? },
                        repo: r.get(11)?,
                        branch: r.get(12)?,
                        pos: Pos { x: r.get(4)?, y: r.get(5)? },
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                        counts: Counts::new(),
                        statuses: vec![],
                        holders: vec![],
                        card_crons: vec![],
                    },
                ))
            })?;
            for row in rows {
                let (id, mut b) = row?;
                b.path = format!("{}/{}", p.key, b.key);
                b.card_crons = R { conn }.card_crons(id, now)?;
                b.statuses = flow_of(conn, id)?.0.into_iter().map(|s| Status { layout: None, ..s }).collect();
                b.counts.insert(INBOX.to_string(), 0);
                for s in &b.statuses {
                    b.counts.insert(s.key.clone(), 0);
                }
                let mut cs = conn.prepare("SELECT col, COUNT(*) FROM cards WHERE board = ?1 AND deleted_at IS NULL GROUP BY col")?;
                for c in cs.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                    let (col, n) = c?;
                    *b.counts.entry(col).or_default() += n;
                }
                let mut hs = conn.prepare(&format!(
                    "SELECT holder FROM cards c WHERE board = ?1 AND holder IS NOT NULL AND deleted_at IS NULL ORDER BY {}, pos",
                    COL_ORDER
                ))?;
                for h in hs.query_map([id], |r| r.get::<_, String>(0))? {
                    b.holders.push(h?);
                }
                p.boards.push(b);
            }
        }
        Ok(Workspace { rev: self.rev()?, now, projects, agents: self.agents(now)?, hosts: self.hosts(now)?, dispatches: self.active_dispatches()? })
    }

    pub fn labels(&self, project: &str) -> Result<Vec<Label>> {
        labels_of(self.conn, project)
    }

    /// 白板的工作流（按顺序，含布局）
    pub fn flow(&self, board: &BoardRef) -> Result<Flow> {
        flow_of(self.conn, board.id)
    }

    /// 白板快照。full 为 false 时每个终点状态只带最近移入的 FINAL_RECENT 张（按最后一次移入 / 新建的动态，不按 pos：
    /// 人可能把新完成的放在最上面，也可能放在最下面），省略的张数记在 omitted 里。留下的卡片在 cols 里仍按 pos 排，
    /// 省略的卡片可能夹在它们中间。终点状态的卡片只增不减，全带的话快照越来越大（每次推送都要重取）
    pub fn board_snapshot(&self, board: &BoardRef, now: i64, full: bool) -> Result<BoardSnapshot> {
        let flow = flow_of(self.conn, board.id)?;
        let mut cols = Cols::new();
        cols.insert(INBOX.to_string(), vec![]);
        for s in &flow.0 {
            cols.insert(s.key.clone(), vec![]);
        }
        let mut omitted = BTreeMap::new();
        let mut skip: Vec<i64> = vec![];
        if !full {
            let mut st = self.conn.prepare_cached(
                "SELECT c.id FROM cards c WHERE c.board = ?1 AND c.col = ?2 AND c.deleted_at IS NULL
                 ORDER BY (SELECT MAX(a.id) FROM activity a WHERE a.board = c.board AND a.card = c.id AND a.action IN ('move', 'create', 'restore')) DESC,
                          c.id DESC
                 LIMIT -1 OFFSET ?3",
            )?;
            for s in flow.0.iter().filter(|s| s.fin) {
                let ids = st
                    .query_map(params![board.id, s.key, FINAL_RECENT as i64], |r| r.get::<_, i64>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if !ids.is_empty() {
                    omitted.insert(s.key.clone(), ids.len());
                    skip.extend(ids);
                }
            }
        }
        let mut cards = BTreeMap::new();
        for c in cards_query(self.conn, board.id, None, &skip)? {
            cols.entry(c.column.clone()).or_default().push(c.id);
            cards.insert(c.id, c);
        }
        let mut log = events_query(
            self.conn,
            "WHERE e.board = ?1 ORDER BY e.id DESC LIMIT 30",
            params![board.id],
        )?;
        log.reverse();
        let (saved, keep_sorted, repo, branch) =
            self.conn.query_row("SELECT saved_layout, keep_sorted, repo, branch FROM boards WHERE id = ?1", [board.id], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, bool>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
            })?;
        // 已保存的布局里只留现在还在的状态
        let saved_layout = saved.and_then(|j| serde_json::from_str::<BTreeMap<String, Tile>>(&j).ok()).map(|mut m| {
            m.retain(|k, _| flow.get(k).is_some());
            m
        });
        let trash: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM cards WHERE board = ?1 AND deleted_at IS NOT NULL", [board.id], |r| r.get(0))?;
        Ok(BoardSnapshot {
            rev: self.rev()?,
            now,
            path: board.path(),
            statuses: flow.0,
            cols,
            cards,
            omitted,
            log,
            saved_layout,
            keep_sorted,
            trash,
            repo,
            branch,
        })
    }

    pub fn card(&self, board: &BoardRef, id: i64) -> Result<CardDetail> {
        let card = cards_of(self.conn, board.id, Some(id))?
            .pop()
            .ok_or_else(|| not_found(id))?;
        self.card_detail(board, card)
    }

    /// 回收站里的卡片（按删除时间，最近删的在最后）
    pub fn trash(&self, board: &BoardRef) -> Result<Vec<TrashCard>> {
        // 「删除前在哪个状态」「谁删的」都在 cards 上，和卡片的其余字段一起读出来
        cards_from(self.conn, "AND c.board = ?1 AND c.deleted_at IS NOT NULL AND ?2 IS NULL", params![board.id, None::<i64>], &[])
            .map(|v| {
                v.into_iter()
                    .map(|(card, row)| {
                        let deleted_at = row.deleted_at.unwrap_or(card.created_at);
                        TrashCard {
                            card,
                            deleted_at,
                            deleted_by: row.deleted_by,
                            col: row.del_col.unwrap_or_default(),
                            pos: row.del_pos.unwrap_or(0),
                            held_from: row.del_from,
                            was_held: row.del_held,
                        }
                    })
                    .collect()
            })
    }

    /// 这张卡片在不在回收站里（给卡片命令用：在回收站里的卡片不能编辑、移动、加标签、评论，报 IN_TRASH）
    pub fn in_trash(&self, board: &BoardRef, id: i64) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NOT NULL",
            params![board.id, id],
            |r| r.get::<_, i64>(0),
        )? > 0)
    }

    fn card_detail(&self, board: &BoardRef, card: Card) -> Result<CardDetail> {
        let id = card.id;
        let mut st = self.conn.prepare(
            "SELECT id, at, actor_kind, actor_name, kind, action, text, edited_at FROM activity
             WHERE board = ?1 AND card = ?2 ORDER BY id",
        )?;
        let activity = st
            .query_map(params![board.id, id], |r| {
                Ok(Activity {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    actor: row_actor(r.get(2)?, r.get(3)?),
                    kind: r.get(4)?,
                    action: r.get(5)?,
                    text: r.get(6)?,
                    edited_at: r.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let attachments = crate::files::attachments_of(self.conn, board.id, id)?;
        let split_into = self.split_into(board.id, id)?;
        Ok(CardDetail { card, activity, attachments, split_into })
    }

    /// 「拆自」是这张的卡片（不含回收站里的），按编号
    pub(crate) fn split_into(&self, b: i64, id: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT id FROM cards WHERE board = ?1 AND split_from = ?2 AND deleted_at IS NULL ORDER BY id")?;
        let ids = st.query_map(params![b, id], |r| r.get(0))?.collect::<std::result::Result<Vec<i64>, _>>()?;
        Ok(ids)
    }

    /// id 之后的全局事件（服务端推给界面）
    pub fn events_since(&self, after: i64, limit: i64) -> Result<Vec<Event>> {
        events_query(self.conn, "WHERE e.id > ?1 ORDER BY e.id LIMIT ?2", params![after, limit])
    }

    /// 一块白板的动态，按 id 倒序（新的在前），按游标向前翻页。
    ///
    /// `before` 是上一页最后一条的 id：只取 id 比它小的，因此两页之间不重不漏；
    /// 不传表示从最新一条开始。翻到底（没有更早的了）返回空数组，不报错。
    /// `limit` 的取值范围由调用方（HTTP 接口）校验。
    pub fn events_page(&self, board: &BoardRef, before: Option<i64>, limit: i64) -> Result<Vec<Event>> {
        match before {
            Some(before) => events_query(
                self.conn,
                "WHERE e.board = ?1 AND e.id < ?2 ORDER BY e.id DESC LIMIT ?3",
                params![board.id, before, limit],
            ),
            None => events_query(self.conn, "WHERE e.board = ?1 ORDER BY e.id DESC LIMIT ?2", params![board.id, limit]),
        }
    }

    pub fn last_event_id(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COALESCE(MAX(id), 0) FROM events", [], |r| r.get(0))?)
    }
}

fn not_found(id: i64) -> BoardError {
    BoardError::new(ErrorCode::NotFound, format!("卡片 #{id} 不存在"))
}

fn resolve(conn: &Connection, raw: &str) -> Result<BoardRef> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(BoardError::code(ErrorCode::NoBoard));
    }
    let (pk, bk) = split_path(raw);
    let exists: Option<String> =
        conn.query_row("SELECT key FROM projects WHERE key = ?1", [pk], |r| r.get(0)).optional()?;
    if exists.is_none() {
        return Err(BoardError::new(
            ErrorCode::BoardNotFound,
            format!("项目「{pk}」不存在，用 board boards 查看可用的白板"),
        ));
    }
    let mut st = conn.prepare("SELECT id, key FROM boards WHERE project = ?1 ORDER BY sort, created_at, key")?;
    let boards: Vec<(i64, String)> = st
        .query_map([pk], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let keys = || boards.iter().map(|b| b.1.as_str()).collect::<Vec<_>>();
    match bk {
        None => {
            if boards.len() == 1 {
                let (id, key) = boards[0].clone();
                return Ok(BoardRef { id, project: pk.into(), key });
            }
            if boards.is_empty() {
                return Err(BoardError::new(ErrorCode::BoardNotFound, format!("项目「{pk}」里还没有白板")));
            }
            Err(BoardError::new(
                ErrorCode::BoardNotFound,
                format!(
                    "项目「{pk}」有 {} 块白板，请写成 {pk}/<白板>：{}",
                    boards.len(),
                    keys().join("、")
                ),
            )
            .with_detail(serde_json::json!({ "boards": keys() })))
        }
        Some(bk) => match boards.iter().find(|b| b.1 == bk) {
            Some((id, key)) => Ok(BoardRef { id: *id, project: pk.into(), key: key.clone() }),
            None => Err(BoardError::new(
                ErrorCode::BoardNotFound,
                format!(
                    "项目「{pk}」里没有白板「{bk}」，可选：{}",
                    if boards.is_empty() { "（空）".to_string() } else { keys().join("、") }
                ),
            )
            .with_detail(serde_json::json!({ "boards": keys() }))),
        },
    }
}

fn labels_of(conn: &Connection, project: &str) -> Result<Vec<Label>> {
    let mut st = conn.prepare(
        "SELECT l.id, l.name, l.color, (SELECT COUNT(*) FROM card_labels cl JOIN cards c ON c.board = cl.board AND c.id = cl.card \
         WHERE cl.label = l.id AND c.deleted_at IS NULL), l.human_only, l.priority \
         FROM labels l WHERE l.project = ?1 ORDER BY l.sort, l.id",
    )?;
    let out = st
        .query_map([project], |r| {
            Ok(Label { id: r.get(0)?, name: r.get(1)?, color: r.get(2)?, uses: r.get(3)?, human_only: r.get(4)?, priority: r.get(5)? })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

/// 卡片按「未整理」→ 工作流顺序排列（c 是 cards 的别名）
const COL_ORDER: &str = "CASE WHEN c.col = 'inbox' THEN -1 ELSE \
     COALESCE((SELECT s.sort FROM statuses s WHERE s.board = c.board AND s.key = c.col), 1000000) END";

pub(crate) fn flow_of(conn: &Connection, board: i64) -> Result<Flow> {
    let mut st = conn.prepare_cached(
        "SELECT key, name, color, claim, lock, final, lx, lw, ly, view, error, order_by, order_desc FROM statuses WHERE board = ?1 ORDER BY sort, key",
    )?;
    let v = st
        .query_map([board], |r| {
            let (lx, lw, ly): (Option<i64>, Option<i64>, Option<f64>) = (r.get(6)?, r.get(7)?, r.get(8)?);
            let fin = r.get::<_, i64>(5)? != 0;
            let view: Option<String> = r.get(9)?;
            let order_by: Option<String> = r.get(11)?;
            Ok(Status {
                key: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                claim: r.get::<_, i64>(3)? != 0,
                lock: r.get::<_, i64>(4)? != 0,
                fin,
                err: r.get::<_, i64>(10)? != 0,
                view: view.as_deref().and_then(CardView::parse).unwrap_or(CardView::default_for(fin)),
                order: Order { by: order_by.as_deref().and_then(OrderBy::parse).unwrap_or_default(), desc: r.get::<_, i64>(12)? != 0 },
                layout: match (lx, lw, ly) {
                    (Some(x), Some(w), Some(y)) => Some(Tile { x, w, y }),
                    _ => None,
                },
                to: vec![],
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if v.is_empty() {
        return Err(BoardError::new(ErrorCode::Internal, "白板没有工作流"));
    }
    let edges: Vec<(String, String)> = conn
        .prepare_cached("SELECT src, dst FROM transitions WHERE board = ?1")?
        .query_map([board], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    // 每个状态的连线按工作流顺序排列
    let order: Vec<String> = v.iter().map(|s| s.key.clone()).collect();
    let mut v = v;
    for s in v.iter_mut() {
        s.to = order.iter().filter(|k| edges.iter().any(|(a, b)| a == &s.key && b == *k)).cloned().collect();
    }
    Ok(Flow(v))
}

pub(crate) fn insert_default_flow(conn: &Connection, board: i64) -> Result<()> {
    for (i, (key, name, color, claim, lock, fin)) in DEFAULT_FLOW.iter().enumerate() {
        conn.execute(
            "INSERT INTO statuses (board, key, name, color, sort, claim, lock, final) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![board, key, name, color, i as i64, *claim as i64, *lock as i64, *fin as i64],
        )?;
    }
    for (a, b) in DEFAULT_EDGES {
        conn.execute("INSERT INTO transitions (board, src, dst) VALUES (?1, ?2, ?3)", params![board, a, b])?;
    }
    Ok(())
}

/// 正常（不在回收站里）的卡片
pub(crate) fn cards_of(conn: &Connection, board: i64, only: Option<i64>) -> Result<Vec<Card>> {
    cards_query(conn, board, only, &[])
}

/// 同 cards_of，skip 里的卡片不取（在 SQL 里排除，免得为它们算子查询）。回收站里的卡片不取
fn cards_query(conn: &Connection, board: i64, only: Option<i64>, skip: &[i64]) -> Result<Vec<Card>> {
    // 只有给了 id 时才有 ?2（SQLite 里多出来的绑定参数也算错）
    let cond = if only.is_some() {
        "AND c.board = ?1 AND c.id = ?2 AND c.deleted_at IS NULL"
    } else {
        "AND c.board = ?1 AND ?2 IS NULL AND c.deleted_at IS NULL"
    };
    Ok(cards_from(conn, cond, params![board, only], skip)?.into_iter().map(|(c, _)| c).collect())
}

/// 卡片以及回收站的几列（删除时间、谁删的、删除前的状态和位置）；deleted_at 为 None 表示不在回收站里
struct TrashRow {
    deleted_at: Option<i64>,
    deleted_by: Actor,
    del_col: Option<String>,
    del_pos: Option<i64>,
    /// 删除前领取它的那个可领取状态
    del_from: Option<String>,
    /// 删除前是不是被持有
    del_held: bool,
}

/// 按条件读出卡片（不带 skip 时排除 skip 里的）。`cond` 接在 WHERE 后面，用参数 ?1…
fn cards_from(conn: &Connection, cond: &str, p: impl rusqlite::Params, skip: &[i64]) -> Result<Vec<(Card, TrashRow)>> {
    let skip = if skip.is_empty() {
        String::new()
    } else {
        format!("AND c.id NOT IN ({})", skip.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","))
    };
    let sql = format!(
        "SELECT c.id, c.title, c.body, c.col, c.holder, c.claimed_at, c.beat_at, c.created_at, c.updated_at,
                c.created_by_kind, c.created_by_name, c.claimed_from,
                (SELECT COUNT(*) FROM activity a WHERE a.board = c.board AND a.card = c.id AND a.kind = 'comment'),
                (SELECT GROUP_CONCAT(cl.label) FROM card_labels cl JOIN labels l ON l.id = cl.label
                   WHERE cl.board = c.board AND cl.card = c.id),
                (SELECT r.started_at FROM card_runs r WHERE r.board = c.board AND r.card = c.id AND r.ended_at IS NULL),
                (SELECT COALESCE(SUM(r.ended_at - r.started_at), 0) FROM card_runs r WHERE r.board = c.board AND r.card = c.id AND r.ended_at IS NOT NULL),
                (SELECT COUNT(*) FROM card_runs r WHERE r.board = c.board AND r.card = c.id),
                (SELECT COUNT(*) FROM card_runs r WHERE r.board = c.board AND r.card = c.id AND r.how IN ('lost', 'timeout')),
                (SELECT COALESCE(SUM(r.beats), 0) FROM card_runs r WHERE r.board = c.board AND r.card = c.id),
                (SELECT MAX(r.ended_at) FROM card_runs r WHERE r.board = c.board AND r.card = c.id),
                COALESCE(c.moved_at, c.created_at),
                c.seen_at,
                c.deleted_at, c.deleted_by_kind, c.deleted_by_name, c.del_col, c.del_pos, c.del_from, c.del_held,
                c.split_from
         FROM cards c WHERE 1 = 1 {cond} {skip} ORDER BY {COL_ORDER}, c.pos, c.id"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(p, |r| {
        let labels: Option<String> = r.get(13)?;
        let mut labels: Vec<i64> = labels
            .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
            .unwrap_or_default();
        labels.sort_unstable();
        let card = Card {
            id: r.get(0)?,
            title: r.get(1)?,
            body: r.get(2)?,
            column: r.get(3)?,
            holder: r.get(4)?,
            claimed_at: r.get(5)?,
            beat_at: r.get(6)?,
            claimed_from: r.get(11)?,
            run_started_at: r.get(14)?,
            work_ms: r.get(15)?,
            runs: r.get(16)?,
            runs_lost: r.get(17)?,
            beats: r.get(18)?,
            finished_at: r.get(19)?,
            created_at: r.get(7)?,
            updated_at: r.get(8)?,
            moved_at: r.get(20)?,
            created_by: row_actor(r.get(9)?, r.get(10)?),
            comments: r.get(12)?,
            labels,
            seen_at: r.get(21)?,
            split_from: r.get(29)?,
        };
        let by_kind: Option<String> = r.get(23)?;
        let row = TrashRow {
            deleted_at: r.get(22)?,
            deleted_by: row_actor(by_kind.unwrap_or_else(|| "human".into()), r.get::<_, Option<String>>(24)?.unwrap_or_default()),
            del_col: r.get(25)?,
            del_pos: r.get(26)?,
            del_from: r.get(27)?,
            del_held: r.get::<_, Option<bool>>(28)?.unwrap_or(false),
        };
        Ok((card, row))
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn events_query(conn: &Connection, tail: &str, p: impl rusqlite::Params) -> Result<Vec<Event>> {
    let sql = format!(
        "SELECT e.id, b.project || '/' || b.key, e.card, e.title, e.at, e.actor_kind, e.actor_name, e.op, e.text, e.via, e.client
         FROM events e JOIN boards b ON b.id = e.board {tail}"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(p, |r| {
        Ok(Event {
            id: r.get(0)?,
            board: r.get(1)?,
            card_id: r.get(2)?,
            title: r.get(3)?,
            at: r.get(4)?,
            actor: row_actor(r.get(5)?, r.get(6)?),
            op: r.get(7)?,
            text: r.get(8)?,
            via: Via::parse(&r.get::<_, String>(9)?),
            client: r.get(10)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/* ============================== 写 ============================== */

/// 写事务里的操作集合。卡片规则见 engine.rs
pub struct W<'a> {
    pub(crate) tx: &'a Transaction<'a>,
    pub(crate) now: i64,
    pub(crate) origin: Origin,
    pub(crate) changed: bool,
    /// 机器或派发有变化（另外把 meta.hosts 加一）
    pub(crate) hosts_changed: bool,
    /// 正在为派发领取：卡片的排队派发不挡住这次领取（见 hosts.rs）
    pub(crate) dispatch_ok: bool,
}

/// 新节点在画布上的默认位置：项目在左列，白板在项目右侧依次往下排
const LAYOUT_PROJECT_GAP: f64 = 320.0;
const LAYOUT_BOARD_X: f64 = 420.0;
const LAYOUT_BOARD_GAP: f64 = 200.0;

impl<'a> W<'a> {
    pub fn now(&self) -> i64 {
        self.now
    }

    pub(crate) fn collect<T>(
        &self,
        sql: &str,
        p: impl rusqlite::Params,
        f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let mut st = self.tx.prepare(sql)?;
        let rows = st.query_map(p, f)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn read(&self) -> R<'_> {
        R { conn: self.tx }
    }

    pub fn resolve(&self, raw: &str) -> Result<BoardRef> {
        resolve(self.tx, raw)
    }

    fn board_count(&self) -> Result<i64> {
        Ok(self.tx.query_row("SELECT COUNT(*) FROM boards", [], |r| r.get(0))?)
    }

    fn need_name(name: &str, what: &str) -> Result<String> {
        let n = name.trim();
        if n.is_empty() {
            return Err(BoardError::usage(format!("{what}名称不能为空")));
        }
        Ok(n.chars().take(60).collect())
    }

    fn check_color(color: Option<&str>, palette: &[&str], fallback: usize) -> Result<String> {
        match color {
            None => Ok(palette[fallback % palette.len()].to_string()),
            Some(c) => palette
                .iter()
                .find(|p| p.eq_ignore_ascii_case(c))
                .map(|p| p.to_string())
                .ok_or_else(|| BoardError::usage(format!("颜色必须取自色板：{}", palette.join(" ")))),
        }
    }

    /// 项目、标签的颜色：色板里的颜色，或取色器选的任意 #RRGGBB（统一成大写）；不给时按 fallback 取色板里的
    fn check_any_color(color: Option<&str>, palette: &[&str], fallback: usize) -> Result<String> {
        match color.map(str::trim) {
            Some(c) if c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit()) => Ok(c.to_ascii_uppercase()),
            Some(c) if !palette.iter().any(|p| p.eq_ignore_ascii_case(c)) => Err(BoardError::usage(format!("颜色要用色板里的颜色或 #RRGGBB：{c}"))),
            c => Self::check_color(c, palette, fallback),
        }
    }

    fn check_project_color(color: Option<&str>, fallback: usize) -> Result<String> {
        Self::check_any_color(color, &PROJECT_COLORS, fallback)
    }

    /* ---------------- 项目 ---------------- */

    pub fn create_project(
        &mut self,
        name: &str,
        key: &str,
        desc: &str,
        color: Option<&str>,
        pos: Option<Pos>,
        first: NewBoard,
    ) -> Result<String> {
        let name = Self::need_name(name, "项目")?;
        let key = key.trim();
        if !valid_key(key) {
            return Err(BoardError::usage("项目标识只能用小写字母、数字和 -，以字母或数字开头，最长 32 个字符"));
        }
        let taken: Option<String> =
            self.tx.query_row("SELECT key FROM projects WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        if taken.is_some() {
            return Err(BoardError::usage(format!("项目标识「{key}」已被占用")));
        }
        let (n, max_y, max_sort): (i64, Option<f64>, Option<i64>) = self.tx.query_row(
            "SELECT COUNT(*), MAX(pos_y), MAX(sort) FROM projects",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let color = Self::check_project_color(color, n as usize)?;
        let pos = pos.unwrap_or(Pos { x: 0.0, y: max_y.map(|y| y + LAYOUT_PROJECT_GAP).unwrap_or(0.0) });
        self.tx.execute(
            "INSERT INTO projects (key, name, description, color, pos_x, pos_y, sort, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![key, name, desc.trim(), color, pos.x, pos.y, max_sort.unwrap_or(-1) + 1, self.now],
        )?;
        self.changed = true;
        let first_pos = first.pos.or(Some(Pos { x: pos.x + LAYOUT_BOARD_X, y: pos.y - 10.0 }));
        self.create_board(key, &first.name, &first.key, &first.desc, first_pos)?;
        Ok(key.to_string())
    }

    pub fn update_project(&mut self, key: &str, name: Option<&str>, desc: Option<&str>, color: Option<&str>) -> Result<()> {
        self.project_exists(key)?;
        if let Some(n) = name {
            let n = Self::need_name(n, "项目")?;
            self.tx.execute("UPDATE projects SET name = ?1 WHERE key = ?2", params![n, key])?;
        }
        if let Some(d) = desc {
            self.tx.execute("UPDATE projects SET description = ?1 WHERE key = ?2", params![d.trim(), key])?;
        }
        if let Some(c) = color {
            let c = Self::check_project_color(Some(c), 0)?;
            self.tx.execute("UPDATE projects SET color = ?1 WHERE key = ?2", params![c, key])?;
        }
        self.changed = true;
        Ok(())
    }

    pub fn delete_project(&mut self, key: &str) -> Result<()> {
        self.project_exists(key)?;
        let others: i64 =
            self.tx.query_row("SELECT COUNT(*) FROM boards WHERE project != ?1", [key], |r| r.get(0))?;
        if others == 0 {
            return Err(BoardError::usage("整个工作区至少要保留一块白板"));
        }
        self.tx.execute("DELETE FROM projects WHERE key = ?1", [key])?;
        self.changed = true;
        Ok(())
    }

    fn project_exists(&self, key: &str) -> Result<()> {
        let found: Option<String> =
            self.tx.query_row("SELECT key FROM projects WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        match found {
            Some(_) => Ok(()),
            None => Err(BoardError::new(ErrorCode::BoardNotFound, format!("项目「{key}」不存在"))),
        }
    }

    /* ---------------- 白板 ---------------- */

    pub fn create_board(&mut self, project: &str, name: &str, key: &str, desc: &str, pos: Option<Pos>) -> Result<String> {
        self.project_exists(project)?;
        let name = Self::need_name(name, "白板")?;
        let key = key.trim();
        if !valid_key(key) {
            return Err(BoardError::usage("白板标识只能用小写字母、数字和 -，以字母或数字开头，最长 32 个字符"));
        }
        let taken: Option<i64> = self
            .tx
            .query_row("SELECT id FROM boards WHERE project = ?1 AND key = ?2", params![project, key], |r| r.get(0))
            .optional()?;
        if taken.is_some() {
            return Err(BoardError::usage(format!("「{project}」里已经有标识为「{key}」的白板")));
        }
        let (px, py): (f64, f64) =
            self.tx.query_row("SELECT pos_x, pos_y FROM projects WHERE key = ?1", [project], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let (max_y, max_sort): (Option<f64>, Option<i64>) = self.tx.query_row(
            "SELECT MAX(pos_y), MAX(sort) FROM boards WHERE project = ?1",
            [project],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let pos = pos.unwrap_or(Pos {
            x: px + LAYOUT_BOARD_X,
            y: max_y.map(|y| y + LAYOUT_BOARD_GAP).unwrap_or(py - 10.0),
        });
        self.tx.execute(
            "INSERT INTO boards (project, key, name, description, pos_x, pos_y, sort, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![project, key, name, desc.trim(), pos.x, pos.y, max_sort.unwrap_or(-1) + 1, self.now],
        )?;
        insert_default_flow(self.tx, self.tx.last_insert_rowid())?;
        self.changed = true;
        Ok(format!("{project}/{key}"))
    }

    pub fn update_board(&mut self, path: &str, name: Option<&str>, desc: Option<&str>, readme: Option<&str>) -> Result<()> {
        let b = self.resolve_exact(path)?;
        if let Some(t) = readme {
            self.tx.execute("UPDATE boards SET readme = ?1 WHERE id = ?2", params![t.trim_end(), b.id])?;
        }
        if let Some(n) = name {
            let n = Self::need_name(n, "白板")?;
            self.tx.execute("UPDATE boards SET name = ?1 WHERE id = ?2", params![n, b.id])?;
        }
        if let Some(d) = desc {
            self.tx.execute("UPDATE boards SET description = ?1 WHERE id = ?2", params![d.trim(), b.id])?;
        }
        self.changed = true;
        Ok(())
    }

    /// 白板的环境检查（PRD §9.1）：一行命令，最长 500 个字符；None 表示这一项不改，空字符串表示不检查
    pub fn set_ready_check(&mut self, path: &str, cmd: Option<&str>, win: Option<&str>) -> Result<()> {
        let b = self.resolve_exact(path)?;
        let clean = |t: &str| -> Result<String> {
            let t = t.trim();
            if t.contains('\n') || t.contains('\r') {
                return Err(BoardError::usage("环境检查只能写一行：几条命令用 && 连起来"));
            }
            if t.chars().count() > READY_CHECK_MAX {
                return Err(BoardError::usage(format!("环境检查最长 {READY_CHECK_MAX} 个字符")));
            }
            Ok(t.to_string())
        };
        if let Some(t) = cmd {
            self.tx.execute("UPDATE boards SET ready_check = ?1 WHERE id = ?2", params![clean(t)?, b.id])?;
        }
        if let Some(t) = win {
            self.tx.execute("UPDATE boards SET ready_check_win = ?1 WHERE id = ?2", params![clean(t)?, b.id])?;
        }
        self.changed = true;
        // 机器拿到的检查命令变了：board host 收到推送后马上重新检查
        self.hosts_changed = true;
        Ok(())
    }

    /// 白板的代码仓库地址和集成分支（PRD §4）：都是可选的两项，None 表示这一项不改，空字符串表示清空
    pub fn set_repo_branch(&mut self, path: &str, repo: Option<&str>, branch: Option<&str>) -> Result<()> {
        let b = self.resolve_exact(path)?;
        let clean = |t: &str, max: usize, what: &str| -> Result<String> {
            let t = t.trim();
            if t.contains('\n') || t.contains('\r') {
                return Err(BoardError::usage(format!("{what}只能写一行")));
            }
            if t.chars().count() > max {
                return Err(BoardError::usage(format!("{what}最长 {max} 个字符")));
            }
            Ok(t.to_string())
        };
        if let Some(t) = repo {
            self.tx.execute("UPDATE boards SET repo = ?1 WHERE id = ?2", params![clean(t, REPO_MAX, "仓库地址")?, b.id])?;
        }
        if let Some(t) = branch {
            let t = clean(t, BRANCH_MAX, "集成分支")?;
            if t.chars().any(|c| c.is_whitespace() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')) {
                return Err(BoardError::usage("集成分支只能写分支名，不能有空格和 ~ ^ : ? * [ \\ 这些字符"));
            }
            self.tx.execute("UPDATE boards SET branch = ?1 WHERE id = ?2", params![t, b.id])?;
        }
        self.changed = true;
        Ok(())
    }

    pub fn delete_board(&mut self, path: &str) -> Result<()> {
        let b = self.resolve_exact(path)?;
        if self.board_count()? <= 1 {
            return Err(BoardError::usage("整个工作区至少要保留一块白板"));
        }
        self.tx.execute("DELETE FROM boards WHERE id = ?1", [b.id])?;
        self.changed = true;
        Ok(())
    }

    /// 界面上的白板操作必须写全路径
    pub(crate) fn resolve_exact(&self, path: &str) -> Result<BoardRef> {
        if !path.contains('/') {
            return Err(BoardError::new(ErrorCode::BoardNotFound, format!("白板「{path}」不存在")));
        }
        self.resolve(path)
    }

    /// 画布上拖动节点：只保存位置
    pub fn set_pos(&mut self, kind: &str, id: &str, pos: Pos) -> Result<()> {
        let (x, y) = (pos.x.round(), pos.y.round());
        let n = match kind {
            "project" => self.tx.execute("UPDATE projects SET pos_x = ?1, pos_y = ?2 WHERE key = ?3", params![x, y, id])?,
            "board" => {
                let b = self.resolve_exact(id)?;
                self.tx.execute("UPDATE boards SET pos_x = ?1, pos_y = ?2 WHERE id = ?3", params![x, y, b.id])?
            }
            _ => return Err(BoardError::usage(format!("未知的节点类型「{kind}」"))),
        };
        if n == 0 {
            return Err(BoardError::new(ErrorCode::BoardNotFound, format!("「{id}」不存在")));
        }
        self.changed = true;
        Ok(())
    }

    /* ---------------- 标签（属于项目，项目下所有白板共用） ---------------- */

    fn label_name_taken(&self, project: &str, name: &str, except: Option<i64>) -> Result<bool> {
        let lower = name.to_lowercase();
        let names: Vec<(i64, String)> =
            self.collect("SELECT id, name FROM labels WHERE project = ?1", [project], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(names.iter().any(|(id, n)| Some(*id) != except && n.to_lowercase() == lower))
    }

    pub fn create_label(&mut self, project: &str, name: &str, color: Option<&str>, human_only: bool) -> Result<Label> {
        self.project_exists(project)?;
        let name: String = name.trim().chars().take(24).collect();
        if name.is_empty() {
            return Err(BoardError::usage("标签名称不能为空"));
        }
        if name.starts_with('+') || name.starts_with('-') {
            return Err(BoardError::usage("标签名称不能以 + 或 - 开头"));
        }
        if self.label_name_taken(project, &name, None)? {
            return Err(BoardError::usage(format!("标签「{name}」已存在")));
        }
        let (n, max_sort): (i64, Option<i64>) = self.tx.query_row(
            "SELECT COUNT(*), MAX(sort) FROM labels WHERE project = ?1",
            [project],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let color = Self::check_any_color(color, &LABEL_COLORS, n as usize)?;
        self.tx.execute(
            "INSERT INTO labels (project, name, color, sort, human_only) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project, name, color, max_sort.unwrap_or(-1) + 1, human_only],
        )?;
        self.changed = true;
        Ok(Label { id: self.tx.last_insert_rowid(), name, color, uses: 0, human_only, priority: false })
    }

    pub fn update_label(
        &mut self,
        project: &str,
        id: i64,
        name: Option<&str>,
        color: Option<&str>,
        human_only: Option<bool>,
        priority: Option<bool>,
    ) -> Result<()> {
        self.label_exists(project, id)?;
        if let Some(n) = name {
            let n: String = n.trim().chars().take(24).collect();
            if n.is_empty() {
                return Err(BoardError::usage("标签名称不能为空"));
            }
            if n.starts_with('+') || n.starts_with('-') {
                return Err(BoardError::usage("标签名称不能以 + 或 - 开头"));
            }
            if self.label_name_taken(project, &n, Some(id))? {
                return Err(BoardError::usage(format!("标签「{n}」已存在")));
            }
            self.tx.execute("UPDATE labels SET name = ?1 WHERE id = ?2", params![n, id])?;
        }
        if let Some(c) = color {
            let c = Self::check_any_color(Some(c), &LABEL_COLORS, 0)?;
            self.tx.execute("UPDATE labels SET color = ?1 WHERE id = ?2", params![c, id])?;
        }
        if let Some(h) = human_only {
            self.tx.execute("UPDATE labels SET human_only = ?1 WHERE id = ?2", params![h, id])?;
        }
        if let Some(v) = priority {
            self.tx.execute("UPDATE labels SET priority = ?1 WHERE id = ?2", params![v, id])?;
        }
        self.changed = true;
        Ok(())
    }

    /// 按给定顺序重排标签（界面上拖拽排序）；`ids` 里的标签必须都属于这个项目。
    ///
    /// 没出现在 `ids` 里的标签按原来的顺序补在后面（新建的标签因此总在最后），`sort` 依次写成 0,1,2…
    /// 任何一个 id 不合法都整体失败、不改动任何数据（写操作在同一个事务里）。
    pub fn reorder_labels(&mut self, project: &str, ids: &[i64]) -> Result<()> {
        self.project_exists(project)?;
        let mut rest: Vec<i64> = self.collect("SELECT id FROM labels WHERE project = ?1 ORDER BY sort, id", [project], |r| r.get(0))?;
        for (i, id) in ids.iter().enumerate() {
            if ids[..i].contains(id) {
                return Err(BoardError::usage(format!("标签 id {id} 在顺序里出现了两次")));
            }
            match rest.iter().position(|x| x == id) {
                Some(p) => {
                    rest.remove(p);
                }
                None => {
                    let other: Option<String> =
                        self.tx.query_row("SELECT project FROM labels WHERE id = ?1", [id], |r| r.get(0)).optional()?;
                    return Err(match other {
                        Some(p) => BoardError::usage(format!("标签 id {id} 属于项目「{p}」，不能排到「{project}」里")),
                        None => BoardError::usage(format!("标签 id {id} 不存在")),
                    });
                }
            }
        }
        for (i, id) in ids.iter().chain(rest.iter()).enumerate() {
            self.tx.execute("UPDATE labels SET sort = ?1 WHERE id = ?2", params![i as i64, id])?;
        }
        self.changed = true;
        Ok(())
    }

    /// 删除标签：它会从项目内所有卡片上移除（外键级联）
    pub fn delete_label(&mut self, project: &str, id: i64) -> Result<()> {
        self.label_exists(project, id)?;
        self.tx.execute("DELETE FROM labels WHERE id = ?1", [id])?;
        self.changed = true;
        Ok(())
    }

    fn label_exists(&self, project: &str, id: i64) -> Result<()> {
        let found: Option<i64> = self
            .tx
            .query_row("SELECT id FROM labels WHERE id = ?1 AND project = ?2", params![id, project], |r| r.get(0))
            .optional()?;
        found.map(|_| ()).ok_or_else(|| BoardError::usage(format!("项目「{project}」里没有这个标签")))
    }

    /* ---------------- 动态 / 事件 ---------------- */

    /// 在卡片动态里追加一条，并写入全局事件流
    pub(crate) fn track(&mut self, board: i64, card: i64, who: &Actor, action: &str, text: &str) -> Result<()> {
        let kind = if action == "comment" { "comment" } else { "system" };
        self.tx.execute(
            "INSERT INTO activity (board, card, at, actor_kind, actor_name, kind, action, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![board, card, self.now, who.kind.as_str(), who.name, kind, action, text],
        )?;
        self.tx.execute("UPDATE cards SET updated_at = ?1 WHERE board = ?2 AND id = ?3", params![self.now, board, card])?;
        // 回收站里的卡片也记动态（删除、永久删除）；卡片已经不在了（永久删除之后）时标题为空
        let title: String = self
            .tx
            .query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2", params![board, card], |r| r.get(0))
            .unwrap_or_default();
        self.event(board, card, &title, who, action, text)
    }

    pub(crate) fn event(&mut self, board: i64, card: i64, title: &str, who: &Actor, op: &str, text: &str) -> Result<()> {
        let system = who.kind == ActorKind::System;
        let via = if system { Via::System } else { self.origin.via };
        let client = if system { None } else { self.origin.client.as_deref() };
        self.tx.execute(
            "INSERT INTO events (board, card, title, at, actor_kind, actor_name, op, text, via, client)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![board, card, title, self.now, who.kind.as_str(), who.name, op, text, via.as_str(), client],
        )?;
        let id = self.tx.last_insert_rowid();
        if id % 100 == 0 {
            self.tx.execute(
                "DELETE FROM events WHERE board = ?1 AND id <= (SELECT id FROM events WHERE board = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
                params![board, EVENTS_KEEP],
            )?;
        }
        self.changed = true;
        Ok(())
    }
}
