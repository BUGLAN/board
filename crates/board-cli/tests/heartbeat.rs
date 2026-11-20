//! `board heartbeat start | stop | final`：后台心跳在 board 里实现，Windows、macOS、Linux 行为一致（CI 在 Windows 上也跑这个文件）。
//! 心跳间隔用 BOARD_HEARTBEAT_INTERVAL 缩短；名字的本机状态文件写在测试自己的临时目录里；
//! 「agent 进程」是测试起的一个长时间运行的子进程，用 BOARD_HEARTBEAT_PID 指定。check 要在同一个环境里做（状态文件记着是哪个 agent 进程 check 的）。

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
    fn state(&self) -> PathBuf {
        self.tmp.path().join("board-heartbeat").join("claude_heartbeat.json")
    }
    fn loop_pid(&self) -> Option<u32> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(self.state()).ok()?).ok()?;
        v["heartbeat"].as_str()?.split(':').next()?.parse().ok()
    }
    /// 换一个名字（同一个临时目录 = 同一台机器）
    fn named(mut self, name: &str) -> Env {
        self.vars.retain(|(k, _)| *k != "BOARD_AS");
        self.vars.push(("BOARD_AS", name.to_string()));
        self
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
    assert_eq!(run(&["check", "--as", ME]).code, 0);
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

    // agent 进程不在了：循环自己停下，状态里不再记它，不发 final（名字还在线，别人 check 同名被拒）
    agent.kill().unwrap();
    agent.wait().unwrap();
    wait_until("agent 退出后循环停下", Duration::from_secs(15), || !alive(lp));
    assert!(e.loop_pid().is_none());
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
    assert_eq!(run(&e, &["check", "--as", ME]).code, 0);

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
    assert!(!e.state().exists(), "名字离线后本机的状态文件删掉");
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
    assert_eq!(run(&e, &["check", "--as", ME]).code, 0);
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
    assert_eq!(run(&["check", "--as", ME]).code, 0);
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

/// board start：一条命令占用名字、启动后台心跳、领取、返回卡片和角色；同一个会话里做下一张卡时接着用名字和心跳
#[test]
fn board_start_checks_beats_and_claims_in_one_command() {
    let s = start();
    let mut agent = fake_agent();
    let e = Env::new(&s, &agent);
    // 名字由 Board 生成：不带 BOARD_AS
    let vars: Vec<(&str, String)> = e.vars.iter().filter(|(k, _)| *k != "BOARD_AS").cloned().collect();
    let run = |args: &[&str]| board(&s.home, &vars, args);
    // 建卡用另一个名字（带一个附件）
    let pic = e.tmp.path().join("shot.png");
    std::fs::write(&pic, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0]).unwrap();
    let maker = e.vars.iter().filter(|(k, _)| *k != "BOARD_AS" && *k != "BOARD_HEARTBEAT_PID").cloned().chain([("BOARD_AS", "test/maker".to_string())]).collect::<Vec<_>>();
    assert_eq!(board(&s.home, &maker, &["check", "--as", "test/maker"]).code, 0);
    let r = board(&s.home, &maker, &["new", "一", "--image", pic.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(board(&s.home, &maker, &["new", "二"]).code, 0);

    // 第一次用要说自己是哪个 agent
    let r = run(&["start", "1", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"), "{}{}", r.out, r.err);

    let r = run(&["start", "1", "--agent", "claude", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let c = r.json();
    let name = c["session"]["name"].as_str().unwrap().to_string();
    assert!(name.starts_with("claude"), "{name}");
    assert_eq!((c["session"]["started"].as_bool(), c["session"]["resumed"].as_bool()), (Some(true), Some(false)));
    assert_eq!((c["id"].as_i64(), c["column"].as_str(), c["holder"].as_str()), (Some(1), Some("doing"), Some(name.as_str())));
    // 交审、做不下去去哪：不用自己从工作流里认
    assert_eq!((c["review"]["key"].as_str(), c["giveup"]["key"].as_str()), (Some("review"), Some("check")));
    assert!(c["repo"].is_string() && c["branch"].is_string());
    // 附件已经下载到本地
    let files = c["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert!(Path::new(files[0]["path"].as_str().unwrap()).is_file(), "{files:?}");
    // 后台心跳在跑：这张卡开始计时
    wait_until("开始计时", Duration::from_secs(10), || beats(&s, &s.home, &vars, "1") >= 2);

    // 卡片已经在自己手里时再 start：接着做，不重新占名字
    let c = run(&["start", "1", "--json"]).json();
    assert_eq!((c["session"]["name"].as_str(), c["session"]["started"].as_bool(), c["session"]["resumed"].as_bool()), (Some(name.as_str()), Some(false), Some(true)));

    // 交审后做下一张：不带 --agent，名字和心跳接着用；不带 ID 领最上面的
    let r = run(&["submit", "1", "--comment", "做完了"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let r = run(&["start"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("已领取") && r.out.contains(&name) && r.out.contains("接着用") && r.out.contains("board submit 2"), "{}", r.out);
    wait_until("第二张开始计时", Duration::from_secs(10), || beats(&s, &s.home, &vars, "2") >= 2);
    // 没有可领的了
    assert_eq!(run(&["giveup", "2", "--comment", "卡住了"]).code, 0);
    assert_eq!(run(&["start", "--json"]).error_code(), "EMPTY_TODO");

    assert_eq!(run(&["heartbeat", "stop"]).code, 0);
    agent.kill().unwrap();
    agent.wait().unwrap();
}

/// board start 先占名字、启动后台心跳，再领取：领取失败（这里是空白板，EMPTY_TODO）时名字照样在线、心跳照样在跑。
/// 报错里写着用的名字（--json 的 error.detail.session；文字输出在这次新占用时多一行提醒），不再领卡时 board heartbeat stop 收尾
#[test]
fn board_start_failure_keeps_the_session_and_says_so() {
    let s = start();
    let mut agent = fake_agent();
    let e = Env::new(&s, &agent);
    let vars: Vec<(&str, String)> = e.vars.iter().filter(|(k, _)| *k != "BOARD_AS").cloned().collect();
    let r = board(&s.home, &vars, &["start", "--agent", "claude"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    assert!(r.err.contains("EMPTY_TODO"), "{}", r.err);
    let line = r.err.lines().find(|l| l.contains("已占用")).unwrap_or_else(|| panic!("没有提醒名字已占用：{}", r.err));
    assert!(line.contains("后台心跳在跑") && line.contains("board heartbeat stop"), "{line}");

    // 再来一次：接着用同一个名字，报错里有 session，started 为 false，文字输出不再提醒
    let r = board(&s.home, &vars, &["start", "--json"]);
    assert_eq!(r.error_code(), "EMPTY_TODO", "{}{}", r.out, r.err);
    let session = r.json()["error"]["detail"]["session"].clone();
    let name = session["name"].as_str().unwrap_or_else(|| panic!("报错里没有名字：{}", r.out)).to_string();
    assert!(name.starts_with("claude") && line.contains(&name), "{name}：{line}");
    assert_eq!(session["started"], false);
    assert!(!board(&s.home, &vars, &["start"]).err.contains("已占用"));

    // 名字在线、本机的后台心跳在跑
    assert!(online(&s, &name), "{name} 应该在线");
    let state = e.tmp.path().join("board-heartbeat").join(format!("{}.json", name.replace('/', "_")));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    let pid: u32 = v["heartbeat"].as_str().and_then(|h| h.split(':').next()).and_then(|p| p.parse().ok()).unwrap_or_else(|| panic!("没有后台心跳：{v}"));
    assert!(alive(pid), "后台心跳进程 {pid} 应该在跑");

    let r = board(&s.home, &vars, &["heartbeat", "stop"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(!online(&s, &name), "stop 之后名字离线");
    agent.kill().unwrap();
    agent.wait().unwrap();
}

/// board host 启动的会话里（环境里有 BOARD_HOST）：board start 不占名字、不启动心跳，用 BOARD_AS；卡片已经领好就接着做
#[test]
fn board_start_in_a_host_session_only_reads() {
    let s = start();
    let agent = fake_agent();
    let mut e = Env::new(&s, &agent);
    let run = |e: &Env, args: &[&str]| board(&s.home, &e.vars, args);
    // board host 替它 check、领好了卡（这里用普通命令模拟）
    assert_eq!(run(&e, &["check", "--as", ME]).code, 0);
    assert_eq!(run(&e, &["new", "一"]).code, 0);
    assert_eq!(run(&e, &["claim", "1"]).code, 0);
    e.vars.push(("BOARD_HOST", "mbp".to_string()));
    let r = run(&e, &["start", "1", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let c = r.json();
    assert_eq!((c["session"]["name"].as_str(), c["session"]["started"].as_bool(), c["session"]["resumed"].as_bool()), (Some(ME), Some(false), Some(true)));
    assert_eq!(e.loop_pid(), None, "没有启动后台心跳：心跳由 board host 发");
    // 不归自己持有的卡：不领取，NOT_HOLDER；不带 ID 也不领
    assert_eq!(run(&e, &["new", "二"]).code, 0);
    let r = run(&e, &["start", "2", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_HOLDER"), "{}{}", r.out, r.err);
    assert_eq!(run(&e, &["show", "2", "--json"]).json()["column"], "todo");
    assert_eq!(run(&e, &["start", "--json"]).error_code(), "USAGE");
    let mut agent = agent;
    agent.kill().unwrap();
    agent.wait().unwrap();
}

/// 名字在服务端是否在线（另起一个干净环境 check 同名：在线就 NAME_TAKEN）
fn online(s: &Server, name: &str) -> bool {
    let r = s.raw(&s.env(), &["beat", "--as", name, "--json"]);
    // 不带会话标识的心跳：在线时被拒（NAME_TAKEN），不在线时 NAME_OFFLINE；不会改动名字的状态
    r.error_code() == "NAME_TAKEN"
}

/// 同一台机器上两个会话用了同一个名字：B 停不掉、顶不掉 A 的心跳，也不能让名字离线；B 用这个名字做别的事照常执行但有提醒。
/// 别的机器用同一个令牌也不能让它离线。A 自己 stop 照常；A 的进程不在、名字离线后，B 可以 check 并接管
#[test]
fn same_name_sessions_do_not_interfere() {
    let s = start();
    s.create_label("UI");
    let a = fake_agent();
    let b = fake_agent();
    let ea = Env::new(&s, &a);
    // 同一个临时目录（同一台机器），另一个 agent 进程
    let mut eb = Env { vars: ea.vars.clone(), tmp: tempfile::tempdir().unwrap() };
    eb.agent(&b);
    let run = |e: &Env, args: &[&str]| board(&s.home, &e.vars, args);
    assert_eq!(run(&ea, &["check", "--as", ME]).code, 0);
    assert_eq!(run(&ea, &["new", "一"]).code, 0);
    assert_eq!(run(&ea, &["claim", "1"]).code, 0);
    assert_eq!(run(&ea, &["heartbeat", "start"]).code, 0);
    let lp = ea.loop_pid().unwrap();
    // B check 同名：NAME_TAKEN（B 的脚本没看这个结果，接着用这个名字）
    assert_eq!(run(&eb, &["check", "--as", ME, "--json"]).error_code(), "NAME_TAKEN");
    // B stop：本机拒绝，不停 A 的循环、不发 final
    let r = run(&eb, &["heartbeat", "stop", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NAME_TAKEN"), "{}{}", r.out, r.err);
    assert!(r.json()["error"]["message"].as_str().unwrap().contains(&format!("进程 {}", a.id())), "{}", r.out);
    // B start：同样拒绝，不顶掉 A 的循环
    assert_eq!(run(&eb, &["heartbeat", "start", "--json"]).error_code(), "NAME_TAKEN");
    // B 直接 beat --final：本机拒绝
    assert_eq!(run(&eb, &["beat", "--final", "--json"]).error_code(), "NAME_TAKEN");
    assert_eq!(ea.loop_pid(), Some(lp));
    assert!(alive(lp));
    assert!(online(&s, ME), "名字仍在线");
    let held = run(&ea, &["show", "1", "--json"]).json();
    assert_eq!(held["holder"].as_str(), Some(ME), "A 的卡没被释放");
    // B 用 A 的名字做别的事：照常执行，但有提醒
    let r = run(&eb, &["label", "1", "+UI", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.json()["warning"].as_str().unwrap().contains("另一个 agent 进程"), "{}", r.out);
    let r = run(&eb, &["show", "1"]);
    assert!(r.err.contains("你可能用了别人的名字"), "{}", r.err);
    assert!(!run(&ea, &["show", "1"]).err.contains("别人的名字"), "A 自己用不提醒");

    // 另一台机器（没有本机状态文件）、同一个令牌：服务端拒绝（会话标识对不上），名字仍在线
    let other = tempfile::tempdir().unwrap();
    let mut ec = ea.vars.clone();
    ec.retain(|(k, _)| !matches!(*k, "TMPDIR" | "TMP" | "TEMP"));
    let t = other.path().to_string_lossy().to_string();
    ec.extend([("TMPDIR", t.clone()), ("TMP", t.clone()), ("TEMP", t)]);
    let r = board(&s.home, &ec, &["beat", "--final", "--json"]);
    assert_eq!(r.error_code(), "NAME_TAKEN", "{}{}", r.out, r.err);
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("另一个会话"), "{}", r.out);
    assert!(online(&s, ME));

    // A 自己 stop：照常停掉循环、发 final、名字离线
    let r = run(&ea, &["heartbeat", "stop"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(!alive(lp));
    assert!(!online(&s, ME));
    drop(a);
    drop(b);
}

/// A 的 agent 进程不在了、名字离线以后：B check 成功，B 的 heartbeat start 接管本机的状态文件
#[test]
fn takes_over_after_owner_is_gone() {
    let s = start();
    let mut a = fake_agent();
    let b = fake_agent();
    let ea = Env::new(&s, &a);
    let mut eb = Env { vars: ea.vars.clone(), tmp: tempfile::tempdir().unwrap() };
    eb.agent(&b);
    let run = |e: &Env, args: &[&str]| board(&s.home, &e.vars, args);
    assert_eq!(run(&ea, &["check", "--as", ME]).code, 0);
    assert_eq!(run(&ea, &["heartbeat", "start"]).code, 0);
    let lp = ea.loop_pid().unwrap();
    a.kill().unwrap();
    a.wait().unwrap();
    wait_until("A 的循环停下", Duration::from_secs(15), || !alive(lp));
    // 服务端 50 秒后才判离线：测试里直接标离线
    board_core::Store::open(&s.db).unwrap().conn().execute("UPDATE agents SET off_at = beat_at WHERE name = ?1", [ME]).unwrap();
    assert_eq!(run(&eb, &["check", "--as", ME]).code, 0);
    let r = run(&eb, &["heartbeat", "start", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["agent_pid"].as_u64(), Some(b.id() as u64));
    let lb = ea.loop_pid().unwrap();
    assert!(alive(lb));
    assert_eq!(run(&eb, &["heartbeat", "stop"]).code, 0);
    drop(b);
}

/// board host 派发的名字：派发会话里的 agent 执行 heartbeat stop，名字不离线
#[test]
fn host_owned_name_cannot_be_stopped_by_the_agent() {
    let s = start();
    let name = "claude-mbp/opus-5.5";
    {
        let mut store = board_core::Store::open(&s.db).unwrap();
        store
            .write(board_core::now_ms(), board_core::Origin::cli(), |w| w.agent_check_owned(name, None, board_core::NameOwner::Host).map(|_| ()))
            .unwrap();
    }
    let agent = fake_agent();
    let mut e = Env::new(&s, &agent).named(name);
    e.vars.push(("BOARD_HOST", "mbp".into()));
    let r = board(&s.home, &e.vars, &["heartbeat", "stop", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NAME_TAKEN"), "{}{}", r.out, r.err);
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("board host"), "{}", r.out);
    assert!(online(&s, name), "派发不受影响");
    drop(agent);
}
