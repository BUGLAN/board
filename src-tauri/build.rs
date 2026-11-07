fn main() {
    // 窗口打开的是线上页面（远程地址）：自定义命令必须在 ACL 里登记，再由 capabilities 放行
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["desktop_login", "new_window", "window_ready", "set_board_env", "remember_theme"])),
    )
    .expect("tauri-build 失败");
}
