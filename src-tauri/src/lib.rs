//! Board 桌面端：窗口直接打开线上的 Board（默认 https://board.buglan.com），
//! 与网页端同源，登录用同一个 GitHub 会话。
//!
//! - 地址可以用环境变量 `BOARD_URL` 覆盖（开发时指向 http://localhost:1420 或本机服务端）
//! - GitHub 登录在窗口里完成；会话 Cookie 保存在 webview 里，所有窗口共用
//! - 顶栏即标题栏（macOS 交通灯叠在顶栏左侧），按住顶栏空白处拖动窗口
//! - 「文件 → 新建窗口」（⌘⇧N / Ctrl+Shift+N）：每个窗口可以打开不同的白板

use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

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

fn open_window(app: &AppHandle) -> tauri::Result<()> {
    let url = app.state::<Board>().0.clone();
    let n = WINDOWS.fetch_add(1, Ordering::SeqCst);
    let mut builder = WebviewWindowBuilder::new(app, format!("board-{n}"), WebviewUrl::External(url))
        .title("Board")
        .inner_size(1360.0, 860.0)
        .min_inner_size(380.0, 560.0)
        .initialization_script("window.__BOARD_SHELL__ = 'desktop';");
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
    builder.build()?;
    Ok(())
}

pub fn run() {
    let url = match board_url() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    tauri::Builder::default()
        .manage(Board(url))
        .setup(|app| {
            let handle = app.handle();
            let new_window = MenuItem::with_id(handle, "new-window", "新建窗口", true, Some("CmdOrCtrl+Shift+N"))?;
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
            open_window(handle)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Board 桌面端启动失败");
}
