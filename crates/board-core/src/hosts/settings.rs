//! 机器设置：界面上的修改（`HostPatch`）、工作空间和目录、暂停、移除、改名。

use super::*;

/// 界面上修改机器设置
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPatch {
    #[serde(default)]
    pub max: Option<i64>,
    #[serde(default)]
    pub per_session: Option<i64>,
    /// 旧界面的单次最长运行：现在所有机器一个值，改的就是全局的（`sched.rs`）
    #[serde(default)]
    pub max_run: Option<i64>,
    /// 旧界面的没输出多久算卡住：同上
    #[serde(default)]
    pub idle_run: Option<i64>,
    /// skills 用服务器的（true）还是不改（false）
    #[serde(default)]
    pub sync_skills: Option<bool>,
    /// MCP 用服务器的（true）还是不改（false）
    #[serde(default)]
    pub sync_mcp: Option<bool>,
    /// Git 凭据用服务器的（true）还是不改（false）
    #[serde(default)]
    pub sync_git: Option<bool>,
    /// 领取规则（整个列表替换）
    #[serde(default)]
    pub rules: Option<Vec<ClaimRule>>,
    /// 兜底的 agent；空字符串表示取消兜底
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub agent: Option<String>,
    /// 旧界面的「定时领取」开关：当成自动接单的开关
    #[serde(default)]
    pub cron_on: Option<bool>,
    /// 旧界面的定时领取时间表：不再用，收到也不存
    #[serde(default)]
    pub cron: Option<Vec<String>>,
    /// 参与自动接单
    #[serde(default)]
    pub auto: Option<bool>,
    /// 接单时段（整个列表替换）：一天里的几段 `[起, 止)`，分钟
    #[serde(default)]
    pub windows: Option<Vec<crate::sched::Window>>,
    /// 内置 agent 自动更新（默认关）
    #[serde(default)]
    pub agent_auto: Option<bool>,
    /// 一个 agent 的启动环境
    #[serde(default)]
    pub env: Option<EnvPatch>,
    /// 一个 agent 的模型：不改 / 指定（供应商、模型、强度、接口）
    #[serde(default)]
    pub run: Option<RunPatch>,
    /// 恢复一个连续异常退出、暂停了接单的 agent（写它的标识）
    #[serde(default)]
    pub resume: Option<String>,
    /// 工作空间（整个替换）。空字符串表示不配工作空间；None 表示不改
    #[serde(default, deserialize_with = "workspace_patch")]
    pub workspace: Option<HostWorkspace>,
    /// 旧的白板与目录（整个列表替换）：界面上改的是工作空间，这一项只给旧版界面用，收下后换成工作空间
    #[serde(default)]
    pub dirs: Option<Vec<HostDir>>,
}

/// `workspace` 在补丁里是字符串（只改路径）、对象或者 null（不配）；不写这个键时是不改
pub(super) fn workspace_patch<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<HostWorkspace>, D::Error> {
    use serde::de::Error as _;
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Null => Some(HostWorkspace::default()),
        serde_json::Value::String(s) => Some(HostWorkspace { path: s, ..Default::default() }),
        v => Some(serde_json::from_value(v).map_err(D::Error::custom)?),
    })
}

/// 改一个 agent 的启动环境
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EnvPatch {
    pub agent: String,
    pub env: String,
}

/// 旧版界面改一个 agent 的「模型怎么配」：机器上不再指定模型（#576），收到就拒绝（`USAGE`）。字段留着只为认得出这种请求
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunPatch {
    pub agent: String,
}

impl<'a> W<'a> {
    /// 换这台机器的工作空间（服务端成为新设置的主版本，board host 写回本机配置）。`--dir` 单独覆盖的白板
    /// 原样保留，其他白板由工作空间派生；路径只去掉首尾空白，`~` 由 board host 展开
    pub(super) fn host_set_workspace(&mut self, name: &str, raw: &HostWorkspace, by: &str) -> Result<()> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let stored = h.workspace();
        let mut next = clean_workspace(raw);
        // 本机 `--dir` 启动的白板由启动参数决定，界面上改不了：提交里没带上时替它补回去；
        // 不配工作空间（路径留空）时一块都不留
        let fixed: Vec<HostDirOverride> = stored.iter().flat_map(|w| w.overrides.iter().cloned()).collect();
        if !next.path.is_empty() {
            for f in &fixed {
                match next.overrides.iter().find(|o| o.board == f.board) {
                    Some(o) if o.path != f.path => {
                        return Err(BoardError::usage(format!("{} 用 --dir 固定，不能改；要在那台机器上改启动参数", f.board)));
                    }
                    Some(_) => {}
                    None => next.overrides.push(f.clone()),
                }
            }
        }
        for o in &next.overrides {
            if !crate::valid_dir_key(&o.board) {
                return Err(BoardError::usage(format!("要写成 <项目> 或 <项目>/<白板>：{}", o.board)));
            }
            if o.board.contains('/') {
                self.resolve(&o.board)?;
            } else {
                self.project_exists(&o.board)?;
            }
        }
        if next.path.is_empty() {
            // 不配工作空间：旧的「白板与目录」也一起清掉，否则还会按它派生
            if stored.is_none() && h.dirs.is_empty() {
                return Ok(());
            }
            self.tx.execute("UPDATE hosts SET workspace = NULL, dirs = '[]', dirs_rev = ?2, dirs_at = ?3, dirs_by = ?4 WHERE id = ?1", params![
                h.id,
                h.dirs_rev + 1,
                self.now,
                by.trim().to_string()
            ])?;
            self.touch_hosts();
            return Ok(());
        }
        // 已经摆着的老白板（升级前逐块配的、board host 报过的）跟着换成工作空间派生的路径
        // 派生时带上项目目录（#488）；存的 next 不带
        let with_projects = HostWorkspace { projects: h.proj_dirs.clone(), ..next.clone() };
        let dirs = derived_dirs(&derived_dirs(&h.dirs, stored.as_ref(), &h.os), Some(&with_projects), &h.os);
        if same_workspace(stored.as_ref(), Some(&next)) && derived_dirs(&h.dirs, stored.as_ref(), &h.os).iter().map(|d| &d.board).eq(dirs.iter().map(|d| &d.board)) {
            return Ok(());
        }
        for d in &dirs {
            self.resolve(&d.board)?;
        }
        self.tx.execute(
            "UPDATE hosts SET workspace = ?2, dirs = ?3, dirs_rev = ?4, dirs_at = ?5, dirs_by = ?6 WHERE id = ?1",
            params![
                h.id,
                serde_json::to_string(&next).unwrap_or_default(),
                serde_json::to_string(&dirs).unwrap_or_else(|_| "[]".into()),
                h.dirs_rev + 1,
                self.now,
                by.trim().to_string()
            ],
        )?;
        self.touch_hosts();
        Ok(())
    }

    /// 旧界面提交的「白板与目录」：换成工作空间（同一台机器上几块白板取共同的上两级，对不上的留成 `--dir` 覆盖），
    /// 然后按工作空间保存
    pub(super) fn host_set_dirs(&mut self, name: &str, raw: &[HostDir], by: &str) -> Result<()> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let mut seen: Vec<String> = vec![];
        for d in raw {
            let board = d.board.trim().to_string();
            if !crate::valid_board(&board) {
                return Err(BoardError::usage(format!("白板要写成 <项目>/<白板>：{}", d.board)));
            }
            if d.path.trim().is_empty() {
                return Err(BoardError::usage(format!("白板 {board} 还没有填目录")));
            }
            if seen.contains(&board) {
                return Err(BoardError::usage(format!("白板 {board} 写了两次")));
            }
            self.resolve(&board)?;
            seen.push(board);
        }
        let cur = h.workspace();
        let fixed: Vec<HostDirOverride> = cur.iter().flat_map(|w| w.overrides.iter().cloned()).collect();
        if raw.is_empty() {
            return self.host_set_workspace(name, &HostWorkspace { overrides: fixed, ..Default::default() }, by);
        }
        let mut next = exported_workspace(raw).ok_or_else(|| {
            BoardError::usage("这几块白板的目录不在同一个工作空间里：改成只填一个工作空间，每个项目的目录是 <工作空间>/<项目>")
        })?;
        for f in fixed {
            if !next.overrides.iter().any(|o| o.board == f.board) {
                next.overrides.push(f);
            }
        }
        self.host_set_workspace(name, &next, by)
    }

    /// 界面上请求在一台机器的某个目录里克隆仓库；真正执行的是那台机器的 board host
    pub fn host_clone_request(&mut self, name: &str, board: &str, url: &str, by: &str) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let url = url.trim();
        if url.is_empty() {
            return Err(BoardError::usage("先填仓库地址"));
        }
        let ws = h.workspace();
        let Some(d0) = host_board_dir(ws.as_ref(), board, &h.os) else {
            let project = board.split_once('/').map_or(board, |(p, _)| p);
            return Err(BoardError::usage(format!(
                "机器「{name}」还没有配工作空间：先在机器详情里填上，{project} 项目的目录是 <工作空间>/{project}"
            )));
        };
        let mut dirs = derived_dirs(&h.dirs, ws.as_ref(), &h.os);
        let d = dirs
            .iter_mut()
            .find(|d| d.board == board)
            .ok_or_else(|| BoardError::usage(format!("机器「{name}」没有白板 {board} 的目录（{}）", d0.path)))?;
        if d.fixed {
            return Err(BoardError::usage(format!("{board} 用 --dir 固定，不能从界面上克隆")));
        }
        if d.status.as_deref() != Some("missing") {
            return Err(BoardError::usage(format!("{board} 的目录现在不需要克隆")));
        }
        if d.clone.as_ref().is_some_and(|c| c.state == "queued" || c.state == "running") {
            return Err(BoardError::usage(format!("{board} 已经有一个克隆在进行")));
        }
        d.clone = Some(HostClone {
            state: "queued".into(),
            url: url.into(),
            message: "等待 board host 开始克隆".into(),
            updated_at: self.now,
            by: by.trim().to_string(),
        });
        self.tx.execute(
            "UPDATE hosts SET dirs = ?2 WHERE id = ?1",
            params![h.id, serde_json::to_string(&dirs).unwrap_or_else(|_| "[]".into())],
        )?;
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// 界面上修改机器设置（任务下发需求 §四）
    pub fn host_update(&mut self, name: &str, p: &HostPatch) -> Result<HostInfo> {
        self.host_update_as(name, p, "人")
    }

    /// 同 [`Self::host_update`]；by 是界面上是谁改的（写进「工作空间」的修改记录）
    pub fn host_update_as(&mut self, name: &str, p: &HostPatch, by: &str) -> Result<HostInfo> {
        // 旧界面提交的是白板与目录：换成工作空间。两个都给了时以工作空间为准
        if let Some(ws) = &p.workspace {
            self.host_set_workspace(name, ws, by)?;
        } else if p.dirs.is_some() {
            self.host_set_dirs(name, p.dirs.as_deref().unwrap_or_default(), by)?;
        }
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if let Some(max) = p.max {
            if !(1..=HOST_MAX_LIMIT).contains(&max) {
                return Err(BoardError::usage(format!("最多几个会话要在 1 到 {HOST_MAX_LIMIT} 之间")));
            }
            self.tx.execute("UPDATE hosts SET max = ?2 WHERE id = ?1", params![h.id, max])?;
        }
        if let Some(n) = p.per_session {
            if !(1..=PER_SESSION_LIMIT).contains(&n) {
                return Err(BoardError::usage(format!("一个会话最多做几张要在 1 到 {PER_SESSION_LIMIT} 之间")));
            }
            self.tx.execute("UPDATE hosts SET per_session = ?2 WHERE id = ?1", params![h.id, n])?;
        }
        // 跑多久停现在所有机器一个值：旧界面从机器设置里改，改的就是全局的
        if p.max_run.is_some() || p.idle_run.is_some() {
            self.set_run_limits(p.max_run, p.idle_run)?;
        }
        self.config_by = by.to_string();
        // skills、MCP、Git 凭据的开关（用服务器的 / 不改）：变了就重新算要写成的样子、下发同步请求
        for k in &SYNC_KINDS {
            let Some(on) = (k.patch)(p) else { continue };
            if on != (k.on)(&h) {
                self.tx.execute(&format!("UPDATE hosts SET {} = ?2 WHERE id = ?1", k.col), params![h.id, on as i64])?;
                if let Some((yes, no)) = k.op {
                    self.host_op(h.id, by, if on { yes } else { no })?;
                }
                self.config_changed = true;
            }
        }
        // 领取规则、兜底现在是全局的一份（`sched.rs`）：旧界面还从机器设置里改，改的就是全局的
        if p.rules.is_some() || p.fallback.is_some() {
            self.sched_update(p.rules.as_deref(), p.fallback.as_deref())?;
        }
        if let Some(on) = p.agent_auto {
            self.tx.execute("UPDATE hosts SET agent_auto = ?2 WHERE id = ?1", params![h.id, on as i64])?;
        }
        if let Some(paused) = p.paused {
            self.tx.execute("UPDATE hosts SET paused = ?2 WHERE id = ?1", params![h.id, paused as i64])?;
            // 恢复接单：排空结束；为升级暂停的也算人恢复了，升级完不再替人改
            if !paused {
                if h.drain_at.is_some() {
                    self.host_op(h.id, by, "恢复接单，排空结束")?;
                }
                self.tx.execute("UPDATE hosts SET drain_at = NULL, up_pause = 0 WHERE id = ?1", [h.id])?;
            }
            // 暂停接单时这一轮就结束；恢复接单后不补（PRD §9.1）
            if paused {
                self.tx.execute("UPDATE hosts SET round_at = NULL WHERE id = ?1", [h.id])?;
            }
        }
        if let Some(agent) = &p.agent {
            if !h.resolved().iter().any(|a| &a.id == agent && a.found) {
                return Err(BoardError::new(ErrorCode::AgentNotOnHost, format!("机器「{name}」上没有找到 agent「{agent}」")));
            }
            self.tx.execute("UPDATE hosts SET agent = ?2 WHERE id = ?1", params![h.id, agent])?;
        }
        if let Some(on) = p.auto.or(p.cron_on) {
            self.tx.execute("UPDATE hosts SET auto = ?2 WHERE id = ?1", params![h.id, on as i64])?;
        }
        if let Some(w) = &p.windows {
            let w = crate::sched::tidy_windows(w).map_err(BoardError::usage)?;
            self.tx.execute("UPDATE hosts SET windows = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&w).unwrap_or_default()])?;
        }
        // 机器上不再给 agent 指定供应商、模型（#576）：旧版界面还会发，说清楚去哪改
        if p.run.is_some() {
            return Err(BoardError::usage("机器上不再给 agent 指定模型：用哪个模型按角色定（Agent → 自动领取 → 分工），派发时带给这一个 agent 进程"));
        }
        if let Some(id) = p.resume.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
            let a = h.agents.iter().find(|a| a.id == id).ok_or_else(|| BoardError::usage(format!("机器「{name}」上没有 agent「{id}」")))?;
            // 没暂停的不用恢复（重复点、别的窗口已经点过）：不记操作。连续异常退出从现在重新数；额度用完的暂停（#668）提前结束
            let tripped = self.read().agent_tripped(&h, id)?;
            let quota = h.run.get(id).and_then(|r| r.quota_until).is_some_and(|t| t > self.now);
            if tripped || quota {
                let mut run = h.run.clone();
                let e = run.entry(id.to_string()).or_default();
                if tripped {
                    e.resumed_at = Some(self.now);
                }
                e.quota_until = None;
                self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&run).unwrap_or_default()])?;
                self.host_op(h.id, by, &format!("恢复了 {} 接单", a.name))?;
            }
            // 额度按账号算（#669）：报了同一账号、也因为额度暂停着的别的机器一起恢复
            if quota {
                for o in self.same_account(&h, id)? {
                    let mut run = o.run.clone();
                    let Some(e) = run.get_mut(id).filter(|e| e.quota_until.is_some_and(|t| t > self.now)) else { continue };
                    e.quota_until = None;
                    self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![o.id, serde_json::to_string(&run).unwrap_or_default()])?;
                    self.host_op(o.id, by, &format!("恢复了 {} 接单（同一账号，在 {name} 上恢复）", a.name))?;
                }
            }
        }
        if let Some(e) = &p.env {
            let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
            let check = h.env_check(&e.agent, &e.env)?;
            let mut run = h.run.clone();
            run.entry(e.agent.clone()).or_default().env = Some(check.env);
            self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&run).unwrap_or_default()])?;
            // 换了启动环境：配置文件、skill 目录可能在别的地方（WSL 里），重新写一次
            self.config_changed = true;
        }
        self.touch_hosts();
        // 暂停、恢复、会话上限、恢复 agent 都会改变谁能接：等着的「任意机器」派发重新挑一遍
        self.assign_any()?;
        self.read().host(name, self.now)
    }

    /// 旧版 board host 的「开始一轮领取」兼容接口：现在机器在接单时段里持续领卡，没有「一轮」了；
    /// 接口留着给旧版 board host 调，什么也不做，返回机器现在的样子
    pub fn host_round(&mut self, name: &str) -> Result<HostInfo> {
        self.read().host(name, self.now)
    }

    /// 移除机器（任务下发需求 §九），不吊销令牌：见 [`Self::host_remove_revoke`]。
    /// 服务端删除机器都走 `host_remove_revoke`（带不带吊销由请求决定），只有测试用不吊销的这一个
    #[cfg(test)]
    pub fn host_remove(&mut self, name: &str, by: &Actor, who: &str) -> Result<()> {
        self.host_remove_revoke(name, by, who, false).map(|_| ())
    }

    /// 移除机器（任务下发需求 §九）：取消它所有排队中的派发，停止正在跑的派发，删除机器设置；派发记录保留。
    /// revoke 时同时吊销它最近一次上报用的令牌（只在别的在线机器都不用这个令牌时），
    /// 返回吊销的令牌 ID
    pub fn host_remove_revoke(&mut self, name: &str, by: &Actor, who: &str, revoke: bool) -> Result<Option<i64>> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let mut revoked = None;
        if revoke {
            let t = self.read().host_token(&h, self.now)?;
            let Some(t) = t else {
                return Err(BoardError::usage(format!("机器「{name}」没有能吊销的令牌（还没上报过，或令牌已经吊销）")));
            };
            if !t.shared.is_empty() {
                return Err(BoardError::usage(format!("令牌「{}」还在被在线的机器 {} 使用，不能一起吊销", t.name, t.shared.join("、"))));
            }
            self.tx.execute("DELETE FROM auth_tokens WHERE id = ?1", [t.id])?;
            revoked = Some(t.id);
        }
        // 后派的先回去：卡片回到可领取状态最上面后仍按派发的先后排
        let queued: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE host_id = ?1 AND state = 'queued' ORDER BY id DESC", [h.id], |r| r.get(0))?;
        for id in queued {
            let d = self.drow(id)?;
            // 「任意机器」的派发不跟着这台机器取消：机器删掉之后收回，换别的机器（下面的 assign_any）
            if d.any_host {
                continue;
            }
            self.close_dispatch(&d, "canceled", None)?;
            let back = self.unreserve(d.board, d.card, by)?;
            let text = format!("移除了机器 {}，派发取消（{}）{back}", h.name, Self::dispatch_words(&d));
            self.track_dispatch(d.board, d.card, by, "dispatch", &text)?;
        }
        let running: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE host = ?1 AND state = 'running'", [name], |r| r.get(0))?;
        for id in running {
            self.request_stop(id, by, who)?;
        }
        self.tx.execute("DELETE FROM hosts WHERE id = ?1", [h.id])?;
        self.chats_drop(h.id)?;
        self.tx.execute("DELETE FROM host_ops WHERE host_id = ?1", [h.id])?;
        self.read().drop_host_picks(h.id)?;
        self.touch_hosts();
        self.assign_any()?;
        Ok(revoked)
    }

    /// 机器页顶部的「全部暂停 / 全部恢复」：所有机器一起设成暂停接单（或恢复），排队的派发照样排队。
    /// 返回所有机器
    pub fn hosts_pause_all(&mut self, paused: bool) -> Result<Vec<HostInfo>> {
        self.tx.execute("UPDATE hosts SET paused = ?1", [paused as i64])?;
        if paused {
            // 暂停接单时这一轮就结束（与单台机器一样，PRD §9.1）
            self.tx.execute("UPDATE hosts SET round_at = NULL WHERE round_at IS NOT NULL", [])?;
        }
        self.touch_hosts();
        self.assign_any()?;
        self.read().hosts(self.now)
    }

    /// 机器页顶部的「全部停止」：请求停止所有正在跑的派发，同时全部暂停（界面二次确认后调用）。
    /// 各台机器的 board host 收到停止请求后结束 agent 进程树并上报；排队的派发继续排队
    pub fn hosts_stop_all(&mut self, by: &Actor, who: &str) -> Result<Vec<HostInfo>> {
        let running: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE state = 'running' ORDER BY id", [], |r| r.get(0))?;
        for id in running {
            self.request_stop(id, by, who)?;
        }
        self.hosts_pause_all(true)
    }

    /// 机器改名（桌面端「设置 → 本机」保存机器名时调用）：机器记录原地改名，id 不变，
    /// 所有设置（最多几个会话、默认 agent、领取规则、定时领取、暂停 …）和历史派发记录都保留，
    /// 派发记录里的机器名也跟着改（在跑的派发按机器名找）。
    /// - 新名字的规则和 [`check_host_name`] 相同；正被另一台**在线**机器使用时 `HOST_NAME_TAKEN`；
    ///   被一台**离线**机器占用时 `USAGE`（先在「机器」里移除它再改）
    /// - 旧名字在服务端没有记录（这台机器还没连上来过）时 `NO_SUCH_HOST`：没有要改的
    pub fn host_rename(&mut self, from: &str, to: &str) -> Result<HostInfo> {
        let to = to.trim();
        check_host_name(to).map_err(BoardError::usage)?;
        let h = self.host_row(from)?.ok_or_else(|| no_such_host(from))?;
        if h.name != to {
            if let Some(other) = self.host_row(to)? {
                return Err(if other.online(self.now) {
                    BoardError::new(
                        ErrorCode::HostNameTaken,
                        format!("机器名「{to}」正被在线的 board host 使用：先停掉那台机器上的 board host，或换一个名字"),
                    )
                    .with_detail(json!({ "host": to }))
                } else {
                    BoardError::usage(format!("机器名「{to}」已经有一台离线的机器：在「机器」里移除它之后再改"))
                });
            }
            self.tx.execute("UPDATE hosts SET name = ?2 WHERE id = ?1", params![h.id, to])?;
            self.tx.execute("UPDATE dispatches SET host = ?2 WHERE host_id = ?1", params![h.id, to])?;
            self.tx.execute("UPDATE chats SET host = ?2 WHERE host_id = ?1", params![h.id, to])?;
            // 定时建卡按机器名派发：跟着改名
            self.tx.execute("UPDATE card_crons SET host = ?2 WHERE host = ?1", params![h.name, to])?;
            // 机器令牌绑定的名字也跟着改：移除后再连上来照样能登记（#507）
            if let Some(t) = h.token {
                if token_host_column(self.tx)? {
                    self.tx.execute("UPDATE auth_tokens SET host = ?2 WHERE id = ?1 AND host = ?3", params![t, to, h.name])?;
                }
            }
            self.touch_hosts();
        }
        self.read().host(to, self.now)
    }
}
