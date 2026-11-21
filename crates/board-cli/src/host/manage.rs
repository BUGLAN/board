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
        // 每个请求一个目录（整台机器是 all，单查是 agent 标识）：同时做的几个互不覆盖日志、提示词
        let tmp = std::env::temp_dir().join(format!("board-selftest-{}", self.name)).join(ask.agent.as_deref().unwrap_or("all"));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
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
        let pids = self.selftest_pids.clone();
        let h = std::thread::spawn(move || selftest::run(agents, gits, tmp, pids));
        self.selftest.insert(ask.id, h);
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
        let c = self.acp_command(spec, check, &env, tmp, &prompt_file, 0, None)?;
        Ok((c, Some(Stream::Acp)))
    }
}
