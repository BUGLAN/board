//! 卡片规则引擎（PRD §8）。所有规则只在同一块白板内生效。
//!
//! 规则按状态的角色和连线判断，而不是按固定的列名：可领取、锁定、终点（PRD §7）。agent 只能沿连线移动卡片。
//! 每条命令在写事务里执行：先释放这块白板上到期的领取（规则 7），再执行命令。
//! 违反规则返回 BoardError，事务整体回滚。

use crate::error::{BoardError, ErrorCode, Result};
use crate::model::*;
use crate::store::{Store, R, W};

/// 回收站保留多久：删除后 30 天内可以恢复（PRD §8 规则 10、§10.3）
pub const TRASH_KEEP_MS: i64 = 30 * 24 * 3600 * 1000;
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
        /// 界面、CLI 都可以传 "inbox"；默认首个可领取状态
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
        /// 界面在排序状态下拖动：目标状态整块的顺序（人看到的顺序，含被拖的卡片），一起保存为优先级。
        /// 不在目标状态里的 id 忽略；目标状态里没列出的卡片按原顺序补在后面
        #[serde(default)]
        order: Option<Vec<i64>>,
        /// 界面拖进快照省略了卡片的终点状态：放到这张卡片前面（按看到的下一张定位，优先于 index）。不在目标状态里时忽略
        #[serde(default)]
        before: Option<i64>,
        /// 同上，看到的最后一张后面（落在可见列表末尾时）
        #[serde(default)]
        after: Option<i64>,
    },
    Claim {
        #[serde(default)]
        id: Option<i64>,
    },
    Comment {
        id: i64,
        text: String,
    },
    /// 评论并移动（界面「复盘台」的「打回」「回复并放回」）：先发评论，再按 Move 的规则移到 column（最下面）。
    /// 在同一个写事务里：移动被拒绝时评论也不留下
    Reply {
        id: i64,
        text: String,
        /// 状态标识或名称
        column: String,
    },
    /// 人修改自己发的评论（界面）：原位更新内容，记下修改时间，不新增动态
    #[serde(rename = "edit_comment")]
    EditComment {
        id: i64,
        /// 评论在动态里的 ID
        activity: i64,
        text: String,
    },
    /// 人删除自己发的评论（界面）：从动态里移除，不新增动态
    #[serde(rename = "delete_comment")]
    DeleteComment {
        id: i64,
        /// 评论在动态里的 ID
        activity: i64,
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
    /// 删除（仅人）：移进回收站，30 天内可以恢复（规则 10）
    Delete {
        id: i64,
    },
    /// 恢复（仅人）：从回收站恢复到原来状态的最下面（原来的状态已经删了就回到首个可领取状态），不保留领取
    Restore {
        id: i64,
    },
    /// 永久删除（仅人）：从回收站里删掉，附件一起删，不能恢复
    Purge {
        id: i64,
    },
    /// 合并（仅人，界面「未整理」页）：把 from 里的条目并进 id，只留下 id。都要在「未整理」里。
    /// 标题、描述按给的写；不给描述时，目标原文在前，每条来源一节（标题 + 描述 + 合并自哪一条）接在后面。
    /// 标签取并集，来源的附件和评论归到 id，来源条目删除
    Merge {
        id: i64,
        from: Vec<i64>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        body: Option<String>,
    },
    /// 人看过「未整理」里的条目（PRD §10.5）：展开一条、点「全部标为已读」。已经是看过的跳过；
    /// 没给 id 时是这块白板「未整理」里的全部条目。这是界面上的读操作，不新增动态
    Seen {
        #[serde(default)]
        id: Option<i64>,
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

/// 回收站里一张卡片删除前所在的状态和在状态里的位置，以及删除前领取它的那个可领取状态
/// （只有删掉时还被持有的卡片才有；恢复时据此回到「当初领取它的可领取状态」）
struct TrashMeta {
    del_col: Option<String>,
    del_pos: Option<i64>,
    del_from: Option<String>,
    /// 删除前是不是被持有
    held: bool,
}

impl TrashCard {
    /// 永久删除时返回给界面、CLI 的卡片（不含动态和附件）
    fn into_detail(self) -> CardDetail {
        CardDetail { card: self.card, activity: vec![], attachments: vec![] }
    }
}

impl R<'_> {
    /// 回收站里的一张卡片的详情（不在回收站里时为 None）
    pub fn trash_of(&self, board: &BoardRef, id: i64) -> Result<Option<CardDetail>> {
        Ok(self.trash(board)?.into_iter().find(|t| t.card.id == id).map(TrashCard::into_detail))
    }
}

/// 回收站里删除超过 30 天的卡片永久删除（附件一起删）。附件记录随卡片级联删除，不再被引用的文件由 [`Store::gc_files`] 清理
pub fn purge_trash(store: &mut Store, now: i64) -> Result<usize> {
    if store.conn().query_row("SELECT COUNT(*) FROM cards WHERE deleted_at IS NOT NULL AND deleted_at < ?1", [now - TRASH_KEEP_MS], |r| {
        r.get::<_, i64>(0)
    })? == 0
    {
        return Ok(0);
    }
    store.write(now, Origin::system(), |w| w.purge_trash(now - TRASH_KEEP_MS))
}

impl W<'_> {
    /// 回收站最多留 TRASH_MAX 张：多出来的按删除时间先清掉最早的（附件由 gc_files 清理）
    fn trim_trash(&mut self, b: i64, keep: i64) -> Result<()> {
        let extra: Vec<i64> = self.collect(
            "SELECT id FROM cards WHERE board = ?1 AND deleted_at IS NOT NULL AND id != ?2
             ORDER BY deleted_at DESC, id DESC LIMIT -1 OFFSET ?3",
            params![b, keep, TRASH_MAX - 1],
            |r| r.get(0),
        )?;
        for id in extra {
            self.tx.execute("DELETE FROM cards WHERE board = ?1 AND id = ?2", params![b, id])?;
        }
        Ok(())
    }

    /// 删除时间早于 older_than 的回收站卡片永久删除（附件记录一起删，不再被引用的文件由 gc_files 清理）。返回删掉的张数
    pub fn purge_trash(&mut self, older_than: i64) -> Result<usize> {
        Ok(self.tx.execute("DELETE FROM cards WHERE deleted_at IS NOT NULL AND deleted_at < ?1", [older_than])?)
    }
}

impl<'a> W<'a> {
    /// 执行一条卡片命令，返回执行后的卡片（删除时返回删除前的卡片）
    pub fn exec(&mut self, board: &BoardRef, actor: &Actor, cmd: Cmd) -> Result<CardDetail> {
        // 评论并移动：拆成评论、移动两步，都在调用方的这一个写事务里（任何一步出错整个写回滚）
        if let Cmd::Reply { id, text, column } = cmd {
            if text.trim().is_empty() {
                return Err(BoardError::usage("要写意见（评论内容不能为空）"));
            }
            self.exec(board, actor, Cmd::Comment { id, text })?;
            return self.exec(board, actor, Cmd::Move { id, column, top: false, index: None, order: None, before: None, after: None });
        }
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
                let start = flow.home();
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
                self.check_labels(board, &labels)?;
                self.check_human_only(actor, &labels)?;
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
                // 人自己记下的不算「新」（PRD §10.5）
                if actor.is_human() {
                    self.touch_seen(b, &[id])?;
                }
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
                let edited = !changes.is_empty();
                for text in changes {
                    self.track(b, id, actor, "edit", &text)?;
                }
                // 人在界面上打开这一条改过：算看过（PRD §10.5）
                if edited && actor.is_human() {
                    self.touch_seen(b, &[id])?;
                }
                id
            }

            /* ---------------- 移动 / 排序 ---------------- */
            Cmd::Move { id, column, top, index, order, before, after } => {
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
                // 错误节点：agent 可以沿连线移入，只有人能移出（agent 移出去就又会被领走、重试）
                if actor.is_agent() && from != to && flow.is_error(&from) {
                    return Err(BoardError::new(ErrorCode::ForbiddenColumn, format!("只有人能把卡片移出错误节点「{}」", flow.name(&from))));
                }
                if actor.is_agent() && from != to {
                    // 规则 11：「未整理」只归人管
                    if from == INBOX || to == INBOX {
                        return Err(BoardError::new(
                            ErrorCode::ForbiddenColumn,
                            format!("只有人能把卡片移入或移出「{INBOX_NAME}」"),
                        ));
                    }
                    // 规则 12：agent 只能沿连线移动
                    if !flow.has_edge(&from, &to) {
                        return Err(BoardError::new(
                            ErrorCode::ForbiddenColumn,
                            format!("「{}」没有连到「{}」，不能这样移动。「{}」能去：{}", flow.name(&from), flow.name(&to), flow.name(&from), flow.outs(&from)),
                        )
                        .with_detail(json!({ "from": from, "to": flow.get(&from).map(|s| s.to.clone()).unwrap_or_default() })));
                    }
                }
                // 锚点：终点状态的快照省略了更早的卡片（不一定在一头），界面算不出整列里的位置，按看到的相邻卡片定位
                let anchor = match (before, after) {
                    (Some(x), _) | (None, Some(x)) if x != id => {
                        let mut ids = self.col_ids(b, &to)?;
                        ids.retain(|c| *c != id);
                        ids.iter().position(|c| *c == x).map(|p| if before.is_some() { p } else { p + 1 })
                    }
                    _ => None,
                };
                let at = match (anchor.or(index), top) {
                    (Some(i), _) => At::Index(i),
                    (None, true) => At::Top,
                    (None, false) => At::Bottom,
                };
                let (old, mut new) = self.place(b, id, Some(&from), &to, at)?;
                let mut whole = false;
                if let Some(order) = &order {
                    let cur = self.col_ids(b, &to)?;
                    let mut ids: Vec<i64> = Vec::with_capacity(cur.len());
                    for x in order.iter().chain(cur.iter()) {
                        if cur.contains(x) && !ids.contains(x) {
                            ids.push(*x);
                        }
                    }
                    if ids != cur {
                        self.write_col(b, &to, &ids)?;
                        whole = true;
                    }
                    new = ids.iter().position(|x| *x == id).unwrap_or(new);
                }
                // 拖出可领取状态：排队中的派发自动取消（任务下发）
                if from != to && flow.is_claim(&from) && !flow.is_claim(&to) {
                    self.cancel_queued_on_move(b, id, actor)?;
                }
                // 人把「未整理」的条目整理进工作流：算看过（PRD §10.5）
                if from == INBOX && from != to && actor.is_human() {
                    self.touch_seen(b, &[id])?;
                }
                if from != to {
                    // 规则 8：离开锁定状态即释放
                    let released = r.holder.is_some() && !flow.is_lock(&to);
                    let run = if released { self.clear_claim(b, id, "move")? } else { None };
                    let text = if from == INBOX {
                        format!("整理好了，从「{INBOX_NAME}」移到「{}」", flow.name(&to))
                    } else {
                        format!(
                            "从「{}」移到「{}」{}",
                            flow.name(&from),
                            flow.name(&to),
                            if released { format!("，领取已释放{}", took(run)) } else { String::new() }
                        )
                    };
                    self.log_move(b, id, "move", &from, &to, actor, &flow)?;
                    self.track(b, id, actor, "move", &text)?;
                } else if whole {
                    let text = format!("调整了在「{}」中的顺序（第 {} → {} 位），并按排序后看到的顺序保存了整块的优先级", flow.name(&to), old + 1, new + 1);
                    self.track(b, id, actor, "reorder", &text)?;
                } else if old != new {
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
                let mine = self.holding(b, &actor.name)?;
                let already = |m: i64| {
                    BoardError::new(
                        ErrorCode::AlreadyHolding,
                        format!("你已经持有 #{m}，一个 agent 在一块白板上同时只能持有一张卡片"),
                    )
                    .with_detail(json!({ "holding": m }))
                };
                let (target, from) = match id {
                    None => {
                        if let Some(m) = mine {
                            return Err(already(m));
                        }
                        // 规则 1：按工作流顺序，第一个有卡片、并且连到锁定状态的可领取状态；有排队派发的卡片跳过
                        match self.next_claimable(b, &flow, &[])? {
                            Some(f) => f,
                            None => {
                                let names: Vec<String> = flow.0.iter().filter(|s| s.claim).map(|s| format!("「{}」", s.name)).collect();
                                return Err(BoardError::new(
                                    ErrorCode::EmptyTodo,
                                    format!("可领取的状态{}里没有能领取的卡片", names.join("")),
                                ));
                            }
                        }
                    }
                    Some(id) => {
                        let r = self.row(b, id)?;
                        // 再次领取自己持有的卡片：已经是你的，什么也不做
                        if is_holder(&r) {
                            return self.read().card(board, id);
                        }
                        if !flow.is_claim(&r.col) {
                            let names: Vec<String> = flow.0.iter().filter(|s| s.claim).map(|s| format!("「{}」", s.name)).collect();
                            return Err(BoardError::new(
                                ErrorCode::NotInTodo,
                                format!("#{id} 在「{}」，不在可领取的状态{}", flow.name(&r.col), names.join("")),
                            ));
                        }
                        if let Some(m) = mine {
                            return Err(already(m));
                        }
                        // 卡片有排队中的派发：只有这次派发自己能领（任务下发）
                        if !self.dispatch_ok {
                            if let Some((_, host)) = self.queued_dispatch(b, id)? {
                                return Err(BoardError::new(
                                    ErrorCode::Dispatched,
                                    format!("#{id} 已经派发给机器 {host}，正在排队：等它开始，或请人先取消派发"),
                                )
                                .with_detail(json!({ "host": host })));
                            }
                        }
                        (id, r.col)
                    }
                };
                let Some(dest) = flow.claim_target(&from) else {
                    return Err(BoardError::new(
                        ErrorCode::ForbiddenColumn,
                        format!("「{}」没有连到任何锁定状态，不能从这里领取", flow.name(&from)),
                    ));
                };
                self.place(b, target, Some(&from), &dest.key, At::Bottom)?;
                self.tx.execute(
                    "UPDATE cards SET holder = ?1, claimed_at = ?2, beat_at = NULL, final_at = NULL, claimed_from = ?3 WHERE board = ?4 AND id = ?5",
                    params![actor.name, self.now, from, b, target],
                )?;
                self.log_move(b, target, "claim", &from, &dest.key, actor, &flow)?;
                self.track(b, target, actor, "claim", &format!("领取了卡片，从「{}」进入「{}」", flow.name(&from), dest.name))?;
                target
            }

            // 上面已经拆成评论和移动
            Cmd::Reply { id, .. } => id,

            /* ---------------- 评论 ---------------- */
            Cmd::Comment { id, text } => {
                self.row(b, id)?; // 卡片要存在
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err(BoardError::usage("评论内容不能为空"));
                }
                self.bind_attachments(b, id, &text)?;
                self.track(b, id, actor, "comment", &text)?;
                id
            }

            /* ---------------- 修改评论 ----------------
               只有人能改，只能改人发的评论（PRD §8）；agent 和系统的动态不能改 */
            Cmd::EditComment { id, activity, text } => {
                self.row(b, id)?;
                let found: Option<(String, String, String)> = self
                    .tx
                    .query_row(
                        "SELECT kind, actor_kind, actor_name FROM activity WHERE id = ?1 AND board = ?2 AND card = ?3",
                        params![activity, b, id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((kind, by_kind, by_name)) = found else {
                    return Err(BoardError::new(ErrorCode::NotFound, format!("#{id} 没有这条评论")));
                };
                if kind != "comment" {
                    return Err(BoardError::usage("只能修改评论，系统动态不能修改"));
                }
                if !actor.is_human() || by_kind != ActorKind::Human.as_str() || by_name != actor.name {
                    return Err(BoardError::code(ErrorCode::NotAuthor));
                }
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err(BoardError::usage("评论内容不能为空"));
                }
                self.bind_attachments(b, id, &text)?;
                self.tx.execute("UPDATE activity SET text = ?1, edited_at = ?2 WHERE id = ?3", params![text, self.now, activity])?;
                self.tx.execute("UPDATE cards SET updated_at = ?1 WHERE board = ?2 AND id = ?3", params![self.now, b, id])?;
                let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2", params![b, id], |r| r.get(0))?;
                self.event(b, id, &title, actor, "edit_comment", &text)?;
                id
            }

            /* ---------------- 删除评论 ----------------
               只有人能删，只能删人发的评论（PRD §8）；agent 和系统的动态不能删 */
            Cmd::DeleteComment { id, activity } => {
                self.row(b, id)?;
                let found: Option<(String, String, String)> = self
                    .tx
                    .query_row(
                        "SELECT kind, actor_kind, actor_name FROM activity WHERE id = ?1 AND board = ?2 AND card = ?3",
                        params![activity, b, id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((kind, by_kind, by_name)) = found else {
                    return Err(BoardError::new(ErrorCode::NotFound, format!("#{id} 没有这条评论")));
                };
                if kind != "comment" {
                    return Err(BoardError::usage("只能删除评论，系统动态不能删除"));
                }
                if !actor.is_human() || by_kind != ActorKind::Human.as_str() || by_name != actor.name {
                    return Err(BoardError::code(ErrorCode::NotAuthor));
                }
                self.tx.execute("DELETE FROM activity WHERE id = ?1", params![activity])?;
                self.tx.execute("UPDATE cards SET updated_at = ?1 WHERE board = ?2 AND id = ?3", params![self.now, b, id])?;
                let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2", params![b, id], |r| r.get(0))?;
                self.event(b, id, &title, actor, "delete_comment", "删除了评论")?;
                id
            }

            /* ---------------- 标签 ----------------
               任何状态都可以，包括别人持有的卡片；不续期（PRD §6）。仅人可打的标签 agent 不能加、去 */
            Cmd::Label { id, add, remove } => {
                self.row(b, id)?;
                self.check_labels(board, &add)?;
                self.check_labels(board, &remove)?;
                self.check_human_only(actor, &add)?;
                self.check_human_only(actor, &remove)?;
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
               单张卡片的心跳 `board beat <ID>`：效果与 agent 级心跳（`board beat`，见 agents.rs）对这张卡片做的相同。
               final：这一次结束（记耗时），之后 50 秒内要沿连线移走，否则释放；之后 agent 级心跳不再刷新这张卡片。
               final 之后、移走之前再来的普通心跳（例如脚本停止时还在途的那一次）被拒绝：不新开一次、不延长释放窗口；
               要接着做得重新领取。卡片已经不归自己（例如已交审）时 final 什么也不做 */
            Cmd::Beat { id, done } => {
                if !actor.is_agent() {
                    return Err(BoardError::usage("心跳只能由 agent 通过 CLI 发送"));
                }
                let r = self.row(b, id)?;
                if done {
                    if is_holder(&r) {
                        self.count_beat(b, id)?;
                        if let Some((n, ms)) = self.close_run(b, id, self.now, "final")? {
                            self.tx.execute("UPDATE cards SET beat_at = ?1, final_at = ?1 WHERE board = ?2 AND id = ?3", params![self.now, b, id])?;
                            self.track(b, id, actor, "beat", &format!("结束了心跳，正常退出，第 {n} 次耗时 {}", dur_words(ms)))?;
                        }
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
                let finished: Option<i64> =
                    self.tx.query_row("SELECT final_at FROM cards WHERE board = ?1 AND id = ?2", params![b, id], |r| r.get(0))?;
                if finished.is_some() {
                    return Err(BoardError::usage(format!(
                        "#{id} 这次的心跳已经结束（final）：先交审或交给人处理；要接着做，等领取释放后重新 board claim {id}"
                    )));
                }
                self.beat_card(b, id, actor)?;
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
                let start = flow.back_to(self.claimed_from(b, id)?.as_deref()).clone();
                // 卡片被拿走时如果这次派发还在跑，同时请求停止：agent 没有理由继续做，也避免卡片被再领后两个 agent 做同一张
                let running = self.read().card_dispatch(board, id)?;
                self.place(b, id, Some(&r.col), &start.key, At::Top)?;
                let run = self.clear_claim(b, id, "release")?;
                self.log_move(b, id, "release", &r.col, &start.key, actor, &flow)?;
                self.track(b, id, actor, "release", &format!("强制释放了 {who} 的领取，卡片回到「{}」最上面{}", start.name, took(run)))?;
                if let Some(d) = running.filter(|d| d.state == "running") {
                    self.request_stop(d.id, actor, actor.display())?;
                }
                id
            }

            /* ---------------- 删除（仅人，规则 10） ----------------
               移进回收站：卡片从白板、claim、搜索、大盘里消失（这些查询都排除 deleted_at 非空的卡片），
               动态、评论、附件都还在，30 天内可以恢复（规则 10、§10.3 回收站） */
            Cmd::Delete { id } => {
                if !actor.is_human() {
                    return Err(BoardError::usage("删除只能在界面上做"));
                }
                let mut detail = self.read().card(board, id)?;
                let holder = self.holder_of(b, id)?;
                self.on_card_delete(b, id, actor)?;
                // 删除前领取它的那个可领取状态：下面 clear_claim 会把它清空，先取出来存进 del_from，
                // 恢复时靠它回到「当初领取它的可领取状态」（PRD §8 规则 10）
                let from = self.claimed_from(b, id)?;
                // 持有的先释放：进行中的这一次在此时结束
                if let Some(who) = holder.as_deref() {
                    let run = self.clear_claim(b, id, "release")?;
                    detail.card.holder = None;
                    detail.card.claimed_at = None;
                    detail.card.claimed_from = None;
                    self.track(
                        b,
                        id,
                        actor,
                        "release",
                        &format!("删除了卡片，{who} 的领取已释放{}", took(run)),
                    )?;
                }
                self.tx.execute(
                    "UPDATE cards SET deleted_at = ?1, deleted_by_kind = ?2, deleted_by_name = ?3, del_col = ?4, del_pos = ?5,
                            del_from = ?6, del_held = ?7
                     WHERE board = ?8 AND id = ?9",
                    params![
                        self.now,
                        actor.kind.as_str(),
                        actor.name,
                        detail.card.column,
                        self.row_pos(b, id)?,
                        from,
                        holder.is_some() as i64,
                        b,
                        id
                    ],
                )?;
                self.event(b, id, &detail.card.title, actor, "delete", &format!("{} 删除", actor.display()))?;
                self.trim_trash(b, id)?;
                return Ok(detail);
            }

            /* ---------------- 恢复（仅人，规则 10） ----------------
               回到原来状态的最下面（原来的状态已经删了就回到首个可领取状态；原来在锁定状态里的，回到当初领取它的可领取状态）；
               原来的位置空着就放回原来的位置。恢复后不保留领取，动态里记一条 */
            Cmd::Restore { id } => {
                if !actor.is_human() {
                    return Err(BoardError::usage("只有人能从回收站恢复"));
                }
                let Some(row) = self.trash_row(b, id)? else {
                    return Err(BoardError::new(ErrorCode::NotFound, format!("#{id} 不在回收站里")));
                };
                let saved = row.del_col.clone().unwrap_or_default();
                // 删除前在锁定状态（被删时还在做）：回到当初领取它的可领取状态；
                // 「未整理」不在工作流里，之前在这里判断不到，被当成「原来所在的状态已经删除」扔回待办
                let (mut target, at) = if saved == INBOX {
                    (saved.clone(), At::Index(row.del_pos.unwrap_or(0).max(0) as usize))
                } else if let Some(s) = flow.get(&saved) {
                    if s.lock {
                        let from = row.del_from.clone().filter(|k| flow.is_claim(k)).unwrap_or_else(|| flow.home().key.clone());
                        (from, At::Bottom)
                    } else {
                        // 还在原来的状态里：原来的位置上排第几就放回第几（还是原来那一列时才不会和别的卡片撞位）
                        (saved.clone(), At::Index(row.del_pos.unwrap_or(0).max(0) as usize))
                    }
                } else {
                    // 原来的状态已经删了：首个可领取状态最下面
                    (flow.home().key.clone(), At::Bottom)
                };
                // 「未整理」不在工作流里，是合法的去处
                if target != INBOX && !flow.is_claim(&target) && flow.get(&target).is_none() {
                    target = flow.home().key.clone();
                }
                self.tx.execute(
                    "UPDATE cards SET deleted_at = NULL, deleted_by_kind = NULL, deleted_by_name = NULL, del_col = NULL, del_pos = NULL,
                            del_from = NULL, del_held = NULL,
                            holder = NULL, claimed_at = NULL, beat_at = NULL, final_at = NULL, claimed_from = NULL, moved_at = ?1
                     WHERE board = ?2 AND id = ?3",
                    params![self.now, b, id],
                )?;
                self.place(b, id, None, &target, at)?;
                self.changed = true;
                let text = if target == saved {
                    format!("从回收站恢复，放回「{}」", flow.name(&target))
                } else if flow.get(&saved).is_none() {
                    format!("从回收站恢复，原来所在的状态已经删除，放回「{}」", flow.name(&target))
                } else if row.held {
                    format!("从回收站恢复，原来在「{}」（被删时还在做），放回「{}」", flow.name(&saved), flow.name(&target))
                } else {
                    format!("从回收站恢复，原来在「{}」，放回「{}」", flow.name(&saved), flow.name(&target))
                };
                self.log_move(b, id, "restore", &target, &target, actor, &flow)?;
                self.track(b, id, actor, "restore", &text)?;
                id
            }

            /* ---------------- 永久删除（仅人） ----------------
               和删除前一样：排队中的派发取消，正在跑的请求停止；卡片、动态、评论、附件一并删除 */
            Cmd::Purge { id } => {
                if !actor.is_human() {
                    return Err(BoardError::usage("永久删除只能在界面上做"));
                }
                let Some(detail) = self.read().trash_of(board, id)? else {
                    return Err(BoardError::new(ErrorCode::NotFound, format!("#{id} 不在回收站里")));
                };
                self.on_card_delete(b, id, actor)?;
                self.event(b, id, &detail.card.title, actor, "purge", "永久删除了卡片")?;
                self.tx.execute("DELETE FROM cards WHERE board = ?1 AND id = ?2", params![b, id])?;
                return Ok(detail);
            }

            /* ---------------- 合并（仅人，只在「未整理」里） ---------------- */
            Cmd::Merge { id, from, title, body } => {
                if !actor.is_human() {
                    return Err(BoardError::usage("合并只能在界面上做"));
                }
                let from: Vec<i64> = dedup(&from).into_iter().filter(|x| *x != id).collect();
                if from.is_empty() {
                    return Err(BoardError::usage("至少要选一条别的条目并进来"));
                }
                let target = self.row(b, id)?;
                // 「看过」要在删掉来源、把动态挪过去之前取（PRD §10.5）
                let target_seen = self.seen_at(b, id)?.is_some();
                let mut srcs: Vec<(i64, Row)> = Vec::with_capacity(from.len());
                let mut seen: Vec<bool> = Vec::with_capacity(from.len());
                for f in &from {
                    srcs.push((*f, self.row(b, *f)?));
                    seen.push(self.seen_at(b, *f)?.is_some());
                }
                for (cid, r) in std::iter::once((id, &target)).chain(srcs.iter().map(|(i, r)| (*i, r))) {
                    if r.col != INBOX {
                        return Err(BoardError::usage(format!(
                            "#{cid} 在「{}」：只能合并「{INBOX_NAME}」里的条目",
                            flow.name(&r.col)
                        )));
                    }
                }
                let title = match title {
                    Some(t) => {
                        let t = t.trim().to_string();
                        if t.is_empty() {
                            return Err(BoardError::code(ErrorCode::EmptyTitle));
                        }
                        t
                    }
                    None => target.title.clone(),
                };
                let body = match body {
                    Some(text) => text.trim().to_string(),
                    None => {
                        let mut parts: Vec<String> = vec![target.body.trim().to_string()];
                        for (f, r) in &srcs {
                            let who: (String, String) = self.tx.query_row(
                                "SELECT created_by_kind, created_by_name FROM cards WHERE board = ?1 AND id = ?2",
                                params![b, f],
                                |x| Ok((x.get(0)?, x.get(1)?)),
                            )?;
                            let who = if who.0 == ActorKind::Agent.as_str() { format!(" · {}", who.1) } else { String::new() };
                            parts.push(
                                [format!("## {}", r.title), r.body.trim().to_string(), format!("> 合并自 #{f}{who}")]
                                    .into_iter()
                                    .filter(|x| !x.is_empty())
                                    .collect::<Vec<_>>()
                                    .join("\n\n"),
                            );
                        }
                        parts.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n\n")
                    }
                };
                for (f, r) in &srcs {
                    // 标签取并集；附件、评论归到留下的条目（描述里引用的图片不断）
                    self.tx.execute(
                        "INSERT OR IGNORE INTO card_labels (board, card, label) SELECT board, ?1, label FROM card_labels WHERE board = ?2 AND card = ?3",
                        params![id, b, f],
                    )?;
                    self.tx.execute("UPDATE attachments SET card = ?1 WHERE board = ?2 AND card = ?3", params![id, b, f])?;
                    self.tx.execute("UPDATE activity SET card = ?1 WHERE board = ?2 AND card = ?3 AND kind = 'comment'", params![id, b, f])?;
                    self.on_card_delete(b, *f, actor)?;
                    self.event(b, *f, &r.title, actor, "delete", &format!("合并到 #{id}"))?;
                    self.tx.execute("DELETE FROM cards WHERE board = ?1 AND id = ?2", params![b, f])?;
                }
                self.tx.execute("UPDATE cards SET title = ?1, body = ?2 WHERE board = ?3 AND id = ?4", params![title, body, b, id])?;
                self.bind_attachments(b, id, &body)?;
                let names: Vec<String> = srcs.iter().map(|(f, r)| format!("#{f}「{}」", r.title)).collect();
                let mut text = format!("合并了 {}", names.join("、"));
                if title != target.title {
                    text.push_str(&format!("，标题改为「{title}」"));
                }
                self.track(b, id, actor, "merge", &text)?;
                // 并进来的条目里有一条人看过：留下的这条也算看过（来源已经不在「未整理」里，点也没了）
                if !target_seen && seen.into_iter().any(|s| s) {
                    self.touch_seen(b, &[id])?;
                }
                id
            }

            /* ---------------- 看过（仅人，只在「未整理」里；PRD §10.5） ----------------
               展开一条、点「全部标为已读」都走这里：记下人看过的时间，条目的点消失，同步到所有窗口和机器。
               不新增动态；已经看过的不再刷新时间（重复执行、别的窗口也在看时结果不变） */
            Cmd::Seen { id } => {
                if !actor.is_human() {
                    return Err(BoardError::usage("「看过」只能在界面上做"));
                }
                let ids: Vec<i64> = match id {
                    Some(id) => {
                        let r = self.row(b, id)?;
                        if r.col != INBOX {
                            return Err(BoardError::usage(format!("#{id} 在「{}」，不在「{INBOX_NAME}」里", flow.name(&r.col))));
                        }
                        vec![id]
                    }
                    None => self.collect("SELECT id FROM cards WHERE board = ?1 AND col = ?2 AND seen_at IS NULL", params![b, INBOX], |r| r.get(0))?,
                };
                self.touch_seen(b, &ids)?;
                return match id {
                    Some(id) => self.read().card(board, id),
                    // 全部标为已读：不是对某一张卡片的操作，返回一块空白
                    None => Ok(CardDetail { card: blank_card(), activity: vec![], attachments: vec![] }),
                };
            }
        };
        self.read().card(board, id)
    }

    /// 规则 7：从领取或最近一次心跳算起，连续 5 次（50 秒）没收到心跳，领取释放，卡片回到领取它时所在的可领取状态最上面
    pub(crate) fn sweep_board(&mut self, b: i64) -> Result<usize> {
        let gap = BEAT_MS * BEAT_MISSES;
        // 最早断掉的最后插入 → 位于最上面
        let due: Vec<(i64, String, String, Option<i64>)> = self.collect(
            "SELECT id, holder, col, beat_at FROM cards
             WHERE board = ?1 AND holder IS NOT NULL AND COALESCE(beat_at, claimed_at, 0) + ?3 <= ?2
             ORDER BY COALESCE(beat_at, claimed_at, 0) DESC, id DESC",
            params![b, self.now, gap],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        if due.is_empty() {
            return Ok(0);
        }
        let flow = crate::store::flow_of(self.tx, b)?;
        for (id, who, col, beat) in &due {
            let start = flow.back_to(self.claimed_from(b, *id)?.as_deref()).clone();
            self.place(b, *id, Some(col), &start.key, At::Top)?;
            // 进行中的这一次以最后一次心跳为结束
            let run = match beat {
                Some(t) => self.close_run(b, *id, *t, "lost")?,
                None => None,
            };
            self.clear_claim(b, *id, "lost")?;
            let back = format!("卡片回到「{}」最上面", start.name);
            let (action, text) = match (run, beat) {
                (Some((n, ms)), _) => (
                    "lost",
                    format!("心跳中断：连续 {BEAT_MISSES} 次没有收到 {who} 的心跳，判定异常退出，第 {n} 次耗时约 {}，{back}", dur_words(ms)),
                ),
                (None, Some(_)) => ("timeout", format!("{who} 结束心跳后 {} 秒内没有交审或放回，{back}", gap / 1000)),
                (None, None) => ("timeout", format!("{who} 领取后 {} 秒内没有开始心跳，{back}", gap / 1000)),
            };
            self.log_move(b, *id, action, col, &start.key, &Actor::system(), &flow)?;
            self.track(b, *id, &Actor::system(), action, &text)?;
        }
        Ok(due.len())
    }

    /* ---------------- 辅助 ---------------- */

    fn row(&self, b: i64, id: i64) -> Result<Row> {
        self.tx
            .query_row(
                "SELECT col, holder, title, body FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL",
                params![b, id],
                |r| Ok(Row { col: r.get(0)?, holder: r.get(1)?, title: r.get(2)?, body: r.get(3)? }),
            )
            .optional()?
            .ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("卡片 #{id} 不存在")))
    }

    /// 记下人看过这些条目的时间（PRD §10.5）。已经看过的不刷新；「未整理」以外的卡片不管；
    /// 真的改了才让 rev 加一（界面收到推送后刷新，点消失）
    fn touch_seen(&mut self, b: i64, ids: &[i64]) -> Result<()> {
        let mut st = self.tx.prepare_cached("UPDATE cards SET seen_at = ?1 WHERE board = ?2 AND id = ?3 AND col = ?4 AND seen_at IS NULL")?;
        let mut changed = false;
        for id in ids {
            changed |= st.execute(params![self.now, b, id, INBOX])? > 0;
        }
        if changed {
            self.changed = true;
        }
        Ok(())
    }

    fn seen_at(&self, b: i64, id: i64) -> Result<Option<i64>> {
        Ok(self
            .tx
            .query_row("SELECT seen_at FROM cards WHERE board = ?1 AND id = ?2", params![b, id], |r| r.get(0))
            .optional()?
            .flatten())
    }

    /// 卡片现在的持有者（卡片自己身上的 holder 列）
    fn holder_of(&self, b: i64, id: i64) -> Result<Option<String>> {
        Ok(self.tx.query_row("SELECT holder FROM cards WHERE board = ?1 AND id = ?2", params![b, id], |r| r.get(0))?)
    }

    /// 卡片在所属状态里的位置（pos）；不在回收站里时才能取
    fn row_pos(&self, b: i64, id: i64) -> Result<i64> {
        Ok(self.tx.query_row("SELECT pos FROM cards WHERE board = ?1 AND id = ?2", params![b, id], |r| r.get(0))?)
    }

    /// 回收站里这张卡片删除前在哪个状态、排第几位、领取它的那个可领取状态（不在回收站里时为 None）
    fn trash_row(&self, b: i64, id: i64) -> Result<Option<TrashMeta>> {
        Ok(self
            .tx
            .query_row(
                "SELECT del_col, del_pos, del_from, del_held FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NOT NULL",
                params![b, id],
                |r| {
                    Ok(TrashMeta {
                        del_col: r.get(0)?,
                        del_pos: r.get(1)?,
                        del_from: r.get(2)?,
                        held: r.get::<_, Option<bool>>(3)?.unwrap_or(false),
                    })
                },
            )
            .optional()?)
    }

    pub(crate) fn col_ids(&self, b: i64, col: &str) -> Result<Vec<i64>> {
        self.collect(
            "SELECT id FROM cards WHERE board = ?1 AND col = ?2 AND deleted_at IS NULL ORDER BY pos, id",
            params![b, col],
            |r| r.get(0),
        )
    }

    /// 卡片在状态里的位置（从 0 数）；不在这个状态时为 None
    fn rank(&self, b: i64, col: &str, id: i64) -> Result<Option<usize>> {
        let n: Option<i64> = self
            .tx
            .query_row(
                "SELECT (SELECT COUNT(*) FROM cards d WHERE d.board = c.board AND d.col = c.col AND d.deleted_at IS NULL
                           AND (d.pos < c.pos OR (d.pos = c.pos AND d.id < c.id)))
                 FROM cards c WHERE c.board = ?1 AND c.col = ?2 AND c.id = ?3",
                params![b, col, id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(n.map(|n| n as usize))
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
        // 换了状态（含新建）：记下移入时间，界面「保留顺序」按移入时间排时用；同一状态里调整顺序不算
        if from != Some(to) {
            self.tx.execute("UPDATE cards SET moved_at = ?1 WHERE board = ?2 AND id = ?3", params![self.now, b, id])?;
        }
        if let At::Bottom = at {
            // 放到最下面：只改这一张（MAX(pos) + 1），不重写整列；终点状态的卡片只增不减，重写整列的耗时随张数增长。
            // 离开的状态留下的空位不影响顺序（按 pos 排）
            let old = match from {
                Some(f) => Some(self.rank(b, f, id)?),
                None => None,
            };
            let (n, max): (i64, Option<i64>) = self.tx.query_row(
                "SELECT COUNT(*), MAX(pos) FROM cards WHERE board = ?1 AND col = ?2 AND id != ?3",
                params![b, to, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            self.tx.execute(
                "UPDATE cards SET col = ?1, pos = ?2 WHERE board = ?3 AND id = ?4",
                params![to, max.map_or(0, |m| m + 1), b, id],
            )?;
            self.changed = true;
            return Ok((old.flatten().unwrap_or(0), n as usize));
        }
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

    /// 领取它时所在的可领取状态
    fn claimed_from(&self, b: i64, id: i64) -> Result<Option<String>> {
        Ok(self.tx.query_row("SELECT claimed_from FROM cards WHERE board = ?1 AND id = ?2", params![b, id], |r| r.get(0))?)
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

    /// 持有者对一张还没 final 的卡片的一次心跳：更新心跳时间并计入心跳次数；没有进行中的一次时，
    /// 这一次心跳就是开始（动态里记「开始第 n 次」）。
    /// 只更新心跳时间不算变更（rev 不变，不推送给界面），开始新的一次才算（PRD §11）
    pub(crate) fn beat_card(&mut self, b: i64, id: i64, actor: &Actor) -> Result<()> {
        self.tx.execute("UPDATE cards SET beat_at = ?1 WHERE board = ?2 AND id = ?3", params![self.now, b, id])?;
        let open: bool = self.tx.query_row(
            "SELECT COUNT(*) > 0 FROM card_runs WHERE board = ?1 AND card = ?2 AND ended_at IS NULL",
            params![b, id],
            |r| r.get(0),
        )?;
        if !open {
            let n: i64 =
                self.tx.query_row("SELECT COUNT(*) + 1 FROM card_runs WHERE board = ?1 AND card = ?2", params![b, id], |r| r.get(0))?;
            self.tx.execute(
                "INSERT INTO card_runs (board, card, n, agent, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![b, id, n, actor.name, self.now],
            )?;
            self.track(
                b,
                id,
                actor,
                "beat",
                &format!("开始第 {n} 次（第一次心跳；每 {} 秒一次，连续 {BEAT_MISSES} 次没收到判定异常退出）", BEAT_MS / 1000),
            )?;
        }
        self.count_beat(b, id)
    }

    /// 进行中的这一次收到一次心跳
    pub(crate) fn count_beat(&mut self, b: i64, id: i64) -> Result<()> {
        self.tx.execute("UPDATE card_runs SET beats = beats + 1 WHERE board = ?1 AND card = ?2 AND ended_at IS NULL", params![b, id])?;
        Ok(())
    }

    /// 结束进行中的这一次，返回（第几次，耗时）；没有进行中的一次时返回 None
    pub(crate) fn close_run(&mut self, b: i64, id: i64, end: i64, how: &str) -> Result<Option<(i64, i64)>> {
        let open: Option<(i64, i64)> = self
            .tx
            .query_row(
                "SELECT n, started_at FROM card_runs WHERE board = ?1 AND card = ?2 AND ended_at IS NULL",
                params![b, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((n, started)) = open else { return Ok(None) };
        let end = end.max(started);
        self.tx.execute(
            "UPDATE card_runs SET ended_at = ?1, how = ?2 WHERE board = ?3 AND card = ?4 AND n = ?5",
            params![end, how, b, id, n],
        )?;
        self.changed = true;
        Ok(Some((n, end - started)))
    }

    /// 释放领取；进行中的这一次在现在结束。返回结束的那一次（第几次，耗时）
    pub(crate) fn clear_claim(&mut self, b: i64, id: i64, how: &str) -> Result<Option<(i64, i64)>> {
        let run = self.close_run(b, id, self.now, how)?;
        self.tx.execute(
            "UPDATE cards SET holder = NULL, claimed_at = NULL, beat_at = NULL, final_at = NULL, claimed_from = NULL WHERE board = ?1 AND id = ?2",
            params![b, id],
        )?;
        self.changed = true;
        Ok(run)
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

    /// 仅人可打的标签只有人能加、去：agent 碰到时拒绝（USAGE），原因里写明标签名
    fn check_human_only(&self, actor: &Actor, ids: &[i64]) -> Result<()> {
        if !actor.is_agent() {
            return Ok(());
        }
        for id in ids {
            let (name, human_only): (String, bool) =
                self.tx.query_row("SELECT name, human_only FROM labels WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if human_only {
                return Err(BoardError::usage(format!("标签「{name}」只有人能打（仅人可打），agent 不能加、去")));
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

/// 不对应任何卡片的命令（「全部标为已读」）返回的空白卡片
fn blank_card() -> Card {
    Card {
        id: 0,
        title: String::new(),
        body: String::new(),
        column: String::new(),
        holder: None,
        claimed_at: None,
        beat_at: None,
        run_started_at: None,
        work_ms: 0,
        runs: 0,
        runs_lost: 0,
        beats: 0,
        finished_at: None,
        claimed_from: None,
        labels: vec![],
        created_at: 0,
        updated_at: 0,
        moved_at: 0,
        created_by: Actor::system(),
        comments: 0,
        seen_at: None,
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

/// 动态里的「，第 N 次耗时 X」
fn took(run: Option<(i64, i64)>) -> String {
    run.map(|(n, ms)| format!("，第 {n} 次耗时 {}", dur_words(ms))).unwrap_or_default()
}

/// 23 分 10 秒 / 1 小时 5 分 / 8 秒
pub fn dur_words(ms: i64) -> String {
    let t = ms.max(0) / 1000; // 与界面的计时一致：不足一秒的部分舍去
    let (h, m, s) = (t / 3600, t % 3600 / 60, t % 60);
    if h > 0 {
        format!("{h} 小时 {m} 分")
    } else if m > 0 {
        format!("{m} 分 {s} 秒")
    } else {
        format!("{s} 秒")
    }
}
