//! \`board doctor\`：一条命令把环境问题说清楚。
//!
//! 检查项（每项都有稳定的 key、布尔状态和「下一步怎么做」）：
//! - \`board_path\`：正在运行的 board 的绝对路径（方便加进 PATH）
//! - \`path\`：PATH 里能不能找到 board，找到的是不是同一个文件
//! - \`board_url\`：BOARD_URL 是否设置、格式对不对
//! - \`server\`：服务端能不能连上，版本号是多少（\`GET /api/health\`）
//! - \`board_token\`：BOARD_TOKEN 是否设置且有效（打一次需要鉴权的请求 \`GET /api/workspace\`）
//! - \`board_as\`：BOARD_AS 是否设置
//! - \`board\`：BOARD 是否设置
//!
//! 全部通过退出 0，有没通过的退出 1。令牌原文永远不打印、不进 JSON。

use serde_json::{json, Value};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

use crate::backend::env;
use crate::{Output, Paint};

/// 命令名；PATH 检查找的就是它
const EXE_NAME: &str = "board";

/* ------------------------------ 检查项 ------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 通过
    Ok,
    /// 能用，但有需要注意的地方
    Warn,
    /// 没通过：环境有问题
    Fail,
    /// 前面的检查没通过，这一项没法检查
    Skip,
}

impl Status {
    fn key(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Skip => "skip",
        }
    }
    /// 这一项算不算「能用」：warn 仍然能用，skip 不行
    fn passed(self) -> bool {
        matches!(self, Status::Ok | Status::Warn)
    }
}

#[derive(Debug, Clone)]
pub struct Check {
    pub key: &'static str,
    pub title: &'static str,
    pub status: Status,
    pub detail: String,
    pub next: Option<String>,
}

impl Check {
    fn ok(key: &'static str, title: &'static str, detail: impl Into<String>) -> Check {
        Check { key, title, status: Status::Ok, detail: detail.into(), next: None }
    }
    fn warn(key: &'static str, title: &'static str, detail: impl Into<String>, next: impl Into<String>) -> Check {
        Check { key, title, status: Status::Warn, detail: detail.into(), next: Some(next.into()) }
    }
    fn fail(key: &'static str, title: &'static str, detail: impl Into<String>, next: impl Into<String>) -> Check {
        Check { key, title, status: Status::Fail, detail: detail.into(), next: Some(next.into()) }
    }
    fn skip(key: &'static str, title: &'static str, detail: impl Into<String>) -> Check {
        Check { key, title, status: Status::Skip, detail: detail.into(), next: None }
    }
}

/* ------------------------------ 输入 ------------------------------ */

/// doctor 检查用到的环境，单独拎出来，测试里可以随便造。
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// 正在运行的 board 的绝对路径
    pub exe: PathBuf,
    /// PATH 环境变量的原样
    pub path_var: Option<OsString>,
    pub board_url: Option<String>,
    pub token: Option<String>,
    pub board_as: Option<String>,
    pub board: Option<String>,
}

impl Input {
    pub fn from_env() -> Input {
        Input {
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from(EXE_NAME)),
            path_var: std::env::var_os("PATH"),
            board_url: env("BOARD_URL"),
            token: env("BOARD_TOKEN"),
            board_as: env("BOARD_AS"),
            board: env("BOARD"),
        }
    }
}

/* ------------------------------ 服务端探测 ------------------------------ */

#[derive(Debug, Clone)]
pub struct Health {
    pub version: String,
    pub commit: String,
}

#[derive(Debug, Clone)]
pub enum ProbeError {
    /// 连不上（DNS、拒绝连接、超时……）
    Unreachable(String),
    /// 服务端说令牌无效或已吊销
    InvalidToken,
    /// 服务端要求登录（没有可用身份）
    NotLoggedIn,
    /// 其他 HTTP 拒绝
    Rejected { status: u16, message: String },
    /// 返回的数据不是预期的 JSON
    BadResponse(String),
}

impl ProbeError {
    fn describe(&self) -> String {
        match self {
            ProbeError::Unreachable(e) => format!("连不上：{e}"),
            ProbeError::InvalidToken => "令牌无效或已被吊销".to_string(),
            ProbeError::NotLoggedIn => "服务端要求登录".to_string(),
            ProbeError::Rejected { status, message } => format!("HTTP {status}：{message}"),
            ProbeError::BadResponse(e) => format!("服务端返回的数据无法解析：{e}"),
        }
    }
    fn next_step(&self) -> &'static str {
        match self {
            ProbeError::Unreachable(_) => "确认 BOARD_URL 的地址和端口、服务端是否在运行、网络或代理是否正常",
            ProbeError::InvalidToken => "令牌写错或已被吊销：在界面「设置 → Agent 接入」新建一个，再 export BOARD_TOKEN=<令牌>",
            _ => "确认服务端是这次部署的版本；可以先在浏览器打开 BOARD_URL/api/health 看看",
        }
    }
}

/// 服务端探测抽成 trait：真实现走 HTTP，测试里用假的。
pub trait Probe {
    /// \`GET /api/health\`：不需要登录，带服务端版本
    fn health(&self, url: &str) -> Result<Health, ProbeError>;
    /// \`GET /api/workspace\`：需要令牌，用来验证令牌是否有效
    fn authorized(&self, url: &str, token: &str) -> Result<(), ProbeError>;
}

/// 真的 HTTP 探测（doctor 要快，超时 10 秒）
pub struct HttpProbe {
    agent: ureq::Agent,
}

impl HttpProbe {
    pub fn new() -> HttpProbe {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .user_agent(concat!("board-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        HttpProbe { agent }
    }
}

impl Default for HttpProbe {
    fn default() -> Self {
        HttpProbe::new()
    }
}

impl Probe for HttpProbe {
    fn health(&self, url: &str) -> Result<Health, ProbeError> {
        let res = self
            .agent
            .get(format!("{}/api/health", base(url)))
            .call()
            .map_err(|e| ProbeError::Unreachable(e.to_string()))?;
        let mut res = res;
        let status = res.status();
        let text = res.body_mut().read_to_string().map_err(|e| ProbeError::BadResponse(e.to_string()))?;
        if !status.is_success() {
            return Err(ProbeError::Rejected { status: status.as_u16(), message: brief(&text) });
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| ProbeError::BadResponse(e.to_string()))?;
        Ok(Health {
            version: v["version"].as_str().unwrap_or("未知").to_string(),
            commit: v["commit"].as_str().unwrap_or_default().to_string(),
        })
    }

    fn authorized(&self, url: &str, token: &str) -> Result<(), ProbeError> {
        let res = self
            .agent
            .get(format!("{}/api/workspace", base(url)))
            .header("authorization", format!("Bearer {token}"))
            .call()
            .map_err(|e| ProbeError::Unreachable(e.to_string()))?;
        let mut res = res;
        let status = res.status();
        if status.is_success() {
            return Ok(());
        }
        let text = res.body_mut().read_to_string().unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        match v["error"]["code"].as_str().unwrap_or_default() {
            "INVALID_TOKEN" => Err(ProbeError::InvalidToken),
            "NOT_LOGGED_IN" => Err(ProbeError::NotLoggedIn),
            _ => Err(ProbeError::Rejected { status: status.as_u16(), message: brief(&text) }),
        }
    }
}

/* ------------------------------ 诊断 ------------------------------ */

pub struct Report {
    pub cli_version: String,
    pub checks: Vec<Check>,
    pub server: Option<Health>,
}

impl Report {
    fn passed(&self) -> usize {
        self.checks.iter().filter(|c| c.status.passed()).count()
    }
    fn failed(&self) -> usize {
        self.checks.iter().filter(|c| c.status == Status::Fail).count()
    }
    fn skipped(&self) -> usize {
        self.checks.iter().filter(|c| c.status == Status::Skip).count()
    }
    pub fn ok(&self) -> bool {
        self.failed() == 0
    }
    pub fn exit_code(&self) -> i32 {
        if self.ok() { 0 } else { 1 }
    }

    /// \`--json\`：每项都有 key 和布尔状态，agent 直接解析
    pub fn to_json(&self) -> Value {
        let checks: Vec<Value> = self
            .checks
            .iter()
            .map(|c| {
                json!({
                    "key": c.key,
                    "title": c.title,
                    "ok": c.status.passed(),
                    "status": c.status.key(),
                    "detail": c.detail,
                    "next": c.next,
                })
            })
            .collect();
        json!({
            "ok": self.ok(),
            "exit_code": self.exit_code(),
            "version": self.cli_version,
            "summary": {
                "total": self.checks.len(),
                "passed": self.passed(),
                "failed": self.failed(),
                "skipped": self.skipped(),
            },
            "server": match &self.server {
                Some(h) => json!({ "reachable": true, "version": h.version, "commit": h.commit }),
                None => json!({ "reachable": false }),
            },
            "checks": checks,
        })
    }

    /// 给人看的一段结论：逐项打勾，没通过的带「下一步」
    pub fn to_text(&self, p: &Paint) -> String {
        let w = self.checks.iter().map(|c| width(c.title)).max().unwrap_or(0) + 2;
        let mut s = String::new();
        s += &format!("{} {}\n\n", p.bold("board doctor"), p.dim(&format!("— 环境自检（board CLI {}）", self.cli_version)));
        for c in &self.checks {
            let mark = match c.status {
                Status::Ok => p.green("✓"),
                Status::Warn => p.yellow("!"),
                Status::Fail => p.red("✗"),
                Status::Skip => p.dim("·"),
            };
            s += &format!("  {mark} {}  {}\n", pad(c.title, w), c.detail);
            if let Some(next) = &c.next {
                s += &format!("      {} {}\n", p.dim("下一步："), next);
            }
        }
        s += "\n";
        if self.ok() {
            s += &format!("{} {} 项全部通过，可以干活。\n", p.green("结论："), self.checks.len());
        } else {
            let extra = if self.skipped() > 0 { format!("，{} 项因此跳过", self.skipped()) } else { String::new() };
            s += &format!(
                "{} {} 项通过，{} 项没通过{}。按上面的「下一步」处理后重新运行 board doctor。\n",
                p.red("结论："),
                self.passed(),
                self.failed(),
                extra
            );
        }
        s
    }
}

pub fn diagnose(input: &Input, probe: &dyn Probe) -> Report {
    let mut checks = Vec::new();

    // 1. board 自己在哪里：打印绝对路径，方便加进 PATH
    checks.push(Check::ok("board_path", "board 自身路径", input.exe.display().to_string()));

    // 2. PATH：agent 直接敲 board 能不能敲到，敲到的是不是这个文件
    let exe_dir = input.exe.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    match find_on_path(EXE_NAME, input.path_var.as_deref()) {
        Some(found) if same_file(&found, &input.exe) => {
            checks.push(Check::ok("path", "PATH 里的 board", format!("{}（就是当前运行的 board）", found.display())));
        }
        Some(found) => {
            checks.push(Check::warn(
                "path",
                "PATH 里的 board",
                format!("PATH 上的 board 是另一个文件：{}（当前运行的是 {}）", found.display(), input.exe.display()),
                path_hint(&exe_dir),
            ));
        }
        None => {
            checks.push(Check::fail("path", "PATH 里的 board", "PATH 里找不到 board：agent 直接敲 board 会报「找不到命令」", path_hint(&exe_dir)));
        }
    }

    // 3. BOARD_URL：设了没有、格式对不对
    let url = input.board_url.as_deref();
    let usable_url = url.filter(|u| is_http_url(u));
    match url {
        Some(u) if is_http_url(u) => checks.push(Check::ok("board_url", "BOARD_URL", u)),
        Some(u) => checks.push(Check::fail(
            "board_url",
            "BOARD_URL",
            format!("需要以 http:// 或 https:// 开头，现在是：{u}"),
            "把它改成服务端地址，例如 export BOARD_URL=https://board.example.com",
        )),
        None => checks.push(Check::fail(
            "board_url",
            "BOARD_URL",
            "没有设置：board 会直接读写本机数据库，而不是服务端",
            "export BOARD_URL=<服务端地址>；只在本机用可以忽略这一项",
        )),
    }

    // 4. 服务端：连不连得上、版本是多少
    let mut health = None;
    if let Some(u) = usable_url {
        match probe.health(u) {
            Ok(h) => {
                checks.push(Check::ok("server", "服务端版本", format!("连上了：board {}（提交 {}）", h.version, short_commit(&h.commit))));
                health = Some(h);
            }
            Err(e) => {
                checks.push(Check::fail(
                    "server",
                    "服务端版本",
                    format!("连不上 {u}：{}", e.describe()),
                    format!("确认 BOARD_URL 的地址和端口、服务端是否在运行、网络或代理是否正常；浏览器打开 {u}/api/health 也能验证"),
                ));
            }
        }
    } else {
        checks.push(Check::skip("server", "服务端版本", "没有可用的 BOARD_URL，跳过"));
    }

    // 5. BOARD_TOKEN：设了没有、能不能通过一次需要鉴权的请求
    match (usable_url, input.token.as_deref()) {
        (None, _) => checks.push(Check::skip("board_token", "BOARD_TOKEN", "没有可用的 BOARD_URL，跳过令牌检查")),
        (Some(_), None) => checks.push(Check::fail(
            "board_token",
            "BOARD_TOKEN",
            "设置了 BOARD_URL 但没有 BOARD_TOKEN",
            "在界面「设置 → Agent 接入」新建访问令牌，再 export BOARD_TOKEN=<令牌>（命令不会回显令牌）",
        )),
        (Some(u), Some(t)) if health.is_some() => match probe.authorized(u, t) {
            Ok(()) => checks.push(Check::ok("board_token", "BOARD_TOKEN", "令牌有效（用 /api/workspace 验证；不显示令牌原文）")),
            Err(e) => checks.push(Check::fail("board_token", "BOARD_TOKEN", format!("令牌没通过校验：{}", e.describe()), e.next_step())),
        },
        (Some(_), Some(_)) => checks.push(Check::skip("board_token", "BOARD_TOKEN", "服务端连不上，没法校验令牌")),
    }

    // 6. 身份与白板
    match &input.board_as {
        Some(v) => checks.push(Check::ok("board_as", "BOARD_AS", v.clone())),
        None => checks.push(Check::fail(
            "board_as",
            "BOARD_AS",
            "没有设置：命令不知道你是谁",
            "export BOARD_AS=<你的名字>，例如 export BOARD_AS=deepseek-cli",
        )),
    }
    match &input.board {
        Some(v) => checks.push(Check::ok("board", "BOARD", v.clone())),
        None => checks.push(Check::fail(
            "board",
            "BOARD",
            "没有设置：命令不知道读写哪块白板",
            "export BOARD=<项目>/<白板>，用 board boards 查看可选的白板",
        )),
    }

    Report { cli_version: env!("CARGO_PKG_VERSION").to_string(), checks, server: health }
}

/// 入口：跑一遍检查，返回要打印的东西和退出码
pub fn run(json: bool, p: &Paint) -> Output {
    let report = diagnose(&Input::from_env(), &HttpProbe::new());
    let body = if json { Output::Json(report.to_json()) } else { Output::Text(report.to_text(p)) };
    match report.exit_code() {
        0 => body,
        code => Output::Exit(code, Box::new(body)),
    }
}

/* ------------------------------ 小工具 ------------------------------ */

fn base(url: &str) -> &str {
    url.trim_end_matches('/')
}

fn brief(text: &str) -> String {
    let s = text.trim();
    let mut out: String = s.chars().take(200).collect();
    if s.chars().count() > 200 {
        out.push('…');
    }
    out
}

fn is_http_url(u: &str) -> bool {
    u.starts_with("http://") || u.starts_with("https://")
}

fn short_commit(commit: &str) -> &str {
    if commit.len() > 7 { &commit[..7] } else { commit }
}

/// PATH 里 cd 到 name：先找带平台后缀的（Windows 上是 board.exe），再找不带后缀的
pub fn find_on_path(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    for dir in std::env::split_paths(path_var) {
        let raw = dir.to_string_lossy().trim_matches('"').to_string();
        let dir = Path::new(&raw);
        if dir.as_os_str().is_empty() {
            continue;
        }
        for cand in [dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)), dir.join(name)] {
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

#[cfg(windows)]
fn path_hint(dir: &Path) -> String {
    let d = dir.display();
    format!("PowerShell：$env:Path += \";{d}\"；bash：export PATH=\"$PATH:{d}\"；长期生效：把 {d} 加进系统的 PATH 环境变量")
}

#[cfg(not(windows))]
fn path_hint(dir: &Path) -> String {
    let d = dir.display();
    format!("export PATH=\"$PATH:{d}\"（写进 ~/.zshrc 或 ~/.bashrc 长期生效）")
}

fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn pad(s: &str, w: usize) -> String {
    let n = width(s);
    if n >= w { s.to_string() } else { format!("{s}{}", " ".repeat(w - n)) }
}

/* ------------------------------ 测试 ------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Answer {
        Ok,
        Fail(ProbeError),
    }

    struct FakeProbe {
        health: Answer,
        auth: Answer,
    }

    impl FakeProbe {
        fn ok() -> FakeProbe {
            FakeProbe { health: Answer::Ok, auth: Answer::Ok }
        }
    }

    impl Probe for FakeProbe {
        fn health(&self, _url: &str) -> Result<Health, ProbeError> {
            match &self.health {
                Answer::Ok => Ok(Health { version: "0.0.0-test".into(), commit: "0123456789".into() }),
                Answer::Fail(e) => Err(e.clone()),
            }
        }
        fn authorized(&self, _url: &str, _token: &str) -> Result<(), ProbeError> {
            match &self.auth {
                Answer::Ok => Ok(()),
                Answer::Fail(e) => Err(e.clone()),
            }
        }
    }

    fn exe_name() -> String {
        format!("{EXE_NAME}{}", std::env::consts::EXE_SUFFIX)
    }

    fn input_in(dir: &Path) -> Input {
        Input {
            exe: dir.join(exe_name()),
            path_var: Some(dir.as_os_str().to_os_string()),
            board_url: Some("http://127.0.0.1:1".into()),
            token: Some("bd_secret_token".into()),
            board_as: Some("tester".into()),
            board: Some("default/main".into()),
        }
    }

    fn check<'a>(report: &'a Report, key: &str) -> &'a Check {
        report.checks.iter().find(|c| c.key == key).unwrap_or_else(|| panic!("缺少检查项 {key}"))
    }

    #[test]
    fn all_checks_pass() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(minidir(dir.path()), b"x").unwrap();
        let report = diagnose(&input_in(dir.path()), &FakeProbe::ok());
        assert!(report.ok());
        assert_eq!(report.exit_code(), 0);
        let keys = ["board_path", "path", "board_url", "server", "board_token", "board_as", "board"];
        for key in keys {
            assert_eq!(check(&report, key).status, Status::Ok, "{key}");
        }
        let v = report.to_json();
        assert_eq!(v["ok"], true);
        assert_eq!(v["summary"]["failed"], 0);
        assert!(!serde_json::to_string(&v).unwrap().contains("bd_secret_token"), "不能打印令牌原文");
        assert!(!report.to_text(&Paint { on: false }).contains("bd_secret_token"));
    }

    #[test]
    fn path_missing_fails_with_next_step() {
        let exe_dir = tempfile::tempdir().unwrap();
        let empty_dir = tempfile::tempdir().unwrap();
        std::fs::write(minidir(exe_dir.path()), b"x").unwrap();
        let mut input = input_in(exe_dir.path());
        input.path_var = Some(empty_dir.path().as_os_str().to_os_string());
        let report = diagnose(&input, &FakeProbe::ok());
        let c = check(&report, "path");
        assert_eq!(c.status, Status::Fail);
        assert!(c.detail.contains("PATH 里找不到 board"), "{}", c.detail);
        assert!(c.next.as_deref().unwrap_or_default().contains("PATH"));
        assert_eq!(report.exit_code(), 1);
        let text = report.to_text(&Paint { on: false });
        assert!(text.contains('✗') && text.contains("下一步："), "{text}");
    }

    #[test]
    fn missing_board_url_fails_and_skips_dependent_checks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(minidir(dir.path()), b"x").unwrap();
        let mut input = input_in(dir.path());
        input.board_url = None;
        input.token = None;
        let report = diagnose(&input, &FakeProbe::ok());
        assert_eq!(report.exit_code(), 1);
        let c = check(&report, "board_url");
        assert_eq!(c.status, Status::Fail);
        assert!(c.detail.contains("没有设置"), "{}", c.detail);
        assert!(c.next.as_deref().unwrap_or_default().contains("BOARD_URL"));
        assert_eq!(check(&report, "server").status, Status::Skip);
        assert_eq!(check(&report, "board_token").status, Status::Skip);
    }

    #[test]
    fn invalid_token_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(minidir(dir.path()), b"x").unwrap();
        let probe = FakeProbe { health: Answer::Ok, auth: Answer::Fail(ProbeError::InvalidToken) };
        let report = diagnose(&input_in(dir.path()), &probe);
        assert_eq!(report.exit_code(), 1);
        let c = check(&report, "board_token");
        assert_eq!(c.status, Status::Fail);
        assert!(c.detail.contains("令牌"), "{}", c.detail);
        assert!(c.next.as_deref().unwrap_or_default().contains("Agent 接入"));
        assert!(!serde_json::to_string(&report.to_json()).unwrap().contains("bd_secret_token"));
    }

    #[test]
    fn unreachable_server_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(minidir(dir.path()), b"x").unwrap();
        let probe = FakeProbe { health: Answer::Fail(ProbeError::Unreachable("Connection refused".into())), auth: Answer::Ok };
        let report = diagnose(&input_in(dir.path()), &probe);
        assert_eq!(report.exit_code(), 1);
        let c = check(&report, "server");
        assert_eq!(c.status, Status::Fail);
        assert!(c.detail.contains("连不上"), "{}", c.detail);
        assert_eq!(check(&report, "board_token").status, Status::Skip);
    }

    fn minidir(dir: &Path) -> PathBuf {
        dir.join(exe_name())
    }
}
