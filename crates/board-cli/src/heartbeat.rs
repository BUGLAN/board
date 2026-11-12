//! `board heartbeat start | stop | final <ID>`：一个 agent 会话只有一路后台心跳（PRD §8 规则 6）。
//!
//! - `start`：check 之后执行一次。先同步发一次 agent 级心跳（`board beat`），失败直接报错；成功后启动后台循环
//!   （`board heartbeat _loop`，不在 help 里列出），立即返回。循环每 10 秒发一次 agent 级心跳，同时维持名字在线和
//!   这个名字持有、还没 final 的每张卡片。
//! - `stop`：先让后台循环停下并等它退出（正在发的那次心跳先发完），再发 `board beat --final`，保证 final 是最后一条
//!   心跳；名字立刻离线。重复执行不报错。
//! - `final <ID>`：等同 `board beat <ID> --final`，结束这张卡片这一次的计时；之后 50 秒内要沿连线移走。
//!
//! 后台循环跟随 agent 进程（从 board 往上找第一个不是 shell 的祖先进程，Claude Code、Codex 等；可以用
//! `BOARD_HEARTBEAT_PID` 指定）。下面这些情况循环自己停下，不发 final：
//! - agent 进程不在了（崩溃、被关掉）→ 服务端 50 秒收不到心跳，名字离线，持有的卡片回到领取起点；
//! - 心跳被拒绝（名字已离线 `NAME_OFFLINE`、令牌失效等）。连不上服务端（`UNREACHABLE`）时继续重试。
//!
//! 进程信息用 sysinfo 取（Windows 上是 NtQuerySystemInformation / OpenProcess），不用 ps、WMI、PowerShell。
//!
//! 本机状态文件按名字区分：`<临时目录>/board-heartbeat/<名字>.json`，记着 check 这个名字的 agent 进程、服务端发的会话标识、
//! 后台循环的进程（进程号:启动时间）。`board check` 成功时写下；循环每 200 毫秒看一次，记的循环不是自己了（stop、重新 start）
//! 就在发完手上这次心跳后退出。
//!
//! 同名会话互相干扰（同一台机器上两个会话用了同一个名字）的保护：
//! - 服务端：agent 级心跳和 final 带着会话标识，和名字最近一次 check 发出的对不上就拒绝（`NAME_TAKEN`）；
//! - 本机：`start`、`stop`、不带 ID 的 `board beat` 只在 check 这个名字的 agent 进程里生效。名字属于另一个还活着的 agent 进程时
//!   拒绝（`NAME_TAKEN`，写明是哪个进程），不停它的循环、不发 final；那个进程已经不在了才接管。`--force` 只放开本机这一层。
//! 旧版写的 `<名字>.pid`（只有循环的进程号）照旧能被 stop、start 停掉。

use crate::backend::Backend;
use board_core::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

/// 循环看一次 pid 文件的间隔
const TICK: Duration = Duration::from_millis(200);
/// 新循环启动后，pid 文件还没写好的宽限时间（start 在启动进程之后才写 pid 文件）
const GRACE: Duration = Duration::from_secs(3);
/// stop 等循环退出的上限：一次请求最长 30 秒，再多给 5 秒；超时强制结束
const STOP_WAIT: Duration = Duration::from_secs(35);

/// 这些进程不算 agent：往上找时跳过
const SHELLS: [&str; 18] = [
    "sh", "bash", "zsh", "dash", "ksh", "mksh", "fish", "env", "nohup", "login", "timeout", "sudo", "cmd", "powershell", "pwsh",
    "conhost", "wsl", "board",
];

/// 两次心跳的间隔：`BOARD_HEARTBEAT_INTERVAL`（秒，只给测试用），默认 10 秒
fn interval() -> Duration {
    let secs = std::env::var("BOARD_HEARTBEAT_INTERVAL").ok().and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| *s > 0.0);
    secs.map(Duration::from_secs_f64).unwrap_or(Duration::from_millis(BEAT_MS as u64))
}

/* ------------------------------ 进程 ------------------------------ */

/// 一个进程：进程号 + 启动时间（防止进程号被复用后认错）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    /// 启动时间（秒）；0 表示不知道，不比较
    pub start: u64,
}

impl Proc {
    pub fn parse(s: &str) -> Option<Proc> {
        let mut it = s.split(|c: char| c == ':' || c.is_whitespace()).filter(|x| !x.is_empty());
        let pid = it.next()?.parse().ok().filter(|p| *p > 0)?;
        let start = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        Some(Proc { pid, start })
    }
    fn arg(&self) -> String {
        format!("{}:{}", self.pid, self.start)
    }
}

pub(crate) struct Procs(System);

impl Procs {
    pub(crate) fn new() -> Option<Procs> {
        System::new().ok().map(Procs)
    }
    fn refresh(&mut self, pids: Option<&[Pid]>) {
        let which = match pids {
            Some(p) => ProcessesToUpdate::Some(p),
            None => ProcessesToUpdate::All,
        };
        self.0.refresh_processes_specifics(which, true, ProcessRefreshKind::nothing());
    }
    /// 进程还在（进程号对得上，启动时间也对得上；僵尸进程算不在）
    pub(crate) fn alive(&mut self, p: Proc) -> bool {
        let pid = Pid::from_u32(p.pid);
        self.refresh(Some(&[pid]));
        match self.0.process(pid) {
            Some(x) => {
                !matches!(x.status(), ProcessStatus::Zombie | ProcessStatus::Dead) && (p.start == 0 || x.start_time() == p.start)
            }
            None => false,
        }
    }
    pub(crate) fn get(&mut self, pid: u32) -> Option<Proc> {
        let id = Pid::from_u32(pid);
        self.refresh(Some(&[id]));
        self.0.process(id).map(|x| Proc { pid, start: x.start_time() })
    }
    /// 是 board 自己的进程（旧版 heartbeat.sh / heartbeat.ps1 的循环、或复用了进程号的别的进程都不是）
    pub(crate) fn is_board(&mut self, p: Proc) -> bool {
        let pid = Pid::from_u32(p.pid);
        self.refresh(Some(&[pid]));
        self.0.process(pid).is_some_and(|x| base_name(&x.name().to_string_lossy()) == "board")
    }
    /// 强制结束（stop 等超时）
    fn kill(&mut self, p: Proc) {
        let pid = Pid::from_u32(p.pid);
        self.refresh(Some(&[pid]));
        if let Some(x) = self.0.process(pid) {
            if p.start == 0 || x.start_time() == p.start {
                x.kill();
            }
        }
    }
}

/// 进程名去掉路径、`.exe` 和登录 shell 的 `-` 前缀，转成小写
fn base_name(name: &str) -> String {
    let n = name.rsplit(['/', '\\']).next().unwrap_or(name).trim_start_matches('-').to_lowercase();
    n.strip_suffix(".exe").map(str::to_string).unwrap_or(n)
}

/// 后台循环要跟随的 agent 进程：`BOARD_HEARTBEAT_PID`，否则从 board 往上找第一个不是 shell 的祖先进程
pub fn agent_process() -> Option<Proc> {
    let mut ps = Procs::new()?;
    if let Some(v) = std::env::var("BOARD_HEARTBEAT_PID").ok().filter(|s| !s.trim().is_empty()) {
        let pid = Proc::parse(&v)?.pid;
        return ps.get(pid);
    }
    ps.refresh(None);
    let me = ps.0.process(Pid::from_u32(std::process::id()))?;
    let mut child_start = me.start_time();
    let mut cur = me.parent()?;
    for _ in 0..64 {
        let x = ps.0.process(cur)?;
        // Windows 不会给孤儿进程重新找父进程：父进程号可能已经被一个更晚启动的进程复用，那就不是我们的祖先
        if x.start_time() > child_start {
            return None;
        }
        let name = base_name(&x.name().to_string_lossy());
        if !SHELLS.contains(&name.as_str()) {
            return Some(Proc { pid: cur.as_u32(), start: x.start_time() });
        }
        child_start = x.start_time();
        cur = x.parent()?;
        if cur.as_u32() <= 1 {
            return None;
        }
    }
    None
}

/* ------------------------------ 本机状态文件 ------------------------------ */

/// 一个名字在本机的状态：谁 check 的、会话标识、后台循环
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub name: String,
    /// 服务端在 check 时发的会话标识；旧版服务端没有
    #[serde(default)]
    pub session: Option<String>,
    /// check 这个名字的 agent 进程（进程号:启动时间）
    #[serde(default)]
    pub agent: Option<String>,
    /// 后台循环（进程号:启动时间）
    #[serde(default)]
    pub heartbeat: Option<String>,
}

impl State {
    fn agent(&self) -> Option<Proc> {
        self.agent.as_deref().and_then(Proc::parse)
    }
    fn heartbeat(&self) -> Option<Proc> {
        self.heartbeat.as_deref().and_then(Proc::parse)
    }
}

fn dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("board-heartbeat");
    std::fs::create_dir_all(&dir).map_err(|e| BoardError::usage(format!("无法创建 {}：{e}", dir.display())))?;
    Ok(dir)
}

fn key(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect()
}

fn state_file(name: &str) -> Result<PathBuf> {
    Ok(dir()?.join(format!("{}.json", key(name))))
}

/// 旧版的 pid 文件（只有循环的进程号）
fn legacy_pidfile(name: &str) -> Result<PathBuf> {
    Ok(dir()?.join(format!("{}.pid", key(name))))
}

pub fn read_state(name: &str) -> Option<State> {
    let f = state_file(name).ok()?;
    serde_json::from_str::<State>(&std::fs::read_to_string(f).ok()?).ok().filter(|s| s.name == name)
}

fn write_state(st: &State) -> Result<()> {
    let f = state_file(&st.name)?;
    let tmp = f.with_extension(format!("json.{}", std::process::id()));
    let body = serde_json::to_string_pretty(st).unwrap_or_default();
    std::fs::write(&tmp, body).map_err(|e| BoardError::usage(format!("写不了 {}：{e}", tmp.display())))?;
    std::fs::rename(&tmp, &f).map_err(|e| BoardError::usage(format!("写不了 {}：{e}", f.display())))
}

fn remove_state(name: &str) {
    if let Ok(f) = state_file(name) {
        let _ = std::fs::remove_file(f);
    }
}

/// 等一个循环进程退出，超时强制结束。不是 board 的进程（旧版脚本、进程号被复用）不等也不结束
fn wait_loop(p: Proc) {
    let Some(mut ps) = Procs::new() else { return };
    if p.start == 0 || !ps.is_board(p) {
        return;
    }
    let begun = Instant::now();
    while ps.alive(p) {
        if begun.elapsed() > STOP_WAIT {
            ps.kill(p);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 停掉旧版 pid 文件里的循环：删掉文件就是让它停下
fn stop_legacy(name: &str) -> Result<()> {
    let f = legacy_pidfile(name)?;
    let p = std::fs::read_to_string(&f).ok().and_then(|s| Proc::parse(&s));
    let _ = std::fs::remove_file(&f);
    if let Some(p) = p {
        wait_loop(p);
    }
    Ok(())
}

/// 停掉状态里记的后台循环并等它退出（先把记录清掉，循环看到就停下）；没有循环时什么也不做
fn stop_loop(st: &mut State) -> Result<()> {
    stop_legacy(&st.name)?;
    let Some(p) = st.heartbeat() else { return Ok(()) };
    st.heartbeat = None;
    write_state(st)?;
    wait_loop(p);
    Ok(())
}

/// 名字属于本机另一个还活着的 agent 进程：返回那个进程（me 是调用方自己的 agent 进程）
fn foreign(st: &State, me: Option<Proc>) -> Option<Proc> {
    let other = st.agent()?;
    if me.is_some_and(|m| m.pid == other.pid && (m.start == 0 || other.start == 0 || m.start == other.start)) {
        return None;
    }
    Procs::new().is_some_and(|mut ps| ps.alive(other)).then_some(other)
}

fn taken_locally(name: &str, other: Proc, what: &str) -> BoardError {
    BoardError::new(
        ErrorCode::NameTaken,
        format!(
            "名字「{name}」是本机另一个还在运行的 agent 进程（进程 {}）check 的：{what}。你 check 时多半得到了 NAME_TAKEN——换一个带后缀的名字重新 check；确实是本机残留、要清理时加 --force",
            other.pid
        ),
    )
    .with_detail(serde_json::json!({ "name": name, "agent_pid": other.pid }))
}

/// 其他带 --as 的命令用：名字是本机另一个还活着的 agent 进程 check 的，返回给人看的提醒；否则为空
pub fn borrowed_warning(name: &str) -> Option<String> {
    let st = read_state(name)?;
    st.agent.as_ref()?;
    let other = foreign(&st, agent_process())?;
    Some(format!(
        "名字「{name}」是本机另一个 agent 进程（进程 {}）check 的，你可能用了别人的名字：check 得到 NAME_TAKEN 时要换带后缀的名字",
        other.pid
    ))
}

/// 不带 ID 的 `board beat` 要带的会话标识：名字属于本机另一个还活着的 agent 进程时拒绝（force 放开）
pub fn session_for(name: &str, force: bool) -> Result<Option<String>> {
    let Some(st) = read_state(name) else { return Ok(None) };
    if !force {
        if let Some(other) = foreign(&st, agent_process()) {
            return Err(taken_locally(name, other, "不能替它发心跳"));
        }
    }
    Ok(st.session)
}

/// `board check` 成功后：记下这个名字属于调用它的 agent 进程和服务端发的会话标识。
/// 本机还留着这个名字之前的后台循环（名字已经离线，它的心跳会被拒绝）就先停掉
pub fn checked(name: &str, session: Option<String>) -> Result<()> {
    let mut old = read_state(name).unwrap_or(State { name: name.to_string(), ..Default::default() });
    stop_loop(&mut old)?;
    write_state(&State { name: name.to_string(), session, agent: agent_process().map(|a| a.arg()), heartbeat: None })
}

/* ------------------------------ 命令 ------------------------------ */

pub struct Started {
    pub loop_pid: u32,
    pub agent: Option<Proc>,
    pub interval: Duration,
}

/// `board heartbeat start`：名字属于本机另一个还活着的 agent 进程时拒绝，不顶掉它的循环（force 放开）；
/// 那个进程已经不在了才接管
pub fn start(name: &str, force: bool) -> Result<Started> {
    let agent = agent_process();
    let mut st = read_state(name).unwrap_or(State { name: name.to_string(), ..Default::default() });
    if !force {
        if let Some(other) = foreign(&st, agent) {
            return Err(taken_locally(name, other, "它的后台心跳还在跑，不顶掉它"));
        }
    }
    stop_loop(&mut st)?;
    // 第一次心跳同步发：名字不在线、令牌不对、会话对不上、连不上，都直接报错
    Backend::open()?.agent_beat(name, false, st.session.as_deref())?;

    let exe = std::env::current_exe().map_err(|e| BoardError::usage(format!("找不到 board 自己的路径：{e}")))?;
    let mut cmd = Command::new(exe);
    cmd.args(["heartbeat", "_loop", &agent.map(|a| a.arg()).unwrap_or_else(|| "-".into())])
        .env("BOARD_AS", name)
        .env("BOARD_HEARTBEAT_SESSION", st.session.clone().unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = spawn_detached(&mut cmd).map_err(|e| BoardError::usage(format!("启动后台心跳失败：{e}")))?;
    let pid = child.id();
    let start = Procs::new().and_then(|mut ps| ps.get(pid)).map(|p| p.start).unwrap_or(0);
    // 接管：名字现在跟随这个 agent 进程，循环记成新的这个
    st.agent = agent.map(|a| a.arg()).or(st.agent);
    st.heartbeat = Some(Proc { pid, start }.arg());
    write_state(&st)?;
    // 不等子进程：它在后台一直跑，start 立即返回
    drop(child);
    Ok(Started { loop_pid: pid, agent, interval: interval() })
}

/// 后台循环脱离当前终端和进程组：调用它的 shell 结束、agent 的工具调用超时被结束，都不会带走它
#[cfg(unix)]
fn spawn_detached(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0).spawn()
}

#[cfg(windows)]
fn spawn_detached(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetHandleInformation(h: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    }
    // CreateProcess 会把所有可继承的句柄带给子进程。board 的标准输入输出通常是 agent 工具调用的管道（可继承），
    // 后台循环要是继承了它们，管道就一直不关，调用方等不到输出结束而卡住。start 马上就退出，这里把它们改成不可继承
    for h in [std::io::stdin().as_raw_handle(), std::io::stdout().as_raw_handle(), std::io::stderr().as_raw_handle()] {
        if !h.is_null() {
            unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0) };
        }
    }
    // 先尝试脱离 agent 工具调用所在的作业对象（作业结束时会结束里面所有进程）；作业不允许脱离时退回普通方式
    match cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB).spawn() {
        Ok(c) => Ok(c),
        Err(_) => cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).spawn(),
    }
}

/// `board heartbeat stop`：返回发 final 时名字是否还在线（false：之前已经离线，重复执行）。
/// 名字属于本机另一个还活着的 agent 进程时拒绝：不停它的循环、不发 final（force 放开）
pub fn stop(name: &str, force: bool) -> Result<Option<AgentInfo>> {
    let mut st = read_state(name).unwrap_or(State { name: name.to_string(), ..Default::default() });
    if !force {
        if let Some(other) = foreign(&st, agent_process()) {
            return Err(taken_locally(name, other, "不停它的后台心跳，也不发 final"));
        }
    }
    stop_loop(&mut st)?;
    let out = match Backend::open()?.agent_beat(name, true, st.session.as_deref()) {
        Ok(info) => Ok(Some(info)),
        Err(e) if e.code == ErrorCode::NameOffline => Ok(None),
        Err(e) => Err(e),
    };
    // 名字已经离线（或本来就离线）：本机的记录没用了
    if out.is_ok() {
        remove_state(name);
    }
    out
}

/// `board heartbeat _loop <agent>`：后台循环，只由 start 启动
pub fn run_loop(name: &str, agent: Option<Proc>) {
    let me = std::process::id();
    let begun = Instant::now();
    let every = interval();
    let Ok(mut be) = Backend::open() else { return };
    let session = std::env::var("BOARD_HEARTBEAT_SESSION").ok().filter(|s| !s.trim().is_empty());
    let mut ps = Procs::new();
    // 还是不是自己在跑：状态里记的循环被清掉（stop）或换成了别的进程（重新 start）就停下
    let mine = || match read_state(name).and_then(|st| st.heartbeat()) {
        Some(p) => p.pid == me,
        None => begun.elapsed() < GRACE,
    };
    let bye = || {
        if let Some(mut st) = read_state(name).filter(|st| st.heartbeat().is_some_and(|p| p.pid == me)) {
            st.heartbeat = None;
            let _ = write_state(&st);
        }
    };
    let mut next = Instant::now() + every;
    loop {
        std::thread::sleep(TICK);
        if !mine() {
            return;
        }
        if Instant::now() < next {
            continue;
        }
        // agent 不在了：不发 final，交给服务端判定名字离线、卡片心跳中断
        if let (Some(a), Some(ps)) = (agent, ps.as_mut()) {
            if !ps.alive(a) {
                return bye();
            }
        }
        match be.agent_beat(name, false, session.as_deref()) {
            Ok(_) => {}
            Err(e) if e.code == ErrorCode::Unreachable => {}
            Err(_) => return bye(),
        }
        next = Instant::now() + every;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proc() {
        assert_eq!(Proc::parse("123 456\n"), Some(Proc { pid: 123, start: 456 }));
        assert_eq!(Proc::parse("123:456"), Some(Proc { pid: 123, start: 456 }));
        assert_eq!(Proc::parse("123"), Some(Proc { pid: 123, start: 0 }));
        assert_eq!(Proc::parse("-"), None);
        assert_eq!(Proc::parse("0"), None);
    }

    #[test]
    fn shell_names() {
        assert_eq!(base_name("/bin/zsh"), "zsh");
        assert_eq!(base_name("-bash"), "bash");
        assert_eq!(base_name(r"C:\Windows\System32\WindowsPowerShell\v1.0\PowerShell.EXE"), "powershell");
        assert_eq!(base_name("node"), "node");
    }

    #[test]
    fn finds_a_live_ancestor() {
        // 测试进程的祖先里总有一个不是 shell 的进程（cargo），而且它活着
        let a = agent_process().expect("应该能找到 agent 进程");
        assert_ne!(a.pid, std::process::id());
        assert!(Procs::new().unwrap().alive(a));
    }
}
