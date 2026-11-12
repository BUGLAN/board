//! 跨白板搜卡片（PRD §9、§10.1）：一次在所有白板（含「未整理」）里找，不用先切到那块白板。
//!
//! 四种写法：
//! - 普通文字：按标题、描述、评论搜，不区分大小写。空格分开多个词时先要求每个词都出现在这张卡片上；
//!   这样一张都没有时退回「任一词命中」，宁可多列几张也不给出空列表。
//! - `#12`：所有白板里编号是 12 的卡片（不同白板的卡片编号各自从头开始，可能不止一张）。
//! - `oss/main#3`：直接定位那块白板上的这张卡。
//! - `label:bug`：项目里名叫 bug 的标签（标签属于项目，不同项目可以有同名标签）。
//!
//! 结果按相关度、再按更新时间排序，最多 [`SEARCH_LIMIT`] 条。现在是直接查 SQLite：
//! 一万张卡片一次搜索在几十毫秒内（cards 扫一遍够快），还不需要全文索引。

use crate::error::{BoardError, Result};
use crate::model::{INBOX, INBOX_NAME};
use crate::store::R;
use rusqlite::{params_from_iter, ToSql};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// 一次最多返回多少条（PRD §9：最多 50 条）
pub const SEARCH_LIMIT: usize = 50;

/// 粗筛候选的上限：只取最可能相关的一批，再在内存里按相关度排序
const CANDIDATES_MAX: i64 = 200;

/// 一条搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// 白板路径：项目/白板
    pub board: String,
    pub id: i64,
    pub title: String,
    /// 状态标识；「未整理」是 `inbox`
    pub column: String,
    /// 状态名称（「未整理」是「未整理」）
    pub status: String,
    /// 匹配到哪里：title | body | comment | any（只命中了其中一部分词）| ref（`#ID`、`项目/白板#ID`）| label（`label:<标签>`）
    pub matched: String,
    /// 标签名，按项目里的标签顺序
    pub labels: Vec<String>,
    pub updated_at: i64,
    /// 相关度：越大越相关（编号、标签 1000，标题 100，描述 40，评论 25，部分命中 10）
    pub score: i64,
}

/// `GROUP_CONCAT` 的分隔符：标签名之间用它连起来，标签名里的逗号不会把名字拆开
pub const LABEL_SEP: char = '\u{1f}';

/// 结果的公共部分
struct Base {
    board: String,
    id: i64,
    title: String,
    column: String,
    status: String,
    labels: Vec<String>,
    updated_at: i64,
}

/// 公共列（`c` 是 cards、`b` 是 boards、`s` 是状态表）
const COLS: &str = "b.project || '/' || b.key, c.id, c.title, c.col, \
     COALESCE((SELECT s.name FROM statuses s WHERE s.board = c.board AND s.key = c.col), ''), \
     COALESCE((SELECT GROUP_CONCAT(l.name, char(31)) FROM card_labels cl JOIN labels l ON l.id = cl.label \
               WHERE cl.board = c.board AND cl.card = c.id), ''), \
     c.updated_at";

/// 卡片和白板
const FROM: &str = "FROM cards c JOIN boards b ON b.id = c.board";

/// 带公共列的一条完整查询
fn select() -> String {
    format!("SELECT {COLS} {FROM}")
}

impl R<'_> {
    /// 跨白板搜索。`limit` 会被夹到 1..=[`SEARCH_LIMIT`]
    pub fn search(&self, raw: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let q = raw.trim();
        if q.is_empty() {
            return Err(BoardError::usage("搜索内容不能为空"));
        }
        let limit = limit.clamp(1, SEARCH_LIMIT) as i64;

        // 项目/白板#ID
        if let Some((left, right)) = q.split_once('#') {
            let left = left.trim();
            if left.contains('/') {
                let id: i64 = right
                    .trim()
                    .parse()
                    .map_err(|_| BoardError::usage(format!("「{q}」里的编号不是数字，写成像 oss/main#3 这样")))?;
                let b = self.resolve(left)?;
                let sql = format!("{} WHERE c.board = ?1 AND c.id = ?2 LIMIT {limit}", select());
                let mut out = self.rows(&sql, vec![&b.id, &id])?;
                mark(&mut out, "ref", 1000);
                return Ok(out);
            }
        }
        // #ID
        if let Some(id) = q.strip_prefix('#').and_then(|s| s.trim().parse::<i64>().ok()) {
            let sql = format!("{} WHERE c.id = ?1 ORDER BY b.project, b.key LIMIT {limit}", select());
            let mut out = self.rows(&sql, vec![&id])?;
            mark(&mut out, "ref", 1000);
            return Ok(out);
        }
        // label:标签
        if let Some(name) = q.strip_prefix("label:").or_else(|| q.strip_prefix("label：")) {
            let name = name.trim().to_lowercase();
            if name.is_empty() {
                return Err(BoardError::usage("标签写成 label:bug 这样"));
            }
            let sql = format!(
                "{} WHERE EXISTS (SELECT 1 FROM card_labels cl JOIN labels l ON l.id = cl.label \
                   WHERE cl.board = c.board AND cl.card = c.id AND lower(l.name) = ?1) \
                 ORDER BY c.updated_at DESC LIMIT {limit}",
                select()
            );
            let mut out = self.rows(&sql, vec![&name])?;
            mark(&mut out, "label", 1000);
            return Ok(out);
        }
        self.by_text(q, limit)
    }

    /// 普通文字：按标题、描述、评论搜
    fn by_text(&self, q: &str, limit: i64) -> Result<Vec<SearchHit>> {
        let tokens: Vec<String> = q.split_whitespace().map(|t| t.to_lowercase()).collect();
        if tokens.is_empty() {
            return Err(BoardError::usage("搜索内容不能为空"));
        }
        let window = (limit * 4).min(CANDIDATES_MAX);
        let mut hits = self.text_hits(&tokens, &q.to_lowercase(), true, window)?;
        if hits.is_empty() && tokens.len() > 1 {
            hits = self.text_hits(&tokens, &q.to_lowercase(), false, window)?;
        }
        // 相关度，再按更新时间；同一张卡片只留一条
        let mut seen = BTreeSet::new();
        hits.retain(|h| seen.insert((h.board.clone(), h.id)));
        hits.sort_by(|a, b| b.score.cmp(&a.score).then(b.updated_at.cmp(&a.updated_at)));
        hits.truncate(limit as usize);
        Ok(hits)
    }

    /// `all` 为 true 时每个词都要命中，false 时任一词命中即可
    fn text_hits(&self, tokens: &[String], whole: &str, all: bool, window: i64) -> Result<Vec<SearchHit>> {
        let n = tokens.len();
        let mut conds: Vec<String> = Vec::new();
        let mut params: Vec<&dyn ToSql> = Vec::new();
        let mut likes: Vec<String> = Vec::new();
        for t in tokens {
            likes.push(format!("%{t}%"));
        }
        for i in 0..n {
            let like = i + 1;
            let exact = n + i + 1;
            // LIKE 先粗筛（用户输入里的 % 和 _ 是通配符），再用 instr 判一次真的含有这个词
            conds.push(format!(
                "(lower(c.title) LIKE ?{like} OR lower(c.body) LIKE ?{like} OR EXISTS (SELECT 1 FROM activity a \
                   WHERE a.board = c.board AND a.card = c.id AND a.kind = 'comment' AND lower(a.text) LIKE ?{like})) \
                 AND (instr(lower(c.title), ?{exact}) > 0 OR instr(lower(c.body), ?{exact}) > 0 OR EXISTS (SELECT 1 \
                   FROM activity a WHERE a.board = c.board AND a.card = c.id AND a.kind = 'comment' AND instr(lower(a.text), ?{exact}) > 0))"
            ));
        }
        for l in &likes {
            params.push(l);
        }
        for t in tokens {
            params.push(t);
        }
        let flag = 2 * n + 1;
        let whole = whole.to_string();
        params.push(&whole);
        params.push(&window);
        let join = if all { " AND " } else { " OR " };
        let sql = format!(
            "SELECT {COLS}, instr(lower(c.title), ?{flag}) > 0, instr(lower(c.body), ?{flag}) > 0, \
               EXISTS (SELECT 1 FROM activity a WHERE a.board = c.board AND a.card = c.id AND a.kind = 'comment' \
                 AND instr(lower(a.text), ?{flag}) > 0) \
             {FROM} WHERE {} \
             ORDER BY c.updated_at DESC LIMIT ?{}",
            conds.join(join),
            2 * n + 2
        );
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(params.into_iter()), |r| {
            let (matched, score) = if r.get::<_, i64>(7)? != 0 {
                ("title", 100)
            } else if r.get::<_, i64>(8)? != 0 {
                ("body", 40)
            } else if r.get::<_, i64>(9)? != 0 {
                ("comment", 25)
            } else {
                // 整串没命中：`any` 模式里只命中了其中一部分词
                ("any", 10)
            };
            Ok(hit(base(r)?, matched, score))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 执行一条完整的查询
    fn rows(&self, sql: &str, params: Vec<&dyn ToSql>) -> Result<Vec<SearchHit>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st.query_map(params_from_iter(params.into_iter()), |r| Ok(hit(base(r)?, "", 0)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

fn mark(hits: &mut [SearchHit], matched: &str, score: i64) {
    for h in hits.iter_mut() {
        h.matched = matched.to_string();
        h.score = score;
    }
}

fn base(r: &rusqlite::Row<'_>) -> rusqlite::Result<Base> {
    let column: String = r.get(3)?;
    let name: String = r.get(4)?;
    let labels: String = r.get(5)?;
    Ok(Base {
        board: r.get(0)?,
        id: r.get(1)?,
        title: r.get(2)?,
        status: if column == INBOX {
            INBOX_NAME.to_string()
        } else if name.is_empty() {
            column.clone()
        } else {
            name
        },
        column,
        labels: if labels.is_empty() { vec![] } else { labels.split(LABEL_SEP).map(str::to_string).collect() },
        updated_at: r.get(6)?,
    })
}

fn hit(b: Base, matched: &str, score: i64) -> SearchHit {
    SearchHit {
        board: b.board,
        id: b.id,
        title: b.title,
        column: b.column,
        status: b.status,
        matched: matched.to_string(),
        labels: b.labels,
        updated_at: b.updated_at,
        score,
    }
}
