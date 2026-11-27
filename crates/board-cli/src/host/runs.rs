//! 派发：在白板目录里启动 agent（board acp）、看它是否结束、时限、停止、收尾和上报。

use super::*;
use crate::proctree::Job;

/// 写 dsh 一次性派发的 profile 补丁（内容固定，每次派发覆盖写）
pub(super) fn write_dsh_patch(path: &Path) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, DSH_PATCH)
}

/// 写一个只给这次派发用的配置文件：服务端地址和令牌，只有自己能读（0600）。
///
/// 还有 `[agent_env]`：这次派发要带给 agent 的 `BOARD_*` 变量。agent 会过滤传给命令的环境变量：Codex 按 shell 环境策略
/// 只放行 `core` 里的几个（`BOARD_*` 不在里面，`shell` 工具里就看不到连接），dsh 去掉名字里带 `TOKEN` 的。
/// `scripts/board` 看到 `BOARD_AGENT_ENV=1` 时从 `BOARD_CONFIG` 读这一节、再设进它自己的环境，它起的 `board` 就有了
/// （[`crate::backend`] 读配置时也用它们，所以没经过包装脚本、直接执行 `board` 时也连得上）
pub(super) fn write_run_config(path: &Path, url: &str, token: &str, vars: &[(String, String)]) -> Result<()> {
    board_core::config::write_new(path, |c| {
        c.set(config::URL, url);
        c.set(config::TOKEN, token);
        let mut t = toml_edit::Table::new();
        for (k, v) in vars {
            t.insert(k, toml_edit::value(v.as_str()));
        }
        c.set_table("agent_env", t);
    })
}

/* ------------------------------ 进程 ------------------------------ */

/// 退出码；被信号结束时按 shell 的习惯记成 128 + 信号（kill -9 → 137）
pub(super) fn exit_code(s: std::process::ExitStatus) -> Option<i64> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = s.signal() {
            return Some(128 + sig as i64);
        }
    }
    s.code().map(|c| c as i64)
}

/* ------------------------------ 一次在跑的派发 ------------------------------ */

pub(super) struct Run {
    pub(super) d: Dispatch,
    pub(super) child: Child,
    /// board acp 的标准输入：host 只拿着写端、从不写，host 退出后管道关闭，board acp 据此取消
    pub(super) _stdin: Option<std::process::ChildStdin>,
    /// Windows：装着 board acp 和它启动的整棵进程树；句柄关闭时系统一并结束它们
    pub(super) job: Option<Job>,
    /// 启动环境
    pub(super) env: Env,
    /// WSL 里 agent 的进程号写在这里（BOARD_PIDFILE），结束时删除
    pub(super) pidfile: Option<PathBuf>,
    pub(super) log: PathBuf,
    /// 这次派发的配置文件（BOARD_CONFIG），结束时删除
    pub(super) conf: PathBuf,
    /// agent 的进程号和启动时间（`<派发 ID>.proc`）：host 异常退出后重启时据此结束留下的 agent，结束时删除
    pub(super) procfile: PathBuf,
    /// 日志已经上传到的位置（字节）
    pub(super) sent: u64,
    /// 日志按哪种事件流转写后上传；只有原始输出的 agent 原样上传
    pub(super) fmt: Option<Stream>,
    /// 结构化事件（会话「过程」，events.rs）：解析器（只有原始输出的 agent 没有）
    pub(super) parser: Option<events::Parser>,
    /// 日志已经解析到的位置（字节）
    pub(super) ev_read: u64,
    /// 解析出来、还没传上去的事件（连不上服务端时攒着，最多 2000 条）
    pub(super) ev_queue: Vec<RunEvent>,
    /// 最后几个事件（工具调用和结果合成一条）：异常退出的评论里写它们
    pub(super) ev_recent: VecDeque<RunEvent>,
    /// 这次派发的格式还没报给服务端（开始时、会话换到下一张时）
    pub(super) fmt_due: bool,
    /// SIGTERM 的时间
    pub(super) term_at: Option<Instant>,
    pub(super) killed: bool,
    pub(super) spawned: Instant,
    /// agent 在哪个目录里启动：board host 备好的 worktree，或者项目目录
    pub(super) cwd: PathBuf,
    /// `cwd` 是 board host 备好的 worktree：派发结束时里面没推的改动由 board host 保存成 wip 分支
    pub(super) managed: bool,
    /// 服务端已经结束了这次派发（机器掉线后卡片被释放）才停的：agent 退出后看看它的目录里有没有没推的改动
    pub(super) lost: bool,
    /// 已经按机器的兜底设置要求停止：原因（「运行超过 3 小时…」）。停止流程走完后上报结束，结果「超时」
    pub(super) timeout: Option<String>,
    /// 最近一次有输出（日志变长）的时间：判断「没输出多久算卡住」
    pub(super) output_at: Instant,
    /// 上一次看到的日志长度（判断有没有新的输出）
    pub(super) seen_len: u64,
}

impl Run {
    /// 结束 agent 进程树（hard：强制）。WSL：在 WSL 里结束 agent 的进程组，强制时再结束 wsl.exe 的进程树
    pub(super) fn stop(&self, hard: bool) {
        if self.env.wsl().is_some() {
            let pid = self.pidfile.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.trim().parse::<u32>().ok());
            match pid {
                Some(pid) => envs::wsl_kill(&self.env, pid, hard),
                // 还没写进程号（刚启动）：直接结束 wsl.exe
                None => self.terminate_tree(true),
            }
            if hard {
                self.terminate_tree(true);
            }
        } else {
            self.terminate_tree(hard);
        }
    }

    /// 主进程已经退出：结束同一进程组里剩下的进程（agent 起的子进程）。
    ///
    /// Windows 上结束 Job 里剩下的进程：父进程先退出也不影响，孙进程仍然在 Job 里。
    /// 调用前要先把主进程回收（agent 自己退出时 `reap` 已经 `try_wait` 过，board host 退出时先 `wait()`）。
    pub(super) fn kill_rest(&self) {
        if let Some(env) = self.env.wsl().map(|_| &self.env) {
            let pid = self.pidfile.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.trim().parse::<u32>().ok());
            if let Some(pid) = pid {
                envs::wsl_kill(env, pid, true);
            }
        }
        self.terminate_tree(true);
    }

    /// 结束 agent 的整棵进程树：Windows 用 Job，其他系统按进程组
    fn terminate_tree(&self, hard: bool) {
        match &self.job {
            Some(job) => job.terminate(hard),
            None => kill_tree(self.child.id(), hard),
        }
    }

    /// board acp 写的额度文件（`<派发 ID>.quota`，和配置文件放在一起）
    pub(super) fn quota_file(&self) -> PathBuf {
        self.conf.with_extension("quota")
    }

    /// board acp 看到的额度（#668）：没写（没看到、旧版）时为空
    pub(super) fn quota(&self) -> Option<board_core::hosts::Quota> {
        std::fs::read_to_string(self.quota_file()).ok().and_then(|t| serde_json::from_str(&t).ok())
    }

    pub(super) fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.conf);
        // 提示词文件（`<派发 ID>.prompt`，和配置文件放在一起）
        let _ = std::fs::remove_file(self.conf.with_extension("prompt"));
        let _ = std::fs::remove_file(&self.procfile);
        let _ = std::fs::remove_file(self.quota_file());
        if let Some(dir) = self.conf.parent() {
            remove_secret_files(dir, self.d.id);
        }
        if let Some(p) = &self.pidfile {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn provider_file(run: &Path, id: i64) -> PathBuf {
    run.join(format!("{id}.provider.json"))
}

fn dsh_patch_file(run: &Path, id: i64) -> PathBuf {
    run.join(format!("{id}.dsh-patch.yml"))
}

/// 删掉一次派发/对话的供应商和 dsh 补丁文件（里面有 key）
pub(super) fn remove_secret_files(run: &Path, id: i64) {
    let _ = std::fs::remove_file(provider_file(run, id));
    let _ = std::fs::remove_file(dsh_patch_file(run, id));
}

/// 启动过程中兜底删密钥文件：成功建出 Run / ChatRun 后由它自己的 cleanup 接着管
pub(super) struct SecretFiles {
    paths: Vec<PathBuf>,
    keep: bool,
}

impl SecretFiles {
    pub(super) fn for_role(run: &Path, id: i64, role: Option<&RoleRun>) -> SecretFiles {
        let mut paths = vec![];
        if role.and_then(|r| r.provider_file.as_ref()).is_some() {
            paths.push(provider_file(run, id));
        }
        if role.and_then(|r| r.dsh_patch.as_ref()).is_some() {
            paths.push(dsh_patch_file(run, id));
        }
        SecretFiles { paths, keep: false }
    }

    pub(super) fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for SecretFiles {
    fn drop(&mut self) {
        if !self.keep {
            for path in &self.paths {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// 收尾时记一笔结束，等 `report_ended()` 上报给服务端：取最后几行做错误摘录，再截断过大的日志。
///
/// 截断要在 agent 确实不在了之后：它还活着时换掉文件内容，下次写入会从旧位置接着写、中间留一段空洞。
/// `how` 是给服务端的结果（`exit`、`host_exit`）；兜底停止过（`Run.timeout` 有值）的无论哪种都报 `timeout`，
/// 原因照原样带上
pub(super) fn ended_of(r: &Run, code: Option<i64>, how: &'static str, secrets: &[&str], quota: Option<board_core::hosts::Quota>) -> Ended {
    let tail = tail_lines(&r.log, TAIL, r.fmt).iter().map(|l| redact::redact(l, secrets)).collect();
    let _ = truncate_log(&r.log, LOG_HEAD, LOG_TAIL, LOG_MAX);
    let (how, why) = if r.timeout.is_some() { ("timeout", r.timeout.clone()) } else { (how, None) };
    Ended { id: r.d.id, code, tail, events: recent_lines(r), how, why, quota }
}

/// 一次派发的额度文件（见 [`Run::quota_file`]）
pub(super) fn quota_file_of(run: &Path, id: i64) -> PathBuf {
    run.join(format!("{id}.quota"))
}

/// 一次派发收尾后给调用方打印用的信息
pub(super) struct RunSummary {
    pub(super) board: String,
    pub(super) card: i64,
    pub(super) name: Option<String>,
    /// 派发已经被服务端结束（机器掉线）
    pub(super) lost: bool,
    /// 兜底停止的原因
    pub(super) why: Option<String>,
    /// 派发 ID
    pub(super) id: i64,
    /// agent 在哪里跑的；`managed` 时是 board host 备好的 worktree
    pub(super) cwd: PathBuf,
    pub(super) managed: bool,
    /// 有人请求停止（或 board host 自己要停它）之后才退出的
    pub(super) stopped: bool,
    /// agent 最后做的几件事（事件的文字，没有事件流时是日志最后几行）
    pub(super) tail: Vec<String>,
}

/// 还没上报成功的结束（连不上服务端时下一轮再报）
pub(super) struct Ended {
    pub(super) id: i64,
    pub(super) code: Option<i64>,
    pub(super) tail: Vec<String>,
    /// 最后 10 个事件的文字（RunEvent::line）
    pub(super) events: Vec<String>,
    pub(super) how: &'static str,
    /// how = timeout 时给人看的原因
    pub(super) why: Option<String>,
    /// board acp 看到的额度（#668）
    pub(super) quota: Option<board_core::hosts::Quota>,
}

/// 结束上报失败：派发不存在这类明确的 4xx 不用再试；其余错误（连不上、5xx 等）留到下一轮
pub(super) fn permanent_end_error(code: ErrorCode) -> bool {
    matches!(code, ErrorCode::NotFound | ErrorCode::BoardNotFound | ErrorCode::NoSuchHost | ErrorCode::Usage)
}

/// 定时领取：这一分钟到点、机器没暂停、也没有正在进行的一轮
/// 会话（同一个 agent 进程）正在跑的那一张：board next 之后服务端开了新的派发，session 相同、ID 更大
/// 旧版服务端不发 session（为 0）：不跟随
pub(super) fn successor(d: &Dispatch, running: &[Dispatch]) -> Option<Dispatch> {
    if d.session == 0 {
        return None;
    }
    running.iter().filter(|x| x.session == d.session && x.id > d.id).max_by_key(|x| x.id).cloned()
}

/// 派到这台机器的卡片在哪个目录里做：先看派发开始那一刻记下的目录（`<工作空间>/<项目>`，机器之后换了
/// 工作空间也还按这里），没有再按现在的工作空间算
pub(super) fn dispatch_dir(d: &board_core::Dispatch, ws: &HostWorkspace, os: &str) -> Option<PathBuf> {
    d.path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(|p| expand(p))
        .or_else(|| board_core::hosts::derive_dir(ws, &d.board, os).map(|x| expand(&x.path)))
}

/// 断开 OFFLINE_WARN 时打印的提醒
pub(super) fn offline_note(gap: Duration, running: usize) -> String {
    let base = format!("和服务端断开 {} 秒了", gap.as_secs());
    if running == 0 {
        return format!("{base}（继续重试）");
    }
    format!(
        "{base}：{running} 个 agent 还在跑。超过 50 秒服务端会判这台机器掉线、放回它们的卡片，别的机器可能接走；恢复连接后这些 agent 会被结束（它们推集成分支前会先用 board hold 确认领取）"
    )
}

/// 目录里没推到远端的改动：没提交的文件、不在任何远端分支上的提交。不是 git 仓库、都推了时为空；
/// 没有远端的仓库不看提交（所有提交都不在远端，说明不了什么）
pub(super) fn unpushed(dir: &Path) -> Option<String> {
    if !dir.is_dir() || git_line(dir, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return None;
    }
    let mut parts = vec![];
    let dirty = git_line(dir, &["status", "--porcelain"]).map(|s| s.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0);
    if dirty > 0 {
        parts.push(format!("{dirty} 个文件没提交"));
    }
    if git_line(dir, &["remote"]).is_some() {
        if let Some(head) = git_line(dir, &["rev-parse", "--short", "HEAD"]) {
            if git_line(dir, &["branch", "-r", "--contains", "HEAD"]).is_none() {
                parts.push(format!("提交 {head} 不在任何远端分支上"));
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("，"))
}

/// 启动 agent 的提示词（一行 skill 调用，后面带 board host 的身份提示）。
/// 做卡：定时领取的会话做完一张后用 board next 接着做（手动派发的会话 board next 会返回 SESSION_DONE）；
/// 拆解「未整理」里的条目：不领取，先按粒度预检（偏小的不拆：能直接做的不建卡，只是原始需求的整理成一张），
/// 拆完评论后直接结束（PRD §9.1）
/// 按角色派发时这一个进程的设置（见 [`Host::role_run`]）
pub(super) struct RoleRun {
    /// 模型、强度：空 = 不切
    pub(super) model: String,
    pub(super) effort: String,
    /// `providers/set` 的参数文件（Claude Code、Codex 用供应商时）
    pub(super) provider_file: Option<PathBuf>,
    /// dsh 用供应商时：这次的补丁文件和要接上的几条
    pub(super) dsh_patch: Option<(PathBuf, String)>,
    /// dsh 用供应商时的 key（放进环境变量）
    pub(super) dsh_key: Option<String>,
    /// 「官方订阅」的角色（#667）：board acp 开会话前确认 agent 用的是订阅（`--auth subscription`）
    pub(super) subscription: bool,
}

/// 写只有自己能读的文件（里面有 key）
fn write_secret(path: &Path, text: &str) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    use std::io::Write as _;
    let mut f = o.open(path)?;
    if let Err(e) = f.write_all(text.as_bytes()) {
        drop(f);
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

pub(super) fn dispatch_prompt(d: &Dispatch, project: &str, board: &str, name: &str, host: &str, ready: Option<&str>) -> String {
    if d.kind == board_core::hosts::KIND_SPLIT {
        let card = d.card;
        return format!(
            "/board-skills 拆解 {project}项目之中的{board}白板中的#{card}需求（board host 派发：BOARD_AS={name}，BOARD_HOST={host}；名字已经上线，不要 check、不要心跳、不要 claim，不要创建 goal；先按粒度预检：这一条如果只做一件事、能整个做完，就不要拆——已经写清范围和验收标准、能直接做的不建卡，在原条目上评一句「任务粒度合适，未进行拆解」后直接结束；只是原始需求的整理成一张能直接做的卡（board new --inbox --from {card}），在原条目上评论整理成了哪张后直接结束；偏大才拆，拆出的卡用 board new --inbox --from {card}，在原条目上评论拆成了哪几张后直接结束，不要 board next）"
        );
    }
    // board host 已经备好代码时告诉 agent：当前目录就是它的 worktree，不用自己建、不要删
    let code = match ready {
        Some(branch) => format!("；代码已经备好：当前目录就是你这个会话的 worktree，停在刚拉下来的 origin/{branch} 上，不要另建 worktree，会话结束时也不要删它"),
        None => String::new(),
    };
    format!(
        "/board-skills 开始执行 {project}项目之中的{board}白板中的#{}任务（board host 派发：BOARD_AS={name}，BOARD_HOST={host}；卡片已由你领取，不要 check、不要 claim、不要心跳，不要创建 goal{code}；交审或交给人之后执行 board next --json，领到卡片就接着做，返回 SESSION_DONE 就结束）",
        d.card
    )
}

/// board-skills 给这个名字的会话开的 worktree：主仓库目录旁边的 `<仓库目录名>.wt/<名字，/ 换成 ->`
pub(super) fn skill_worktree(repo: &Path, name: &str) -> Option<PathBuf> {
    let common = git_line(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let main = Path::new(&common).parent()?.to_path_buf();
    let base = main.file_name()?.to_string_lossy().to_string();
    Some(main.parent()?.join(format!("{base}.wt")).join(name.replace('/', "-")))
}

/// 一次派发结束后可能留着改动的地方：agent 的 worktree（board-skills 的约定）和白板目录本身；只列有没推改动的
pub(super) fn leftovers(repo: &Path, name: &str) -> Vec<(PathBuf, String)> {
    let mut dirs: Vec<PathBuf> = vec![];
    if !name.is_empty() {
        if let Some(wt) = skill_worktree(repo, name) {
            dirs.push(wt);
        }
    }
    dirs.push(repo.to_path_buf());
    dirs.into_iter().filter_map(|d| unpushed(&d).map(|w| (d, w))).collect()
}

impl Host<'_> {
    /// 上一个同名 board host 没走完退出流程（被 kill -9、崩溃）时留下的 agent：按 run 目录里的 `.proc`
    /// 结束进程号和启动时间都对得上的进程组（先 SIGTERM，几秒后还在就 SIGKILL；WSL 在 WSL 里结束 agent 的进程组）
    pub(super) fn kill_orphans(&self) {
        let Ok(dir) = std::fs::read_dir(&self.run) else { return };
        let Some(mut ps) = Procs::new() else { return };
        let mut left: Vec<Proc> = vec![];
        for e in dir.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("proc") {
                continue;
            }
            let Ok(s) = std::fs::read_to_string(&path) else { continue };
            let mut it = s.split_whitespace();
            let (Some(pid), Some(start)) = (it.next(), it.next()) else { continue };
            // 不知道启动时间的不动：进程号可能已经被复用
            let Some(proc) = Proc::parse(&format!("{pid} {start}")).filter(|p| p.start != 0) else { continue };
            let env = it.next().unwrap_or_default();
            let what = it.next().unwrap_or_default();
            if !ps.alive(proc) {
                continue;
            }
            match self.envs.iter().find(|x| x.id == env && x.wsl().is_some()) {
                Some(wsl) => {
                    let inner = std::fs::read_to_string(path.with_extension("pid")).ok().and_then(|s| s.trim().parse::<u32>().ok());
                    if let Some(pid) = inner {
                        envs::wsl_kill(wsl, pid, true);
                    }
                    kill_tree(proc.pid, true);
                }
                None => {
                    kill_tree(proc.pid, false);
                    left.push(proc);
                }
            }
            self.say(format!("{} 结束上一次留下的 agent：{what}（进程 {}）", self.p.yellow("◼"), proc.pid));
        }
        let begun = Instant::now();
        while left.iter().any(|p| ps.alive(*p)) && begun.elapsed() < ORPHAN_KILL_AFTER {
            std::thread::sleep(Duration::from_millis(100));
        }
        for p in left {
            if ps.alive(p) {
                kill_tree(p.pid, true);
            }
        }
    }

    /// 启动 `board acp` 的命令（acp.rs）：在 check 的启动环境里、dir 目录里跑这个 agent 的 ACP 入口，提示词在 prompt_file。
    /// 派发和自检都用它；标准输出、错误和 BOARD_* 环境变量由调用方接上。extra_turns：这一轮结束时名字还持有卡片，
    /// 最多再续几轮（0 = 不续）
    pub(super) fn acp_command(&self, spec: &Spec, check: &Check, env: &Env, dir: &Path, prompt_file: &Path, extra_turns: u32, role: Option<&RoleRun>, more: &[String]) -> std::result::Result<Command, String> {
        if let Some(since) = spec.builtin.map(|b| b.acp_since).filter(|s| !s.is_empty()) {
            if older_than(&check.version, since) {
                return Err(format!("{} 的版本太旧（{}）：它的 ACP 从 {since} 起才有，先升级", spec.name, check.version.trim()));
            }
        }
        // dsh 的 acp profile 也挂着 goal：带一份关掉自动续轮的补丁，见 DSH_PATCH。按角色用供应商的派发另写一份，
        // 后面接上这次的路由（只对这一个进程）
        let patch = match role.and_then(|r| r.dsh_patch.as_ref()) {
            Some((file, _)) => file.clone(),
            None => self.run.join("dsh-acp.patch.yml"),
        };
        if spec.acp.iter().any(|a| a.contains("{patch}")) {
            write_dsh_patch(&patch).map_err(|e| format!("无法写 {}：{e}", patch.display()))?;
            if let Some((_, extra)) = role.and_then(|r| r.dsh_patch.as_ref()) {
                let base = std::fs::read_to_string(&patch).unwrap_or_default();
                std::fs::write(&patch, format!("{base}\n{extra}")).map_err(|e| format!("无法写 {}：{e}", patch.display()))?;
            }
        }
        let (prog, args) = spec.entry(check, &patch.display().to_string())?;
        let mut c = Command::new(&self.exe);
        c.arg("acp").arg("--env").arg(&env.id).arg("--env-prog").arg(&env.prog).arg("--cwd").arg(dir).arg("--prompt-file").arg(prompt_file);
        if !spec.acp_mode.is_empty() {
            c.arg("--mode").arg(&spec.acp_mode);
        }
        match role {
            // 按角色派发：模型、强度按 ACP 配置项的类别切；用供应商时先 providers/set（文件里有 key，board acp 读完就删）
            Some(r) => {
                if !r.model.is_empty() {
                    c.arg("--set").arg(format!("@model={}", r.model));
                }
                if !r.effort.is_empty() {
                    c.arg("--set").arg(format!("@thought_level={}", r.effort));
                }
                if let Some(f) = &r.provider_file {
                    c.arg("--provider-file").arg(f);
                }
                if r.subscription {
                    c.arg("--auth").arg("subscription");
                }
            }
            // agent 配置文件里现在的思考强度：会话配置里有 reasoning_effort 这一项时切过去（dsh 的 ACP 只能这样定强度）
            None => {
                if let Some((_, effort)) = self.agent_now.get(&spec.id).filter(|(_, e)| !e.trim().is_empty()) {
                    c.arg("--set").arg(format!("reasoning_effort={}", effort.trim()));
                }
            }
        }
        // 结束这一轮就是结束会话：卡片还在手里时续一轮（#454）
        if extra_turns > 0 {
            c.arg("--extra-turns").arg(extra_turns.to_string());
        }
        // 机器对话：`--chat <状态文件>`、`--resume <会话>`（chats.rs）
        c.args(more);
        c.arg("--").arg(&prog).args(&args).current_dir(dir);
        for (k, v) in launch_env(spec.acp_vars(check), &prog, env.wsl().is_some(), &self.host_dir().join("npm-cache")) {
            c.env(k, v);
        }
        // dsh 用角色的供应商：key 只给这一个进程（补丁里写的是环境变量名）
        if let Some(key) = role.and_then(|r| r.dsh_key.as_ref()) {
            c.env(crate::providersync::ROLE_KEY_ENV, key);
        }
        // 在 Claude Code 的会话里运行 board host 时，别让 agent 以为自己是嵌套会话
        c.env_remove("CLAUDECODE");
        Ok(c)
    }

    /// 按角色派发（`d.profile` 不为空）：这一个进程用的模型、强度、供应商。角色用供应商时现取一次供应商的完整内容
    /// （含 key，只给这台机器的令牌），Claude Code、Codex 写进 0600 的文件交给 board acp，dsh 写这次的补丁
    pub(super) fn role_run(&self, d: &Dispatch) -> std::result::Result<Option<RoleRun>, String> {
        if d.profile.is_empty() {
            return Ok(None);
        }
        let model = d.model.clone().unwrap_or_default();
        let effort = d.effort.clone().unwrap_or_default();
        let mut out = RoleRun { model: model.clone(), effort: effort.clone(), provider_file: None, dsh_patch: None, dsh_key: None, subscription: false };
        if d.provider.is_empty() {
            return Ok(Some(out));
        }
        // 官方订阅：不换线路，只在开会话前确认是订阅（#667）
        if d.provider == board_core::profiles::SUBSCRIPTION {
            out.subscription = true;
            return Ok(Some(out));
        }
        let lib: crate::providersync::Lib = self
            .rm()
            .get(&format!("/api/hosts/{}/providers", self.name), &[])
            .map_err(|e| format!("取不到角色「{}」用的供应商「{}」：{}", d.profile_name, d.provider, e.message))?;
        self.remember_secrets(lib.providers.iter().filter_map(|p| p.api_key.clone()));
        let p = lib.providers.iter().find(|p| p.name == d.provider).ok_or_else(|| format!("共用配置里已经没有供应商「{}」了：改一下角色「{}」", d.provider, d.profile_name))?;
        match crate::providersync::route_for(&d.agent, p, &model, &effort)? {
            crate::providersync::Route::Acp(v) => {
                let f = provider_file(&self.run, d.id);
                let _ = std::fs::create_dir_all(&self.run);
                write_secret(&f, &v.to_string()).map_err(|e| format!("无法写 {}：{e}", f.display()))?;
                out.provider_file = Some(f);
            }
            crate::providersync::Route::Dsh { patch, key } => {
                out.dsh_patch = Some((dsh_patch_file(&self.run, d.id), patch));
                out.dsh_key = Some(key);
            }
        }
        Ok(Some(out))
    }

    /// 机器的兜底设置：单次最长运行、没输出多久算卡住（分钟，0 = 不限）。每次从服务端的最新设置算，
    /// 改了不用重启 board host 就生效
    pub(super) fn limits(&self) -> (Option<i64>, Option<i64>) {
        let Some(w) = &self.work else { return (None, None) };
        let mins = |n: i64| (n > 0).then_some(n);
        (mins(w.host.max_run), mins(w.host.idle_run))
    }

    /// 到时间按「人停止」的流程结束进程树（先 SIGTERM，10 秒后 SIGKILL），原因记在 `Run.timeout` 里，
    /// 进程退出后由 reap 上报结束（结果「超时」）。返回有没有新的超时
    ///
    /// - 单次最长运行：从启动算起
    /// - 没输出多久算卡住：只有已经输出过内容的派发生效。内置模板都用结构化输出，运行中每一步都写日志；配置文件里改成
    ///   只在结束时打印的（例如 Claude Code 的 `-p` 文本输出）运行中一行都没有，所以这条不适用于一行都还没输出的派发
    pub(super) fn enforce_timeouts(&mut self) -> bool {
        let (max_run, idle_run) = self.limits();
        let min = minute();
        let mut said = vec![];
        let mut any = false;
        for r in &mut self.runs.iter_mut().filter(|r| r.timeout.is_none()) {
            // 日志变长了就是有输出：「没输出多久算卡住」从这里算
            let len = std::fs::metadata(&r.log).map(|m| m.len()).unwrap_or(0);
            if len > r.seen_len {
                r.seen_len = len;
                r.output_at = Instant::now();
            }
            // 原因只写「运行超过 30 分钟」「20 分钟没有任何输出」：服务端在评论里补上「，board host 停止了它」
            let run = max_run.filter(|n| r.spawned.elapsed() >= min * *n as u32).map(|n| {
                format!("运行超过 {}", span_words(Duration::from_secs(n as u64 * 60)))
            });
            let idle = idle_run.filter(|n| r.seen_len > 0 && r.output_at.elapsed() >= min * *n as u32).map(|n| {
                format!("{}没有任何输出", span_words(Duration::from_secs(n as u64 * 60)))
            });
            let Some(why) = run.or(idle) else { continue };
            r.timeout = Some(why.clone());
            if r.term_at.is_none() {
                r.stop(false);
                r.term_at = Some(Instant::now());
            }
            said.push(format!("停止 {}#{}：{why}", r.d.board, r.d.card));
            any = true;
        }
        for s in said {
            self.say(format!("{} {s}", self.p.yellow("◼")));
        }
        any
    }

    /// agent 退出：上传剩下的日志，上报结束。返回有没有派发结束
    pub(super) fn reap(&mut self) -> bool {
        let mut done = false;
        let mut i = 0;
        while i < self.runs.len() {
            let status = match self.runs[i].child.try_wait() {
                Ok(Some(s)) => Some(s),
                Ok(None) => None,
                Err(_) => None,
            };
            let Some(status) = status else {
                i += 1;
                continue;
            };
            let r = self.runs.remove(i);
            let code = exit_code(status);
            let ended = self.finish_run(r, code, "exit");
            let what = match &ended.why {
                Some(why) => why.clone(),
                None => format!("agent 退出，退出码 {}", code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into())),
            };
            self.say(format!(
                "{} {}  {}  {}",
                self.p.dim("■"),
                self.p.cyan(&format!("{}#{}", ended.board, ended.card)),
                ended.name.as_deref().unwrap_or_default(),
                self.p.dim(&what)
            ));
            self.handoff(&ended, code);
            // 派发已经被服务端结束（机器掉线、卡片可能已被别人接走）：agent 的改动没推上去的话，写明在哪里，方便人捞回来
            if ended.lost && !ended.managed {
                if let Some(repo) = ws_dir(&self.ws, &ended.board, &self.os) {
                    for (dir, what) in leftovers(&repo, ended.name.as_deref().unwrap_or_default()) {
                        self.say(format!(
                            "{} {} 里有没推到远端的改动（{what}）：{}#{} 的派发已被服务端结束，需要的话去那里捞回来",
                            self.p.yellow("!"),
                            dir.display(),
                            ended.board,
                            ended.card
                        ));
                    }
                }
            }
            done = true;
        }
        if done {
            self.report_ended();
        }
        done
    }

    /// 一次派发结束的收尾，`reap`（agent 自己退出）和 `shutdown`（board host 退出）共用：
    ///
    /// 1. 主进程已经不在了，它起的子进程可能还在同一个进程组里：`kill_rest()` 一起结束
    /// 2. 删掉这次派发自己的临时文件（配置、进程号、结果文件）
    /// 3. 上传剩下的日志和事件（`last = true`，最后不完整的一行也读）
    /// 4. 取最后几行做错误摘录，再截断过大的日志。截断要在 agent 确实不在了之后：它还活着时换掉文件内容，
    ///    下次写入会从旧位置接着写、中间留一段空洞
    /// 5. 记一笔 `Ended`，等 `report_ended()` 上报给服务端
    ///
    /// `how` 是给服务端的结果：`exit`（agent 退出）、`host_exit`（board host 退出时停的）；
    /// 兜底停止过（`Run.timeout` 有值）的无论哪种都报 `timeout`，原因照原样带上。
    pub(super) fn finish_run(&mut self, mut r: Run, code: Option<i64>, how: &'static str) -> RunSummary {
        r.kill_rest();
        let quota = r.quota();
        r.cleanup();
        self.flush_log(&r, true);
        self.flush_events(&mut r, true);
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let ended = ended_of(&r, code, how, &secrets, quota);
        let tail = if ended.events.is_empty() { ended.tail.iter().rev().take(10).rev().cloned().collect() } else { ended.events.clone() };
        self.ended.push(ended);
        RunSummary {
            board: r.d.board.clone(),
            card: r.d.card,
            name: r.d.name.clone(),
            lost: r.lost,
            why: r.timeout.clone(),
            id: r.d.id,
            cwd: r.cwd.clone(),
            managed: r.managed,
            stopped: r.term_at.is_some(),
            tail,
        }
    }

    /// 派发结束时 agent 的 worktree 里还有没推的改动（没提交的文件、不在远端的提交）：提交、推成 `wip/<白板标识>/<ID>`，
    /// 再让服务端在卡片上评论（怎么结束的、留下了什么、agent 最后做了什么）。agent 自己交审了、或者自己推了 wip 分支时
    /// worktree 是干净的，什么也不做。推不上去时在本机输出里写明，改动留在 worktree 里（下一次派发不会动它）
    pub(super) fn save_wip(&mut self, ended: &RunSummary, code: Option<i64>) {
        let how = if ended.lost {
            "机器和服务端断开超过 50 秒，派发被服务端结束，卡片可能已经被别的机器接走".to_string()
        } else if let Some(why) = &ended.why {
            why.clone()
        } else if ended.stopped {
            "被停止".to_string()
        } else {
            format!("agent 退出，退出码 {}", code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into()))
        };
        match save_run_wip(ended, &self.name) {
            Ok(None) => {}
            Ok(Some(w)) => {
                self.say(format!(
                    "{} {} 没推的改动（{}）已经保存到分支 {}",
                    self.p.green("✓"),
                    self.p.cyan(&format!("{}#{}", ended.board, ended.card)),
                    w.changes,
                    w.branch
                ));
                let secrets = self.secrets();
                let tail: Vec<String> = ended.tail.iter().map(|l| crate::redact::redact(l, &secrets.iter().map(String::as_str).collect::<Vec<_>>())).collect();
                let body = json!({ "branch": w.branch, "how": how, "changes": w.changes, "tail": tail });
                if let Err(e) = self.rm().post::<Value>(&format!("/api/dispatches/{}/wip", ended.id), body) {
                    self.say(format!("{} 没能在卡片上评论 wip 分支 {}：{}", self.p.yellow("!"), w.branch, e.message));
                }
            }
            Err(e) => self.say(format!(
                "{} {} 里有没推到远端的改动，没能保存成 wip 分支（{e}）：留在那里，需要的话去捞回来",
                self.p.yellow("!"),
                ended.cwd.display()
            )),
        }
    }

    /// 半成品交接：board host 备的 worktree 里还有没推到远端的改动，保存成 wip 分支并在卡片上评论，接手的不用从头来。
    /// `reap`（agent 自己退出）和 `shutdown`（升级 / 重启 / 退出桌面端）都走这里；用完留着 worktree，只解锁不删除（#622）。
    pub(super) fn handoff(&mut self, ended: &RunSummary, code: Option<i64>) {
        if ended.managed {
            self.save_wip(ended, code);
            crate::code::release(&ended.cwd);
        }
    }

    /// 上报结束；连不上时留到下一轮
    pub(super) fn report_ended(&mut self) {
        let ended = std::mem::take(&mut self.ended);
        for e in ended {
            let body = json!({ "exitCode": e.code, "tail": e.tail, "events": e.events, "how": e.how, "why": e.why, "quota": e.quota });
            match self.rm().post::<Dispatch>(&format!("/api/dispatches/{}/end", e.id), body) {
                Ok(d) => {
                    let res = d.result.as_deref().map(result_words).unwrap_or("还在跑");
                    self.say(format!("{} {}  结果：{}", self.p.dim("·"), self.p.cyan(&format!("{}#{}", d.board, d.card)), self.p.bold(res)));
                }
                // 派发不存在这类明确的拒绝不用再报；其余错误（连不上、5xx 等）留到下一轮重试
                Err(err) if permanent_end_error(err.code) => {}
                Err(_) => self.ended.push(e),
            }
        }
    }

    /// 停止请求：服务端要停的、服务端已经不当它在跑的（机器掉线、被移除），结束进程树；10 秒后还在就强制结束
    pub(super) fn enforce(&mut self) {
        let Some(w) = &self.work else { return };
        let work_at = self.work_at;
        let mut say: Vec<String> = vec![];
        for r in self.runs.iter_mut() {
            if r.term_at.is_none() {
                let on_server = w.running.iter().find(|x| x.id == r.d.id);
                let why = match on_server {
                    Some(x) => x.stop_by.as_ref().map(|by| format!("{by} 请求停止")),
                    None if work_at > r.spawned => {
                        r.lost = true;
                        Some("服务端上这次派发已经结束".to_string())
                    }
                    None => None,
                };
                if let Some(why) = why {
                    r.stop(false);
                    r.term_at = Some(Instant::now());
                    say.push(format!("停止 {}#{}：{why}", r.d.board, r.d.card));
                }
            } else if !r.killed && r.term_at.is_some_and(|t| t.elapsed() >= KILL_AFTER) {
                r.stop(true);
                r.killed = true;
            }
        }
        for s in say {
            self.say(format!("{} {s}", self.p.yellow("◼")));
        }
    }

    /// 有空位、没有暂停时：按创建先后开始排队的派发；正在一轮领取时向服务端要下一张。返回有没有变化
    pub(super) fn fill(&mut self) -> Result<bool> {
        let Some(w) = self.work.clone() else { return Ok(false) };
        // 等重启、升级时不开始新的派发
        let restarting = w.host.ask.as_ref().is_some_and(|a| a.kind == "restart" || a.kind == "upgrade") || self.updating.is_some();
        if self.hold || w.host.paused || restarting {
            return Ok(false);
        }
        let max = w.host.max.max(1) as usize;
        let mut changed = false;
        for q in &w.queued {
            if self.live() >= max {
                return Ok(changed);
            }
            if self.runs.iter().any(|r| r.d.id == q.id) {
                continue;
            }
            let body = json!({ "instance": self.instance, "id": q.id });
            match self.rm().post::<Dispatch>(&format!("/api/hosts/{}/begin", self.name), body) {
                Ok(d) => {
                    changed = true;
                    self.started(d);
                }
                Err(e) if matches!(e.code, ErrorCode::HostNameTaken | ErrorCode::InvalidToken) => return Err(e),
                Err(_) => {
                    self.hold = true;
                    return Ok(true);
                }
            }
        }
        if w.host.round {
            while self.live() < max {
                let body = json!({ "instance": self.instance });
                match self.rm().post::<Option<Dispatch>>(&format!("/api/hosts/{}/pull", self.name), body) {
                    Ok(Some(d)) => {
                        changed = true;
                        self.started(d);
                    }
                    Ok(None) => {
                        self.hold = true;
                        return Ok(true);
                    }
                    Err(e) if matches!(e.code, ErrorCode::HostNameTaken | ErrorCode::InvalidToken) => return Err(e),
                    Err(_) => {
                        self.hold = true;
                        return Ok(true);
                    }
                }
            }
        }
        Ok(changed)
    }

    /// 还没在停的 agent 个数（占着会话位：一个会话 = 一个 agent 进程）
    pub(super) fn live(&self) -> usize {
        self.runs.len()
    }

    /// 服务端开始了一次派发（已领取）：在本机启动 agent；没领到时只打印
    pub(super) fn started(&mut self, d: Dispatch) {
        if d.state != "running" {
            self.say(format!(
                "{} {}  {}",
                self.p.dim("·"),
                self.p.cyan(&format!("{}#{}", d.board, d.card)),
                self.p.dim(&format!("没领到（{}）", d.result.as_deref().map(result_words).unwrap_or("已结束")))
            ));
            return;
        }
        // 先记进任务里：重新取到任务之前不会被当成「服务端已经结束」
        if let Some(w) = self.work.as_mut() {
            w.running.push(d.clone());
            w.queued.retain(|x| x.id != d.id);
        }
        // 缓存里的命令路径不在了（升级失败被 npm 挪走、被人删了）：先重新找一遍，不拿失效的路径去启动（#536）
        self.recheck_if_gone(&d.agent);
        // 派发前更新代码：拉最新、备好这个会话的 worktree（PRD §9.1「派发前更新代码」）；没成就不启动 agent
        let started = self.prepare(&d).and_then(|code| self.spawn(&d, code.as_ref()));
        match started {
            Ok(r) => {
                // 先记进 runs 再打印：之后无论发生什么，退出流程都会结束它
                let env = r.env.name.clone();
                self.runs.push(r);
                let what = if d.kind == board_core::hosts::KIND_SPLIT { format!("拆解：{}", d.title) } else { d.title.clone() };
                self.say(format!(
                    "{} {}  {}  {}  {}",
                    self.p.green("▶"),
                    self.p.cyan(&format!("{}#{}", d.board, d.card)),
                    self.p.yellow(d.name.as_deref().unwrap_or_default()),
                    self.p.dim(&env),
                    what
                ));
            }
            Err(msg) => {
                self.say(format!("{} {}  {}", self.p.red("✗"), self.p.cyan(&format!("{}#{}", d.board, d.card)), msg));
                let log = self.logs.join(format!("{}.log", d.id));
                let _ = std::fs::write(&log, format!("{msg}\n"));
                self.ended.push(Ended { id: d.id, code: None, tail: vec![msg], events: vec![], how: "exit", why: None, quota: None });
                self.report_ended();
            }
        }
    }

    /// 这次派发带给 agent 的 `BOARD_*` 变量，写进它的 `BOARD_CONFIG` 的 `[agent_env]`（见 [`write_run_config`]）：
    /// `scripts/board` 读它们再设进自己的环境——agent 会过滤传给命令的环境变量（Codex 的 shell 环境策略、
    /// dsh 去掉带 TOKEN 的），只靠进程环境传给 agent 的话，它执行 `board` 时就看不到连接。
    /// 不含派发 ID 之后才写的 `BOARD_PIDFILE`（那是给 WSL 包装脚本的）
    pub(super) fn agent_env(&self, d: &Dispatch, name: &str, conf: &Path) -> Vec<(String, String)> {
        let s = |v: &str| v.to_string();
        vec![
            ("BOARD_URL".into(), s(self.rm().url())),
            ("BOARD_TOKEN".into(), s(self.rm().token())),
            ("BOARD_AS".into(), s(name)),
            ("BOARD_HOST".into(), s(&self.name)),
            ("BOARD".into(), s(&d.board)),
            ("BOARD_CONFIG".into(), conf.display().to_string()),
            ("BOARD_EXE".into(), self.exe.display().to_string()),
        ]
    }

    /// 派发前把代码准备好：项目目录是有 `origin` 的 git 仓库时，先 `git fetch`，再备好这个会话的 worktree（停在最新的
    /// 集成分支上），agent 就在 worktree 里启动（`code.rs`）。返回 None 表示照旧在项目目录里启动：没有仓库的项目、
    /// 拆解（不改代码）、WSL 里的 agent（worktree 要由 WSL 里的 git 来建，还是由 skill 自己建）。
    /// 拉不到最新代码时记下来、马上报给服务端：这台机器先不接这个项目的卡
    pub(super) fn prepare(&mut self, d: &Dispatch) -> std::result::Result<Option<crate::code::Ready>, String> {
        if d.kind == board_core::hosts::KIND_SPLIT || d.env.as_deref().is_some_and(|e| e.starts_with("wsl:")) {
            return Ok(None);
        }
        let Some(dir) = dispatch_dir(d, &self.ws, &self.os) else { return Ok(None) };
        if !crate::code::is_repo(&dir) || !crate::code::has_origin(&dir) {
            return Ok(None);
        }
        // 服务端的 Git 凭据：同步之后才克隆、才有的仓库在这里补配上，拉代码和 agent 推送都用它
        if let Some(line) = gitsync::ensure(&self.host_dir()) {
            self.say(format!("{} {line}", self.p.green("✓")));
        }
        // 没有会话名（旧版、异常的服务端）：不准备、不删任何目录，这张派发直接结束（#508）
        let name = d.name.clone().filter(|n| !n.trim().is_empty()).ok_or_else(|| "派发没有会话名，没有启动 agent：服务端可能太旧，先升级服务端".to_string())?;
        let in_use: Vec<PathBuf> = self.runs.iter().map(|r| r.cwd.clone()).collect();
        let online = self.online_names();
        match crate::code::prepare(&dir, &name, d.id, &d.branch, &in_use, online.as_deref()) {
            Ok(r) => {
                if r.adopted {
                    self.say(format!("{} 接手了空闲的 worktree {}（依赖、构建产物都在）", self.p.dim("·"), r.cwd.display()));
                }
                if self.fetch_errs.iter().any(|(p, ..)| *p == dir) {
                    self.fetch_errs.retain(|(p, ..)| *p != dir);
                    self.beat_soon = true;
                }
                match &r.stale {
                    Some((dir, None)) => self.say(format!("{} 删掉了残留的 worktree 目录 {}（git 已经不登记它），重新建好", self.p.dim("·"), dir.display())),
                    Some((dir, Some(e))) => self.say(format!(
                        "{} 残留的 worktree 目录 {} 删不掉（{e}）：这次在 {} 里做，下次用到这个名字时再删",
                        self.p.yellow("!"),
                        dir.display(),
                        r.cwd.display()
                    )),
                    None => {}
                }
                if let Some((dir, why)) = &r.kept {
                    self.say(format!(
                        "{} {} 像是残留的 worktree 目录，但{why}：没有删它，确认没用后手动删；这次在 {} 里做",
                        self.p.yellow("!"),
                        dir.display(),
                        r.cwd.display()
                    ));
                }
                if let Some((left, what)) = &r.left {
                    self.say(format!(
                        "{} {} 里有上一次没推到远端的改动（{what}），没有动它：这次在 {} 里做",
                        self.p.yellow("!"),
                        left.display(),
                        r.cwd.display()
                    ));
                }
                Ok(Some(r))
            }
            Err(crate::code::Fail::Fetch(e)) => {
                self.fetch_errs.retain(|(p, ..)| *p != dir);
                self.fetch_errs.push((dir.clone(), e.clone(), Instant::now()));
                self.beat_soon = true;
                Err(format!("拉不到 {} 的最新代码，没有启动 agent：{e}", dir.display()))
            }
            Err(crate::code::Fail::Other(e)) => Err(format!("没能准备好代码，没有启动 agent：{e}")),
        }
    }

    /// 在派发记录写的启动环境里启动 agent。旧版服务端不写启动环境、或写的环境本机已经没有：按本机的默认（第一个找到的）
    pub(super) fn spawn(&self, d: &Dispatch, code: Option<&crate::code::Ready>) -> std::result::Result<Run, String> {
        let spec = self.agents.iter().find(|a| a.id == d.agent).ok_or_else(|| format!("本机没有 agent「{}」", d.agent))?;
        let cmd0 = spec.bin.clone();
        let check = d
            .env
            .as_deref()
            .and_then(|e| spec.checks.iter().find(|c| c.env == e))
            .or_else(|| spec.default_check())
            .ok_or_else(|| format!("本机没找到 agent「{}」的命令 {cmd0}", d.agent))?;
        let env = self.envs.iter().find(|e| e.id == check.env).cloned().ok_or_else(|| format!("本机没有启动环境「{}」", check.env))?;
        let prog = check.prog.clone().ok_or_else(|| format!("在{}里没找到 agent「{}」的命令 {cmd0}", env.name, d.agent))?;
        // 服务端不会派给缺 Node.js 的环境；旧版服务端照旧派过来时在这里拦住，说清缺什么（#458）
        if let Some(m) = &check.missing {
            return Err(format!("在{}里缺 {m}：agent「{}」跑不起来，先装上", env.name, d.agent));
        }
        let dir = dispatch_dir(d, &self.ws, &self.os).ok_or_else(|| format!("本机还没有白板 {} 的目录：先配好工作空间", d.board))?;
        if !dir.is_dir() {
            // 目录还没建好（服务端没有这块白板的目录）：按工作空间建一个空的，agent 在里面做
            std::fs::create_dir_all(&dir).map_err(|e| format!("目录不存在：{}（建不了：{e}）", dir.display()))?;
        }
        // agent 在哪里启动：board host 备好的 worktree，没备（没有仓库的项目、拆解、WSL）时是项目目录
        let run_dir = code.map_or_else(|| dir.clone(), |c| c.cwd.clone());
        let name = d.name.clone().unwrap_or_default();
        let (project, board) = d.board.split_once('/').unwrap_or((&d.board, ""));
        let mut prompt = dispatch_prompt(d, project, board, &name, &self.name, code.map(|c| c.branch.as_str()));
        // 角色的附加说明接在提示词后面
        if !d.note.trim().is_empty() {
            prompt += &format!("\n\n角色「{}」的附加说明：{}", d.profile_name, d.note.trim());
        }
        let role = self.role_run(d)?;
        let secret_files = SecretFiles::for_role(&self.run, d.id, role.as_ref());
        // 提示词放进文件交给 board acp（作为 session/prompt 发出去），不放进命令行
        let prompt_file = self.run.join(format!("{}.prompt", d.id));
        let _ = std::fs::create_dir_all(&self.run);
        std::fs::write(&prompt_file, &prompt).map_err(|e| format!("无法写 {}：{e}", prompt_file.display()))?;
        // 拆解不领卡：不续轮；做卡的这一轮结束时卡片还在手里就续（#454）
        let turns = if d.kind == board_core::hosts::KIND_SPLIT { 0 } else { crate::acp::EXTRA_TURNS };
        // board acp 把这次看到的额度写在这里，结束时随派发上报（#668）
        let quota_file = quota_file_of(&self.run, d.id);
        let _ = std::fs::remove_file(&quota_file);
        let more = ["--quota-file".to_string(), quota_file.display().to_string()];
        let mut c = self.acp_command(spec, check, &env, &run_dir, &prompt_file, turns, role.as_ref(), &more).inspect_err(|_| {
            let _ = std::fs::remove_file(&prompt_file);
            if let Some(f) = role.as_ref().and_then(|r| r.provider_file.as_ref()) {
                let _ = std::fs::remove_file(f);
            }
        })?;
        let log = self.logs.join(format!("{}.log", d.id));
        let (out, err) = create_log(&log).map_err(|e| format!("无法创建日志 {}：{e}", log.display()))?;
        let conf = self.run.join(format!("{}.toml", d.id));
        write_run_config(&conf, self.rm().url(), self.rm().token(), &self.agent_env(d, &name, &conf)).map_err(|e| e.message)?;
        c.env("BOARD_URL", self.rm().url())
            .env("BOARD_TOKEN", self.rm().token())
            .env("BOARD_AS", &name)
            .env("BOARD_HOST", &self.name)
            .env("BOARD", &d.board)
            .env("BOARD_CONFIG", &conf)
            // shell 的启动脚本里设了 BOARD_URL / BOARD_TOKEN 也不影响：包装脚本用这两个再设一遍
            .env("BOARD_HOST_URL", self.rm().url())
            .env("BOARD_HOST_TOKEN", self.rm().token())
            // skill 自带的 scripts/board 看到它就用 host 自己的 board：版本和 host 一致（skill 里打包的 board 可能更旧）
            .env("BOARD_EXE", &self.exe)
            .stdin(Stdio::piped())
            .stdout(out)
            .stderr(err);
        // WSL：agent 的进程号写进 BOARD_PIDFILE（环境变量经 WSLENV 传进去，见 acp_command）
        let mut pidfile = None;
        if env.wsl().is_some() {
            let pf = self.run.join(format!("{}.pid", d.id));
            let _ = std::fs::remove_file(&pf);
            c.env("BOARD_PIDFILE", &pf);
            pidfile = Some(pf);
        }
        envs::detach(&mut c);
        let mut child = c.spawn().map_err(|e| {
            let _ = std::fs::remove_file(&conf);
            let _ = std::fs::remove_file(&prompt_file);
            format!("在{}里启动 {prog}（board acp）失败：{e}", env.name)
        })?;
        let stdin = child.stdin.take();
        let job = Job::attach(&child).inspect_err(|_| {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&conf);
            let _ = std::fs::remove_file(&prompt_file);
        });
        let job = match job {
            Ok(job) => job,
            Err(e) => return Err(format!("无法把 board acp 放进 Job Object（停止时结束整棵进程树）：{e}")),
        };
        // 记下 agent 的进程号和启动时间：这个 host 没走完退出流程时，同名重启后据此结束它
        let procfile = self.run.join(format!("{}.proc", d.id));
        let start = Procs::new().and_then(|mut ps| ps.get(child.id())).map(|x| x.start).unwrap_or(0);
        let _ = std::fs::write(&procfile, format!("{} {start} {} {}#{}\n", child.id(), env.id, d.board, d.card));
        let fmt = Some(Stream::Acp);
        let now = Instant::now();
        let run = Run {
            d: d.clone(),
            child,
            _stdin: stdin,
            job: Some(job),
            env,
            pidfile,
            log,
            conf,
            procfile,
            sent: 0,
            fmt,
            parser: fmt.map(events::Parser::new),
            ev_read: 0,
            ev_queue: vec![],
            ev_recent: VecDeque::new(),
            fmt_due: true,
            term_at: None,
            killed: false,
            spawned: now,
            cwd: run_dir.clone(),
            managed: code.is_some(),
            lost: false,
            timeout: None,
            output_at: now,
            seen_len: 0,
        };
        secret_files.keep();
        Ok(run)
    }

    /// 服务端上在线的名字：判断别的会话留下的 worktree 是不是空闲用。查不到时为空（锁着的一律当成有人在用）
    fn online_names(&self) -> Option<Vec<String>> {
        self.rm().get::<Vec<board_core::model::AgentInfo>>("/api/agents", &[]).ok().map(|a| a.into_iter().map(|a| a.name).collect())
    }

    /// 清理空闲的 worktree（`code.rs` 的 `sweep`）：超过 36 小时没用的、多出 4 个的删掉，有没推改动的留着并写明
    pub(super) fn sweep_worktrees(&mut self) {
        let mut repos: Vec<PathBuf> = ws_pairs(&self.ws, &self.boards, &self.os).into_iter().map(|(_, p)| p).collect();
        repos.sort();
        repos.dedup();
        let in_use: Vec<PathBuf> = self.runs.iter().map(|r| r.cwd.clone()).collect();
        let online = self.online_names();
        for repo in repos {
            if !crate::code::is_repo(&repo) {
                continue;
            }
            use crate::code::{Swept, WT_IDLE_MAX, WT_UNUSED};
            for (wt, kept) in crate::code::sweep(&repo, &in_use, online.as_deref(), WT_UNUSED, WT_IDLE_MAX, std::time::SystemTime::now()) {
                match kept {
                    Swept::Removed => self.say(format!("{} 清理了 36 小时没用的 worktree {}", self.p.dim("·"), wt.display())),
                    Swept::Extra => self.say(format!("{} 空闲的 worktree 超过 {WT_IDLE_MAX} 个，清理了最久没用的 {}", self.p.dim("·"), wt.display())),
                    Swept::Unpushed(what) => self.say(format!("{} worktree {} 超过 36 小时没用，但有没推到远端的改动（{what}），留着", self.p.yellow("!"), wt.display())),
                    Swept::NoOwner(why) => {
                        self.say(format!("{} worktree {} 超过 36 小时没用，但{why}：不自动删，确认没用后手动删", self.p.yellow("!"), wt.display()))
                    }
                }
            }
        }
    }
}

/// 把一次结束的派发留下的改动保存到 `wip/<白板>/<ID>`；不是 board host 备的 worktree、或者已经干净时返回 None。
/// 单独写出来是因为 agent 自己退出和 board host 退出（升级 / 重启）都要走这一份保存逻辑。
pub(super) fn save_run_wip(ended: &RunSummary, name: &str) -> std::result::Result<Option<crate::code::Wip>, String> {
    if !ended.managed {
        return Ok(None);
    }
    let key = ended.board.split_once('/').map_or(ended.board.as_str(), |(_, b)| b);
    let branch = format!("wip/{key}/{}", ended.card);
    let alt = format!("{branch}-{name}");
    let message = format!("wip: {}#{} 没做完的改动（board host 在派发结束时保存）", ended.board, ended.card);
    crate::code::save_wip(&ended.cwd, &branch, &alt, &message)
}
