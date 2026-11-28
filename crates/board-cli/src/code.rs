//! 派发前把项目的代码准备好（PRD §9.1「派发前更新代码」）：board host 每次开始一张卡之前，先在项目目录里
//! `git fetch origin`，再给这个会话备好一个停在最新集成分支上的 worktree，agent 就在这个 worktree 里启动。
//!
//! 以前 agent 是在项目的主目录里启动、自己照 skill 的约定去建 worktree：主目录克隆一次之后没人更新，它启动时读到的
//! AGENTS.md、仓库里自带的 skill 可能是很久以前的。现在：
//!
//! - **拉最新**（[`fetch`]）：拉不下来（没权限、网络不通）就不启动 agent，原因报给服务端，这台机器先不接这个项目的卡。
//! - **worktree**（[`prepare`]，`board base` 用的是 [`session_worktree`]）：放在 `<仓库目录名>.wt/` 里，锁的原因写名字。
//!   名字为空、格式不对时不准备（#508）。按这个顺序找（#622）：
//!   1. 锁着这个名字的：接着用，切到最新的集成分支——依赖、编译缓存都还在。里面有没推的改动就不动它，往下找；
//!   2. **接手一个空闲的**（[`idle`]、[`adopt`]）：别的会话用完留下的，最近用过的优先，锁上这个名字。
//!      换了名字（换模型、并发加后缀）、换了会话也不用从头装依赖、从头编译；
//!   3. 都没有才新建：目录名是 `<名字，/ 换成 ->-<名字的哈希前 8 位>`（[`dir_name`]：`a/b-c` 和 `a-b/c` 不会落到
//!      同一个目录）；那个目录被占着（别的名字接手了、里面有没推的改动）时换一个目录名。
//!
//!   目录在、git 却不登记它（[`leftover`]：`git worktree remove` 删到一半，Windows 上文件被占用、路径太长时常见）：
//!   那一次的任务已经结束了，有所有者记录、而且记录对得上才删掉重建；删不掉、没有记录时这次另开一个，agent 照常启动。
//! - **空闲**：用 git 自带的 worktree 锁表示有没有人在用。没锁的，或者锁着、但锁的名字已经离线的，而且里面没有没推的
//!   改动，才算空闲。只认 board 建的（所有者记录对得上）：人手动建在 `.wt/` 下的不接手、不清理。
//!   会话结束时 worktree 是干净的就解锁（board host 是 [`release`]，手动的会话照 skill `git worktree unlock`）。
//! - **所有者记录**：board 建的每个 worktree 在 `<仓库目录名>.wt/.board-worktree/<目录名>.json` 记下主仓库、
//!   目录和会话名（被接手时改成接手的会话）。删目录（残留目录、清理）之前都要它在、对得上，否则只报告、不删（#508）。
//! - **清理**（[`sweep`]，board host 定时做，`board base` 每次也做）：空闲的 worktree 超过 36 小时没用就删掉；
//!   空闲的超过 4 个时从最久没用的删起。有没推改动的留着，没有所有者记录的（旧版 board host 建的）也留着、写明原因。
//!
//! 只在项目目录是 git 仓库、有 `origin` 时做；没有仓库的项目、拆解、WSL 里的 agent（worktree 要由 WSL 里的 git 来建）
//! 照旧在项目目录里启动。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

/// `git fetch` 最多等多久：board host 每 10 秒上报一次、50 秒没上报算掉线，拉取不能把它拖到掉线
pub const FETCH_LIMIT: Duration = Duration::from_secs(30);
/// 建、切换 worktree 最多等多久
pub const WT_LIMIT: Duration = Duration::from_secs(60);
/// 空闲的 worktree 多久没用就清理
pub const WT_UNUSED: Duration = Duration::from_secs(36 * 3600);
/// 空闲的 worktree 最多留几个：多出来的从最久没用的删起
pub const WT_IDLE_MAX: usize = 4;
/// 接手、清理一个 worktree 时占着它的记号文件多久算过期（进程中途被杀留下的）
const GUARD_STALE: Duration = Duration::from_secs(120);
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
    let mut child = crate::gitcmd::command()
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("没法运行 git：{e}"))?;
    let start = Instant::now();
    // 大多数 git 命令几毫秒就结束：从 1 毫秒起翻倍等，最多 40 毫秒，不让每次调用都白等 40 毫秒
    let mut nap = Duration::from_millis(1);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} 秒还没做完，已经停掉", limit.as_secs()));
            }
            Ok(None) => {
                std::thread::sleep(nap);
                nap = (nap * 2).min(Duration::from_millis(40));
            }
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

/// git 正停在一个没做完的操作里。`marker` 是 git 的标记文件，`what` 是操作名，`resolve` 是继续 / 放弃的命令
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InProgress {
    pub marker: &'static str,
    pub what: &'static str,
    pub resolve: &'static str,
}

/// 检查 rebase、merge、cherry-pick 是不是停在中途。用 `git rev-parse --git-path` 找标记，
/// worktree（`.git` 是文件）里也能找到正确的位置
pub fn in_progress(dir: &Path) -> Result<Option<InProgress>, String> {
    const MARKERS: [(&str, &str, &str); 4] = [
        ("rebase-merge", "rebase", "git rebase --continue 或 git rebase --abort"),
        ("rebase-apply", "rebase", "git rebase --continue 或 git rebase --abort"),
        ("MERGE_HEAD", "merge", "git merge --continue 或 git merge --abort"),
        ("CHERRY_PICK_HEAD", "cherry-pick", "git cherry-pick --continue 或 git cherry-pick --abort"),
    ];
    for (marker, what, resolve) in MARKERS {
        let path = git(dir, &["rev-parse", "--git-path", marker], QUICK)?;
        if dir.join(path).exists() {
            return Ok(Some(InProgress { marker, what, resolve }));
        }
    }
    Ok(None)
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
    } else if let Ok(head) = git(repo, &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"], QUICK) {
        head.strip_prefix("refs/remotes/origin/").or_else(|| head.strip_prefix("origin/")).unwrap_or(&head).to_string()
    } else {
        git(repo, &["branch", "--show-current"], QUICK).ok().filter(|b| !b.is_empty()).ok_or("认不出集成分支：在项目设置里填上")?
    };
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")], QUICK).map_err(|_| format!("远端没有分支 {branch}"))?;
    Ok(branch)
}

/// git 内部用的完整远端引用：`origin/main` → `refs/remotes/origin/main`。
/// 短名可能被同名的本地分支顶替，内部命令一律用完整引用。
fn remote_ref(branch: &str) -> String {
    if branch.starts_with("refs/") {
        branch.to_string()
    } else if branch.starts_with("origin/") {
        format!("refs/remotes/{branch}")
    } else {
        format!("refs/remotes/origin/{branch}")
    }
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
}

fn owner_file(wt: &Path) -> Option<PathBuf> {
    Some(wt.parent()?.join(OWNERS).join(format!("{}.json", wt.file_name()?.to_string_lossy())))
}

fn read_owner(wt: &Path) -> Option<Owner> {
    serde_json::from_str(&std::fs::read_to_string(owner_file(wt)?).ok()?).ok()
}

/// 记下这个 worktree 归 board host 的这个会话（已经有、而且是同一个仓库同一个目录时不改）
fn write_owner(repo: &Path, wt: &Path, name: &str) {
    let (Some(main), Some(file)) = (main_repo(repo), owner_file(wt)) else { return };
    if read_owner(wt).is_some_and(|o| same_path(&o.repo, &main) && same_path(&o.path, wt) && o.name == name) {
        return;
    }
    let o = Owner { repo: main, path: wt.to_path_buf(), name: name.to_string() };
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
#[cfg(test)]
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

/// 这个 worktree 上一次被 board 用是什么时候；不是 board 备的（没有记号）时为空
fn used_at(wt: &Path) -> Option<SystemTime> {
    let dir = git(wt, &["rev-parse", "--absolute-git-dir"], QUICK).ok()?;
    std::fs::metadata(Path::new(&dir).join(MARK)).ok()?.modified().ok()
}

/// 这把锁算不算「有人在用」。没锁：没人用。锁的原因是一个会话名、而且这个名字已经离线：会话崩溃、被杀之后留下的，
/// 没人用。其余都算有人在用：名字在线；不知道谁在线（`online` 为空：连不上服务端）；原因不是会话名（人自己锁的）
fn held(lock: &Option<String>, online: Option<&[String]>) -> bool {
    match lock {
        None => false,
        Some(reason) => !(board_core::model::check_agent_name(reason).is_ok() && online.is_some_and(|o| !o.iter().any(|n| n == reason))),
    }
}

/// `wt` 直接在放 worktree 的目录 `root` 下面
fn under(root: &Path, wt: &Path) -> bool {
    wt.parent().is_some_and(|p| same_path(p, root))
}

/// 空闲的 worktree，最近用过的在前：board 建的（所有者记录对得上）、没人在用（见 [`held`]）、里面没有没推的改动。
/// `in_use` 是调用方自己知道正在用的（board host 正在跑的会话），`online` 是服务端上在线的名字
fn idle(repo: &Path, in_use: &[PathBuf], online: Option<&[String]>) -> Vec<(PathBuf, SystemTime)> {
    let Some(root) = wt_root(repo) else { return vec![] };
    let mut out = vec![];
    for (wt, lock) in worktree_locks(repo) {
        if !under(&root, &wt) || in_use.iter().any(|p| same_path(p, &wt)) || held(&lock, online) {
            continue;
        }
        // 目录没了（登记还在）、没有对得上的所有者记录、有没推的改动：都不算
        if !wt.join(".git").is_file() || owned(repo, &wt, None).is_err() || unpushed(&wt).is_some() {
            continue;
        }
        out.push((wt.clone(), used_at(&wt).unwrap_or(SystemTime::UNIX_EPOCH)));
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// 接手、清理一个 worktree 期间占着它：`<仓库目录名>.wt/.board-worktree/<目录名>.busy` 只有一个进程建得出来
/// （`create_new`），别的进程换下一个。`git worktree lock` 自己是先看再写，两个进程同时锁同一个都会成功，所以要这一层。
/// 进程中途被杀留下的文件过了 [`GUARD_STALE`] 不算数
struct Guard(PathBuf);

impl Guard {
    fn take(wt: &Path) -> Option<Guard> {
        let file = owner_file(wt)?.with_extension("busy");
        if std::fs::metadata(&file).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default() > GUARD_STALE) {
            let _ = std::fs::remove_file(&file);
        }
        std::fs::create_dir_all(file.parent()?).ok()?;
        std::fs::OpenOptions::new().write(true).create_new(true).open(&file).ok()?;
        Some(Guard(file))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 接手一个空闲的 worktree：锁上 `name`（原来锁着离线名字的先解锁）、所有者记录改成 `name`、记下使用时间。
/// 没接手成（别的会话先拿走了、已经不空闲了）返回 false，调用方换下一个
fn adopt(repo: &Path, wt: &Path, name: &str, online: Option<&[String]>) -> bool {
    let Some(_guard) = Guard::take(wt) else { return false };
    // 占住之后再看一眼：从列出空闲的到现在，它可能已经被别的会话锁上了
    let Some((_, lock)) = worktree_locks(repo).into_iter().find(|(q, _)| same_path(q, wt)) else { return false };
    if held(&lock, online) || unpushed(wt).is_some() {
        return false;
    }
    let path = wt.to_string_lossy().to_string();
    if lock.is_some() && git(repo, &["worktree", "unlock", &path], QUICK).is_err() {
        return false;
    }
    if git(repo, &["worktree", "lock", "--reason", name, &path], QUICK).is_err() {
        return false;
    }
    write_owner(repo, wt, name);
    touch(wt);
    true
}

/// 会话结束：worktree 里没有没推的改动时解锁，留给下一个会话接手；有的话原样留着（还锁着，不会被接手、清理）。
/// 返回有没有解锁
pub fn release(wt: &Path) -> bool {
    if unpushed(wt).is_some() || !matches!(in_progress(wt), Ok(None)) {
        return false;
    }
    touch(wt);
    git(wt, &["worktree", "unlock", &wt.to_string_lossy()], QUICK).is_ok()
}

/// 没被占用的目录名：`first` 后面加 `-2`、`-3`……
fn spare(first: &Path) -> PathBuf {
    let base = first.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    (2..).map(|i| first.with_file_name(format!("{base}-{i}"))).find(|p| !p.exists()).unwrap_or_else(|| first.to_path_buf())
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
    /// 这次是接手了别的会话留下的空闲 worktree
    pub adopted: bool,
}

/// `board base` 拿到的 worktree 是怎么来的
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Got {
    /// 本来就锁着这个名字：接着用
    Mine,
    /// 接手了一个空闲的
    Adopted,
    /// 新建的
    Created,
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

/// 拉最新，备好名字是 `name` 的会话的 worktree，停在最新的集成分支上：锁着这个名字的接着用，没有就接手一个空闲的，
/// 都没有才新建。`id` 是派发 ID（另开 worktree 时用它起名）；`in_use` 是 board host 正在跑的会话用着的，
/// `online` 是服务端上在线的名字（判断空闲用，见 [`held`]）。名字为空、格式不对时什么都不做（不拉、不建、不删）
pub fn prepare(repo: &Path, name: &str, id: i64, want_branch: &str, in_use: &[PathBuf], online: Option<&[String]>) -> Result<Ready, Fail> {
    board_core::model::check_agent_name(name).map_err(|e| Fail::Other(format!("会话名不对，不准备 worktree：{e}")))?;
    fetch(repo).map_err(Fail::Fetch)?;
    let branch = integration_branch(repo, want_branch).map_err(Fail::Other)?;
    let tip = remote_ref(&branch);
    let tip_name = format!("origin/{branch}");
    let first = worktree_path(repo, name).ok_or_else(|| Fail::Other("找不到主仓库目录".into()))?;
    let root = wt_root(repo).ok_or_else(|| Fail::Other("找不到主仓库目录".into()))?;
    let locks = worktree_locks(repo);
    let registered = |p: &Path| locks.iter().any(|(k, _)| same_path(k, p));
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
    // 1. 锁着这个名字的（按名字建的那个目录排前面）：干净的接着用；有上一次没推的改动就不动它，往下找
    let mut mine: Vec<&PathBuf> = locks.iter().filter(|(q, r)| r.as_deref() == Some(name) && under(&root, q) && q.exists()).map(|(q, _)| q).collect();
    mine.sort_by_key(|q| !same_path(q, &first));
    let mut found = None;
    for wt in mine {
        match unpushed(wt) {
            Some(what) => left = left.or(Some((wt.clone(), what))),
            None => {
                found = Some(wt.clone());
                break;
            }
        }
    }
    // 2. 接手一个空闲的，最近用过的优先
    let mut adopted = false;
    if found.is_none() {
        found = idle(repo, in_use, online).into_iter().map(|(wt, _)| wt).find(|wt| adopt(repo, wt, name, online));
        adopted = found.is_some();
    }
    match found {
        Some(wt) => {
            git(&wt, &["checkout", "--quiet", "--detach", &tip], WT_LIMIT).map_err(|e| Fail::Other(format!("{} 切不到 {tip_name}：{e}", wt.display())))?;
            target = wt;
        }
        // 3. 新建。按名字算出的目录被占着（别的名字接手了、里面有没推的改动）时另开一个
        None => {
            if target.exists() {
                target = another.clone();
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Fail::Other(format!("无法创建 {}：{e}", parent.display())))?;
            }
            let path = target.to_string_lossy().to_string();
            git(repo, &["worktree", "add", "--quiet", "--detach", "--lock", "--reason", name, &path, &tip], WT_LIMIT)
                .map_err(|e| Fail::Other(format!("建不了 worktree {path}：{e}")))?;
        }
    }
    touch(&target);
    write_owner(repo, &target, name);
    Ok(Ready { cwd: target, branch, left, stale, kept, adopted })
}

/// 这个仓库登记着的 worktree 和锁的原因：（路径，锁的原因）。没锁时为 None，锁了没写原因时是空字符串
pub fn worktree_locks(repo: &Path) -> Vec<(PathBuf, Option<String>)> {
    let text = git(repo, &["worktree", "list", "--porcelain"], QUICK).unwrap_or_default();
    let mut out: Vec<(PathBuf, Option<String>)> = vec![];
    for l in text.lines() {
        if let Some(p) = l.strip_prefix("worktree ") {
            out.push((PathBuf::from(p), None));
        } else if l == "locked" || l.starts_with("locked ") {
            if let Some(last) = out.last_mut() {
                last.1 = Some(l.strip_prefix("locked").unwrap_or_default().trim().to_string());
            }
        }
    }
    out
}

/// `board base` 用的会话 worktree：返回（路径，怎么来的）。不切换、不查干不干净，见 base.rs。
/// - 当前目录（`here`）所在的 worktree 锁着这个名字（board host 备好的、skill 以前按名字建的）：用它；
/// - `<仓库目录名>.wt/` 里有锁着这个名字的：接着用；
/// - 接手一个空闲的（[`idle`]），最近用过的优先；`online` 是服务端上在线的名字；
/// - 都没有：建在 [`worktree_path`]，锁的原因、所有者记录都和 [`prepare`] 一样。那里有残留目录、而且所有者记录
///   证明是这个会话的就删掉重建；被别的占着（别的名字接手了、不是这个仓库的 worktree）时换一个目录名
pub fn session_worktree(repo: &Path, here: &Path, name: &str, tip: &str, online: Option<&[String]>) -> Result<(PathBuf, Got), String> {
    board_core::model::check_agent_name(name).map_err(|e| format!("名字不对，不准备 worktree：{e}"))?;
    let root = wt_root(repo).ok_or("找不到主仓库目录")?;
    let tip = remote_ref(tip);
    let mut locks = worktree_locks(repo);
    let stale: Vec<PathBuf> = locks.iter().filter(|(q, r)| r.as_deref() == Some(name) && under(&root, q) && !q.exists()).map(|(q, _)| q.clone()).collect();
    if !stale.is_empty() {
        for p in &stale {
            let path = p.to_string_lossy().to_string();
            git(repo, &["worktree", "unlock", &path], QUICK).map_err(|e| format!("解不开已经不存在的 worktree {}：{e}", p.display()))?;
        }
        git(repo, &["worktree", "prune"], QUICK).map_err(|e| format!("清不掉已经不存在的 worktree 登记：{e}"))?;
        locks = worktree_locks(repo);
    }
    let mine = |p: &Path| locks.iter().any(|(q, r)| r.as_deref() == Some(name) && q.exists() && same_path(q, p));
    let found = git(here, &["rev-parse", "--show-toplevel"], QUICK)
        .ok()
        .map(PathBuf::from)
        .filter(|top| mine(top))
        .or_else(|| locks.iter().find(|(q, r)| r.as_deref() == Some(name) && under(&root, q) && q.exists()).map(|(q, _)| q.clone()));
    if let Some(wt) = found {
        touch(&wt);
        return Ok((wt, Got::Mine));
    }
    if let Some(wt) = idle(repo, &[], online).into_iter().map(|(wt, _)| wt).find(|wt| adopt(repo, wt, name, online)) {
        return Ok((wt, Got::Adopted));
    }
    let mut target = worktree_path(repo, name).ok_or("找不到主仓库目录")?;
    if target.exists() {
        let registered = locks.iter().any(|(q, _)| same_path(q, &target));
        if !registered && leftover(repo, &target) && owned(repo, &target, Some(name)).is_ok() {
            std::fs::remove_dir_all(&target).map_err(|e| format!("删不掉残留的 worktree 目录 {}：{e}", target.display()))?;
            let _ = git(repo, &["worktree", "prune"], QUICK);
        } else {
            target = spare(&target);
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建 {}：{e}", parent.display()))?;
    }
    let path = target.to_string_lossy().to_string();
    git(repo, &["worktree", "add", "--quiet", "--detach", "--lock", "--reason", name, &path, &tip], WT_LIMIT).map_err(|e| format!("建不了 worktree {path}：{e}"))?;
    touch(&target);
    write_owner(repo, &target, name);
    Ok((target, Got::Created))
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
    if let Some(ip) = in_progress(wt)? {
        return Err(format!("worktree 还在 {} 中（{}）：先 {}，没有提交、没有推送，锁也没有解", ip.what, ip.marker, ip.resolve));
    }
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
    if git(wt, &["push", "--quiet", "--no-verify", "origin", &format!("HEAD:refs/heads/{branch}")], FETCH_LIMIT).is_err() {
        used = alt.to_string();
        git(wt, &["push", "--quiet", "--no-verify", "--force", "origin", &format!("HEAD:refs/heads/{alt}")], FETCH_LIMIT)?;
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
    /// 超过时限没用，删掉了
    Removed,
    /// 空闲的太多，这个最久没用，删掉了
    Extra,
    /// 有没推到远端的改动，留着：留着什么
    Unpushed(String),
    /// 没有对得上的所有者记录（旧版 board host 建的），留着：为什么，确认没用后由人删
    NoOwner(String),
}

/// 删掉一个空闲的 worktree（连同所有者记录）。别的进程正在接手、清理它时不删
fn remove(repo: &Path, wt: &Path) -> bool {
    let Some(_guard) = Guard::take(wt) else { return false };
    let path = wt.to_string_lossy().to_string();
    let _ = git(repo, &["worktree", "unlock", &path], QUICK);
    if git(repo, &["worktree", "remove", &path], WT_LIMIT).is_err() {
        return false;
    }
    if let Some(f) = owner_file(wt) {
        let _ = std::fs::remove_file(f);
    }
    true
}

/// 清理 board 备的 worktree，board host 定时做、`board base` 每次也做，规则一样：
/// - 超过 `older` 没用的：空闲的删掉；有没推改动的、没有对得上的所有者记录的留着并报告；
/// - 剩下空闲的超过 `keep` 个：从最久没用的删起，删到剩 `keep` 个。
///
/// 有人在用的不碰：`in_use`（调用方自己正在用的）、锁着在线名字的（`online`，见 [`held`]）。
/// 没有使用记号的（人手动建在 `.wt/` 下的）也不碰
pub fn sweep(repo: &Path, in_use: &[PathBuf], online: Option<&[String]>, older: Duration, keep: usize, now: SystemTime) -> Vec<(PathBuf, Swept)> {
    let Some(root) = wt_root(repo) else { return vec![] };
    let mut out = vec![];
    let mut fresh: Vec<(PathBuf, SystemTime)> = vec![];
    for (wt, lock) in worktree_locks(repo) {
        if !under(&root, &wt) || in_use.iter().any(|p| same_path(p, &wt)) || held(&lock, online) {
            continue;
        }
        let Some(at) = used_at(&wt) else { continue };
        let old = now.duration_since(at).unwrap_or_default() >= older;
        if let Err(why) = owned(repo, &wt, None) {
            if old {
                out.push((wt, Swept::NoOwner(why)));
            }
            continue;
        }
        if let Some(what) = unpushed(&wt) {
            if old {
                out.push((wt, Swept::Unpushed(what)));
            }
            continue;
        }
        if !old {
            fresh.push((wt, at));
        } else if remove(repo, &wt) {
            out.push((wt, Swept::Removed));
        }
    }
    fresh.sort_by(|a, b| b.1.cmp(&a.1));
    // 最近用过的 `keep` 个留着，其余从最久没用的删起
    for (wt, _) in fresh.into_iter().skip(keep).rev() {
        if remove(repo, &wt) {
            out.push((wt, Swept::Extra));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let ok = crate::gitcmd::command()
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

    /// 不知道谁在线、没有正在跑的会话时备 worktree：锁着的一律当成有人在用
    fn prep(repo: &Path, name: &str, id: i64, branch: &str) -> Result<Ready, Fail> {
        prepare(repo, name, id, branch, &[], None)
    }

    /// 谁都不在线
    const NOBODY: Option<&[String]> = Some(&[]);

    /// 把 worktree 的使用记号改成 `ago` 之前
    fn used(wt: &Path, ago: Duration) {
        let dir = git(wt, &["rev-parse", "--absolute-git-dir"], QUICK).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(Path::new(&dir).join(MARK)).unwrap();
        f.set_modified(SystemTime::now() - ago).unwrap();
    }

    fn lock_of(r: &Rig, wt: &Path) -> Option<String> {
        worktree_locks(&r.repo).into_iter().find(|(q, _)| same_path(q, wt)).and_then(|(_, l)| l)
    }

    /// rebase、merge、cherry-pick 的标记都能认出来（`--git-path` 在 worktree 里也指向对的地方）
    #[test]
    fn in_progress_detects_the_git_markers() {
        let r = rig();
        assert_eq!(in_progress(&r.repo).unwrap(), None);
        let markers = [
            ("rebase-merge", "rebase", true),
            ("rebase-apply", "rebase", true),
            ("MERGE_HEAD", "merge", false),
            ("CHERRY_PICK_HEAD", "cherry-pick", false),
        ];
        for (marker, what, dir) in markers {
            let path = r.repo.join(git(&r.repo, &["rev-parse", "--git-path", marker], QUICK).unwrap());
            if dir {
                std::fs::create_dir_all(&path).unwrap();
            } else {
                std::fs::write(&path, "x").unwrap();
            }
            let got = in_progress(&r.repo).unwrap().unwrap();
            assert_eq!((got.marker, got.what), (marker, what), "{marker}");
            if dir {
                std::fs::remove_dir_all(&path).unwrap();
            } else {
                std::fs::remove_file(&path).unwrap();
            }
            assert_eq!(in_progress(&r.repo).unwrap(), None);
        }
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
        let a = prep(&r.repo, "claude-mbp/opus-5.5", 1, "").unwrap();
        assert_eq!(a.cwd, r.repo.parent().unwrap().join("proj.wt").join(dir_name("claude-mbp/opus-5.5")));
        assert!(dir_name("claude-mbp/opus-5.5").starts_with("claude-mbp-opus-5.5-"), "可读的前缀加哈希");
        let o = read_owner(&a.cwd).expect("建好时写了所有者记录");
        assert_eq!(o.name, "claude-mbp/opus-5.5");
        assert!(same_path(&o.repo, &r.repo) && same_path(&o.path, &a.cwd));
        assert_eq!(a.branch, "main", "没写集成分支时用远端的默认分支");
        assert_eq!(text(&a.cwd.join("AGENTS.md")), "v2\n");
        assert_eq!(text(&r.repo.join("AGENTS.md")), "v1\n", "主目录不动");
        // 依赖、编译缓存这类没被跟踪、被忽略的文件留着
        std::fs::write(a.cwd.join(".git-ignored-cache"), "x").unwrap();
        std::fs::write(a.cwd.join(".gitignore"), ".git-ignored-cache\n.gitignore\n").unwrap();
        push(&r, "v3\n");
        let b = prep(&r.repo, "claude-mbp/opus-5.5", 2, "main").unwrap();
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
        let a = prep(&r.repo, "dsh-win/x", 1, "").unwrap();
        std::fs::write(a.cwd.join("half.txt"), "做到一半").unwrap();
        let b = prep(&r.repo, "dsh-win/x", 7, "").unwrap();
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
        let a = prep(&r.repo, "win/codex", 1, "").unwrap();
        std::fs::create_dir_all(a.cwd.join("node_modules/pkg")).unwrap();
        std::fs::write(a.cwd.join("node_modules/pkg/a.node"), "x").unwrap();
        std::fs::write(a.cwd.join("half.txt"), "失败那次留下的").unwrap();
        std::fs::remove_file(a.cwd.join("AGENTS.md")).unwrap();
        half_removed(&a.cwd);
        assert!(!worktrees(&r.repo).iter().any(|w| same_path(w, &a.cwd)), "git 已经不登记它");
        assert!(leftover(&r.repo, &a.cwd));
        push(&r, "v2\n");
        let b = prep(&r.repo, "win/codex", 2, "").unwrap();
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
        let c = prep(&r.repo, "win/codex", 3, "").unwrap();
        assert_eq!((c.cwd.clone(), c.stale.clone()), (b.cwd.clone(), Some((b.cwd.clone(), None))));
        assert!(unpushed(&c.cwd).is_none());
        // 下一次就是普通的接着用
        assert_eq!(prep(&r.repo, "win/codex", 4, "").unwrap().stale, None);
    }

    /// 同名目录不是这个仓库的 worktree（自己就是一个仓库、或指向别的仓库）：不碰，照旧报错
    #[test]
    fn prepare_keeps_a_dir_that_is_not_a_leftover_of_this_repo() {
        let r = rig();
        let dir = worktree_path(&r.repo, "a/b").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        run(&dir, &["init", "-q"]);
        assert!(!leftover(&r.repo, &dir));
        assert!(matches!(prep(&r.repo, "a/b", 1, "").unwrap_err(), Fail::Other(e) if e.contains("不是这个仓库的 worktree")));
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
        let a = prep(&r.repo, "win/codex", 1, "").unwrap();
        let locked = a.cwd.join("node_modules");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("a.node"), "x").unwrap();
        half_removed(&a.cwd);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let b = prep(&r.repo, "win/codex", 9, "");
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
        assert_eq!(prep(&r.repo, "a/b", 1, "release").unwrap_err(), Fail::Other("远端没有分支 release".into()));
        run(&r.repo, &["remote", "set-url", "origin", "/nonexistent/repo.git"]);
        assert!(matches!(prep(&r.repo, "a/b", 1, "").unwrap_err(), Fail::Fetch(_)));
    }

    /// 半成品交接：没提交的文件和没推的提交一起推成 wip 分支，之后 worktree 算干净的；都推了时什么也不做；
    /// 那条分支上已经有别的内容时改推带机器名的分支
    #[test]
    fn save_wip_commits_and_pushes_whatever_was_not_pushed() {
        let r = rig();
        let wt = prep(&r.repo, "a/b", 1, "").unwrap().cwd;
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
        let other = prep(&r.repo, "c/d", 2, "").unwrap().cwd;
        std::fs::write(other.join("other.txt"), "另一份").unwrap();
        let w = save_wip(&other, "wip/main/7", "wip/main/7-win", "wip").unwrap().unwrap();
        assert_eq!(w.branch, "wip/main/7-win");
        assert_eq!(w.changes, "1 个没提交的文件（other.txt）");
    }

    /// 停在 merge 中途时半成品不能提交、推送，锁也不解；先 --continue / --abort 再来
    #[test]
    fn save_wip_refuses_while_git_is_in_progress() {
        let r = rig();
        let wt = prep(&r.repo, "a/b", 1, "").unwrap().cwd;
        std::fs::write(wt.join("half.txt"), "还没提交").unwrap();
        let marker = wt.join(git(&wt, &["rev-parse", "--git-path", "MERGE_HEAD"], QUICK).unwrap());
        std::fs::write(&marker, "x").unwrap();
        let before = git(&wt, &["rev-parse", "HEAD"], QUICK).unwrap();
        let err = save_wip(&wt, "wip/main/7", "wip/main/7-mbp", "wip").unwrap_err();
        assert!(err.contains("merge") && err.contains("git merge --continue") && err.contains("git merge --abort"), "{err}");
        assert_eq!(git(&wt, &["rev-parse", "HEAD"], QUICK).unwrap(), before, "没有提交");
        assert!(git(&r.up, &["rev-parse", "--verify", "--quiet", "refs/heads/wip/main/7"], QUICK).is_err(), "没有推送");
        assert!(!release(&wt), "锁留着，不会被接手");
        assert_eq!(lock_of(&r, &wt).as_deref(), Some("a/b"));
        std::fs::remove_file(marker).unwrap();
        let w = save_wip(&wt, "wip/main/7", "wip/main/7-mbp", "wip").unwrap().unwrap();
        assert_eq!(w.branch, "wip/main/7");
    }

    /// 清理：board host 备的、超过时限没用的干净 worktree 删掉；正在用的、有没推改动的、不是 board host 备的都留着
    #[test]
    fn sweep_removes_only_clean_unused_worktrees_made_by_the_host() {
        let r = rig();
        let clean = prep(&r.repo, "a/clean", 1, "").unwrap().cwd;
        let busy = prep(&r.repo, "a/busy", 2, "").unwrap().cwd;
        let dirty = prep(&r.repo, "a/dirty", 3, "").unwrap().cwd;
        std::fs::write(dirty.join("half.txt"), "x").unwrap();
        // 人手动跑的会话自己建的：没有使用记号
        let manual = r.repo.parent().unwrap().join("proj.wt").join("manual");
        run(&r.repo, &["worktree", "add", "-q", "--detach", manual.to_str().unwrap(), "origin/main"]);
        let later = SystemTime::now() + Duration::from_secs(37 * 3600);
        // 还没到时限：都不动
        assert!(sweep(&r.repo, &[busy.clone()], NOBODY, WT_UNUSED, WT_IDLE_MAX, SystemTime::now()).is_empty());
        // 不知道谁在线：锁着的一律当成有人在用，不删
        assert!(sweep(&r.repo, &[busy.clone()], None, WT_UNUSED, WT_IDLE_MAX, later).is_empty());
        let got = sweep(&r.repo, &[busy.clone()], NOBODY, WT_UNUSED, WT_IDLE_MAX, later);
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
        let other = prep(&r.repo, "a/b", 1, "").unwrap().cwd;
        std::fs::write(other.join("half.txt"), "别的会话没推的改动").unwrap();
        let root = other.parent().unwrap().to_path_buf();
        // 根目录里再放一个非 worktree 的东西：它没有 .git，旧版会把 `.wt` 根目录当残留删掉
        std::fs::write(root.join("cache.bin"), "x").unwrap();
        for bad in ["", "   ", "noslash", "A/B", "../x", "a/b/c"] {
            let e = prep(&r.repo, bad, 2, "").unwrap_err();
            assert!(matches!(&e, Fail::Other(m) if m.contains("会话名不对")), "{bad:?}: {e:?}");
        }
        assert!(root.join("cache.bin").exists() && other.join("half.txt").exists(), ".wt 和里面的东西都在");
    }

    /// `a/b-c` 和 `a-b/c` 只换 `/` 时是同一个目录：现在各是各的，互不影响（#508）
    #[test]
    fn colliding_names_get_separate_worktrees() {
        let r = rig();
        assert_ne!(dir_name("a/b-c"), dir_name("a-b/c"));
        let x = prep(&r.repo, "a/b-c", 1, "").unwrap().cwd;
        let y = prep(&r.repo, "a-b/c", 2, "").unwrap().cwd;
        assert_ne!(x, y);
        std::fs::write(x.join("mine.txt"), "x 的改动").unwrap();
        let y2 = prep(&r.repo, "a-b/c", 3, "").unwrap();
        assert_eq!((y2.cwd.clone(), y2.left.clone()), (y.clone(), None), "y 没看到 x 的改动，接着用自己的");
        assert_eq!(std::fs::read_to_string(x.join("mine.txt")).unwrap(), "x 的改动");
    }

    /// 像残留 worktree 的目录（`.git` 没了），但没有所有者记录、或者记录是别的会话的：不删，另开一个，写明原因；
    /// 有记录、对得上的半删除目录照样删掉重建（#508）
    #[test]
    fn leftover_without_matching_owner_record_is_kept() {
        let r = rig();
        // 旧版 board host 建的（没有记录）、删到一半：里面还有没推的改动
        let a = prep(&r.repo, "win/codex", 1, "").unwrap();
        std::fs::write(a.cwd.join("half.txt"), "没推的改动").unwrap();
        half_removed(&a.cwd);
        std::fs::remove_file(a.cwd.join(".git")).unwrap();
        std::fs::remove_file(owner_file(&a.cwd).unwrap()).unwrap();
        let b = prep(&r.repo, "win/codex", 5, "").unwrap();
        assert_eq!(b.cwd, a.cwd.with_file_name(format!("{}-5", dir_name("win/codex"))));
        assert!(matches!(&b.kept, Some((p, why)) if *p == a.cwd && why.contains("所有者记录")), "{:?}", b.kept);
        assert_eq!(b.stale, None);
        assert_eq!(std::fs::read_to_string(a.cwd.join("half.txt")).unwrap(), "没推的改动", "没删");
        // 记录是别的会话的：一样不删
        let o = Owner { repo: r.repo.clone(), path: a.cwd.clone(), name: "other/name".into() };
        std::fs::write(owner_file(&a.cwd).unwrap(), serde_json::to_string(&o).unwrap()).unwrap();
        let c = prep(&r.repo, "win/codex", 6, "").unwrap();
        assert!(matches!(&c.kept, Some((_, why)) if why.contains("对不上")), "{:?}", c.kept);
        assert!(a.cwd.join("half.txt").exists());
        // 记录对得上：半删除的目录照样删掉；这个名字已经有另开的那个（锁着它），接着用那个
        let o = Owner { name: "win/codex".into(), ..o };
        std::fs::write(owner_file(&a.cwd).unwrap(), serde_json::to_string(&o).unwrap()).unwrap();
        let d = prep(&r.repo, "win/codex", 7, "").unwrap();
        assert_eq!((d.cwd.clone(), d.stale.clone(), d.kept.clone()), (b.cwd.clone(), Some((a.cwd.clone(), None)), None));
        assert!(!a.cwd.exists());
    }

    /// 清理：没有所有者记录的（旧版 board host 建的，有使用记号）超过时限也只报告、不删（#508）
    #[test]
    fn sweep_keeps_host_worktrees_without_owner_record() {
        let r = rig();
        let old = prep(&r.repo, "a/old", 1, "").unwrap().cwd;
        std::fs::remove_file(owner_file(&old).unwrap()).unwrap();
        let later = SystemTime::now() + Duration::from_secs(37 * 3600);
        let got = sweep(&r.repo, &[], NOBODY, WT_UNUSED, WT_IDLE_MAX, later);
        assert!(matches!(got.as_slice(), [(p, Swept::NoOwner(why))] if same_path(p, &old) && why.contains("所有者记录")), "{got:?}");
        assert!(old.exists());
    }

    /// 用完留着、下一个会话接手（#622）：会话结束解锁之后，另一个名字的会话拿到的是同一个 worktree——锁和所有者记录
    /// 换成它，停在最新的集成分支上，依赖、构建产物（被忽略的文件）都还在
    #[test]
    fn prepare_adopts_an_idle_worktree_left_by_another_session() {
        let r = rig();
        let a = prep(&r.repo, "mbp/opus-5.5", 1, "").unwrap();
        assert!(!a.adopted);
        std::fs::create_dir_all(a.cwd.join("target/debug")).unwrap();
        std::fs::write(a.cwd.join("target/debug/built"), "x").unwrap();
        let exclude = PathBuf::from(git(&a.cwd, &["rev-parse", "--path-format=absolute", "--git-path", "info/exclude"], QUICK).unwrap());
        std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
        std::fs::write(&exclude, "target/\n").unwrap();
        assert!(release(&a.cwd), "干净的 worktree 结束时解锁");
        assert_eq!(lock_of(&r, &a.cwd), None);
        push(&r, "v2\n");
        let b = prep(&r.repo, "mbp/gpt-6.1-high", 2, "").unwrap();
        assert_eq!((b.cwd.clone(), b.adopted), (a.cwd.clone(), true), "接手 A 留下的，不新建");
        assert_eq!(lock_of(&r, &b.cwd).as_deref(), Some("mbp/gpt-6.1-high"));
        assert_eq!(read_owner(&b.cwd).unwrap().name, "mbp/gpt-6.1-high");
        assert_eq!(text(&b.cwd.join("AGENTS.md")), "v2\n");
        assert!(b.cwd.join("target/debug/built").exists(), "构建产物还在");
        // A 这个名字再来：自己原来的目录被 B 占着，没有空闲的，另开一个
        let c = prep(&r.repo, "mbp/opus-5.5", 3, "").unwrap();
        assert_eq!((c.cwd.clone(), c.adopted), (a.cwd.with_file_name(format!("{}-3", dir_name("mbp/opus-5.5"))), false));
    }

    /// 哪些算空闲：没锁的；锁着、名字已经离线的（会话崩溃留下的）。锁着在线名字的、不知道谁在线时锁着的、
    /// 有没推改动的、没有所有者记录的（人手动建在 `.wt/` 下的）、调用方正在用的都不接手
    #[test]
    fn only_idle_board_worktrees_are_adopted() {
        let r = rig();
        let crashed = prep(&r.repo, "a/crashed", 1, "").unwrap().cwd;
        let on = ["a/crashed".to_string()];
        let none: &[PathBuf] = &[];
        assert!(idle(&r.repo, none, Some(&on)).is_empty(), "名字在线：有人在用");
        assert!(idle(&r.repo, none, None).is_empty(), "不知道谁在线：当成有人在用");
        assert_eq!(idle(&r.repo, none, NOBODY).len(), 1, "名字离线、里面干净：空闲");
        assert!(idle(&r.repo, std::slice::from_ref(&crashed), NOBODY).is_empty(), "调用方正在用的不算");
        // 人自己锁的（原因不是会话名）、人手动建的（没有所有者记录）
        let manual = r.repo.parent().unwrap().join("proj.wt").join("manual");
        run(&r.repo, &["worktree", "add", "-q", "--detach", manual.to_str().unwrap(), "origin/main"]);
        assert_eq!(idle(&r.repo, none, NOBODY).len(), 1, "没有所有者记录的不算");
        run(&r.repo, &["worktree", "unlock", crashed.to_str().unwrap()]);
        run(&r.repo, &["worktree", "lock", "--reason", "先别动", crashed.to_str().unwrap()]);
        assert!(idle(&r.repo, none, NOBODY).is_empty(), "人自己锁的不算");
        run(&r.repo, &["worktree", "unlock", crashed.to_str().unwrap()]);
        // 有没推的改动
        std::fs::write(crashed.join("half.txt"), "没推的").unwrap();
        assert!(idle(&r.repo, none, NOBODY).is_empty());
        let b = prepare(&r.repo, "b/new", 2, "", &[], NOBODY).unwrap();
        assert!(!b.adopted && b.cwd != crashed && b.cwd != manual, "都不能接手：新建");
        assert_eq!(std::fs::read_to_string(crashed.join("half.txt")).unwrap(), "没推的");
        // 锁着离线名字的被接手：先解掉原来的锁
        std::fs::remove_file(crashed.join("half.txt")).unwrap();
        run(&r.repo, &["worktree", "lock", "--reason", "a/crashed", crashed.to_str().unwrap()]);
        let on = ["b/new".to_string()];
        let c = prepare(&r.repo, "c/next", 3, "", &[], Some(&on)).unwrap();
        assert_eq!((c.cwd.clone(), c.adopted), (crashed.clone(), true));
        assert_eq!(lock_of(&r, &crashed).as_deref(), Some("c/next"));
    }

    /// 两个会话同时抢同一个空闲的：占着它的那个拿到，另一个换下一个（这里没有下一个：新建）；
    /// 占着的进程被杀留下的记号过期后不算数
    #[test]
    fn a_worktree_being_adopted_is_skipped_by_the_other_session() {
        let r = rig();
        let a = prep(&r.repo, "a/x", 1, "").unwrap().cwd;
        assert!(release(&a));
        let guard = Guard::take(&a).expect("占住");
        assert!(Guard::take(&a).is_none(), "同一个只有一个占得住");
        assert!(!adopt(&r.repo, &a, "b/y", NOBODY));
        let b = prep(&r.repo, "b/y", 2, "").unwrap();
        assert!(!b.adopted && b.cwd != a);
        assert_eq!(lock_of(&r, &a), None, "没被动过");
        // 清理也不删正被占着的
        used(&a, Duration::from_secs(40 * 3600));
        assert!(sweep(&r.repo, &[b.cwd.clone()], NOBODY, WT_UNUSED, WT_IDLE_MAX, SystemTime::now()).is_empty() && a.exists());
        // 记号过期（进程被杀）：照常接手
        let file = guard.0.clone();
        std::mem::forget(guard);
        std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_modified(SystemTime::now() - GUARD_STALE - Duration::from_secs(1)).unwrap();
        used(&a, Duration::ZERO);
        assert!(adopt(&r.repo, &a, "c/z", NOBODY));
        assert!(!file.exists(), "接手完记号删掉");
    }

    /// `board base` 的顺序：锁着自己名字的 → 接手空闲的 → 新建；按名字算出的目录被别的名字接手了就换一个目录名
    #[test]
    fn session_worktree_reuses_adopts_then_creates() {
        let r = rig();
        let (a, got) = session_worktree(&r.repo, &r.repo, "t/alpha", "origin/main", NOBODY).unwrap();
        assert_eq!((a.clone(), got), (worktree_path(&r.repo, "t/alpha").unwrap(), Got::Created));
        assert_eq!(session_worktree(&r.repo, &r.repo, "t/alpha", "origin/main", NOBODY).unwrap(), (a.clone(), Got::Mine));
        // 另一个会话在跑时（名字在线）不接手它的
        let on = ["t/alpha".to_string()];
        let (b, got) = session_worktree(&r.repo, &r.repo, "t/beta", "origin/main", Some(&on)).unwrap();
        assert_eq!(got, Got::Created);
        assert_ne!(a, b);
        // alpha 结束、解锁：gamma 接手 alpha 留下的（beta 还在线）
        assert!(release(&a));
        let on = ["t/beta".to_string()];
        assert_eq!(session_worktree(&r.repo, &r.repo, "t/gamma", "origin/main", Some(&on)).unwrap(), (a.clone(), Got::Adopted));
        assert_eq!(read_owner(&a).unwrap().name, "t/gamma");
        // alpha 再来：自己的目录被 gamma 占着，换一个目录名新建，不报错
        let on = ["t/beta".to_string(), "t/gamma".to_string()];
        let (c, got) = session_worktree(&r.repo, &r.repo, "t/alpha", "origin/main", Some(&on)).unwrap();
        assert_eq!((c.clone(), got), (a.with_file_name(format!("{}-2", dir_name("t/alpha"))), Got::Created));
        assert_eq!(lock_of(&r, &c).as_deref(), Some("t/alpha"));
    }

    /// 接手时最近用过的优先：它的产物最新，要重编的最少
    #[test]
    fn the_most_recently_used_idle_worktree_is_adopted_first() {
        let r = rig();
        let old = prep(&r.repo, "a/old", 1, "").unwrap().cwd;
        let new = prep(&r.repo, "a/new", 2, "").unwrap().cwd;
        let mid = prep(&r.repo, "a/mid", 3, "").unwrap().cwd;
        for (wt, hours) in [(&old, 20), (&new, 1), (&mid, 5)] {
            assert!(release(wt));
            used(wt, Duration::from_secs(hours * 3600));
        }
        let order: Vec<PathBuf> = idle(&r.repo, &[], None).into_iter().map(|(p, _)| p).collect();
        assert_eq!(order, vec![new.clone(), mid, old]);
        assert_eq!(prep(&r.repo, "b/x", 4, "").unwrap().cwd, new);
    }

    /// 清理的两条规则：空闲超过 36 小时的删掉；剩下空闲的超过 4 个时从最久没用的删起。有人在用的不算在里面、也不删
    #[test]
    fn sweep_removes_idle_worktrees_past_the_age_and_count_limits() {
        let r = rig();
        // 先都建好（锁着时不会被下一个接手），再一起结束
        let hours = [40u64, 30, 20, 10, 5, 2, 1];
        let wts: Vec<PathBuf> = (0..hours.len()).map(|i| prep(&r.repo, &format!("a/s{i}"), i as i64 + 1, "").unwrap().cwd).collect();
        // 正在用的（名字在线，很久没记使用时间）：不算、不删
        let busy = prep(&r.repo, "a/busy", 9, "").unwrap().cwd;
        used(&busy, Duration::from_secs(50 * 3600));
        for (wt, h) in wts.iter().zip(hours) {
            assert!(release(wt));
            used(wt, Duration::from_secs(h * 3600));
        }
        let on = ["a/busy".to_string()];
        let got = sweep(&r.repo, &[], Some(&on), WT_UNUSED, WT_IDLE_MAX, SystemTime::now());
        let removed: Vec<(usize, Swept)> = got.iter().map(|(p, s)| (wts.iter().position(|w| same_path(w, p)).expect("只删空闲的"), s.clone())).collect();
        assert_eq!(removed, vec![(0, Swept::Removed), (1, Swept::Extra), (2, Swept::Extra)], "40 小时的过期；剩 6 个空闲的，删掉最久没用的 2 个");
        assert!(busy.exists() && wts[3..].iter().all(|w| w.exists()) && wts[..3].iter().all(|w| !w.exists()));
        assert!(read_owner(&wts[1]).is_none(), "所有者记录一起删");
        // 再来一次：正好 4 个，不动
        assert!(sweep(&r.repo, &[], Some(&on), WT_UNUSED, WT_IDLE_MAX, SystemTime::now()).is_empty());
    }

    /// 会话结束：干净的解锁；有没推的改动就原样留着、还锁着（不会被接手、清理）
    #[test]
    fn release_unlocks_only_a_clean_worktree() {
        let r = rig();
        let a = prep(&r.repo, "a/x", 1, "").unwrap().cwd;
        std::fs::write(a.join("half.txt"), "没推的").unwrap();
        assert!(!release(&a));
        assert_eq!(lock_of(&r, &a).as_deref(), Some("a/x"));
        used(&a, Duration::from_secs(40 * 3600));
        let got = sweep(&r.repo, &[], NOBODY, WT_UNUSED, WT_IDLE_MAX, SystemTime::now());
        assert!(matches!(got.as_slice(), [(_, Swept::Unpushed(_))]) && a.join("half.txt").exists(), "{got:?}");
        std::fs::remove_file(a.join("half.txt")).unwrap();
        let marker = a.join(git(&a, &["rev-parse", "--git-path", "MERGE_HEAD"], QUICK).unwrap());
        std::fs::write(&marker, "x").unwrap();
        assert!(!release(&a), "操作中途不解锁");
        assert_eq!(lock_of(&r, &a).as_deref(), Some("a/x"));
        std::fs::remove_file(marker).unwrap();
        assert!(release(&a));
        assert_eq!(lock_of(&r, &a), None);
    }
}
