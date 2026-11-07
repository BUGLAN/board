//! SQLite 存储。
//!
//! - 网页端、桌面端、CLI 都直接读写同一个数据库文件（默认 `~/.board/board.db`，`BOARD_DB` 可覆盖）。
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
INSERT OR IGNORE INTO meta (key, value) VALUES ('rev', 0), ('schema', 1);

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
  UNIQUE (project, key)
);

CREATE TABLE IF NOT EXISTS labels (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES projects(key) ON DELETE CASCADE,
  name    TEXT NOT NULL,
  color   TEXT NOT NULL,
  sort    INTEGER NOT NULL DEFAULT 0
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
  expires_at      INTEGER,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  created_by_kind TEXT NOT NULL,
  created_by_name TEXT NOT NULL,
  PRIMARY KEY (board, id)
);
CREATE INDEX IF NOT EXISTS cards_col ON cards (board, col, pos);

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

-- 每块白板的工作流（PRD §7）。lx / lw / ly 是状态块的布局，空表示用默认布局
CREATE TABLE IF NOT EXISTS statuses (
  board INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  key   TEXT NOT NULL,
  name  TEXT NOT NULL,
  color TEXT NOT NULL,
  sort  INTEGER NOT NULL,
  claim INTEGER NOT NULL DEFAULT 0,
  lock  INTEGER NOT NULL DEFAULT 0,
  final INTEGER NOT NULL DEFAULT 0,
  lx    INTEGER,
  lw    INTEGER,
  ly    REAL,
  PRIMARY KEY (board, key)
);
"#;

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

pub(crate) fn split_path(raw: &str) -> (&str, Option<&str>) {
    match raw.find('/') {
        Some(i) => (&raw[..i], Some(&raw[i + 1..])),
        None => (raw, None),
    }
}

pub struct Store {
    conn: Connection,
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
        Store::setup(conn)
    }

    pub fn open_in_memory() -> Result<Store> {
        Store::setup(Connection::open_in_memory()?)
    }

    fn setup(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        // 旧数据库里的白板没有工作流：补上默认的四个状态
        let missing: Vec<i64> = conn
            .prepare("SELECT id FROM boards b WHERE NOT EXISTS (SELECT 1 FROM statuses s WHERE s.board = b.id)")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for b in missing {
            insert_default_flow(&conn, b)?;
        }
        Ok(Store { conn })
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
        let mut w = W { tx: &tx, now, origin, changed: false };
        let out = f(&mut w)?;
        let changed = w.changed;
        if changed {
            tx.execute("UPDATE meta SET value = value + 1 WHERE key = 'rev'", [])?;
        }
        tx.commit()?;
        Ok(out)
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

    /// 规则 7：扫描所有白板，释放到期的领取。返回释放的张数
    pub fn sweep(&mut self, now: i64) -> Result<usize> {
        // 先用只读查询判断，避免没有到期卡片时也抢写锁
        let due: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM cards WHERE holder IS NOT NULL AND expires_at IS NOT NULL AND expires_at <= ?1",
            [now],
            |r| r.get(0),
        )?;
        if due == 0 {
            return Ok(0);
        }
        self.write(now, Origin::system(), |w| {
            let boards: Vec<i64> = w.collect(
                "SELECT DISTINCT board FROM cards WHERE holder IS NOT NULL AND expires_at IS NOT NULL AND expires_at <= ?1",
                params![now],
                |r| r.get(0),
            )?;
            let mut n = 0;
            for b in boards {
                n += w.sweep_board(b)?;
            }
            Ok(n)
        })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

/* ============================== 读 ============================== */

pub struct R<'a> {
    conn: &'a Connection,
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
                        COALESCE((SELECT MAX(at) FROM events e WHERE e.board = b.id), b.created_at)
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
                        pos: Pos { x: r.get(4)?, y: r.get(5)? },
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                        counts: Counts::new(),
                        statuses: vec![],
                        holders: vec![],
                    },
                ))
            })?;
            for row in rows {
                let (id, mut b) = row?;
                b.path = format!("{}/{}", p.key, b.key);
                b.statuses = flow_of(conn, id)?.0.into_iter().map(|s| Status { layout: None, ..s }).collect();
                b.counts.insert(INBOX.to_string(), 0);
                for s in &b.statuses {
                    b.counts.insert(s.key.clone(), 0);
                }
                let mut cs = conn.prepare("SELECT col, COUNT(*) FROM cards WHERE board = ?1 GROUP BY col")?;
                for c in cs.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                    let (col, n) = c?;
                    *b.counts.entry(col).or_default() += n;
                }
                let mut hs = conn.prepare(&format!(
                    "SELECT holder FROM cards c WHERE board = ?1 AND holder IS NOT NULL ORDER BY {}, pos",
                    COL_ORDER
                ))?;
                for h in hs.query_map([id], |r| r.get::<_, String>(0))? {
                    b.holders.push(h?);
                }
                p.boards.push(b);
            }
        }
        Ok(Workspace { rev: self.rev()?, now, projects })
    }

    pub fn labels(&self, project: &str) -> Result<Vec<Label>> {
        labels_of(self.conn, project)
    }

    /// 白板的工作流（按顺序，含布局）
    pub fn flow(&self, board: &BoardRef) -> Result<Flow> {
        flow_of(self.conn, board.id)
    }

    pub fn board_snapshot(&self, board: &BoardRef, now: i64) -> Result<BoardSnapshot> {
        let flow = flow_of(self.conn, board.id)?;
        let mut cols = Cols::new();
        cols.insert(INBOX.to_string(), vec![]);
        for s in &flow.0 {
            cols.insert(s.key.clone(), vec![]);
        }
        let mut cards = BTreeMap::new();
        for c in cards_of(self.conn, board.id, None)? {
            cols.entry(c.column.clone()).or_default().push(c.id);
            cards.insert(c.id, c);
        }
        let mut log = events_query(
            self.conn,
            "WHERE e.board = ?1 ORDER BY e.id DESC LIMIT 30",
            params![board.id],
        )?;
        log.reverse();
        Ok(BoardSnapshot { rev: self.rev()?, now, path: board.path(), statuses: flow.0, cols, cards, log })
    }

    /// 按列和列内顺序列出卡片
    pub fn cards(&self, board: &BoardRef) -> Result<Vec<Card>> {
        cards_of(self.conn, board.id, None)
    }

    pub fn card(&self, board: &BoardRef, id: i64) -> Result<CardDetail> {
        let card = cards_of(self.conn, board.id, Some(id))?
            .pop()
            .ok_or_else(|| not_found(id))?;
        let mut st = self.conn.prepare(
            "SELECT id, at, actor_kind, actor_name, kind, action, text FROM activity
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
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(CardDetail { card, activity })
    }

    /// id 之后的全局事件（服务端推给界面）
    pub fn events_since(&self, after: i64, limit: i64) -> Result<Vec<Event>> {
        events_query(self.conn, "WHERE e.id > ?1 ORDER BY e.id LIMIT ?2", params![after, limit])
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
        "SELECT l.id, l.name, l.color, (SELECT COUNT(*) FROM card_labels cl WHERE cl.label = l.id)
         FROM labels l WHERE l.project = ?1 ORDER BY l.sort, l.id",
    )?;
    let out = st
        .query_map([project], |r| Ok(Label { id: r.get(0)?, name: r.get(1)?, color: r.get(2)?, uses: r.get(3)? }))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

/// 卡片按「未整理」→ 工作流顺序排列（c 是 cards 的别名）
const COL_ORDER: &str = "CASE WHEN c.col = 'inbox' THEN -1 ELSE \
     COALESCE((SELECT s.sort FROM statuses s WHERE s.board = c.board AND s.key = c.col), 1000000) END";

pub(crate) fn flow_of(conn: &Connection, board: i64) -> Result<Flow> {
    let mut st = conn.prepare_cached(
        "SELECT key, name, color, claim, lock, final, lx, lw, ly FROM statuses WHERE board = ?1 ORDER BY sort, key",
    )?;
    let v = st
        .query_map([board], |r| {
            let (lx, lw, ly): (Option<i64>, Option<i64>, Option<f64>) = (r.get(6)?, r.get(7)?, r.get(8)?);
            Ok(Status {
                key: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                claim: r.get::<_, i64>(3)? != 0,
                lock: r.get::<_, i64>(4)? != 0,
                fin: r.get::<_, i64>(5)? != 0,
                layout: match (lx, lw, ly) {
                    (Some(x), Some(w), Some(y)) => Some(Tile { x, w, y }),
                    _ => None,
                },
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if v.is_empty() {
        return Err(BoardError::new(ErrorCode::Internal, "白板没有工作流"));
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
    Ok(())
}

fn cards_of(conn: &Connection, board: i64, only: Option<i64>) -> Result<Vec<Card>> {
    let sql = format!(
        "SELECT c.id, c.title, c.body, c.col, c.holder, c.claimed_at, c.expires_at, c.created_at, c.updated_at,
                c.created_by_kind, c.created_by_name,
                (SELECT COUNT(*) FROM activity a WHERE a.board = c.board AND a.card = c.id AND a.kind = 'comment'),
                (SELECT GROUP_CONCAT(cl.label) FROM card_labels cl JOIN labels l ON l.id = cl.label
                   WHERE cl.board = c.board AND cl.card = c.id)
         FROM cards c WHERE c.board = ?1 {} ORDER BY {COL_ORDER}, c.pos",
        if only.is_some() { "AND c.id = ?2" } else { "AND ?2 IS NULL" }
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(params![board, only], |r| {
        let col: String = r.get(3)?;
        let labels: Option<String> = r.get(12)?;
        let mut labels: Vec<i64> = labels
            .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
            .unwrap_or_default();
        labels.sort_unstable();
        Ok(Card {
            id: r.get(0)?,
            title: r.get(1)?,
            body: r.get(2)?,
            column: col,
            holder: r.get(4)?,
            claimed_at: r.get(5)?,
            expires_at: r.get(6)?,
            created_at: r.get(7)?,
            updated_at: r.get(8)?,
            created_by: row_actor(r.get(9)?, r.get(10)?),
            comments: r.get(11)?,
            labels,
        })
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
        let color = Self::check_color(color, &PROJECT_COLORS, n as usize)?;
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
            let c = Self::check_color(Some(c), &PROJECT_COLORS, 0)?;
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

    pub fn update_board(&mut self, path: &str, name: Option<&str>, desc: Option<&str>) -> Result<()> {
        let b = self.resolve_exact(path)?;
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

    pub fn create_label(&mut self, project: &str, name: &str, color: Option<&str>) -> Result<Label> {
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
        let color = Self::check_color(color, &LABEL_COLORS, n as usize)?;
        self.tx.execute(
            "INSERT INTO labels (project, name, color, sort) VALUES (?1, ?2, ?3, ?4)",
            params![project, name, color, max_sort.unwrap_or(-1) + 1],
        )?;
        self.changed = true;
        Ok(Label { id: self.tx.last_insert_rowid(), name, color, uses: 0 })
    }

    pub fn update_label(&mut self, project: &str, id: i64, name: Option<&str>, color: Option<&str>) -> Result<()> {
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
            let c = Self::check_color(Some(c), &LABEL_COLORS, 0)?;
            self.tx.execute("UPDATE labels SET color = ?1 WHERE id = ?2", params![c, id])?;
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
        let title: String =
            self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2", params![board, card], |r| r.get(0))?;
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
