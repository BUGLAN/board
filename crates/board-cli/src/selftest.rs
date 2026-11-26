//! 自检（PRD §10.12「自检」）：界面上点「自检」后，board host 在后台线程里做，不占会话位、不影响在跑的派发。
//!
//! - agent：在它设置的启动环境里、进一个临时目录，带上设置好的模型和思考强度，用 docs/agents.md 第 2 步那句
//!   不改任何文件的提示词跑一次（最长 3 分钟）。agent 说的话（ACP 的消息片段接起来）里答出了 SKILL.md 第一个二级标题，
//!   或者答出了 `board-skills`（不看大小写、连字符这些写法）又没在说找不到它，算通过；没登录、超时、不认识 skill 分别写明。
//! - git：每块白板的目录 `git fetch origin`，再 `git push --dry-run origin HEAD:refs/heads/board-selftest`
//!   （只检查权限，不会真的推上去）；失败时分清没有权限、没有 origin、网络不通。
//! - 供应商（整台机器时）：共用配置里的每个供应商，每种接口发一次列模型的请求（`<地址>/models`，Anthropic 是
//!   `<地址>/v1/models`），带上 key。连不上（解析不了、拒绝连接、超时）、key 被拒（401、403）算没通过；别的回应
//!   （包括 404：有的中转不给模型列表）都算连得上。服务端据此算用这个供应商的角色在这台机器上能不能跑（#574）。
//!   agent 一项因为没登录没通过（原因以 [`board_core::profiles::LOGIN_FAILED`] 开头）时，本机配置的角色在这台机器上跑不了。
//! - board 版本是否落后由服务端在收到结果时补上。
//!
//! 命令在 host/manage.rs 里按派发同样的办法拼好（启动环境、模型、强度），这里只负责跑、限时、看输出下结论。

use crate::logfmt::{self, Stream};
use board_core::profiles::LOGIN_FAILED;
use board_core::SelfTestItem;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 问 agent 的那一句（docs/agents.md「本机验证」）：不改任何文件，答完就退出
pub const PROMPT: &str = "/board-skills 不要执行任何命令。只回答两行：第一行是你加载的 skill 的 name，第二行是它 SKILL.md 里第一个二级标题。";
/// agent 一项最长多久
pub const AGENT_LIMIT: Duration = Duration::from_secs(180);
/// 每条 git 命令最长多久
const GIT_LIMIT: Duration = Duration::from_secs(60);
/// 问供应商的每个请求最长多久
const PROVIDER_LIMIT: Duration = Duration::from_secs(20);
/// 试推送的分支（`--dry-run`，不会真的出现在远端）
pub const PROBE_REF: &str = "HEAD:refs/heads/board-selftest";

/// 检查一个 agent：拼好的命令（拼不出来时是原因），以及它的日志按哪种事件流转写
pub struct AgentJob {
    pub id: String,
    pub cmd: std::result::Result<Command, String>,
    pub stream: Option<Stream>,
    pub log: PathBuf,
}

/// 检查一块白板的目录能不能推送
pub struct GitJob {
    pub board: String,
    pub dir: PathBuf,
}

/// 检查一个供应商连不连得上：名称、各接口的地址（服务端规范化过的）、key
pub struct ProviderJob {
    pub name: String,
    pub endpoints: Vec<board_core::providers::ProviderEndpoint>,
    pub key: String,
}

/// 正在跑的子进程：board host 退出时结束它们
pub type Pids = Arc<Mutex<Vec<u32>>>;

/// 一条命令怎么结束的
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// 没能启动
    Spawn(String),
    /// 超时，已经结束它
    Timeout,
    /// 退出了：退出码 0 吗、退出码
    Exit(bool, Option<i32>),
}

/// 跑一条命令，输出（标准输出和标准错误）写进 log，最多等 limit
fn run_limited(mut cmd: Command, log: &Path, limit: Duration, pids: &Pids) -> Outcome {
    let files = std::fs::File::create(log).and_then(|out| out.try_clone().map(|err| (out, err)));
    let (out, err) = match files {
        Ok(x) => x,
        Err(e) => return Outcome::Spawn(format!("无法创建日志 {}：{e}", log.display())),
    };
    cmd.stdin(Stdio::null()).stdout(out).stderr(err);
    crate::envs::detach(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Outcome::Spawn(e.to_string()),
    };
    let pid = child.id();
    if let Ok(mut p) = pids.lock() {
        p.push(pid);
    }
    let begun = Instant::now();
    let out = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Outcome::Exit(s.success(), s.code()),
            Ok(None) if begun.elapsed() < limit => std::thread::sleep(Duration::from_millis(200)),
            _ => {
                crate::envs::kill_tree(pid, true);
                let _ = child.kill();
                let _ = child.wait();
                break Outcome::Timeout;
            }
        }
    };
    if let Ok(mut p) = pids.lock() {
        p.retain(|x| *x != pid);
    }
    out
}

/// 问一个 agent 自己用的身份（#667）和本机账号能用的模型（#666）：跑 `board acp --auth probe`（拼好的命令），读它写到标准输出的
/// 那一行 `{"auth": …, "models": …}`。跑不起来、超时、没写出来时身份的种类是 `unknown`、label 写原因，模型表写原因。最多等 `limit`
pub fn probe_agent(cmd: Command, log: &Path, limit: Duration, pids: &Pids) -> (board_core::hosts::AgentAuth, board_core::hosts::LocalModels) {
    use board_core::hosts::{AgentAuth, LocalModels, AUTH_UNKNOWN};
    #[derive(serde::Deserialize)]
    struct Line {
        auth: AgentAuth,
        #[serde(default)]
        models: Option<LocalModels>,
    }
    let failed = |why: String| (AgentAuth { kind: AUTH_UNKNOWN.into(), label: why.clone(), ..Default::default() }, LocalModels { error: why, ..Default::default() });
    match run_limited(cmd, log, limit, pids) {
        Outcome::Spawn(e) => return failed(format!("启动失败：{e}")),
        Outcome::Timeout => return failed(format!("{} 秒内没问完", limit.as_secs())),
        Outcome::Exit(..) => {}
    }
    // 标准错误也在这个文件里（agent 的提示）：从后往前找那一行 JSON
    let text = std::fs::read_to_string(log).unwrap_or_default();
    text.lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Line>(l.trim()).ok())
        .find(|x| !x.auth.kind.is_empty())
        .map(|x| (x.auth, x.models.unwrap_or_else(|| LocalModels { error: "没列出模型".into(), ..Default::default() })))
        .unwrap_or_else(|| failed("没问出来".into()))
}

/// 一种接口列模型的地址、是不是 Anthropic 的写法
fn probe_url(interface: &str, url: &str) -> Option<(String, bool)> {
    let base = board_core::providers::normalize_url(interface, url);
    if base.is_empty() {
        return None;
    }
    Some(if interface == "anthropic" { (format!("{base}/v1/models"), true) } else { (format!("{base}/models"), false) })
}

/// 检查一个供应商：每种接口问一次，第一个没通过的写进原因。key 不出现在原因、日志里
pub fn check_provider(job: &ProviderJob) -> SelfTestItem {
    let item = |ok: bool, why: Option<String>, detail: Option<String>| SelfTestItem { kind: "provider".into(), key: job.name.clone(), ok, why, detail, at: 0 };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(PROVIDER_LIMIT))
        .http_status_as_error(false)
        .user_agent(concat!("board-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut seen = vec![];
    for e in &job.endpoints {
        let Some((url, anthropic)) = probe_url(&e.interface, &e.url) else { continue };
        let mut req = agent.get(&url).header("authorization", format!("Bearer {}", job.key));
        if anthropic {
            req = req.header("x-api-key", &job.key).header("anthropic-version", "2023-06-01");
        }
        match req.call() {
            Err(err) => return item(false, Some(format!("连不上 {url}：{}", redact_key(&err.to_string(), &job.key))), None),
            Ok(res) if matches!(res.status().as_u16(), 401 | 403) => {
                return item(false, Some(format!("{url} 拒绝了 API key（HTTP {}）", res.status().as_u16())), None);
            }
            Ok(res) => seen.push(format!("{} HTTP {}", e.interface, res.status().as_u16())),
        }
    }
    if seen.is_empty() {
        return item(false, Some("这个供应商没有地址".into()), None);
    }
    item(true, None, Some(seen.join(" · ")))
}

fn redact_key(s: &str, key: &str) -> String {
    if key.is_empty() {
        s.to_string()
    } else {
        s.replace(key, "***")
    }
}

/// 输出里像「没有登录」的那一行。`lines` 是转写后的日志，原始 JSON 事件里的 uuid、session_id 等元数据不算；
/// 401 只认单独的状态码（uuid 里的 `401e8ee6-…` 不算）
fn login_line(lines: &[String]) -> Option<String> {
    // 「要求先登录」「authentication required」：适配器在 session/new 按 ACP 规范回 -32000 时 board acp 记的（#667）
    const MARKS: [&str; 13] = [
        "要求先登录",
        "authentication required",
        "not logged in",
        "please run /login",
        "run /login",
        "please log in",
        "login required",
        "not authenticated",
        "invalid api key",
        "authentication_error",
        "authentication failed",
        "unauthorized",
        "未登录",
    ];
    lines
        .iter()
        .find(|l| {
            let l = l.to_lowercase();
            MARKS.iter().any(|m| l.contains(m)) || has_status_401(&l)
        })
        .map(|l| short(l.trim()))
}

/// 单独的 401（前后不是字母数字）：`status 401 Unauthorized` 算，`uuid: 401e8ee6…`、`1401` 不算
fn has_status_401(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(2)).any(|i| {
        &b[i..i + 3] == b"401"
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
            && (i + 3 == b.len() || !b[i + 3].is_ascii_alphanumeric())
    })
}

fn short(s: &str) -> String {
    let s: String = s.chars().take(200).collect();
    s
}

/// board-skills 的 SKILL.md（和 board host 装进各 agent skill 目录的是同一版）：自检问的第二行是它第一个二级标题
const SKILL_MD: &str = include_str!("../../../.agents/skills/board-skills/SKILL.md");

/// 只留字母、数字、汉字，转小写：`board-skills`、`Board Skills`、`` `board_skills` ``、`board‑skills`（不换行连字符）都是 `boardskills`
fn fold(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// SKILL.md 里第一个二级标题（折叠后）
fn skill_heading() -> String {
    SKILL_MD.lines().find_map(|l| l.strip_prefix("## ")).map(fold).unwrap_or_default()
}

/// 说没有这个 skill 的话（折叠后比较）：回答里带着 skill 的名字、但其实在说找不到它，不算认识
const DENIALS: [&str; 16] = [
    "没有找到",
    "没找到",
    "找不到",
    "未找到",
    "不认识",
    "没有这个",
    "没有加载",
    "未加载",
    "无法加载",
    "不存在",
    "not found",
    "can't find",
    "cannot find",
    "could not find",
    "unknown skill",
    "no such skill",
];

/// agent 说的话（不含思考、工具调用、日志里别的行）：ACP 把这一轮所有 `agent_message_chunk` 的文字按顺序接起来，
/// 一个词被拆进几帧（模型一个 token 一帧、`board acp` 每 2 秒落一次、中间夹着 `usage_update`）也能拼回原样；
/// 不是 ACP 的输出（测试用的假脚本）用原始的行。agent 回显的提示词去掉
pub fn spoken(stream: Option<Stream>, raw: &[String]) -> String {
    let text = match stream {
        Some(Stream::Acp) => raw
            .iter()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
            .filter_map(|v| {
                let u = logfmt::update_of(&v)?;
                (u["sessionUpdate"].as_str() == Some("agent_message_chunk")).then(|| logfmt::content_text(&u["content"]))
            })
            .collect::<String>(),
        None => raw.join("\n"),
    };
    text.replace(PROMPT, "")
}

/// 回答说明 agent 认识 board-skills：答出了 SKILL.md 第一个二级标题；或者答出了名字，又没有在说找不到它。
/// 名字、标题都按 [`fold`] 比较，不看大小写、连字符、空白、反引号这些写法上的差别
fn knows_skill(said: &str) -> bool {
    let said = fold(said);
    let heading = skill_heading();
    if !heading.is_empty() && said.contains(&heading) {
        return true;
    }
    said.contains(&fold(board_core::skills::BUILTIN)) && !DENIALS.iter().any(|d| said.contains(&fold(d)))
}

/// agent 这一项的结论：通过吗、原因、回答的前几行。`lines` 是转写后的日志，`said` 是 agent 说的话（[`spoken`]）
pub fn judge_agent(out: &Outcome, lines: &[String], said: &str) -> (bool, Option<String>, Option<String>) {
    // 回答：去掉提示词本身（有的 agent 会回显）、开始和结束的摘要行
    let answer: Vec<&str> = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.contains(PROMPT) && !l.starts_with("● ") && !l.starts_with("■ "))
        .collect();
    let detail = (!answer.is_empty()).then(|| answer.iter().take(4).copied().collect::<Vec<_>>().join("\n"));
    let knows = knows_skill(said);
    let login = login_line(lines);
    match out {
        Outcome::Spawn(e) => (false, Some(format!("启动失败：{e}")), None),
        Outcome::Timeout => {
            let why = match &login {
                Some(l) => format!("{LOGIN_FAILED}：{l}"),
                None => format!("{} 分钟内没有回答完（超时）", AGENT_LIMIT.as_secs() / 60),
            };
            (false, Some(why), detail)
        }
        // 已经答出 board-skills 就算通过：日志里顺带的报错（重试、接口抖动）不影响结论
        Outcome::Exit(..) if knows => (true, None, detail),
        Outcome::Exit(..) if login.is_some() => (false, Some(format!("{LOGIN_FAILED}：{}", login.unwrap_or_default())), detail),
        Outcome::Exit(false, code) => {
            let last = lines.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty()).map(short).unwrap_or_default();
            let code = code.map(|c| c.to_string()).unwrap_or_else(|| "（被信号结束）".into());
            (false, Some(if last.is_empty() { format!("退出码 {code}") } else { format!("退出码 {code}：{last}") }), detail)
        }
        Outcome::Exit(true, _) => (
            false,
            Some("回答里没有 board-skills：agent 不认识这个 skill，在它的启动环境里装好 board-skills（docs/agents.md「装 board-skills」）".into()),
            detail,
        ),
    }
}

/// git 失败的原因（输出用 LC_ALL=C 跑出来的英文）
pub fn git_why(output: &str) -> String {
    let lower = output.to_lowercase();
    let last = output.lines().rev().map(str::trim).find(|l| !l.is_empty()).map(short).unwrap_or_default();
    let has = |marks: &[&str]| marks.iter().any(|m| lower.contains(m));
    let line = |marks: &[&str]| {
        output
            .lines()
            .map(str::trim)
            .find(|l| {
                let l = l.to_lowercase();
                marks.iter().any(|m| l.contains(m))
            })
            .map(short)
            .unwrap_or_else(|| last.clone())
    };
    const NET: [&str; 7] = [
        "could not resolve host",
        "network is unreachable",
        "connection timed out",
        "operation timed out",
        "connection refused",
        "failed to connect",
        "no route to host",
    ];
    const AUTH: [&str; 9] = [
        "permission denied",
        "403",
        "authentication failed",
        "could not read username",
        "could not read password",
        "access denied",
        "marked as read only",
        "write access",
        "denied to",
    ];
    const GONE: [&str; 3] = ["repository not found", "does not appear to be a git repository", "does not exist"];
    if has(&NET) {
        format!("网络不通：{}", line(&NET))
    } else if has(&AUTH) {
        format!("没有推送权限：{}", line(&AUTH))
    } else if has(&GONE) {
        format!("远端仓库不存在（或没有权限访问）：{}", line(&GONE))
    } else {
        last
    }
}

fn git(dir: &Path, args: &[&str], log: &Path, limit: Duration, pids: &Pids) -> (Outcome, String) {
    let mut c = crate::gitcmd::command();
    c.arg("-C").arg(dir).args(args).env("LC_ALL", "C").env("LANG", "C");
    let out = run_limited(c, log, limit, pids);
    let text = std::fs::read_to_string(log).unwrap_or_default();
    (out, text)
}

fn tail(text: &str, n: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    (!lines.is_empty()).then(|| lines[lines.len().saturating_sub(n)..].join("\n"))
}

/// 检查一块白板的目录：是 git 仓库、有 origin、拉得下来、推得上去（dry-run）
pub fn check_git(job: &GitJob, logs: &Path, pids: &Pids) -> SelfTestItem {
    let item = |ok: bool, why: Option<String>, detail: Option<String>| SelfTestItem { kind: "git".into(), key: job.board.clone(), ok, why, detail, at: 0 };
    if !job.dir.is_dir() {
        return item(false, Some(format!("目录不存在：{}", job.dir.display())), None);
    }
    let safe: String = job.board.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let log = logs.join(format!("git-{safe}.log"));
    let (o, _) = git(&job.dir, &["rev-parse", "--git-dir"], &log, GIT_LIMIT, pids);
    if o != Outcome::Exit(true, Some(0)) {
        return item(false, Some(format!("不是 git 仓库：{}", job.dir.display())), None);
    }
    let (o, url) = git(&job.dir, &["remote", "get-url", "origin"], &log, GIT_LIMIT, pids);
    if o != Outcome::Exit(true, Some(0)) {
        return item(false, Some("没有 origin：在这个目录里 git remote add origin <仓库地址>".into()), None);
    }
    let url = url.trim().to_string();
    let step = |o: Outcome, text: String, what: &str| -> Option<SelfTestItem> {
        match o {
            Outcome::Exit(true, _) => None,
            Outcome::Timeout => Some(item(false, Some(format!("{what}超过 {} 秒没做完：网络不通，或在等输入密码", GIT_LIMIT.as_secs())), tail(&text, 3))),
            Outcome::Spawn(e) => Some(item(false, Some(format!("没法运行 git：{e}")), None)),
            Outcome::Exit(false, _) => Some(item(false, Some(format!("{what}失败，{}", git_why(&text))), tail(&text, 3))),
        }
    };
    let (o, text) = git(&job.dir, &["fetch", "origin"], &log, GIT_LIMIT, pids);
    if let Some(bad) = step(o, text, "git fetch origin ") {
        return bad;
    }
    let (o, text) = git(&job.dir, &["push", "--dry-run", "origin", PROBE_REF], &log, GIT_LIMIT, pids);
    if let Some(bad) = step(o, text, "git push --dry-run ") {
        return bad;
    }
    item(true, None, Some(format!("origin {url}")))
}

/// 检查一个 agent
pub fn check_agent(job: AgentJob, pids: &Pids) -> SelfTestItem {
    let item = |(ok, why, detail): (bool, Option<String>, Option<String>)| SelfTestItem { kind: "agent".into(), key: job.id.clone(), ok, why, detail, at: 0 };
    let cmd = match job.cmd {
        Ok(c) => c,
        Err(why) => return item((false, Some(why), None)),
    };
    let out = run_limited(cmd, &job.log, AGENT_LIMIT, pids);
    let raw: Vec<String> = std::fs::read_to_string(&job.log).unwrap_or_default().lines().map(str::to_string).collect();
    let said = spoken(job.stream, &raw);
    let lines = logfmt::render(job.stream, raw);
    item(judge_agent(&out, &lines, &said))
}

/// 全部检查：每个 agent、每个供应商、每块白板各一个线程，一起跑
pub fn run(agents: Vec<AgentJob>, providers: Vec<ProviderJob>, gits: Vec<GitJob>, logs: PathBuf, pids: Pids) -> Vec<SelfTestItem> {
    let mut handles = vec![];
    for job in providers {
        handles.push(std::thread::spawn(move || check_provider(&job)));
    }
    for job in agents {
        let pids = pids.clone();
        handles.push(std::thread::spawn(move || check_agent(job, &pids)));
    }
    for job in gits {
        let (pids, logs) = (pids.clone(), logs.clone());
        handles.push(std::thread::spawn(move || check_git(&job, &logs, &pids)));
    }
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// 本地起一个只回一次的 HTTP 服务：回 `status`，记下请求头
    fn one_shot(status: &str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let status = status.to_string();
        let h = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = c.read(&mut buf).unwrap_or(0);
            let _ = write!(c, "HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        (url, h)
    }

    fn ep(interface: &str, url: &str) -> board_core::providers::ProviderEndpoint {
        board_core::providers::ProviderEndpoint { interface: interface.into(), url: url.into() }
    }

    /// #574：供应商连不连得上。key 被拒（401）没通过；404（中转不给模型列表）也算连得上；连不上的地址写明，原因里没有 key
    #[test]
    fn providers_are_probed() {
        let (url, h) = one_shot("401 Unauthorized");
        let bad = check_provider(&ProviderJob { name: "relay".into(), endpoints: vec![ep("anthropic", &format!("{url}/v1"))], key: "sk-secret".into() });
        let req = h.join().unwrap();
        assert!(!bad.ok && bad.kind == "provider" && bad.key == "relay", "{bad:?}");
        assert!(bad.why.as_deref().unwrap().contains("拒绝了 API key（HTTP 401）"), "{bad:?}");
        assert!(req.starts_with("GET /v1/models "), "Anthropic 地址去掉 /v1 再接 /v1/models：{req}");
        assert!(req.to_lowercase().contains("x-api-key: sk-secret"), "{req}");

        let (url, h) = one_shot("404 Not Found");
        let ok = check_provider(&ProviderJob { name: "r2".into(), endpoints: vec![ep("openai-chat", &format!("{url}/v1/"))], key: "k".into() });
        let req = h.join().unwrap();
        assert!(ok.ok, "{ok:?}");
        assert!(req.starts_with("GET /v1/models "), "{req}");
        assert!(req.to_lowercase().contains("authorization: bearer k"), "{req}");

        // 没人监听的端口
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let gone = check_provider(&ProviderJob { name: "r3".into(), endpoints: vec![ep("openai-chat", &format!("http://127.0.0.1:{port}/v1"))], key: "sk-secret".into() });
        let why = gone.why.unwrap();
        assert!(!gone.ok && why.starts_with(&format!("连不上 http://127.0.0.1:{port}/v1/models")), "{why}");
        assert!(!why.contains("sk-secret"));
        let none = check_provider(&ProviderJob { name: "r4".into(), endpoints: vec![], key: String::new() });
        assert_eq!(none.why.as_deref(), Some("这个供应商没有地址"));
    }

    /// 和 check_agent 一样：从原始日志算出 agent 说的话、转写后的行，再下结论
    fn judge(out: &Outcome, stream: Option<Stream>, raw: &[&str]) -> (bool, Option<String>, Option<String>) {
        let raw = sv(raw);
        let said = spoken(stream, &raw);
        judge_agent(out, &logfmt::render(stream, raw), &said)
    }

    /// ACP 的一帧 session/update
    fn upd(update: serde_json::Value) -> String {
        serde_json::json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s", "update": update } }).to_string()
    }

    fn say(t: &str) -> String {
        upd(serde_json::json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": t } }))
    }

    fn acp(frames: &[String]) -> (bool, Option<String>, Option<String>) {
        let refs: Vec<&str> = frames.iter().map(String::as_str).collect();
        judge(&Outcome::Exit(true, Some(0)), Some(Stream::Acp), &refs)
    }

    #[test]
    fn agent_answers_are_judged() {
        let ok = Outcome::Exit(true, Some(0));
        // 认得 skill
        let (pass, why, detail) = judge(&ok, None, &["board-skills", "参数"]);
        assert!(pass && why.is_none());
        assert_eq!(detail.as_deref(), Some("board-skills\n参数"));
        // 回显的提示词里也有 board-skills，不算
        let (pass, why, _) = judge(&ok, None, &[PROMPT, "我没有找到这个 skill"]);
        assert!(!pass && why.unwrap().contains("回答里没有 board-skills"));
        // ACP 的帧：会话标识以 401 开头、发出去的提示词里也有 board-skills，转写后都不该影响结论
        let raw = [
            r#"{"id":3,"jsonrpc":"2.0","method":"session/prompt","params":{"prompt":[{"text":"/board-skills 不要执行任何命令。","type":"text"}],"sessionId":"401e8ee6-e6c2-4642-86d5-3d3fc9e6de00"}}"#,
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"401e8ee6-e6c2-4642-86d5-3d3fc9e6de00","update":{"sessionUpdate":"usage_update","used":1,"size":2}}}"#,
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"401e8ee6-e6c2-4642-86d5-3d3fc9e6de00","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"board-skills\n参数"}}}}"#,
        ];
        let lines = logfmt::render(Some(Stream::Acp), sv(&raw));
        assert_eq!(lines, sv(&["board-skills", "参数"]));
        let (pass, why, _) = judge(&ok, Some(Stream::Acp), &raw);
        assert!(pass, "{why:?}");
        // 已经答出 board-skills：日志里顺带的 401 重试不影响结论
        let (pass, why, _) = judge(&ok, None, &["✗ Reconnecting... 1/5 (unexpected status 401 Unauthorized: Invalid token)", "board-skills", "参数"]);
        assert!(pass, "{why:?}");
        // 没登录（假脚本输出 Not logged in 后退出）
        let (pass, why, _) = judge(&Outcome::Exit(false, Some(1)), None, &["Not logged in · Please run /login"]);
        assert!(!pass);
        assert!(why.unwrap().starts_with("没有登录：Not logged in"));
        // 适配器按 ACP 规范在 session/new 回 -32000：board acp 写「agent 要求先登录」（#667）
        let (pass, why, _) = judge(&Outcome::Exit(false, Some(2)), None, &["✗ agent 要求先登录：Authentication required"]);
        assert!(!pass);
        assert!(why.unwrap().starts_with("没有登录：✗ agent 要求先登录"), "要求先登录也算没登录");
        // 超时
        let (pass, why, _) = judge(&Outcome::Timeout, None, &[]);
        assert!(!pass && why.unwrap().contains("超时"));
        // 退出码非 0：写最后一行
        let (_, why, _) = judge(&Outcome::Exit(false, Some(2)), None, &["boom"]);
        assert_eq!(why.as_deref(), Some("退出码 2：boom"));
        let (_, why, _) = judge(&Outcome::Spawn("No such file or directory".into()), None, &[]);
        assert_eq!(why.as_deref(), Some("启动失败：No such file or directory"));
    }

    /// #582：答对了的 agent 不能判「没通过」。名字被拆进几帧（中间夹着 usage_update、每 2 秒落一次）、写法不同
    /// （大小写、下划线、不换行连字符、反引号）、只答对了 SKILL.md 第一个二级标题，都算认得
    #[test]
    fn correct_answers_in_other_shapes_pass() {
        let heading = SKILL_MD.lines().find_map(|l| l.strip_prefix("## ")).unwrap();
        let usage = upd(serde_json::json!({ "sessionUpdate": "usage_update", "used": 1, "size": 2 }));
        // 一个 token 一帧，夹着 usage_update：转写后「board」「-skills」各成一行
        let frames = [say("board"), usage.clone(), say("-sk"), usage.clone(), say("ills\n"), say(heading)];
        let lines = logfmt::render(Some(Stream::Acp), frames.to_vec());
        assert!(!lines.iter().any(|l| l.contains("board-skills")), "转写后的每一行里都没有完整的名字：{lines:?}");
        let (pass, why, _) = acp(&frames);
        assert!(pass, "{why:?}");
        // 只拆了名字、第二行答错了，也认得
        let (pass, why, _) = acp(&[say("board"), usage, say("-skills\n参数")]);
        assert!(pass, "{why:?}");
        for name in ["Board Skills", "`board_skills`", "board\u{2011}skills", "**name**: BOARD-SKILLS"] {
            let (pass, why, _) = acp(&[say(&format!("{name}\n不记得了"))]);
            assert!(pass, "{name}：{why:?}");
        }
        // 名字没照抄，但答出了 SKILL.md 第一个二级标题
        let (pass, why, _) = acp(&[say(&format!("Board 的 skill\n## {heading}"))]);
        assert!(pass, "{why:?}");
    }

    /// 真正不认识 skill 的仍判失败：没提名字；提了名字但在说找不到；只在思考、工具调用、回显的提示词里出现名字
    #[test]
    fn unknown_skill_still_fails() {
        let not_known = |r: (bool, Option<String>, Option<String>)| !r.0 && r.1.is_some_and(|w| w.contains("回答里没有 board-skills"));
        assert!(not_known(acp(&[say("我不知道你说的是哪个 skill。")])));
        assert!(not_known(acp(&[say("我没有找到 board-skills 这个 skill。")])));
        assert!(not_known(acp(&[say("Sorry, the skill `board-skills` was not found.")])));
        assert!(not_known(acp(&[say("I can't find a skill named board-skills.")])));
        let think = upd(serde_json::json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "用户要 board-skills" } }));
        let tool = upd(serde_json::json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "kind": "read", "title": "Read ~/.agents/skills/board-skills/SKILL.md", "status": "failed" }));
        assert!(not_known(acp(&[think, tool, say("没有这个 skill")])));
        // agent 把提示词原样说了一遍
        assert!(not_known(acp(&[say(PROMPT), say("\n好的")])));
        assert!(not_known(judge(&Outcome::Exit(true, Some(0)), None, &["我没有找到 board-skills"])));
    }

    #[test]
    fn login_markers_ignore_uuid_metadata() {
        assert!(!has_status_401("uuid: 401e8ee6-e6c2-4642-86d5-3d3fc9e6de00"));
        assert!(!has_status_401("1401"));
        assert!(has_status_401("unexpected status 401 Unauthorized"));
        assert!(has_status_401("http 401"));
    }

    #[test]
    fn git_failures_are_named() {
        assert!(git_why("ssh: Could not resolve host: github.com\nfatal: Could not read from remote repository.").starts_with("网络不通："));
        assert!(git_why("remote: Permission to a/b.git denied to someone.\nfatal: unable to access 'https://github.com/a/b.git/': The requested URL returned error: 403").starts_with("没有推送权限："));
        assert!(git_why("ERROR: The key you are authenticating with has been marked as read only.").starts_with("没有推送权限："));
        assert!(git_why("fatal: could not read Username for 'https://github.com': terminal prompts disabled").starts_with("没有推送权限："));
        assert!(git_why("fatal: '/nope/repo.git' does not appear to be a git repository\nfatal: Could not read from remote repository.").starts_with("远端仓库不存在"));
        assert_eq!(git_why("error: something odd\n"), "error: something odd");
    }

    fn sh(dir: &Path, script: &str) {
        let ok = Command::new("sh").arg("-c").arg(script).current_dir(dir).output().unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    }

    /// 真的 git：本地裸仓库当 origin 能推送；origin 指向不存在的地址时失败；远端不会出现 board-selftest 分支
    #[test]
    fn git_check_with_local_remotes() {
        if crate::gitcmd::command().arg("--version").output().is_err() {
            return;
        }
        // 用 shell 串起几条 git：Windows 上没有 sh，这台机器上也就跑不了这个用例
        if cfg!(windows) {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        sh(root, "git init -q --bare origin.git && git init -q work && cd work && git -c user.email=t@t -c user.name=t commit -q --allow-empty -m init && git remote add origin ../origin.git && git push -q origin HEAD:refs/heads/main");
        let pids: Pids = Arc::default();
        let good = check_git(&GitJob { board: "a/b".into(), dir: root.join("work") }, root, &pids);
        assert!(good.ok, "{good:?}");
        assert!(good.detail.unwrap().contains("origin.git"));
        let refs = crate::gitcmd::command().args(["-C", root.join("origin.git").to_str().unwrap(), "for-each-ref"]).output().unwrap();
        assert!(!String::from_utf8_lossy(&refs.stdout).contains("board-selftest"), "dry-run 不会真的推上去");
        // origin 改成不存在的地址
        sh(root, "cd work && git remote set-url origin /nope/missing.git");
        let bad = check_git(&GitJob { board: "a/b".into(), dir: root.join("work") }, root, &pids);
        assert!(!bad.ok && bad.why.as_deref().unwrap().contains("远端仓库不存在"), "{bad:?}");
        // 没有 origin、不是 git 仓库、目录不存在
        sh(root, "cd work && git remote remove origin && mkdir ../plain");
        assert!(check_git(&GitJob { board: "a/b".into(), dir: root.join("work") }, root, &pids).why.unwrap().contains("没有 origin"));
        assert!(check_git(&GitJob { board: "a/b".into(), dir: root.join("plain") }, root, &pids).why.unwrap().contains("不是 git 仓库"));
        assert!(check_git(&GitJob { board: "a/b".into(), dir: root.join("gone") }, root, &pids).why.unwrap().contains("目录不存在"));
    }

    /// 假的 agent 脚本（不执行真实的 agent 命令）：一个答出 board-skills，一个说没登录
    #[cfg(unix)]
    #[test]
    fn agent_check_with_fake_scripts() {
        let d = tempfile::tempdir().unwrap();
        let pids: Pids = Arc::default();
        let job = |script: &str, id: &str| {
            let mut c = Command::new("sh");
            c.arg("-c").arg(script).current_dir(d.path());
            AgentJob { id: id.into(), cmd: Ok(c), stream: None, log: d.path().join(format!("{id}.log")) }
        };
        let good = check_agent(job("printf 'board-skills\\n参数\\n'", "good"), &pids);
        assert!(good.ok && good.kind == "agent" && good.key == "good", "{good:?}");
        let bad = check_agent(job("echo 'Not logged in · Please run /login'; exit 1", "bad"), &pids);
        assert!(!bad.ok && bad.why.as_deref().unwrap().starts_with("没有登录"), "{bad:?}");
        let gone = check_agent(AgentJob { id: "x".into(), cmd: Err("在登录 shell 里没找到命令 claude".into()), stream: None, log: d.path().join("x.log") }, &pids);
        assert_eq!(gone.why.as_deref(), Some("在登录 shell 里没找到命令 claude"));
        assert!(pids.lock().unwrap().is_empty());
    }
}
