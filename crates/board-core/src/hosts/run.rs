//! 派发的执行：排队和空位、会话接着做、开始、结束和结算、日志和结构化事件。

use super::*;

/// 异常退出、超时时 board host 报的最后的输出：原始输出最后 20 行，和最后 10 个事件的文字（旧版 board host 没有）
pub(super) struct Tail<'a> {
    pub(super) lines: &'a [String],
    pub(super) events: &'a [String],
    /// board acp 看到的额度（#668）；旧版 board host 不报
    pub(super) quota: Option<&'a Quota>,
}

impl Tail<'_> {
    /// 评论里接在原因后面的部分：有事件时写「最后 N 个事件」，原始输出折叠在后面；没有事件时照旧写「日志最后 N 行」
    pub(super) fn words(&self) -> String {
        let fence = |ls: &[String]| format!("```\n{}\n```", ls.join("\n").replace("```", "ˋˋˋ"));
        let events: Vec<String> = self.events.iter().filter(|l| !l.trim().is_empty()).cloned().collect();
        if events.is_empty() {
            if self.lines.is_empty() {
                return String::new();
            }
            return format!("\n\n日志最后 {} 行：\n\n{}", self.lines.len(), fence(self.lines));
        }
        let mut t = format!("\n\n最后 {} 个事件：\n\n{}", events.len(), fence(&events));
        if !self.lines.is_empty() {
            t += &format!("\n\n<details><summary>原始输出最后 {} 行</summary>\n\n{}\n\n</details>", self.lines.len(), fence(self.lines));
        }
        t
    }
}

/// 派发是怎么结束的
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Finish<'a> {
    /// agent 进程退出
    Exit,
    /// board host 退出前结束了 agent 进程
    HostExit,
    /// board host 按机器的兜底设置结束了 agent 进程（运行超时 / 长时间没有输出）；带给人看的原因
    Timeout(Option<&'a str>),
    /// board host 重启后，本机没有这次派发的进程
    Missing,
}

/// `board next` 的 SESSION_DONE：这个会话不再接着做
pub(super) fn session_done_error(why: &str) -> BoardError {
    BoardError::new(ErrorCode::SessionDone, format!("{why}：结束这个会话"))
}

/// 规则拒绝不算失败（存储故障照样报错）
pub(super) fn soft<T>(r: Result<T>) -> Result<()> {
    match r {
        Err(e) if e.code == ErrorCode::Internal => Err(e),
        _ => Ok(()),
    }
}

pub(super) fn found_list(h: &HostRow) -> String {
    let v: Vec<String> = h.resolved().into_iter().filter(|a| a.found).map(|a| a.id).collect();
    if v.is_empty() { "（一个也没找到）".into() } else { v.join("、") }
}

impl<'a> W<'a> {
    pub(super) fn drow(&self, id: i64) -> Result<DRow> {
        self.tx
            .query_row(
                "SELECT id, board, card, host_id, host, agent, agent_label, name, source, state, stop_by, started_at, env_name, env, COALESCE(session, id), model, effort, kind, path, any_host, any_agent, profile, profile_name FROM dispatches WHERE id = ?1",
                [id],
                |r| {
                    Ok(DRow {
                        id: r.get(0)?,
                        board: r.get(1)?,
                        card: r.get(2)?,
                        host_id: r.get(3)?,
                        host: r.get(4)?,
                        agent: r.get(5)?,
                        agent_label: r.get(6)?,
                        name: r.get(7)?,
                        source: r.get(8)?,
                        state: r.get(9)?,
                        stop_by: r.get(10)?,
                        started_at: r.get(11)?,
                        env_name: r.get(12)?,
                        env: r.get(13)?,
                        session: r.get(14)?,
                        model: r.get(15)?,
                        effort: r.get(16)?,
                        kind: r.get(17)?,
                        path: r.get(18)?,
                        any_host: r.get::<_, i64>(19)? != 0,
                        any_agent: r.get::<_, i64>(20)? != 0,
                        profile: r.get(21)?,
                        profile_name: r.get(22)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("派发 #{id} 不存在")))
    }

    pub(super) fn board_of(&self, id: i64) -> Result<BoardRef> {
        Ok(self.tx.query_row("SELECT id, project, key FROM boards WHERE id = ?1", [id], |r| {
            Ok(BoardRef { id: r.get(0)?, project: r.get(1)?, key: r.get(2)? })
        })?)
    }

    pub(super) fn card_exists(&self, b: i64, id: i64) -> Result<bool> {
        Ok(self.tx.query_row("SELECT COUNT(*) FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![b, id], |r| r.get::<_, i64>(0))? > 0)
    }

    /// 在卡片动态里记一条派发的事；卡片已经删除时不记
    pub(super) fn track_dispatch(&mut self, b: i64, card: i64, who: &Actor, action: &str, text: &str) -> Result<()> {
        if self.card_exists(b, card)? {
            self.track(b, card, who, action, text)?;
        }
        Ok(())
    }

    pub(crate) fn touch_hosts(&mut self) {
        self.hosts_changed = true;
        self.changed = true;
    }

    /// 卡片排队中的派发（别的 agent 领取时用）
    pub(crate) fn queued_dispatch(&self, b: i64, card: i64) -> Result<Option<(i64, String)>> {
        Ok(self
            .tx
            .query_row(
                "SELECT id, host FROM dispatches WHERE board = ?1 AND card = ?2 AND state = 'queued' ORDER BY id LIMIT 1",
                params![b, card],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// 派发排队：卡片沿连线进入领取时会进的锁定状态（排在那一块最下面），没有持有者，`claimed_from` 记下原来的状态。
    /// 排队的卡片别人领不到，留在可领取状态里看起来像「谁都能领」（PRD §9.1「占用」）。
    /// 人从可领取状态以外（待审核、错误节点……）派发时一步进 [`Self::dispatch_entry`] 的锁定状态，中间不经过可领取状态。
    /// 卡片在「未整理」、锁定状态、被持有、或者那个状态没有连到锁定状态时不动。返回进入的状态名称（写进动态）
    pub(super) fn reserve(&mut self, b: i64, card: i64, by: &Actor) -> Result<Option<String>> {
        let row: Option<(String, Option<String>)> = self
            .tx
            .query_row("SELECT col, holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![b, card], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((col, None)) = row else { return Ok(None) };
        let flow = flow_of(self.tx, b)?;
        let dest = if flow.is_claim(&col) {
            flow.claim_target(&col).cloned()
        } else if col != INBOX && !flow.is_lock(&col) {
            // 人从别的状态派发（dispatch_card_check 已经认过）：直接进锁定状态，claimed_from 记原来的状态，取消时回那里
            Self::dispatch_entry(&flow).map(|(_, t)| t.clone())
        } else {
            None
        };
        let Some(dest) = dest else { return Ok(None) };
        self.place(b, card, Some(&col), &dest.key, At::Bottom)?;
        self.tx.execute("UPDATE cards SET claimed_from = ?1 WHERE board = ?2 AND id = ?3", params![col, b, card])?;
        self.log_move(b, card, "dispatch", &col, &dest.key, by, &flow)?;
        Ok(Some(dest.name))
    }

    /// 排队的派发没开始就结束了（取消、没领到、会话结束、移除机器）：卡片还在排队时进的锁定状态里、没有人持有时，
    /// 回到原来的可领取状态最上面（和释放一样，PRD §8 规则 7）；已经不在锁定状态里的（在可领取状态里——例如锁定状态被删掉时
    /// 被移了回去，或在错误节点、需要人处理的状态、「未整理」里）只清掉记录、不挪它：错误节点只有人能移出（#428）。
    /// 返回写进动态的后半句（没有移动时为空）
    pub(super) fn unreserve(&mut self, b: i64, card: i64, by: &Actor) -> Result<String> {
        let row: Option<(String, Option<String>, Option<String>)> = self
            .tx
            .query_row(
                "SELECT col, holder, claimed_from FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL",
                params![b, card],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((col, None, from)) = row else { return Ok(String::new()) };
        let flow = flow_of(self.tx, b)?;
        // 没有持有者时 claimed_from 只在派发排队时记（释放会清掉它）；锁定状态被删掉时它也被清掉，这时按「在锁定状态里」认
        if from.is_none() && !flow.is_lock(&col) {
            return Ok(String::new());
        }
        self.tx.execute("UPDATE cards SET claimed_from = NULL WHERE board = ?1 AND id = ?2", params![b, card])?;
        if col == INBOX || !flow.is_lock(&col) {
            return Ok(String::new());
        }
        // 回到派发前的状态：可领取状态，或人派发时所在的别的状态（错误节点、待审核……）；那个状态没了时回首个可领取状态
        let dest = from.as_deref().and_then(|k| flow.get(k)).filter(|s| !s.lock).unwrap_or_else(|| flow.home()).clone();
        self.place(b, card, Some(&col), &dest.key, At::Top)?;
        self.log_move(b, card, "dispatch", &col, &dest.key, by, &flow)?;
        Ok(format!("，卡片回到「{}」最上面", dest.name))
    }

    /// 锁定状态里被持有的卡片排在派发排队的卡片前面：领取进来（含排队后原地开始）时放在第一张排队的卡片前面，没有就放到最下面
    pub(crate) fn lock_slot(&self, b: i64, col: &str, card: i64) -> Result<At> {
        let queued: Vec<i64> = self.collect("SELECT card FROM dispatches WHERE board = ?1 AND state = 'queued'", [b], |r| r.get(0))?;
        if queued.is_empty() {
            return Ok(At::Bottom);
        }
        let ids: Vec<i64> = self.col_ids(b, col)?.into_iter().filter(|x| *x != card).collect();
        Ok(match ids.iter().position(|x| queued.contains(x)) {
            Some(i) => At::Index(i),
            None => At::Bottom,
        })
    }

    /// 按规则 1 找下一张可领取的卡片：在连到锁定状态的可领取状态里（按工作流顺序，状态内从上到下），
    /// 没有排队或在跑的派发的卡片中，带优先标签的先领；都不带时取最前面那张。
    /// `skip` 是调用方额外要跳过的卡片（定时领取传这一轮已经派发过的卡片）。
    pub(crate) fn next_claimable(&self, b: i64, flow: &Flow, skip: &[i64]) -> Result<Option<(i64, String)>> {
        let list = self.claimable_order(b, flow, skip)?;
        let urgent = self.priority_cards(b)?;
        Ok(list.iter().find(|(id, _)| urgent.contains(id)).or(list.first()).cloned())
    }

    /// 可领取的卡片，按规则 1 的顺序（连到锁定状态的可领取状态按工作流顺序、状态内从上到下；错误节点、锁定、终点、
    /// 「未整理」都不算，被持有的也不算），去掉有排队或在跑的派发的和 `skip` 里的；第二项是所在状态。
    /// 在跑的也要去掉（#428）：人强制释放后卡片已经回到可领取状态，旧派发的 agent 还没退出，这时再领就是两个 agent 做同一张，
    /// 自动领取会一轮接一轮地重派。等旧派发结束（停下、或机器掉线后被收掉）再领
    pub(super) fn claimable_order(&self, b: i64, flow: &Flow, skip: &[i64]) -> Result<Vec<(i64, String)>> {
        let mut taken: Vec<i64> =
            self.collect("SELECT card FROM dispatches WHERE board = ?1 AND state IN ('queued', 'running')", [b], |r| r.get(0))?;
        taken.extend_from_slice(skip);
        let mut out = vec![];
        for s in flow.0.iter().filter(|s| s.claim && !s.fin && flow.claim_target(&s.key).is_some()) {
            let ids: Vec<i64> = self.collect(
                "SELECT id FROM cards WHERE board = ?1 AND col = ?2 AND holder IS NULL AND deleted_at IS NULL ORDER BY pos, id",
                params![b, s.key],
                |r| r.get(0),
            )?;
            out.extend(ids.into_iter().filter(|id| !taken.contains(id)).map(|id| (id, s.key.clone())));
        }
        Ok(out)
    }

    /// 带优先标签的卡片
    pub(super) fn priority_cards(&self, b: i64) -> Result<Vec<i64>> {
        self.collect(
            "SELECT DISTINCT cl.card FROM card_labels cl JOIN labels l ON l.id = cl.label WHERE cl.board = ?1 AND l.priority = 1",
            [b],
            |r| r.get(0),
        )
    }

    /// `board next`（只在 board host 启动的会话里用）：结算这个名字正在跑的派发，按开会话时排好的顺序开始这个会话排着的
    /// 下一张（以同一个名字领取，同一台机器、agent、启动环境）。都在一个写事务里。领取失败的那张结果「没领到」，接着取再下一张。
    ///
    /// 返回 `Ok(Err(SESSION_DONE))` 时这个会话不再接着做：还排着的派发已经取消、卡片不动。闭包返回错误会让写事务回滚，
    /// 所以调用方先提交、再把这个错误交给 agent（见 [`crate::Store::next`]）。
    pub fn dispatch_next(&mut self, name: &str) -> Result<std::result::Result<CardDetail, BoardError>> {
        let cur: Option<i64> = self
            .tx
            .query_row("SELECT id FROM dispatches WHERE name = ?1 AND state = 'running' ORDER BY id DESC LIMIT 1", [name], |r| r.get(0))
            .optional()?;
        let Some(cur) = cur else {
            return Err(BoardError::usage(format!(
                "board next 只能在 board host 启动的会话里用：{name} 没有正在跑的派发"
            )));
        };
        let d = self.drow(cur)?;
        let board = self.board_of(d.board)?;
        let holder: Option<String> = self
            .tx
            .query_row("SELECT holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0))
            .optional()?
            .flatten();
        if holder.as_deref() == Some(name) {
            return Err(BoardError::new(
                ErrorCode::AlreadyHolding,
                format!("#{0} 还由你持有：先 board submit {0}（做不下去 board giveup {0}），再 board next", d.card),
            ));
        }
        let h = self.host_row(&d.host)?;
        let why = if d.source != "cron" {
            Some(format!("这个会话是{}派发启动的，只做指定的那一张", source_words(&d.source)))
        } else if d.stop_by.is_some() {
            Some("有人停止了这个会话".into())
        } else {
            match &h {
                None => Some(format!("机器「{}」已被移除", d.host)),
                Some(h) if h.paused => Some(format!("机器「{}」暂停接单", h.name)),
                // 规则被删光：做完当前这张就结束，排着的取消
                Some(h) if h.claim_rules(self.now).is_none() => Some(format!("机器「{}」已经没有领取规则", h.name)),
                Some(_) => None,
            }
        };
        if let Some(why) = why {
            return self.session_done(&d, &why);
        }
        let h = h.expect("上面已经处理了机器不存在");
        let queue: Vec<i64> =
            self.collect("SELECT id FROM dispatches WHERE session = ?1 AND state = 'queued' ORDER BY id", [d.session], |r| r.get(0))?;
        if queue.is_empty() {
            return self.session_done(&d, "这个会话排着的卡片都开始过或取消了");
        }
        // 结算上一张：卡片已经不由这个名字持有，在交审状态记「已交审」，其他状态记「做不下去」
        let flow = flow_of(self.tx, d.board)?;
        let col: Option<String> =
            self.tx.query_row("SELECT col FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0)).optional()?;
        let result = if col.as_deref().is_some_and(|c| flow.is_review(c)) { "review" } else { "stuck" };
        self.close_row(&d, result, None)?;
        let actor = Actor::agent(name.to_string());
        let took = d.started_at.map(|t| format!("，用时 {}", crate::engine::dur_words(self.now - t))).unwrap_or_default();
        let text = format!("派发结束：{}（{}{took}），接着做下一张", result_words(result), Self::dispatch_words(&d));
        self.track_dispatch(d.board, d.card, &actor, "dispatch", &text)?;
        // 按排好的顺序以同一个名字领取；领不到的记「没领到」，接着取再下一张
        for id in queue {
            let q = self.drow(id)?;
            // 开会话之后角色被删掉、或者不再能在这台机器上跑（改了「只在这些机器上跑」）：这张也不再交给它，取消排队、卡片回到可领取状态
            if !q.profile.is_empty() && !h.can_run(&q.profile, self.now) {
                self.close_row(&q, "canceled", None)?;
                let back = self.unreserve(q.board, q.card, &Actor::system())?;
                let text = format!("派发取消：这台机器不再跑「{}」（{}）{back}", q.profile_name, Self::dispatch_words(&q));
                self.track_dispatch(q.board, q.card, &Actor::system(), "dispatch", &text)?;
                continue;
            }
            let q = DRow { name: Some(name.to_string()), env: d.env.clone(), env_name: d.env_name.clone(), model: d.model.clone(), effort: d.effort.clone(), ..q };
            self.dispatch_ok = true;
            let claimed = self.exec(&board, &actor, Cmd::Claim { id: Some(q.card) });
            self.dispatch_ok = false;
            match claimed {
                Ok(_) => {
                    // 接着做的这一张记下这次的目录（和第一张开始时的算法一样）
                    if q.path.is_none() {
                        let ws = h.workspace();
                        let path = host_board_dir(ws.as_ref(), &board.path(), &h.os).map(|x| x.path);
                        if path.is_some() {
                            self.tx.execute("UPDATE dispatches SET path = ?2 WHERE id = ?1", params![q.id, path])?;
                        }
                    }
                    self.tx.execute(
                        "UPDATE dispatches SET state = 'running', name = ?2, env = ?3, env_name = ?4, started_at = ?5, model = ?6, effort = ?7 WHERE id = ?1",
                        params![q.id, name, q.env, q.env_name, self.now, q.model, q.effort],
                    )?;
                    let seq: i64 = self.tx.query_row("SELECT COUNT(*) FROM dispatches WHERE session = ?1 AND id <= ?2", params![d.session, q.id], |r| r.get(0))?;
                    let who = match &q.env_name {
                        Some(e) => format!("{}（{e}）", q.agent_label),
                        None => format!("{} ", q.agent_label),
                    };
                    let text = format!("派发开始：{} 上的 {who}以 {name} 接着做（定时，这个会话的第 {seq} 张{}）", h.name, opts_words(&q.model, &q.effort));
                    self.track(d.board, q.card, &actor, "dispatch", &text)?;
                    self.touch_hosts();
                    return Ok(Ok(self.read().card(&board, q.card)?));
                }
                Err(e) if e.code == ErrorCode::Internal => return Err(e),
                Err(e) => {
                    self.close_row(&q, "unclaimed", None)?;
                    let back = self.unreserve(q.board, q.card, &Actor::system())?;
                    let text = format!("派发没领到：{}（{}）{back}", e.message, Self::dispatch_words(&q));
                    self.track_dispatch(q.board, q.card, &Actor::system(), "dispatch", &text)?;
                }
            }
        }
        Ok(Err(session_done_error("这个会话排着的卡片都没领到")))
    }

    /// 会话不再接着做：取消这个会话还排着的派发（卡片不动），返回 SESSION_DONE（由调用方提交后交给 agent）
    pub(super) fn session_done(&mut self, d: &DRow, why: &str) -> Result<std::result::Result<CardDetail, BoardError>> {
        self.cancel_session_rest(d.session, why)?;
        Ok(Err(session_done_error(why)))
    }

    /// 会话不再接着做（board next 返回 SESSION_DONE、进程退出、被人停止、机器掉线）：这个会话还排着的派发都取消，
    /// 结果「已取消」，卡片回到原来的可领取状态最上面；这一轮还没结束时由新开的会话重新排
    pub(super) fn cancel_session_rest(&mut self, session: i64, why: &str) -> Result<()> {
        // 后排的先回去：卡片一张张回到可领取状态最上面，最后回去的在最上面，回去后仍是排好的顺序
        let ids: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE session = ?1 AND state = 'queued' ORDER BY id DESC", [session], |r| r.get(0))?;
        for id in ids {
            let q = self.drow(id)?;
            self.close_dispatch(&q, "canceled", None)?;
            let back = self.unreserve(q.board, q.card, &Actor::system())?;
            let text = format!("会话结束（{why}），排着的派发取消（{}）{back}", Self::dispatch_words(&q));
            self.track_dispatch(q.board, q.card, &Actor::system(), "dispatch", &text)?;
        }
        Ok(())
    }

    /// 开会话时排好后面几张（第一张已经开始）：从同一块白板、按领取规则交给同一个角色的可领取卡片里取，连第一张最多
    /// 「一个会话最多做几张」张。先后：带优先标签的在前（按规则 1 的顺序），再是其余卡片里和第一张共同标签多的
    /// （领取规则里用到的标签不算，它只表示谁来做），一样多时按规则 1 的顺序。每张记一条排队的派发，属于这个会话
    /// 除了这台机器，现在还有几台机器也能自动领这块白板上交给这个角色的卡：在自动接单、在线、没在等重启、
    /// 还有空的会话位、接得了这块白板、能跑这个角色、它的 agent 没有暂停接单
    pub(super) fn session_peers(&self, h: &HostRow, board: &BoardRef, profile: &str, agent: &str) -> Result<usize> {
        let mut n = 0;
        for x in self.read().host_rows()? {
            if x.id == h.id || !x.online(self.now) || !x.auto_active(self.now) || x.restarting() || !x.can_run(profile, self.now) {
                continue;
            }
            if self.sessions_on(&x)? >= x.max || self.read().board_unfit(&x, board)?.is_some() || self.read().agent_tripped(&x, agent)? {
                continue;
            }
            n += 1;
        }
        Ok(n)
    }

    pub(super) fn plan_session(&mut self, h: &HostRow, rules: &Routes<'_>, first: &Dispatch, board: &BoardRef, flow: &Flow, skip: &[i64]) -> Result<()> {
        let want = (h.per_session.max(1) - 1) as usize;
        if want == 0 {
            return Ok(());
        }
        let mut skip = skip.to_vec();
        skip.push(first.card);
        let rule_labels = rules.inner.labels();
        let profile: String = self.tx.query_row("SELECT profile FROM dispatches WHERE id = ?1", [first.id], |r| r.get(0))?;
        let mine: Vec<String> = card_label_names(self.tx, board.id, first.card)?.into_iter().filter(|l| !rule_labels.contains(l)).collect();
        let urgent = self.priority_cards(board.id)?;
        // （不带优先标签，共同标签数取反，规则 1 的顺序，卡片）
        let mut list: Vec<(bool, std::cmp::Reverse<usize>, usize, i64)> = vec![];
        for (i, (id, _)) in self.claimable_order(board.id, flow, &skip)?.into_iter().enumerate() {
            let labels = card_label_names(self.tx, board.id, id)?;
            if rules.route(&labels) != Some(profile.as_str()) {
                continue;
            }
            let hot = urgent.contains(&id);
            let common = if hot { 0 } else { labels.iter().filter(|l| mine.contains(l)).count() };
            list.push((!hot, std::cmp::Reverse(common), i, id));
        }
        list.sort();
        // 按现在也能接这些卡的机器分摊：4 张卡、2 台机器都有空位，就各排 2 张，而不是先来的一台排满 4 张
        let peers = self.session_peers(h, board, &profile, &first.agent)?;
        let share = (list.len() + 1).div_ceil(peers + 1);
        list.truncate(want.min(share.saturating_sub(1)));
        if list.is_empty() {
            return Ok(());
        }
        let total = list.len() + 1;
        let name = first.name.clone().unwrap_or_default();
        let by = Actor::system();
        for (k, (.., card)) in list.into_iter().enumerate() {
            let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2", params![board.id, card], |r| r.get(0))?;
            self.tx.execute(
                "INSERT INTO dispatches (board, card, title, host_id, host, agent, agent_label, source, by_kind, by_name, state, created_at, session,
                                         profile, profile_name, provider, note, model, effort)
                 SELECT ?1, ?2, ?3, ?4, ?5, agent, agent_label, 'cron', ?6, ?7, 'queued', ?8, ?9, profile, profile_name, provider, note, model, effort
                   FROM dispatches WHERE id = ?9",
                params![board.id, card, title, h.id, h.name, by.kind.as_str(), by.name, self.now, first.id],
            )?;
            let at = self.reserve(board.id, card, &by)?.map(|n| format!("在「{n}」")).unwrap_or_default();
            let who = if first.profile_name.is_empty() { first.agent_label.clone() } else { first.profile_name.clone() };
            let text = format!("派发到 {} · {who}（定时），会话第 {}/{total} 张，{at}排队等 {name} 接着做", h.name, k + 2);
            self.track(board.id, card, &by, "dispatch", &text)?;
        }
        self.touch_hosts();
        Ok(())
    }

    /// 写一条派发的结束（不动名字：board next 接着做下一张时用）
    pub(super) fn close_row(&mut self, d: &DRow, result: &str, exit_code: Option<i64>) -> Result<()> {
        self.tx.execute(
            "UPDATE dispatches SET state = 'done', result = ?2, ended_at = ?3, exit_code = ?4 WHERE id = ?1",
            params![d.id, result, self.now, exit_code],
        )?;
        self.touch_hosts();
        Ok(())
    }

    /// 超时：卡片仍由这个名字持有。和 final 一样结束这一次（记耗时、之后 50 秒内要移走），
    /// 但这一次记成 `how = timeout`：大盘「按 agent」里算进异常退出
    pub(super) fn timeout_beat(&mut self, b: i64, card: i64, actor: &Actor) -> Result<()> {
        // 先像普通心跳一样打一次：没有进行中的一次时由它开始（生产里 board host 早就打过心跳了）
        self.beat_card(b, card, actor)?;
        if let Some((n, ms)) = self.close_run(b, card, self.now, "timeout")? {
            self.tx.execute("UPDATE cards SET beat_at = ?1, final_at = ?1 WHERE board = ?2 AND id = ?3", params![self.now, b, card])?;
            self.track(b, card, actor, "beat", &format!("结束了心跳：超时，第 {n} 次耗时 {}", crate::engine::dur_words(ms)))?;
        }
        Ok(())
    }

    /// 写一条派发的结束，并让它的名字离线（名字只给这个会话用）
    pub(super) fn close_dispatch(&mut self, d: &DRow, result: &str, exit_code: Option<i64>) -> Result<()> {
        self.close_row(d, result, exit_code)?;
        if result == "crashed" {
            self.trip_check(d)?;
        }
        if let Some(name) = &d.name {
            let online: bool = self.tx.query_row(
                "SELECT COUNT(*) > 0 FROM agents WHERE name = ?1 AND off_at IS NULL",
                [name],
                |r| r.get(0),
            )?;
            if online {
                self.tx.execute("UPDATE agents SET off_at = ?2 WHERE name = ?1", params![name, self.now])?;
            }
        }
        self.touch_hosts();
        Ok(())
    }

    /// 一次派发异常退出之后：这个 agent 在这台机器上正好连续异常退出了 [`crate::fit::TRIP_AFTER`] 次，就记一条操作记录
    /// （暂停本身不用存：`fit.rs` 的 `agent_tripped` 每次按派发记录数出来）
    pub(super) fn trip_check(&mut self, d: &DRow) -> Result<()> {
        let Some(h) = self.host_row(&d.host)?.filter(|h| h.id == d.host_id) else { return Ok(()) };
        if self.read().agent_streak(&h, &d.agent)? == crate::fit::TRIP_AFTER {
            let text = format!(
                "{} 连续 {} 次异常退出，暂停了它在这台机器上接单：修好后在机器详情里点「恢复」",
                d.agent_label,
                crate::fit::TRIP_AFTER
            );
            self.host_op(h.id, "board", &text)?;
            self.touch_hosts();
        }
        Ok(())
    }

    /// 和这台机器上这个 agent 登录同一账号（agent + 邮箱，不区分大小写，#669）的别的机器；这台没报邮箱时没有
    pub(super) fn same_account(&self, h: &HostRow, agent: &str) -> Result<Vec<HostRow>> {
        let Some(key) = h.account_of(agent) else { return Ok(vec![]) };
        Ok(self.read().host_rows()?.into_iter().filter(|o| o.id != h.id && o.account_of(agent).as_deref() == Some(key.as_str())).collect())
    }

    /// 记下 board acp 看到的额度（#668），作为这台机器上这个 agent 最近一次的额度。额度用完时这台机器上这个 agent 的
    /// 本机配置角色暂停接单到重置时间（取不到时 1 小时后），记一条操作；额度按账号算，报了同一账号的别的机器一起暂停到
    /// 这个时间（#669，已经暂停到更晚的不动），各记一条。返回暂停到什么时候（没用完时为 None）
    pub(super) fn note_quota(&mut self, d: &DRow, q: &Quota) -> Result<Option<i64>> {
        let Some(h) = self.host_row(&d.host)?.filter(|h| h.id == d.host_id) else { return Ok(None) };
        let mut q = q.clone();
        q.at = self.now;
        q.text = q.text.chars().take(300).collect();
        let until = q.exhausted.then(|| q.resets_at.filter(|t| *t > self.now).unwrap_or(self.now + QUOTA_PAUSE_DEFAULT));
        let mut run = h.run.clone();
        let e = run.entry(d.agent.clone()).or_default();
        e.quota = Some(q);
        if let Some(u) = until {
            e.quota_until = Some(u);
        }
        self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&run).unwrap_or_default()])?;
        if let Some(u) = until {
            let when = crate::store::when_words(u, self.now);
            let text = format!("{} 的额度用完，本机配置的角色暂停接单到 {when}，到点自动恢复", d.agent_label);
            self.host_op(h.id, "board", &text)?;
            for o in self.same_account(&h, &d.agent)? {
                let mut run = o.run.clone();
                let e = run.entry(d.agent.clone()).or_default();
                if e.quota_until.is_some_and(|t| t >= u) {
                    continue;
                }
                e.quota_until = Some(u);
                self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![o.id, serde_json::to_string(&run).unwrap_or_default()])?;
                let text = format!("{} 的额度用完（{} 上报），本机配置的角色暂停接单到 {when}，到点自动恢复", d.agent_label, h.name);
                self.host_op(o.id, "board", &text)?;
            }
        }
        self.touch_hosts();
        Ok(until)
    }

    /// 动态里的「mbp · Claude Code · mbp/opus-5.5，手动」；有启动环境时「mbp · Codex · Git Bash · mbp/gpt-5，手动」
    pub(super) fn dispatch_words(d: &DRow) -> String {
        // 「任意机器」的派发还没挑到机器
        let host = if d.host.is_empty() { "任意机器" } else { d.host.as_str() };
        let mut s = format!("{host} · {}", d.agent_label);
        if !d.profile_name.is_empty() {
            s += &format!("（{}）", d.profile_name);
        }
        if let Some(e) = &d.env_name {
            s += &format!(" · {e}");
        }
        if let Some(n) = &d.name {
            s += &format!(" · {n}");
        }
        s + "，" + source_words(&d.source)
    }

    /// 开始一次排队中的派发（board host 有空位时按创建先后调用）：机器在线、没有暂停、有空位才开始
    /// （「空位」是会话位：同时在跑的几个 agent 进程还没到「最多几个会话」）
    pub fn dispatch_begin(&mut self, host: &str, instance: &str, id: i64, token: Option<i64>) -> Result<Dispatch> {
        let h = self.host_of(host, instance)?;
        let d = self.drow(id)?;
        if d.state != "queued" || d.host_id != h.id {
            return Err(BoardError::usage(format!("派发 #{id} 不在机器「{host}」的排队里")));
        }
        if d.session != d.id {
            return Err(BoardError::usage(format!("派发 #{id} 排在会话里，由那个会话的 board next 开始")));
        }
        if h.paused {
            return Err(BoardError::usage(format!("机器「{host}」暂停接单")));
        }
        if h.restarting() {
            return Err(BoardError::usage(format!("机器「{host}」正在等重启或升级，不开始新的派发")));
        }
        if self.sessions_on(&h)? >= h.max {
            return Err(BoardError::usage(format!("机器「{host}」没有空位（最多 {} 个会话同时跑）", h.max)));
        }
        self.start(&h, d, token)
    }

    /// `<机器名>/<模型名>-<强度>` 这个名字没被在线 agent 用、也没持有卡片就返回它，否则按 [`dispatch_agent_name`] 的规则
    /// 依次试末尾的 `-1`、`-2` …。派发开始时和 `board check --agent <标识>` 都用它
    pub(crate) fn free_agent_name(&self, agent: &str, host: &str, model: &str, effort: &str) -> Result<String> {
        let gap = crate::agents::NAME_GAP_MS;
        let mut seq = 0u32;
        loop {
            let n = dispatch_agent_name(agent, host, model, effort, seq);
            let taken: bool = self.tx.query_row(
                "SELECT COUNT(*) > 0 FROM agents WHERE name = ?1
                   AND ((off_at IS NULL AND beat_at + ?3 > ?2)
                        OR EXISTS (SELECT 1 FROM cards WHERE holder = ?1 AND deleted_at IS NULL))",
                params![n, self.now, gap],
                |r| r.get(0),
            )?;
            if !taken {
                return Ok(n);
            }
            // 不带序号的被占用：依次试 -1、-2 …
            seq += 1;
        }
    }

    /// 生成名字、check、以它领取卡片。领不到时这次派发结束，结果「没领到」
    pub(super) fn start(&mut self, h: &HostRow, d: DRow, token: Option<i64>) -> Result<Dispatch> {
        // 名字：<机器名>/<模型名>-<强度>（win/opus-5.5-high），见 free_agent_name。模型、强度来自
        // board host 上报的 agent 数据（配置文件 [agents.<标识>] model/effort，内置模板有默认值），
        // 没有模型时用 agent 标识
        // 按角色派的：模型、强度是角色的（本机配置、没指定模型时用机器上报的这个 agent 现在的模型）
        let (mut run_model, mut run_effort) = h.run_opts(&d.agent);
        let (mut name_model, _) = h.model_for(&d.agent);
        let mut name_effort = effort_of(&h.agents, &d.agent);
        if !d.profile.is_empty() {
            let role_model = d.model.clone().filter(|m| !m.is_empty());
            if let Some(m) = role_model.clone() {
                name_model = m.clone();
                run_model = Some(m);
            }
            // 强度是角色的；本机配置、官方订阅既没设模型也没设强度时，会话用的是机器报的默认模型和它的默认强度（#666，
            // 端到端实测名字只有 e2e/opus），这时照报的写
            let role_effort = d.effort.clone().filter(|e| !e.is_empty());
            let local = h.profile(&d.profile).is_some_and(|p| p.provider.is_empty() || p.provider == crate::profiles::SUBSCRIPTION);
            if role_effort.is_some() || role_model.is_some() || !local {
                run_effort = role_effort;
            }
            name_effort = run_effort.clone().unwrap_or_default();
        }
        let name = self.free_agent_name(&d.agent, &h.name, &name_model, &name_effort)?;
        // 用哪个启动环境：按开始这一刻的机器设置
        let env = h.agents.iter().find(|a| a.id == d.agent).and_then(|a| h.pick(a)).map(|c| c.env.clone());
        let env_name = env.as_deref().map(|e| h.env_name(e));
        self.tx.execute(
            "UPDATE dispatches SET name = ?2, env = ?3, env_name = ?4, model = ?5, effort = ?6 WHERE id = ?1",
            params![d.id, name, env, env_name, run_model, run_effort],
        )?;
        let d = DRow { name: Some(name.clone()), env_name: env_name.clone(), model: run_model.clone(), effort: run_effort.clone(), ..d };
        // 这个名字属于这台 board host：派发会话里的 agent 自己 heartbeat stop 也不会让它离线
        self.agent_check_with_agent(Some(&d.agent), &name, token, crate::agents::NameOwner::Host)?;
        let board = self.board_of(d.board)?;
        let actor = Actor::agent(name.clone());
        // 拆解：不领取、不移动这一条，名字上线就开始（host 替它发心跳）
        if d.split() {
            self.tx.execute("UPDATE dispatches SET state = 'running', started_at = ?2, session = ?1 WHERE id = ?1", params![d.id, self.now])?;
            self.touch_hosts();
            let who = match &env_name {
                Some(e) => format!("{}（{e}）", d.agent_label),
                None => format!("{} ", d.agent_label),
            };
            let text = format!("拆解开始：{} 上的 {who}以 {name} 拆解这一条（{}{}）", h.name, source_words(&d.source), opts_words(&d.model, &d.effort));
            self.track_dispatch(d.board, d.card, &actor, "dispatch", &text)?;
            return self.read().dispatch(d.id);
        }
        self.dispatch_ok = true;
        let claimed = self.exec(&board, &actor, Cmd::Claim { id: Some(d.card) });
        self.dispatch_ok = false;
        match claimed {
            Ok(_) => {
                // 记下这一次在哪个目录里做（`<工作空间>/<项目>`）：之后机器换了工作空间，这台机器还按这里找仓库
                let workspace = h.workspace();
                let path = workspace.as_ref().and_then(|w| derive_dir(w, &board.path(), &h.os)).map(|x| x.path);
                self.tx.execute(
                    "UPDATE dispatches SET state = 'running', started_at = ?2, session = ?1, path = COALESCE(?3, path) WHERE id = ?1",
                    params![d.id, self.now, path],
                )?;
                self.touch_hosts();
                // 「mbp 上的 Claude Code（Git Bash）以 mbp/opus-5.5 领取」；旧版 board host 没有启动环境：「mbp 上的 Claude Code 以 …」
                let who = match &env_name {
                    Some(e) => format!("{}（{e}）", d.agent_label),
                    None => format!("{} ", d.agent_label),
                };
                let text = format!("派发开始：{} 上的 {who}以 {name} 领取（{}{}）", h.name, source_words(&d.source), opts_words(&d.model, &d.effort));
                self.track(d.board, d.card, &actor, "dispatch", &text)?;
            }
            Err(e) if e.code == ErrorCode::Internal => return Err(e),
            Err(e) => {
                self.close_dispatch(&d, "unclaimed", None)?;
                let back = self.unreserve(d.board, d.card, &Actor::system())?;
                let text = format!("派发没领到：{}（{}）{back}", e.message, Self::dispatch_words(&d));
                self.track_dispatch(d.board, d.card, &Actor::system(), "dispatch", &text)?;
            }
        }
        self.read().dispatch(d.id)
    }

    /// 定时领取（任务下发需求 §六）：正在一轮领取、没有暂停、有空位（会话位没满）时，在这台机器有目录的白板上按 --dir 的顺序轮流，
    /// 按领取规则挑下一张卡片和它的 agent。一条领取规则也没有时不领卡（这一轮直接结束，返回 None）。
    /// 规则没命中的卡片、也没有可领取的卡片时这一轮结束，返回 None
    pub fn dispatch_pull(&mut self, host: &str, instance: &str, token: Option<i64>) -> Result<Option<Dispatch>> {
        let h = self.host_of(host, instance)?;
        // 现在不在自动接单（没开、暂停接单、没有领取规则、不在接单时段里）：这一段接单结束
        if !h.auto_active(self.now) {
            if h.round_at.is_some() {
                self.tx.execute("UPDATE hosts SET round_at = NULL WHERE id = ?1", [h.id])?;
                self.touch_hosts();
            }
            return Ok(None);
        }
        if h.restarting() || self.sessions_on(&h)? >= h.max {
            return Ok(None);
        }
        // 规则按这台机器筛过：「由这些机器做」里明确排除了这台机器的 agent，它的卡这台机器不领
        let cached = h.claim_rules(self.now);
        let Some(rules) = cached.as_ref().map(|r| h.routes(r)) else { return Ok(None) };
        // 这一段接单从什么时候开始（进入接单时段后第一次来领时记下）
        let since = match h.round_at {
            Some(t) => t,
            None => {
                self.tx.execute("UPDATE hosts SET round_at = ?2 WHERE id = ?1", params![h.id, self.now])?;
                self.now
            }
        };
        // 参与自动领取的白板（`boards.auto`），按项目、白板的顺序轮流；这台机器接不了的在下面跳过
        let boards: Vec<String> = self.collect(
            "SELECT b.project || '/' || b.key FROM boards b JOIN projects p ON p.key = b.project
             WHERE b.auto = 1 ORDER BY p.sort, p.created_at, p.key, b.sort, b.created_at, b.key",
            [],
            |r| r.get(0),
        )?;
        if boards.is_empty() {
            return Ok(None);
        }
        let n = boards.len();
        for k in 0..n {
            let i = (h.rr.max(0) as usize + k) % n;
            let Ok(board) = self.resolve(&boards[i]) else { continue };
            // 这台机器接不了的白板（目录不对）跳过
            if self.read().board_unfit(&h, &board)?.is_some() {
                continue;
            }
            let flow = flow_of(self.tx, board.id)?;
            // 这台机器这一段接单里没做成的卡片先不再领（原来的「这一轮不再领」）：派发异常退出、被停止、超时之后卡片一般已经在
            // 错误节点 / 需要人处理的状态里；工作流里没有能接住它的状态时卡片 50 秒会回到可领取状态最上面，不跳的话同一个
            // 空位会反复领到它、白跑 agent。人取消的排队也不再排。做完了（已交审、做不下去）又被人拖回可领取状态的照常领；
            // 别的机器不受影响。出了接单时段、关了自动接单、暂停接单之后这一段结束，下一段重新算
            let horizon = since;
            let skip: Vec<i64> = self.collect(
                "SELECT DISTINCT card FROM dispatches WHERE board = ?1 AND host_id = ?2 AND created_at >= ?3 AND state = 'done'
                   AND (result IN ('crashed', 'stopped', 'timeout') OR (result = 'canceled' AND stop_by IS NOT NULL))",
                params![board.id, h.id, horizon],
                |r| r.get(0),
            )?;
            // 规则会交给一个这台机器能跑的角色的卡片，按规则 1 的先后（带优先标签的在前）取第一张
            let found = h.resolved();
            let mut hit: Option<(i64, HostAgent, crate::profiles::Profile)> = None;
            let urgent = self.priority_cards(board.id)?;
            let mut order = self.claimable_order(board.id, &flow, &skip)?;
            order.sort_by_key(|(id, _)| !urgent.contains(id));
            // 连续异常退出、暂停了接单的 agent：规则交给它的卡片先不领
            let mut tripped: BTreeMap<String, bool> = BTreeMap::new();
            for (id, _) in order {
                let Some(want) = rules.route(&card_label_names(self.tx, board.id, id)?) else { continue };
                let Some(p) = h.profile(want) else { continue };
                let off = match tripped.get(&p.agent) {
                    Some(t) => *t,
                    None => {
                        let t = self.read().agent_tripped(&h, &p.agent)?;
                        tripped.insert(p.agent.clone(), t);
                        t
                    }
                };
                if off {
                    continue;
                }
                if let Some(a) = found.iter().find(|x| x.id == p.agent) {
                    hit = Some((id, a.clone(), p.clone()));
                    break;
                }
            }
            let Some((card, agent, role)) = hit else { continue };
            self.tx.execute("UPDATE hosts SET rr = ?2 WHERE id = ?1", params![h.id, ((i + 1) % n) as i64])?;
            let d = self.insert_dispatch(board.id, card, &h, &agent.id, &agent.name, Some(&role), "cron", &Actor::system(), "", None)?;
            let d = self.start(&h, self.drow(d.id)?, token)?;
            if d.state == "running" {
                // 开会话时把这个会话要做的卡片一次排好：先把一个会话排满，再开下一个
                self.plan_session(&h, &rules, &d, &board, &flow, &skip)?;
                return self.read().dispatch(d.id).map(Some);
            }
        }
        Ok(None)
    }

    /// 派发结束（任务下发需求 §七「结束」）：board host 在 agent 进程退出后上报退出码和日志最后 20 行。
    /// `why` 只在 `how = timeout` 时有：写给人看的原因（「运行超过 3 小时…」）
    pub fn dispatch_end(&mut self, id: i64, exit_code: Option<i64>, tail: &[String], how: EndHow, why: Option<&str>) -> Result<Dispatch> {
        self.dispatch_end_with(id, exit_code, tail, &[], how, why)
    }

    /// 同 [`W::dispatch_end`]，另外带上最后几个事件的文字（runlog.rs `RunEvent::line`）：异常退出、超时的评论里写它们，
    /// 原始输出的最后 20 行折叠在后面。旧版 board host 不报事件，评论照旧写「日志最后 20 行」
    pub fn dispatch_end_with(&mut self, id: i64, exit_code: Option<i64>, tail: &[String], events: &[String], how: EndHow, why: Option<&str>) -> Result<Dispatch> {
        self.dispatch_end_full(id, exit_code, tail, events, how, why, None)
    }

    /// 同 [`W::dispatch_end_with`]，另外带上 board acp 看到的额度（#668）：记成这台机器上这个 agent 最近一次的额度；
    /// 额度用完（`exhausted`）时结果记「额度用完」，不算异常退出，卡片放回领取前的状态，本机配置的角色暂停接单到重置时间
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_end_full(&mut self, id: i64, exit_code: Option<i64>, tail: &[String], events: &[String], how: EndHow, why: Option<&str>, quota: Option<&Quota>) -> Result<Dispatch> {
        let mut d = self.drow(id)?;
        // 进程退出时 board host 可能还不知道会话已经换到下一张（board next）：结算这个会话正在跑的那一张
        if d.state == "done" {
            let cur: Option<i64> = self
                .tx
                .query_row(
                    "SELECT id FROM dispatches WHERE COALESCE(session, id) = ?1 AND state = 'running' ORDER BY id DESC LIMIT 1",
                    [d.session],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(cur) = cur {
                d = self.drow(cur)?;
            }
        }
        let id = d.id;
        if d.state == "running" {
            let f = match how {
                EndHow::Exit => Finish::Exit,
                EndHow::HostExit => Finish::HostExit,
                EndHow::Timeout => Finish::Timeout(why.map(str::trim).filter(|w| !w.is_empty())),
            };
            self.finish(&d, exit_code, &Tail { lines: tail, events, quota }, f)?;
        }
        self.read().dispatch(id)
    }

    pub(super) fn finish(&mut self, d: &DRow, exit_code: Option<i64>, tail: &Tail, how: Finish) -> Result<()> {
        // 额度（#668）：先记下；这一轮因为额度用完结束（agent 自己退出、没人停）时本机配置的角色暂停到重置时间
        let paused = match tail.quota {
            Some(q) => self.note_quota(d, q)?,
            None => None,
        };
        let quota = paused.filter(|_| matches!(how, Finish::Exit) && d.stop_by.is_none());
        // 超时按异常退出的规则处理，但原因写在评论里，结果记「超时」
        let because = match how {
            Finish::Timeout(why) => Some(why.map(str::to_string).unwrap_or_else(|| "board host 按机器的兜底设置在超时后停止了它".into())),
            _ => None,
        };
        let Some(name) = d.name.clone() else {
            return self.close_dispatch(d, if because.is_some() { "timeout" } else { "crashed" }, exit_code);
        };
        let actor = Actor::agent(name.clone());
        let stop_by = match how {
            Finish::HostExit => Some(format!("board host（{}）", d.host)),
            _ => d.stop_by.clone(),
        };
        if d.split() {
            return self.finish_split(d, &actor, exit_code, tail, how, because, stop_by, quota);
        }
        let row: Option<(String, Option<String>)> = self
            .tx
            .query_row("SELECT col, holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let board = self.board_of(d.board)?;
        let flow = flow_of(self.tx, d.board)?;
        let result = match &row {
            // 额度用完、卡片仍由这个名字持有：不是 agent 的问题，卡片放回领取前的状态最上面，别的机器接着做
            Some((col, Some(holder))) if *holder == name && quota.is_some() => {
                let claimed_from: Option<String> = self
                    .tx
                    .query_row("SELECT claimed_from FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0))
                    .optional()?
                    .flatten();
                let start = flow.back_to(claimed_from.as_deref()).clone();
                self.place(d.board, d.card, Some(col), &start.key, At::Top)?;
                self.clear_claim(d.board, d.card, "release")?;
                self.log_move(d.board, d.card, "release", col, &start.key, &actor, &flow)?;
                let until = quota.unwrap_or(self.now);
                let mut text = format!(
                    "{} 上 {} 的额度用完，{} 恢复：卡片回到「{}」最上面，别的机器可以接着做",
                    d.host,
                    d.agent_label,
                    crate::store::when_words(until, self.now),
                    start.name
                );
                if let Some(t) = tail.quota.map(|q| q.text.trim()).filter(|t| !t.is_empty()) {
                    text += &format!("（{t}）");
                }
                self.track(d.board, d.card, &actor, "release", &text)?;
                "quota"
            }
            // 卡片仍由这个名字持有：发 final，沿连线移到「错误」节点（没有时去需要人处理的状态），评论原因。
            // 这几步尽力而为：规则拒绝（例如刚好被超时释放）不影响记下派发的结果
            Some((col, Some(holder))) if *holder == name => {
                // 超时：和 final 一样结束这次计时，但记成「超时」，大盘里算异常退出（§10.13 按 agent）
                match how {
                    Finish::Timeout(_) => soft(self.timeout_beat(d.board, d.card, &actor))?,
                    _ => soft(self.exec(&board, &actor, Cmd::Beat { id: d.card, done: true }))?,
                }
                let still: Option<String> = self
                    .tx
                    .query_row("SELECT holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0))
                    .optional()?
                    .flatten();
                let error = flow.error_after(col);
                let human = flow.human_after(col);
                // 错误节点优先于需要人处理的状态：卡片离开可领取状态，配了定时领取也不会被下一轮重领
                let target = error.as_ref().or(human.as_ref());
                let still_held = still.as_deref() == Some(name.as_str());
                if still_held {
                    if let Some(to) = target {
                        soft(self.exec(&board, &actor, Cmd::Move { id: d.card, column: to.key.clone(), top: false, index: None, order: None, before: None, after: None }))?;
                    }
                }
                let mut text = match (&stop_by, how) {
                    (_, Finish::Timeout(_)) => {
                        let mut t = format!("{}，board host 停止了它", because.clone().unwrap_or_default());
                        t += &tail.words();
                        t
                    }
                    (_, Finish::HostExit) => format!("board host（{}）退出，停止了这次派发", d.host),
                    (Some(who), _) => format!("{who} 停止了这次派发"),
                    (None, Finish::Missing) => {
                        "agent 异常退出（board host 上没有这次派发的 agent 进程：board host 重启过，或开始派发时连接中断）".to_string()
                    }
                    (None, _) => {
                        let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into());
                        let mut t = format!("agent 异常退出（退出码 {code}）");
                        t += &tail.words();
                        t
                    }
                };
                // 工作流里从当前状态连不出去能接住异常退出的状态：评论写明卡片会回到哪里，别让人以为它悄悄消失了
                if still_held && target.is_none() {
                    let claimed_from: Option<String> = self
                        .tx
                        .query_row("SELECT claimed_from FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0))
                        .optional()?
                        .flatten();
                    let back = flow.back_to(claimed_from.as_deref()).name.clone();
                    // 状态存在、只是从当前状态连不出去：单独说明，免得人以为白板没配
                    // 一个错误节点 = 有锁定状态连过去，所以白板上有错误节点、而这里没找到，就是从当前状态到不了
                    let unreachable = |what: &str, ok: Option<&Status>, exists: bool| {
                        if ok.is_none() && exists {
                            format!("这块白板有{what}状态，但从「{}」连不出去；", flow.name(col))
                        } else {
                            String::new()
                        }
                    };
                    let no_error = unreachable("错误", error, flow.0.iter().any(|s| flow.is_error(&s.key)));
                    let no_human = unreachable("需要人处理", human, flow.0.iter().any(|s| flow.is_human(&s.key)));
                    text += &format!("\n\n没有能接住异常退出的状态：{no_error}{no_human}卡片 50 秒后会回到「{}」最上面。", back);
                }
                soft(self.exec(&board, &actor, Cmd::Comment { id: d.card, text }))?;
                if because.is_some() {
                    "timeout"
                } else if stop_by.is_some() {
                    "stopped"
                } else {
                    "crashed"
                }
            }
            _ if stop_by.is_some() => "stopped",
            Some((col, _)) if flow.is_review(col) => "review",
            _ => "stuck",
        };
        self.close_dispatch(d, result, exit_code)?;
        let took = d.started_at.map(|t| format!("，用时 {}", crate::engine::dur_words(self.now - t))).unwrap_or_default();
        let code = exit_code.map(|c| format!("，退出码 {c}")).unwrap_or_default();
        let text = format!("派发结束：{}（{}{took}{code}）", result_words(result), Self::dispatch_words(d));
        self.track_dispatch(d.board, d.card, &actor, "dispatch", &text)?;
        // 进程不在了：这个会话还排着的取消
        let why = match (&stop_by, how) {
            (_, Finish::HostExit) => format!("board host（{}）退出", d.host),
            (Some(who), _) => format!("{who} 停止了这个会话"),
            _ => "agent 进程退出".to_string(),
        };
        self.cancel_session_rest(d.session, &why)
    }

    /// 拆解结束（PRD §9.1）：卡片在任何情况下都不动。退出码 0 时按这次派发期间建了几张「拆自」是这条的卡记「已拆解」或「不用拆」；
    /// 退出码非 0（或 board host 上没有进程）记「异常退出」，人停止、board host 退出、超时记「已停止」，都在这一条上评论原因
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_split(&mut self, d: &DRow, actor: &Actor, exit_code: Option<i64>, tail: &Tail, how: Finish, because: Option<String>, stop_by: Option<String>, quota: Option<i64>) -> Result<()> {
        let since = d.started_at.unwrap_or(self.now);
        let made: i64 = self.tx.query_row(
            "SELECT COUNT(*) FROM cards WHERE board = ?1 AND split_from = ?2 AND created_at >= ?3",
            params![d.board, d.card, since],
            |r| r.get(0),
        )?;
        let (result, why) = match (how, &stop_by) {
            (Finish::Timeout(_), _) => ("stopped", Some(format!("{}，board host 停止了这次拆解{}", because.unwrap_or_default(), tail.words()))),
            (Finish::HostExit, _) => ("stopped", Some(format!("board host（{}）退出，停止了这次拆解", d.host))),
            (_, Some(who)) => ("stopped", Some(format!("{who} 停止了这次拆解"))),
            (Finish::Missing, None) => (
                "crashed",
                Some("拆解的 agent 异常退出（board host 上没有这次派发的 agent 进程：board host 重启过，或开始派发时连接中断）".to_string()),
            ),
            (Finish::Exit, None) if exit_code == Some(0) => (if made > 0 { "split" } else { "fine" }, None),
            (Finish::Exit, None) if quota.is_some() => (
                "quota",
                Some(format!("{} 上 {} 的额度用完，{} 恢复：这次拆解没做完", d.host, d.agent_label, crate::store::when_words(quota.unwrap_or(self.now), self.now))),
            ),
            (Finish::Exit, None) => {
                let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into());
                ("crashed", Some(format!("拆解的 agent 异常退出（退出码 {code}）{}", tail.words())))
            }
        };
        if let Some(text) = why {
            let board = self.board_of(d.board)?;
            soft(self.exec(&board, actor, Cmd::Comment { id: d.card, text }))?;
        }
        self.close_dispatch(d, result, exit_code)?;
        let took = d.started_at.map(|t| format!("，用时 {}", crate::engine::dur_words(self.now - t))).unwrap_or_default();
        let code = exit_code.map(|c| format!("，退出码 {c}")).unwrap_or_default();
        let made = if made > 0 { format!("，拆出 {made} 条") } else { String::new() };
        let text = format!("拆解结束：{}（{}{took}{code}{made}）", result_words(result), Self::dispatch_words(d));
        self.track_dispatch(d.board, d.card, actor, "dispatch", &text)?;
        Ok(())
    }

    /// 追加日志，只保留最后 500 行（不算变更，不推送；界面打开日志时自己去取）
    pub fn dispatch_log_append(&mut self, id: i64, lines: &[String]) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        let log: Option<String> = self.tx.query_row("SELECT log FROM dispatches WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        let log = log.ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("派发 #{id} 不存在")))?;
        let mut all: Vec<&str> = if log.is_empty() { vec![] } else { log.split('\n').collect() };
        all.extend(lines.iter().map(|l| l.trim_end_matches(['\r', '\n'])));
        let keep = all.len().saturating_sub(LOG_KEEP);
        self.tx.execute("UPDATE dispatches SET log = ?2 WHERE id = ?1", params![id, all[keep..].join("\n")])?;
        Ok(())
    }

    /// 追加结构化事件（runlog.rs）：`fmt` 是 agent 的输出格式（空 = 只有原始输出），每次都带上，覆盖原来的。
    /// 工具结果合进同一个调用先到的那一条；单条超过 4 KB 截断；超过 2000 条删最早的，记下删了几条。
    /// 不算变更（不推整个工作区），只把 meta.runs 加一：实时推送带上它和有新事件的派发，打开着的会话据此马上取
    pub fn dispatch_events_append(&mut self, id: i64, fmt: Option<&str>, events: &[crate::runlog::RunEvent]) -> Result<usize> {
        self.events_append(EvTable::DISPATCH, id, fmt, events)
    }

    /// 派发、机器对话共用的追加事件（见 [`W::dispatch_events_append`]）：`t` 是存在哪张表
    pub(super) fn events_append(&mut self, t: EvTable, id: i64, fmt: Option<&str>, events: &[crate::runlog::RunEvent]) -> Result<usize> {
        use crate::runlog::{RunEvent, EVENTS_KEEP, KINDS};
        let EvTable { owner, events: evt, col, what } = t;
        let exists: Option<i64> = self.tx.query_row(&format!("SELECT id FROM {owner} WHERE id = ?1"), [id], |r| r.get(0)).optional()?;
        if exists.is_none() {
            return Err(BoardError::new(ErrorCode::NotFound, format!("{what} #{id} 不存在")));
        }
        if let Some(f) = fmt {
            let f: String = f.trim().chars().take(32).collect();
            self.tx.execute(&format!("UPDATE {owner} SET fmt = ?2 WHERE id = ?1 AND fmt IS NOT ?2"), params![id, f])?;
        }
        if events.is_empty() {
            return Ok(0);
        }
        self.tx.execute("INSERT INTO meta (key, value) VALUES ('runs', 1) ON CONFLICT(key) DO UPDATE SET value = value + 1", [])?;
        let rev: i64 = self.tx.query_row("SELECT value FROM meta WHERE key = 'runs'", [], |r| r.get(0))?;
        let mut n = 0;
        for e in events {
            let mut e = e.clone();
            if !KINDS.contains(&e.kind.as_str()) {
                e.kind = "text".into();
            }
            e.id = 0;
            if e.at <= 0 {
                e.at = self.now;
            }
            let call = e.call.take().filter(|c| !c.is_empty()).map(|c| c.chars().take(200).collect::<String>());
            if e.kind == "tool" {
                if let Some(c) = call.as_deref() {
                    let old: Option<(i64, String)> = self
                        .tx
                        .query_row(
                            &format!("SELECT id, data FROM {evt} WHERE {col} = ?1 AND call = ?2 ORDER BY id DESC LIMIT 1"),
                            params![id, c],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    if let Some((eid, data)) = old {
                        let mut prev: RunEvent = serde_json::from_str(&data).unwrap_or_else(|_| RunEvent::new("tool"));
                        prev.merge(&e);
                        prev.fit();
                        let data = serde_json::to_string(&prev).unwrap_or_default();
                        self.tx.execute(&format!("UPDATE {evt} SET data = ?2, rev = ?3 WHERE id = ?1"), params![eid, data, rev])?;
                        n += 1;
                        continue;
                    }
                }
            }
            e.fit();
            let data = serde_json::to_string(&e).unwrap_or_default();
            self.tx.execute(&format!("INSERT INTO {evt} ({col}, kind, call, rev, data) VALUES (?1, ?2, ?3, ?4, ?5)"), params![id, e.kind, call, rev, data])?;
            n += 1;
        }
        let total: i64 = self.tx.query_row(&format!("SELECT COUNT(*) FROM {evt} WHERE {col} = ?1"), [id], |r| r.get(0))?;
        if total > EVENTS_KEEP {
            let drop = total - EVENTS_KEEP;
            self.tx.execute(&format!("DELETE FROM {evt} WHERE id IN (SELECT id FROM {evt} WHERE {col} = ?1 ORDER BY id LIMIT ?2)"), params![id, drop])?;
            self.tx.execute(&format!("UPDATE {owner} SET ev_dropped = ev_dropped + ?2 WHERE id = ?1"), params![id, drop])?;
        }
        Ok(n)
    }

    /// 结束超过 30 天的派发只留 `result` 事件和最后 50 条（服务端每 10 分钟一次，每次最多处理 200 个派发）。返回删了几条
    pub fn prune_run_events(&mut self) -> Result<usize> {
        use crate::runlog::{EVENTS_OLD_KEEP, EVENTS_OLD_MS};
        let ids: Vec<i64> = self.collect(
            "SELECT id FROM dispatches WHERE state = 'done' AND ev_old = 0 AND ended_at IS NOT NULL AND ended_at <= ?1 ORDER BY id LIMIT 200",
            [self.now - EVENTS_OLD_MS],
            |r| r.get(0),
        )?;
        let mut total = 0;
        for id in ids {
            let n = self.tx.execute(
                "DELETE FROM dispatch_events WHERE dispatch = ?1 AND kind != 'result' \
                 AND id NOT IN (SELECT id FROM dispatch_events WHERE dispatch = ?1 ORDER BY id DESC LIMIT ?2)",
                params![id, EVENTS_OLD_KEEP],
            )?;
            self.tx.execute("UPDATE dispatches SET ev_old = 1, ev_dropped = ev_dropped + ?2 WHERE id = ?1", params![id, n as i64])?;
            total += n;
        }
        Ok(total)
    }
}
