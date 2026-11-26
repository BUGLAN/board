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
//! 身份（#667）：`--auth subscription`（按「官方订阅」角色派发）、`--auth only`（board host 问身份）时，`initialize` 之后、
//! 开会话之前先问 agent 自己用的身份：agent 公布了 `auth.status`（ACP Authentication State Query RFD，Claude Code）就发
//! `auth/status`；只公布了 `_meta.authStatus`（扩展：Codex）就等它推 `_auth/status_update`，最多等 [`AUTH_WAIT`]。
//! `subscription`：身份不是订阅（`kind: account`）就出错结束、写明身份，不开会话。`only`：不开会话，把身份写成一行 JSON
//! 到标准输出就退出（帧不写日志），退出码 0。`probe`（board host 上报用）：问完身份再列本机账号能用的模型（#666）：
//! `session/options`（ACP Session options RFD）给模型下拉、默认模型和默认模型的强度，再开一个不发提示词的会话，逐个
//! `session/set_config_option` 切到别的模型、读出它的强度，关掉会话；输出一行 `{"auth": …, "models": …}`。不发提示词，不耗额度。
//!
//! 模型、强度（#666）：v2 的 agent 公布了 `session.options` 时，`--set @model=…`、`--set @thought_level=…` 不再开会话后切：
//! 先 `session/options` 认出这两项的 configId，再在 `session/new` 的 `configOptions` 里带上初始值（先模型后强度）；回应里实际的值
//! 和要的不一样（账号里没有这个模型、这个模型不支持这个强度）就出错结束、写明原因，不再静默按默认跑。没公布的（v1 的 dsh、
//! 旧版适配器）照旧开会话后 `session/set_config_option`。
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
use board_core::hosts::{AgentAuth, LocalModel, LocalModels, Quota, AUTH_ACCOUNT, AUTH_NONE, AUTH_UNKNOWN};
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
/// 问身份时最多等 agent 多久（Claude Code 读 `claude auth status` 最多 5 秒；Codex 要读一次账号）
pub const AUTH_WAIT: Duration = Duration::from_secs(30);
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

const USAGE: &str = "用法：board acp --env <启动环境> [--env-prog <程序>] --cwd <目录> (--prompt-file <文件> | --chat <状态文件> [--resume <会话>] [--idle-exit <秒>] | --auth only) [--auth subscription] [--provider-file <文件>] [--mode <模式>] [--set <选项>=<值>]… [--extra-turns <次数>] [--quota-file <文件>] -- <ACP 入口命令> [参数…]";

/// 开会话前要不要问身份（`--auth`，#667）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AuthMode {
    /// 不问
    #[default]
    Skip,
    /// 「官方订阅」的派发：不是订阅就不开会话
    Subscription,
    /// 只问身份，写到标准输出就退出
    Only,
    /// 问身份、再列本机账号能用的模型和强度，写到标准输出就退出（board host 上报用，#666）
    Probe,
}

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
    /// 开会话前问身份（`--auth subscription | only`）
    pub auth: AuthMode,
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
        if !matches!(k, "--env" | "--env-prog" | "--cwd" | "--prompt-file" | "--provider-file" | "--mode" | "--set" | "--extra-turns" | "--chat" | "--resume" | "--idle-exit" | "--quota-file" | "--auth") {
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
    let auth = match m.get("--auth").map(|x| x.trim()) {
        None => AuthMode::Skip,
        Some("subscription") => AuthMode::Subscription,
        Some("only") => AuthMode::Only,
        Some("probe") => AuthMode::Probe,
        Some(x) => return Err(format!("--auth 只能是 subscription、only 或 probe：{x}")),
    };
    let idle_exit = match m.get("--idle-exit") {
        Some(v) => v.trim().parse::<u64>().map_err(|_| format!("--idle-exit 要写秒数：{v}"))?,
        None => CHAT_IDLE_EXIT,
    };
    Ok(Opts {
        env: need("--env")?,
        env_prog: m.get("--env-prog").cloned().unwrap_or_default(),
        cwd: PathBuf::from(need("--cwd")?),
        prompt_file: if chat.is_some() || matches!(auth, AuthMode::Only | AuthMode::Probe) { m.get("--prompt-file").map(PathBuf::from).unwrap_or_default() } else { PathBuf::from(need("--prompt-file")?) },
        mode: m.get("--mode").cloned().filter(|x| !x.trim().is_empty()),
        sets,
        provider_file: m.get("--provider-file").filter(|x| !x.trim().is_empty()).map(PathBuf::from),
        extra_turns,
        quota_file: m.get("--quota-file").filter(|x| !x.trim().is_empty()).map(PathBuf::from),
        auth,
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
    /// `auth/status`（#667）
    Auth,
    /// `session/options`（#666）：派发时认 configId，列模型时取模型下拉
    Options,
    /// 列模型：不发提示词的会话
    ProbeNew,
    /// 列模型：切到一个模型读它的强度
    ProbeModel,
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
    /// 开会话前问不问身份（#667）
    auth_mode: AuthMode,
    /// agent 自己用的身份：`auth/status` 的回应或推来的 `_auth/status_update`
    pub auth: Option<AgentAuth>,
    /// 在等 agent 推身份（只公布了 `_meta.authStatus` 的，或者 `auth/status` 的回应里看不出种类的）
    pub waiting_auth: bool,
    /// agent 会推 `_auth/status_update`（公布了 `_meta.authStatus`）
    auth_push: bool,
    /// agent 公布了 `session.options`（Session options RFD）：认 `session/options`、开会话时认 `configOptions` 初始值（#666）
    can_options: bool,
    /// 开会话时带上的初始值：（configId、类别、值），回应里核对
    initial: Vec<(String, String, String)>,
    /// 列模型（`--auth probe`）：结果
    pub models: Option<LocalModels>,
    /// 列模型：模型那一项的 configId、还没切的模型、正在切的模型
    probe_model_id: String,
    probe_queue: Vec<String>,
    probe_current: String,
    /// 列模型做完了（或做不了）
    pub probe_done: bool,
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
            auth_mode: AuthMode::Skip,
            auth: None,
            waiting_auth: false,
            auth_push: false,
            can_options: false,
            initial: vec![],
            models: None,
            probe_model_id: String::new(),
            probe_queue: vec![],
            probe_current: String::new(),
            probe_done: false,
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

    /// 开会话前问身份（`--auth`）
    pub(crate) fn with_auth(mut self, mode: AuthMode) -> Client {
        self.auth_mode = mode;
        self
    }

    /// 握手完、身份也问完（或不用问）之后：换供应商，或开会话
    fn after_init(&mut self) -> Vec<Value> {
        match self.provider.take() {
            Some(p) => vec![self.request("providers/set", p, Pending::Provider)],
            None => self.new_session(),
        }
    }

    /// 问到身份（或问不出来）之后：`only` 到此为止；`subscription` 不是订阅就出错结束，不开会话
    fn after_auth(&mut self) -> Vec<Value> {
        match self.auth_mode {
            AuthMode::Only => vec![],
            AuthMode::Probe => self.probe_options(),
            AuthMode::Subscription => {
                let a = self.auth.clone().unwrap_or_else(|| AgentAuth { kind: AUTH_UNKNOWN.into(), ..Default::default() });
                if a.kind == AUTH_ACCOUNT {
                    return self.after_init();
                }
                self.outcome = Some(Outcome::Failed(format!("按「官方订阅」派发，agent 用的不是订阅：{}，没有开会话", a.words())));
                vec![]
            }
            AuthMode::Skip => self.after_init(),
        }
    }

    /// 等 agent 推身份的时候过了 [`AUTH_WAIT`]：按问不出来算
    pub(crate) fn auth_timeout(&mut self) -> Vec<Value> {
        if !self.waiting_auth {
            return vec![];
        }
        self.waiting_auth = false;
        self.auth = Some(AgentAuth { kind: AUTH_UNKNOWN.into(), label: format!("{} 秒内没报身份", AUTH_WAIT.as_secs()), ..Default::default() });
        self.after_auth()
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
        // 模型、强度当开会话的初始值带上（#666）：先 session/options 认出 configId
        if self.can_options && self.proto == Proto::V2 && self.sets.iter().any(|(k, _)| k.starts_with('@')) && self.initial.is_empty() {
            return vec![self.request("session/options", json!({ "cwd": self.cwd }), Pending::Options)];
        }
        let mut params = json!({ "cwd": self.cwd, "mcpServers": [] });
        let initial: Vec<Value> = self.initial.iter().filter(|(id, _, _)| !id.is_empty()).map(|(id, _, v)| json!({ "configId": id, "type": "id", "value": v })).collect();
        if !initial.is_empty() {
            params["configOptions"] = Value::Array(initial);
        }
        vec![self.request("session/new", params, Pending::New)]
    }

    /// 列模型（#666）：先 `session/options`；agent 没公布时列不出来
    fn probe_options(&mut self) -> Vec<Value> {
        if !self.can_options {
            self.models = Some(LocalModels { error: "这个版本的 agent 不支持 session/options，列不出模型".into(), ..Default::default() });
            self.probe_done = true;
            return vec![];
        }
        vec![self.request("session/options", json!({ "cwd": self.cwd }), Pending::Options)]
    }

    /// 列模型：切下一个模型；都切完了就结束
    fn probe_next(&mut self) -> Vec<Value> {
        let Some(s) = self.session.clone() else {
            self.probe_done = true;
            return vec![];
        };
        if self.probe_queue.is_empty() {
            self.probe_done = true;
            return vec![];
        }
        let m = self.probe_queue.remove(0);
        self.probe_current = m.clone();
        let params = set_option(&s, &self.probe_model_id.clone(), &m, self.proto);
        vec![self.request("session/set_config_option", params, Pending::ProbeModel)]
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
                // 身份推送（#667）：扩展的 `_auth/status_update`（`{authStatus}`）和 RFD 的 `auth/status_update`（`{state, accounts}`）
                if method == "_auth/status_update" || method == "auth/status_update" {
                    let p = &v["params"];
                    let a = if p["authStatus"].is_object() { auth_of_ext(&p["authStatus"]) } else { auth_of_status(p) };
                    // RFD 的 `unknown` 不盖掉已经知道的
                    if a.kind != AUTH_UNKNOWN || self.auth.is_none() {
                        self.auth = Some(a);
                    }
                    if self.waiting_auth {
                        self.waiting_auth = false;
                        return self.after_auth();
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
                // 列模型时出错：记下原因，有多少算多少
                Pending::Options if self.auth_mode == AuthMode::Probe => {
                    self.models = Some(LocalModels { error: format!("session/options 失败：{msg}"), ..Default::default() });
                    self.probe_done = true;
                }
                // 派发时认不了 configId：照旧开会话、开完再切
                Pending::Options => {
                    self.initial.push((String::new(), String::new(), String::new()));
                    return self.new_session();
                }
                Pending::ProbeNew => {
                    if let Some(m) = self.models.as_mut() {
                        m.error = format!("开会话失败（{msg}），只列出了默认模型的强度");
                    }
                    self.probe_done = true;
                }
                Pending::ProbeModel => return self.probe_next(),
                Pending::Auth => {
                    if self.auth.as_ref().is_some_and(|x| x.kind != AUTH_UNKNOWN) {
                        return self.after_auth();
                    }
                    if self.auth_push {
                        self.waiting_auth = true;
                        return vec![];
                    }
                    self.auth = Some(AgentAuth { kind: AUTH_UNKNOWN.into(), label: format!("auth/status 失败：{msg}"), ..Default::default() });
                    return self.after_auth();
                }
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
                self.can_options = caps["session"]["options"].is_object() || caps["sessionCapabilities"]["options"].is_object();
                if self.auth_mode == AuthMode::Skip {
                    return self.after_init();
                }
                // 身份（#667）：公布了 `auth.status` 就问；只公布了 `_meta.authStatus` 的等它推；都没有的问不出来。
                // codex-acp 2.12 两种都公布，但 `auth/status` 的回应里没有种类，种类只在推送里：回应看不出种类时接着等推送
                self.auth_push = caps["_meta"]["authStatus"].is_object() || caps["auth"]["_meta"]["authStatus"].is_object();
                if caps["auth"]["status"].is_object() {
                    return vec![self.request("auth/status", json!({}), Pending::Auth)];
                }
                if self.auth_push {
                    if self.auth.is_none() {
                        self.waiting_auth = true;
                        return vec![];
                    }
                    return self.after_auth();
                }
                self.auth = Some(AgentAuth { kind: AUTH_UNKNOWN.into(), label: "这个版本的 agent 不报身份".into(), ..Default::default() });
                self.after_auth()
            }
            Pending::Options if self.auth_mode == AuthMode::Probe => {
                let mut m = models_of(r);
                // 默认模型的强度已经在这里了；别的模型开个不发提示词的会话逐个切
                self.probe_queue = m.models.iter().map(|x| x.id.clone()).filter(|id| *id != m.model).collect();
                self.probe_model_id = model_option(r).and_then(|o| option_id(o).map(str::to_string)).unwrap_or_default();
                if self.probe_queue.is_empty() || self.probe_model_id.is_empty() {
                    self.models = Some(m);
                    self.probe_done = true;
                    return vec![];
                }
                m.error = String::new();
                self.models = Some(m);
                vec![self.request("session/new", json!({ "cwd": self.cwd, "mcpServers": [] }), Pending::ProbeNew)]
            }
            Pending::Options => {
                // 派发：`@model`、`@thought_level` 按类别找到 configId，当初始值带上（先模型后强度）；找不到的留到开会话后再切
                let mut cats: Vec<&(String, String)> = self.sets.iter().filter(|(k, _)| k.starts_with('@')).collect();
                cats.sort_by_key(|(k, _)| if k == "@model" { 0 } else { 1 });
                for (k, v) in cats {
                    let cat = &k[1..];
                    if let Some(id) = r["configOptions"].as_array().into_iter().flatten().find(|o| o["category"].as_str() == Some(cat)).and_then(option_id) {
                        self.initial.push((id.to_string(), cat.to_string(), v.clone()));
                    }
                }
                let sent: Vec<String> = self.initial.iter().map(|(_, c, _)| format!("@{c}")).collect();
                self.sets.retain(|(k, _)| !sent.contains(k));
                // 一项都对不上：开会话时不带初始值（下面核对时照样会发现设不上）
                if self.initial.is_empty() {
                    self.initial.push((String::new(), String::new(), String::new()));
                }
                self.new_session()
            }
            Pending::ProbeNew => {
                let Some(id) = r["sessionId"].as_str().filter(|s| !s.is_empty()) else {
                    if let Some(m) = self.models.as_mut() {
                        m.error = "开会话没拿到 sessionId，只列出了默认模型的强度".into();
                    }
                    self.probe_done = true;
                    return vec![];
                };
                self.session = Some(id.to_string());
                self.probe_next()
            }
            Pending::ProbeModel => {
                let efforts = efforts_of(r);
                let cur = self.probe_current.clone();
                if let Some(x) = self.models.as_mut().and_then(|m| m.models.iter_mut().find(|x| x.id == cur)) {
                    x.efforts = efforts;
                }
                self.probe_next()
            }
            Pending::Auth => {
                let a = auth_of_status(r);
                if a.kind != AUTH_UNKNOWN {
                    self.auth = Some(a);
                    return self.after_auth();
                }
                // 回应里看不出种类：已经推来了就用推的，否则等推送（最多 AUTH_WAIT）
                if self.auth.as_ref().is_some_and(|x| x.kind != AUTH_UNKNOWN) {
                    return self.after_auth();
                }
                if self.auth_push {
                    self.waiting_auth = true;
                    return vec![];
                }
                self.auth = Some(a);
                self.after_auth()
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
                // 开会话时带的初始值（#666）：回应里是实际用的值，和要的不一样就是设不上，出错结束、不按默认跑
                if let Some(why) = initial_mismatch(r, &self.initial) {
                    self.outcome = Some(Outcome::Failed(why));
                    return vec![];
                }
                let proto = self.proto;
                // 认出了 session/options 的 v2 agent：剩下按类别的（初始值里没对上的）也不能静默跳过
                if self.can_options && proto == Proto::V2 {
                    if let Some((k, v)) = self.sets.iter().find(|(k, v)| k.starts_with('@') && config_set(r, s, k, v, proto).is_none()) {
                        self.outcome = Some(Outcome::Failed(format!("设不上{}：agent 没有「{v}」这一项", if k == "@model" { "模型" } else { "强度" })));
                        return vec![];
                    }
                }
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
/// `resetsAt` 是 Unix 秒（看着像毫秒的照用），`utilization` 是 0–1 的比例（大于 1 的当百分数）。现在的 Claude Code 把每个
/// 窗口的用量放在 `unifiedWindows.<窗口>` 里（`{status, resetsAt, utilization}`），顶层没有 `utilization`：顶层没有的从
/// `rateLimitType` 那个窗口里取（本机端到端实测）
fn rate_of(v: &Value) -> Option<Quota> {
    let status = v["status"].as_str().filter(|s| !s.is_empty())?;
    let window = v["rateLimitType"].as_str().unwrap_or_default().to_string();
    let unified = &v["unifiedWindows"][window.as_str()];
    let num = |k: &str| v[k].as_f64().or_else(|| unified[k].as_f64());
    let resets_at = num("resetsAt").filter(|t| *t > 0.0).map(|t| if t < 1e11 { (t * 1000.0) as i64 } else { t as i64 });
    let used = num("utilization").filter(|u| u.is_finite() && *u >= 0.0).map(|u| if u <= 1.0 { (u * 100.0).round() as i64 } else { u.round() as i64 });
    Some(Quota { status: status.to_string(), window, used, resets_at, ..Default::default() })
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

/* ------------------------------ 模型和强度（#666） ------------------------------ */

/// 配置项里类别是 model 的那一项
fn model_option(r: &Value) -> Option<&Value> {
    r["configOptions"].as_array()?.iter().find(|o| o["category"].as_str() == Some("model"))
}

/// 选项的（值、显示名），分组的也展开
fn option_pairs(v: &Value) -> Vec<(String, String)> {
    let mut out = vec![];
    for o in v.as_array().into_iter().flatten() {
        if let Some(x) = o["value"].as_str() {
            out.push((x.to_string(), o["name"].as_str().unwrap_or_default().to_string()));
        }
        out.extend(option_pairs(&o["options"]));
    }
    out
}

/// 「agent 自己的默认」这种选项：不算一个模型、一档强度
fn is_default(v: &str) -> bool {
    v.eq_ignore_ascii_case("default")
}

/// 会话配置里现在这个模型能设的强度（类别 thought_level 的选项，去掉 default）；没有这一项时为空
fn efforts_of(r: &Value) -> Vec<String> {
    r["configOptions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|o| o["category"].as_str() == Some("thought_level"))
        .map(|o| option_pairs(&o["options"]).into_iter().map(|(v, _)| v).filter(|v| !is_default(v)).collect())
        .unwrap_or_default()
}

/// `session/options` 的回应 → 模型表：模型下拉（去掉 default）、默认模型、默认模型的强度和默认强度
fn models_of(r: &Value) -> LocalModels {
    let at = board_core::now_ms();
    let Some(o) = model_option(r) else {
        return LocalModels { at, error: "agent 没有给模型下拉".into(), ..Default::default() };
    };
    let cur = o["currentValue"].as_str().unwrap_or_default().to_string();
    let efforts = efforts_of(r);
    let effort = r["configOptions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|x| x["category"].as_str() == Some("thought_level"))
        .and_then(|x| x["currentValue"].as_str())
        .filter(|v| !is_default(v))
        .unwrap_or_default()
        .to_string();
    let models = option_pairs(&o["options"])
        .into_iter()
        .filter(|(v, _)| !is_default(v))
        .map(|(id, name)| LocalModel { efforts: if id == cur { efforts.clone() } else { vec![] }, id, name })
        .collect();
    LocalModels { models, model: if is_default(&cur) { String::new() } else { cur }, effort, at, error: String::new() }
}

/// 开会话时带的初始值和回应里实际的值对不上时的原因：「账号里没有模型 X」「模型 X 不支持强度 Y」
fn initial_mismatch(r: &Value, initial: &[(String, String, String)]) -> Option<String> {
    let list = r["configOptions"].as_array().cloned().unwrap_or_default();
    let cur = |cat: &str| list.iter().find(|o| o["category"].as_str() == Some(cat)).and_then(|o| o["currentValue"].as_str()).map(str::to_string);
    for (id, cat, want) in initial.iter().filter(|(id, _, _)| !id.is_empty()) {
        let got = list.iter().find(|o| option_id(o) == Some(id.as_str())).and_then(|o| o["currentValue"].as_str());
        if got == Some(want.as_str()) {
            continue;
        }
        return Some(match cat.as_str() {
            "model" => format!("账号里没有模型 {want}（会话用的是 {}）：没有按默认模型跑", got.unwrap_or("默认")),
            "thought_level" => format!("模型 {} 不支持强度 {want}：没有按默认强度跑", cur("model").unwrap_or_else(|| "默认".into())),
            _ => format!("设不上 {id} = {want}"),
        });
    }
    None
}

/* ------------------------------ 身份（#667） ------------------------------ */

/// 扩展的 `authStatus`（`{kind, label, detail?, account?: {plan, …}}`，Claude Code、Codex 一样）→ 身份。邮箱不留
fn auth_of_ext(v: &Value) -> AgentAuth {
    let s = |x: &Value| x.as_str().unwrap_or_default().trim().to_string();
    let kind = s(&v["kind"]);
    AgentAuth { kind: if kind.is_empty() { AUTH_UNKNOWN.into() } else { kind }, label: s(&v["label"]), detail: s(&v["detail"]), plan: s(&v["account"]["plan"]) }
}

/// ACP `auth/status` 的回应（`{state, accounts}`）→ 身份。种类只有 Claude Code 在账号的 `_meta.claudeCode.kind` 里给；
/// 没有时登录了也按问不出来算（看不出是不是订阅）
fn auth_of_status(r: &Value) -> AgentAuth {
    let s = |x: &Value| x.as_str().unwrap_or_default().trim().to_string();
    match r["state"].as_str() {
        Some("unauthenticated") => AgentAuth { kind: AUTH_NONE.into(), label: "Not logged in".into(), ..Default::default() },
        Some("authenticated") => {
            let list = r["accounts"].as_array().cloned().unwrap_or_default();
            let a = list.iter().find(|a| a["active"].as_bool() == Some(true)).or(list.first()).cloned().unwrap_or(Value::Null);
            let meta = &a["_meta"]["claudeCode"];
            let kind = s(&meta["kind"]);
            AgentAuth {
                kind: if kind.is_empty() { AUTH_UNKNOWN.into() } else { kind },
                label: s(&a["label"]),
                detail: [s(&a["description"]), s(&meta["detail"])].into_iter().find(|d| !d.is_empty()).unwrap_or_default(),
                plan: s(&meta["plan"]),
            }
        }
        _ => AgentAuth { kind: AUTH_UNKNOWN.into(), ..Default::default() },
    }
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

/// 在启动环境里拉起 agent 的 ACP 入口：标准输出一行一行送进返回的通道（读完送 None）。启动失败时写明原因、返回退出码
fn spawn_agent(o: &Opts) -> std::result::Result<(Child, Option<ChildStdin>, mpsc::Receiver<Option<String>>), i32> {
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
    Ok((child, stdin, rx))
}

/// 列模型（`--auth probe`）最多用多久（问完身份之后算）：Claude Code 每切一个模型要等它重读配置
pub const PROBE_LIMIT: Duration = Duration::from_secs(150);

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
    let (mut child, mut stdin, rx) = match spawn_agent(o) {
        Ok(x) => x,
        Err(code) => {
            say(&unknown(format!("启动 {} 失败", o.prog)), None);
            return code;
        }
    };
    let quiet = |stdin: &mut Option<ChildStdin>, frames: Vec<Value>| {
        for f in frames {
            if let Some(w) = stdin.as_mut() {
                if writeln!(w, "{f}").and_then(|_| w.flush()).is_err() {
                    *stdin = None;
                }
            }
        }
    };
    let cwd = o.cwd.display().to_string();
    let mut client = Client::new(&cwd, "", None).with_auth(o.auth);
    quiet(&mut stdin, client.start());
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
                quiet(&mut stdin, f);
                continue;
            }
        } else if started.elapsed() >= HANDSHAKE {
            break unknown(format!("agent {} 秒内没有完成握手", HANDSHAKE.as_secs()));
        }
        match rx.recv_timeout(TICK) {
            Ok(Some(line)) => {
                let t = line.trim();
                if let Some(v) = t.starts_with('{').then(|| serde_json::from_str::<Value>(t).ok()).flatten().filter(Value::is_object) {
                    let reply = client.on_frame(&v);
                    quiet(&mut stdin, reply);
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
    quiet(&mut stdin, client.close());
    drop(stdin);
    wait(&mut child, GRACE);
    EXIT_OK
}

fn run(o: &Opts) -> i32 {
    let prompt = match std::fs::read_to_string(&o.prompt_file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ board acp：读不了提示词 {}：{e}", o.prompt_file.display());
            return EXIT_PROTOCOL;
        }
    };
    let (mut child, mut stdin, rx) = match spawn_agent(o) {
        Ok(x) => x,
        Err(code) => return code,
    };
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
        // 现在的 Claude Code：用量在 unifiedWindows.<窗口> 里（本机端到端实测的样子）
        let mut c = working(true);
        c.on_frame(&usage("s1", json!({ "status": "allowed", "rateLimitType": "five_hour", "resetsAt": 1795676400, "overageStatus": "rejected",
            "unifiedWindows": { "five_hour": { "status": "allowed", "resetsAt": 1795676400, "utilization": 0.3 } } })));
        let q = c.quota.clone().unwrap();
        assert_eq!((q.window.as_str(), q.used, q.resets_at), ("five_hour", Some(30), Some(1_795_676_400_000)));
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

    /// 「官方订阅」（#667）：Claude Code 公布了 auth.status → 先问 auth/status，是订阅才开会话；是 API key 就出错、写明身份
    #[test]
    fn subscription_asks_auth_status_before_the_session() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {} }, "_meta": { "authStatus": {} } } });
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        let f = c.on_frame(&reply(0, init.clone()));
        assert_eq!((f[0]["method"].as_str(), f[0]["id"].as_i64()), (Some("auth/status"), Some(1)));
        let max = json!({ "state": "authenticated", "accounts": [{ "label": "Claude Max", "active": true, "email": "a@b.c", "_meta": { "claudeCode": { "kind": "account", "plan": "max" } } }] });
        let f = c.on_frame(&reply(1, max));
        assert_eq!(f[0]["method"], "session/new", "是订阅：开会话");
        assert_eq!(c.auth, Some(AgentAuth { kind: "account".into(), label: "Claude Max".into(), detail: String::new(), plan: "max".into() }));
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, init));
        let key = json!({ "state": "authenticated", "accounts": [{ "label": "Anthropic API key", "description": "ANTHROPIC_API_KEY", "active": true, "_meta": { "claudeCode": { "kind": "api_key", "detail": "ANTHROPIC_API_KEY" } } }] });
        assert!(c.on_frame(&reply(1, key)).is_empty(), "不是订阅：不开会话");
        assert_eq!(c.outcome, Some(Outcome::Failed("按「官方订阅」派发，agent 用的不是订阅：Anthropic API key（ANTHROPIC_API_KEY），没有开会话".into())));
    }

    /// Codex 只公布 `_meta.authStatus`：等它推 `_auth/status_update`；没登录、网关都不开会话；不报身份的、等不到的也不开
    #[test]
    fn subscription_waits_for_the_pushed_auth_status() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "auth": { "_meta": { "authStatus": {} } } } });
        let push = |st: Value| json!({ "jsonrpc": "2.0", "method": "_auth/status_update", "params": { "authStatus": st } });
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        assert!(c.on_frame(&reply(0, init.clone())).is_empty());
        assert!(c.waiting_auth && c.handshaking());
        let f = c.on_frame(&push(json!({ "kind": "account", "label": "ChatGPT Plus", "account": { "email": "a@b.c", "plan": "plus" } })));
        assert_eq!(f[0]["method"], "session/new");
        assert_eq!(c.auth.as_ref().map(|a| (a.label.as_str(), a.plan.as_str())), Some(("ChatGPT Plus", "plus")));
        for (st, want) in [
            (json!({ "kind": "gateway", "label": "Custom model gateway", "detail": "relay" }), "Custom model gateway（relay）"),
            (json!({ "kind": "none", "label": "Not logged in" }), "Not logged in"),
        ] {
            let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
            c.start();
            c.on_frame(&reply(0, init.clone()));
            assert!(c.on_frame(&push(st)).is_empty());
            assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.contains(want)), "{:?}", c.outcome);
        }
        // 推之前就到了：不用等
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&push(json!({ "kind": "account", "label": "ChatGPT Pro" })));
        assert_eq!(c.on_frame(&reply(0, init.clone()))[0]["method"], "session/new");
        // 等不到
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, init));
        assert!(c.auth_timeout().is_empty());
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.contains("秒内没报身份")), "{:?}", c.outcome);
        // 不报身份的 agent（dsh、旧版适配器）
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, json!({ "protocolVersion": 1 })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.contains("不报身份")), "{:?}", c.outcome);
        // 不问身份（默认）：照旧直接开会话
        let mut c = Client::new("/w", "p", None);
        c.start();
        assert_eq!(c.on_frame(&reply(0, json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {} } } })))[0]["method"], "session/new");
    }

    /// codex-acp 2.12：公布了 auth.status，但回应里没有种类（只有 label），种类在推送里：不能按「不知道」算，要用推送的
    #[test]
    fn auth_status_without_kind_waits_for_the_push() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {}, "_meta": { "authStatus": {} } } } });
        let push = json!({ "jsonrpc": "2.0", "method": "_auth/status_update", "params": { "authStatus": { "kind": "account", "label": "ChatGPT Pro", "account": { "plan": "pro" } } } });
        let answer = json!({ "state": "authenticated", "accounts": [{ "label": "ChatGPT Pro", "active": true }] });
        // 回应先到：接着等推送
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, init.clone()));
        assert!(c.on_frame(&reply(1, answer.clone())).is_empty() && c.waiting_auth);
        assert_eq!(c.on_frame(&push)[0]["method"], "session/new");
        // 推送先到：回应来了直接用推送的
        let mut c = Client::new("/w", "p", None).with_auth(AuthMode::Subscription);
        c.start();
        c.on_frame(&reply(0, init));
        assert!(c.on_frame(&push).is_empty(), "还在等 auth/status 的回应");
        assert_eq!(c.on_frame(&reply(1, answer))[0]["method"], "session/new");
        assert_eq!(c.auth.map(|a| a.kind), Some("account".to_string()));
    }

    /// 会话配置项：模型（当前值、可选值）、强度（当前值、可选值；None = 这个模型没有强度这一项）
    fn opts_of(model: &str, models: &[&str], effort: Option<(&str, &[&str])>) -> Value {
        let mut list = vec![json!({ "configId": "model", "category": "model", "type": "select", "currentValue": model,
            "options": models.iter().map(|m| json!({ "value": m, "name": m.to_uppercase() })).collect::<Vec<_>>() })];
        if let Some((cur, vals)) = effort {
            list.push(json!({ "configId": "effort", "category": "thought_level", "type": "select", "currentValue": cur,
                "options": vals.iter().map(|v| json!({ "value": v, "name": v })).collect::<Vec<_>>() }));
        }
        Value::Array(list)
    }

    /// 列模型（#666）：session/options 给模型和默认模型的强度，再开不发提示词的会话逐个切，读出每个模型的强度；default 不算
    #[test]
    fn probe_lists_models_and_their_efforts() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {} }, "session": { "options": {} } } });
        let mut c = Client::new("/w", "", None).with_auth(AuthMode::Probe);
        c.start();
        assert_eq!(c.on_frame(&reply(0, init))[0]["method"], "auth/status");
        let f = c.on_frame(&reply(1, json!({ "state": "authenticated", "accounts": [{ "label": "Claude Max", "_meta": { "claudeCode": { "kind": "account" } } }] })));
        assert_eq!((f[0]["method"].as_str(), &f[0]["params"]), (Some("session/options"), &json!({ "cwd": "/w" })));
        let f = c.on_frame(&reply(2, json!({ "configOptions": opts_of("opus", &["default", "opus", "haiku", "sonnet"], Some(("high", &["default", "low", "high", "max"]))) })));
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["mcpServers"].clone()), (Some("session/new"), json!([])));
        let f = c.on_frame(&reply(3, json!({ "sessionId": "p1" })));
        assert_eq!(f[0]["params"], json!({ "sessionId": "p1", "configId": "model", "type": "id", "value": "haiku" }));
        let f = c.on_frame(&reply(4, json!({ "configOptions": opts_of("haiku", &["opus", "haiku", "sonnet"], None) })));
        assert_eq!(f[0]["params"]["value"], "sonnet");
        assert!(!c.probe_done);
        assert!(c.on_frame(&reply(5, json!({ "configOptions": opts_of("sonnet", &["opus", "haiku", "sonnet"], Some(("default", &["low", "medium"]))) }))).is_empty());
        assert!(c.probe_done);
        let m = c.models.clone().unwrap();
        assert_eq!((m.model.as_str(), m.effort.as_str(), m.error.as_str()), ("opus", "high", ""));
        let got: Vec<(String, String, Vec<String>)> = m.models.into_iter().map(|x| (x.id, x.name, x.efforts)).collect();
        let e = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(got, vec![
            ("opus".to_string(), "OPUS".to_string(), e(&["low", "high", "max"])),
            ("haiku".to_string(), "HAIKU".to_string(), vec![]),
            ("sonnet".to_string(), "SONNET".to_string(), e(&["low", "medium"])),
        ]);
        assert_eq!(c.close()[0]["method"], "session/close", "列完关掉会话");
        // 不支持 session/options 的 agent：写明原因
        let mut c = Client::new("/w", "", None).with_auth(AuthMode::Probe);
        c.start();
        c.on_frame(&reply(0, json!({ "protocolVersion": 2, "capabilities": { "auth": { "status": {} } } })));
        c.on_frame(&reply(1, json!({ "state": "unauthenticated" })));
        assert!(c.probe_done && c.models.unwrap().error.contains("session/options"));
    }

    /// 派发（#666）：v2、公布了 session.options 的 agent，模型和强度当 session/new 的初始值带上（先模型后强度）；回应里的值
    /// 和要的不一样（账号里没有、这个模型不支持）就出错结束，不按默认跑；对得上就照常往下
    #[test]
    fn dispatch_sends_model_and_effort_as_initial_values() {
        let init = json!({ "protocolVersion": 2, "capabilities": { "session": { "options": {} } } });
        let start = || {
            let mut c = Client::new("/w", "做卡", None).with_sets(vec![("@thought_level".into(), "max".into()), ("@model".into(), "sonnet".into())]);
            c.start();
            assert_eq!(c.on_frame(&reply(0, init.clone()))[0]["method"], "session/options");
            let f = c.on_frame(&reply(1, json!({ "configOptions": opts_of("opus", &["opus", "sonnet", "haiku"], Some(("high", &["low", "high", "max"]))) })));
            assert_eq!(f[0]["method"], "session/new");
            assert_eq!(
                f[0]["params"]["configOptions"],
                json!([{ "configId": "model", "type": "id", "value": "sonnet" }, { "configId": "effort", "type": "id", "value": "max" }]),
                "先模型后强度"
            );
            c
        };
        let mut c = start();
        let f = c.on_frame(&reply(2, json!({ "sessionId": "s1", "configOptions": opts_of("sonnet", &["opus", "sonnet"], Some(("max", &["low", "max"]))) })));
        assert_eq!(f[0]["method"], "session/prompt", "设上了：不用再切");
        let mut c = start();
        c.on_frame(&reply(2, json!({ "sessionId": "s1", "configOptions": opts_of("opus", &["opus"], Some(("max", &["max"]))) })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.starts_with("账号里没有模型 sonnet")), "{:?}", c.outcome);
        let mut c = start();
        c.on_frame(&reply(2, json!({ "sessionId": "s1", "configOptions": opts_of("sonnet", &["sonnet"], None) })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(ref w)) if w.starts_with("模型 sonnet 不支持强度 max")), "{:?}", c.outcome);
        // 没公布 session.options（旧版适配器）：照旧开会话后切
        let mut c = Client::new("/w", "做卡", None).with_sets(vec![("@model".into(), "sonnet".into())]);
        c.start();
        let f = c.on_frame(&reply(0, json!({ "protocolVersion": 2, "capabilities": {} })));
        assert!(f[0]["params"].get("configOptions").is_none() && f[0]["method"] == "session/new");
    }

    #[test]
    fn auth_status_payloads() {
        assert_eq!(auth_of_status(&json!({ "state": "unauthenticated" })).kind, "none");
        assert_eq!(auth_of_status(&json!({ "state": "unknown" })).kind, "unknown");
        // 登录了、却看不出种类（没有 claudeCode 的 _meta）：按不知道算
        let a = auth_of_status(&json!({ "state": "authenticated", "accounts": [{ "label": "Someone" }] }));
        assert_eq!((a.kind.as_str(), a.label.as_str()), ("unknown", "Someone"));
        assert_eq!(auth_of_ext(&json!({ "kind": "external", "label": "AWS Bedrock" })).words(), "AWS Bedrock");
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--auth", "only", "--", "x"])).is_ok(), "only 不要提示词");
        assert_eq!(parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--auth", "subscription", "--", "x"])).unwrap().auth, AuthMode::Subscription);
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--prompt-file", "/p", "--auth", "x", "--", "x"])).unwrap_err().contains("--auth"));
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
