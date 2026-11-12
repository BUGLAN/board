//! 让机器重启后自动接任务（PRD §9「让机器重启后自动接任务」）：
//!
//! - `board host --install [--name …] [--dir …]`：按当前参数装成开机自启的服务，立即启动，打印怎么看状态、怎么卸载。
//!   macOS 写 launchd 用户代理（`~/Library/LaunchAgents/dev.board.host.plist`），Linux 写 systemd 用户服务
//!   （`~/.config/systemd/user/board-host.service`，提示 `loginctl enable-linger` 才能在没登录时运行），
//!   Windows 建一个登录时运行的计划任务（「Board host」）。服务文件的内容在 `board_core::service`。
//! - 服务运行的是 `board host --service …`：守护进程。它带着同样的参数启动真正的 board host（环境变量
//!   `BOARD_HOST_SUPERVISED=1`），把输出写进 `~/.board/host/host.log`（超过 5 MB 轮转成 host.log.1、host.log.2）；
//!   board host 异常退出（退出码非 0、被 kill -9）10 秒后重启，退出码 75（远程「重启」「升级」）马上重启，
//!   正常退出（0）时自己也退出。守护进程收到 SIGTERM（停止服务）时先让 board host 收尾再退出。
//! - `board host --uninstall`：停掉并删掉服务。
//! - 服务只读配置文件 `~/.board/config.toml` 里的服务端地址和令牌，不带 shell 里的 `BOARD_URL` / `BOARD_TOKEN`：
//!   装之前检查配置文件里有。

use crate::config;
use crate::{Output, Paint};
use board_core::service::{self as svc, LAUNCHD_HOST, RESTART_SECS, SYSTEMD_HOST, WINDOWS_TASK};
use board_core::*;
use serde_json::json;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// board host 由守护进程启动时设这个环境变量：远程「重启」时退出码 75 交给守护进程重启
pub const SUPERVISED: &str = "BOARD_HOST_SUPERVISED";
/// host.log 超过多大轮转
const LOG_MAX: u64 = 5 * 1024 * 1024;
/// 轮转保留几份旧的（host.log.1、host.log.2）
const LOG_KEEP: usize = 2;

static QUIT: AtomicBool = AtomicBool::new(false);

/// 由守护进程启动
pub fn supervised() -> bool {
    std::env::var(SUPERVISED).is_ok_and(|v| v.trim() == "1")
}

/// 重启前等多久：`BOARD_HOST_RESTART_MS`（只给测试用），默认 10 秒
fn restart_after() -> Duration {
    std::env::var("BOARD_HOST_RESTART_MS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(RESTART_SECS))
}

fn host_dir() -> Result<PathBuf> {
    let d = config::board_dir()?.join("host");
    std::fs::create_dir_all(&d).map_err(|e| BoardError::usage(format!("无法创建 {}：{e}", d.display())))?;
    Ok(d)
}

/* ------------------------------ 日志 ------------------------------ */

/// 按大小轮转的日志：每行前面不再加时间（board host 自己的输出带时间），守护进程自己的话带日期时间
struct Log {
    path: PathBuf,
    file: Option<std::fs::File>,
    size: u64,
}

impl Log {
    fn open(path: PathBuf) -> Log {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Log { path, file, size }
    }

    fn rotate(&mut self) {
        self.file = None;
        let name = |i: usize| PathBuf::from(format!("{}.{i}", self.path.display()));
        let _ = std::fs::remove_file(name(LOG_KEEP));
        for i in (1..LOG_KEEP).rev() {
            let _ = std::fs::rename(name(i), name(i + 1));
        }
        let _ = std::fs::rename(&self.path, name(1));
        *self = Log::open(self.path.clone());
    }

    fn line(&mut self, s: &str) {
        if self.size >= LOG_MAX {
            self.rotate();
        }
        if let Some(f) = self.file.as_mut() {
            if writeln!(f, "{s}").is_ok() {
                self.size += s.len() as u64 + 1;
            }
        }
    }

    fn say(&mut self, s: &str) {
        let t = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        self.line(&format!("── {t} {s}"));
    }
}

/* ------------------------------ 守护进程 ------------------------------ */

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
}

/// 让 board host 收尾退出：Unix 发 SIGTERM（它按 Ctrl+C 处理：先结束 agent、再离线）；Windows 直接结束进程树
fn ask_stop(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    crate::envs::kill_tree(pid, true);
}

/// `board host --service [--name …] [--dir …]`：守护进程（见模块说明）。`host_args` 原样传给 board host
pub fn supervise(host_args: Vec<String>) -> Result<()> {
    on_signals();
    let dir = host_dir()?;
    let pidfile = dir.join("service.pid");
    let _ = std::fs::write(&pidfile, format!("{}\n", std::process::id()));
    let log = Arc::new(Mutex::new(Log::open(dir.join("host.log"))));
    let say = |s: &str| {
        if let Ok(mut l) = log.lock() {
            l.say(s);
        }
    };
    let exe = std::env::current_exe().map_err(|e| BoardError::usage(format!("找不到 board 自己：{e}")))?;
    say(&format!("守护进程启动（进程 {}）：board host {}", std::process::id(), host_args.join(" ")));
    let mut n = 0u64;
    let out = loop {
        if QUIT.load(Ordering::SeqCst) {
            break Ok(());
        }
        n += 1;
        let child = Command::new(&exe)
            .arg("host")
            .args(&host_args)
            .env(SUPERVISED, "1")
            .env("NO_COLOR", "1")
            .env_remove("BOARD_HOST_STDIN")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                say(&format!("启动 board host 失败：{e}；{} 秒后重试", restart_after().as_secs()));
                wait_or_quit(restart_after());
                continue;
            }
        };
        say(&format!("启动 board host（第 {n} 次，进程 {}）", child.id()));
        let mut readers = vec![];
        for r in [child.stdout.take().map(|o| Box::new(o) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|e| Box::new(e) as Box<dyn std::io::Read + Send>)]
            .into_iter()
            .flatten()
        {
            let log = log.clone();
            readers.push(std::thread::spawn(move || {
                for line in std::io::BufReader::new(r).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(mut l) = log.lock() {
                        l.line(&line);
                    }
                }
            }));
        }
        let mut asked: Option<Instant> = None;
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) => {}
                Err(_) => break None,
            }
            if QUIT.load(Ordering::SeqCst) && asked.is_none() {
                say("收到停止信号：让 board host 收尾退出");
                ask_stop(child.id());
                asked = Some(Instant::now());
            }
            // board host 收尾最多等 30 秒（它结束 agent 要 10 秒）
            if asked.is_some_and(|t| t.elapsed() > Duration::from_secs(30)) {
                crate::envs::kill_tree(child.id(), true);
                let _ = child.kill();
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        for r in readers {
            let _ = r.join();
        }
        let code = status.and_then(|s| s.code());
        if QUIT.load(Ordering::SeqCst) {
            say(&format!("board host 已退出（{}），守护进程退出", describe(status)));
            break Ok(());
        }
        match code {
            Some(0) => {
                say("board host 正常退出，守护进程也退出");
                break Ok(());
            }
            Some(75) => say("board host 要重启（远程重启或升级）：马上重启"),
            _ => {
                say(&format!("board host 异常退出（{}）：{} 秒后重启", describe(status), restart_after().as_secs_f64()));
                wait_or_quit(restart_after());
            }
        }
    };
    if std::fs::read_to_string(&pidfile).is_ok_and(|s| s.trim() == std::process::id().to_string()) {
        let _ = std::fs::remove_file(&pidfile);
    }
    out
}

fn describe(s: Option<std::process::ExitStatus>) -> String {
    let Some(s) = s else { return "状态未知".into() };
    if let Some(c) = s.code() {
        return format!("退出码 {c}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = s.signal() {
            return format!("被信号 {sig} 结束");
        }
    }
    "被结束".into()
}

fn wait_or_quit(d: Duration) {
    let t = Instant::now();
    while t.elapsed() < d && !QUIT.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/* ------------------------------ 安装、卸载 ------------------------------ */

/// 服务里运行的参数：`<board> host --service [--name …] [--dir …]`。`--dir` 里的相对路径按当前目录换成绝对路径
/// （服务的工作目录不是这里）；`~` 原样保留，board host 按家目录展开
pub fn service_args(exe: &Path, name: Option<&str>, dirs: &[String], cwd: &Path) -> Result<Vec<String>> {
    let mut out = vec![exe.display().to_string(), "host".into(), "--service".into()];
    if let Some(n) = name.map(str::trim).filter(|s| !s.is_empty()) {
        board_core::hosts::check_host_name(n).map_err(BoardError::usage)?;
        out.push("--name".into());
        out.push(n.into());
    }
    for d in dirs {
        let (board, path) = d.split_once('=').ok_or_else(|| BoardError::usage(format!("--dir 要写成 <项目>/<白板>=<路径>：{d}")))?;
        let (board, path) = (board.trim(), path.trim());
        if !board_core::valid_board(board) {
            return Err(BoardError::usage(format!("白板要写成 <项目>/<白板>：{board}")));
        }
        let p = Path::new(path);
        let path = if path.starts_with('~') || p.is_absolute() { path.to_string() } else { cwd.join(p).display().to_string() };
        out.push("--dir".into());
        out.push(format!("{board}={path}"));
    }
    Ok(out)
}

/// 服务里的环境变量：装的时候 shell 里的 PATH（launchd、systemd 给的 PATH 很短，git、agent 找不到），
/// 和 HOME（配置文件 ~/.board/config.toml 跟着它找；和装的时候是同一个）
fn env_for_service() -> Vec<(String, String)> {
    ["PATH", "HOME"].iter().filter_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()).map(|v| (k.to_string(), v))).collect()
}

fn run(cmd: &str, args: &[&str]) -> std::result::Result<String, String> {
    let mut c = Command::new(cmd);
    c.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    match c.output() {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => {
            let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let e = if e.is_empty() { String::from_utf8_lossy(&o.stdout).trim().to_string() } else { e };
            Err(format!("{cmd} {}：{}", args.join(" "), if e.is_empty() { format!("退出码 {:?}", o.status.code()) } else { e }))
        }
        Err(e) => Err(format!("没法运行 {cmd}：{e}")),
    }
}

fn home() -> Result<PathBuf> {
    config::board_dir()?.parent().map(Path::to_path_buf).ok_or_else(|| BoardError::usage("找不到家目录"))
}

/// 服务文件放在哪里、怎么看状态
struct Plan {
    file: PathBuf,
    status: String,
}

fn plan() -> Result<Plan> {
    let home = home()?;
    if cfg!(target_os = "macos") {
        Ok(Plan { file: home.join("Library/LaunchAgents").join(format!("{LAUNCHD_HOST}.plist")), status: format!("launchctl print gui/$(id -u)/{LAUNCHD_HOST}") })
    } else if cfg!(windows) {
        Ok(Plan { file: config::board_dir()?.join("host").join("board-host-task.xml"), status: format!("schtasks /Query /TN \"{WINDOWS_TASK}\" /V /FO LIST") })
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".config"));
        Ok(Plan { file: base.join("systemd/user").join(SYSTEMD_HOST), status: format!("systemctl --user status {SYSTEMD_HOST}") })
    }
}

#[cfg(unix)]
fn uid() -> u32 {
    unsafe { libc::getuid() }
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| BoardError::usage(format!("无法创建 {}：{e}", d.display())))?;
    }
    std::fs::write(path, bytes).map_err(|e| BoardError::usage(format!("写不了 {}：{e}", path.display())))
}

/// `board host --install`
pub fn install(name: Option<&str>, dirs: &[String], json: bool, p: &Paint) -> Result<Output> {
    let exe = std::env::current_exe().map_err(|e| BoardError::usage(format!("找不到 board 自己：{e}")))?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    if crate::update::bundled_with_desktop(&exe) {
        return Err(BoardError::usage(
            "这是桌面端自带的 board：在桌面端「设置 → 本机」打开「让这台电脑接任务」和「开机时自动启动」，不用装成服务",
        ));
    }
    // 服务只读配置文件：环境变量里的地址、令牌带不过去
    let file = config::File::load()?;
    if file.get(config::URL)?.is_none() || file.get(config::TOKEN)?.is_none() {
        return Err(BoardError::new(
            ErrorCode::NotLoggedIn,
            format!("{} 里还没有服务端地址和令牌（服务不带 shell 里的 BOARD_URL、BOARD_TOKEN）：先 board login --url <服务端地址> --token <令牌>", config::shown()),
        ));
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let args = service_args(&exe, name, dirs, &cwd)?;
    let plan = plan()?;
    let dir = host_dir()?;
    let log = dir.join("host.log");
    let mut notes: Vec<String> = vec![];
    if cfg!(target_os = "macos") {
        #[cfg(unix)]
        {
            let out = dir.join("service.out");
            let text = svc::launchd_plist(LAUNCHD_HOST, &args, &env_for_service(), true, Some(&out.display().to_string()));
            write(&plan.file, text.as_bytes())?;
            let target = format!("gui/{}", uid());
            let _ = run("launchctl", &["bootout", &format!("{target}/{LAUNCHD_HOST}")]);
            // bootout 是异步的：刚卸下马上 bootstrap 可能报 5（I/O error），等一下再试
            let mut last = Ok(String::new());
            for _ in 0..10 {
                last = run("launchctl", &["bootstrap", &target, &plan.file.display().to_string()]);
                if last.is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            last.map_err(|e| BoardError::usage(format!("launchd 没能加载服务：{e}")))?;
        }
    } else if cfg!(windows) {
        let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
            (Ok(d), Ok(u)) if !d.is_empty() => format!("{d}\\{u}"),
            (_, Ok(u)) => u,
            _ => return Err(BoardError::usage("取不到当前用户名（USERNAME）")),
        };
        write(&plan.file, &svc::utf16le(&svc::windows_task_xml(&user, &args)))?;
        run("schtasks", &["/Create", "/TN", WINDOWS_TASK, "/XML", &plan.file.display().to_string(), "/F"]).map_err(|e| BoardError::usage(format!("建计划任务失败：{e}")))?;
        let _ = run("schtasks", &["/End", "/TN", WINDOWS_TASK]);
        run("schtasks", &["/Run", "/TN", WINDOWS_TASK]).map_err(|e| BoardError::usage(format!("启动计划任务失败：{e}")))?;
    } else {
        write(&plan.file, svc::systemd_unit(&args, &env_for_service()).as_bytes())?;
        let sys = |a: &[&str]| run("systemctl", &[&["--user"], a].concat()).map_err(|e| BoardError::usage(format!("{e}（这台机器要有 systemd 用户服务）")));
        sys(&["daemon-reload"])?;
        sys(&["enable", SYSTEMD_HOST])?;
        sys(&["restart", SYSTEMD_HOST])?;
        let user = std::env::var("USER").unwrap_or_default();
        let linger = run("loginctl", &["show-user", &user, "--property=Linger"]).unwrap_or_default();
        if !linger.contains("Linger=yes") {
            notes.push(format!("要在没有人登录时（开机后、退出登录后）也运行，执行一次：loginctl enable-linger {user}（可能要 sudo）"));
        }
    }
    let uninstall = "board host --uninstall".to_string();
    if json {
        return Ok(Output::Json(json!({
            "ok": true,
            "file": plan.file.display().to_string(),
            "args": args,
            "log": log.display().to_string(),
            "status": plan.status,
            "uninstall": uninstall,
            "notes": notes,
        })));
    }
    let mut s = format!("{} 已装成开机自启的服务，正在启动：{}\n", p.green("✓"), plan.file.display());
    s += &format!("  {}{}\n", p.dim("运行    "), args.join(" "));
    s += &format!("  {}{}（board host 异常退出 {RESTART_SECS} 秒后自动重启）\n", p.dim("日志    "), log.display());
    s += &format!("  {}{}\n", p.dim("状态    "), plan.status);
    s += &format!("  {}{}\n", p.dim("卸载    "), uninstall);
    for n in &notes {
        s += &format!("{} {n}\n", p.yellow("!"));
    }
    Ok(Output::Text(s))
}

/// `board host --uninstall`：停掉并删掉服务；守护进程还在就结束它（连同它启动的 board host）
pub fn uninstall(json: bool, p: &Paint) -> Result<Output> {
    let plan = plan()?;
    let existed = plan.file.exists();
    if cfg!(target_os = "macos") {
        #[cfg(unix)]
        {
            let _ = run("launchctl", &["bootout", &format!("gui/{}/{LAUNCHD_HOST}", uid())]);
        }
    } else if cfg!(windows) {
        let _ = run("schtasks", &["/End", "/TN", WINDOWS_TASK]);
        let _ = run("schtasks", &["/Delete", "/TN", WINDOWS_TASK, "/F"]);
    } else {
        let _ = run("systemctl", &["--user", "disable", "--now", SYSTEMD_HOST]);
    }
    let _ = std::fs::remove_file(&plan.file);
    if !cfg!(target_os = "macos") && !cfg!(windows) {
        let _ = run("systemctl", &["--user", "daemon-reload"]);
    }
    // 计划任务结束时不一定带走子进程：按 pid 文件结束守护进程
    if let Ok(dir) = host_dir() {
        let pidfile = dir.join("service.pid");
        if let Some(pid) = std::fs::read_to_string(&pidfile).ok().and_then(|s| s.trim().parse::<u32>().ok()) {
            if pid != std::process::id() {
                ask_stop(pid);
            }
        }
    }
    if json {
        return Ok(Output::Json(json!({ "ok": true, "removed": existed, "file": plan.file.display().to_string() })));
    }
    Ok(Output::Text(if existed {
        format!("{} 已卸载：{} 删掉了，重启后不再自动运行 board host\n", p.green("✓"), plan.file.display())
    } else {
        format!("{} 没有装过服务（{} 不存在）；已确认停掉\n", p.green("✓"), plan.file.display())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_args_make_dirs_absolute_and_check_names() {
        let exe = Path::new("/usr/local/bin/board");
        let cwd = Path::new("/home/me/work");
        let a = service_args(exe, Some("devbox"), &["a/b=repo".into(), "c/d=~/x".into(), "e/f=/srv/y".into()], cwd).unwrap();
        // 相对路径按当前目录拼：分隔符跟着系统走（Windows 上是 \）
        let abs = format!("a/b={}", cwd.join("repo").display());
        assert_eq!(a, ["/usr/local/bin/board", "host", "--service", "--name", "devbox", "--dir", &abs, "--dir", "c/d=~/x", "--dir", "e/f=/srv/y"]);
        // 不给机器名、目录时跟着配置文件走
        assert_eq!(service_args(exe, None, &[], cwd).unwrap(), ["/usr/local/bin/board", "host", "--service"]);
        assert!(service_args(exe, Some("Dev Box"), &[], cwd).is_err());
        assert!(service_args(exe, None, &["nope".into()], cwd).is_err());
        assert!(service_args(exe, None, &["bad=/x".into()], cwd).is_err());
    }

    #[test]
    fn log_rotates_by_size() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("host.log");
        let mut l = Log::open(path.clone());
        l.size = LOG_MAX;
        l.line("first after rotate");
        assert!(std::fs::read_to_string(&path).unwrap().contains("first after rotate"));
        l.size = LOG_MAX;
        l.line("second");
        l.size = LOG_MAX;
        l.line("third");
        assert!(d.path().join("host.log.1").exists() && d.path().join("host.log.2").exists());
        assert!(!d.path().join("host.log.3").exists(), "只留 {LOG_KEEP} 份旧的");
        assert_eq!(std::fs::read_to_string(d.path().join("host.log.2")).unwrap().trim(), "first after rotate");
    }
}
