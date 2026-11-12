//! 自检（PRD §10.12「自检」）：界面上点「自检」后，board host 在后台线程里做，不占并发位、不影响在跑的派发。
//!
//! - agent：在它设置的启动环境里、进一个临时目录，带上设置好的模型和思考强度，用 docs/agents.md 第 2 步那句
//!   不改任何文件的提示词跑一次（最长 3 分钟）。回答里有 `board-skills` 算通过；没登录、超时、不认识 skill 分别写明。
//! - git：每块白板的目录 `git fetch origin`，再 `git push --dry-run origin HEAD:refs/heads/board-selftest`
//!   （只检查权限，不会真的推上去）；失败时分清没有权限、没有 origin、网络不通。
//! - board 版本是否落后由服务端在收到结果时补上。
//!
//! 命令在 host.rs 里按派发同样的办法拼好（启动环境、模型、强度），这里只负责跑、限时、看输出下结论。

use crate::logfmt::{self, Stream};
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

/// 输出里像「没有登录」的那一行
fn login_line(raw: &str) -> Option<String> {
    const MARKS: [&str; 12] = [
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
        "401",
        "未登录",
    ];
    raw.lines().find(|l| {
        let l = l.to_lowercase();
        MARKS.iter().any(|m| l.contains(m))
    })
    .map(|l| short(l.trim()))
}

fn short(s: &str) -> String {
    let s: String = s.chars().take(200).collect();
    s
}

/// agent 这一项的结论：通过吗、原因、回答的前几行。`raw` 是原样的输出，`lines` 是转写后的
pub fn judge_agent(out: &Outcome, raw: &str, lines: &[String]) -> (bool, Option<String>, Option<String>) {
    // 回答：去掉提示词本身（有的 agent 会回显）、开始和结束的摘要行
    let answer: Vec<&str> = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.contains(PROMPT) && !l.starts_with("● ") && !l.starts_with("■ "))
        .collect();
    let detail = (!answer.is_empty()).then(|| answer.iter().take(4).copied().collect::<Vec<_>>().join("\n"));
    let knows = answer.iter().any(|l| l.contains("board-skills"));
    let login = login_line(raw);
    match out {
        Outcome::Spawn(e) => (false, Some(format!("启动失败：{e}")), None),
        Outcome::Timeout => {
            let why = match &login {
                Some(l) => format!("没有登录：{l}"),
                None => format!("{} 分钟内没有回答完（超时）", AGENT_LIMIT.as_secs() / 60),
            };
            (false, Some(why), detail)
        }
        Outcome::Exit(..) if knows && login.is_none() => (true, None, detail),
        Outcome::Exit(..) if login.is_some() => (false, Some(format!("没有登录：{}", login.unwrap_or_default())), detail),
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
    let mut c = Command::new("git");
    c.arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0").env("LC_ALL", "C").env("LANG", "C");
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
    let raw = std::fs::read_to_string(&job.log).unwrap_or_default();
    let lines = logfmt::render(job.stream, raw.lines().map(str::to_string).collect());
    item(judge_agent(&out, &raw, &lines))
}

/// 全部检查：每个 agent、每块白板各一个线程，一起跑
pub fn run(agents: Vec<AgentJob>, gits: Vec<GitJob>, logs: PathBuf, pids: Pids) -> Vec<SelfTestItem> {
    let mut handles = vec![];
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

    #[test]
    fn agent_answers_are_judged() {
        let ok = Outcome::Exit(true, Some(0));
        // 认得 skill
        let (pass, why, detail) = judge_agent(&ok, "board-skills\n参数\n", &sv(&["board-skills", "参数"]));
        assert!(pass && why.is_none());
        assert_eq!(detail.as_deref(), Some("board-skills\n参数"));
        // 回显的提示词里也有 board-skills，不算
        let (pass, why, _) = judge_agent(&ok, PROMPT, &sv(&[PROMPT, "我没有找到这个 skill"]));
        assert!(!pass && why.unwrap().contains("回答里没有 board-skills"));
        // 没登录（假脚本输出 Not logged in 后退出）
        let (pass, why, _) = judge_agent(&Outcome::Exit(false, Some(1)), "Not logged in · Please run /login\n", &sv(&["Not logged in · Please run /login"]));
        assert!(!pass);
        assert!(why.unwrap().starts_with("没有登录：Not logged in"));
        // 超时
        let (pass, why, _) = judge_agent(&Outcome::Timeout, "", &[]);
        assert!(!pass && why.unwrap().contains("超时"));
        // 退出码非 0：写最后一行
        let (_, why, _) = judge_agent(&Outcome::Exit(false, Some(2)), "boom\n", &sv(&["boom"]));
        assert_eq!(why.as_deref(), Some("退出码 2：boom"));
        let (_, why, _) = judge_agent(&Outcome::Spawn("No such file or directory".into()), "", &[]);
        assert_eq!(why.as_deref(), Some("启动失败：No such file or directory"));
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
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        sh(root, "git init -q --bare origin.git && git init -q work && cd work && git -c user.email=t@t -c user.name=t commit -q --allow-empty -m init && git remote add origin ../origin.git && git push -q origin HEAD:refs/heads/main");
        let pids: Pids = Arc::default();
        let good = check_git(&GitJob { board: "a/b".into(), dir: root.join("work") }, root, &pids);
        assert!(good.ok, "{good:?}");
        assert!(good.detail.unwrap().contains("origin.git"));
        let refs = Command::new("git").args(["-C", root.join("origin.git").to_str().unwrap(), "for-each-ref"]).output().unwrap();
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
