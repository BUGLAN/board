fn main() {
    // 构建时可以注入默认的服务端地址（`BOARD_DEFAULT_URL=https://board.example.com`）：不注入时没有默认地址，
    // 桌面端第一次打开显示「连接服务端」页（见 src/server.rs、docs/deploy.md）
    let default_url = std::env::var("BOARD_DEFAULT_URL").unwrap_or_default();
    println!("cargo:rustc-env=BOARD_DEFAULT_URL={}", default_url.trim());
    println!("cargo:rerun-if-env-changed=BOARD_DEFAULT_URL");

    // 安装包自带的 board 命令行（bundle.externalBin）由 scripts/build-sidecar.mjs 编好放进 src-tauri/binaries，
    // 那个目录不进 git。`cargo test -p board-desktop`、`cargo build` 用不到它，没有时按「不带 sidecar」构建，
    // 否则 tauri-build 直接报 resource path ... doesn't exist，新克隆的仓库、CI 里连测试都跑不起来。
    // 打安装包（`npm run desktop:build`）和 `desktop:dev` 都会先跑 build-sidecar.mjs，那时文件在，照常带上。
    let sidecar = sidecar_path();
    println!("cargo:rerun-if-changed={}", sidecar.display());
    if !sidecar.is_file() {
        println!(
            "cargo:warning=没有 {}，这次构建不带自带的 board 命令行（桌面端测试不需要它；打安装包前先 node scripts/build-sidecar.mjs）",
            sidecar.display()
        );
        std::env::set_var("TAURI_CONFIG", without_sidecar(std::env::var("TAURI_CONFIG").ok()));
    }

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

/// `bundle.externalBin` 指向的文件：Tauri 只认带目标三元组的名字（board-<triple>[.exe]）
fn sidecar_path() -> std::path::PathBuf {
    let triple = std::env::var("TARGET").expect("cargo 会设置 TARGET");
    let ext = if triple.contains("windows") { ".exe" } else { "" };
    std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("cargo 会设置 CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(format!("board-{triple}{ext}"))
}

/// 在已有的 `TAURI_CONFIG`（tauri-build 会把它合并到 tauri.conf.json 上）上加一句「不带 sidecar」
fn without_sidecar(current: Option<String>) -> String {
    let mut config: serde_json::Value = current
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    config["bundle"]["externalBin"] = serde_json::json!([]);
    config.to_string()
}
