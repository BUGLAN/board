//! 里程碑（PRD §6「版本标签」、§9 `board versions sync`、§10.15）。
//!
//! - 版本 = 名称是 `v` 加数字开头的标签（`v0.1.21`、`v2`），不另设字段。
//! - 版本记录：发版时间和这一版的提交，由 `board versions sync` 在代码仓库里从 git tag 和提交历史反推后写进来
//!   （[`W::sync_release`]）：两个相邻 tag 之间的提交属于后一个 tag；提交说明里的「卡片：项目/白板#ID」把提交连到卡片，
//!   这些卡片带上这个版本的标签（标签不存在时新建）。同一个版本再同步一次，记录换成新的，已经带着标签的卡片不重复打。
//! - 里程碑页的数据（[`R::milestones`]）：项目里所有版本（带不带记录都列出）、带版本标签的卡片，以及
//!   「未发版」（还没有版本标签、已经开始做的卡片：不在未整理、不在可领取状态；终点状态里的只算最近一次发版之后移进去的）
//!   和「没有版本」（最近一次发版之前就到了终点、却没有版本标签的卡片，最多 [`NONE_LIMIT`] 张）。

use crate::error::{BoardError, ErrorCode, Result};
use crate::model::{Actor, Label, INBOX};
use crate::Cmd;
use crate::store::{R, W};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

pub(crate) const SCHEMA: &str = r#"
-- 版本记录（PRD §10.15）：board versions sync 从 git tag 反推后写入；name 是 tag 名，也是版本标签的名称
CREATE TABLE IF NOT EXISTS releases (
  project     TEXT NOT NULL REFERENCES projects(key) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  -- tag 的时间（毫秒）
  released_at INTEGER NOT NULL,
  -- 这一版的提交（JSON 数组：{h, at, s, cards: ["白板#ID", …]}），按时间从新到旧
  commits     TEXT NOT NULL,
  synced_at   INTEGER NOT NULL,
  PRIMARY KEY (project, name)
);
"#;

/// 自动新建的版本标签用的颜色（色板里的紫色）
pub const VERSION_COLOR: &str = "#8B5CF6";

/// 「没有版本」最多列多少张（最近到终点的在前）
pub const NONE_LIMIT: usize = 300;

/// 一次同步最多带多少个提交
const COMMITS_MAX: usize = 5000;

/// 版本标签：名称是 `v`（不区分大小写）加数字开头
pub fn is_version_name(name: &str) -> bool {
    let mut c = name.trim().chars();
    matches!(c.next(), Some('v' | 'V')) && c.next().is_some_and(|d| d.is_ascii_digit())
}

/// 版本号比较：去掉开头的 v，按点分开逐段比，数字段按数值、其他按文本；`v0.1.10` 比 `v0.1.9` 大
pub fn version_cmp(a: &str, b: &str) -> Ordering {
    let parts = |s: &str| s.trim().trim_start_matches(['v', 'V']).split('.').map(str::to_string).collect::<Vec<_>>();
    let (x, y) = (parts(a), parts(b));
    for i in 0..x.len().max(y.len()) {
        let (p, q) = (x.get(i).map(String::as_str).unwrap_or("0"), y.get(i).map(String::as_str).unwrap_or("0"));
        let o = match (p.parse::<u64>(), q.parse::<u64>()) {
            (Ok(m), Ok(n)) => m.cmp(&n),
            _ => p.cmp(q),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// 一个提交
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReleaseCommit {
    /// 短提交号
    pub h: String,
    /// 提交时间（毫秒）
    pub at: i64,
    /// 提交说明的第一行
    pub s: String,
    /// 连到的卡片：这个项目里的「白板#ID」（白板写标识）；没写「卡片：…」的提交为空
    #[serde(default)]
    pub cards: Vec<String>,
}

/// `POST /api/releases` 的请求：一个版本的记录
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseSync {
    pub project: String,
    /// tag 名，也是版本标签的名称
    pub name: String,
    pub released_at: i64,
    #[serde(default)]
    pub commits: Vec<ReleaseCommit>,
}

/// 同步一个版本的结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub label: Label,
    /// 这次新建了版本标签
    pub created: bool,
    /// 这次新打上标签的卡片（「白板#ID」）
    pub added: Vec<String>,
    /// 已经带着这个标签的张数
    pub already: usize,
    /// 找不到的卡片（白板不存在、卡片不存在或在回收站里）
    pub missing: Vec<String>,
}

/// 里程碑页的一个版本
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    /// 版本标签的 id
    pub label: i64,
    pub name: String,
    pub color: String,
    /// 发版时间；没有版本记录（标签是手动建的、还没打 tag）时为空
    pub released_at: Option<i64>,
    /// 这一版的提交（从新到旧）；没有版本记录时为空
    pub commits: Vec<ReleaseCommit>,
}

/// 卡片属于里程碑页的哪一组
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    /// 带版本标签（属于哪个版本看 labels）
    Version,
    /// 未发版
    Next,
    /// 没有版本
    None,
}

/// 里程碑页的一张卡片
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneCard {
    /// 白板路径：项目/白板
    pub board: String,
    pub id: i64,
    pub title: String,
    /// 状态标识
    pub column: String,
    /// 全部标签的 id（按项目里标签的顺序）
    pub labels: Vec<i64>,
    pub holder: Option<String>,
    pub created_at: i64,
    /// 移入现在这个状态的时间：终点状态里的就是做完的时间
    pub moved_at: Option<i64>,
    pub group: Group,
}

/// `GET /api/milestones?project=<项目>`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Milestones {
    /// 版本，版本号从新到旧
    pub versions: Vec<VersionInfo>,
    /// 最近一次发版的时间（所有版本记录里最晚的）；还没有版本记录时为空
    pub last_release: Option<i64>,
    pub cards: Vec<MilestoneCard>,
}

/// 「白板#ID」拆成白板标识和编号
fn parse_ref(r: &str) -> Option<(&str, i64)> {
    let (b, id) = r.trim().rsplit_once('#')?;
    let id = id.parse::<i64>().ok().filter(|n| *n > 0)?;
    (!b.is_empty() && !b.contains('/')).then_some((b, id))
}

impl W<'_> {
    /// 写入一个版本的记录（`board versions sync`）：版本标签不存在时新建；提交里连到的卡片带上这个标签
    /// （由系统打，动态里写「添加了标签 vX」，仅人可打的标签也照打）；记录按项目和名称覆盖
    pub fn sync_release(&mut self, req: ReleaseSync) -> Result<SyncResult> {
        let project = req.project.trim().to_string();
        let name: String = req.name.trim().to_string();
        if !is_version_name(&name) {
            return Err(BoardError::usage(format!("版本名称要以 v 加数字开头：{name}")));
        }
        if name.chars().count() > 24 {
            return Err(BoardError::usage(format!("版本名称最多 24 个字：{name}")));
        }
        if req.commits.len() > COMMITS_MAX {
            return Err(BoardError::usage(format!("一个版本最多 {COMMITS_MAX} 个提交")));
        }
        // 项目不存在时 create_label、查标签都会报错，这里先给出明确的原因
        let exists: Option<String> = self.tx.query_row("SELECT key FROM projects WHERE key = ?1", [&project], |r| r.get(0)).optional()?;
        if exists.is_none() {
            return Err(BoardError::new(ErrorCode::BoardNotFound, format!("项目「{project}」不存在")));
        }

        let found: Option<(i64, String, String, bool, bool)> = self
            .tx
            .query_row(
                "SELECT id, name, color, human_only, priority FROM labels WHERE project = ?1 AND lower(name) = lower(?2)",
                params![project, name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (label, created) = match found {
            Some((id, name, color, human_only, priority)) => (Label { id, name, color, uses: 0, human_only, priority }, false),
            None => (self.create_label(&project, &name, Some(VERSION_COLOR), false)?, true),
        };

        // 连到的卡片：去重，按出现的顺序
        let mut refs: Vec<String> = Vec::new();
        for c in &req.commits {
            for r in &c.cards {
                let r = r.trim().to_string();
                if !refs.contains(&r) {
                    refs.push(r);
                }
            }
        }
        let system = Actor::system();
        let (mut added, mut already, mut missing) = (Vec::new(), 0usize, Vec::new());
        for r in &refs {
            let Some((board_key, id)) = parse_ref(r) else {
                missing.push(r.clone());
                continue;
            };
            let Ok(board) = self.resolve(&format!("{project}/{board_key}")) else {
                missing.push(r.clone());
                continue;
            };
            let deleted: Option<Option<i64>> = self
                .tx
                .query_row("SELECT deleted_at FROM cards WHERE board = ?1 AND id = ?2", params![board.id, id], |r| r.get(0))
                .optional()?;
            if !matches!(deleted, Some(None)) {
                missing.push(r.clone());
                continue;
            }
            let has: i64 = self.tx.query_row(
                "SELECT COUNT(*) FROM card_labels WHERE board = ?1 AND card = ?2 AND label = ?3",
                params![board.id, id, label.id],
                |r| r.get(0),
            )?;
            if has > 0 {
                already += 1;
                continue;
            }
            self.exec(&board, &system, Cmd::Label { id, add: vec![label.id], remove: vec![] })?;
            added.push(format!("{board_key}#{id}"));
        }

        let mut commits = req.commits;
        commits.sort_by(|a, b| b.at.cmp(&a.at));
        self.tx.execute(
            "INSERT INTO releases (project, name, released_at, commits, synced_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (project, name) DO UPDATE SET released_at = excluded.released_at, commits = excluded.commits, synced_at = excluded.synced_at",
            params![project, label.name, req.released_at, serde_json::to_string(&commits).unwrap_or_else(|_| "[]".into()), self.now],
        )?;
        self.changed = true;
        let uses: i64 = self.tx.query_row(
            "SELECT COUNT(*) FROM card_labels cl JOIN cards c ON c.board = cl.board AND c.id = cl.card WHERE cl.label = ?1 AND c.deleted_at IS NULL",
            [label.id],
            |r| r.get(0),
        )?;
        Ok(SyncResult { label: Label { uses, ..label }, created, added, already, missing })
    }
}

impl R<'_> {
    /// 里程碑页的数据（见模块说明）
    pub fn milestones(&self, project: &str) -> Result<Milestones> {
        let exists: Option<String> = self.conn.query_row("SELECT key FROM projects WHERE key = ?1", [project], |r| r.get(0)).optional()?;
        if exists.is_none() {
            return Err(BoardError::new(ErrorCode::BoardNotFound, format!("项目「{project}」不存在")));
        }

        // 版本记录（按名称，不区分大小写）
        let mut releases: HashMap<String, (i64, Vec<ReleaseCommit>)> = HashMap::new();
        let mut last_release: Option<i64> = None;
        {
            let mut st = self.conn.prepare("SELECT name, released_at, commits FROM releases WHERE project = ?1")?;
            let rows = st.query_map([project], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
            for row in rows {
                let (name, at, commits) = row?;
                last_release = Some(last_release.map_or(at, |t: i64| t.max(at)));
                releases.insert(name.to_lowercase(), (at, serde_json::from_str(&commits).unwrap_or_default()));
            }
        }

        // 版本标签
        let mut versions: Vec<VersionInfo> = Vec::new();
        let mut label_order: HashMap<i64, usize> = HashMap::new();
        {
            let mut st = self.conn.prepare("SELECT id, name, color FROM labels WHERE project = ?1 ORDER BY sort, id")?;
            let rows = st.query_map([project], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            for (i, row) in rows.enumerate() {
                let (id, name, color) = row?;
                label_order.insert(id, i);
                if is_version_name(&name) {
                    let rel = releases.remove(&name.to_lowercase());
                    versions.push(VersionInfo {
                        label: id,
                        name,
                        color,
                        released_at: rel.as_ref().map(|(at, _)| *at),
                        commits: rel.map(|(_, c)| c).unwrap_or_default(),
                    });
                }
            }
        }
        versions.sort_by(|a, b| version_cmp(&b.name, &a.name));
        let version_ids: BTreeSet<i64> = versions.iter().map(|v| v.label).collect();

        // 项目里所有没删的卡片：标签、所在状态的规则一起取出来，再在内存里分组
        struct Row {
            board: String,
            id: i64,
            title: String,
            column: String,
            labels: Vec<i64>,
            holder: Option<String>,
            created_at: i64,
            moved_at: Option<i64>,
            claim: bool,
            final_: bool,
        }
        let mut st = self.conn.prepare(
            "SELECT b.project || '/' || b.key, c.id, c.title, c.col, c.holder, c.created_at, c.moved_at,
                    COALESCE(s.claim, 0), COALESCE(s.final, 0),
                    COALESCE((SELECT GROUP_CONCAT(cl.label) FROM card_labels cl WHERE cl.board = c.board AND cl.card = c.id), '')
             FROM cards c JOIN boards b ON b.id = c.board
             LEFT JOIN statuses s ON s.board = c.board AND s.key = c.col
             WHERE b.project = ?1 AND c.deleted_at IS NULL",
        )?;
        let rows = st.query_map([project], |r| {
            let labels: String = r.get(9)?;
            Ok(Row {
                board: r.get(0)?,
                id: r.get(1)?,
                title: r.get(2)?,
                column: r.get(3)?,
                holder: r.get(4)?,
                created_at: r.get(5)?,
                moved_at: r.get(6)?,
                claim: r.get::<_, i64>(7)? != 0,
                final_: r.get::<_, i64>(8)? != 0,
                labels: labels.split(',').filter_map(|s| s.parse().ok()).collect(),
            })
        })?;

        let mut cards: Vec<MilestoneCard> = Vec::new();
        let mut none: Vec<MilestoneCard> = Vec::new();
        for row in rows {
            let mut r = row?;
            r.labels.sort_by_key(|l| label_order.get(l).copied().unwrap_or(usize::MAX));
            let group = if r.labels.iter().any(|l| version_ids.contains(l)) {
                Group::Version
            } else if r.column == INBOX || r.claim {
                continue;
            } else if !r.final_ {
                Group::Next
            } else if last_release.is_some_and(|t| r.moved_at.unwrap_or(r.created_at) > t) {
                Group::Next
            } else {
                Group::None
            };
            let c = MilestoneCard {
                board: r.board,
                id: r.id,
                title: r.title,
                column: r.column,
                labels: r.labels,
                holder: r.holder,
                created_at: r.created_at,
                moved_at: r.moved_at,
                group,
            };
            if group == Group::None {
                none.push(c);
            } else {
                cards.push(c);
            }
        }
        none.sort_by(|a, b| b.moved_at.unwrap_or(b.created_at).cmp(&a.moved_at.unwrap_or(a.created_at)));
        none.truncate(NONE_LIMIT);
        cards.extend(none);
        Ok(Milestones { versions, last_release, cards })
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn version_names() {
        for n in ["v0.1.21", "V2", "v1.0-beta", " v3 "] {
            assert!(is_version_name(n), "{n}");
        }
        for n in ["visual", "v", "0.1.2", "版本 v1", "bug", ""] {
            assert!(!is_version_name(n), "{n}");
        }
    }

    #[test]
    fn version_order() {
        assert_eq!(version_cmp("v0.1.10", "v0.1.9"), Ordering::Greater);
        assert_eq!(version_cmp("v1", "v0.9.9"), Ordering::Greater);
        assert_eq!(version_cmp("v0.2", "v0.2.0"), Ordering::Equal);
        assert_eq!(version_cmp("V0.1.2", "v0.1.3"), Ordering::Less);
    }

    #[test]
    fn refs() {
        assert_eq!(parse_ref("main#228"), Some(("main", 228)));
        assert_eq!(parse_ref(" main#3 "), Some(("main", 3)));
        assert_eq!(parse_ref("main#0"), None);
        assert_eq!(parse_ref("oss/main#3"), None);
        assert_eq!(parse_ref("#3"), None);
        assert_eq!(parse_ref("main"), None);
    }
}
