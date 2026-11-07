//! 卡片规则引擎（PRD §8）。所有规则只在同一块白板内生效。
//!
//! 规则按状态的角色判断，而不是按固定的列名：领取起点、锁定、终点（PRD §7）。
//! 每条命令在写事务里执行：先释放这块白板上到期的领取（规则 7），再执行命令。
//! 违反规则返回 BoardError，事务整体回滚。

use crate::error::{BoardError, ErrorCode, Result};
use crate::model::*;
use crate::store::{hhmm, W};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::json;

/// 对一张卡片的操作。界面通过 HTTP 发送同样的 JSON（`{"op": "move", ...}`）
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Cmd {
    New {
        title: String,
        #[serde(default)]
        body: Option<String>,
        /// 只有界面会传 "inbox"；默认领取起点
        #[serde(default)]
        column: Option<String>,
        #[serde(default)]
        top: bool,
        #[serde(default)]
        labels: Vec<i64>,
    },
    Edit {
        id: i64,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        body: Option<String>,
    },
    Move {
        id: i64,
        /// 状态标识或名称
        column: String,
        #[serde(default)]
        top: bool,
        /// 界面拖拽松手的位置（目标状态里，不计被拖的卡片本身）
        #[serde(default)]
        index: Option<usize>,
    },
    Claim {
        #[serde(default)]
        id: Option<i64>,
    },
    Comment {
        id: i64,
        text: String,
    },
    Label {
        id: i64,
        #[serde(default)]
        add: Vec<i64>,
        #[serde(default)]
        remove: Vec<i64>,
    },
    /// 心跳：持有者每 10 秒发一次，表明还在工作（续期领取）；`final` 表示正常结束
    Beat {
        id: i64,
        #[serde(default, rename = "final")]
        done: bool,
    },
    Release {
        id: i64,
    },
    Delete {
        id: i64,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum At {
    Top,
    Bottom,
    Index(usize),
}

struct Row {
    col: String,
    holder: Option<String>,
    title: String,
    body: String,
}

impl<'a> W<'a> {
    /// 执行一条卡片命令，返回执行后的卡片（删除时返回删除前的卡片）
    pub fn exec(&mut self, board: &BoardRef, actor: &Actor, cmd: Cmd) -> Result<CardDetail> {
        self.sweep_board(board.id)?;
        let b = board.id;
        let flow = self.read().flow(board)?;
        let is_holder = |r: &Row| actor.is_agent() && r.holder.as_deref() == Some(actor.name.as_str());

        let id = match cmd {
            /* ---------------- 新建 ---------------- */
            Cmd::New { title, body, column, top, labels } => {
                let title = title.trim().to_string();
                if title.is_empty() {
                    return Err(BoardError::code(ErrorCode::EmptyTitle));
                }
                let start = flow.claim();
                let col = match column.as_deref().map(|c| flow.resolve(c)) {
                    None => start.key.clone(),
                    Some(Some(k)) if k == start.key || k == INBOX => k,
                    Some(_) => {
                        return Err(BoardError::usage(format!(
                            "新建只能放进「{}」或「{INBOX_NAME}」",
                            start.name
                        )))
                    }
                };
                if col == INBOX && !actor.is_human() {
                    return Err(BoardError::usage("「未整理」只在界面上新建"));
                }
                self.check_labels(board, &labels)?;
                self.tx.execute("UPDATE boards SET seq = seq + 1 WHERE id = ?1", [b])?;
                let id: i64 = self.tx.query_row("SELECT seq FROM boards WHERE id = ?1", [b], |r| r.get(0))?;
                self.tx.execute(
                    "INSERT INTO cards (board, id, title, body, col, pos, created_at, updated_at, created_by_kind, created_by_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6, ?7, ?8)",
                    params![b, id, title, body.as_deref().unwrap_or_default().trim(), col, self.now, actor.kind.as_str(), actor.name],
                )?;
                // 新建弹窗里先上传的附件归到这张卡片
                self.bind_attachments(b, id, body.as_deref().unwrap_or_default())?;
                self.place(b, id, None, &col, if top { At::Top } else { At::Bottom })?;
                for l in dedup(&labels) {
                    self.tx.execute("INSERT INTO card_labels (board, card, label) VALUES (?1, ?2, ?3)", params![b, id, l])?;
                }
                let text = if col == INBOX {
                    "记下了一条未整理的条目".to_string()
                } else if top {
                    format!("新建了卡片，放在「{}」最上面", start.name)
                } else {
                    "新建了卡片".to_string()
                };
                self.track(b, id, actor, "create", &text)?;
                id
            }

            /* ---------------- 编辑 ---------------- */
            Cmd::Edit { id, title, body } => {
                let r = self.row(b, id)?;
                if let Some(holder) = r.holder.clone().filter(|_| !is_holder(&r)) {
                    return Err(BoardError::new(ErrorCode::NotHolder, format!("#{id} 由 {holder} 持有，只有持有者能编辑"))
                        .with_detail(json!({ "holder": holder })));
                }
                let mut changes: Vec<String> = vec![];
                if let Some(t) = title {
                    let t = t.trim().to_string();
                    if t.is_empty() {
                        return Err(BoardError::code(ErrorCode::EmptyTitle));
                    }
                    if t != r.title {
                        self.tx.execute("UPDATE cards SET title = ?1 WHERE board = ?2 AND id = ?3", params![t, b, id])?;
                        changes.push(format!("把标题从「{}」改为「{}」", r.title, t));
                    }
                }
                if let Some(body) = body {
                    let body = body.trim().to_string();
                    if body != r.body {
                        self.tx.execute("UPDATE cards SET body = ?1 WHERE board = ?2 AND id = ?3", params![body, b, id])?;
                        self.bind_attachments(b, id, &body)?;
                        changes.push("修改了描述".into());
                    }
                }
                if is_holder(&r) {
                    self.renew(b, id)?; // 规则 6
                }
                for text in changes {
                    self.track(b, id, actor, "edit", &text)?;
                }
                id
            }

            /* ---------------- 移动 / 排序 ---------------- */
            Cmd::Move { id, column, top, index } => {
                let r = self.row(b, id)?;
                let to = flow
                    .resolve(&column)
                    .ok_or_else(|| BoardError::usage(format!("未知的状态「{}」，可选：{}", column.trim(), flow.choices())))?;
                let from = r.col.clone();
                // 规则 5：被持有的卡片只有持有者能移动
                if let Some(holder) = r.holder.clone().filter(|_| !is_holder(&r)) {
                    let msg = if actor.is_human() {
                        format!("#{id} 由 {holder} 持有，只有持有者能移动。需要的话，打开卡片「强制释放」。")
                    } else {
                        format!("#{id} 由 {holder} 持有，你不是持有者")
                    };
                    return Err(BoardError::new(ErrorCode::NotHolder, msg).with_detail(json!({ "holder": holder })));
                }
                // 规则 4：进入锁定状态只能靠领取；持有者可以在锁定状态之间移动
                let keep_claim = is_holder(&r) && flow.is_lock(&from);
                if flow.is_lock(&to) && from != to && !keep_claim {
                    return Err(BoardError::new(
                        ErrorCode::ForbiddenColumn,
                        format!("「{}」已锁定：卡片只能由 agent 通过 board claim 领取进入", flow.name(&to)),
                    ));
                }
                // 规则 9：终点只归人管
                if actor.is_agent() && (flow.is_final(&to) || flow.is_final(&from)) {
                    let msg = if flow.is_final(&to) {
                        format!("只有人能把卡片移入「{}」", flow.name(&to))
                    } else {
                        format!("只有人能把卡片移出「{}」", flow.name(&from))
                    };
                    return Err(BoardError::new(ErrorCode::ForbiddenColumn, msg));
                }
                let at = match (index, top) {
                    (Some(i), _) => At::Index(i),
                    (None, true) => At::Top,
                    (None, false) => At::Bottom,
                };
                let (old, new) = self.place(b, id, Some(&from), &to, at)?;
                if from != to {
                    // 规则 8：离开锁定状态即释放
                    let released = r.holder.is_some() && !flow.is_lock(&to);
                    if released {
                        self.clear_claim(b, id)?;
                    } else if is_holder(&r) {
                        self.renew(b, id)?;
                    }
                    let text = if from == INBOX {
                        format!("整理好了，从「{INBOX_NAME}」移到「{}」", flow.name(&to))
                    } else {
                        format!(
                            "从「{}」移到「{}」{}",
                            flow.name(&from),
                            flow.name(&to),
                            if released { "，领取已释放" } else { "" }
                        )
                    };
                    self.track(b, id, actor, "move", &text)?;
                } else if old != new {
                    if is_holder(&r) {
                        self.renew(b, id)?;
                    }
                    let text = format!("调整了在「{}」中的顺序（第 {} → {} 位）", flow.name(&to), old + 1, new + 1);
                    self.track(b, id, actor, "reorder", &text)?;
                }
                id
            }

            /* ---------------- 领取 ---------------- */
            Cmd::Claim { id } => {
                if !actor.is_agent() {
                    return Err(BoardError::usage("只有 agent 能通过 CLI 领取卡片"));
                }
                let start = flow.claim();
                let mine = self.holding(b, &actor.name)?;
                let already = |m: i64| {
                    BoardError::new(
                        ErrorCode::AlreadyHolding,
                        format!("你已经持有 #{m}，一个 agent 在一块白板上同时只能持有一张卡片"),
                    )
                    .with_detail(json!({ "holding": m }))
                };
                let target = match id {
                    None => {
                        if let Some(m) = mine {
                            return Err(already(m));
                        }
                        match self.col_ids(b, &start.key)?.first() {
                            Some(first) => *first,
                            None => {
                                return Err(BoardError::new(
                                    ErrorCode::EmptyTodo,
                                    format!("「{}」里没有可领取的卡片", start.name),
                                ))
                            }
                        }
                    }
                    Some(id) => {
                        let r = self.row(b, id)?;
                        // 再次领取自己持有的卡片 = 续期（规则 6）
                        if is_holder(&r) {
                            let exp = self.renew(b, id)?;
                            self.track(b, id, actor, "renew", &format!("重新领取，到期时间续至 {}", hhmm(exp)))?;
                            return self.read().card(board, id);
                        }
                        if r.col != start.key {
                            return Err(BoardError::new(
                                ErrorCode::NotInTodo,
                                format!("#{id} 在「{}」，不在领取起点「{}」", flow.name(&r.col), start.name),
                            ));
                        }
                        if let Some(m) = mine {
                            return Err(already(m));
                        }
                        id
                    }
                };
                let Some(dest) = flow.claim_target() else {
                    return Err(BoardError::new(ErrorCode::ForbiddenColumn, "工作流里没有锁定状态，无法领取"));
                };
                self.place(b, target, Some(&start.key), &dest.key, At::Bottom)?;
                let exp = self.now + CLAIM_MS;
                self.tx.execute(
                    "UPDATE cards SET holder = ?1, claimed_at = ?2, expires_at = ?3 WHERE board = ?4 AND id = ?5",
                    params![actor.name, self.now, exp, b, target],
                )?;
                self.track(b, target, actor, "claim", &format!("领取了卡片，进入「{}」，到期 {}", dest.name, hhmm(exp)))?;
                target
            }

            /* ---------------- 评论 ---------------- */
            Cmd::Comment { id, text } => {
                let r = self.row(b, id)?;
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err(BoardError::usage("评论内容不能为空"));
                }
                if is_holder(&r) {
                    self.renew(b, id)?; // 规则 6
                }
                self.bind_attachments(b, id, &text)?;
                self.track(b, id, actor, "comment", &text)?;
                id
            }

            /* ---------------- 标签 ----------------
               任何状态都可以，包括别人持有的卡片；不续期（PRD §6） */
            Cmd::Label { id, add, remove } => {
                self.row(b, id)?;
                self.check_labels(board, &add)?;
                self.check_labels(board, &remove)?;
                let current: Vec<i64> = self.collect(
                    "SELECT label FROM card_labels WHERE board = ?1 AND card = ?2",
                    params![b, id],
                    |r| r.get(0),
                )?;
                let added: Vec<i64> = dedup(&add).into_iter().filter(|l| !current.contains(l) && !remove.contains(l)).collect();
                let removed: Vec<i64> = dedup(&remove).into_iter().filter(|l| current.contains(l)).collect();
                for l in &added {
                    self.tx.execute("INSERT INTO card_labels (board, card, label) VALUES (?1, ?2, ?3)", params![b, id, l])?;
                }
                for l in &removed {
                    self.tx.execute("DELETE FROM card_labels WHERE board = ?1 AND card = ?2 AND label = ?3", params![b, id, l])?;
                }
                if !added.is_empty() {
                    let names = self.label_names(&added)?;
                    self.track(b, id, actor, "label", &format!("添加了标签 {names}"))?;
                }
                if !removed.is_empty() {
                    let names = self.label_names(&removed)?;
                    self.track(b, id, actor, "label", &format!("移除了标签 {names}"))?;
                }
                id
            }

            /* ---------------- 心跳（仅 agent） ----------------
               第一次心跳记一条动态，之后只更新 beat_at 并续期，不记动态。
               final：持有者正常结束心跳；卡片已经不归自己（例如已交审）时什么也不做 */
            Cmd::Beat { id, done } => {
                if !actor.is_agent() {
                    return Err(BoardError::usage("心跳只能由 agent 通过 CLI 发送"));
                }
                let r = self.row(b, id)?;
                let beating: Option<i64> =
                    self.tx.query_row("SELECT beat_at FROM cards WHERE board = ?1 AND id = ?2", params![b, id], |r| r.get(0))?;
                if done {
                    if is_holder(&r) && beating.is_some() {
                        self.tx.execute("UPDATE cards SET beat_at = NULL WHERE board = ?1 AND id = ?2", params![b, id])?;
                        self.changed = true;
                        self.track(b, id, actor, "beat", "结束了心跳，正常退出")?;
                    }
                    return self.read().card(board, id);
                }
                if !is_holder(&r) {
                    let msg = match &r.holder {
                        Some(h) => format!("#{id} 由 {h} 持有，你不是持有者"),
                        None => format!("#{id} 没有被领取：先 board claim {id}"),
                    };
                    return Err(BoardError::new(ErrorCode::NotHolder, msg));
                }
                self.tx.execute("UPDATE cards SET beat_at = ?1 WHERE board = ?2 AND id = ?3", params![self.now, b, id])?;
                self.renew(b, id)?;
                if beating.is_none() {
                    self.track(
                        b,
                        id,
                        actor,
                        "beat",
                        &format!("开始发送心跳（每 {} 秒一次，连续 {BEAT_MISSES} 次没收到判定异常退出）", BEAT_MS / 1000),
                    )?;
                }
                return self.read().card(board, id);
            }

            /* ---------------- 强制释放（仅人） ---------------- */
            Cmd::Release { id } => {
                if !actor.is_human() {
                    return Err(BoardError::usage("只有人能在界面上强制释放"));
                }
                let r = self.row(b, id)?;
                let Some(who) = r.holder.clone() else {
                    return Err(BoardError::usage(format!("#{id} 没有被领取")));
                };
                let start = flow.claim();
                self.place(b, id, Some(&r.col), &start.key, At::Top)?;
                self.clear_claim(b, id)?;
                self.track(b, id, actor, "release", &format!("强制释放了 {who} 的领取，卡片回到「{}」最上面", start.name))?;
                id
            }

            /* ---------------- 删除（仅人，规则 10） ---------------- */
            Cmd::Delete { id } => {
                if !actor.is_human() {
                    return Err(BoardError::usage("删除只能在界面上做"));
                }
                let detail = self.read().card(board, id)?;
                self.event(b, id, &detail.card.title, actor, "delete", "删除了卡片")?;
                self.tx.execute("DELETE FROM cards WHERE board = ?1 AND id = ?2", params![b, id])?;
                return Ok(detail);
            }
        };
        self.read().card(board, id)
    }

    /// 规则 7：释放这块白板上到期的领取，以及心跳中断（连续 5 次没收到）的领取，卡片回到领取起点最上面
    pub(crate) fn sweep_board(&mut self, b: i64) -> Result<usize> {
        let gap = BEAT_MS * BEAT_MISSES;
        // 最早到期的最后插入 → 位于最上面
        let expired: Vec<(i64, Option<String>, String, bool)> = self.collect(
            "SELECT id, holder, col, beat_at IS NOT NULL AND beat_at + ?3 <= ?2 FROM cards
             WHERE board = ?1 AND holder IS NOT NULL
               AND ((expires_at IS NOT NULL AND expires_at <= ?2) OR (beat_at IS NOT NULL AND beat_at + ?3 <= ?2))
             ORDER BY MIN(COALESCE(expires_at, beat_at + ?3), COALESCE(beat_at + ?3, expires_at)) DESC, id DESC",
            params![b, self.now, gap],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        if expired.is_empty() {
            return Ok(0);
        }
        let start = crate::store::flow_of(self.tx, b)?;
        let start = start.claim();
        for (id, who, col, lost) in &expired {
            self.place(b, *id, Some(col), &start.key, At::Top)?;
            self.clear_claim(b, *id)?;
            let who = who.clone().unwrap_or_default();
            if *lost {
                let text = format!("连续 {BEAT_MISSES} 次没有收到 {who} 的心跳，判定异常退出，卡片回到「{}」最上面", start.name);
                self.track(b, *id, &Actor::system(), "lost", &text)?;
            } else {
                self.track(b, *id, &Actor::system(), "timeout", &format!("领取超时，自动释放（原持有者 {who}）"))?;
            }
        }
        Ok(expired.len())
    }

    /* ---------------- 辅助 ---------------- */

    fn row(&self, b: i64, id: i64) -> Result<Row> {
        self.tx
            .query_row(
                "SELECT col, holder, title, body FROM cards WHERE board = ?1 AND id = ?2",
                params![b, id],
                |r| Ok(Row { col: r.get(0)?, holder: r.get(1)?, title: r.get(2)?, body: r.get(3)? }),
            )
            .optional()?
            .ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("卡片 #{id} 不存在")))
    }

    pub(crate) fn col_ids(&self, b: i64, col: &str) -> Result<Vec<i64>> {
        self.collect("SELECT id FROM cards WHERE board = ?1 AND col = ?2 ORDER BY pos, id", params![b, col], |r| r.get(0))
    }

    fn write_col(&self, b: i64, col: &str, ids: &[i64]) -> Result<()> {
        let mut st = self.tx.prepare_cached("UPDATE cards SET col = ?1, pos = ?2 WHERE board = ?3 AND id = ?4")?;
        for (i, id) in ids.iter().enumerate() {
            st.execute(params![col, i as i64, b, id])?;
        }
        Ok(())
    }

    /// 把卡片放到目标状态的指定位置，返回（原位置，新位置）
    pub(crate) fn place(&mut self, b: i64, id: i64, from: Option<&str>, to: &str, at: At) -> Result<(usize, usize)> {
        let mut from_list = match from {
            Some(f) => self.col_ids(b, f)?,
            None => vec![],
        };
        let old = from_list.iter().position(|x| *x == id);
        if let Some(i) = old {
            from_list.remove(i);
        }
        let mut to_list = if from == Some(to) { from_list.clone() } else { self.col_ids(b, to)? };
        to_list.retain(|x| *x != id);
        let idx = match at {
            At::Top => 0,
            At::Bottom => to_list.len(),
            At::Index(i) => i.min(to_list.len()),
        };
        to_list.insert(idx, id);
        if let Some(f) = from {
            if f != to {
                self.write_col(b, f, &from_list)?;
            }
        }
        self.write_col(b, to, &to_list)?;
        self.changed = true;
        Ok((old.unwrap_or(0), idx))
    }

    fn holding(&self, b: i64, name: &str) -> Result<Option<i64>> {
        Ok(self
            .tx
            .query_row(
                "SELECT id FROM cards WHERE board = ?1 AND holder = ?2 ORDER BY pos LIMIT 1",
                params![b, name],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// 领取到期时间重新算为 30 分钟后
    fn renew(&mut self, b: i64, id: i64) -> Result<i64> {
        let exp = self.now + CLAIM_MS;
        self.tx.execute("UPDATE cards SET expires_at = ?1 WHERE board = ?2 AND id = ?3", params![exp, b, id])?;
        self.changed = true;
        Ok(exp)
    }

    pub(crate) fn clear_claim(&mut self, b: i64, id: i64) -> Result<()> {
        self.tx.execute(
            "UPDATE cards SET holder = NULL, claimed_at = NULL, expires_at = NULL, beat_at = NULL WHERE board = ?1 AND id = ?2",
            params![b, id],
        )?;
        self.changed = true;
        Ok(())
    }

    /// 标签必须属于白板所在的项目
    fn check_labels(&self, board: &BoardRef, ids: &[i64]) -> Result<()> {
        for id in ids {
            let ok: Option<i64> = self
                .tx
                .query_row("SELECT id FROM labels WHERE id = ?1 AND project = ?2", params![id, board.project], |r| r.get(0))
                .optional()?;
            if ok.is_none() {
                let names: Vec<String> =
                    self.collect("SELECT name FROM labels WHERE project = ?1 ORDER BY sort, id", [&board.project], |r| r.get(0))?;
                return Err(BoardError::usage(format!(
                    "项目里没有这个标签，可选：{}",
                    if names.is_empty() { "（还没有标签）".into() } else { names.join("、") }
                )));
            }
        }
        Ok(())
    }

    fn label_names(&self, ids: &[i64]) -> Result<String> {
        let mut out = String::new();
        for id in ids {
            let n: String = self.tx.query_row("SELECT name FROM labels WHERE id = ?1", [id], |r| r.get(0))?;
            out.push_str(&format!("「{n}」"));
        }
        Ok(out)
    }

    /// 示例数据用：直接设置卡片标签，不记动态
    pub(crate) fn set_labels_raw(&mut self, b: i64, card: i64, labels: &[i64]) -> Result<()> {
        self.tx.execute("DELETE FROM card_labels WHERE board = ?1 AND card = ?2", params![b, card])?;
        for l in labels {
            self.tx.execute("INSERT INTO card_labels (board, card, label) VALUES (?1, ?2, ?3)", params![b, card, l])?;
        }
        self.changed = true;
        Ok(())
    }

    /// 示例数据用：在指定时间执行
    pub(crate) fn at(&mut self, t: i64) -> &mut Self {
        self.now = t;
        self
    }
}

fn dedup(v: &[i64]) -> Vec<i64> {
    let mut out: Vec<i64> = vec![];
    for x in v {
        if !out.contains(x) {
            out.push(*x);
        }
    }
    out
}
