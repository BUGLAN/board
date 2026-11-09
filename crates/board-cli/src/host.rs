//! `board host`：在这台机器上常驻，接服务端派来的卡片，在本机启动 agent（任务下发需求 §三、§七）。
//!
//! ```text
//! board host [--name <机器名>] [--dir <项目>/<白板>=<路径>]...
//! ```
//!
//! - 参数也可以写在 `~/.board/config.toml` 的 `[host]`、`[agents.<标识>]` 里，命令行优先（见 config.rs）。
//! - 启动时找本机 agent：配置文件里的 `[agents.*]` 加上内置模板，逐个检查命令在不在 PATH 里、取版本号
//!   （`<命令> --version` 的第一行）；找不到的也上报，标为「没找到」。
//! - 每 10 秒向服务端上报一次（机器心跳），同时替正在跑的派发的名字发 agent 级心跳；订阅实时推送，
//!   机器和派发有变化（推送里的 `hosts` 变了）就马上重新取自己的任务。
//! - 有空位、没有暂停时，按创建先后开始排队中的派发：服务端生成名字 `<agent 标识>/<机器名>-<序号>`、check、领取，
//!   host 在这块白板对应的目录里启动 agent（`{prompt}` 换成 `/board-skills 开始执行 …` 那一行，作为一个参数传入，
//!   不经过 shell），环境变量带上 `BOARD_URL`、`BOARD_TOKEN`、`BOARD_AS`、`BOARD_HOST`。
//! - 正在一轮领取时（定时到点、界面上「现在领一轮」），有空位就向服务端要下一张（定时领取）。
//! - agent 的标准输出和标准错误写到 `~/.board/host/logs/<派发 ID>.log`，每 5 秒上传新增的行。
//! - agent 退出后上报退出码和日志最后 20 行，服务端据此记下结果（已交审 / 做不下去 / 异常退出 / 已停止）。
//! - 人停止：结束 agent 进程树（先 SIGTERM，10 秒后 SIGKILL；Windows 用 taskkill 结束进程树），再上报。
//! - `Ctrl+C`（或 SIGTERM）：先按「人停止」结束本机由它启动的全部 agent，再让机器离线，然后退出。
//!   桌面端在后台运行它时设置 `BOARD_HOST_STDIN=1`：标准输入关闭（桌面端退出）也按 Ctrl+C 处理。
//! - 机器名的 pid 文件 `~/.board/host/<机器名>.pid`（「进程号 启动时间 标识」）：board host 异常退出后在同一台机器上重启，
//!   用里面的标识接回还在线的名字；这时服务端上还在跑、本机却没有进程的派发按「异常退出」处理。

use crate::backend::{Backend, Remote};
use crate::config::{self, AgentConf};
use crate::heartbeat::{Proc, Procs};
use crate::Paint;
use board_core::hosts::{check_agent_id, check_host_name, host_name_from, result_words};
use board_core::*;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// 内置 agent 模板：(标识, 名称, 命令模板)。配置文件里同标识的模板覆盖它
const BUILTIN: [(&str, &str, &[&str]); 1] =
    [("claude", "Claude Code", &["claude", "-p", "{prompt}", "--dangerously-skip-permissions"])];

/// 主循环每一轮的间隔
const TICK: Duration = Duration::from_millis(200);
/// 多久上传一次新增的日志
const LOG_EVERY: Duration = Duration::from_secs(5);
/// 停止 agent：SIGTERM 之后等多久再 SIGKILL
const KILL_AFTER: Duration = Duration::from_secs(10);
/// 异常退出时评论里带的日志行数
const TAIL: usize = 20;
/// 一次最多上传多少字节的新日志（多出来的只留最后这些：服务端本来也只保留最后 500 行）
const LOG_CHUNK: u64 = 256 * 1024;

/// 收到 Ctrl+C / SIGTERM，或者（桌面端）标准输入关闭
static QUIT: AtomicBool = AtomicBool::new(false);

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
    cmd: Vec<String>,
    /// 找到的命令；没找到时为空
    exe: Option<PathBuf>,
    version: String,
}

impl Spec {
    fn report(&self) -> HostAgent {
        HostAgent { id: self.id.clone(), name: self.name.clone(), version: self.version.clone(), found: self.exe.is_some() }
    }
}

/// 内置模板加上配置文件里的模板（同标识覆盖内置）；顺序：内置的在前，配置文件里新加的按文件顺序排在后面
fn agent_specs(conf: &[AgentConf]) -> Result<Vec<Spec>> {
    let mut out: Vec<(String, String, Vec<String>)> =
        BUILTIN.iter().map(|(id, name, cmd)| (id.to_string(), name.to_string(), cmd.iter().map(|s| s.to_string()).collect())).collect();
    for a in conf {
        check_agent_id(&a.id).map_err(BoardError::usage)?;
        let name = a.name.clone().unwrap_or_else(|| a.id.clone());
        match out.iter_mut().find(|x| x.0 == a.id) {
            Some(x) => {
                x.2 = a.cmd.clone();
                if let Some(n) = &a.name {
                    x.1 = n.clone();
                }
            }
            None => out.push((a.id.clone(), name, a.cmd.clone())),
        }
    }
    // 各个 agent 的 --version 一起跑：一个慢的不拖住启动
    Ok(std::thread::scope(|s| {
        let jobs: Vec<_> = out
            .into_iter()
            .map(|(id, name, cmd)| {
                s.spawn(move || {
                    let exe = cmd.first().and_then(|c| which(c));
                    let version = exe.as_deref().map(version_of).unwrap_or_default();
                    Spec { id, name, cmd, exe, version }
                })
            })
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok()).collect()
    }))
}

/// 在 PATH 里找命令（Windows 上按 PATHEXT 补扩展名）；写了路径的直接看文件在不在
fn which(cmd: &str) -> Option<PathBuf> {
    let p = expand(cmd);
    if p.components().count() > 1 {
        return is_exe(&p).then_some(p);
    }
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_lowercase())
            .collect()
    } else {
        vec![]
    };
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        if cfg!(windows) {
            for e in &exts {
                let f = dir.join(format!("{cmd}{e}"));
                if f.is_file() {
                    return Some(f);
                }
            }
        } else {
            let f = dir.join(cmd);
            if is_exe(&f) {
                return Some(f);
            }
        }
    }
    None
}

fn is_exe(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// `<命令> --version` 的第一行（5 秒没结果算取不到，结束它）
fn version_of(exe: &Path) -> String {
    let mut c = Command::new(exe);
    c.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    no_window(&mut c);
    let Ok(mut child) = c.spawn() else { return String::new() };
    let begun = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if begun.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
        }
    }
    let Ok(o) = child.wait_with_output() else { return String::new() };
    let text = if o.stdout.iter().any(|b| !b.is_ascii_whitespace()) { o.stdout } else { o.stderr };
    String::from_utf8_lossy(&text).lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().chars().take(60).collect()
}

/// `~` 开头的路径换成用户目录
fn expand(p: &str) -> PathBuf {
    let home = || {
        let keys: [&str; 2] = if cfg!(windows) { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
        keys.iter().find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
    };
    match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') => match home() {
            Some(h) => h.join(rest.trim_start_matches(['/', '\\'])),
            None => PathBuf::from(p),
        },
        _ => PathBuf::from(p),
    }
}

/// board host 自己的目录：`~/.board/host`
fn host_dir() -> Result<PathBuf> {
    let d = config::path()?.parent().map(|p| p.join("host")).ok_or_else(|| BoardError::usage("找不到 ~/.board"))?;
    std::fs::create_dir_all(d.join("logs")).map_err(|e| BoardError::usage(format!("无法创建 {}：{e}", d.display())))?;
    Ok(d)
}

/* ------------------------------ 进程 ------------------------------ */

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn no_window(_c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        _c.creation_flags(CREATE_NO_WINDOW);
    }
}

/// agent 进程自成一组：终端里的 Ctrl+C 不直接打到它身上（由 host 按「人停止」处理），停止时整组结束
fn own_group(c: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        c.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
}

/// 结束进程树：先 SIGTERM（hard 时 SIGKILL）整个进程组；Windows 直接结束进程树
fn signal_tree(pid: u32, hard: bool) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), if hard { libc::SIGKILL } else { libc::SIGTERM });
    }
    #[cfg(windows)]
    {
        let _ = hard;
        let mut c = Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        no_window(&mut c);
        let _ = c.status();
    }
}

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
    // 桌面端在后台运行时：标准输入关闭（桌面端退出、崩溃）就退出
    if std::env::var("BOARD_HOST_STDIN").is_ok_and(|v| v.trim() == "1") {
        std::thread::spawn(|| {
            let mut buf = [0u8; 256];
            let mut stdin = std::io::stdin();
            while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
            QUIT.store(true, Ordering::SeqCst);
        });
    }
}

/* ------------------------------ 一次在跑的派发 ------------------------------ */

struct Run {
    d: Dispatch,
    child: Child,
    log: PathBuf,
    /// 日志已经上传到的位置（字节）
    sent: u64,
    /// SIGTERM 的时间
    term_at: Option<Instant>,
    killed: bool,
    spawned: Instant,
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

/* ------------------------------ host ------------------------------ */

struct Host<'a> {
    p: &'a Paint,
    be: Backend,
    name: String,
    instance: String,
    /// 第一次上报时带上：上一次异常退出的 board host 的标识
    resume: Option<String>,
    os: String,
    agents: Vec<Spec>,
    dirs: Vec<(String, PathBuf)>,
    pidfile: PathBuf,
    logs: PathBuf,
    runs: Vec<Run>,
    ended: Vec<Ended>,
    work: Option<HostWork>,
    /// 最近一次取到任务的时间
    work_at: Instant,
    /// 开始派发、要下一张失败（或这一轮已经领完）之后，等重新取到任务再试
    hold: bool,
    /// 上一次打印的连接错误（同样的错误不重复打印）
    last_err: String,
}

/// `board host`：一直运行，直到 Ctrl+C
pub fn run(opts: Opts, p: &Paint) -> Result<()> {
    let file = config::File::load()?;
    let conf = file.host()?;
    // 机器名
    let name = match opts.name.as_deref().map(str::trim).filter(|s| !s.is_empty()).or(conf.name.as_deref()) {
        Some(n) => n.to_string(),
        None => host_name_from(&sysinfo::System::host_name().unwrap_or_default()),
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
    let mut dirs: Vec<(String, PathBuf)> = vec![];
    for (board, path) in raw {
        if !board.contains('/') || board.split('/').any(|x| !valid_key(x)) || board.matches('/').count() != 1 {
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
    let be = Backend::open()?;
    let agents = agent_specs(&conf.agents)?;
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
    let mut h = Host {
        p,
        be,
        name,
        instance,
        resume,
        os,
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
    };
    on_signals();
    // 第一次上报：名字被占用、令牌不对、连不上，都直接报错退出
    let w = h.beat(false)?;
    h.set_work(w);
    h.resume = None;
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

impl Host<'_> {
    fn say(&self, s: String) {
        println!("{} {s}", self.p.dim(&clock()));
    }

    fn report(&self, done: bool) -> HostReport {
        HostReport {
            name: self.name.clone(),
            instance: self.instance.clone(),
            resume: self.resume.clone(),
            os: self.os.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            agents: self.agents.iter().map(Spec::report).collect(),
            dirs: self.dirs.iter().map(|(b, p)| HostDir { board: b.clone(), path: p.display().to_string() }).collect(),
            running: self.runs.iter().map(|r| r.d.id).collect(),
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
        self.work = Some(w);
        self.work_at = Instant::now();
        self.hold = false;
    }

    /// 启动后打印一次：机器、agent、白板与目录
    fn hello(&self) {
        let p = self.p;
        let mut s = format!("{} {} 已上线：{}\n", p.green("✓"), p.bold(&self.name), p.cyan(self.rm().url()));
        s += &format!("  {}{} · board {}\n", p.dim("系统    "), self.os, env!("CARGO_PKG_VERSION"));
        let def = self.work.as_ref().and_then(|w| w.host.agent.clone());
        for (i, a) in self.agents.iter().enumerate() {
            let head = if i == 0 { p.dim("agent   ") } else { "        ".into() };
            let state = match &a.exe {
                Some(_) => format!("{}{}", if a.version.is_empty() { "找到了".to_string() } else { a.version.clone() }, if def.as_deref() == Some(&a.id) { p.yellow("（默认）") } else { String::new() }),
                None => p.red("没找到"),
            };
            s += &format!("  {head}{} {}  {state}\n", p.cyan(&a.id), a.name);
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
        print!("{s}");
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
                Err(err) if err.code == ErrorCode::Unreachable => self.ended.push(e),
                Err(_) => {}
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
                    signal_tree(r.child.id(), false);
                    r.term_at = Some(Instant::now());
                    say.push(format!("停止 {}#{}：{why}", r.d.board, r.d.card));
                }
            } else if !r.killed && r.term_at.is_some_and(|t| t.elapsed() >= KILL_AFTER) {
                signal_tree(r.child.id(), true);
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
                self.say(format!(
                    "{} {}  {}  {}",
                    self.p.green("▶"),
                    self.p.cyan(&format!("{}#{}", d.board, d.card)),
                    self.p.yellow(d.name.as_deref().unwrap_or_default()),
                    d.title
                ));
                self.runs.push(r);
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

    fn spawn(&self, d: &Dispatch) -> std::result::Result<Run, String> {
        let spec = self.agents.iter().find(|a| a.id == d.agent).ok_or_else(|| format!("本机没有 agent「{}」", d.agent))?;
        let exe = spec.exe.clone().ok_or_else(|| format!("本机没找到 agent「{}」的命令 {}", d.agent, spec.cmd.first().cloned().unwrap_or_default()))?;
        let dir = self.dirs.iter().find(|(b, _)| *b == d.board).map(|(_, p)| p.clone()).ok_or_else(|| format!("本机没有白板 {} 的目录", d.board))?;
        if !dir.is_dir() {
            return Err(format!("目录不存在：{}", dir.display()));
        }
        let name = d.name.clone().unwrap_or_default();
        let (project, board) = d.board.split_once('/').unwrap_or((&d.board, ""));
        let prompt = format!("/board-skills 开始执行 {project}项目之中的{board}白板中的#{}任务", d.card);
        let log = self.logs.join(format!("{}.log", d.id));
        let out = std::fs::File::create(&log).map_err(|e| format!("无法创建日志 {}：{e}", log.display()))?;
        let err = out.try_clone().map_err(|e| format!("无法创建日志 {}：{e}", log.display()))?;
        let mut c = Command::new(&exe);
        c.args(spec.cmd.iter().skip(1).map(|a| a.replace("{prompt}", &prompt)))
            .current_dir(&dir)
            .env("BOARD_URL", self.rm().url())
            .env("BOARD_TOKEN", self.rm().token())
            .env("BOARD_AS", &name)
            .env("BOARD_HOST", &self.name)
            .env("BOARD", &d.board)
            // 在 Claude Code 的会话里运行 board host 时，别让 agent 以为自己是嵌套会话
            .env_remove("CLAUDECODE")
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        own_group(&mut c);
        let child = c.spawn().map_err(|e| format!("启动 {} 失败：{e}", exe.display()))?;
        Ok(Run { d: d.clone(), child, log, sent: 0, term_at: None, killed: false, spawned: Instant::now() })
    }

    /// 定时领取：到了设置的时间点（机器本地时区），机器在线、没有暂停、没有正在进行的一轮，就开始一轮
    fn cron(&mut self, last: &mut String) {
        let m = chrono::Local::now().format("%H:%M").to_string();
        if m == *last {
            return;
        }
        *last = m.clone();
        let Some(w) = &self.work else { return };
        let h = &w.host;
        if !h.cron_on || !h.cron.contains(&m) || h.paused || h.round {
            return;
        }
        match self.rm().post::<HostInfo>(&format!("/api/hosts/{}/round", self.name), json!({})) {
            Ok(_) => {
                self.say(format!("{} {m} 到点，开始一轮领取", self.p.green("⏰")));
                self.hold = false;
                if let Some(w) = self.work.as_mut() {
                    w.host.round = true;
                }
            }
            Err(e) => self.say(format!("{} {m} 到点，没能开始领取：{}", self.p.yellow("!"), e.message)),
        }
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
                signal_tree(r.child.id(), false);
                r.term_at = Some(Instant::now());
            }
        }
        let begun = Instant::now();
        while self.runs.iter_mut().any(|r| matches!(r.child.try_wait(), Ok(None))) && begun.elapsed() < KILL_AFTER {
            std::thread::sleep(Duration::from_millis(100));
        }
        for r in self.runs.iter_mut() {
            if matches!(r.child.try_wait(), Ok(None)) {
                signal_tree(r.child.id(), true);
                let _ = r.child.wait();
            }
        }
        let runs = std::mem::take(&mut self.runs);
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
    fn expands_home() {
        let home = expand("~");
        assert!(home.is_absolute() || home == PathBuf::from("~"));
        assert_eq!(expand("/abs/x"), PathBuf::from("/abs/x"));
    }
}
