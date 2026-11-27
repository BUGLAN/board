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
//! `session/cancel`，最多等 5 秒。派发时 board host 还拿着标准输入的写端：读到 EOF 说明 host 不在了，同样取消这一轮。
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

mod client;
mod merge;
mod quota;
mod run;

pub use run::main;
pub(crate) use run::ChatState;
#[cfg(test)]
pub(crate) use run::{chat_cmd, ChatCmd};

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(test)]
mod testutil {
    use serde_json::{json, Value};

    pub(super) fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    pub(super) fn reply(id: i64, result: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    }

    pub(super) fn state(session: &str, update: Value) -> Value {
        let mut u = update;
        u["sessionUpdate"] = json!("state_update");
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": u } })
    }
}
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

/// 查额度（`--usage`）最多等多久：起 agent、开会话、跑一条本地命令
pub const USAGE_LIMIT: Duration = Duration::from_secs(90);
/// 列模型（`--auth probe`）最多用多久（问完身份之后算）：Claude Code 每切一个模型要等它重读配置
pub const PROBE_LIMIT: Duration = Duration::from_secs(150);

pub const EXIT_OK: i32 = 0;
pub const EXIT_PROTOCOL: i32 = 2;
pub const EXIT_LIMIT: i32 = 3;
/// 这一轮因为订阅账号的额度用完结束（#668）
pub const EXIT_QUOTA: i32 = 4;
pub const EXIT_CANCELLED: i32 = 130;

/// 做卡的派发最多续几轮（board host 传 `--extra-turns`）
pub const EXTRA_TURNS: u32 = 3;

const USAGE: &str = "用法：board acp --env <启动环境> [--env-prog <程序>] --cwd <目录> (--prompt-file <文件> | --chat <状态文件> [--resume <会话>] [--idle-exit <秒>] | --auth only | --usage <本地命令>) [--auth subscription] [--provider-file <文件>] [--mode <模式>] [--set <选项>=<值>]… [--extra-turns <次数>] [--quota-file <文件>] -- <ACP 入口命令> [参数…]";

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
    /// 查额度（`--usage /usage`、`--usage /status`）：开会话、发这条本地命令（不调模型、不花额度），读出两个窗口，
    /// 写一行 `{"usage": …}` 到标准输出就退出
    pub usage: Option<String>,
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
        if !matches!(k, "--env" | "--env-prog" | "--cwd" | "--prompt-file" | "--provider-file" | "--mode" | "--set" | "--extra-turns" | "--chat" | "--resume" | "--idle-exit" | "--quota-file" | "--auth" | "--usage") {
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
    let usage = match m.get("--usage").map(|x| x.trim()) {
        None => None,
        Some(x) if x.starts_with('/') && x.len() > 1 && !x.contains(char::is_whitespace) => Some(x.to_string()),
        Some(x) => return Err(format!("--usage 要写一条本地命令（/usage、/status）：{x}")),
    };
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
        prompt_file: if chat.is_some() || usage.is_some() || matches!(auth, AuthMode::Only | AuthMode::Probe) { m.get("--prompt-file").map(PathBuf::from).unwrap_or_default() } else { PathBuf::from(need("--prompt-file")?) },
        mode: m.get("--mode").cloned().filter(|x| !x.trim().is_empty()),
        sets,
        provider_file: m.get("--provider-file").filter(|x| !x.trim().is_empty()).map(PathBuf::from),
        extra_turns,
        quota_file: m.get("--quota-file").filter(|x| !x.trim().is_empty()).map(PathBuf::from),
        auth,
        usage,
        resume: m.get("--resume").map(|x| x.trim().to_string()).filter(|x| !x.is_empty()),
        idle_exit,
        chat,
        prog,
        args: tail[1..].to_vec(),
    })
}
