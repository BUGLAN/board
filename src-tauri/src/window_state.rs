//! 记住窗口大小、位置和最大化状态：关闭时写盘，下次启动恢复。
//!
//! - 每次窗口关闭都写一次，多个窗口时以最后关闭的为准
//! - 保存的是物理像素，恢复时先看位置是否还在某块显示器上；拔掉外接屏后回到主屏居中
//! - 首次启动（没有保存过的状态）窗口不超过当前屏幕的工作区
//!
//! 状态文件在 app config 目录，例如 Windows 的 %APPDATA%\dev.board.desktop\window-state.txt。
//! 不用 serde：一行空格分隔的 x y 宽 高 最大化（0/1），坏了就当没有。

use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent};

const FILE: &str = "window-state.txt";
/// 默认窗口大小（逻辑像素）；首次启动时会被主屏工作区限制
const DEFAULT_W: f64 = 1360.0;
const DEFAULT_H: f64 = 860.0;
/// 与 tauri.conf.json / 页面一致的最小窗口
const MIN_W: f64 = 380.0;
const MIN_H: f64 = 560.0;

#[derive(Clone, Copy, Debug)]
pub struct Saved {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(FILE))
}


/// 读上次关窗时保存的状态；没有或内容不对时返回 None
pub fn load(app: &AppHandle) -> Option<Saved> {
    parse(&fs::read_to_string(path(app)?).ok()?)
}

/// 状态文件内容 → Saved；格式不对、宽高为 0 时返回 None（当作首次启动）
fn parse(text: &str) -> Option<Saved> {
    let v: Vec<&str> = text.split_whitespace().collect();
    if v.len() != 5 {
        return None;
    }
    let width = v[2].parse::<u32>().ok()?;
    let height = v[3].parse::<u32>().ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    Some(Saved {
        x: v[0].parse().ok()?,
        y: v[1].parse().ok()?,
        width,
        height,
        maximized: v[4] == "1",
    })
}

fn render(saved: Saved) -> String {
    format!("{} {} {} {} {}", saved.x, saved.y, saved.width, saved.height, if saved.maximized { 1 } else { 0 })
}

/// 窗口关闭时调用：把当前大小、位置、是否最大化写到状态文件
pub fn save(window: &WebviewWindow) {
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else { return };
    let maximized = window.is_maximized().unwrap_or(false);
    let Some(file) = path(&window.app_handle()) else { return };
    if let Some(dir) = file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let state = render(Saved { x: pos.x, y: pos.y, width: size.width, height: size.height, maximized });
    let _ = fs::write(file, state);
}

/// 首次启动时给窗口的初始大小：默认大小不超过主屏工作区（换算成逻辑像素给 builder）
pub fn initial_size(app: &AppHandle) -> Option<(f64, f64)> {
    let m = app.primary_monitor().ok().flatten()?;
    let area = *m.work_area();
    let scale = m.scale_factor();
    let aw = area.size.width as f64 / scale;
    let ah = area.size.height as f64 / scale;
    Some(clamp_size(aw, ah))
}

/// 默认窗口大小限制在屏幕工作区内；工作区比最小窗口还小时以工作区为准
fn clamp_size(aw: f64, ah: f64) -> (f64, f64) {
    (DEFAULT_W.min(aw).max(MIN_W.min(aw)), DEFAULT_H.min(ah).max(MIN_H.min(ah)))
}

/// 恢复上次的位置和大小；位置已经不在任何显示器上时回到主屏居中。
/// 窗口刚创建时系统还会应用一次初始尺寸，稍等再设置，免得被覆盖。
pub fn apply(window: &WebviewWindow, saved: Saved) {
    let window = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        apply_now(&window, saved);
    });
}

fn apply_now(window: &WebviewWindow, saved: Saved) {
    let monitors = window.available_monitors().unwrap_or_default();
    if monitors.iter().any(|m| overlaps(saved, m.position().x, m.position().y, m.size().width, m.size().height)) {
        let _ = window.set_size(PhysicalSize::new(saved.width, saved.height));
        let _ = window.set_position(PhysicalPosition::new(saved.x, saved.y));
        if saved.maximized {
            let _ = window.maximize();
        }
        return;
    }
    center(window, saved.width, saved.height);
}

/// 主屏居中；大小不超过工作区
fn center(window: &WebviewWindow, width: u32, height: u32) {
    let Some(m) = window.primary_monitor().ok().flatten() else { return };
    let area = *m.work_area();
    let w = width.min(area.size.width);
    let h = height.min(area.size.height);
    let x = area.position.x + (area.size.width.saturating_sub(w) / 2) as i32;
    let y = area.position.y + (area.size.height.saturating_sub(h) / 2) as i32;
    let _ = window.set_size(PhysicalSize::new(w, h));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// 保存的窗口矩形（物理像素）是否和某块显示器有交集
fn overlaps(saved: Saved, mx: i32, my: i32, mw: u32, mh: u32) -> bool {
    let (mx2, my2) = (mx.saturating_add(mw as i32), my.saturating_add(mh as i32));
    saved.x < mx2 && saved.x.saturating_add(saved.width as i32) > mx && saved.y < my2 && saved.y.saturating_add(saved.height as i32) > my
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_render_roundtrip() {
        let s = Saved { x: -1200, y: 40, width: 1360, height: 860, maximized: true };
        assert_eq!(parse(&render(s)).map(|v| (v.x, v.y, v.width, v.height, v.maximized)), Some((-1200, 40, 1360, 860, true)));
        assert_eq!(parse("0 0 1360 860 0").map(|v| v.maximized), Some(false));
        // 坏文件当作首次启动
        for bad in ["", "1 2 3", "0 0 0 860 0", "0 0 1360 x 0", "a b c d e"] {
            assert!(parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn first_launch_fits_work_area() {
        // 普通屏幕：默认 1360×860 原样
        assert_eq!(clamp_size(1920.0, 1040.0), (1360.0, 860.0));
        // 小屏（1366×768、150% 缩放后逻辑 911×512）：不超过工作区
        let (w, h) = clamp_size(911.0, 512.0);
        assert_eq!((w, h), (911.0, 512.0));
        assert!(w <= 911.0 && h <= 512.0);
        // 工作区比最小窗口还小：以工作区为准
        assert_eq!(clamp_size(300.0, 400.0), (300.0, 400.0));
    }

    #[test]
    fn overlaps_checks_every_monitor() {
        // 笔记本主屏 1920×1080 + 右侧外接屏 2560×1440
        assert!(overlaps(Saved { x: 100, y: 100, width: 800, height: 600, maximized: false }, 0, 0, 1920, 1080));
        assert!(overlaps(Saved { x: 2500, y: 200, width: 800, height: 600, maximized: false }, 1920, 0, 2560, 1440));
        // 外接屏拔掉后，保存的位置不在任何显示器上
        assert!(!overlaps(Saved { x: 2500, y: 200, width: 800, height: 600, maximized: false }, 0, 0, 1920, 1080));
        assert!(!overlaps(Saved { x: 1920, y: 0, width: 800, height: 600, maximized: false }, 0, 0, 1920, 1080));
        // 半截在外面也算还在屏幕上
        assert!(overlaps(Saved { x: 1800, y: 900, width: 800, height: 600, maximized: false }, 0, 0, 1920, 1080));
        // 上方负坐标（显示器在主屏上方）
        assert!(overlaps(Saved { x: 0, y: -900, width: 800, height: 600, maximized: false }, 0, -1080, 1920, 1080));
    }
}

/// 给窗口注册关闭时保存状态的处理器（多个窗口时各自保存，最后的为准）
pub fn remember_on_close(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::CloseRequested { .. }) {
            if let Some(w) = app.get_webview_window(&label) {
                save(&w);
            }
        }
    });
}
