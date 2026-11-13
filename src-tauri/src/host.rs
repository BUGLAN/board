//! 「设置 → 本机」：让这台电脑接任务（任务下发需求 §三）。
//!
//! - 开关打开后，桌面端用自带的 board 命令行在后台运行 `board host`（环境变量 `BOARD_HOST_STDIN=1`：
//!   标准输入关闭就按 Ctrl+C 处理——先结束它启动的 agent，再让机器离线）。
//! - 开关打开时关掉窗口也继续运行：最后一个窗口关闭时不退出程序；macOS 点 Dock 图标、其他系统点托盘图标重新打开窗口。
//!   退出桌面端（菜单「退出」、托盘「退出」）时关闭 board host 的标准输入，它自己收尾后退出。
//! - 机器名、白板与目录写进 `~/.board/config.toml` 的 `[host]`（board host 读它）。保存时不重启 board host：
//!   往它的标准输入写一行 `reload` 通知它重新读配置，换上新的机器名、白板与目录，正在跑的 agent 不受影响。
//! - 开关记在应用配置目录的 `host.json`，下次打开桌面端自动运行。
//! - board host 的输出写到 `~/.board/host/desktop.log`；页面上能看到它在不在跑和最近的输出。
//! - macOS 从 Finder 打开的程序拿不到 shell 里的 PATH：先用登录 shell 取一次 PATH，board host 才找得到 claude 这些命令。
//! - 远程管理（PRD §9.1）：界面上请求「重启 board host」时，Windows 上 board host 不能原地换进程，退出码 75 交给这里重新启动；
//!   请求「升级」时 board host 退出码 76，这里检查、下载、验签、安装桌面端更新（和设置里手动更新同一套 updater），装好后重启桌面端，
//!   重启后照常接任务；失败时带上原因（`BOARD_HOST_ASK_ERROR`）重新启动 board host，它上报给服务端，服务端恢复接单（见 [`watch`]）。

use board_core::config;
use board_core::hosts::{check_host_name, host_name_from};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
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
    /// 开机时自动启动（见 autostart.rs）
    pub autostart: bool,
}

/* ------------------------------ 配置文件 ------------------------------ */

fn log_path() -> Option<PathBuf> {
    config::board_dir().ok().map(|d| d.join("host").join("desktop.log"))
}

/// 读 `[host]`：机器名、白板与目录（按文件里的顺序）；以及有没有服务端地址和令牌
fn read_conf() -> (String, Vec<Dir>, bool) {
    let Ok(c) = config::Config::load() else { return (String::new(), vec![], false) };
    let has = |k: &str| c.get(k).ok().flatten().is_some();
    let logged_in = (has(config::URL) || crate::server::env_url().is_some()) && has(config::TOKEN);
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
        if !board_core::valid_board(b) {
            return Err(format!("白板要写成 <项目>/<白板>：{}", d.board));
        }
        if seen.contains(&b) {
            return Err(format!("白板 {b} 写了两次"));
        }
        seen.push(b);
        if d.path.trim().is_empty() {
            return Err(format!("{b} 还没有选目录"));
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

/// 启动 board host 时要从环境里去掉的变量。
///
/// 桌面端自己怎么连服务端（`BOARD_URL` 可能指向本机页面 :1420）、从哪个终端启动（里面可能开着别的服务端的令牌，
/// 或者某个 agent 会话的 `BOARD_AS` / `BOARD`），都和后台的 board host 无关：它只用配置文件
/// （或 `BOARD_CONFIG` 指的那份）里的地址和令牌，agent 名字和白板由它自己按派发设置。
/// 继承下来的话，环境变量优先于配置文件，board host 会拿别人的令牌去连配置文件里的服务端，只得到 `INVALID_TOKEN`。
const INHERITED: [&str; 9] = [
    "BOARD_URL",
    "BOARD_TOKEN",
    "BOARD_HOST_URL",
    "BOARD_HOST_TOKEN",
    "BOARD_CONFIG",
    "BOARD_AS",
    "BOARD_HOST",
    "BOARD",
    "BOARD_EXE",
];

/// 启动 board host 的进程参数（[`start_proc`] 和 [`rename_host`] 共用：两条命令都只认配置文件）
fn base_command(cli: &Path) -> Command {
    let mut c = Command::new(cli);
    for k in INHERITED {
        c.env_remove(k);
    }
    c.env("NO_COLOR", "1");
    c
}

fn start_proc(cli: &Path, ask_error: Option<&str>) -> Result<Running, String> {
    let log = log_path().ok_or("找不到用户目录")?;
    if let Some(d) = log.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let out = std::fs::File::create(&log).map_err(|e| format!("无法创建 {}：{e}", log.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let mut c = base_command(cli);
    c.arg("host").env("BOARD_HOST_STDIN", "1").stdin(Stdio::piped()).stdout(out).stderr(err);
    if let Some(e) = ask_error {
        c.env("BOARD_HOST_ASK_ERROR", e);
    }
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

/// 停止：关闭标准输入（board host 按 Ctrl+C 收尾：结束 agent、让机器离线），等它真的退出。到时间还没退出时把它交回来
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

/// 保存了配置、装好了 board-skills：在跑的 board host 收到一行 `reload` 后重新读 `[host]`、重新检查 skill（不重启）。
/// 返回有没有通知到（没在跑、写不进去时 false）
pub(crate) fn notify_reload<R: Runtime>(app: &AppHandle<R>) -> bool {
    use std::io::Write;
    let state = app.state::<HostProc>();
    let Ok(mut g) = state.0.lock() else { return false };
    let Some(r) = g.as_mut() else { return false };
    if !matches!(r.child.try_wait(), Ok(None)) {
        return false;
    }
    r.stdin.as_mut().is_some_and(|i| i.write_all(b"reload\n").and_then(|_| i.flush()).is_ok())
}

/// 打开时启动（已经在跑就不动）
pub fn ensure_started<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    start_with(app, None)
}

/// 同 [`ensure_started`]；ask_error 是上一次升级没成的原因，新启动的 board host 第一次上报时带给服务端
fn start_with<R: Runtime>(app: &AppHandle<R>, ask_error: Option<&str>) -> Result<(), String> {
    let cli = crate::cli_path().ok_or("没找到桌面端自带的 board 命令行")?;
    let state = app.state::<HostProc>();
    let mut g = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(r) = g.as_mut() {
        if matches!(r.child.try_wait(), Ok(None)) {
            return Ok(());
        }
    }
    *g = Some(start_proc(&cli, ask_error)?);
    Ok(())
}

/// board host 要桌面端重启它（和 board 命令行的 `host::EXIT_RESTART` 一致）
const EXIT_RESTART: i32 = 75;
/// board host 要桌面端升级（和 board 命令行的 `host::EXIT_UPGRADE` 一致）
const EXIT_UPGRADE: i32 = 76;

/// 后台盯着 board host（每秒看一次）：按远程「重启」退出（75）就重新启动它；按「升级」退出（76）就升级桌面端。
/// 别的退出码不管（界面上照实显示停了、退出码多少）
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let code = {
            let state = app.state::<HostProc>();
            let Ok(mut g) = state.0.lock() else { continue };
            let code = g.as_mut().and_then(|r| r.child.try_wait().ok().flatten()).and_then(|s| s.code());
            if matches!(code, Some(EXIT_RESTART | EXIT_UPGRADE)) {
                g.take();
            }
            code
        };
        match code {
            Some(EXIT_RESTART) => {
                if let Err(e) = start_with(&app, None) {
                    eprintln!("重新启动 board host 失败：{e}");
                }
            }
            Some(EXIT_UPGRADE) => tauri::async_runtime::block_on(upgrade(app.clone())),
            _ => {}
        }
    });
}

/// 远程「升级」：检查、下载、验签、安装桌面端更新（只认配置里的公钥，见 update.rs），装好后重启桌面端；
/// 失败时带上原因重新启动 board host，它上报后服务端恢复接单
async fn upgrade(app: AppHandle) {
    use tauri_plugin_updater::UpdaterExt;
    let done: Result<(), String> = async {
        let updater = app.updater().map_err(|e| format!("更新器没有配置好：{e}"))?;
        let update = updater.check().await.map_err(|e| format!("检查更新失败：{e}"))?.ok_or("没有找到比现在新的桌面端版本")?;
        let version = update.version.clone();
        update.download_and_install(|_, _| {}, || {}).await.map_err(|e| format!("下载或安装 {version} 失败：{e}"))?;
        Ok(())
    }
    .await;
    match done {
        // Windows 上安装器已经关掉外壳、装完会把新版拉起来；macOS / Linux 重启换成新版本。重启后 board host 照常启动
        Ok(()) => app.restart(),
        Err(e) => {
            if let Err(e2) = start_with(&app, Some(&format!("升级失败：{e}"))) {
                eprintln!("升级失败（{e}），重新启动 board host 也失败：{e2}");
            }
        }
    }
}

/// board host 收尾最长要等 agent 退出（10 秒）再上报几次：给足时间
pub const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(40);

/// 停掉后台的 board host（退出桌面端、关掉开关），等它真的退出。
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
            Err(format!("board host {} 秒内没有退出；稍后再试，或退出桌面端", wait.as_secs()))
        }
    }
}

fn hostname() -> String {
    if cfg!(windows) {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    } else {
        std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    }
}

/// 从文件尾往前读这么多字节找最后 n 行：日志很大时不再整份读进来（设置页每 4 秒看一次状态）
const LOG_TAIL_BYTES: u64 = 64 * 1024;

/// 日志最后 n 行（去掉颜色控制符和空行）：从文件尾 seek 读一块，不够 n 行时再往前扩（最多读到文件头）
fn log_tail(n: usize) -> Vec<String> {
    match log_path() {
        Some(p) => tail_of(&p, n),
        None => vec![],
    }
}

fn tail_of(path: &Path, n: usize) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut size = LOG_TAIL_BYTES;
    loop {
        let from = len.saturating_sub(size);
        if f.seek(SeekFrom::Start(from)).is_err() {
            return vec![];
        }
        let mut buf = Vec::with_capacity((len - from) as usize);
        if f.read_to_end(&mut buf).is_err() {
            return vec![];
        }
        let text = String::from_utf8_lossy(&buf);
        // 不是从文件头读的：第一行可能只剩半截，丢掉
        let all: Vec<&str> = text.lines().skip(if from > 0 { 1 } else { 0 }).collect();
        let lines: Vec<String> = all.iter().map(|l| l.replace('\u{1b}', "")).filter(|l| !l.trim().is_empty()).collect();
        if lines.len() >= n || from == 0 {
            return lines[lines.len().saturating_sub(n)..].to_vec();
        }
        // 这一块里的行不够：往前扩一倍再读
        size *= 2;
    }
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
        autostart: crate::autostart::enabled(),
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
        autostart: false,
    })
}

/// 生效的机器名：配置里没写（空）时用本机主机名——和 board host 启动时的算法一致
fn effective_name(configured: &str) -> String {
    let n = configured.trim();
    if n.is_empty() {
        host_name_from(&hostname())
    } else {
        n.to_string()
    }
}

/// 调自带的 board host rename，让服务端把这台机器的记录原地改名（设置和历史派发记录保留）。
/// 失败时 Err((错误码, 原因))：错误码和原因取自 board 的输出（「✗ 错误码  原因」）
fn rename_host(new_name: &str) -> Result<(), (String, String)> {
    let Some(cli) = crate::cli_path() else {
        return Err((String::new(), "没找到桌面端自带的 board 命令行：机器名没法在服务端上改，重新安装桌面端试试".into()));
    };
    let mut c = base_command(&cli);
    c.args(["host", "rename", new_name]).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let out = c.output().map_err(|e| (String::new(), format!("运行 board host rename 失败：{e}")))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or_default().trim();
    let line = line.strip_prefix('✗').map(str::trim).unwrap_or(line);
    let (code, msg) = line.split_once("  ").map(|(a, b)| (a.trim(), b.trim())).unwrap_or((line, line));
    Err((code.to_string(), if msg.is_empty() { code.to_string() } else { msg.to_string() }))
}

/// 保存机器名、白板与目录（写进 ~/.board/config.toml 的 [host]）。不重启 board host：写完配置通知在跑的
/// board host（标准输入一行 `reload`），它重新读配置，换上新的机器名、白板与目录，正在跑的 agent 不受影响。
/// 机器名变了：先让服务端把机器记录原地改名（设置和历史派发记录保留），成功了再写配置；
/// 改名失败（新名字已被别的机器占用等）时不写配置，返回原因，board host 用旧名字接着跑。
/// 在 board host 读到新配置之前，它用旧名字的上报由服务端按进程标识认回改名后的记录，不会新建一台机器。
/// 确认（机器名、白板与目录、免审批运行的风险）由页面用应用内的确认框做完再调用这里
#[tauri::command]
pub async fn host_save(app: AppHandle, name: String, dirs: Vec<Dir>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let name = name.trim();
        check_conf(name, &dirs)?;
        let (old_name, _old_dirs, logged_in) = read_conf();
        let new = effective_name(name);
        if logged_in && effective_name(&old_name) != new {
            if let Err((code, msg)) = rename_host(&new) {
                // 服务端还没有这台机器的记录（从没运行过 board host）：没有要改的，直接按新名字建
                if code != "NO_SUCH_HOST" {
                    return Err(msg);
                }
            }
        }
        save_conf(name, &dirs)?;
        // 在跑：通知它重新读配置；没在跑（之前出错退出了）才启动，启动时就读新配置
        if enabled(&app) && !notify_reload(&app) {
            ensure_started(&app)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 打开 / 关掉「让这台电脑接任务」。打开时连同页面上的机器名、白板与目录一起传来（人在应用内的
/// 确认框里看过、确认过），先校验再写进配置，然后启动 board host；关掉不用确认
#[tauri::command]
pub async fn host_enable(app: AppHandle, on: bool, name: String, dirs: Vec<Dir>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if on {
            check_conf(name.trim(), &dirs)?;
            save_conf(&name, &dirs)?;
        }
        save_flag(&app, on)?;
        if on {
            // 打开「让这台电脑接任务」时默认一起打开开机自启（可以在设置里单独关掉）；没开成不影响接任务，设置页上开关是关的
            if let Err(e) = crate::autostart::set(true) {
                eprintln!("开机时自动启动没打开：{e}");
            }
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

/// 打开 / 关掉「开机时自动启动」（登录系统时在后台启动桌面端，见 autostart.rs）
#[tauri::command]
pub async fn host_autostart(on: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::autostart::set(on)?;
        Ok(crate::autostart::enabled())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 「白板与目录」选择文件夹：弹系统的文件夹选择框，从这一行现在的目录开始（存在时，`~` 按家目录展开）。
/// 返回选中的目录；取消时返回 None。只取路径、不写配置：保存仍走 host_save（目录变化由页面确认）
#[tauri::command]
pub async fn pick_folder(app: AppHandle, start: String) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        let mut dlg = app.dialog().file().set_title("选择这块白板的代码仓库");
        let start = config::expand(start.trim());
        if start.is_dir() {
            dlg = dlg.set_directory(start);
        }
        match dlg.blocking_pick_folder() {
            None => Ok(None),
            Some(p) => p.into_path().map(|p| Some(p.to_string_lossy().into_owned())).map_err(|e| e.to_string()),
        }
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

    /// 日志最后几行从文件尾读：大文件不整份读进来；第一行只剩半截时丢掉，往前扩一块接着找
    #[test]
    fn log_tail_reads_from_end() {
        let d = tempfile_dir("logtail");
        let f = d.join("desktop.log");
        let body: String = (0..500).map(|i| format!("第 {i} 行\n")).collect();
        std::fs::write(&f, &body).unwrap();
        // 定位用的文件由调用方给好了路径：这里直接测读文件的那段
        assert_eq!(tail_of(&f, 3), vec!["第 497 行", "第 498 行", "第 499 行"]);
        // 空行、颜色控制符去掉
        std::fs::write(&f, "\u{1b}[32m好\u{1b}[0m\n\n  \n尾行\n").unwrap();
        assert_eq!(tail_of(&f, 10), vec!["[32m好[0m", "尾行"]);
        // 一个 64 KB 里凑不够 n 行（每行很长）：往前扩一块，不把中间丢了
        let big: String = (0..40).map(|i| format!("{i} {}\n", "x".repeat(4000))).collect();
        std::fs::write(&f, &big).unwrap();
        assert_eq!(tail_of(&f, 20).len(), 20);
        let _ = std::fs::remove_dir_all(&d);
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
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 生效的机器名：配置里没写时用本机主机名（和 board host 启动时一致），写了就去掉空白
    #[test]
    fn effective_names_match_board_host() {
        assert_eq!(effective_name(""), host_name_from(&hostname()));
        assert_eq!(effective_name("  win  "), "win");
        assert_eq!(effective_name("macbook"), "macbook");
    }

    /// 桌面端启动的 board host（和改机器名的 board 命令）只认配置文件：从启动环境继承的
    /// 服务端地址、令牌、`BOARD_CONFIG`、名字和白板都要去掉，否则会拿别人的令牌去连配置文件里的服务端
    #[test]
    fn host_ignores_inherited_connection_env() {
        let probe = base_command(Path::new("/bin/true"));
        for k in INHERITED {
            assert_eq!(probe.get_envs().filter(|(n, v)| *n == k && v.is_none()).count(), 1, "没去掉 {k}");
        }
        // 路径、颜色这些还得有：去掉的只能是连接相关的
        assert!(probe.get_envs().any(|(n, v)| n == "NO_COLOR" && v == Some("1".as_ref())));
    }

    fn tempfile_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("board-host-test-{}-{name}", std::process::id()));
        // 上一次被中断时留下的目录里可能有测试用的空目录「nope」：先清干净，再当作空目录
        let _ = std::fs::remove_dir_all(&d);
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
        std::fs::write(&f, "url = \"https://site.example.com\"\ntoken = \"bd_x\"\n").unwrap();
        let dirs = vec![Dir { board: "default/main".into(), path: repo.display().to_string() }];
        write_conf(&f, true, "macbook", &dirs).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.starts_with("url = \"https://site.example.com\"\ntoken = \"bd_x\"\n"), "{text}");
        assert!(text.contains("[host]\nname = \"macbook\"\ndirs = { \"default/main\" = "), "{text}");
        // 机器名清空：去掉 name
        write_conf(&f, true, "", &dirs).unwrap();
        assert!(!std::fs::read_to_string(&f).unwrap().contains("name ="));
        // 目录不存在也照样保存（#185：board host 上报「不存在」，界面上可以克隆），写进去的是填的路径
        let gone = vec![Dir { board: "default/main".into(), path: d.join("nope").display().to_string() }];
        write_conf(&f, true, "macbook", &gone).unwrap();
        assert!(std::fs::read_to_string(&f).unwrap().contains("nope"), "{}", std::fs::read_to_string(&f).unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    // 设置 → 本机保存机器名：现有配置不是合法 TOML 时，先备份原文件，再写一份合法的 [host]
    #[test]
    fn saving_host_recovers_bad_toml() {
        let d = tempfile_dir("write-bad");
        let repo = d.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let dir = d.join(".board");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("config.toml");
        let bad = "# 旧配置\n[agents.claude]\ncmd = [\"bash\", \"-c\", \"echo agent \\; exec sleep 300\", \"-\"]\n";
        std::fs::write(&f, bad).unwrap();
        let dirs = vec![Dir { board: "default/main".into(), path: repo.display().to_string() }];
        write_conf(&f, true, "macbook", &dirs).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains("[host]\nname = \"macbook\""), "{text}");
        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("config.toml.bad-"))
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        assert_eq!(std::fs::read_to_string(dir.join(&backups[0])).unwrap(), bad);
        let _ = std::fs::remove_dir_all(&d);
    }
}
