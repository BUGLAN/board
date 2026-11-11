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
//!   机器和派发有变化（推送里的 `hosts` 变了）就马上重新取自己的任务。
//! - 有空位、没有暂停时，按创建先后开始排队中的派发：服务端生成名字 `<agent 标识>-<机器名>/<模型名>`、check、领取，
//!   host 在这块白板对应的目录里、在派发记录写的启动环境里启动 agent（`{prompt}` 换成 `/board-skills 开始执行 …`
//!   那一行，`{patch}` 换成这次派发的 profile 补丁路径；都作为一个参数传入，不拼进 shell 命令文本），环境变量带上 `BOARD_URL`、`BOARD_TOKEN`、`BOARD_AS`、`BOARD_HOST`、`BOARD`，以及 `BOARD_CONFIG`：
//!   指向这次派发自己的配置文件 `~/.board/host/run/<机器名>/<派发 ID>.toml`（地址和令牌，0600，派发结束时删除）。
//!   有的 agent 不把名字像密钥的环境变量传给它执行的命令（dsh 去掉名字里带 KEY、PASSWORD、SECRET、TOKEN 的），
//!   `board` 照样能从 `BOARD_CONFIG` 拿到令牌。还有 `BOARD_EXE`（board 自己的路径）：skill 的 `scripts/board`
//!   看到它就用它，免得 skill 里打包的旧版 board 不认识这些变量。
//! - 正在一轮领取时（定时到点、界面上「现在领一轮」），有空位就向服务端要下一张（定时领取）。
//! - agent 的标准输出和标准错误写到 `~/.board/host/logs/<派发 ID>.log`，每 5 秒上传新增的行。
//! - agent 退出后上报退出码和日志最后 20 行，服务端据此记下结果（已交审 / 做不下去 / 异常退出 / 已停止）。
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

use crate::backend::{Backend, Remote};
use crate::config::{self, AgentConf};
use crate::envs::{self, expand, kill_tree, Check, Env};
use crate::heartbeat::{Proc, Procs};
use crate::{Output, Paint};
use board_core::hosts::{check_agent_id, check_host_name, host_name_from, result_words};
use board_core::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
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
}

/// 顺序就是默认 agent 的顺序：本机找到的第一个是默认（机器设置里可以改）
const BUILTIN: [Builtin; 4] = [
    Builtin {
        id: "dsh",
        name: "DeepSeek Harness",
        cmd: &["dsh", "--profile", "headless", "--patch", "{patch}", "--permission-mode", "danger-full-access", "{prompt}"],
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
    },
    Builtin {
        id: "claude",
        name: "Claude Code",
        cmd: &["claude", "-p", "{prompt}", "--dangerously-skip-permissions"],
        model: "opus-5.5",
        also: &["~/.local/bin/claude", "~/.claude/local/claude"],
        skills: &["~/.claude/skills"],
        // Claude Code 不读 .agents/skills
        project: &[".claude/skills"],
        git_bash: false,
        win: &["pwsh", "powershell", "git-bash", "wsl:*", "cmd"],
    },
    Builtin {
        id: "pi",
        name: "pi",
        cmd: &["pi", "-p", "{prompt}"],
        model: "",
        also: &[],
        skills: &["~/.agents/skills", "~/.pi/agent/skills"],
        project: &[".agents/skills", ".pi/skills"],
        git_bash: true,
        // pi 不支持 cmd
        win: &["git-bash", "wsl:*", "pwsh", "powershell"],
    },
    Builtin {
        id: "codex",
        name: "Codex",
        cmd: &["codex", "exec", "--dangerously-bypass-approvals-and-sandbox", "{prompt}"],
        model: "",
        also: &[],
        skills: &["$CODEX_HOME/skills"],
        project: &[".agents/skills"],
        git_bash: false,
        win: &["pwsh", "powershell", "git-bash", "wsl:*", "cmd"],
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
/// 停止 agent：SIGTERM 之后等多久再 SIGKILL
const KILL_AFTER: Duration = Duration::from_secs(10);
/// 重启时结束上一次留下的 agent：SIGTERM 后等这么久，还在就 SIGKILL
const ORPHAN_KILL_AFTER: Duration = Duration::from_secs(5);
/// 异常退出时评论里带的日志行数
const TAIL: usize = 20;
/// 一次最多上传多少字节的新日志（多出来的只留最后这些：服务端本来也只保留最后 500 行）
const LOG_CHUNK: u64 = 256 * 1024;

/// 收到 Ctrl+C / SIGTERM，或者（桌面端）标准输入关闭
static QUIT: AtomicBool = AtomicBool::new(false);
/// （桌面端）标准输入里来了一行 `reload`：配置文件的 `[host]` 改了，或者装好了 board-skills，重新读、重新检查
static RELOAD: AtomicBool = AtomicBool::new(false);

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
}

impl Spec {
    /// 默认的启动环境：按顺序第一个找到命令的
    fn default_check(&self) -> Option<&Check> {
        self.checks.iter().find(|c| c.found)
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
}

/// 内置模板加上配置文件里的模板（同标识覆盖内置）；顺序：内置的在前，配置文件里新加的按文件顺序排在后面。
/// 每个 agent 在它支持、本机有的启动环境里各检查一次
fn agent_specs(conf: &[AgentConf], envs: &[Env]) -> Result<Vec<Spec>> {
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
        })
        .collect();
    for a in conf {
        check_agent_id(&a.id).map_err(BoardError::usage)?;
        match out.iter_mut().find(|x| x.id == a.id) {
            Some(x) => {
                // 换了命令：内置的备用位置不再适用
                if x.cmd.first() != a.cmd.first() {
                    x.also.clear();
                }
                x.cmd = a.cmd.clone();
                if let Some(n) = &a.name {
                    x.name = n.clone();
                }
                if let Some(m) = &a.model {
                    x.model = m.clone();
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
    /// SIGTERM 的时间
    term_at: Option<Instant>,
    killed: bool,
    spawned: Instant,
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
    /// Windows 上不做：进程号可能已经被复用，taskkill /T 会结束别的进程树
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

/// 还没上报成功的结束（连不上服务端时下一轮再报）
struct Ended {
    id: i64,
    code: Option<i64>,
    tail: Vec<String>,
    how: &'static str,
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

/// 日志最后 n 行
fn tail_lines(path: &Path, n: usize) -> Vec<String> {
    let (lines, _) = new_lines(path, 0, true);
    let lines: Vec<String> = lines.into_iter().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

fn clock() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
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
    pidfile: PathBuf,
    logs: PathBuf,
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
    /// 机器名、白板与目录跟着配置文件变（命令行没给 `--name` / `--dir`）
    follow_name: bool,
    follow_dirs: bool,
    /// 上一次读到的配置文件 `[host]`（机器名、白板与目录原文）：没变就不重新校验
    conf_seen: (Option<String>, Vec<(String, String)>),
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
    let dirs = parse_dirs(raw)?;
    let be = Backend::open_host()?;
    let envs = envs::discover();
    let mut agents = agent_specs(&conf.agents, &envs)?;
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
        pidfile,
        logs: hd.join("logs"),
        runs: vec![],
        ended: vec![],
        work: None,
        work_at: Instant::now(),
        hold: false,
        last_err: String::new(),
        follow_name,
        follow_dirs,
        conf_seen: (conf.name.clone(), conf.dirs.clone()),
    };
    on_signals();
    // 第一次上报：名字被占用、令牌不对、连不上，都直接报错退出。在这之前不碰 run 目录：同名的 host 可能还在跑
    let w = h.beat(false)?;
    h.set_work(w);
    h.resume = None;
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
    out
}

/// 配置文件里的机器名；没写时用本机主机名（转成合法的机器名）
fn conf_name(name: Option<&str>) -> String {
    match name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => host_name_from(&sysinfo::System::host_name().unwrap_or_default()),
    }
}

/// 校验白板与目录：白板写成 <项目>/<白板>、不重复，目录存在（相对路径按当前目录）
fn parse_dirs(raw: Vec<(String, String)>) -> Result<Vec<(String, PathBuf)>> {
    let mut dirs: Vec<(String, PathBuf)> = vec![];
    for (board, path) in raw {
        if !board_core::config::valid_board(&board) {
            return Err(BoardError::usage(format!("白板要写成 <项目>/<白板>：{board}")));
        }
        if dirs.iter().any(|(b, _)| *b == board) {
            return Err(BoardError::usage(format!("白板 {board} 写了两次")));
        }
        let mut dir = expand(&path);
        if dir.is_relative() {
            dir = std::env::current_dir().map(|c| c.join(&dir)).unwrap_or(dir);
        }
        if !dir.is_dir() {
            return Err(BoardError::usage(format!("目录不存在：{}（{board}）", dir.display())));
        }
        dirs.push((board, dir));
    }
    Ok(dirs)
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

    fn report(&self, done: bool) -> HostReport {
        HostReport {
            name: self.name.clone(),
            instance: self.instance.clone(),
            resume: self.resume.clone(),
            os: self.os.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            agents: self.agents.iter().map(Spec::report).collect(),
            envs: self.envs.iter().map(Env::report).collect(),
            dirs: self.dirs.iter().map(|(b, p)| HostDir { board: b.clone(), path: p.display().to_string() }).collect(),
            // 已经退出、但结束还没上报成功的派发也算在跑：否则服务端会先按「本机没有这个进程」结束它，丢掉退出码和日志
            running: self.runs.iter().map(|r| r.d.id).chain(self.ended.iter().map(|e| e.id)).collect(),
            done,
        }
    }

    fn rm(&self) -> &Remote {
        self.be.remote()
    }

    fn beat(&self, done: bool) -> Result<HostWork> {
        let rep = serde_json::to_value(self.report(done)).unwrap_or(Value::Null);
        self.rm().post(&format!("/api/hosts/{}/beat", self.name), rep)
    }

    fn set_work(&mut self, w: HostWork) {
        // 会话里 agent 执行了 board next：同一个进程换到这个会话正在跑的那一张，之后的日志、停止、结束都跟着它。
        // 换之前把已经写出的日志传给上一张
        let switches: Vec<(usize, Dispatch)> =
            self.runs.iter().enumerate().filter_map(|(i, r)| successor(&r.d, &w.running).map(|n| (i, n))).collect();
        for (i, n) in switches {
            let (lines, to) = new_lines(&self.runs[i].log, self.runs[i].sent, false);
            if !lines.is_empty() {
                let _ = self.rm().post::<Value>(&format!("/api/dispatches/{}/log", self.runs[i].d.id), json!({ "lines": lines }));
            }
            self.runs[i].sent = to;
            self.say(format!(
                "{} {}  {}  {}",
                self.p.green("▶"),
                self.p.cyan(&format!("{}#{}", n.board, n.card)),
                self.p.yellow(n.name.as_deref().unwrap_or_default()),
                self.p.dim(&format!("同一个会话接着做 · {}", n.title))
            ));
            self.runs[i].d = n;
        }
        self.work = Some(w);
        self.work_at = Instant::now();
        self.hold = false;
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
            // 桌面端保存了配置：换了名字、目录就马上上报，界面「机器」里立刻看到
            if RELOAD.swap(false, Ordering::SeqCst) && self.reload() {
                next_beat = now;
            }
            if now >= next_beat {
                next_beat = now + every;
                fetch = false;
                match self.beat(false) {
                    Ok(w) => {
                        self.set_work(w);
                        self.last_err.clear();
                    }
                    Err(e) => self.trouble(e, &mut next_beat)?,
                }
            } else if fetch {
                fetch = false;
                match self.rm().get::<HostWork>(&format!("/api/hosts/{}/work", self.name), &[]) {
                    Ok(w) => self.set_work(w),
                    Err(e) => self.trouble(e, &mut next_beat)?,
                }
            }
            if self.reap() {
                fetch = true;
            }
            self.enforce();
            if self.fill()? {
                fetch = true;
            }
            self.cron(&mut last_min);
            if now >= next_log {
                next_log = now + LOG_EVERY;
                self.upload_logs();
                self.report_ended();
            }
            std::thread::sleep(TICK);
        }
    }

    /// 桌面端通知（标准输入一行 `reload`）：「设置 → 本机」保存了配置，或者「一键安装」装好了 board-skills。
    /// 先重新看一遍 skill，再重新读配置。返回有没有变（变了马上上报）
    fn reload(&mut self) -> bool {
        let skills = self.recheck_skills();
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
        let raw = (conf.name.clone(), conf.dirs.clone());
        if raw == self.conf_seen {
            return false;
        }
        self.conf_seen = raw;
        let mut changed = false;
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
                Ok(dirs) if dirs != self.dirs => {
                    for a in &mut self.agents {
                        a.check_skills(&self.envs, &dirs);
                    }
                    let list: Vec<String> = dirs.iter().map(|(b, d)| format!("{b} → {}", d.display())).collect();
                    let list = if list.is_empty() { "（没有）".to_string() } else { list.join("，") };
                    self.say(format!("{} 白板与目录已更新：{list}", self.p.green("✓")));
                    self.dirs = dirs;
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
            // 主进程退出了，它起的子进程可能还在（同一进程组）：一起结束
            r.kill_rest();
            r.cleanup();
            let code = exit_code(status);
            self.flush_log(&r, true);
            let tail = tail_lines(&r.log, TAIL);
            self.ended.push(Ended { id: r.d.id, code, tail, how: "exit" });
            self.say(format!(
                "{} {}  {}  {}",
                self.p.dim("■"),
                self.p.cyan(&format!("{}#{}", r.d.board, r.d.card)),
                r.d.name.as_deref().unwrap_or_default(),
                self.p.dim(&format!("agent 退出，退出码 {}", code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into())))
            ));
            done = true;
        }
        if done {
            self.report_ended();
        }
        done
    }

    /// 上报结束；连不上时留到下一轮
    fn report_ended(&mut self) {
        let ended = std::mem::take(&mut self.ended);
        for e in ended {
            let body = json!({ "exitCode": e.code, "tail": e.tail, "how": e.how });
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
                    None if work_at > r.spawned => Some("服务端上这次派发已经结束".to_string()),
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
        if self.hold || w.host.paused {
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

    /// 还没在停的 agent 个数（占着并发位）
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
                self.ended.push(Ended { id: d.id, code: None, tail: vec![msg], how: "exit" });
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
        let out = std::fs::File::create(&log).map_err(|e| format!("无法创建日志 {}：{e}", log.display()))?;
        let err = out.try_clone().map_err(|e| format!("无法创建日志 {}：{e}", log.display()))?;
        let conf = self.run.join(format!("{}.toml", d.id));
        write_run_config(&conf, self.rm().url(), self.rm().token()).map_err(|e| e.message)?;
        let patch_arg = patch.display().to_string();
        let args: Vec<String> = spec.cmd.iter().skip(1).map(|a| a.replace("{prompt}", &prompt).replace("{patch}", &patch_arg)).collect();
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
        Ok(Run { d: d.clone(), child, env, pidfile, log, conf, procfile, sent: 0, term_at: None, killed: false, spawned: Instant::now() })
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

    fn upload_logs(&mut self) {
        let mut sent: Vec<(usize, u64)> = vec![];
        for (i, r) in self.runs.iter().enumerate() {
            let (lines, to) = new_lines(&r.log, r.sent, false);
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
        let (lines, _) = new_lines(&r.log, r.sent, last);
        if !lines.is_empty() {
            let _ = self.rm().post::<Value>(&format!("/api/dispatches/{}/log", r.d.id), json!({ "lines": lines }));
        }
    }

    /// 退出：按「人停止」结束本机所有 agent（先 SIGTERM，10 秒后 SIGKILL），上报结束，再让机器离线
    fn shutdown(&mut self, graceful: bool) {
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
        for r in &runs {
            r.cleanup();
        }
        let _ = std::fs::remove_dir_all(&self.run);
        for mut r in runs {
            let code = r.child.wait().ok().and_then(exit_code);
            self.flush_log(&r, true);
            self.ended.push(Ended { id: r.d.id, code, tail: tail_lines(&r.log, TAIL), how: "host_exit" });
        }
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
        assert_eq!(tail_lines(&f, 2), vec!["two", "thr"]);
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
        let nope = |id: &str| AgentConf { id: id.into(), name: None, model: None, cmd: vec![format!("/nope/{id}"), "{prompt}".into()] };
        let conf = vec![
            nope("dsh"),
            nope("claude"),
            nope("pi"),
            nope("codex"),
            AgentConf { id: "mycli".into(), name: Some("My CLI".into()), model: Some("gpt-5".into()), cmd: vec!["/nope/mycli".into(), "{prompt}".into()] },
        ];
        let specs = agent_specs(&conf, &[Env::direct()]).unwrap();
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
        let conf = vec![AgentConf { id: "dsh".into(), name: None, model: Some("deepseek-reasoner".into()), cmd: vec!["/nope/dsh".into(), "{prompt}".into()] }];
        let specs = agent_specs(&conf, &[Env::direct()]).unwrap();
        assert_eq!(specs[0].model, "deepseek-reasoner");
    }

    #[test]
    fn expands_home() {
        let home = expand("~");
        assert!(home.is_absolute() || home == PathBuf::from("~"));
        assert_eq!(expand("/abs/x"), PathBuf::from("/abs/x"));
    }

    #[test]
    fn skill_check_sees_project_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".agents").join("skills").join(SKILL)).unwrap();
        std::fs::write(repo.join(".agents").join("skills").join(SKILL).join("SKILL.md"), "x").unwrap();
        let spec = Spec {
            id: "pi".into(),
            name: "pi".into(),
            model: String::new(),
            cmd: vec!["pi".into()],
            skills: vec!["/board-test-no-such-skills".into()],
            project: vec![".agents/skills".into(), ".pi/skills".into()],
            git_bash: true,
            checks: vec![Check { env: "direct".into(), found: true, version: String::new(), prog: Some("pi".into()) }],
            no_skill: BTreeMap::new(),
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
        std::fs::write(root.join("cmd").join("git.exe"), "").unwrap();
        std::fs::write(root.join("bin").join("bash.exe"), "").unwrap();
        assert_eq!(envs::git_bash_beside(&root.join("cmd").join("git.exe")), Some(root.join("bin")));
        assert_eq!(envs::git_bash_beside(&root.join("bin").join("git.exe")), Some(root.join("bin")));
    }
}
