//! 快捷提示词（default/main#413、#572）：新建卡片描述里常写的片段，整个服务器一份列表，网页、桌面端、手机共用。
//!
//! - 读出整列（按列表顺序）、整列写入；没有单条增删：列表很短，界面每次改完整列保存
//! - 写入的校验和界面 `web/src/lib/prompts.ts` 一致：id、标题去掉首尾空白后必填，内容的 `\r\n` 归一成 `\n`、
//!   不能只有空白，id 不能重复；不合规的整列拒绝（`USAGE`），不丢掉其中几条

use crate::store::{R, W};
use crate::{BoardError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub(crate) const SCHEMA: &str = r#"
-- 快捷提示词：pos 是列表里的顺序（从 0 开始）
CREATE TABLE IF NOT EXISTS prompts (
  id    TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  text  TEXT NOT NULL,
  pos   INTEGER NOT NULL
);
"#;

/// 一条快捷提示词
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    /// 列表里唯一，界面按它找编辑、删除的目标
    pub id: String,
    pub title: String,
    /// 原样插入描述的 Markdown 文本
    pub text: String,
}

/// 按界面的规则整理、校验一整列；有一条不合规就整列拒绝
fn normalize(list: Vec<Prompt>) -> Result<Vec<Prompt>> {
    let mut ids = HashSet::new();
    let mut out = Vec::with_capacity(list.len());
    for (i, p) in list.into_iter().enumerate() {
        let n = i + 1;
        let id = p.id.trim().to_string();
        let title = p.title.trim().to_string();
        let text = p.text.replace("\r\n", "\n").replace('\r', "\n");
        if id.is_empty() {
            return Err(BoardError::usage(format!("第 {n} 条快捷提示词缺 id")));
        }
        if title.is_empty() {
            return Err(BoardError::usage(format!("第 {n} 条快捷提示词的标题是必填的")));
        }
        if text.trim().is_empty() {
            return Err(BoardError::usage(format!("快捷提示词「{title}」的内容是必填的")));
        }
        if !ids.insert(id.clone()) {
            return Err(BoardError::usage(format!("快捷提示词的 id「{id}」重复了")));
        }
        out.push(Prompt { id, title, text });
    }
    Ok(out)
}

impl R<'_> {
    /// 整列快捷提示词，按列表顺序
    pub fn prompts(&self) -> Result<Vec<Prompt>> {
        let mut st = self.conn.prepare("SELECT id, title, text FROM prompts ORDER BY pos, id")?;
        let rows = st.query_map([], |r| Ok(Prompt { id: r.get(0)?, title: r.get(1)?, text: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

impl W<'_> {
    /// 整列换成 list（顺序照 list），返回整理后的整列
    pub fn prompts_put(&mut self, list: Vec<Prompt>) -> Result<Vec<Prompt>> {
        let list = normalize(list)?;
        self.tx.execute("DELETE FROM prompts", [])?;
        let mut st = self.tx.prepare("INSERT INTO prompts (id, title, text, pos) VALUES (?1, ?2, ?3, ?4)")?;
        for (i, p) in list.iter().enumerate() {
            st.execute(rusqlite::params![p.id, p.title, p.text, i as i64])?;
        }
        Ok(list)
    }
}
