//! 桌面端检查更新（Tauri updater）：设置页手动「检查更新」、启动后自动检查一次。
//!
//! - 更新清单 `latest.json` 由发版 workflow 生成，和安装包一起附在 GitHub Release 上（见 `scripts/updater-manifest.mjs`），
//!   地址在 tauri.conf.json 的 `plugins.updater.endpoints`：`.../releases/latest/download/latest.json`，总是指向最新一个 Release
//! - 只认配置里的**公钥**：下载的安装包用 `plugins.updater.pubkey` 验签（minisign），签名对不上不安装；私钥只在 CI secret 里
//! - 只有版本号更大时才更新：清单写错、把旧版本挂成 latest 都不会把正在用的版本换掉
//! - 检查只看版本号，不改任何东西；下载、验签、安装失败都退回「可重试」状态，当前运行的版本不受影响
//! - Windows：安装器（passive，带进度）启动后外壳退出，装完安装器把新版拉起来；macOS / Linux：装好后由界面调用 `update_restart` 重启
//! - 页面拿不到 `Update` 对象，由这里按版本号记着检查到的更新，页面只传版本号

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
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
                if !should_report(reported, sent, all) {
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

/// 这一块下载完该不该报一次进度：按 1% 报，小文件也能看到进展，又不会把事件刷爆。
/// `reported` 是上一次报出去时的字节数，`total` 是服务端给的总大小（没给时每次都报，否则进度一直不动）
fn should_report(reported: u64, sent: u64, total: Option<u64>) -> bool {
    match total {
        Some(all) if all > 0 => (sent - reported) * 100 >= all,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 没检查过（或者已经装过一次）时没有可装的更新：消息告诉人先检查
    #[test]
    fn no_pending_update() {
        let slot = Mutex::new(None);
        let e = take(&slot, "0.1.13").err().unwrap();
        assert!(e.contains("检查更新"), "{e}");
    }

    // 下载进度按 1% 报一次，不会每次回调都推事件：模拟 download_and_install 的回调循环，
    // 判定走生产代码里的 should_report
    #[test]
    fn progress_threshold() {
        let (total, mut sent, mut reported, mut events) = (1000u64, 0u64, 0u64, 0);
        for _ in 0..334 {
            sent += 3;
            if !should_report(reported, sent, Some(total)) {
                continue;
            }
            reported = sent;
            events += 1;
        }
        // 一块 3 字节，攒够 1%（10 字节）才报：每 12 字节报一次，1002 字节里 83 次；不按 1% 的话是 334 次（每次回调都报）。
        // 最后一次不足 1% 的不报，下载结束时页面自己算完成
        assert_eq!(sent, 1002);
        assert_eq!(events, 83);
        assert_eq!(reported, 996, "报的最后一块是 996 字节（再往下不够 1%）");
    }

    // 第一块就超过 1%、总大小未知、总大小为 0：都不能一直不报
    #[test]
    fn progress_reports_when_total_unknown_or_tiny() {
        assert!(should_report(0, 500, Some(1000)), "第一块就超过 1%");
        assert!(should_report(0, 1, None), "不知道总大小时每次都报");
        assert!(should_report(0, 1, Some(0)), "总大小为 0 时不能除零，也不能一直不报");
        assert!(!should_report(0, 9, Some(1000)), "还差一点到 1% 的不报");
        assert!(should_report(0, 10, Some(1000)), "到了 1% 就报");
    }
}
