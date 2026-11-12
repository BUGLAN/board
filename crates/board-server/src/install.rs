//! `GET /install.sh`、`GET /install.ps1`：命令行版 board 的安装脚本（PRD §9「安装与升级」），不需要登录。
//!
//! 脚本在仓库的 `scripts/install.sh`、`scripts/install.ps1`，编译时打包进来；返回前把占位符换成：
//! - `@BOARD_VERSION@`：服务端自己的版本号（装的 board 和服务端一致）
//! - `@BOARD_SERVER@`：服务端对外的地址（`board login --url` 用）：GitHub 登录时是 `BOARD_PUBLIC_URL`，
//!   本机模式按请求的 Host（经过反向代理时看 `X-Forwarded-Proto`、`X-Forwarded-Host`）
//! - `@BOARD_DOWNLOAD@`：发版附件的地址，默认 `https://github.com/buglan/board/releases/download/v<版本>`；
//!   服务端的环境变量 `BOARD_DOWNLOAD` 可以换成镜像

use crate::auth::Auth;
use crate::AppState;
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

const SH: &str = include_str!("../../../scripts/install.sh");
const PS1: &str = include_str!("../../../scripts/install.ps1");

/// 发版附件所在的 GitHub Releases（和 board update 一样）
const RELEASES: &str = "https://github.com/buglan/board/releases";

/// 写进脚本的值只能是这些字符：脚本里用单引号包着，不能让请求头带进引号、换行、$ 之类
fn safe(v: &str) -> bool {
    !v.is_empty() && v.len() <= 512 && v.chars().all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!&()*+,;=%".contains(c))
}

/// 服务端对外的地址
pub fn public_url(auth: &Auth, h: &HeaderMap) -> Option<String> {
    if let Auth::GitHub(gh) = auth {
        if !gh.public_url.is_empty() {
            return Some(gh.public_url.trim_end_matches('/').to_string());
        }
    }
    let first = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).map(|s| s.split(',').next().unwrap_or_default().trim().to_string());
    let host = first("x-forwarded-host").filter(|s| !s.is_empty()).or_else(|| first("host"))?;
    let proto = first("x-forwarded-proto").filter(|p| p == "https" || p == "http").unwrap_or_else(|| "http".into());
    Some(format!("{proto}://{host}"))
}

/// 发版附件的地址
pub fn download_base(version: &str) -> String {
    match std::env::var("BOARD_DOWNLOAD").ok().map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty()) {
        Some(b) => b,
        None => format!("{RELEASES}/download/v{version}"),
    }
}

/// 换好占位符的脚本；地址里有不该有的字符时 None
pub fn render(script: &str, version: &str, server: &str, download: &str) -> Option<String> {
    if !(safe(version) && safe(server) && safe(download)) {
        return None;
    }
    Some(script.replace("@BOARD_VERSION@", version).replace("@BOARD_SERVER@", server).replace("@BOARD_DOWNLOAD@", download))
}

fn serve(s: &AppState, h: &HeaderMap, script: &str) -> Response {
    let version = crate::VERSION;
    let Some(server) = public_url(&s.0.auth, h) else {
        return (StatusCode::BAD_REQUEST, "请求里没有 Host，生成不了安装脚本").into_response();
    };
    match render(script, version, &server, &download_base(version)) {
        Some(body) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], body).into_response(),
        None => (StatusCode::BAD_REQUEST, "服务端地址或下载地址里有不支持的字符，生成不了安装脚本").into_response(),
    }
}

pub async fn sh(State(s): State<AppState>, h: HeaderMap) -> Response {
    serve(&s, &h, SH)
}

pub async fn ps1(State(s): State<AppState>, h: HeaderMap) -> Response {
    serve(&s, &h, PS1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::process::Command;

    #[test]
    fn placeholders_are_filled_and_unsafe_values_refused() {
        let out = render(SH, "1.2.3", "https://board.example.com", "https://github.com/buglan/board/releases/download/v1.2.3").unwrap();
        assert!(out.contains("BOARD_VERSION='1.2.3'"));
        assert!(out.contains("BOARD_SERVER='https://board.example.com'"));
        assert!(out.contains(":-https://github.com/buglan/board/releases/download/v1.2.3}"));
        assert!(!out.contains("@BOARD_VERSION@") && !out.contains("@BOARD_SERVER@") && !out.contains("@BOARD_DOWNLOAD@"));
        let ps = render(PS1, "1.2.3", "http://127.0.0.1:7420", "https://x/y").unwrap();
        assert!(ps.contains("$BoardServer = 'http://127.0.0.1:7420'"));
        assert!(render(SH, "1.2.3", "http://evil'; rm -rf ~; '", "https://x").is_none());
        assert!(render(SH, "1.2.3", "http://a\nb", "https://x").is_none());
        assert!(render(SH, "1.2.3", "http://$(id)", "https://x").is_none());
    }

    #[test]
    fn public_url_from_proxy_headers() {
        let mut h = HeaderMap::new();
        h.insert("host", HeaderValue::from_static("127.0.0.1:7420"));
        assert_eq!(public_url(&Auth::Local, &h).as_deref(), Some("http://127.0.0.1:7420"));
        h.insert("x-forwarded-host", HeaderValue::from_static("board.example.com"));
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert_eq!(public_url(&Auth::Local, &h).as_deref(), Some("https://board.example.com"));
        assert_eq!(public_url(&Auth::Local, &HeaderMap::new()), None);
    }

    /// 安装脚本的平台判断（sh 的 --platform-of 只算平台名）
    #[test]
    fn install_sh_platforms() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("install.sh");
        std::fs::write(&f, SH).unwrap();
        let plat = |os: &str, arch: &str| {
            let out = Command::new("sh").arg(&f).args(["--platform-of", os, arch]).output().unwrap();
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        };
        assert_eq!(plat("Darwin", "arm64").as_deref(), Some("darwin-arm64"));
        assert_eq!(plat("Darwin", "x86_64").as_deref(), Some("darwin-x64"));
        assert_eq!(plat("Linux", "x86_64").as_deref(), Some("linux-x64"));
        assert_eq!(plat("Linux", "amd64").as_deref(), Some("linux-x64"));
        assert_eq!(plat("Linux", "aarch64").as_deref(), Some("linux-arm64"));
        assert_eq!(plat("Linux", "armv7l"), None);
        assert_eq!(plat("FreeBSD", "x86_64"), None);
        // 没有经过服务端（占位符还在）：拒绝安装
        let out = Command::new("sh").arg(&f).output().unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("没有经过服务端"));
    }

    /// 校验文件对不上：中止，现有的 board 不动
    #[test]
    fn install_sh_refuses_bad_checksum() {
        let d = tempfile::tempdir().unwrap();
        let assets = d.path().join("assets");
        let bin = d.path().join("bin");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("board"), "old").unwrap();
        let plat = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => "darwin-arm64",
            ("macos", _) => "darwin-x64",
            ("linux", "aarch64") => "linux-arm64",
            ("linux", _) => "linux-x64",
            _ => return,
        };
        let archive = format!("board-{plat}.tar.gz");
        std::fs::write(assets.join(&archive), "not really a tarball").unwrap();
        std::fs::write(assets.join("board-sha256.txt"), format!("{}  {archive}\n", "0".repeat(64))).unwrap();
        let script = render(SH, "9.9.9", "http://127.0.0.1:1", "unused").unwrap();
        let f = d.path().join("install.sh");
        std::fs::write(&f, script).unwrap();
        let out = Command::new("sh")
            .arg(&f)
            .env("BOARD_DOWNLOAD", &assets)
            .env("BOARD_INSTALL_DIR", &bin)
            .env("HOME", d.path())
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("SHA-256 对不上"), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(std::fs::read_to_string(bin.join("board")).unwrap(), "old");
    }
}
