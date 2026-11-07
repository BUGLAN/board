//! Board 桌面端：窗口直接打开线上的 Board（默认 https://board.buglan.com），
//! 与网页端同源，登录用同一个 GitHub 会话。
//!
//! - 地址可以用环境变量 `BOARD_URL` 覆盖（开发时指向 http://localhost:1420 或本机服务端）
//! - GitHub 登录在系统默认浏览器里完成（用上浏览器里已有的 GitHub 登录状态），登录结果经本机回环地址
//!   交回窗口，见 [`login`]。会话 Cookie 保存在 webview 里，所有窗口共用
//! - 顶栏即标题栏，按住顶栏空白处拖动窗口：
//!   macOS 交通灯叠在顶栏左侧；Windows 去掉系统标题栏和菜单栏，最小化 / 最大化 / 关闭由界面画在顶栏右侧
//! - 设置页新建令牌后，可以一键把 BOARD_URL、BOARD_TOKEN 写成长期生效的环境变量，见 [`env`]
//! - 新建窗口（⌘⇧N / Ctrl+Shift+N）：每个窗口可以打开不同的白板。macOS 在「文件」菜单里；Windows 没有菜单栏，
//!   由界面响应快捷键，命令面板里也有

mod env;
mod login;

use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const DEFAULT_URL: &str = "https://board.buglan.com";

static WINDOWS: AtomicUsize = AtomicUsize::new(0);

struct Board(Url);

fn board_url() -> Result<Url, String> {
    let raw = std::env::var("BOARD_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_string());
    raw.parse().map_err(|e| format!("BOARD_URL 不是合法的地址「{raw}」：{e}"))
}

/// 告诉页面它运行在哪种外壳里（每次加载页面前执行）
fn shell_script() -> &'static str {
    if cfg!(target_os = "windows") {
        // 没有系统标题栏：页面自己画窗口按钮，并在画好后调用 window_ready
        "window.__BOARD_SHELL__ = 'desktop'; window.__BOARD_TITLEBAR__ = 'custom';"
    } else {
        "window.__BOARD_SHELL__ = 'desktop';"
    }
}

fn open_window(app: &AppHandle) -> tauri::Result<()> {
    let url = app.state::<Board>().0.clone();
    let n = WINDOWS.fetch_add(1, Ordering::SeqCst);
    let mut builder = WebviewWindowBuilder::new(app, format!("board-{n}"), WebviewUrl::External(url))
        .title("Board")
        .inner_size(1360.0, 860.0)
        .min_inner_size(380.0, 560.0)
        // 拖入文件交给页面处理（卡片附件）；否则 Tauri 会拦下文件拖放，页面收不到 drop 事件
        .disable_drag_drop_handler()
        .initialization_script(shell_script());
    // 新窗口错开一点，免得完全叠在上一个窗口上
    if n > 0 {
        if let Some(prev) = app.webview_windows().values().next() {
            if let (Ok(pos), Ok(scale)) = (prev.outer_position(), prev.scale_factor()) {
                let p = pos.to_logical::<f64>(scale);
                builder = builder.position(p.x + 28.0, p.y + 28.0);
            }
        }
    }
    // macOS：顶栏即标题栏，交通灯垂直居中在 60px 高的顶栏里
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(20.0, 34.0));
    #[cfg(target_os = "windows")]
    let builder = custom_titlebar::apply(builder);
    builder.build()?;
    Ok(())
}

/// Windows：去掉系统标题栏（保留阴影、圆角和边缘缩放），窗口按钮由页面画。
/// 页面没能加载出来（断网、服务端出错）时不会有窗口按钮，这时恢复系统标题栏，免得窗口关不掉、拖不动
#[cfg(target_os = "windows")]
mod custom_titlebar {
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::time::Duration;
    use tauri::webview::PageLoadEvent;
    use tauri::{Manager, Runtime, WebviewWindow, WebviewWindowBuilder};

    /// 当前页面已经画好窗口按钮的窗口
    #[derive(Default)]
    pub struct Ready(Mutex<HashSet<String>>);

    pub fn apply<'a, R: Runtime, M: Manager<R>>(b: WebviewWindowBuilder<'a, R, M>) -> WebviewWindowBuilder<'a, R, M> {
        b.decorations(false).on_page_load(|w, p| {
            match p.event() {
                PageLoadEvent::Started => {
                    w.state::<Ready>().0.lock().unwrap().remove(w.label());
                }
                PageLoadEvent::Finished => {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(3));
                        if !w.state::<Ready>().0.lock().unwrap().contains(w.label()) {
                            let _ = w.set_decorations(true);
                        }
                    });
                }
            }
        })
    }

    pub fn ready<R: Runtime>(w: &WebviewWindow<R>) {
        w.state::<Ready>().0.lock().unwrap().insert(w.label().to_string());
        if w.is_decorated().unwrap_or(false) {
            let _ = w.set_decorations(false);
        }
    }
}

/* ------------------------------ 页面调用的命令 ------------------------------ */

/// 在系统默认浏览器里用 GitHub 登录，返回一次性登录码（页面拿它打开 /auth/desktop 换取会话）
#[tauri::command]
async fn desktop_login(window: WebviewWindow, board: tauri::State<'_, Board>) -> Result<String, login::LoginError> {
    let url = board.0.clone();
    let res = tauri::async_runtime::spawn_blocking(move || login::run(&url))
        .await
        .unwrap_or_else(|e| Err(login::LoginError { reason: "io".into(), login: None, message: Some(e.to_string()) }));
    // 浏览器那边有了结果：把窗口带回前台
    if !matches!(&res, Err(e) if e.reason == "superseded") {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    res
}

/// 新建窗口（Windows 没有菜单栏，由页面的快捷键和命令面板调用）。
/// 必须是异步命令：Windows 上在同步命令里创建窗口会卡死
#[tauri::command]
async fn new_window(app: AppHandle) -> Result<(), String> {
    open_window(&app).map_err(|e| e.to_string())
}

/// 设置页新建令牌后「一键写入环境变量」：BOARD_URL 用外壳打开的地址，令牌由页面传入（见 [`env`]）
#[tauri::command]
async fn set_board_env(app: AppHandle, token: String) -> Result<env::Written, String> {
    let url = env::origin_of(&app.state::<Board>().0)?;
    tauri::async_runtime::spawn_blocking(move || env::write(&url, token.trim()))
        .await
        .map_err(|e| e.to_string())?
}

/// 页面已经画好窗口按钮（只在 Windows 上有意义）
#[tauri::command]
fn window_ready(window: WebviewWindow) {
    #[cfg(target_os = "windows")]
    custom_titlebar::ready(&window);
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}

pub fn run() {
    let url = match board_url() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let builder = tauri::Builder::default().manage(Board(url));
    #[cfg(target_os = "windows")]
    let builder = builder.manage(custom_titlebar::Ready::default());
    builder
        .invoke_handler(tauri::generate_handler![desktop_login, new_window, window_ready, set_board_env])
        .setup(|app| {
            let handle = app.handle();
            // Windows 不设菜单栏：顶栏就是标题栏，编辑快捷键由 webview 自己处理
            if !cfg!(target_os = "windows") {
                let new_window = MenuItem::with_id(handle, "new-window", "新建窗口", true, Some("CmdOrCtrl+Shift+N"))?;
                let file = SubmenuBuilder::new(handle, "文件").item(&new_window).separator().close_window().build()?;
                let edit = SubmenuBuilder::new(handle, "编辑")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .select_all()
                    .build()?;
                let window = SubmenuBuilder::new(handle, "窗口").minimize().maximize().separator().fullscreen().build()?;
                let menu = if cfg!(target_os = "macos") {
                    let app_menu = SubmenuBuilder::new(handle, "Board")
                        .about(None)
                        .separator()
                        .services()
                        .separator()
                        .hide()
                        .hide_others()
                        .show_all()
                        .separator()
                        .quit()
                        .build()?;
                    MenuBuilder::new(handle).items(&[&app_menu, &file, &edit, &window]).build()?
                } else {
                    MenuBuilder::new(handle).items(&[&file, &edit, &window]).build()?
                };
                app.set_menu(menu)?;
                app.on_menu_event(|app, e| {
                    if e.id() == "new-window" {
                        let _ = open_window(app);
                    }
                });
            }
            open_window(handle)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Board 桌面端启动失败");
}
