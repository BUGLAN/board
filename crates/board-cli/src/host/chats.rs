//! 机器对话（board-core `hosts/chat.rs`，PRD §9.1「机器对话」）：一个对话一个 `board acp --chat` 进程。
//!
//! - 启动：和派发一样在这个 agent 的启动环境里跑（按角色带模型、强度、供应商），工作目录是这个启动环境里的用户主目录
//!   （WSL 是 WSL 里的主目录）。不生成名字、不 check、不领卡、不发心跳，不注入 `BOARD_AS`、`BOARD_HOST`（连同
//!   `BOARD_URL`、`BOARD_TOKEN` 这些连接一起去掉：对话里的 agent 不用 board）。权限请求和派发一样自动允许（board acp）。
//!   记过 ACP 会话的带 `--resume` 接回。
//! - 消息：任务里有还没交的（`ChatTask.prompts`）就写进它的标准输入（`{"seq", "prompt"}`），回报收下到第几条；
//!   停止这一轮（`ChatTask.cancel`）写 `{"cancel": true}`。人写的消息原样交给它，前后不加说明。
//! - 状态：board acp 把会话标识、这一轮在不在跑、结束原因写进状态文件（`run/chat-<ID>.state`），这里每一轮看一次：
//!   会话标识变了回报、这一轮结束了回报 `idle`。
//! - 输出、事件：和派发一样写本机日志（`logs/chat-<ID>.log`，每次启动接着写），转写、解析、脱敏后每 2 秒回报一次。
//! - 兜底设置（单次最长运行、没输出多久算卡住）按每一轮算：到了只停这一轮（`session/cancel`），对话不结束。
//! - 不占会话位：不算进「最多几个会话」，暂停接单、接单时段都不管它。
//! - agent 进程空闲 10 分钟自己退出（board acp `--idle-exit`），退出时回报退出码；下一条消息来时重新启动、接回会话。
//!   board host 退出时一起结束。

use super::*;
use board_core::hosts::{ChatReport, ChatTask};
use std::process::ChildStdin;

/// 一个在跑的对话进程
pub(super) struct ChatRun {
    pub(super) id: i64,
    pub(super) child: Child,
    pub(super) env: Env,
    pub(super) stdin: Option<ChildStdin>,
    pub(super) state: PathBuf,
    pub(super) log: PathBuf,
    pub(super) procfile: PathBuf,
    pub(super) pidfile: Option<PathBuf>,
    /// 日志已经上传、解析到的位置（字节）
    pub(super) sent: u64,
    pub(super) ev_read: u64,
    pub(super) parser: events::Parser,
    pub(super) fmt_due: bool,
    /// 交给它的消息到第几条
    pub(super) took: i64,
    /// 回报过收下到第几条
    pub(super) took_sent: i64,
    /// 处理过的停止请求、回报过的
    pub(super) canceled: i64,
    pub(super) canceled_sent: i64,
    /// 回报过「这一轮结束」的消息序号
    pub(super) idle_sent: i64,
    /// 回报过的 ACP 会话
    pub(super) acp_sent: Option<String>,
    /// 这一轮从什么时候开始、最近一次有输出（日志变长）、上一次看到的日志长度（兜底按每一轮算）
    pub(super) round_at: Option<Instant>,
    pub(super) output_at: Instant,
    pub(super) seen_len: u64,
    /// 这一轮已经按兜底设置停了：原因（回报成一条出错）
    pub(super) limited: Option<String>,
    pub(super) term_at: Option<Instant>,
}

impl ChatRun {
    fn write(&mut self, line: &str) -> bool {
        let Some(w) = self.stdin.as_mut() else { return false };
        if writeln!(w, "{line}").and_then(|_| w.flush()).is_err() {
            self.stdin = None;
            return false;
        }
        true
    }

    fn stop(&self, hard: bool) {
        if self.env.wsl().is_some() {
            if let Some(pid) = self.pidfile.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.trim().parse::<u32>().ok()) {
                envs::wsl_kill(&self.env, pid, hard);
            }
            kill_tree(self.child.id(), true);
        } else {
            kill_tree(self.child.id(), hard);
        }
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.procfile);
        let _ = std::fs::remove_file(&self.state);
        if let Some(p) = &self.pidfile {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// 交给 board acp 的一行：发一条消息
pub(super) fn prompt_line(seq: i64, text: &str) -> String {
    json!({ "seq": seq, "prompt": text }).to_string()
}

/// 这一轮结束了、还没回报：board acp 的状态里最近一轮是已经交给它的最后一条、不在跑、有结束原因
pub(super) fn round_done(st: &crate::acp::ChatState, took: i64, idle_sent: i64) -> Option<String> {
    (!st.busy && st.seq > idle_sent && st.seq >= took).then(|| st.stop.clone()).flatten()
}

/// 对话里的 agent 不用 board：派发时注入的这些变量都不给它（board host 自己的环境里可能有）
const CHAT_DROP_ENV: [&str; 9] = ["BOARD_AS", "BOARD_HOST", "BOARD", "BOARD_CONFIG", "BOARD_URL", "BOARD_TOKEN", "BOARD_HOST_URL", "BOARD_HOST_TOKEN", "BOARD_AGENT_ENV"];

impl Host<'_> {
    /// 本机有进程的对话（上报里带上，服务端据此知道哪一轮没人在跑了）
    pub(super) fn chat_ids(&self) -> Vec<i64> {
        self.chats.iter().map(|c| c.id).collect()
    }

    /// 每一轮：按任务启动、发消息、停止这一轮；看 board acp 的状态回报；兜底；收尾退出了的
    pub(super) fn chat_tick(&mut self) {
        let tasks: Vec<ChatTask> = self.work.as_ref().map(|w| w.chats.clone()).unwrap_or_default();
        for t in &tasks {
            let i = match self.chats.iter().position(|c| c.id == t.id) {
                Some(i) => Some(i),
                // 有还没交的消息才启动（只有停止请求、或者这一轮在跑而本机没进程的，由服务端按上报处理）
                None if t.prompts.iter().any(|p| p.seq > 0) => match self.chat_spawn(t) {
                    Ok(c) => {
                        self.say(format!("{} 对话 #{}：启动 {}", self.p.green("▶"), t.id, t.profile_name.clone().if_empty(&t.agent)));
                        self.chats.push(c);
                        Some(self.chats.len() - 1)
                    }
                    Err(why) => {
                        self.say(format!("{} 对话 #{} 起不来：{why}", self.p.red("✗"), t.id));
                        let took = t.prompts.iter().map(|p| p.seq).max();
                        self.chat_post(t.id, ChatReport { took, error: Some(format!("在这台机器上启动 agent 失败：{why}")), ..Default::default() });
                        None
                    }
                },
                None => None,
            };
            let Some(i) = i else { continue };
            let c = &mut self.chats[i];
            let from = c.took;
            for p in t.prompts.iter().filter(|p| p.seq > from) {
                if !c.write(&prompt_line(p.seq, &p.text)) {
                    break;
                }
                c.took = p.seq;
                c.round_at = Some(Instant::now());
                c.output_at = Instant::now();
                c.limited = None;
            }
            if t.cancel > c.canceled {
                c.write(r#"{"cancel":true}"#);
                c.canceled = t.cancel;
            }
        }
        // 状态、兜底。回报成功了才记下报过的（连不上时下一轮再报）
        let (max_run, idle_run) = self.limits();
        let min = minute();
        let mut posts: Vec<(i64, ChatReport, i64)> = vec![];
        for c in self.chats.iter_mut() {
            let mut rep = ChatReport::default();
            let mut idle_seq = c.idle_sent;
            if c.took > c.took_sent {
                rep.took = Some(c.took);
            }
            if let Some(st) = crate::acp::ChatState::load(&c.state) {
                if st.session.is_some() && st.session != c.acp_sent {
                    rep.acp = st.session.clone();
                }
                if let Some(stop) = round_done(&st, c.took, c.idle_sent) {
                    rep.idle = Some(stop);
                    idle_seq = st.seq;
                    c.round_at = None;
                }
            }
            if c.canceled > c.canceled_sent {
                rep.canceled = Some(c.canceled);
            }
            // 兜底：按这一轮算，到了只停这一轮
            let len = std::fs::metadata(&c.log).map(|m| m.len()).unwrap_or(0);
            if len > c.seen_len {
                c.seen_len = len;
                c.output_at = Instant::now();
            }
            if let (Some(at), None) = (c.round_at, &c.limited) {
                let run = max_run.filter(|n| at.elapsed() >= min * *n as u32).map(|n| format!("这一轮运行超过 {}", span_words(Duration::from_secs(n as u64 * 60))));
                let idle = idle_run.filter(|n| c.output_at.elapsed() >= min * *n as u32).map(|n| format!("这一轮{}没有任何输出", span_words(Duration::from_secs(n as u64 * 60))));
                if let Some(why) = run.or(idle) {
                    c.write(r#"{"cancel":true}"#);
                    rep.error = Some(format!("{why}，board host 停止了这一轮"));
                    c.limited = Some(why);
                }
            }
            if rep.took.is_some() || rep.acp.is_some() || rep.idle.is_some() || rep.error.is_some() || rep.canceled.is_some() {
                posts.push((c.id, rep, idle_seq));
            }
        }
        for (id, rep, idle_seq) in posts {
            let ok = self.chat_post(id, rep.clone());
            if let (true, Some(c)) = (ok, self.chats.iter_mut().find(|c| c.id == id)) {
                c.took_sent = rep.took.unwrap_or(c.took_sent);
                if rep.acp.is_some() {
                    c.acp_sent = rep.acp.clone();
                }
                c.idle_sent = idle_seq;
                c.canceled_sent = rep.canceled.unwrap_or(c.canceled_sent);
            }
        }
        self.chat_reap();
    }

    /// 回报一次。成功返回 true；连不上时丢掉这次的（下一次状态没变就不再报：收下、结束这类以服务端为准，下一次任务里还会带着）
    fn chat_post(&self, id: i64, rep: ChatReport) -> bool {
        let body = serde_json::to_value(&rep).unwrap_or(Value::Null);
        self.rm().post::<Value>(&format!("/api/hosts/{}/chats/{id}", self.name), body).is_ok()
    }

    /// 每 2 秒：新增的输出（转写、脱敏）和解析出的事件传上去
    pub(super) fn chat_upload(&mut self) {
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let mut posts: Vec<(usize, u64, u64, ChatReport)> = vec![];
        for (i, c) in self.chats.iter_mut().enumerate() {
            let (rep, sent, read) = chat_output(c, false, &secrets);
            if rep.lines.is_empty() && rep.events.is_empty() && rep.fmt.is_none() {
                c.sent = sent;
                c.ev_read = read;
                continue;
            }
            posts.push((i, sent, read, rep));
        }
        for (i, sent, read, rep) in posts {
            let id = self.chats[i].id;
            if self.chat_post(id, rep) {
                let c = &mut self.chats[i];
                c.sent = sent;
                c.ev_read = read;
                c.fmt_due = false;
            }
        }
    }

    /// 退出了的对话进程：传完剩下的输出，回报退出码，删临时文件
    fn chat_reap(&mut self) {
        let mut i = 0;
        while i < self.chats.len() {
            let Ok(Some(status)) = self.chats[i].child.try_wait() else {
                i += 1;
                continue;
            };
            let c = self.chats.remove(i);
            self.chat_finish(c, exit_code(status));
        }
    }

    fn chat_finish(&mut self, mut c: ChatRun, code: Option<i64>) {
        if cfg!(unix) && c.env.wsl().is_none() {
            kill_tree(c.child.id(), true);
        }
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let (mut rep, _, _) = chat_output(&mut c, true, &secrets);
        // 退出前最后一轮的结束
        if let Some(st) = crate::acp::ChatState::load(&c.state) {
            if let Some(stop) = round_done(&st, c.took, c.idle_sent) {
                rep.idle = Some(stop);
            }
            if st.session.is_some() && st.session != c.acp_sent {
                rep.acp = st.session;
            }
        }
        rep.exit = code.or(Some(-1));
        if c.took > c.took_sent {
            rep.took = Some(c.took);
        }
        self.chat_post(c.id, rep);
        let _ = truncate_log(&c.log, LOG_HEAD, LOG_TAIL, LOG_MAX);
        c.cleanup();
        self.say(format!("{} 对话 #{}：agent 退出（退出码 {}）", self.p.dim("■"), c.id, code.map(|x| x.to_string()).unwrap_or_else(|| "未知".into())));
    }

    /// board host 退出：结束所有对话进程（先关标准输入让 board acp 收尾，10 秒后还在就强制结束）
    pub(super) fn chat_shutdown(&mut self) {
        for c in self.chats.iter_mut() {
            c.stdin = None;
            c.stop(false);
            c.term_at = Some(Instant::now());
        }
        let begun = Instant::now();
        while self.chats.iter_mut().any(|c| matches!(c.child.try_wait(), Ok(None))) && begun.elapsed() < KILL_AFTER {
            std::thread::sleep(Duration::from_millis(100));
        }
        let chats = std::mem::take(&mut self.chats);
        for mut c in chats {
            if matches!(c.child.try_wait(), Ok(None)) {
                c.stop(true);
            }
            let code = c.child.wait().ok().and_then(exit_code);
            self.chat_finish(c, code);
        }
    }

    /// 启动一个对话的 `board acp --chat`
    fn chat_spawn(&self, t: &ChatTask) -> std::result::Result<ChatRun, String> {
        let spec = self.agents.iter().find(|a| a.id == t.agent).ok_or_else(|| format!("本机没有 agent「{}」", t.agent))?;
        let check = t
            .env
            .as_deref()
            .and_then(|e| spec.checks.iter().find(|c| c.env == e))
            .or_else(|| spec.default_check())
            .ok_or_else(|| format!("本机没找到 agent「{}」的命令 {}", t.agent, spec.bin))?;
        let env = self.envs.iter().find(|e| e.id == check.env).cloned().ok_or_else(|| format!("本机没有启动环境「{}」", check.env))?;
        if let Some(m) = &check.missing {
            return Err(format!("在{}里缺 {m}：agent「{}」跑不起来，先装上", env.name, t.agent));
        }
        let home = chat_home(&env).ok_or_else(|| format!("找不到{}里的用户主目录", env.name))?;
        // 按角色带上模型、强度、供应商：和派发同一套（role_run 只看这几项和 id，id 用负数，文件名不和派发撞）
        let role = self.role_run(&chat_dispatch(t))?;
        let _ = std::fs::create_dir_all(&self.run);
        let state = self.run.join(format!("chat-{}.state", t.id));
        let _ = std::fs::remove_file(&state);
        let mut more = vec!["--chat".to_string(), state.display().to_string()];
        if let Some(s) = t.acp.as_deref().filter(|s| !s.is_empty()) {
            more.push("--resume".into());
            more.push(s.to_string());
        }
        let mut c = self.acp_command(spec, check, &env, &home, Path::new(""), 0, role.as_ref(), &more).inspect_err(|_| {
            if let Some(f) = role.as_ref().and_then(|r| r.provider_file.as_ref()) {
                let _ = std::fs::remove_file(f);
            }
        })?;
        // 日志接着写（每次启动不清空）：之前的输出已经传过了，从现在的长度往后读
        let log = self.logs.join(format!("chat-{}.log", t.id));
        let (out, err) = append_log(&log).map_err(|e| format!("无法打开日志 {}：{e}", log.display()))?;
        let start_len = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
        for k in CHAT_DROP_ENV {
            c.env_remove(k);
        }
        c.env("BOARD_EXE", &self.exe).stdin(Stdio::piped()).stdout(out).stderr(err);
        let mut pidfile = None;
        if env.wsl().is_some() {
            let pf = self.run.join(format!("chat-{}.pid", t.id));
            let _ = std::fs::remove_file(&pf);
            c.env("BOARD_PIDFILE", &pf);
            pidfile = Some(pf);
        }
        envs::detach(&mut c);
        let mut child = c.spawn().map_err(|e| format!("在{}里启动 board acp 失败：{e}", env.name))?;
        let stdin = child.stdin.take();
        let procfile = self.run.join(format!("chat-{}.proc", t.id));
        let pstart = Procs::new().and_then(|mut ps| ps.get(child.id())).map(|x| x.start).unwrap_or(0);
        let _ = std::fs::write(&procfile, format!("{} {pstart} {} 对话#{}\n", child.id(), env.id, t.id));
        let now = Instant::now();
        Ok(ChatRun {
            id: t.id,
            child,
            env,
            stdin,
            state,
            log,
            procfile,
            pidfile,
            sent: start_len,
            ev_read: start_len,
            parser: events::Parser::new(Stream::Acp),
            fmt_due: true,
            took: 0,
            took_sent: 0,
            canceled: 0,
            canceled_sent: 0,
            idle_sent: 0,
            acp_sent: t.acp.clone(),
            round_at: None,
            output_at: now,
            seen_len: start_len,
            limited: None,
            term_at: None,
        })
    }
}

/// 读日志里新增的：转写后的行、解析出的事件（都脱敏）。返回回报和新的位置（上传、解析）
fn chat_output(c: &mut ChatRun, last: bool, secrets: &[&str]) -> (ChatReport, u64, u64) {
    let (lines, sent) = new_lines(&c.log, c.sent, last);
    let lines = upload_lines_redacted(lines, Some(Stream::Acp), secrets);
    let (raw, read) = new_lines(&c.log, c.ev_read, last);
    let at = now_ms();
    let mut events = vec![];
    for line in raw.into_iter().filter(|l| !shell_noise(l)) {
        for mut e in c.parser.feed(&line, at) {
            redact_event(&mut e, secrets);
            events.push(e);
        }
    }
    let events = board_core::runlog::merge_calls(events);
    let fmt = c.fmt_due.then(|| Stream::Acp.key().to_string());
    (ChatReport { lines, events, fmt, ..Default::default() }, sent, read)
}

/// 对话在哪个目录里跑：这个启动环境里的用户主目录（WSL 是 WSL 里的，换成 Windows 上能访问的路径）
fn chat_home(env: &Env) -> Option<PathBuf> {
    if env.wsl().is_some() {
        return envs::wsl_unc(env, "~");
    }
    let h = expand("~");
    h.is_dir().then_some(h)
}

/// 打开（没有就建）对话的本机日志：接着写，权限 0600
fn append_log(path: &Path) -> std::io::Result<(std::fs::File, std::fs::File)> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    restrict_log(path);
    let err = f.try_clone()?;
    Ok((f, err))
}

/// `role_run` 用的派发：只有角色的几项（模型、强度、供应商），id 用负数（文件名不和派发撞）
fn chat_dispatch(t: &ChatTask) -> Dispatch {
    Dispatch {
        id: -t.id,
        board: String::new(),
        card: 0,
        title: String::new(),
        host_id: 0,
        host: String::new(),
        agent: t.agent.clone(),
        agent_label: String::new(),
        name: None,
        env: t.env.clone(),
        env_name: None,
        source: String::new(),
        by: Actor::system(),
        state: "running".into(),
        result: None,
        created_at: 0,
        started_at: None,
        ended_at: None,
        exit_code: None,
        stop_by: None,
        beat_at: None,
        session: 0,
        seq: 1,
        total: 1,
        model: t.model.clone(),
        effort: t.effort.clone(),
        kind: String::new(),
        path: None,
        any: false,
        repo: String::new(),
        branch: String::new(),
        profile: t.profile.clone(),
        profile_name: t.profile_name.clone(),
        provider: t.provider.clone(),
        note: String::new(),
    }
}

trait IfEmpty {
    fn if_empty(self, or: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, or: &str) -> String {
        if self.is_empty() { or.to_string() } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::ChatState;

    #[test]
    fn messages_go_to_board_acp_as_is() {
        let v: Value = serde_json::from_str(&prompt_line(3, "看看主目录\n下哪些最大")).unwrap();
        assert_eq!((v["seq"].as_i64(), v["prompt"].as_str()), (Some(3), Some("看看主目录\n下哪些最大")));
        assert_eq!(crate::acp::chat_cmd(&prompt_line(3, "x")), Some(crate::acp::ChatCmd::Prompt(3, "x".into())));
    }

    #[test]
    fn a_round_is_done_once_its_last_message_finished() {
        let st = |busy: bool, seq: i64, stop: Option<&str>| ChatState { session: Some("s".into()), ready: true, busy, seq, stop: stop.map(str::to_string) };
        assert_eq!(round_done(&st(false, 2, Some("end_turn")), 2, 1), Some("end_turn".into()));
        assert_eq!(round_done(&st(true, 2, None), 2, 1), None, "还在跑");
        assert_eq!(round_done(&st(false, 2, Some("end_turn")), 2, 2), None, "报过了");
        assert_eq!(round_done(&st(false, 1, Some("end_turn")), 2, 0), None, "交了第 2 条、board acp 还没开始它");
        assert_eq!(round_done(&st(false, 2, Some("cancelled")), 2, 1), Some("cancelled".into()));
    }
}
