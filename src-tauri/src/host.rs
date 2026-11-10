//! 「设置 → 本机」：让这台电脑接任务（任务下发需求 §三）。
//!
//! - 开关打开后，桌面端用自带的 board 命令行在后台运行 `board host`（环境变量 `BOARD_HOST_STDIN=1`：
//!   标准输入关闭就按 Ctrl+C 处理——先结束它启动的 agent，再让机器离线）。
//! - 开关打开时关掉窗口也继续运行：最后一个窗口关闭时不退出程序；macOS 点 Dock 图标、其他系统点托盘图标重新打开窗口。
//!   退出桌面端（菜单「退出」、托盘「退出」）时关闭 board host 的标准输入，它自己收尾后退出。
//! - 机器名、白板与目录写进 `~/.board/config.toml` 的 `[host]`（board host 读它）；保存时如果正在运行就重启它。
//! - 开关记在应用配置目录的 `host.json`，下次打开桌面端自动运行。
//! - board host 的输出写到 `~/.board/host/desktop.log`；页面上能看到它在不在跑和最近的输出。
//! - macOS 从 Finder 打开的程序拿不到 shell 里的 PATH：先用登录 shell 取一次 PATH，board host 才找得到 claude 这些命令。

use board_core::config;
use board_core::hosts::{check_host_name, host_name_from};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

const FLAG_FILE: &str = "host.json";
const TRAY_ID: &str = "board-host";

/// 后台运行的 board host
#[derive(Default)]
pub struct HostProc(Mutex<Option<Running>>);

struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dir {
    pub board: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// 「让这台电脑接任务」打开了
    pub enabled: bool,
    /// board host 正在运行
    pub running: bool,
    /// 停了的话：退出码
    pub exit: Option<i32>,
    /// 配置文件里的机器名（空表示用本机主机名）
    pub name: String,
    /// 不填时用的机器名
    pub default_name: String,
    pub dirs: Vec<Dir>,
    /// board host 最近的输出
    pub log: Vec<String>,
    /// 配置文件里有服务端地址和令牌
    pub logged_in: bool,
    /// 桌面端自带的 board 命令行找到了
    pub cli: bool,
}

/* ------------------------------ 配置文件 ------------------------------ */

fn log_path() -> Option<PathBuf> {
    config::board_dir().ok().map(|d| d.join("host").join("desktop.log"))
}

/// 读 `[host]`：机器名、白板与目录（按文件里的顺序）；以及有没有服务端地址和令牌
fn read_conf() -> (String, Vec<Dir>, bool) {
    let Ok(c) = config::Config::load() else { return (String::new(), vec![], false) };
    let has = |k: &str| c.get(k).ok().flatten().is_some();
    let logged_in = (has(config::URL) || std::env::var("BOARD_URL").is_ok_and(|v| !v.trim().is_empty())) && has(config::TOKEN);
    let Ok(h) = c.host() else { return (String::new(), vec![], logged_in) };
    let dirs = h.dirs.into_iter().map(|(board, path)| Dir { board, path }).collect();
    (h.name.unwrap_or_default(), dirs, logged_in)
}

/// 校验机器名、白板与目录（规则与 board 命令行相同，都在 board_core）
fn check_conf(name: &str, dirs: &[Dir]) -> Result<(), String> {
    if !name.is_empty() {
        check_host_name(name)?;
    }
    let mut seen: Vec<&str> = vec![];
    for d in dirs {
        let b = d.board.trim();
        if !config::valid_board(b) {
            return Err(format!("白板要写成 <项目>/<白板>：{}", d.board));
        }
        if seen.contains(&b) {
            return Err(format!("白板 {b} 写了两次"));
        }
        seen.push(b);
        if d.path.trim().is_empty() {
            return Err(format!("{b} 还没有选目录"));
        }
        if !config::expand(d.path.trim()).is_dir() {
            return Err(format!("目录不存在：{}（{b}）", d.path.trim()));
        }
    }
    Ok(())
}

/// 写 `[host]`，保留文件里的其他配置项和注释；机器名为空时去掉 name（用本机主机名）
pub fn write_conf(path: &Path, own_dir: bool, name: &str, dirs: &[Dir]) -> Result<(), String> {
    let name = name.trim();
    check_conf(name, dirs)?;
    let dirs: Vec<(String, String)> = dirs.iter().map(|d| (d.board.trim().to_string(), d.path.trim().to_string())).collect();
    config::update_at(path, own_dir, |c| c.set_host(Some(name).filter(|n| !n.is_empty()), &dirs).map(|_| ((), true))).map_err(|e| e.message)
}

/// 写 `[host]` 到 board 命令行用的配置文件（`BOARD_CONFIG`，否则 `~/.board/config.toml`）
fn save_conf(name: &str, dirs: &[Dir]) -> Result<(), String> {
    let path = config::path().map_err(|e| e.message)?;
    let own = config::board_dir().is_ok_and(|d| path.parent() == Some(d.as_path()));
    write_conf(&path, own, name, dirs)
}

/* ------------------------------ 开关 ------------------------------ */

fn flag_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(FLAG_FILE))
}

pub fn enabled<R: Runtime>(app: &AppHandle<R>) -> bool {
    flag_path(app).and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|s| s.contains("true"))
}

fn save_flag<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<(), String> {
    let p = flag_path(app).ok_or("找不到配置目录")?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, format!("{{\"enabled\": {on}}}")).map_err(|e| e.to_string())
}

/* ------------------------------ 进程 ------------------------------ */

/// 登录 shell 里的 PATH（macOS / Linux 从图形界面打开的程序拿不到）；取不到时用当前的加上常见目录
fn shell_path() -> Option<String> {
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let out = Command::new(shell).args(["-ilc", "printf '\\n__BOARD_PATH__%s' \"$PATH\""]).stdin(Stdio::null()).stderr(Stdio::null()).output();
            let _ = tx.send(out);
        });
        let from_shell = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .ok()
            .and_then(|o| o.ok())
            .and_then(|o| String::from_utf8_lossy(&o.stdout).rsplit("__BOARD_PATH__").next().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty());
        let mut parts: Vec<String> = from_shell.or_else(|| std::env::var("PATH").ok()).unwrap_or_default().split(':').map(str::to_string).collect();
        if let Some(h) = config::home() {
            for extra in [".local/bin", ".npm-global/bin", ".bun/bin", ".cargo/bin"] {
                parts.push(h.join(extra).display().to_string());
            }
        }
        for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            parts.push(extra.into());
        }
        let mut seen = vec![];
        parts.retain(|p| !p.is_empty() && !seen.contains(p) && {
            seen.push(p.clone());
            true
        });
        Some(parts.join(":"))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn start_proc(cli: &Path) -> Result<Running, String> {
    let log = log_path().ok_or("找不到用户目录")?;
    if let Some(d) = log.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let out = std::fs::File::create(&log).map_err(|e| format!("无法创建 {}：{e}", log.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let mut c = Command::new(cli);
    c.arg("host").env("BOARD_HOST_STDIN", "1").env("NO_COLOR", "1").stdin(Stdio::piped()).stdout(out).stderr(err);
    // 桌面端开发时 BOARD_URL 指向本机页面（:1420），不是服务端：board host 用配置文件里的地址
    c.env_remove("BOARD_URL");
    if let Some(p) = shell_path() {
        c.env("PATH", p);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = c.spawn().map_err(|e| format!("启动 board host 失败：{e}"))?;
    let stdin = child.stdin.take();
    Ok(Running { child, stdin })
}

/// 停止：关闭标准输入，board host 自己结束 agent、让机器离线后退出（最多等 wait）
/// 关闭标准输入（board host 按 Ctrl+C 收尾：结束 agent、让机器离线），等它真的退出。到时间还没退出时把它交回来
fn stop_proc(mut r: Running, wait: std::time::Duration) -> Result<(), Running> {
    drop(r.stdin.take());
    let begun = std::time::Instant::now();
    loop {
        if !matches!(r.child.try_wait(), Ok(None)) {
            return Ok(());
        }
        if begun.elapsed() >= wait {
            return Err(r);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// 打开时启动（已经在跑就不动）
pub fn ensure_started<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let cli = crate::cli_path().ok_or("没找到桌面端自带的 board 命令行")?;
    let state = app.state::<HostProc>();
    let mut g = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(r) = g.as_mut() {
        if matches!(r.child.try_wait(), Ok(None)) {
            return Ok(());
        }
    }
    *g = Some(start_proc(&cli)?);
    Ok(())
}

/// board host 收尾最长要等 agent 退出（10 秒）再上报几次：给足时间
pub const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(40);

/// 停掉后台的 board host（退出桌面端、关掉开关、改了设置要重启时），等它真的退出。
/// 到时间还没退出：留在状态里（界面照实显示在跑），返回错误，调用方不要启动新的（新旧两个会重名）
pub fn stop<R: Runtime>(app: &AppHandle<R>, wait: std::time::Duration) -> Result<(), String> {
    let state = app.state::<HostProc>();
    let r = state.0.lock().ok().and_then(|mut g| g.take());
    let Some(r) = r else { return Ok(()) };
    match stop_proc(r, wait) {
        Ok(()) => Ok(()),
        Err(r) => {
            if let Ok(mut g) = state.0.lock() {
                *g = Some(r);
            }
            Err(format!("board host {} 秒内没有退出，没有重启；稍后再试，或退出桌面端", wait.as_secs()))
        }
    }
}

/// 让这台电脑接任务之前，由外壳弹系统确认框，写明机器名和每个目录（页面被注入脚本也绕不过去）。返回人是否确认
fn confirm_host<R: Runtime>(app: &AppHandle<R>, title: &str, name: &str, dirs: &[Dir]) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};
    let shown = if name.trim().is_empty() { format!("{}（本机主机名）", host_name_from(&hostname())) } else { name.trim().to_string() };
    let list = if dirs.is_empty() {
        "（还没有白板目录）".to_string()
    } else {
        dirs.iter().map(|d| format!("· {} → {}", d.board.trim(), d.path.trim())).collect::<Vec<_>>().join("\n")
    };
    let msg = format!(
        "机器名：{shown}\n\n白板与目录：\n{list}\n\n派发到这台电脑的卡片，agent 会在对应目录里以免审批模式运行。只在信任这些白板时确认。"
    );
    // 默认按钮（回车）是「取消」：误按回车不会让这台电脑开始接任务，要点「确认」才算
    let res = app
        .dialog()
        .message(msg)
        .title(title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom("取消".into(), "确认".into()))
        .blocking_show_with_result();
    matches!(res, MessageDialogResult::Custom(ref s) if s == "确认")
}

/// 人在确认框里点了取消
pub const CANCELLED: &str = "已取消，没有改动";

fn hostname() -> String {
    if cfg!(windows) {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    } else {
        std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    }
}

fn log_tail(n: usize) -> Vec<String> {
    let Some(p) = log_path() else { return vec![] };
    let mut s = String::new();
    if std::fs::File::open(p).and_then(|mut f| f.read_to_string(&mut s)).is_err() {
        return vec![];
    }
    // 去掉颜色控制符
    let lines: Vec<String> = s.lines().map(|l| l.replace('\u{1b}', "")).filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

pub fn status<R: Runtime>(app: &AppHandle<R>) -> Status {
    let (name, dirs, logged_in) = read_conf();
    let (running, exit) = match app.state::<HostProc>().0.lock() {
        Ok(mut g) => match g.as_mut().map(|r| r.child.try_wait()) {
            Some(Ok(None)) => (true, None),
            Some(Ok(Some(s))) => (false, s.code()),
            _ => (false, None),
        },
        Err(_) => (false, None),
    };
    let host = hostname();
    Status {
        enabled: enabled(app),
        running,
        exit,
        name,
        default_name: host_name_from(&host),
        dirs,
        log: log_tail(40),
        logged_in,
        cli: crate::cli_path().is_some(),
    }
}

/* ------------------------------ 托盘 ------------------------------ */

/// 开关打开时在托盘放一个图标：关掉窗口后从这里重新打开或退出
pub fn tray(app: &AppHandle, on: bool) {
    if !on {
        let _ = app.remove_tray_by_id(TRAY_ID);
        return;
    }
    if app.tray_by_id(TRAY_ID).is_some() {
        return;
    }
    use tauri::menu::{MenuBuilder, MenuItem};
    let Ok(open) = MenuItem::with_id(app, "tray-open", "打开 Board", true, None::<&str>) else { return };
    let Ok(quit) = MenuItem::with_id(app, "tray-quit", "退出（这台电脑不再接任务）", true, None::<&str>) else { return };
    let Ok(menu) = MenuBuilder::new(app).item(&open).separator().item(&quit).build() else { return };
    let mut b = tauri::tray::TrayIconBuilder::with_id(TRAY_ID).menu(&menu).tooltip("Board：这台电脑在接任务").on_menu_event(|app, e| {
        match e.id().as_ref() {
            "tray-open" => crate::show_window(app),
            "tray-quit" => app.exit(0),
            _ => {}
        }
    });
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    let _ = b.build(app);
}

/* ------------------------------ 页面调用的命令 ------------------------------ */

/// 「设置 → 本机」的现状
#[tauri::command]
pub async fn host_status(app: AppHandle) -> Status {
    tauri::async_runtime::spawn_blocking(move || status(&app)).await.unwrap_or_else(|_| Status {
        enabled: false,
        running: false,
        exit: None,
        name: String::new(),
        default_name: String::new(),
        dirs: vec![],
        log: vec![],
        logged_in: false,
        cli: false,
    })
}

/// 保存机器名、白板与目录（写进 ~/.board/config.toml 的 [host]）；正在运行时重启 board host。
/// 目录有变化时先弹系统确认框（接任务打开着时，改了目录就是让 agent 去新目录干活）
#[tauri::command]
pub async fn host_save(app: AppHandle, name: String, dirs: Vec<Dir>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        check_conf(name.trim(), &dirs)?;
        let (_, old, _) = read_conf();
        let key = |ds: &[Dir]| ds.iter().map(|d| (d.board.trim().to_string(), d.path.trim().to_string())).collect::<Vec<_>>();
        if key(&old) != key(&dirs) && !confirm_host(&app, "修改这台电脑接任务的白板目录？", &name, &dirs) {
            return Err(CANCELLED.into());
        }
        save_conf(&name, &dirs)?;
        if enabled(&app) {
            stop(&app, STOP_WAIT)?;
            ensure_started(&app)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 打开 / 关掉「让这台电脑接任务」。打开要在系统确认框里确认
#[tauri::command]
pub async fn host_enable(app: AppHandle, on: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        // 打开前由外壳确认（关掉不用确认）
        if on {
            let (name, dirs, _) = read_conf();
            if !confirm_host(&app, "让这台电脑开始接任务？", &name, &dirs) {
                return Err(CANCELLED.into());
            }
        }
        save_flag(&app, on)?;
        if on {
            ensure_started(&app)?;
        } else {
            stop(&app, STOP_WAIT)?;
        }
        let a = app.clone();
        let _ = app.run_on_main_thread(move || tray(&a, on));
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 停 board host：等它真的退出；到时间还没退出就交回来（不能接着启动新的）
    #[cfg(unix)]
    #[test]
    fn stop_waits_for_exit() {
        let spawn = |script: &str| {
            let mut child = Command::new("sh").args(["-c", script]).stdin(Stdio::piped()).spawn().unwrap();
            let stdin = child.stdin.take();
            Running { child, stdin }
        };
        // 标准输入关闭后收尾 0.3 秒再退出：等到它退出
        let begun = std::time::Instant::now();
        assert!(stop_proc(spawn("cat >/dev/null; sleep 0.3"), std::time::Duration::from_secs(5)).is_ok());
        assert!(begun.elapsed() >= std::time::Duration::from_millis(300));
        // 不理会标准输入、一直不退出：到时间交回来，进程还在
        let r = stop_proc(spawn("sleep 30"), std::time::Duration::from_millis(300));
        let mut r = r.err().expect("没退出时应当交回来");
        assert!(matches!(r.child.try_wait(), Ok(None)));
        let _ = r.child.kill();
        let _ = r.child.wait();
    }

    #[test]
    fn checks() {
        let d = tempfile_dir("checks");
        let ok = vec![Dir { board: "default/main".into(), path: d.display().to_string() }];
        assert!(check_conf("macbook", &ok).is_ok());
        assert!(check_conf("", &ok).is_ok());
        assert!(check_conf("MacBook", &ok).unwrap_err().contains("机器名"));
        let bad = vec![Dir { board: "default".into(), path: d.display().to_string() }];
        assert!(check_conf("", &bad).unwrap_err().contains("<项目>/<白板>"));
        let twice = vec![ok[0].clone(), ok[0].clone()];
        assert!(check_conf("", &twice).unwrap_err().contains("两次"));
        std::fs::remove_dir_all(&d).ok();
    }

    fn tempfile_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("board-host-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writes_host_section_and_keeps_other_items() {
        let d = tempfile_dir("write");
        let repo = d.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let f = d.join(".board").join("config.toml");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "url = \"https://board.buglan.com\"\ntoken = \"bd_x\"\n").unwrap();
        let dirs = vec![Dir { board: "default/main".into(), path: repo.display().to_string() }];
        write_conf(&f, true, "macbook", &dirs).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.starts_with("url = \"https://board.buglan.com\"\ntoken = \"bd_x\"\n"), "{text}");
        assert!(text.contains("[host]\nname = \"macbook\"\ndirs = { \"default/main\" = "), "{text}");
        // 机器名清空：去掉 name
        write_conf(&f, true, "", &dirs).unwrap();
        assert!(!std::fs::read_to_string(&f).unwrap().contains("name ="));
        // 目录不存在
        let bad = vec![Dir { board: "default/main".into(), path: d.join("nope").display().to_string() }];
        assert!(write_conf(&f, true, "macbook", &bad).unwrap_err().contains("目录不存在"));
        std::fs::remove_dir_all(&d).ok();
    }
}
