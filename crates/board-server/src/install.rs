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
    // 下面几个用例真的跑一遍 install.sh（需要 sh）
    #[cfg(unix)]
    use std::process::Stdio;

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
    /// install.sh 只支持 darwin、linux：Windows 上（包括 Git Bash 里 uname 报 MINGW64）脚本自己就会拒绝，所以只在 unix 上跑
    #[cfg(unix)]
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

    #[test]
    fn shell_scripts_checkout_as_lf() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = Command::new("git").arg("-C").arg(&root).args(["ls-files", "--eol", "*.sh"]).output().unwrap();
        assert!(out.status.success(), "git ls-files --eol 失败：{}", String::from_utf8_lossy(&out.stderr));
        let output = String::from_utf8_lossy(&out.stdout);
        assert!(!output.is_empty(), "没查到 shell 脚本");
        for line in output.lines() {
            assert!(line.contains("attr/text eol=lf"), "shell 脚本没有按 LF 检出：{line}");
        }
    }

    /// `curl ... | sh -s -- --help` 时脚本从标准输入运行，帮助文本不能再从 `$0` 读。
    #[cfg(unix)]
    #[test]
    fn install_sh_help_works_from_stdin() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("install.sh");
        std::fs::write(&f, SH).unwrap();
        let out = Command::new("sh")
            .args(["-s", "--", "--help"])
            .stdin(Stdio::from(std::fs::File::open(&f).unwrap()))
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "stderr:\n{}", String::from_utf8_lossy(&out.stderr));
        assert!(text.contains("--dir <项目>=<路径>"), "{text}");
        assert!(text.contains("每个项目的目录是 <工作空间>/<项目>"), "{text}");
        assert!(!text.contains("<项目>/<白板>="), "{text}");
    }

    #[cfg(unix)]
    fn host_platform() -> &'static str {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => "darwin-arm64",
            ("macos", _) => "darwin-x64",
            ("linux", "aarch64") => "linux-arm64",
            ("linux", _) => "linux-x64",
            (os, arch) => unreachable!("install.sh 只支持 darwin、linux：{os} {arch}"),
        }
    }

    #[cfg(unix)]
    fn sha256(path: &std::path::Path) -> String {
        let out = Command::new("sh")
            .args(["-c", "sha256sum \"$1\" 2>/dev/null || shasum -a 256 \"$1\"", "sh"])
            .arg(path)
            .output()
            .unwrap();
        assert!(out.status.success(), "算 SHA-256 失败：{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap().to_lowercase()
    }

    /// 在 noexec 的临时目录里直接运行下载的 board 会失败，所以版本核对要在安装目录里的临时文件上做。
    #[cfg(unix)]
    #[test]
    fn install_sh_verifies_binary_in_install_dir() {
        use std::os::unix::fs::PermissionsExt;

        let d = tempfile::tempdir().unwrap();
        let assets = d.path().join("assets");
        let bin = d.path().join("bin");
        let unpacked = d.path().join("unpacked");
        let log = d.path().join("calls.log");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(&unpacked).unwrap();

        let board = unpacked.join("board");
        std::fs::write(&board, "#!/bin/sh\nprintf '%s\\n' \"$0\" >> \"$BOARD_TEST_LOG\"\nprintf 'board 9.9.9\\n'\n").unwrap();
        std::fs::set_permissions(&board, std::fs::Permissions::from_mode(0o755)).unwrap();
        let platform = host_platform();
        let archive = format!("board-{platform}.tar.gz");
        let status = Command::new("tar")
            .current_dir(&unpacked)
            .args(["-czf"])
            .arg(assets.join(&archive))
            .arg("board")
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::write(assets.join("board-sha256.txt"), format!("{}  {archive}\n", sha256(&assets.join(&archive)))).unwrap();

        let f = d.path().join("install.sh");
        std::fs::write(&f, render(SH, "9.9.9", "http://127.0.0.1:1", "unused").unwrap()).unwrap();
        let out = Command::new("sh")
            .arg(&f)
            .env("BOARD_DOWNLOAD", &assets)
            .env("BOARD_INSTALL_DIR", &bin)
            .env("BOARD_TEST_LOG", &log)
            .env("HOME", d.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        let calls: Vec<_> = std::fs::read_to_string(&log).unwrap().lines().map(str::to_string).collect();
        assert_eq!(calls.len(), 2, "{calls:?}");
        let staged = std::path::Path::new(&calls[0]);
        assert_eq!(staged.parent(), Some(bin.as_path()), "第一次 --version 没有在安装目录里执行：{calls:?}");
        assert!(staged.file_name().unwrap().to_string_lossy().starts_with(".board.new."), "{calls:?}");
        assert_eq!(std::path::Path::new(&calls[1]), bin.join("board"), "{calls:?}");
    }

    /// 脚本里后面紧跟非 ASCII 字符的 `$VAR`：(行号, 变量名)
    fn vars_before_non_ascii(text: &str) -> Vec<(usize, String)> {
        let mut bad = vec![];
        for (i, line) in text.lines().enumerate() {
            let b = line.as_bytes();
            let mut k = 0;
            while k < b.len() {
                if b[k] == b'$' && b.get(k + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') {
                    let mut e = k + 1;
                    while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
                        e += 1;
                    }
                    if e < b.len() && !b[e].is_ascii() {
                        bad.push((i + 1, line[k + 1..e].to_string()));
                    }
                    k = e;
                } else {
                    k += 1;
                }
            }
        }
        bad
    }

    /// shell 脚本：`.sh` 结尾，或第一行是 sh、bash 这类的 shebang（skill 的 `scripts/board` 没有扩展名）
    fn is_shell_script(path: &str, data: &[u8]) -> bool {
        if path.ends_with(".sh") {
            return true;
        }
        let Some(first) = data.strip_prefix(b"#!") else { return false };
        let first = String::from_utf8_lossy(first.split(|c| *c == b'\n').next().unwrap_or_default());
        let mut words = first.split_whitespace();
        let mut prog = words.next().unwrap_or_default().rsplit('/').next().unwrap_or_default();
        if prog == "env" {
            prog = words.find(|w| !w.starts_with('-')).unwrap_or_default();
        }
        ["sh", "bash", "dash", "ksh", "zsh"].contains(&prog)
    }

    /// 变量名后面紧跟全角标点、中文时要写成 `${VAR}`：macOS 的 /bin/sh（bash 3.2）在 UTF-8 下会把后面那个字节
    /// 算进变量名，`set -u` 下直接报 unbound variable，脚本在第一句提示就退出（卡片 #378）。
    /// 查 git 跟踪的所有 shell 脚本：install.sh、deploy/ 下的部署脚本、skill 的脚本（卡片 #383）
    #[test]
    fn shell_scripts_brace_vars_before_non_ascii() {
        assert_eq!(vars_before_non_ascii("a $X（b）${Y}（c）$Z d $W"), vec![(1, "X".to_string())]);
        assert!(is_shell_script("x/board", b"#!/usr/bin/env bash\n") && is_shell_script("deploy/a.sh", b""));
        assert!(!is_shell_script("scripts/a.mjs", b"#!/usr/bin/env node\n") && !is_shell_script("a.png", b"\x89PNG"));

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = std::process::Command::new("git").arg("-C").arg(&root).args(["ls-files", "-z"]).output().expect("要用 git 列出仓库里的文件");
        assert!(out.status.success(), "git ls-files 失败：{}", String::from_utf8_lossy(&out.stderr));
        let mut scripts = vec![];
        let mut bad = vec![];
        for rel in String::from_utf8_lossy(&out.stdout).split('\0').filter(|s| !s.is_empty()) {
            // 删了还没提交的文件读不到，跳过
            let Ok(data) = std::fs::read(root.join(rel)) else { continue };
            if !is_shell_script(rel, &data) {
                continue;
            }
            for (n, var) in vars_before_non_ascii(&String::from_utf8_lossy(&data)) {
                bad.push(format!("{rel} 第 {n} 行的 ${var} 后面紧跟非 ASCII 字符，写成 ${{{var}}}"));
            }
            scripts.push(rel.to_string());
        }
        for must in ["scripts/install.sh", "deploy/backup.sh", "deploy/build-linux.sh", ".agents/skills/board-skills/scripts/board"] {
            assert!(scripts.iter().any(|s| s == must), "没查到 {must}，查了：{scripts:?}");
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    /// 校验文件对不上：中止，现有的 board 不动（同上面：脚本只支持 darwin、linux）
    #[cfg(unix)]
    #[test]
    fn install_sh_refuses_bad_checksum() {
        let d = tempfile::tempdir().unwrap();
        let assets = d.path().join("assets");
        let bin = d.path().join("bin");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("board"), "old").unwrap();
        // 平台名要和本机一致：脚本按 uname 找发版附件
        let plat = host_platform();
        let archive = format!("board-{plat}.tar.gz");
        std::fs::write(assets.join(&archive), "not really a tarball").unwrap();
        std::fs::write(assets.join("board-sha256.txt"), format!("{}  {archive}\n", "0".repeat(64))).unwrap();
        let script = render(SH, "9.9.9", "http://127.0.0.1:1", "unused").unwrap();
        let f = d.path().join("install.sh");
        std::fs::write(&f, script).unwrap();
        let out = Command::new("sh")
            .arg(&f)
            // UTF-8 下跑：macOS 的 /bin/sh（bash 3.2）这时才会把变量后面的全角字符算进变量名（卡片 #378）
            .env("LC_ALL", "en_US.UTF-8")
            .env("BOARD_DOWNLOAD", &assets)
            .env("BOARD_INSTALL_DIR", &bin)
            .env("HOME", d.path())
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("SHA-256 对不上"), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(std::fs::read_to_string(bin.join("board")).unwrap(), "old");
    }

    /// 替换 board.exe 失败时恢复旧版本、清掉临时文件，并说明恢复位置
    #[cfg(windows)]
    #[test]
    fn install_ps1_rolls_back_failed_binary_replacement() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let script = root.join("install.ps1");
        let harness = root.join("test.ps1");
        let rendered = render(PS1, "9.9.9", "http://127.0.0.1:1", "unused").unwrap();
        // Windows PowerShell 5.1 按 ANSI 读没有 BOM 的 UTF-8 脚本，测试文件里补上 BOM
        std::fs::write(&script, format!("\u{feff}{rendered}")).unwrap();
        let harness_script = r#"$ErrorActionPreference = 'Stop'
$root = $env:BOARD_INSTALL_TEST_ROOT
$assets = Join-Path $root 'assets'
$binDir = Join-Path $root 'bin'
New-Item -ItemType Directory -Force $assets | Out-Null
New-Item -ItemType Directory -Force $binDir | Out-Null

$fake = Join-Path $root 'fake-board.exe'
Add-Type -TypeDefinition 'public class Program { public static void Main(string[] args) { System.Console.WriteLine("board 9.9.9"); } }' -OutputAssembly $fake -OutputType ConsoleApplication
$stage = Join-Path $root 'archive'
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item -LiteralPath $fake -Destination (Join-Path $stage 'board.exe')
$zip = Join-Path $assets 'board-windows-x64.zip'
Compress-Archive -LiteralPath (Join-Path $stage 'board.exe') -DestinationPath $zip -Force
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
Set-Content -LiteralPath (Join-Path $assets 'board-sha256.txt') -Value "$hash  board-windows-x64.zip" -Encoding Ascii
Set-Content -LiteralPath (Join-Path $binDir 'board.exe') -Value 'old' -NoNewline -Encoding Ascii

$env:BOARD_DOWNLOAD = $assets
$env:BOARD_INSTALL_DIR = $binDir
$env:PROCESSOR_ARCHITECTURE = 'AMD64'
Remove-Item Env:PROCESSOR_ARCHITEW6432 -ErrorAction SilentlyContinue

function Move-Item {
  param([string]$LiteralPath, [string]$Destination, [switch]$Force)
  if ($Destination -eq (Join-Path $env:BOARD_INSTALL_DIR 'board.exe') -and (Split-Path -Leaf $LiteralPath) -like 'board.new.*.exe') {
    throw '模拟替换 board.exe 失败'
  }
  Microsoft.PowerShell.Management\Move-Item -LiteralPath $LiteralPath -Destination $Destination -Force:$Force
}

$failed = $false
$message = ''
try {
  . $env:BOARD_INSTALL_TEST_SCRIPT
} catch {
  $failed = $true
  $message = $_.Exception.Message
}
if (-not $failed) { throw '安装脚本没有按预期失败' }
if (-not (Test-Path -LiteralPath (Join-Path $binDir 'board.exe'))) { throw '替换失败后 board.exe 不存在' }
if ((Get-Content -Raw -LiteralPath (Join-Path $binDir 'board.exe')) -ne 'old') { throw '替换失败后 board.exe 不是旧版本' }
if (Test-Path -LiteralPath (Join-Path $binDir 'board.old.exe')) { throw '回滚后还留着 board.old.exe' }
if (Get-ChildItem -LiteralPath $binDir -Filter 'board.new.*.exe') { throw '回滚后没有清理临时文件' }
if ($message -notmatch '已恢复旧版本') { throw "错误信息没有说明已回滚：$message" }
Write-Host 'PASS'
"#;
        // 同理，Windows PowerShell 5.1 需要 BOM 才能正确读取用例里的中文
        std::fs::write(&harness, format!("\u{feff}{harness_script}")).unwrap();

        let out = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&harness)
            .env("BOARD_INSTALL_TEST_ROOT", root)
            .env("BOARD_INSTALL_TEST_SCRIPT", &script)
            // 从 PowerShell 7 的测试进程启动 Windows PowerShell 时，继承的 PSModulePath 可能找不到系统自带模块
            .env("PSModulePath", r"C:\Program Files\WindowsPowerShell\Modules;C:\Windows\System32\WindowsPowerShell\v1.0\Modules")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "安装脚本回滚测试失败\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("PASS"));
    }
}
