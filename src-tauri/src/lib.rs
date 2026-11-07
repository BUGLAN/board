//! Board 桌面端。
//!
//! 启动时在本机随机端口上运行与网页端相同的 Board 服务（board-server），
//! 窗口加载同一套界面，通过注入的 `window.__BOARD_API__` 访问这个服务。
//! 数据库与 `board` 命令行共用：默认 `~/.board/board.db`，`BOARD_DB` 可指定。
//!
//! - 顶栏即标题栏（macOS 交通灯叠在顶栏左侧），按住顶栏空白处拖动窗口
//! - 「文件 → 新建窗口」（⌘⇧N / Ctrl+Shift+N）：每个窗口可以打开不同的白板

use board_core::Store;
use board_server::{bind, serve, AppState, Config};
use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

static WINDOWS: AtomicUsize = AtomicUsize::new(0);

struct Api(String);

fn start_server() -> Result<String, String> {
    let cfg = Config {
        db: Store::default_path(),
        demo: std::env::var_os("BOARD_DEMO").is_some(),
        serve_web: false,
        cors_origins: [
            "tauri://localhost",
            "http://tauri.localhost",
            "https://tauri.localhost",
            "http://localhost:1420",
            "http://127.0.0.1:1420",
        ]
        .map(String::from)
        .to_vec(),
    };
    let state = AppState::open(&cfg).map_err(|e| format!("打开数据库 {} 失败：{e}", cfg.db.display()))?;
    let listener = bind(([127, 0, 0, 1], 0).into()).map_err(|e| format!("无法启动本地服务：{e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    std::thread::Builder::new()
        .name("board-server".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                if let Err(e) = serve(state, cfg, listener).await {
                    eprintln!("Board 服务异常退出：{e}");
                }
            });
        })
        .map_err(|e| e.to_string())?;
    Ok(format!("http://127.0.0.1:{port}"))
}

fn open_window(app: &AppHandle) -> tauri::Result<()> {
    let api = app.state::<Api>().0.clone();
    let n = WINDOWS.fetch_add(1, Ordering::SeqCst);
    let script = format!(
        "window.__BOARD_API__ = {}; window.__BOARD_SHELL__ = 'desktop';",
        serde_json::to_string(&api).unwrap_or_default()
    );
    let mut builder = WebviewWindowBuilder::new(app, format!("board-{n}"), WebviewUrl::App("index.html".into()))
        .title("Board")
        .inner_size(1360.0, 860.0)
        .min_inner_size(380.0, 560.0)
        .initialization_script(&script);
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
    let api = match start_server() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    tauri::Builder::default()
        .manage(Api(api))
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
