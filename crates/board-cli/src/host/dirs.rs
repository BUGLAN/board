//! 工作空间和目录：把服务端上的项目在盘上摆好（建目录、克隆）、目录的 git 状态检查。

use super::*;

/// 一块白板目录的 git 状态（目录在不在、是不是 git 仓库、origin、当前分支）和看的时间。
/// 目录状态不看文件系统时间：改分支、换远端都不碰目录的修改时间，按时间判断会看到旧的
pub(super) struct DirCheck {
    pub(super) path: PathBuf,
    pub(super) status: &'static str,
    pub(super) origin: Option<String>,
    pub(super) branch: Option<String>,
    /// 看的时间：到没到时间重新看按它算
    pub(super) at: Instant,
    /// 同一次看的系统时间（毫秒），原样上报成 checkedAt。不能每次心跳用 `now_ms() - at.elapsed()` 现算：
    /// 两个时钟各自取整，结果在相邻两毫秒之间来回跳，服务端当成机器有变化，给所有界面推送、让它们重取白板（default/main#469）
    pub(super) checked_at: i64,
}

impl DirCheck {
    /// 刚看完的结果
    pub(super) fn new(path: PathBuf, status: &'static str, origin: Option<String>, branch: Option<String>) -> Self {
        DirCheck { path, status, origin, branch, at: Instant::now(), checked_at: now_ms() }
    }
}

pub(super) struct CloneRun {
    pub(super) board: String,
    pub(super) url: String,
    /// queued | running | done | failed
    pub(super) state: String,
    pub(super) message: String,
    pub(super) updated_at: i64,
    pub(super) child: Option<Child>,
    pub(super) log: PathBuf,
    /// 工作空间自动摆目录时起的（服务端上新建了项目、改了项目目录），不是界面上点的「克隆」：失败时在本机输出里说一声
    pub(super) auto: bool,
}

/// `--workspace` 的值：去掉首尾空白，空的当作没给
pub(super) fn workspace_arg(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// `--dir <项目>=<路径>`（旧写法 `<项目>/<白板>=<路径>`，可以有多个）拆成 (项目或白板, 路径)；写成别的样子时说明怎么写
pub(super) fn parse_raw_dirs(raw: Vec<String>) -> Result<Vec<(String, String)>> {
    raw.iter()
        .map(|d| {
            d.split_once('=')
                .map(|(b, p)| (b.trim().to_string(), p.trim().to_string()))
                .filter(|(b, p)| !b.is_empty() && !p.is_empty())
                .ok_or_else(|| BoardError::usage(format!("--dir 要写成 <项目>=<路径>：{d}")))
        })
        .collect()
}

/// 两个工作空间是不是一样：路径和 `--dir` 覆盖的目录（顺序不比）
pub(super) fn same_ws(a: &HostWorkspace, b: &HostWorkspace) -> bool {
    a.path == b.path
        && a.overrides.len() == b.overrides.len()
        && a.overrides.iter().all(|x| b.overrides.iter().any(|y| y.board == x.board && y.path == x.path))
}

/// 启动时先认的白板：旧写法 `--dir <项目>/<白板>` 单独覆盖的那几块。其余白板的目录是 `<工作空间>/<项目>`，
/// 盘上看不出有哪几块白板，从服务端取到白板列表后再补上（见 [`run`]、`Host::set_work`）
pub(super) fn ws_boards(ws: &HostWorkspace, _os: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for o in &ws.overrides {
        if board_core::valid_board(&o.board) && !out.contains(&o.board) {
            out.push(o.board.clone());
        }
    }
    out
}

/// 工作空间的目录：`~` 展开，相对路径按当前目录换成绝对路径
pub(super) fn workspace_path(raw: &str) -> Result<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(BoardError::usage("工作空间不能是空的：填一个目录，每个项目的目录是 <工作空间>/<项目>"));
    }
    let mut dir = expand(raw);
    if dir.is_relative() {
        dir = std::env::current_dir().map(|c| c.join(&dir)).unwrap_or(dir);
    }
    Ok(dir)
}

/// 命令行、配置文件里的工作空间和 `--dir` 覆盖的目录：去掉首尾空白，工作空间展开 `~`
pub(super) fn to_workspace(raw: Option<&str>, overrides: &[(String, String)]) -> Result<HostWorkspace> {
    let mut out: Vec<HostDirOverride> = vec![];
    for (b, p) in overrides {
        let (b, p) = (b.trim(), p.trim());
        if b.is_empty() || p.is_empty() {
            continue;
        }
        // 项目、白板写错时整份配置不用（重新读配置时接着用原来的），不能悄悄把工作空间清掉
        if !board_core::valid_dir_key(b) {
            return Err(BoardError::usage(format!("要写成 <项目> 或 <项目>/<白板>：{b}")));
        }
        if out.iter().any(|o| o.board == b) {
            return Err(BoardError::usage(format!("白板 {b} 写了两次")));
        }
        out.push(HostDirOverride { board: b.to_string(), path: p.to_string() });
    }
    let path = match workspace_arg(raw) {
        Some(w) => {
            // 存展开后的路径：相对路径按启动时的目录，之后重新读配置、重启也还是它
            let dir = workspace_path(&w)?;
            dir.display().to_string()
        }
        None => String::new(),
    };
    Ok(HostWorkspace { path, overrides: out, ..Default::default() })
}

/// 服务端把工作空间写回本机配置；保留 `[host]` 里的机器名和文件里的其他项、注释
pub(super) fn save_host_workspace(ws: &str) -> Result<()> {
    config::update(|c| {
        let name = c.host()?.name;
        c.set_host(name.as_deref(), Some(ws))?;
        Ok(((), true))
    })
}

/// 项目设置里填了项目目录的项目：项目标识 → 目录名（#488；旧服务端没有这一项，都用项目标识）
pub(super) fn project_dirs(w: &Workspace) -> std::collections::BTreeMap<String, String> {
    w.projects.iter().filter(|p| !p.dir.trim().is_empty()).map(|p| (p.key.clone(), p.dir.trim().to_string())).collect()
}

/// 把服务端上摆着的项目在盘上摆好：每个项目的目录是 `<工作空间>/<项目>`，项目下的几块白板共用它。已经有的不动；
/// 项目设了仓库地址时在那里 `git clone`（进度和失败原因照常上报，记在先轮到的那块白板上），没设时建一个空目录。
/// 一个项目失败不影响其他项目，返回失败的那几块白板。
///
/// 克隆是同步的，只在启动时、第一次上报之前用；之后服务端上新摆的项目由 `Host::place_dirs` 在后台克隆（#680）
pub(super) fn bootstrap_dirs(
    ws: &HostWorkspace,
    boards: &[(String, Option<String>)],
    checks: &mut Vec<DirCheck>,
    clones: &mut Vec<CloneRun>,
    os: &str,
    logs: &Path,
) -> Vec<(String, String)> {
    let mut failed: Vec<(String, String)> = vec![];
    if ws.path.trim().is_empty() {
        return failed;
    }
    if let Err(e) = std::fs::create_dir_all(expand(&ws.path)) {
        return vec![(String::new(), format!("无法创建工作空间 {}：{e}", ws.path))];
    }
    let _ = std::fs::create_dir_all(logs);
    for (board, repo) in boards {
        let Some(dir) = board_core::hosts::derive_dir(ws, board, os).map(|x| expand(&x.path)) else { continue };
        // `--dir` 覆盖的（这块白板或它所在的项目）、已经有目录的（同一个项目里前一块白板刚建好的也算）：不动
        if board_core::hosts::derive_dir(ws, board, os).is_some_and(|d| d.fixed) || dir.is_dir() {
            continue;
        }
        let repos = repo.as_deref().map(str::trim).filter(|r| !r.is_empty());
        let mut fail = |why: String| failed.push((board.clone(), why));
        if let Some(parent) = dir.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                fail(format!("无法创建 {}：{e}", parent.display()));
                continue;
            }
        }
        match repos {
            None => {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    fail(format!("无法创建 {}：{e}", dir.display()));
                }
            }
            Some(url) => {
                let safe: String = board.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
                let log = logs.join(format!("clone-{safe}.log"));
                let host_dir = logs.parent().map(Path::to_path_buf).unwrap_or_default();
                let spawn = create_log(&log).and_then(|(out, err)| {
                    crate::gitcmd::command()
                        .args(gitsync::clone_args(url, &host_dir))
                        .args(["clone", "--", url])
                        .arg(&dir)
                        .stdin(Stdio::null())
                        .stdout(out)
                        .stderr(err)
                        .status()
                });
                match spawn {
                    Ok(s) if s.success() => {
                        let _ = gitsync::ensure(&host_dir);
                        checks.push(DirCheck::new(dir, "ok", Some(url.to_string()), None))
                    }
                    Ok(s) => {
                        let why = clone_tail(&log).unwrap_or_else(|| format!("git clone 失败（退出码 {:?}）", s.code()));
                        fail(format!("{url} 克隆到 {} 失败：{why}", dir.display()));
                        clones.push(CloneRun { board: board.clone(), url: url.to_string(), state: "failed".into(), message: why, updated_at: now_ms(), child: None, log, auto: true });
                    }
                    Err(e) => fail(format!("没法运行 git clone：{e}")),
                }
            }
        }
    }
    failed
}

/// 这块白板在这台机器上的目录（它所在项目的 `<工作空间>/<项目>`，`--dir` 覆盖的用覆盖的路径；`~` 展开成实际的路径）
pub(super) fn ws_dir(ws: &HostWorkspace, board: &str, os: &str) -> Option<PathBuf> {
    board_core::hosts::derive_dir(ws, board, os).map(|d| expand(&d.path))
}

/// 几块白板的 (白板, 目录)
pub(super) fn ws_pairs(ws: &HostWorkspace, boards: &[String], os: &str) -> Vec<(String, PathBuf)> {
    boards.iter().filter_map(|b| ws_dir(ws, b, os).map(|p| (b.clone(), p))).collect()
}

pub(super) fn git_line(dir: &Path, args: &[&str]) -> Option<String> {
    let out = crate::gitcmd::command().arg("-C").arg(dir).args(args).stdin(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// 目录状态：存在且是 git 仓库（同时取 origin、当前分支）、不存在、存在但不是 git 仓库
pub(super) fn check_git_dir(dir: &Path) -> (&'static str, Option<String>, Option<String>) {
    if !dir.exists() {
        return ("missing", None, None);
    }
    if !dir.is_dir() || git_line(dir, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return ("not_git", None, None);
    }
    let origin = git_line(dir, &["remote", "get-url", "origin"]);
    let branch = git_line(dir, &["branch", "--show-current"]);
    ("ok", origin, branch)
}

pub(super) fn clone_tail(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().and_then(|s| s.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()))
}

impl Host<'_> {
    /// 该重新看目录状态了：目录刚变过（配置、克隆完）、到时间了、或者还没有结果
    pub(super) fn dir_check_due(checks: &[DirCheck], path: &Path) -> bool {
        match checks.iter().find(|c| c.path == path) {
            Some(c) => c.at.elapsed() >= DIR_CHECK_EVERY,
            None => true,
        }
    }

    /// 上次拉不到最新代码的项目目录：隔 5 分钟在后台再拉一次，拉成了就恢复接这个项目的卡。
    /// 不在主循环里拉（#680）：远端卡住时一次要等 30 秒，几个目录加起来会把心跳拖到服务端判这台机器掉线。
    /// 返回有没有变化（变了马上上报）
    pub(super) fn poll_fetches(&mut self) -> bool {
        let mut changed = false;
        let (done, busy): (Vec<_>, Vec<_>) = std::mem::take(&mut self.fetch_jobs).into_iter().partition(|(_, job)| job.is_finished());
        self.fetch_jobs = busy;
        for (dir, job) in done {
            let out = job.join().unwrap_or_else(|_| Err("拉取的线程出错".into()));
            // 拉的这段时间里派发前的拉取已经拉成了（记录去掉了）：不用再说
            let Some(i) = self.fetch_errs.iter().position(|(p, ..)| *p == dir) else { continue };
            match out {
                Ok(()) => {
                    self.fetch_errs.remove(i);
                    self.say(format!("{} {} 又能拉到最新代码了", self.p.green("✓"), dir.display()));
                    changed = true;
                }
                Err(e) => {
                    let x = &mut self.fetch_errs[i];
                    changed |= x.1 != e;
                    x.1 = e;
                    x.2 = Instant::now();
                }
            }
        }
        let due: Vec<PathBuf> = self
            .fetch_errs
            .iter()
            .filter(|(p, _, at)| at.elapsed() >= DIR_CHECK_EVERY && !self.fetch_jobs.iter().any(|(q, _)| q == p))
            .map(|(p, ..)| p.clone())
            .collect();
        for dir in due {
            let d = dir.clone();
            self.fetch_jobs.push((dir, std::thread::spawn(move || crate::code::fetch(&d))));
        }
        changed
    }

    /// 正在后台克隆的目录：克隆完之前不看它的 git 状态、上报成「不存在」（服务端不往这里派卡），也不在里面启动 agent
    pub(super) fn cloning(&self) -> Vec<PathBuf> {
        self.clones.iter().filter(|c| c.state == "running").filter_map(|c| ws_dir(&self.ws, &c.board, &self.os)).collect()
    }

    /// 每块白板目录的 git 状态：到时间的重新看一遍，其余沿用上一次的结果（心跳不跑 git 子进程）
    pub(super) fn refresh_dir_checks(&mut self) {
        let dirs: Vec<PathBuf> = ws_pairs(&self.ws, &self.boards, &self.os).into_iter().map(|(_, p)| p).collect();
        let cloning = self.cloning();
        // 克隆到一半的仓库可能已经像个 git 仓库了：不看，克隆完马上重新看（poll_clones 清掉旧结果）
        self.dir_checks.retain(|c| dirs.contains(&c.path) && !cloning.contains(&c.path));
        for dir in dirs {
            if cloning.contains(&dir) || !Self::dir_check_due(&self.dir_checks, &dir) {
                continue;
            }
            let (status, origin, branch) = check_git_dir(&dir);
            let fresh = DirCheck::new(dir.clone(), status, origin, branch);
            match self.dir_checks.iter_mut().find(|c| c.path == dir) {
                Some(c) => *c = fresh,
                None => self.dir_checks.push(fresh),
            }
        }
    }

    /// 每块白板：工作空间派生的（或 `--dir` 覆盖的）路径、目录状态、git 的 origin / 分支、克隆进度
    pub(super) fn dir_reports(&mut self) -> Vec<HostDir> {
        self.refresh_dir_checks();
        let ws = self.ws.clone();
        let boards = self.boards.clone();
        let cloning = self.cloning();
        boards
            .iter()
            .filter_map(|board| board_core::hosts::derive_dir(&ws, board, &self.os))
            .map(|d| {
                let board = &d.board;
                let path = expand(&d.path);
                let raw = d.path.clone();
                let c = self.dir_checks.iter().find(|c| c.path == path);
                let (status, origin, branch) = match c {
                    Some(c) => (c.status, c.origin.clone(), c.branch.clone()),
                    None => ("missing", None, None),
                };
                let checked_at = c.map_or_else(now_ms, |c| c.checked_at);
                // 克隆记在先轮到的那块白板上；同一个项目的其他白板共用这个目录，正在克隆时也带上它
                let clone = self.clones.iter().rev().find(|c| c.board == *board).or_else(|| {
                    self.clones.iter().rev().find(|c| c.state == "running" && cloning.contains(&path) && ws_dir(&self.ws, &c.board, &self.os).as_ref() == Some(&path))
                });
                let clone = clone.map(|c| HostClone {
                    state: c.state.clone(),
                    url: c.url.clone(),
                    message: c.message.clone(),
                    updated_at: c.updated_at,
                    by: "本机".into(),
                });
                HostDir {
                    board: board.clone(),
                    path: raw,
                    fixed: d.fixed,
                    status: Some(status.into()),
                    origin,
                    branch,
                    checked_at: Some(checked_at),
                    clone,
                    reported: true,
                    fetch_error: self.fetch_errs.iter().find(|(p, ..)| *p == path).map(|(_, e, _)| e.clone()),
                }
            })
            .collect()
    }

    pub(super) fn set_work(&mut self, w: HostWork) {
        self.sync_server_dirs(&w.host);
        // 服务端上的白板变了（新建了白板、项目）：所在项目在工作空间里还没有目录的按 `<工作空间>/<项目>` 建好
        let new: Vec<String> = w.host.dirs.iter().map(|d| d.board.clone()).filter(|b| !self.boards.contains(b)).collect();
        if !new.is_empty() {
            let server: Vec<(String, Option<String>)> = match self.rm().get::<Workspace>("/api/workspace", &[]) {
                Ok(v) => v
                    .projects
                    .iter()
                    .flat_map(|p| p.boards.iter().map(|b| (b.path.clone(), Some(b.repo.clone()).filter(|r| !r.trim().is_empty()))))
                    .filter(|(b, _)| new.contains(b))
                    .collect(),
                Err(_) => vec![],
            };
            self.place_dirs(&server);
            for b in new {
                self.boards.push(b);
            }
            self.boards.sort();
            self.boards.dedup();
        }
        // 会话里 agent 执行了 board next：同一个进程换到这个会话正在跑的那一张，之后的日志、停止、结束都跟着它。
        // 换之前把已经写出的日志传给上一张
        let switches: Vec<(usize, Dispatch)> =
            self.runs.iter().enumerate().filter_map(|(i, r)| successor(&r.d, &w.running).map(|n| (i, n))).collect();
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        for (i, n) in switches {
            let (lines, to) = new_lines(&self.runs[i].log, self.runs[i].sent, false);
            let lines = upload_lines_redacted(lines, self.runs[i].fmt, &secrets);
            if !lines.is_empty() {
                let _ = self.rm().post::<Value>(&format!("/api/dispatches/{}/log", self.runs[i].d.id), json!({ "lines": lines }));
            }
            self.runs[i].sent = to;
            // 已经写出的事件也记到上一张；之后的记到新的这张，开头再报一次格式
            let mut r = self.runs.remove(i);
            self.flush_events(&mut r, false);
            r.fmt_due = true;
            self.runs.insert(i, r);
            self.say(format!(
                "{} {}  {}  {}",
                self.p.green("▶"),
                self.p.cyan(&format!("{}#{}", n.board, n.card)),
                self.p.yellow(n.name.as_deref().unwrap_or_default()),
                self.p.dim(&format!("同一个会话接着做 · {}", n.title))
            ));
            self.runs[i].d = n;
        }
        // 列目录（界面选工作空间）：在后台列，列完回传
        let fresh: Vec<LsAsk> = w.ls.iter().filter(|l| !self.ls_seen.contains(&l.id)).cloned().collect();
        for l in fresh {
            self.ls_seen.push(l.id);
            let (rm, name) = (self.rm().clone(), self.name.clone());
            std::thread::spawn(move || {
                let out = crate::lsdir::list(&l.path);
                let _ = rm.post::<serde_json::Value>(&format!("/api/hosts/{name}/ls/{}", l.id), serde_json::to_value(&out).unwrap_or_default());
            });
        }
        if self.ls_seen.len() > 200 {
            self.ls_seen.drain(..100);
        }
        self.work = Some(w);
        self.work_at = Instant::now();
        self.hold = false;
        self.start_clones();
    }

    /// 服务端的工作空间是主版本：配置文件没变、但服务端改了（别的设备上改的）就写回 `[host] workspace` 并直接换上
    /// （工作空间里的白板按新的工作空间派生目录）。`--workspace` / `--dir` 启动时这一项不跟，服务端也不会让它改
    pub(super) fn sync_server_dirs(&mut self, h: &HostInfo) {
        self.dirs_rev = h.dirs_rev;
        self.sync_project_dirs(h);
        let Some(ws) = h.workspace.as_ref().filter(|w| !w.path.trim().is_empty()) else { return };
        // `--dir` 覆盖的那几块以本机启动参数为准
        let mut want = ws.clone();
        for o in &self.ws.overrides {
            match want.overrides.iter_mut().find(|x| x.board == o.board) {
                Some(x) => x.path = o.path.clone(),
                None => want.overrides.push(o.clone()),
            }
        }
        if !self.follow_dirs || same_ws(&want, &self.ws) {
            return;
        }
        match save_host_workspace(&want.path) {
            Ok(()) => {
                // 之前摆着的白板、服务端报过的白板，都摆到新的工作空间里
                let mut boards = self.boards.clone();
                for d in h.dirs.iter() {
                    if !boards.contains(&d.board) {
                        boards.push(d.board.clone());
                    }
                }
                boards.sort();
                boards.dedup();
                self.ws = HostWorkspace { projects: self.ws.projects.clone(), ..want.clone() };
                self.boards = boards;
                for a in &mut self.agents {
                    a.check_skills(&self.envs, &ws_pairs(&self.ws, &self.boards, &self.os));
                }
                self.say(format!("{} 服务端改的工作空间已生效：{}", self.p.green("✓"), want.path));
                self.conf_seen.1 = Some(want.path);
            }
            Err(e) => self.say(format!("{} 服务端改的工作空间没有写进配置：{}", self.p.yellow("!"), e.message)),
        }
    }

    /// 项目设置里的项目目录（#488）：以服务端为准（`--workspace` / `--dir` 启动时也跟），变了就按新目录重新摆一遍：
    /// 新目录不存在时照常建好或克隆，旧目录不动。旧服务端不发这一项，都用项目标识
    pub(super) fn sync_project_dirs(&mut self, h: &HostInfo) {
        let Some(projects) = h.workspace.as_ref().map(|w| w.projects.clone()) else { return };
        if projects == self.ws.projects {
            return;
        }
        let before = self.ws.projects.clone();
        self.ws.projects = projects;
        let moved: Vec<String> = self
            .boards
            .iter()
            .filter(|b| {
                let p = b.split_once('/').map(|(p, _)| p).unwrap_or_default();
                before.get(p) != self.ws.projects.get(p)
            })
            .cloned()
            .collect();
        if moved.is_empty() {
            return;
        }
        let server: Vec<(String, Option<String>)> = match self.rm().get::<Workspace>("/api/workspace", &[]) {
            Ok(v) => v
                .projects
                .iter()
                .flat_map(|p| p.boards.iter().map(|b| (b.path.clone(), Some(b.repo.clone()).filter(|r| !r.trim().is_empty()))))
                .filter(|(b, _)| moved.contains(b))
                .collect(),
            Err(_) => moved.iter().map(|b| (b.clone(), None)).collect(),
        };
        self.place_dirs(&server);
        for a in &mut self.agents {
            a.check_skills(&self.envs, &ws_pairs(&self.ws, &self.boards, &self.os));
        }
        let mut seen: Vec<String> = vec![];
        for b in &moved {
            if let Some(d) = ws_dir(&self.ws, b, &self.os) {
                let line = d.display().to_string();
                if !seen.contains(&line) {
                    self.say(format!("{} 项目目录改了：{} → {line}", self.p.green("✓"), b.split_once('/').map(|(p, _)| p).unwrap_or(b)));
                    seen.push(line);
                }
            }
        }
    }

    /// 工作空间里出问题的一行：`board` 为空时是工作空间本身
    fn say_ws(&self, board: &str, why: &str) {
        self.say(format!("{} 工作空间里 {}：{why}", self.p.yellow("!"), if board.is_empty() { "（工作空间）" } else { board }));
    }

    /// 服务端上新摆的白板（新建了白板、项目，改了项目目录）在盘上摆好，和启动时的 [`bootstrap_dirs`] 一样：已经有目录的不动，
    /// 没设仓库地址的建一个空目录，设了的 `git clone`。克隆在后台跑（[`Self::start_clone`]，由 [`Self::poll_clones`] 收尾），
    /// 不在主循环里等（#680）：大仓库、网络慢时 clone 可能几分钟不返回，主循环停住就发不出上报，50 秒后服务端判这台机器掉线，
    /// 在跑的派发全被结束。克隆完之前这个目录上报成「不存在、正在克隆」，服务端不往这里派卡
    pub(super) fn place_dirs(&mut self, boards: &[(String, Option<String>)]) {
        if self.ws.path.trim().is_empty() {
            return;
        }
        if let Err(e) = std::fs::create_dir_all(expand(&self.ws.path)) {
            self.say_ws("", &format!("无法创建工作空间 {}：{e}", self.ws.path));
            return;
        }
        let mut started = false;
        for (board, repo) in boards {
            let Some(d) = board_core::hosts::derive_dir(&self.ws, board, &self.os) else { continue };
            let dir = expand(&d.path);
            // `--dir` 覆盖的、已经有目录的、正在克隆的（同一个项目里前一块白板刚开始克隆的也算）：不动
            if d.fixed || dir.is_dir() || self.cloning().contains(&dir) {
                continue;
            }
            match repo.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
                None => {
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        self.say_ws(board, &format!("无法创建 {}：{e}", dir.display()));
                    }
                }
                Some(url) => {
                    self.start_clone(board, url, true);
                    match self.clones.last() {
                        Some(c) if c.board == *board && c.state == "failed" => {
                            let why = c.message.clone();
                            self.say_ws(board, &why);
                        }
                        _ => started = true,
                    }
                }
            }
        }
        // 马上上报：服务端早点知道这个目录正在克隆，不往这里派卡
        if started {
            self.beat_soon = true;
        }
    }

    /// 服务端请求了克隆，就在本机执行 `git clone`；一块白板失败不影响其他白板
    pub(super) fn start_clones(&mut self) {
        let Some(w) = &self.work else { return };
        let jobs: Vec<(String, String)> = w
            .host
            .dirs
            .iter()
            .filter(|d| d.clone.as_ref().is_some_and(|c| c.state == "queued"))
            .filter(|d| !self.clones.iter().any(|c| c.board == d.board && (c.state == "queued" || c.state == "running")))
            .filter_map(|d| d.clone.as_ref().map(|c| (d.board.clone(), c.url.clone())))
            .collect();
        for (board, url) in jobs {
            self.start_clone(&board, &url, false);
        }
    }

    /// 在后台开始 `git clone`（`auto`：工作空间自动摆目录时起的，见 [`CloneRun::auto`]）；开始不了时记一条失败的
    pub(super) fn start_clone(&mut self, board: &str, url: &str, auto: bool) {
        let Some(dir) = ws_dir(&self.ws, board, &self.os) else { return };
        let _ = std::fs::create_dir_all(&self.logs);
        let safe: String = board
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        let log = self.logs.join(format!("clone-{safe}.log"));
        let (out, err) = match create_log(&log) {
            Ok(f) => f,
            Err(e) => {
                self.clones.push(CloneRun {
                    board: board.into(),
                    url: url.into(),
                    state: "failed".into(),
                    message: format!("无法创建克隆日志：{e}"),
                    updated_at: now_ms(),
                    child: None,
                    log,
                    auto,
                });
                return;
            }
        };
        if let Some(parent) = dir.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.clones.push(CloneRun {
                    board: board.into(),
                    url: url.into(),
                    state: "failed".into(),
                    message: format!("无法创建 {}：{e}", parent.display()),
                    updated_at: now_ms(),
                    child: None,
                    log,
                    auto,
                });
                return;
            }
        }
        let child = crate::gitcmd::command()
            .args(gitsync::clone_args(url, &self.host_dir()))
            .args(["clone", "--", url])
            .arg(&dir)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn();
        match child {
            Ok(child) => self.clones.push(CloneRun {
                board: board.into(),
                url: url.into(),
                state: "running".into(),
                message: "正在克隆…".into(),
                updated_at: now_ms(),
                child: Some(child),
                log,
                auto,
            }),
            Err(e) => self.clones.push(CloneRun {
                board: board.into(),
                url: url.into(),
                state: "failed".into(),
                message: format!("没法运行 git clone：{e}"),
                updated_at: now_ms(),
                child: None,
                log,
                auto,
            }),
        }
    }

    pub(super) fn poll_clones(&mut self) {
        let mut done: Vec<String> = vec![];
        let mut failed: Vec<(String, String)> = vec![];
        let mut ended = false;
        for c in &mut self.clones {
            if c.state != "running" {
                continue;
            }
            let status = c.child.as_mut().and_then(|child| child.try_wait().ok().flatten());
            if let Some(status) = status {
                let tail = clone_tail(&c.log);
                if status.success() {
                    c.state = "done".into();
                    c.message = "克隆完成".into();
                    // 目录里刚有了仓库：下一次上报重新看这块白板的目录状态，不等 DIR_CHECK_EVERY
                    done.push(c.board.clone());
                } else {
                    c.state = "failed".into();
                    c.message = tail.unwrap_or_else(|| format!("git clone 失败（退出码 {:?}）", status.code()));
                    if c.auto {
                        failed.push((c.board.clone(), c.url.clone()));
                    }
                }
                c.updated_at = now_ms();
                c.child = None;
                ended = true;
            } else if let Some(line) = clone_tail(&c.log) {
                c.message = line;
            }
        }
        // 工作空间自动摆目录时起的克隆失败了：和启动时同步克隆失败一样说一声
        for (board, url) in failed {
            let dir = ws_dir(&self.ws, &board, &self.os).map(|d| d.display().to_string()).unwrap_or_default();
            let why = self.clones.iter().rev().find(|c| c.board == board).map(|c| c.message.clone()).unwrap_or_default();
            self.say_ws(&board, &format!("{url} 克隆到 {dir} 失败：{why}"));
        }
        // 刚克隆完的白板：删掉旧的目录状态，下一次上报马上重新看
        let paths: Vec<PathBuf> = ws_pairs(&self.ws, &self.boards, &self.os).into_iter().filter(|(b, _)| done.contains(b)).map(|(_, p)| p).collect();
        if !paths.is_empty() {
            let _ = gitsync::ensure(&self.host_dir());
            // 仓库里自带的 board-skills 现在才看得到
            let pairs = ws_pairs(&self.ws, &self.boards, &self.os);
            for a in &mut self.agents {
                a.check_skills(&self.envs, &pairs);
            }
        }
        self.dir_checks.retain(|c| !paths.contains(&c.path));
        // 克隆结束（成了、没成）：马上上报，服务端据此决定能不能往这里派卡
        if ended {
            self.beat_soon = true;
        }
    }
}
