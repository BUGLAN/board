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

fn home() -> Option<PathBuf> {
    let keys: [&str; 2] = if cfg!(windows) { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
    keys.iter().find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
}

fn config_path() -> Result<PathBuf, String> {
    home().map(|h| h.join(".board").join("config.toml")).ok_or_else(|| "找不到用户目录".to_string())
}

fn log_path() -> Option<PathBuf> {
    home().map(|h| h.join(".board").join("host").join("desktop.log"))
}

fn load_doc(path: &Path) -> Result<toml_edit::DocumentMut, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    text.parse().map_err(|e| format!("~/.board/config.toml 不是合法的 TOML：{e}"))
}

/// 读 `[host]`：机器名、白板与目录（按文件里的顺序）；以及有没有服务端地址和令牌
fn read_conf() -> (String, Vec<Dir>, bool) {
    let Ok(path) = config_path() else { return (String::new(), vec![], false) };
    let Ok(doc) = load_doc(&path) else { return (String::new(), vec![], false) };
    let has = |k: &str| doc.get(k).and_then(|v| v.as_str()).is_some_and(|s| !s.trim().is_empty());
    let logged_in = (has("url") || std::env::var("BOARD_URL").is_ok_and(|v| !v.trim().is_empty())) && has("token");
    let host = doc.get("host").and_then(|h| h.as_table_like());
    let name = host.and_then(|h| h.get("name")).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let dirs = host
        .and_then(|h| h.get("dirs"))
        .and_then(|d| d.as_table_like())
        .map(|t| t.iter().filter_map(|(k, v)| v.as_str().map(|p| Dir { board: k.to_string(), path: p.to_string() })).collect())
        .unwrap_or_default();
    (name, dirs, logged_in)
}

/// 机器名：小写字母、数字、. 和 -，以字母或数字开头，最长 24 个字符（与 board 命令行相同）
fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 24
        && n.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

fn valid_board(b: &str) -> bool {
    let key = |k: &str| {
        !k.is_empty()
            && k.len() <= 32
            && k.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    matches!(b.split_once('/'), Some((p, k)) if key(p) && key(k))
}

fn expand(p: &str) -> PathBuf {
    match (p.strip_prefix('~'), home()) {
        (Some(rest), Some(h)) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') => h.join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(p),
    }
}

/// 写 `[host]`，保留文件里的其他配置项和注释；机器名为空时去掉 name（用本机主机名）
pub fn write_conf(path: &Path, name: &str, dirs: &[Dir]) -> Result<(), String> {
    let name = name.trim();
    if !name.is_empty() && !valid_name(name) {
        return Err(format!("机器名「{name}」不对：只能用小写字母、数字、. 和 -，以字母或数字开头，最长 24 个字符"));
    }
    let mut seen: Vec<&str> = vec![];
    for d in dirs {
        if !valid_board(d.board.trim()) {
            return Err(format!("白板要写成 <项目>/<白板>：{}", d.board));
        }
        if seen.contains(&d.board.trim()) {
            return Err(format!("白板 {} 写了两次", d.board.trim()));
        }
        seen.push(d.board.trim());
        if d.path.trim().is_empty() {
            return Err(format!("{} 还没有选目录", d.board.trim()));
        }
        if !expand(d.path.trim()).is_dir() {
            return Err(format!("目录不存在：{}（{}）", d.path.trim(), d.board.trim()));
        }
    }
    let mut doc = load_doc(path)?;
    if doc.get("host").and_then(|h| h.as_table_like()).is_none() {
        doc["host"] = toml_edit::table();
    }
    let host = doc["host"].as_table_like_mut().ok_or("[host] 格式不对")?;
    if name.is_empty() {
        host.remove("name");
    } else {
        host.insert("name", toml_edit::value(name));
    }
    let mut t = toml_edit::InlineTable::new();
    for d in dirs {
        t.insert(d.board.trim(), d.path.trim().into());
    }
    host.insert("dirs", toml_edit::value(t));
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(".config.toml.tmp");
    {
        use std::io::Write;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(doc.to_string().as_bytes()).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
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
        if let Some(h) = home() {
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
fn stop_proc(mut r: Running, wait: std::time::Duration) {
    drop(r.stdin.take());
    let begun = std::time::Instant::now();
    while begun.elapsed() < wait {
        if matches!(r.child.try_wait(), Ok(Some(_))) {
            return;
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

/// 停掉后台的 board host（退出桌面端、关掉开关、改了设置要重启时）
pub fn stop<R: Runtime>(app: &AppHandle<R>, wait: std::time::Duration) {
    let r = app.state::<HostProc>().0.lock().ok().and_then(|mut g| g.take());
    if let Some(r) = r {
        stop_proc(r, wait);
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
    let host = if cfg!(windows) {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    } else {
        std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    };
    Status {
        enabled: enabled(app),
        running,
        exit,
        name,
        default_name: default_name(&host),
        dirs,
        log: log_tail(40),
        logged_in,
        cli: crate::cli_path().is_some(),
    }
}

/// 不填机器名时 board host 用的名字：主机名转成小写，非法字符换成 -（与 board 命令行相同）
fn default_name(raw: &str) -> String {
    let s: String = raw
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' { c } else { '-' })
        .collect();
    let s: String = s.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).chars().take(24).collect();
    let s = s.trim_end_matches(['-', '.']).to_string();
    if s.is_empty() { "host".into() } else { s }
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

/// 保存机器名、白板与目录（写进 ~/.board/config.toml 的 [host]）；正在运行时重启 board host
#[tauri::command]
pub async fn host_save(app: AppHandle, name: String, dirs: Vec<Dir>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        write_conf(&config_path()?, &name, &dirs)?;
        if enabled(&app) {
            stop(&app, std::time::Duration::from_secs(15));
            ensure_started(&app)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 打开 / 关掉「让这台电脑接任务」
#[tauri::command]
pub async fn host_enable(app: AppHandle, on: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        save_flag(&app, on)?;
        if on {
            ensure_started(&app)?;
        } else {
            stop(&app, std::time::Duration::from_secs(15));
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

    #[test]
    fn names() {
        assert!(valid_name("macbook"));
        assert!(valid_name("lans-mbp.local"));
        assert!(!valid_name("MacBook"));
        assert!(!valid_name("-x"));
        assert_eq!(default_name("Lans-MacBook-Pro.local"), "lans-macbook-pro.local");
        assert!(valid_board("default/main"));
        assert!(!valid_board("default"));
    }

    #[test]
    fn writes_host_section_and_keeps_other_items() {
        let d = std::env::temp_dir().join(format!("board-host-test-{}", std::process::id()));
        let repo = d.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let f = d.join(".board").join("config.toml");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "url = \"https://board.buglan.com\"\ntoken = \"bd_x\"\n").unwrap();
        let dirs = vec![Dir { board: "default/main".into(), path: repo.display().to_string() }];
        write_conf(&f, "macbook", &dirs).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.starts_with("url = \"https://board.buglan.com\"\ntoken = \"bd_x\"\n"), "{text}");
        assert!(text.contains("[host]\nname = \"macbook\"\ndirs = { \"default/main\" = "), "{text}");
        // 机器名清空：去掉 name
        write_conf(&f, "", &dirs).unwrap();
        assert!(!std::fs::read_to_string(&f).unwrap().contains("name ="));
        // 目录不存在
        let bad = vec![Dir { board: "default/main".into(), path: d.join("nope").display().to_string() }];
        assert!(write_conf(&f, "macbook", &bad).unwrap_err().contains("目录不存在"));
        std::fs::remove_dir_all(&d).ok();
    }
}
