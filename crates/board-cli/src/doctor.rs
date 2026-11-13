//! \`board doctor\`：一条命令把环境问题说清楚。
//!
//! 检查项（每项都有稳定的 key、布尔状态和「下一步怎么做」）：
//! - \`board_path\`：正在运行的 board 的绝对路径（方便加进 PATH）
//! - \`path\`：PATH 里能不能找到 board，找到的是不是同一个文件
//! - \`board_url\`：服务端地址有没有配置、格式对不对、来自哪里（环境变量 BOARD_URL 还是 ~/.board/config.toml）
//! - \`server\`：服务端能不能连上，版本号是多少（\`GET /api/health\`）
//! - \`board_token\`：访问令牌有没有配置、来自哪里、是否有效（打一次需要鉴权的请求 \`GET /api/workspace\`）
//! - \`board_as\`：名字（\`--as\` 或 BOARD_AS）的格式，以及是否在线（\`GET /api/agents\`）；没提供时跳过
//! - \`board\`：白板（\`--board\` 或 BOARD）；没提供时跳过
//!
//! 环境变量和配置文件都有值时状态为 warn：环境变量覆盖了配置文件。
//! 没有 fail 时退出 0，有 fail 时退出 1。令牌原文永远不打印、不进 JSON。

use serde_json::{json, Value};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{self, env, Source};
use crate::text::{pad, width};
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
    /// 服务端地址、令牌：值来自哪里
    pub source: Option<Source>,
}

impl Check {
    fn ok(key: &'static str, title: &'static str, detail: impl Into<String>) -> Check {
        Check { key, title, status: Status::Ok, detail: detail.into(), next: None, source: None }
    }
    fn warn(key: &'static str, title: &'static str, detail: impl Into<String>, next: impl Into<String>) -> Check {
        Check { key, title, status: Status::Warn, detail: detail.into(), next: Some(next.into()), source: None }
    }
    fn fail(key: &'static str, title: &'static str, detail: impl Into<String>, next: impl Into<String>) -> Check {
        Check { key, title, status: Status::Fail, detail: detail.into(), next: Some(next.into()), source: None }
    }
    fn skip(key: &'static str, title: &'static str, detail: impl Into<String>) -> Check {
        Check { key, title, status: Status::Skip, detail: detail.into(), next: None, source: None }
    }
    fn from(mut self, src: Option<Source>) -> Check {
        self.source = src;
        self
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
    /// 环境变量 BOARD_URL / BOARD_TOKEN
    pub url_env: Option<String>,
    pub token_env: Option<String>,
    /// ~/.board/config.toml 里的 url / token
    pub url_file: Option<String>,
    pub token_file: Option<String>,
    /// 配置文件读不了时的原因
    pub file_error: Option<String>,
    /// --as 或 BOARD_AS
    pub board_as: Option<String>,
    /// --board 或 BOARD
    pub board: Option<String>,
}

impl Input {
    pub fn from_env(as_flag: Option<&str>, board_flag: Option<&str>) -> Input {
        let flag = |v: Option<&str>| v.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        let (mut url_file, mut token_file, mut file_error) = (None, None, None);
        match config::File::load() {
            Ok(f) => {
                match f.get(config::URL) {
                    Ok(v) => url_file = v,
                    Err(e) => file_error = Some(e.message),
                }
                match f.get(config::TOKEN) {
                    Ok(v) => token_file = v,
                    Err(e) => file_error = Some(e.message),
                }
            }
            Err(e) => file_error = Some(e.message),
        }
        Input {
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from(EXE_NAME)),
            path_var: std::env::var_os("PATH"),
            url_env: env("BOARD_URL"),
            token_env: env("BOARD_TOKEN"),
            url_file,
            token_file,
            file_error,
            // 没写 --as / BOARD_AS 时用本机 check 时记下的名字（board check --agent <标识> 生成的）
            board_as: flag(as_flag).or_else(|| env("BOARD_AS")).or_else(crate::heartbeat::current_name),
            board: flag(board_flag).or_else(|| env("BOARD")),
        }
    }

    /// 实际使用的服务端地址：环境变量优先
    fn url(&self) -> Option<(&str, Source)> {
        pick(&self.url_env, &self.url_file)
    }
    fn token(&self) -> Option<(&str, Source)> {
        pick(&self.token_env, &self.token_file)
    }
}

fn pick<'a>(env: &'a Option<String>, file: &'a Option<String>) -> Option<(&'a str, Source)> {
    env.as_deref().map(|v| (v, Source::Env)).or_else(|| file.as_deref().map(|v| (v, Source::File)))
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
            ProbeError::Unreachable(_) => "确认服务端地址和端口、服务端是否在运行、网络或代理是否正常",
            ProbeError::InvalidToken => "令牌写错或已被吊销：在界面「设置 → Agent 接入」新建一个，再 board login --token <令牌>",
            _ => "确认服务端是这次部署的版本；可以先在浏览器打开 <服务端地址>/api/health 看看",
        }
    }
}

/// 服务端探测抽成 trait：真实现走 HTTP，测试里用假的。
pub trait Probe {
    /// \`GET /api/health\`：不需要登录，带服务端版本
    fn health(&self, url: &str) -> Result<Health, ProbeError>;
    /// \`GET /api/workspace\`：需要令牌，用来验证令牌是否有效
    fn authorized(&self, url: &str, token: &str) -> Result<(), ProbeError>;
    /// \`GET /api/agents\`：在线的 agent 名字
    fn online(&self, url: &str, token: &str) -> Result<Vec<String>, ProbeError>;
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
        self.authed(url, "/api/workspace", token).map(|_| ())
    }

    fn online(&self, url: &str, token: &str) -> Result<Vec<String>, ProbeError> {
        let v = self.authed(url, "/api/agents", token)?;
        let list = v.as_array().ok_or_else(|| ProbeError::BadResponse("在线列表不是数组".into()))?;
        Ok(list.iter().filter_map(|a| a["name"].as_str().map(str::to_string)).collect())
    }
}

impl HttpProbe {
    /// 带令牌的 GET，返回 JSON
    fn authed(&self, url: &str, path: &str, token: &str) -> Result<Value, ProbeError> {
        let res = self
            .agent
            .get(format!("{}{path}", base(url)))
            .header("authorization", format!("Bearer {token}"))
            .call()
            .map_err(|e| ProbeError::Unreachable(e.to_string()))?;
        let mut res = res;
        let status = res.status();
        let text = res.body_mut().read_to_string().unwrap_or_default();
        if status.is_success() {
            return serde_json::from_str(&text).map_err(|e| ProbeError::BadResponse(e.to_string()));
        }
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
                    "source": c.source.map(Source::key),
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

    let login = "board login --url <服务端地址>（令牌在界面「设置 → Agent 接入」新建）";
    let file = config::shown();

    // 3. 服务端地址：有没有、格式对不对、来自哪里；环境变量和配置文件都有时 warn
    let url = input.url();
    let usable_url = url.map(|u| u.0).filter(|u| is_http_url(u));
    match url {
        None => checks.push(Check::fail(
            "board_url",
            "服务端地址",
            match &input.file_error {
                Some(e) => format!("环境变量 BOARD_URL 没有设置，配置文件读不了：{e}"),
                None => format!("没有配置：环境变量 BOARD_URL 和 {file} 里都没有，所有命令都会失败"),
            },
            login,
        )),
        Some((u, src)) if !is_http_url(u) => checks.push(
            Check::fail(
                "board_url",
                "服务端地址",
                format!("需要以 http:// 或 https:// 开头，现在是：{u}（来自{}）", config::origin(src, "BOARD_URL")),
                match src {
                    Source::Env => "改正环境变量 BOARD_URL，或者删掉它，改用 board login --url <服务端地址>".to_string(),
                    Source::File => login.to_string(),
                },
            )
            .from(Some(src)),
        ),
        Some((u, Source::Env)) if input.url_file.is_some() => checks.push(
            Check::warn(
                "board_url",
                "服务端地址",
                format!("{u}（来自环境变量 BOARD_URL，它覆盖了 {file} 里的 {}）", input.url_file.as_deref().unwrap_or_default()),
                "从 shell 配置文件（例如 ~/.zshrc、~/.bashrc）里删掉 export BOARD_URL，只用 board login 写的配置文件",
            )
            .from(Some(Source::Env)),
        ),
        Some((u, src)) => {
            checks.push(Check::ok("board_url", "服务端地址", format!("{u}（来自{}）", config::origin(src, "BOARD_URL"))).from(Some(src)))
        }
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
                    format!("确认服务端地址和端口、服务端是否在运行、网络或代理是否正常；浏览器打开 {u}/api/health 也能验证"),
                ));
            }
        }
    } else {
        checks.push(Check::skip("server", "服务端版本", "没有可用的服务端地址，跳过"));
    }

    // 5. 访问令牌：有没有、来自哪里、能不能通过一次需要鉴权的请求；环境变量和配置文件都有时 warn
    let token = input.token();
    let both = input.token_env.is_some() && input.token_file.is_some();
    let mut token_ok = false;
    match (usable_url, token) {
        (None, _) => checks.push(Check::skip("board_token", "访问令牌", "没有可用的服务端地址，跳过令牌检查")),
        (Some(u), None) => checks.push(Check::fail(
            "board_token",
            "访问令牌",
            format!("没有配置：环境变量 BOARD_TOKEN 和 {file} 里都没有"),
            format!("board login --url {u}（令牌在界面「设置 → Agent 接入」新建；命令不会回显令牌）"),
        )),
        (Some(u), Some((t, src))) if health.is_some() => {
            let from = config::origin(src, "BOARD_TOKEN");
            match probe.authorized(u, t) {
                Ok(()) if both => checks.push(
                    Check::warn(
                        "board_token",
                        "访问令牌",
                        format!("令牌有效（来自环境变量 BOARD_TOKEN，它覆盖了 {file} 里的令牌；不显示令牌原文）"),
                        "从 shell 配置文件（例如 ~/.zshrc、~/.bashrc）里删掉 export BOARD_TOKEN，只用 board login 写的配置文件",
                    )
                    .from(Some(src)),
                ),
                Ok(()) => checks.push(
                    Check::ok("board_token", "访问令牌", format!("令牌有效（来自{from}，用 /api/workspace 验证；不显示令牌原文）")).from(Some(src)),
                ),
                Err(e) => checks.push(
                    Check::fail("board_token", "访问令牌", format!("令牌（来自{from}）没通过校验：{}", e.describe()), e.next_step()).from(Some(src)),
                ),
            }
            token_ok = checks.last().is_some_and(|c| c.status.passed());
        }
        (Some(_), Some((_, src))) => {
            checks.push(Check::skip("board_token", "访问令牌", "服务端连不上，没法校验令牌").from(Some(src)))
        }
    }

    // 6. 名字：没提供时跳过；提供了就检查格式和是否在线
    match &input.board_as {
        None => checks.push(Check::skip("board_as", "名字", "没有提供（先 board check --agent <标识> 或 --as <名字>），跳过")),
        Some(n) => match board_core::check_agent_name(n) {
            Err(e) => checks.push(Check::fail("board_as", "名字", e, "按格式取名，例如 claude/opus-5.5，再 board check --as <名字>")),
            Ok(()) => match (usable_url, token) {
                (Some(u), Some((t, _))) if token_ok => match probe.online(u, t) {
                    Ok(list) if list.iter().any(|x| x == n) => checks.push(Check::ok("board_as", "名字", format!("{n}（在线）"))),
                    Ok(_) => checks.push(Check::fail(
                        "board_as",
                        "名字",
                        format!("{n} 不在线：没有 check，或心跳中断超过 {} 秒", board_core::NAME_GAP_MS / 1000),
                        format!("board check --as {n}（名字由 Board 生成时重跑 board check --agent <标识>），再 board heartbeat start"),
                    )),
                    Err(e) => checks.push(Check::fail("board_as", "名字", format!("{n}：查不到在线列表：{}", e.describe()), e.next_step())),
                },
                _ => checks.push(Check::skip("board_as", "名字", format!("{n} 格式正确；服务端或令牌没通过，没法检查是否在线"))),
            },
        },
    }

    // 7. 白板：没提供时跳过
    match &input.board {
        Some(v) => checks.push(Check::ok("board", "白板", v.clone())),
        None => checks.push(Check::skip("board", "白板", "没有提供（--board 或 BOARD），跳过")),
    }

    Report { cli_version: env!("CARGO_PKG_VERSION").to_string(), checks, server: health }
}

/// 入口：跑一遍检查，返回要打印的东西和退出码
pub fn run(json: bool, p: &Paint, as_flag: Option<&str>, board_flag: Option<&str>) -> Output {
    let report = diagnose(&Input::from_env(as_flag, board_flag), &HttpProbe::new());
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

/* ------------------------------ 测试 ------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "test/alpha";

    #[derive(Clone)]
    enum Answer {
        Ok,
        Fail(ProbeError),
    }

    struct FakeProbe {
        health: Answer,
        auth: Answer,
        online: Vec<String>,
    }

    impl FakeProbe {
        fn ok() -> FakeProbe {
            FakeProbe { health: Answer::Ok, auth: Answer::Ok, online: vec![NAME.into()] }
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
        fn online(&self, _url: &str, _token: &str) -> Result<Vec<String>, ProbeError> {
            Ok(self.online.clone())
        }
    }

    fn exe_name() -> String {
        format!("{EXE_NAME}{}", std::env::consts::EXE_SUFFIX)
    }

    /// 地址和令牌都来自配置文件
    fn input_in(dir: &Path) -> Input {
        Input {
            exe: dir.join(exe_name()),
            path_var: Some(dir.as_os_str().to_os_string()),
            url_file: Some("http://127.0.0.1:1".into()),
            token_file: Some("bd_secret_token".into()),
            board_as: Some(NAME.into()),
            board: Some("default/main".into()),
            ..Input::default()
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
        assert!(check(&report, "board_url").detail.contains(&config::shown()));
        assert_eq!(check(&report, "board_token").source, Some(Source::File));
        let v = report.to_json();
        assert_eq!(v["ok"], true);
        assert_eq!(v["summary"]["failed"], 0);
        assert!(!serde_json::to_string(&v).unwrap().contains("bd_secret_token"), "不能打印令牌原文");
        assert!(!report.to_text(&Paint { on: false }).contains("bd_secret_token"));
    }

    #[test]
    fn env_overrides_file_is_warn() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(minidir(dir.path()), b"x").unwrap();
        let mut input = input_in(dir.path());
        input.url_env = Some("http://127.0.0.1:2".into());
        input.token_env = Some("bd_env_token".into());
        let report = diagnose(&input, &FakeProbe::ok());
        assert!(report.ok(), "warn 不算失败");
        for key in ["board_url", "board_token"] {
            let c = check(&report, key);
            assert_eq!((c.status, c.source), (Status::Warn, Some(Source::Env)), "{key}");
            assert!(c.detail.contains("覆盖"), "{}", c.detail);
            assert!(c.next.as_deref().unwrap_or_default().contains("export"), "{c:?}");
        }
        assert!(check(&report, "board_url").detail.starts_with("http://127.0.0.1:2"));
        let text = serde_json::to_string(&report.to_json()).unwrap();
        assert!(text.contains("\"source\":\"env\"") && !text.contains("bd_env_token") && !text.contains("bd_secret_token"));
    }

    #[test]
    fn name_and_board_skipped_when_missing_and_name_checked_when_given() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(minidir(dir.path()), b"x").unwrap();
        let mut input = input_in(dir.path());
        input.board_as = None;
        input.board = None;
        let report = diagnose(&input, &FakeProbe::ok());
        assert_eq!(check(&report, "board_as").status, Status::Skip);
        assert_eq!(check(&report, "board").status, Status::Skip);
        assert!(report.ok(), "没提供不判失败");
        // 格式不对
        input.board_as = Some("claude".into());
        let report = diagnose(&input, &FakeProbe::ok());
        assert_eq!(check(&report, "board_as").status, Status::Fail);
        // 不在线
        input.board_as = Some("test/beta".into());
        let report = diagnose(&input, &FakeProbe::ok());
        let c = check(&report, "board_as");
        assert_eq!(c.status, Status::Fail);
        assert!(c.next.as_deref().unwrap_or_default().contains("board check --as test/beta"), "{c:?}");
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
    fn missing_url_fails_and_skips_dependent_checks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(minidir(dir.path()), b"x").unwrap();
        let mut input = input_in(dir.path());
        input.url_file = None;
        input.token_file = None;
        let report = diagnose(&input, &FakeProbe::ok());
        assert_eq!(report.exit_code(), 1);
        let c = check(&report, "board_url");
        assert_eq!(c.status, Status::Fail);
        assert!(c.detail.contains("没有配置"), "{}", c.detail);
        assert!(c.next.as_deref().unwrap_or_default().contains("board login --url <服务端地址>"));
        assert_eq!(check(&report, "server").status, Status::Skip);
        assert_eq!(check(&report, "board_token").status, Status::Skip);
        assert_eq!(check(&report, "board_as").status, Status::Skip);
    }

    #[test]
    fn invalid_token_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(minidir(dir.path()), b"x").unwrap();
        let probe = FakeProbe { auth: Answer::Fail(ProbeError::InvalidToken), ..FakeProbe::ok() };
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
        let probe = FakeProbe { health: Answer::Fail(ProbeError::Unreachable("Connection refused".into())), ..FakeProbe::ok() };
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
