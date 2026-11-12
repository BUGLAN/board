//! `board update [--latest]`：把单独安装的 board 原地升级（PRD §9「安装与升级」）。
//!
//! - 默认升级到服务端的版本（`GET /api/health`，不需要令牌），`--latest` 升级到最新发版。
//! - 下载发版附件里本平台的压缩包（`board-<平台>.tar.gz`，Windows 是 `.zip`）和校验文件 `board-sha256.txt`，
//!   SHA-256 对不上就中止，现有的 board 不动；解压出来的新 board 先跑一次 `--version` 核对版本号，再原地替换。
//! - 桌面端自带的 board 跟着桌面端升级：拒绝执行，提示用桌面端的「检查更新」。
//! - 本机有 `board host` 在跑时提示重启它（远程「升级」由 board host 自己执行 `board update` 后重启，不提示）。
//! - 下载地址：`https://github.com/buglan/board/releases/download/v<版本>/…`（`--latest` 用 `releases/latest/download/…`）；
//!   环境变量 `BOARD_DOWNLOAD` 换成别的地址或本机目录（镜像、测试用），和安装脚本一样。

use crate::doctor::{HttpProbe, Probe, ProbeError};
use crate::{config, Output, Paint};
use board_core::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// 发版附件所在的 GitHub Releases
pub const RELEASES: &str = "https://github.com/buglan/board/releases";
/// 校验文件：每行 `<sha256>  <文件名>`
pub const SUMS: &str = "board-sha256.txt";
/// 下载一个附件最多等多久
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// 附件最大多少（board 压缩后几 MB，留足余量）
const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// 发版附件里的平台名（和 skill 的 `bin/<平台>` 一样）；没有发版附件的平台是 None
pub fn platform_of(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("windows", "x86_64") => "windows-x64",
        _ => return None,
    })
}

/// 本平台的压缩包名
pub fn archive_name(platform: &str) -> String {
    if platform.starts_with("windows-") { format!("board-{platform}.zip") } else { format!("board-{platform}.tar.gz") }
}

/// 从校验文件里找某个文件的 SHA-256（小写十六进制）。认 `sha256sum` 的两种写法：`<值>  <名>`、`<值> *<名>`
pub fn expected_sum(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (sum, name) = l.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == file && sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit())).then(|| sum.to_ascii_lowercase())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// 这个 board 是不是桌面端自带的：macOS 在 `*.app/Contents/MacOS/` 里，其他系统和桌面端程序 `board-desktop` 放在同一个目录
pub fn bundled_with_desktop(exe: &Path) -> bool {
    let s = exe.to_string_lossy().replace('\\', "/");
    if s.contains(".app/Contents/MacOS/") {
        return true;
    }
    exe.parent().is_some_and(|d| d.join("board-desktop").is_file() || d.join("board-desktop.exe").is_file())
}

/// 下载地址的前缀：`BOARD_DOWNLOAD` 优先（网址或本机目录）；否则按版本号，没有版本号时是最新发版
fn download_base(version: Option<&str>) -> String {
    if let Some(b) = config::env("BOARD_DOWNLOAD") {
        return b.trim_end_matches('/').to_string();
    }
    match version {
        Some(v) => format!("{RELEASES}/download/v{v}"),
        None => format!("{RELEASES}/latest/download"),
    }
}

/// 取一个附件：网址用 HTTP（跟随 GitHub 的跳转），其他当成本机目录
fn fetch(base: &str, name: &str) -> Result<Vec<u8>> {
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        let path = Path::new(base).join(name);
        return std::fs::read(&path).map_err(|e| BoardError::usage(format!("读不到 {}：{e}", path.display())));
    }
    let url = format!("{base}/{name}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .http_status_as_error(false)
        .user_agent(concat!("board-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut res = agent.get(&url).call().map_err(|e| BoardError::new(ErrorCode::Unreachable, format!("下载 {url} 失败：{e}")))?;
    let status = res.status().as_u16();
    if status == 404 {
        return Err(BoardError::usage(format!("发版附件里没有 {name}（{url} 404）：这一版可能还没有单独的 board 包")));
    }
    if !(200..300).contains(&status) {
        return Err(BoardError::new(ErrorCode::Unreachable, format!("下载 {url} 失败：HTTP {status}")));
    }
    res.body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(|e| BoardError::new(ErrorCode::Unreachable, format!("下载 {url} 中断：{e}")))
}

/// 服务端的版本号（不需要令牌）。没有服务端地址时报 NOT_LOGGED_IN
fn server_version() -> Result<(String, String)> {
    let conn = config::Conn::resolve()?;
    let Some((url, _)) = conn.url else {
        return Err(BoardError::new(
            ErrorCode::NotLoggedIn,
            "不知道服务端地址：先 board login --url <服务端地址> --token <令牌>；或者用 board update --latest 升级到最新发版",
        ));
    };
    match HttpProbe::new().health(&url) {
        Ok(h) => Ok((h.version, url)),
        Err(ProbeError::Unreachable(e)) => Err(BoardError::new(ErrorCode::Unreachable, format!("连不上服务端 {url}：{e}"))),
        Err(ProbeError::Rejected { status, message }) => Err(BoardError::usage(format!("服务端 {url} 拒绝了版本查询（HTTP {status}）：{message}"))),
        Err(ProbeError::BadResponse(e)) => Err(BoardError::usage(format!("服务端 {url} 返回的版本信息看不懂：{e}"))),
        Err(e) => Err(BoardError::usage(format!("取不到服务端 {url} 的版本：{e:?}"))),
    }
}

/// 解压到 dir，返回解压出来的 board 可执行文件。Windows 10 以后自带的 tar（bsdtar）也能解 zip
fn extract(archive: &Path, dir: &Path) -> Result<PathBuf> {
    let out = Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dir)
        .output()
        .map_err(|e| BoardError::usage(format!("解压要用 tar，没法运行：{e}")))?;
    if !out.status.success() {
        return Err(BoardError::usage(format!("解压 {} 失败：{}", archive.display(), String::from_utf8_lossy(&out.stderr).trim())));
    }
    let bin = dir.join(format!("board{}", std::env::consts::EXE_SUFFIX));
    if !bin.is_file() {
        return Err(BoardError::usage(format!("压缩包里没有 board{}", std::env::consts::EXE_SUFFIX)));
    }
    Ok(bin)
}

/// 新 board 的版本号（`--version --json`）
fn version_of(bin: &Path) -> Result<String> {
    let out = Command::new(bin)
        .args(["--version", "--json"])
        .env("NO_COLOR", "1")
        .output()
        .map_err(|e| BoardError::usage(format!("新的 board 运行不了：{e}")))?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    v["version"].as_str().map(str::to_string).ok_or_else(|| BoardError::usage("新的 board 没有给出版本号，不替换"))
}

/// 用 new 原地替换 exe。macOS、Linux 直接改名覆盖（正在运行的进程不受影响）；
/// Windows 上正在运行的 exe 不能覆盖、但能改名：先把它改成 board.old.exe，再把新的放进来，下次升级时删掉旧的
fn replace(new: &Path, exe: &Path) -> Result<()> {
    let denied = |e: std::io::Error| {
        let dir = exe.parent().map(|d| d.display().to_string()).unwrap_or_default();
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            BoardError::usage(format!("没有权限写 {dir}：用安装 board 时的用户运行（装在系统目录里时用 sudo board update）"))
        } else {
            BoardError::usage(format!("替换 {} 失败：{e}", exe.display()))
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755)).map_err(denied)?;
        std::fs::rename(new, exe).map_err(denied)
    }
    #[cfg(not(unix))]
    {
        let old = old_path(exe);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old).map_err(denied)?;
        if let Err(e) = std::fs::rename(new, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(denied(e));
        }
        Ok(())
    }
}

/// Windows 上替换下来的旧版本
#[cfg_attr(unix, allow(dead_code))]
fn old_path(exe: &Path) -> PathBuf {
    exe.with_file_name(format!("board.old{}", std::env::consts::EXE_SUFFIX))
}

/// 本机还在跑、用的就是 exe 这个文件的 board host（不含自己和启动自己的那个）：进程号。
/// 要在替换之前查：替换之后旧进程的可执行文件已经不是这个路径了
fn running_hosts(exe: &Path) -> Vec<u32> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let Ok(mut sys) = System::new() else { return vec![] };
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always).with_exe(UpdateKind::Always),
    );
    let me = std::process::id();
    let parent = sys.process(sysinfo::Pid::from_u32(me)).and_then(|p| p.parent()).map(|p| p.as_u32());
    let mut out: Vec<u32> = sys
        .processes()
        .iter()
        .filter(|(pid, _)| pid.as_u32() != me && Some(pid.as_u32()) != parent)
        .filter(|(_, p)| p.exe().is_some_and(|e| std::fs::canonicalize(e).map(|e| e == exe).unwrap_or(e == exe)))
        .filter(|(_, p)| p.cmd().iter().skip(1).map(|s| s.to_string_lossy()).find(|a| !a.starts_with('-')).is_some_and(|a| a == "host"))
        .map(|(pid, _)| pid.as_u32())
        .collect();
    out.sort();
    out
}

pub fn run(latest: bool, json: bool, p: &Paint) -> Result<Output> {
    let current = env!("CARGO_PKG_VERSION");
    let exe = std::env::current_exe().map_err(|e| BoardError::usage(format!("找不到正在运行的 board：{e}")))?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    if bundled_with_desktop(&exe) {
        return Err(BoardError::usage(format!(
            "这个 board 是桌面端自带的（{}），跟着桌面端一起升级：在桌面端点「检查更新」。board update 只升级单独安装的 board",
            exe.display()
        )));
    }
    #[cfg(not(unix))]
    let _ = std::fs::remove_file(old_path(&exe));
    let platform = platform_of(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        BoardError::usage(format!("发版附件里没有这个平台（{} {}）的 board：用 cargo install 从源码装", std::env::consts::OS, std::env::consts::ARCH))
    })?;
    let (target, server) = if latest {
        (None, None)
    } else {
        let (v, url) = server_version()?;
        (Some(v), Some(url))
    };
    if target.as_deref() == Some(current) {
        let msg = format!("已经是 {current}，和服务端一致，不用升级");
        return Ok(if json {
            Output::Json(json!({ "ok": true, "updated": false, "version": current, "path": exe.display().to_string() }))
        } else {
            Output::Text(format!("{} {msg}\n", p.green("✓")))
        });
    }

    let base = download_base(target.as_deref());
    let archive = archive_name(platform);
    let sums = String::from_utf8_lossy(&fetch(&base, SUMS)?).into_owned();
    let want = expected_sum(&sums, &archive).ok_or_else(|| BoardError::usage(format!("校验文件 {SUMS} 里没有 {archive}，不替换现在的 board")))?;
    let bytes = fetch(&base, &archive)?;
    let got = sha256_hex(&bytes);
    if got != want {
        return Err(BoardError::usage(format!(
            "{archive} 的 SHA-256 对不上（应为 {want}，下载到的是 {got}）：下载不完整或被改过，没有替换现在的 board"
        )));
    }

    // 解压在 board 所在目录里（同一个文件系统，替换时是一次改名）
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    let work = dir.join(format!(".board-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            BoardError::usage(format!("没有权限写 {}：用安装 board 时的用户运行（装在系统目录里时用 sudo board update）", dir.display()))
        } else {
            BoardError::usage(format!("建不了临时目录 {}：{e}", work.display()))
        }
    })?;
    let hosts = if std::env::var_os("BOARD_UPDATE_BY_HOST").is_some() { vec![] } else { running_hosts(&exe) };
    let result = (|| -> Result<String> {
        let file = work.join(&archive);
        std::fs::write(&file, &bytes).map_err(|e| BoardError::usage(format!("写不了 {}：{e}", file.display())))?;
        let new = extract(&file, &work)?;
        let got = version_of(&new)?;
        if let Some(t) = &target {
            if &got != t {
                return Err(BoardError::usage(format!("下载到的 board 是 {got}，不是要升级到的 {t}，不替换")));
            }
        }
        if got != current {
            replace(&new, &exe)?;
        }
        Ok(got)
    })();
    let _ = std::fs::remove_dir_all(&work);
    let to = result?;

    let updated = to != current;
    if json {
        return Ok(Output::Json(json!({
            "ok": true,
            "updated": updated,
            "from": current,
            "version": to,
            "path": exe.display().to_string(),
            "server": server,
            "hosts": hosts,
        })));
    }
    let mut s = if updated {
        format!("{} 已从 {current} 升级到 {}：{}\n", p.green("✓"), p.bold(&to), exe.display())
    } else {
        format!("{} 已经是最新发版 {to}，不用升级\n", p.green("✓"))
    };
    if updated && !hosts.is_empty() {
        let ids = hosts.iter().map(u32::to_string).collect::<Vec<_>>().join("、");
        s.push_str(&format!(
            "{} 本机有 board host 在跑（进程 {ids}），还是旧版本：重启它（Ctrl+C 后重新运行）才会用上 {to}；也可以在界面上机器详情里点「重启」\n",
            p.yellow("!")
        ));
    }
    Ok(Output::Text(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platforms_with_release_archives() {
        assert_eq!(platform_of("macos", "aarch64"), Some("darwin-arm64"));
        assert_eq!(platform_of("macos", "x86_64"), Some("darwin-x64"));
        assert_eq!(platform_of("linux", "x86_64"), Some("linux-x64"));
        assert_eq!(platform_of("linux", "aarch64"), Some("linux-arm64"));
        assert_eq!(platform_of("windows", "x86_64"), Some("windows-x64"));
        assert_eq!(platform_of("windows", "aarch64"), None);
        assert_eq!(platform_of("freebsd", "x86_64"), None);
        assert_eq!(archive_name("linux-x64"), "board-linux-x64.tar.gz");
        assert_eq!(archive_name("windows-x64"), "board-windows-x64.zip");
    }

    #[test]
    fn checksum_lines() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a}  board-linux-x64.tar.gz\n{b} *board-windows-x64.zip\nbad  board-darwin-arm64.tar.gz\n");
        assert_eq!(expected_sum(&sums, "board-linux-x64.tar.gz"), Some(a));
        assert_eq!(expected_sum(&sums, "board-windows-x64.zip"), Some("b".repeat(64)));
        assert_eq!(expected_sum(&sums, "board-darwin-arm64.tar.gz"), None);
        assert_eq!(expected_sum(&sums, "board-linux-arm64.tar.gz"), None);
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn desktop_bundled_board_is_recognized() {
        assert!(bundled_with_desktop(Path::new("/Applications/Board.app/Contents/MacOS/board")));
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("board");
        std::fs::write(&exe, b"").unwrap();
        assert!(!bundled_with_desktop(&exe));
        std::fs::write(d.path().join("board-desktop.exe"), b"").unwrap();
        assert!(bundled_with_desktop(&exe));
    }
}
