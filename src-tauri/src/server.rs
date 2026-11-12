//! 桌面端连哪个服务端：按顺序取环境变量 `BOARD_URL`、配置文件 `~/.board/config.toml` 的 `url`
//! （和 `board` 命令行同一项）、构建时注入的默认地址 `BOARD_DEFAULT_URL`；都没有时显示「连接服务端」页，由人填写。
//!
//! - 代码里不写死具体地址：自己部署的服务端在构建时用 `BOARD_DEFAULT_URL` 注入（见 docs/deploy.md），
//!   不注入的发布版第一次打开就是「连接服务端」页
//! - 地址先补全（`localhost:7420` → `http://localhost:7420`），只支持 http / https；本机地址（localhost、127.x、
//!   [::1]）和公网域名都可以
//! - 填写、切换之前先请求 `<地址>/api/health`，返回 `{"ok":true}` 才认作 Board 服务端，通过了才写进配置文件
//! - 窗口只给「当前这个地址」开放本机命令（见 lib.rs 的 `allow_configured_url`）：换了地址，旧地址的页面就调不到本机命令
//! - 换地址会关掉所有窗口，按新地址重新打开（本机在跑 board host 时由人先确认，再按新地址重启它）；
//!   地址来自环境变量 `BOARD_URL` 时环境变量改不了：只写配置文件，提示重新打开桌面端才生效

use board_core::config;
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime, Url};

/// 构建时注入的默认地址（`BOARD_DEFAULT_URL`，`build.rs` 读它公开给这里）；没有注入时是空
pub const DEFAULT_URL: &str = env!("BOARD_DEFAULT_URL");

/// 校验 `<地址>/api/health` 的最长时间
const HEALTH_WAIT: Duration = Duration::from_secs(8);

/// 当前打开的服务端；None：还没连服务端（显示「连接服务端」页）
pub struct Current(pub Mutex<Option<Url>>);

pub fn env_url() -> Option<String> {
    std::env::var("BOARD_URL").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// 配置文件里的 `url`；读不了（没登录过、文件坏了）时当没有
pub fn config_url() -> Option<String> {
    config::Config::load().ok().and_then(|c| c.get(config::URL).ok().flatten())
}

/// 决定打开哪个地址：`BOARD_URL` → 配置文件的 `url` → 构建时注入的默认地址 → None（显示「连接服务端」页）
pub fn resolve() -> Result<Option<Url>, String> {
    if let Some(raw) = env_url() {
        return normalize(&raw).map(Some).map_err(|e| format!("BOARD_URL {e}"));
    }
    if let Some(raw) = config_url() {
        return normalize(&raw).map(Some).map_err(|e| format!("{} 里的 url {e}", config::shown()));
    }
    if !DEFAULT_URL.trim().is_empty() {
        return normalize(DEFAULT_URL).map(Some).map_err(|e| format!("构建时注入的默认地址 {e}"));
    }
    Ok(None)
}

/// 把填进来的地址收拾成可用的地址：去掉首尾空白和路径、查询参数（只留 origin），没有协议时按 http 补上；
/// 只支持 http / https，并且要带主机名
pub fn normalize(raw: &str) -> Result<Url, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("地址是空的".into());
    }
    let with_scheme = if s.contains("://") { s.to_string() } else { format!("http://{s}") };
    let mut u: Url = with_scheme.parse().map_err(|_| format!("不是合法的地址「{s}」"))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(format!("只能用 http 或 https 地址「{s}」"));
    }
    if u.host_str().is_none_or(str::is_empty) {
        return Err(format!("地址里没有主机名「{s}」"));
    }
    // 只保留 origin：后面的人填的路径、查询参数都不带
    if u.path() != "/" && !u.path().is_empty() {
        u.set_path("/");
    }
    u.set_query(None);
    u.set_fragment(None);
    Ok(u)
}

/// 配置里存的写法：origin
pub fn origin(u: &Url) -> String {
    u.origin().ascii_serialization()
}

/// 是不是本机地址（localhost、127.x、[::1]）：这类地址的页面也能调本机命令
pub fn is_local_host(u: &Url) -> bool {
    match u.host_str() {
        Some(h) => {
            let h = h.trim_start_matches('[').trim_end_matches(']');
            h.eq_ignore_ascii_case("localhost")
                || h.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
        }
        None => false,
    }
}

/// 校验回应是不是 Board 服务端的 `/api/health`：`{"ok":true,…}`。返回版本号（旧服务端没有时为空）
pub fn read_health(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        return None;
    }
    Some(v.get("version").and_then(|x| x.as_str()).unwrap_or_default().to_string())
}

/// 请求 `<地址>/api/health`，确认对方是 Board 服务端；返回它的版本号（旧服务端没有时为空）
pub fn health(url: &Url) -> Result<String, String> {
    let mut target = url.clone();
    target.set_path("/api/health");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HEALTH_WAIT))
        // 4xx / 5xx 也当作「不是 Board」处理，不要当成传输错误
        .http_status_as_error(false)
        .user_agent(concat!("board-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let res = agent.get(target.as_str()).call().map_err(|e| format!("连不上 {}：{e}", origin(url)))?;
    if !res.status().is_success() {
        return Err(format!("{} 返回了 HTTP {}，不是 Board 服务端", origin(url), res.status()));
    }
    let mut res = res;
    let body = res.body_mut().read_to_string().map_err(|e| format!("读不出 {} 的回应：{e}", origin(url)))?;
    read_health(&body).ok_or_else(|| format!("{} 返回的不是 Board 服务端的回应", origin(url)))
}

/* ------------------------------ 页面调用的命令 ------------------------------ */

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    /// 收拾过的地址（origin）
    pub url: String,
    /// 服务端版本（旧服务端没有时为空）
    pub version: String,
    /// 地址来自哪里：env / config / default（BOARD_URL 等的页面拿这个决定「切换服务端」怎么显示）
    pub from: String,
    /// 地址不是来自环境变量（切换会写进配置文件）
    pub configured: bool,
    /// 写进配置文件的位置，给人看的
    pub target: String,
    /// 这是本机地址
    pub local: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connected {
    pub url: String,
    /// 改了当前打开的服务端：窗口已经关掉，和外壳一起重启；页面不用再做什么
    pub reload: bool,
    /// 写了配置文件，但要重启之后才生效（正在跑的进程换不了开放的地址）
    pub restart: bool,
}

fn checked(url: &Url, from: &str, version: String) -> Checked {
    Checked {
        url: origin(url),
        version,
        from: from.to_string(),
        configured: from != "env",
        target: config::shown(),
        local: is_local_host(url),
    }
}

/// 「设置 → 账号 · 切换服务端」显示当前连的是哪个地址
#[tauri::command]
pub async fn server_current<R: Runtime>(app: AppHandle<R>) -> Checked {
    let (url, from) = {
        let st = app.state::<Current>();
        let cur = st.0.lock().ok().and_then(|g| g.clone());
        let from = if env_url().is_some() {
            "env"
        } else if config_url().is_some() {
            "config"
        } else if cur.is_some() {
            "saved"
        } else {
            "default"
        };
        (cur, from)
    };
    let Some(url) = url else {
        return Checked { url: String::new(), version: String::new(), from: from.into(), configured: false, target: config::shown(), local: false };
    };
    // 版本号拿不到不影响显示：只显示地址
    let version = tauri::async_runtime::spawn_blocking({
        let url = url.clone();
        move || health(&url).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    checked(&url, from, version)
}

/// 校验一个地址是不是 Board 服务端（只校验，不保存、不切换）：返回它的版本号
#[tauri::command]
pub async fn server_check(raw: String) -> Result<Checked, String> {
    let url = normalize(&raw)?;
    let from = if env_url().is_some() { "env" } else { "input" };
    let version = tauri::async_runtime::spawn_blocking({
        let url = url.clone();
        move || health(&url)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(checked(&url, from, version))
}

/// 把地址写进 `~/.board/config.toml` 的 `url`（保留其他配置项和注释，和 `board login` 同一个入口）
pub fn save_url(url: &str) -> Result<(), String> {
    config::update(|c| {
        c.set(config::URL, url);
        Ok(((), true))
    })
    .map_err(|e| e.message)
}

/// 改成新的服务端：画好的窗口换不了开放给谁，所以关掉所有窗口、按新地址重新打开。
/// 调用方要保证这一步在主线程上做：关窗口、建窗口、加能力都是主线程的事
pub fn switch_to(app: &AppHandle, url: Url) -> Result<(), String> {
    {
        let st = app.state::<Current>();
        let mut g = st.0.lock().map_err(|e| e.to_string())?;
        *g = Some(url.clone());
    }
    for w in app.webview_windows().values() {
        let _ = w.close();
    }
    // 新窗口按新地址放行本机命令（和启动时同一份权限）
    if let Some(cap) = crate::configured_capability(&url) {
        app.add_capability(cap).map_err(|e| e.to_string())?;
    }
    crate::open_window(app).map_err(|e| e.to_string())
}


/// 人的确认之后切到新地址：先校验这个地址是 Board 服务端，通过后写进 `~/.board/config.toml`，再按新地址重新打开。
/// - 连着的还是这个地址：什么都不做
/// - 地址来自环境变量 `BOARD_URL`：环境变量改不了，和配置文件里写的不一样时只写配置文件，提示重启后生效
/// - 本机在跑 board host：先停掉（它读的是配置文件），重新打开后按新地址再启动（见 lib.rs 的启动流程）
#[tauri::command]
pub async fn server_connect(app: AppHandle, raw: String) -> Result<Connected, String> {
    let url = normalize(&raw)?;
    let new = origin(&url);
    let cur = app.state::<Current>().0.lock().ok().and_then(|g| g.clone());
    if cur.as_ref().is_some_and(|c| origin(c) == new) {
        return Ok(Connected { url: new, reload: false, restart: false });
    }
    let from_env = env_url().is_some();
    // 先校验。校验和写文件会卡网络、磁盘：放到后台线程
    let u = url.clone();
    tauri::async_runtime::spawn_blocking(move || health(&u)).await.map_err(|e| e.to_string())??;
    if from_env && config_url().is_none_or(|s| normalize(&s).ok().map(|x| origin(&x)).as_deref() != Some(new.as_str())) {
        // 环境变量改不了，只写配置文件：下次打开桌面端按新地址来
        let n = new.clone();
        tauri::async_runtime::spawn_blocking(move || save_url(&n)).await.map_err(|e| e.to_string())??;
        return Ok(Connected { url: new, reload: false, restart: true });
    }
    if from_env {
        return Ok(Connected { url: new, reload: false, restart: true });
    }
    let n = new.clone();
    tauri::async_runtime::spawn_blocking(move || save_url(&n)).await.map_err(|e| e.to_string())??;
    // 本机在跑 board host：它读的是配置文件，先停下来；重新打开窗口后启动流程按新地址再启动
    if crate::host::status(&app).running {
        crate::host::stop(&app, crate::host::STOP_WAIT)?;
    }
    // 关窗口、建窗口都在主线程上做；先把新地址记下来，新窗口建出来时按它放行本机命令
    app.state::<Current>().0.lock().map(|mut g| *g = Some(url.clone())).map_err(|e| e.to_string())?;
    // 打开了「让这台电脑接任务」：按新地址重新启动 board host（启动时读配置文件里的新地址）
    if crate::host::enabled(&app) {
        if let Err(e) = crate::host::ensure_started(&app) {
            eprintln!("没能按新地址启动 board host：{e}");
        }
    }
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Err(e) = switch_to(&handle, url) {
            eprintln!("没能按新地址重新打开窗口：{e}");
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(Connected { url: new, reload: true, restart: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_fills_scheme_and_keeps_origin() {
        assert_eq!(normalize("localhost:7420").unwrap().as_str(), "http://localhost:7420/");
        assert_eq!(normalize("  https://board.example.com  ").unwrap().as_str(), "https://board.example.com/");
        assert_eq!(normalize("http://127.0.0.1:7420/some/path?q=1#x").unwrap().as_str(), "http://127.0.0.1:7420/");
        assert_eq!(normalize("https://board.example.com/board").unwrap().as_str(), "https://board.example.com/");
    }

    #[test]
    fn normalize_rejects_junk() {
        assert!(normalize("").is_err());
        assert!(normalize("   ").is_err());
        assert!(normalize("ftp://board.example.com").is_err());
        assert!(normalize("not a url").is_err());
        assert!(normalize("http://").is_err());
    }

    #[test]
    fn local_hosts() {
        assert!(is_local_host(&normalize("localhost:7420").unwrap()));
        assert!(is_local_host(&normalize("http://127.0.0.1:7420").unwrap()));
        assert!(is_local_host(&normalize("http://[::1]:7420").unwrap()));
        assert!(!is_local_host(&normalize("http://192.168.1.5:7420").unwrap()));
        assert!(!is_local_host(&normalize("https://board.example.com").unwrap()));
    }

    /// 默认地址来自构建时的 BOARD_DEFAULT_URL：不注入时为空（第一次打开显示「连接服务端」页），注入时要能解析
    #[test]
    fn default_url_is_injected_at_build_time() {
        assert!(DEFAULT_URL.is_empty() || normalize(DEFAULT_URL).is_ok(), "BOARD_DEFAULT_URL={DEFAULT_URL}");
    }
}
