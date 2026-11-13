//! `board host`：在这台机器上常驻，接服务端派来的卡片，在本机启动 agent（任务下发需求 §三、§七）。
//!
//! ```text
//! board host [--name <机器名>] [--dir <项目>/<白板>=<路径>]...
//! ```
//!
//! - 参数也可以写在 `~/.board/config.toml` 的 `[host]`、`[agents.<标识>]` 里，命令行优先（见 config.rs）。
//!   桌面端「设置 → 本机」保存后在标准输入里写一行 `reload` 通知它：重新读 `[host]`，机器名、白板与目录变了就直接换上，
//!   不用重启，正在跑的 agent 不受影响（命令行给了 `--name` / `--dir` 的那一项不跟着配置文件变）。机器名变了先让服务端把
//!   机器记录原地改名（桌面端已经改过的不再改），改名失败就接着用原来的名字。桌面端「一键安装 board-skills」装好后也发一行
//!   `reload`：先重新看一遍每个 agent 的 skill，看得到了马上上报，界面上的提醒随之消失。
//! - 启动时找本机的启动环境（直接启动、登录 shell、cmd、PowerShell、Git Bash、WSL 的发行版 …，见 envs.rs），
//!   再找本机 agent：配置文件里的 `[agents.*]` 加上内置模板（[`BUILTIN`]：dsh、claude、pi、codex），在每个 agent 支持、
//!   本机有的启动环境里各检查一次命令找不找得到（内置模板在直接启动、登录 shell 里还会看几个常见的安装位置）、取版本号
//!   （`<命令> --version` 的第一行）；找不到的也上报，标为「没找到」。内置模板的顺序就是默认 agent 的顺序：本机找到的第一个
//!   是默认。每个 agent 用哪个启动环境由机器设置决定（服务端写进派发记录）。找到了、但在它用的环境里看不到 `board-skills`
//!   的 agent 启动时提醒一行。怎么接入新的 agent 见 docs/agents.md。
//! - 每 10 秒向服务端上报一次（机器心跳），同时替正在跑的派发的名字发 agent 级心跳；订阅实时推送，
//!   机器和派发有变化（推送里的 `hosts` 变了）就马上重新取自己的任务。每块白板目录的 git 状态（origin、
//!   当前分支）只每 5 分钟重新看一次，配置、目录、克隆完时马上重看：心跳不跑 git 子进程。
//! - 有空位、没有暂停时，按创建先后开始排队中的派发：服务端生成名字 `<agent 标识>-<机器名>/<模型名>`、check、领取，
//!   host 在这块白板对应的目录里、在派发记录写的启动环境里启动 agent（`{prompt}` 换成 `/board-skills 开始执行 …`
//!   那一行，`{patch}` 换成这次派发的 profile 补丁路径；都作为一个参数传入，不拼进 shell 命令文本），环境变量带上 `BOARD_URL`、`BOARD_TOKEN`、`BOARD_AS`、`BOARD_HOST`、`BOARD`，以及 `BOARD_CONFIG`：
//!   指向这次派发自己的配置文件 `~/.board/host/run/<机器名>/<派发 ID>.toml`（地址和令牌，0600，派发结束时删除）。
//!   有的 agent 不把名字像密钥的环境变量传给它执行的命令（dsh 去掉名字里带 KEY、PASSWORD、SECRET、TOKEN 的），
//!   `board` 照样能从 `BOARD_CONFIG` 拿到令牌。还有 `BOARD_EXE`（board 自己的路径）：skill 的 `scripts/board`
//!   看到它就用它，免得 skill 里打包的旧版 board 不认识这些变量。
//! - 正在一轮领取时（定时到点、界面上「现在领一轮」），有空位就向服务端要下一张（定时领取）。
//! - agent 的标准输出和标准错误写到 `~/.board/host/logs/<派发 ID>.log`（0600），每 5 秒上传新增的行（最多最后 500 行）。
//!   内置模板都用 agent 的结构化输出（Claude Code `--output-format stream-json --verbose`、pi `--mode json`、
//!   Codex `exec --json`、dsh `--json`），本机日志文件里存原样的事件，上传的「原始输出」用转写后的一行一行（见 logfmt.rs）。
//! - 同一份输出逐行解析成结构化事件（agent 说的话、工具调用、出错、结束时的用量，见 events.rs），每 2 秒上传一次新增的，
//!   界面「会话」的「过程」按它画；会话换到下一张卡后，事件记到当时在做的那张卡上。配置文件覆盖了命令、又没带事件流
//!   参数的，或者配置文件里新加的 agent，只报格式为空（界面只能看原始输出）。上传、写评论之前脱敏（令牌和常见格式的密钥换成
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
//!   第二个只报 HOST_NAME_TAKEN，不删前一个的派发配置。
//! - 输出（标准输出）写不进去时（管道读端已关、终端已关）照常运行和退出，不因为打印失败中断退出流程。
//! - 远程管理（PRD §9.1）：任务里带着界面上发来的请求。「重新检查」重新找启动环境、agent、skill 后马上上报；「重启」「升级」等本机
//!   没有在跑的 agent 后先回报「开始处理」，再让机器离线、重新启动自己（macOS、Linux 原地 exec；Windows 桌面端退出码 75）。
//!   升级时命令行运行的执行 `board update` 后用新版本重启，桌面端运行的退出码 76 交给桌面端升级；失败的原因下一次上报带给服务端。
//! - 白板的环境检查（PRD §9.1）：按服务端发来的命令在白板目录里跑，见 [`ReadyRun`]。

use crate::backend::{Backend, Remote};
use crate::config::{self, AgentConf};
use crate::envs::{self, expand, kill_tree, Check, Env};
use crate::heartbeat::{Proc, Procs};
use crate::events;
use crate::logfmt::{self, Stream};
use crate::redact;
use crate::selftest;
use crate::mcpsync;
use crate::skills;
use crate::{Output, Paint};
use board_core::hosts::{check_agent_id, check_host_name, host_name_from, result_words, DirReady, HostAsk, LOG_KEEP, READY_LIMIT_MS, READY_TAIL};
use board_core::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// 内置的 agent 模板（接入方法见 docs/agents.md）。配置文件里同标识的 `[agents.<标识>]` 覆盖它
struct Builtin {
    id: &'static str,
    name: &'static str,
    /// 命令模板：非交互地做完一个任务就退出，不问权限；`{prompt}` 换成提示词、`{patch}` 换成
    /// 这次派发的 profile 补丁路径，都作为一个参数传入
    cmd: &'static [&'static str],
    /// 模型名：派发的 agent 名字（`<agent 标识>-<机器名>/<模型名>`）的第二段。配置文件里的 `model` 覆盖它；
    /// 空表示这个 agent 没有默认的模型（pi、codex 里配），生成名字时服务端用 agent 标识代替
    model: &'static str,
    /// 命令不在 PATH 里时再看这些位置（`~` 是用户目录）
    also: &'static [&'static str],
    /// 这个 agent 读用户级 skill 的目录：启动时看 `board-skills` 装没装
    skills: &'static [&'static str],
    /// 这个 agent 在工作目录（仓库根目录）里读 skill 的相对目录：放在这里也算装好
    project: &'static [&'static str],
    /// Windows 上启动时把 Git Bash 的目录放到 PATH 前面：pi 从 PATH 找 `bash.exe`，
    /// Git 装在非 `%ProgramFiles%` 位置时（例如 D:\Git）才能不落到 WSL 的 bash
    git_bash: bool,
    /// Windows 上支持的启动环境，顺序就是默认选择的顺序（`wsl:*` 是每个 WSL 发行版）；
    /// macOS、Linux 上都是登录 shell → 直接启动（[`envs::UNIX_ORDER`]）
    win: &'static [&'static str],
    /// 机器设置里选了模型时插到 `{prompt}` 前面的参数（`{model}` 换成模型）；空表示不能选模型
    model_args: &'static [&'static str],
    /// 选了思考强度时插到 `{prompt}` 前面的参数（`{effort}` 换成档位）；空表示不能选强度
    effort_args: &'static [&'static str],
    /// 支持的思考强度档位（按各自 `--help` 核对过），从低到高
    efforts: &'static [&'static str],
    /// 机器设置「模型」下拉里列的常用模型（也可以手填）
    models: &'static [&'static str],
    /// 命令模板里打开结构化输出的参数从哪个版本起才有（`<命令> --version` 的版本号）：本机版本更低时去掉
    /// [`Builtin::json_flags`]，按原始输出运行。空表示不看版本
    json_since: &'static str,
    json_flags: &'static [&'static str],
}

/// 顺序就是默认 agent 的顺序：本机找到的第一个是默认（机器设置里可以改）
const BUILTIN: [Builtin; 4] = [
    Builtin {
        id: "dsh",
        name: "DeepSeek Harness",
        // --json：运行中把事件（说的话、工具调用、每一步的用量）写成 JSONL（0.2.2 的 headless 才有，见 json_since）
        cmd: &["dsh", "--profile", "headless", "--json", "--patch", "{patch}", "--permission-mode", "danger-full-access", "{prompt}"],
        model: "deepseek-v4.1-flash",
        // 桌面端没在「设置」里装命令行时，用 app 里自带的启动脚本
        also: &[
            "/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh",
            "~/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh",
        ],
        skills: &["~/.agents/skills"],
        project: &[".agents/skills"],
        git_bash: false,
        win: &["pwsh", "powershell", "cmd"],
        // DeepSeek Harness 的 headless profile 和补丁里没有能按次设置模型、思考强度的项：不显示这两项
        model_args: &[],
        effort_args: &[],
        efforts: &[],
        models: &[],
        // 按 0.2.2-rc.1 的 dsh-headless 核实过 --json；更旧的不认这个参数，去掉它按原始输出运行
        json_since: "0.2.2",
        json_flags: &["--json"],
    },
    Builtin {
        id: "claude",
        name: "Claude Code",
        // -p 默认只在结束时打印结果：用事件流，运行中每一步都写进日志（logfmt.rs 转写）
        cmd: &["claude", "-p", "{prompt}", "--dangerously-skip-permissions", "--output-format", "stream-json", "--verbose"],
        model: "opus-5.5",
        also: &["~/.local/bin/claude", "~/.claude/local/claude"],
        skills: &["~/.claude/skills"],
        // Claude Code 不读 .agents/skills
        project: &[".claude/skills"],
        git_bash: false,
        win: &["pwsh", "powershell", "git-bash", "wsl:*", "cmd"],
        // claude --help：--model <别名或全名>、--effort <low, medium, high, xhigh, max>
        model_args: &["--model", "{model}"],
        effort_args: &["--effort", "{effort}"],
        efforts: &["low", "medium", "high", "xhigh", "max"],
        models: &["opus", "sonnet", "haiku", "fable"],
        json_since: "",
        json_flags: &[],
    },
    Builtin {
        id: "pi",
        name: "pi",
        // -p 只在结束时打印最终回答：--mode json 同样跑完就退出，运行中写 JSONL 事件（logfmt.rs 转写）
        cmd: &["pi", "--mode", "json", "{prompt}"],
        model: "",
        also: &[],
        skills: &["~/.agents/skills", "~/.pi/agent/skills"],
        project: &[".agents/skills", ".pi/skills"],
        git_bash: true,
        // pi 不支持 cmd
        win: &["git-bash", "wsl:*", "pwsh", "powershell"],
        // pi --help：--model <模式或 provider/id>、--thinking <off, minimal, low, medium, high, xhigh, max>
        model_args: &["--model", "{model}"],
        effort_args: &["--thinking", "{effort}"],
        efforts: &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        models: &[],
        json_since: "",
        json_flags: &[],
    },
    Builtin {
        id: "codex",
        name: "Codex",
        // exec --json：运行中把事件（命令、改文件、说的话、每一轮的用量）写成 JSONL（docs/exec.md）
        cmd: &["codex", "exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "{prompt}"],
        model: "",
        also: &[],
        skills: &["$CODEX_HOME/skills"],
        project: &[".agents/skills"],
        git_bash: false,
        win: &["pwsh", "powershell", "git-bash", "wsl:*", "cmd"],
        // codex exec：-m <模型>、-c model_reasoning_effort=<minimal|low|medium|high>（配置项按 Codex 文档；本机不执行 codex 核对）
        model_args: &["-m", "{model}"],
        effort_args: &["-c", "model_reasoning_effort={effort}"],
        efforts: &["minimal", "low", "medium", "high"],
        models: &["gpt-5-codex", "gpt-5"],
        json_since: "",
        json_flags: &[],
    },
];

/// dsh 一次性派发时写给它的 profile 补丁：headless profile 默认仍挂着 goal 工具和
/// goal-round-driver，任务完成后会自动续轮，进程退出时把被终止的续轮当成最后一轮
/// （aborted/disposed），退出码变成 1。派发是一次性任务，关掉 goal 工具和续轮。
const DSH_HEADLESS_PATCH: &str = include_str!("dsh-headless.patch.yml");

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
/// 异常退出时评论里带的日志行数
const TAIL: usize = 20;
/// 单个本机日志超过这么大时，只留开头 1 MB 和最后 10 MB，中间写一行省略说明
const LOG_MAX: u64 = 50 * 1024 * 1024;
const LOG_HEAD: u64 = 1 * 1024 * 1024;
const LOG_TAIL: u64 = 10 * 1024 * 1024;
/// 本机日志保留几天（结束超过这么多天的删掉），配置文件 `[host] log_days` 可以改
const LOG_DAYS: i64 = 30;
/// 多久看一次要不要清理本机日志（启动时一次，之后每天一次）
const LOG_CLEAN_EVERY: Duration = Duration::from_secs(24 * 3600);
/// 一次最多读多少字节的新日志（多出来的只留最后这些）。Claude Code、pi 的事件流一行可能很长（工具结果整段在里面），
/// 转写后只剩一两行，所以读得多一些
const LOG_CHUNK: u64 = 4 * 1024 * 1024;
/// 白板目录的 git 状态（origin、当前分支）多久重新看一次：机器心跳 10 秒一次，不能每次都跑 git 子进程
const DIR_CHECK_EVERY: Duration = Duration::from_secs(300);

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
/// skills 共用库没变时多久再核对一次本机（有人手动改了、删了同步装的目录）
const SKILLS_EVERY: Duration = Duration::from_secs(600);

/// 远程管理请求处理完、退出主循环之后要做的事
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// 用同样的参数重新启动自己（重新读配置、重新检查）
    Restart,
    /// 桌面端运行：退出，交给桌面端升级
    DesktopUpgrade,
}

/// 由桌面端在后台运行（`BOARD_HOST_STDIN=1`）
fn under_desktop() -> bool {
    std::env::var("BOARD_HOST_STDIN").is_ok_and(|v| v.trim() == "1")
}

/// 上报间隔：`BOARD_HOST_INTERVAL`（秒，只给测试用），默认 10 秒
fn interval() -> Duration {
    let secs = std::env::var("BOARD_HOST_INTERVAL").ok().and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| *s > 0.0);
    secs.map(Duration::from_secs_f64).unwrap_or(Duration::from_millis(BEAT_MS as u64))
}

/// 命令行参数
pub struct Opts {
    pub name: Option<String>,
    /// `--dir <项目>/<白板>=<路径>`，可以有多个
    pub dirs: Vec<String>,
}

/* ------------------------------ 本机 agent ------------------------------ */

struct Spec {
    id: String,
    name: String,
    /// 模型名：派发的 agent 名字（`<agent 标识>-<机器名>/<模型名>`）的第二段；上报给服务端
    model: String,
    cmd: Vec<String>,
    /// 读用户级 skill 的目录（内置模板才有）
    skills: Vec<String>,
    /// 在仓库目录里读 skill 的相对目录（内置模板才有）
    project: Vec<String>,
    /// Windows 上启动时把 Git Bash 的目录放到 PATH 前面
    git_bash: bool,
    /// 在它支持、本机有的每个启动环境里的检查结果，按默认选择的顺序
    checks: Vec<Check>,
    /// 找到了命令、却看不到 `board-skills` 的启动环境 → 该装到哪里（启动时由 [`Spec::check_skills`] 检查一次）
    no_skill: BTreeMap<String, String>,
    /// 选了模型、思考强度时追加的参数；能选的档位、常用模型（见 [`Builtin`]）
    model_args: Vec<String>,
    effort_args: Vec<String>,
    efforts: Vec<String>,
    models: Vec<String>,
    /// 内置模板的结构化输出参数要求的最低版本和参数（见 [`Builtin::json_since`]）；配置文件覆盖了命令时没有
    json_gate: Option<(String, Vec<String>)>,
    /// 共用配置里有这个标识的模板，但本机配置文件里有同标识的 `[agents.*]`：用的是本机的（上报给界面标出来）
    local_override: bool,
}

/// 版本号前面的数字：`0.2.2-rc.1` → [0, 2, 2]、`2.1.311 (Claude Code)` → [2, 1, 311]；没有数字时为空
fn version_nums(v: &str) -> Vec<u64> {
    let start = v.find(|c: char| c.is_ascii_digit()).unwrap_or(v.len());
    let head: String = v[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    head.split('.').filter_map(|x| x.parse().ok()).collect()
}

/// `have` 比 `need` 旧（只比数字部分；看不出版本时当作够新）
fn older_than(have: &str, need: &str) -> bool {
    let (a, b) = (version_nums(have), version_nums(need));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let n = a.len().max(b.len());
    let pad = |v: &[u64]| (0..n).map(|i| v.get(i).copied().unwrap_or(0)).collect::<Vec<_>>();
    pad(&a) < pad(&b)
}

/// 启动 agent 的参数（命令模板里第一个之后的）：`{prompt}`、`{patch}` 换成提示词和补丁路径；
/// 机器设置里选了模型、思考强度时，把对应的参数插到 `{prompt}` 那一个前面（没有 `{prompt}` 时放在最后）。
/// 都没选时和原来完全一样
fn agent_args(cmd: &[String], model_args: &[String], effort_args: &[String], model: Option<&str>, effort: Option<&str>, prompt: &str, patch: &str) -> Vec<String> {
    let mut extra: Vec<String> = vec![];
    if let Some(m) = model.filter(|_| !model_args.is_empty()) {
        extra.extend(model_args.iter().map(|a| a.replace("{model}", m)));
    }
    if let Some(e) = effort.filter(|_| !effort_args.is_empty()) {
        extra.extend(effort_args.iter().map(|a| a.replace("{effort}", e)));
    }
    let mut out: Vec<String> = vec![];
    let mut placed = extra.is_empty();
    for a in cmd.iter().skip(1) {
        if !placed && a.contains("{prompt}") {
            out.append(&mut extra);
            placed = true;
        }
        out.push(a.replace("{prompt}", prompt).replace("{patch}", patch));
    }
    out.append(&mut extra);
    out
}

impl Spec {
    /// 默认的启动环境：按顺序第一个找到命令的
    fn default_check(&self) -> Option<&Check> {
        self.checks.iter().find(|c| c.found)
    }

    /// 在这个启动环境里用的命令模板：内置模板的结构化输出参数要求更新的版本、本机的又更旧时去掉它们
    fn cmd_for(&self, check: &Check) -> Vec<String> {
        match &self.json_gate {
            Some((since, flags)) if older_than(&check.version, since) => self.cmd.iter().filter(|a| !flags.contains(a)).cloned().collect(),
            _ => self.cmd.clone(),
        }
    }

    fn report(&self) -> HostAgent {
        let def = self.default_check();
        HostAgent {
            id: self.id.clone(),
            name: self.name.clone(),
            version: def.map(|c| c.version.clone()).unwrap_or_default(),
            found: def.is_some(),
            envs: self.checks.iter().map(|c| EnvCheck { no_skill: self.no_skill.get(&c.env).cloned(), ..c.report() }).collect(),
            env: None,
            no_skill: def.and_then(|c| self.no_skill.get(&c.env).cloned()),
            model: self.model.clone(),
            opts: Some(AgentOpts {
                model: !self.model_args.is_empty(),
                models: if self.model_args.is_empty() { vec![] } else { self.models.clone() },
                efforts: if self.effort_args.is_empty() { vec![] } else { self.efforts.clone() },
            }),
            run_model: None,
            run_effort: None,
            run_env: None,
            local_override: self.local_override,
        }
    }

    /// 在每个找到了命令的启动环境里看一次 `board-skills`，看不到的记下该装到哪里（终端和界面上的提醒用同一句）
    fn check_skills(&mut self, envs: &[Env], dirs: &[(String, PathBuf)]) {
        self.no_skill = envs
            .iter()
            .filter(|e| self.misses_skill(e, dirs))
            .map(|e| {
                let user: Vec<String> = self.skills.iter().map(|d| format!("{d}/{SKILL}")).collect();
                let place = if e.wsl().is_some() { format!("{}里的 ", e.name) } else { String::new() };
                let repo: Vec<String> = self.project.iter().map(|d| format!("仓库的 {d}/{SKILL}")).collect();
                (e.id.clone(), format!("{place}{}", [user, repo].concat().join(" 或 ")))
            })
            .collect();
    }

    /// 在 env 里找到了、但它读 skill 的目录里都没有 `board-skills/SKILL.md`。
    /// 用户级目录来自内置模板（WSL 里看 WSL 用户目录下的）；项目级目录看每块白板对应的仓库目录里这个 agent 会读的那几个
    /// （例如 Claude Code 只读 .claude/skills，不读 .agents/skills）
    fn misses_skill(&self, env: &Env, dirs: &[(String, PathBuf)]) -> bool {
        if self.skills.is_empty() || !self.checks.iter().any(|c| c.env == env.id && c.found) {
            return false;
        }
        let has = |d: &Path| d.join(SKILL).join("SKILL.md").is_file();
        !envs::has_skill(env, &self.skills, SKILL) && !dirs.iter().any(|(_, p)| self.project.iter().any(|s| has(&p.join(s))))
    }
}

/// 一个模板（内置的，或配置文件里的）
struct Template {
    id: String,
    name: String,
    /// 模型名（派发名字的第二段）；配置文件里的 `model` 覆盖内置模板的
    model: String,
    cmd: Vec<String>,
    /// 命令不在 PATH 里时再看的位置（只在直接启动、登录 shell 里用）
    also: Vec<String>,
    skills: Vec<String>,
    project: Vec<String>,
    git_bash: bool,
    /// 支持的启动环境，按默认选择的顺序
    order: Vec<String>,
    model_args: Vec<String>,
    effort_args: Vec<String>,
    efforts: Vec<String>,
    models: Vec<String>,
    json_gate: Option<(String, Vec<String>)>,
    /// 来自服务端下发的模板（共用配置）
    server: bool,
    local_override: bool,
}

/// 内置模板、服务端下发的共用模板（共用配置「agent 模板」）、配置文件里的模板，后面的覆盖前面同标识的；
/// 顺序：内置的在前，共用模板里新加的按标识排在后面，配置文件里新加的再按文件顺序排在后面。
/// 本机配置文件覆盖了共用模板的，标上 `local_override`。每个 agent 在它支持、本机有的启动环境里各检查一次
fn agent_specs(conf: &[AgentConf], server: &[AgentTemplate], envs: &[Env]) -> Result<Vec<Spec>> {
    let strs = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let order = |win: &[&str]| if cfg!(windows) { strs(win) } else { strs(envs::UNIX_ORDER) };
    let mut out: Vec<Template> = BUILTIN
        .iter()
        .map(|b| Template {
            id: b.id.to_string(),
            name: b.name.to_string(),
            model: b.model.to_string(),
            cmd: strs(b.cmd),
            also: strs(b.also),
            skills: strs(b.skills),
            project: strs(b.project),
            git_bash: b.git_bash,
            order: order(b.win),
            model_args: strs(b.model_args),
            effort_args: strs(b.effort_args),
            efforts: strs(b.efforts),
            models: strs(b.models),
            json_gate: (!b.json_since.is_empty()).then(|| (b.json_since.to_string(), strs(b.json_flags))),
            server: false,
            local_override: false,
        })
        .collect();
    let order_of = |win: &[String]| if cfg!(windows) { win.to_vec() } else { strs(envs::UNIX_ORDER) };
    for t in server {
        // 服务端校验过；旧版服务端、坏数据里不对的跳过
        if check_agent_id(&t.id).is_err() || t.cmd.is_empty() {
            continue;
        }
        match out.iter_mut().find(|x| x.id == t.id) {
            Some(x) => {
                if x.cmd.first() != t.cmd.first() {
                    x.also.clear();
                }
                if x.cmd != t.cmd {
                    x.json_gate = None;
                }
                x.cmd = t.cmd.clone();
                if !t.name.is_empty() {
                    x.name = t.name.clone();
                }
                if !t.model.is_empty() {
                    x.model = t.model.clone();
                }
                if !t.model_args.is_empty() {
                    x.model_args = t.model_args.clone();
                }
                if !t.effort_args.is_empty() {
                    x.effort_args = t.effort_args.clone();
                }
                if !t.efforts.is_empty() {
                    x.efforts = t.efforts.clone();
                }
                if !t.win.is_empty() {
                    x.order = order_of(&t.win);
                }
                x.server = true;
            }
            None => out.push(Template {
                id: t.id.clone(),
                name: if t.name.is_empty() { t.id.clone() } else { t.name.clone() },
                model: t.model.clone(),
                cmd: t.cmd.clone(),
                also: vec![],
                skills: vec![],
                project: vec![],
                git_bash: false,
                order: if t.win.is_empty() { order(envs::CONF_WIN_ORDER) } else { order_of(&t.win) },
                model_args: t.model_args.clone(),
                effort_args: t.effort_args.clone(),
                efforts: if t.efforts.is_empty() { ["low", "medium", "high"].map(String::from).to_vec() } else { t.efforts.clone() },
                models: vec![],
                json_gate: None,
                server: true,
                local_override: false,
            }),
        }
    }
    for a in conf {
        check_agent_id(&a.id).map_err(BoardError::usage)?;
        match out.iter_mut().find(|x| x.id == a.id) {
            Some(x) => {
                // 换了命令：内置的备用位置不再适用
                if x.cmd.first() != a.cmd.first() {
                    x.also.clear();
                }
                // 命令是配置文件写的：照原样用，不再按版本增删参数
                if x.cmd != a.cmd {
                    x.json_gate = None;
                }
                // 本机配置覆盖了共用配置里的模板：以本机为准，上报时标出来
                x.local_override = x.server;
                x.cmd = a.cmd.clone();
                if let Some(n) = &a.name {
                    x.name = n.clone();
                }
                if let Some(m) = &a.model {
                    x.model = m.clone();
                }
                if let Some(v) = &a.model_args {
                    x.model_args = v.clone();
                }
                if let Some(v) = &a.effort_args {
                    x.effort_args = v.clone();
                }
                if let Some(v) = &a.efforts {
                    x.efforts = v.clone();
                }
            }
            None => out.push(Template {
                id: a.id.clone(),
                name: a.name.clone().unwrap_or_else(|| a.id.clone()),
                model: a.model.clone().unwrap_or_default(),
                cmd: a.cmd.clone(),
                also: vec![],
                skills: vec![],
                project: vec![],
                git_bash: false,
                order: order(envs::CONF_WIN_ORDER),
                model_args: a.model_args.clone().unwrap_or_default(),
                effort_args: a.effort_args.clone().unwrap_or_default(),
                efforts: a.efforts.clone().unwrap_or_else(|| ["low", "medium", "high"].map(String::from).to_vec()),
                models: vec![],
                json_gate: None,
                server: false,
                local_override: false,
            }),
        }
    }
    // 每个 agent 在每个环境里的检查一起跑：一个慢的不拖住启动
    Ok(std::thread::scope(|s| {
        let jobs: Vec<_> = out
            .into_iter()
            .map(|t| {
                let checks: Vec<_> = envs::supported(&t.order, envs)
                    .into_iter()
                    .map(|e| {
                        let (cmd, also) = (t.cmd.first().cloned().unwrap_or_default(), t.also.clone());
                        s.spawn(move || envs::probe(e, &cmd, &also))
                    })
                    .collect();
                (t, checks)
            })
            .collect();
        jobs.into_iter()
            .map(|(t, checks)| Spec {
                id: t.id,
                name: t.name,
                model: t.model,
                cmd: t.cmd,
                skills: t.skills,
                project: t.project,
                git_bash: t.git_bash,
                checks: checks.into_iter().filter_map(|j| j.join().ok()).collect(),
                no_skill: BTreeMap::new(),
                model_args: t.model_args,
                effort_args: t.effort_args,
                efforts: t.efforts,
                models: t.models,
                json_gate: t.json_gate,
                local_override: t.local_override,
            })
            .collect()
    }))
}

/// board host 自己的目录：`~/.board/host`
fn host_dir() -> Result<PathBuf> {
    let d = config::board_dir()?.join("host");
    std::fs::create_dir_all(d.join("logs")).map_err(|e| BoardError::usage(format!("无法创建 {}：{e}", d.display())))?;
    Ok(d)
}

/// 写 dsh 一次性派发的 profile 补丁（内容固定，每次派发覆盖写）
fn write_dsh_patch(path: &Path) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, DSH_HEADLESS_PATCH)
}

/// 写一个只给这次派发用的配置文件：服务端地址和令牌，只有自己能读（0600）
fn write_run_config(path: &Path, url: &str, token: &str) -> Result<()> {
    board_core::config::write_new(path, |c| {
        c.set(config::URL, url);
        c.set(config::TOKEN, token);
    })
}

/* ------------------------------ 进程 ------------------------------ */

/// 退出码；被信号结束时按 shell 的习惯记成 128 + 信号（kill -9 → 137）
fn exit_code(s: std::process::ExitStatus) -> Option<i64> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = s.signal() {
            return Some(128 + sig as i64);
        }
    }
    s.code().map(|c| c as i64)
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

/* ------------------------------ 一次在跑的派发 ------------------------------ */

struct Run {
    d: Dispatch,
    child: Child,
    /// 启动环境
    env: Env,
    /// WSL 里 agent 的进程号写在这里（BOARD_PIDFILE），结束时删除
    pidfile: Option<PathBuf>,
    log: PathBuf,
    /// 这次派发的配置文件（BOARD_CONFIG），结束时删除
    conf: PathBuf,
    /// agent 的进程号和启动时间（`<派发 ID>.proc`）：host 异常退出后重启时据此结束留下的 agent，结束时删除
    procfile: PathBuf,
    /// 日志已经上传到的位置（字节）
    sent: u64,
    /// 日志按哪种事件流转写后上传；只有原始输出的 agent 原样上传
    fmt: Option<Stream>,
    /// 结构化事件（会话「过程」，events.rs）：解析器（只有原始输出的 agent 没有）
    parser: Option<events::Parser>,
    /// 日志已经解析到的位置（字节）
    ev_read: u64,
    /// 解析出来、还没传上去的事件（连不上服务端时攒着，最多 2000 条）
    ev_queue: Vec<RunEvent>,
    /// 最后几个事件（工具调用和结果合成一条）：异常退出的评论里写它们
    ev_recent: VecDeque<RunEvent>,
    /// 这次派发的格式还没报给服务端（开始时、会话换到下一张时）
    fmt_due: bool,
    /// SIGTERM 的时间
    term_at: Option<Instant>,
    killed: bool,
    spawned: Instant,
    /// 服务端已经结束了这次派发（机器掉线后卡片被释放）才停的：agent 退出后看看它的目录里有没有没推的改动
    lost: bool,
    /// 已经按机器的兜底设置要求停止：原因（「运行超过 3 小时…」）。停止流程走完后上报结束，结果「超时」
    timeout: Option<String>,
    /// 最近一次有输出（日志变长）的时间：判断「没输出多久算卡住」
    output_at: Instant,
    /// 上一次看到的日志长度（判断有没有新的输出）
    seen_len: u64,
}

impl Run {
    /// 结束 agent 进程树（hard：强制）。WSL：在 WSL 里结束 agent 的进程组，强制时再结束 wsl.exe 的进程树
    fn stop(&self, hard: bool) {
        if self.env.wsl().is_some() {
            let pid = self.pidfile.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.trim().parse::<u32>().ok());
            match pid {
                Some(pid) => envs::wsl_kill(&self.env, pid, hard),
                // 还没写进程号（刚启动）：直接结束 wsl.exe
                None => kill_tree(self.child.id(), true),
            }
            if hard {
                kill_tree(self.child.id(), true);
            }
        } else {
            kill_tree(self.child.id(), hard);
        }
    }

    /// 主进程已经退出：结束同一进程组里剩下的进程（agent 起的子进程）。
    ///
    /// Windows 上不做：进程号可能已经被复用，`taskkill /T` 会结束别的进程树。
    /// 调用前要先把主进程回收（agent 自己退出时 `reap` 已经 `try_wait` 过，board host 退出时先 `wait()`）：
    /// 不然 `kill` / `taskkill` 打的还是主进程自己
    fn kill_rest(&self) {
        if let Some(env) = self.env.wsl().map(|_| &self.env) {
            let pid = self.pidfile.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.trim().parse::<u32>().ok());
            if let Some(pid) = pid {
                envs::wsl_kill(env, pid, true);
            }
        } else if cfg!(unix) {
            kill_tree(self.child.id(), true);
        }
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.conf);
        let _ = std::fs::remove_file(&self.procfile);
        if let Some(p) = &self.pidfile {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// 收尾时记一笔结束，等 `report_ended()` 上报给服务端：取最后几行做错误摘录，再截断过大的日志。
///
/// 截断要在 agent 确实不在了之后：它还活着时换掉文件内容，下次写入会从旧位置接着写、中间留一段空洞。
/// `how` 是给服务端的结果（`exit`、`host_exit`）；兜底停止过（`Run.timeout` 有值）的无论哪种都报 `timeout`，
/// 原因照原样带上
fn ended_of(r: &Run, code: Option<i64>, how: &'static str, secrets: &[&str]) -> Ended {
    let tail = tail_lines(&r.log, TAIL, r.fmt).iter().map(|l| redact::redact(l, secrets)).collect();
    let _ = truncate_log(&r.log, LOG_HEAD, LOG_TAIL, LOG_MAX);
    let (how, why) = if r.timeout.is_some() { ("timeout", r.timeout.clone()) } else { (how, None) };
    Ended { id: r.d.id, code, tail, events: recent_lines(r), how, why }
}

/// 一次派发收尾后给调用方打印用的信息
struct RunSummary {
    board: String,
    card: i64,
    name: Option<String>,
    /// 派发已经被服务端结束（机器掉线）
    lost: bool,
    /// 兜底停止的原因
    why: Option<String>,
}

/// 还没上报成功的结束（连不上服务端时下一轮再报）
struct Ended {
    id: i64,
    code: Option<i64>,
    tail: Vec<String>,
    /// 最后 10 个事件的文字（RunEvent::line）
    events: Vec<String>,
    how: &'static str,
    /// how = timeout 时给人看的原因
    why: Option<String>,
}

/// 读日志里 from 之后新增的完整行，返回（行，新的位置）；last 为 true 时最后不完整的一行也算
fn new_lines(path: &Path, from: u64, last: bool) -> (Vec<String>, u64) {
    let Ok(mut f) = std::fs::File::open(path) else { return (vec![], from) };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len <= from {
        return (vec![], from);
    }
    let start = from.max(len.saturating_sub(LOG_CHUNK));
    if f.seek(SeekFrom::Start(start)).is_err() {
        return (vec![], from);
    }
    let mut buf = Vec::new();
    if f.take(len - start).read_to_end(&mut buf).is_err() {
        return (vec![], from);
    }
    // 跳过了一部分：从下一个整行开始
    let mut skip = 0;
    if start > from {
        skip = buf.iter().position(|b| *b == b'\n').map(|i| i + 1).unwrap_or(buf.len());
    }
    let end = if last { buf.len() } else { buf.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0) };
    if end <= skip {
        return (vec![], if start > from { start + skip as u64 } else { from });
    }
    let text = String::from_utf8_lossy(&buf[skip..end]);
    let lines = text.lines().map(|l| l.trim_end_matches('\r').to_string()).collect();
    (lines, start + end as u64)
}

/// 登录 shell 自己打的启动、退出提示（见 [`board_core::hosts::shell_noise`]）：不是检查命令、也不是 agent 的输出
fn shell_noise(line: &str) -> bool {
    board_core::hosts::shell_noise(line)
}

/// 日志最后 n 行（转写之后的）
fn tail_lines(path: &Path, n: usize, fmt: Option<Stream>) -> Vec<String> {
    let (lines, _) = new_lines(path, 0, true);
    let lines: Vec<String> = logfmt::render(fmt, lines).into_iter().filter(|l| !l.trim().is_empty() && !shell_noise(l)).collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

/// 要上传的行：转写，只留最后 500 行（服务端本来也只保留这么多）
fn upload_lines(lines: Vec<String>, fmt: Option<Stream>) -> Vec<String> {
    let lines: Vec<String> = logfmt::render(fmt, lines).into_iter().filter(|l| !shell_noise(l)).collect();
    lines[lines.len().saturating_sub(LOG_KEEP)..].to_vec()
}

/// 要上传的行：转写，只留最后 500 行，再脱敏（令牌、密钥不上传到服务端）
fn upload_lines_redacted(lines: Vec<String>, fmt: Option<Stream>, secrets: &[&str]) -> Vec<String> {
    upload_lines(lines, fmt).iter().map(|l| redact::redact(l, secrets)).collect()
}

/// 解析日志里新增的行，得到结构化事件（只有原始输出的 agent 只挪位置）。last：最后不完整的一行也算（agent 已经退出）。
/// 事件里的文字先脱敏，再放进待上传的队列和「最后几个事件」。登录 shell 自己打的提示不是 agent 的输出，直接丢掉
fn read_events(r: &mut Run, last: bool, secrets: &[&str]) {
    let (lines, to) = new_lines(&r.log, r.ev_read, last);
    r.ev_read = to;
    let Some(p) = r.parser.as_mut() else { return };
    let at = now_ms();
    for line in lines.into_iter().filter(|l| !shell_noise(l)) {
        for mut e in p.feed(&line, at) {
            redact_event(&mut e, secrets);
            remember(&mut r.ev_recent, &e);
            r.ev_queue.push(e);
        }
    }
    let keep = board_core::runlog::EVENTS_KEEP as usize;
    if r.ev_queue.len() > keep {
        let drop = r.ev_queue.len() - keep;
        r.ev_queue.drain(..drop);
    }
}

/// 事件里的文字脱敏（令牌、密钥不上传到服务端，也不写进评论）
fn redact_event(e: &mut RunEvent, secrets: &[&str]) {
    for f in [&mut e.text, &mut e.tool, &mut e.args, &mut e.out] {
        if let Some(x) = f.as_mut() {
            *x = redact::redact(x, secrets);
        }
    }
}

/// 记进「最后几个事件」：工具结果合进同一个调用
fn remember(recent: &mut VecDeque<RunEvent>, e: &RunEvent) {
    if e.kind == "tool" && e.call.is_some() {
        if let Some(prev) = recent.iter_mut().rev().find(|p| p.kind == "tool" && p.call == e.call) {
            prev.merge(e);
            return;
        }
    }
    recent.push_back(e.clone());
    while recent.len() > board_core::runlog::EVENTS_TAIL {
        recent.pop_front();
    }
}

/// 异常退出评论里的「最后 10 个事件」
fn recent_lines(r: &Run) -> Vec<String> {
    r.ev_recent.iter().map(RunEvent::line).collect()
}

/// 日志文件的权限设成 0600（只有自己能读）。Windows 上不设：ACL 不是这个模型，用户目录本来就只有本人能进
fn restrict_log(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// 新建（或清空）一个日志文件，权限 0600。返回标准输出、标准错误两份句柄
fn create_log(path: &Path) -> std::io::Result<(std::fs::File, std::fs::File)> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    #[cfg(unix)]
    let f = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?
    };
    #[cfg(not(unix))]
    let f = std::fs::File::create(path)?;
    restrict_log(path);
    let err = f.try_clone()?;
    Ok((f, err))
}

/// 太大了只留开头和结尾：超过 `max` 字节时，留下开头 `head` 字节、最后 `tail` 字节，中间换成一行省略说明。
/// 返回截断后的长度（没截断时 None）
fn truncate_log(path: &Path, head: u64, tail: u64, max: u64) -> std::io::Result<Option<u64>> {
    let len = std::fs::metadata(path)?.len();
    if len <= max {
        return Ok(None);
    }
    let mut f = std::fs::File::open(path)?;
    let mut first = Vec::new();
    (&mut f).take(head).read_to_end(&mut first)?;
    f.seek(SeekFrom::End(-(tail as i64)))?;
    let mut last = Vec::new();
    f.read_to_end(&mut last)?;
    // 别从半行开始、也别在半行结束
    if let Some(i) = first.iter().rposition(|b| *b == b'\n') {
        first.truncate(i + 1);
    }
    if let Some(i) = last.iter().position(|b| *b == b'\n') {
        last.drain(..=i);
    }
    let note = format!("\n[日志太大（{} 字节），中间 {} 字节已省略：只保留开头 {} KB 和最后 {} KB]\n", len, len - first.len() as u64 - last.len() as u64, head / 1024, tail / 1024);
    let mut out = Vec::with_capacity(first.len() + note.len() + last.len());
    out.extend_from_slice(&first);
    out.extend_from_slice(note.as_bytes());
    out.extend_from_slice(&last);
    std::fs::write(path, &out)?;
    restrict_log(path);
    Ok(Some(out.len() as u64))
}

/// 本机派发日志清理：`~/.board/host/logs` 里结束超过 `days` 天的删掉。`keep` 是正在跑的派发 ID，
/// 它们的日志不删（改过文件时间的、还在写的都按结束时间算：看修改时间）。
/// 返回删掉的个数。日志文件名是派发 ID 或 `ready-*`、`clone-*`、`update` 这类
fn clean_logs(dir: &Path, days: i64, keep: &[i64], now: std::time::SystemTime) -> usize {
    let cutoff = now.checked_sub(Duration::from_secs((days.max(1) as u64) * 24 * 3600));
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".log") {
            continue;
        }
        // 正在跑的派发：文件名就是派发 ID
        let id: Option<i64> = name.trim_end_matches(".log").parse().ok();
        if id.is_some_and(|id| keep.contains(&id)) {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let Ok(modified) = m.modified() else { continue };
        if cutoff.is_some_and(|c| modified < c) && std::fs::remove_file(&p).is_ok() {
            removed += 1;
        }
    }
    removed
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

/// 结束上报失败：派发不存在这类明确的 4xx 不用再试；其余错误（连不上、5xx 等）留到下一轮
fn permanent_end_error(code: ErrorCode) -> bool {
    matches!(code, ErrorCode::NotFound | ErrorCode::BoardNotFound | ErrorCode::NoSuchHost | ErrorCode::Usage)
}

/// 定时领取：这一分钟到点、机器没暂停、也没有正在进行的一轮
/// 会话（同一个 agent 进程）正在跑的那一张：board next 之后服务端开了新的派发，session 相同、ID 更大
/// 旧版服务端不发 session（为 0）：不跟随
fn successor(d: &Dispatch, running: &[Dispatch]) -> Option<Dispatch> {
    if d.session == 0 {
        return None;
    }
    running.iter().filter(|x| x.session == d.session && x.id > d.id).max_by_key(|x| x.id).cloned()
}

/// 这一分钟（本地时间）有哪条定时表达式到点。旧版服务端发来的 HH:MM 按「每天这个时间」算；解析不了的跳过
fn cron_due(at: &chrono::DateTime<chrono::Local>, cron_on: bool, cron: &[String], paused: bool, round: bool) -> bool {
    use chrono::{Datelike, Timelike};
    if !cron_on || paused || round {
        return false;
    }
    cron.iter().filter_map(|t| board_core::cron::normalize(t).ok()).filter_map(|t| board_core::cron::Cron::parse(&t).ok()).any(|c| {
        c.matches(at.minute(), at.hour(), at.day(), at.month(), at.weekday().num_days_from_sunday())
    })
}

/// 定时领取的一个 tick：同一分钟只处理一次。不用领取时记下这一分钟；要领取时调 `start`，
/// 成功才记下（失败了留到同一分钟的下一个 tick 重试）。返回这一 tick 是否调了 `start`
fn cron_tick(m: &str, last: &mut String, due: bool, start: impl FnOnce() -> bool) -> bool {
    if m == last {
        return false;
    }
    if !due {
        *last = m.to_string();
        return false;
    }
    if start() {
        *last = m.to_string();
    }
    true
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
    dirs: Vec<(String, PathBuf)>,
    /// 配置里写的目录原文（`~` 不展开），上报和服务端写回配置都用它
    dir_raw: Vec<(String, String)>,
    /// 命令行 `--dir` 固定的白板：界面上只读
    fixed_dirs: BTreeMap<String, bool>,
    /// 已经从服务端应用到的目录设置版本
    dirs_rev: i64,
    /// 每块白板目录的 git 状态（最近一次看的结果和看的时间）：心跳时沿用，隔 DIR_CHECK_EVERY 再跑 git
    dir_checks: Vec<DirCheck>,
    /// 服务端请求的克隆任务（以及刚结束的）
    clones: Vec<CloneRun>,
    /// 每块白板的环境检查（正在跑的、最近一次的结果）
    readies: Vec<ReadyRun>,
    /// 下一次取到任务时把所有白板的环境检查重跑一遍（启动、重新读配置之后）
    ready_all: bool,
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
    /// 机器名、白板与目录跟着配置文件变（命令行没给 `--name` / `--dir`）
    follow_name: bool,
    follow_dirs: bool,
    /// 上一次读到的配置文件 `[host]`（机器名、白板与目录原文、日志保留天数）：没变就不重新校验
    conf_seen: (Option<String>, Vec<(String, String)>, Option<i64>),
    /// 正在后台做的自检（请求序号、线程）
    selftest: Option<(i64, std::thread::JoinHandle<Vec<SelfTestItem>>)>,
    /// 做完了、还没回报成功的自检结果（连不上服务端时下一轮再报）
    selftest_out: Option<(i64, Vec<SelfTestItem>)>,
    /// 已经做完并回报的自检请求：取到的任务里还是它时不再做
    selftest_seen: Option<i64>,
    /// 自检启动的子进程：退出时结束它们
    selftest_pids: selftest::Pids,
    /// skills 共用库：正在后台做的同步
    skill_job: Option<std::thread::JoinHandle<skills::Synced>>,
    /// 最近一次同步的结果（上报里带上；新版 board host 总是带，空表示还没同步完）
    skill_states: Vec<board_core::skills::SkillState>,
    /// 上一次同步时的库和目录：变了就再同步一次
    skill_key: Option<String>,
    skill_at: Instant,
    /// 重新检查、重新读配置后马上再同步一次（skills 和 MCP 一起）
    skill_force: bool,
    /// MCP 共用配置：正在后台做的同步、最近的结果、上一次的索引和 agent、时间
    mcp_job: Option<std::thread::JoinHandle<mcpsync::Synced>>,
    mcp_states: Vec<board_core::mcp::McpState>,
    mcp_key: Option<String>,
    mcp_at: Instant,
    mcp_force: bool,
    /// 共用配置里的 agent 模板（服务端随任务下发）：已经合进本机 agent 的那一份
    templates: Vec<AgentTemplate>,
    /// 马上上报一次（共用模板变了、重新检查完 agent 之后）
    beat_soon: bool,
}

/// 一块白板目录的 git 状态（目录在不在、是不是 git 仓库、origin、当前分支）和看的时间。
/// 目录状态不看文件系统时间：改分支、换远端都不碰目录的修改时间，按时间判断会看到旧的
struct DirCheck {
    path: PathBuf,
    status: &'static str,
    origin: Option<String>,
    branch: Option<String>,
    at: Instant,
}

/// 一块白板的环境检查（PRD §9.1「环境检查」）：在白板目录里跑白板设置的那条命令，退出码 0 算通过。
/// 启动时、重新读配置时、每天一次、白板的检查命令（或目录）改了之后各跑一次，最长 2 分钟
struct ReadyRun {
    board: String,
    cmd: String,
    dir: PathBuf,
    /// 正在跑的进程和开始的时间
    child: Option<(Child, Instant)>,
    log: PathBuf,
    /// 最近一次跑完的结果（命令可能是改之前的：服务端只认命令对得上的）
    last: Option<DirReady>,
    /// 上一次跑的时候目录不存在：目录有了（克隆完）就重跑
    missing: bool,
}

/// 环境检查每天重跑一次
const READY_EVERY_MS: i64 = 24 * 3600 * 1000;

impl ReadyRun {
    /// 结束正在跑的检查（连同它起的子进程）
    fn kill(&mut self) {
        if let Some((mut child, _)) = self.child.take() {
            kill_tree(child.id(), true);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct CloneRun {
    board: String,
    url: String,
    /// queued | running | done | failed
    state: String,
    message: String,
    updated_at: i64,
    child: Option<Child>,
    log: PathBuf,
}

/// `board host`：一直运行，直到 Ctrl+C
pub fn run(opts: Opts, p: &Paint) -> Result<()> {
    let file = config::File::load()?;
    let conf = file.host()?;
    let follow_name = opts.name.as_deref().is_none_or(|s| s.trim().is_empty());
    let follow_dirs = opts.dirs.is_empty();
    // 机器名
    let name = match opts.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => conf_name(conf.name.as_deref()),
    };
    check_host_name(&name).map_err(BoardError::usage)?;
    // 白板与目录：命令行给了就只用命令行的
    let raw: Vec<(String, String)> = if opts.dirs.is_empty() {
        conf.dirs.clone()
    } else {
        opts.dirs
            .iter()
            .map(|d| {
                d.split_once('=')
                    .map(|(b, p)| (b.trim().to_string(), p.trim().to_string()))
                    .filter(|(b, p)| !b.is_empty() && !p.is_empty())
                    .ok_or_else(|| BoardError::usage(format!("--dir 要写成 <项目>/<白板>=<路径>：{d}")))
            })
            .collect::<Result<_>>()?
    };
    let (dirs, dir_raw) = parse_dirs(raw)?;
    let log_days = conf.log_days.unwrap_or(LOG_DAYS);
    let fixed_dirs = if follow_dirs {
        BTreeMap::new()
    } else {
        dir_raw.iter().map(|(b, _)| (b.clone(), true)).collect()
    };
    let be = Backend::open_host()?;
    let envs = envs::discover();
    // 共用配置里的 agent 模板随第一次取到的任务下发，到时再合进来（见 Host::apply_templates）
    let mut agents = agent_specs(&conf.agents, &[], &envs)?;
    for a in &mut agents {
        a.check_skills(&envs, &dirs);
    }
    let os = sysinfo::System::long_os_version()
        .ok()
        .or_else(|| sysinfo::System::name().ok())
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    let hd = host_dir()?;
    let pidfile = hd.join(format!("{name}.pid"));
    // 上一次在这台机器上用这个名字的 board host：进程不在了就接回它的名字
    let resume = std::fs::read_to_string(&pidfile).ok().and_then(|s| {
        let mut it = s.split_whitespace();
        let proc = Proc::parse(&format!("{} {}", it.next()?, it.next()?))?;
        let inst = it.next()?.to_string();
        let alive = Procs::new().is_some_and(|mut ps| ps.alive(proc) && ps.is_board(proc));
        (!alive).then_some(inst)
    });
    let instance = format!("{:x}{:x}", now_ms(), std::process::id() as u64 * 2654435761 % 0xffff_ffff);
    let run = hd.join("run").join(&name);
    let exe = std::env::current_exe().map_err(|e| BoardError::usage(format!("找不到 board 自己的路径：{e}")))?;
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
        dirs,
        dir_raw,
        fixed_dirs,
        dirs_rev: 0,
        dir_checks: vec![],
        clones: vec![],
        readies: vec![],
        ready_all: true,
        asked: None,
        ask_error: std::env::var("BOARD_HOST_ASK_ERROR").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        updating: None,
        after: None,
        pidfile,
        logs: hd.join("logs"),
        log_days,
        runs: vec![],
        ended: vec![],
        work: None,
        work_at: Instant::now(),
        hold: false,
        last_err: String::new(),
        last_ok: Instant::now(),
        offline_warned: false,
        follow_name,
        follow_dirs,
        conf_seen: (conf.name.clone(), conf.dirs.clone(), conf.log_days),
        selftest: None,
        selftest_out: None,
        selftest_seen: None,
        selftest_pids: selftest::Pids::default(),
        skill_job: None,
        skill_states: vec![],
        skill_key: None,
        skill_at: Instant::now(),
        skill_force: false,
        mcp_job: None,
        mcp_states: vec![],
        mcp_key: None,
        mcp_at: Instant::now(),
        mcp_force: false,
        templates: vec![],
        beat_soon: false,
    };
    on_signals();
    // 第一次上报：名字被占用、令牌不对、连不上，都直接报错退出。在这之前不碰 run 目录：同名的 host 可能还在跑
    let w = h.beat(false)?;
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

/// 用同样的参数重新启动自己（远程「重启 board host」、命令行升级装好新版本之后）。
/// macOS、Linux 原地换进程（进程号、桌面端接的标准输入都不变）；Windows 上桌面端运行时退出码 EXIT_RESTART 交给桌面端重启，
/// 命令行运行时启动一个新的、自己退出
fn restart_self(exe: &Path, p: &Paint) -> Result<()> {
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

/// 配置文件里的机器名；没写时用本机主机名（转成合法的机器名）
fn conf_name(name: Option<&str>) -> String {
    match name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => host_name_from(&sysinfo::System::host_name().unwrap_or_default()),
    }
}

/// 校验白板与目录：白板写成 <项目>/<白板>、不重复；目录可以暂时不存在（界面上显示状态、可以克隆）。
/// 返回展开后的目录（相对路径按当前目录）和配置里写的原文
fn parse_dirs(raw: Vec<(String, String)>) -> Result<(Vec<(String, PathBuf)>, Vec<(String, String)>)> {
    let mut dirs: Vec<(String, PathBuf)> = vec![];
    let mut raw_out: Vec<(String, String)> = vec![];
    for (board, path) in raw {
        let board = board.trim().to_string();
        let path = path.trim().to_string();
        if !board_core::valid_board(&board) {
            return Err(BoardError::usage(format!("白板要写成 <项目>/<白板>：{board}")));
        }
        if dirs.iter().any(|(b, _)| *b == board) {
            return Err(BoardError::usage(format!("白板 {board} 写了两次")));
        }
        if path.is_empty() {
            return Err(BoardError::usage(format!("{board} 还没有目录")));
        }
        let mut dir = expand(&path);
        if dir.is_relative() {
            dir = std::env::current_dir().map(|c| c.join(&dir)).unwrap_or(dir);
        }
        dirs.push((board.clone(), dir));
        raw_out.push((board, path));
    }
    Ok((dirs, raw_out))
}

/// 服务端把目录设置写回本机配置；保留 `[host]` 里的机器名和文件里的其他项、注释
fn save_host_dirs(dirs: &[(String, String)]) -> Result<()> {
    config::update(|c| {
        let name = c.host()?.name;
        c.set_host(name.as_deref(), dirs)?;
        Ok(((), true))
    })
}

fn git_line(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).stdin(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// 断开 OFFLINE_WARN 时打印的提醒
fn offline_note(gap: Duration, running: usize) -> String {
    let base = format!("和服务端断开 {} 秒了", gap.as_secs());
    if running == 0 {
        return format!("{base}（继续重试）");
    }
    format!(
        "{base}：{running} 个 agent 还在跑。超过 50 秒服务端会判这台机器掉线、放回它们的卡片，别的机器可能接走；恢复连接后这些 agent 会被结束（它们推集成分支前会先用 board hold 确认领取）"
    )
}

/// 目录里没推到远端的改动：没提交的文件、不在任何远端分支上的提交。不是 git 仓库、都推了时为空；
/// 没有远端的仓库不看提交（所有提交都不在远端，说明不了什么）
fn unpushed(dir: &Path) -> Option<String> {
    if !dir.is_dir() || git_line(dir, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return None;
    }
    let mut parts = vec![];
    let dirty = git_line(dir, &["status", "--porcelain"]).map(|s| s.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0);
    if dirty > 0 {
        parts.push(format!("{dirty} 个文件没提交"));
    }
    if git_line(dir, &["remote"]).is_some() {
        if let Some(head) = git_line(dir, &["rev-parse", "--short", "HEAD"]) {
            if git_line(dir, &["branch", "-r", "--contains", "HEAD"]).is_none() {
                parts.push(format!("提交 {head} 不在任何远端分支上"));
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("，"))
}

/// board-skills 给这个名字的会话开的 worktree：主仓库目录旁边的 `<仓库目录名>.wt/<名字，/ 换成 ->`
fn skill_worktree(repo: &Path, name: &str) -> Option<PathBuf> {
    let common = git_line(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let main = Path::new(&common).parent()?.to_path_buf();
    let base = main.file_name()?.to_string_lossy().to_string();
    Some(main.parent()?.join(format!("{base}.wt")).join(name.replace('/', "-")))
}

/// 一次派发结束后可能留着改动的地方：agent 的 worktree（board-skills 的约定）和白板目录本身；只列有没推改动的
fn leftovers(repo: &Path, name: &str) -> Vec<(PathBuf, String)> {
    let mut dirs: Vec<PathBuf> = vec![];
    if !name.is_empty() {
        if let Some(wt) = skill_worktree(repo, name) {
            dirs.push(wt);
        }
    }
    dirs.push(repo.to_path_buf());
    dirs.into_iter().filter_map(|d| unpushed(&d).map(|w| (d, w))).collect()
}

/// 目录状态：存在且是 git 仓库（同时取 origin、当前分支）、不存在、存在但不是 git 仓库
fn check_git_dir(dir: &Path) -> (&'static str, Option<String>, Option<String>) {
    if !dir.exists() {
        return ("missing", None, None);
    }
    if !dir.is_dir() || git_line(dir, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return ("not_git", None, None);
    }
    let origin = git_line(dir, &["remote", "get-url", "origin"]);
    let branch = git_line(dir, &["branch", "--show-current"]);
    ("ok", origin, branch)
}

fn clone_tail(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().and_then(|s| s.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()))
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

    /// 上一个同名 board host 没走完退出流程（被 kill -9、崩溃）时留下的 agent：按 run 目录里的 `.proc`
    /// 结束进程号和启动时间都对得上的进程组（先 SIGTERM，几秒后还在就 SIGKILL；WSL 在 WSL 里结束 agent 的进程组）
    fn kill_orphans(&self) {
        let Ok(dir) = std::fs::read_dir(&self.run) else { return };
        let Some(mut ps) = Procs::new() else { return };
        let mut left: Vec<Proc> = vec![];
        for e in dir.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("proc") {
                continue;
            }
            let Ok(s) = std::fs::read_to_string(&path) else { continue };
            let mut it = s.split_whitespace();
            let (Some(pid), Some(start)) = (it.next(), it.next()) else { continue };
            // 不知道启动时间的不动：进程号可能已经被复用
            let Some(proc) = Proc::parse(&format!("{pid} {start}")).filter(|p| p.start != 0) else { continue };
            let env = it.next().unwrap_or_default();
            let what = it.next().unwrap_or_default();
            if !ps.alive(proc) {
                continue;
            }
            match self.envs.iter().find(|x| x.id == env && x.wsl().is_some()) {
                Some(wsl) => {
                    let inner = std::fs::read_to_string(path.with_extension("pid")).ok().and_then(|s| s.trim().parse::<u32>().ok());
                    if let Some(pid) = inner {
                        envs::wsl_kill(wsl, pid, true);
                    }
                    kill_tree(proc.pid, true);
                }
                None => {
                    kill_tree(proc.pid, false);
                    left.push(proc);
                }
            }
            self.say(format!("{} 结束上一次留下的 agent：{what}（进程 {}）", self.p.yellow("◼"), proc.pid));
        }
        let begun = Instant::now();
        while left.iter().any(|p| ps.alive(*p)) && begun.elapsed() < ORPHAN_KILL_AFTER {
            std::thread::sleep(Duration::from_millis(100));
        }
        for p in left {
            if ps.alive(p) {
                kill_tree(p.pid, true);
            }
        }
    }

    fn report(&mut self, done: bool) -> HostReport {
        HostReport {
            name: self.name.clone(),
            instance: self.instance.clone(),
            resume: self.resume.clone(),
            os: self.os.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            agents: self.agents.iter().map(Spec::report).collect(),
            envs: self.envs.iter().map(Env::report).collect(),
            dirs: self.dir_reports(),
            dirs_rev: self.dirs_rev,
            // 已经退出、但结束还没上报成功的派发也算在跑：否则服务端会先按「本机没有这个进程」结束它，丢掉退出码和日志
            running: self.runs.iter().map(|r| r.d.id).chain(self.ended.iter().map(|e| e.id)).collect(),
            done,
            asked: self.asked,
            ask_error: self.ask_error.clone(),
            selftest: true,
            skills: Some(self.skill_states.clone()),
            mcp: Some(self.mcp_states.clone()),
        }
    }

    /// 处理界面上发来的远程管理请求（PRD §9.1「远程管理」）。返回 (要马上上报, 要退出主循环)
    /// - 重新检查：重新查找启动环境、本机 agent、board-skills（环境检查也重跑），马上上报
    /// - 重启、升级：等本机没有在跑的 agent（服务端已经按「人停止」通知要结束的那些），先上报「开始处理」，
    ///   再重启自己；升级时桌面端运行的交给桌面端，命令行运行的执行 `board update`，装好后用新版本重启
    fn handle_ask(&mut self) -> (bool, bool) {
        let Some(ask) = self.work.as_ref().and_then(|w| w.host.ask.clone()) else { return (false, false) };
        if self.asked == Some(ask.id) || self.updating.is_some() {
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
            _ => (false, false),
        }
    }

    fn ask_started(&mut self, ask: &HostAsk) -> (bool, bool) {
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
    fn poll_update(&mut self) -> (bool, bool) {
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
                    "这个版本的 board 还不能自己升级（没有 board update）：到这台机器上手动升级一次".to_string()
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

    fn update_failed(&mut self, why: String) {
        self.say(format!("{} 升级失败：{why}", self.p.red("✗")));
        self.ask_error = Some(format!("升级失败：{why}"));
    }

    /// 重新检查（远程请求）：重新找启动环境、本机 agent（找不找得到、版本号）、board-skills，环境检查也重跑。
    /// 配置文件读不了、agent 配置不对时保留原来的 agent
    fn recheck(&mut self) {
        self.skill_force = true;
        self.mcp_force = true;
        self.envs = envs::discover();
        match config::File::load().and_then(|f| f.host()).and_then(|c| agent_specs(&c.agents, &self.templates, &self.envs)) {
            Ok(a) => self.agents = a,
            Err(e) => self.say(format!("{} 重新检查 agent 没成：{}（接着用原来的）", self.p.yellow("!"), e.message)),
        }
        for a in &mut self.agents {
            a.check_skills(&self.envs, &self.dirs);
        }
        self.ready_all = true;
        self.plan_readies();
        let found: Vec<String> = self.agents.iter().filter(|a| a.report().found).map(|a| a.id.clone()).collect();
        self.say(format!(
            "{} 重新检查完了：启动环境 {} 个，找到 agent {}",
            self.p.green("✓"),
            self.envs.len(),
            if found.is_empty() { "（没有）".to_string() } else { found.join("、") }
        ));
    }

    /// 该重新看目录状态了：目录刚变过（配置、克隆完）、到时间了、或者还没有结果
    fn dir_check_due(checks: &[DirCheck], path: &Path) -> bool {
        match checks.iter().find(|c| c.path == path) {
            Some(c) => c.at.elapsed() >= DIR_CHECK_EVERY,
            None => true,
        }
    }

    /// 每块白板目录的 git 状态：到时间的重新看一遍，其余沿用上一次的结果（心跳不跑 git 子进程）
    fn refresh_dir_checks(&mut self) {
        let dirs: Vec<PathBuf> = self.dirs.iter().map(|(_, p)| p.clone()).collect();
        self.dir_checks.retain(|c| dirs.contains(&c.path));
        for dir in dirs {
            if !Self::dir_check_due(&self.dir_checks, &dir) {
                continue;
            }
            let (status, origin, branch) = check_git_dir(&dir);
            let at = Instant::now();
            match self.dir_checks.iter_mut().find(|c| c.path == dir) {
                Some(c) => {
                    c.status = status;
                    c.origin = origin;
                    c.branch = branch;
                    c.at = at;
                }
                None => self.dir_checks.push(DirCheck { path: dir, status, origin, branch, at }),
            }
        }
    }

    /// 每块白板：配置里写的路径、是否 --dir 固定、目录状态、git 的 origin / 分支、克隆进度
    fn dir_reports(&mut self) -> Vec<HostDir> {
        self.refresh_dir_checks();
        self.dirs
            .iter()
            .enumerate()
            .map(|(i, (board, path))| {
                let raw = self.dir_raw.get(i).map(|(_, p)| p.clone()).unwrap_or_else(|| path.display().to_string());
                let c = self.dir_checks.iter().find(|c| c.path == *path);
                let (status, origin, branch) = match c {
                    Some(c) => (c.status, c.origin.clone(), c.branch.clone()),
                    None => ("missing", None, None),
                };
                let checked_at = c.map(|c| now_ms() - c.at.elapsed().as_millis() as i64).unwrap_or_else(now_ms);
                let clone = self.clones.iter().rev().find(|c| c.board == *board).map(|c| HostClone {
                    state: c.state.clone(),
                    url: c.url.clone(),
                    message: c.message.clone(),
                    updated_at: c.updated_at,
                    by: "本机".into(),
                });
                HostDir {
                    board: board.clone(),
                    path: raw,
                    fixed: self.fixed_dirs.get(board).copied().unwrap_or(false),
                    status: Some(status.into()),
                    origin,
                    branch,
                    checked_at: Some(checked_at),
                    clone,
                    check: None,
                    ready: self.readies.iter().find(|r| r.board == *board).and_then(|r| r.last.clone()),
                }
            })
            .collect()
    }

    fn rm(&self) -> &Remote {
        self.be.remote()
    }

    fn beat(&mut self, done: bool) -> Result<HostWork> {
        let rep = serde_json::to_value(self.report(done)).unwrap_or(Value::Null);
        self.rm().post(&format!("/api/hosts/{}/beat", self.name), rep)
    }

    fn set_work(&mut self, w: HostWork) {
        self.sync_server_dirs(&w.host);
        // 会话里 agent 执行了 board next：同一个进程换到这个会话正在跑的那一张，之后的日志、停止、结束都跟着它。
        // 换之前把已经写出的日志传给上一张
        let switches: Vec<(usize, Dispatch)> =
            self.runs.iter().enumerate().filter_map(|(i, r)| successor(&r.d, &w.running).map(|n| (i, n))).collect();
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        for (i, n) in switches {
            let (lines, to) = new_lines(&self.runs[i].log, self.runs[i].sent, false);
            let lines = upload_lines_redacted(lines, self.runs[i].fmt, &secrets);
            if !lines.is_empty() {
                let _ = self.rm().post::<Value>(&format!("/api/dispatches/{}/log", self.runs[i].d.id), json!({ "lines": lines }));
            }
            self.runs[i].sent = to;
            // 已经写出的事件也记到上一张；之后的记到新的这张，开头再报一次格式
            let mut r = self.runs.remove(i);
            self.flush_events(&mut r, false);
            r.fmt_due = true;
            self.runs.insert(i, r);
            self.say(format!(
                "{} {}  {}  {}",
                self.p.green("▶"),
                self.p.cyan(&format!("{}#{}", n.board, n.card)),
                self.p.yellow(n.name.as_deref().unwrap_or_default()),
                self.p.dim(&format!("同一个会话接着做 · {}", n.title))
            ));
            self.runs[i].d = n;
        }
        let templates = w.templates.clone();
        self.work = Some(w);
        self.work_at = Instant::now();
        self.hold = false;
        if let Some(t) = templates.filter(|t| *t != self.templates) {
            self.apply_templates(t);
        }
        self.start_clones();
        self.plan_readies();
    }

    /// 共用配置里的 agent 模板变了：合进本机 agent（本机配置文件的同标识优先），重新检查一遍，马上上报。不用重启
    fn apply_templates(&mut self, t: Vec<AgentTemplate>) {
        let first = self.templates.is_empty();
        self.templates = t;
        match config::File::load().and_then(|f| f.host()).and_then(|c| agent_specs(&c.agents, &self.templates, &self.envs)) {
            Ok(a) => self.agents = a,
            Err(e) => {
                self.say(format!("{} 共用配置里的 agent 模板没有用上：{}（接着用原来的）", self.p.yellow("!"), e.message));
                return;
            }
        }
        for a in &mut self.agents {
            a.check_skills(&self.envs, &self.dirs);
        }
        self.skill_force = true;
        self.mcp_force = true;
        self.beat_soon = true;
        let ids: Vec<String> = self.templates.iter().map(|t| t.id.clone()).collect();
        let over: Vec<String> = self.agents.iter().filter(|a| a.local_override).map(|a| a.id.clone()).collect();
        if !(first && ids.is_empty()) {
            let mut line = format!(
                "{} 共用配置的 agent 模板：{}",
                self.p.green("✓"),
                if ids.is_empty() { "（没有）".to_string() } else { ids.join("、") }
            );
            if !over.is_empty() {
                line += &format!("；本机配置覆盖：{}", over.join("、"));
            }
            self.say(line);
        }
    }

    /// 按服务端发来的检查命令决定哪些白板要（重新）跑环境检查：没跑过、命令或目录变了、目录刚有了、
    /// 上次跑完超过一天、启动或重新读配置之后。白板不再检查（或这台机器不再有它的目录）时停掉、丢掉结果
    fn plan_readies(&mut self) {
        let Some(w) = &self.work else { return };
        let want: Vec<(String, String, PathBuf)> = w
            .host
            .dirs
            .iter()
            .filter_map(|d| {
                let cmd = d.check.clone()?;
                let dir = self.dirs.iter().find(|(b, _)| *b == d.board)?.1.clone();
                Some((d.board.clone(), cmd, dir))
            })
            .collect();
        for r in self.readies.iter_mut().filter(|r| !want.iter().any(|(b, ..)| *b == r.board)) {
            r.kill();
        }
        self.readies.retain(|r| want.iter().any(|(b, ..)| *b == r.board));
        let all = std::mem::take(&mut self.ready_all);
        let now = now_ms();
        for (board, cmd, dir) in want {
            let due = match self.readies.iter().find(|r| r.board == board) {
                None => true,
                Some(r) if r.cmd != cmd || r.dir != dir => true,
                Some(r) if r.child.is_some() => false,
                Some(r) if r.missing && dir.is_dir() => true,
                Some(r) => all || r.last.as_ref().is_none_or(|l| now - l.at >= READY_EVERY_MS),
            };
            if due {
                self.start_ready(&board, &cmd, &dir);
            }
        }
    }

    /// 开始跑一块白板的环境检查（同一块白板还在跑的先结束）。目录不存在时直接记为没通过
    fn start_ready(&mut self, board: &str, cmd: &str, dir: &Path) {
        let last = match self.readies.iter_mut().position(|r| r.board == board) {
            Some(i) => {
                let mut old = self.readies.remove(i);
                old.kill();
                old.last
            }
            None => None,
        };
        let _ = std::fs::create_dir_all(&self.logs);
        let safe: String = board.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
        let log = self.logs.join(format!("ready-{safe}.log"));
        restrict_log(&log);
        let mut r = ReadyRun { board: board.into(), cmd: cmd.into(), dir: dir.to_path_buf(), child: None, log, last, missing: false };
        let fail = |r: &mut ReadyRun, why: String| {
            r.last = Some(DirReady { cmd: r.cmd.clone(), ok: false, code: None, tail: vec![why], at: now_ms() });
        };
        if !dir.is_dir() {
            r.missing = true;
            fail(&mut r, format!("目录不存在：{}", dir.display()));
        } else {
            let files = create_log(&r.log);
            match files {
                Err(e) => fail(&mut r, format!("无法创建检查日志：{e}")),
                Ok((out, err)) => {
                    let mut c = envs::shell_line(&self.envs, cmd, dir);
                    c.stdin(Stdio::null()).stdout(out).stderr(err);
                    envs::detach(&mut c);
                    match c.spawn() {
                        Ok(child) => r.child = Some((child, Instant::now())),
                        Err(e) => fail(&mut r, format!("没法运行检查命令：{e}")),
                    }
                }
            }
        }
        if r.child.is_some() {
            self.say(format!("{} {}  {}", self.p.dim("…"), self.p.cyan(board), self.p.dim(&format!("环境检查：{cmd}"))));
        } else {
            self.ready_said(&r);
        }
        self.readies.push(r);
    }

    /// 看一眼正在跑的环境检查：跑完的记下结果，超过 2 分钟的结束它、算没通过。返回有没有新结果（有就马上上报）
    fn poll_readies(&mut self) -> bool {
        let mut done: Vec<usize> = vec![];
        for (i, r) in self.readies.iter_mut().enumerate() {
            let Some((child, begun)) = r.child.as_mut() else { continue };
            let status = child.try_wait().ok().flatten();
            let late = begun.elapsed() >= Duration::from_millis(READY_LIMIT_MS as u64);
            if status.is_none() && !late {
                continue;
            }
            // 登录 shell 自己打的启动、退出提示不算输出（#232，见 shell_noise）：tail_lines 已经去掉，
            // 先多取几行，去掉噪声之后仍能凑满 READY_TAIL 行
            let mut tail = tail_lines(&r.log, READY_TAIL + 5, None);
            let (ok, code) = match status {
                Some(s) => (s.success(), exit_code(s)),
                None => {
                    r.kill();
                    tail.push(format!("超过 {} 分钟没跑完，已结束", READY_LIMIT_MS / 60_000));
                    (false, None)
                }
            };
            let skip = tail.len().saturating_sub(READY_TAIL);
            r.last = Some(DirReady { cmd: r.cmd.clone(), ok, code, tail: tail[skip..].to_vec(), at: now_ms() });
            r.child = None;
            done.push(i);
        }
        for &i in &done {
            self.ready_said(&self.readies[i]);
        }
        !done.is_empty()
    }

    /// 打印环境检查的结果
    fn ready_said(&self, r: &ReadyRun) {
        let Some(l) = &r.last else { return };
        if l.ok {
            self.say(format!("{} {}  环境检查通过", self.p.green("✓"), self.p.cyan(&r.board)));
        } else {
            let last = l.tail.iter().rev().find(|x| !x.trim().is_empty()).cloned().unwrap_or_else(|| format!("退出码 {:?}", l.code));
            self.say(format!(
                "{} {}  环境检查没通过：{}  {}",
                self.p.red("✗"),
                self.p.cyan(&r.board),
                last,
                self.p.dim("这台机器不接这块白板的卡，装好之后重新检查")
            ));
        }
    }

    /// 服务端的目录设置是主版本：配置文件没变、但服务端改了（别的设备上改的）就写回 `[host]` 并直接换上。
    /// `--dir` 启动时这一项不跟，服务端也不会让它改
    fn sync_server_dirs(&mut self, h: &HostInfo) {
        if !self.follow_dirs {
            self.dirs_rev = h.dirs_rev;
            return;
        }
        let desired: Vec<(String, String)> = h.dirs.iter().map(|d| (d.board.clone(), d.path.clone())).collect();
        if desired != self.dir_raw {
            match save_host_dirs(&desired) {
                Ok(()) => match parse_dirs(desired.clone()) {
                    Ok((dirs, raw)) => {
                        for a in &mut self.agents {
                            a.check_skills(&self.envs, &dirs);
                        }
                        let list: Vec<String> = raw.iter().map(|(b, d)| format!("{b} → {d}")).collect();
                        let list = if list.is_empty() { "（没有）".to_string() } else { list.join("，") };
                        self.say(format!("{} 服务端改的白板与目录已生效：{list}", self.p.green("✓")));
                        self.dirs = dirs;
                        self.dir_raw = raw;
                        self.conf_seen.1 = desired;
                        self.dirs_rev = h.dirs_rev;
                    }
                    Err(e) => self.say(format!("{} 服务端改的白板与目录没有生效：{}", self.p.yellow("!"), e.message)),
                },
                Err(e) => self.say(format!("{} 服务端改的白板与目录没有写进配置：{}", self.p.yellow("!"), e.message)),
            }
        } else {
            self.dirs_rev = h.dirs_rev;
        }
    }

    /// 服务端请求了克隆，就在本机执行 `git clone`；一块白板失败不影响其他白板
    fn start_clones(&mut self) {
        let Some(w) = &self.work else { return };
        let jobs: Vec<(String, String)> = w
            .host
            .dirs
            .iter()
            .filter(|d| d.clone.as_ref().is_some_and(|c| c.state == "queued"))
            .filter(|d| !self.clones.iter().any(|c| c.board == d.board && (c.state == "queued" || c.state == "running")))
            .filter_map(|d| d.clone.as_ref().map(|c| (d.board.clone(), c.url.clone())))
            .collect();
        for (board, url) in jobs {
            self.start_clone(&board, &url);
        }
    }

    fn start_clone(&mut self, board: &str, url: &str) {
        let Some((_, dir)) = self.dirs.iter().find(|(b, _)| b == board).cloned() else { return };
        let _ = std::fs::create_dir_all(&self.logs);
        let safe: String = board
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        let log = self.logs.join(format!("clone-{safe}.log"));
        let (out, err) = match create_log(&log) {
            Ok(f) => f,
            Err(e) => {
                self.clones.push(CloneRun {
                    board: board.into(),
                    url: url.into(),
                    state: "failed".into(),
                    message: format!("无法创建克隆日志：{e}"),
                    updated_at: now_ms(),
                    child: None,
                    log,
                });
                return;
            }
        };
        if let Some(parent) = dir.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.clones.push(CloneRun {
                    board: board.into(),
                    url: url.into(),
                    state: "failed".into(),
                    message: format!("无法创建 {}：{e}", parent.display()),
                    updated_at: now_ms(),
                    child: None,
                    log,
                });
                return;
            }
        }
        let child = Command::new("git")
            .args(["clone", "--", url])
            .arg(&dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn();
        match child {
            Ok(child) => self.clones.push(CloneRun {
                board: board.into(),
                url: url.into(),
                state: "running".into(),
                message: "正在克隆…".into(),
                updated_at: now_ms(),
                child: Some(child),
                log,
            }),
            Err(e) => self.clones.push(CloneRun {
                board: board.into(),
                url: url.into(),
                state: "failed".into(),
                message: format!("没法运行 git clone：{e}"),
                updated_at: now_ms(),
                child: None,
                log,
            }),
        }
    }

    /// skills 共用库（PRD §10.12「共用配置」）：同步做完了就记下结果（变了返回 true，马上上报）；
    /// 没在做、库或要写的目录变了（重新检查后、超过 10 分钟）就在后台开始一次
    fn poll_skills(&mut self) -> bool {
        let mut changed = false;
        if self.skill_job.as_ref().is_some_and(|h| h.is_finished()) {
            if let Ok(out) = self.skill_job.take().expect("上面看过").join() {
                for c in &out.changes {
                    self.say(format!("{} skills：{c}", self.p.green("✓")));
                }
                let bad: Vec<String> = out
                    .states
                    .iter()
                    .filter(|s| s.state == "failed" || s.state == "behind")
                    .map(|s| format!("{}/{}：{}", s.dir, s.skill, s.why.as_deref().unwrap_or("没同步")))
                    .collect();
                if !bad.is_empty() && out.states != self.skill_states {
                    self.say(format!("{} skills 没同步好：{}", self.p.yellow("!"), bad.join("；")));
                }
                changed = out.states != self.skill_states;
                self.skill_states = out.states;
                // 装上、换掉了东西：重新看一遍各 agent 看不看得到 board-skills（界面上「缺 skill」的提醒跟着变）
                if !out.changes.is_empty() && self.recheck_skills() {
                    changed = true;
                }
            }
        }
        if self.skill_job.is_some() {
            return changed;
        }
        let Some(lib) = self.work.as_ref().and_then(|w| w.skills.clone()) else { return changed };
        let used: Vec<HostAgent> = self.work.as_ref().map(|w| w.host.agents.clone()).unwrap_or_default();
        let mut list: Vec<(String, Env, String)> = vec![];
        for spec in &self.agents {
            let Some(u) = used.iter().find(|a| a.id == spec.id && a.found) else { continue };
            let Some(dir) = spec.skills.first() else { continue };
            let env_id = u.env.clone().or_else(|| spec.default_check().map(|c| c.env.clone())).unwrap_or_else(|| "direct".into());
            if let Some(env) = self.envs.iter().find(|e| e.id == env_id) {
                list.push((spec.id.clone(), env.clone(), dir.clone()));
            }
        }
        let key = format!(
            "{:?}|{:?}",
            lib.iter().map(|s| (&s.name, &s.version)).collect::<Vec<_>>(),
            list.iter().map(|(a, e, d)| (a, &e.id, d)).collect::<Vec<_>>()
        );
        if !self.skill_force && self.skill_key.as_deref() == Some(key.as_str()) && self.skill_at.elapsed() < SKILLS_EVERY {
            return changed;
        }
        self.skill_force = false;
        self.skill_key = Some(key);
        self.skill_at = Instant::now();
        let rm = self.rm().clone();
        let exe = self.exe.clone();
        self.skill_job = Some(std::thread::spawn(move || {
            let (targets, mut bad) = skills::host_targets(list, &lib);
            let mut out = skills::sync(&lib, &targets, Some(&exe), |name| rm.get(&format!("/api/skills/{name}"), &[]));
            out.states.append(&mut bad);
            out
        }));
        changed
    }

    /// MCP 共用配置（PRD §10.12「共用配置」）：同步做完了记下结果（变了返回 true，马上上报）；
    /// 没在做、索引或找到的 agent 变了（重新检查后、超过 10 分钟）就在后台开始一次：先用令牌取完整内容（含密钥），再写各 agent 的配置
    fn poll_mcp(&mut self) -> bool {
        let mut changed = false;
        if self.mcp_job.as_ref().is_some_and(|h| h.is_finished()) {
            if let Ok(out) = self.mcp_job.take().expect("上面看过").join() {
                for c in &out.changes {
                    self.say(format!("{} MCP：{c}", self.p.green("✓")));
                }
                let bad: Vec<String> = out.states.iter().filter(|s| s.state == "failed").map(|s| format!("{} 的 {}：{}", s.agent, s.server, s.why.as_deref().unwrap_or("失败"))).collect();
                if !bad.is_empty() && out.states != self.mcp_states {
                    self.say(format!("{} MCP 没同步好：{}", self.p.yellow("!"), bad.join("；")));
                }
                changed = out.states != self.mcp_states;
                self.mcp_states = out.states;
            }
        }
        if self.mcp_job.is_some() {
            return changed;
        }
        let Some(index) = self.work.as_ref().and_then(|w| w.mcp.clone()) else { return changed };
        let used: Vec<HostAgent> = self.work.as_ref().map(|w| w.host.agents.clone()).unwrap_or_default();
        let mut found: Vec<(String, Env, Option<String>)> = vec![];
        for spec in &self.agents {
            let Some(u) = used.iter().find(|a| a.id == spec.id && a.found) else { continue };
            let env_id = u.env.clone().or_else(|| spec.default_check().map(|c| c.env.clone())).unwrap_or_else(|| "direct".into());
            let Some(env) = self.envs.iter().find(|e| e.id == env_id) else { continue };
            let prog = spec.checks.iter().find(|c| c.env == env_id).and_then(|c| c.prog.clone());
            found.push((spec.id.clone(), env.clone(), prog));
        }
        let key = format!("{:?}|{:?}", index.iter().map(|s| (&s.name, &s.version)).collect::<Vec<_>>(), found.iter().map(|(a, e, _)| (a, &e.id)).collect::<Vec<_>>());
        if !self.mcp_force && self.mcp_key.as_deref() == Some(key.as_str()) && self.mcp_at.elapsed() < SKILLS_EVERY {
            return changed;
        }
        self.mcp_force = false;
        self.mcp_key = Some(key);
        self.mcp_at = Instant::now();
        let rm = self.rm().clone();
        let name = self.name.clone();
        let owned = self.logs.parent().map(|d| d.join("mcp-sync.json")).unwrap_or_else(|| PathBuf::from("mcp-sync.json"));
        let any = !index.is_empty();
        self.mcp_job = Some(std::thread::spawn(move || {
            // 共用配置是空的：不用取，直接按空的同步（删掉以前同步加的）
            let lib: Vec<board_core::mcp::McpServer> = if any {
                match rm.get(&format!("/api/hosts/{name}/mcp"), &[]) {
                    Ok(l) => l,
                    Err(e) => {
                        let mut states = vec![];
                        for (a, _, _) in found.iter().filter(|(a, _, _)| board_core::mcp::AGENTS.contains(&a.as_str())) {
                            for s in &index {
                                states.push(board_core::mcp::McpState { server: s.name.clone(), agent: a.clone(), state: "failed".into(), why: Some(format!("取不到 MCP 配置：{}", e.message)) });
                            }
                        }
                        return mcpsync::Synced { states, changes: vec![] };
                    }
                }
            } else {
                vec![]
            };
            let mut targets = vec![];
            let mut unsupported = vec![];
            let mut bad = vec![];
            for (agent, env, prog) in found {
                if !board_core::mcp::AGENTS.contains(&agent.as_str()) {
                    unsupported.push(agent);
                    continue;
                }
                match mcpsync::file_of(&agent, &env) {
                    Some((file, shown)) => targets.push(mcpsync::Target { agent, env, prog, file, shown }),
                    None => {
                        for s in lib.iter().filter(|s| s.for_agent(&agent)) {
                            bad.push(board_core::mcp::McpState { server: s.name.clone(), agent: agent.clone(), state: "failed".into(), why: Some(format!("取不到{}里的配置文件位置", env.name)) });
                        }
                    }
                }
            }
            let mut out = mcpsync::sync(&lib, &targets, &unsupported, &owned);
            out.states.append(&mut bad);
            out
        }));
        changed
    }

    /// 自检（PRD §10.12「自检」）：取到的任务里有新的请求就在后台线程里做；做完回报，回报失败下一轮再报
    fn poll_selftest(&mut self) {
        if self.selftest.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            let (id, h) = self.selftest.take().expect("上面看过");
            let items = h.join().unwrap_or_default();
            self.selftest_out = Some((id, items));
        }
        if let Some((id, items)) = self.selftest_out.take() {
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
                    self.selftest_seen = Some(id);
                }
                Err(e) if e.code == ErrorCode::Unreachable => self.selftest_out = Some((id, items)),
                Err(e) => {
                    self.say(format!("{} 自检结果没报上去：{}", self.p.yellow("!"), e.message));
                    self.selftest_seen = Some(id);
                }
            }
        }
        if self.selftest.is_some() || self.selftest_out.is_some() {
            return;
        }
        let Some(ask) = self.work.as_ref().and_then(|w| w.host.selftest.ask.clone()) else { return };
        if self.selftest_seen == Some(ask.id) {
            return;
        }
        self.start_selftest(&ask);
    }

    /// 在后台开始一次自检：agent 按派发同样的办法拼命令（启动环境、模型、思考强度），进一个临时目录问一句；
    /// 整台机器时再检查每块白板的目录能不能推送
    fn start_selftest(&mut self, ask: &board_core::SelfTestAsk) {
        let what = ask.agent.as_deref().map(|a| format!(" {a}")).unwrap_or_default();
        self.say(format!("{} {} 请求自检{what}", self.p.cyan("◇"), ask.by));
        let tmp = std::env::temp_dir().join(format!("board-selftest-{}", self.name));
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
            self.dirs.iter().map(|(b, d)| selftest::GitJob { board: b.clone(), dir: d.clone() }).collect()
        };
        let pids = self.selftest_pids.clone();
        let h = std::thread::spawn(move || selftest::run(agents, gits, tmp, pids));
        self.selftest = Some((ask.id, h));
    }

    /// 自检时启动一个 agent 的命令：和派发一样的启动环境、模型、思考强度，工作目录是临时目录
    fn selftest_command(&self, spec: &Spec, used: Option<&HostAgent>, tmp: &Path) -> std::result::Result<(Command, Option<Stream>), String> {
        let cmd0 = spec.cmd.first().cloned().unwrap_or_default();
        let check = used
            .and_then(|u| u.env.as_deref())
            .and_then(|e| spec.checks.iter().find(|c| c.env == e))
            .or_else(|| spec.default_check())
            .ok_or_else(|| format!("本机没找到 {} 的命令 {cmd0}", spec.name))?;
        let env = self.envs.iter().find(|e| e.id == check.env).cloned().ok_or_else(|| format!("本机没有启动环境「{}」", check.env))?;
        let prog = check.prog.clone().ok_or_else(|| format!("在{}里没找到 {} 的命令 {cmd0}", env.name, spec.name))?;
        let patch = self.run.join("dsh-headless.patch.yml");
        if spec.cmd.iter().any(|a| a.contains("{patch}")) {
            write_dsh_patch(&patch).map_err(|e| format!("无法写 {}：{e}", patch.display()))?;
        }
        let model = used.and_then(|u| u.run_model.as_deref());
        let effort = used.and_then(|u| u.run_effort.as_deref());
        let cmd = spec.cmd_for(check);
        let args = agent_args(&cmd, &spec.model_args, &spec.effort_args, model, effort, selftest::PROMPT, &patch.display().to_string());
        let stream = logfmt::stream_for(&spec.id, &cmd);
        let mut c = env.command(&prog, &args, Some(tmp), true);
        c.env_remove("CLAUDECODE");
        if cfg!(windows) && spec.git_bash && env.wsl().is_none() {
            if let Some(bin) = envs::git_bash_dir() {
                let old = std::env::var_os("PATH").unwrap_or_default();
                let mut parts: Vec<std::ffi::OsString> = vec![bin.into_os_string()];
                parts.extend(std::env::split_paths(&old).map(|p| p.into_os_string()));
                if let Ok(path) = std::env::join_paths(parts) {
                    c.env("PATH", path);
                }
            }
        }
        Ok((c, stream))
    }

    fn poll_clones(&mut self) {
        let mut done: Vec<String> = vec![];
        for c in &mut self.clones {
            if c.state != "running" {
                continue;
            }
            let status = c.child.as_mut().and_then(|child| child.try_wait().ok().flatten());
            if let Some(status) = status {
                let tail = clone_tail(&c.log);
                if status.success() {
                    c.state = "done".into();
                    c.message = "克隆完成".into();
                    // 目录里刚有了仓库：下一次上报重新看这块白板的目录状态，不等 DIR_CHECK_EVERY
                    done.push(c.board.clone());
                } else {
                    c.state = "failed".into();
                    c.message = tail.unwrap_or_else(|| format!("git clone 失败（退出码 {:?}）", status.code()));
                }
                c.updated_at = now_ms();
                c.child = None;
            } else if let Some(line) = clone_tail(&c.log) {
                c.message = line;
            }
        }
        // 刚克隆完的白板：删掉旧的目录状态，下一次上报马上重新看
        let paths: Vec<PathBuf> = self
            .dirs
            .iter()
            .filter(|(b, _)| done.contains(b))
            .map(|(_, p)| p.clone())
            .collect();
        self.dir_checks.retain(|c| !paths.contains(&c.path));
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
            let u = used(&a.id).unwrap_or_else(|| a.report());
            let env = u.env.clone().or_else(|| a.default_check().map(|c| c.env.clone()));
            let state = if u.found {
                let v = if u.version.is_empty() { "找到了".to_string() } else { u.version.clone() };
                let e = env.as_deref().map(|e| p.dim(&format!(" · {}", env_name(e)))).unwrap_or_default();
                format!("{v}{e}{}", if def.as_deref() == Some(&a.id) { p.yellow("（默认）") } else { String::new() })
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
        if self.dirs.is_empty() {
            s += &format!("  {}{}\n", p.dim("白板    "), p.yellow("（没有）：用 --dir <项目>/<白板>=<路径> 加上，否则接不到卡片"));
        }
        for (i, (b, d)) in self.dirs.iter().enumerate() {
            let head = if i == 0 { p.dim("白板    ") } else { "        ".into() };
            s += &format!("  {head}{} → {}\n", p.cyan(b), d.display());
        }
        // 服务端上没有的白板
        if let Ok(ws) = self.rm().get::<Workspace>("/api/workspace", &[]) {
            for (b, _) in &self.dirs {
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
        let mut last_min = String::new();
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
            // 环境检查有了新结果：马上上报，界面和派发立刻按新结果算
            if self.poll_readies() {
                next_beat = now;
            }
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
                match self.beat(false) {
                    Ok(w) => {
                        self.connected();
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
            // skills 共用库：库变了（或重新检查后、每 10 分钟）在后台同步，结果马上上报
            if self.poll_skills() {
                next_beat = Instant::now();
            }
            // MCP 共用配置：一样在后台写进各 agent 的配置
            if self.poll_mcp() {
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
            if self.fill()? {
                fetch = true;
            }
            self.cron(&mut last_min);
            if now >= next_events {
                next_events = now + EVENTS_EVERY;
                self.upload_events();
            }
            if now >= next_log {
                next_log = now + LOG_EVERY;
                self.upload_logs();
                self.report_ended();
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
        self.skill_force = true;
        self.mcp_force = true;
        let skills = self.recheck_skills();
        // 重新读配置时把环境检查重跑一遍（可能刚装好了缺的工具）
        self.ready_all = true;
        self.reload_conf() || skills
    }

    /// 重新在每个启动环境里看一次 `board-skills`（桌面端一键安装之后）。返回检查结果有没有变；新看得到的打印一行
    fn recheck_skills(&mut self) -> bool {
        let mut changed = false;
        let mut fixed = Vec::new();
        for a in &mut self.agents {
            let before = a.no_skill.clone();
            a.check_skills(&self.envs, &self.dirs);
            if before != a.no_skill {
                changed = true;
                if a.no_skill.len() < before.len() {
                    fixed.push(a.id.clone());
                }
            }
        }
        if !fixed.is_empty() {
            self.say(format!("{} {} 看得到 {SKILL} 了", self.p.green("✓"), fixed.join("、")));
        }
        changed
    }

    /// 重新读配置文件的 `[host]`，不重启，换上新的机器名、白板与目录，正在跑的 agent 不受影响。
    /// 返回有没有换。配置不对（名字不合规、目录不存在）、服务端不让改名时打印一行，接着用原来的
    fn reload_conf(&mut self) -> bool {
        if !self.follow_name && !self.follow_dirs {
            return false;
        }
        let Ok(conf) = config::File::load().and_then(|f| f.host()) else { return false };
        let raw = (conf.name.clone(), conf.dirs.clone(), conf.log_days);
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
            match parse_dirs(conf.dirs) {
                Ok((dirs, raw)) if dirs != self.dirs => {
                    for a in &mut self.agents {
                        a.check_skills(&self.envs, &dirs);
                    }
                    let list: Vec<String> = dirs.iter().map(|(b, d)| format!("{b} → {}", d.display())).collect();
                    let list = if list.is_empty() { "（没有）".to_string() } else { list.join("，") };
                    self.say(format!("{} 白板与目录已更新：{list}", self.p.green("✓")));
                    self.dirs = dirs;
                    self.dir_raw = raw;
                    changed = true;
                }
                Ok(_) => {}
                Err(e) => self.say(format!("{} 白板与目录没有更新：{}（接着用原来的）", self.p.yellow("!"), e.message)),
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

    /// 机器的兜底设置：单次最长运行、没输出多久算卡住（分钟，0 = 不限）。每次从服务端的最新设置算，
    /// 改了不用重启 board host 就生效
    fn limits(&self) -> (Option<i64>, Option<i64>) {
        let Some(w) = &self.work else { return (None, None) };
        let mins = |n: i64| (n > 0).then_some(n);
        (mins(w.host.max_run), mins(w.host.idle_run))
    }

    /// 到时间按「人停止」的流程结束进程树（先 SIGTERM，10 秒后 SIGKILL），原因记在 `Run.timeout` 里，
    /// 进程退出后由 reap 上报结束（结果「超时」）。返回有没有新的超时
    ///
    /// - 单次最长运行：从启动算起
    /// - 没输出多久算卡住：只有已经输出过内容的派发生效。内置模板都用结构化输出，运行中每一步都写日志；配置文件里改成
    ///   只在结束时打印的（例如 Claude Code 的 `-p` 文本输出）运行中一行都没有，所以这条不适用于一行都还没输出的派发
    fn enforce_timeouts(&mut self) -> bool {
        let (max_run, idle_run) = self.limits();
        let min = minute();
        let mut said = vec![];
        let mut any = false;
        for r in &mut self.runs.iter_mut().filter(|r| r.timeout.is_none()) {
            // 日志变长了就是有输出：「没输出多久算卡住」从这里算
            let len = std::fs::metadata(&r.log).map(|m| m.len()).unwrap_or(0);
            if len > r.seen_len {
                r.seen_len = len;
                r.output_at = Instant::now();
            }
            // 原因只写「运行超过 30 分钟」「20 分钟没有任何输出」：服务端在评论里补上「，board host 停止了它」
            let run = max_run.filter(|n| r.spawned.elapsed() >= min * *n as u32).map(|n| {
                format!("运行超过 {}", span_words(Duration::from_secs(n as u64 * 60)))
            });
            let idle = idle_run.filter(|n| r.seen_len > 0 && r.output_at.elapsed() >= min * *n as u32).map(|n| {
                format!("{}没有任何输出", span_words(Duration::from_secs(n as u64 * 60)))
            });
            let Some(why) = run.or(idle) else { continue };
            r.timeout = Some(why.clone());
            if r.term_at.is_none() {
                r.stop(false);
                r.term_at = Some(Instant::now());
            }
            said.push(format!("停止 {}#{}：{why}", r.d.board, r.d.card));
            any = true;
        }
        for s in said {
            self.say(format!("{} {s}", self.p.yellow("◼")));
        }
        any
    }

    /// agent 退出：上传剩下的日志，上报结束。返回有没有派发结束
    fn reap(&mut self) -> bool {
        let mut done = false;
        let mut i = 0;
        while i < self.runs.len() {
            let status = match self.runs[i].child.try_wait() {
                Ok(Some(s)) => Some(s),
                Ok(None) => None,
                Err(_) => None,
            };
            let Some(status) = status else {
                i += 1;
                continue;
            };
            let r = self.runs.remove(i);
            let code = exit_code(status);
            let ended = self.finish_run(r, code, "exit");
            let what = match &ended.why {
                Some(why) => why.clone(),
                None => format!("agent 退出，退出码 {}", code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into())),
            };
            self.say(format!(
                "{} {}  {}  {}",
                self.p.dim("■"),
                self.p.cyan(&format!("{}#{}", ended.board, ended.card)),
                ended.name.as_deref().unwrap_or_default(),
                self.p.dim(&what)
            ));
            // 派发已经被服务端结束（机器掉线、卡片可能已被别人接走）：agent 的改动没推上去的话，写明在哪里，方便人捞回来
            if ended.lost {
                if let Some((_, repo)) = self.dirs.iter().find(|(b, _)| *b == ended.board) {
                    for (dir, what) in leftovers(repo, ended.name.as_deref().unwrap_or_default()) {
                        self.say(format!(
                            "{} {} 里有没推到远端的改动（{what}）：{}#{} 的派发已被服务端结束，需要的话去那里捞回来",
                            self.p.yellow("!"),
                            dir.display(),
                            ended.board,
                            ended.card
                        ));
                    }
                }
            }
            done = true;
        }
        if done {
            self.report_ended();
        }
        done
    }

    /// 一次派发结束的收尾，`reap`（agent 自己退出）和 `shutdown`（board host 退出）共用：
    ///
    /// 1. 主进程已经不在了，它起的子进程可能还在同一个进程组里：`kill_rest()` 一起结束
    /// 2. 删掉这次派发自己的临时文件（配置、进程号、结果文件）
    /// 3. 上传剩下的日志和事件（`last = true`，最后不完整的一行也读）
    /// 4. 取最后几行做错误摘录，再截断过大的日志。截断要在 agent 确实不在了之后：它还活着时换掉文件内容，
    ///    下次写入会从旧位置接着写、中间留一段空洞
    /// 5. 记一笔 `Ended`，等 `report_ended()` 上报给服务端
    ///
    /// `how` 是给服务端的结果：`exit`（agent 退出）、`host_exit`（board host 退出时停的）；
    /// 兜底停止过（`Run.timeout` 有值）的无论哪种都报 `timeout`，原因照原样带上。
    fn finish_run(&mut self, mut r: Run, code: Option<i64>, how: &'static str) -> RunSummary {
        r.kill_rest();
        r.cleanup();
        self.flush_log(&r, true);
        self.flush_events(&mut r, true);
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        self.ended.push(ended_of(&r, code, how, &secrets));
        RunSummary { board: r.d.board.clone(), card: r.d.card, name: r.d.name.clone(), lost: r.lost, why: r.timeout.clone() }
    }

    /// 上报结束；连不上时留到下一轮
    fn report_ended(&mut self) {
        let ended = std::mem::take(&mut self.ended);
        for e in ended {
            let body = json!({ "exitCode": e.code, "tail": e.tail, "events": e.events, "how": e.how, "why": e.why });
            match self.rm().post::<Dispatch>(&format!("/api/dispatches/{}/end", e.id), body) {
                Ok(d) => {
                    let res = d.result.as_deref().map(result_words).unwrap_or("还在跑");
                    self.say(format!("{} {}  结果：{}", self.p.dim("·"), self.p.cyan(&format!("{}#{}", d.board, d.card)), self.p.bold(res)));
                }
                // 派发不存在这类明确的拒绝不用再报；其余错误（连不上、5xx 等）留到下一轮重试
                Err(err) if permanent_end_error(err.code) => {}
                Err(_) => self.ended.push(e),
            }
        }
    }

    /// 停止请求：服务端要停的、服务端已经不当它在跑的（机器掉线、被移除），结束进程树；10 秒后还在就强制结束
    fn enforce(&mut self) {
        let Some(w) = &self.work else { return };
        let work_at = self.work_at;
        let mut say: Vec<String> = vec![];
        for r in self.runs.iter_mut() {
            if r.term_at.is_none() {
                let on_server = w.running.iter().find(|x| x.id == r.d.id);
                let why = match on_server {
                    Some(x) => x.stop_by.as_ref().map(|by| format!("{by} 请求停止")),
                    None if work_at > r.spawned => {
                        r.lost = true;
                        Some("服务端上这次派发已经结束".to_string())
                    }
                    None => None,
                };
                if let Some(why) = why {
                    r.stop(false);
                    r.term_at = Some(Instant::now());
                    say.push(format!("停止 {}#{}：{why}", r.d.board, r.d.card));
                }
            } else if !r.killed && r.term_at.is_some_and(|t| t.elapsed() >= KILL_AFTER) {
                r.stop(true);
                r.killed = true;
            }
        }
        for s in say {
            self.say(format!("{} {s}", self.p.yellow("◼")));
        }
    }

    /// 有空位、没有暂停时：按创建先后开始排队的派发；正在一轮领取时向服务端要下一张。返回有没有变化
    fn fill(&mut self) -> Result<bool> {
        let Some(w) = self.work.clone() else { return Ok(false) };
        // 等重启、升级时不开始新的派发
        let restarting = w.host.ask.as_ref().is_some_and(|a| a.kind == "restart" || a.kind == "upgrade") || self.updating.is_some();
        if self.hold || w.host.paused || restarting {
            return Ok(false);
        }
        let max = w.host.max.max(1) as usize;
        let mut changed = false;
        for q in &w.queued {
            if self.live() >= max {
                return Ok(changed);
            }
            if self.runs.iter().any(|r| r.d.id == q.id) {
                continue;
            }
            let body = json!({ "instance": self.instance, "id": q.id });
            match self.rm().post::<Dispatch>(&format!("/api/hosts/{}/begin", self.name), body) {
                Ok(d) => {
                    changed = true;
                    self.started(d);
                }
                Err(e) if matches!(e.code, ErrorCode::HostNameTaken | ErrorCode::InvalidToken) => return Err(e),
                Err(_) => {
                    self.hold = true;
                    return Ok(true);
                }
            }
        }
        if w.host.round {
            while self.live() < max {
                let body = json!({ "instance": self.instance });
                match self.rm().post::<Option<Dispatch>>(&format!("/api/hosts/{}/pull", self.name), body) {
                    Ok(Some(d)) => {
                        changed = true;
                        self.started(d);
                    }
                    Ok(None) => {
                        self.hold = true;
                        return Ok(true);
                    }
                    Err(e) if matches!(e.code, ErrorCode::HostNameTaken | ErrorCode::InvalidToken) => return Err(e),
                    Err(_) => {
                        self.hold = true;
                        return Ok(true);
                    }
                }
            }
        }
        Ok(changed)
    }

    /// 还没在停的 agent 个数（占着会话位：一个会话 = 一个 agent 进程）
    fn live(&self) -> usize {
        self.runs.len()
    }

    /// 服务端开始了一次派发（已领取）：在本机启动 agent；没领到时只打印
    fn started(&mut self, d: Dispatch) {
        if d.state != "running" {
            self.say(format!(
                "{} {}  {}",
                self.p.dim("·"),
                self.p.cyan(&format!("{}#{}", d.board, d.card)),
                self.p.dim(&format!("没领到（{}）", d.result.as_deref().map(result_words).unwrap_or("已结束")))
            ));
            return;
        }
        // 先记进任务里：重新取到任务之前不会被当成「服务端已经结束」
        if let Some(w) = self.work.as_mut() {
            w.running.push(d.clone());
            w.queued.retain(|x| x.id != d.id);
        }
        match self.spawn(&d) {
            Ok(r) => {
                // 先记进 runs 再打印：之后无论发生什么，退出流程都会结束它
                let env = r.env.name.clone();
                self.runs.push(r);
                self.say(format!(
                    "{} {}  {}  {}  {}",
                    self.p.green("▶"),
                    self.p.cyan(&format!("{}#{}", d.board, d.card)),
                    self.p.yellow(d.name.as_deref().unwrap_or_default()),
                    self.p.dim(&env),
                    d.title
                ));
            }
            Err(msg) => {
                self.say(format!("{} {}  {}", self.p.red("✗"), self.p.cyan(&format!("{}#{}", d.board, d.card)), msg));
                let log = self.logs.join(format!("{}.log", d.id));
                let _ = std::fs::write(&log, format!("{msg}\n"));
                self.ended.push(Ended { id: d.id, code: None, tail: vec![msg], events: vec![], how: "exit", why: None });
                self.report_ended();
            }
        }
    }

    /// 在派发记录写的启动环境里启动 agent。旧版服务端不写启动环境、或写的环境本机已经没有：按本机的默认（第一个找到的）
    fn spawn(&self, d: &Dispatch) -> std::result::Result<Run, String> {
        let spec = self.agents.iter().find(|a| a.id == d.agent).ok_or_else(|| format!("本机没有 agent「{}」", d.agent))?;
        let cmd0 = spec.cmd.first().cloned().unwrap_or_default();
        let check = d
            .env
            .as_deref()
            .and_then(|e| spec.checks.iter().find(|c| c.env == e))
            .or_else(|| spec.default_check())
            .ok_or_else(|| format!("本机没找到 agent「{}」的命令 {cmd0}", d.agent))?;
        let env = self.envs.iter().find(|e| e.id == check.env).cloned().ok_or_else(|| format!("本机没有启动环境「{}」", check.env))?;
        let prog = check.prog.clone().ok_or_else(|| format!("在{}里没找到 agent「{}」的命令 {cmd0}", env.name, d.agent))?;
        let dir = self.dirs.iter().find(|(b, _)| *b == d.board).map(|(_, p)| p.clone()).ok_or_else(|| format!("本机没有白板 {} 的目录", d.board))?;
        if !dir.is_dir() {
            return Err(format!("目录不存在：{}", dir.display()));
        }
        let name = d.name.clone().unwrap_or_default();
        let (project, board) = d.board.split_once('/').unwrap_or((&d.board, ""));
        // 定时领取的会话做完一张后用 board next 接着做（手动派发的会话 board next 会返回 SESSION_DONE）
        let prompt = format!(
            "/board-skills 开始执行 {project}项目之中的{board}白板中的#{}任务（board host 派发：BOARD_AS={name}，BOARD_HOST={}；卡片已由你领取，不要 check、不要 claim、不要心跳，不要创建 goal；交审或交给人之后执行 board next --json，领到卡片就接着做，返回 SESSION_DONE 就结束）",
            d.card, self.name
        );
        // dsh 的一次性派发要带一份禁用 goal 的 profile 补丁，见 DSH_HEADLESS_PATCH
        let patch = self.run.join("dsh-headless.patch.yml");
        let uses_patch = spec.cmd.iter().any(|a| a.contains("{patch}"));
        if uses_patch {
            write_dsh_patch(&patch).map_err(|e| format!("无法写 {}：{e}", patch.display()))?;
        }
        let log = self.logs.join(format!("{}.log", d.id));
        let (out, err) = create_log(&log).map_err(|e| format!("无法创建日志 {}：{e}", log.display()))?;
        let conf = self.run.join(format!("{}.toml", d.id));
        write_run_config(&conf, self.rm().url(), self.rm().token()).map_err(|e| e.message)?;
        let patch_arg = patch.display().to_string();
        // 机器设置里选了模型、思考强度（服务端写进了派发记录）就按模板传给 agent
        let cmd = spec.cmd_for(check);
        let args = agent_args(&cmd, &spec.model_args, &spec.effort_args, d.model.as_deref(), d.effort.as_deref(), &prompt, &patch_arg);
        let mut c = env.command(&prog, &args, Some(&dir), true);
        c.env("BOARD_URL", self.rm().url())
            .env("BOARD_TOKEN", self.rm().token())
            .env("BOARD_AS", &name)
            .env("BOARD_HOST", &self.name)
            .env("BOARD", &d.board)
            .env("BOARD_CONFIG", &conf)
            // shell 的启动脚本里设了 BOARD_URL / BOARD_TOKEN 也不影响：包装脚本用这两个再设一遍
            .env("BOARD_HOST_URL", self.rm().url())
            .env("BOARD_HOST_TOKEN", self.rm().token())
            // skill 自带的 scripts/board 看到它就用 host 自己的 board：版本和 host 一致（skill 里打包的 board 可能更旧）
            .env("BOARD_EXE", &self.exe)
            // 在 Claude Code 的会话里运行 board host 时，别让 agent 以为自己是嵌套会话
            .env_remove("CLAUDECODE")
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        // WSL：环境变量经 WSLENV 传进去（BOARD_CONFIG 换成 WSL 路径，不传 BOARD_EXE），agent 的进程号写进 BOARD_PIDFILE
        let mut pidfile = None;
        if env.wsl().is_some() {
            let pf = self.run.join(format!("{}.pid", d.id));
            let _ = std::fs::remove_file(&pf);
            c.env("BOARD_PIDFILE", &pf).env("WSLENV", envs::wslenv(std::env::var("WSLENV").ok().as_deref()));
            pidfile = Some(pf);
        }
        // pi 在 Windows 上从 PATH 找 bash.exe；把 Git Bash 放到最前，免得落到没启用的 WSL bash
        if cfg!(windows) && spec.git_bash && env.wsl().is_none() {
            if let Some(bin) = envs::git_bash_dir() {
                let old = std::env::var_os("PATH").unwrap_or_default();
                let mut parts: Vec<std::ffi::OsString> = vec![bin.into_os_string()];
                parts.extend(std::env::split_paths(&old).map(|p| p.into_os_string()));
                if let Ok(path) = std::env::join_paths(parts) {
                    c.env("PATH", path);
                }
            }
        }
        envs::detach(&mut c);
        let child = c.spawn().map_err(|e| {
            let _ = std::fs::remove_file(&conf);
            format!("在{}里启动 {prog} 失败：{e}", env.name)
        })?;
        // 记下 agent 的进程号和启动时间：这个 host 没走完退出流程时，同名重启后据此结束它
        let procfile = self.run.join(format!("{}.proc", d.id));
        let start = Procs::new().and_then(|mut ps| ps.get(child.id())).map(|x| x.start).unwrap_or(0);
        let _ = std::fs::write(&procfile, format!("{} {start} {} {}#{}\n", child.id(), env.id, d.board, d.card));
        let fmt = logfmt::stream_for(&spec.id, &cmd);
        let now = Instant::now();
        Ok(Run {
            d: d.clone(),
            child,
            env,
            pidfile,
            log,
            conf,
            procfile,
            sent: 0,
            fmt,
            parser: fmt.map(events::Parser::new),
            ev_read: 0,
            ev_queue: vec![],
            ev_recent: VecDeque::new(),
            fmt_due: true,
            term_at: None,
            killed: false,
            spawned: now,
            lost: false,
            timeout: None,
            output_at: now,
            seen_len: 0,
        })
    }

    /// 定时领取：到了设置的时间点（机器本地时区），机器在线、没有暂停、没有正在进行的一轮，就开始一轮
    fn cron(&mut self, last: &mut String) {
        // 同一分钟只处理一次：多条表达式同时到点也只开始一轮
        let now = chrono::Local::now();
        let m = now.format("%Y-%m-%d %H:%M").to_string();
        let Some(w) = &self.work else { return };
        let due = cron_due(&now, w.host.cron_on, &w.host.cron, w.host.paused, w.host.round);
        cron_tick(&m, last, due, || match self.rm().post::<HostInfo>(&format!("/api/hosts/{}/round", self.name), json!({})) {
            Ok(_) => {
                self.say(format!("{} {m} 到点，开始一轮领取", self.p.green("⏰")));
                self.hold = false;
                if let Some(w) = self.work.as_mut() {
                    w.host.round = true;
                }
                true
            }
            Err(e) => {
                self.say(format!("{} {m} 到点，没能开始领取：{}", self.p.yellow("!"), e.message));
                false
            }
        });
    }

    /// 清理本机派发日志：结束超过 `log_days` 天的删掉，正在跑的留着；留下来的单个超过 50 MB 的只保留开头和结尾
    /// （只动已经结束的：agent 可能还拿着文件句柄，换掉内容会在它下次写入时留一段空洞）
    fn clean_logs(&mut self) {
        let keep: Vec<i64> = self.runs.iter().map(|r| r.d.id).collect();
        let days = self.log_days;
        let removed = clean_logs(&self.logs, days, &keep, std::time::SystemTime::now());
        if removed > 0 {
            self.say(format!("{} 删掉了 {removed} 个结束超过 {days} 天的本机日志", self.p.dim("·")));
        }
        let Ok(rd) = std::fs::read_dir(&self.logs) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".log") {
                continue;
            }
            let id: Option<i64> = name.trim_end_matches(".log").parse().ok();
            if id.is_some_and(|id| keep.contains(&id)) {
                continue;
            }
            if let Ok(Some(_)) = truncate_log(&e.path(), LOG_HEAD, LOG_TAIL, LOG_MAX) {
                self.say(format!("{} {name} 超过 50 MB，只留了开头 1 MB 和最后 10 MB", self.p.dim("·")));
            }
        }
    }

    /// 这次派发注入给 agent 的密钥值（当前令牌、`BOARD_TOKEN` 这类环境变量）：上传日志、写异常退出评论之前
    /// 换成 `<已隐藏>`，本机日志保留原文
    fn secrets(&self) -> Vec<String> {
        let mut out = vec![self.rm().token().to_string()];
        for k in ["BOARD_TOKEN", "BOARD_HOST_TOKEN"] {
            if let Some(v) = std::env::var(k).ok().filter(|v| !v.trim().is_empty()) {
                out.push(v);
            }
        }
        out.sort();
        out.dedup();
        out
    }

    fn upload_logs(&mut self) {
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let mut sent: Vec<(usize, u64)> = vec![];
        for (i, r) in self.runs.iter().enumerate() {
            // 这里只会有还在跑的派发（结束的已经从 runs 里移走并截断过）：不截断正在写的日志，
            // agent 还拿着它的句柄，换掉文件内容会在它下次写入时从旧位置接着写、中间留一段空洞
            let (lines, to) = new_lines(&r.log, r.sent, false);
            let lines = upload_lines_redacted(lines, r.fmt, &secrets);
            if lines.is_empty() {
                if to != r.sent {
                    sent.push((i, to));
                }
                continue;
            }
            if self.rm().post::<Value>(&format!("/api/dispatches/{}/log", r.d.id), json!({ "lines": lines })).is_ok() {
                sent.push((i, to));
            }
        }
        for (i, to) in sent {
            self.runs[i].sent = to;
        }
    }

    /// 上传一次派发剩下的日志（结束时）
    fn flush_log(&self, r: &Run, last: bool) {
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let (lines, _) = new_lines(&r.log, r.sent, last);
        let lines = upload_lines_redacted(lines, r.fmt, &secrets);
        if !lines.is_empty() {
            let _ = self.rm().post::<Value>(&format!("/api/dispatches/{}/log", r.d.id), json!({ "lines": lines }));
        }
    }

    /// 每 2 秒：解析每个在跑的派发新增的输出，把新的结构化事件传上去
    fn upload_events(&mut self) {
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let mut runs = std::mem::take(&mut self.runs);
        for r in runs.iter_mut() {
            read_events(r, false, &secrets);
            self.post_events(r);
        }
        self.runs = runs;
    }

    /// 一次派发结束（last：agent 已经退出，最后不完整的一行也读）或会话换到下一张之前：读完新增的输出，把事件都传上去
    fn flush_events(&self, r: &mut Run, last: bool) {
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        read_events(r, last, &secrets);
        self.post_events(r);
    }

    /// 传事件：带上格式（只有原始输出的 agent 是空字符串）。连不上、服务端出错（5xx）时留到下一次；其他拒绝（旧版服务端
    /// 没有这个接口、令牌不让调、派发不存在）就丢掉，不反复重试
    fn post_events(&self, r: &mut Run) {
        if r.ev_queue.is_empty() && !r.fmt_due {
            return;
        }
        let events = board_core::runlog::merge_calls(std::mem::take(&mut r.ev_queue));
        let fmt = r.fmt.map(Stream::key).unwrap_or("");
        match self.rm().post::<Value>(&format!("/api/dispatches/{}/events", r.d.id), json!({ "fmt": fmt, "events": events })) {
            Ok(_) => r.fmt_due = false,
            Err(e) if matches!(e.code, ErrorCode::Unreachable | ErrorCode::Internal) => r.ev_queue = events,
            Err(_) => r.fmt_due = false,
        }
    }

    /// 退出：按「人停止」结束本机所有 agent（先 SIGTERM，10 秒后 SIGKILL），上报结束，再让机器离线
    fn shutdown(&mut self, graceful: bool) {
        for r in self.readies.iter_mut() {
            r.kill();
        }
        // 自检启动的 agent、git：一起结束（结果不再回报，请求过期后可以重新自检）
        for pid in self.selftest_pids.lock().map(|p| p.clone()).unwrap_or_default() {
            kill_tree(pid, true);
        }
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
            self.finish_run(r, code, "host_exit");
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
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// 去掉 Windows 上 `canonicalize()` 加的 `\\?\` 前缀：git 不把带前缀的路径当本地路径
    /// （`git clone \\?\C:\…` 会报「hostname contains invalid characters」）
    fn plain(p: &Path) -> PathBuf {
        match p.to_string_lossy().strip_prefix(r"\\?\") {
            Some(rest) => PathBuf::from(rest),
            None => p.to_path_buf(),
        }
    }

    /// 派发被服务端结束后找没推的改动：没提交的文件、不在远端的提交；agent 的 worktree 按 board-skills 的约定找
    #[test]
    fn leftovers_find_unpushed_changes_in_skill_worktree() {
        let t = tempfile::tempdir().unwrap();
        let root = plain(&t.path().canonicalize().unwrap());
        let remote = root.join("remote.git");
        std::fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "-q", "--bare", "-b", "main"]);
        let repo = root.join("board");
        git(&root, &["clone", "-q", remote.to_str().unwrap(), "board"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "a"]);
        git(&repo, &["push", "-q", "origin", "HEAD:main"]);
        assert!(unpushed(&repo).is_none(), "都推了");
        let wt = skill_worktree(&repo, "claude-mbp/opus-5.5").unwrap();
        assert_eq!(wt, root.join("board.wt").join("claude-mbp-opus-5.5"));
        git(&repo, &["worktree", "add", "-q", "--detach", wt.to_str().unwrap(), "HEAD"]);
        assert_eq!(skill_worktree(&wt, "x/y").unwrap(), root.join("board.wt").join("x-y"), "在 worktree 里也以主仓库为准");
        assert!(leftovers(&repo, "claude-mbp/opus-5.5").is_empty());
        // worktree 里改了没提交、提交了没推
        std::fs::write(wt.join("b.txt"), "b").unwrap();
        assert_eq!(unpushed(&wt).as_deref(), Some("1 个文件没提交"));
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-q", "-m", "b"]);
        let got = leftovers(&repo, "claude-mbp/opus-5.5");
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].0, wt);
        assert!(got[0].1.starts_with("提交 ") && got[0].1.ends_with("不在任何远端分支上"), "{}", got[0].1);
        // 推到 wip 分支之后就不算了
        git(&wt, &["push", "-q", "origin", "HEAD:refs/heads/wip/main/1"]);
        git(&wt, &["fetch", "-q", "origin"]);
        assert!(leftovers(&repo, "claude-mbp/opus-5.5").is_empty());
        // 不是 git 仓库、不存在：不算
        assert!(unpushed(&root).is_none());
        assert!(unpushed(&root.join("nope")).is_none());
    }

    #[test]
    fn offline_note_says_what_happens_to_running_agents() {
        assert_eq!(offline_note(Duration::from_secs(31), 0), "和服务端断开 31 秒了（继续重试）");
        let s = offline_note(Duration::from_secs(30), 2);
        assert!(s.starts_with("和服务端断开 30 秒了：2 个 agent 还在跑"), "{s}");
        assert!(s.contains("50 秒") && s.contains("board hold"), "{s}");
    }

    /// 卡片 #199：超过 50 MB 的本机日志只留开头 1 MB、最后 10 MB，中间一行省略说明；
    /// 不超过的不动
    #[test]
    fn huge_logs_keep_head_and_tail_with_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("1.log");
        // 20 MB，每行 100 字节：全留下就太占地方
        let line = format!("{}\n", "x".repeat(99));
        let mut body = String::new();
        body.push_str("HEAD-MARK\n");
        while body.len() < 20 * 1024 * 1024 {
            body.push_str(&line);
        }
        body.push_str("TAIL-MARK\n");
        std::fs::write(&f, &body).unwrap();
        let max = 5 * 1024 * 1024;
        let (head, tail) = (1 * 1024 * 1024, 2 * 1024 * 1024);
        let len = truncate_log(&f, head, tail, max).unwrap().expect("超过 50 MB 才截断");
        let got = std::fs::read_to_string(&f).unwrap();
        assert_eq!(got.len() as u64, len, "返回的长度和写回去的一致");
        assert!(got.starts_with("HEAD-MARK\n"), "开头留着");
        assert!(got.ends_with("TAIL-MARK\n"), "结尾留着");
        assert!(got.contains("中间") && got.contains("已省略"), "有一行省略说明：{got:.0}");
        assert!((got.len() as u64) < 4 * 1024 * 1024, "截断后小了很多：{}", got.len());
        // 刚好到上限：不截断
        let small = dir.path().join("2.log");
        std::fs::write(&small, vec![b'a'; max as usize]).unwrap();
        assert_eq!(truncate_log(&small, head, tail, max).unwrap(), None);
        assert_eq!(std::fs::metadata(&small).unwrap().len(), max);
    }

    /// 卡片 #199：结束超过 30 天的本机日志在 board host 启动后被删除，30 天内的还在；
    /// 正在跑的派发的日志不删；目录里不是日志的文件不碰
    #[test]
    fn old_logs_are_removed_but_recent_and_running_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        let old = now - Duration::from_secs(31 * 24 * 3600);
        let recent = now - Duration::from_secs(29 * 24 * 3600);
        let files = [
            ("1.log", std::time::SystemTime::now()),
            ("2.log", recent),
            ("3.log", old),
            ("ready-default-main.log", old),
            ("clone-default-main.log", old),
        ];
        for (name, t) in files {
            let p = dir.path().join(name);
            std::fs::write(&p, "x\n").unwrap();
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        std::fs::write(dir.path().join("10.pid"), "x").unwrap();
        // 1 号派发还在跑：它的日志再老也留着
        std::fs::File::options().write(true).open(dir.path().join("1.log")).unwrap().set_modified(old).unwrap();
        // 删掉 3.log 和 ready、clone 两个：1.log（还在跑）、2.log（30 天内）、10.pid 留着
        let removed = clean_logs(dir.path(), 30, &[1], now);
        assert_eq!(removed, 3);
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(left.contains(&"1.log".to_string()) && left.contains(&"2.log".to_string()), "{left:?}");
        assert!(!left.contains(&"3.log".to_string()) && !left.contains(&"clone-default-main.log".to_string()), "{left:?}");
        assert!(left.contains(&"10.pid".to_string()), "不是日志的文件不碰：{left:?}");
        // 天数改成 1：29 天前的也删
        let removed = clean_logs(dir.path(), 1, &[1], now);
        assert_eq!(removed, 1);
        assert!(dir.path().join("1.log").exists(), "还在跑的留着");
    }

    /// 上传的行先转写、再脱敏；本机日志文件保留原文
    #[test]
    fn uploads_are_redacted_but_the_local_file_keeps_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("7.log");
        let token = "bd_0123456789abcdef0123456789abcdef";
        std::fs::write(&f, format!("BOARD_TOKEN={token}\nAuthorization: Bearer {token}\nworking\n")).unwrap();
        let (lines, _) = new_lines(&f, 0, false);
        let up = upload_lines_redacted(lines, None, &[token]);
        assert_eq!(up, vec!["BOARD_TOKEN=<已隐藏>", "Authorization: Bearer <已隐藏>", "working"]);
        let local = std::fs::read_to_string(&f).unwrap();
        assert!(local.contains(token), "本机日志保留原文：{local}");
    }

    /// 日志文件只有自己能读（0600）
    #[cfg(unix)]
    #[test]
    fn log_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("logs").join("1.log");
        create_log(&f).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        // 已经存在的（以前建的 0644）也改成 0600
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        restrict_log(&f);
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        // 截断之后还是 0600
        std::fs::write(&f, vec![b'x'; 200]).unwrap();
        truncate_log(&f, 10, 10, 100).unwrap().unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    }

    /// 卡片 #232：环境检查在 `$SHELL -l -i -c` 里跑，CI 上没有终端，bash 往 stderr 打作业控制提示、退出时打 `logout`。
    /// 这些不能进上报的 tail（不然界面显示「环境检查没通过：logout」）。真正的错误要留着
    #[test]
    fn ready_tail_skips_login_shell_noise() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("ready-default-main.log");
        std::fs::write(
            &f,
            "bash: cannot set terminal process group (-1): Inappropriate ioctl for device\nbash: no job control in this shell\nchecking\nmissing ok.txt\nlogout\n",
        )
        .unwrap();
        let tail = tail_lines(&f, READY_TAIL + 5, None);
        assert_eq!(tail, vec!["checking", "missing ok.txt"]);
        // 只有噪声时 tail 是空的：调用方按退出码写原因，不显示 logout
        std::fs::write(&f, "bash: no job control in this shell\nlogout\n").unwrap();
        let tail = tail_lines(&f, READY_TAIL + 5, None);
        assert!(tail.is_empty(), "{tail:?}");
    }

    /// 卡片 #232：Ubuntu 的 `/etc/bash.bashrc` 在登录 shell 里打 sudo 提示（本仓库 CI 的 ubuntu-latest 就有）。
    /// 它和 bash 的作业控制提示一样不能进 tail、不能混进会话的「过程」
    #[test]
    fn ready_tail_skips_the_sudo_hint_too() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("ready-default-main.log");
        std::fs::write(
            &f,
            "bash: cannot set terminal process group (-1): Inappropriate ioctl for device\nbash: no job control in this shell\nTo run a command as administrator (user \"root\"), use \"sudo <command>\".\nSee \"man sudo_root\" for details.\n\nchecking\nmissing ok.txt\n",
        )
        .unwrap();
        assert_eq!(tail_lines(&f, READY_TAIL + 5, None), vec!["checking", "missing ok.txt"]);
        // 会话的「原始输出」里也不出现（空行本来就不算内容）
        let up: Vec<String> = upload_lines(new_lines(&f, 0, true).0, None).into_iter().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(up, vec!["checking", "missing ok.txt"]);
    }

    #[test]
    fn reads_only_whole_lines() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.log");
        std::fs::write(&f, "one\ntwo\nthr").unwrap();
        let (lines, at) = new_lines(&f, 0, false);
        assert_eq!(lines, vec!["one", "two"]);
        assert_eq!(at, 8);
        let (lines, at2) = new_lines(&f, at, true);
        assert_eq!(lines, vec!["thr"]);
        assert_eq!(at2, 11);
        assert_eq!(tail_lines(&f, 2, None), vec!["two", "thr"]);
    }

    /// 卡片 #211、#184：内置模板都用结构化输出启动（Claude Code、pi 的 -p 只在结束时打印结果；Codex、dsh 的 JSON 里
    /// 才有工具调用和用量）。上传和异常退出评论里的「日志最后 20 行」是转写后的一行一行；本机日志文件里还是原样的事件
    #[test]
    fn quiet_agents_stream_progress_into_the_log() {
        let cmd = |id: &str| BUILTIN.iter().find(|b| b.id == id).unwrap().cmd.join(" ");
        assert!(cmd("claude").contains("-p {prompt}") && cmd("claude").ends_with("--output-format stream-json --verbose"), "{}", cmd("claude"));
        assert_eq!(cmd("pi"), "pi --mode json {prompt}", "pi 的 -p 只写最终回答，--mode json 才有过程");
        assert_eq!(cmd("codex"), "codex exec --json --dangerously-bypass-approvals-and-sandbox {prompt}");
        assert!(cmd("dsh").starts_with("dsh --profile headless --json "), "{}", cmd("dsh"));
        // 每个内置模板都认得出事件流
        for b in &BUILTIN {
            let c: Vec<String> = b.cmd.iter().map(|s| s.to_string()).collect();
            assert!(logfmt::stream_for(b.id, &c).is_some(), "{} 的内置模板要用结构化输出", b.id);
        }

        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("7.log");
        let ev = |v: serde_json::Value| v.to_string() + "\n";
        let mut raw = String::from("[claude-code:unrecognized_model] {\"model\":\"x\"}\n");
        raw += &ev(json!({ "type": "system", "subtype": "init", "model": "opus", "cwd": "/repo", "claude_code_version": "2.1.311" }));
        raw += &ev(json!({ "type": "stream_event", "event": {}, "parent_tool_use_id": null }));
        raw += &ev(json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "git status" } }] }, "parent_tool_use_id": null }));
        // 工具结果整段在一行里（这里 1 MB）：转写后只剩一行
        let big = "x".repeat(1024 * 1024);
        raw += &ev(json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": format!("On branch main\n{big}") }] }, "parent_tool_use_id": null }));
        std::fs::write(&f, &raw).unwrap();
        let (lines, at) = new_lines(&f, 0, false);
        assert_eq!(at, raw.len() as u64, "一行很长也整行读到");
        let up = upload_lines(lines, Some(Stream::Claude));
        assert_eq!(
            up,
            vec![
                "[claude-code:unrecognized_model] {\"model\":\"x\"}",
                "● 开始 · Claude Code 2.1.311 · opus · /repo",
                "▸ Bash  git status",
                "  ↳ On branch main  （共 2 行）",
            ]
        );
        assert_eq!(tail_lines(&f, 2, Some(Stream::Claude)), vec!["▸ Bash  git status", "  ↳ On branch main  （共 2 行）"]);
        // 不转写的 agent 原样上传，最多最后 500 行
        let many: Vec<String> = (0..800).map(|i| format!("line {i}")).collect();
        let up = upload_lines(many, None);
        assert_eq!((up.len(), up[0].as_str()), (LOG_KEEP, "line 300"));
    }

    #[test]
    fn builtin_order_and_overrides() {
        // 内置顺序就是默认顺序：dsh 第一
        assert_eq!(BUILTIN.iter().map(|b| b.id).collect::<Vec<_>>(), vec!["dsh", "claude", "pi", "codex"]);
        assert!(BUILTIN.iter().find(|b| b.id == "pi").unwrap().git_bash, "pi 在 Windows 上要先找 Git Bash");
        assert!(!BUILTIN.iter().find(|b| b.id == "pi").unwrap().win.contains(&"cmd"), "pi 不支持 cmd");
        for b in &BUILTIN {
            check_agent_id(b.id).unwrap();
            assert_eq!(b.cmd.iter().filter(|a| a.contains("{prompt}")).count(), 1, "{} 的模板里要有一个 {{prompt}}", b.id);
            assert!(!b.win.contains(&"direct"), "内置模板在 Windows 上都经过 shell 启动");
        }
        // dsh 的一次性派发要带禁用 goal 的 profile 补丁；其他 agent 不带
        let dsh = BUILTIN.iter().find(|b| b.id == "dsh").unwrap();
        assert_eq!(dsh.cmd.iter().filter(|a| a.contains("{patch}")).count(), 1, "dsh 的模板里要有一个 {{patch}}");
        assert!(DSH_HEADLESS_PATCH.contains("goal-round-driver"), "补丁要关掉 goal-round-driver");
        assert!(DSH_HEADLESS_PATCH.contains("tool-goal"), "补丁要关掉 goal 工具");
        assert!(BUILTIN.iter().filter(|b| b.id != "dsh").all(|b| !b.cmd.iter().any(|a| a.contains("{patch}"))));
        // 派发名字的第二段（模型名）：dsh、claude 有默认值（dsh-win/deepseek-v4.1-flash、claude-mbp/opus-5.5 这类），
        // pi、codex 没有默认的模型（各自配好 provider 才有）
        assert_eq!(dsh.model, "deepseek-v4.1-flash");
        assert_eq!(BUILTIN.iter().find(|b| b.id == "claude").unwrap().model, "opus-5.5");
        assert!(BUILTIN.iter().find(|b| b.id == "pi").unwrap().model.is_empty());
        assert!(BUILTIN.iter().find(|b| b.id == "codex").unwrap().model.is_empty());
        // 内置 agent 全部换成找不到的命令：本机真装了 dsh、pi、codex 也不去执行它们的 --version
        // （本机执行 codex 会被 macOS 拦下、反复弹「包含恶意软件」的提示）
        let nope = |id: &str| AgentConf { id: id.into(), name: None, model: None, cmd: vec![format!("/nope/{id}"), "{prompt}".into()], ..Default::default() };
        let conf = vec![
            nope("dsh"),
            nope("claude"),
            nope("pi"),
            nope("codex"),
            AgentConf { id: "mycli".into(), name: Some("My CLI".into()), model: Some("gpt-5".into()), cmd: vec!["/nope/mycli".into(), "{prompt}".into()], ..Default::default() },
        ];
        let specs = agent_specs(&conf, &[], &[Env::direct()]).unwrap();
        assert_eq!(specs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["dsh", "claude", "pi", "codex", "mycli"]);
        let claude = specs.iter().find(|s| s.id == "claude").unwrap();
        assert_eq!(claude.name, "Claude Code");
        assert_eq!(claude.model, "opus-5.5", "配置文件没写 model：保留内置模板的默认值");
        assert!(claude.default_check().is_none(), "换了命令就不再看内置的备用位置");
        assert_eq!(specs[4].name, "My CLI");
        assert_eq!(specs[4].model, "gpt-5");
        assert_eq!(specs[4].checks.len(), 1, "只有直接启动");
        assert!(!specs[4].misses_skill(&Env::direct(), &[]), "配置文件里的 agent 不检查 skill");
        let r = specs[4].report();
        assert_eq!((r.found, r.envs.len(), r.env, r.no_skill, r.model.as_str()), (false, 1, None, None, "gpt-5"));
        // 配置文件里的 model 覆盖内置模板的
        let conf = vec![AgentConf { id: "dsh".into(), name: None, model: Some("deepseek-reasoner".into()), cmd: vec!["/nope/dsh".into(), "{prompt}".into()], ..Default::default() }];
        let specs = agent_specs(&conf, &[], &[Env::direct()]).unwrap();
        assert_eq!(specs[0].model, "deepseek-reasoner");
    }

    fn sv(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// 机器设置里的模型、思考强度怎么传给 agent（PRD §9.1）
    #[test]
    fn model_and_effort_args() {
        let b = |id: &str| BUILTIN.iter().find(|b| b.id == id).unwrap();
        let args = |id: &str, m: Option<&str>, e: Option<&str>| {
            let x = b(id);
            agent_args(&sv(x.cmd), &sv(x.model_args), &sv(x.effort_args), m, e, "P", "/tmp/patch.yml")
        };
        // 都没选：和原来的拼法完全一样
        for x in &BUILTIN {
            let old: Vec<String> = x.cmd.iter().skip(1).map(|a| a.replace("{prompt}", "P").replace("{patch}", "/tmp/patch.yml")).collect();
            assert_eq!(args(x.id, None, None), old, "{} 没选时命令不变", x.id);
        }
        // Claude Code：--model、--effort 插在提示词前面
        assert_eq!(
            args("claude", Some("sonnet"), Some("high")),
            sv(&["-p", "--model", "sonnet", "--effort", "high", "P", "--dangerously-skip-permissions", "--output-format", "stream-json", "--verbose"])
        );
        // Codex：-m、-c model_reasoning_effort=…，子命令 exec 还在最前
        assert_eq!(
            args("codex", Some("gpt-5-codex"), Some("high")),
            sv(&["exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "-m", "gpt-5-codex", "-c", "model_reasoning_effort=high", "P"])
        );
        // pi：--model、--thinking；只选一项时只传那一项
        assert_eq!(args("pi", Some("anthropic/claude-sonnet"), Some("xhigh")), sv(&["--mode", "json", "--model", "anthropic/claude-sonnet", "--thinking", "xhigh", "P"]));
        assert_eq!(args("pi", None, Some("low")), sv(&["--mode", "json", "--thinking", "low", "P"]));
        // dsh 没有这两项：选了也不传
        assert_eq!(args("dsh", Some("x"), Some("high")), args("dsh", None, None));
        // 模板里没有 {prompt}：放在最后
        assert_eq!(agent_args(&sv(&["a", "b"]), &sv(&["--m", "{model}"]), &[], Some("x"), None, "P", ""), sv(&["b", "--m", "x"]));
        // 上报能选什么：claude 能选模型和五档强度；dsh 都不能；配置文件里新加的 agent 写了 effort_args 没写 efforts 时是三档
        let conf = vec![
            AgentConf { id: "claude".into(), cmd: sv(&["/nope/claude", "{prompt}"]), ..Default::default() },
            AgentConf { id: "dsh".into(), cmd: sv(&["/nope/dsh", "{prompt}"]), ..Default::default() },
            AgentConf { id: "mycli".into(), cmd: sv(&["/nope/mycli", "{prompt}"]), effort_args: Some(sv(&["--effort={effort}"])), ..Default::default() },
            AgentConf { id: "plain".into(), cmd: sv(&["/nope/plain", "{prompt}"]), ..Default::default() },
        ];
        let specs = agent_specs(&conf, &[], &[Env::direct()]).unwrap();
        let opts = |id: &str| specs.iter().find(|s| s.id == id).unwrap().report().opts.unwrap();
        assert_eq!(opts("claude"), AgentOpts { model: true, models: sv(&["opus", "sonnet", "haiku", "fable"]), efforts: sv(&["low", "medium", "high", "xhigh", "max"]) });
        assert_eq!(opts("dsh"), AgentOpts::default());
        assert_eq!(opts("mycli"), AgentOpts { model: false, models: vec![], efforts: sv(&["low", "medium", "high"]) });
        assert_eq!(opts("plain"), AgentOpts::default(), "没写 model_args、effort_args 的 agent 不显示这两项");
    }

    #[test]
    fn expands_home() {
        let home = expand("~");
        assert!(home.is_absolute() || home == PathBuf::from("~"));
        assert_eq!(expand("/abs/x"), PathBuf::from("/abs/x"));
    }

    /// 目录状态缓存：刚看过的沿用，隔 DIR_CHECK_EVERY 才重新跑 git 子进程（心跳 10 秒一次，不能每次都跑）；
    /// 目录变了（或克隆完）就丢掉旧结果。上报的 checkedAt 是真正跑 git 的那一次的时间
    #[test]
    fn dir_checks_are_cached_between_beats() {
        let t = tempfile::tempdir().unwrap();
        let repo = plain(&t.path().canonicalize().unwrap());
        git(&repo, &["init", "-q", "-b", "main"]);
        let taken = |checks: &[DirCheck]| checks.iter().find(|c| c.path == repo).map(|c| (c.status, c.at));
        // 还没有结果：当成不存在（不会跑 git 以外的东西，也不会让上报出错）
        assert!(Host::dir_check_due(&[], &repo));
        let mut checks = vec![];
        let (status, origin, branch) = check_git_dir(&repo);
        assert_eq!(status, "ok");
        checks.push(DirCheck { path: repo.clone(), status, origin, branch, at: Instant::now() });
        let first = taken(&checks).unwrap();
        // 刚看过：不到时间，不再跑 git；结果和看的时间都不变
        assert!(!Host::dir_check_due(&checks, &repo));
        assert_eq!(taken(&checks).unwrap().1, first.1);
        // 到时间了：重新看，看的时间换新的
        checks[0].at = Instant::now() - DIR_CHECK_EVERY;
        assert!(Host::dir_check_due(&checks, &repo));
        let (status, origin, branch) = check_git_dir(&repo);
        checks[0] = DirCheck { path: repo.clone(), status, origin, branch, at: Instant::now() };
        assert_eq!(taken(&checks).unwrap().0, "ok");
        assert!(taken(&checks).unwrap().1 > first.1);
        // 目录没了：到时间之后看到「不存在」
        std::fs::remove_dir_all(&repo).unwrap();
        checks[0].at = Instant::now() - DIR_CHECK_EVERY;
        let (status, _, _) = check_git_dir(&repo);
        assert_eq!(status, "missing");
    }

    #[test]
    fn missing_dirs_are_reported_instead_of_failing_startup() {
        let missing = "~/board-test-no-such-dir";
        let (dirs, raw) = parse_dirs(vec![("default/main".into(), missing.into())]).unwrap();
        assert_eq!(raw, vec![("default/main".to_string(), missing.to_string())], "配置里的 ~ 原样保留");
        assert_eq!(dirs[0].0, "default/main");
        assert!(!dirs[0].1.is_dir(), "目录可以暂时不存在，交给界面显示并克隆");
    }

    #[test]
    fn skill_check_sees_project_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".agents").join("skills").join(SKILL)).unwrap();
        std::fs::write(repo.join(".agents").join("skills").join(SKILL).join("SKILL.md"), "x").unwrap();
        let spec = Spec {
            json_gate: None,
            local_override: false,
            id: "pi".into(),
            name: "pi".into(),
            model: String::new(),
            cmd: vec!["pi".into()],
            skills: vec!["/board-test-no-such-skills".into()],
            project: vec![".agents/skills".into(), ".pi/skills".into()],
            git_bash: true,
            checks: vec![Check { env: "direct".into(), found: true, version: String::new(), prog: Some("pi".into()) }],
            no_skill: BTreeMap::new(),
            model_args: vec![],
            effort_args: vec![],
            efforts: vec![],
            models: vec![],
        };
        let direct = Env::direct();
        let dirs = [("default/main".into(), repo.clone())];
        assert!(!spec.misses_skill(&direct, &dirs), "项目里的 .agents/skills 算装好了");
        assert!(spec.misses_skill(&direct, &[]), "没有项目目录时仍然提醒");

        // 上报给服务端：看不到的标出该装到哪里，看得到的不标
        let mut spec = spec;
        spec.check_skills(std::slice::from_ref(&direct), &[]);
        let r = spec.report();
        let to = r.no_skill.clone().expect("看不到 board-skills 要上报");
        assert!(to.contains("/board-test-no-such-skills/board-skills") && to.contains(".agents/skills/board-skills"), "{to}");
        assert_eq!(r.envs[0].no_skill.as_deref(), Some(to.as_str()), "每个启动环境各自上报");
        spec.check_skills(std::slice::from_ref(&direct), &dirs);
        let r = spec.report();
        assert_eq!((r.no_skill, r.envs[0].no_skill.clone()), (None, None), "装了的不标");

        // Claude Code 不读 .agents/skills：仓库里只有 .agents/skills 时仍然提醒，提醒里写 .claude/skills
        let mut claude = Spec { id: "claude".into(), project: vec![".claude/skills".into()], ..spec };
        assert!(claude.misses_skill(&direct, &dirs), "Claude Code 看不到仓库的 .agents/skills");
        claude.check_skills(std::slice::from_ref(&direct), &dirs);
        let to = claude.report().no_skill.unwrap();
        assert!(to.contains("仓库的 .claude/skills/board-skills") && !to.contains(".agents/skills"), "{to}");
    }

    /// 共用配置的 agent 模板（#188）：合在内置模板后面、本机配置文件前面；本机配置覆盖了共用模板时标出来
    #[test]
    fn server_templates_merge_between_builtin_and_local_config() {
        let t = |id: &str, cmd: &[&str]| AgentTemplate { id: id.into(), cmd: cmd.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let server = vec![
            AgentTemplate { name: "Kimi".into(), model: "k2".into(), model_args: vec!["--model".into(), "{model}".into()], ..t("kimi", &["/nope/kimi", "{prompt}"]) },
            t("claude", &["/nope/claude", "-p", "{prompt}"]),
            t("pi", &["/nope/pi", "{prompt}"]),
        ];
        let local = vec![AgentConf { id: "pi".into(), cmd: vec!["/nope/my-pi".into(), "{prompt}".into()], ..Default::default() }];
        let specs = agent_specs(&local, &server, &[]).unwrap();
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["dsh", "claude", "pi", "codex", "kimi"], "共用模板里新加的排在内置的后面");
        let kimi = specs.iter().find(|s| s.id == "kimi").unwrap();
        assert_eq!((kimi.name.as_str(), kimi.model.as_str(), kimi.cmd[0].as_str()), ("Kimi", "k2", "/nope/kimi"));
        assert!(kimi.report().opts.unwrap().model, "模板写了传模型的参数：能选模型");
        assert_eq!(kimi.efforts, ["low", "medium", "high"]);
        let claude = specs.iter().find(|s| s.id == "claude").unwrap();
        assert_eq!(claude.cmd, ["/nope/claude", "-p", "{prompt}"], "共用模板覆盖内置命令");
        assert_eq!(claude.name, "Claude Code", "没写名称时用内置的");
        assert!(!claude.model_args.is_empty(), "没写传参方式时用内置的");
        assert!(claude.json_gate.is_none() && !claude.report().local_override);
        let pi = specs.iter().find(|s| s.id == "pi").unwrap();
        assert_eq!(pi.cmd[0], "/nope/my-pi", "本机配置文件优先");
        assert!(pi.report().local_override, "本机配置覆盖了共用模板：上报时标出来");
        // 没有共用模板时，本机配置覆盖内置模板不算
        let specs = agent_specs(&local, &[], &[]).unwrap();
        assert!(!specs.iter().find(|s| s.id == "pi").unwrap().local_override);
    }

    /// dsh 的 --json 从 0.2.2 起才有：本机更旧的去掉它，按原始输出运行；配置文件覆盖了命令的照原样用
    #[test]
    fn json_flag_is_dropped_for_old_dsh() {
        assert!(older_than("0.2.1", "0.2.2") && older_than("dsh 0.1.9", "0.2.2") && older_than("0.2", "0.2.2"));
        assert!(!older_than("0.2.2-rc.1", "0.2.2") && !older_than("0.10.0", "0.2.2") && !older_than("1.0", "0.2.2"));
        assert!(!older_than("", "0.2.2") && !older_than("unknown", "0.2.2"), "看不出版本时当作够新");
        let specs = agent_specs(&[], &[], &[]).unwrap();
        let dsh = specs.iter().find(|s| s.id == "dsh").unwrap();
        let check = |v: &str| Check { env: "direct".into(), found: true, version: v.into(), prog: Some("dsh".into()) };
        assert!(dsh.cmd_for(&check("0.2.2-rc.1")).contains(&"--json".to_string()));
        let old = dsh.cmd_for(&check("0.2.0"));
        assert!(!old.contains(&"--json".to_string()), "{old:?}");
        assert_eq!(logfmt::stream_for("dsh", &old), None, "去掉之后按原始输出处理");
        let conf = vec![AgentConf { id: "dsh".into(), cmd: vec!["dsh".into(), "--json".into(), "{prompt}".into()], ..Default::default() }];
        let specs = agent_specs(&conf, &[], &[]).unwrap();
        let dsh = specs.iter().find(|s| s.id == "dsh").unwrap();
        assert!(dsh.cmd_for(&check("0.1.0")).contains(&"--json".to_string()), "配置文件写的命令照原样用");
    }

    /// 结构化事件：按行解析、脱敏，异常退出评论里的最后几个事件把工具调用和结果合成一条
    #[test]
    fn events_are_read_from_the_log_and_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("9.log");
        let ev = |v: serde_json::Value| v.to_string() + "\n";
        let mut raw = String::from("warning: something\n");
        raw += &ev(json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "令牌是 sekrit-token-123" }] }, "parent_tool_use_id": null }));
        raw += &ev(json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "cargo test" } }] }, "parent_tool_use_id": null }));
        std::fs::write(&log, &raw).unwrap();
        let mut r = test_run(&log, Some(Stream::Claude));
        read_events(&mut r, false, &["sekrit-token-123"]);
        assert_eq!(r.ev_read, raw.len() as u64);
        assert_eq!(r.ev_queue.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["text", "message", "tool"]);
        assert_eq!(r.ev_queue[1].text.as_deref(), Some("令牌是 <已隐藏>"));
        // 结果后到：最后几个事件里合进同一条；最后一行没有换行，last 时也读
        let more = ev(json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "test failed", "is_error": true }] }, "parent_tool_use_id": null }));
        std::fs::write(&log, raw.clone() + more.trim_end()).unwrap();
        read_events(&mut r, false, &[]);
        assert_eq!(r.ev_queue.len(), 3, "不完整的一行先不读");
        read_events(&mut r, true, &[]);
        assert_eq!(r.ev_queue.len(), 4);
        let lines = recent_lines(&r);
        assert_eq!(lines.last().unwrap(), "▸ Bash  cargo test  ✗ test failed");
        assert_eq!(lines.len(), 3);
        // 只有原始输出的 agent：只挪位置，不出事件
        let mut r = test_run(&log, None);
        read_events(&mut r, true, &[]);
        assert!(r.ev_queue.is_empty() && r.ev_read > 0);
    }

    fn test_run(log: &Path, fmt: Option<Stream>) -> Run {
        let child = Command::new("true").spawn().unwrap();
        Run {
            d: Dispatch {
                id: 9,
                board: "default/main".into(),
                card: 1,
                title: String::new(),
                host_id: 1,
                host: "mbp".into(),
                agent: "claude".into(),
                agent_label: "Claude Code".into(),
                name: None,
                env: None,
                env_name: None,
                source: "manual".into(),
                by: board_core::Actor::system(),
                state: "running".into(),
                result: None,
                created_at: 0,
                started_at: None,
                ended_at: None,
                exit_code: None,
                stop_by: None,
                beat_at: None,
                session: 9,
                seq: 1,
                total: 1,
                model: None,
                effort: None,
            },
            child,
            env: Env::direct(),
            pidfile: None,
            log: log.to_path_buf(),
            conf: log.with_extension("toml"),
            procfile: log.with_extension("proc"),
            sent: 0,
            fmt,
            parser: fmt.map(events::Parser::new),
            ev_read: 0,
            ev_queue: vec![],
            ev_recent: VecDeque::new(),
            fmt_due: true,
            term_at: None,
            killed: false,
            spawned: Instant::now(),
            lost: false,
            timeout: None,
            output_at: Instant::now(),
            seen_len: 0,
        }
    }

    /// 卡片 #251：`reap` 与 `shutdown` 共用的结束逻辑。兜底停止过的一律报「超时」（原因照原样带上），
    /// 否则用调用方给的结果；错误摘录里不出现令牌
    #[test]
    fn ended_takes_the_timeout_reason_and_redacts() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("9.log");
        std::fs::write(&log, "BOARD_TOKEN=secret-token\nboom\n").unwrap();
        let r = test_run(&log, None);
        let e = ended_of(&r, Some(3), "exit", &["secret-token"]);
        assert_eq!((e.id, e.code, e.how, e.why.as_deref()), (9, Some(3), "exit", None));
        assert!(e.tail.iter().all(|l| !l.contains("secret-token")), "{:?}", e.tail);
        // board host 退出时 agent 还活着：主进程退出后用调用方给的结果
        let e = ended_of(&r, None, "host_exit", &[]);
        assert_eq!((e.how, e.why.as_deref()), ("host_exit", None));
        // 兜底停止过：报「超时」，原因写清楚
        let mut r = r;
        r.timeout = Some("运行超过 30 分钟".into());
        let e = ended_of(&r, None, "host_exit", &[]);
        assert_eq!((e.how, e.why.as_deref()), ("timeout", Some("运行超过 30 分钟")));
    }

    #[test]
    fn end_reports_retry_only_on_transient_errors() {
        assert!(permanent_end_error(ErrorCode::NotFound), "派发不存在：不用再报");
        assert!(permanent_end_error(ErrorCode::Usage));
        assert!(!permanent_end_error(ErrorCode::Unreachable), "连不上要重试");
        assert!(!permanent_end_error(ErrorCode::Internal), "服务端 5xx 要重试");
    }

    /// board next 之后同一个进程换到会话里新的那一张；别的会话、旧版服务端（没有 session）不跟随
    #[test]
    fn run_follows_its_session() {
        let d = |id: i64, session: i64| Dispatch {
            id,
            board: "default/main".into(),
            card: id,
            title: String::new(),
            host_id: 1,
            host: "mbp".into(),
            agent: "claude".into(),
            agent_label: "Claude Code".into(),
            name: Some("claude-mbp/opus-5.5".into()),
            env: None,
            env_name: None,
            source: "cron".into(),
            by: board_core::Actor::system(),
            state: "running".into(),
            result: None,
            created_at: 0,
            started_at: None,
            ended_at: None,
            exit_code: None,
            stop_by: None,
            beat_at: None,
            session,
            seq: 1,
            total: 1,
            model: None,
            effort: None,
        };
        let cur = d(1, 1);
        assert_eq!(successor(&cur, &[d(1, 1), d(5, 1), d(3, 3)]).map(|x| x.id), Some(5));
        assert!(successor(&cur, &[d(1, 1), d(3, 3)]).is_none(), "别的会话不跟随");
        assert!(successor(&d(1, 0), &[d(2, 0)]).is_none(), "旧版服务端不跟随");
    }

    #[test]
    fn cron_failed_minute_is_retried() {
        use chrono::TimeZone;
        let at = |d: u32, h: u32, m: u32| chrono::Local.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap();
        // 2026-11-09 是周一
        let cron = vec!["0 3 * * *".to_string()];
        // 不是定时点、暂停、正在进行一轮、没开定时都不发
        assert!(cron_due(&at(9, 3, 0), true, &cron, false, false));
        assert!(!cron_due(&at(9, 3, 0), true, &cron, true, false));
        assert!(!cron_due(&at(9, 3, 0), true, &cron, false, true));
        assert!(!cron_due(&at(9, 3, 1), true, &cron, false, false));
        assert!(!cron_due(&at(9, 3, 0), false, &cron, false, false));
        // 每 5 分钟、工作日 9 点；旧版服务端发来的 HH:MM 照样按每天这个时间
        let every5 = vec!["*/5 * * * *".to_string()];
        assert!(cron_due(&at(9, 14, 35), true, &every5, false, false));
        assert!(!cron_due(&at(9, 14, 36), true, &every5, false, false));
        let workday = vec!["0 9 * * 1-5".to_string()];
        assert!(cron_due(&at(9, 9, 0), true, &workday, false, false), "周一");
        assert!(!cron_due(&at(14, 9, 0), true, &workday, false, false), "周六");
        assert!(cron_due(&at(9, 3, 0), true, &["03:00".to_string()], false, false));
        // 两条同一分钟都到点：cron_tick 按分钟只处理一次（下面）
        assert!(cron_due(&at(9, 9, 0), true, &[workday[0].clone(), "0 9 * * *".into()], false, false));

        let mut last = String::new();
        let mut calls = 0;
        // 到点、请求失败：不记下这一分钟，同一分钟的下一个 tick 再试
        assert!(cron_tick("03:00", &mut last, true, || {
            calls += 1;
            false
        }));
        assert_eq!(last, "");
        assert!(cron_tick("03:00", &mut last, true, || {
            calls += 1;
            true
        }));
        assert_eq!((last.as_str(), calls), ("03:00", 2), "第二次成功才记下");
        // 成功之后同一分钟不再发
        assert!(!cron_tick("03:00", &mut last, true, || panic!("同一分钟不该再发")));
        // 不用领取的分钟记下来，不调 start
        assert!(!cron_tick("03:01", &mut last, false, || panic!("不到点不该发")));
        assert_eq!(last, "03:01");
    }

    #[cfg(windows)]
    #[test]
    fn finds_git_bash_beside_git() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("cmd")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("mingw64").join("bin")).unwrap();
        std::fs::write(root.join("cmd").join("git.exe"), "").unwrap();
        std::fs::write(root.join("mingw64").join("bin").join("git.exe"), "").unwrap();
        std::fs::write(root.join("bin").join("bash.exe"), "").unwrap();
        assert_eq!(envs::git_bash_beside(&root.join("cmd").join("git.exe")), Some(root.join("bin")));
        // PATH 里先找到的是 mingw64\bin 里的 git（#226）：也要找到 <Git>\bin\bash.exe
        assert_eq!(envs::git_bash_beside(&root.join("mingw64").join("bin").join("git.exe")), Some(root.join("bin")));
        assert_eq!(envs::git_bash_beside(&root.join("bin").join("git.exe")), Some(root.join("bin")));
        // 只有 usr\bin\bash.exe 的老布局
        let usr = tempfile::tempdir().unwrap();
        let u = usr.path().join("mingw64").join("bin");
        std::fs::create_dir_all(&u).unwrap();
        std::fs::create_dir_all(usr.path().join("usr").join("bin")).unwrap();
        std::fs::write(u.join("git.exe"), "").unwrap();
        std::fs::write(usr.path().join("usr").join("bin").join("bash.exe"), "").unwrap();
        assert_eq!(envs::git_bash_beside(&u.join("git.exe")), Some(usr.path().join("usr").join("bin")));
        // 哪里都没有 bash.exe
        let bare = tempfile::tempdir().unwrap();
        let b = bare.path().join("mingw64").join("bin");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("git.exe"), "").unwrap();
        assert_eq!(envs::git_bash_beside(&b.join("git.exe")), None);
    }
}
