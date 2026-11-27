//! `board host`：在这台机器上常驻，接服务端派来的卡片，在本机启动 agent（任务下发需求 §三、§七）。
//!
//! ```text
//! board host [--name <机器名>] [--workspace <工作空间>] [--dir <项目>=<路径>]...
//! ```
//!
//! - 参数也可以写在 `~/.board/config.toml` 的 `[host]`、`[agents.<标识>]` 里，命令行优先（见 config.rs）。
//!   桌面端「设置 → 本机」保存后在标准输入里写一行 `reload` 通知它：重新读 `[host]`，机器名、工作空间变了就直接换上，
//!   不用重启，正在跑的 agent 不受影响（命令行给了 `--name` / `--workspace` / `--dir` 的那一项不跟着配置文件变）。机器名变了先让服务端把
//!   机器记录原地改名（桌面端已经改过的不再改），改名失败就接着用原来的名字。桌面端「一键安装 board-skills」装好后也发一行
//!   `reload`：先重新看一遍每个 agent 的 skill，看得到了马上上报，界面上的提醒随之消失。
//! - 启动时找本机的启动环境（直接启动、登录 shell、cmd、PowerShell、Git Bash、WSL 的发行版 …，见 envs.rs），
//!   再找本机 agent：配置文件里的 `[agents.*]` 加上内置模板（[`BUILTIN`]：dsh、claude、codex），在每个 agent 支持、
//!   本机有的启动环境里各检查一次命令找不找得到（内置模板在直接启动、登录 shell 里还会看几个常见的安装位置）、取版本号
//!   （`<命令> --version` 的第一行）；找不到的也上报，标为「没找到」。内置模板的顺序就是默认 agent 的顺序：本机找到的第一个
//!   是默认。每个 agent 用哪个启动环境由机器设置决定（服务端写进派发记录）。找到了、但在它用的环境里看不到 `board-skills`
//!   的 agent 启动时提醒一行。怎么接入新的 agent 见 docs/agents.md。
//! - 工作空间（PRD §9.1）：每台设备只配一个目录，每块白板在这台机器上的目录是 `<工作空间>/<项目>`。启动时、之后取到
//!   任务时把服务端上摆着的白板在盘上摆好（不存在时建目录，白板设了仓库地址时 `git clone`）；`--dir` 单独覆盖某几块。
//!   旧配置的 `[host] dirs` 读进来换成工作空间（见 [`to_workspace`]、[`bootstrap_dirs`]）。
//! - 每 10 秒向服务端上报一次（机器心跳），同时替正在跑的派发的名字发 agent 级心跳；订阅实时推送，
//!   机器和派发有变化（推送里的 `hosts` 变了）就马上重新取自己的任务。每块白板目录的 git 状态（origin、
//!   当前分支）只每 5 分钟重新看一次，配置、目录、克隆完时马上重看：心跳不跑 git 子进程。
//! - 有空位、没有暂停时，按创建先后开始排队中的派发：服务端生成名字 `<机器名>/<模型名>-<强度>`、check、领取，
//!   host 在这块白板对应的目录里、在派发记录写的启动环境里启动 agent（`{prompt}` 换成 `/board-skills 开始执行 …`
//!   那一行，`{patch}` 换成这次派发的 profile 补丁路径；都作为一个参数传入，不拼进 shell 命令文本），环境变量带上 `BOARD_URL`、`BOARD_TOKEN`、`BOARD_AS`、`BOARD_HOST`、`BOARD`，以及 `BOARD_CONFIG`：
//!   指向这次派发自己的配置文件 `~/.board/host/run/<机器名>/<派发 ID>.toml`（地址和令牌，0600，派发结束时删除）。
//!   有的 agent 不把名字像密钥的环境变量传给它执行的命令（dsh 去掉名字里带 KEY、PASSWORD、SECRET、TOKEN 的），
//!   `board` 照样能从 `BOARD_CONFIG` 拿到令牌。还有 `BOARD_EXE`（board 自己的路径）：skill 的 `scripts/board`
//!   看到它就用它，免得 skill 里打包的旧版 board 不认识这些变量。
//! - 服务端说这台机器正在自动接单时（机器信息里的 `round`：开了自动接单、在接单时段里；旧版服务端是「正在领一轮」），有空位就向服务端要下一张（定时领取）。
//! - agent 用 ACP 启动：board host 启动 `board acp`（acp.rs），它在启动环境里执行 agent 的 ACP 入口（`AgentDef::acp`），
//!   握手（要 ACP v2，只会 v1 的 agent 按 v1）、发提示词、自动允许权限请求；做卡的派发带 `--extra-turns`：这一轮做完、
//!   名字还持有卡片时，v2 后台还在跑就等 agent 自己接着做，否则在同一会话里续一轮（最多 3 轮，#454、#461），拆解、自检不续。
//!   `board acp` 的标准输出和标准错误写到 `~/.board/host/logs/<派发 ID>.log`（0600）：
//!   一行一帧 ACP 消息。每 5 秒上传新增的行（最多最后 500 行），「原始输出」用转写后的一行一行（见 logfmt.rs）。
//! - 同一份日志逐行解析成结构化事件（agent 说的话、工具调用、出错、结束时的用量，见 events.rs），每 2 秒上传一次新增的，
//!   界面「会话」的「过程」按它画；会话换到下一张卡后，事件记到当时在做的那张卡上。上传、写评论之前脱敏（令牌和常见格式的密钥换成
//!   `<已隐藏>`，见 redact.rs），本机日志保留原文。单个日志超过 50 MB 只留开头 1 MB 和最后 10 MB；
//!   结束超过 30 天（`[host] log_days`）的日志在启动时和之后每天删一次。
//! - agent 退出后上报退出码、最后 10 个事件的文字和原始输出最后 20 行，服务端据此记下结果（已交审 / 做不下去 / 异常退出 /
//!   已停止），异常退出的评论里写这些。
//! - 人停止：结束 agent 进程树，包括启动环境的外壳（先 SIGTERM，10 秒后 SIGKILL；Windows 用 taskkill 结束进程树；
//!   WSL 里在 WSL 中结束 agent 的进程组），再上报。
//! - `Ctrl+C`（或 SIGTERM）：先按「人停止」结束本机由它启动的全部 agent，再让机器离线，然后退出。
//!   桌面端在后台运行它时设置 `BOARD_HOST_STDIN=1`：标准输入关闭（桌面端退出）也按 Ctrl+C 处理；读到一行 `reload` 时重新读配置。
//! - 机器名的 pid 文件 `~/.board/host/<机器名>.pid`（「进程号 启动时间 标识」）：board host 异常退出后在同一台机器上重启，
//!   用里面的标识接回还在线的名字；这时服务端上还在跑、本机却没有进程的派发按「异常退出」处理。
//! - 每个在跑的 agent 记在 `~/.board/host/run/<机器名>/<派发 ID>.proc`（「进程号 启动时间 启动环境 白板#卡片」）。
//!   board host 没走完退出流程（被 kill -9、崩溃）时 agent 会留下来接着跑：同名重启、第一次上报成功之后，
//!   先结束这些文件里还对得上的进程组，再清 run 目录。第一次上报之前不碰 run 目录：同名的 host 还在跑时，
//!   第二个每 2 秒重试一次、最多等 60 秒，旧 host 退出后接回名字；仍然占着才报 HOST_NAME_TAKEN，不删前一个的派发配置。
//! - 输出（标准输出）写不进去时（管道读端已关、终端已关）照常运行和退出，不因为打印失败中断退出流程。
//! - 远程管理（PRD §9.1）：任务里带着界面上发来的请求。「重新检查」重新找启动环境、agent、skill 后马上上报；「重启」「升级」等本机
//!   没有在跑的 agent 后先回报「开始处理」，再让机器离线、重新启动自己（macOS、Linux 原地 exec；Windows 桌面端退出码 75）。
//!   升级时命令行运行的执行 `board update` 后用新版本重启，桌面端运行的退出码 76 交给桌面端升级；失败的原因下一次上报带给服务端。
//!
//! 这个目录里：`mod.rs` 是参数、主循环、上报和订阅、重新读配置、退出；`agents.rs` 找本机 agent、安装和更新；
//! `runs.rs` 派发的启动、时限、停止和收尾；`logs.rs` 日志和事件的上传、清理；`dirs.rs` 工作空间、克隆、目录检查；
//! `sync.rs` 同步配置和导入 skills；`manage.rs` 远程管理和自检；`tests.rs` 单元测试。

use crate::backend::{Backend, Remote};
use crate::config::{self, AgentConf};
use crate::envs::{self, expand, kill_tree, Check, Env};
use crate::heartbeat::{Proc, Procs};
use crate::events;
use crate::gitsync;
use crate::logfmt::{self, Stream};
use crate::redact;
use crate::selftest;
use crate::mcpsync;
use crate::providersync;
use crate::skills;
use crate::output::{Output, Paint};
use board_core::hosts::{check_agent_id, check_host_name, host_name_from, result_words, HostAsk, LOG_KEEP};
use board_core::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

// 内置的 agent 模板：描述只在 board-core 的 `agentdef.rs` 里定义一份（接入方法见 docs/agents.md）。
// 配置文件里同标识的 `[agents.<标识>]` 覆盖它；顺序就是默认 agent 的顺序：本机找到的第一个是默认（机器设置里可以改）
use board_core::agentdef::{AgentDef as Builtin, AGENT_DEFS as BUILTIN};

mod agents;
mod chats;
mod dirs;
mod logs;
mod manage;
mod runs;
mod sync;

use agents::*;
use chats::*;
use dirs::*;
use logs::*;
use manage::*;
use runs::*;
use sync::*;

/// dsh 派发时写给它的 profile 补丁（`--profile acp --patch`）：acp profile 也挂着 goal 工具和
/// goal-round-driver，这一轮做完后会自动续轮，prompt 迟迟不结束。派发是一次性任务，关掉 goal 工具和续轮。
const DSH_PATCH: &str = include_str!("../dsh-acp.patch.yml");

/// agent 用的 skill
const SKILL: &str = "board-skills";

/// 主循环每一轮的间隔
const TICK: Duration = Duration::from_millis(200);
/// 多久上传一次新增的日志
const LOG_EVERY: Duration = Duration::from_secs(5);
/// 多久上传一次新增的结构化事件（会话「过程」：开始跑之后 5 秒内要看得到第一条）
const EVENTS_EVERY: Duration = Duration::from_secs(2);
/// 停止 agent：SIGTERM 之后等多久再 SIGKILL
const KILL_AFTER: Duration = Duration::from_secs(10);
/// 和服务端断开多久时在本机输出里提醒一次（服务端 50 秒没收到上报就判这台机器掉线）
const OFFLINE_WARN: Duration = Duration::from_secs(30);
/// 重启时结束上一次留下的 agent：SIGTERM 后等这么久，还在就 SIGKILL
const ORPHAN_KILL_AFTER: Duration = Duration::from_secs(5);
/// 重启后第一次上报撞上旧 board host 还在收尾：每 2 秒重试一次，默认最多等 60 秒
const RESTART_RETRY: Duration = Duration::from_secs(2);
/// 异常退出时评论里带的日志行数
const TAIL: usize = 20;
/// 单个本机日志超过这么大时，只留开头 1 MB 和最后 10 MB，中间写一行省略说明
const LOG_MAX: u64 = 50 * 1024 * 1024;
const LOG_HEAD: u64 = 1024 * 1024;
const LOG_TAIL: u64 = 10 * 1024 * 1024;
/// 本机日志保留几天（结束超过这么多天的删掉），配置文件 `[host] log_days` 可以改
const LOG_DAYS: i64 = 30;
/// 多久看一次要不要清理本机日志（启动时一次，之后每天一次）
const LOG_CLEAN_EVERY: Duration = Duration::from_secs(24 * 3600);
/// 一次最多读多少字节的新日志（多出来的只留最后这些）。Claude Code 的事件流一行可能很长（工具结果整段在里面），
/// 转写后只剩一两行，所以读得多一些
const LOG_CHUNK: u64 = 4 * 1024 * 1024;
/// 白板目录的 git 状态（origin、当前分支）多久重新看一次：机器心跳 10 秒一次，不能每次都跑 git 子进程
const DIR_CHECK_EVERY: Duration = Duration::from_secs(300);
/// 内置 agent 的版本多久查一次：不在 10 秒心跳里查；打开机器详情看到的是最近一次的结果
const AGENT_CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
const AGENT_CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// 安装/更新一个 agent 最多跑多久（配置文件 `[host] agent_job_minutes` 可以改）。npm 多平台包很大
/// （Codex 单个平台包就四百多 MB），慢网络下 10 分钟不够；npm 先装进临时目录，超时也不动旧版本
const AGENT_JOB_LIMIT: Duration = Duration::from_secs(30 * 60);

/// 「一分钟」有多长：`BOARD_HOST_MINUTE_MS`（毫秒，只给测试用），默认 60 秒。
/// 兜底设置（单次最长运行、没输出多久算卡住）的最小档是分钟，测试里用它把分钟缩短
fn minute() -> Duration {
    let ms = std::env::var("BOARD_HOST_MINUTE_MS").ok().and_then(|s| s.trim().parse::<u64>().ok()).filter(|n| *n > 0);
    ms.map(Duration::from_millis).unwrap_or(Duration::from_secs(60))
}

/// 收到 Ctrl+C / SIGTERM，或者（桌面端）标准输入关闭
static QUIT: AtomicBool = AtomicBool::new(false);
/// （桌面端）标准输入里来了一行 `reload`：配置文件的 `[host]` 改了，或者装好了 board-skills，重新读、重新检查
static RELOAD: AtomicBool = AtomicBool::new(false);

/// 桌面端运行、装成服务（守护进程）时，board host 要它们重启自己（Windows 上不能原地换进程）
pub const EXIT_RESTART: i32 = 75;
/// 桌面端运行时，board host 要桌面端升级（下载、验签、安装新版本后重启；失败时用 `BOARD_HOST_ASK_ERROR` 带上原因重新启动它）
pub const EXIT_UPGRADE: i32 = 76;
/// `board update` 最长跑多久
const UPDATE_LIMIT: Duration = Duration::from_secs(600);

/// 由桌面端在后台运行（`BOARD_HOST_STDIN=1`）
fn under_desktop() -> bool {
    std::env::var("BOARD_HOST_STDIN").is_ok_and(|v| v.trim() == "1")
}

/// 上报间隔：`BOARD_HOST_INTERVAL`（秒，只给测试用），默认 10 秒
fn interval() -> Duration {
    let secs = std::env::var("BOARD_HOST_INTERVAL").ok().and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| *s > 0.0);
    secs.map(Duration::from_secs_f64).unwrap_or(Duration::from_millis(BEAT_MS as u64))
}

/// 等旧 board host 退出的上限：`BOARD_HOST_RESTART_WAIT_MS`（毫秒，只给测试用），默认 60 秒
fn restart_wait() -> Duration {
    let ms = std::env::var("BOARD_HOST_RESTART_WAIT_MS").ok().and_then(|s| s.trim().parse::<u64>().ok()).filter(|n| *n > 0);
    ms.map(Duration::from_millis).unwrap_or(Duration::from_secs(60))
}

/// 命令行参数
pub struct Opts {
    pub name: Option<String>,
    /// `--workspace <路径>`：这台设备的工作空间，每个项目的目录是 `<工作空间>/<项目>`
    pub workspace: Option<String>,
    /// `--dir <项目>=<路径>`，可以有多个：单独指定某个项目的目录（旧写法 `<项目>/<白板>=<路径>` 只管那一块白板，兼容保留）
    pub dirs: Vec<String>,
}

/// board host 自己的目录：`~/.board/host`
fn host_dir() -> Result<PathBuf> {
    let d = config::board_dir()?.join("host");
    std::fs::create_dir_all(d.join("logs")).map_err(|e| BoardError::usage(format!("无法创建 {}：{e}", d.display())))?;
    Ok(d)
}

/// Ctrl+C、SIGTERM、关掉终端：先结束 agent 再退出
fn on_signals() {
    #[cfg(unix)]
    {
        extern "C" fn handle(_: libc::c_int) {
            QUIT.store(true, Ordering::SeqCst);
        }
        let h = handle as extern "C" fn(libc::c_int) as libc::sighandler_t;
        unsafe {
            libc::signal(libc::SIGINT, h);
            libc::signal(libc::SIGTERM, h);
            libc::signal(libc::SIGHUP, h);
        }
    }
    #[cfg(windows)]
    {
        unsafe extern "system" fn handle(_: u32) -> i32 {
            QUIT.store(true, Ordering::SeqCst);
            1
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn SetConsoleCtrlHandler(h: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
        }
        unsafe {
            SetConsoleCtrlHandler(Some(handle), 1);
        }
    }
    // 桌面端在后台运行时：一行 `reload` 是「设置 → 本机」保存了配置；标准输入关闭（桌面端退出、崩溃）就退出
    if std::env::var("BOARD_HOST_STDIN").is_ok_and(|v| v.trim() == "1") {
        std::thread::spawn(|| {
            use std::io::BufRead;
            let mut line = String::new();
            let mut stdin = std::io::stdin().lock();
            loop {
                line.clear();
                match stdin.read_line(&mut line) {
                    Ok(n) if n > 0 => {
                        if line.trim() == "reload" {
                            RELOAD.store(true, Ordering::SeqCst);
                        }
                    }
                    _ => break,
                }
            }
            QUIT.store(true, Ordering::SeqCst);
        });
    }
}

fn clock() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// 多久：3 小时、20 分钟、1 分 30 秒（和 board-core 的 dur_words 一样，但整分钟不带秒）
fn span_words(d: Duration) -> String {
    let t = d.as_secs();
    let (h, m) = (t / 3600, t % 3600 / 60);
    if h > 0 && m > 0 {
        format!("{h} 小时 {m} 分")
    } else if h > 0 {
        format!("{h} 小时")
    } else if m > 0 {
        format!("{m} 分钟")
    } else {
        format!("{t} 秒")
    }
}

/* ------------------------------ host ------------------------------ */

struct Host<'a> {
    p: &'a Paint,
    be: Backend,
    name: String,
    instance: String,
    /// 第一次上报时带上：上一次异常退出的 board host 的标识
    resume: Option<String>,
    os: String,
    /// 本机的启动环境
    envs: Vec<Env>,
    agents: Vec<Spec>,
    /// 工作空间（`<工作空间>/<项目>`）和 `--dir` 单独覆盖的目录：上报、启动 agent 都用它
    ws: HostWorkspace,
    /// 工作空间里摆着的白板：路径由 `ws` 派生，这里只记「是哪几块」（服务端摆着、盘上已经有目录的）
    boards: Vec<String>,
    /// 已经从服务端应用到的目录设置版本
    dirs_rev: i64,
    /// 每块白板目录的 git 状态（最近一次看的结果和看的时间）：心跳时沿用，隔 DIR_CHECK_EVERY 再跑 git
    dir_checks: Vec<DirCheck>,
    /// 服务端请求的克隆任务（以及刚结束的）
    clones: Vec<CloneRun>,
    /// 已经开始处理（或做完）的远程管理请求：上报里带上它，服务端据此清掉请求
    asked: Option<i64>,
    /// 升级（重启）没成的原因：下一次上报带上，成功上报后清掉。桌面端升级失败后用 `BOARD_HOST_ASK_ERROR` 传进来
    ask_error: Option<String>,
    /// 正在跑的 `board update`（命令行运行时的升级）
    updating: Option<(Child, Instant, PathBuf)>,
    /// 退出主循环之后要做的事（重启、交给桌面端升级）
    after: Option<After>,
    pidfile: PathBuf,
    logs: PathBuf,
    /// 本机日志保留几天（`[host] log_days`，默认 30）
    log_days: i64,
    /// board 自己的路径（BOARD_EXE）
    exe: PathBuf,
    /// 每次派发的配置文件放在这里：`~/.board/host/run/<机器名>`
    run: PathBuf,
    runs: Vec<Run>,
    /// 机器对话的 agent 进程（chats.rs）：不占会话位
    chats: Vec<ChatRun>,
    ended: Vec<Ended>,
    work: Option<HostWork>,
    /// 最近一次取到任务的时间
    work_at: Instant,
    /// 开始派发、要下一张失败（或这一轮已经领完）之后，等重新取到任务再试
    hold: bool,
    /// 上一次打印的连接错误（同样的错误不重复打印）
    last_err: String,
    /// 最近一次和服务端通上的时间；断开超过 OFFLINE_WARN 时提醒一次（offline_warned），恢复时再说一声
    last_ok: Instant,
    offline_warned: bool,
    /// 机器名、工作空间跟着配置文件变（命令行没给 `--name` / `--workspace` / `--dir`）
    follow_name: bool,
    follow_dirs: bool,
    /// 上一次读到的配置文件 `[host]`（机器名、工作空间、日志保留天数）：没变就不重新校验
    conf_seen: (Option<String>, Option<String>, Option<i64>),
    /// 正在后台做的自检：请求序号 → 线程（几个 agent 的单查可以同时做）
    selftest: HashMap<i64, std::thread::JoinHandle<Vec<SelfTestItem>>>,
    /// 后台查各内置 agent 最新版本（每天一次，不在心跳里查）
    agent_latest: Option<std::thread::JoinHandle<Vec<(String, std::result::Result<String, String>)>>>,
    next_agent_check: Instant,
    /// 正在后台安装/更新一个 agent
    agent_job: Option<AgentJob>,
    /// 做完了、还没回报成功的自检结果（连不上服务端时下一轮再报）
    selftest_out: Vec<(i64, Vec<SelfTestItem>)>,
    /// 已经做完并回报的自检请求：取到的任务里还有它时不再做；服务端清掉之后从这里去掉
    selftest_seen: HashSet<i64>,
    /// 自检启动的子进程：退出时结束它们
    selftest_pids: selftest::Pids,
    /// 每个会报身份的 agent（Claude Code、Codex）自己用的身份（#667），随上报带上
    auth: BTreeMap<String, board_core::hosts::AgentAuth>,
    /// 上一次问身份时 agent 的版本：版本变了（启动时为空）就再问一次；自检时清掉
    auth_ver: BTreeMap<String, String>,
    /// 每个会报身份的 agent 本机账号能用的模型和强度（#666），随上报带上
    local_models: BTreeMap<String, board_core::hosts::LocalModels>,
    /// 正在后台问身份和模型的：agent 标识 → 线程
    auth_jobs: HashMap<String, std::thread::JoinHandle<(board_core::hosts::AgentAuth, board_core::hosts::LocalModels)>>,
    /// 额度：还没报上去的（跑卡时 agent 推来的、按「刷新」查到或没查到的），随下一次上报带上
    quota_out: Vec<board_core::hosts::QuotaReport>,
    /// 额度：每个在跑的派发最近一次读到的额度文件（变了才报）
    quota_seen: HashMap<i64, board_core::hosts::Quota>,
    /// 额度：正在后台查的（agent 标识 → 线程）
    quota_jobs: HashMap<String, std::thread::JoinHandle<std::result::Result<board_core::hosts::Quota, String>>>,
    /// 额度：已经处理过的「刷新」请求时间（agent 标识 → 请求时间），同一个请求不查第二次
    quota_done: HashMap<String, i64>,
    /// 额度：上次读额度文件的时间
    quota_read: Instant,
    /// 同步配置（机器详情与配置同步重构）：正在后台做的（回应的请求序号，启动时自己写的那一次为空）
    sync_job: Option<(Option<i64>, std::thread::JoinHandle<SyncOut>)>,
    /// 做完了、还没报上去的结果（连不上服务端时下一轮再报）
    sync_out: Option<SyncResult>,
    /// 已经做过（或正在做）的同步请求序号
    sync_seen: Option<i64>,
    /// 启动后写过一次了
    sync_started: bool,
    /// 每个内置 agent 配置文件里现在的模型和强度（上报：派发名字和派发记录用它）
    agent_now: BTreeMap<String, (String, String)>,
    /// 列目录：回答过的请求
    ls_seen: Vec<i64>,
    /// 各 agent skill 目录里不是 Board 装的 skill（上报里带上，界面「从这台机器导入」用）和这些目录（导入时找回去）
    local_skills: Vec<board_core::skills::LocalSkill>,
    skill_targets: Vec<skills::Target>,
    /// 从这台机器导入 skills：正在后台做的（请求序号）、做完还没报上去的结果、报过的序号
    import_job: Option<(i64, std::thread::JoinHandle<(Vec<String>, Vec<SkillImportFail>)>)>,
    import_out: Option<(i64, Vec<String>, Vec<SkillImportFail>)>,
    import_seen: Option<i64>,
    /// 马上上报一次（重新检查完 agent、同步完配置之后）
    beat_soon: bool,
    /// 最近一次 `git fetch` 没成的项目目录：目录、原因、什么时候试的。随上报带给服务端（这台机器先不接这个项目的卡），
    /// 每 5 分钟重试一次，拉成了就去掉
    fetch_errs: Vec<(PathBuf, String, Instant)>,
    /// 下一次清理没用的 worktree 的时间
    next_wt_sweep: Instant,
    /// 资源（CPU、内存、GPU、磁盘）：后台每 10 秒采一次，上报时带上最近一次
    res: crate::res::Sampler,
}

/// `board host`：一直运行，直到 Ctrl+C
pub fn run(opts: Opts, p: &Paint) -> Result<()> {
    let file = config::File::load()?;
    let conf = file.host()?;
    let follow_name = opts.name.as_deref().is_none_or(|s| s.trim().is_empty());
    // 机器名
    let name = match opts.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => conf_name(conf.name.as_deref()),
    };
    check_host_name(&name).map_err(BoardError::usage)?;
    // 工作空间、单独覆盖的目录：命令行给了（`--workspace`、`--dir`）就只用命令行的，否则用配置文件里的；
    // `follow_dirs` 是「工作空间跟着配置文件变」（两个都没给时）
    let cmd_overrides = parse_raw_dirs(opts.dirs.clone())?;
    let from_cmd = workspace_arg(opts.workspace.as_deref()).is_some() || !cmd_overrides.is_empty();
    let raw_ws = workspace_arg(opts.workspace.as_deref()).or_else(|| workspace_arg(conf.workspace.as_deref()));
    let raw_overrides = if from_cmd { cmd_overrides } else { conf.dirs.clone() };
    let mut ws = to_workspace(raw_ws.as_deref(), &raw_overrides)?;
    let follow_dirs = !from_cmd;
    let log_days = conf.log_days.unwrap_or(LOG_DAYS);
    let os = sysinfo::System::long_os_version()
        .ok()
        .or_else(|| sysinfo::System::name().ok())
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    let mut be = Backend::open_host()?;
    let envs = envs::discover();
    // 盘上已经摆着的白板（`--dir` 覆盖的，加上服务端上报过的）：启动时先按它查一遍 board-skills
    let mut boards = ws_boards(&ws, &os);
    // 服务端上摆着的白板：所在项目在工作空间里还没有目录的，先按 `<工作空间>/<项目>` 建好（项目设了仓库地址的 `git clone`）
    let server: Vec<(String, Option<String>)> = match be.workspace() {
        Ok(w) => {
            // 项目设置里的项目目录（#488）：先按它摆，免得在 `<工作空间>/<项目标识>` 白建一份
            ws.projects = project_dirs(&w);
            w.projects.iter().flat_map(|p| p.boards.iter().map(|b| (b.path.clone(), Some(b.repo.clone()).filter(|r| !r.trim().is_empty())))).collect()
        }
        Err(e) => {
            let _ = writeln!(std::io::stdout(), "{} 取不到服务端上的白板：{}（工作空间里还没有目录的白板派来时再建）", p.yellow("!"), e.message);
            vec![]
        }
    };
    let mut checks: Vec<DirCheck> = vec![];
    let mut cloned: Vec<CloneRun> = vec![];
    let failed = bootstrap_dirs(&ws, &server, &mut checks, &mut cloned, &os, &host_dir()?.join("logs"));
    for (board, why) in &failed {
        let what = if board.is_empty() { String::new() } else { format!("{board}：") };
        let _ = writeln!(std::io::stdout(), "{} 工作空间里{what}{why}", p.yellow("!"));
    }
    for (board, _) in &server {
        if !boards.contains(board) {
            boards.push(board.clone());
        }
    }
    boards.sort();
    let mut agents = agent_specs(&conf.agents, &envs)?;
    for a in &mut agents {
        a.check_skills(&envs, &ws_pairs(&ws, &boards, &os));
    }
    let agent_now = models_now(&agents, &envs);
    let hd = host_dir()?;
    let pidfile = hd.join(format!("{name}.pid"));
    // 上一次在这台机器上用这个名字的 board host：进程不在了就接回它的名字；还在收尾就先等它退出
    let previous = saved_instance(&pidfile);
    let previous_alive = previous.as_ref().is_some_and(|(proc, _)| board_alive(*proc));
    let resume = previous.as_ref().filter(|_| !previous_alive).map(|(_, inst)| inst.clone());
    let predecessor = previous.filter(|_| previous_alive);
    let instance = format!("{:x}{:x}", now_ms(), std::process::id() as u64 * 2654435761 % 0xffff_ffff);
    let run = hd.join("run").join(&name);
    let exe = std::env::current_exe().map_err(|e| BoardError::usage(format!("找不到 board 自己的路径：{e}")))?;
    let res = crate::res::Sampler::start(envs::expand(&ws.path));
    let mut h = Host {
        run,
        exe,
        p,
        be,
        name,
        instance,
        resume,
        os,
        envs,
        agents,
        ws,
        boards,
        dirs_rev: 0,
        dir_checks: checks,
        clones: cloned,
        asked: None,
        ask_error: std::env::var("BOARD_HOST_ASK_ERROR").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        updating: None,
        after: None,
        pidfile,
        logs: hd.join("logs"),
        log_days,
        runs: vec![],
        chats: vec![],
        ended: vec![],
        work: None,
        work_at: Instant::now(),
        hold: false,
        last_err: String::new(),
        last_ok: Instant::now(),
        offline_warned: false,
        follow_name,
        follow_dirs,
        conf_seen: (conf.name.clone(), conf.workspace.clone(), conf.log_days),
        selftest: HashMap::new(),
        selftest_out: vec![],
        selftest_seen: HashSet::new(),
        selftest_pids: selftest::Pids::default(),
        auth: BTreeMap::new(),
        auth_ver: BTreeMap::new(),
        local_models: BTreeMap::new(),
        auth_jobs: HashMap::new(),
        quota_out: vec![],
        quota_seen: HashMap::new(),
        quota_jobs: HashMap::new(),
        quota_done: HashMap::new(),
        quota_read: Instant::now(),
        agent_latest: None,
        next_agent_check: Instant::now(),
        agent_job: None,
        sync_job: None,
        sync_out: None,
        sync_seen: None,
        sync_started: false,
        agent_now,
        ls_seen: vec![],
        local_skills: vec![],
        skill_targets: vec![],
        import_job: None,
        import_out: None,
        import_seen: None,
        beat_soon: false,
        fetch_errs: vec![],
        next_wt_sweep: Instant::now(),
        res,
    };
    on_signals();
    // 第一次上报：名字被旧 host 占着就先等它退出；别的占用、令牌不对、连不上都直接报错退出。
    // 在这之前不碰 run 目录：同名的 host 可能还在跑
    let w = h.first_beat(predecessor)?;
    h.set_work(w);
    h.resume = None;
    if let Some(e) = h.ask_error.take() {
        h.say(format!("{} {e}", p.yellow("!")));
    }
    // 上一次（被 kill -9、崩溃）留下的 agent 和派发配置文件
    h.kill_orphans();
    let _ = std::fs::remove_dir_all(&h.run);
    let me = Procs::new().and_then(|mut ps| ps.get(std::process::id())).map(|x| x.start).unwrap_or(0);
    let _ = std::fs::write(&h.pidfile, format!("{} {me} {}\n", std::process::id(), h.instance));
    h.hello();
    let rx = h.subscribe();
    let out = h.main_loop(rx);
    if let Err(e) = &out {
        h.say(format!("{} {}  {}", p.red("✗"), p.red_bold(e.code.as_str()), e.message));
    }
    h.shutdown(out.is_ok());
    match h.after {
        Some(After::Restart) => return restart_self(&h.exe, p),
        Some(After::DesktopUpgrade) => std::process::exit(EXIT_UPGRADE),
        None => {}
    }
    out
}

/// 配置文件里的机器名；没写时用本机主机名（转成合法的机器名）
fn conf_name(name: Option<&str>) -> String {
    match name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => host_name_from(&sysinfo::System::host_name().unwrap_or_default()),
    }
}

/// 机器 pid 文件里的旧进程和实例标识
fn saved_instance(path: &Path) -> Option<(Proc, String)> {
    let s = std::fs::read_to_string(path).ok()?;
    let mut it = s.split_whitespace();
    let proc = Proc::parse(&format!("{} {}", it.next()?, it.next()?))?;
    Some((proc, it.next()?.to_string()))
}

/// 进程还在，而且是 board
fn board_alive(proc: Proc) -> bool {
    Procs::new().is_some_and(|mut ps| ps.alive(proc) && ps.is_board(proc))
}

/// `board host rename <新机器名>`：让服务端把这台机器的记录原地改名（桌面端「设置 → 本机」保存机器名时调用）。
/// 旧名字取配置文件 `[host].name`，没写时用本机主机名——和 `board host` 启动时的算法一致，
/// 保证改的就是服务端记录的这台机器。设置和历史派发记录都保留
pub fn rename(to: &str, json: bool, p: &Paint) -> Result<Output> {
    let to = to.trim();
    check_host_name(to).map_err(BoardError::usage)?;
    let conf = config::File::load()?.host()?;
    let from = conf_name(conf.name.as_deref());
    check_host_name(&from).map_err(BoardError::usage)?;
    if from == to {
        return Ok(if json {
            Output::Json(json!({ "ok": true, "name": to, "unchanged": true }))
        } else {
            Output::Text(format!("{} 机器名没有变：{}\n", p.dim("·"), p.bold(to)))
        });
    }
    let mut be = Backend::open()?;
    let h = be.host_rename(&from, to)?;
    if json {
        return Ok(Output::Json(serde_json::to_value(&h).unwrap_or_default()));
    }
    Ok(Output::Text(format!(
        "{} 机器已改名：{} → {}  {}\n",
        p.green("✓"),
        p.bold(&from),
        p.bold(to),
        p.dim("设置和历史派发记录都保留")
    )))
}

impl Host<'_> {
    /// 打印一行。写不进去（管道读端已关、终端已关）就算了：不能让打印失败打断启动 agent、退出流程
    fn say(&self, s: String) {
        let _ = writeln!(std::io::stdout(), "{} {s}", self.p.dim(&clock()));
    }

    fn report(&mut self, done: bool) -> HostReport {
        self.res.set_ws(envs::expand(&self.ws.path));
        HostReport {
            name: self.name.clone(),
            instance: self.instance.clone(),
            resume: self.resume.clone(),
            os: self.os.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            agents: self
                .agents
                .iter()
                .map(|a| HostAgent {
                    auth: self.auth.get(&a.id).cloned(),
                    local_models: self.local_models.get(&a.id).cloned(),
                    // 会按「刷新」查额度：内置的 Claude Code、Codex
                    quota_probe: a.builtin.is_some() && board_core::hosts::QUOTA_PROBE_AGENTS.iter().any(|(id, _)| *id == a.id),
                    ..a.report(self.agent_now.get(&a.id))
                })
                .collect(),
            envs: self.envs.iter().map(Env::report).collect(),
            workspace: Some(self.ws.clone()),
            dirs: self.dir_reports(),
            dirs_rev: self.dirs_rev,
            // 已经退出、但结束还没上报成功的派发也算在跑：否则服务端会先按「本机没有这个进程」结束它，丢掉退出码和日志
            running: self.runs.iter().map(|r| r.d.id).chain(self.ended.iter().map(|e| e.id)).collect(),
            chats: Some(self.chat_ids()),
            done,
            asked: self.asked,
            ask_error: self.ask_error.clone(),
            selftest: true,
            config_sync: true,
            rescan_skills: true,
            local_skills: Some(self.local_skills.clone()),
            res: self.res.latest(),
            quotas: self.quota_out.clone(),
        }
    }

    fn rm(&self) -> &Remote {
        self.be.remote()
    }

    fn beat(&mut self, done: bool) -> Result<HostWork> {
        let rep = serde_json::to_value(self.report(done)).unwrap_or(Value::Null);
        self.rm().post(&format!("/api/hosts/{}/beat", self.name), rep)
    }

    /// 第一次上报：旧进程还在收尾时名字被占着，就等它退出；退出后带上它的实例标识接回名字
    fn first_beat(&mut self, predecessor: Option<(Proc, String)>) -> Result<HostWork> {
        let Some((prev, instance)) = predecessor else { return self.beat(false) };
        let begun = Instant::now();
        let wait = restart_wait();
        let mut waiting = false;
        loop {
            match self.beat(false) {
                Ok(w) => return Ok(w),
                Err(e) if e.code == ErrorCode::HostNameTaken => {
                    if !waiting {
                        waiting = true;
                        self.say(format!(
                            "{} 机器名 {} 还由上一个 board host 使用，等它退出（最多 {} 秒）",
                            self.p.yellow("!"),
                            self.name,
                            wait.as_secs()
                        ));
                    }
                    if !board_alive(prev) {
                        self.resume = Some(instance.clone());
                    }
                    if QUIT.load(Ordering::SeqCst) {
                        return Err(e);
                    }
                    let left = wait.saturating_sub(begun.elapsed());
                    if left.is_zero() {
                        return Err(e);
                    }
                    std::thread::sleep(RESTART_RETRY.min(left));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// board host 自己的目录（`~/.board/host`，日志目录的上一级）：同步配置的备份、Git 凭据放在这里
    fn host_dir(&self) -> PathBuf {
        self.logs.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
    }

    /// 启动后打印一次：机器、agent、白板与目录
    fn hello(&self) {
        let p = self.p;
        let mut s = format!("{} {} 已上线：{}\n", p.green("✓"), p.bold(&self.name), p.cyan(self.rm().url()));
        s += &format!("  {}{} · board {}\n", p.dim("系统    "), self.os, env!("CARGO_PKG_VERSION"));
        let envs: Vec<String> = self.envs.iter().map(|e| e.name.clone()).collect();
        s += &format!("  {}{}\n", p.dim("环境    "), envs.join("、"));
        let def = self.work.as_ref().and_then(|w| w.host.agent.clone());
        // 服务端按机器设置算出来的：用哪个启动环境、在里面找没找到
        let used = |id: &str| self.work.as_ref().and_then(|w| w.host.agents.iter().find(|a| a.id == id).cloned());
        let env_name = |id: &str| self.envs.iter().find(|e| e.id == id).map(|e| e.name.clone()).unwrap_or_else(|| id.to_string());
        for (i, a) in self.agents.iter().enumerate() {
            let head = if i == 0 { p.dim("agent   ") } else { "        ".into() };
            let u = used(&a.id).unwrap_or_else(|| a.report(None));
            let env = u.env.clone().or_else(|| a.default_check().map(|c| c.env.clone()));
            let state = if u.found {
                let v = if u.version.is_empty() { "找到了".to_string() } else { u.version.clone() };
                let e = env.as_deref().map(|e| p.dim(&format!(" · {}", env_name(e)))).unwrap_or_default();
                format!("{v}{e}{}", if def.as_deref() == Some(&a.id) { p.yellow("（默认）") } else { String::new() })
            } else if let Some(m) = &u.missing {
                // 装了、缺 Node.js（npx）：#458
                let place = env.as_deref().map(|e| format!("在{}里", env_name(e))).unwrap_or_default();
                p.red(&format!("{}{place}缺 {m}", if u.version.is_empty() { String::new() } else { format!("{} · ", u.version) }))
            } else if let Some(e) = env.as_deref().filter(|_| a.default_check().is_some()) {
                p.red(&format!("在{}里没找到", env_name(e)))
            } else {
                p.red("没找到")
            };
            s += &format!("  {head}{} {}  {state}\n", p.cyan(&a.id), a.name);
        }
        for a in &self.agents {
            let Some(env) = used(&a.id).and_then(|u| u.env).or_else(|| a.default_check().map(|c| c.env.clone())) else { continue };
            let Some(e) = self.envs.iter().find(|x| x.id == env) else { continue };
            let Some(to) = a.no_skill.get(&e.id) else { continue };
            s += &format!("  {} {} 在{}里看不到 {SKILL}：派给它的卡片会做不成，先装到 {to}（见 docs/agents.md）\n", p.yellow("!"), a.id, e.name);
        }
        let pairs = ws_pairs(&self.ws, &self.boards, &self.os);
        if self.ws.path.trim().is_empty() {
            s += &format!("  {}{}\n", p.dim("工作空间"), p.yellow("（没有）：用 --workspace <路径> 或 --dir <项目>=<路径> 加上，否则接不到卡片"));
        } else {
            s += &format!("  {}{}\n", p.dim("工作空间"), p.cyan(&self.ws.path));
        }
        if pairs.is_empty() {
            s += &format!("  {}{}\n", p.dim("白板    "), p.dim("（工作空间里还没有白板的目录：派来卡片时按 <工作空间>/<项目> 建好）"));
        }
        for (i, (b, d)) in pairs.iter().enumerate() {
            let head = if i == 0 { p.dim("白板    ") } else { "        ".into() };
            let why = if d.is_dir() { "" } else { "（还没有目录）" };
            s += &format!("  {head}{} → {}{}\n", p.cyan(b), d.display(), p.dim(why));
        }
        // 服务端上没有的白板（工作空间里的目录、`--dir` 覆盖的）
        if let Ok(ws) = self.rm().get::<Workspace>("/api/workspace", &[]) {
            for (b, _) in &pairs {
                if !ws.projects.iter().flat_map(|x| &x.boards).any(|x| &x.path == b) {
                    s += &format!("  {} 服务端没有白板 {b}，这台机器接不到它的卡片\n", p.yellow("!"));
                }
            }
        }
        s += &format!("  {}\n", p.dim("Ctrl+C 退出（先结束本机由它启动的 agent）"));
        let _ = write!(std::io::stdout(), "{s}");
    }

    /// 订阅实时推送：推送里的 hosts（机器和派发的版本）有变化就通知主循环
    fn subscribe(&self) -> mpsc::Receiver<i64> {
        let (tx, rx) = mpsc::channel();
        let rm = self.rm().clone();
        std::thread::spawn(move || {
            let mut wait = Duration::from_millis(500);
            loop {
                let begun = Instant::now();
                let _ = rm.stream(|data| {
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if let Some(h) = v["hosts"].as_i64() {
                            let _ = tx.send(h);
                        }
                    }
                });
                if begun.elapsed() > Duration::from_secs(30) {
                    wait = Duration::from_millis(500);
                }
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_secs(10));
            }
        });
        rx
    }

    fn main_loop(&mut self, rx: mpsc::Receiver<i64>) -> Result<()> {
        let every = interval();
        let mut next_beat = Instant::now() + every;
        let mut next_log = Instant::now() + LOG_EVERY;
        let mut next_events = Instant::now() + EVENTS_EVERY;
        // 启动时清一次本机日志，之后每天一次
        let mut next_clean = Instant::now();
        let mut fetch = false;
        let mut seen: Option<i64> = None;
        loop {
            if QUIT.load(Ordering::SeqCst) {
                return Ok(());
            }
            while let Ok(v) = rx.try_recv() {
                if seen != Some(v) || v < 0 {
                    seen = Some(v);
                    fetch = true;
                }
            }
            let now = Instant::now();
            self.poll_clones();
            // 桌面端保存了配置：换了名字、目录就马上上报，界面「机器」里立刻看到
            if RELOAD.swap(false, Ordering::SeqCst) && self.reload() {
                next_beat = now;
            }
            // 共用模板变了、重新检查过 agent：马上上报
            if std::mem::take(&mut self.beat_soon) {
                next_beat = now;
            }
            if now >= next_beat {
                next_beat = now + every;
                fetch = false;
                let quotas = self.quota_out.len();
                match self.beat(false) {
                    Ok(w) => {
                        self.connected();
                        // 报上去了的额度不再带（上报期间不会有新的：都在这个循环里加）
                        self.quota_out.drain(..quotas);
                        self.set_work(w);
                        self.last_err.clear();
                        self.ask_error = None;
                    }
                    Err(e) => self.trouble(e, &mut next_beat)?,
                }
            } else if fetch {
                fetch = false;
                match self.rm().get::<HostWork>(&format!("/api/hosts/{}/work", self.name), &[]) {
                    Ok(w) => {
                        self.connected();
                        self.set_work(w);
                    }
                    Err(e) => self.trouble(e, &mut next_beat)?,
                }
            }
            if !self.offline_warned && self.last_ok.elapsed() >= OFFLINE_WARN {
                self.offline_warned = true;
                self.say(format!("{} {}", self.p.yellow("!"), offline_note(self.last_ok.elapsed(), self.runs.len())));
            }
            if self.reap() {
                fetch = true;
            }
            // 自检：界面上点了就在后台做，做完回报
            self.poll_selftest();
            // 身份（#667）：启动时、agent 版本变了、自检时在后台问一次，问到了马上上报
            if self.poll_auth() {
                next_beat = Instant::now();
            }
            // 额度：跑卡时 agent 推来的、人点「刷新」查到的，有新的马上上报
            if self.poll_quota() {
                next_beat = Instant::now();
            }
            // agent 版本每天在后台查一次；安装/更新在后台跑，不占会话位
            if self.poll_agent_latest() {
                next_beat = Instant::now();
            }
            if self.poll_agent_job() {
                next_beat = Instant::now();
            }
            // 从这台机器导入 skills：界面上选了就在后台传进库，做完回报
            self.poll_skill_import();
            // 同步配置：启动时写一次，服务器上一改（收到同步请求）再写一次，写完回报、马上上报
            if self.poll_sync() {
                next_beat = Instant::now();
            }
            // 远程管理：重新检查、重启、升级
            let (soon, quit) = self.handle_ask();
            let (soon2, quit2) = self.poll_update();
            if quit || quit2 {
                return Ok(());
            }
            if soon || soon2 {
                next_beat = Instant::now();
            }
            self.enforce_timeouts();
            self.enforce();
            // 机器对话：发消息、停止这一轮、看状态（不占会话位，不受暂停接单、接单时段限制）
            self.chat_tick();
            if self.fill()? {
                fetch = true;
            }
            if now >= next_events {
                next_events = now + EVENTS_EVERY;
                self.upload_events();
                self.chat_upload();
            }
            if now >= next_log {
                next_log = now + LOG_EVERY;
                self.upload_logs();
                self.report_ended();
            }
            // 空闲的 worktree 超过 36 小时没用的、多出 4 个的清掉：启动时一次，之后每小时看一次
            if now >= self.next_wt_sweep {
                self.next_wt_sweep = now + Duration::from_secs(3600);
                self.sweep_worktrees();
            }
            // 本机日志清理：启动时一次，之后每天一次
            if now >= next_clean {
                next_clean = now + LOG_CLEAN_EVERY;
                self.clean_logs();
            }
            std::thread::sleep(TICK);
        }
    }

    /// 桌面端通知（标准输入一行 `reload`）：「设置 → 本机」保存了配置，或者「一键安装」装好了 board-skills。
    /// 先重新看一遍 skill，再重新读配置。返回有没有变（变了马上上报）
    fn reload(&mut self) -> bool {
        // 桌面端「一键安装」、改了本机配置：像刚启动一样再写一次配置（board-skills 跟着装上）
        self.sync_started = false;
        let skills = self.recheck_skills();
        self.reload_conf() || skills
    }

    /// 重新读配置文件的 `[host]`，不重启，换上新的机器名、白板与目录，正在跑的 agent 不受影响。
    /// 返回有没有换。配置不对（名字不合规、目录不存在）、服务端不让改名时打印一行，接着用原来的
    fn reload_conf(&mut self) -> bool {
        if !self.follow_name && !self.follow_dirs {
            return false;
        }
        let Ok(conf) = config::File::load().and_then(|f| f.host()) else { return false };
        let raw = (conf.name.clone(), conf.workspace.clone(), conf.log_days);
        if raw == self.conf_seen {
            return false;
        }
        self.conf_seen = raw;
        let mut changed = false;
        // 日志保留天数：跟着配置换，下一次清理按新的来
        let days = conf.log_days.unwrap_or(LOG_DAYS);
        if days != self.log_days {
            self.log_days = days;
            self.say(format!("{} 本机日志保留 {days} 天", self.p.green("✓")));
        }
        if self.follow_name {
            let to = conf_name(conf.name.as_deref());
            if to != self.name {
                match self.rename_to(&to) {
                    Ok(()) => changed = true,
                    Err(e) => self.say(format!(
                        "{} 机器名没有改成 {to}：{}  {}（接着用 {}）",
                        self.p.yellow("!"),
                        e.code.as_str(),
                        e.message,
                        self.name
                    )),
                }
            }
        }
        if self.follow_dirs {
            // `[host] workspace`，加上 `[host] dirs`（旧写法：单独给某几块白板指定目录）
            match to_workspace(conf.workspace.as_deref(), &conf.dirs) {
                Ok(ws) if !same_ws(&ws, &self.ws) => {
                    // 项目目录（#488）来自服务端，不在配置文件里：留着
                    self.ws = HostWorkspace { projects: self.ws.projects.clone(), ..ws };
                    // 服务端上的白板照旧（目录跟着新的工作空间派生），加上新写的 `--dir` 白板
                    for b in ws_boards(&self.ws, &self.os) {
                        if !self.boards.contains(&b) {
                            self.boards.push(b);
                        }
                    }
                    self.boards.sort();
                    for a in &mut self.agents {
                        a.check_skills(&self.envs, &ws_pairs(&self.ws, &self.boards, &self.os));
                    }
                    let path = if self.ws.path.is_empty() { "（没有）".to_string() } else { self.ws.path.clone() };
                    self.say(format!("{} 工作空间已更新：{path}", self.p.green("✓")));
                    changed = true;
                }
                Ok(_) => {}
                Err(e) => self.say(format!("{} 工作空间没有更新：{}（接着用原来的）", self.p.yellow("!"), e.message)),
            }
        }
        changed
    }

    /// 换机器名：服务端把机器记录原地改名（设置、历史派发记录保留），再换 pid 文件。
    /// 桌面端保存时已经先改过名（旧名字在服务端没有记录了，NO_SUCH_HOST）就直接换。
    /// 正在跑的 agent 的派发配置留在旧名字的 run 目录里，等它们都结束、board host 下次启动再换到新名字的目录
    fn rename_to(&mut self, to: &str) -> Result<()> {
        check_host_name(to).map_err(BoardError::usage)?;
        let from = self.name.clone();
        match self.be.host_rename(&from, to) {
            Ok(_) => {}
            Err(e) if e.code == ErrorCode::NoSuchHost => {}
            Err(e) => return Err(e),
        }
        let pidfile = self.pidfile.with_file_name(format!("{to}.pid"));
        if let Ok(s) = std::fs::read_to_string(&self.pidfile) {
            if s.contains(&self.instance) {
                let _ = std::fs::write(&pidfile, s);
                let _ = std::fs::remove_file(&self.pidfile);
            }
        }
        self.pidfile = pidfile;
        if self.runs.is_empty() {
            let _ = std::fs::remove_dir_all(&self.run);
            self.run = self.run.with_file_name(to);
        }
        self.name = to.to_string();
        self.say(format!("{} 机器名已改为 {}（原来是 {from}）", self.p.green("✓"), self.p.bold(to)));
        Ok(())
    }

    /// 和服务端通上了：断开时提醒过的，说一声恢复了
    fn connected(&mut self) {
        if self.offline_warned {
            self.say(format!("{} 和服务端恢复连接（断开了 {} 秒）", self.p.green("✓"), self.last_ok.elapsed().as_secs()));
            self.offline_warned = false;
        }
        self.last_ok = Instant::now();
    }

    /// 请求失败：名字被别的 board host 占用时退出；机器被移除时马上重新上报（按新机器建立）；其他（连不上等）打印一次后继续重试
    fn trouble(&mut self, e: BoardError, next_beat: &mut Instant) -> Result<()> {
        match e.code {
            ErrorCode::HostNameTaken | ErrorCode::InvalidToken | ErrorCode::NotLoggedIn => Err(e),
            ErrorCode::NoSuchHost => {
                *next_beat = Instant::now();
                Ok(())
            }
            _ => {
                let msg = format!("{}  {}", e.code.as_str(), e.message);
                if msg != self.last_err {
                    self.say(format!("{} {msg}（继续重试）", self.p.yellow("!")));
                    self.last_err = msg;
                }
                Ok(())
            }
        }
    }

    /// 退出：按「人停止」结束本机所有 agent（先 SIGTERM，10 秒后 SIGKILL），上报结束，再让机器离线
    fn shutdown(&mut self, graceful: bool) {
        if let Some(mut job) = self.agent_job.take() {
            kill_tree(job.child.id(), true);
            let _ = job.child.kill();
            let _ = job.child.wait();
            if let Some(stage) = &job.stage {
                stage.discard();
            }
        }
        // 自检启动的 agent、git：一起结束（结果不再回报，请求过期后可以重新自检）
        for pid in self.selftest_pids.lock().map(|p| p.clone()).unwrap_or_default() {
            kill_tree(pid, true);
        }
        self.chat_shutdown();
        if !self.runs.is_empty() {
            self.say(format!("{} 正在退出：结束 {} 个 agent", self.p.yellow("◼"), self.runs.len()));
        }
        for r in self.runs.iter_mut() {
            if r.term_at.is_none() {
                r.stop(false);
                r.term_at = Some(Instant::now());
            }
        }
        let begun = Instant::now();
        while self.runs.iter_mut().any(|r| matches!(r.child.try_wait(), Ok(None))) && begun.elapsed() < KILL_AFTER {
            std::thread::sleep(Duration::from_millis(100));
        }
        for r in self.runs.iter_mut() {
            if matches!(r.child.try_wait(), Ok(None)) {
                r.stop(true);
                let _ = r.child.wait();
            }
        }
        let runs = std::mem::take(&mut self.runs);
        for mut r in runs {
            // 先回收主进程，`finish_run` 里的 `kill_rest` 才是对已经退出的进程号下手
            let code = r.child.wait().ok().and_then(exit_code);
            // board host 退出时已经在按兜底停止的，照样报「超时」：原因写清楚
            let ended = self.finish_run(r, code, "host_exit");
            // 升级 / 重启 / 退出桌面端也会结束正在跑的派发：和 agent 自己退出一样保存没推的改动
            self.handoff(&ended, code);
        }
        // 派发的临时文件由 `finish_run` 逐个删掉；WSL 的进程号文件要等 `kill_rest` 读完再删整个目录
        let _ = std::fs::remove_dir_all(&self.run);
        self.report_ended();
        if graceful {
            let _ = self.beat(true);
        }
        // pid 文件还是自己的才删
        if std::fs::read_to_string(&self.pidfile).is_ok_and(|s| s.contains(&self.instance)) {
            let _ = std::fs::remove_file(&self.pidfile);
        }
        self.say(format!("{} {} 已离线", self.p.green("✓"), self.p.bold(&self.name)));
    }
}

#[cfg(test)]
mod tests;
