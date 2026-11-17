//! `board acp`：board host 和 agent 之间的一层，讲 ACP（Agent Client Protocol v1，<https://agentclientprotocol.com>）。
//!
//! board host 一个派发启动一个 `board acp`（进程模型、停止、超时、日志都和以前一样按「一个派发一个进程、输出进日志文件」），
//! 它在这次派发的启动环境里拉起 agent 的 ACP 入口（dsh 自带的 `dsh --profile acp`、Claude Code 的
//! `claude-agent-acp`、Codex 的 `codex-acp`），走一遍：
//!
//! 1. `initialize`：协议版本 1；Client 的 `fs`、`terminal` 能力都不声明（agent 自己在 worktree 里读写、执行）。
//! 2. `session/new`：工作目录是派发的目录，`mcpServers` 为空（MCP 由 Board 写进各 agent 自己的配置文件）。
//! 3. 带了 `--set <选项>=<值>` 并且 agent 的会话配置里有这一项、这个值时切过去（dsh 的 `reasoning_effort`）；
//!    带了 `--mode` 并且 agent 公布了这个模式时切过去（Claude Code 的 `bypassPermissions`、Codex 的 `agent-full-access`）。
//! 4. `session/prompt`：一条文本，就是派发的提示词（`--prompt-file`，不放进命令行）。
//! 5. 续轮（#454）：这一轮正常结束（`end_turn`），并且带了 `--extra-turns <次数>`（board host 只给做卡的派发带）时，先看这次
//!    派发的名字（`BOARD_AS`）在这块白板（`BOARD`）上还持有没持有卡片（`GET /api/agents`，连接和 board 命令一样）。还持有，
//!    就是卡片既没交审、也没交给人：在同一个会话里再发一条提示词续一轮（写明还持有哪张、按 board-skills 做完或交给人），
//!    最多续这么多轮，每次在标准错误写一行。不持有、查不到就收尾。agent 结束这一轮的原因各种各样（把长命令放到后台等通知、
//!    停下来问人、做到一半就总结），派发里没有人接话，不续的话结束这一轮就等于结束会话。
//! 6. 收尾：关掉 agent 的标准输入，最多等 5 秒让它退出，再退出自己。
//!
//! 期间 agent 发来的请求：`session/request_permission` 自动允许（和以前跳过审批的参数同一个口径，Board 没有人工审批）；
//! 别的（`fs/*`、`terminal/*` 这些没声明的能力）回 `-32601`。收到 SIGTERM、SIGINT（人点停止、board host 退出）时发
//! `session/cancel`，最多等 5 秒。
//!
//! 标准输出是派发日志：agent 发来的每一帧、自己发出的每一帧一行一个（JSON），board host 按 `acp` 格式转写、解析成事件
//! （logfmt.rs、events.rs）。连续的消息片段（`agent_message_chunk`、`agent_thought_chunk`）合成一帧再写，一段话是一行；
//! 攒着的片段最多 2 秒落一次，日志不会因为 agent 在说一大段话就长时间不动（「没输出多久算卡住」看日志）。
//! agent 的标准错误直接进日志；`board acp` 自己的提示写标准错误（`✗ …`）。
//!
//! 退出码：`end_turn` 0（续满了卡片还在手里也是 0，board host 按卡片还由它持有记成异常退出）；被取消 130；
//! `max_tokens`、`max_turn_requests`、`refusal` 3；握手失败、协议错误 2；prompt 还没结束 agent 就退出了：agent 的退出码
//! （它是 0 时记 1）。

use crate::envs::Env;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// 协议大版本
const PROTOCOL: i64 = 1;
/// 握手（initialize、session/new、切模式）多久没回应算失败：agent 第一次启动可能要装依赖、读配置，给足时间
const HANDSHAKE: Duration = Duration::from_secs(180);
/// 取消后等 agent 回 `cancelled`、收尾时等 agent 退出各最多多久
const GRACE: Duration = Duration::from_secs(5);
/// 攒着的消息片段最多多久落一次日志
const CHUNK_FLUSH: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(200);

pub const EXIT_OK: i32 = 0;
pub const EXIT_PROTOCOL: i32 = 2;
pub const EXIT_LIMIT: i32 = 3;
pub const EXIT_CANCELLED: i32 = 130;

/// 做卡的派发最多续几轮（board host 传 `--extra-turns`）
pub const EXTRA_TURNS: u32 = 3;

static TERM: AtomicBool = AtomicBool::new(false);

const USAGE: &str = "用法：board acp --env <启动环境> [--env-prog <程序>] --cwd <目录> --prompt-file <文件> [--mode <模式>] [--set <选项>=<值>]… [--extra-turns <次数>] -- <ACP 入口命令> [参数…]";

/// 命令行
#[derive(Debug, PartialEq)]
pub(crate) struct Opts {
    pub env: String,
    pub env_prog: String,
    pub cwd: PathBuf,
    pub prompt_file: PathBuf,
    pub mode: Option<String>,
    /// 会话开始后要切的配置（`--set reasoning_effort=high`）
    pub sets: Vec<(String, String)>,
    /// 这一轮结束时名字还持有卡片，最多再续几轮（`--extra-turns`，0 = 不续）
    pub extra_turns: u32,
    pub prog: String,
    pub args: Vec<String>,
}

pub(crate) fn parse(args: &[String]) -> Result<Opts, String> {
    let split = args.iter().position(|a| a == "--").ok_or(USAGE)?;
    let (head, tail) = (&args[..split], &args[split + 1..]);
    let mut m: HashMap<&str, String> = HashMap::new();
    let mut sets = vec![];
    let mut i = 0;
    while i < head.len() {
        let k = head[i].as_str();
        if !matches!(k, "--env" | "--env-prog" | "--cwd" | "--prompt-file" | "--mode" | "--set" | "--extra-turns") {
            return Err(format!("不认识的参数 {k}。{USAGE}"));
        }
        let v = head.get(i + 1).ok_or_else(|| format!("{k} 后面要有值。{USAGE}"))?;
        if k == "--set" {
            let (id, val) = v.split_once('=').filter(|(a, b)| !a.is_empty() && !b.is_empty()).ok_or_else(|| format!("--set 要写成 <选项>=<值>：{v}"))?;
            sets.push((id.to_string(), val.to_string()));
        } else {
            m.insert(k, v.clone());
        }
        i += 2;
    }
    let need = |k: &str| m.get(k).cloned().ok_or_else(|| format!("缺 {k}。{USAGE}"));
    let prog = tail.first().cloned().ok_or(USAGE)?;
    let extra_turns = match m.get("--extra-turns") {
        Some(v) => v.trim().parse::<u32>().map_err(|_| format!("--extra-turns 要写次数（0、1、2 …）：{v}"))?,
        None => 0,
    };
    Ok(Opts {
        env: need("--env")?,
        env_prog: m.get("--env-prog").cloned().unwrap_or_default(),
        cwd: PathBuf::from(need("--cwd")?),
        prompt_file: PathBuf::from(need("--prompt-file")?),
        mode: m.get("--mode").cloned().filter(|x| !x.trim().is_empty()),
        sets,
        extra_turns,
        prog,
        args: tail[1..].to_vec(),
    })
}

/* ------------------------------ 协议 ------------------------------ */

/// 等着回应的请求
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pending {
    Init,
    New,
    Mode,
    Prompt,
}

/// 这一次运行怎么结束的
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    /// prompt 的响应：`stopReason`
    Stopped(String),
    /// 握手、prompt 出错（原因）
    Failed(String),
}

/// 客户端的状态：收一帧、给出要发的帧。不碰进程和文件，单元测试直接喂帧
pub(crate) struct Client {
    next: i64,
    pending: HashMap<i64, Pending>,
    session: Option<String>,
    cwd: String,
    prompt: String,
    mode: Option<String>,
    sets: Vec<(String, String)>,
    /// session/new 之后、prompt 之前要发的配置请求（一个一个发）
    queue: Vec<(&'static str, Value)>,
    pub outcome: Option<Outcome>,
    /// 已经发了 session/prompt（之后才算「进行中」，取消才有意义）
    prompting: bool,
}

impl Client {
    pub(crate) fn new(cwd: &str, prompt: &str, mode: Option<&str>) -> Client {
        Client {
            next: 0,
            pending: HashMap::new(),
            session: None,
            cwd: cwd.into(),
            prompt: prompt.into(),
            mode: mode.map(str::to_string),
            sets: vec![],
            queue: vec![],
            outcome: None,
            prompting: false,
        }
    }

    /// 会话开始后要切的配置（`--set`）
    pub(crate) fn with_sets(mut self, sets: Vec<(String, String)>) -> Client {
        self.sets = sets;
        self
    }

    /// 发下一个配置请求；都发完了发 prompt
    fn step(&mut self) -> Vec<Value> {
        if self.queue.is_empty() {
            return self.prompt();
        }
        let (method, params) = self.queue.remove(0);
        vec![self.request(method, params, Pending::Mode)]
    }

    fn request(&mut self, method: &str, params: Value, what: Pending) -> Value {
        let id = self.next;
        self.next += 1;
        self.pending.insert(id, what);
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    /// 握手的第一帧
    pub(crate) fn start(&mut self) -> Vec<Value> {
        let params = json!({
            "protocolVersion": PROTOCOL,
            "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
            "clientInfo": { "name": "board", "title": "Board", "version": env!("CARGO_PKG_VERSION") },
        });
        vec![self.request("initialize", params, Pending::Init)]
    }

    /// 还在握手（没发 prompt、也没结束）
    pub(crate) fn handshaking(&self) -> bool {
        !self.prompting && self.outcome.is_none()
    }

    /// 取消：prompt 进行中时发 `session/cancel`（通知，没有回应；agent 用 `cancelled` 结束这一轮）
    pub(crate) fn cancel(&mut self) -> Vec<Value> {
        match (&self.session, self.prompting && self.outcome.is_none()) {
            (Some(s), true) => vec![json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": s } })],
            _ => vec![],
        }
    }

    fn prompt(&mut self) -> Vec<Value> {
        let text = self.prompt.clone();
        self.send_prompt(&text)
    }

    /// 续轮：这一轮结束了、卡片还没做完时，在同一个会话里再发一条 prompt，回到「进行中」（取消照样有效）
    pub(crate) fn again(&mut self, text: &str) -> Vec<Value> {
        if self.session.is_none() || self.outcome.is_none() {
            return vec![];
        }
        self.outcome = None;
        self.send_prompt(text)
    }

    fn send_prompt(&mut self, text: &str) -> Vec<Value> {
        let Some(s) = self.session.clone() else { return vec![] };
        self.prompting = true;
        let params = json!({ "sessionId": s, "prompt": [{ "type": "text", "text": text }] });
        vec![self.request("session/prompt", params, Pending::Prompt)]
    }

    /// 收到 agent 的一帧，返回要发回去的帧
    pub(crate) fn on_frame(&mut self, v: &Value) -> Vec<Value> {
        let id = v.get("id").filter(|x| !x.is_null());
        match (v.get("method").and_then(Value::as_str), id) {
            // agent 发来的请求
            (Some(method), Some(id)) => vec![answer(method, id, &v["params"])],
            // 通知：只记日志
            (Some(_), None) => vec![],
            // 回应
            (None, Some(id)) => match id.as_i64().and_then(|i| self.pending.remove(&i)) {
                Some(what) => self.on_response(what, v),
                None => vec![],
            },
            (None, None) => vec![],
        }
    }

    fn on_response(&mut self, what: Pending, v: &Value) -> Vec<Value> {
        let r = &v["result"];
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            let msg = error_text(e);
            match what {
                // 切不了模式、配置不影响干活：接着发下一个，最后发 prompt（权限请求还会自动允许）
                Pending::Mode => return self.step(),
                Pending::Init => self.outcome = Some(Outcome::Failed(format!("initialize 失败：{msg}"))),
                Pending::New if e["code"].as_i64() == Some(-32000) => self.outcome = Some(Outcome::Failed(format!("agent 要求先登录：{msg}"))),
                Pending::New => self.outcome = Some(Outcome::Failed(format!("session/new 失败：{msg}"))),
                Pending::Prompt => self.outcome = Some(Outcome::Failed(format!("session/prompt 失败：{msg}"))),
            }
            return vec![];
        }
        match what {
            Pending::Init => {
                let params = json!({ "cwd": self.cwd, "mcpServers": [] });
                vec![self.request("session/new", params, Pending::New)]
            }
            Pending::New => {
                let Some(s) = r["sessionId"].as_str().filter(|s| !s.is_empty()) else {
                    self.outcome = Some(Outcome::Failed("session/new 的回应里没有 sessionId".into()));
                    return vec![];
                };
                self.session = Some(s.to_string());
                let mut queue: Vec<(&'static str, Value)> = self.sets.iter().filter_map(|(id, v)| config_set(r, s, id, v)).collect();
                queue.extend(self.mode.clone().and_then(|m| mode_switch(r, s, &m)));
                self.queue = queue;
                self.step()
            }
            Pending::Mode => self.step(),
            Pending::Prompt => {
                let reason = r["stopReason"].as_str().unwrap_or("end_turn").to_string();
                self.outcome = Some(Outcome::Stopped(reason));
                vec![]
            }
        }
    }
}

/// 回答 agent 的请求：权限自动允许；其他方法没有（没声明的能力）
fn answer(method: &str, id: &Value, params: &Value) -> Value {
    match method {
        "session/request_permission" => json!({ "jsonrpc": "2.0", "id": id, "result": { "outcome": permission(params) } }),
        _ => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Board 不支持 {method}") } }),
    }
}

/// 选「总是允许」，没有就「允许这一次」；都没有时回取消
fn permission(params: &Value) -> Value {
    let opts = params["options"].as_array().cloned().unwrap_or_default();
    let kind = |k: &str| opts.iter().find(|o| o["kind"].as_str() == Some(k)).and_then(|o| o["optionId"].as_str()).map(str::to_string);
    let pick = kind("allow_always").or_else(|| kind("allow_once")).or_else(|| {
        opts.iter().find(|o| o["kind"].as_str().is_some_and(|k| k.starts_with("allow"))).and_then(|o| o["optionId"].as_str()).map(str::to_string)
    });
    match pick {
        Some(id) => json!({ "outcome": "selected", "optionId": id }),
        None => json!({ "outcome": "cancelled" }),
    }
}

/// 按 session/new 的回应决定怎么切到 mode：会话配置里有「模式」这一项、且有这个值时用 `session/set_config_option`；
/// 否则公布的模式里有它时用 `session/set_mode`；都没有时不切
fn mode_switch(r: &Value, session: &str, mode: &str) -> Option<(&'static str, Value)> {
    for o in r["configOptions"].as_array().into_iter().flatten() {
        let is_mode = o["id"].as_str() == Some("mode") || o["category"].as_str() == Some("mode");
        if is_mode && option_values(&o["options"]).iter().any(|v| v == mode) {
            let id = o["id"].as_str().unwrap_or("mode");
            return Some(("session/set_config_option", json!({ "sessionId": session, "configId": id, "value": mode })));
        }
    }
    let modes = r["modes"]["availableModes"].as_array().into_iter().flatten();
    if modes.into_iter().any(|m| m["id"].as_str() == Some(mode)) {
        return Some(("session/set_mode", json!({ "sessionId": session, "modeId": mode })));
    }
    None
}

/// 会话配置里有 id 这一项、而且有这个值时：`session/set_config_option`
fn config_set(r: &Value, session: &str, id: &str, value: &str) -> Option<(&'static str, Value)> {
    let o = r["configOptions"].as_array()?.iter().find(|o| o["id"].as_str() == Some(id))?;
    let offered = option_values(&o["options"]).iter().any(|v| v == value);
    offered.then(|| ("session/set_config_option", json!({ "sessionId": session, "configId": id, "value": value })))
}

/// 选项的值：`[{value, name}]`，也可能分组 `[{group, options: [...]}]`
fn option_values(v: &Value) -> Vec<String> {
    let mut out = vec![];
    for o in v.as_array().into_iter().flatten() {
        if let Some(x) = o["value"].as_str() {
            out.push(x.to_string());
        }
        out.extend(option_values(&o["options"]));
    }
    out
}

fn error_text(e: &Value) -> String {
    let msg = e["message"].as_str().unwrap_or("出错了");
    match e["data"].as_str().or_else(|| e["data"]["message"].as_str()) {
        Some(d) if !d.is_empty() && d != msg => format!("{msg}（{d}）"),
        _ => msg.to_string(),
    }
}

/// 结束原因 → 退出码
pub(crate) fn exit_for(o: &Outcome) -> i32 {
    match o {
        Outcome::Stopped(r) if r == "end_turn" => EXIT_OK,
        Outcome::Stopped(r) if r == "cancelled" => EXIT_CANCELLED,
        Outcome::Stopped(_) => EXIT_LIMIT,
        Outcome::Failed(_) => EXIT_PROTOCOL,
    }
}

/* ------------------------------ 日志：合并消息片段 ------------------------------ */

/// 连续的同一种消息片段合成一帧
#[derive(Default)]
pub(crate) struct Merger {
    /// （片段种类、sessionId、攒的文字、第一段的时间）
    buf: Option<(String, Value, String, Instant)>,
}

impl Merger {
    /// 一帧 agent 发来的：返回现在要写进日志的行（片段先攒着）
    pub(crate) fn push(&mut self, v: &Value) -> Vec<String> {
        if let Some((kind, session, text)) = chunk(v) {
            match &mut self.buf {
                Some((k, s, t, _)) if *k == kind && *s == session => {
                    t.push_str(&text);
                    return vec![];
                }
                _ => {
                    let out: Vec<String> = self.flush().into_iter().collect();
                    self.buf = Some((kind, session, text, Instant::now()));
                    return out;
                }
            }
        }
        let mut out: Vec<String> = self.flush().into_iter().collect();
        out.push(v.to_string());
        out
    }

    /// 攒的片段已经超过 CHUNK_FLUSH：落一次
    pub(crate) fn due(&self) -> bool {
        self.buf.as_ref().is_some_and(|(_, _, _, at)| at.elapsed() >= CHUNK_FLUSH)
    }

    pub(crate) fn flush(&mut self) -> Option<String> {
        let (kind, session, text, _) = self.buf.take()?;
        let update = json!({ "sessionUpdate": kind, "content": { "type": "text", "text": text } });
        Some(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": update } }).to_string())
    }
}

/// 是消息片段（文字）时：（种类、sessionId、文字）
fn chunk(v: &Value) -> Option<(String, Value, String)> {
    if v["method"].as_str() != Some("session/update") {
        return None;
    }
    let u = &v["params"]["update"];
    let kind = u["sessionUpdate"].as_str().filter(|k| matches!(*k, "agent_message_chunk" | "agent_thought_chunk"))?;
    let text = u["content"]["text"].as_str().filter(|_| u["content"]["type"].as_str() == Some("text"))?;
    Some((kind.to_string(), v["params"]["sessionId"].clone(), text.to_string()))
}

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
    let code = run(&o);
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

/// 写一行日志（标准输出）
fn log(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// 发一帧给 agent，也写进日志
fn send(stdin: &mut Option<ChildStdin>, frames: Vec<Value>) {
    for f in frames {
        let line = f.to_string();
        log(&line);
        if let Some(w) = stdin.as_mut() {
            if writeln!(w, "{line}").and_then(|_| w.flush()).is_err() {
                *stdin = None;
            }
        }
    }
}

fn run(o: &Opts) -> i32 {
    let prompt = match std::fs::read_to_string(&o.prompt_file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ board acp：读不了提示词 {}：{e}", o.prompt_file.display());
            return EXIT_PROTOCOL;
        }
    };
    let env = Env::from_parts(&o.env, &o.env_prog);
    let mut c = env.command(&o.prog, &o.args, Some(&o.cwd), true);
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => {
            eprintln!("✗ board acp：在{}里启动 {} 失败：{e}", env.name, o.prog);
            return EXIT_PROTOCOL;
        }
    };
    let mut stdin = child.stdin.take();
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
    let cwd = o.cwd.display().to_string();
    let mut client = Client::new(&cwd, &prompt, o.mode.as_deref()).with_sets(o.sets.clone());
    let mut merger = Merger::default();
    send(&mut stdin, client.start());
    let started = Instant::now();
    // 已经续了几轮（见 follow_up）
    let mut turns: u32 = 0;
    let mut cancel_at: Option<Instant> = None;
    // agent 进程已经退出的时间：它留下的子进程可能还拿着标准输出，读不到 EOF；退出后再收一会儿剩下的输出就收尾
    let mut gone_at: Option<Instant> = None;
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
        if merger.due() {
            if let Some(l) = merger.flush() {
                log(&l);
            }
        }
        match rx.recv_timeout(TICK) {
            Ok(Some(line)) => {
                let t = line.trim();
                let v = if t.starts_with('{') { serde_json::from_str::<Value>(t).ok().filter(Value::is_object) } else { None };
                match v {
                    Some(v) => {
                        for l in merger.push(&v) {
                            log(&l);
                        }
                        let reply = client.on_frame(&v);
                        send(&mut stdin, reply);
                    }
                    // 不是协议帧（agent 不该往标准输出写别的）：原样记下
                    None if !t.is_empty() => {
                        if let Some(l) = merger.flush() {
                            log(&l);
                        }
                        log(&line);
                    }
                    None => {}
                }
                if client.outcome.is_some() {
                    // 这一轮结束了：卡片还在这个名字手里，就在同一个会话里续一轮（#454）
                    match follow_up(o, &client, turns, stdin.is_some()) {
                        Some(text) => {
                            turns += 1;
                            send(&mut stdin, client.again(&text));
                        }
                        None => break,
                    }
                }
            }
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    if let Some(l) = merger.flush() {
        log(&l);
    }
    // 收尾：关掉 agent 的标准输入（ACP 的 agent 读到 EOF 就退出），等一会儿，还不退就结束它
    drop(stdin);
    let status = wait(&mut child, GRACE);
    match &client.outcome {
        Some(o) => {
            if let Outcome::Failed(why) = o {
                eprintln!("✗ {why}");
            }
            exit_for(o)
        }
        None if TERM.load(Ordering::SeqCst) => EXIT_CANCELLED,
        None => {
            let code = status.and_then(|s| s.code()).unwrap_or(1);
            eprintln!("✗ agent 在这一轮结束之前就退出了（退出码 {code}）");
            if code == 0 { 1 } else { code }
        }
    }
}

/// 这一轮正常结束（`end_turn`）之后要不要续一轮：带了 `--extra-turns`、这次派发的名字（`BOARD_AS`）在这块白板（`BOARD`）上
/// 还持有卡片时，返回续轮的提示词，并在标准错误写一行（进日志，「过程」里看得到）。`done` 是已经续了几轮，
/// `alive` 是 agent 的标准输入还写得进去
fn follow_up(o: &Opts, client: &Client, done: u32, alive: bool) -> Option<String> {
    let ended = client.outcome.as_ref() == Some(&Outcome::Stopped("end_turn".into()));
    if o.extra_turns == 0 || !ended || !alive || TERM.load(Ordering::SeqCst) {
        return None;
    }
    let var = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let (Some(name), Some(board)) = (var("BOARD_AS"), var("BOARD")) else {
        eprintln!("✗ 环境变量里没有 BOARD_AS、BOARD：不知道这次派发的名字和白板，不续轮");
        return None;
    };
    match held(&name, &board) {
        Ok(None) => None,
        Ok(Some((id, title))) if done < o.extra_turns => {
            let n = done + 1;
            eprintln!("↻ 第 {n} 次续轮（最多 {} 次）：{board}#{id} 还由 {name} 持有，没交审，也没交给人", o.extra_turns);
            Some(nudge(&board, id, &title, n, o.extra_turns))
        }
        Ok(Some((id, _))) => {
            eprintln!("✗ 续了 {done} 轮，{board}#{id} 还由 {name} 持有：不再续，收尾");
            None
        }
        Err(e) => {
            eprintln!("✗ 查不到 {name} 还持有没持有卡片（{e}）：不续轮，收尾");
            None
        }
    }
}

/// 这个名字在这块白板上持有的卡片（编号、标题）：`GET /api/agents` 列出在线的名字和它们持有的卡片。
/// 连接和 board 命令一样（board host 给的 `BOARD_URL`、`BOARD_TOKEN`、`BOARD_CONFIG`），请求最多等 10 秒
fn held(name: &str, board: &str) -> Result<Option<(i64, String)>, String> {
    let b = crate::backend::Backend::open_host().map_err(|e| e.message)?;
    let agents: Vec<board_core::AgentInfo> = b.remote().get("/api/agents", &[]).map_err(|e| e.message)?;
    let card = agents.into_iter().filter(|a| a.name == name).flat_map(|a| a.cards).find(|c| c.board == board);
    Ok(card.map(|c| (c.card.id, c.card.title)))
}

/// 续轮的提示词：还持有哪张、为什么要接着做、做不下去怎么办
fn nudge(board: &str, id: i64, title: &str, n: u32, max: u32) -> String {
    format!(
        "这一轮结束了，但你还持有 {board}#{id}「{title}」：没交审，也没交给人。这次派发没有人回答问题，结束这一轮就是结束会话，\
         卡片会记成异常退出。不要停下来等人确认：按 board-skills 接着做完、交审；做不下去就按「做不下去」交给人。\
         长命令在前台跑完，不要放到后台再结束这一轮。（第 {n} 次续轮，最多 {max} 次）"
    )
}

/// 等 agent 退出，最多 limit；到时还在就结束它（整棵进程树交给 board host 收尾）
fn wait(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let until = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Some(s),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                return child.wait().ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

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
        let o = parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--extra-turns", "3", "--", "x"])).unwrap();
        assert_eq!((o.extra_turns, o.prog.as_str()), (3, "x"));
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--extra-turns", "many", "--", "x"])).unwrap_err().contains("--extra-turns"));
    }

    /// 续轮（#454）：这一轮结束后在同一个会话里再发一条 prompt，回到「进行中」，取消照样有效
    #[test]
    fn again_sends_another_prompt_in_the_same_session() {
        let mut c = Client::new("/w", "p", None);
        assert!(c.again("x").is_empty(), "还没有会话：不发");
        c.start();
        c.on_frame(&reply(0, json!({})));
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s" })));
        assert_eq!((f[0]["method"].as_str(), f[0]["id"].as_i64()), (Some("session/prompt"), Some(2)));
        assert!(c.again("x").is_empty(), "这一轮还没结束：不发");
        c.on_frame(&reply(2, json!({ "stopReason": "end_turn" })));
        assert_eq!(c.outcome, Some(Outcome::Stopped("end_turn".into())));
        let f = c.again("还持有 #7：接着做完");
        assert_eq!((f[0]["method"].as_str(), f[0]["id"].as_i64()), (Some("session/prompt"), Some(3)));
        assert_eq!(f[0]["params"], json!({ "sessionId": "s", "prompt": [{ "type": "text", "text": "还持有 #7：接着做完" }] }));
        assert_eq!(c.outcome, None, "续的这一轮在进行中");
        assert!(!c.handshaking());
        assert_eq!(c.cancel()[0]["method"], "session/cancel");
        // 续的这一轮结束：照常记结果
        c.on_frame(&reply(3, json!({ "stopReason": "end_turn" })));
        assert_eq!(c.outcome, Some(Outcome::Stopped("end_turn".into())));
    }

    #[test]
    fn the_nudge_says_which_card_and_what_to_do() {
        let t = nudge("default/main", 454, "ACP 续轮", 1, 3);
        assert!(t.contains("default/main#454「ACP 续轮」"), "{t}");
        assert!(t.contains("按 board-skills 接着做完") && t.contains("做不下去"), "{t}");
        assert!(t.contains("不要放到后台") && t.ends_with("（第 1 次续轮，最多 3 次）"), "{t}");
    }

    fn reply(id: i64, result: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    }

    /// 走一遍：initialize → session/new → 切模式 → prompt → end_turn
    #[test]
    fn handshake_mode_and_prompt() {
        let mut c = Client::new("/w", "/board-skills 开始执行", Some("bypassPermissions"));
        let f = c.start();
        assert_eq!((f[0]["method"].as_str(), f[0]["id"].as_i64(), f[0]["params"]["protocolVersion"].as_i64()), (Some("initialize"), Some(0), Some(1)));
        assert_eq!(f[0]["params"]["clientCapabilities"]["terminal"], false);
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 1, "agentCapabilities": {} })));
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["cwd"].as_str()), (Some("session/new"), Some("/w")));
        assert_eq!(f[0]["params"]["mcpServers"], json!([]));
        assert!(c.handshaking());
        // 会话配置里有 mode 这一项（Claude Code）：用 set_config_option
        let opts = json!([{ "id": "mode", "category": "mode", "type": "select", "currentValue": "default", "options": [{ "value": "default" }, { "value": "bypassPermissions" }] }]);
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s1", "configOptions": opts })));
        assert_eq!(f[0]["method"], "session/set_config_option");
        assert_eq!(f[0]["params"], json!({ "sessionId": "s1", "configId": "mode", "value": "bypassPermissions" }));
        assert!(c.cancel().is_empty(), "还没发 prompt：没有可取消的");
        let f = c.on_frame(&reply(2, json!({})));
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["prompt"][0]["text"].as_str()), (Some("session/prompt"), Some("/board-skills 开始执行")));
        assert!(!c.handshaking());
        // 期间的通知不用回
        assert!(c.on_frame(&json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s1", "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "hi" } } } })).is_empty());
        assert_eq!(c.cancel()[0], json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": "s1" } }));
        assert!(c.on_frame(&reply(3, json!({ "stopReason": "end_turn" }))).is_empty());
        assert_eq!(c.outcome, Some(Outcome::Stopped("end_turn".into())));
        assert_eq!(exit_for(c.outcome.as_ref().unwrap()), 0);
    }

    /// --set：会话配置里有这一项、这个值时先切它，再切模式，最后 prompt；没有的跳过
    #[test]
    fn config_sets_go_before_the_mode_and_the_prompt() {
        let mut c = Client::new("/w", "p", Some("full")).with_sets(vec![("reasoning_effort".into(), "high".into()), ("model".into(), "nope".into())]);
        c.start();
        c.on_frame(&reply(0, json!({})));
        let opts = json!([
            { "id": "model", "options": [{ "value": "a" }] },
            { "id": "reasoning_effort", "options": [{ "value": "low" }, { "value": "high" }] }
        ]);
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s", "configOptions": opts, "modes": { "availableModes": [{ "id": "full" }] } })));
        assert_eq!(f[0]["params"], json!({ "sessionId": "s", "configId": "reasoning_effort", "value": "high" }));
        let f = c.on_frame(&reply(2, json!({})));
        assert_eq!(f[0]["method"], "session/set_mode", "model=nope 没有这个值：跳过");
        let f = c.on_frame(&reply(3, json!({})));
        assert_eq!(f[0]["method"], "session/prompt");
    }

    #[test]
    fn mode_falls_back_to_set_mode_or_skips() {
        let r = json!({ "sessionId": "s", "modes": { "currentModeId": "auto", "availableModes": [{ "id": "read-only" }, { "id": "agent-full-access" }] } });
        assert_eq!(mode_switch(&r, "s", "agent-full-access"), Some(("session/set_mode", json!({ "sessionId": "s", "modeId": "agent-full-access" }))));
        assert_eq!(mode_switch(&r, "s", "bypassPermissions"), None, "没公布这个模式：不切");
        // 分组的选项也认得
        let r = json!({ "configOptions": [{ "id": "perm", "category": "mode", "options": [{ "group": "g", "options": [{ "value": "yolo" }] }] }] });
        assert_eq!(mode_switch(&r, "s", "yolo").map(|x| x.1["configId"].clone()), Some(json!("perm")));
        // 没要模式：session/new 之后直接 prompt
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&reply(0, json!({})));
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s", "modes": r })));
        assert_eq!(f[0]["method"], "session/prompt");
        // 切模式失败也接着干活
        let mut c = Client::new("/w", "p", Some("x"));
        c.start();
        c.on_frame(&reply(0, json!({})));
        c.on_frame(&reply(1, json!({ "sessionId": "s", "modes": { "availableModes": [{ "id": "x" }] } })));
        let f = c.on_frame(&json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": -32603, "message": "nope" } }));
        assert_eq!(f[0]["method"], "session/prompt");
    }

    #[test]
    fn requests_from_the_agent_get_answers() {
        let mut c = Client::new("/w", "p", None);
        let ask = |id: Value, options: Value| json!({ "jsonrpc": "2.0", "id": id, "method": "session/request_permission", "params": { "sessionId": "s", "toolCall": { "toolCallId": "t" }, "options": options } });
        let both = json!([{ "optionId": "once", "kind": "allow_once" }, { "optionId": "always", "kind": "allow_always" }, { "optionId": "no", "kind": "reject_once" }]);
        assert_eq!(c.on_frame(&ask(json!(7), both))[0], json!({ "jsonrpc": "2.0", "id": 7, "result": { "outcome": { "outcome": "selected", "optionId": "always" } } }));
        let once = json!([{ "optionId": "no", "kind": "reject_once" }, { "optionId": "ok", "kind": "allow_once" }]);
        assert_eq!(c.on_frame(&ask(json!("a"), once))[0]["result"]["outcome"]["optionId"], "ok", "字符串的 id 原样带回");
        let none = json!([{ "optionId": "no", "kind": "reject_once" }]);
        assert_eq!(c.on_frame(&ask(json!(8), none))[0]["result"]["outcome"], json!({ "outcome": "cancelled" }));
        let fs = c.on_frame(&json!({ "jsonrpc": "2.0", "id": 9, "method": "fs/read_text_file", "params": {} }));
        assert_eq!(fs[0]["error"]["code"], -32601);
        // 不认识的回应、没有 id 的东西：不理
        assert!(c.on_frame(&reply(99, json!({}))).is_empty());
    }

    #[test]
    fn failures_and_stop_reasons_become_exit_codes() {
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&json!({ "jsonrpc": "2.0", "id": 0, "error": { "code": -32603, "message": "boom", "data": "details" } }));
        assert_eq!(c.outcome, Some(Outcome::Failed("initialize 失败：boom（details）".into())));
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&reply(0, json!({})));
        c.on_frame(&json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32000, "message": "Authentication required" } }));
        assert_eq!(c.outcome, Some(Outcome::Failed("agent 要求先登录：Authentication required".into())));
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&reply(0, json!({})));
        c.on_frame(&reply(1, json!({})));
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.contains("sessionId")));
        assert_eq!(exit_for(&Outcome::Stopped("cancelled".into())), 130);
        assert_eq!(exit_for(&Outcome::Stopped("max_tokens".into())), 3);
        assert_eq!(exit_for(&Outcome::Stopped("refusal".into())), 3);
        assert_eq!(exit_for(&Outcome::Failed("x".into())), 2);
    }

    fn piece(kind: &str, text: &str) -> Value {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s", "update": { "sessionUpdate": kind, "content": { "type": "text", "text": text } } } })
    }

    #[test]
    fn message_pieces_are_merged_into_one_line() {
        let mut m = Merger::default();
        assert!(m.push(&piece("agent_thought_chunk", "想")).is_empty());
        let out = m.push(&piece("agent_message_chunk", "先"));
        assert_eq!(out.len(), 1, "换了种类：先落思考");
        assert!(out[0].contains("\"agent_thought_chunk\"") && out[0].contains("想"));
        assert!(m.push(&piece("agent_message_chunk", "领卡，")).is_empty());
        assert!(m.push(&piece("agent_message_chunk", "再看代码。")).is_empty());
        let tool = json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s", "update": { "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "ls" } } });
        let out = m.push(&tool);
        assert_eq!(out.len(), 2, "攒的话先落，再写工具调用");
        let v: Value = serde_json::from_str(&out[0]).unwrap();
        assert_eq!(v["params"]["update"]["content"]["text"], "先领卡，再看代码。");
        assert_eq!(v["params"]["update"]["sessionUpdate"], "agent_message_chunk");
        assert!(out[1].contains("tool_call"));
        assert!(m.flush().is_none());
        assert!(!m.due());
    }
}
