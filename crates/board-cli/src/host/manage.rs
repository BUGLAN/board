//! 远程管理：界面上发来的请求（重新检查、重启、升级、排空）、自检。

use super::*;

/// 远程管理请求处理完、退出主循环之后要做的事
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum After {
    /// 用同样的参数重新启动自己（重新读配置、重新检查）
    Restart,
    /// 桌面端运行：退出，交给桌面端升级
    DesktopUpgrade,
}

/// 用同样的参数重新启动自己（远程「重启 board host」、命令行升级装好新版本之后）。
/// macOS、Linux 原地换进程（进程号、桌面端接的标准输入都不变）；Windows 上桌面端运行时退出码 EXIT_RESTART 交给桌面端重启，
/// 命令行运行时启动一个新的、自己退出
pub(super) fn restart_self(exe: &Path, p: &Paint) -> Result<()> {
    let _ = writeln!(std::io::stdout(), "{} {}", p.dim(&clock()), "重新启动 board host…");
    // 装成服务时由守护进程重启（Windows 上不能原地换进程，换了守护进程也看不到新的那个）
    if crate::service::supervised() {
        std::process::exit(EXIT_RESTART);
    }
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = Command::new(exe).args(&args).env_remove("BOARD_HOST_ASK_ERROR").exec();
        Err(BoardError::usage(format!("重新启动 board host 失败：{e}")))
    }
    #[cfg(not(unix))]
    {
        if under_desktop() {
            std::process::exit(EXIT_RESTART);
        }
        Command::new(exe)
            .args(&args)
            .env_remove("BOARD_HOST_ASK_ERROR")
            .spawn()
            .map_err(|e| BoardError::usage(format!("重新启动 board host 失败：{e}")))?;
        std::process::exit(0);
    }
}

impl Host<'_> {
    /// 处理界面上发来的远程管理请求（PRD §9.1「远程管理」）。返回 (要马上上报, 要退出主循环)
    /// - 重新检查：重新查找启动环境、本机 agent、board-skills，马上上报
    /// - 重启、升级：等本机没有在跑的 agent（服务端已经按「人停止」通知要结束的那些），先上报「开始处理」，
    ///   再重启自己；升级时桌面端运行的交给桌面端，命令行运行的执行 `board update`，装好后用新版本重启
    pub(super) fn handle_ask(&mut self) -> (bool, bool) {
        let Some(ask) = self.work.as_ref().and_then(|w| w.host.ask.clone()) else { return (false, false) };
        if self.asked == Some(ask.id) || self.updating.is_some() || self.agent_job.is_some() {
            return (false, false);
        }
        match ask.kind.as_str() {
            "recheck" => {
                self.recheck();
                self.asked = Some(ask.id);
                (true, false)
            }
            "restart" | "upgrade" => {
                if !self.runs.is_empty() || !self.ended.is_empty() {
                    return (false, false);
                }
                // 先让服务端知道已经开始处理（清掉请求），免得重启后的新进程又处理一遍
                self.asked = Some(ask.id);
                match self.beat(false) {
                    Ok(w) => self.set_work(w),
                    Err(_) => {
                        self.asked = None;
                        return (false, false);
                    }
                }
                self.ask_started(&ask)
            }
            "agent-install" | "agent-update" => self.begin_agent_ask(&ask),
            "rescan-skills" => {
                self.asked = Some(ask.id);
                if !(self.skill_targets.is_empty() && self.sync_job.is_some()) {
                    let local = skills::local_skills(&self.skill_targets);
                    if local != self.local_skills {
                        self.local_skills = local;
                        self.beat_soon = true;
                    }
                }
                self.say(format!("{} 重扫完了本机的 skills：{} 个", self.p.green("✓"), self.local_skills.len()));
                (true, false)
            }
            _ => (false, false),
        }
    }

    pub(super) fn ask_started(&mut self, ask: &HostAsk) -> (bool, bool) {
        let by = &ask.by;
        if ask.kind == "restart" {
            self.say(format!("{} {by} 请求重启 board host", self.p.yellow("↻")));
            self.after = Some(After::Restart);
            return (false, true);
        }
        let to = ask.to.clone().unwrap_or_default();
        self.say(format!("{} {by} 请求升级到 {to}", self.p.yellow("↑")));
        if under_desktop() {
            self.after = Some(After::DesktopUpgrade);
            return (false, true);
        }
        let _ = std::fs::create_dir_all(&self.logs);
        let log = self.logs.join("update.log");
        let started = create_log(&log).and_then(|(out, err)| {
            let mut c = Command::new(&self.exe);
            c.arg("update").stdin(Stdio::null()).stdout(out).stderr(err).env("NO_COLOR", "1").env("BOARD_UPDATE_BY_HOST", "1");
            envs::detach(&mut c);
            c.spawn()
        });
        match started {
            Ok(child) => {
                self.updating = Some((child, Instant::now(), log));
                (false, false)
            }
            Err(e) => {
                self.update_failed(format!("没法运行 board update：{e}"));
                (true, false)
            }
        }
    }

    /// 命令行运行时的升级（`board update`）跑完了没有：成功就用新版本重启（返回要退出主循环），失败记下原因马上上报
    pub(super) fn poll_update(&mut self) -> (bool, bool) {
        let Some((child, begun, log)) = self.updating.as_mut() else { return (false, false) };
        let status = child.try_wait().ok().flatten();
        if status.is_none() && begun.elapsed() < UPDATE_LIMIT {
            return (false, false);
        }
        let log = log.clone();
        let tail = tail_lines(&log, 5, None);
        let (child, ..) = self.updating.take().expect("上面取过");
        let mut child = child;
        match status {
            Some(s) if s.success() => {
                self.say(format!("{} 升级好了：{}", self.p.green("✓"), tail.last().cloned().unwrap_or_default()));
                self.after = Some(After::Restart);
                (false, true)
            }
            Some(_) => {
                let text = tail.join(" ");
                let why = if text.contains("未知命令") {
                    // 服务端认这半句（NO_SELF_UPDATE）：旧版本不会自己升级，自动升级不再重试
                    format!("这个版本的 board 还不能自己升级（{}）：到这台机器上手动升级一次", board_core::hosts::NO_SELF_UPDATE)
                } else {
                    tail.last().cloned().unwrap_or_else(|| "board update 失败".into())
                };
                self.update_failed(why);
                (true, false)
            }
            None => {
                kill_tree(child.id(), true);
                let _ = child.kill();
                let _ = child.wait();
                self.update_failed(format!("board update 超过 {} 分钟没跑完，已结束", UPDATE_LIMIT.as_secs() / 60));
                (true, false)
            }
        }
    }

    pub(super) fn update_failed(&mut self, why: String) {
        self.say(format!("{} 升级失败：{why}", self.p.red("✗")));
        self.ask_error = Some(format!("升级失败：{why}"));
    }

    /// 自检（PRD §10.12「自检」）：取到的任务里每个新的请求都开一个后台线程做（几个 agent 的单查同时做）；
    /// 做完一个回报一个，回报失败下一轮再报
    pub(super) fn poll_selftest(&mut self) {
        let done: Vec<i64> = self.selftest.iter().filter(|(_, h)| h.is_finished()).map(|(id, _)| *id).collect();
        for id in done {
            let h = self.selftest.remove(&id).expect("上面看过");
            let items = h.join().unwrap_or_default();
            self.selftest_out.push((id, items));
        }
        let mut outs = std::mem::take(&mut self.selftest_out).into_iter();
        while let Some((id, items)) = outs.next() {
            let body = json!({ "instance": self.instance, "id": id, "items": items });
            match self.rm().post::<HostInfo>(&format!("/api/hosts/{}/selftested", self.name), body) {
                Ok(_) => {
                    let bad: Vec<&SelfTestItem> = items.iter().filter(|i| !i.ok).collect();
                    if bad.is_empty() {
                        self.say(format!("{} 自检做完：{} 项都通过", self.p.green("✓"), items.len()));
                    } else {
                        let why: Vec<String> = bad.iter().map(|i| format!("{}：{}", i.key, i.why.as_deref().unwrap_or("没通过"))).collect();
                        self.say(format!("{} 自检做完：{} 项没通过（{}）", self.p.yellow("!"), bad.len(), why.join("；")));
                    }
                    self.selftest_seen.insert(id);
                }
                Err(e) if e.code == ErrorCode::Unreachable => {
                    // 连不上：这个和后面的都留到下一轮再报
                    self.selftest_out.push((id, items));
                    self.selftest_out.extend(outs.by_ref());
                }
                Err(e) => {
                    self.say(format!("{} 自检结果没报上去：{}", self.p.yellow("!"), e.message));
                    self.selftest_seen.insert(id);
                }
            }
        }
        let Some(asks) = self.work.as_ref().map(|w| w.host.selftest.asks.clone()) else { return };
        // 服务端已经清掉的请求不用再记
        self.selftest_seen.retain(|id| asks.iter().any(|a| a.id == *id));
        for ask in asks {
            if self.selftest.contains_key(&ask.id) || self.selftest_seen.contains(&ask.id) || self.selftest_out.iter().any(|(id, _)| *id == ask.id) {
                continue;
            }
            self.start_selftest(&ask);
        }
    }

    /// 在后台开始一次自检：agent 按派发同样的办法拼命令（启动环境、模型、思考强度），进一个临时目录问一句；
    /// 整台机器时再检查每块白板的目录能不能推送
    pub(super) fn start_selftest(&mut self, ask: &board_core::SelfTestAsk) {
        let what = ask.agent.as_deref().map(|a| format!(" {a}")).unwrap_or_default();
        self.say(format!("{} {} 请求自检{what}", self.p.cyan("◇"), ask.by));
        // 整台机器共用一个目录，单查按 agent 分开：同时做的几个互不覆盖日志、提示词
        let tmp = match crate::securetmp::selftest_dir(ask.agent.as_deref()).and_then(|dir| {
            if let Err(e) = crate::securetmp::reset_private_dir(&dir) {
                return Err(board_core::BoardError::usage(format!("无法重置自检目录 {}：{e}", dir.display())));
            }
            Ok(dir)
        }) {
            Ok(dir) => dir,
            Err(e) => {
                self.say(format!("{} 自检没法开始：{}", self.p.yellow("!"), e.message));
                self.selftest_seen.insert(ask.id);
                return;
            }
        };
        // 身份也重新问一次（#667）
        self.auth_ver.retain(|id, _| ask.agent.as_deref().is_some_and(|a| a != id));
        let used: Vec<HostAgent> = self.work.as_ref().map(|w| w.host.agents.clone()).unwrap_or_default();
        let mut agents = vec![];
        for spec in &self.agents {
            if ask.agent.as_deref().is_some_and(|a| a != spec.id) {
                continue;
            }
            let u = used.iter().find(|a| a.id == spec.id);
            // 整台机器时只查找得到的 agent；只查一个时找不到也写明
            if ask.agent.is_none() && !u.is_some_and(|u| u.found) {
                continue;
            }
            let log = tmp.join(format!("agent-{}.log", spec.id));
            let (cmd, stream) = match self.selftest_command(spec, u, &tmp) {
                Ok((c, st)) => (Ok(c), st),
                Err(e) => (Err(e), None),
            };
            agents.push(selftest::AgentJob { id: spec.id.clone(), cmd, stream, log });
        }
        let gits: Vec<selftest::GitJob> = if ask.agent.is_some() {
            vec![]
        } else {
            ws_pairs(&self.ws, &self.boards, &self.os).into_iter().map(|(b, d)| selftest::GitJob { board: b, dir: d }).collect()
        };
        // 整台机器时再问共用配置里的每个供应商（带 key 的完整内容用这台机器的令牌取）；取不到时每个都写原因
        let providers: Vec<selftest::ProviderJob> = if ask.agent.is_some() { vec![] } else { self.provider_jobs() };
        let pids = self.selftest_pids.clone();
        let h = std::thread::spawn(move || selftest::run(agents, providers, gits, tmp, pids));
        self.selftest.insert(ask.id, h);
    }

    /// 自检要问的供应商：服务端发来的索引里有供应商时取完整内容（带 key）。取不到就不查（服务端照旧按上一次的结果算）
    fn provider_jobs(&self) -> Vec<selftest::ProviderJob> {
        let has = self.work.as_ref().and_then(|w| w.providers.as_ref()).is_some_and(|p| !p.providers.is_empty());
        if !has {
            return vec![];
        }
        match self.rm().get::<crate::providersync::Lib>(&format!("/api/hosts/{}/providers", self.name), &[]) {
            Ok(lib) => {
                self.remember_secrets(lib.providers.iter().filter_map(|p| p.api_key.clone()));
                lib.providers
                    .into_iter()
                    .map(|p| selftest::ProviderJob { name: p.name, endpoints: p.endpoints, key: p.api_key.unwrap_or_default() })
                    .collect()
            }
            Err(e) => {
                self.say(format!("{} 自检取不到模型供应商配置：{}", self.p.yellow("!"), e.message));
                vec![]
            }
        }
    }

    /// 额度：返回有没有新的要报（有就马上上报）。
    /// - 跑卡时：board acp 一看到 agent 推的额度就写进这次派发的额度文件，这里读到变了就记下来
    /// - 人点「刷新」：取到的任务里某个 agent 的 `quota_ask` 比上次查到（或没查到）的都新、这里也还没处理过，就在后台跑
    ///   `board acp --usage <本地命令>`（Claude Code 的 `/usage`、Codex 的 `/status`，不调模型、不花额度），查完记下来
    pub(super) fn poll_quota(&mut self) -> bool {
        let mut changed = false;
        let push = |out: &mut Vec<board_core::hosts::QuotaReport>, r: board_core::hosts::QuotaReport| {
            // 同一个 agent 只留最新的一条
            out.retain(|x| x.agent != r.agent);
            out.push(r);
        };
        // 额度文件每 2 秒读一次
        let read_files = self.quota_read.elapsed() >= Duration::from_secs(2);
        if read_files {
            self.quota_read = Instant::now();
        }
        for r in self.runs.iter().filter(|_| read_files) {
            let Some(q) = r.quota() else { continue };
            if self.quota_seen.get(&r.d.id) != Some(&q) {
                self.quota_seen.insert(r.d.id, q.clone());
                push(&mut self.quota_out, board_core::hosts::QuotaReport { agent: r.d.agent.clone(), quota: Some(q), error: None });
                changed = true;
            }
        }
        let alive: HashSet<i64> = self.runs.iter().map(|r| r.d.id).collect();
        self.quota_seen.retain(|id, _| alive.contains(id));
        // 查完的
        let done: Vec<String> = self.quota_jobs.iter().filter(|(_, h)| h.is_finished()).map(|(id, _)| id.clone()).collect();
        for id in done {
            let Some(h) = self.quota_jobs.remove(&id) else { continue };
            let got = h.join().unwrap_or_else(|_| Err("查额度的线程崩了".into()));
            match &got {
                Ok(_) => self.say(format!("{} {} 的额度查到了", self.p.dim("·"), id)),
                Err(e) => self.say(format!("{} {} 的额度没查到：{e}", self.p.yellow("!"), id)),
            }
            let (quota, error) = match got {
                Ok(q) => (Some(q), None),
                Err(e) => (None, Some(e)),
            };
            push(&mut self.quota_out, board_core::hosts::QuotaReport { agent: id, quota, error });
            changed = true;
        }
        // 新的「刷新」请求：10 分钟以前点的（这台机器那时不在线）不再查
        let asks: Vec<(String, i64)> = self
            .work
            .as_ref()
            .map(|w| {
                w.host
                    .agents
                    .iter()
                    .filter_map(|a| {
                        let ask = a.quota_ask?;
                        let seen = a.quota.as_ref().map_or(0, |q| q.at).max(a.quota_fail.as_ref().map_or(0, |f| f.at));
                        (ask > seen && w.now - ask < 10 * 60 * 1000).then(|| (a.id.clone(), ask))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (id, ask) in asks {
            if self.quota_jobs.contains_key(&id) || self.quota_done.get(&id).is_some_and(|t| *t >= ask) {
                continue;
            }
            self.quota_done.insert(id.clone(), ask);
            let Some(cmd) = board_core::hosts::QUOTA_PROBE_AGENTS.iter().find(|(a, _)| *a == id).map(|(_, c)| c.to_string()) else { continue };
            match self.usage_command(&id, &cmd) {
                Ok((c, log)) => {
                    let pids = self.selftest_pids.clone();
                    let limit = crate::acp::USAGE_LIMIT + Duration::from_secs(30);
                    self.quota_jobs.insert(id, std::thread::spawn(move || selftest::probe_usage(c, &log, limit, &pids)));
                }
                Err(e) => {
                    push(&mut self.quota_out, board_core::hosts::QuotaReport { agent: id, quota: None, error: Some(e) });
                    changed = true;
                }
            }
        }
        changed
    }

    /// 查额度的命令：和派发一样的启动环境，工作目录是临时目录；日志写到临时目录里
    fn usage_command(&self, id: &str, cmd: &str) -> std::result::Result<(Command, PathBuf), String> {
        let spec = self.agents.iter().find(|a| a.id == id && a.builtin.is_some()).ok_or_else(|| format!("这台机器上没有 {id}"))?;
        let used = self.work.as_ref().and_then(|w| w.host.agents.iter().find(|a| a.id == id).cloned());
        let check = used
            .as_ref()
            .and_then(|u| u.env.as_deref())
            .and_then(|e| spec.checks.iter().find(|c| c.env == e))
            .or_else(|| spec.default_check())
            .ok_or_else(|| format!("本机没找到 {} 的命令", spec.name))?;
        if check.prog.is_none() || check.missing.is_some() {
            return Err(format!("本机没找到 {} 的命令", spec.name));
        }
        let env = self.envs.iter().find(|e| e.id == check.env).cloned().ok_or_else(|| format!("本机没有启动环境「{}」", check.env))?;
        let tmp = crate::securetmp::usage_dir(id).map_err(|e| e.message)?;
        let c = self.acp_command(spec, check, &env, &tmp, Path::new(""), 0, None, &["--usage".to_string(), cmd.to_string()])?;
        Ok((c, tmp.join(format!("{id}.log"))))
    }

    /// 身份（#667）和本机账号能用的模型（#666）：收下问完的；会报身份的 agent（[`board_core::agentdef::SUBSCRIPTION_AGENTS`]）找到了命令、版本和上次问的
    /// 时候不一样（启动时、更新了、自检清掉了）就在后台再问一次（`board acp --auth probe`：身份和账号能用的模型，不发提示词、不花 token）。
    /// 返回身份有没有变（变了马上上报）
    pub(super) fn poll_auth(&mut self) -> bool {
        let mut changed = false;
        let done: Vec<String> = self.auth_jobs.iter().filter(|(_, h)| h.is_finished()).map(|(id, _)| id.clone()).collect();
        for id in done {
            let Some(h) = self.auth_jobs.remove(&id) else { continue };
            if let Ok((a, m)) = h.join() {
                if self.auth.get(&id) != Some(&a) {
                    self.say(format!("{} {} 用的身份：{}", self.p.dim("·"), id, a.words()));
                    self.auth.insert(id.clone(), a);
                    changed = true;
                }
                // 模型表（#666）：取的时间每次都不一样，比的是内容
                let same = self.local_models.get(&id).is_some_and(|old| board_core::hosts::LocalModels { at: 0, ..old.clone() } == board_core::hosts::LocalModels { at: 0, ..m.clone() });
                if !same {
                    if m.known() {
                        self.say(format!("{} {} 的账号能用 {} 个模型", self.p.dim("·"), id, m.models.len()));
                    }
                    self.local_models.insert(id, m);
                    changed = true;
                }
            }
        }
        let used: Vec<HostAgent> = self.work.as_ref().map(|w| w.host.agents.clone()).unwrap_or_default();
        let mut start = vec![];
        for spec in self.agents.iter().filter(|a| a.builtin.is_some() && board_core::agentdef::SUBSCRIPTION_AGENTS.contains(&a.id.as_str())) {
            let u = used.iter().find(|a| a.id == spec.id);
            let Some(check) = u.and_then(|u| u.env.as_deref()).and_then(|e| spec.checks.iter().find(|c| c.env == e)).or_else(|| spec.default_check()) else {
                continue;
            };
            if check.prog.is_none() || check.missing.is_some() || self.auth_jobs.contains_key(&spec.id) || self.auth_ver.get(&spec.id) == Some(&check.version) {
                continue;
            }
            let Some(env) = self.envs.iter().find(|e| e.id == check.env).cloned() else { continue };
            let tmp = match crate::securetmp::auth_dir(&spec.id) {
                Ok(dir) => dir,
                Err(e) => {
                    self.say(format!("{} 问不了 {} 的身份：{}", self.p.yellow("!"), spec.id, e.message));
                    self.auth_ver.insert(spec.id.clone(), check.version.clone());
                    continue;
                }
            };
            match self.acp_command(spec, check, &env, &tmp, Path::new(""), 0, None, &["--auth".to_string(), "probe".to_string()]) {
                Ok(c) => start.push((spec.id.clone(), check.version.clone(), c, tmp.join(format!("{}.log", spec.id)))),
                Err(_) => {
                    self.auth_ver.insert(spec.id.clone(), check.version.clone());
                }
            }
        }
        for (id, ver, cmd, log) in start {
            self.auth_ver.insert(id.clone(), ver);
            let pids = self.selftest_pids.clone();
            let limit = crate::acp::AUTH_WAIT + crate::acp::PROBE_LIMIT + Duration::from_secs(180);
            self.auth_jobs.insert(id, std::thread::spawn(move || selftest::probe_agent(cmd, &log, limit, &pids)));
        }
        changed
    }

    /// 自检时启动一个 agent 的命令：和派发一样的启动环境、模型、思考强度，工作目录是临时目录
    pub(super) fn selftest_command(&self, spec: &Spec, used: Option<&HostAgent>, tmp: &Path) -> std::result::Result<(Command, Option<Stream>), String> {
        let cmd0 = spec.bin.clone();
        let check = used
            .and_then(|u| u.env.as_deref())
            .and_then(|e| spec.checks.iter().find(|c| c.env == e))
            .or_else(|| spec.default_check())
            .ok_or_else(|| format!("本机没找到 {} 的命令 {cmd0}", spec.name))?;
        let env = self.envs.iter().find(|e| e.id == check.env).cloned().ok_or_else(|| format!("本机没有启动环境「{}」", check.env))?;
        check.prog.as_ref().ok_or_else(|| format!("在{}里没找到 {} 的命令 {cmd0}", env.name, spec.name))?;
        if let Some(m) = &check.missing {
            return Err(format!("在{}里缺 {m}：{} 跑不起来，先装上", env.name, spec.name));
        }
        let prompt_file = tmp.join(format!("prompt-{}.txt", spec.id));
        std::fs::write(&prompt_file, selftest::PROMPT).map_err(|e| format!("无法写 {}：{e}", prompt_file.display()))?;
        // 自检不领卡：不续轮
        let c = self.acp_command(spec, check, &env, tmp, &prompt_file, 0, None, &[])?;
        Ok((c, Some(Stream::Acp)))
    }
}
