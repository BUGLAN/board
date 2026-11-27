//! 机器对话（hosts/chat.rs，PRD §9.1「机器对话」）：开始、发消息、停止这一轮、board host 回报、不占会话位、离线、掉线。

use crate::hosts::{Chat, ChatReport, HOST_GAP_MS};
use crate::runlog::RunEvent;
use crate::*;

const T0: i64 = 1_700_000_000_000;
const S: i64 = 1000;
const B: &str = "default/main";

fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.ensure_default(T0).unwrap();
    s
}

fn report(name: &str, instance: &str, running: Vec<i64>, chats: Option<Vec<i64>>) -> HostReport {
    HostReport {
        name: name.into(),
        instance: instance.into(),
        os: "macOS 15.4".into(),
        version: "0.1.50".into(),
        agents: vec![HostAgent { id: "claude".into(), name: "Claude Code".into(), version: "2.4.1".into(), found: true, model: "opus-5.5".into(), ..Default::default() }],
        workspace: Some(HostWorkspace { path: "/tmp".into(), ..Default::default() }),
        dirs: vec![HostDir { board: B.into(), path: "/tmp/default".into(), ..Default::default() }],
        running,
        chats,
        selftest: true,
        config_sync: true,
        rescan_skills: true,
        ..Default::default()
    }
}

fn beat(s: &mut Store, at: i64, chats: Option<Vec<i64>>) -> HostWork {
    s.write(at, Origin::cli(), |w| w.host_beat(&report("mbp", "i1", vec![], chats), Some(1))).unwrap()
}

fn start(s: &mut Store, at: i64, text: &str) -> Result<Chat> {
    s.write(at, Origin::ui(None), |w| w.chat_start("mbp", None, Some("claude"), None, text, "lan"))
}

fn prompt(s: &mut Store, at: i64, id: i64, text: &str) -> Result<Chat> {
    s.write(at, Origin::ui(None), |w| w.chat_prompt(id, text))
}

fn rep(s: &mut Store, at: i64, id: i64, r: ChatReport) -> Result<Chat> {
    s.write(at, Origin::cli(), |w| w.chat_report(id, "mbp", &r))
}

fn events(s: &Store, id: i64) -> Vec<RunEvent> {
    s.read(|r| r.chat_view(id, 0)).unwrap().events
}

fn work(s: &Store, at: i64) -> HostWork {
    s.read(|r| r.host_work("mbp", at)).unwrap()
}

#[test]
fn start_sends_the_message_as_is_and_the_host_gets_it() {
    let mut s = store();
    beat(&mut s, T0, Some(vec![]));
    let c = start(&mut s, T0 + S, "  看看主目录下哪些文件夹最大  ").unwrap();
    assert_eq!((c.turn.as_str(), c.title.as_str(), c.agent.as_str(), c.by.as_str()), ("busy", "看看主目录下哪些文件夹最大", "claude", "lan"));
    // 事件里记一条人发的消息，原样（前后不加说明）
    let ev = events(&s, c.id);
    assert_eq!(ev.len(), 1);
    assert_eq!((ev[0].kind.as_str(), ev[0].text.as_deref()), ("prompt", Some("看看主目录下哪些文件夹最大")));
    // board host 随任务拿到这条消息，同样是原样
    let w = work(&s, T0 + S);
    assert_eq!(w.chats.len(), 1);
    assert_eq!(w.chats[0].prompts.len(), 1);
    assert_eq!((w.chats[0].prompts[0].seq, w.chats[0].prompts[0].text.as_str()), (1, "看看主目录下哪些文件夹最大"));
    assert!(w.chats[0].busy && w.chats[0].acp.is_none());
    // 机器的操作记录：开始对话（agent）
    let h = s.read(|r| r.host("mbp", T0 + S)).unwrap();
    assert!(h.ops.iter().any(|o| o.text == "开始对话（Claude Code）" && o.by == "lan"), "{:?}", h.ops);
    // 工作区里带着，派发里没有
    let ws = s.read(|r| r.workspace(T0 + S)).unwrap();
    assert_eq!(ws.chats.len(), 1);
    assert!(ws.dispatches.is_empty());
    // 空消息不行
    assert_eq!(start(&mut s, T0 + S, "   ").unwrap_err().code, ErrorCode::Usage);
}

#[test]
fn one_round_at_a_time_and_the_host_reports_it_done() {
    let mut s = store();
    beat(&mut s, T0, Some(vec![]));
    let c = start(&mut s, T0, "第一条").unwrap();
    // 这一轮没结束：不能发下一条
    let e = prompt(&mut s, T0 + S, c.id, "第二条").unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    // board host 收下、记下会话、回报输出和事件
    let got = rep(
        &mut s,
        T0 + 2 * S,
        c.id,
        ChatReport { took: Some(1), acp: Some("sess-1".into()), lines: vec!["> 第一条".into()], fmt: Some("acp".into()), events: vec![RunEvent::text("message", "好的")], ..Default::default() },
    )
    .unwrap();
    assert_eq!(got.turn, "busy");
    let w = work(&s, T0 + 2 * S);
    assert!(w.chats[0].prompts.is_empty(), "收下的不再发");
    assert_eq!(w.chats[0].acp.as_deref(), Some("sess-1"));
    // 这一轮结束
    let got = rep(&mut s, T0 + 3 * S, c.id, ChatReport { idle: Some("end_turn".into()), ..Default::default() }).unwrap();
    assert_eq!((got.turn.as_str(), got.last_at), ("idle", T0 + 3 * S));
    assert!(work(&s, T0 + 3 * S).chats.is_empty(), "空闲、没有消息的不发给 board host");
    // 接着发：第二条，序号 2；接回会话用记下的 sess-1
    let got = prompt(&mut s, T0 + 4 * S, c.id, "第二条").unwrap();
    assert_eq!(got.turn, "busy");
    let w = work(&s, T0 + 4 * S);
    assert_eq!((w.chats[0].prompts[0].seq, w.chats[0].acp.as_deref()), (2, Some("sess-1")));
    let v = s.read(|r| r.chat_view(c.id, 0)).unwrap();
    assert_eq!(v.log, vec!["> 第一条".to_string()]);
    let kinds: Vec<&str> = v.events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["prompt", "message", "prompt"]);
}

#[test]
fn stop_this_round_before_and_after_the_host_took_it() {
    let mut s = store();
    beat(&mut s, T0, Some(vec![]));
    // 还没交给 board host：直接撤回，这一轮结束
    let c = start(&mut s, T0, "装一下 ripgrep").unwrap();
    let got = s.write(T0 + S, Origin::ui(None), |w| w.chat_cancel(c.id)).unwrap();
    assert_eq!(got.turn, "idle");
    assert!(work(&s, T0 + S).chats.is_empty());
    let last = events(&s, c.id).pop().unwrap();
    assert!(last.kind == "result" && last.stopped);
    // 交了的：等 board host 发 session/cancel，结束后回报
    prompt(&mut s, T0 + 2 * S, c.id, "再装 fd").unwrap();
    rep(&mut s, T0 + 3 * S, c.id, ChatReport { took: Some(2), ..Default::default() }).unwrap();
    let got = s.write(T0 + 4 * S, Origin::ui(None), |w| w.chat_cancel(c.id)).unwrap();
    assert_eq!(got.turn, "busy", "等 agent 停下");
    let w = work(&s, T0 + 4 * S);
    assert_eq!(w.chats[0].cancel, 2);
    let stopped = RunEvent { kind: "result".into(), ok: Some(false), stopped: true, ..Default::default() };
    let got = rep(&mut s, T0 + 5 * S, c.id, ChatReport { canceled: Some(2), idle: Some("cancelled".into()), events: vec![stopped], ..Default::default() }).unwrap();
    assert_eq!(got.turn, "idle");
    assert!(work(&s, T0 + 5 * S).chats.is_empty(), "停止请求清掉了");
    let last = events(&s, c.id).pop().unwrap();
    assert!(last.kind == "result" && last.stopped);
    // 停下来之后可以接着发
    assert_eq!(prompt(&mut s, T0 + 6 * S, c.id, "好了吗").unwrap().turn, "busy");
}

#[test]
fn chats_do_not_take_a_session_slot() {
    let mut s = store();
    beat(&mut s, T0, Some(vec![]));
    // 最多 1 个会话、正在做一张卡；还暂停了接单
    let card = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve(B)?;
            w.exec(&b, &Actor::human(), Cmd::New { title: "README 加一个英文版".into(), body: None, column: None, top: false, labels: vec![], from: None })
        })
        .unwrap()
        .card
        .id;
    let d = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve(B)?;
            w.dispatch_create(&b, card, "mbp", Some("claude"), None, &Actor::human())
        })
        .unwrap();
    s.write(T0 + S, Origin::cli(), |w| w.dispatch_begin("mbp", "i1", d.id, Some(1))).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_update_as("mbp", &HostPatch { paused: Some(true), ..Default::default() }, "lan")).unwrap();
    // 对话照样能开
    let c = start(&mut s, T0 + 2 * S, "看看磁盘").unwrap();
    assert_eq!(c.turn, "busy");
    let w = work(&s, T0 + 2 * S);
    assert_eq!((w.running.len(), w.chats.len()), (1, 1));
    // 派发、会话位不受影响：在跑的还是那一张卡
    let ws = s.read(|r| r.workspace(T0 + 2 * S)).unwrap();
    assert_eq!(ws.dispatches.iter().map(|x| x.card).collect::<Vec<_>>(), vec![card]);
}

#[test]
fn offline_host_rejects_and_lost_rounds_end_with_an_error() {
    let mut s = store();
    beat(&mut s, T0, Some(vec![]));
    let c = start(&mut s, T0, "看看主目录").unwrap();
    rep(&mut s, T0 + S, c.id, ChatReport { took: Some(1), ..Default::default() }).unwrap();
    // board host 重启了：上报里没有这个对话的进程，这一轮记一条出错、结束，对话留着
    let restarted = |chats: Option<Vec<i64>>| HostReport { resume: Some("i1".into()), ..report("mbp", "i2", vec![], chats) };
    s.write(T0 + 2 * S, Origin::cli(), |w| w.host_beat(&restarted(Some(vec![])), Some(1))).unwrap();
    let got = s.read(|r| r.chat(c.id)).unwrap();
    assert_eq!(got.turn, "idle");
    let last = events(&s, c.id).pop().unwrap();
    assert_eq!(last.kind, "error");
    assert!(last.text.as_deref().unwrap_or_default().contains("这一轮没做完"));
    // 旧版 board host（不报 chats）：不动
    prompt(&mut s, T0 + 3 * S, c.id, "再看看").unwrap();
    rep(&mut s, T0 + 3 * S, c.id, ChatReport { took: Some(2), ..Default::default() }).unwrap();
    s.write(T0 + 4 * S, Origin::cli(), |w| w.host_beat(&restarted(None), Some(1))).unwrap();
    assert_eq!(s.read(|r| r.chat(c.id)).unwrap().turn, "busy");
    // 机器掉线：这一轮记一条出错；离线时不能开、不能发
    s.sweep(T0 + 4 * S + HOST_GAP_MS + S).unwrap();
    assert_eq!(s.read(|r| r.chat(c.id)).unwrap().turn, "idle");
    let at = T0 + 4 * S + HOST_GAP_MS + 2 * S;
    let e = start(&mut s, at, "在吗").unwrap_err();
    assert!(e.code == ErrorCode::Usage && e.message.contains("离线"), "{}", e.message);
    let e = prompt(&mut s, at, c.id, "在吗").unwrap_err();
    assert!(e.message.contains("离线"), "{}", e.message);
}

#[test]
fn only_the_owning_host_reports_and_removal_drops_chats() {
    let mut s = store();
    beat(&mut s, T0, Some(vec![]));
    s.write(T0, Origin::cli(), |w| w.host_beat(&report("win", "w1", vec![], Some(vec![])), Some(1))).unwrap();
    let c = start(&mut s, T0, "看看主目录").unwrap();
    let e = s.write(T0 + S, Origin::cli(), |w| w.chat_report(c.id, "win", &ChatReport { took: Some(1), ..Default::default() })).unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    // agent 进程退出、这一轮还没回答完：记一条出错，对话留着
    rep(&mut s, T0 + S, c.id, ChatReport { took: Some(1), ..Default::default() }).unwrap();
    let got = rep(&mut s, T0 + 2 * S, c.id, ChatReport { exit: Some(1), ..Default::default() }).unwrap();
    assert_eq!(got.turn, "idle");
    assert!(events(&s, c.id).last().unwrap().text.as_deref().unwrap_or_default().contains("退出码 1"));
    // 移除机器：对话一起删
    s.write(T0 + 3 * S, Origin::ui(None), |w| w.host_remove("mbp", &Actor::human(), "lan")).unwrap();
    assert_eq!(s.read(|r| r.chat(c.id)).unwrap_err().code, ErrorCode::NotFound);
}

/// 带工作空间和目录状态上报（#624）
fn beat_dirs(s: &mut Store, at: i64, ws: Option<&str>, status: Option<&str>) {
    let mut r = report("mbp", "i1", vec![], Some(vec![]));
    // 没配工作空间时 board host 报空路径（不报 = 不变）
    r.workspace = Some(HostWorkspace { path: ws.unwrap_or_default().into(), ..Default::default() });
    r.dirs = match (ws, status) {
        (Some(p), Some(st)) => vec![HostDir { board: B.into(), path: format!("{p}/default"), status: Some(st.into()), ..Default::default() }],
        _ => vec![],
    };
    s.write(at, Origin::cli(), |w| w.host_beat(&r, Some(1))).unwrap();
}

fn start_in(s: &mut Store, at: i64, project: Option<&str>) -> Result<Chat> {
    s.write(at, Origin::ui(None), |w| w.chat_start("mbp", None, Some("claude"), project, "看看依赖", "lan"))
}

#[test]
fn a_chat_can_run_in_a_project_dir_of_the_host() {
    let mut s = store();
    beat_dirs(&mut s, T0, Some("~/repo"), Some("not_git"));
    // 不是 git 仓库也能选；存下的是按工作空间算出的路径（~ 不展开），board host 的任务里带着
    let c = start_in(&mut s, T0 + S, Some("default")).unwrap();
    assert_eq!((c.project.as_str(), c.cwd.as_deref()), ("default", Some("~/repo/default")));
    let w = work(&s, T0 + S);
    assert_eq!(w.chats.iter().find(|t| t.id == c.id).and_then(|t| t.cwd.clone()).as_deref(), Some("~/repo/default"));
    let h = s.read(|r| r.host("mbp", T0 + S)).unwrap();
    assert!(h.ops.iter().any(|o| o.text == "开始对话（Claude Code，default）"), "{:?}", h.ops);
    // 不选：主目录，两项都空；任务里也没有
    let home = start_in(&mut s, T0 + S, None).unwrap();
    assert_eq!((home.project.as_str(), home.cwd.as_deref()), ("", None));
    assert_eq!(work(&s, T0 + S).chats.iter().find(|t| t.id == home.id).unwrap().cwd, None);
    // 之后换了工作空间：已有对话还是原来的路径
    beat_dirs(&mut s, T0 + 2 * S, Some("~/elsewhere"), Some("ok"));
    assert_eq!(s.read(|r| r.chat(c.id)).unwrap().cwd.as_deref(), Some("~/repo/default"));
}

#[test]
fn a_project_dir_must_exist_on_the_host() {
    let mut s = store();
    beat_dirs(&mut s, T0, Some("~/repo"), Some("ok"));
    assert_eq!(start_in(&mut s, T0 + S, Some("nope")).unwrap_err().code, ErrorCode::BoardNotFound);
    beat_dirs(&mut s, T0 + S, Some("~/repo"), Some("missing"));
    let e = start_in(&mut s, T0 + S, Some("default")).unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::Usage, "目录不存在：~/repo/default"));
    assert!(s.read(|r| r.chats()).unwrap().is_empty(), "都没开出对话");
    // 一直没配工作空间的机器：只能在主目录里跑
    let mut s = store();
    beat_dirs(&mut s, T0, None, None);
    let e = start_in(&mut s, T0 + S, Some("default")).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("没配工作空间"), "{}", e.message);
    assert!(start_in(&mut s, T0 + S, None).is_ok());
}
