//! `board heartbeat start | stop | final`：后台心跳在 board 里实现，Windows、macOS、Linux 行为一致（CI 在 Windows 上也跑这个文件）。
//! 心跳间隔用 BOARD_HEARTBEAT_INTERVAL 缩短；pid 文件写在测试自己的临时目录里；
//! 「agent 进程」是测试起的一个长时间运行的子进程，用 BOARD_HEARTBEAT_PID 指定。

mod common;
use common::*;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

const ME: &str = "claude/heartbeat";

/// 模拟 agent：一个跑两分钟的进程
fn fake_agent() -> Child {
    let mut c = if cfg!(windows) {
        let mut c = Command::new("ping");
        c.args(["-n", "120", "127.0.0.1"]);
        c
    } else {
        let mut c = Command::new("sleep");
        c.arg("120");
        c
    };
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap()
}

struct Env {
    vars: Vec<(&'static str, String)>,
    tmp: tempfile::TempDir,
}

impl Env {
    fn new(s: &Server, agent: &Child) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path().to_string_lossy().to_string();
        let mut vars = s.env();
        vars.extend([
            ("BOARD_AS", ME.to_string()),
            ("BOARD", "default".to_string()),
            ("BOARD_HEARTBEAT_INTERVAL", "0.5".to_string()),
            ("BOARD_HEARTBEAT_PID", agent.id().to_string()),
            ("TMPDIR", t.clone()),
            ("TMP", t.clone()),
            ("TEMP", t),
        ]);
        Env { vars, tmp }
    }
    fn agent(&mut self, agent: &Child) {
        self.vars.retain(|(k, _)| *k != "BOARD_HEARTBEAT_PID");
        self.vars.push(("BOARD_HEARTBEAT_PID", agent.id().to_string()));
    }
    fn pidfile(&self) -> PathBuf {
        self.tmp.path().join("board-heartbeat").join("claude_heartbeat.pid")
    }
    fn loop_pid(&self) -> Option<u32> {
        std::fs::read_to_string(self.pidfile()).ok()?.split_whitespace().next()?.parse().ok()
    }
}

fn alive(pid: u32) -> bool {
    let mut sys = System::new().unwrap();
    let p = Pid::from_u32(pid);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[p]), true, ProcessRefreshKind::nothing());
    sys.process(p).is_some_and(|x| !matches!(x.status(), ProcessStatus::Zombie | ProcessStatus::Dead))
}

fn wait_until(what: &str, limit: Duration, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < limit, "{what}：{limit:?} 内没有发生");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn beats(s: &Server, home: &Path, env: &[(&str, String)], id: &str) -> i64 {
    let r = board(home, env, &["show", id, "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let _ = s;
    r.json()["work"]["beats"].as_i64().unwrap_or(0)
}

#[test]
fn start_returns_at_once_and_stops_when_agent_exits() {
    let s = start();
    let mut agent = fake_agent();
    let e = Env::new(&s, &agent);
    let run = |args: &[&str]| board(&s.home, &e.vars, args);
    assert_eq!(s.check(ME).code, 0);
    assert_eq!(run(&["new", "一"]).code, 0);
    assert_eq!(run(&["claim", "1", "--json"]).code, 0);

    // start 立即返回：只发一次同步心跳，循环在后台
    let t = Instant::now();
    let r = run(&["heartbeat", "start", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(t.elapsed() < Duration::from_secs(10), "start 用了 {:?}", t.elapsed());
    let v = r.json();
    assert_eq!(v["agent_pid"].as_u64(), Some(agent.id() as u64));
    let lp = v["loop_pid"].as_u64().unwrap() as u32;
    assert_eq!(e.loop_pid(), Some(lp));
    assert!(alive(lp));

    // 后台循环持续发心跳，维持持有的卡片
    let first = beats(&s, &s.home, &e.vars, "1");
    wait_until("后台心跳", Duration::from_secs(15), || beats(&s, &s.home, &e.vars, "1") >= first + 2);

    // agent 进程不在了：循环自己停下，删掉 pid 文件，不发 final（名字还在线，别人 check 同名被拒）
    agent.kill().unwrap();
    agent.wait().unwrap();
    wait_until("agent 退出后循环停下", Duration::from_secs(15), || !alive(lp));
    assert!(!e.pidfile().exists());
    assert_eq!(s.check(ME).error_code(), "NAME_TAKEN", "没有发 final，名字要等服务端 50 秒后判定离线");
    let n = beats(&s, &s.home, &e.vars, "1");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(beats(&s, &s.home, &e.vars, "1"), n, "循环停下后不再有心跳");
}

#[test]
fn stop_stops_loop_then_goes_offline_and_is_idempotent() {
    let s = start();
    let mut agent = fake_agent();
    let mut e = Env::new(&s, &agent);
    let run = |e: &Env, args: &[&str]| board(&s.home, &e.vars, args);
    assert_eq!(s.check(ME).code, 0);

    let r = run(&e, &["heartbeat", "start"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("心跳已开始") && r.out.contains(&format!("跟随 agent 进程 {}", agent.id())), "{}", r.out);
    let lp = e.loop_pid().unwrap();

    // 重新 start：旧循环先停掉，换成新的
    let r = run(&e, &["heartbeat", "start", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let lp2 = e.loop_pid().unwrap();
    assert_ne!(lp, lp2);
    wait_until("旧循环退出", Duration::from_secs(10), || !alive(lp));

    // stop：先等循环退出，再发 final，名字立刻离线（可以马上重新 check）
    let t = Instant::now();
    let r = run(&e, &["heartbeat", "stop"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("已离线"), "{}", r.out);
    assert!(t.elapsed() < Duration::from_secs(10), "stop 用了 {:?}", t.elapsed());
    assert!(!alive(lp2), "stop 返回时循环已经退出");
    assert!(!e.pidfile().exists());
    assert_eq!(run(&e, &["beat", "--json"]).error_code(), "NAME_OFFLINE");

    // 重复 stop 不报错
    let r = run(&e, &["heartbeat", "stop", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let r = run(&e, &["heartbeat", "stop"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);

    // 名字离线时 start 直接报错，不留后台循环
    let r = run(&e, &["heartbeat", "start", "--json"]);
    assert_eq!(r.error_code(), "NAME_OFFLINE");
    assert!(e.loop_pid().is_none());

    // 换一个 agent 进程，重新 check 后再 start、stop
    agent.kill().unwrap();
    agent.wait().unwrap();
    let mut agent2 = fake_agent();
    e.agent(&agent2);
    assert_eq!(s.check(ME).code, 0);
    assert_eq!(run(&e, &["heartbeat", "start"]).code, 0);
    assert_eq!(run(&e, &["heartbeat", "stop"]).code, 0);
    agent2.kill().unwrap();
    agent2.wait().unwrap();
}

#[test]
fn final_ends_the_run_and_card_can_move() {
    let s = start();
    let mut agent = fake_agent();
    let e = Env::new(&s, &agent);
    let run = |args: &[&str]| board(&s.home, &e.vars, args);
    assert_eq!(s.check(ME).code, 0);
    assert_eq!(run(&["new", "一"]).code, 0);
    assert_eq!(run(&["claim", "1", "--json"]).code, 0);
    assert_eq!(run(&["heartbeat", "start"]).code, 0);
    wait_until("开始计时", Duration::from_secs(10), || beats(&s, &s.home, &e.vars, "1") >= 2);

    let r = run(&["heartbeat", "final", "#1"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("心跳已结束"), "{}", r.out);
    // final 之后后台心跳不再刷新这张卡片
    let n = beats(&s, &s.home, &e.vars, "1");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(beats(&s, &s.home, &e.vars, "1"), n);
    assert_eq!(run(&["move", "1", "review", "--json"]).code, 0);

    assert_eq!(run(&["heartbeat", "final", "--json"]).error_code(), "USAGE");
    assert_eq!(run(&["heartbeat", "--json"]).error_code(), "USAGE");
    assert_eq!(run(&["heartbeat", "stop"]).code, 0);
    agent.kill().unwrap();
    agent.wait().unwrap();
}
