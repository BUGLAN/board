//! 任务下发（board host）的规则：机器上报、手动派发的校验、开始、结束、占用、掉线、定时领取、移除（任务下发需求 §十二）。

use crate::hosts::EndHow;
use crate::*;

const T0: i64 = 1_700_000_000_000;
const S: i64 = 1000;
const B: &str = "default/main";

fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.ensure_default(T0).unwrap();
    s
}

fn claude(found: bool) -> HostAgent {
    HostAgent { id: "claude".into(), name: "Claude Code".into(), version: if found { "2.4.1".into() } else { String::new() }, found, ..Default::default() }
}

fn report(name: &str, instance: &str, running: Vec<i64>) -> HostReport {
    HostReport {
        name: name.into(),
        instance: instance.into(),
        resume: None,
        os: "macOS 15.4".into(),
        version: "0.1.1".into(),
        agents: vec![claude(true), HostAgent { id: "pi".into(), name: "pi".into(), version: String::new(), found: false, ..Default::default() }],
        envs: vec![],
        dirs: vec![HostDir { board: B.into(), path: "/tmp/board".into() }],
        running,
        done: false,
    }
}

fn beat(s: &mut Store, at: i64, rep: &HostReport) -> Result<HostWork> {
    s.write(at, Origin::cli(), |w| w.host_beat(rep, Some(1)))
}

fn exec(s: &mut Store, at: i64, who: &Actor, cmd: Cmd) -> Result<CardDetail> {
    let origin = if who.is_human() { Origin::ui(None) } else { Origin::cli() };
    s.write(at, origin, |w| {
        let b = w.resolve(B)?;
        w.exec(&b, who, cmd)
    })
}

fn new_card(s: &mut Store, at: i64, title: &str) -> i64 {
    exec(s, at, &Actor::human(), Cmd::New { title: title.into(), body: None, column: None, top: false, labels: vec![] }).unwrap().card.id
}

fn dispatch(s: &mut Store, at: i64, card: i64, host: &str, agent: Option<&str>) -> Result<Dispatch> {
    s.write(at, Origin::ui(None), |w| {
        let b = w.resolve(B)?;
        w.dispatch_create(&b, card, host, agent, &Actor::human())
    })
}

fn begin(s: &mut Store, at: i64, id: i64) -> Result<Dispatch> {
    s.write(at, Origin::cli(), |w| w.dispatch_begin("mbp", "i1", id, Some(1)))
}

fn end(s: &mut Store, at: i64, id: i64, code: Option<i64>, tail: &[&str]) -> Dispatch {
    let tail: Vec<String> = tail.iter().map(|x| x.to_string()).collect();
    s.write(at, Origin::cli(), |w| w.dispatch_end(id, code, &tail, EndHow::Exit)).unwrap()
}

fn card(s: &Store, id: i64) -> CardDetail {
    s.read(|r| {
        let b = r.resolve(B)?;
        r.card(&b, id)
    })
    .unwrap()
}

fn get(s: &Store, id: i64) -> Dispatch {
    s.read(|r| r.dispatch(id)).unwrap()
}

fn host(s: &Store, at: i64) -> HostInfo {
    s.read(|r| r.host("mbp", at)).unwrap()
}

#[test]
fn host_registers_with_defaults_and_name_is_exclusive() {
    let mut s = store();
    let w = beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let h = w.host;
    assert!(h.online);
    assert_eq!(h.max, 1);
    assert!(!h.paused);
    assert_eq!(h.agent.as_deref(), Some("claude"), "默认 agent 是第一个找到的");
    assert!(!h.cron_on && h.cron.is_empty());
    assert!(h.agents.iter().any(|a| a.id == "pi" && !a.found), "没找到的 agent 也上报");
    // 同名的第二个 host（另一个进程）：HOST_NAME_TAKEN
    let e = beat(&mut s, T0 + S, &report("mbp", "i2", vec![])).unwrap_err();
    assert_eq!(e.code, ErrorCode::HostNameTaken);
    // 同一台机器上异常退出后重启：带上一次的标识接回
    let mut r = report("mbp", "i2", vec![]);
    r.resume = Some("i1".into());
    beat(&mut s, T0 + 2 * S, &r).unwrap();
    // 离线 50 秒之后，别的进程可以用这个名字
    s.sweep(T0 + 60 * S).unwrap();
    assert!(!host(&s, T0 + 60 * S).online);
    beat(&mut s, T0 + 61 * S, &report("mbp", "i3", vec![])).unwrap();
    // 名字格式
    assert_eq!(beat(&mut s, T0, &report("MBP!", "x", vec![])).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(hosts::host_name_from("Lans-MacBook-Pro.local"), "lans-macbook-pro.local");
    assert_eq!(hosts::host_name_from("__WIN PC__"), "win-pc");
}

#[test]
fn dispatch_validation_follows_the_table() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    // 不在可领取状态
    exec(&mut s, T0, &Actor::human(), Cmd::Move { id: b, column: "review".into(), top: false, index: None, order: None }).unwrap();
    assert_eq!(dispatch(&mut s, T0, b, "mbp", None).unwrap_err().code, ErrorCode::NotInTodo);
    // 机器不存在
    assert_eq!(dispatch(&mut s, T0, a, "nope", None).unwrap_err().code, ErrorCode::NoSuchHost);
    // agent 没找到 / 没有这种 agent
    assert_eq!(dispatch(&mut s, T0, a, "mbp", Some("pi")).unwrap_err().code, ErrorCode::AgentNotOnHost);
    assert_eq!(dispatch(&mut s, T0, a, "mbp", Some("codex")).unwrap_err().code, ErrorCode::AgentNotOnHost);
    // 没有这块白板的目录
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None)).unwrap();
    let c = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve("default/other")?;
            w.exec(&b, &Actor::human(), Cmd::New { title: "C".into(), body: None, column: None, top: false, labels: vec![] })
        })
        .unwrap()
        .card
        .id;
    let e = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve("default/other")?;
            w.dispatch_create(&b, c, "mbp", None, &Actor::human())
        })
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::BoardNotOnHost);
    // 不带 agent 用默认 agent；同一张卡片不能派发两次
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    assert_eq!((d.agent.as_str(), d.state.as_str(), d.source.as_str()), ("claude", "queued", "manual"));
    assert_eq!(dispatch(&mut s, T0, a, "mbp", None).unwrap_err().code, ErrorCode::AlreadyDispatched);
}

#[test]
fn begin_claims_with_generated_name_and_end_reports_result() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!(d.state, "running");
    assert_eq!(d.name.as_deref(), Some("claude/mbp-1"));
    let c = card(&s, a);
    assert_eq!(c.card.holder.as_deref(), Some("claude/mbp-1"));
    assert_eq!(c.card.column, "doing");
    // host 上报时替名字发心跳：开始第 1 次
    beat(&mut s, T0 + 10 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    assert!(card(&s, a).card.run_started_at.is_some());
    // agent 按 skill 交审后退出
    let me = Actor::agent("claude/mbp-1");
    exec(&mut s, T0 + 20 * S, &me, Cmd::Beat { id: a, done: true }).unwrap();
    exec(&mut s, T0 + 21 * S, &me, Cmd::Move { id: a, column: "review".into(), top: false, index: None, order: None }).unwrap();
    let d = end(&mut s, T0 + 22 * S, d.id, Some(0), &[]);
    assert_eq!((d.state.as_str(), d.result.as_deref()), ("done", Some("review")));
    let acts: Vec<String> = card(&s, a).activity.iter().filter(|x| x.action == "dispatch").map(|x| x.text.clone()).collect();
    assert!(acts.iter().any(|t| t.starts_with("派发开始")), "{acts:?}");
    assert!(acts.iter().any(|t| t.starts_with("派发结束：已交审")), "{acts:?}");
    // 名字只给这次派发用，结束后离线；下一次派发序号递增
    assert!(s.read(|r| r.require_online("claude/mbp-1", T0 + 23 * S)).is_err());
    let b = new_card(&mut s, T0, "B");
    let d2 = dispatch(&mut s, T0 + 30 * S, b, "mbp", None).unwrap();
    assert_eq!(begin(&mut s, T0 + 31 * S, d2.id).unwrap().name.as_deref(), Some("claude/mbp-2"));
}

#[test]
fn crash_moves_card_to_check_with_exit_code_and_log() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    beat(&mut s, T0 + 10 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    let d = end(&mut s, T0 + 12 * S, d.id, Some(137), &["line 1", "boom"]);
    assert_eq!(d.result.as_deref(), Some("crashed"));
    let c = card(&s, a);
    assert_eq!(c.card.column, "check");
    assert!(c.card.holder.is_none());
    let comment = c.activity.iter().rev().find(|x| x.kind == "comment").unwrap();
    assert!(comment.text.contains("退出码 137") && comment.text.contains("boom"), "{}", comment.text);
}

#[test]
fn stop_kills_then_moves_to_check() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    // 人点「停止」：只记下请求，host 结束进程后上报
    let r = s.write(T0 + 2 * S, Origin::ui(None), |w| w.dispatch_cancel(d.id, &Actor::human(), "buglan")).unwrap();
    assert_eq!((r.state.as_str(), r.stop_by.as_deref()), ("running", Some("buglan")));
    let w = s.read(|r| r.host_work("mbp", T0 + 2 * S)).unwrap();
    assert_eq!(w.running[0].stop_by.as_deref(), Some("buglan"));
    let d = end(&mut s, T0 + 5 * S, d.id, Some(143), &[]);
    assert_eq!(d.result.as_deref(), Some("stopped"));
    let c = card(&s, a);
    assert_eq!(c.card.column, "check");
    assert!(c.activity.iter().any(|x| x.kind == "comment" && x.text == "buglan 停止了这次派发"));
}

#[test]
fn queued_dispatch_occupies_the_card() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    s.write(T0, Origin::cli(), |w| w.agent_check("alpha/x", None)).unwrap();
    let alpha = Actor::agent("alpha/x");
    assert_eq!(exec(&mut s, T0 + S, &alpha, Cmd::Claim { id: Some(a) }).unwrap_err().code, ErrorCode::Dispatched);
    // 不带 ID 的领取跳过它
    assert_eq!(exec(&mut s, T0 + S, &alpha, Cmd::Claim { id: None }).unwrap().card.id, b);
    // 人把它拖出可领取状态：派发自动取消
    exec(&mut s, T0 + 2 * S, &Actor::human(), Cmd::Move { id: a, column: "review".into(), top: false, index: None, order: None }).unwrap();
    assert_eq!(get(&s, d.id).result.as_deref(), Some("canceled"));
}

#[test]
fn capacity_and_pause_keep_dispatches_queued() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    let da = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let db = dispatch(&mut s, T0, b, "mbp", None).unwrap();
    begin(&mut s, T0 + S, da.id).unwrap();
    assert_eq!(begin(&mut s, T0 + S, db.id).unwrap_err().code, ErrorCode::Usage, "同时跑 1 个：没有空位");
    let w = s.read(|r| r.host_work("mbp", T0 + S)).unwrap();
    assert_eq!((w.running.len(), w.queued.len()), (1, 1));
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(2), paused: Some(true), ..Default::default() })).unwrap();
    assert_eq!(begin(&mut s, T0 + 2 * S, db.id).unwrap_err().code, ErrorCode::Usage, "暂停接单");
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { paused: Some(false), ..Default::default() })).unwrap();
    assert_eq!(begin(&mut s, T0 + 3 * S, db.id).unwrap().state, "running");
    // 设置校验
    let e = s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(9), ..Default::default() })).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    let e = s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { agent: Some("pi".into()), ..Default::default() })).unwrap_err();
    assert_eq!(e.code, ErrorCode::AgentNotOnHost);
    let h = s
        .write(T0, Origin::ui(None), |w| {
            w.host_update("mbp", &HostPatch { cron_on: Some(true), cron: Some(vec!["3:00".into(), "23:00".into(), "03:00".into()]), ..Default::default() })
        })
        .unwrap();
    assert_eq!(h.cron, vec!["03:00", "23:00"]);
}

#[test]
fn host_offline_releases_card_and_marks_lost() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    beat(&mut s, T0 + 10 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    // 之后没有上报：50 秒后卡片回到原状态最上面，派发结果「机器掉线」
    s.sweep(T0 + 61 * S).unwrap();
    assert_eq!(get(&s, d.id).result.as_deref(), Some("lost"));
    let c = card(&s, a);
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), ("todo", None));
    assert!(!host(&s, T0 + 61 * S).online);
}

#[test]
fn restarted_host_without_process_reports_crash() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    // board host 异常退出，20 秒内重启：本机没有对应进程
    let mut r = report("mbp", "i2", vec![]);
    r.resume = Some("i1".into());
    beat(&mut s, T0 + 20 * S, &r).unwrap();
    assert_eq!(get(&s, d.id).result.as_deref(), Some("crashed"));
    let c = card(&s, a);
    assert_eq!(c.card.column, "check");
    let comment = c.activity.iter().rev().find(|x| x.kind == "comment").expect("应该有结束评论");
    assert!(comment.text.contains("board host 上没有这次派发的 agent 进程"), "{}", comment.text);
    assert!(comment.text.contains("board host 重启过，或开始派发时连接中断"), "{}", comment.text);
    assert!(!comment.text.contains("重启后没有找到"), "{}", comment.text);
}

#[test]
fn cron_round_fills_slots_rotates_boards_and_ends() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None)).unwrap();
    let mut rep = report("mbp", "i1", vec![]);
    rep.dirs.push(HostDir { board: "default/other".into(), path: "/tmp/other".into() });
    beat(&mut s, T0, &rep).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    let o = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve("default/other")?;
            w.exec(&b, &Actor::human(), Cmd::New { title: "O".into(), body: None, column: None, top: false, labels: vec![] })
        })
        .unwrap()
        .card
        .id;
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(2), ..Default::default() })).unwrap();
    let pull = |s: &mut Store, at: i64| s.write(at, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap();
    // 没有开始一轮时不领
    assert!(pull(&mut s, T0 + S).is_none());
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d1 = pull(&mut s, T0 + 2 * S).unwrap();
    let d2 = pull(&mut s, T0 + 2 * S).unwrap();
    assert_eq!((d1.board.as_str(), d1.card, d1.source.as_str()), (B, a, "cron"));
    assert_eq!((d2.board.as_str(), d2.card), ("default/other", o), "一块白板领一张再换下一块");
    assert!(pull(&mut s, T0 + 2 * S).is_none(), "没有空位");
    assert!(host(&s, T0 + 2 * S).round, "空位满了，这一轮还没结束");
    // 一张结束后马上领下一张
    end(&mut s, T0 + 3 * S, d2.id, Some(0), &[]);
    let d3 = pull(&mut s, T0 + 4 * S).unwrap();
    assert_eq!(d3.card, b);
    end(&mut s, T0 + 5 * S, d1.id, Some(1), &[]);
    // 都领完了：这一轮结束
    assert!(pull(&mut s, T0 + 6 * S).is_none());
    assert!(!host(&s, T0 + 6 * S).round);
    // 刚才异常退出的 #a 在「需人工校验」，不会再被领
    assert_eq!(card(&s, a).card.column, "check");
}

#[test]
fn remove_cancels_queue_stops_running_and_comes_back_as_new() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(3), ..Default::default() })).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    let da = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let db = dispatch(&mut s, T0, b, "mbp", None).unwrap();
    let da = begin(&mut s, T0 + S, da.id).unwrap();
    s.write(T0 + 2 * S, Origin::ui(None), |w| w.host_remove("mbp", &Actor::human(), "buglan")).unwrap();
    assert_eq!(get(&s, db.id).result.as_deref(), Some("canceled"));
    assert_eq!(get(&s, da.id).stop_by.as_deref(), Some("buglan"));
    assert!(s.read(|r| r.host("mbp", T0)).is_err());
    // 那台机器的 host 再上报：作为新机器出现，设置是默认值；原来在跑的派发照样收到停止请求
    let w = beat(&mut s, T0 + 5 * S, &report("mbp", "i1", vec![da.id])).unwrap();
    assert_eq!(w.host.max, 1);
    assert_eq!(w.running.len(), 1);
    assert_eq!(end(&mut s, T0 + 6 * S, da.id, None, &[]).result.as_deref(), Some("stopped"));
    // 派发记录保留
    assert!(s.read(|r| r.host_recent(da.host_id, 50)).unwrap().iter().any(|x| x.id == da.id));
}

#[test]
fn log_keeps_last_500_lines() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let rev = s.rev().unwrap();
    let lines: Vec<String> = (0..600).map(|i| format!("line {i}")).collect();
    s.write(T0, Origin::cli(), |w| w.dispatch_log_append(d.id, &lines[..300])).unwrap();
    s.write(T0, Origin::cli(), |w| w.dispatch_log_append(d.id, &lines[300..])).unwrap();
    let log = s.read(|r| r.dispatch_log(d.id)).unwrap();
    assert_eq!(log.len(), 500);
    assert_eq!(log[0], "line 100");
    assert_eq!(s.rev().unwrap(), rev, "上传日志不算变更");
}

#[test]
fn flow_roles() {
    let s = store();
    let flow = s.read(|r| r.flow(&r.resolve(B)?)).unwrap();
    assert!(flow.is_review("review"));
    assert!(flow.is_human("check"));
    assert!(!flow.is_review("todo") && !flow.is_human("rework"));
    assert_eq!(flow.human_after("doing").map(|s| s.key.as_str()), Some("check"));
}

/* ---------------- 启动环境（启动环境与桌面端需求 §二） ---------------- */

fn env(id: &str, name: &str) -> HostEnv {
    HostEnv { id: id.into(), name: name.into(), path: String::new() }
}

fn check(env: &str, found: bool, version: &str) -> EnvCheck {
    EnvCheck { env: env.into(), found, version: version.into() }
}

/// 一台 Windows 机器：cmd、PowerShell 7、Git Bash、WSL（Ubuntu）。pi 不支持 cmd；WSL 里没装 pi
fn win_report(instance: &str) -> HostReport {
    let mut r = report("mbp", instance, vec![]);
    r.envs = vec![env("cmd", "cmd"), env("pwsh", "PowerShell 7"), env("git-bash", "Git Bash"), env("wsl:Ubuntu", "WSL（Ubuntu）")];
    r.agents = vec![
        HostAgent {
            id: "pi".into(),
            name: "pi".into(),
            version: "1.5.0".into(),
            found: true,
            envs: vec![check("git-bash", true, "1.5.0"), check("wsl:Ubuntu", false, ""), check("pwsh", true, "1.5.0")],
            env: None,
        },
        HostAgent {
            id: "claude".into(),
            name: "Claude Code".into(),
            version: "2.4.1".into(),
            found: true,
            envs: vec![check("pwsh", false, ""), check("git-bash", false, ""), check("wsl:Ubuntu", true, "2.5.0"), check("cmd", false, "")],
            env: None,
        },
    ];
    r
}

fn set_env(s: &mut Store, agent: &str, env: &str) -> Result<HostInfo> {
    s.write(T0, Origin::ui(None), |w| {
        w.host_update("mbp", &HostPatch { env: Some(EnvPatch { agent: agent.into(), env: env.into() }), ..Default::default() })
    })
}

#[test]
fn env_defaults_to_first_found_in_supported_order() {
    let mut s = store();
    let h = beat(&mut s, T0, &win_report("i1")).unwrap().host;
    assert_eq!(h.envs.len(), 4);
    let pi = h.agents.iter().find(|a| a.id == "pi").unwrap();
    assert_eq!((pi.env.as_deref(), pi.found, pi.version.as_str()), (Some("git-bash"), true, "1.5.0"));
    let claude = h.agents.iter().find(|a| a.id == "claude").unwrap();
    assert_eq!((claude.env.as_deref(), claude.found, claude.version.as_str()), (Some("wsl:Ubuntu"), true, "2.5.0"), "按顺序跳过没找到的");
    // 派发记录写启动环境，动态里写明
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", Some("pi")).unwrap();
    assert_eq!(d.env, None, "排队时还没定");
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!((d.env.as_deref(), d.env_name.as_deref()), (Some("git-bash"), Some("Git Bash")));
    let acts: Vec<String> = card(&s, a).activity.into_iter().map(|x| x.text).collect();
    assert!(acts.iter().any(|t| t.starts_with("派发开始：mbp 上的 pi（Git Bash）以 pi/mbp-1 领取")), "{acts:?}");
}

#[test]
fn env_setting_is_validated_and_used_at_start() {
    let mut s = store();
    beat(&mut s, T0, &win_report("i1")).unwrap();
    // 不支持、本机没有、没找到：USAGE
    assert_eq!(set_env(&mut s, "pi", "cmd").unwrap_err().code, ErrorCode::Usage, "pi 不支持 cmd");
    assert_eq!(set_env(&mut s, "pi", "powershell").unwrap_err().code, ErrorCode::Usage, "本机没有 Windows PowerShell");
    let e = set_env(&mut s, "pi", "wsl:Ubuntu").unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("WSL（Ubuntu）") && e.message.contains("没找到"), "{}", e.message);
    assert_eq!(set_env(&mut s, "nope", "pwsh").unwrap_err().code, ErrorCode::Usage);
    // 改成 PowerShell 7：下一次派发就用它
    let h = set_env(&mut s, "pi", "pwsh").unwrap();
    assert_eq!(h.agents.iter().find(|a| a.id == "pi").unwrap().env.as_deref(), Some("pwsh"));
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", Some("pi")).unwrap();
    assert_eq!(begin(&mut s, T0 + S, d.id).unwrap().env.as_deref(), Some("pwsh"));
    // 设置的环境本机已经没有了（卸载了 PowerShell 7）：按默认规则选
    let mut r = win_report("i1");
    r.envs.retain(|e| e.id != "pwsh");
    for a in r.agents.iter_mut() {
        a.envs.retain(|c| c.env != "pwsh");
    }
    let h = beat(&mut s, T0 + 2 * S, &r).unwrap().host;
    assert_eq!(h.agents.iter().find(|a| a.id == "pi").unwrap().env.as_deref(), Some("git-bash"));
}

#[test]
fn agent_not_found_in_set_env_is_not_on_host() {
    let mut s = store();
    beat(&mut s, T0, &win_report("i1")).unwrap();
    set_env(&mut s, "claude", "wsl:Ubuntu").unwrap();
    // 之后 WSL 里的 claude 卸载了：设置的环境里找不到，就算这台机器没找到 claude（不换到别的环境）
    let mut r = win_report("i1");
    r.agents[1].envs = vec![check("pwsh", true, "2.4.0"), check("wsl:Ubuntu", false, "")];
    let h = beat(&mut s, T0 + S, &r).unwrap().host;
    let claude = h.agents.iter().find(|a| a.id == "claude").unwrap();
    assert_eq!((claude.env.as_deref(), claude.found), (Some("wsl:Ubuntu"), false));
    let a = new_card(&mut s, T0, "A");
    let e = dispatch(&mut s, T0 + S, a, "mbp", Some("claude")).unwrap_err();
    assert_eq!(e.code, ErrorCode::AgentNotOnHost);
    assert!(e.message.contains("WSL（Ubuntu）"), "{}", e.message);
    // 默认 agent 也跟着变：pi
    assert_eq!(h.agent.as_deref(), Some("pi"));
}

#[test]
fn old_host_without_envs_starts_direct() {
    let mut s = store();
    let h = beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap().host;
    assert!(h.envs.is_empty());
    assert!(h.agents.iter().all(|a| a.env.is_none()), "旧版 board host：界面上不显示启动环境");
    assert_eq!(set_env(&mut s, "claude", "direct").unwrap_err().code, ErrorCode::Usage);
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!((d.env, d.env_name), (None, None));
}

#[test]
fn old_database_gets_env_columns() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("old.db");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE hosts (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, token INTEGER, instance TEXT NOT NULL,
               os TEXT NOT NULL DEFAULT '', version TEXT NOT NULL DEFAULT '', agents TEXT NOT NULL DEFAULT '[]', dirs TEXT NOT NULL DEFAULT '[]',
               max INTEGER NOT NULL DEFAULT 1, paused INTEGER NOT NULL DEFAULT 0, agent TEXT, cron_on INTEGER NOT NULL DEFAULT 0,
               cron TEXT NOT NULL DEFAULT '[]', round_at INTEGER, rr INTEGER NOT NULL DEFAULT 0, seq INTEGER NOT NULL DEFAULT 0,
               first_at INTEGER NOT NULL, beat_at INTEGER NOT NULL, off_at INTEGER);
             INSERT INTO hosts (name, instance, first_at, beat_at) VALUES ('mbp', 'i0', 1, 1);",
        )
        .unwrap();
    }
    let s = Store::open(&db).unwrap();
    let h = s.read(|r| r.host("mbp", 2)).unwrap();
    assert!(h.envs.is_empty());
}

#[test]
fn new_host_reads_work_from_old_server() {
    // 旧版服务端的任务里没有启动环境：新版 board host 照样能读
    let v = serde_json::json!({
        "host": { "id": 1, "name": "mbp", "os": "macOS", "version": "0.1.3", "agents": [{ "id": "claude", "name": "Claude Code", "version": "2.4.1", "found": true }],
                  "dirs": [], "max": 1, "paused": false, "agent": "claude", "cronOn": false, "cron": [], "round": false, "online": true,
                  "firstAt": 1, "beatAt": 1, "offAt": null },
        "running": [], "queued": [], "now": 1
    });
    let w: HostWork = serde_json::from_value(v).unwrap();
    assert!(w.host.envs.is_empty() && w.host.agents[0].envs.is_empty());
}
