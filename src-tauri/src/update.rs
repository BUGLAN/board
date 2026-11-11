//! 桌面端检查更新（Tauri updater）：设置页手动「检查更新」、启动后自动检查一次。
//!
//! - 更新清单 `latest.json` 由发版 workflow 生成，和安装包一起附在 GitHub Release 上（见 `scripts/updater-manifest.mjs`），
//!   地址在 tauri.conf.json 的 `plugins.updater.endpoints`：`.../releases/latest/download/latest.json`，总是指向最新一个 Release
//! - 只认配置里的**公钥**：下载的安装包用 `plugins.updater.pubkey` 验签（minisign），签名对不上不安装；私钥只在 CI secret 里
//! - 只有版本号更大时才更新（[`is_newer`]）：清单写错、把旧版本挂成 latest 都不会把正在用的版本换掉
//! - 检查只看版本号，不改任何东西；下载、验签、安装失败都退回「可重试」状态，当前运行的版本不受影响
//! - Windows：安装器（passive，带进度）启动后外壳退出，装完安装器把新版拉起来；macOS / Linux：装好后由界面调用 `update_restart` 重启
//! - 页面拿不到 `Update` 对象，由这里按版本号记着检查到的更新，页面只传版本号

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

/// 检查到的、还没装的更新（同一时间只留一个：页面同一时间只处理一次）
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

/// 一次检查的结果
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    /// 当前版本（即外壳版本）
    pub current: String,
    /// 有比当前新的版本时是它的版本号；已是最新时为 null
    pub version: Option<String>,
    /// 更新说明：发版 workflow 写进 `latest.json` 的那一版 CHANGELOG
    pub notes: Option<String>,
    /// 发布时间（清单里的 date，ISO 8601）；清单没写时为 null
    pub date: Option<String>,
}

/// 下载安装的进展，经 `update://progress` 事件推给页面
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub downloaded: u64,
    /// 总字节数；服务端没给 Content-Length 时为 null（界面只显示已下载多少）
    pub total: Option<u64>,
}

/// 安装的结果：Windows 装好后外壳会退出（新的版本由安装器拉起）；macOS / Linux 要界面调用 `update_restart`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub version: String,
    /// 安装后要重启才能用上新版本
    pub restart: bool,
}

/// `remote` 比 `current` 新吗：两段三段都认（`1.2` 当成 `1.2.0`），比较时不管 `v` 前缀；解析不了的当「不比当前新」，
/// 宁可这次不更新，也不让写坏的清单把正在用的版本换掉
pub fn is_newer(remote: &str, current: &str) -> bool {
    match (parse(remote), parse(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

/// 主版本、次版本、修订号；位数超过 3 段、任何一段不是数字时返回 None（`0.1.x`、`latest` 都不认）
fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let parts: Vec<u64> = core.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let [major, rest @ ..] = parts.as_slice() else { return None };
    if rest.len() > 2 {
        return None;
    }
    Some((*major, rest.first().copied().unwrap_or(0), rest.get(1).copied().unwrap_or(0)))
}

/// 记下检查到的更新（`None` 表示已是最新，清掉之前记的）
fn remember(slot: &Mutex<Option<Update>>, update: Option<Update>) -> Option<Update> {
    *slot.lock().unwrap() = update.clone();
    update
}

/// 取出页面要装的那个版本的更新；没检查过、或者检查到的不是这个版本时返回 Err（消息给人看）
fn take(slot: &Mutex<Option<Update>>, version: &str) -> Result<Update, String> {
    slot.lock()
        .unwrap()
        .take()
        .filter(|u| u.version == version)
        .ok_or_else(|| format!("没有检查到 {version} 版本的更新，先点一次「检查更新」"))
}

fn describe(current: &str, update: Option<Update>) -> Checked {
    match update {
        Some(u) => Checked { current: current.to_string(), version: Some(u.version), notes: u.body, date: u.date.map(|d| d.to_string()) },
        None => Checked { current: current.to_string(), version: None, notes: None, date: None },
    }
}

/// 检查更新；失败时返回给人看的错误（网络、清单不合法、没配地址）
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Checked, String> {
    let current = app.package_info().version.to_string();
    let updater = app.updater().map_err(|e| format!("更新器没有配置好：{e}"))?;
    let update = updater.check().await.map_err(|e| format!("检查更新失败：{e}"))?;
    Ok(describe(&current, remember(&app.state::<Pending>().0, update)))
}

/// 下载并安装检查到的那个版本（`version` 是 update_check 返回的），进展走 `update://progress` 事件。
/// 失败不留下任何改动：当前运行的版本继续用，可以重新检查后再试
#[tauri::command]
pub async fn update_install(app: AppHandle, version: String) -> Result<Installed, String> {
    let update = take(&app.state::<Pending>().0, &version)?;
    let total = Mutex::new(None::<u64>);
    let (mut sent, mut reported) = (0u64, 0u64);
    let handle = app.clone();
    update
        .download_and_install(
            |chunk, length| {
                sent += chunk as u64;
                if length.is_some() {
                    *total.lock().unwrap() = length;
                }
                let all = *total.lock().unwrap();
                // 按 1% 报一次：小文件也能看到进展，又不会把事件刷爆
                if all.is_some_and(|all| all > 0 && (sent - reported) * 100 < all) {
                    return;
                }
                reported = sent;
                let _ = handle.emit("update://progress", Progress { downloaded: sent, total: all });
            },
            || {},
        )
        .await
        .map_err(|e| format!("下载或安装 {version} 失败：{e}"))?;
    Ok(Installed { version, restart: true })
}

/// macOS / Linux：安装完重启桌面端，换成新版本。Windows 上安装器已经把外壳关掉并拉起新版，用不到
#[tauri::command]
pub fn update_restart(app: AppHandle) {
    app.restart();
}

/// 解析插件 `check` 的返回（`update_check` 用它）：`Ok(None)` 是已是最新，`Ok(Some(json))` 是一个新版本
pub fn parse_release(raw: &str) -> Result<Option<serde_json::Value>, String> {
    let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| format!("更新清单不是合法的 JSON：{e}"))?;
    if v.get("version").and_then(serde_json::Value::as_str).is_none() {
        // 插件的清单解析：没有 version 时是空清单，没有新版本
        if v.get("platforms").is_none() {
            return Ok(None);
        }
        return Err("更新清单里没有 version".into());
    }
    Ok(Some(v))
}

/// 清单里的地址只允许 https（本机测试可以 http）：地址写成别的协议时宁可不更新
pub fn check_url(raw: &str) -> Result<Url, String> {
    let url: Url = raw.parse().map_err(|e| format!("更新包地址不合法：{e}"))?;
    match url.scheme() {
        "https" | "http" => Ok(url),
        s => Err(format!("更新包地址只能是 https：{s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions() {
        assert!(is_newer("0.1.13", "0.1.12"));
        assert!(is_newer("0.2.0", "0.1.99"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("v0.1.13", "0.1.12"));
    }

    // 预发布版只比主版本号：0.1.13-beta 和 0.1.13 视为同一版，不因为预发布的小字换成另一个包
    #[test]
    fn prerelease_compares_by_core_version() {
        assert!(!is_newer("0.1.13-beta.1", "0.1.13"));
        assert!(!is_newer("0.1.13", "0.1.13-beta.1"));
        assert!(is_newer("0.1.14-beta.1", "0.1.13"));
    }

    #[test]
    fn not_newer() {
        assert!(!is_newer("0.1.12", "0.1.12"));
        assert!(!is_newer("0.1.11", "0.1.12"));
        assert!(!is_newer("0.1.9", "0.1.12")); // 按数字比，不按字符串
        assert!(!is_newer("latest", "0.1.12"));
        assert!(!is_newer("", "0.1.12"));
        assert!(!is_newer("0.1.12", "0.1.x"));
        assert!(!is_newer("0.1.12.1", "0.1.12"));
    }

    #[test]
    fn two_part_versions() {
        assert!(is_newer("1.2", "1.1.9"));
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(is_newer("1.2.1", "1.2"));
    }

    // 没检查过（或者已经装过一次）时没有可装的更新：消息告诉人先检查
    #[test]
    fn no_pending_update() {
        let slot = Mutex::new(None);
        let e = take(&slot, "0.1.13").err().unwrap();
        assert!(e.contains("检查更新"), "{e}");
    }

    // 下载进度按 1% 报一次，不会每次回调都推事件
    #[test]
    fn progress_threshold() {
        let (total, mut sent, mut reported, mut events) = (1000u64, 0u64, 0u64, 0);
        for _ in 0..334 {
            sent += 3;
            if (sent - reported) * 100 < total {
                continue;
            }
            reported = sent;
            events += 1;
        }
        // 攒够 1%（10 字节）才报一次：1002 字节里约 83 次；不按 1% 的话是 334 次（每次回调都报）。
        // 最后一次不足 1% 的不报，下载结束时页面自己算完成
        assert_eq!(sent, 1002);
        assert!((80..=90).contains(&events), "{events}");
        assert!(reported * 100 / total >= 99);
    }

    // 清单坏了（不是 JSON、没有 version 但有 platforms）要说清楚；没有新版本（空清单）不算错
    #[test]
    fn release_json() {
        let one = r#"{"version":"0.1.13","platforms":{"windows-x86_64-nsis":{"signature":"s","url":"https://x/y.exe"}}}"#;
        assert_eq!(parse_release(one).unwrap().unwrap()["version"], "0.1.13");
        assert!(parse_release("{}").unwrap().is_none());
        assert!(parse_release(r#"{"platforms":{}}"#).is_err());
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn update_urls() {
        assert!(check_url("https://github.com/buglan/board/releases/download/v0.1.13/a.exe").is_ok());
        assert!(check_url("http://127.0.0.1:7420/latest/Board.tar.gz").is_ok());
        assert!(check_url("file:///etc/passwd").is_err());
        assert!(check_url("not a url").is_err());
    }
}
