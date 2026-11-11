//! 卡片流转记录（大盘需求「数据」）。
//!
//! 卡片每换一次状态写一行 `moves`，与写动态（`activity`）在同一个事务里：
//! 人或 agent 移动（`move`，含整理、删除状态时卡片被移走）、领取（`claim`）、人强制释放（`release`）、
//! 心跳中断或超时后回到可领取状态（`lost` / `timeout`）。`kind` 就是那条动态的 `action`。
//! 状态的 key 和移动当时的名称都记下来：状态之后改名、删除，统计仍能按当时的名称显示。
//!
//! 旧数据库没有这张表：第一次打开时从动态的文字里解析回填（只跑一次，`meta.moves` 记版本）。

use crate::error::Result;
use crate::model::{Actor, Flow, INBOX, INBOX_NAME};
use crate::store::W;
use rusqlite::{params, Connection};
use std::collections::HashMap;

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS moves (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  board      INTEGER NOT NULL,
  card       INTEGER NOT NULL,
  at         INTEGER NOT NULL,
  -- move 移动 | claim 领取 | release 人强制释放 | lost 心跳中断 | timeout 领取后没有心跳、final 后没有交审
  kind       TEXT NOT NULL,
  -- 状态 key：回填时按名称找不到现有状态（改过名、删掉了）填 NULL
  from_key   TEXT,
  to_key     TEXT,
  -- 移动当时的状态名称；回填的释放、超时从上一条记录推出原来的状态，推不出时为 NULL
  from_name  TEXT,
  to_name    TEXT NOT NULL,
  actor_kind TEXT NOT NULL,
  actor_name TEXT NOT NULL,
  FOREIGN KEY (board, card) REFERENCES cards(board, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS moves_board ON moves (board, at);
CREATE INDEX IF NOT EXISTS activity_at ON activity (board, at);
CREATE INDEX IF NOT EXISTS card_runs_start ON card_runs (board, started_at);
"#;

/// 回填的版本：`meta.moves` 小于它时回填一次
const BACKFILL_VERSION: i64 = 1;

impl W<'_> {
    /// 记一次换状态（调用方在同一个事务里写动态）
    pub(crate) fn log_move(&mut self, b: i64, card: i64, kind: &str, from: &str, to: &str, who: &Actor, flow: &Flow) -> Result<()> {
        self.tx.execute(
            "INSERT INTO moves (board, card, at, kind, from_key, to_key, from_name, to_name, actor_kind, actor_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![b, card, self.now, kind, from, to, flow.name(from), flow.name(to), who.kind.as_str(), who.name],
        )?;
        Ok(())
    }
}

/// 「」里的文字：marker 之后到下一个「」」为止
fn quoted_after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let i = text.find(marker)? + marker.len();
    let rest = &text[i..];
    rest.find('」').map(|j| &rest[..j])
}

/// 从一条动态解析出（原来的状态名称, 去往的状态名称）；原来的状态写在文字里没有时为 None
pub(crate) fn parse(action: &str, text: &str) -> Option<(Option<String>, String)> {
    let pair = |a: &str, b: &str| Some((Some(quoted_after(text, a)?.to_string()), quoted_after(text, b)?.to_string()));
    match action {
        // 状态「A」被删除，卡片移到「B」
        "move" if text.starts_with("状态「") => pair("状态「", "移到「"),
        // 从「A」移到「B」（被释放时后面跟「，领取已释放…」）、整理好了，从「未整理」移到「B」
        "move" => pair("从「", "移到「"),
        // 领取了卡片，从「A」进入「B」
        "claim" => pair("从「", "进入「"),
        // 强制释放了 X 的领取，卡片回到「B」最上面；心跳中断……卡片回到「B」最上面
        "release" | "lost" | "timeout" => Some((None, quoted_after(text, "回到「")?.to_string())),
        _ => None,
    }
}

/// 旧数据库：从动态回填流转记录（只跑一次）
pub(crate) fn backfill(conn: &Connection) -> Result<()> {
    let done: i64 = conn.query_row("SELECT COALESCE((SELECT value FROM meta WHERE key = 'moves'), 0)", [], |r| r.get(0))?;
    if done >= BACKFILL_VERSION {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    let any: i64 = tx.query_row("SELECT COUNT(*) FROM moves", [], |r| r.get(0))?;
    if any == 0 {
        // 每块白板现在的状态：名称 → key
        let mut names: HashMap<i64, HashMap<String, String>> = HashMap::new();
        {
            let mut st = tx.prepare("SELECT board, key, name FROM statuses ORDER BY board, sort, key")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            for row in rows {
                let (b, k, n) = row?;
                names.entry(b).or_default().entry(n).or_insert(k);
            }
        }
        let key_of = |b: i64, name: &str| -> Option<String> {
            if name == INBOX_NAME {
                return Some(INBOX.to_string());
            }
            names.get(&b).and_then(|m| m.get(name)).cloned()
        };
        let mut st = tx.prepare(
            "SELECT board, card, at, action, text, actor_kind, actor_name FROM activity
             WHERE action IN ('move', 'claim', 'release', 'lost', 'timeout') ORDER BY id",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        // 每张卡片上一条记录去往的状态（释放、超时的原来状态从这里推）
        let mut last: HashMap<(i64, i64), (Option<String>, String)> = HashMap::new();
        let mut ins = tx.prepare(
            "INSERT INTO moves (board, card, at, kind, from_key, to_key, from_name, to_name, actor_kind, actor_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for row in rows {
            let (b, card, at, action, text, ak, an) = row?;
            let Some((from, to)) = parse(&action, &text) else { continue };
            let (from_key, from_name) = match from {
                Some(n) => (key_of(b, &n), Some(n)),
                None => match last.get(&(b, card)) {
                    Some((k, n)) => (k.clone(), Some(n.clone())),
                    None => (None, None),
                },
            };
            let to_key = key_of(b, &to);
            ins.execute(params![b, card, at, action, from_key, to_key, from_name, to, ak, an])?;
            last.insert((b, card), (to_key, to));
        }
    }
    tx.execute("INSERT INTO meta (key, value) VALUES ('moves', ?1) ON CONFLICT(key) DO UPDATE SET value = ?1", [BACKFILL_VERSION])?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_move_shapes() {
        assert_eq!(parse("move", "从「待办」移到「进行中」"), Some((Some("待办".into()), "进行中".into())));
        assert_eq!(
            parse("move", "从「进行中」移到「待审核」，领取已释放（第 1 次耗时 3 分钟）"),
            Some((Some("进行中".into()), "待审核".into()))
        );
        assert_eq!(parse("move", "整理好了，从「未整理」移到「待办」"), Some((Some("未整理".into()), "待办".into())));
        assert_eq!(parse("move", "状态「测试」被删除，卡片移到「待办」，领取已释放"), Some((Some("测试".into()), "待办".into())));
        assert_eq!(parse("claim", "领取了卡片，从「返工」进入「进行中」"), Some((Some("返工".into()), "进行中".into())));
        assert_eq!(parse("release", "强制释放了 alpha 的领取，卡片回到「待办」最上面"), Some((None, "待办".into())));
        assert_eq!(parse("lost", "心跳中断：……卡片回到「返工」最上面"), Some((None, "返工".into())));
        assert_eq!(parse("release", "「进行中」取消了锁定，alpha 的领取已释放"), None);
        assert_eq!(parse("comment", "从「A」移到「B」"), None);
    }
}
