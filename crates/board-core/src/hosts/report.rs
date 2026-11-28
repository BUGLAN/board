//! 机器上报（心跳）：`board host` 每次上报的内容、服务端的处理、给它的任务，以及掉线机器的清扫。

use super::*;

/// board host 每 10 秒的上报（`POST /api/hosts/beat`）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostReport {
    pub name: String,
    /// 这个 board host 进程的标识
    pub instance: String,
    /// 同一台机器上一次异常退出的 board host 的标识：用来接回还在线的名字
    #[serde(default)]
    pub resume: Option<String>,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub agents: Vec<HostAgent>,
    /// 启动环境（旧版 board host 不上报）
    #[serde(default)]
    pub envs: Vec<HostEnv>,
    /// 本机配置里的工作空间（`[host] workspace`，`--workspace` / `--dir`）；旧版 board host 不上报
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<HostWorkspace>,
    /// 本机派生出来、检查过的白板目录（`<工作空间>/<项目>`，加上 `--dir` 覆盖的）
    #[serde(default)]
    pub dirs: Vec<HostDir>,
    /// board host 已经应用的服务端工作空间版本；本机配置先改了时还是旧版本
    #[serde(default)]
    pub dirs_rev: i64,
    /// 本机正在跑（有进程）的派发
    #[serde(default)]
    pub running: Vec<i64>,
    /// 本机有 agent 进程的机器对话（chat.rs）；旧版 board host 不带
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chats: Option<Vec<i64>>,
    /// board host 退出：机器马上离线
    #[serde(default, rename = "final")]
    pub done: bool,
    /// 处理完（或开始处理）的请求序号（`HostAsk.id`）：服务端收到后清掉这个请求
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<i64>,
    /// 升级（或重启）没成：原因，记进操作记录；为升级暂停的接单随之恢复
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_error: Option<String>,
    /// 会做自检（PRD §10.12「自检」）；旧版 board host 不带
    #[serde(default)]
    pub selftest: bool,
    /// 会按请求同步配置、上报结果（[`ConfigSync`]）；旧版 board host 不带
    #[serde(default)]
    pub config_sync: bool,
    /// 会按请求重扫本机的 skill（`rescan-skills`）；旧版 board host 不带
    #[serde(default)]
    pub rescan_skills: bool,
    /// 本机各 agent skill 目录里、不是同步装的 skill（「从机器导入」用）；旧版 board host 不带，带了就算会导入
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_skills: Option<Vec<crate::skills::LocalSkill>>,
    /// 资源（PRD §10.12「资源」）：后台最近一次采样；旧版 board host 不带
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub res: Option<crate::hostres::HostRes>,
    /// 上次上报之后看到的额度：跑卡时 agent 推来的、按「刷新」查到（或没查到）的；没有新的时为空
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quotas: Vec<QuotaReport>,
}

/// 发给一台 board host 的任务：设置、在跑的派发（含停止请求）、排队的派发（按创建先后）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostWork {
    pub host: HostInfo,
    pub running: Vec<Dispatch>,
    pub queued: Vec<Dispatch>,
    pub now: i64,
    /// skills 共用库（不带内容）：board host 按它同步本机；服务端在前面补上随它版本的 board-skills。旧版服务端没有这一项
    #[serde(default)]
    pub skills: Option<Vec<crate::skills::SkillInfo>>,
    /// MCP 服务器共用配置的索引（名称、版本）：变了 board host 用令牌接口取完整内容。旧版服务端没有这一项
    #[serde(default)]
    pub mcp: Option<Vec<crate::mcp::McpIndex>>,
    /// 模型供应商共用配置：索引（取完整内容用）；每个 agent 用哪个在 `host.agents` 的 `run_*` 里。旧版服务端没有这一项
    #[serde(default)]
    pub providers: Option<crate::providers::ProviderWork>,
    /// Git 凭据的版本号（变了 board host 用令牌接口取完整内容）；服务端没设时是空的。旧版服务端没有这一项
    #[serde(default)]
    pub git: Option<String>,
    /// 这台机器要写成的样子（[`R::config_key`]）：board host 同步完带着它回报，服务端据此知道写好的是哪一版
    #[serde(default)]
    pub config_key: String,
    /// 等它列的目录（选工作空间用）：服务端放在内存里、发任务时补上，board host 列完回传（`POST /api/hosts/{名字}/ls/{ID}`）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ls: Vec<LsAsk>,
    /// 机器对话（chat.rs）：有还没交给它的消息、停止请求、这一轮在跑的。旧版服务端没有
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chats: Vec<ChatTask>,
}

/// 一个「列目录」请求：空路径是最上面一层（Windows 列盘符，macOS / Linux 是用户目录）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsAsk {
    pub id: i64,
    #[serde(default)]
    pub path: String,
}

/// 一个子文件夹
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// 名称（最上面一层是完整路径：`C:\`、`/Users/me`）
    pub name: String,
    /// 完整路径（`~` 已经展开）
    pub path: String,
    /// 它本身是 git 仓库（里面有 `.git`）
    #[serde(default)]
    pub git: bool,
    /// 它下面一层里有几个 git 仓库
    #[serde(default)]
    pub repos: i64,
}

/// 列目录的结果（board host 回传）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirList {
    /// 列的是哪个文件夹（`~` 已经展开）；最上面一层为空
    #[serde(default)]
    pub path: String,
    /// 从最上面一层到它自己的每一级（第一个是空的最上面一层）：界面据此一层一列展开
    #[serde(default)]
    pub chain: Vec<String>,
    #[serde(default)]
    pub entries: Vec<DirEntry>,
    /// 列不了：原因（不存在、没有权限）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<'a> W<'a> {
    /// board host 的上报（每 10 秒；启动时第一次）：建立或更新机器，替在跑的派发发心跳，返回它的任务。
    ///
    /// - 机器名正被另一个在线的 board host 使用（标识不同，也不是 resume 接回）时 `HOST_NAME_TAKEN`
    /// - 令牌绑定：机器记下这次上报用的令牌（`token`，人和本机模式为空），之后令牌能调用的 board host 接口只认它
    ///   （[`R::host_owned_by`]）。已有的机器绑着另一个还没吊销的令牌时，别的令牌不能上报（在线、离线、resume 都一样）：
    ///   `HOST_NAME_TAKEN`，免得别的令牌接管这台机器、再读走只发给它的密钥。原来的令牌被吊销了、或还没绑过令牌时可以换绑
    /// - 被移除过的机器再连上来，按新机器建立设置
    /// - 在跑的派发：本机有进程的，替它的名字发 agent 级心跳（维持名字在线和卡片的持有）；本机没有进程的
    ///   （board host 重启后接回），按「agent 退出，卡片仍由这个名字持有」处理，结果「异常退出」
    /// - `final`：board host 退出，机器马上离线
    pub fn host_beat(&mut self, rep: &HostReport, token: Option<i64>) -> Result<HostWork> {
        // 旧版 board host 还上报已经移除的内置 agent（pi）：丢掉，机器详情、派发对话框里都不出现（#484）
        let cleaned;
        let had_removed = rep.agents.iter().any(|a| crate::agentdef::is_removed(&a.id, a.custom));
        let rep = if had_removed {
            cleaned = HostReport { agents: rep.agents.iter().filter(|a| !crate::agentdef::is_removed(&a.id, a.custom)).cloned().collect(), ..rep.clone() };
            &cleaned
        } else {
            rep
        };
        // 上报的 git origin 去掉 http(s) 地址里的用户信息再用（#686）：用令牌克隆的仓库会把令牌原样报上来，
        // 存进库里之后任何令牌 GET /api/workspace 都读得到
        let stripped;
        let rep = if rep.dirs.iter().any(|d| d.origin.as_deref().is_some_and(|o| crate::fit::strip_userinfo(o) != o)) {
            stripped = HostReport {
                dirs: rep.dirs.iter().map(|d| HostDir { origin: d.origin.as_deref().map(crate::fit::strip_userinfo), ..d.clone() }).collect(),
                ..rep.clone()
            };
            &stripped
        } else {
            rep
        };
        check_host_name(&rep.name).map_err(BoardError::usage)?;
        if rep.instance.trim().is_empty() {
            return Err(BoardError::usage("缺少 instance"));
        }
        for a in &rep.agents {
            check_agent_id(&a.id).map_err(BoardError::usage)?;
        }
        for e in &rep.envs {
            if e.id.trim().is_empty() || e.id.len() > 64 {
                return Err(BoardError::usage(format!("启动环境标识不对：{}", e.id)));
            }
        }
        let mut seen: Vec<&str> = vec![];
        let known = self.host_of_proc(&rep.name, &rep.instance)?;
        // 旧版 board host 的 `board` 一列是白板标识（不是 `<项目>/<白板>`）：没配工作空间的机器按老办法用它。
        // 服务端按工作空间配好了的（新版）不认白板标识，存着的旧数据也不改
        let old = rep.workspace.is_none()
            && known.as_ref().is_none_or(|h| h.workspace_raw.is_none() || h.workspace().is_none_or(|w| w.path.is_empty()));
        for d in &rep.dirs {
            let legacy = old && !crate::valid_board(d.board.trim());
            if !legacy && !crate::valid_board(d.board.trim()) {
                return Err(BoardError::usage(format!("白板要写成 <项目>/<白板>：{}", d.board)));
            }
            if d.path.trim().is_empty() {
                return Err(BoardError::usage(format!("白板 {} 还没有目录", d.board)));
            }
            if seen.contains(&d.board.trim()) {
                return Err(BoardError::usage(format!("白板 {} 写了两次", d.board)));
            }
            seen.push(d.board.trim());
        }
        let now = self.now;
        let agents = serde_json::to_string(&rep.agents).unwrap_or_else(|_| "[]".into());
        let envs = serde_json::to_string(&rep.envs).unwrap_or_else(|_| "[]".into());
        // 改名后还用旧名字上报：认回改名后的记录（见 host_of_proc），下面都用记录里的名字
        let found = self.host_of_proc(&rep.name, &rep.instance)?;
        if let Some(t) = token {
            match found.as_ref() {
                // 机器就绑着这个令牌：照常上报
                Some(h) if h.token == Some(t) => {}
                Some(h) => {
                    // 绑着另一个还在的令牌：不让接管（见上面「令牌绑定」）
                    if let Some(old) = h.token {
                        if let Some(tn) = self.read().live_token_name(old)? {
                            return Err(BoardError::new(
                                ErrorCode::HostNameTaken,
                                format!(
                                    "机器名「{}」绑定的是另一个访问令牌「{tn}」：换一个 --name；或在界面「Agent → 访问令牌」吊销那个令牌、或在「机器」里移除这台机器，再用「机器 → 添加机器」生成的命令连上来",
                                    h.name
                                ),
                            )
                            .with_detail(json!({ "host": h.name })));
                        }
                    }
                    // 原来的令牌吊销了、或还没绑过令牌：换绑也算登记，同新机器（#507）
                    self.read().token_may_register(t, &h.name)?;
                }
                // 新机器：只有「添加机器」新建的、绑定了这个机器名的令牌能登记（#507）。
                // 结束上报（`final`）照旧按机器不存在处理
                None if !rep.done => self.read().token_may_register(t, &rep.name)?,
                None => {}
            }
        }
        let name = found.as_ref().map_or_else(|| rep.name.clone(), |h| h.name.clone());
        match found {
            None => {
                if rep.done {
                    return Err(no_such_host(&rep.name));
                }
                let mut workspace = rep.workspace.clone().map(|w| clean_workspace(&w)).filter(|w| !w.path.is_empty() || !w.overrides.is_empty());
                let mut dirs = rep.dirs.clone();
                for d in &mut dirs {
                    d.clone = None;
                    d.reported = true;
                }
                // 旧版 board host（只报白板与目录）：导出成工作空间，老机器升级前照样能接卡
                if workspace.is_none() {
                    workspace = exported_workspace(&dirs);
                }
                if workspace.is_none() {
                    // 导出不了（旧版报的是白板标识、不是路径）：存一个空的，按老办法用 dirs
                    workspace = Some(HostWorkspace {
                        path: String::new(),
                        // 这几块在盘上已经摆着（旧版 `--dir` 固定）：不算只给界面上改
                        overrides: dirs.iter().map(|d| HostDirOverride { board: d.board.clone(), path: d.path.clone() }).collect(),
                        ..Default::default()
                    });
                }
                let dirs = serde_json::to_string(&dirs).unwrap_or_else(|_| "[]".into());
                let workspace = workspace.map(|w| {
                    serde_json::to_string(&w).unwrap_or_else(|_| "{\"path\":\"\"}".into())
                }).unwrap_or_default();
                self.tx.execute(
                    // sync_git 显式写 0：新机器默认不改。老库的这一列是 #386 迁移时 ALTER 加的，表上的默认值还是 1，
                    // 改 CREATE TABLE 不会改已经存在的表，所以不能靠列默认值
                    "INSERT INTO hosts (name, token, instance, os, version, agents, workspace, dirs, dirs_rev, dirs_at, dirs_by, envs, first_at, beat_at, sync_git)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULLIF(?7, ''), ?8, 1, ?9, '本机', ?10, ?9, ?9, 0)",
                    params![rep.name, token, rep.instance, rep.os, rep.version, agents, workspace, dirs, now, envs],
                )?;
                if rep.selftest {
                    let st = SelfTest { able: true, ..Default::default() };
                    self.tx.execute("UPDATE hosts SET selftest = ?2 WHERE name = ?1", params![rep.name, serde_json::to_string(&st).unwrap_or_default()])?;
                }
                if let Some(local) = &rep.local_skills {
                    let si = SkillImport { able: true, rescan: rep.rescan_skills, local: local.clone(), ..Default::default() };
                    self.tx.execute("UPDATE hosts SET skill_import = ?2 WHERE name = ?1", params![rep.name, serde_json::to_string(&si).unwrap_or_default()])?;
                }
                if rep.config_sync {
                    let cs = ConfigSync { able: true, ..Default::default() };
                    self.tx.execute("UPDATE hosts SET config_sync = ?2 WHERE name = ?1", params![rep.name, serde_json::to_string(&cs).unwrap_or_default()])?;
                }
                if let Some(res) = rep.res.clone().and_then(|r| r.received(None, now)) {
                    self.tx.execute("UPDATE hosts SET res = ?2 WHERE name = ?1", params![rep.name, serde_json::to_string(&res).unwrap_or_default()])?;
                }
                // 添加机器时选了「上线后照 <机器> 配」（hostcopy.rs）：照来源机器现在的设置配
                self.host_preset_apply(&rep.name, token)?;
                self.touch_hosts();
            }
            Some(h) => {
                let online = h.online(now);
                if online && h.instance != rep.instance && rep.resume.as_deref() != Some(h.instance.as_str()) {
                    return Err(BoardError::new(
                        ErrorCode::HostNameTaken,
                        format!("机器名「{}」正被在线的 board host 使用：换一个 --name，或先停掉那台机器上的 board host", rep.name),
                    )
                    .with_detail(json!({ "host": rep.name })));
                }
                if rep.done {
                    if h.instance == rep.instance && h.off_at.is_none() {
                        self.tx.execute("UPDATE hosts SET off_at = ?2, round_at = NULL WHERE id = ?1", params![h.id, now])?;
                        self.chats_lost(h.id, None, "board host 退出了")?;
                        self.touch_hosts();
                        // 分给它、还没开始的「任意机器」派发收回，换别的机器
                        self.assign_any()?;
                    }
                    return self.read().host_work(&name, now);
                }
                // 工作空间：以前存的是旧的「白板与目录」时，上报里带了工作空间（新版 board host）才换成它
                let stored_raw = h.workspace_raw.as_deref();
                let stored = workspace_of(stored_raw, &h.dirs);
                // 旧版 board host（不报工作空间，只报白板与目录）：存着的没有工作空间时，从它报的目录导出一次
                let exported = stored_raw.is_none().then(|| exported_workspace(&rep.dirs)).flatten();
                let reported = rep
                    .workspace
                    .as_ref()
                    .map(|w| clean_workspace(w))
                    .filter(|w| !w.path.is_empty() || !w.overrides.is_empty())
                    .or_else(|| exported.clone());
                let (mut workspace, mut changed_settings) = (stored.clone(), false);
                if let Some(w) = reported.clone() {
                    // 和存着的比：变了、并且没有远程修改在先就是本机改的
                    changed_settings = !same_workspace(stored.as_ref(), Some(&w)) && rep.dirs_rev == h.dirs_rev;
                    if changed_settings || (exported.is_some() && stored.is_none()) {
                        workspace = Some(w);
                        changed_settings = true;
                    }
                }
                // 存着的是「没有工作空间」、旧版报的白板标识派生不出目录：按它报的目录导出一个（老机器升级前照样
                // 能派卡），老的白板标识留着，服务端按老办法用
                if let (Some(cur), Some(exp)) = (workspace.as_mut().filter(|w| w.path.trim().is_empty()), exported_workspace(&rep.dirs)) {
                    cur.path = exp.path;
                    for o in exp.overrides {
                        if !cur.overrides.iter().any(|x| x.board == o.board) {
                            cur.overrides.push(o);
                        }
                    }
                    changed_settings = true;
                }
                // 存着每块白板的目录：board host 报的目录状态合进来；新露出来的白板记下它的目录（路径由工作空间派生）。
                // 旧版 board host 上报的白板标识也存着，工作空间为空时照样能派卡
                let mut dirs = h.dirs.clone();
                // 存着的路径按现在的工作空间和项目目录重新派生：项目设置里改了项目目录（#488），board host 按新路径报上来的
                // 检查结果才合得上；路径变了的，旧路径的检查结果、克隆状态不再算数
                if let Some(ws) = workspace.as_ref().map(|w| HostWorkspace { projects: h.proj_dirs.clone(), ..w.clone() }) {
                    for d in dirs.iter_mut() {
                        if let Some(x) = derive_dir(&ws, &d.board, &rep.os).filter(|x| x.path != d.path) {
                            *d = HostDir { board: d.board.clone(), path: x.path, fixed: x.fixed, ..Default::default() };
                        }
                    }
                }
                merge_dir_checks(&mut dirs, &rep.dirs);
                if let Some(rw) = reported.as_ref().filter(|rw| !rw.path.is_empty()).map(|w| HostWorkspace { projects: h.proj_dirs.clone(), ..w.clone() }) {
                    let rw = &rw;
                    for r in &rep.dirs {
                        if dirs.iter().any(|d| d.board == r.board) {
                            continue;
                        }
                        if let Some(d) = derive_dir(rw, &r.board, &rep.os) {
                            dirs.push(HostDir { reported: true, ..d });
                        }
                    }
                    // 刚露出来的白板：这次上报里带着的目录状态马上合进来，不等下一次上报
                    merge_dir_checks(&mut dirs, &rep.dirs);
                }
                // 报上来的目录：不在存着的主体里的（`--dir` 覆盖的、旧版 board host 的白板标识）补上；工作空间派生得出
                // 那几块的不补，免得按上报的白板标识再存一份派发不了的目录
                for r in &rep.dirs {
                    if dirs.iter().any(|d| d.board == r.board) || workspace.as_ref().and_then(|w| derive_dir(w, &r.board, &rep.os)).is_some() {
                        continue;
                    }
                    let mut d = r.clone();
                    d.reported = true;
                    dirs.push(d);
                }
                let mut dirs_rev = h.dirs_rev;
                let mut dirs_at = h.dirs_at;
                let mut dirs_by = h.dirs_by.clone();
                if changed_settings {
                    dirs_rev = h.dirs_rev + 1;
                    dirs_at = Some(now);
                    dirs_by = Some("本机".into());
                }
                let dirs = serde_json::to_string(&dirs).unwrap_or_else(|_| "[]".into());
                let workspace = workspace.as_ref().map(|w| serde_json::to_string(w).unwrap_or_default()).unwrap_or_default();
                let differs = !online
                    || h.instance != rep.instance
                    || h.os != rep.os
                    || h.version != rep.version
                    || h.agents_raw != agents
                    || h.envs_raw != envs
                    || h.dirs_raw != dirs
                    || h.workspace_raw.clone().unwrap_or_default() != workspace;
                self.tx.execute(
                    "UPDATE hosts SET token = ?2, instance = ?3, os = ?4, version = ?5, agents = ?6, dirs = ?7, beat_at = ?8, off_at = NULL, envs = ?9,
                                      dirs_rev = ?10, dirs_at = ?11, dirs_by = ?12, workspace = NULLIF(?13, '') WHERE id = ?1",
                    params![h.id, token, rep.instance, rep.os, rep.version, agents, dirs, now, envs, dirs_rev, dirs_at, dirs_by, workspace],
                )?;
                // 资源：每次都在变，只写库、不算机器有变化（不推送）；界面开着机器详情时每 10 秒取一次机器列表。
                // 这次没带（旧版 board host、后台采样卡住太久）：清掉，界面写「不上报」，不留着越来越旧的数据
                let res = rep.res.clone().and_then(|r| r.received(h.res.as_ref(), now)).map(|r| serde_json::to_string(&r).unwrap_or_default());
                self.tx.execute("UPDATE hosts SET res = ?2 WHERE id = ?1", params![h.id, res])?;
                // 换了一个 board host 进程（重启接回）：上一个进程的那一轮领取不再继续
                if h.instance != rep.instance && h.round_at.is_some() {
                    self.tx.execute("UPDATE hosts SET round_at = NULL WHERE id = ?1", [h.id])?;
                }
                self.host_acks(&h, rep)?;
                self.auto_upgrade_host(&name)?;
                self.auto_update_agents(&name, rep)?;
                // 换成了会（或不会）按请求同步配置的 board host
                if h.config_sync.able != rep.config_sync {
                    let cs = ConfigSync { able: rep.config_sync, ask: if rep.config_sync { h.config_sync.ask.clone() } else { None }, ..h.config_sync.clone() };
                    self.tx.execute("UPDATE hosts SET config_sync = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&cs).unwrap_or_default()])?;
                    self.touch_hosts();
                    self.config_changed = true;
                }
                // 找到的 agent、启动环境变了：要写的地方跟着变，重新算一遍要不要同步
                if h.agents_raw != agents || h.envs_raw != envs {
                    self.config_changed = true;
                }
                // 本机的 skill（「从机器导入」）：带了、变了才写；带了就算会导入。换回旧版 board host 时把重扫能力也降下来
                if let Some(local) = &rep.local_skills {
                    if !h.skill_import.able || h.skill_import.rescan != rep.rescan_skills || &h.skill_import.local != local {
                        let si = SkillImport { able: true, rescan: rep.rescan_skills, local: local.clone(), ..h.skill_import.clone() };
                        self.tx.execute("UPDATE hosts SET skill_import = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&si).unwrap_or_default()])?;
                        self.touch_hosts();
                    }
                } else if h.skill_import.rescan {
                    let si = SkillImport { rescan: false, ..h.skill_import.clone() };
                    self.tx.execute("UPDATE hosts SET skill_import = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&si).unwrap_or_default()])?;
                    self.touch_hosts();
                }
                // 换成了会（或不会）自检的 board host
                if h.selftest.able != rep.selftest {
                    let st = SelfTest { able: rep.selftest, ..h.selftest.clone() };
                    self.tx.execute("UPDATE hosts SET selftest = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&st).unwrap_or_default()])?;
                    self.touch_hosts();
                }
                if differs {
                    self.touch_hosts();
                }
            }
        }
        // 额度：跑卡时 agent 推来的、按「刷新」查到（或没查到）的
        if !rep.quotas.is_empty() {
            self.note_quota_reports(&name, &rep.quotas)?;
        }
        // 掉线后回来（PRD §9.1「掉线后接回」）：它报着还在跑、服务端已经按「机器掉线」结束的派发，卡片还没被别人领走就接回
        for id in &rep.running {
            self.dispatch_revive(*id, &name, token)?;
        }
        // 在跑的派发。board host 报的是它知道的派发 ID：board next 刚换到下一张、board host 还没取到新任务时，
        // 它报的还是上一张。同一个会话（同一个进程）里的任何一张在报，就算有进程
        let running: Vec<i64> =
            self.collect("SELECT id FROM dispatches WHERE host = ?1 AND state = 'running' ORDER BY id", [&name], |r| r.get(0))?;
        let mut alive: Vec<i64> = vec![];
        for id in &rep.running {
            if let Some(s) = self
                .tx
                .query_row("SELECT COALESCE(session, id) FROM dispatches WHERE id = ?1", [id], |r| r.get::<_, i64>(0))
                .optional()?
            {
                alive.push(s);
            }
        }
        for id in running {
            let d = self.drow(id)?;
            if rep.running.contains(&id) || alive.contains(&d.session) {
                if let Some(name) = &d.name {
                    match self.agent_beat_by(name, false, crate::agents::BeatBy::Host) {
                        Ok(_) => {}
                        // 名字已经离线：扫描会按机器掉线结束这次派发
                        Err(e) if e.code == ErrorCode::NameOffline => {}
                        Err(e) => return Err(e),
                    }
                }
            } else {
                self.finish(&d, None, &Tail { lines: &[], events: &[], quota: None }, Finish::Missing)?;
            }
        }
        // 机器对话：这一轮在跑、消息已经交给它，它却没报这个对话的进程（board host 重启过、进程没了）
        if let (Some(alive), Some(h)) = (rep.chats.as_deref(), self.host_row(&name)?) {
            self.chats_lost(h.id, Some(alive), "agent 进程不在了（board host 重启过）")?;
        }
        // 出了接单时段（或关了自动接单）：这一段接单结束，下次进入时重新算
        if let Some(h) = self.host_row(&name)? {
            if h.round_at.is_some() && !h.auto_active(now) {
                self.tx.execute("UPDATE hosts SET round_at = NULL WHERE id = ?1", [h.id])?;
            }
        }
        if had_removed {
            if let Some(h) = self.host_row(&name)? {
                self.drop_removed_run(h.id, &rep.agents)?;
            }
        }
        // 这台机器上线了、空出位置了：等着的「任意机器」派发挑一遍机器，分给它的随这次的任务一起发回去
        self.assign_any()?;
        self.read().host_work(&name, now)
    }

    /// board host 上报的额度：查到的记成这台机器上这个 agent 最近一次的额度（时间按服务端收到的算），清掉上次没查到的原因；
    /// 没查到的记下原因。不暂停接单（额度用完的暂停只看派发结束时的，[`W::note_quota`]）
    pub(super) fn note_quota_reports(&mut self, name: &str, reports: &[QuotaReport]) -> Result<()> {
        let Some(h) = self.host_row(name)? else { return Ok(()) };
        let mut run = h.run.clone();
        for r in reports {
            let e = run.entry(r.agent.clone()).or_default();
            match (&r.quota, &r.error) {
                (Some(q), _) => {
                    let mut q = q.clone();
                    q.at = self.now;
                    q.text = q.text.chars().take(300).collect();
                    e.quota = Some(q);
                    e.quota_fail = None;
                }
                (None, Some(why)) => e.quota_fail = Some(QuotaFail { at: self.now, why: why.chars().take(300).collect() }),
                (None, None) => {}
            }
        }
        if run != h.run {
            self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&run).unwrap_or_default()])?;
            self.touch_hosts();
        }
        Ok(())
    }

    /// 人点「刷新额度」：记下请求时间，这台机器的 board host 取到任务就查一次（Claude Code 发 `/usage`、Codex 发 `/status`，
    /// 不调模型、不花额度），查完随上报带回。机器要在线、这个 agent 要会查（[`QUOTA_PROBE_AGENTS`]、board host 报了 `quota_probe`）
    pub fn host_quota_ask(&mut self, name: &str, agent: &str) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let a = h.resolved().into_iter().find(|a| a.id == agent).ok_or_else(|| BoardError::usage(format!("机器「{name}」上没有 agent「{agent}」")))?;
        if !h.online(self.now) {
            return Err(BoardError::usage(format!("{name} 离线：上线后才能查额度")));
        }
        if !QUOTA_PROBE_AGENTS.iter().any(|(id, _)| *id == agent) {
            return Err(BoardError::usage(format!("{} 不会报额度", a.name)));
        }
        if !a.quota_probe {
            return Err(BoardError::usage(format!("{name} 的 board 版本太旧，升级后才能查额度")));
        }
        let mut run = h.run.clone();
        run.entry(agent.to_string()).or_default().quota_ask = Some(self.now);
        self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&run).unwrap_or_default()])?;
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// 机器的运行设置（hosts.run）里去掉已经移除的内置 agent 那一项；这台机器上有同名的自定义 agent 时留着（#484）
    pub(super) fn drop_removed_run(&mut self, id: i64, agents: &[HostAgent]) -> Result<()> {
        let raw: String = self.tx.query_row("SELECT run FROM hosts WHERE id = ?1", [id], |r| r.get(0))?;
        let mut run: BTreeMap<String, serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
        let before = run.len();
        run.retain(|k, _| !crate::agentdef::REMOVED_IDS.contains(&k.as_str()) || agents.iter().any(|a| &a.id == k));
        if run.len() != before {
            self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![id, serde_json::to_string(&run).unwrap_or_default()])?;
        }
        Ok(())
    }

    /// 只让在线的 board host 重新取一次任务（服务端内存里有了新的「列目录」请求）
    pub fn hosts_poke(&mut self) {
        self.touch_hosts();
    }

    /// 扫描（服务端每秒一次）：上报中断超过 50 秒的机器标为离线（这一轮领取不再继续）；名字已经离线的在跑派发结束，
    /// 结果「机器掉线」。卡片按现有的心跳超时规则释放（在 sweep_board 里，先于这里执行）
    pub(crate) fn sweep_hosts(&mut self) -> Result<usize> {
        let n = self.tx.execute(
            "UPDATE hosts SET off_at = beat_at + ?2, round_at = NULL WHERE off_at IS NULL AND beat_at + ?2 <= ?1",
            params![self.now, HOST_GAP_MS],
        )?;
        if n > 0 {
            self.touch_hosts();
            // 掉线的机器上还没开始的「任意机器」派发收回，换别的机器
            self.assign_any()?;
            // 掉线的机器上还没回答完的对话：这一轮记一条出错，对话留着
            let off: Vec<i64> = self.collect("SELECT id FROM hosts WHERE off_at = beat_at + ?1", [HOST_GAP_MS], |r| r.get(0))?;
            for id in off {
                self.chats_lost(id, None, &format!("机器掉线（超过 {} 秒没有上报）", HOST_GAP_MS / 1000))?;
            }
        }
        let lost: Vec<i64> = self.collect(
            "SELECT d.id FROM dispatches d LEFT JOIN agents a ON a.name = d.name
             WHERE d.state = 'running' AND (a.name IS NULL OR a.off_at IS NOT NULL OR a.beat_at + ?2 <= ?1)",
            params![self.now, HOST_GAP_MS],
            |r| r.get(0),
        )?;
        for id in &lost {
            let d = self.drow(*id)?;
            self.close_dispatch(&d, "lost", None)?;
            let text = format!("派发结束：机器掉线，超过 {} 秒没有收到 {} 的上报（{}）", HOST_GAP_MS / 1000, d.host, Self::dispatch_words(&d));
            self.track_dispatch(d.board, d.card, &Actor::system(), "dispatch", &text)?;
            self.cancel_session_rest(d.session, "机器掉线")?;
        }
        Ok(n + lost.len())
    }
}
