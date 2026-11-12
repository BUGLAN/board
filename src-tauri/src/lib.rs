//! Board 桌面端：窗口打开 Board 服务端的页面，与网页端同源，登录用同一个 GitHub 会话。
//!
//! - 连哪个服务端见 [`server`]：环境变量 `BOARD_URL` → 配置文件 `~/.board/config.toml` 的 `url`（和 `board` 命令行同一项）
//!   → 构建时注入的 `BOARD_DEFAULT_URL`；都没有时显示「连接服务端」页，由人填写（先校验 `/api/health` 再写进配置文件）。
//!   设置 → 账号里的「切换服务端」换地址，换完按新地址重新打开窗口，本机命令只对新地址开放。代码里不写死地址
//! - GitHub 登录在系统默认浏览器里完成（用上浏览器里已有的 GitHub 登录状态），登录结果经本机回环地址
//!   交回窗口，见 [`login`]。会话 Cookie 保存在 webview 里，所有窗口共用
//! - 顶栏即标题栏，按住顶栏空白处拖动窗口：
//!   macOS 交通灯叠在顶栏左侧；Windows 去掉系统标题栏和菜单栏，最小化 / 最大化 / 关闭由界面画在顶栏右侧
//! - 设置页新建令牌后，可以一键把服务端地址和令牌写进 board 命令行的配置文件 `~/.board/config.toml`，见 [`env`]
//! - 新建窗口（⌘⇧N / Ctrl+Shift+N）：每个窗口可以打开不同的白板。macOS 在「文件」菜单里；Windows 没有菜单栏，
//!   由界面响应快捷键，命令面板里也有
//! - 窗口先打开本地启动页，连得上服务端再跳到线上页面；窗口底色跟随网页端的主题，慢网、断网都不白屏，见 [`splash`]
//! - 「设置 → 本机」打开「让这台电脑接任务」后，在后台用自带的 board 命令行运行 `board host`，关掉窗口也继续运行
//!   （macOS 点 Dock 图标、其他系统点托盘图标重新打开窗口），退出桌面端时结束，见 [`host`]
//! - 检查更新：设置页手动检查、启动后自动检查一次，确认后下载、验签、安装，见 [`update`]
//! - 「设置 → 本机」里 agent 看不到 board-skills 时一键安装到它读 skill 的目录，见 [`skill`]

mod env;
mod host;
mod login;
mod server;
mod skill;
mod splash;
mod update;

use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::{AppHandle, Manager, Theme, Url, WebviewWindow, WebviewWindowBuilder};

static WINDOWS: AtomicUsize = AtomicUsize::new(0);

/// 当前打开的服务端（见 [`server`]）；还没连服务端时是 None，窗口打开本地启动页（「连接服务端」页）
fn board_url(app: &AppHandle) -> Option<Url> {
    app.state::<server::Current>().0.lock().ok().and_then(|g| g.clone())
}

/// 桌面端自带的 board 命令行：externalBin 把它放在可执行文件旁边
/// （Windows 安装目录里的 board.exe；macOS 是 Board.app/Contents/MacOS/board）
fn cli_path() -> Option<std::path::PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let name = if cfg!(target_os = "windows") { "board.exe" } else { "board" };
    let p = dir.join(name);
    p.is_file().then_some(p)
}

/// 告诉页面它运行在哪种外壳里、外壳的版本、自带的 board 命令行在哪（每次加载页面前执行）
fn shell_script(app: &AppHandle) -> String {
    let version = app.package_info().version.to_string();
    let cli = cli_path()
        .map(|p| serde_json::to_string(&p.display().to_string()).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    if cfg!(target_os = "windows") {
        // 没有系统标题栏：页面自己画窗口按钮，并在画好后调用 window_ready
        format!("window.__BOARD_SHELL__ = 'desktop'; window.__BOARD_TITLEBAR__ = 'custom'; window.__BOARD_SHELL_VERSION__ = '{version}'; window.__BOARD_CLI__ = {cli};")
    } else {
        format!("window.__BOARD_SHELL__ = 'desktop'; window.__BOARD_SHELL_VERSION__ = '{version}'; window.__BOARD_CLI__ = {cli};")
    }
}

/// Windows 任务栏按 24 DIP 绘制窗口图标。Tauri 默认窗口图标在进程 DPI 感知建立前取到 32px，
/// 200% 缩放下会被任务栏放大到 48px 而模糊；这里按窗口实际缩放从 exe 图标资源读取精确帧。
#[cfg(target_os = "windows")]
fn taskbar_icon_size(scale_factor: f64) -> u32 {
    (24.0 * scale_factor).round().clamp(16.0, 256.0) as u32
}

#[cfg(target_os = "windows")]
fn apply_taskbar_icon(window: &WebviewWindow) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let size = taskbar_icon_size(scale);
    if let Ok(icon) = tauri::image::Image::from_icon_resource(
        tauri::utils::platform::WINDOWS_APP_ICON_RESOURCE_ID,
        size,
        size,
    ) {
        let _ = window.set_icon(icon);
    }
}

/// 显示已有的窗口；没有窗口（都关掉了，程序还在后台接任务）时新开一个
pub(crate) fn show_window(app: &AppHandle) {
    match app.webview_windows().values().next() {
        Some(w) => {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
        None => {
            let _ = open_window(app);
        }
    }
}

fn open_window(app: &AppHandle) -> tauri::Result<()> {
    let n = WINDOWS.fetch_add(1, Ordering::SeqCst);
    let theme = splash::load_theme(app);
    // 跟随系统时先按已有窗口的深浅色猜，建好窗口后再按系统设置校正（还没开始绘制）
    let guess = match theme {
        splash::Theme::Dark => true,
        splash::Theme::Light => false,
        splash::Theme::Auto => app.webview_windows().values().next().and_then(|w| w.theme().ok()) == Some(Theme::Dark),
    };
    // 先打开本地启动页，连得上服务端再由启动页跳到线上页面
    let mut builder = WebviewWindowBuilder::new(app, format!("board-{n}"), splash::url())
        .title("Board")
        // 窗口、webview 的底色：启动页画出来之前、跳转途中露出来的都是主题底色，不是白色
        .background_color(splash::background(guess))
        .inner_size(1360.0, 860.0)
        .min_inner_size(380.0, 560.0)
        // 拖入文件交给页面处理（卡片附件）；否则 Tauri 会拦下文件拖放，页面收不到 drop 事件
        .disable_drag_drop_handler()
        .initialization_script(&shell_script(app));
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
    let window = builder.build()?;
    #[cfg(target_os = "windows")]
    apply_taskbar_icon(&window);
    if theme == splash::Theme::Auto {
        let dark = window.theme().ok() == Some(Theme::Dark);
        if dark != guess {
            let _ = window.set_background_color(Some(splash::background(dark)));
        }
    }
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
async fn desktop_login(window: WebviewWindow, app: AppHandle) -> Result<String, login::LoginError> {
    let url = board_url(&app).ok_or_else(|| login::LoginError::not_connected())?;
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

/// 设置页新建令牌后「一键写入配置文件」：服务端地址用外壳打开的地址，令牌由页面传入（见 [`env`]）。
/// 命令名沿用 set_board_env，新旧页面和外壳都能互相调用
#[tauri::command]
async fn set_board_env(app: AppHandle, token: String) -> Result<env::Written, String> {
    let url = board_url(&app).ok_or("还没有连接服务端")?;
    let url = env::origin_of(&url)?;
    tauri::async_runtime::spawn_blocking(move || env::write(&url, token.trim()))
        .await
        .map_err(|e| e.to_string())?
}

/// 网页端应用主题时告诉外壳（auto / light / dark）：下次启动页和窗口底色按它来
#[tauri::command]
fn remember_theme(app: AppHandle, theme: String) -> Result<(), String> {
    let t = splash::Theme::parse(&theme).ok_or_else(|| format!("未知的主题「{theme}」"))?;
    splash::save_theme(&app, t)
}

/// 页面已经画好窗口按钮（只在 Windows 上有意义）
#[tauri::command]
fn window_ready(window: WebviewWindow) {
    #[cfg(target_os = "windows")]
    custom_titlebar::ready(&window);
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}

/// capabilities/default.json 只放行构建时注入的默认地址（BOARD_DEFAULT_URL）和启动页。
/// 当前打开的服务端是别的地址（配置文件、BOARD_URL、人填的地址，开发时的 localhost:1420）时，
/// 只给这一个来源放行同样的命令，不放行整个 localhost：窗口导航到本机别的端口上的页面时调不了本机命令。
/// 换了地址就是新开窗口，按新地址重新调用这里——旧地址的页面从此调不到本机命令
pub(crate) fn allow_configured_url(app: &AppHandle) -> tauri::Result<()> {
    if let Some(url) = board_url(app) {
        if let Some(cap) = configured_capability(&url) {
            app.add_capability(cap)?;
        }
    }
    Ok(())
}

/// 给一个服务端地址放行本机命令的能力（见 [`allow_configured_url`]）；是默认地址（default.json 已经放行）时是 None
fn configured_capability(url: &Url) -> Option<tauri::ipc::CapabilityBuilder> {
    let origin = url.origin().ascii_serialization();
    let default = server::normalize(server::DEFAULT_URL).ok().map(|u| u.origin().ascii_serialization());
    if default.as_deref() == Some(origin.as_str()) {
        return None;
    }
    let mut cap = tauri::ipc::CapabilityBuilder::new("configured-url").remote(format!("{origin}/*")).window("*");
    for p in default_permissions() {
        cap = cap.permission(p);
    }
    Some(cap)
}

/// default.json 里的权限（和它保持一份）
fn default_permissions() -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(include_str!("../capabilities/default.json")).expect("default.json");
    v["permissions"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

pub fn run() {
    // 连哪个服务端：BOARD_URL → 配置文件的 url → 构建时注入的默认地址；都没有时先显示「连接服务端」页
    let url = match server::resolve() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let builder = tauri::Builder::default()
        // 必须第一个注册：第二次启动时把参数交给已经在跑的这个，自己退出
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(server::Current(std::sync::Mutex::new(url)))
        .manage(host::HostProc::default())
        .manage(update::Pending::default());
    #[cfg(target_os = "windows")]
    let builder = builder.manage(custom_titlebar::Ready::default());
    builder
        // 启动页：任何路径都返回内联 HTML（见 splash）；还没连服务端时就是「连接服务端」页
        .register_uri_scheme_protocol(splash::SCHEME, |ctx, _req| {
            let app = ctx.app_handle();
            splash::respond(app, board_url(app).as_ref())
        })
        .invoke_handler(tauri::generate_handler![
            desktop_login,
            new_window,
            window_ready,
            set_board_env,
            remember_theme,
            server::server_current,
            server::server_check,
            server::server_connect,
            host::host_status,
            host::host_save,
            host::host_enable,
            host::pick_folder,
            skill::skill_install,
            update::update_check,
            update::update_install,
            update::update_restart
        ])
        .setup(|app| {
            let handle = app.handle();
            allow_configured_url(handle)?;
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
            // board host 按远程「重启」「升级」退出时接手（见 host::watch）
            host::watch(handle.clone());
            // 上次打开了「让这台电脑接任务」：连上了服务端就在后台运行 board host（还没连时等连接好再启动，见 server.rs）
            if host::enabled(handle) && board_url(handle).is_some() {
                if let Err(e) = host::ensure_started(handle) {
                    eprintln!("{e}");
                }
                host::tray(handle, true);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Board 桌面端启动失败")
        .run(|app, e| match e {
            // 最后一个窗口关掉：在接任务时不退出，留在后台（Dock / 托盘图标重新打开窗口）
            tauri::RunEvent::ExitRequested { code: None, api, .. } if host::enabled(app) => api.prevent_exit(),
            // 退出桌面端：关闭 board host 的标准输入，它自己结束 agent、让机器离线后退出
            tauri::RunEvent::Exit => {
                let _ = host::stop(app, std::time::Duration::from_secs(2));
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { has_visible_windows: false, .. } => show_window(app),
            _ => {}
        });
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::taskbar_icon_size;

    #[test]
    fn taskbar_icon_size_follows_windows_scaling() {
        assert_eq!(taskbar_icon_size(1.0), 24);
        assert_eq!(taskbar_icon_size(1.25), 30);
        assert_eq!(taskbar_icon_size(1.5), 36);
        assert_eq!(taskbar_icon_size(1.75), 42);
        assert_eq!(taskbar_icon_size(2.0), 48);
    }
}

#[cfg(test)]
mod acl_tests {
    use super::*;

    /// 不注入默认地址的构建里，源码和配置里都搜不到写死的服务端地址（默认地址只从 BOARD_DEFAULT_URL 来）
    #[test]
    fn no_hardcoded_server_in_code_and_config() {
        for (name, text) in [
            ("lib.rs", include_str!("lib.rs")),
            ("server.rs", include_str!("server.rs")),
            ("env.rs", include_str!("env.rs")),
            ("splash.rs", include_str!("splash.rs")),
            ("capabilities/default.json", include_str!("../capabilities/default.json")),
        ] {
            assert!(!text.contains(&["buglan", ".com"].concat()), "{name} 里还有写死的服务端地址");
        }
    }

    /// 发布版不放行 localhost / 127.0.0.1 的任意端口：只放行启动页，当前连的服务端按地址另外放行同一份权限。
    /// 代码里不写死服务端地址：默认地址只在构建时注入（BOARD_DEFAULT_URL）
    #[test]
    fn remote_urls_and_permissions() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let urls: Vec<&str> = v["remote"]["urls"].as_array().unwrap().iter().map(|u| u.as_str().unwrap()).collect();
        assert_eq!(urls, ["http://boardsplash.localhost/*"]);
        let perms = default_permissions();
        assert!(perms.iter().any(|p| p == "allow-host-enable"), "{perms:?}");
        assert!(perms.iter().any(|p| p == "core:default"), "{perms:?}");
        // 切换服务端：「设置 → 账号」要调这三个
        for p in ["allow-server-current", "allow-server-check", "allow-server-connect"] {
            assert!(perms.iter().any(|x| x == p), "缺 {p}：{perms:?}");
        }
        // 检查更新：三个命令都要放行，否则线上页面里「检查更新」调不通
        for p in ["allow-update-check", "allow-update-install", "allow-update-restart"] {
            assert!(perms.iter().any(|x| x == p), "缺 {p}：{perms:?}");
        }
    }
}
