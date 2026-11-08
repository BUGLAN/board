//! agent 名字（PRD §3）。
//!
//! - 名字表只增不删：名字、首次 check 时间、最近一次心跳、令牌 ID，以及离线时间（在线时为空）。
//! - 在线：最近一次心跳在 50 秒以内（每 10 秒一次，连续 5 次没收到算离线），并且没有主动下线。
//! - `board check --as <名字>`：名字正被在线 agent 使用时 `NAME_TAKEN`；否则标为在线，心跳时间记为现在。
//!   查重和占用在同一个写事务里完成，同时 check 同一个名字只有一个成功。
//! - `board beat`（agent 级心跳）：更新名字的心跳时间，并对这个名字持有、还没 final 的每张卡片
//!   做一次与 `board beat <ID>` 相同的心跳。名字已离线时 `NAME_OFFLINE`。
//! - `board beat --final`（不带 ID）：名字立刻离线。它持有的卡片不再收到心跳，按规则 7 在 50 秒内释放。
//! - 严格模式：带令牌的卡片命令、上传附件里的名字必须在线（[`R::require_online`]）。
//! - 只有上线、下线算变更（推送给界面）；只更新心跳时间的 `board beat` 不推送。

use crate::error::{BoardError, ErrorCode, Result};
use crate::model::*;
use crate::store::{cards_of, R, W};
use rusqlite::{params, OptionalExtension};
use serde_json::json;

pub(crate) const SCHEMA: &str = r#"
-- agent 名字（PRD §3）：只增不删。在线 = off_at 为空，并且最近一次心跳在 50 秒以内
CREATE TABLE IF NOT EXISTS agents (
  name       TEXT PRIMARY KEY,
  checked_at INTEGER NOT NULL, -- 首次 check 的时间
  beat_at    INTEGER NOT NULL, -- 最近一次心跳（check 也算一次）
  token      INTEGER,          -- 最近一次 check 用的访问令牌 ID
  off_at     INTEGER           -- 离线的时间：board beat --final，或扫描发现心跳中断；在线时为空
);
"#;

/// 名字多久没有心跳算离线：连续 5 次（50 秒）
pub const NAME_GAP_MS: i64 = BEAT_MS * BEAT_MISSES;

/// 名字表里的一行：(最近一次心跳, 离线时间)
type Row = (i64, Option<i64>);

fn online(row: Option<Row>, now: i64) -> bool {
    matches!(row, Some((beat, None)) if beat + NAME_GAP_MS > now)
}

fn offline(name: &str) -> BoardError {
    BoardError::new(
        ErrorCode::NameOffline,
        format!("名字「{name}」不在线：没有 check，或心跳中断超过 {} 秒，先 board check --as {name}", NAME_GAP_MS / 1000),
    )
    .with_detail(json!({ "name": name }))
}

impl<'a> R<'a> {
    fn agent_row(&self, name: &str) -> Result<Option<Row>> {
        Ok(self
            .conn
            .query_row("SELECT beat_at, off_at FROM agents WHERE name = ?1", [name], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
    }

    /// 严格模式：带令牌的卡片命令、上传附件之前，名字必须在线，否则 `NAME_OFFLINE`
    pub fn require_online(&self, name: &str, now: i64) -> Result<()> {
        if online(self.agent_row(name)?, now) {
            Ok(())
        } else {
            Err(offline(name))
        }
    }

    /// 一个名字的情况（不管在不在线），以及它持有的卡片；没有 check 过时为空
    pub fn agent(&self, name: &str) -> Result<Option<AgentInfo>> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row("SELECT checked_at, beat_at FROM agents WHERE name = ?1", [name], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((checked_at, beat_at)) = row else { return Ok(None) };
        Ok(Some(AgentInfo { name: name.to_string(), checked_at, beat_at, cards: self.held(name)? }))
    }

    /// 在线的 agent（所有白板），按名字排序
    pub fn agents(&self, now: i64) -> Result<Vec<AgentInfo>> {
        let mut st = self.conn.prepare(
            "SELECT name, checked_at, beat_at FROM agents WHERE off_at IS NULL AND beat_at + ?2 > ?1 ORDER BY name",
        )?;
        let rows: Vec<(String, i64, i64)> =
            st.query_map(params![now, NAME_GAP_MS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
        rows.into_iter()
            .map(|(name, checked_at, beat_at)| Ok(AgentInfo { cards: self.held(&name)?, name, checked_at, beat_at }))
            .collect()
    }

    /// 名字持有的卡片（按项目、白板的顺序）
    fn held(&self, name: &str) -> Result<Vec<HeldCard>> {
        let mut st = self.conn.prepare(
            "SELECT c.board, b.project || '/' || b.key, c.id FROM cards c
             JOIN boards b ON b.id = c.board
             JOIN projects p ON p.key = b.project
             WHERE c.holder = ?1 ORDER BY p.sort, p.created_at, b.sort, b.created_at, c.id",
        )?;
        let rows: Vec<(i64, String, i64)> =
            st.query_map([name], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
        let mut out = vec![];
        for (b, path, id) in rows {
            if let Some(card) = cards_of(self.conn, b, Some(id))?.pop() {
                out.push(HeldCard { board: path, card });
            }
        }
        Ok(out)
    }
}

impl<'a> W<'a> {
    /// `board check --as <名字>`：格式不对 `USAGE`；正被在线 agent 使用 `NAME_TAKEN`；
    /// 否则标为在线，心跳时间记为现在（首次 check 时间保留第一次的）
    pub fn agent_check(&mut self, name: &str, token: Option<i64>) -> Result<AgentInfo> {
        check_agent_name(name).map_err(BoardError::usage)?;
        if online(self.read().agent_row(name)?, self.now) {
            return Err(BoardError::new(
                ErrorCode::NameTaken,
                format!("名字「{name}」正被在线 agent 使用：加一个和本会话相关的后缀（例如任务名或分支名）再 check，例如 {name}-<后缀>"),
            )
            .with_detail(json!({ "name": name })));
        }
        self.tx.execute(
            "INSERT INTO agents (name, checked_at, beat_at, token, off_at) VALUES (?1, ?2, ?2, ?3, NULL)
             ON CONFLICT (name) DO UPDATE SET beat_at = excluded.beat_at, token = excluded.token, off_at = NULL",
            params![name, self.now, token],
        )?;
        self.changed = true;
        self.read().agent(name)?.ok_or_else(|| offline(name))
    }

    /// agent 级心跳 `board beat`：更新名字的心跳时间；对这个名字持有、还没 final 的每张卡片做一次心跳
    /// （与 `board beat <ID>` 相同）。名字不在线时 `NAME_OFFLINE`。
    ///
    /// `done`（`board beat --final`）：名字立刻离线；已经离线时什么也不做。返回名字的情况和它持有的卡片
    pub fn agent_beat(&mut self, name: &str, done: bool) -> Result<AgentInfo> {
        let row = self.read().agent_row(name)?;
        let is_online = online(row, self.now);
        if done {
            if row.is_none() {
                return Err(offline(name));
            }
            if is_online {
                self.tx.execute("UPDATE agents SET off_at = ?2 WHERE name = ?1", params![name, self.now])?;
                self.changed = true;
            }
            return self.read().agent(name)?.ok_or_else(|| offline(name));
        }
        if !is_online {
            return Err(offline(name));
        }
        self.tx.execute("UPDATE agents SET beat_at = ?2 WHERE name = ?1", params![name, self.now])?;
        let boards: Vec<i64> =
            self.collect("SELECT DISTINCT board FROM cards WHERE holder = ?1 AND final_at IS NULL", [name], |r| r.get(0))?;
        let actor = Actor::agent(name);
        for b in boards {
            // 先释放这块白板上已经到期的领取（规则 7），再给还持有的卡片发心跳
            self.sweep_board(b)?;
            let ids: Vec<i64> = self.collect(
                "SELECT id FROM cards WHERE board = ?1 AND holder = ?2 AND final_at IS NULL",
                params![b, name],
                |r| r.get(0),
            )?;
            for id in ids {
                self.beat_card(b, id, &actor)?;
            }
        }
        self.read().agent(name)?.ok_or_else(|| offline(name))
    }

    /// 心跳中断超过 50 秒的名字标为离线（服务端每秒扫描一次）。返回离线的个数
    pub(crate) fn sweep_agents(&mut self) -> Result<usize> {
        let n = self.tx.execute(
            "UPDATE agents SET off_at = beat_at + ?2 WHERE off_at IS NULL AND beat_at + ?2 <= ?1",
            params![self.now, NAME_GAP_MS],
        )?;
        if n > 0 {
            self.changed = true;
        }
        Ok(n)
    }
}
