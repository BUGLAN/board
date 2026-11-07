//! 工作流的修改（PRD §7）：新增、改名、改色、切换规则、调整顺序、删除状态，以及状态块布局。
//! 只在界面上做；所有改动都让 rev 加一，已打开的窗口随之刷新。

use crate::error::{BoardError, Result};
use crate::model::*;
use crate::store::{flow_of, W};
use crate::engine::At;
use rusqlite::params;
use serde::Deserialize;
use std::collections::BTreeMap;

/// 修改一个状态；没给的字段不变
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StatusPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    /// 只能设为 true：把这个状态设为领取起点，原来的领取起点自动取消
    #[serde(default)]
    pub claim: Option<bool>,
    #[serde(default)]
    pub lock: Option<bool>,
    #[serde(default, rename = "final")]
    pub fin: Option<bool>,
}

const NAME_MAX: usize = 12;

impl<'a> W<'a> {
    fn status_name(&self, flow: &Flow, name: &str, except: Option<&str>) -> Result<String> {
        let n: String = name.trim().chars().collect();
        if n.is_empty() {
            return Err(BoardError::usage("状态名称不能为空"));
        }
        if n.chars().count() > NAME_MAX {
            return Err(BoardError::usage(format!("状态名称最长 {NAME_MAX} 个字")));
        }
        let lower = n.to_lowercase();
        if lower == INBOX || n == INBOX_NAME {
            return Err(BoardError::usage(format!("「{INBOX_NAME}」是保留的名称")));
        }
        if flow.0.iter().any(|s| Some(s.key.as_str()) != except && (s.name.to_lowercase() == lower || s.key == n)) {
            return Err(BoardError::usage(format!("这块白板已经有状态「{n}」")));
        }
        Ok(n)
    }

    fn status_color(color: &str) -> Result<String> {
        STATUS_COLORS
            .iter()
            .find(|c| c.eq_ignore_ascii_case(color.trim()))
            .map(|c| c.to_string())
            .ok_or_else(|| BoardError::usage(format!("颜色必须取自色板：{}", STATUS_COLORS.join(" "))))
    }

    fn flow_status<'f>(flow: &'f Flow, key: &str) -> Result<&'f Status> {
        flow.get(key).ok_or_else(|| BoardError::usage(format!("没有状态「{key}」")))
    }

    /// 新增状态：加在工作流最后，标识依次是 s1、s2…
    pub fn create_status(&mut self, path: &str, name: &str, color: Option<&str>) -> Result<Status> {
        let b = self.resolve_exact(path)?;
        let flow = flow_of(self.tx, b.id)?;
        let name = self.status_name(&flow, name, None)?;
        let custom = flow.0.iter().filter(|s| !DEFAULT_FLOW.iter().any(|d| d.0 == s.key)).count();
        let color = match color {
            Some(c) => Self::status_color(c)?,
            None => STATUS_COLORS[4 + custom % (STATUS_COLORS.len() - 4)].to_string(),
        };
        let n = flow.0.iter().filter_map(|s| s.key.strip_prefix('s')?.parse::<i64>().ok()).max().unwrap_or(0) + 1;
        let key = format!("s{n}");
        let sort: i64 = self.tx.query_row("SELECT COALESCE(MAX(sort), -1) + 1 FROM statuses WHERE board = ?1", [b.id], |r| r.get(0))?;
        self.tx.execute(
            "INSERT INTO statuses (board, key, name, color, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![b.id, key, name, color, sort],
        )?;
        self.changed = true;
        Ok(Status { key, name, color, claim: false, lock: false, fin: false, layout: None })
    }

    pub fn update_status(&mut self, path: &str, key: &str, p: StatusPatch) -> Result<()> {
        let b = self.resolve_exact(path)?;
        let flow = flow_of(self.tx, b.id)?;
        let s = Self::flow_status(&flow, key)?.clone();
        if let Some(n) = &p.name {
            let n = self.status_name(&flow, n, Some(key))?;
            self.tx.execute("UPDATE statuses SET name = ?1 WHERE board = ?2 AND key = ?3", params![n, b.id, key])?;
        }
        if let Some(c) = &p.color {
            let c = Self::status_color(c)?;
            self.tx.execute("UPDATE statuses SET color = ?1 WHERE board = ?2 AND key = ?3", params![c, b.id, key])?;
        }
        match p.claim {
            Some(true) => {
                self.tx.execute("UPDATE statuses SET claim = (key = ?2) WHERE board = ?1", params![b.id, key])?;
            }
            Some(false) if s.claim => {
                return Err(BoardError::usage("每块白板必须有一个领取起点：把另一个状态设为领取起点即可"));
            }
            _ => {}
        }
        if let Some(lock) = p.lock {
            self.tx.execute("UPDATE statuses SET lock = ?1 WHERE board = ?2 AND key = ?3", params![lock as i64, b.id, key])?;
            if s.lock && !lock {
                // 不再锁定：里面被持有的卡片释放
                let held: Vec<(i64, String)> = self.collect(
                    "SELECT id, holder FROM cards WHERE board = ?1 AND col = ?2 AND holder IS NOT NULL ORDER BY pos",
                    params![b.id, key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                for (id, who) in held {
                    self.clear_claim(b.id, id)?;
                    self.track(b.id, id, &Actor::human(), "release", &format!("「{}」取消了锁定，{who} 的领取已释放", s.name))?;
                }
            }
        }
        if let Some(f) = p.fin {
            self.tx.execute("UPDATE statuses SET final = ?1 WHERE board = ?2 AND key = ?3", params![f as i64, b.id, key])?;
        }
        self.changed = true;
        Ok(())
    }

    /// 按给定顺序重排工作流；必须正好是现有的全部状态
    pub fn reorder_statuses(&mut self, path: &str, keys: &[String]) -> Result<()> {
        let b = self.resolve_exact(path)?;
        let flow = flow_of(self.tx, b.id)?;
        let mut a: Vec<&str> = keys.iter().map(|k| k.as_str()).collect();
        let mut c: Vec<&str> = flow.0.iter().map(|s| s.key.as_str()).collect();
        a.sort_unstable();
        c.sort_unstable();
        if a != c {
            return Err(BoardError::usage("顺序里必须正好包含这块白板的全部状态"));
        }
        for (i, k) in keys.iter().enumerate() {
            self.tx.execute("UPDATE statuses SET sort = ?1 WHERE board = ?2 AND key = ?3", params![i as i64, b.id, k])?;
        }
        self.changed = true;
        Ok(())
    }

    /// 删除状态：里面的卡片移到领取起点最下面，被持有的同时释放。领取起点不能删除
    pub fn delete_status(&mut self, path: &str, key: &str) -> Result<()> {
        let b = self.resolve_exact(path)?;
        let flow = flow_of(self.tx, b.id)?;
        let s = Self::flow_status(&flow, key)?.clone();
        if s.claim {
            return Err(BoardError::usage("领取起点不能删除：先把另一个状态设为领取起点"));
        }
        let start = flow.claim().clone();
        for id in self.col_ids(b.id, key)? {
            let held = self.tx.query_row("SELECT holder IS NOT NULL FROM cards WHERE board = ?1 AND id = ?2", params![b.id, id], |r| {
                r.get::<_, bool>(0)
            })?;
            self.place(b.id, id, Some(key), &start.key, At::Bottom)?;
            if held {
                self.clear_claim(b.id, id)?;
            }
            let text = format!("状态「{}」被删除，卡片移到「{}」{}", s.name, start.name, if held { "，领取已释放" } else { "" });
            self.track(b.id, id, &Actor::human(), "move", &text)?;
        }
        self.tx.execute("DELETE FROM statuses WHERE board = ?1 AND key = ?2", params![b.id, key])?;
        self.changed = true;
        Ok(())
    }

    /// 保存状态块布局。空表示复原为默认布局
    pub fn set_flow_layout(&mut self, path: &str, tiles: &BTreeMap<String, Tile>) -> Result<()> {
        let b = self.resolve_exact(path)?;
        let flow = flow_of(self.tx, b.id)?;
        if tiles.is_empty() {
            self.tx.execute("UPDATE statuses SET lx = NULL, lw = NULL, ly = NULL WHERE board = ?1", [b.id])?;
            self.changed = true;
            return Ok(());
        }
        for (k, t) in tiles {
            Self::flow_status(&flow, k)?;
            if t.w < 1 || t.x < 0 || t.x + t.w > 12 || !t.y.is_finite() {
                return Err(BoardError::usage("布局超出了 12 栏网格"));
            }
            self.tx.execute(
                "UPDATE statuses SET lx = ?1, lw = ?2, ly = ?3 WHERE board = ?4 AND key = ?5",
                params![t.x, t.w, t.y, b.id, k],
            )?;
        }
        self.changed = true;
        Ok(())
    }
}
