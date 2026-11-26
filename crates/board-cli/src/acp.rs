//! `board acp`：board host 和 agent 之间的一层，讲 ACP（Agent Client Protocol，<https://agentclientprotocol.com>）。
//! 握手时要 v2（草案），agent 只会 v1 时按 v1 走（ACP 的版本协商：agent 回它能讲的最新版本）：Claude Code
//! （`claude-agent-acp`）、Codex（`codex-acp`）回 2，dsh 回 1（#461）。
//!
//! board host 一个派发启动一个 `board acp`（进程模型、停止、超时、日志都和以前一样按「一个派发一个进程、输出进日志文件」），
//! 它在这次派发的启动环境里拉起 agent 的 ACP 入口（dsh 自带的 `dsh --profile acp`、Claude Code 的
//! `claude-agent-acp`、Codex 的 `codex-acp`），走一遍：
//!
//! 1. `initialize`：要协议版本 2，同时带上 v1、v2 两套字段（v2 的 `info`、`capabilities`，v1 的 `clientInfo`、
//!    `clientCapabilities`），只会 v1 的 agent 照样认得。不声明 Client 能力（agent 自己在 worktree 里读写、执行）。
//! 2. 带了 `--provider-file <文件>`（按角色派发、角色用供应商时，board host 写的，读完就删）：`providers/set`，
//!    给这一个 agent 进程换上角色的供应商（ACP providers RFD；Claude Code 的 `main`、Codex 的 `openai`，地址和 key 的请求头），
//!    只对这个进程生效，不改 agent 的全局配置文件。失败时不往下走（用错的供应商干活不如不干）。
//! 3. `session/new`：工作目录是派发的目录，`mcpServers` 为空（MCP 由 Board 写进各 agent 自己的配置文件）。
//! 4. 带了 `--set <选项>=<值>` 并且 agent 的会话配置里有这一项、这个值时切过去（dsh 的 `reasoning_effort`）；`<选项>` 写成
//!    `@<类别>` 时按 ACP 配置项的类别找（`@model` 模型、`@thought_level` 思考强度：角色的模型、强度这样切）；
//!    带了 `--mode` 并且 agent 公布了这个模式时切过去（Claude Code 的 `bypassPermissions`、Codex 的 `agent-full-access`）。
//!    v2 的配置项用 `configId`、切的时候带 `type: "id"`，模式只在配置项里（没有 `session/set_mode`）。
//! 5. `session/prompt`：一条文本，就是派发的提示词（`--prompt-file`，不放进命令行）。v1 的回应跨整轮，带 `stopReason`；
//!    v2 的回应只表示收下了（`messageId`），这一轮的进展看 `state_update`：`running` / `requires_action` 是在干活，
//!    `idle` 带 `stopReason` 是这一轮做完了；agent 也可以不等提示词自己再开一轮（例如处理跑完的后台任务），那时又报 `running`。
//! 6. 这一轮做完以后（#454、#461）：先等它稳定下来——最后一帧之后安静 2 秒，最多等 10 秒（v1 收迟到的更新，v2 是规范要的
//!    宽限期：后台任务跑完可能紧接着开下一轮）。带了 `--extra-turns <次数>`（board host 只给做卡的派发带）时，再看这次派发的
//!    名字（`BOARD_AS`）在这块白板（`BOARD`）上还持有没持有卡片（`GET /api/agents`，连接和 board 命令一样）：
//!    - 不持有（交审了、交给人了）、查不到：收尾。
//!    - 还持有，v2 报的 `background` 不空（后台还有命令、任务在跑）：不收也不续，等 agent 跑完自己接着做；每 30 秒再查一次卡片。
//!    - 还持有，后台为空或 agent 不报（v1、Codex）：在同一个会话里再发一条提示词续一轮（写明还持有哪张、后台命令在这一轮里
//!      等它跑完、按 board-skills 做完或交给人），最多续这么多轮，每次在标准错误写一行；续满了收尾。
//! 7. 收尾：v2 先发 `session/close`，再关掉 agent 的标准输入，最多等 5 秒让它退出，再退出自己。
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
//! 退出码：`end_turn` 0（续满了卡片还在手里也是 0，board host 按卡片还由它持有记成异常退出）；被取消、收到 SIGTERM 130；
//! `max_tokens`、`max_turn_requests`、`refusal` 3；握手失败、协议错误、v2 这一轮出错（`stopReason: error`）2；
//! 这一轮因为订阅账号的额度用完出错 4（见下）；这一轮还没结束 agent 就退出了：agent 的退出码（它是 0 时记 1）。
//!
//! 额度（#668）：只用跑的时候顺带拿到的，不另外去查。Claude Code（`claude-agent-acp` 0.107.3）的 `usage_update` 在
//! `_meta["_claude/rateLimit"]` 里带 Agent SDK 的 `rate_limit_info`（状态、窗口、已用比例、重置时间）；额度用完时这一轮以
//! 出错结束，错误的 `data` 是 `{errorKind: "rate_limit", rateLimit}`，文字以 SDK 的 `USAGE_LIMIT_ERROR_PREFIXES` 开头。
//! Codex（`codex-acp` 2.7.0）不往 ACP 上发额度；用完时这一轮以出错结束，`data` 是 `{message, codexErrorInfo: "usageLimitExceeded"}`
//! （Zed 那一版 codex-acp 是 `session/prompt` 回错、`codex_error_info: "usage_limit_exceeded"`，也认），重置时间只在文字里
//! （「Try again at 3:15 PM.」，本机时区）。认出额度用完时退出码 4；带了 `--quota-file` 时把最近一次看到的额度（和是不是用完、
//! 重置时间）写进去，board host 随派发结束上报。

use crate::envs::Env;
use board_core::hosts::Quota;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// 握手时要的协议大版本：ACP v2（草案）。agent 只会 v1 时回 1，按 v1 走
const PROTOCOL: i64 = 2;
/// 握手（initialize、session/new、切模式）多久没回应算失败：agent 第一次启动可能要装依赖、读配置，给足时间
const HANDSHAKE: Duration = Duration::from_secs(180);
/// 取消后等 agent 回 `cancelled`、收尾时等 agent 退出各最多多久
const GRACE: Duration = Duration::from_secs(5);
/// 攒着的消息片段最多多久落一次日志
const CHUNK_FLUSH: Duration = Duration::from_secs(2);
/// 这一轮做完以后，最后一帧之后安静多久算稳定下来（v1 收迟到的更新，v2 的宽限期），以及最多等多久
const SETTLE_QUIET: Duration = Duration::from_secs(2);
const SETTLE_MAX: Duration = Duration::from_secs(10);
/// v2：卡片还在手里、后台还在跑时，隔多久再查一次卡片
const RECHECK: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_millis(200);

pub const EXIT_OK: i32 = 0;
pub const EXIT_PROTOCOL: i32 = 2;
pub const EXIT_LIMIT: i32 = 3;
/// 这一轮因为订阅账号的额度用完结束（#668）
pub const EXIT_QUOTA: i32 = 4;
pub const EXIT_CANCELLED: i32 = 130;

/// 做卡的派发最多续几轮（board host 传 `--extra-turns`）
pub const EXTRA_TURNS: u32 = 3;

static TERM: AtomicBool = AtomicBool::new(false);

const USAGE: &str = "用法：board acp --env <启动环境> [--env-prog <程序>] --cwd <目录> (--prompt-file <文件> | --chat <状态文件> [--resume <会话>] [--idle-exit <秒>]) [--provider-file <文件>] [--mode <模式>] [--set <选项>=<值>]… [--extra-turns <次数>] [--quota-file <文件>] -- <ACP 入口命令> [参数…]";

/// 对话模式（`--chat`）：多久没有新消息就收尾退出（board host 下一条消息来时再启动、接回会话）
pub const CHAT_IDLE_EXIT: u64 = 10 * 60;

/// 命令行
#[derive(Debug, PartialEq)]
pub(crate) struct Opts {
    pub env: String,
    pub env_prog: String,
    pub cwd: PathBuf,
    pub prompt_file: PathBuf,
    pub mode: Option<String>,
    /// 会话开始后要切的配置（`--set reasoning_effort=high`、`--set @model=sonnet`）
    pub sets: Vec<(String, String)>,
    /// `providers/set` 的参数（JSON，含 key）所在的文件：读完就删
    pub provider_file: Option<PathBuf>,
    /// 这一轮结束时名字还持有卡片，最多再续几轮（`--extra-turns`，0 = 不续）
    pub extra_turns: u32,
    /// 结束时把看到的额度写进这个文件（`--quota-file`，board host 读了随派发结束上报）
    pub quota_file: Option<PathBuf>,
    /// 机器对话（`--chat <状态文件>`）：提示词从标准输入一条一条来，每一轮结束不退出；会话标识、这一轮的状态写进这个文件
    pub chat: Option<PathBuf>,
    /// 对话：接回这个 ACP 会话（`--resume`）
    pub resume: Option<String>,
    /// 对话：多久（秒）没有新消息就收尾退出（`--idle-exit`）
    pub idle_exit: u64,
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
        if !matches!(k, "--env" | "--env-prog" | "--cwd" | "--prompt-file" | "--provider-file" | "--mode" | "--set" | "--extra-turns" | "--chat" | "--resume" | "--idle-exit" | "--quota-file") {
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
    let chat = m.get("--chat").filter(|x| !x.trim().is_empty()).map(PathBuf::from);
    let idle_exit = match m.get("--idle-exit") {
        Some(v) => v.trim().parse::<u64>().map_err(|_| format!("--idle-exit 要写秒数：{v}"))?,
        None => CHAT_IDLE_EXIT,
    };
    Ok(Opts {
        env: need("--env")?,
        env_prog: m.get("--env-prog").cloned().unwrap_or_default(),
        cwd: PathBuf::from(need("--cwd")?),
        prompt_file: if chat.is_some() { m.get("--prompt-file").map(PathBuf::from).unwrap_or_default() } else { PathBuf::from(need("--prompt-file")?) },
        mode: m.get("--mode").cloned().filter(|x| !x.trim().is_empty()),
        sets,
        provider_file: m.get("--provider-file").filter(|x| !x.trim().is_empty()).map(PathBuf::from),
        extra_turns,
        quota_file: m.get("--quota-file").filter(|x| !x.trim().is_empty()).map(PathBuf::from),
        resume: m.get("--resume").map(|x| x.trim().to_string()).filter(|x| !x.is_empty()),
        idle_exit,
        chat,
        prog,
        args: tail[1..].to_vec(),
    })
}

/* ------------------------------ 协议 ------------------------------ */

/// 等着回应的请求
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pending {
    Init,
    Provider,
    New,
    /// 对话：接回会话（`session/resume`、`session/load`）
    Resume,
    Load,
    Mode,
    Prompt,
    Close,
}

/// 协商到的协议版本
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Proto {
    V1,
    V2,
}

/// 这一次运行怎么结束的
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    /// 这一轮做完了：v1 是 prompt 的回应，v2 是 `idle` 的 `stopReason`
    Stopped(String),
    /// 握手、prompt 出错，v2 这一轮出错（原因）
    Failed(String),
    /// 这一轮因为订阅账号的额度用完出错（原因；额度见 [`Client::quota`]）
    Quota(String),
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
    /// initialize 之后、session/new 之前要发的 `providers/set` 参数
    provider: Option<Value>,
    /// session/new 之后、prompt 之前要发的配置请求（一个一个发）
    queue: Vec<(&'static str, Value)>,
    /// 这一轮怎么结束的；还在干活（v2 报了 `running`）时为空
    pub outcome: Option<Outcome>,
    /// 已经发了 session/prompt（之后才算「进行中」，取消才有意义）
    prompting: bool,
    /// 握手协商到的版本（握手完成前当 v1）
    pub proto: Proto,
    /// v2：最近一次报的后台工作有几项；`None` 是 agent 不报（不知道有没有），不是没有
    pub background: Option<usize>,
    /// 最近一次看到的额度（Claude 的 `usage_update` 带的）；额度用完时 `exhausted`，带上重置时间
    pub quota: Option<Quota>,
    /// 对话模式：配置切完后不发提示词，等 [`Client::ask`]
    chat: bool,
    /// 对话：要接回的会话
    resume: Option<String>,
    /// 已经发了接回请求的会话（回应里不一定带 sessionId）
    pending_resume: Option<String>,
    /// agent 公布的能力：接回会话（`sessionCapabilities.resume`）、载入会话（`loadSession`，会回放历史）
    can_resume: bool,
    can_load: bool,
    /// `session/load` 正在回放历史：这些更新不记日志（之前的对话已经记过了）
    pub replaying: bool,
    /// 对话：会话开好、配置切完，可以发提示词了
    pub ready: bool,
    /// 对话：接回会话的结果（给 board host 看的一句话）；开了新会话时说明为什么
    pub note: Option<String>,
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
            provider: None,
            queue: vec![],
            outcome: None,
            prompting: false,
            proto: Proto::V1,
            background: None,
            quota: None,
            chat: false,
            resume: None,
            pending_resume: None,
            can_resume: false,
            can_load: false,
            replaying: false,
            ready: false,
            note: None,
        }
    }

    /// 机器对话：提示词由 [`Client::ask`] 一条一条发；有 `resume` 时先接回这个会话
    pub(crate) fn for_chat(mut self, resume: Option<String>) -> Client {
        self.chat = true;
        self.resume = resume;
        self
    }

    /// 会话标识（开好、接回之后才有）
    pub(crate) fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    /// 对话：发一条提示词（会话开好了、这一轮没在跑时）
    pub(crate) fn ask(&mut self, text: &str) -> Vec<Value> {
        if !self.ready || (self.prompting && self.outcome.is_none()) {
            return vec![];
        }
        self.outcome = None;
        self.send_prompt(text)
    }

    /// 对话：这一轮在跑
    pub(crate) fn busy(&self) -> bool {
        self.prompting && self.outcome.is_none()
    }

    /// 会话开始后要切的配置（`--set`）
    pub(crate) fn with_sets(mut self, sets: Vec<(String, String)>) -> Client {
        self.sets = sets;
        self
    }

    /// 握手后、开会话前换上的供应商（`providers/set` 的参数）
    pub(crate) fn with_provider(mut self, provider: Option<Value>) -> Client {
        self.provider = provider;
        self
    }

    fn new_session(&mut self) -> Vec<Value> {
        // 对话接回会话：agent 会 `session/resume` 就用它（不回放历史），只会 `session/load` 就用它（回放的历史不记日志），
        // 都不会时开新会话、说明之前的上下文没了
        if let Some(id) = self.resume.take() {
            let params = json!({ "sessionId": id, "cwd": self.cwd, "mcpServers": [] });
            self.pending_resume = Some(id.clone());
            if self.can_resume {
                return vec![self.request("session/resume", params, Pending::Resume)];
            }
            if self.can_load {
                self.replaying = true;
                return vec![self.request("session/load", params, Pending::Load)];
            }
            self.note = Some("agent 不支持接回会话（session/resume、session/load）：开了新会话，之前的对话它不记得了".into());
        }
        let params = json!({ "cwd": self.cwd, "mcpServers": [] });
        vec![self.request("session/new", params, Pending::New)]
    }

    /// 发下一个配置请求；都发完了发 prompt
    fn step(&mut self) -> Vec<Value> {
        if self.queue.is_empty() {
            if self.chat {
                self.ready = true;
                return vec![];
            }
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

    /// 握手的第一帧：要 v2，同时带上 v1 的两个字段，只会 v1 的 agent（dsh）照样认得、回 `protocolVersion: 1`
    pub(crate) fn start(&mut self) -> Vec<Value> {
        let info = json!({ "name": "board", "title": "Board", "version": env!("CARGO_PKG_VERSION") });
        let params = json!({
            "protocolVersion": PROTOCOL,
            "info": info,
            "capabilities": {},
            "clientInfo": info,
            "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
        });
        vec![self.request("initialize", params, Pending::Init)]
    }

    /// 收尾：v2 发 `session/close`（停掉会话里还在跑的工作，包括后台的），不等回应；v1 不发
    pub(crate) fn close(&mut self) -> Vec<Value> {
        match (&self.session, self.proto) {
            (Some(s), Proto::V2) => {
                let params = json!({ "sessionId": s });
                vec![self.request("session/close", params, Pending::Close)]
            }
            _ => vec![],
        }
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
            // 通知：记日志；v2 的 `state_update` 还决定这一轮在不在干活
            (Some(method), None) => {
                if method == "session/update" {
                    self.on_usage(&v["params"]);
                    if self.proto == Proto::V2 {
                        self.on_update(&v["params"]);
                    }
                }
                vec![]
            }
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
                // 收尾时发的，不等也不管
                Pending::Close => {}
                Pending::Init => self.outcome = Some(Outcome::Failed(format!("initialize 失败：{msg}"))),
                Pending::Provider => self.outcome = Some(Outcome::Failed(format!("换供应商（providers/set）失败：{msg}"))),
                Pending::New if e["code"].as_i64() == Some(-32000) => self.outcome = Some(Outcome::Failed(format!("agent 要求先登录：{msg}"))),
                Pending::New => self.outcome = Some(Outcome::Failed(format!("session/new 失败：{msg}"))),
                // 接不回（会话已经没了、agent 记不住）：开新会话，说明之前的上下文没了
                Pending::Resume | Pending::Load => {
                    self.replaying = false;
                    self.note = Some(format!("接不回之前的会话（{msg}）：开了新会话，之前的对话它不记得了"));
                    return self.new_session();
                }
                Pending::Prompt => self.outcome = Some(self.turn_error(e, format!("session/prompt 失败：{msg}"))),
            }
            return vec![];
        }
        match what {
            Pending::Init => {
                // 版本协商：agent 回它能讲的、不高于我们要的最新版本
                self.proto = if r["protocolVersion"].as_i64().unwrap_or(1) >= 2 { Proto::V2 } else { Proto::V1 };
                // 能力：v1 在 agentCapabilities，v2 在 capabilities
                let caps = if r["agentCapabilities"].is_object() { &r["agentCapabilities"] } else { &r["capabilities"] };
                self.can_load = caps["loadSession"].as_bool().unwrap_or(false);
                self.can_resume = ["sessionCapabilities", "session", "sessions"].iter().any(|k| caps[*k]["resume"].is_object() || caps[*k]["resume"].as_bool() == Some(true));
                match self.provider.take() {
                    Some(p) => vec![self.request("providers/set", p, Pending::Provider)],
                    None => self.new_session(),
                }
            }
            Pending::Provider => self.new_session(),
            Pending::Resume | Pending::Load => {
                self.replaying = false;
                let id = v["result"]["sessionId"].as_str().map(str::to_string).or_else(|| self.pending_resume.take()).unwrap_or_default();
                self.session = Some(id.clone());
                self.note.get_or_insert_with(|| format!("接回了之前的会话（{}）", if what == Pending::Resume { "session/resume" } else { "session/load" }));
                let proto = self.proto;
                let mut queue: Vec<(&'static str, Value)> = self.sets.iter().filter_map(|(k, val)| config_set(r, &id, k, val, proto)).collect();
                queue.extend(self.mode.clone().and_then(|m| mode_switch(r, &id, &m, proto)));
                self.queue = queue;
                self.step()
            }
            Pending::New => {
                let Some(s) = r["sessionId"].as_str().filter(|s| !s.is_empty()) else {
                    self.outcome = Some(Outcome::Failed("session/new 的回应里没有 sessionId".into()));
                    return vec![];
                };
                self.session = Some(s.to_string());
                let proto = self.proto;
                let mut queue: Vec<(&'static str, Value)> = self.sets.iter().filter_map(|(id, v)| config_set(r, s, id, v, proto)).collect();
                queue.extend(self.mode.clone().and_then(|m| mode_switch(r, s, &m, proto)));
                self.queue = queue;
                self.step()
            }
            Pending::Mode => self.step(),
            Pending::Close => vec![],
            // v1 的回应跨整轮，带着这一轮的结束原因；v2 的回应只表示提示词收下了，这一轮的结束看 `state_update`
            Pending::Prompt if self.proto == Proto::V2 => vec![],
            Pending::Prompt => {
                let reason = r["stopReason"].as_str().unwrap_or("end_turn").to_string();
                self.outcome = Some(Outcome::Stopped(reason));
                vec![]
            }
        }
    }

    /// v2 的 `session/update`：这个会话的 `state_update` 决定这一轮在不在干活（子会话的、别的更新只记日志）。
    /// `running`、`requires_action` 和认不出的状态都算在干活（规范：认不出的状态不能当 `idle`）；`idle` 带 `stopReason`
    /// 是这一轮做完了，不带的是 agent 重报一次（例如后台列表变了），不算新的结束
    fn on_update(&mut self, params: &Value) {
        let u = &params["update"];
        if u["sessionUpdate"].as_str() != Some("state_update") || params["sessionId"].as_str() != self.session.as_deref() {
            return;
        }
        self.background = u["background"].as_array().map(Vec::len);
        if !self.prompting {
            return;
        }
        match u["state"].as_str() {
            Some("idle") => match u["stopReason"].as_str() {
                Some("error") => self.outcome = Some(self.turn_error(&u["error"], format!("这一轮出错：{}", error_text(&u["error"])))),
                Some(reason) => self.outcome = Some(Outcome::Stopped(reason.to_string())),
                None => {}
            },
            _ => self.outcome = None,
        }
    }

    /// `usage_update`：Claude 在 `_meta["_claude/rateLimit"]` 里带额度（#668），记下最近一次的。子会话报的也算（同一个账号）
    fn on_usage(&mut self, params: &Value) {
        let u = &params["update"];
        if u["sessionUpdate"].as_str() != Some("usage_update") {
            return;
        }
        if let Some(q) = rate_of(&u["_meta"]["_claude/rateLimit"]) {
            self.quota = Some(q);
        }
    }

    /// agent 回的这一轮出错（`error` 是 ACP 的错误对象）：额度用完时是 [`Outcome::Quota`]，[`Client::quota`] 记上用完和
    /// 重置时间；别的照旧是 [`Outcome::Failed`]
    fn turn_error(&mut self, error: &Value, why: String) -> Outcome {
        match quota_of(error, self.quota.as_ref(), chrono::Local::now()) {
            Some(q) => {
                self.quota = Some(q);
                Outcome::Quota(why)
            }
            None => Outcome::Failed(why),
        }
    }
}

/* ------------------------------ 额度（#668） ------------------------------ */

/// 额度用完的报错文字以这些开头：Claude Agent SDK 的 `USAGE_LIMIT_ERROR_PREFIXES`（0.3.331），Codex 的「You've hit your
/// usage limit」也以第一个开头
const USAGE_LIMIT_PREFIXES: &[&str] = &[
    "You've hit your",
    "You've reached your",
    "You're out of usage credits",
    "Your org is out of usage · add funds to continue",
    "Your org is out of usage · contact your admin",
    "Your seat type doesn't include usage credits",
    "Your seat type doesn't include usage",
    "Your usage allocation has been disabled by your admin",
    "Your group's usage limit is set to $0",
    "Fable 5 requires usage credits",
    "You're out of extra usage",
    "Your seat type doesn't include extra usage",
];

/// Agent SDK 的 `rate_limit_info`（`{status, resetsAt, rateLimitType, utilization, …}`）→ 额度；不是对象、没有状态时为 None。
/// `resetsAt` 是 Unix 秒（看着像毫秒的照用），`utilization` 是 0–1 的比例（大于 1 的当百分数）
fn rate_of(v: &Value) -> Option<Quota> {
    let status = v["status"].as_str().filter(|s| !s.is_empty())?;
    let resets_at = v["resetsAt"].as_f64().filter(|t| *t > 0.0).map(|t| if t < 1e11 { (t * 1000.0) as i64 } else { t as i64 });
    let used = v["utilization"].as_f64().filter(|u| u.is_finite() && *u >= 0.0).map(|u| if u <= 1.0 { (u * 100.0).round() as i64 } else { u.round() as i64 });
    Some(Quota {
        status: status.to_string(),
        window: v["rateLimitType"].as_str().unwrap_or_default().to_string(),
        used,
        resets_at,
        ..Default::default()
    })
}

/// 这一轮的错误是不是额度用完；是的话返回记上用完、重置时间的额度（`seen` 是之前 `usage_update` 带的）。
/// 认的是：Claude 的 `data.errorKind` 为 `rate_limit` 且额度状态是 `rejected`（一时的 429 是 `allowed`，不算）；
/// Codex 的 `data.codexErrorInfo` 为 `usageLimitExceeded`（Zed 版：`codex_error_info` 为 `usage_limit_exceeded`）；
/// 或者报错文字以 [`USAGE_LIMIT_PREFIXES`] 开头。
/// 重置时间先用额度信息里的，没有时从「Try again at …」里读（Codex，本机时区）
fn quota_of(error: &Value, seen: Option<&Quota>, now: chrono::DateTime<chrono::Local>) -> Option<Quota> {
    let data = &error["data"];
    let texts: Vec<&str> = [error["message"].as_str(), data["message"].as_str(), data.as_str()].into_iter().flatten().map(str::trim).filter(|t| !t.is_empty()).collect();
    let rate = rate_of(&data["rateLimit"]).or_else(|| seen.cloned());
    let rejected = rate.as_ref().is_some_and(|r| r.status == "rejected");
    let claude = data["errorKind"].as_str() == Some("rate_limit") && rejected;
    let codex = data["codexErrorInfo"].as_str() == Some("usageLimitExceeded") || data["codex_error_info"].as_str() == Some("usage_limit_exceeded");
    let words = texts.iter().any(|t| USAGE_LIMIT_PREFIXES.iter().any(|p| t.starts_with(p)));
    if !(claude || codex || words) {
        return None;
    }
    // 给人看的原文：最长的那段（Claude 是 message，Codex 是 data.message；message 有时只是「Internal error」）
    let text = texts.iter().max_by_key(|t| t.chars().count()).map(|t| t.to_string()).unwrap_or_default();
    let mut q = rate.filter(|_| rejected).unwrap_or_default();
    q.exhausted = true;
    if q.resets_at.is_none() {
        q.resets_at = texts.iter().find_map(|t| try_again_at(t, now));
    }
    q.text = text.chars().take(300).collect();
    Some(q)
}

/// Codex 的「… Try again at 3:15 PM.」「… or try again at Nov 26th, 2026 3:15 PM.」→ 毫秒时间戳（本机时区，和 Codex
/// 写的时候一样）。只有时刻的是今天的，已经过了就当明天的
fn try_again_at(text: &str, now: chrono::DateTime<chrono::Local>) -> Option<i64> {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
    let lower = text.to_ascii_lowercase();
    let at = lower.find("try again at ")? + "try again at ".len();
    let rest = text.get(at..)?.trim();
    let rest = rest.split_once(". ").map_or(rest, |(a, _)| a).trim_end_matches('.').trim();
    let local = |n: NaiveDateTime| chrono::Local.from_local_datetime(&n).earliest().map(|t| t.timestamp_millis());
    if let Ok(t) = NaiveTime::parse_from_str(rest, "%I:%M %p") {
        let today = now.date_naive().and_time(t);
        let ms = local(today)?;
        return Some(if ms < now.timestamp_millis() { ms + 24 * 3600 * 1000 } else { ms });
    }
    // 「Nov 26th, 2026 3:15 PM」：去掉日子后面的 st / nd / rd / th
    let (date, time) = rest.split_once(", ").and_then(|(md, yt)| yt.split_once(' ').map(|(y, t)| (format!("{} {y}", md.trim_end_matches(char::is_alphabetic)), t.to_string())))?;
    let d = NaiveDate::parse_from_str(&date, "%b %d %Y").ok()?;
    let t = NaiveTime::parse_from_str(&time, "%I:%M %p").ok()?;
    local(d.and_time(t))
}

/// 毫秒时间戳 → 本机时间「11-26 15:15」（写进日志给人看）
fn local_words(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).earliest().map(|t| t.format("%m-%d %H:%M").to_string()).unwrap_or_default()
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
/// 否则（只有 v1）公布的模式里有它时用 `session/set_mode`；都没有时不切
fn mode_switch(r: &Value, session: &str, mode: &str, proto: Proto) -> Option<(&'static str, Value)> {
    for o in r["configOptions"].as_array().into_iter().flatten() {
        let is_mode = option_id(o) == Some("mode") || o["category"].as_str() == Some("mode");
        if is_mode && option_values(&o["options"]).iter().any(|v| v == mode) {
            return Some(("session/set_config_option", set_option(session, option_id(o).unwrap_or("mode"), mode, proto)));
        }
    }
    let modes = r["modes"]["availableModes"].as_array().into_iter().flatten();
    if proto == Proto::V1 && modes.into_iter().any(|m| m["id"].as_str() == Some(mode)) {
        return Some(("session/set_mode", json!({ "sessionId": session, "modeId": mode })));
    }
    None
}

/// 会话配置里有 id 这一项、而且有这个值时：`session/set_config_option`。id 写成 `@<类别>` 时按配置项的类别找
/// （`@model`、`@thought_level`；找不到类别时 `@thought_level` 再按 dsh 的 `reasoning_effort` 找）
fn config_set(r: &Value, session: &str, id: &str, value: &str, proto: Proto) -> Option<(&'static str, Value)> {
    let list = r["configOptions"].as_array()?;
    let o = match id.strip_prefix('@') {
        Some(cat) => list
            .iter()
            .find(|o| o["category"].as_str() == Some(cat))
            .or_else(|| (cat == "thought_level").then(|| list.iter().find(|o| option_id(o) == Some("reasoning_effort"))).flatten())?,
        None => list.iter().find(|o| option_id(o) == Some(id))?,
    };
    let id = option_id(o)?;
    let offered = option_values(&o["options"]).iter().any(|v| v == value);
    offered.then(|| ("session/set_config_option", set_option(session, id, value, proto)))
}

/// 配置项的标识：v2 叫 `configId`，v1 叫 `id`
fn option_id(o: &Value) -> Option<&str> {
    o["configId"].as_str().or_else(|| o["id"].as_str())
}

/// `session/set_config_option` 的参数：v2 要写明值的类型（下拉选项是 `id`）
fn set_option(session: &str, id: &str, value: &str, proto: Proto) -> Value {
    match proto {
        Proto::V1 => json!({ "sessionId": session, "configId": id, "value": value }),
        Proto::V2 => json!({ "sessionId": session, "configId": id, "type": "id", "value": value }),
    }
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
        Outcome::Quota(_) => EXIT_QUOTA,
    }
}

/* ------------------------------ 日志：合并消息片段 ------------------------------ */

/// 连续的同一种消息片段合成一帧。每个会话各攒各的（子 agent 的 sessionId 形如
/// `<主会话>:subagent:<uuid>`）：子 agent 的片段和主 agent 交错时，不把主 agent 的话冲散。
#[derive(Default)]
pub(crate) struct Merger {
    /// （sessionId、片段种类、攒的文字、第一段的时间）；一个会话同时只有一种片段，按开始攒的先后排
    bufs: Vec<(Value, String, String, Instant)>,
}

impl Merger {
    /// 一帧 agent 发来的：返回现在要写进日志的行（片段先攒着）
    pub(crate) fn push(&mut self, v: &Value) -> Vec<String> {
        if let Some((kind, session, text)) = chunk(v) {
            if let Some(i) = self.bufs.iter().position(|(s, _, _, _)| *s == session) {
                if self.bufs[i].1 == kind {
                    self.bufs[i].2.push_str(&text);
                    return vec![];
                }
                // 同一个会话换了种类：先把它之前那段落下来，保持这个会话里的先后
                let line = frame(self.bufs.remove(i));
                self.bufs.push((session, kind, text, Instant::now()));
                return vec![line];
            }
            self.bufs.push((session, kind, text, Instant::now()));
            return vec![];
        }
        // 不是片段：这个会话攒的话先落，再写这一帧；没有 sessionId 的全局帧先落全部
        let session = &v["params"]["sessionId"];
        let mut out = if session.is_null() { self.flush() } else { self.take_session(session) };
        out.push(v.to_string());
        out
    }

    /// 有片段已经攒够 CHUNK_FLUSH
    pub(crate) fn due(&self) -> bool {
        self.bufs.iter().any(|(_, _, _, at)| at.elapsed() >= CHUNK_FLUSH)
    }

    /// 攒够 CHUNK_FLUSH 的片段落下来（按开始攒的先后）
    pub(crate) fn flush_due(&mut self) -> Vec<String> {
        let mut due = vec![];
        let mut keep = vec![];
        for buf in std::mem::take(&mut self.bufs) {
            if buf.3.elapsed() >= CHUNK_FLUSH {
                due.push(frame(buf));
            } else {
                keep.push(buf);
            }
        }
        self.bufs = keep;
        due
    }

    /// 全部落下来（退出、写非协议行之前）
    pub(crate) fn flush(&mut self) -> Vec<String> {
        std::mem::take(&mut self.bufs).into_iter().map(frame).collect()
    }

    /// 先落某个会话攒的片段
    fn take_session(&mut self, session: &Value) -> Vec<String> {
        let mut hit = vec![];
        let mut keep = vec![];
        for buf in std::mem::take(&mut self.bufs) {
            if buf.0 == *session {
                hit.push(frame(buf));
            } else {
                keep.push(buf);
            }
        }
        self.bufs = keep;
        hit
    }
}

/// 一个片段攒成的日志行（和 agent 发来的 `session/update` 同形）
fn frame((session, kind, text, _): (Value, String, String, Instant)) -> String {
    let update = json!({ "sessionUpdate": kind, "content": { "type": "text", "text": text } });
    json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": update } }).to_string()
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
    let code = match o.chat.clone() {
        Some(state) => run_chat(&o, &state),
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

/// 写一行日志（标准输出）
fn log(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// 发一帧给 agent，也写进日志
/// 写进日志的样子：`providers/set` 的请求头（里面是 API key）换成 `***`，别的原样
fn loggable(f: &Value) -> String {
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
    // 角色的供应商：读完就删（里面有 key），读不了、格式不对就不往下走
    let provider = match &o.provider_file {
        None => None,
        Some(f) => {
            let got = std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
            let _ = std::fs::remove_file(f);
            match got {
                Ok(v) => Some(v),
                Err(e) => {
                    eprintln!("✗ board acp：读不了供应商设置 {}：{e}", f.display());
                    let _ = child.kill();
                    return EXIT_PROTOCOL;
                }
            }
        }
    };
    let cwd = o.cwd.display().to_string();
    let mut client = Client::new(&cwd, &prompt, o.mode.as_deref()).with_sets(o.sets.clone()).with_provider(provider);
    let mut merger = Merger::default();
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
            for l in merger.flush_due() {
                log(&l);
            }
        }
        match rx.recv_timeout(TICK) {
            Ok(Some(line)) => {
                last_frame = Instant::now();
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
    let status = wait(&mut child, GRACE);
    if let (Some(f), Some(q)) = (&o.quota_file, &client.quota) {
        if let Err(e) = std::fs::write(f, serde_json::to_string(q).unwrap_or_default()) {
            eprintln!("✗ board acp：写不了额度文件 {}：{e}", f.display());
        }
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
    let provider = match &o.provider_file {
        None => None,
        Some(f) => {
            let got = std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
            let _ = std::fs::remove_file(f);
            match got {
                Ok(v) => Some(v),
                Err(e) => {
                    eprintln!("✗ board acp：读不了供应商设置 {}：{e}", f.display());
                    let _ = child.kill();
                    return EXIT_PROTOCOL;
                }
            }
        }
    };
    let cwd = o.cwd.display().to_string();
    let mut client = Client::new(&cwd, "", o.mode.as_deref()).with_sets(o.sets.clone()).with_provider(provider).for_chat(o.resume.clone());
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
                let v = if t.starts_with('{') { serde_json::from_str::<Value>(t).ok().filter(Value::is_object) } else { None };
                match v {
                    Some(v) => {
                        // session/load 回放的历史之前已经记过：不再记日志
                        let replay = client.replaying && v["method"].as_str() == Some("session/update");
                        if !replay {
                            for l in merger.push(&v) {
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
    wait(&mut child, GRACE);
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
        let o = parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--extra-turns", "3", "--provider-file", "/r", "--", "x"])).unwrap();
        assert_eq!((o.extra_turns, o.prog.as_str(), o.provider_file), (3, "x", Some(PathBuf::from("/r"))));
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
        assert!(t.contains("在这一轮里用工具等它跑完") && t.ends_with("（第 1 次续轮，最多 3 次）"), "{t}");
    }

    fn reply(id: i64, result: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    }

    /// 走一遍 v1（agent 只会 v1，例如 dsh）：initialize → session/new → 切模式 → prompt → end_turn
    #[test]
    fn handshake_mode_and_prompt() {
        let mut c = Client::new("/w", "/board-skills 开始执行", Some("bypassPermissions"));
        let f = c.start();
        // 要 v2，同时带上 v1 的字段：只会 v1 的 agent 照样认得
        assert_eq!((f[0]["method"].as_str(), f[0]["id"].as_i64(), f[0]["params"]["protocolVersion"].as_i64()), (Some("initialize"), Some(0), Some(2)));
        assert_eq!((f[0]["params"]["info"]["name"].as_str(), f[0]["params"]["clientInfo"]["name"].as_str()), (Some("board"), Some("board")));
        assert_eq!((&f[0]["params"]["capabilities"], &f[0]["params"]["clientCapabilities"]["terminal"]), (&json!({}), &json!(false)));
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 1, "agentCapabilities": {} })));
        assert_eq!(c.proto, Proto::V1, "agent 回 1：按 v1 走");
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
        assert!(c.close().is_empty(), "v1 收尾不发 session/close：关标准输入就行");
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

    /// 对话：agent 会 session/resume 就接回（不回放）；会话开好后不发提示词，等 ask；人写的消息原样发出去；
    /// 一轮结束后同一个会话里接着发
    #[test]
    fn chat_resumes_and_sends_messages_as_is() {
        let mut c = Client::new("/home/me", "", None).for_chat(Some("s1".into()));
        c.start();
        let caps = json!({ "loadSession": true, "sessionCapabilities": { "resume": {}, "close": {} } });
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 1, "agentCapabilities": caps })));
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["sessionId"].as_str(), f[0]["params"]["cwd"].as_str()), (Some("session/resume"), Some("s1"), Some("/home/me")));
        assert!(!c.replaying, "resume 不回放历史");
        assert!(c.ask("早").is_empty(), "会话还没接回：不发");
        let f = c.on_frame(&reply(1, json!({})));
        assert!(f.is_empty(), "接回后不自己发提示词");
        assert!(c.ready && !c.busy());
        assert_eq!(c.session(), Some("s1"));
        assert!(c.note.as_deref().unwrap().contains("接回"));
        let f = c.ask("看看主目录下哪些文件夹最大");
        assert_eq!(f[0]["method"], "session/prompt");
        assert_eq!(f[0]["params"]["prompt"], json!([{ "type": "text", "text": "看看主目录下哪些文件夹最大" }]), "原样，前后不加说明");
        assert!(c.busy());
        assert!(c.ask("第二条").is_empty(), "这一轮没结束：不发");
        c.on_frame(&reply(2, json!({ "stopReason": "end_turn" })));
        assert!(!c.busy());
        let f = c.ask("清一下 ~/.cache");
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["sessionId"].as_str()), (Some("session/prompt"), Some("s1")));
        // 停止这一轮
        assert_eq!(c.cancel()[0]["method"], "session/cancel");
        c.on_frame(&reply(3, json!({ "stopReason": "cancelled" })));
        assert_eq!(c.outcome, Some(Outcome::Stopped("cancelled".into())));
        assert!(!c.busy());
    }

    /// 对话：只会 session/load 时用它（回放的历史不记日志）；都不会、接不回时开新会话，说明之前的对话没了
    #[test]
    fn chat_falls_back_to_load_then_new_session() {
        let mut c = Client::new("/home/me", "", None).for_chat(Some("s1".into()));
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 1, "agentCapabilities": { "loadSession": true } })));
        assert_eq!(f[0]["method"], "session/load");
        assert!(c.replaying);
        let f = c.on_frame(&json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32002, "message": "没有这个会话" } }));
        assert!(!c.replaying);
        assert_eq!(f[0]["method"], "session/new", "接不回：开新会话");
        assert!(c.note.as_deref().unwrap().contains("接不回"));
        c.on_frame(&reply(2, json!({ "sessionId": "s2" })));
        assert!(c.ready);
        assert_eq!(c.session(), Some("s2"));

        let mut c = Client::new("/home/me", "", None).for_chat(Some("s1".into()));
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 1, "agentCapabilities": {} })));
        assert_eq!(f[0]["method"], "session/new");
        assert!(c.note.as_deref().unwrap().contains("不支持接回"));
        // 第一次（没有要接回的）：直接开新会话，不写说明
        let mut c = Client::new("/home/me", "", None).for_chat(None);
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 2, "capabilities": {} })));
        assert_eq!(f[0]["method"], "session/new");
        assert!(c.note.is_none());
    }

    fn state(session: &str, update: Value) -> Value {
        let mut u = update;
        u["sessionUpdate"] = json!("state_update");
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": u } })
    }

    /// 走一遍 v2（Claude Code、Codex）：配置项用 configId、切的时候带 type；prompt 的回应只是收下了，
    /// 这一轮的进展看 state_update；后台列表跟着记；收尾发 session/close
    #[test]
    fn v2_turns_follow_state_updates() {
        let mut c = Client::new("/w", "/board-skills 开始执行", Some("bypassPermissions"));
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 2, "info": { "name": "claude-agent-acp" }, "capabilities": {} })));
        assert_eq!((c.proto, f[0]["method"].as_str()), (Proto::V2, Some("session/new")));
        let opts = json!([{ "configId": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "auto", "options": [{ "value": "auto", "name": "Auto" }, { "value": "bypassPermissions", "name": "Bypass" }] }]);
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s1", "configOptions": opts })));
        assert_eq!(f[0]["params"], json!({ "sessionId": "s1", "configId": "mode", "type": "id", "value": "bypassPermissions" }));
        // 还没发 prompt：会话开始时报的状态不算这一轮
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "end_turn" })));
        assert_eq!(c.outcome, None);
        let f = c.on_frame(&reply(2, json!({ "configOptions": opts })));
        assert_eq!(f[0]["method"], "session/prompt");
        assert!(c.on_frame(&reply(3, json!({ "messageId": "m1" }))).is_empty());
        assert_eq!(c.outcome, None, "v2 的 prompt 回应只是收下了");
        c.on_frame(&state("s1", json!({ "state": "running" })));
        assert_eq!(c.outcome, None);
        assert_eq!(c.cancel()[0]["method"], "session/cancel", "在干活：取消有效");
        // 别的会话（子 agent）的状态不算
        c.on_frame(&state("child", json!({ "state": "idle", "stopReason": "end_turn" })));
        assert_eq!(c.outcome, None);
        // 这一轮做完了，后台还有一个终端在跑
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "end_turn", "background": [{ "type": "terminal", "terminalId": "t1" }], "usage": { "inputTokens": 5 } })));
        assert_eq!((c.outcome.clone(), c.background), (Some(Outcome::Stopped("end_turn".into())), Some(1)));
        // 后台跑完：agent 重报一次 idle（不带 stopReason），不是新的结束；结束原因照旧
        c.on_frame(&state("s1", json!({ "state": "idle", "background": [] })));
        assert_eq!((c.outcome.clone(), c.background), (Some(Outcome::Stopped("end_turn".into())), Some(0)));
        // agent 自己接着做（处理跑完的后台任务）：又在干活
        c.on_frame(&state("s1", json!({ "state": "running", "background": [] })));
        assert_eq!(c.outcome, None);
        // 认不出的状态也算在干活
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "end_turn" })));
        c.on_frame(&state("s1", json!({ "state": "_compacting" })));
        assert_eq!(c.outcome, None);
        // 这一轮出错：退出码 2，原因写明
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": { "code": -32603, "message": "rate limited" } })));
        assert_eq!(c.outcome, Some(Outcome::Failed("这一轮出错：rate limited".into())));
        assert_eq!(exit_for(c.outcome.as_ref().unwrap()), 2);
        // 不报后台：None（不知道），不是 0
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "end_turn" })));
        assert_eq!(c.background, None);
        let f = c.close();
        assert_eq!((f[0]["method"].as_str(), &f[0]["params"]), (Some("session/close"), &json!({ "sessionId": "s1" })));
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

    /// --set：会话配置里有这一项、这个值时先切它，再切模式，最后 prompt；没有的跳过
    /// 角色用供应商：握手后先 `providers/set`（这一个进程换上它），再开会话；模型、强度按配置项的类别切。日志里的 key 遮住
    #[test]
    fn provider_is_set_before_the_session_and_sets_go_by_category() {
        let provider = json!({ "providerId": "main", "apiType": "anthropic", "baseUrl": "https://relay.example", "headers": { "Authorization": "Bearer sk-1" } });
        let mut c = Client::new("/w", "p", None).with_sets(vec![("@model".into(), "model-a".into()), ("@thought_level".into(), "high".into())]).with_provider(Some(provider.clone()));
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 2 })));
        assert_eq!((f[0]["method"].as_str(), &f[0]["params"]), (Some("providers/set"), &provider));
        assert!(loggable(&f[0]).contains("\"Authorization\":\"***\"") && !loggable(&f[0]).contains("sk-1"), "{}", loggable(&f[0]));
        let f = c.on_frame(&reply(1, json!({})));
        assert_eq!(f[0]["method"], "session/new");
        let opts = json!([
            { "configId": "model", "category": "model", "options": [{ "value": "model-a" }, { "value": "model-b" }] },
            { "configId": "effort", "category": "thought_level", "options": [{ "value": "low" }, { "value": "high" }] }
        ]);
        let f = c.on_frame(&reply(2, json!({ "sessionId": "s", "configOptions": opts })));
        assert_eq!(f[0]["params"], json!({ "sessionId": "s", "configId": "model", "type": "id", "value": "model-a" }));
        let f = c.on_frame(&reply(3, json!({})));
        assert_eq!(f[0]["params"]["configId"], "effort");
        // providers/set 失败：不开会话
        let mut c = Client::new("/w", "p", None).with_provider(Some(json!({})));
        c.start();
        c.on_frame(&reply(0, json!({ "protocolVersion": 2 })));
        assert!(c.on_frame(&json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32603, "message": "Model discovery failed" } })).is_empty());
        assert!(matches!(&c.outcome, Some(Outcome::Failed(m)) if m.contains("换供应商")));
        // dsh 的强度没有类别：按 reasoning_effort 找
        let mut c = Client::new("/w", "p", None).with_sets(vec![("@thought_level".into(), "max".into())]);
        c.start();
        c.on_frame(&reply(0, json!({})));
        let f = c.on_frame(&reply(1, json!({ "sessionId": "s", "configOptions": [{ "id": "reasoning_effort", "options": [{ "value": "max" }] }] })));
        assert_eq!(f[0]["params"]["configId"], "reasoning_effort");
    }

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
        assert_eq!(mode_switch(&r, "s", "agent-full-access", Proto::V1), Some(("session/set_mode", json!({ "sessionId": "s", "modeId": "agent-full-access" }))));
        assert_eq!(mode_switch(&r, "s", "bypassPermissions", Proto::V1), None, "没公布这个模式：不切");
        assert_eq!(mode_switch(&r, "s", "agent-full-access", Proto::V2), None, "v2 没有 session/set_mode：只看配置项");
        // 分组的选项也认得；v2 的配置项叫 configId
        let r = json!({ "configOptions": [{ "id": "perm", "category": "mode", "options": [{ "group": "g", "options": [{ "value": "yolo" }] }] }] });
        assert_eq!(mode_switch(&r, "s", "yolo", Proto::V1).map(|x| x.1["configId"].clone()), Some(json!("perm")));
        let r2 = json!({ "configOptions": [{ "configId": "perm", "category": "mode", "options": [{ "value": "yolo" }] }] });
        assert_eq!(mode_switch(&r2, "s", "yolo", Proto::V2).map(|x| x.1), Some(json!({ "sessionId": "s", "configId": "perm", "type": "id", "value": "yolo" })));
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

    fn usage(session: &str, rate: Value) -> Value {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": { "sessionUpdate": "usage_update", "used": 10, "size": 200000, "_meta": { "_claude/rateLimit": rate } } } })
    }

    /// 走到这一轮开始干活：v2（Claude Code）或 v1（Codex）
    fn working(v2: bool) -> Client {
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&reply(0, json!({ "protocolVersion": if v2 { 2 } else { 1 } })));
        c.on_frame(&reply(1, json!({ "sessionId": "s1" })));
        if v2 {
            c.on_frame(&reply(2, json!({ "messageId": "m1" })));
            c.on_frame(&state("s1", json!({ "state": "running" })));
        }
        c
    }

    /// 额度（#668）：Claude 跑的时候 usage_update 带的额度记下来；额度用完（errorKind rate_limit、状态 rejected）退出码 4，
    /// 带上重置时间；一时的 429（状态 allowed）照旧算出错
    #[test]
    fn claude_usage_limit_is_its_own_outcome() {
        let mut c = working(true);
        c.on_frame(&usage("s1", json!({ "status": "allowed_warning", "resetsAt": 1795000000, "rateLimitType": "five_hour", "utilization": 0.91 })));
        let q = c.quota.clone().unwrap();
        assert_eq!((q.status.as_str(), q.window.as_str(), q.used, q.resets_at, q.exhausted), ("allowed_warning", "five_hour", Some(91), Some(1_795_000_000_000), false));
        let rate = json!({ "status": "rejected", "resetsAt": 1795003600, "rateLimitType": "five_hour", "utilization": 1.0 });
        let err = json!({ "code": -32603, "message": "You've hit your session limit · resets 3pm", "data": { "errorKind": "rate_limit", "rateLimit": rate } });
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": err })));
        assert!(matches!(c.outcome, Some(Outcome::Quota(ref w)) if w.contains("You've hit your session limit")), "{:?}", c.outcome);
        assert_eq!(exit_for(c.outcome.as_ref().unwrap()), EXIT_QUOTA);
        let q = c.quota.clone().unwrap();
        assert_eq!((q.exhausted, q.resets_at, q.used, q.text.as_str()), (true, Some(1_795_003_600_000), Some(100), "You've hit your session limit · resets 3pm"));
        // 一时的 429：状态 allowed，文字也不是额度用完
        let mut c = working(true);
        let err = json!({ "code": -32603, "message": "API Error: 429 overloaded", "data": { "errorKind": "rate_limit", "rateLimit": { "status": "allowed" } } });
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": err })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(_))), "{:?}", c.outcome);
        assert_eq!(c.quota, None);
        // 没带 data、只有文字（旧版适配器）：照样认；重置时间用之前 usage_update 报的 rejected
        let mut c = working(true);
        c.on_frame(&usage("s1", json!({ "status": "rejected", "resetsAt": 1795007200 })));
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": { "code": -32603, "message": "You've reached your weekly limit" } })));
        assert!(matches!(c.outcome, Some(Outcome::Quota(_))));
        assert_eq!(c.quota.as_ref().and_then(|q| q.resets_at), Some(1_795_007_200_000));
    }

    /// Codex：Zed 版（v1）session/prompt 回错、data 里 codex_error_info；codex-acp 2.7.0（v2）这一轮以 error 结束、data 里
    /// codexErrorInfo。重置时间从「try again at」读
    #[test]
    fn codex_usage_limit_reads_the_reset_time_from_the_text() {
        let mut c = working(false);
        let text = "You've hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at Nov 26th, 2099 3:15 PM.";
        let err = json!({ "code": -32603, "message": "Internal error", "data": { "message": text, "codex_error_info": "usage_limit_exceeded" } });
        c.on_frame(&json!({ "jsonrpc": "2.0", "id": 2, "error": err }));
        assert!(matches!(c.outcome, Some(Outcome::Quota(ref w)) if w.starts_with("session/prompt 失败：Internal error")), "{:?}", c.outcome);
        let q = c.quota.clone().unwrap();
        let want = chrono::Local.with_ymd_and_hms(2099, 11, 26, 15, 15, 0).unwrap().timestamp_millis();
        assert_eq!((q.exhausted, q.resets_at, q.text.as_str(), q.status.as_str()), (true, Some(want), text, ""));
        // codex-acp 2.7.0（v2）：这一轮以 error 结束，data 里是 codexErrorInfo
        let mut c = working(true);
        let err = json!({ "code": -32603, "message": "Internal error", "data": { "message": "You've hit your usage limit. Try again later.", "codexErrorInfo": "usageLimitExceeded" } });
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": err })));
        assert!(matches!(c.outcome, Some(Outcome::Quota(_))), "{:?}", c.outcome);
        assert_eq!(c.quota.as_ref().map(|q| (q.exhausted, q.resets_at)), Some((true, None)));
        let mut c = working(true);
        let err = json!({ "code": -32603, "message": "Internal error", "data": { "message": "rate limited, retrying failed", "codexErrorInfo": "rateLimitExceeded" } });
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": err })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(_))), "一时的限流不算：{:?}", c.outcome);
        // 别的出错不算
        let mut c = working(false);
        c.on_frame(&json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": -32603, "message": "Internal error", "data": { "message": "stream disconnected", "codex_error_info": "response_stream_disconnected" } } }));
        assert!(matches!(c.outcome, Some(Outcome::Failed(_))));
        assert_eq!(c.quota, None);
    }

    use chrono::TimeZone;

    #[test]
    fn try_again_at_reads_codex_times() {
        let now = chrono::Local.with_ymd_and_hms(2026, 11, 25, 16, 0, 0).unwrap();
        let at = |y, m, d, h, mi| chrono::Local.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().timestamp_millis();
        assert_eq!(try_again_at("You've hit your usage limit. Try again at 5:30 PM.", now), Some(at(2026, 11, 25, 17, 30)));
        // 只有时刻、已经过了：明天的
        assert_eq!(try_again_at("You've hit your usage limit. Try again at 9:05 AM.", now), Some(at(2026, 11, 26, 9, 5)));
        assert_eq!(try_again_at("… or try again at Dec 1st, 2026 12:00 AM.", now), Some(at(2026, 12, 1, 0, 0)));
        assert_eq!(try_again_at("… or try again at Nov 22nd, 2026 11:45 PM.", now), Some(at(2026, 11, 22, 23, 45)));
        assert_eq!(try_again_at("You've hit your usage limit. Try again later.", now), None);
        assert_eq!(try_again_at("boom", now), None);
        // 没有重置时间：额度用完，时间为空（服务端按 1 小时算）
        let e = json!({ "message": "Internal error", "data": { "message": "You've hit your usage limit. Try again later.", "codex_error_info": "usage_limit_exceeded" } });
        let q = quota_of(&e, None, now).unwrap();
        assert_eq!((q.exhausted, q.resets_at), (true, None));
    }

    #[test]
    fn quota_file_is_an_option() {
        let o = parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--quota-file", "/q", "--", "x"])).unwrap();
        assert_eq!(o.quota_file, Some(PathBuf::from("/q")));
        assert_eq!(opts(0).quota_file, None);
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
        assert!(m.flush().is_empty());
        assert!(!m.due());
    }

    fn piece_of(session: &str, kind: &str, text: &str) -> Value {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": { "sessionUpdate": kind, "content": { "type": "text", "text": text } } } })
    }

    /// 子 agent 的片段和主 agent 交错时，不把主 agent 的一句话拆成多段；两个子 agent 各自的话也各是一条（default/main#617）
    #[test]
    fn interleaved_subagent_chunks_do_not_split_messages() {
        let mut m = Merger::default();
        let (main, a, b) = ("main", "main:subagent:a", "main:subagent:b");
        assert!(m.push(&piece_of(main, "agent_message_chunk", "定向测试暴露的是")).is_empty());
        assert!(m.push(&piece_of(a, "agent_thought_chunk", "子 agent 在想")).is_empty());
        assert!(m.push(&piece_of(main, "agent_message_chunk", "三处测试桩")).is_empty());
        assert!(m.push(&piece_of(b, "agent_thought_chunk", "另一个在想")).is_empty());
        assert!(m.push(&piece_of(main, "agent_message_chunk", "问题而非")).is_empty());
        // 子 agent a 从「想」转到「说」：只落它自己的思考，主 agent 的话不受影响
        let one = m.push(&piece_of(a, "agent_message_chunk", "子 agent 说的话"));
        assert_eq!(one.len(), 1, "{one:#?}");
        assert!(one[0].contains("子 agent 在想"));
        // 主会话来一个工具调用：主会话攒的话先落成一条，工具调用跟上
        let tool = json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": main, "update": { "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "ls" } } });
        let out = m.push(&tool);
        assert_eq!(out.len(), 2, "主会话只有一条消息 + 工具调用：{out:#?}");
        let v: Value = serde_json::from_str(&out[0]).unwrap();
        assert_eq!(v["params"]["sessionId"], main);
        assert_eq!(v["params"]["update"]["content"]["text"], "定向测试暴露的是三处测试桩问题而非");
        assert!(out[1].contains("tool_call"));
        // 两个子 agent 各自的话各落一条，互不拼接
        let rest = m.flush();
        assert_eq!(rest.len(), 2, "{rest:#?}");
        let texts: Vec<String> = rest
            .iter()
            .map(|l| {
                let v: Value = serde_json::from_str(l).unwrap();
                assert!(v["params"]["sessionId"].as_str().unwrap().starts_with("main:subagent:"));
                v["params"]["update"]["content"]["text"].as_str().unwrap().to_string()
            })
            .collect();
        assert!(texts.iter().any(|t| t == "子 agent 说的话"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "另一个在想"), "{texts:?}");
        assert!(!texts.iter().any(|t| t.contains("定向测试暴露")), "{texts:?}");
    }
}
