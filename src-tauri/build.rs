fn main() {
    // 构建时可以注入默认的服务端地址（`BOARD_DEFAULT_URL=https://board.example.com`）：不注入时没有默认地址，
    // 桌面端第一次打开显示「连接服务端」页（见 src/server.rs、docs/deploy.md）
    let default_url = std::env::var("BOARD_DEFAULT_URL").unwrap_or_default();
    println!("cargo:rustc-env=BOARD_DEFAULT_URL={}", default_url.trim());
    println!("cargo:rerun-if-env-changed=BOARD_DEFAULT_URL");
    // 窗口打开的是服务端的页面（远程地址）：自定义命令必须在 ACL 里登记，再由 capabilities 放行
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
        "desktop_login",
        "new_window",
        "window_ready",
        "set_board_env",
        "remember_theme",
        "server_current",
        "server_check",
        "server_connect",
        "host_status",
        "host_save",
        "host_enable",
        "host_autostart",
        "pick_folder",
        "mcp_local",
        "skill_install",
        "update_check",
        "update_install",
        "update_restart",
    ])))
    .expect("tauri-build 失败");
}
