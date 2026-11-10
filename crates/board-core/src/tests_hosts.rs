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
    assert_eq!(hosts::host_name_from("My-MacBook-Pro.local"), "my-macbook-pro.local");
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

/// 优先标签（PRD §8 规则 1）：有排队派发的优先卡片照样被跳过；定时领取一轮里优先卡片先被派发；手动派发不受影响
#[test]
fn priority_labels_in_queue_and_cron() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let p = s.write(T0, Origin::ui(None), |w| w.create_label("default", "急", None, false)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_label("default", p.id, None, None, None, Some(true))).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    let c = new_card(&mut s, T0, "C");
    // A 在最上面、不带优先标签
    for id in [b, c] {
        exec(&mut s, T0, &Actor::human(), Cmd::Label { id, add: vec![p.id], remove: vec![] }).unwrap();
    }
    // B 有排队中的派发：不带 ID 的领取跳过它，领下一张优先的 C
    let d = dispatch(&mut s, T0, b, "mbp", None).unwrap();
    assert_eq!(d.card, b, "手动派发照常派指定的卡片");
    s.write(T0, Origin::cli(), |w| w.agent_check("alpha/x", None)).unwrap();
    assert_eq!(exec(&mut s, T0 + S, &Actor::agent("alpha/x"), Cmd::Claim { id: None }).unwrap().card.id, c);

    // 定时领取：A 在最上面、B 带优先标签，先派 B
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let p = s.write(T0, Origin::ui(None), |w| w.create_label("default", "急", None, false)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_label("default", p.id, None, None, None, Some(true))).unwrap();
    new_card(&mut s, T0, "A");
    let b2 = new_card(&mut s, T0, "B");
    exec(&mut s, T0, &Actor::human(), Cmd::Label { id: b2, add: vec![p.id], remove: vec![] }).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d1 = s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().unwrap();
    assert_eq!(d1.card, b2, "带优先标签的先派发");
    let _ = a;
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
    assert_eq!(h.cron, vec!["0 3 * * *", "0 23 * * *"], "旧的 HH:MM 换成等价的 cron 表达式，重复的只留一条");
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
fn restart_grace_keeps_claims_names_and_running_dispatch() {
    // #95：服务端停了 10 分钟后重启，停机的时间不计入心跳超时
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.db");
    let back = T0 + 10 * 60 * S;
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
        // 名字 check、领取、心跳
        s.write(T0, Origin::cli(), |w| w.agent_check("claude/alpha", Some(1))).unwrap();
        let a = new_card(&mut s, T0, "A");
        exec(&mut s, T0, &Actor::agent("claude/alpha"), Cmd::Claim { id: Some(a) }).unwrap();
        exec(&mut s, T0 + S, &Actor::agent("claude/alpha"), Cmd::Beat { id: a, done: false }).unwrap();
        // 另一张卡由机器上的派发在跑
        let b = new_card(&mut s, T0, "B");
        let d = dispatch(&mut s, T0, b, "mbp", None).unwrap();
        let d = begin(&mut s, T0 + S, d.id).unwrap();
        assert_eq!(d.state, "running");
        beat(&mut s, T0 + 2 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    }
    // 停机 10 分钟后以新的时间打开服务：启动时顺延心跳
    let mut s = Store::open(&path).unwrap();
    s.resume_after_downtime(back).unwrap();
    assert!(s.read(|r| r.agents(back)).unwrap().iter().any(|a| a.name == "claude/alpha"), "名字仍在线");
    assert!(host(&s, back).online, "机器仍在线");
    assert_eq!(card(&s, 1).card.holder.as_deref(), Some("claude/alpha"), "卡片仍由原名字持有");
    assert_eq!(card(&s, 2).card.holder.as_deref(), Some("claude/mbp-1"));
    assert_eq!(get(&s, 1).state, "running", "在跑的派发仍然是 running");
    // 规则 7 不变：顺延之后再过 50 秒没有心跳，照常释放、离线
    assert_eq!(s.sweep(back + 49 * S).unwrap(), 0);
    assert!(s.sweep(back + 50 * S).unwrap() >= 1);
    assert_eq!(card(&s, 1).card.holder, None);
    assert_eq!(card(&s, 2).card.holder, None);
    assert!(s.read(|r| r.agents(back + 50 * S)).unwrap().is_empty(), "名字都离线");
    assert!(!host(&s, back + 50 * S).online);
    let d = get(&s, 1);
    assert_eq!((d.state.as_str(), d.result.as_deref()), ("done", Some("lost")));
}

#[test]
fn restart_grace_does_not_cover_already_offline_rows() {
    // 停机前已经下线的名字、机器不复活；没有心跳的领取按领取时间算
    let mut s = store();
    s.write(T0, Origin::cli(), |w| w.agent_check("claude/alpha", Some(1))).unwrap();
    s.write(T0, Origin::cli(), |w| w.agent_beat("claude/alpha", true)).unwrap();
    s.write(T0, Origin::cli(), |w| w.host_beat(&report("mbp", "i1", vec![]), Some(1))).unwrap();
    s.write(T0 + S, Origin::cli(), |w| {
        let rep = HostReport { done: true, ..report("mbp", "i1", vec![]) };
        w.host_beat(&rep, Some(1))
    })
    .unwrap();
    s.resume_after_downtime(T0 + 10 * 60 * S).unwrap();
    assert!(!s.read(|r| r.agents(T0 + 10 * 60 * S)).unwrap().iter().any(|a| a.name == "claude/alpha"));
    assert!(!host(&s, T0 + 10 * 60 * S).online);
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
fn crashed_dispatch_without_human_status_explains_and_skips_this_round() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| w.delete_status(B, "check")).unwrap();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(1), ..Default::default() })).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().expect("定时领取应该开始一张");
    assert_eq!(d.card, a);
    // 异常退出：工作流没有 check，卡片留在进行中，评论要说明会回到待办
    let d = end(&mut s, T0 + 3 * S, d.id, Some(3), &["boom"]);
    assert_eq!((d.state.as_str(), d.result.as_deref()), ("done", Some("crashed")));
    let c = card(&s, a);
    assert_eq!(c.card.column, "doing");
    let comment = c.activity.iter().rev().find(|x| x.kind == "comment").expect("应该有结束评论");
    assert!(comment.text.contains("没有需要人处理的状态"), "{}", comment.text);
    assert!(comment.text.contains("卡片 50 秒后会回到「待办」最上面"), "{}", comment.text);
    // 50 秒后卡片回到待办；这一轮还没结束时，不能再领到它
    beat(&mut s, T0 + 55 * S, &report("mbp", "i1", vec![])).unwrap();
    s.sweep(T0 + 60 * S).unwrap();
    assert_eq!(card(&s, a).card.column, "todo");
    assert!(s.write(T0 + 61 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none());
    assert!(!host(&s, T0 + 61 * S).round, "没有可领的卡，这一轮结束");
    // 下一轮（round_at 更新后）可以重新领
    s.write(T0 + 62 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d2 = s.write(T0 + 63 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().expect("下一轮应该可以再领");
    assert_eq!(d2.card, a);
}

#[test]
fn stopped_dispatch_without_human_status_skips_this_round() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| w.delete_status(B, "check")).unwrap();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(1), ..Default::default() })).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().expect("定时领取应该开始一张");
    // 人停止：工作流没有 check，评论说明会回到待办
    s.write(T0 + 3 * S, Origin::ui(None), |w| w.dispatch_cancel(d.id, &Actor::human(), "buglan")).unwrap();
    let d = end(&mut s, T0 + 4 * S, d.id, None, &[]);
    assert_eq!(d.result.as_deref(), Some("stopped"));
    let c = card(&s, a);
    let comment = c.activity.iter().rev().find(|x| x.kind == "comment").unwrap();
    assert!(comment.text.contains("buglan 停止了这次派发"), "{}", comment.text);
    assert!(comment.text.contains("没有需要人处理的状态"), "{}", comment.text);
    // 本轮的卡片释放后不会被重领
    beat(&mut s, T0 + 55 * S, &report("mbp", "i1", vec![])).unwrap();
    s.sweep(T0 + 60 * S).unwrap();
    assert!(s.write(T0 + 61 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none());
}

#[test]
fn releasing_running_dispatch_requests_stop() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!(card(&s, a).card.holder.as_deref(), Some("claude/mbp-1"));
    s.write(T0 + 2 * S, Origin::ui(None), |w| {
        let b = w.resolve(B)?;
        w.exec(&b, &Actor::human(), Cmd::Release { id: a })?;
        Ok(())
    })
    .unwrap();
    assert_eq!(get(&s, d.id).stop_by.as_deref(), Some("人"), "强制释放要同时请求停止在跑的派发");
    let d = end(&mut s, T0 + 3 * S, d.id, None, &[]);
    assert_eq!(d.result.as_deref(), Some("stopped"));
}

#[test]
fn pausing_during_round_clears_it_and_does_not_resume() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let _ = new_card(&mut s, T0, "A");
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(host(&s, T0 + S).round);
    let h = s.write(T0 + 2 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { paused: Some(true), ..Default::default() })).unwrap();
    assert!(!h.round, "暂停接单时这一轮结束");
    let h = s.write(T0 + 3 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { paused: Some(false), ..Default::default() })).unwrap();
    assert!(!h.round, "恢复接单后不自动继续");
    assert!(s.write(T0 + 4 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none());
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
    EnvCheck { env: env.into(), found, version: version.into(), no_skill: None }
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
            no_skill: None,
        },
        HostAgent {
            id: "claude".into(),
            name: "Claude Code".into(),
            version: "2.4.1".into(),
            found: true,
            envs: vec![check("pwsh", false, ""), check("git-bash", false, ""), check("wsl:Ubuntu", true, "2.5.0"), check("cmd", false, "")],
            env: None,
            no_skill: None,
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
fn missing_skill_follows_the_env_in_use() {
    let mut s = store();
    // Git Bash 里看不到 board-skills，PowerShell 7 里看得到
    let mut r = win_report("i1");
    let pi = r.agents.iter_mut().find(|a| a.id == "pi").unwrap();
    pi.no_skill = Some("~/.agents/skills/board-skills".into());
    pi.envs[0].no_skill = Some("~/.agents/skills/board-skills".into());
    let h = beat(&mut s, T0, &r).unwrap().host;
    let get = |h: &HostInfo, id: &str| h.agents.iter().find(|a| a.id == id).unwrap().no_skill.clone();
    assert_eq!(get(&h, "pi").as_deref(), Some("~/.agents/skills/board-skills"), "默认用 Git Bash：提醒");
    assert_eq!(get(&h, "claude"), None, "装了的不标");
    let h = set_env(&mut s, "pi", "pwsh").unwrap();
    assert_eq!(get(&h, "pi"), None, "换到看得到的启动环境：不提醒");
    // 旧版 board host 不上报这一项（也不上报启动环境）：不提醒
    let json = r#"{"name":"old","instance":"o1","agents":[{"id":"claude","name":"Claude Code","version":"2.4.1","found":true}]}"#;
    let old: HostReport = serde_json::from_str(json).unwrap();
    let h = beat(&mut s, T0, &old).unwrap().host;
    assert_eq!(get(&h, "claude"), None);
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

/* ---------------- 会话接着做（board next） ---------------- */

/// agent 交审：final，再移到「待审核」
fn submit(s: &mut Store, at: i64, name: &str, id: i64) {
    let a = Actor::agent(name.to_string());
    exec(s, at, &a, Cmd::Beat { id, done: true }).unwrap();
    exec(s, at, &a, Cmd::Move { id, column: "review".into(), top: false, index: None, order: None }).unwrap();
}

fn next(s: &mut Store, at: i64, name: &str) -> Result<CardDetail> {
    s.write(at, Origin::cli(), |w| w.dispatch_next(name))
}

fn pull(s: &mut Store, at: i64) -> Option<Dispatch> {
    s.write(at, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", Some(1))).unwrap()
}

fn set_host(s: &mut Store, p: HostPatch) -> Result<HostInfo> {
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &p))
}

fn label(s: &mut Store, name: &str) -> i64 {
    s.write(T0, Origin::ui(None), |w| w.create_label("default", name, None, false)).unwrap().id
}

fn tag(s: &mut Store, card: i64, l: i64) {
    exec(s, T0, &Actor::human(), Cmd::Label { id: card, add: vec![l], remove: vec![] }).unwrap();
}

fn session_of(s: &Store, session: i64) -> Vec<Dispatch> {
    s.read(|r| r.host_recent(1, 50)).unwrap().into_iter().chain(s.read(|r| r.active_dispatches()).unwrap()).filter(|d| d.session == session).collect()
}

#[test]
fn per_session_setting_is_validated() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!(host(&s, T0).per_session, 4, "默认 4");
    for bad in [0, 9] {
        assert_eq!(set_host(&mut s, HostPatch { per_session: Some(bad), ..Default::default() }).unwrap_err().code, ErrorCode::Usage);
    }
    assert_eq!(set_host(&mut s, HostPatch { per_session: Some(8), ..Default::default() }).unwrap().per_session, 8);
}

/// N = 4、6 张同标签的卡片、同时跑 1 个：第一个会话做 4 张后 SESSION_DONE，第二个会话做剩下 2 张；每张卡各一条派发
#[test]
fn session_does_up_to_n_cards_then_a_new_one_starts() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let ui = label(&mut s, "UI");
    let cards: Vec<i64> = (0..6).map(|i| new_card(&mut s, T0, &format!("卡{i}"))).collect();
    for c in &cards {
        tag(&mut s, *c, ui);
    }
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d1 = pull(&mut s, T0 + S).unwrap();
    let name = d1.name.clone().unwrap();
    assert_eq!((d1.card, d1.session), (cards[0], d1.id));
    let mut t = T0 + 2 * S;
    for want in &cards[1..4] {
        let cur = s.read(|r| r.active_dispatches()).unwrap().into_iter().find(|d| d.state == "running").unwrap();
        submit(&mut s, t, &name, cur.card);
        let c = next(&mut s, t, &name).unwrap();
        assert_eq!((c.card.id, c.card.holder.as_deref()), (*want, Some(name.as_str())), "同一个名字领下一张");
        t += S;
    }
    // 做满 4 张
    let cur = s.read(|r| r.active_dispatches()).unwrap().into_iter().find(|d| d.state == "running").unwrap();
    submit(&mut s, t, &name, cur.card);
    assert_eq!(next(&mut s, t, &name).unwrap_err().code, ErrorCode::SessionDone);
    // board host 还拿着第一张的 ID 上报：进程照样算在跑，不会被当成异常退出
    beat(&mut s, t, &report("mbp", "i1", vec![d1.id])).unwrap();
    assert_eq!(get(&s, cur.id).state, "running");
    // 进程退出：board host 报第一张的 ID，结算的是会话正在跑的那一张
    let e = end(&mut s, t + S, d1.id, Some(0), &[]);
    assert_eq!((e.id, e.result.as_deref()), (cur.id, Some("review")));
    let all = session_of(&s, d1.id);
    assert_eq!(all.len(), 4, "每张卡各一条派发");
    assert!(all.iter().all(|d| d.result.as_deref() == Some("review") && d.source == "cron"));
    assert_eq!(card(&s, cards[1]).activity.iter().filter(|a| a.text.starts_with("派发开始")).count(), 1);
    // 一个会话结束、还有空位：开新会话，做剩下的 2 张
    let d5 = pull(&mut s, t + 2 * S).unwrap();
    assert_eq!((d5.card, d5.session), (cards[4], d5.id));
    let name2 = d5.name.clone().unwrap();
    assert_ne!(name2, name, "新会话换一个名字");
    submit(&mut s, t + 3 * S, &name2, cards[4]);
    assert_eq!(next(&mut s, t + 3 * S, &name2).unwrap().card.id, cards[5]);
    submit(&mut s, t + 4 * S, &name2, cards[5]);
    assert_eq!(next(&mut s, t + 4 * S, &name2).unwrap_err().code, ErrorCode::SessionDone, "没有可领取的卡片");
}

/// 怎么挑下一张：优先标签 > 和第一张共同标签最多 > 规则 1 的顺序
#[test]
fn next_prefers_priority_then_common_labels() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let ui = label(&mut s, "UI");
    let api = label(&mut s, "接口");
    let first = new_card(&mut s, T0, "第一张");
    tag(&mut s, first, ui);
    let x = new_card(&mut s, T0, "X");
    let y = new_card(&mut s, T0, "Y");
    tag(&mut s, y, ui);
    let z = new_card(&mut s, T0, "Z");
    tag(&mut s, z, api);
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { per_session: Some(8), ..Default::default() })).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = pull(&mut s, T0 + S).unwrap();
    let name = d.name.unwrap();
    assert_eq!(d.card, first);
    // X 没有共同标签、Y 带 UI：领 Y
    submit(&mut s, T0 + 2 * S, &name, first);
    assert_eq!(next(&mut s, T0 + 2 * S, &name).unwrap().card.id, y);
    // 候选里都没有共同标签：按规则 1 的顺序领 X
    submit(&mut s, T0 + 3 * S, &name, y);
    assert_eq!(next(&mut s, T0 + 3 * S, &name).unwrap().card.id, x);
    // 出现带优先标签的卡片：领它，不管标签是否相近
    let urgent = label(&mut s, "急");
    s.write(T0, Origin::ui(None), |w| w.update_label("default", urgent, None, None, None, Some(true))).unwrap();
    let w = new_card(&mut s, T0, "W");
    tag(&mut s, w, ui);
    tag(&mut s, z, urgent);
    submit(&mut s, T0 + 4 * S, &name, x);
    assert_eq!(next(&mut s, T0 + 4 * S, &name).unwrap().card.id, z);
}

/// N = 1、手动派发、不是 board host 的会话、上一张还没交审、暂停接单、有人停止
#[test]
fn next_refuses_when_session_should_end() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    for t in ["A", "B", "C"] {
        new_card(&mut s, T0, t);
    }
    // 不是 board host 启动的会话：USAGE
    s.write(T0, Origin::cli(), |w| w.agent_check("alpha/x", None)).unwrap();
    assert_eq!(next(&mut s, T0, "alpha/x").unwrap_err().code, ErrorCode::Usage);
    // 手动派发的会话：SESSION_DONE
    let d = dispatch(&mut s, T0, 1, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    let name = d.name.clone().unwrap();
    // 上一张还由这个名字持有：ALREADY_HOLDING
    assert_eq!(next(&mut s, T0 + S, &name).unwrap_err().code, ErrorCode::AlreadyHolding);
    submit(&mut s, T0 + 2 * S, &name, 1);
    assert_eq!(next(&mut s, T0 + 2 * S, &name).unwrap_err().code, ErrorCode::SessionDone);
    end(&mut s, T0 + 3 * S, d.id, Some(0), &[]);
    // N = 1：和改动前一样，做一张就结束
    set_host(&mut s, HostPatch { per_session: Some(1), ..Default::default() }).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = pull(&mut s, T0 + 4 * S).unwrap();
    let name = d.name.clone().unwrap();
    submit(&mut s, T0 + 5 * S, &name, d.card);
    assert_eq!(next(&mut s, T0 + 5 * S, &name).unwrap_err().code, ErrorCode::SessionDone);
    end(&mut s, T0 + 6 * S, d.id, Some(0), &[]);
    // 暂停接单：在跑的会话做完当前这张，board next 返回 SESSION_DONE
    set_host(&mut s, HostPatch { per_session: Some(4), ..Default::default() }).unwrap();
    s.write(T0 + 7 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = pull(&mut s, T0 + 7 * S).unwrap();
    let name = d.name.clone().unwrap();
    set_host(&mut s, HostPatch { paused: Some(true), ..Default::default() }).unwrap();
    submit(&mut s, T0 + 8 * S, &name, d.card);
    let e = next(&mut s, T0 + 8 * S, &name).unwrap_err();
    assert_eq!(e.code, ErrorCode::SessionDone);
    assert!(e.message.contains("暂停接单"), "{}", e.message);
}

/// 会话做第 2 张时异常退出、被停止：第 1 张的结果不变，第 2 张按异常退出 / 已停止处理，不再领下一张
#[test]
fn second_card_crash_or_stop_keeps_first_result() {
    for stop in [false, true] {
        let mut s = store();
        beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
        for t in ["A", "B", "C"] {
            new_card(&mut s, T0, t);
        }
        s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
        let d1 = pull(&mut s, T0 + S).unwrap();
        let name = d1.name.clone().unwrap();
        submit(&mut s, T0 + 2 * S, &name, d1.card);
        let c2 = next(&mut s, T0 + 2 * S, &name).unwrap().card.id;
        let d2 = s.read(|r| r.active_dispatches()).unwrap().into_iter().find(|d| d.card == c2).unwrap();
        assert_eq!(get(&s, d1.id).result.as_deref(), Some("review"), "取下一张时结算上一张");
        if stop {
            s.write(T0 + 3 * S, Origin::ui(None), |w| w.dispatch_cancel(d2.id, &Actor::human(), "buglan")).unwrap();
            assert_eq!(next(&mut s, T0 + 3 * S, &name).unwrap_err().code, ErrorCode::AlreadyHolding);
        }
        let e = end(&mut s, T0 + 4 * S, d1.id, Some(1), &["boom"]);
        assert_eq!(e.id, d2.id);
        assert_eq!(e.result.as_deref(), Some(if stop { "stopped" } else { "crashed" }));
        assert_eq!(get(&s, d1.id).result.as_deref(), Some("review"), "第 1 张的结果不变");
        assert_eq!(card(&s, c2).card.column, "check");
    }
}

/// 同时跑 2 个、N = 4：两个会话各自取下一张，不会领到同一张卡，在跑的不超过 2 个
#[test]
fn two_sessions_never_take_the_same_card() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    for i in 0..6 {
        new_card(&mut s, T0, &format!("卡{i}"));
    }
    set_host(&mut s, HostPatch { max: Some(2), ..Default::default() }).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let a = pull(&mut s, T0 + S).unwrap();
    let b = pull(&mut s, T0 + S).unwrap();
    assert!(pull(&mut s, T0 + S).is_none(), "没有空位");
    let (na, nb) = (a.name.unwrap(), b.name.unwrap());
    let mut seen = vec![a.card, b.card];
    let (mut ca, mut cb) = (a.card, b.card);
    for k in 0..2 {
        let t = T0 + (2 + k) * S;
        submit(&mut s, t, &na, ca);
        ca = next(&mut s, t, &na).unwrap().card.id;
        submit(&mut s, t, &nb, cb);
        cb = next(&mut s, t, &nb).unwrap().card.id;
        seen.extend([ca, cb]);
        let running = s.read(|r| r.active_dispatches()).unwrap().into_iter().filter(|d| d.state == "running").count();
        assert_eq!(running, 2);
    }
    let mut uniq = seen.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), seen.len(), "{seen:?}");
}

/* ---------------- 领取规则：按标签筛卡片、按标签选 agent ---------------- */

/// 一台找到了 claude、dsh 的机器
fn report_cd(instance: &str) -> HostReport {
    let mut r = report("mbp", instance, vec![]);
    r.agents = vec![claude(true), HostAgent { id: "dsh".into(), name: "DeepSeek Harness".into(), version: "1.0".into(), found: true, ..Default::default() }];
    r
}

fn rule(label: &str, agent: &str) -> ClaimRule {
    cond("in", &[label], agent)
}

fn cond(op: &str, labels: &[&str], agent: &str) -> ClaimRule {
    ClaimRule { op: op.into(), labels: labels.iter().map(|l| l.to_string()).collect(), agent: agent.into(), ..Default::default() }
}

fn rules(s: &mut Store, list: &[(&str, &str)], fallback: Option<&str>) -> Result<HostInfo> {
    set_host(
        s,
        HostPatch {
            rules: Some(list.iter().map(|(l, a)| rule(l, a)).collect()),
            fallback: Some(fallback.unwrap_or("").into()),
            ..Default::default()
        },
    )
}

/// 一轮领取能派发的全部（同时跑 8 个，派发完为止）：（卡片，agent）
fn round_all(s: &mut Store, at: i64) -> Vec<(i64, String)> {
    set_host(s, HostPatch { max: Some(8), ..Default::default() }).unwrap();
    s.write(at, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let mut out = vec![];
    while let Some(d) = pull(s, at) {
        out.push((d.card, d.agent.clone()));
    }
    out
}

#[test]
fn rules_pick_cards_and_agents() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    let lc = label(&mut s, "claude");
    let ld = label(&mut s, "deepseek");
    let c = new_card(&mut s, T0, "C");
    tag(&mut s, c, lc);
    let d = new_card(&mut s, T0, "D");
    tag(&mut s, d, ld);
    let none = new_card(&mut s, T0, "无标签");
    // 规则 claude → claude：只派带 claude 的那张，另外两张留在待办，这一轮结束
    rules(&mut s, &[("claude", "claude")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(c, "claude".to_string())]);
    assert!(!host(&s, T0 + S).round, "没有匹配的卡片：这一轮结束");
    assert_eq!(card(&s, d).card.column, "todo");
    assert_eq!(card(&s, none).card.column, "todo");
}

#[test]
fn rules_route_labels_to_agents_and_fallback() {
    // 规则 claude → claude、deepseek → dsh：分别由对应的 agent 做
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    let lc = label(&mut s, "Claude");
    let ld = label(&mut s, "deepseek");
    let c = new_card(&mut s, T0, "C");
    tag(&mut s, c, lc);
    let d = new_card(&mut s, T0, "D");
    tag(&mut s, d, ld);
    let none = new_card(&mut s, T0, "无标签");
    // 标签名大小写不同也能匹配
    rules(&mut s, &[("CLAUDE", "claude"), ("deepseek", "dsh")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(c, "claude".into()), (d, "dsh".into())]);

    // 规则 claude → claude，兜底 dsh：不带标签的、带 deepseek 的由 dsh 做；先后仍按领取顺序
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    let lc = label(&mut s, "claude");
    let ld = label(&mut s, "deepseek");
    let n1 = new_card(&mut s, T0, "无标签");
    let c = new_card(&mut s, T0, "C");
    tag(&mut s, c, lc);
    let d = new_card(&mut s, T0, "D");
    tag(&mut s, d, ld);
    rules(&mut s, &[("claude", "claude")], Some("dsh")).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(n1, "dsh".into()), (c, "claude".into()), (d, "dsh".into())]);
    let _ = none;
}

#[test]
fn rules_first_match_wins_and_missing_labels_are_fine() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    let lc = label(&mut s, "claude");
    let ld = label(&mut s, "deepseek");
    let both = new_card(&mut s, T0, "两个都带");
    tag(&mut s, both, ld);
    tag(&mut s, both, lc);
    // 一张卡片同时带两条规则的标签：用排在前面的那条
    rules(&mut s, &[("claude", "claude"), ("deepseek", "dsh")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(both, "claude".into())]);
    // 这台机器有目录的项目里都没有的标签：拒绝
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    new_card(&mut s, T0, "A");
    let e = rules(&mut s, &[("没有这个标签", "claude")], None).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("没有这个标签"), "{}", e.message);
}

#[test]
fn rules_count_unmatched_cards() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    let lc = label(&mut s, "claude");
    let a = new_card(&mut s, T0, "A");
    new_card(&mut s, T0, "B");
    assert_eq!(host(&s, T0).unmatched, 0, "没有规则时不显示");
    rules(&mut s, &[("claude", "claude")], None).unwrap();
    assert_eq!(host(&s, T0).unmatched, 2);
    tag(&mut s, a, lc);
    assert_eq!(host(&s, T0).unmatched, 1);
    let ws = s.read(|r| r.hosts(T0)).unwrap();
    assert_eq!(ws[0].unmatched, 1, "工作区里的机器也带上");
    rules(&mut s, &[("claude", "claude")], Some("dsh")).unwrap();
    assert_eq!(host(&s, T0).unmatched, 0, "设了兜底后不再显示");
}

#[test]
fn rules_agent_must_exist_and_missing_agent_disables_rule() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    // 这台机器上没有 pi：拒绝
    assert_eq!(rules(&mut s, &[("claude", "pi")], None).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(rules(&mut s, &[], Some("pi")).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(rules(&mut s, &[("a", "claude"), ("A", "dsh")], None).unwrap_err().code, ErrorCode::Usage, "同一个标签写两次");
    let ld = label(&mut s, "deepseek");
    let d = new_card(&mut s, T0, "D");
    tag(&mut s, d, ld);
    let info = rules(&mut s, &[("deepseek", "dsh")], Some("claude")).unwrap();
    assert!(info.rules[0].active && info.fallback_active);
    // dsh 之后找不到了：这条规则不生效，界面上标出来；带 deepseek 的卡片也不归兜底
    beat(&mut s, T0 + S, &report("mbp", "i1", vec![])).unwrap();
    let h = host(&s, T0 + S);
    assert!(!h.rules[0].active);
    assert!(round_all(&mut s, T0 + 2 * S).is_empty());
    // 原样保存（agent 找不到了）照样可以，不会把规则弄丢
    assert!(rules(&mut s, &[("deepseek", "dsh")], Some("claude")).is_ok());
    // 手动派发不受规则影响
    assert_eq!(dispatch(&mut s, T0 + 3 * S, d, "mbp", Some("claude")).unwrap().agent, "claude");
}

/// 会话里 board next 只领按规则属于这个会话的 agent 的卡片；规则用到的标签不算共同标签
#[test]
fn next_follows_rules() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    let lc = label(&mut s, "claude");
    let ld = label(&mut s, "deepseek");
    let ui = label(&mut s, "UI");
    let first = new_card(&mut s, T0, "第一张");
    tag(&mut s, first, lc);
    tag(&mut s, first, ui);
    let other = new_card(&mut s, T0, "给 dsh");
    tag(&mut s, other, ld);
    let x = new_card(&mut s, T0, "X");
    tag(&mut s, x, lc);
    let y = new_card(&mut s, T0, "Y");
    tag(&mut s, y, lc);
    tag(&mut s, y, ui);
    rules(&mut s, &[("claude", "claude"), ("deepseek", "dsh")], None).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = pull(&mut s, T0 + S).unwrap();
    assert_eq!((d.card, d.agent.as_str()), (first, "claude"));
    let name = d.name.unwrap();
    submit(&mut s, T0 + 2 * S, &name, first);
    // X 只带 claude（规则标签不算共同标签），Y 带 claude、UI：领 Y；给 dsh 的那张不会领到
    assert_eq!(next(&mut s, T0 + 2 * S, &name).unwrap().card.id, y);
    submit(&mut s, T0 + 3 * S, &name, y);
    assert_eq!(next(&mut s, T0 + 3 * S, &name).unwrap().card.id, x);
    submit(&mut s, T0 + 4 * S, &name, x);
    assert_eq!(next(&mut s, T0 + 4 * S, &name).unwrap_err().code, ErrorCode::SessionDone, "只剩给 dsh 的卡片");
}

/* ---------------- 定时领取：cron 表达式 ---------------- */

#[test]
fn cron_setting_is_validated_and_normalized() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let set = |s: &mut Store, list: &[&str]| set_host(s, HostPatch { cron: Some(list.iter().map(|x| x.to_string()).collect()), ..Default::default() });
    let e = set(&mut s, &["61 * * * *"]).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("第 1 段（分）"), "{}", e.message);
    let e = set(&mut s, &["* * *"]).unwrap_err();
    assert!(e.message.contains("5 段"), "{}", e.message);
    // 合法的原样保存；旧的 HH:MM 换成等价的表达式；重复的只留一条
    let h = set(&mut s, &["*/5 * * * *", "0  9 * * 1-5", "03:00", "0 3 * * *"]).unwrap();
    assert_eq!(h.cron, vec!["*/5 * * * *", "0 9 * * 1-5", "0 3 * * *"]);
}

/// 升级：旧库里的 HH:MM 时间点迁移成 cron 表达式，不丢配置
#[test]
fn old_hhmm_cron_is_migrated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    }
    rusqlite::Connection::open(&path).unwrap().execute_batch(r#"UPDATE hosts SET cron = '["03:00","23:30"]', cron_on = 1"#).unwrap();
    let s = Store::open(&path).unwrap();
    let h = s.read(|r| r.host("mbp", T0)).unwrap();
    assert!(h.cron_on);
    assert_eq!(h.cron, vec!["0 3 * * *", "30 23 * * *"]);
}

/* ---------------- 领取规则的条件：in / not_in / equal ---------------- */

fn set_conds(s: &mut Store, list: Vec<ClaimRule>, fallback: Option<&str>) -> Result<HostInfo> {
    set_host(s, HostPatch { rules: Some(list), fallback: Some(fallback.unwrap_or("").into()), ..Default::default() })
}

#[test]
fn rule_conditions_in_not_in_equal() {
    let setup = || {
        let mut s = store();
        beat(&mut s, T0, &report_cd("i1")).unwrap();
        let lc = label(&mut s, "claude");
        let ui = label(&mut s, "UI");
        label(&mut s, "deepseek");
        let only_c = new_card(&mut s, T0, "只带 claude");
        tag(&mut s, only_c, lc);
        let only_ui = new_card(&mut s, T0, "只带 UI");
        tag(&mut s, only_ui, ui);
        let both = new_card(&mut s, T0, "claude + UI");
        tag(&mut s, both, lc);
        tag(&mut s, both, ui);
        let none = new_card(&mut s, T0, "无标签");
        (s, only_c, only_ui, both, none)
    };
    // in [claude, UI] → claude：带其中任一个的都由 claude 做
    let (mut s, c, u, b, _) = setup();
    set_conds(&mut s, vec![cond("in", &["claude", "ui"], "claude")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(c, "claude".into()), (u, "claude".into()), (b, "claude".into())]);
    // not in [claude] → dsh：不带 claude 的由 dsh 做，带 claude 的跳过
    let (mut s, _, u, _, n) = setup();
    set_conds(&mut s, vec![cond("not_in", &["claude"], "dsh")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(u, "dsh".into()), (n, "dsh".into())]);
    // equal [claude, UI] → claude：只有正好带这两个的匹配
    let (mut s, _, _, b, _) = setup();
    set_conds(&mut s, vec![cond("equal", &["UI", "Claude"], "claude")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(b, "claude".into())]);
    // 多条按顺序第一条命中：in [claude] → claude 在前，not in [UI] → dsh 在后；只带 claude 的两条都命中，归 claude
    let (mut s, c, _, b, n) = setup();
    set_conds(&mut s, vec![cond("in", &["claude"], "claude"), cond("not_in", &["UI"], "dsh")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(c, "claude".into()), (b, "claude".into()), (n, "dsh".into())]);
    // 没设兜底时未匹配的按新条件算：equal [claude, UI] 只命中一张，另外 3 张没匹配；设了兜底后不显示
    let (mut s, ..) = setup();
    assert_eq!(set_conds(&mut s, vec![cond("equal", &["claude", "UI"], "claude")], None).unwrap().unmatched, 3);
    assert_eq!(set_conds(&mut s, vec![cond("equal", &["claude", "UI"], "claude")], Some("dsh")).unwrap().unmatched, 0);
}

#[test]
fn rule_conditions_are_validated() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    label(&mut s, "claude");
    assert_eq!(set_conds(&mut s, vec![cond("like", &["claude"], "claude")], None).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(set_conds(&mut s, vec![cond("in", &[], "claude")], None).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(set_conds(&mut s, vec![cond("in", &["没有"], "claude")], None).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(set_conds(&mut s, vec![cond("in", &["claude"], "pi")], None).unwrap_err().code, ErrorCode::Usage);
    // 标签之后被删：规则原样保留，界面上标出缺的标签
    let h = set_conds(&mut s, vec![cond("in", &["claude"], "claude")], None).unwrap();
    assert!(h.rules[0].missing.is_empty() && h.rules[0].active);
    let id = s.read(|r| r.labels("default")).unwrap()[0].id;
    s.write(T0, Origin::ui(None), |w| w.delete_label("default", id)).unwrap();
    assert_eq!(host(&s, T0).rules[0].missing, vec!["claude"]);
    assert!(set_conds(&mut s, vec![cond("in", &["claude"], "claude")], None).is_ok(), "原样保存不拦");
}

/// 升级：旧的「单个标签 → agent」换成等价的 in [标签]
#[test]
fn old_single_label_rules_are_migrated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        beat(&mut s, T0, &report_cd("i1")).unwrap();
    }
    rusqlite::Connection::open(&path).unwrap().execute_batch(r#"UPDATE hosts SET rules = '[{"label":"claude","agent":"claude"}]'"#).unwrap();
    let mut s = Store::open(&path).unwrap();
    let h = s.read(|r| r.host("mbp", T0)).unwrap();
    assert_eq!((h.rules[0].op.as_str(), h.rules[0].labels.clone(), h.rules[0].agent.as_str()), ("in", vec!["claude".to_string()], "claude"));
    // 行为不变：带 claude（还带别的标签）的卡片照样由 claude 做
    let lc = label(&mut s, "claude");
    let ui = label(&mut s, "UI");
    let c = new_card(&mut s, T0, "C");
    tag(&mut s, c, lc);
    tag(&mut s, c, ui);
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(c, "claude".into())]);
}
