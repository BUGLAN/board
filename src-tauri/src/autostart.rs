//! 「设置 → 本机」的「开机时自动启动」（PRD §10.6）：登录系统时在后台启动桌面端。
//!
//! - 带 `--autostart` 启动：「让这台电脑接任务」开着时不弹窗口，只显示托盘 / Dock 图标，照常运行 board host；
//!   没开时照常打开窗口。
//! - macOS：launchd 用户代理 `~/Library/LaunchAgents/dev.board.desktop.plist`，登录时执行一次
//!   `open -g -a <Board.app> --args --autostart`（经 LaunchServices 启动，已经开着时不会多开一个）。
//! - Windows：注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的「Board」一项。
//! - Linux：`~/.config/autostart/board.desktop`。
//! - 打开「让这台电脑接任务」时默认一起打开（host.rs 的 host_enable），可以单独关掉。

#[cfg(not(target_os = "windows"))]
use board_core::service::{self as svc, LAUNCHD_DESKTOP};
#[cfg(not(target_os = "windows"))]
use std::path::PathBuf;

/// 开机自启时带的参数
pub const FLAG: &str = "--autostart";

/// 这一次是开机自启起来的
pub fn launched() -> bool {
    std::env::args().any(|a| a == FLAG)
}

#[cfg(not(target_os = "windows"))]
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

/// 登录时执行的命令。macOS 在 .app 里时经 `open -g -a` 启动，其他情况直接执行桌面端程序
fn launch_args() -> Result<Vec<String>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("找不到桌面端程序：{e}"))?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    if cfg!(target_os = "macos") {
        if let Some(app) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) {
            return Ok(["/usr/bin/open", "-g", "-a", &app.display().to_string(), "--args", FLAG].iter().map(|s| s.to_string()).collect());
        }
    }
    Ok(vec![exe.display().to_string(), FLAG.to_string()])
}

#[cfg(target_os = "macos")]
fn file() -> Option<PathBuf> {
    home().map(|h| h.join("Library/LaunchAgents").join(format!("{LAUNCHD_DESKTOP}.plist")))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home().map(|h| h.join(".config")))?;
    Some(base.join("autostart").join("board.desktop"))
}

#[cfg(target_os = "windows")]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(target_os = "windows")]
fn reg(args: &[&str]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg").args(args).creation_flags(0x0800_0000).output().map_err(|e| format!("没法运行 reg：{e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 开机自启开着吗
pub fn enabled() -> bool {
    #[cfg(target_os = "windows")]
    {
        reg(&["query", RUN_KEY, "/v", "Board"]).is_ok()
    }
    #[cfg(not(target_os = "windows"))]
    {
        file().is_some_and(|f| f.is_file())
    }
}

/// 打开 / 关掉开机自启
pub fn set(on: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if on {
            let args = launch_args()?;
            let data = format!("\"{}\" {}", args[0], args[1..].join(" "));
            reg(&["add", RUN_KEY, "/v", "Board", "/t", "REG_SZ", "/d", &data, "/f"]).map_err(|e| format!("写注册表失败：{e}"))?;
        } else if enabled() {
            reg(&["delete", RUN_KEY, "/v", "Board", "/f"]).map_err(|e| format!("改注册表失败：{e}"))?;
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let f = file().ok_or("找不到家目录")?;
        if !on {
            return match std::fs::remove_file(&f) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("删不掉 {}：{e}", f.display())),
                _ => Ok(()),
            };
        }
        let args = launch_args()?;
        let text = if cfg!(target_os = "macos") { svc::launchd_plist(LAUNCHD_DESKTOP, &args, &[], false, None) } else { svc::desktop_entry("Board", &args) };
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("无法创建 {}：{e}", d.display()))?;
        }
        std::fs::write(&f, text).map_err(|e| format!("写不了 {}：{e}", f.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_carry_the_flag() {
        let a = launch_args().unwrap();
        assert_eq!(a.last().map(String::as_str), Some(FLAG));
        // 测试程序不在 .app 里：直接执行自己
        assert!(!a[0].ends_with("/usr/bin/open"));
    }
}
