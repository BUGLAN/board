use super::client::{exit_for, local_words, Client, Outcome, Proto};
use super::merge::Merger;
use super::quota::usage_of_text;
use super::{
    parse, AuthMode, Opts, AUTH_WAIT, EXIT_CANCELLED, EXIT_OK, EXIT_PROTOCOL, EXIT_QUOTA, GRACE, HANDSHAKE, PROBE_LIMIT, RECHECK, SETTLE_MAX, SETTLE_QUIET, TICK, USAGE_LIMIT,
};
use crate::envs::Env;
use crate::proctree::Job;
use board_core::hosts::{AgentAuth, LocalModels, Quota, AUTH_UNKNOWN};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

static TERM: AtomicBool = AtomicBool::new(false);

/* ------------------------------ 进程 ------------------------------ */

/// `board acp …` 的入口，返回退出码
pub fn main(args: &[String]) -> i32 {
    let o = match parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("✗ board acp：{e}");
            return EXIT_PROTOCOL;
        }
    };
    on_signals();
    let code = match o.chat.clone() {
        Some(state) => run_chat(&o, &state),
        None if o.usage.is_some() => run_usage(&o),
        None if matches!(o.auth, AuthMode::Only | AuthMode::Probe) => run_auth(&o),
        None => run(&o),
    };
    let _ = std::io::stdout().flush();
    code
}


fn on_signals() {
    #[cfg(unix)]
    {
        extern "C" fn handle(_: libc::c_int) {
            TERM.store(true, Ordering::SeqCst);
        }
        let h = handle as extern "C" fn(libc::c_int) as libc::sighandler_t;
        unsafe {
            libc::signal(libc::SIGINT, h);
            libc::signal(libc::SIGTERM, h);
            libc::signal(libc::SIGHUP, h);
            // agent 先退出、再往它的标准输入写时不要被 SIGPIPE 结束：写失败当作 agent 已经不在
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        }
    }
}


/// 派发模式：board host 拿着标准输入的写端、从不写。读到 EOF 说明 host 不在了，和收到 SIGTERM 一样
/// 取消这一轮，不再继续耗 agent 的额度。
fn watch_parent() {
    std::thread::spawn(|| {
        let mut buf = [0u8; 16];
        let mut stdin = std::io::stdin().lock();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => {
                    TERM.store(true, Ordering::SeqCst);
                    return;
                }
                Ok(_) => {}
            }
        }
    });
}


/// 写一行日志（标准输出）
fn log(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}


/// agent 发来的帧写进日志的样子：身份推送（`_auth/status_update`、`auth/status_update`）和 `auth/status` 的回应里账号的邮箱、
/// 组织名去掉（#669：邮箱只存在机器数据里，不进派发日志），别的原样
fn without_account(v: &Value) -> std::borrow::Cow<'_, Value> {
    let auth = matches!(v["method"].as_str(), Some("_auth/status_update" | "auth/status_update")) || v["result"]["accounts"].is_array();
    if !auth {
        return std::borrow::Cow::Borrowed(v);
    }
    let mut c = v.clone();
    let strip = |a: &mut serde_json::Map<String, Value>| {
        a.remove("email");
        a.remove("organization");
    };
    if let Some(a) = c.pointer_mut("/params/authStatus/account").and_then(Value::as_object_mut) {
        strip(a);
    }
    for p in ["/params/accounts", "/result/accounts"] {
        for a in c.pointer_mut(p).and_then(Value::as_array_mut).into_iter().flatten().filter_map(Value::as_object_mut) {
            strip(a);
        }
    }
    std::borrow::Cow::Owned(c)
}


/// 发一帧给 agent，也写进日志
/// 写进日志的样子：`providers/set` 的请求头（里面是 API key）换成 `***`，别的原样
pub(super) fn loggable(f: &Value) -> String {
    if f["method"].as_str() != Some("providers/set") {
        return f.to_string();
    }
    let mut c = f.clone();
    if let Some(h) = c["params"]["headers"].as_object_mut() {
        for v in h.values_mut() {
            *v = json!("***");
        }
    }
    c.to_string()
}


fn send(stdin: &mut Option<ChildStdin>, frames: Vec<Value>) {
    for f in frames {
        let line = f.to_string();
        log(&loggable(&f));
        if let Some(w) = stdin.as_mut() {
            if writeln!(w, "{line}").and_then(|_| w.flush()).is_err() {
                *stdin = None;
            }
        }
    }
}


fn send_quiet(stdin: &mut Option<ChildStdin>, frames: Vec<Value>) {
    for f in frames {
        if let Some(w) = stdin.as_mut() {
            if writeln!(w, "{f}").and_then(|_| w.flush()).is_err() {
                *stdin = None;
            }
        }
    }
}


fn parse_frame(line: &str) -> Option<Value> {
    let t = line.trim();
    t.starts_with('{').then(|| serde_json::from_str::<Value>(t).ok()).flatten().filter(Value::is_object)
}


fn read_provider(o: &Opts) -> Result<Option<Value>, String> {
    let Some(f) = &o.provider_file else { return Ok(None) };
    let got = std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
    if let Err(e) = std::fs::remove_file(f).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
        return Err(format!("{} 删不掉：{e}", f.display()));
    }
    got.map(Some).map_err(|e| format!("{}：{e}", f.display()))
}


/// 在启动环境里拉起 agent 的 ACP 入口：标准输出一行一行送进返回的通道（读完送 None）。启动失败时写明原因、返回退出码
fn spawn_agent(o: &Opts) -> std::result::Result<(Child, Job, Option<ChildStdin>, mpsc::Receiver<Option<String>>), i32> {
    let env = Env::from_parts(&o.env, &o.env_prog);
    let mut c = env.command(&o.prog, &o.args, Some(&o.cwd), true);
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => {
            eprintln!("✗ board acp：在{}里启动 {} 失败：{e}", env.name, o.prog);
            return Err(EXIT_PROTOCOL);
        }
    };
    // agent 自己起的子进程也在这棵进程树里：board acp 退出、或等不到它退出要结束时，一起结束
    let job = match Job::attach(&child) {
        Ok(job) => job,
        Err(e) => {
            eprintln!("✗ board acp：无法把 {} 放进 Job Object（结束整棵进程树）：{e}", o.prog);
            let _ = child.kill();
            let _ = child.wait();
            return Err(EXIT_PROTOCOL);
        }
    };
    let stdin = child.stdin.take();
    let (tx, rx) = mpsc::channel::<Option<String>>();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut r = BufReader::new(out);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf).trim_end_matches(['\n', '\r']).to_string();
                        if tx.send(Some(line)).is_err() {
                            return;
                        }
                    }
                }
            }
            let _ = tx.send(None);
        });
    }
    Ok((child, job, stdin, rx))
}


/// 把额度写进 `--quota-file`：先写临时文件再换上，board host 在派发期间读它，读不到写了一半的
fn write_quota(o: &Opts, q: Option<&Quota>) {
    let (Some(f), Some(q)) = (&o.quota_file, q) else { return };
    let tmp = f.with_extension("quota-tmp");
    if let Err(e) = std::fs::write(&tmp, serde_json::to_string(q).unwrap_or_default()).and_then(|_| std::fs::rename(&tmp, f)) {
        eprintln!("✗ board acp：写不了额度文件 {}：{e}", f.display());
    }
}


/// `--usage <本地命令>`：握手、开会话、发这条本地命令（Claude Code 的 `/usage`、Codex 的 `/status`：agent 自己答，
/// 不调模型、不花额度），读出 5 小时、本周两个窗口，关掉会话。帧不写日志，标准输出只有一行 `{"usage": 额度}`；
/// 查不到时 `{"usage": null, "error": 原因}`，退出码都是 0
fn run_usage(o: &Opts) -> i32 {
    let cmd = o.usage.clone().unwrap_or_default();
    let say = |q: Option<&Quota>, why: Option<String>| log(&json!({ "usage": q, "error": why }).to_string());
    let (mut child, job, mut stdin, rx) = match spawn_agent(o) {
        Ok(x) => x,
        Err(code) => {
            say(None, Some(format!("启动 {} 失败", o.prog)));
            return code;
        }
    };
    let cwd = o.cwd.display().to_string();
    let mut client = Client::new(&cwd, "", None).with_usage(&cmd);
    send_quiet(&mut stdin, client.start());
    let started = Instant::now();
    let end = loop {
        if let Some(out) = client.outcome.clone() {
            break out;
        }
        if TERM.load(Ordering::SeqCst) {
            break Outcome::Failed("被停止".into());
        }
        if started.elapsed() >= USAGE_LIMIT {
            break Outcome::Failed(format!("{} 秒内没查完", USAGE_LIMIT.as_secs()));
        }
        match rx.recv_timeout(TICK) {
            Ok(Some(line)) => {
                if let Some(v) = parse_frame(&line) {
                    let reply = client.on_frame(&v);
                    send_quiet(&mut stdin, reply);
                }
            }
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break Outcome::Failed("agent 退出了".into()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    send_quiet(&mut stdin, client.close());
    drop(stdin);
    let _ = wait(&mut child, &job, GRACE);
    match end {
        Outcome::Stopped(_) => match usage_of_text(&client.usage_text, chrono::Local::now()) {
            Some(q) => say(Some(&q), None),
            None => {
                let first: String = client.usage_text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("没有输出").chars().take(120).collect();
                say(None, Some(format!("{cmd} 的输出里没有额度：{first}")));
            }
        },
        Outcome::Failed(why) | Outcome::Quota(why) => say(None, Some(why)),
    }
    0
}


/// `--auth only`（#667）：握手、问身份，不开会话；身份写成一行 JSON（[`AgentAuth`]）到标准输出，退出码 0。
/// `--auth probe`（#666）：问完身份再列模型，写一行 `{"auth": …, "models": …}`。
/// 帧不写日志（标准输出只有这一行）。agent 起不来、握手失败时也写一行，种类是 `unknown`、label 写原因
fn run_auth(o: &Opts) -> i32 {
    let probe = o.auth == AuthMode::Probe;
    let say = |a: &AgentAuth, m: Option<&LocalModels>| {
        let line = if probe { json!({ "auth": a, "models": m }).to_string() } else { serde_json::to_string(a).unwrap_or_default() };
        log(&line);
    };
    let unknown = |why: String| AgentAuth { kind: AUTH_UNKNOWN.into(), label: why, ..Default::default() };
    let (mut child, job, mut stdin, rx) = match spawn_agent(o) {
        Ok(x) => x,
        Err(code) => {
            say(&unknown(format!("启动 {} 失败", o.prog)), None);
            return code;
        }
    };
    let cwd = o.cwd.display().to_string();
    let mut client = Client::new(&cwd, "", None).with_auth(o.auth);
    send_quiet(&mut stdin, client.start());
    let started = Instant::now();
    let mut wait_from: Option<Instant> = None;
    let mut probe_from: Option<Instant> = None;
    let got = loop {
        if let Some(a) = client.auth.clone().filter(|_| !client.waiting_auth) {
            if !probe || client.probe_done {
                break a;
            }
            // 列模型做太久：有多少算多少
            if probe_from.get_or_insert_with(Instant::now).elapsed() >= PROBE_LIMIT {
                let m = client.models.get_or_insert_with(LocalModels::default);
                m.error = format!("{} 秒内没列完", PROBE_LIMIT.as_secs());
                break a;
            }
        }
        if let Some(Outcome::Failed(why)) = &client.outcome {
            break unknown(why.clone());
        }
        if TERM.load(Ordering::SeqCst) {
            break unknown("被停止".into());
        }
        if client.waiting_auth {
            if wait_from.get_or_insert_with(Instant::now).elapsed() >= AUTH_WAIT {
                let f = client.auth_timeout();
                send_quiet(&mut stdin, f);
                continue;
            }
        } else if started.elapsed() >= HANDSHAKE {
            break unknown(format!("agent {} 秒内没有完成握手", HANDSHAKE.as_secs()));
        }
        match rx.recv_timeout(TICK) {
            Ok(Some(line)) => {
                if let Some(v) = parse_frame(&line) {
                    let reply = client.on_frame(&v);
                    send_quiet(&mut stdin, reply);
                }
            }
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                break client.auth.clone().filter(|_| !client.waiting_auth).unwrap_or_else(|| unknown("agent 还没报身份就退出了".into()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    let models = probe.then(|| client.models.clone().unwrap_or_else(|| LocalModels { error: "没列出模型".into(), ..Default::default() }));
    say(&got, models.as_ref());
    // 列模型时开的会话关掉（v2 的 session/close），再关标准输入
    send_quiet(&mut stdin, client.close());
    drop(stdin);
    wait(&mut child, &job, GRACE);
    EXIT_OK
}


fn run(o: &Opts) -> i32 {
    // 角色的供应商：里面有 key，第一步就读完删掉；后面无论提示词、agent 启动还是握手失败都不留文件
    let provider = match read_provider(o) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ board acp：读不了供应商设置 {e}");
            return EXIT_PROTOCOL;
        }
    };
    watch_parent();
    let prompt = match std::fs::read_to_string(&o.prompt_file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ board acp：读不了提示词 {}：{e}", o.prompt_file.display());
            return EXIT_PROTOCOL;
        }
    };
    let (mut child, job, mut stdin, rx) = match spawn_agent(o) {
        Ok(x) => x,
        Err(code) => return code,
    };
    let cwd = o.cwd.display().to_string();
    let mut client = Client::new(&cwd, &prompt, o.mode.as_deref()).with_sets(o.sets.clone()).with_provider(provider).with_auth(o.auth);
    let mut merger = Merger::default();
    // 等 agent 推身份从什么时候开始（`--auth subscription`）
    let mut auth_from: Option<Instant> = None;
    send(&mut stdin, client.start());
    let started = Instant::now();
    // 已经续了几轮（见 decide）
    let mut turns: u32 = 0;
    // 这一轮做完以后走到了哪一步（见 Phase）
    let mut phase = Phase::Working;
    // 最近一次收到 agent 输出的时间：这一轮做完以后，安静多久算稳定下来
    let mut last_frame = Instant::now();
    let mut cancel_at: Option<Instant> = None;
    // agent 进程已经退出的时间：它留下的子进程可能还拿着标准输出，读不到 EOF；退出后再收一会儿剩下的输出就收尾
    let mut gone_at: Option<Instant> = None;
    // 已经写进额度文件的额度
    let mut written: Option<Quota> = None;
    loop {
        if gone_at.is_none() && child.try_wait().ok().flatten().is_some() {
            gone_at = Some(Instant::now());
        }
        if gone_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(1)) {
            break;
        }
        if TERM.load(Ordering::SeqCst) && cancel_at.is_none() {
            cancel_at = Some(Instant::now());
            let frames = client.cancel();
            if frames.is_empty() {
                // 还没开始干活：不用等 agent 回应，直接收尾
                break;
            }
            send(&mut stdin, frames);
        }
        if cancel_at.is_some_and(|at| at.elapsed() >= GRACE) {
            break;
        }
        if client.handshaking() && started.elapsed() >= HANDSHAKE {
            client.outcome = Some(Outcome::Failed(format!("agent {} 秒内没有完成握手（initialize、session/new）", HANDSHAKE.as_secs())));
            break;
        }
        if client.waiting_auth && auth_from.get_or_insert_with(Instant::now).elapsed() >= AUTH_WAIT {
            let f = client.auth_timeout();
            send(&mut stdin, f);
        }
        if merger.due() {
            for l in merger.flush_due() {
                log(&l);
            }
        }
        match rx.recv_timeout(TICK) {
            Ok(Some(line)) => {
                last_frame = Instant::now();
                let t = line.trim();
                match parse_frame(&line) {
                    Some(v) => {
                        for l in merger.push(&without_account(&v)) {
                            log(&l);
                        }
                        let reply = client.on_frame(&v);
                        send(&mut stdin, reply);
                        // 额度变了马上写进额度文件：board host 跑卡期间读它、随上报带上（差不多实时）
                        if client.quota != written {
                            written = client.quota.clone();
                            write_quota(o, written.as_ref());
                        }
                    }
                    // 不是协议帧（agent 不该往标准输出写别的）：原样记下
                    None if !t.is_empty() => {
                        for l in merger.flush() {
                            log(&l);
                        }
                        log(&line);
                    }
                    None => {}
                }
            }
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // 正在停：等 agent 回 `cancelled`（或者到 GRACE）
        if cancel_at.is_some() {
            if client.outcome.is_some() {
                break;
            }
            continue;
        }
        match next_step(o, &mut client, &mut phase, &mut turns, last_frame, stdin.is_some()) {
            Step::Stay => {}
            Step::Finish => break,
            Step::Send(frames) => send(&mut stdin, frames),
        }
    }
    for l in merger.flush() {
        log(&l);
    }
    // 收尾：v2 先关会话（停掉还在跑的，包括后台的），再关掉 agent 的标准输入（ACP 的 agent 读到 EOF 就退出），
    // 等一会儿，还不退就结束它
    send(&mut stdin, client.close());
    drop(stdin);
    let status = wait(&mut child, &job, GRACE);
    // 结束时再写一次（额度用完时记上了用完和重置时间）
    if client.quota != written {
        write_quota(o, client.quota.as_ref());
    }
    match &client.outcome {
        Some(Outcome::Failed(why)) => {
            eprintln!("✗ {why}");
            EXIT_PROTOCOL
        }
        Some(Outcome::Quota(why)) => {
            let when = client.quota.as_ref().and_then(|q| q.resets_at).map(|t| format!("，{} 恢复", local_words(t))).unwrap_or_default();
            eprintln!("✗ 额度用完{when}：{why}");
            EXIT_QUOTA
        }
        // 人点停止、board host 退出：不管 agent 停在哪一步
        _ if TERM.load(Ordering::SeqCst) => EXIT_CANCELLED,
        Some(o) => exit_for(o),
        None => {
            let code = status.and_then(|s| s.code()).unwrap_or(1);
            eprintln!("✗ agent 在这一轮结束之前就退出了（退出码 {code}）");
            if code == 0 { 1 } else { code }
        }
    }
}

/* ------------------------------ 机器对话（--chat） ------------------------------ */

/// 对话模式写给 board host 的状态（`--chat <状态文件>`，每次变了整份重写）
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ChatState {
    /// ACP 会话标识（开好、接回之后才有）
    #[serde(default)]
    pub session: Option<String>,
    /// 这一轮在跑
    #[serde(default)]
    pub busy: bool,
    /// 最近一轮是哪一条消息（board host 给的序号）
    #[serde(default)]
    pub seq: i64,
    /// 最近一轮的结束原因（`end_turn`、`cancelled` …，出错是 `error`）；这一轮还在跑时为空
    #[serde(default)]
    pub stop: Option<String>,
}

impl ChatState {
    /// 写进状态文件：先写临时文件再改名，board host 不会读到写了一半的
    fn save(&self, path: &std::path::Path) {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_string(self).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    pub(crate) fn load(path: &std::path::Path) -> Option<ChatState> {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok())
    }
}


/// board host 从标准输入发来的一行：`{"seq": 3, "prompt": "…"}` 发一条消息，`{"cancel": true}` 停止这一轮
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ChatCmd {
    Prompt(i64, String),
    Cancel,
}

pub(crate) fn chat_cmd(line: &str) -> Option<ChatCmd> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v["cancel"].as_bool() == Some(true) {
        return Some(ChatCmd::Cancel);
    }
    let text = v["prompt"].as_str()?;
    Some(ChatCmd::Prompt(v["seq"].as_i64().unwrap_or(0), text.to_string()))
}


/// 对话模式：握手、开会话（或接回）之后等 board host 发来消息，一条一条在同一个会话里 `session/prompt`；每一轮结束不退出。
/// 没有新消息超过 `--idle-exit` 秒、board host 关掉了标准输入、收到 SIGTERM 时收尾退出。
/// 状态（会话标识、这一轮在不在跑、结束原因）写进状态文件给 board host 看
fn run_chat(o: &Opts, state_file: &std::path::Path) -> i32 {
    // 角色的供应商：里面有 key，第一步就读完删掉
    let provider = match read_provider(o) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ board acp：读不了供应商设置 {e}");
            return EXIT_PROTOCOL;
        }
    };
    let (mut child, job, mut stdin, rx) = match spawn_agent(o) {
        Ok(x) => x,
        Err(code) => return code,
    };
    // board host 的指令：自己的标准输入，一行一条；读到 EOF 是 board host 不在了
    let (ctx, crx) = mpsc::channel::<Option<ChatCmd>>();
    std::thread::spawn(move || {
        let r = BufReader::new(std::io::stdin());
        for line in r.lines() {
            let Ok(line) = line else { break };
            if let Some(cmd) = chat_cmd(&line) {
                if ctx.send(Some(cmd)).is_err() {
                    return;
                }
            }
        }
        let _ = ctx.send(None);
    });
    let cwd = o.cwd.display().to_string();
    let mut client = Client::new(&cwd, "", o.mode.as_deref()).with_sets(o.sets.clone()).with_provider(provider).with_auth(o.auth).for_chat(o.resume.clone());
    let mut merger = Merger::default();
    let mut st = ChatState::default();
    st.save(state_file);
    let mut saved = st.clone();
    send(&mut stdin, client.start());
    let started = Instant::now();
    let mut queue: std::collections::VecDeque<(i64, String)> = Default::default();
    let mut last_activity = Instant::now();
    let mut cancel_at: Option<Instant> = None;
    let mut gone_at: Option<Instant> = None;
    let mut said_note = false;
    let mut failed: Option<String> = None;
    loop {
        if gone_at.is_none() && child.try_wait().ok().flatten().is_some() {
            gone_at = Some(Instant::now());
        }
        if gone_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(1)) {
            break;
        }
        if TERM.load(Ordering::SeqCst) && cancel_at.is_none() {
            cancel_at = Some(Instant::now());
            let frames = client.cancel();
            if frames.is_empty() {
                break;
            }
            send(&mut stdin, frames);
        }
        if cancel_at.is_some_and(|at| at.elapsed() >= GRACE || client.outcome.is_some()) {
            break;
        }
        if !client.ready && client.outcome.is_none() && started.elapsed() >= HANDSHAKE {
            failed = Some(format!("agent {} 秒内没有完成握手（initialize、开会话）", HANDSHAKE.as_secs()));
            break;
        }
        // 握手、开会话就失败了：说明原因，退出
        if !client.ready {
            if let Some(Outcome::Failed(why) | Outcome::Quota(why)) = &client.outcome {
                failed = Some(why.clone());
                break;
            }
        }
        if merger.due() {
            for l in merger.flush_due() {
                log(&l);
            }
        }
        // board host 的指令
        let mut host_gone = false;
        while let Ok(cmd) = crx.try_recv() {
            match cmd {
                None => host_gone = true,
                Some(ChatCmd::Cancel) => {
                    if client.busy() {
                        send(&mut stdin, client.cancel());
                    } else {
                        // 还没发出去的也撤回
                        queue.clear();
                    }
                }
                Some(ChatCmd::Prompt(seq, text)) => queue.push_back((seq, text)),
            }
        }
        if host_gone {
            break;
        }
        if client.ready && !said_note {
            said_note = true;
            if let Some(n) = &client.note {
                eprintln!("⋯ {n}");
            }
        }
        if client.ready && !client.busy() && cancel_at.is_none() {
            if let Some((seq, text)) = queue.pop_front() {
                send(&mut stdin, client.ask(&text));
                st.seq = seq;
                st.busy = true;
                st.stop = None;
                last_activity = Instant::now();
            }
        }
        match rx.recv_timeout(TICK) {
            Ok(Some(line)) => {
                last_activity = Instant::now();
                let t = line.trim();
                match parse_frame(&line) {
                    Some(v) => {
                        // session/load 回放的历史之前已经记过：不再记日志
                        let replay = client.replaying && v["method"].as_str() == Some("session/update");
                        if !replay {
                            for l in merger.push(&without_account(&v)) {
                                log(&l);
                            }
                        }
                        let reply = client.on_frame(&v);
                        send(&mut stdin, reply);
                    }
                    None if !t.is_empty() => {
                        for l in merger.flush() {
                            log(&l);
                        }
                        log(&line);
                    }
                    None => {}
                }
            }
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // 这一轮结束了：写结束原因（出错的写进日志）
        if st.busy && !client.busy() {
            if let Some(out) = &client.outcome {
                st.busy = false;
                st.stop = Some(match out {
                    Outcome::Stopped(r) => r.clone(),
                    Outcome::Failed(why) | Outcome::Quota(why) => {
                        eprintln!("✗ {why}");
                        "error".to_string()
                    }
                });
                last_activity = Instant::now();
            }
        }
        st.session = client.session().map(str::to_string);
        // v2：agent 自己又开了一轮（处理跑完的后台任务）
        if !st.busy && client.busy() {
            st.busy = true;
        }
        // 和上次写进文件的比：变了才写
        if st != saved {
            st.save(state_file);
            saved = st.clone();
        }
        if client.ready && !client.busy() && queue.is_empty() && last_activity.elapsed() >= Duration::from_secs(o.idle_exit) {
            break;
        }
    }
    for l in merger.flush() {
        log(&l);
    }
    send(&mut stdin, client.close());
    drop(stdin);
    wait(&mut child, &job, GRACE);
    st.busy = false;
    st.save(state_file);
    if let Some(why) = failed {
        eprintln!("✗ {why}");
        return EXIT_PROTOCOL;
    }
    if TERM.load(Ordering::SeqCst) {
        return EXIT_CANCELLED;
    }
    EXIT_OK
}


/// 这一轮做完以后走到了哪一步
#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// 等这一轮做完：v1 是 prompt 的回应，v2 是带 `stopReason` 的 `idle`
    Working,
    /// 做完了，等它稳定下来：最后一帧之后安静 SETTLE_QUIET，从这个时间算最多 SETTLE_MAX
    Settling(Instant),
    /// v2：卡片还在手里、后台还在跑：等 agent 跑完自己接着做；到这个时间再查一次卡片
    Waiting(Instant),
}


/// 这一轮的进展之后要做的
#[derive(Debug, PartialEq)]
enum Step {
    Stay,
    Finish,
    /// 发给 agent（续轮的 prompt）
    Send(Vec<Value>),
}


/// 每收一帧、每过一个 TICK 看一次这一轮走到哪了（见模块说明第 5 步）。`last_frame` 是最近一次收到 agent 输出的时间，
/// `alive` 是 agent 的标准输入还写得进去
fn next_step(o: &Opts, client: &mut Client, phase: &mut Phase, turns: &mut u32, last_frame: Instant, alive: bool) -> Step {
    match *phase {
        Phase::Working => {
            let Some(out) = &client.outcome else { return Step::Stay };
            // 出错、被取消、到了上限，或者不续轮的派发（拆解、自检）：收尾
            if *out != Outcome::Stopped("end_turn".into()) || o.extra_turns == 0 {
                return Step::Finish;
            }
            *phase = Phase::Settling(Instant::now());
            Step::Stay
        }
        Phase::Settling(since) => {
            // v2：agent 又报了 `running`，自己接着做了
            if client.outcome.is_none() {
                *phase = Phase::Working;
                return Step::Stay;
            }
            if last_frame.elapsed() < SETTLE_QUIET && since.elapsed() < SETTLE_MAX {
                return Step::Stay;
            }
            decide(o, client, phase, turns, alive, false)
        }
        Phase::Waiting(at) => {
            if client.outcome.is_none() {
                *phase = Phase::Working;
                return Step::Stay;
            }
            // 后台跑完了（agent 重报了一次 `idle`，后台为空或不再报）：稳定下来再看
            if client.background.unwrap_or(0) == 0 {
                *phase = Phase::Settling(Instant::now());
                return Step::Stay;
            }
            if Instant::now() < at {
                return Step::Stay;
            }
            decide(o, client, phase, turns, alive, true)
        }
    }
}


/// 这一轮做完、稳定下来以后：这次派发的名字（`BOARD_AS`）在这块白板（`BOARD`）上还持有卡片吗？不持有、查不到就收尾；
/// 还持有时，v2 后台还有东西在跑就接着等，否则在同一个会话里续一轮（续满了收尾）。每个决定在标准错误写一行
/// （进日志，「过程」里看得到）；`waiting` 是已经在等后台，再查一次时不重复写
fn decide(o: &Opts, client: &mut Client, phase: &mut Phase, turns: &mut u32, alive: bool, waiting: bool) -> Step {
    if !alive || TERM.load(Ordering::SeqCst) {
        return Step::Finish;
    }
    let var = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let (Some(name), Some(board)) = (var("BOARD_AS"), var("BOARD")) else {
        eprintln!("✗ 环境变量里没有 BOARD_AS、BOARD：不知道这次派发的名字和白板，不续轮");
        return Step::Finish;
    };
    let (id, title) = match held(&name, &board) {
        Ok(Some(card)) => card,
        Ok(None) => return Step::Finish,
        Err(e) => {
            eprintln!("✗ 查不到 {name} 还持有没持有卡片（{e}）：不续轮，收尾");
            return Step::Finish;
        }
    };
    let running = if client.proto == Proto::V2 { client.background.unwrap_or(0) } else { 0 };
    if running > 0 {
        if !waiting {
            eprintln!("⋯ 等后台：{board}#{id} 还由 {name} 持有，后台还有 {running} 项在跑，等 agent 跑完自己接着做");
        }
        *phase = Phase::Waiting(Instant::now() + RECHECK);
        return Step::Stay;
    }
    if *turns >= o.extra_turns {
        eprintln!("✗ 续了 {turns} 轮，{board}#{id} 还由 {name} 持有：不再续，收尾");
        return Step::Finish;
    }
    *turns += 1;
    eprintln!("↻ 第 {turns} 次续轮（最多 {} 次）：{board}#{id} 还由 {name} 持有，没交审，也没交给人", o.extra_turns);
    *phase = Phase::Working;
    Step::Send(client.again(&nudge(&board, id, &title, *turns, o.extra_turns)))
}


/// 这个名字在这块白板上持有的卡片（编号、标题）：`GET /api/agents` 列出在线的名字和它们持有的卡片。
/// 连接和 board 命令一样（board host 给的 `BOARD_URL`、`BOARD_TOKEN`、`BOARD_CONFIG`），请求最多等 10 秒
fn held(name: &str, board: &str) -> Result<Option<(i64, String)>, String> {
    let b = crate::backend::Backend::open_host().map_err(|e| e.message)?;
    let agents: Vec<board_core::AgentInfo> = b.remote().get("/api/agents", &[]).map_err(|e| e.message)?;
    let card = agents.into_iter().filter(|a| a.name == name).flat_map(|a| a.cards).find(|c| c.board == board);
    Ok(card.map(|c| (c.card.id, c.card.title)))
}


/// 续轮的提示词：还持有哪张、为什么要接着做、后台命令怎么办、做不下去怎么办
fn nudge(board: &str, id: i64, title: &str, n: u32, max: u32) -> String {
    format!(
        "这一轮结束了，但你还持有 {board}#{id}「{title}」：没交审，也没交给人。这次派发没有人回答问题，不交的话卡片会记成异常退出。\
         不要停下来等人确认：按 board-skills 接着做完、交审；做不下去就按「做不下去」交给人。\
         后台命令还没跑完的话，就在这一轮里用工具等它跑完，不要结束这一轮去等通知。（第 {n} 次续轮，最多 {max} 次）"
    )
}


/// 等 agent 退出，最多 limit；到时还在就结束它（整棵进程树交给 board host 收尾）
fn wait(child: &mut Child, job: &Job, limit: Duration) -> Option<std::process::ExitStatus> {
    let until = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Some(s),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                job.terminate(true);
                let _ = child.kill();
                return child.wait().ok();
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::testutil::{reply, s};
    use crate::acp::{parse, CHAT_IDLE_EXIT};
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn parses_the_command_line() {
        let o = parse(&s(&["--env", "login", "--env-prog", "/bin/zsh", "--cwd", "/w", "--prompt-file", "/p", "--mode", "bypassPermissions", "--set", "reasoning_effort=high", "--", "npx", "-y", "x@1"])).unwrap();
        assert_eq!((o.env.as_str(), o.env_prog.as_str(), o.mode.as_deref(), o.prog.as_str()), ("login", "/bin/zsh", Some("bypassPermissions"), "npx"));
        assert_eq!(o.sets, vec![("reasoning_effort".to_string(), "high".to_string())]);
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--set", "x", "--", "a"])).unwrap_err().contains("<选项>=<值>"));
        assert_eq!(o.args, s(&["-y", "x@1"]));
        // agent 的参数里有 -- 开头的也照原样传
        let o = parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--", "dsh", "--profile", "acp"])).unwrap();
        assert_eq!((o.mode, o.args), (None, s(&["--profile", "acp"])));
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p"])).is_err(), "没有 --");
        assert!(parse(&s(&["--env", "direct", "--prompt-file", "/p", "--", "x"])).unwrap_err().contains("--cwd"));
        assert!(parse(&s(&["--nope", "1", "--", "x"])).is_err());
        // 续轮：board host 只给做卡的派发带；不带是 0（不续）
        assert_eq!(o.extra_turns, 0);
        let o = parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--extra-turns", "3", "--provider-file", "/r", "--", "x"])).unwrap();
        assert_eq!((o.extra_turns, o.prog.as_str(), o.provider_file), (3, "x", Some(PathBuf::from("/r"))));
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--extra-turns", "many", "--", "x"])).unwrap_err().contains("--extra-turns"));
    }

    #[test]
    fn the_nudge_says_which_card_and_what_to_do() {
        let t = nudge("default/main", 454, "ACP 续轮", 1, 3);
        assert!(t.contains("default/main#454「ACP 续轮」"), "{t}");
        assert!(t.contains("按 board-skills 接着做完") && t.contains("做不下去"), "{t}");
        assert!(t.contains("在这一轮里用工具等它跑完") && t.ends_with("（第 1 次续轮，最多 3 次）"), "{t}");
    }

    /// 机器对话（--chat）：命令行不要 --prompt-file；带 --resume、--idle-exit
    #[test]
    fn chat_mode_command_line() {
        let o = parse(&s(&["--env", "direct", "--cwd", "/home/me", "--chat", "/run/chat-3.state", "--resume", "s1", "--", "claude-agent-acp"])).unwrap();
        assert_eq!((o.chat.as_deref(), o.resume.as_deref(), o.idle_exit), (Some(std::path::Path::new("/run/chat-3.state")), Some("s1"), CHAT_IDLE_EXIT));
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--", "x"])).is_err(), "不是对话时照旧要 --prompt-file");
        assert_eq!(chat_cmd(r#"{"seq":2,"prompt":"看看主目录"}"#), Some(ChatCmd::Prompt(2, "看看主目录".into())));
        assert_eq!(chat_cmd(r#"{"cancel":true}"#), Some(ChatCmd::Cancel));
        assert_eq!(chat_cmd("not json"), None);
    }

    fn opts(extra_turns: u32) -> Opts {
        let mut o = parse(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--", "x"].map(String::from)).unwrap();
        o.extra_turns = extra_turns;
        o
    }


    /// 这一轮做完以后的几步（不用查卡片的部分；查卡片在 tests/acp.rs 里连真的服务端测）
    #[test]
    fn steps_after_a_turn() {
        let ended = || {
            let mut c = Client::new("/w", "p", None);
            c.outcome = Some(Outcome::Stopped("end_turn".into()));
            c
        };
        let (mut turns, quiet) = (0, Instant::now() - SETTLE_QUIET);
        // 还在干活：等
        let mut c = Client::new("/w", "p", None);
        let mut phase = Phase::Working;
        assert_eq!(next_step(&opts(3), &mut c, &mut phase, &mut turns, quiet, true), Step::Stay);
        // 不续轮的派发（拆解、自检）、别的结束原因：做完就收尾
        assert_eq!(next_step(&opts(0), &mut ended(), &mut phase, &mut turns, quiet, true), Step::Finish);
        let mut c = Client::new("/w", "p", None);
        c.outcome = Some(Outcome::Stopped("max_tokens".into()));
        assert_eq!(next_step(&opts(3), &mut c, &mut phase, &mut turns, quiet, true), Step::Finish);
        // 做卡的：先等它稳定下来
        let mut c = ended();
        assert_eq!(next_step(&opts(3), &mut c, &mut phase, &mut turns, quiet, true), Step::Stay);
        assert!(matches!(phase, Phase::Settling(_)));
        // 刚收到过输出：还没稳定，接着等
        assert_eq!(next_step(&opts(3), &mut c, &mut phase, &mut turns, Instant::now(), true), Step::Stay);
        assert!(matches!(phase, Phase::Settling(_)));
        // v2：稳定之前 agent 又报了 running，自己接着做了：回到等这一轮
        c.outcome = None;
        assert_eq!(next_step(&opts(3), &mut c, &mut phase, &mut turns, Instant::now(), true), Step::Stay);
        assert_eq!(phase, Phase::Working);
        // 在等后台：后台跑完（后台为空）就重新等它稳定；agent 自己接着做就回到等这一轮
        let mut c = ended();
        c.background = Some(0);
        let mut phase = Phase::Waiting(Instant::now() + RECHECK);
        assert_eq!(next_step(&opts(3), &mut c, &mut phase, &mut turns, quiet, true), Step::Stay);
        assert!(matches!(phase, Phase::Settling(_)));
        c.background = Some(1);
        let mut phase = Phase::Waiting(Instant::now() + RECHECK);
        assert_eq!(next_step(&opts(3), &mut c, &mut phase, &mut turns, quiet, true), Step::Stay, "还没到再查的时间");
        assert!(matches!(phase, Phase::Waiting(_)));
        c.outcome = None;
        assert_eq!(next_step(&opts(3), &mut c, &mut phase, &mut turns, quiet, true), Step::Stay);
        assert_eq!(phase, Phase::Working);
        assert_eq!(turns, 0);
    }

    /// 派发日志里没有账号的邮箱、组织名（#669）：身份推送、`auth/status` 的回应去掉这两项，别的帧原样
    #[test]
    fn logged_auth_frames_have_no_email() {
        let push = json!({ "jsonrpc": "2.0", "method": "_auth/status_update", "params": { "authStatus": { "kind": "account", "label": "Claude Max", "account": { "email": "a@example.com", "organization": "Org", "plan": "max" } } } });
        let rfd = json!({ "jsonrpc": "2.0", "method": "auth/status_update", "params": { "state": "authenticated", "accounts": [{ "label": "Claude Max", "email": "a@example.com" }] } });
        let answer = reply(1, json!({ "state": "authenticated", "accounts": [{ "label": "Claude Max", "active": true, "email": "a@example.com", "organization": "Org" }] }));
        for f in [&push, &rfd, &answer] {
            let l = without_account(f).to_string();
            assert!(!l.contains("a@example.com") && !l.contains("Org"), "{l}");
            assert!(l.contains("Claude Max"), "{l}");
        }
        assert!(without_account(&push).to_string().contains("\"plan\":\"max\""));
        // 别的帧（agent 改的文件里也可能有 email 字段）：原样
        let other = json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "update": { "sessionUpdate": "tool_call", "rawInput": { "email": "b@example.com" } } } });
        assert!(matches!(without_account(&other), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn quota_file_is_an_option() {
        let o = parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--quota-file", "/q", "--", "x"])).unwrap();
        assert_eq!(o.quota_file, Some(PathBuf::from("/q")));
        assert_eq!(opts(0).quota_file, None);
    }
}
