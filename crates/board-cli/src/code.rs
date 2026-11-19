//! 派发前把项目的代码准备好（PRD §9.1「派发前更新代码」）：board host 每次开始一张卡之前，先在项目目录里
//! `git fetch origin`，再给这个会话备好一个停在最新集成分支上的 worktree，agent 就在这个 worktree 里启动。
//!
//! 以前 agent 是在项目的主目录里启动、自己照 skill 的约定去建 worktree：主目录克隆一次之后没人更新，它启动时读到的
//! AGENTS.md、仓库里自带的 skill 可能是很久以前的。现在：
//!
//! - **拉最新**（[`fetch`]）：拉不下来（没权限、网络不通）就不启动 agent，原因报给服务端，这台机器先不接这个项目的卡。
//! - **worktree**（[`prepare`]）：放在 `<仓库目录名>.wt/` 里，目录名是 `<名字，/ 换成 ->-<名字的哈希前 8 位>`（[`dir_name`]：
//!   `a/b-c` 和 `a-b/c` 不会落到同一个目录），锁的原因写名字。名字为空、格式不对时不准备（#508）。
//!   已经有、而且干净（没有没提交的文件、提交都在远端）就接着用，切到最新的集成分支——依赖、编译缓存都还在；
//!   里面有没推的改动就不动它，这次另开一个（名字后面加派发 ID），免得冲掉上一次的半成品。
//!   目录在、git 却不登记它（[`leftover`]：`git worktree remove` 删到一半，Windows 上文件被占用、路径太长时常见）：
//!   那一次的任务已经结束了，有所有者记录、而且记录对得上才删掉重建；删不掉、没有记录时这次另开一个，agent 照常启动。
//! - **所有者记录**：board host 建的每个 worktree 在 `<仓库目录名>.wt/.board-worktree/<目录名>.json` 记下主仓库、
//!   会话名、派发和创建时间。删目录（残留目录、清理）之前都要它在、对得上，否则只报告、不删（#508）。
//! - **清理**（[`sweep`]）：board host 备的 worktree 记着最近一次用的时间；超过 24 小时没用、又是干净的就删掉。
//!   有没推改动的留着，没有所有者记录的（旧版 board host 建的）也留着、写明原因。
//!   人手动跑 `/board-skills` 的会话自己建的 worktree 没有使用记号，不碰。
//!
//! 只在项目目录是 git 仓库、有 `origin` 时做；没有仓库的项目、拆解、WSL 里的 agent（worktree 要由 WSL 里的 git 来建）
//! 照旧在项目目录里启动。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// `git fetch` 最多等多久：board host 每 10 秒上报一次、50 秒没上报算掉线，拉取不能把它拖到掉线
pub const FETCH_LIMIT: Duration = Duration::from_secs(30);
/// 建、切换 worktree 最多等多久
pub const WT_LIMIT: Duration = Duration::from_secs(60);
/// board host 备的 worktree 多久没用就清理
pub const WT_UNUSED: Duration = Duration::from_secs(24 * 3600);
/// worktree 的 git 目录里记最近一次使用时间的文件（不在工作区里，不会让 worktree 变脏）
const MARK: &str = "board-used";
/// `<仓库目录名>.wt/` 里放所有者记录的目录（以 `.` 开头，不会和会话的目录重名）
const OWNERS: &str = ".board-worktree";

/// 在 `dir` 里跑一条 git 命令，最多等 `limit`。成功返回标准输出（去掉首尾空白）；失败返回标准错误的最后一行
pub fn git(dir: &Path, args: &[&str], limit: Duration) -> Result<String, String> {
    let out = git_output(dir, args, limit)?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    Err(git_error(&out))
}

/// 同 [`git`]，但不管退出码，原样返回输出：要按退出码区分结果时用（例如 `git config --get-all` 退出码 1 是这一项不存在）。
/// 没法运行 git、超时时返回错误
pub fn git_output(dir: &Path, args: &[&str], limit: Duration) -> Result<std::process::Output, String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("没法运行 git：{e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} 秒还没做完，已经停掉", limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(format!("等 git 结束时出错：{e}")),
        }
    }
    child.wait_with_output().map_err(|e| format!("读不到 git 的输出：{e}"))
}

/// git 失败时的原因：标准错误的最后一行（最多 300 个字符），没有时写「git 失败」
pub fn git_error(out: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr);
    let last = err.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("git 失败").to_string();
    last.chars().take(300).collect()
}

const QUICK: Duration = Duration::from_secs(10);

pub fn is_repo(dir: &Path) -> bool {
    dir.is_dir() && git(dir, &["rev-parse", "--is-inside-work-tree"], QUICK).as_deref() == Ok("true")
}

pub fn has_origin(dir: &Path) -> bool {
    git(dir, &["remote", "get-url", "origin"], QUICK).is_ok_and(|u| !u.is_empty())
}

/// 把远端的最新提交拉下来（不动工作区）
pub fn fetch(repo: &Path) -> Result<(), String> {
    git(repo, &["fetch", "--quiet", "--prune", "origin"], FETCH_LIMIT).map(|_| ())
}

/// 集成分支：项目设置里写了就是它；没写用远端的默认分支（`origin/HEAD`），再没有用项目目录现在所在的分支。
/// 远端没有这个分支时报错
pub fn integration_branch(repo: &Path, want: &str) -> Result<String, String> {
    let want = want.trim();
    let branch = if !want.is_empty() {
        want.to_string()
    } else if let Ok(head) = git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"], QUICK) {
        head.strip_prefix("origin/").unwrap_or(&head).to_string()
    } else {
        git(repo, &["branch", "--show-current"], QUICK).ok().filter(|b| !b.is_empty()).ok_or("认不出集成分支：在项目设置里填上")?
    };
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")], QUICK).map_err(|_| format!("远端没有分支 {branch}"))?;
    Ok(branch)
}

/// 主仓库目录（`repo` 自己也可能是一个 worktree）
fn main_repo(repo: &Path) -> Option<PathBuf> {
    let common = git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"], QUICK).ok()?;
    Some(Path::new(&common).parent()?.to_path_buf())
}

/// 放 worktree 的目录：主仓库目录旁边的 `<仓库目录名>.wt`
fn wt_root(repo: &Path) -> Option<PathBuf> {
    let main = main_repo(repo)?;
    let base = main.file_name()?.to_string_lossy().to_string();
    Some(main.parent()?.join(format!("{base}.wt")))
}

/// 会话 worktree 的目录名：可读的前缀（名字里的 `/` 换成 `-`）加上名字的 SHA-256 前 8 位。
/// 只换 `/` 时 `a/b-c` 和 `a-b/c` 是同一个目录，两个会话会共用一个工作区、切分支时互相覆盖（#508）
pub fn dir_name(name: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash: String = Sha256::digest(name.as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{}-{hash}", name.replace('/', "-"))
}

/// 这个名字的会话的 worktree：`<仓库目录名>.wt/<dir_name(名字)>`
pub fn worktree_path(repo: &Path, name: &str) -> Option<PathBuf> {
    Some(wt_root(repo)?.join(dir_name(name)))
}

/// 所有者记录：board host 建的 worktree 是谁的。删这个目录之前要它在、而且对得上
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Owner {
    /// 主仓库目录
    repo: PathBuf,
    /// worktree 目录
    path: PathBuf,
    /// 会话名（原样）
    name: String,
    /// 建它的那次派发
    dispatch: i64,
    /// 什么时候建的（Unix 秒）
    created_at: u64,
}

fn owner_file(wt: &Path) -> Option<PathBuf> {
    Some(wt.parent()?.join(OWNERS).join(format!("{}.json", wt.file_name()?.to_string_lossy())))
}

fn read_owner(wt: &Path) -> Option<Owner> {
    serde_json::from_str(&std::fs::read_to_string(owner_file(wt)?).ok()?).ok()
}

/// 记下这个 worktree 归 board host 的这个会话（已经有、而且是同一个仓库同一个目录时不改创建时间）
fn write_owner(repo: &Path, wt: &Path, name: &str, dispatch: i64) {
    let (Some(main), Some(file)) = (main_repo(repo), owner_file(wt)) else { return };
    if read_owner(wt).is_some_and(|o| same_path(&o.repo, &main) && same_path(&o.path, wt) && o.name == name) {
        return;
    }
    let created_at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let o = Owner { repo: main, path: wt.to_path_buf(), name: name.to_string(), dispatch, created_at };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, serde_json::to_string_pretty(&o).unwrap_or_default());
}

/// 能不能删 `wt` 这个目录：在本仓库的 `.wt/` 下面（直接一层、不是 `.wt` 本身）、有所有者记录、记录里是本仓库和这个目录
/// （`name` 不为空时还要是这个会话）。不能删时返回原因
fn owned(repo: &Path, wt: &Path, name: Option<&str>) -> Result<(), String> {
    let root = wt_root(repo).ok_or("找不到主仓库目录")?;
    let inside = wt.parent().is_some_and(|p| same_path(p, &root)) && wt.file_name().is_some_and(|n| !n.is_empty() && !n.to_string_lossy().starts_with('.'));
    if !inside {
        return Err(format!("不在 {} 里", root.display()));
    }
    let Some(o) = read_owner(wt) else {
        return Err("没有 board host 的所有者记录（旧版建的、或者不是 board host 建的）".into());
    };
    let main = main_repo(repo).ok_or("找不到主仓库目录")?;
    if !same_path(&o.repo, &main) || !same_path(&o.path, wt) || name.is_some_and(|n| n != o.name) {
        return Err(format!("所有者记录对不上（记的是 {} 的会话 {}）", o.repo.display(), o.name));
    }
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    c(a) == c(b)
}

/// 这个仓库登记着的 worktree 的路径
fn worktrees(repo: &Path) -> Vec<PathBuf> {
    git(repo, &["worktree", "list", "--porcelain"], QUICK)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("worktree ").map(PathBuf::from))
        .collect()
}

/// 目录里没推到远端的改动：没提交的文件、不在任何远端分支上的提交；都推了时为空
pub fn unpushed(dir: &Path) -> Option<String> {
    let mut parts = vec![];
    let dirty = git(dir, &["status", "--porcelain"], QUICK).map(|s| s.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0);
    if dirty > 0 {
        parts.push(format!("{dirty} 个文件没提交"));
    }
    if let Ok(head) = git(dir, &["rev-parse", "--short", "HEAD"], QUICK) {
        if git(dir, &["branch", "-r", "--contains", "HEAD"], QUICK).is_ok_and(|s| s.is_empty()) {
            parts.push(format!("提交 {head} 不在任何远端分支上"));
        }
    }
    (!parts.is_empty()).then(|| parts.join("，"))
}

/// 记下这个 worktree 刚用过（写在它的 git 目录里）
pub fn touch(wt: &Path) {
    if let Ok(dir) = git(wt, &["rev-parse", "--absolute-git-dir"], QUICK) {
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = std::fs::write(Path::new(&dir).join(MARK), now.to_string());
    }
}

/// 这个 worktree 上一次被 board host 用是什么时候；不是 board host 备的（没有记号）时为空
fn used_at(wt: &Path) -> Option<SystemTime> {
    let dir = git(wt, &["rev-parse", "--absolute-git-dir"], QUICK).ok()?;
    std::fs::metadata(Path::new(&dir).join(MARK)).ok()?.modified().ok()
}

/// 准备好的代码
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    /// agent 在这里启动：这个会话的 worktree
    pub cwd: PathBuf,
    /// worktree 停在 `origin/<这个分支>` 上
    pub branch: String,
    /// 原来的 worktree 里有没推的改动、这次另开了一个时，写原来那个在哪、留着什么
    pub left: Option<(PathBuf, String)>,
    /// 碰到了残留目录（git 不再登记的 worktree）：在哪；删掉了时第二项为空，删不掉（这次另开了一个）时写原因
    pub stale: Option<(PathBuf, Option<String>)>,
    /// 碰到了像残留 worktree 的目录，但没有对得上的所有者记录：没删、这次另开了一个。在哪、为什么没删
    pub kept: Option<(PathBuf, String)>,
}

/// 准备失败
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fail {
    /// 拉不到最新代码：这台机器先不接这个项目的卡，过一会儿再试
    Fetch(String),
    /// 别的原因（远端没有集成分支、建不了 worktree）
    Other(String),
}

/// `dir` 是这个仓库留下的残留 worktree：git 已经不登记它，里面的 `.git` 文件指向本仓库的 `.git/worktrees/…`，
/// 或者 `.git` 文件已经没了（`git worktree remove` 删工作区删到一半时，登记照样删掉，目录留着）。
/// `.git` 是目录（一个完整的克隆）或指向别的仓库时不是
fn leftover(repo: &Path, dir: &Path) -> bool {
    let dotgit = dir.join(".git");
    let Ok(meta) = std::fs::symlink_metadata(&dotgit) else { return true };
    if !meta.is_file() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(&dotgit) else { return false };
    let Some(gitdir) = text.lines().find_map(|l| l.strip_prefix("gitdir:")).map(str::trim) else { return false };
    // 相对路径（worktree.useRelativePaths）相对 worktree 目录；绝对路径 join 之后就是它自己
    let gitdir = dir.join(gitdir);
    let Ok(common) = git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"], QUICK) else { return false };
    // <common>/worktrees/<id>：比 `.git` 这一级（一定还在，能规范化），worktrees/<id> 可能已经删了
    gitdir.parent().and_then(Path::parent).is_some_and(|p| same_path(p, Path::new(&common)))
}

/// 拉最新，备好名字是 `name` 的会话的 worktree，停在最新的集成分支上。`id` 是派发 ID（另开 worktree 时用它起名）。
/// 名字为空、格式不对时什么都不做（不拉、不建、不删）
pub fn prepare(repo: &Path, name: &str, id: i64, want_branch: &str) -> Result<Ready, Fail> {
    board_core::model::check_agent_name(name).map_err(|e| Fail::Other(format!("会话名不对，不准备 worktree：{e}")))?;
    fetch(repo).map_err(Fail::Fetch)?;
    let branch = integration_branch(repo, want_branch).map_err(Fail::Other)?;
    let tip = format!("origin/{branch}");
    let first = worktree_path(repo, name).ok_or_else(|| Fail::Other("找不到主仓库目录".into()))?;
    let known = worktrees(repo);
    let registered = |p: &Path| known.iter().any(|k| same_path(k, p));
    let base = first.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let another = first.with_file_name(format!("{base}-{id}"));
    let mut target = first.clone();
    let mut left = None;
    let mut stale = None;
    let mut kept = None;
    if target.exists() && !registered(&target) {
        if !leftover(repo, &target) {
            return Err(Fail::Other(format!("{} 已经存在，但不是这个仓库的 worktree：挪走或删掉它", target.display())));
        }
        match owned(repo, &target, Some(name)) {
            // 残留目录、是这个会话的：那一次的任务已经结束，不管里面剩下什么，删掉重建；删不掉（文件被占用）就这次另开一个
            Ok(()) => match std::fs::remove_dir_all(&target) {
                Ok(()) => stale = Some((target.clone(), None)),
                Err(e) => {
                    stale = Some((target.clone(), Some(e.to_string())));
                    target = another.clone();
                }
            },
            // 证明不了是这个会话的：不删，这次另开一个
            Err(why) => {
                kept = Some((target.clone(), why));
                target = another.clone();
            }
        }
        // 清掉可能还留着的登记（gitdir 指向已经没了的目录）
        let _ = git(repo, &["worktree", "prune"], QUICK);
    }
    if target.exists() {
        match unpushed(&target) {
            // 上一次留着没推的改动：不动它，这次另开一个
            Some(what) => {
                left = Some((target.clone(), what));
                target = another.clone();
            }
            None => {
                git(&target, &["checkout", "--quiet", "--detach", &tip], WT_LIMIT).map_err(|e| Fail::Other(format!("{} 切不到 {tip}：{e}", target.display())))?;
            }
        }
    }
    if !target.exists() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Fail::Other(format!("无法创建 {}：{e}", parent.display())))?;
        }
        let path = target.to_string_lossy().to_string();
        git(repo, &["worktree", "add", "--quiet", "--detach", "--lock", "--reason", name, &path, &tip], WT_LIMIT)
            .map_err(|e| Fail::Other(format!("建不了 worktree {path}：{e}")))?;
    }
    touch(&target);
    write_owner(repo, &target, name, id);
    Ok(Ready { cwd: target, branch, left, stale, kept })
}

/// 保存下来的半成品
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wip {
    /// 推到了哪条分支
    pub branch: String,
    /// 留下了什么，一句话
    pub changes: String,
}

/// 半成品交接：worktree 里还有没推到远端的改动时，把没提交的文件提交掉（提交人写 board host，不跑钩子），
/// 推到 `branch`；那条分支已经有别的内容、推不进去时改推 `alt`（带机器名，免得盖掉别人的）。
/// worktree 是干净的（都推了）时返回 None
pub fn save_wip(wt: &Path, branch: &str, alt: &str, message: &str) -> Result<Option<Wip>, String> {
    if unpushed(wt).is_none() {
        return Ok(None);
    }
    let status = git(wt, &["status", "--porcelain"], QUICK)?;
    let files: Vec<String> = status.lines().filter(|l| l.len() > 3).map(|l| l[3..].trim().to_string()).collect();
    if !files.is_empty() {
        git(wt, &["add", "-A"], WT_LIMIT)?;
        git(wt, &["-c", "user.name=board host", "-c", "user.email=board-host@localhost", "commit", "--quiet", "--no-verify", "-m", message], WT_LIMIT)?;
    }
    let ahead = git(wt, &["rev-list", "--count", "HEAD", "--not", "--remotes"], QUICK).ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
    let mut used = branch.to_string();
    if git(wt, &["push", "--quiet", "origin", &format!("HEAD:refs/heads/{branch}")], FETCH_LIMIT).is_err() {
        used = alt.to_string();
        git(wt, &["push", "--quiet", "--force", "origin", &format!("HEAD:refs/heads/{alt}")], FETCH_LIMIT)?;
    }
    // 让本地认得这条远端分支：之后这个 worktree 就算干净的，下一次派发可以接着用
    let _ = git(wt, &["fetch", "--quiet", "origin", &format!("+refs/heads/{used}:refs/remotes/origin/{used}")], FETCH_LIMIT);
    // 留下了什么：这次替它提交的文件，和它自己提交了、但没推的提交
    let earlier = ahead.saturating_sub(usize::from(!files.is_empty()));
    let shown: Vec<&str> = files.iter().take(8).map(String::as_str).collect();
    let more = if files.len() > shown.len() { format!(" 等 {} 个", files.len()) } else { String::new() };
    let changes = match (files.is_empty(), earlier) {
        (true, n) => format!("{n} 个没推的提交"),
        (false, 0) => format!("{} 个没提交的文件（{}{more}）", files.len(), shown.join("、")),
        (false, n) => format!("{} 个没提交的文件（{}{more}），另有 {n} 个没推的提交", files.len(), shown.join("、")),
    };
    Ok(Some(Wip { branch: used, changes }))
}

/// 清理一个 worktree 的结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Swept {
    /// 删掉了
    Removed,
    /// 有没推到远端的改动，留着：留着什么
    Unpushed(String),
    /// 没有对得上的所有者记录（旧版 board host 建的），留着：为什么，确认没用后由人删
    NoOwner(String),
}

/// 清理 board host 备的、超过 `older` 没用的 worktree：干净的删掉，有没推改动的留着，没有对得上的所有者记录的留着。
/// `in_use` 是正在跑的会话用着的，不碰；没有使用记号的（人手动跑的会话自己建的）也不碰
pub fn sweep(repo: &Path, in_use: &[PathBuf], older: Duration, now: SystemTime) -> Vec<(PathBuf, Swept)> {
    let Some(root) = wt_root(repo) else { return vec![] };
    let mut out = vec![];
    for wt in worktrees(repo) {
        if !wt.starts_with(&root) && !wt.canonicalize().is_ok_and(|p| root.canonicalize().is_ok_and(|r| p.starts_with(r))) {
            continue;
        }
        if in_use.iter().any(|p| same_path(p, &wt)) {
            continue;
        }
        let Some(at) = used_at(&wt) else { continue };
        if now.duration_since(at).unwrap_or_default() < older {
            continue;
        }
        if let Err(why) = owned(repo, &wt, None) {
            out.push((wt, Swept::NoOwner(why)));
            continue;
        }
        match unpushed(&wt) {
            Some(what) => out.push((wt, Swept::Unpushed(what))),
            None => {
                let path = wt.to_string_lossy().to_string();
                let _ = git(repo, &["worktree", "unlock", &path], QUICK);
                if git(repo, &["worktree", "remove", &path], WT_LIMIT).is_ok() {
                    if let Some(f) = owner_file(&wt) {
                        let _ = std::fs::remove_file(f);
                    }
                    out.push((wt, Swept::Removed));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} 在 {} 失败", dir.display());
    }

    /// 一个「远端」裸仓库、一个往里推提交的上游克隆、一台机器上的项目目录（克隆）
    struct Rig {
        _tmp: tempfile::TempDir,
        up: PathBuf,
        repo: PathBuf,
    }

    /// 去掉 Windows 上 `canonicalize()` 加的 `\\?\` 前缀：git 不把带前缀的路径当本地路径
    /// （`git clone \\?\C:\…` 会报「hostname contains invalid characters」）
    fn plain(p: &Path) -> PathBuf {
        match p.to_string_lossy().strip_prefix(r"\\?\") {
            Some(rest) => PathBuf::from(rest),
            None => p.to_path_buf(),
        }
    }

    /// 读出来的文件按换行归一：Windows 上 git 检出会把 LF 写成 CRLF
    fn text(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
    }

    fn rig() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        // 去掉 Windows 上 `canonicalize()` 加的 `\\?\` 前缀：`git clone \\?\C:\…` 会报「hostname contains invalid characters」
        let root = plain(&tmp.path().canonicalize().unwrap());
        let origin = root.join("origin.git");
        std::fs::create_dir(&origin).unwrap();
        run(&origin, &["init", "-q", "--bare", "-b", "main"]);
        let up = root.join("up");
        run(&root, &["clone", "-q", origin.to_str().unwrap(), "up"]);
        std::fs::write(up.join("AGENTS.md"), "v1\n").unwrap();
        run(&up, &["add", "."]);
        run(&up, &["commit", "-q", "-m", "v1"]);
        run(&up, &["push", "-q", "origin", "HEAD:main"]);
        let ws = root.join("ws");
        std::fs::create_dir(&ws).unwrap();
        run(&ws, &["clone", "-q", origin.to_str().unwrap(), "proj"]);
        Rig { _tmp: tmp, up, repo: ws.join("proj") }
    }

    fn push(r: &Rig, text: &str) {
        std::fs::write(r.up.join("AGENTS.md"), text).unwrap();
        run(&r.up, &["commit", "-q", "-am", text.trim()]);
        run(&r.up, &["push", "-q", "origin", "HEAD:main"]);
    }

    /// 每次派发前拉最新：worktree 里读到的是远端刚推上来的内容，主目录克隆得再早也一样；同一个名字接着用同一个 worktree
    #[test]
    fn prepare_fetches_and_reuses_the_session_worktree_on_the_latest_branch() {
        let r = rig();
        push(&r, "v2\n");
        let a = prepare(&r.repo, "claude-mbp/opus-5.5", 1, "").unwrap();
        assert_eq!(a.cwd, r.repo.parent().unwrap().join("proj.wt").join(dir_name("claude-mbp/opus-5.5")));
        assert!(dir_name("claude-mbp/opus-5.5").starts_with("claude-mbp-opus-5.5-"), "可读的前缀加哈希");
        let o = read_owner(&a.cwd).expect("建好时写了所有者记录");
        assert_eq!((o.name.as_str(), o.dispatch), ("claude-mbp/opus-5.5", 1));
        assert!(same_path(&o.repo, &r.repo) && same_path(&o.path, &a.cwd));
        assert_eq!(a.branch, "main", "没写集成分支时用远端的默认分支");
        assert_eq!(text(&a.cwd.join("AGENTS.md")), "v2\n");
        assert_eq!(text(&r.repo.join("AGENTS.md")), "v1\n", "主目录不动");
        // 依赖、编译缓存这类没被跟踪、被忽略的文件留着
        std::fs::write(a.cwd.join(".git-ignored-cache"), "x").unwrap();
        std::fs::write(a.cwd.join(".gitignore"), ".git-ignored-cache\n.gitignore\n").unwrap();
        push(&r, "v3\n");
        let b = prepare(&r.repo, "claude-mbp/opus-5.5", 2, "main").unwrap();
        assert_eq!(b.cwd, a.cwd, "干净的 worktree 接着用");
        assert!(b.left.is_none());
        assert_eq!(text(&b.cwd.join("AGENTS.md")), "v3\n");
        assert!(b.cwd.join(".git-ignored-cache").exists());
        // 锁着，原因是名字（和 board-skills 的约定一样）
        let list = git(&r.repo, &["worktree", "list", "--porcelain"], QUICK).unwrap();
        assert!(list.contains("locked claude-mbp/opus-5.5"), "{list}");
    }

    /// worktree 里有没推的改动：不动它，这次另开一个，并说明原来那个留着什么
    #[test]
    fn prepare_leaves_unpushed_work_alone_and_opens_another_worktree() {
        let r = rig();
        let a = prepare(&r.repo, "dsh-win/x", 1, "").unwrap();
        std::fs::write(a.cwd.join("half.txt"), "做到一半").unwrap();
        let b = prepare(&r.repo, "dsh-win/x", 7, "").unwrap();
        assert_eq!(b.cwd, a.cwd.with_file_name(format!("{}-7", dir_name("dsh-win/x"))));
        assert!(read_owner(&b.cwd).is_some(), "另开的也有所有者记录");
        assert_eq!(b.left.as_ref().map(|(p, w)| (p.clone(), w.as_str())), Some((a.cwd.clone(), "1 个文件没提交")));
        assert_eq!(std::fs::read_to_string(a.cwd.join("half.txt")).unwrap(), "做到一半");
        assert!(unpushed(&b.cwd).is_none());
    }

    /// 模拟 `git worktree remove` 删到一半：登记（`.git/worktrees/<id>`）没了，目录和里面剩下的文件还在
    fn half_removed(wt: &Path) {
        let admin = git(wt, &["rev-parse", "--absolute-git-dir"], QUICK).unwrap();
        std::fs::remove_dir_all(admin).unwrap();
    }

    /// 残留目录（git 不再登记的 worktree）：不管里面剩下什么，删掉重建，agent 照常启动；`.git` 文件也没了时一样
    #[test]
    fn prepare_deletes_a_leftover_worktree_dir_and_rebuilds_it() {
        let r = rig();
        let a = prepare(&r.repo, "win/codex", 1, "").unwrap();
        std::fs::create_dir_all(a.cwd.join("node_modules/pkg")).unwrap();
        std::fs::write(a.cwd.join("node_modules/pkg/a.node"), "x").unwrap();
        std::fs::write(a.cwd.join("half.txt"), "失败那次留下的").unwrap();
        std::fs::remove_file(a.cwd.join("AGENTS.md")).unwrap();
        half_removed(&a.cwd);
        assert!(!worktrees(&r.repo).iter().any(|w| same_path(w, &a.cwd)), "git 已经不登记它");
        assert!(leftover(&r.repo, &a.cwd));
        push(&r, "v2\n");
        let b = prepare(&r.repo, "win/codex", 2, "").unwrap();
        assert_eq!(b.cwd, a.cwd, "删掉之后在原来的位置重建");
        assert_eq!(b.stale, Some((a.cwd.clone(), None)));
        assert!(b.left.is_none());
        assert!(worktrees(&r.repo).iter().any(|w| same_path(w, &b.cwd)), "重新登记上了");
        assert_eq!(text(&b.cwd.join("AGENTS.md")), "v2\n", "停在最新的集成分支上");
        assert!(!b.cwd.join("half.txt").exists() && !b.cwd.join("node_modules").exists(), "残留的内容不留");
        // 删到 `.git` 文件也没了
        std::fs::write(b.cwd.join("half.txt"), "x").unwrap();
        half_removed(&b.cwd);
        std::fs::remove_file(b.cwd.join(".git")).unwrap();
        let c = prepare(&r.repo, "win/codex", 3, "").unwrap();
        assert_eq!((c.cwd.clone(), c.stale.clone()), (b.cwd.clone(), Some((b.cwd.clone(), None))));
        assert!(unpushed(&c.cwd).is_none());
        // 下一次就是普通的接着用
        assert_eq!(prepare(&r.repo, "win/codex", 4, "").unwrap().stale, None);
    }

    /// 同名目录不是这个仓库的 worktree（自己就是一个仓库、或指向别的仓库）：不碰，照旧报错
    #[test]
    fn prepare_keeps_a_dir_that_is_not_a_leftover_of_this_repo() {
        let r = rig();
        let dir = worktree_path(&r.repo, "a/b").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        run(&dir, &["init", "-q"]);
        assert!(!leftover(&r.repo, &dir));
        assert!(matches!(prepare(&r.repo, "a/b", 1, "").unwrap_err(), Fail::Other(e) if e.contains("不是这个仓库的 worktree")));
        assert!(dir.join(".git").is_dir(), "没被删");
        // 指向别的仓库的 worktree
        let other = worktree_path(&r.repo, "c/d").unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join(".git"), format!("gitdir: {}\n", r.up.join(".git/worktrees/x").display())).unwrap();
        assert!(!leftover(&r.repo, &other));
    }

    /// 残留目录删不掉（Windows 上文件被占用；这里用只读目录模拟）：这次另开一个，agent 照常启动，写明原因
    #[cfg(unix)]
    #[test]
    fn prepare_opens_another_worktree_when_a_leftover_cannot_be_deleted() {
        use std::os::unix::fs::PermissionsExt;
        let r = rig();
        let a = prepare(&r.repo, "win/codex", 1, "").unwrap();
        let locked = a.cwd.join("node_modules");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("a.node"), "x").unwrap();
        half_removed(&a.cwd);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let b = prepare(&r.repo, "win/codex", 9, "");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let b = b.unwrap();
        assert_eq!(b.cwd, a.cwd.with_file_name(format!("{}-9", dir_name("win/codex"))));
        assert!(matches!(&b.stale, Some((p, Some(_))) if *p == a.cwd), "{:?}", b.stale);
        assert!(worktrees(&r.repo).iter().any(|w| same_path(w, &b.cwd)));
    }

    /// 拉不到（远端没了）和别的失败分开报；集成分支在远端没有时写明
    #[test]
    fn prepare_reports_fetch_failures_apart_from_other_failures() {
        let r = rig();
        assert_eq!(prepare(&r.repo, "a/b", 1, "release").unwrap_err(), Fail::Other("远端没有分支 release".into()));
        run(&r.repo, &["remote", "set-url", "origin", "/nonexistent/repo.git"]);
        assert!(matches!(prepare(&r.repo, "a/b", 1, "").unwrap_err(), Fail::Fetch(_)));
    }

    /// 半成品交接：没提交的文件和没推的提交一起推成 wip 分支，之后 worktree 算干净的；都推了时什么也不做；
    /// 那条分支上已经有别的内容时改推带机器名的分支
    #[test]
    fn save_wip_commits_and_pushes_whatever_was_not_pushed() {
        let r = rig();
        let wt = prepare(&r.repo, "a/b", 1, "").unwrap().cwd;
        assert_eq!(save_wip(&wt, "wip/main/7", "wip/main/7-mbp", "wip").unwrap(), None, "干净的 worktree 不用保存");
        // agent 自己提交过一次、没推，又留了两个没提交的文件
        std::fs::write(wt.join("done.txt"), "做完的一半").unwrap();
        run(&wt, &["add", "."]);
        run(&wt, &["commit", "-q", "-m", "做了一半"]);
        std::fs::write(wt.join("half.txt"), "还没提交").unwrap();
        std::fs::write(wt.join("AGENTS.md"), "改了\n").unwrap();
        let w = save_wip(&wt, "wip/main/7", "wip/main/7-mbp", "wip: default/main#7").unwrap().unwrap();
        assert_eq!(w.branch, "wip/main/7");
        assert!(w.changes.starts_with("2 个没提交的文件（") && w.changes.ends_with("，另有 1 个没推的提交"), "{}", w.changes);
        assert!(unpushed(&wt).is_none(), "推上去之后 worktree 算干净的，下一次派发可以接着用");
        // 远端的 wip 分支上三样都在
        let names = git(&r.up, &["fetch", "-q", "origin"], QUICK).and_then(|_| git(&r.up, &["ls-tree", "--name-only", "origin/wip/main/7"], QUICK)).unwrap();
        assert!(names.contains("done.txt") && names.contains("half.txt"), "{names}");
        assert_eq!(git(&r.up, &["log", "-1", "--format=%an|%s", "origin/wip/main/7"], QUICK).unwrap(), "board host|wip: default/main#7");
        // 另一台机器从集成分支另起了一份：那条分支推不进去，改推带机器名的
        let other = prepare(&r.repo, "c/d", 2, "").unwrap().cwd;
        std::fs::write(other.join("other.txt"), "另一份").unwrap();
        let w = save_wip(&other, "wip/main/7", "wip/main/7-win", "wip").unwrap().unwrap();
        assert_eq!(w.branch, "wip/main/7-win");
        assert_eq!(w.changes, "1 个没提交的文件（other.txt）");
    }

    /// 清理：board host 备的、超过时限没用的干净 worktree 删掉；正在用的、有没推改动的、不是 board host 备的都留着
    #[test]
    fn sweep_removes_only_clean_unused_worktrees_made_by_the_host() {
        let r = rig();
        let clean = prepare(&r.repo, "a/clean", 1, "").unwrap().cwd;
        let busy = prepare(&r.repo, "a/busy", 2, "").unwrap().cwd;
        let dirty = prepare(&r.repo, "a/dirty", 3, "").unwrap().cwd;
        std::fs::write(dirty.join("half.txt"), "x").unwrap();
        // 人手动跑的会话自己建的：没有使用记号
        let manual = r.repo.parent().unwrap().join("proj.wt").join("manual");
        run(&r.repo, &["worktree", "add", "-q", "--detach", manual.to_str().unwrap(), "origin/main"]);
        let later = SystemTime::now() + Duration::from_secs(25 * 3600);
        // 还没到时限：都不动
        assert!(sweep(&r.repo, &[busy.clone()], WT_UNUSED, SystemTime::now()).is_empty());
        let got = sweep(&r.repo, &[busy.clone()], WT_UNUSED, later);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.iter().any(|(p, w)| same_path(p, &clean) && *w == Swept::Removed) || !clean.exists());
        assert!(got.iter().any(|(p, w)| same_path(p, &dirty) && *w == Swept::Unpushed("1 个文件没提交".into())));
        assert!(!clean.exists() && busy.exists() && dirty.exists() && manual.exists());
        assert!(read_owner(&clean).is_none(), "删掉的 worktree 的所有者记录一起删");
    }

    /// 名字为空、格式不对：什么都不做——不拉、不建、更不删 `.wt` 根目录和里面别的会话的 worktree（#508）
    #[test]
    fn prepare_refuses_an_empty_or_bad_name_before_touching_anything() {
        let r = rig();
        let other = prepare(&r.repo, "a/b", 1, "").unwrap().cwd;
        std::fs::write(other.join("half.txt"), "别的会话没推的改动").unwrap();
        let root = other.parent().unwrap().to_path_buf();
        // 根目录里再放一个非 worktree 的东西：它没有 .git，旧版会把 `.wt` 根目录当残留删掉
        std::fs::write(root.join("cache.bin"), "x").unwrap();
        for bad in ["", "   ", "noslash", "A/B", "../x", "a/b/c"] {
            let e = prepare(&r.repo, bad, 2, "").unwrap_err();
            assert!(matches!(&e, Fail::Other(m) if m.contains("会话名不对")), "{bad:?}: {e:?}");
        }
        assert!(root.join("cache.bin").exists() && other.join("half.txt").exists(), ".wt 和里面的东西都在");
    }

    /// `a/b-c` 和 `a-b/c` 只换 `/` 时是同一个目录：现在各是各的，互不影响（#508）
    #[test]
    fn colliding_names_get_separate_worktrees() {
        let r = rig();
        assert_ne!(dir_name("a/b-c"), dir_name("a-b/c"));
        let x = prepare(&r.repo, "a/b-c", 1, "").unwrap().cwd;
        let y = prepare(&r.repo, "a-b/c", 2, "").unwrap().cwd;
        assert_ne!(x, y);
        std::fs::write(x.join("mine.txt"), "x 的改动").unwrap();
        let y2 = prepare(&r.repo, "a-b/c", 3, "").unwrap();
        assert_eq!((y2.cwd.clone(), y2.left.clone()), (y.clone(), None), "y 没看到 x 的改动，接着用自己的");
        assert_eq!(std::fs::read_to_string(x.join("mine.txt")).unwrap(), "x 的改动");
    }

    /// 像残留 worktree 的目录（`.git` 没了），但没有所有者记录、或者记录是别的会话的：不删，另开一个，写明原因；
    /// 有记录、对得上的半删除目录照样删掉重建（#508）
    #[test]
    fn leftover_without_matching_owner_record_is_kept() {
        let r = rig();
        // 旧版 board host 建的（没有记录）、删到一半：里面还有没推的改动
        let a = prepare(&r.repo, "win/codex", 1, "").unwrap();
        std::fs::write(a.cwd.join("half.txt"), "没推的改动").unwrap();
        half_removed(&a.cwd);
        std::fs::remove_file(a.cwd.join(".git")).unwrap();
        std::fs::remove_file(owner_file(&a.cwd).unwrap()).unwrap();
        let b = prepare(&r.repo, "win/codex", 5, "").unwrap();
        assert_eq!(b.cwd, a.cwd.with_file_name(format!("{}-5", dir_name("win/codex"))));
        assert!(matches!(&b.kept, Some((p, why)) if *p == a.cwd && why.contains("所有者记录")), "{:?}", b.kept);
        assert_eq!(b.stale, None);
        assert_eq!(std::fs::read_to_string(a.cwd.join("half.txt")).unwrap(), "没推的改动", "没删");
        // 记录是别的会话的：一样不删
        let o = Owner { repo: r.repo.clone(), path: a.cwd.clone(), name: "other/name".into(), dispatch: 9, created_at: 0 };
        std::fs::write(owner_file(&a.cwd).unwrap(), serde_json::to_string(&o).unwrap()).unwrap();
        let c = prepare(&r.repo, "win/codex", 6, "").unwrap();
        assert!(matches!(&c.kept, Some((_, why)) if why.contains("对不上")), "{:?}", c.kept);
        assert!(a.cwd.join("half.txt").exists());
        // 记录对得上：半删除的目录照样删掉重建
        let o = Owner { name: "win/codex".into(), ..o };
        std::fs::write(owner_file(&a.cwd).unwrap(), serde_json::to_string(&o).unwrap()).unwrap();
        let d = prepare(&r.repo, "win/codex", 7, "").unwrap();
        assert_eq!((d.cwd.clone(), d.stale.clone(), d.kept.clone()), (a.cwd.clone(), Some((a.cwd.clone(), None)), None));
        assert!(!a.cwd.join("half.txt").exists());
    }

    /// 清理：没有所有者记录的（旧版 board host 建的，有使用记号）超过时限也只报告、不删（#508）
    #[test]
    fn sweep_keeps_host_worktrees_without_owner_record() {
        let r = rig();
        let old = prepare(&r.repo, "a/old", 1, "").unwrap().cwd;
        std::fs::remove_file(owner_file(&old).unwrap()).unwrap();
        let later = SystemTime::now() + Duration::from_secs(25 * 3600);
        let got = sweep(&r.repo, &[], WT_UNUSED, later);
        assert!(matches!(got.as_slice(), [(p, Swept::NoOwner(why))] if same_path(p, &old) && why.contains("所有者记录")), "{got:?}");
        assert!(old.exists());
    }
}
