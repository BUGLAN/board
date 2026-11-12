//! 任务下发（board host）的规则：机器上报、手动派发的校验、开始、结束、占用、掉线、定时领取、移除（任务下发需求 §十二）。

use crate::hosts::{EndHow, IDLE_RUN_DEFAULT, IDLE_RUN_LIMIT, MAX_RUN_DEFAULT};
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
    HostAgent { id: "claude".into(), name: "Claude Code".into(), version: if found { "2.4.1".into() } else { String::new() }, found, model: "opus-5.5".into(), ..Default::default() }
}

fn dsh(found: bool) -> HostAgent {
    HostAgent { id: "dsh".into(), name: "DeepSeek Harness".into(), version: if found { "1.0".into() } else { String::new() }, found, model: "deepseek-v4.1-flash".into(), ..Default::default() }
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
        dirs: vec![HostDir { board: B.into(), path: "/tmp/board".into(), ..Default::default() }],
        dirs_rev: 0,
        running,
        done: false,
        asked: None,
        ask_error: None,
        selftest: true,
        skills: None,
        mcp: None,
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

/// 建一张带上 `deepseek` 标签的卡片：给机器加了 `deepseek → claude` 规则后照样领得到
fn new_card_tagged(s: &mut Store, title: &str) -> i64 {
    let id = new_card(s, T0, title);
    let l = s.read(|r| r.labels("default")).unwrap_or_default().into_iter().find(|l| l.name.eq_ignore_ascii_case("deepseek"));
    let l = l.unwrap_or_else(|| s.write(T0, Origin::ui(None), |w| w.create_label("default", "deepseek", None, false)).unwrap());
    exec(s, T0, &Actor::human(), Cmd::Label { id, add: vec![l.id], remove: vec![] }).unwrap();
    id
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
    s.write(at, Origin::cli(), |w| w.dispatch_end(id, code, &tail, EndHow::Exit, None)).unwrap()
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
fn host_dirs_are_server_authoritative_and_keep_reported_checks() {
    let mut s = store();
    let w = beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!((w.host.dirs_rev, w.host.dirs[0].path.as_str(), w.host.dirs[0].status.as_deref()), (1, "/tmp/board", None));

    // 在别的设备上把目录改掉：服务端升版本，旧版本的 board host 上报不能把它改回去
    let h = s
        .write(T0 + S, Origin::ui(None), |w| {
            w.host_update_as(
                "mbp",
                &HostPatch {
                    dirs: Some(vec![HostDir { board: B.into(), path: "/tmp/new".into(), ..Default::default() }]),
                    ..Default::default()
                },
                "buglan",
            )
        })
        .unwrap();
    assert_eq!((h.dirs_rev, h.dirs[0].path.as_str(), h.dirs_changed_by.as_deref()), (2, "/tmp/new", Some("buglan")));

    let mut stale = report("mbp", "i1", vec![]);
    stale.dirs[0].status = Some("missing".into());
    stale.dirs[0].checked_at = Some(T0 + 2 * S);
    let w = beat(&mut s, T0 + 2 * S, &stale).unwrap();
    assert_eq!(w.host.dirs[0].path, "/tmp/new", "旧版本上报的是本机配置，不能覆盖服务端设置");
    assert_eq!(w.host.dirs[0].status, None, "旧路径的检查结果不挂到新路径上");

    // 应用了服务端版本后，同一路径的检查结果合进来
    let mut checked = report("mbp", "i1", vec![]);
    checked.dirs_rev = 2;
    checked.dirs[0].path = "/tmp/new".into();
    checked.dirs[0].status = Some("ok".into());
    checked.dirs[0].origin = Some("git@github.com:buglan/board.git".into());
    checked.dirs[0].branch = Some("main".into());
    checked.dirs[0].checked_at = Some(T0 + 3 * S);
    let w = beat(&mut s, T0 + 3 * S, &checked).unwrap();
    assert_eq!(w.host.dirs[0].status.as_deref(), Some("ok"));
    assert_eq!(w.host.dirs[0].origin.as_deref(), Some("git@github.com:buglan/board.git"));
    assert_eq!(w.host.dirs[0].branch.as_deref(), Some("main"));
    assert_eq!(w.host.dirs_rev, 2);

    // 本机配置先改了（版本还是 2）：服务端收下，记成「本机」改的
    let mut local = checked;
    local.dirs[0].path = "/tmp/local".into();
    let w = beat(&mut s, T0 + 4 * S, &local).unwrap();
    assert_eq!((w.host.dirs_rev, w.host.dirs[0].path.as_str(), w.host.dirs_changed_by.as_deref()), (3, "/tmp/local", Some("本机")));
}

#[test]
fn fixed_dirs_are_read_only_and_missing_dir_can_be_cloned() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "第三块", "third", "", None)).unwrap();
    let mut rep = report("mbp", "i1", vec![]);
    rep.dirs = vec![
        HostDir { board: B.into(), path: "/tmp/board".into(), fixed: true, status: Some("missing".into()), checked_at: Some(T0), ..Default::default() },
        HostDir { board: "default/other".into(), path: "/tmp/other".into(), ..Default::default() },
    ];
    beat(&mut s, T0, &rep).unwrap();

    // --dir 固定的行不能删、不能改
    let e = s
        .write(T0 + S, Origin::ui(None), |w| {
            w.host_update("mbp", &HostPatch { dirs: Some(vec![HostDir { board: "default/other".into(), path: "/tmp/other".into(), ..Default::default() }]), ..Default::default() })
        })
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("--dir"));
    let e = s
        .write(T0 + S, Origin::ui(None), |w| {
            w.host_update(
                "mbp",
                &HostPatch {
                    dirs: Some(vec![
                        HostDir { board: B.into(), path: "/tmp/changed".into(), ..Default::default() },
                        HostDir { board: "default/other".into(), path: "/tmp/other".into(), ..Default::default() },
                    ]),
                    ..Default::default()
                },
            )
        })
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);

    // 其他行照常增删改，固定行保留 fixed
    let h = s
        .write(T0 + S, Origin::ui(None), |w| {
            w.host_update(
                "mbp",
                &HostPatch {
                    dirs: Some(vec![
                        HostDir { board: B.into(), path: "/tmp/board".into(), ..Default::default() },
                        HostDir { board: "default/other".into(), path: "/tmp/other-new".into(), ..Default::default() },
                        HostDir { board: "default/third".into(), path: "/tmp/third".into(), ..Default::default() },
                    ]),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    assert_eq!(h.dirs.iter().map(|d| (d.board.as_str(), d.path.as_str(), d.fixed)).collect::<Vec<_>>(), vec![
        (B, "/tmp/board", true),
        ("default/other", "/tmp/other-new", false),
        ("default/third", "/tmp/third", false),
    ]);
    let h = s
        .write(T0 + 2 * S, Origin::ui(None), |w| {
            w.host_update(
                "mbp",
                &HostPatch {
                    dirs: Some(vec![HostDir { board: B.into(), path: "/tmp/board".into(), ..Default::default() }]),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    assert_eq!(h.dirs.iter().map(|d| (d.board.as_str(), d.fixed)).collect::<Vec<_>>(), vec![(B, true)]);

    // 目录不存在：请求克隆，状态排队；board host 上报失败原因
    let mut s2 = store();
    let mut missing = report("mbp", "i1", vec![]);
    missing.dirs[0].status = Some("missing".into());
    missing.dirs[0].checked_at = Some(T0);
    beat(&mut s2, T0, &missing).unwrap();
    let h = s2
        .write(T0 + S, Origin::ui(None), |w| w.host_clone_request("mbp", B, "git@github.com:buglan/board.git", "buglan"))
        .unwrap();
    let clone = h.dirs[0].clone.as_ref().unwrap();
    assert_eq!((clone.state.as_str(), clone.url.as_str(), clone.by.as_str()), ("queued", "git@github.com:buglan/board.git", "buglan"));

    let mut failed = missing;
    failed.dirs[0].clone = Some(HostClone {
        state: "failed".into(),
        url: "git@github.com:buglan/board.git".into(),
        message: "Permission denied (publickey)".into(),
        updated_at: T0 + 2 * S,
        by: "本机".into(),
    });
    let w = beat(&mut s2, T0 + 2 * S, &failed).unwrap();
    let clone = w.host.dirs[0].clone.as_ref().unwrap();
    assert_eq!(clone.state, "failed");
    assert!(clone.message.contains("Permission denied"));
}

#[test]
fn dispatch_validation_follows_the_table() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    // 不在可领取状态
    exec(&mut s, T0, &Actor::human(), Cmd::Move { id: b, column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
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
    // 名字：<agent 标识>-<机器名>/<模型名>，模型名来自 board host 上报的 agent 数据
    assert_eq!(d.name.as_deref(), Some("claude-mbp/opus-5.5"));
    check_agent_name(d.name.as_deref().unwrap()).unwrap();
    let c = card(&s, a);
    assert_eq!(c.card.holder.as_deref(), Some("claude-mbp/opus-5.5"));
    assert_eq!(c.card.column, "doing");
    // host 上报时替名字发心跳：开始第 1 次
    beat(&mut s, T0 + 10 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    assert!(card(&s, a).card.run_started_at.is_some());
    // agent 按 skill 交审后退出
    let me = Actor::agent("claude-mbp/opus-5.5");
    exec(&mut s, T0 + 20 * S, &me, Cmd::Beat { id: a, done: true }).unwrap();
    exec(&mut s, T0 + 21 * S, &me, Cmd::Move { id: a, column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    let d = end(&mut s, T0 + 22 * S, d.id, Some(0), &[]);
    assert_eq!((d.state.as_str(), d.result.as_deref()), ("done", Some("review")));
    let acts: Vec<String> = card(&s, a).activity.iter().filter(|x| x.action == "dispatch").map(|x| x.text.clone()).collect();
    assert!(acts.iter().any(|t| t.starts_with("派发开始")), "{acts:?}");
    assert!(acts.iter().any(|t| t.starts_with("派发结束：已交审")), "{acts:?}");
    // 名字只给这次派发用，结束后离线；下一次派发复用这个名字（序号只在并发时加）
    assert!(s.read(|r| r.require_online("claude-mbp/opus-5.5", T0 + 23 * S)).is_err());
    let b = new_card(&mut s, T0, "B");
    let d2 = dispatch(&mut s, T0 + 30 * S, b, "mbp", None).unwrap();
    assert_eq!(begin(&mut s, T0 + 31 * S, d2.id).unwrap().name.as_deref(), Some("claude-mbp/opus-5.5"));
}

/// 派发的 agent 名字：`<agent 标识>-<机器名>/<模型名>`（卡片 #155 的例子是 `dsh-win/deepseek-v4.1-flash`）
#[test]
fn dispatch_agent_name_format() {
    // 基本格式、并发序号加在模型名后面
    assert_eq!(hosts::dispatch_agent_name("dsh", "win", "deepseek-v4.1-flash", 0), "dsh-win/deepseek-v4.1-flash");
    assert_eq!(hosts::dispatch_agent_name("dsh", "win", "deepseek-v4.1-flash", 2), "dsh-win/deepseek-v4.1-flash-2");
    assert_eq!(hosts::dispatch_agent_name("dsh", "win", "deepseek-v4.1-flash", 3), "dsh-win/deepseek-v4.1-flash-3");
    assert_eq!(hosts::dispatch_agent_name("pi", "mbp", "", 2), "pi-mbp/pi-2");
    // 模型名转成小写、空白和非法字符换成 -；没有模型时（旧版 board host、没配置）用 agent 标识
    assert_eq!(hosts::dispatch_agent_name("claude", "mbp", "Opus 5.5", 0), "claude-mbp/opus-5.5");
    assert_eq!(hosts::dispatch_agent_name("dsh", "win", "  DeepSeek V4.1 Flash ", 0), "dsh-win/deepseek-v4.1-flash");
    assert_eq!(hosts::dispatch_agent_name("pi", "mbp", "", 0), "pi-mbp/pi");
    assert_eq!(hosts::dispatch_agent_name("codex", "mbp", "---", 0), "codex-mbp/codex");
    // 名字都能通过 board check 的格式校验；长机器名、长标识、多位的序号也一样（序号只在并发时加，
    // 最多「同时跑几个」（≤8）个，实际是个位数；模型名放不下时截断）
    let cases = [
        ("dsh", "win", "deepseek-v4.1-flash", 0u32),
        ("dsh", "win", "deepseek-v4.1-flash", 12),
        ("my-agent-16chars", "host-abc1234567890123456", "a-model", 9),
        ("dsh", "win", "gpt-5.1-codex-max-preview-very-long-model-name", 8),
    ];
    for (a, h, m, seq) in cases {
        let n = hosts::dispatch_agent_name(a, h, m, seq);
        check_agent_name(&n).unwrap_or_else(|e| panic!("{n}：{e}"));
        assert!(n.chars().count() <= 48, "{n}");
    }
    // 模型名被截断到放得下为止，序号保留在最后
    assert_eq!(hosts::dispatch_agent_name("my-agent-16chars", "host-abc1234567890123456", "a-model", 9), "my-agent-16chars-host-abc1234567890123456/a-mo-9");
    assert_eq!(
        hosts::dispatch_agent_name("dsh", "win", "gpt-5.1-codex-max-preview-very-long-model-name", 8),
        "dsh-win/gpt-5.1-codex-max-preview-very-long-mo-8"
    );
}

/// 同一台机器同一 agent、模型有多个并发派发时名字仍唯一（加序号）；上一个会话离线后名字复用；
/// `board next` 接着做的会话用同一个名字；agent 自己用 skills 的名字不受影响
#[test]
fn dispatch_names_unique_while_concurrent_and_reused_after() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    set_host(&mut s, HostPatch { max: Some(2), ..Default::default() }).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    // 两个并发派发：第一个不带序号，第二个在模型名后面加 -2
    let da = dispatch(&mut s, T0, a, "mbp", Some("dsh")).unwrap();
    let db = dispatch(&mut s, T0, b, "mbp", Some("dsh")).unwrap();
    let da = begin(&mut s, T0 + S, da.id).unwrap();
    assert_eq!(da.name.as_deref(), Some("dsh-mbp/deepseek-v4.1-flash"));
    check_agent_name(da.name.as_deref().unwrap()).unwrap();
    let db = begin(&mut s, T0 + S, db.id).unwrap();
    assert_eq!(db.name.as_deref(), Some("dsh-mbp/deepseek-v4.1-flash-2"), "并发派发在模型名后面加序号");
    check_agent_name(db.name.as_deref().unwrap()).unwrap();
    // 第一个结束（名字离线）之后：定时领取的会话复用不带序号的名字
    end(&mut s, T0 + 2 * S, da.id, Some(0), &[]);
    let ld = label(&mut s, "deepseek");
    let d = new_card(&mut s, T0, "D");
    let e = new_card(&mut s, T0, "E");
    tag(&mut s, d, ld);
    tag(&mut s, e, ld);
    rules(&mut s, &[("deepseek", "dsh")], None).unwrap();
    s.write(T0 + 3 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let dp = pull(&mut s, T0 + 4 * S).unwrap();
    assert_eq!(dp.agent, "dsh");
    assert_eq!(dp.name.as_deref(), Some("dsh-mbp/deepseek-v4.1-flash"), "离线后的名字可以复用");
    // board next 接着做的会话用同一个名字
    let name = dp.name.clone().unwrap();
    submit(&mut s, T0 + 5 * S, &name, dp.card);
    let nx = next(&mut s, T0 + 5 * S, &name).unwrap();
    assert_eq!(nx.card.holder.as_deref(), Some(name.as_str()), "同一个名字领下一张");
    // agent 自己用 skills 生成的名字（不带机器名）照常 check，和派发的名字互不影响
    s.write(T0, Origin::cli(), |w| w.agent_check("dsh/deepseek-v4.1-flash", None)).unwrap();
    let d = s.read(|r| r.agent("dsh/deepseek-v4.1-flash")).unwrap().unwrap();
    assert_eq!(d.cards.len(), 0);
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
    exec(&mut s, T0 + 2 * S, &Actor::human(), Cmd::Move { id: a, column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
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
    lead_round(&mut s);
    let l = s.read(|r| r.labels("default")).unwrap().into_iter().find(|l| l.name == "deepseek").unwrap().id;
    new_card(&mut s, T0, "A");
    let b2 = new_card(&mut s, T0, "B");
    exec(&mut s, T0, &Actor::human(), Cmd::Label { id: b2, add: vec![l], remove: vec![] }).unwrap();
    let p = s.write(T0, Origin::ui(None), |w| w.create_label("default", "急", None, false)).unwrap();
    exec(&mut s, T0, &Actor::human(), Cmd::Label { id: b2, add: vec![p.id], remove: vec![] }).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_label("default", p.id, None, None, None, Some(true))).unwrap();
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
    assert_eq!(card(&s, 2).card.holder.as_deref(), Some("claude-mbp/opus-5.5"));
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
    rep.dirs.push(HostDir { board: "default/other".into(), path: "/tmp/other".into(), ..Default::default() });
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
    // 规则「全部」→ claude：两块白板的卡片都不带它，靠「其余卡片 →」兜底给 claude，都能领
    s.write(T0, Origin::ui(None), |w| w.create_label("default", "全部", None, false)).unwrap();
    // 一个会话只做一张（N = 1）：这里测的是占空位、多块白板轮流；会话排几张见 session_* 的用例
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { rules: Some(vec![cond("in", &["全部"], "claude")]), fallback: Some("claude".into()), max: Some(2), per_session: Some(1), ..Default::default() })).unwrap();
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
    lead_round(&mut s);
    let a = new_card_tagged(&mut s, "A");
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
    assert!(comment.text.contains("没有能接住异常退出的状态"), "{}", comment.text);
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
    lead_round(&mut s);
    let a = new_card_tagged(&mut s, "A");
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
    assert!(comment.text.contains("没有能接住异常退出的状态"), "{}", comment.text);
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
    assert_eq!(card(&s, a).card.holder.as_deref(), Some("claude-mbp/opus-5.5"));
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
    lead_round(&mut s);
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
    // 默认工作流没有「错误」节点
    assert!(!flow.is_error("check") && !flow.is_error("doing"));
    assert!(flow.error_after("doing").is_none());
}

/// 把状态设成错误节点（状态设置里的「错误节点」开关）
fn set_error(s: &mut Store, key: &str, on: bool) {
    s.write(T0, Origin::ui(None), |w| w.update_status(B, key, StatusPatch { err: Some(on), ..Default::default() })).unwrap();
}

/// 在 B 上新建一个状态并打开「错误节点」，从 doing 连过去。名字故意不带「错误」：错误节点看属性，不看名称
fn add_error_status(s: &mut Store) -> String {
    let st = s.write(T0, Origin::ui(None), |w| w.create_status(B, "搁置", None)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_edges(B, &[(String::from("doing"), st.key.clone())], &[])).unwrap();
    set_error(s, &st.key, true);
    st.key
}

#[test]
fn error_status_is_its_own_role() {
    let mut s = store();
    let key = add_error_status(&mut s);
    let flow = s.read(|r| r.flow(&r.resolve(B)?)).unwrap();
    assert!(flow.is_error(&key), "打开了「错误节点」的状态是错误节点");
    assert_eq!(flow.error_after("doing").map(|s| s.key.as_str()), Some(key.as_str()));
    // 单独一类，不混进交审、需要人处理
    assert!(!flow.is_review(&key) && !flow.is_human(&key));
    assert_eq!(flow.human_after("doing").map(|s| s.key.as_str()), Some("check"), "错误节点不算需要人处理的状态");
    // 不靠名称：叫「错误」但没打开开关的状态不是错误节点；打开了开关、名字里带「人工」的也不算需要人处理
    let named = s.write(T0, Origin::ui(None), |w| w.create_status(B, "错误", None)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_edges(B, &[(String::from("doing"), named.key.clone())], &[])).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_status(B, &key, StatusPatch { name: Some("人工处理".into()), ..Default::default() })).unwrap();
    let flow = s.read(|r| r.flow(&r.resolve(B)?)).unwrap();
    assert!(!flow.is_error(&named.key), "只看名称不算错误节点");
    assert_eq!(flow.error_after("doing").map(|s| s.key.as_str()), Some(key.as_str()));
    assert!(!flow.is_human(&key), "错误节点不当成需要人处理的状态");
    // 关掉开关就不是了
    set_error(&mut s, &key, false);
    let flow = s.read(|r| r.flow(&r.resolve(B)?)).unwrap();
    assert!(!flow.is_error(&key) && flow.error_after("doing").is_none());
}

/// 「错误节点」和可领取、锁定、终点互斥：打开一边关掉另一边；最后一个可领取状态不能改成错误节点
#[test]
fn error_flag_excludes_other_rules() {
    let mut s = store();
    let flow = |s: &Store| s.read(|r| r.flow(&r.resolve(B)?)).unwrap();
    // 打开错误节点：终点、锁定、可领取被关掉
    set_error(&mut s, "done", true);
    let st = flow(&s).get("done").cloned().unwrap();
    assert!(st.err && !st.fin);
    set_error(&mut s, "rework", true);
    let st = flow(&s).get("rework").cloned().unwrap();
    assert!(st.err && !st.claim, "可领取被关掉，下一轮定时领取不会从这里领");
    // 打开可领取：错误节点被关掉
    s.write(T0, Origin::ui(None), |w| w.update_status(B, "rework", StatusPatch { claim: Some(true), ..Default::default() })).unwrap();
    let st = flow(&s).get("rework").cloned().unwrap();
    assert!(st.claim && !st.err);
    // 同时打开两边：拒绝
    let both = s.write(T0, Origin::ui(None), |w| {
        w.update_status(B, "review", StatusPatch { err: Some(true), lock: Some(true), ..Default::default() })
    });
    assert!(both.is_err());
    // 最后一个可领取状态：不能改成错误节点（关掉可领取那一步被拦下）
    s.write(T0, Origin::ui(None), |w| w.update_status(B, "rework", StatusPatch { claim: Some(false), ..Default::default() })).unwrap();
    let last = s.write(T0, Origin::ui(None), |w| w.update_status(B, "todo", StatusPatch { err: Some(true), ..Default::default() }));
    assert!(last.is_err(), "最后一个可领取状态不能变成错误节点");
    assert!(!flow(&s).is_error("todo"));
}

/// agent 沿连线移入错误节点；只有人能移出
#[test]
fn only_humans_move_cards_out_of_error_status() {
    let mut s = store();
    let key = add_error_status(&mut s);
    s.write(T0, Origin::ui(None), |w| w.update_edges(B, &[(key.clone(), String::from("todo"))], &[])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let who = Actor::agent("claude/opus-5.5");
    s.write(T0, Origin::cli(), |w| w.agent_check("claude/opus-5.5", None)).unwrap();
    exec(&mut s, T0, &who, Cmd::Claim { id: Some(a) }).unwrap();
    exec(&mut s, T0 + S, &who, Cmd::Move { id: a, column: key.clone(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    assert_eq!(card(&s, a).card.column, key, "agent 沿连线移入错误节点");
    let out = exec(&mut s, T0 + 2 * S, &who, Cmd::Move { id: a, column: "todo".into(), top: false, index: None, order: None, before: None, after: None });
    assert!(out.is_err(), "有连线也不行：只有人能移出错误节点");
    exec(&mut s, T0 + 3 * S, &Actor::human(), Cmd::Move { id: a, column: "todo".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    assert_eq!(card(&s, a).card.column, "todo");
}

#[test]
fn crashed_dispatch_moves_card_to_error_status() {
    let mut s = store();
    let key = add_error_status(&mut s);
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    lead_round(&mut s);
    let a = new_card_tagged(&mut s, "A");
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(1), ..Default::default() })).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().expect("定时领取应该开始一张");
    assert_eq!(d.card, a);
    let id = d.id;
    // agent 异常退出：卡片沿连线移到「错误」，不留在进行中
    let d = end(&mut s, T0 + 3 * S, id, Some(1), &["boom"]);
    assert_eq!((d.state.as_str(), d.result.as_deref()), ("done", Some("crashed")));
    assert_eq!(card(&s, a).card.column, key, "异常退出的卡片移到错误节点");
    // 50 秒后不会释放回待办，下一轮也领不到它
    beat(&mut s, T0 + 55 * S, &report("mbp", "i1", vec![])).unwrap();
    s.sweep(T0 + 60 * S).unwrap();
    assert_eq!(card(&s, a).card.column, key, "错误节点里的卡片不会被心跳超时释放");
    s.write(T0 + 61 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(s.write(T0 + 62 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none(), "定时领取不会再领错误节点里的卡片");
}

#[test]
fn error_status_wins_over_human_status() {
    let mut s = store();
    let key = add_error_status(&mut s);
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    s.write(T0 + 2 * S, Origin::ui(None), |w| w.dispatch_cancel(d.id, &Actor::human(), "buglan")).unwrap();
    // 人停止：同样去错误节点（doing 同时连着 check 和 错误）
    end(&mut s, T0 + 3 * S, d.id, None, &[]);
    assert_eq!(card(&s, a).card.column, key);
}

#[test]
fn crashed_dispatch_goes_to_human_status_when_no_error_status() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    begin(&mut s, T0 + S, d.id).unwrap();
    // 没有错误节点：行为保持原样，去需要人处理的状态
    end(&mut s, T0 + 2 * S, d.id, Some(2), &[]);
    assert_eq!(card(&s, a).card.column, "check");
}

#[test]
fn crashed_dispatch_explains_unreachable_error_status() {
    let mut s = store();
    // 错误节点存在，但从「进行中」连不出去：另一个锁定状态「测试中」能连过去，doing 不能
    s.write(T0, Origin::ui(None), |w| w.delete_status(B, "check")).unwrap();
    let lock = s.write(T0, Origin::ui(None), |w| w.create_status(B, "测试中", None)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_status(B, &lock.key, StatusPatch { lock: Some(true), ..Default::default() })).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_edges(B, &[("review".into(), lock.key.clone())], &[])).unwrap();
    let st = s.write(T0, Origin::ui(None), |w| w.create_status(B, "错误", None)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_edges(B, &[(lock.key.clone(), st.key.clone())], &[])).unwrap();
    set_error(&mut s, &st.key, true);
    let probe = s.read(|r| r.flow(&r.resolve(B)?)).unwrap();
    assert!(probe.has_edge(&lock.key, &st.key) && probe.is_error(&st.key), "错误节点要能认出来");
    assert!(!probe.has_edge("doing", &st.key), "doing 到不了错误节点");
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    begin(&mut s, T0 + S, d.id).unwrap();
    end(&mut s, T0 + 2 * S, d.id, Some(1), &[]);
    let c = card(&s, a);
    assert_eq!(c.card.column, "doing", "没有连线时不硬移");
    let comment = c.activity.iter().rev().find(|x| x.kind == "comment").expect("应该有结束评论");
    assert!(comment.text.contains("这块白板有错误状态，但从「进行中」连不出去"), "{}", comment.text);
    assert!(comment.text.contains("卡片 50 秒后会回到「待办」最上面"), "{}", comment.text);
}

#[test]
fn review_path_unchanged_with_error_status() {
    let mut s = store();
    add_error_status(&mut s);
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    begin(&mut s, T0 + S, d.id).unwrap();
    // agent 正常交审后退出：卡片已经不在它手里，在交审状态，不动
    exec(&mut s, T0 + 2 * S, &Actor::agent("claude-mbp/opus-5.5"), Cmd::Move { id: a, column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    let d = end(&mut s, T0 + 3 * S, d.id, Some(0), &[]);
    assert_eq!(d.result.as_deref(), Some("review"));
    assert_eq!(card(&s, a).card.column, "review");
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
            model: String::new(),
            ..Default::default()
        },
        HostAgent {
            id: "claude".into(),
            name: "Claude Code".into(),
            version: "2.4.1".into(),
            found: true,
            envs: vec![check("pwsh", false, ""), check("git-bash", false, ""), check("wsl:Ubuntu", true, "2.5.0"), check("cmd", false, "")],
            env: None,
            no_skill: None,
            model: "opus-5.5".into(),
            ..Default::default()
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
    assert!(acts.iter().any(|t| t.starts_with("派发开始：mbp 上的 pi（Git Bash）以 pi-mbp/pi 领取")), "{acts:?}");
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
    exec(s, at, &a, Cmd::Move { id, column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
}

fn next(s: &mut Store, at: i64, name: &str) -> Result<CardDetail> {
    s.next(at, Origin::cli(), name)
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

/// 给机器加一条领取规则 `deepseek → claude`，用例里的卡片都带 `deepseek` 标签（建卡用 [`new_card_tagged`]）。
/// 没有规则时定时领取不领卡、也不能开始一轮，要测定时领取的用例都得先调它
fn lead_round(s: &mut Store) {
    if s.read(|r| r.labels("default")).unwrap_or_default().iter().all(|l| !l.name.eq_ignore_ascii_case("deepseek")) {
        s.write(T0, Origin::ui(None), |w| w.create_label("default", "deepseek", None, false)).unwrap();
    }
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { rules: Some(vec![cond("in", &["deepseek"], "claude")]), fallback: Some(String::new()), ..Default::default() })).unwrap();
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
    let cards: Vec<i64> = (0..6).map(|i| new_card_tagged(&mut s, &format!("卡{i}"))).collect();
    for c in &cards {
        tag(&mut s, *c, ui);
    }
    lead_round(&mut s);
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
    assert_eq!(name2, name, "上一个会话已经离线：新会话复用同一个名字（名字说的是机器、agent、模型，不是会话）");
    submit(&mut s, t + 3 * S, &name2, cards[4]);
    assert_eq!(next(&mut s, t + 3 * S, &name2).unwrap().card.id, cards[5]);
    submit(&mut s, t + 4 * S, &name2, cards[5]);
    assert_eq!(next(&mut s, t + 4 * S, &name2).unwrap_err().code, ErrorCode::SessionDone, "没有可领取的卡片");
}

/// 开会话时怎么排：优先标签 > 和第一张共同标签多的 > 规则 1 的顺序；排好之后不再变
#[test]
fn session_plan_prefers_priority_then_common_labels() {
    // N = 2，第一张带 UI，其余从上到下 X（无共同标签）、Y（带 UI）：排进去的是 Y
    let setup = |extra: &dyn Fn(&mut Store, i64, i64)| {
        let mut s = store();
        beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
        let ui = label(&mut s, "UI");
        let first = new_card_tagged(&mut s, "第一张");
        tag(&mut s, first, ui);
        let x = new_card_tagged(&mut s, "X");
        let y = new_card_tagged(&mut s, "Y");
        extra(&mut s, y, ui);
        set_host(&mut s, HostPatch { per_session: Some(2), ..Default::default() }).unwrap();
        lead_round(&mut s);
        s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
        let d = pull(&mut s, T0 + S).unwrap();
        assert_eq!(d.card, first);
        let planned: Vec<i64> = session_of(&s, d.id).into_iter().filter(|q| q.state == "queued").map(|q| q.card).collect();
        (s, d, x, y, planned)
    };
    let (_, _, _, y, planned) = setup(&|s, y, ui| tag(s, y, ui));
    assert_eq!(planned, vec![y]);
    // 都没有共同标签：按规则 1 的顺序排 X
    let (_, _, x, _, planned) = setup(&|_, _, _| {});
    assert_eq!(planned, vec![x]);
    // 有带优先标签的（Z，在最下面）：排它
    let (mut s, d, x, y, planned) = setup(&|s, y, ui| tag(s, y, ui));
    assert_eq!(planned, vec![y]);
    // 会话开始后再给 X 打优先标签：board next 仍领 Y（排好之后不再变）
    let urgent = label(&mut s, "急");
    s.write(T0, Origin::ui(None), |w| w.update_label("default", urgent, None, None, None, Some(true))).unwrap();
    tag(&mut s, x, urgent);
    let name = d.name.unwrap();
    submit(&mut s, T0 + 2 * S, &name, d.card);
    assert_eq!(next(&mut s, T0 + 2 * S, &name).unwrap().card.id, y);

    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let ui = label(&mut s, "UI");
    let first = new_card_tagged(&mut s, "第一张");
    tag(&mut s, first, ui);
    let _y = new_card_tagged(&mut s, "Y");
    let z = new_card_tagged(&mut s, "Z");
    let urgent = label(&mut s, "急");
    s.write(T0, Origin::ui(None), |w| w.update_label("default", urgent, None, None, None, Some(true))).unwrap();
    tag(&mut s, z, urgent);
    set_host(&mut s, HostPatch { per_session: Some(2), ..Default::default() }).unwrap();
    lead_round(&mut s);
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    // 带优先标签的 Z 先被开始（规则 1）；第一张排进会话
    let d = pull(&mut s, T0 + S).unwrap();
    assert_eq!(d.card, z);
    let planned: Vec<i64> = session_of(&s, d.id).into_iter().filter(|q| q.state == "queued").map(|q| q.card).collect();
    assert_eq!(planned, vec![first]);
}

/// N = 1、手动派发、不是 board host 的会话、上一张还没交审、暂停接单、有人停止
#[test]
fn next_refuses_when_session_should_end() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    lead_round(&mut s);
    for t in ["A", "B", "C"] {
        new_card_tagged(&mut s, t);
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
    s.write(T0 + 7 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = pull(&mut s, T0 + 7 * S).unwrap();
    let name = d.name.clone().unwrap();
    set_host(&mut s, HostPatch { paused: Some(true), ..Default::default() }).unwrap();
    submit(&mut s, T0 + 8 * S, &name, d.card);
    let e = next(&mut s, T0 + 8 * S, &name).unwrap_err();
    assert_eq!(e.code, ErrorCode::SessionDone);
    assert!(e.message.contains("暂停接单"), "{}", e.message);
    // 规则被删光：在跑的会话做完当前这张，board next 返回 SESSION_DONE，也不再自动领卡
    s.write(T0 + 9 * S, Origin::cli(), |w| {
        w.tx.execute("UPDATE hosts SET rules = '[]' WHERE name = 'mbp'", [])?;
        Ok(())
    })
    .unwrap();
    assert!(rules(&mut s, &[], None).unwrap().rules.is_empty());
    assert!(pull(&mut s, T0 + 9 * S).is_none(), "没有领取规则：不领");
}

/// 会话做第 2 张时异常退出、被停止：第 1 张的结果不变，第 2 张按异常退出 / 已停止处理，不再领下一张
#[test]
fn second_card_crash_or_stop_keeps_first_result() {
    for stop in [false, true] {
        let mut s = store();
        beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
        lead_round(&mut s);
        for t in ["A", "B", "C"] {
            new_card_tagged(&mut s, t);
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

/// 一轮领取开几个会话（同时跑 4 个）：先把一个会话排满再开下一个。返回每个会话（开始的那张 + 排着的）
fn open_sessions(cards: usize, per: i64) -> (Store, Vec<Vec<i64>>) {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    for i in 0..cards {
        new_card_tagged(&mut s, &format!("卡{i}"));
    }
    set_host(&mut s, HostPatch { max: Some(4), per_session: Some(per), ..Default::default() }).unwrap();
    lead_round(&mut s);
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let mut sessions = vec![];
    while let Some(d) = pull(&mut s, T0 + S) {
        sessions.push(session_of(&s, d.id).iter().map(|x| x.card).collect::<Vec<_>>());
    }
    (s, sessions)
}

/// 同时跑 4 个、N = 4：4、6、16、20 张卡分别开 1、2（4 + 2）、4、4 个会话；两个会话不会排到同一张卡；N = 1 时 4 张开 4 个会话
#[test]
fn session_fills_one_before_opening_the_next() {
    let sizes = |n: usize, per: i64| open_sessions(n, per).1.iter().map(Vec::len).collect::<Vec<_>>();
    assert_eq!(sizes(4, 4), vec![4]);
    assert_eq!(sizes(6, 4), vec![4, 2]);
    assert_eq!(sizes(16, 4), vec![4, 4, 4, 4]);
    assert_eq!(sizes(4, 1), vec![1, 1, 1, 1], "N = 1：有几个空位就同时开几个会话，各做一张");
    // 20 张：先开 4 个会话，剩下 4 张没有派发；在跑的进程不超过 4 个
    let (mut s, sessions) = open_sessions(20, 4);
    assert_eq!(sessions.iter().map(Vec::len).collect::<Vec<_>>(), vec![4, 4, 4, 4]);
    let mut all: Vec<i64> = sessions.concat();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 16, "两个会话不会排到同一张卡");
    let active = s.read(|r| r.active_dispatches()).unwrap();
    assert_eq!(active.iter().filter(|d| d.state == "running").count(), 4);
    assert!(host(&s, T0 + S).round, "空位满了，这一轮还没结束");
    // 有会话结束后再开新会话，排剩下的 4 张
    let first = active.iter().find(|d| d.state == "running").unwrap().clone();
    let name = first.name.clone().unwrap();
    let mut t = T0 + 2 * S;
    let mut cur = first.card;
    loop {
        submit(&mut s, t, &name, cur);
        match next(&mut s, t, &name) {
            Ok(c) => cur = c.card.id,
            Err(e) => {
                assert_eq!(e.code, ErrorCode::SessionDone);
                break;
            }
        }
        t += S;
    }
    end(&mut s, t, first.id, Some(0), &[]);
    let d = pull(&mut s, t + S).expect("空出位置：开新会话");
    let rest: Vec<i64> = session_of(&s, d.id).iter().map(|x| x.card).collect();
    assert_eq!(rest.len(), 4);
    assert!(rest.iter().all(|c| !all.contains(c)));
}

/// N = 4、6 张同标签的卡、同时跑 1 个：第 1 张在跑，第 2–4 张排在会话里（会话第 N/4 张）；第 5、6 张没有派发；
/// 会话里排着的不给 board host、不占空位；第一个进程做完排着的退出，第二个进程排好并做剩下的
#[test]
fn session_queue_is_planned_and_shown() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let cards: Vec<i64> = (0..6).map(|i| new_card_tagged(&mut s, &format!("卡{i}"))).collect();
    lead_round(&mut s);
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d1 = pull(&mut s, T0 + S).unwrap();
    let name = d1.name.clone().unwrap();
    assert_eq!((d1.card, d1.seq, d1.total), (cards[0], 1, 4));
    let queued: Vec<Dispatch> = session_of(&s, d1.id).into_iter().filter(|d| d.state == "queued").collect();
    assert_eq!(queued.iter().map(|d| (d.card, d.seq, d.total)).collect::<Vec<_>>(), vec![(cards[1], 2, 4), (cards[2], 3, 4), (cards[3], 4, 4)]);
    assert!(queued.iter().all(|d| d.in_session_queue() && d.session == d1.id && d.source == "cron"));
    let a = card(&s, cards[2]).activity.into_iter().rev().find(|a| a.text.starts_with("派发到")).unwrap();
    assert_eq!(a.text, format!("派发到 mbp · Claude Code（定时），会话第 3/4 张，排队等 {name} 接着做"));
    assert!(s.read(|r| r.card_dispatch(&r.resolve(B)?, cards[4])).unwrap().is_none(), "第 5 张没有派发");
    // board host 拿到的排队里没有会话里排着的，也不能用 begin 开始它们
    let w = beat(&mut s, T0 + S, &report("mbp", "i1", vec![d1.id])).unwrap();
    assert!(w.queued.is_empty(), "{:?}", w.queued);
    assert_eq!(begin(&mut s, T0 + S, queued[0].id).unwrap_err().code, ErrorCode::Usage);
    assert!(pull(&mut s, T0 + S).is_none(), "同时跑 1 个：没有空位");
    // board next 按排好的顺序开始
    let mut t = T0 + 2 * S;
    for (k, want) in cards[1..4].iter().enumerate() {
        let cur = s.read(|r| r.active_dispatches()).unwrap().into_iter().find(|d| d.state == "running").unwrap();
        submit(&mut s, t, &name, cur.card);
        let c = next(&mut s, t, &name).unwrap();
        assert_eq!((c.card.id, c.card.holder.as_deref()), (*want, Some(name.as_str())));
        let started = c.activity.iter().rev().find(|a| a.text.starts_with("派发开始")).unwrap();
        assert!(started.text.ends_with(&format!("接着做（定时，这个会话的第 {} 张）", k + 2)), "{}", started.text);
        t += S;
    }
    let cur = s.read(|r| r.active_dispatches()).unwrap().into_iter().find(|d| d.state == "running").unwrap();
    submit(&mut s, t, &name, cur.card);
    let e = next(&mut s, t, &name).unwrap_err();
    assert_eq!(e.code, ErrorCode::SessionDone);
    assert!(e.message.contains("排着的卡片都开始过或取消了"), "{}", e.message);
    let e = end(&mut s, t + S, d1.id, Some(0), &[]);
    assert_eq!((e.id, e.result.as_deref()), (cur.id, Some("review")));
    let all = session_of(&s, d1.id);
    assert_eq!(all.len(), 4, "每张卡各一条派发");
    assert!(all.iter().all(|d| d.result.as_deref() == Some("review")));
    // 第二个进程：排好剩下的 2 张
    let d5 = pull(&mut s, t + 2 * S).unwrap();
    assert_eq!((d5.card, d5.total), (cards[4], 2));
    let name2 = d5.name.clone().unwrap();
    submit(&mut s, t + 3 * S, &name2, cards[4]);
    assert_eq!(next(&mut s, t + 3 * S, &name2).unwrap().card.id, cards[5]);
}

/// 会话里排着的卡：别人领不走、手动派发不了；人取消其中一张或拖出待办，board next 跳过它领后面那张
#[test]
fn session_queue_occupies_cards_and_can_be_canceled() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let cards: Vec<i64> = (0..5).map(|i| new_card_tagged(&mut s, &format!("卡{i}"))).collect();
    let free = new_card(&mut s, T0, "没有派发");
    lead_round(&mut s);
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d1 = pull(&mut s, T0 + S).unwrap();
    let name = d1.name.clone().unwrap();
    let queued: Vec<Dispatch> = session_of(&s, d1.id).into_iter().filter(|d| d.state == "queued").collect();
    assert_eq!(queued.len(), 3);
    s.write(T0, Origin::cli(), |w| w.agent_check("alpha/x", None)).unwrap();
    let alpha = Actor::agent("alpha/x");
    assert_eq!(exec(&mut s, T0 + S, &alpha, Cmd::Claim { id: Some(cards[1]) }).unwrap_err().code, ErrorCode::Dispatched);
    assert_eq!(exec(&mut s, T0 + S, &alpha, Cmd::Claim { id: None }).unwrap().card.id, cards[4], "不带 ID 的领取跳过排着的");
    assert_eq!(dispatch(&mut s, T0 + S, cards[2], "mbp", None).unwrap_err().code, ErrorCode::AlreadyDispatched);
    let _ = free;
    // 人取消第 2 张、把第 3 张拖出待办：都「已取消」，board next 领第 4 张
    s.write(T0 + 2 * S, Origin::ui(None), |w| w.dispatch_cancel(queued[0].id, &Actor::human(), "buglan")).unwrap();
    exec(&mut s, T0 + 2 * S, &Actor::human(), Cmd::Move { id: cards[2], column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    assert_eq!(get(&s, queued[0].id).result.as_deref(), Some("canceled"));
    assert_eq!(get(&s, queued[1].id).result.as_deref(), Some("canceled"));
    submit(&mut s, T0 + 3 * S, &name, d1.card);
    assert_eq!(next(&mut s, T0 + 3 * S, &name).unwrap().card.id, cards[3]);
    // 人取消的那张这一轮不再排：会话结束后新开的会话不会排它
    submit(&mut s, T0 + 4 * S, &name, cards[3]);
    assert_eq!(next(&mut s, T0 + 4 * S, &name).unwrap_err().code, ErrorCode::SessionDone);
    end(&mut s, T0 + 5 * S, d1.id, Some(0), &[]);
    assert!(pull(&mut s, T0 + 6 * S).is_none());
    assert_eq!(card(&s, cards[1]).card.column, "todo");
}

/// N = 4 的会话做第 2 张时异常退出、被人停止：第 1 张结果不变，第 2 张异常退出 / 已停止，第 3、4 张「已取消」、
/// 卡片留在待办，这一轮有空位时由新会话重新排进去。暂停接单：board next 返回 SESSION_DONE，排着的取消
#[test]
fn session_end_cancels_the_rest_and_a_new_session_takes_them() {
    for stop in [false, true] {
        let mut s = store();
        beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
        let cards: Vec<i64> = (0..4).map(|i| new_card_tagged(&mut s, &format!("卡{i}"))).collect();
        set_host(&mut s, HostPatch { max: Some(2), ..Default::default() }).unwrap();
        lead_round(&mut s);
        s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
        let d1 = pull(&mut s, T0 + S).unwrap();
        assert!(pull(&mut s, T0 + S).is_none(), "卡都排进了第一个会话");
        assert!(host(&s, T0 + S).round, "还有会话排着卡片：这一轮不算结束");
        let name = d1.name.clone().unwrap();
        submit(&mut s, T0 + 2 * S, &name, cards[0]);
        assert_eq!(next(&mut s, T0 + 2 * S, &name).unwrap().card.id, cards[1]);
        let d2 = s.read(|r| r.active_dispatches()).unwrap().into_iter().find(|d| d.state == "running").unwrap();
        if stop {
            s.write(T0 + 3 * S, Origin::ui(None), |w| w.dispatch_cancel(d2.id, &Actor::human(), "buglan")).unwrap();
        }
        let e = end(&mut s, T0 + 4 * S, d1.id, Some(1), &["boom"]);
        assert_eq!((e.id, e.result.as_deref()), (d2.id, Some(if stop { "stopped" } else { "crashed" })));
        assert_eq!(get(&s, d1.id).result.as_deref(), Some("review"), "第 1 张的结果不变");
        let rest: Vec<Dispatch> = session_of(&s, d1.id).into_iter().filter(|d| d.card == cards[2] || d.card == cards[3]).collect();
        assert!(rest.iter().all(|d| d.result.as_deref() == Some("canceled")), "{rest:?}");
        assert_eq!(card(&s, cards[2]).card.column, "todo");
        // 这一轮还没结束、有空位：新会话把它们重新排进去
        let d3 = pull(&mut s, T0 + 5 * S).expect("新会话");
        assert_eq!(session_of(&s, d3.id).iter().map(|d| d.card).collect::<Vec<_>>(), vec![cards[2], cards[3]]);
    }

    // 暂停接单：在跑的会话做完当前这张，board next 返回 SESSION_DONE，排着的取消
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let cards: Vec<i64> = (0..3).map(|i| new_card_tagged(&mut s, &format!("卡{i}"))).collect();
    lead_round(&mut s);
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d1 = pull(&mut s, T0 + S).unwrap();
    let name = d1.name.clone().unwrap();
    set_host(&mut s, HostPatch { paused: Some(true), ..Default::default() }).unwrap();
    submit(&mut s, T0 + 2 * S, &name, cards[0]);
    let e = next(&mut s, T0 + 2 * S, &name).unwrap_err();
    assert_eq!(e.code, ErrorCode::SessionDone);
    assert!(e.message.contains("暂停接单"), "{}", e.message);
    let rest: Vec<Dispatch> = session_of(&s, d1.id).into_iter().filter(|d| d.id != d1.id).collect();
    assert_eq!(rest.len(), 2);
    assert!(rest.iter().all(|d| d.result.as_deref() == Some("canceled")), "SESSION_DONE 时排着的已经取消（先提交再报错）");
    let a = card(&s, cards[1]).activity.into_iter().rev().find(|a| a.text.starts_with("会话结束")).unwrap();
    assert!(a.text.contains("暂停接单"), "{}", a.text);
}

/// 机器掉线：会话里排着的取消
#[test]
fn session_queue_is_canceled_when_host_goes_offline() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    for i in 0..3 {
        new_card_tagged(&mut s, &format!("卡{i}"));
    }
    lead_round(&mut s);
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d1 = pull(&mut s, T0 + S).unwrap();
    s.write(T0 + 120 * S, Origin::system(), |w| w.sweep_hosts()).unwrap();
    let all = session_of(&s, d1.id);
    assert_eq!(get(&s, d1.id).result.as_deref(), Some("lost"));
    assert!(all.iter().filter(|d| d.id != d1.id).all(|d| d.result.as_deref() == Some("canceled")), "{all:?}");
}

/// 改「一个会话最多做几张」：已经排好的会话不变，之后新开的按新值排；手动派发仍然一个会话一张
#[test]
fn per_session_change_only_affects_new_sessions() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    for i in 0..8 {
        new_card_tagged(&mut s, &format!("卡{i}"));
    }
    set_host(&mut s, HostPatch { max: Some(2), per_session: Some(2), ..Default::default() }).unwrap();
    lead_round(&mut s);
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d1 = pull(&mut s, T0 + S).unwrap();
    set_host(&mut s, HostPatch { per_session: Some(4), ..Default::default() }).unwrap();
    assert_eq!(session_of(&s, d1.id).len(), 2, "已经排好的会话不变");
    let d2 = pull(&mut s, T0 + S).unwrap();
    assert_eq!(session_of(&s, d2.id).len(), 4, "新开的按新值排");
    // 手动派发：一个会话一张
    let c = new_card(&mut s, T0, "手动");
    let m = dispatch(&mut s, T0 + 2 * S, c, "mbp", None).unwrap();
    assert_eq!((m.seq, m.total), (1, 1));
}

/* ---------------- 领取规则：按标签筛卡片、按标签选 agent ---------------- */

/// 一台找到了 claude、dsh 的机器
fn report_cd(instance: &str) -> HostReport {
    let mut r = report("mbp", instance, vec![]);
    r.agents = vec![claude(true), dsh(true)];
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
    // 这里只看规则把卡片交给谁：一个会话一张（N = 1），每张卡都由 pull 开始
    set_host(s, HostPatch { max: Some(8), per_session: Some(1), ..Default::default() }).unwrap();
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

/// 没有一条领取规则时不自动领取：「现在领一轮」被拒绝，定时到点也不领卡；兜底单独设了也不算规则。
/// 手动派发、`board claim` 不受影响；加一条规则后照常领取
#[test]
fn no_rules_never_auto_claims() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    new_card(&mut s, T0, "A");
    // 「现在领一轮」：没有规则时拒绝，不开这一轮
    let e = s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("领取规则"), "{}", e.message);
    assert!(!host(&s, T0 + S).round);
    // 只设了兜底也一样
    set_host(&mut s, HostPatch { fallback: Some("dsh".into()), cron_on: Some(true), ..Default::default() }).unwrap();
    assert_eq!(s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap_err().code, ErrorCode::Usage);
    assert!(host(&s, T0 + S).rules.is_empty());
    // 定时到点：board host 收到拒绝，不开始这一轮
    assert!(!host(&s, T0 + S).round);
    // 就算有一轮在进行（规则删光之前开的），领卡时也不领，这一轮结束
    set_host(&mut s, HostPatch { max: Some(2), ..Default::default() }).unwrap();
    s.write(T0 + S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap();
    assert_eq!(card(&s, 1).card.column, "todo");
    assert!(s.read(|r| r.active_dispatches()).unwrap().is_empty(), "没有生成派发");
    // 手动派发不受影响（用默认 agent）
    assert_eq!(dispatch(&mut s, T0 + 2 * S, 1, "mbp", None).unwrap().agent, "claude");
    // 别人的 `board claim` 也不受影响
    s.write(T0 + 2 * S, Origin::cli(), |w| w.agent_check("alpha/x", None)).unwrap();
    let b = new_card(&mut s, T0, "B");
    assert_eq!(exec(&mut s, T0 + 3 * S, &Actor::agent("alpha/x"), Cmd::Claim { id: Some(b) }).unwrap().card.id, b);
    // 加一条规则后「现在领一轮」可以开始，只领规则命中的卡片
    let l = label(&mut s, "dsh");
    let c = new_card(&mut s, T0, "C");
    tag(&mut s, c, l);
    rules(&mut s, &[("dsh", "dsh")], None).unwrap();
    s.write(T0 + 4 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert_eq!(pull(&mut s, T0 + 4 * S).unwrap().card, c);
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

/* ---------------- 领取规则的条件：in / all / not_in / equal / only / none ---------------- */

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
fn rule_conditions_all_only_none() {
    // 卡片：A、B、A+B、A+B+C、无标签
    let setup = || {
        let mut s = store();
        beat(&mut s, T0, &report_cd("i1")).unwrap();
        let la = label(&mut s, "A");
        let lb = label(&mut s, "B");
        let lc = label(&mut s, "C");
        let a = new_card(&mut s, T0, "A");
        tag(&mut s, a, la);
        let b = new_card(&mut s, T0, "B");
        tag(&mut s, b, lb);
        let ab = new_card(&mut s, T0, "A + B");
        tag(&mut s, ab, la);
        tag(&mut s, ab, lb);
        let abc = new_card(&mut s, T0, "A + B + C");
        tag(&mut s, abc, la);
        tag(&mut s, abc, lb);
        tag(&mut s, abc, lc);
        let none = new_card(&mut s, T0, "无标签");
        (s, a, b, ab, abc, none)
    };
    // all [A, B] → claude：两个都带的匹配（多出的 C 不管）；只带一个的、没标签的不匹配
    let (mut s, _, _, ab, abc, _) = setup();
    set_conds(&mut s, vec![cond("all", &["A", "B"], "claude")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(ab, "claude".into()), (abc, "claude".into())]);
    // all [A, B] 没设兜底：3 张不匹配
    let (mut s, ..) = setup();
    assert_eq!(set_conds(&mut s, vec![cond("all", &["A", "B"], "claude")], None).unwrap().unmatched, 3);
    // only [A, B] → claude：都在 A、B 里且至少带一个的匹配（A、B、A+B）；带 C 的、没标签的不匹配
    let (mut s, a, b, ab, _, _) = setup();
    set_conds(&mut s, vec![cond("only", &["A", "B"], "claude")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(a, "claude".into()), (b, "claude".into()), (ab, "claude".into())]);
    // only [A, B, C] → claude：四张带标签的都是它的子集
    let (mut s, a, b, ab, abc, _) = setup();
    set_conds(&mut s, vec![cond("only", &["A", "B", "C"], "claude")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(a, "claude".into()), (b, "claude".into()), (ab, "claude".into()), (abc, "claude".into())]);
    // none → dsh：一个标签都没有的卡片
    let (mut s, _, _, _, _, n) = setup();
    set_conds(&mut s, vec![cond("none", &[], "dsh")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(n, "dsh".into())]);
    // 「没有标签」不看标签列表：带着标签保存也按没有标签算，存进去时清掉
    let (mut s, _, _, _, _, n) = setup();
    let h = set_conds(&mut s, vec![cond("none", &["A"], "dsh")], None).unwrap();
    assert!(h.rules[0].labels.is_empty());
    assert_eq!(round_all(&mut s, T0 + S), vec![(n, "dsh".into())]);
    // 多条规则按顺序第一条命中：only [A] → claude 在前，没有标签 → dsh 在后
    let (mut s, a, _, _, _, n) = setup();
    set_conds(&mut s, vec![cond("only", &["A"], "claude"), cond("none", &[], "dsh")], None).unwrap();
    assert_eq!(round_all(&mut s, T0 + S), vec![(a, "claude".into()), (n, "dsh".into())]);
}

#[test]
fn rule_conditions_are_validated() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    label(&mut s, "claude");
    assert_eq!(set_conds(&mut s, vec![cond("like", &["claude"], "claude")], None).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(set_conds(&mut s, vec![cond("in", &[], "claude")], None).unwrap_err().code, ErrorCode::Usage);
    // 只有「没有标签」不带标签列表，其他条件都要至少一个标签；「没有标签」不选标签也能存
    assert_eq!(set_conds(&mut s, vec![cond("all", &[], "claude")], None).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(set_conds(&mut s, vec![cond("only", &[], "claude")], None).unwrap_err().code, ErrorCode::Usage);
    assert!(set_conds(&mut s, vec![cond("none", &[], "claude")], None).is_ok());
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

/// 机器改名（桌面端「设置 → 本机」保存机器名）：原地改名，设置和历史派发记录保留，不新增第二台机器
#[test]
fn rename_host_keeps_settings_and_dispatches() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let id = host(&s, T0).id;
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(3), ..Default::default() })).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!(d.name.as_deref(), Some("claude-mbp/opus-5.5"));
    // 不改名：原样返回
    let h = s.write(T0 + 2 * S, Origin::cli(), |w| w.host_rename("mbp", "mbp")).unwrap();
    assert_eq!((h.name.as_str(), h.id, h.max, h.online), ("mbp", id, 3, true));
    // 机器不存在、名字不对
    assert_eq!(s.write(T0 + 2 * S, Origin::cli(), |w| w.host_rename("nope", "win")).unwrap_err().code, ErrorCode::NoSuchHost);
    assert_eq!(s.write(T0 + 2 * S, Origin::cli(), |w| w.host_rename("mbp", "Win!")).unwrap_err().code, ErrorCode::Usage);
    // 真实流程：board host 停下来（final 上报，机器离线）→ 改名 → 用新名字上报
    let mut fin = report("mbp", "i1", vec![]);
    fin.done = true;
    end(&mut s, T0 + 3 * S, d.id, Some(0), &[]);
    beat(&mut s, T0 + 3 * S, &fin).unwrap();
    let h = s.write(T0 + 4 * S, Origin::cli(), |w| w.host_rename("mbp", "win")).unwrap();
    assert_eq!((h.name.as_str(), h.id, h.max, h.online), ("win", id, 3, false));
    // 派发记录跟着新名字；旧名字不再有机器
    assert_eq!(get(&s, d.id).host, "win");
    assert_eq!(s.read(|r| r.host("mbp", T0 + 4 * S)).unwrap_err().code, ErrorCode::NoSuchHost);
    // 新名字上报：同一台机器接着用（id、设置不变）
    let w = beat(&mut s, T0 + 5 * S, &report("win", "i2", vec![])).unwrap();
    assert_eq!((w.host.id, w.host.max, w.host.online), (id, 3, true));
    // 历史派发记录保留在新名字下
    let recent = s.read(|r| r.host_recent(id, 10)).unwrap();
    let d = recent.iter().find(|x| x.id == d.id).unwrap();
    assert_eq!(d.host, "win");
}

/// 改成被占用的新名字：在线的 HOST_NAME_TAKEN，离线的先去「机器」里移除
#[test]
fn rename_host_rejects_taken_names() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    beat(&mut s, T0, &report("desk", "j1", vec![])).unwrap();
    let e = s.write(T0 + S, Origin::cli(), |w| w.host_rename("mbp", "desk")).unwrap_err();
    assert_eq!(e.code, ErrorCode::HostNameTaken);
    // 两台都下线之后仍然不能改：先在「机器」里移除那台离线的
    s.sweep(T0 + 60 * S).unwrap();
    assert_eq!(s.write(T0 + 61 * S, Origin::cli(), |w| w.host_rename("mbp", "desk")).unwrap_err().code, ErrorCode::Usage);
    s.write(T0 + 61 * S, Origin::ui(None), |w| w.host_remove("desk", &Actor::human(), "buglan")).unwrap();
    let h = s.write(T0 + 61 * S, Origin::cli(), |w| w.host_rename("mbp", "desk")).unwrap();
    assert_eq!(h.name, "desk");
    assert_eq!(s.read(|r| r.host("mbp", T0 + 61 * S)).unwrap_err().code, ErrorCode::NoSuchHost);
}

/// 改名不重启 board host：服务端先改名，board host 读到新配置之前还用旧名字上报、开始派发，
/// 按进程标识认回改名后的那台机器，不新建第二台；在跑的派发照常续心跳。读到新配置后用新名字上报，接着是同一台
#[test]
fn rename_while_host_keeps_running() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let id = host(&s, T0).id;
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(2), ..Default::default() })).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    let h = s.write(T0 + 2 * S, Origin::cli(), |w| w.host_rename("mbp", "win")).unwrap();
    assert_eq!((h.id, h.online), (id, true));
    // 旧名字、同一个进程：认回 win，在跑的派发照常续心跳
    let w = beat(&mut s, T0 + 3 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    assert_eq!((w.host.id, w.host.name.as_str()), (id, "win"));
    assert_eq!(w.running.iter().map(|x| x.id).collect::<Vec<_>>(), vec![d.id]);
    assert_eq!(get(&s, d.id).state, "running");
    // 改名之后派给 win 的卡片，旧名字的 board host 也能开始
    let d2 = dispatch(&mut s, T0 + 3 * S, b, "win", None).unwrap();
    let d2 = begin(&mut s, T0 + 4 * S, d2.id).unwrap();
    assert_eq!(d2.host, "win");
    // 只有一台机器
    let all = s.read(|r| r.hosts(T0 + 4 * S)).unwrap();
    assert_eq!(all.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), vec!["win"]);
    // 读到新配置：新名字、同一个进程接着上报
    let w = beat(&mut s, T0 + 5 * S, &report("win", "i1", vec![d.id, d2.id])).unwrap();
    assert_eq!((w.host.id, w.running.len()), (id, 2));
    assert_eq!(s.read(|r| r.hosts(T0 + 5 * S)).unwrap().len(), 1);
    // 别的进程用旧名字上报：和改过名的机器无关，按新机器建立
    let w = beat(&mut s, T0 + 6 * S, &report("mbp", "j1", vec![])).unwrap();
    assert_ne!(w.host.id, id);
    // 不认识的进程标识、也没有这个名字：开始派发报 NO_SUCH_HOST
    let e = s.write(T0 + 6 * S, Origin::cli(), |w| w.dispatch_pull("desk", "zz", Some(1))).unwrap_err();
    assert_eq!(e.code, ErrorCode::NoSuchHost);
}

/* ---------------- 名字的会话标识（同名会话互相干扰） ---------------- */

#[test]
fn name_session_guards_beats_and_final() {
    use crate::agents::{BeatBy, NameOwner};
    let mut s = store();
    // 旧版 CLI：没有标识，谁发心跳、final 都接受（和以前一样）
    s.write(T0, Origin::cli(), |w| w.agent_check("old/x", None)).unwrap();
    s.write(T0 + S, Origin::cli(), |w| w.agent_beat_by("old/x", false, BeatBy::Cli(Some("whatever")))).unwrap();
    s.write(T0 + S, Origin::cli(), |w| w.agent_beat("old/x", true)).unwrap();
    assert_eq!(s.write(T0 + 2 * S, Origin::cli(), |w| w.agent_beat("old/x", false)).unwrap_err().code, ErrorCode::NameOffline);

    // 新版 CLI：check 时发标识，只有带同一个标识的心跳和 final 被接受
    let info = s.write(T0, Origin::cli(), |w| w.agent_check_owned("new/x", None, NameOwner::Session("abc"))).unwrap();
    assert_eq!(info.session.as_deref(), Some("abc"), "check 返回会话标识");
    assert!(s.read(|r| r.agent("new/x")).unwrap().unwrap().session.is_none(), "别处读不到标识");
    for by in [BeatBy::Cli(None), BeatBy::Cli(Some("xyz"))] {
        let e = s.write(T0 + S, Origin::cli(), |w| w.agent_beat_by("new/x", true, by)).unwrap_err();
        assert_eq!(e.code, ErrorCode::NameTaken);
        assert!(e.message.contains("另一个会话"), "{}", e.message);
    }
    s.read(|r| r.require_online("new/x", T0 + S)).expect("被拒之后名字仍在线");
    s.write(T0 + S, Origin::cli(), |w| w.agent_beat_by("new/x", false, BeatBy::Cli(Some("abc")))).unwrap();
    s.write(T0 + 2 * S, Origin::cli(), |w| w.agent_beat_by("new/x", true, BeatBy::Cli(Some("abc")))).unwrap();
    assert!(s.read(|r| r.require_online("new/x", T0 + 2 * S)).is_err(), "自己的 final 照常让名字离线");
    // 离线后别的会话重新 check：拿到新标识，旧标识的心跳被拒
    s.write(T0 + 3 * S, Origin::cli(), |w| w.agent_check_owned("new/x", None, NameOwner::Session("def"))).unwrap();
    assert_eq!(s.write(T0 + 4 * S, Origin::cli(), |w| w.agent_beat_by("new/x", false, BeatBy::Cli(Some("abc")))).unwrap_err().code, ErrorCode::NameTaken);
}

/// board host 派发生成的名字属于那台 host：派发会话里的 agent 发 final 被拒，名字照样在线、派发不受影响
#[test]
fn dispatch_name_belongs_to_the_host() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    let name = d.name.clone().unwrap();
    let e = s.write(T0 + 2 * S, Origin::cli(), |w| w.agent_beat(&name, true)).unwrap_err();
    assert_eq!(e.code, ErrorCode::NameTaken);
    assert!(e.message.contains("board host"), "{}", e.message);
    // board host 的上报照常替它发心跳
    beat(&mut s, T0 + 30 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    s.read(|r| r.require_online(&name, T0 + 60 * S)).expect("board host 的心跳维持在线");
    assert_eq!(get(&s, d.id).state, "running");
}

/* ------------------------------ 环境检查（PRD §9.1） ------------------------------ */

fn set_ready_check(s: &mut Store, cmd: &str, win: &str) {
    s.write(T0, Origin::ui(None), |w| w.set_ready_check(B, Some(cmd), Some(win))).unwrap();
}

/// board host 上报 default/main 的环境检查结果
fn ready_report(instance: &str, cmd: &str, ok: bool, tail: &[&str]) -> HostReport {
    let mut rep = report("mbp", instance, vec![]);
    rep.dirs[0].ready = Some(hosts::DirReady {
        cmd: cmd.into(),
        ok,
        code: Some(if ok { 0 } else { 127 }),
        tail: tail.iter().map(|x| x.to_string()).collect(),
        at: T0,
    });
    rep
}

#[test]
fn ready_check_is_sent_per_os_and_validated() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!(host(&s, T0).dirs[0].check, None, "没写检查命令：不检查");
    set_ready_check(&mut s, "  cargo --version  ", "where cargo");
    let ws = s.read(|r| r.workspace(T0)).unwrap();
    let b = ws.projects[0].boards.iter().find(|b| b.path == B).unwrap();
    assert_eq!(b.ready_check, ReadyCheck { cmd: "cargo --version".into(), win: "where cargo".into() }, "去掉首尾空白");
    assert_eq!(host(&s, T0).dirs[0].check.as_deref(), Some("cargo --version"), "macOS 跑「所有系统」那条");
    let mut w = report("win", "w1", vec![]);
    w.os = "Windows 11 Pro".into();
    beat(&mut s, T0, &w).unwrap();
    assert_eq!(s.read(|r| r.host("win", T0)).unwrap().dirs[0].check.as_deref(), Some("where cargo"), "Windows 写了单独的就用它");
    set_ready_check(&mut s, "cargo --version", "");
    assert_eq!(s.read(|r| r.host("win", T0)).unwrap().dirs[0].check.as_deref(), Some("cargo --version"), "Windows 没单独写时和所有系统一样");
    // 只能一行、有长度上限
    let e = s.write(T0, Origin::ui(None), |w| w.set_ready_check(B, Some("a\nb"), None)).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    let long = "x".repeat(READY_CHECK_MAX + 1);
    assert_eq!(s.write(T0, Origin::ui(None), |w| w.set_ready_check(B, Some(&long), None)).unwrap_err().code, ErrorCode::Usage);
    // 检查命令不存进机器的目录设置
    let raw: String = s.read(|r| Ok(r.conn.query_row("SELECT dirs FROM hosts WHERE name = 'mbp'", [], |x| x.get(0))?)).unwrap();
    assert!(!raw.contains("\"check\""), "{raw}");
}

#[test]
fn manual_dispatch_needs_a_passed_ready_check() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let id = new_card(&mut s, T0, "A");
    set_ready_check(&mut s, "cargo --version", "");
    // 还没检查（旧版 board host 不上报）：不能派
    let e = dispatch(&mut s, T0, id, "mbp", None).unwrap_err();
    assert_eq!(e.code, ErrorCode::BoardNotReady);
    assert!(e.message.contains("还没跑完"), "{}", e.message);
    // 没通过：写明输出的最后一行
    beat(&mut s, T0 + S, &ready_report("i1", "cargo --version", false, &["zsh:1: command not found: cargo", ""])).unwrap();
    let d = &host(&s, T0 + S).dirs[0];
    assert_eq!(hosts::readiness(d.check.as_deref(), d.ready.as_ref()), hosts::Readiness::Failed("zsh:1: command not found: cargo".into()));
    let e = dispatch(&mut s, T0 + S, id, "mbp", None).unwrap_err();
    assert_eq!(e.code, ErrorCode::BoardNotReady);
    assert!(e.message.contains("command not found: cargo"), "{}", e.message);
    // 后来的心跳不带结果（旧版）：保留上一次的
    beat(&mut s, T0 + 2 * S, &report("mbp", "i1", vec![])).unwrap();
    assert!(host(&s, T0 + 2 * S).dirs[0].ready.is_some());
    // 装好、重新检查通过之后恢复
    beat(&mut s, T0 + 3 * S, &ready_report("i1", "cargo --version", true, &["cargo 1.91.0"])).unwrap();
    let d = dispatch(&mut s, T0 + 3 * S, id, "mbp", None).unwrap();
    // 命令改了：旧结果不算，等重新检查；已经在排队的派发不受影响
    set_ready_check(&mut s, "cargo --version && node --version", "");
    assert_eq!(get(&s, d.id).state, "queued");
    let id2 = new_card(&mut s, T0, "B");
    assert_eq!(dispatch(&mut s, T0 + 4 * S, id2, "mbp", None).unwrap_err().code, ErrorCode::BoardNotReady);
    // 去掉检查命令：和原来一样
    set_ready_check(&mut s, "", "");
    dispatch(&mut s, T0 + 5 * S, id2, "mbp", None).unwrap();
}

/// 卡片 #232：环境检查在登录 shell 里跑，bash 没有终端时会打作业控制提示、退出时打 `logout`。
/// 这些不是命令的输出：上报时去掉，失败原因取真正的错误；zsh 的 `command not found` 这类真错误要留着
#[test]
fn ready_tail_drops_login_shell_noise_but_keeps_real_errors() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    set_ready_check(&mut s, "cargo --version", "");
    let noise = ["bash: cannot set terminal process group (-1): Inappropriate ioctl for device", "bash: no job control in this shell"];
    let with_noise: Vec<&str> = noise.iter().copied().chain(["zsh:1: command not found: cargo"]).chain(["logout"]).collect();
    beat(&mut s, T0 + S, &ready_report("i1", "cargo --version", false, &with_noise)).unwrap();
    // 上报的 tail 里没有噪声，真正的错误还在
    let d = &host(&s, T0 + S).dirs[0];
    let tail = &d.ready.as_ref().unwrap().tail;
    assert_eq!(tail, &["zsh:1: command not found: cargo"], "{tail:?}");
    assert_eq!(hosts::readiness(d.check.as_deref(), d.ready.as_ref()), hosts::Readiness::Failed("zsh:1: command not found: cargo".into()));
    // 只有噪声、没有真正的输出：不拿噪声当失败原因
    beat(&mut s, T0 + 2 * S, &ready_report("i1", "cargo --version", false, &["bash: no job control in this shell", "logout"])).unwrap();
    let d = host(&s, T0 + 2 * S).dirs[0].clone();
    assert_eq!(hosts::readiness(d.check.as_deref(), d.ready.as_ref()), hosts::Readiness::Failed("退出码 127".into()));
    // 通过时 tail 里的噪声也去掉
    beat(&mut s, T0 + 3 * S, &ready_report("i1", "cargo --version", true, &["bash: no job control in this shell", "cargo 1.91.0", "logout"])).unwrap();
    assert_eq!(host(&s, T0 + 3 * S).dirs[0].ready.as_ref().unwrap().tail, ["cargo 1.91.0"]);
    // 噪声的形状：只认固定的几行，普通输出、真正的错误不算
    for line in ["logout", " bash: no job control in this shell ", "bash: cannot set terminal process group (-1): Inappropriate ioctl for device", "stty: standard input: Inappropriate ioctl for device"] {
        assert!(hosts::shell_noise(line), "{line}");
    }
    for line in ["", "zsh:1: command not found: cargo", "logout finished", "missing ok.txt", "checking", "stty -a failed"] {
        assert!(!hosts::shell_noise(line), "{line}");
    }
}

#[test]
fn cron_skips_boards_that_failed_the_ready_check() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    lead_round(&mut s);
    let id = new_card_tagged(&mut s, "A");
    set_ready_check(&mut s, "cargo --version", "");
    beat(&mut s, T0 + S, &ready_report("i1", "cargo --version", false, &["command not found: cargo"])).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(pull(&mut s, T0 + 2 * S).is_none(), "没通过的白板不领");
    assert_eq!(card(&s, id).card.column, "todo");
    // 通过之后下一轮照常领
    beat(&mut s, T0 + 3 * S, &ready_report("i1", "cargo --version", true, &["cargo 1.91.0"])).unwrap();
    s.write(T0 + 3 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = pull(&mut s, T0 + 4 * S).unwrap();
    assert_eq!((d.card, d.state.as_str()), (id, "running"));
}

#[test]
fn old_database_boards_get_no_ready_check() {
    let dir = std::env::temp_dir().join(format!("board-ready-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("b.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("ALTER TABLE boards DROP COLUMN ready_check; ALTER TABLE boards DROP COLUMN ready_check_win").unwrap();
    }
    let s = Store::open(&path).unwrap();
    let ws = s.read(|r| r.workspace(T0)).unwrap();
    assert!(ws.projects[0].boards[0].ready_check.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/* ------------------------------ 代码仓库地址、集成分支（PRD §4） ------------------------------ */

fn ui<T>(s: &mut Store, f: impl FnOnce(&mut W) -> Result<T>) -> Result<T> {
    s.write(T0, Origin::ui(None), f)
}

#[test]
fn board_repo_and_branch_are_optional_and_validated() {
    let mut s = store();
    let board = |s: &Store| s.read(|r| r.workspace(T0)).unwrap().projects[0].boards[0].clone();
    // 新建的白板两项都是空的
    assert!(board(&s).repo.is_empty() && board(&s).branch.is_empty());
    // 去掉首尾空白；白板快照里也带着（界面用它生成链接）
    ui(&mut s, |w| w.set_repo_branch(B, Some("  git@github.com:o/r.git "), Some(" dev "))).unwrap();
    let b = board(&s);
    assert_eq!((b.repo.as_str(), b.branch.as_str()), ("git@github.com:o/r.git", "dev"));
    let snap = s.read(|r| r.resolve(B).and_then(|b| r.board_snapshot(&b, T0, false))).unwrap();
    assert_eq!((snap.repo.as_str(), snap.branch.as_str()), ("git@github.com:o/r.git", "dev"));
    // 一次只改一项：另一项不动
    ui(&mut s, |w| w.set_repo_branch(B, None, Some("release/2"))).unwrap();
    let b = board(&s);
    assert_eq!((b.repo.as_str(), b.branch.as_str()), ("git@github.com:o/r.git", "release/2"));
    // 空字符串是清空
    ui(&mut s, |w| w.set_repo_branch(B, Some(""), Some(""))).unwrap();
    assert!(board(&s).repo.is_empty() && board(&s).branch.is_empty());
    // 写不进去的：换行、超长、分支名里的空格和 git 不认的字符
    for (repo, branch) in [
        (Some("a\nb"), None),
        (Some(&"x".repeat(REPO_MAX + 1)), None),
        (None, Some("a b")),
        (None, Some("a~b")),
        (None, Some("a:b")),
        (None, Some("a?b")),
        (None, Some("a*b")),
        (None, Some("a[b")),
        (None, Some("a\\b")),
        (None, Some(&"x".repeat(BRANCH_MAX + 1))),
        (None, Some("a\nb")),
    ] {
        let e = s.write(T0, Origin::ui(None), |w| w.set_repo_branch(B, repo, branch)).unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{repo:?} {branch:?}: {}", e.message);
    }
}

#[test]
fn old_database_boards_get_no_repo_or_branch() {
    let dir = std::env::temp_dir().join(format!("board-repo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("b.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("ALTER TABLE boards DROP COLUMN repo; ALTER TABLE boards DROP COLUMN branch").unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    let b = s.read(|r| r.workspace(T0)).unwrap().projects[0].boards[0].clone();
    assert!(b.repo.is_empty() && b.branch.is_empty(), "升级前的白板两项都留空，按原来的规则");
    // 升级后照样能设
    ui(&mut s, |w| w.set_repo_branch(B, Some("git@github.com:o/r.git"), Some("dev"))).unwrap();
    let b = s.read(|r| r.workspace(T0)).unwrap().projects[0].boards[0].clone();
    assert_eq!((b.repo.as_str(), b.branch.as_str()), ("git@github.com:o/r.git", "dev"));
    let _ = std::fs::remove_dir_all(&dir);
}

/* ------------------------------ 远程管理（PRD §9.1） ------------------------------ */

fn ask(s: &mut Store, at: i64, kind: &str, now: bool) -> Result<HostInfo> {
    s.write(at, Origin::ui(None), |w| w.host_ask("mbp", kind, now, "octocat"))
}

fn acked(s: &mut Store, at: i64, instance: &str, id: i64) -> HostWork {
    let mut r = report("mbp", instance, vec![]);
    r.asked = Some(id);
    beat(s, at, &r).unwrap()
}

#[test]
fn recheck_is_handed_to_the_host_and_acked() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let h = ask(&mut s, T0 + S, "recheck", false).unwrap();
    let a = h.ask.clone().unwrap();
    assert_eq!((a.kind.as_str(), a.by.as_str()), ("recheck", "octocat"));
    assert_eq!(h.ops[0].text, "请求重新检查");
    assert_eq!(h.ops[0].by, "octocat");
    // 任务里带着请求，board host 照着做
    assert_eq!(beat(&mut s, T0 + 2 * S, &report("mbp", "i1", vec![])).unwrap().host.ask.unwrap().id, a.id);
    // 做完带着序号回报：请求清掉，记下时间和一条操作
    let w = acked(&mut s, T0 + 3 * S, "i1", a.id);
    assert!(w.host.ask.is_none());
    assert_eq!(w.host.rechecked_at, Some(T0 + 3 * S));
    assert!(w.host.ops[0].text.starts_with("重新检查完了"), "{:?}", w.host.ops);
    // 序号对不上（旧请求）不清新的
    let b = ask(&mut s, T0 + 4 * S, "recheck", false).unwrap().ask.unwrap();
    assert_eq!(b.id, a.id + 1);
    assert!(acked(&mut s, T0 + 5 * S, "i1", a.id).host.ask.is_some());
    // 离线的机器不能请求；不认识的操作拒绝
    s.sweep(T0 + 60 * S).unwrap();
    assert_eq!(ask(&mut s, T0 + 60 * S, "recheck", false).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(ask(&mut s, T0 + 60 * S, "reboot", false).unwrap_err().code, ErrorCode::Usage);
}

#[test]
fn restart_stops_running_dispatches_and_blocks_new_ones() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    let d1 = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    begin(&mut s, T0, d1.id).unwrap();
    let d2 = dispatch(&mut s, T0, b, "mbp", None).unwrap();
    let h = ask(&mut s, T0 + S, "restart", false).unwrap();
    assert!(h.ops[0].text.contains("先结束 1 个在跑的派发"), "{:?}", h.ops);
    assert_eq!(get(&s, d1.id).stop_by.as_deref(), Some("octocat"), "在跑的按「人停止」结束");
    // 等重启时不开始新的派发
    end(&mut s, T0 + 2 * S, d1.id, Some(143), &[]);
    assert_eq!(begin(&mut s, T0 + 2 * S, d2.id).unwrap_err().code, ErrorCode::Usage);
    // board host 开始重启（回报序号），之后照常
    let w = acked(&mut s, T0 + 3 * S, "i1", h.ask.unwrap().id);
    assert_eq!(w.host.ops[0].text, "正在重启 board host");
    begin(&mut s, T0 + 3 * S, d2.id).unwrap();
}

#[test]
fn upgrade_pauses_until_done_or_failed() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let h = ask(&mut s, T0 + S, "upgrade", false).unwrap();
    let a = h.ask.clone().unwrap();
    assert_eq!(a.to.as_deref(), Some(env!("CARGO_PKG_VERSION")), "升级到服务端的版本");
    assert!(h.paused, "先暂停接单");
    // board host 开始升级，再以新版本上线：恢复接单，记一条
    acked(&mut s, T0 + 2 * S, "i1", a.id);
    // 旧进程退出前发 final（机器离线），新版本的进程再上来
    beat(&mut s, T0 + 2 * S, &HostReport { done: true, ..report("mbp", "i1", vec![]) }).unwrap();
    let mut up = report("mbp", "i2", vec![]);
    up.version = env!("CARGO_PKG_VERSION").into();
    let w = beat(&mut s, T0 + 3 * S, &up).unwrap();
    assert!(!w.host.paused);
    assert_eq!(w.host.ops[0].text, format!("升级到 {}，重新上线；已恢复接单", env!("CARGO_PKG_VERSION")));
    // 已经是服务端的版本：不用升级
    assert_eq!(ask(&mut s, T0 + 4 * S, "upgrade", false).unwrap_err().code, ErrorCode::Usage);

    // 失败：回报原因，恢复接单
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = ask(&mut s, T0 + S, "upgrade", true).unwrap().ask.unwrap();
    assert!(a.now);
    acked(&mut s, T0 + 2 * S, "i1", a.id);
    let mut r = report("mbp", "i1", vec![]);
    r.ask_error = Some("升级失败：下载失败".into());
    let w = beat(&mut s, T0 + 3 * S, &r).unwrap();
    assert!(!w.host.paused);
    assert_eq!(w.host.ops[0].text, "升级失败：下载失败；已恢复接单");

    // 本来就暂停接单的机器：升级完不替人恢复；取消请求也不恢复
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { paused: Some(true), ..Default::default() })).unwrap();
    ask(&mut s, T0 + S, "upgrade", false).unwrap();
    let h = ask(&mut s, T0 + 2 * S, "cancel", false).unwrap();
    assert!(h.ask.is_none() && h.paused);
    assert_eq!(h.ops[0].text, "取消了升级请求");
}

#[test]
fn drain_pauses_and_resuming_ends_it() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let h = ask(&mut s, T0 + S, "drain", false).unwrap();
    assert!(h.paused && h.drain);
    assert!(h.ops[0].text.starts_with("排空：暂停接单"));
    let h = s.write(T0 + 2 * S, Origin::ui(None), |w| w.host_update_as("mbp", &HostPatch { paused: Some(false), ..Default::default() }, "octocat")).unwrap();
    assert!(!h.paused && !h.drain);
    assert_eq!(h.ops[0].text, "恢复接单，排空结束");
}

#[test]
fn host_token_is_shown_and_can_be_revoked_on_remove() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| {
        w.tx.execute_batch(
            "CREATE TABLE auth_tokens (id INTEGER PRIMARY KEY, name TEXT NOT NULL, hash TEXT NOT NULL, hint TEXT NOT NULL, created_at INTEGER NOT NULL, last_used_at INTEGER);
             INSERT INTO auth_tokens VALUES (1, 'win2 的令牌', 'h1', 'brd_ab12', 0, 123);",
        )?;
        Ok(())
    })
    .unwrap();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let t = host(&s, T0).token.unwrap();
    assert_eq!((t.id, t.name.as_str(), t.hint.as_str(), t.last_used_at), (1, "win2 的令牌", "brd_ab12", Some(123)));
    // 另一台在线的机器也用这个令牌：不能一起吊销
    beat(&mut s, T0, &report("win", "w1", vec![])).unwrap();
    assert_eq!(host(&s, T0).token.unwrap().shared, vec!["win".to_string()]);
    let e = s.write(T0, Origin::ui(None), |w| w.host_remove_revoke("mbp", &Actor::human(), "人", true)).unwrap_err();
    assert!(e.message.contains("win"), "{}", e.message);
    // 那台离线之后可以
    s.write(T0 + 60 * S, Origin::cli(), |w| w.host_beat(&report("mbp", "i1", vec![]), Some(1))).unwrap();
    s.sweep(T0 + 61 * S).unwrap();
    beat(&mut s, T0 + 61 * S, &report("mbp", "i1", vec![])).unwrap();
    let revoked = s.write(T0 + 61 * S, Origin::ui(None), |w| w.host_remove_revoke("mbp", &Actor::human(), "人", true)).unwrap();
    assert_eq!(revoked, Some(1));
    let left: i64 = s.read(|r| Ok(r.conn.query_row("SELECT COUNT(*) FROM auth_tokens", [], |x| x.get(0))?)).unwrap();
    assert_eq!(left, 0);
    assert!(s.read(|r| r.host("mbp", T0)).is_err());
}

#[test]
fn version_order() {
    assert!(hosts::version_lt("0.1.9", "0.1.19"));
    assert!(hosts::version_lt("v0.1.19", "0.2.0"));
    assert!(!hosts::version_lt("0.1.19", "0.1.19"));
    assert!(!hosts::version_lt("1.0.0", "0.9.9"));
}

/* ------------------------------ 模型和思考强度（PRD §9.1） ------------------------------ */

/// 新版 board host：claude 上报能选的模型和强度
fn report_opts(instance: &str) -> HostReport {
    let mut r = report("mbp", instance, vec![]);
    r.agents[0].opts = Some(AgentOpts {
        model: true,
        models: vec!["opus".into(), "sonnet".into()],
        efforts: ["low", "medium", "high", "xhigh", "max"].map(String::from).to_vec(),
    });
    r
}

fn set_run(s: &mut Store, model: Option<&str>, effort: Option<&str>) -> Result<HostInfo> {
    let run = RunPatch { agent: "claude".into(), model: model.map(str::to_string), effort: effort.map(str::to_string) };
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { run: Some(run), ..Default::default() }))
}

#[test]
fn model_and_effort_are_validated_and_shown() {
    let mut s = store();
    // 旧版 board host（不上报能选什么）：不能设
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let e = set_run(&mut s, Some("sonnet"), None).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("升级到"), "{}", e.message);
    beat(&mut s, T0 + S, &report_opts("i1")).unwrap();
    let h = set_run(&mut s, Some(" sonnet "), Some("high")).unwrap();
    let a = h.agents.iter().find(|a| a.id == "claude").unwrap();
    assert_eq!((a.run_model.as_deref(), a.run_effort.as_deref()), (Some("sonnet"), Some("high")));
    // 不支持的档位、不像模型名的值：USAGE
    assert_eq!(set_run(&mut s, None, Some("turbo")).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(set_run(&mut s, Some("son net; rm -rf"), None).unwrap_err().code, ErrorCode::Usage);
    // 只改一项另一项不动；空字符串回到默认
    let h = set_run(&mut s, None, Some("")).unwrap();
    let a = h.agents.iter().find(|a| a.id == "claude").unwrap();
    assert_eq!((a.run_model.as_deref(), a.run_effort.as_deref()), (Some("sonnet"), None));
    // 和启动环境一起存：互不覆盖
    let h = set_run(&mut s, Some(""), None).unwrap();
    assert!(h.agents.iter().all(|a| a.run_model.is_none() && a.run_effort.is_none()));
}

#[test]
fn dispatch_passes_model_and_effort_and_names_after_the_model() {
    let mut s = store();
    beat(&mut s, T0, &report_opts("i1")).unwrap();
    set_run(&mut s, Some("sonnet"), Some("high")).unwrap();
    let id = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, id, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!((d.model.as_deref(), d.effort.as_deref()), (Some("sonnet"), Some("high")));
    assert_eq!(d.name.as_deref(), Some("claude-mbp/sonnet"), "设置了模型时名字第二段用它");
    let acts = card(&s, id).activity;
    assert!(acts.iter().any(|a| a.text.contains("派发开始") && a.text.contains("模型 sonnet，强度 high")), "{acts:?}");
    end(&mut s, T0 + 2 * S, d.id, Some(0), &[]);
    // 都改回默认：名字、派发记录和原来一样
    set_run(&mut s, Some(""), Some("")).unwrap();
    let id2 = new_card(&mut s, T0, "B");
    let d2 = dispatch(&mut s, T0 + 3 * S, id2, "mbp", None).unwrap();
    let d2 = begin(&mut s, T0 + 3 * S, d2.id).unwrap();
    assert_eq!((d2.model, d2.effort, d2.name.as_deref()), (None, None, Some("claude-mbp/opus-5.5")));
}

/* ------------------------------ 会话页（PRD §10.8、§10.12） ------------------------------ */

#[test]
fn session_view_groups_a_process_and_card_history_lists_every_dispatch() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let cards: Vec<i64> = (0..3).map(|i| new_card_tagged(&mut s, &format!("卡{i}"))).collect();
    lead_round(&mut s);
    set_host(&mut s, HostPatch { per_session: Some(3), ..Default::default() }).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d1 = pull(&mut s, T0 + S).unwrap();
    let name = d1.name.clone().unwrap();
    s.write(T0 + S, Origin::cli(), |w| w.dispatch_log_append(d1.id, &["第一张的输出".to_string()])).unwrap();
    // 第一张还在跑：会话里排着后面两张
    let v = s.read(|r| r.session(d1.id)).unwrap();
    assert_eq!(v.session, d1.id);
    assert_eq!(v.parts.iter().map(|p| (p.dispatch.card, p.dispatch.state.as_str())).collect::<Vec<_>>(), vec![(cards[0], "running"), (cards[1], "queued"), (cards[2], "queued")]);
    assert_eq!(v.parts[0].log, vec!["第一张的输出".to_string()]);
    // 接着做完三张：从任一张的派发都能打开整个会话，按先后
    let mut t = T0 + 2 * S;
    for _ in 0..2 {
        let cur = s.read(|r| r.active_dispatches()).unwrap().into_iter().find(|d| d.state == "running").unwrap();
        submit(&mut s, t, &name, cur.card);
        next(&mut s, t, &name).unwrap();
        t += S;
    }
    let last = s.read(|r| r.active_dispatches()).unwrap().into_iter().find(|d| d.state == "running").unwrap();
    s.write(t, Origin::cli(), |w| w.dispatch_log_append(last.id, &["第三张的输出".to_string()])).unwrap();
    let v = s.read(|r| r.session(last.id)).unwrap();
    assert_eq!(v.session, d1.id);
    assert_eq!(v.parts.iter().map(|p| p.dispatch.card).collect::<Vec<_>>(), cards);
    assert_eq!(v.parts[2].log, vec!["第三张的输出".to_string()]);
    assert_eq!(s.read(|r| r.session(9999)).unwrap_err().code, ErrorCode::NotFound);
    // 一张卡做过几次：每次一条，新的在前
    end(&mut s, t, last.id, Some(1), &[]);
    let list = s.read(|r| {
        let b = r.resolve(B)?;
        r.card_dispatches(&b, cards[0])
    })
    .unwrap();
    assert_eq!(list.iter().map(|x| x.id).collect::<Vec<_>>(), vec![d1.id]);
    assert_eq!(list[0].result.as_deref(), Some("review"));
}

/* ------------------------------ 兜底：超时、全部暂停、全部停止 ------------------------------ */

/// #198 验收 1：到时间后 board host 按「人停止」的流程结束进程树、上报超时；结果「超时」，卡片在错误节点，评论写明原因。
/// 超时也计入大盘「按 agent」的异常退出
#[test]
fn timeout_dispatch_records_timeout_and_moves_card_to_error_status() {
    let mut s = store();
    let key = add_error_status(&mut s);
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!(d.state, "running");
    // board host 到时间后结束进程树，上报 timeout 和原因
    let done = s
        .write(T0 + 3 * S, Origin::cli(), |w| w.dispatch_end(d.id, Some(137), &["sleeping".into()], EndHow::Timeout, Some("运行超过 30 分钟，board host 停止了它")))
        .unwrap();
    assert_eq!((done.state.as_str(), done.result.as_deref()), ("done", Some("timeout")));
    assert_eq!(card(&s, a).card.column, key, "超时按异常退出的规则移到错误节点");
    let texts: Vec<String> = card(&s, a).activity.into_iter().map(|x| x.text).collect();
    assert!(texts.iter().any(|t| t.contains("运行超过 30 分钟，board host 停止了它")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("sleeping")), "评论里附上日志最后几行：{texts:?}");
    assert!(texts.iter().any(|t| t.contains("派发结束：超时")), "{texts:?}");
    // 这一次记成 how = timeout：卡片上的异常退出次数 +1
    assert_eq!(card(&s, a).card.runs_lost, 1);
    assert_eq!(card(&s, a).card.holder, None);
}

/// 没有错误节点时，超时去需要人处理的状态（和异常退出一样）
#[test]
fn timeout_dispatch_goes_to_human_status_without_error_status() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    begin(&mut s, T0 + S, d.id).unwrap();
    let d = s
        .write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_end(1, None, &[], EndHow::Timeout, Some("20 分钟没有任何输出，board host 停止了它")))
        .unwrap();
    assert_eq!(d.result.as_deref(), Some("timeout"));
    assert_eq!(card(&s, a).card.column, "check");
    assert!(card(&s, a).activity.iter().any(|x| x.text.contains("20 分钟没有任何输出")));
}

/// 一台机器的兜底设置：默认 3 小时、20 分钟；改了什么范围都拒绝；改了马上能读到（board host 每次上报都拿到最新值）
#[test]
fn host_run_limits_default_and_validation() {
    let mut s = store();
    let w = beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!((w.host.max_run, w.host.idle_run), (MAX_RUN_DEFAULT, IDLE_RUN_DEFAULT));
    let h = s.write(T0, Origin::ui(None), |w| {
        w.host_update("mbp", &HostPatch { max_run: Some(30), idle_run: Some(5), ..Default::default() })
    })
    .unwrap();
    assert_eq!((h.max_run, h.idle_run), (30, 5));
    // 下一次上报（board host 每 10 秒一次）就拿到新值，不用重启
    let w = beat(&mut s, T0 + S, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!((w.host.max_run, w.host.idle_run), (30, 5));
    // 0 = 不限
    let h = s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max_run: Some(0), ..Default::default() })).unwrap();
    assert_eq!(h.max_run, 0);
    // 超出范围拒绝
    for p in [HostPatch { max_run: Some(-1), ..Default::default() }, HostPatch { idle_run: Some(IDLE_RUN_LIMIT + 1), ..Default::default() }] {
        assert_eq!(s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &p)).unwrap_err().code, ErrorCode::Usage);
    }
}

/// 验收 4：全部暂停把所有机器设成暂停、排队的照样排队；全部恢复后恢复接单。
/// 全部停止：所有在跑的派发收到停止请求，同时全部暂停
#[test]
fn pause_all_and_stop_all_touch_every_host() {
    let mut s = store();
    for (name, inst) in [("mbp", "i1"), ("win", "w1")] {
        let mut rep = report(name, inst, vec![]);
        rep.dirs = vec![HostDir { board: B.into(), path: "/tmp/board".into(), ..Default::default() }];
        beat(&mut s, T0, &rep).unwrap();
        s.write(T0, Origin::ui(None), |w| w.host_update(name, &HostPatch { max: Some(2), ..Default::default() })).unwrap();
    }
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    let c = new_card(&mut s, T0, "C");
    let d1 = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d1 = begin(&mut s, T0 + S, d1.id).unwrap();
    let d2 = dispatch(&mut s, T0, b, "win", None).unwrap();
    let d2 = s.write(T0 + S, Origin::cli(), |w| w.dispatch_begin("win", "w1", d2.id, Some(1))).unwrap();
    assert_eq!((d1.state.as_str(), d2.state.as_str()), ("running", "running"));
    // 还有一张排队（mbp 满 2 个之前先再派一张）：两个都跑着，排队
    let d3 = dispatch(&mut s, T0, c, "mbp", None).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(1), ..Default::default() })).unwrap();
    assert_eq!(d3.state, "queued");

    // 全部暂停：两台都暂停，排队的还在排队
    let all = s.write(T0 + 2 * S, Origin::ui(None), |w| w.hosts_pause_all(true)).unwrap();
    assert_eq!(all.iter().map(|h| (h.name.as_str(), h.paused)).collect::<Vec<_>>(), vec![("mbp", true), ("win", true)]);
    assert_eq!(get(&s, d3.id).state, "queued", "排队的派发继续排队");
    assert_eq!((get(&s, d1.id).state.as_str(), get(&s, d2.id).state.as_str()), ("running", "running"), "在跑的不受影响");

    // 全部恢复
    let all = s.write(T0 + 3 * S, Origin::ui(None), |w| w.hosts_pause_all(false)).unwrap();
    assert!(all.iter().all(|h| !h.paused));

    // 全部停止：两台机器上在跑的派发都收到停止请求，同时全部暂停
    let all = s.write(T0 + 4 * S, Origin::ui(None), |w| w.hosts_stop_all(&Actor::human(), "buglan")).unwrap();
    assert!(all.iter().all(|h| h.paused), "全部停止之后全部暂停");
    let stop = |id: i64| get(&s, id).stop_by.clone();
    assert_eq!((stop(d1.id).as_deref(), stop(d2.id).as_deref()), (Some("buglan"), Some("buglan")));
    assert_eq!(get(&s, d3.id).state, "queued", "排队的不在停止之列");
}

/* ------------------------------ 自检（PRD §10.12「自检」） ------------------------------ */

fn selftest(s: &mut Store, at: i64, agent: Option<&str>) -> Result<HostInfo> {
    s.write(at, Origin::ui(None), |w| w.host_selftest("mbp", agent, "octocat"))
}

fn selftest_done(s: &mut Store, at: i64, instance: &str, id: i64, items: Vec<SelfTestItem>) -> HostInfo {
    s.write(at, Origin::cli(), |w| w.host_selftest_done("mbp", instance, id, items)).unwrap()
}

fn item(kind: &str, key: &str, ok: bool, why: Option<&str>) -> SelfTestItem {
    SelfTestItem { kind: kind.into(), key: key.into(), ok, why: why.map(str::to_string), detail: None, at: 0 }
}

#[test]
fn selftest_request_runs_once_and_results_replace_previous() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    // 只能检查找得到的 agent
    assert_eq!(selftest(&mut s, T0, Some("pi")).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(selftest(&mut s, T0, Some("codex")).unwrap_err().code, ErrorCode::Usage);
    let h = selftest(&mut s, T0 + S, None).unwrap();
    let ask = h.selftest.ask.clone().expect("记下请求");
    assert_eq!((ask.agent.as_deref(), ask.by.as_str()), (None, "octocat"));
    // 取任务时 board host 能看到请求
    let w = s.read(|r| r.host_work("mbp", T0 + 2 * S)).unwrap();
    assert_eq!(w.host.selftest.ask.as_ref().map(|a| a.id), Some(ask.id));
    // 同一台机器同时只做一次
    assert_eq!(selftest(&mut s, T0 + 2 * S, Some("claude")).unwrap_err().code, ErrorCode::Usage);
    // 别的进程、序号对不上的回报不算
    let h = selftest_done(&mut s, T0 + 3 * S, "i2", ask.id, vec![item("agent", "claude", true, None)]);
    assert!(h.selftest.ask.is_some() && h.selftest.items.is_empty());
    let h = selftest_done(&mut s, T0 + 3 * S, "i1", ask.id + 5, vec![item("agent", "claude", true, None)]);
    assert!(h.selftest.ask.is_some());
    // 做完：agent、git 两项 + 服务端补上版本（0.1.1 落后于服务端）
    let h = selftest_done(
        &mut s,
        T0 + 4 * S,
        "i1",
        ask.id,
        vec![item("agent", "claude", true, None), item("git", B, false, Some("没有推送权限")), item("bogus", "x", true, None)],
    );
    assert!(h.selftest.ask.is_none());
    let kinds: Vec<(&str, bool)> = h.selftest.items.iter().map(|i| (i.kind.as_str(), i.ok)).collect();
    assert_eq!(kinds, vec![("agent", true), ("git", false), ("version", false)]);
    assert!(h.selftest.items.iter().all(|i| i.at == T0 + 4 * S), "检查时间是服务端收到的时间");
    let v = h.selftest.items.iter().find(|i| i.kind == "version").unwrap();
    assert!(v.why.as_deref().unwrap().contains("0.1.1") && v.why.as_deref().unwrap().contains(env!("CARGO_PKG_VERSION")));
    assert!(h.ops.iter().any(|o| o.text.contains("没通过") && o.text.contains(B)), "{:?}", h.ops);
    // 只查一个 agent：只换它那一项，git 的结果保留到下一次整机自检
    let ask = selftest(&mut s, T0 + 5 * S, Some("claude")).unwrap().selftest.ask.unwrap();
    assert_eq!(ask.agent.as_deref(), Some("claude"));
    let h = selftest_done(&mut s, T0 + 6 * S, "i1", ask.id, vec![item("agent", "claude", false, Some("没有登录")), item("git", B, true, None)]);
    let got: Vec<(&str, bool, i64)> = h.selftest.items.iter().map(|i| (i.kind.as_str(), i.ok, i.at)).collect();
    assert_eq!(got, vec![("agent", false, T0 + 6 * S), ("git", false, T0 + 4 * S), ("version", false, T0 + 6 * S)]);
}

#[test]
fn selftest_needs_online_host_and_stale_request_can_be_replaced() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let first = selftest(&mut s, T0, None).unwrap().selftest.ask.unwrap();
    // 一直没做完（board host 卡住、重启丢了）：超过期限后可以重新请求
    beat(&mut s, T0 + hosts::SELFTEST_STALE_MS, &report("mbp", "i1", vec![])).unwrap();
    let again = selftest(&mut s, T0 + hosts::SELFTEST_STALE_MS + S, None).unwrap().selftest.ask.unwrap();
    assert!(again.id > first.id);
    // 离线的机器不能自检
    s.sweep(T0 + hosts::SELFTEST_STALE_MS + 120 * S).unwrap();
    assert_eq!(selftest(&mut s, T0 + hosts::SELFTEST_STALE_MS + 121 * S, None).unwrap_err().code, ErrorCode::Usage);
    // 旧版 board host（上报里没有 selftest）：不会自检，请求被拒绝；换回新版又可以
    let t = T0 + hosts::SELFTEST_STALE_MS + 130 * S;
    let mut old = report("mbp", "i9", vec![]);
    old.selftest = false;
    assert!(!beat(&mut s, t, &old).unwrap().host.selftest.able);
    let e = selftest(&mut s, t + S, None).unwrap_err();
    assert!(e.message.contains("还不会自检"), "{}", e.message);
    assert!(beat(&mut s, t + 2 * S, &report("mbp", "i9", vec![])).unwrap().host.selftest.able);
}

/* ------------------------------ 结构化事件（runlog.rs，#184） ------------------------------ */

fn ev(kind: &str, text: &str) -> RunEvent {
    RunEvent::text(kind, text)
}

fn tool(call: &str, name: &str, args: &str) -> RunEvent {
    RunEvent { kind: "tool".into(), call: Some(call.into()), tool: Some(name.into()), args: Some(args.into()), ..Default::default() }
}

fn tool_done(call: &str, out: &str, ok: bool) -> RunEvent {
    RunEvent { kind: "tool".into(), call: Some(call.into()), out: Some(out.into()), ok: Some(ok), ..Default::default() }
}

fn push_events(s: &mut Store, at: i64, id: i64, fmt: Option<&str>, events: &[RunEvent]) -> usize {
    s.write(at, Origin::cli(), |w| w.dispatch_events_append(id, fmt, events)).unwrap()
}

fn running_dispatch(s: &mut Store) -> (i64, Dispatch) {
    beat(s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(s, T0, "A");
    let d = dispatch(s, T0, a, "mbp", None).unwrap();
    let d = begin(s, T0 + S, d.id).unwrap();
    (a, d)
}

#[test]
fn events_are_saved_per_dispatch_and_a_tool_result_joins_its_call() {
    let mut s = store();
    let (_, d) = running_dispatch(&mut s);
    let rev0 = s.read(|r| r.runs_rev()).unwrap();
    // 第一批：一句话、一个还没结果的工具调用
    push_events(&mut s, T0 + 2 * S, d.id, Some("claude"), &[ev("message", "先领卡"), tool("t1", "Bash", "cargo test")]);
    let v = s.read(|r| r.session(d.id)).unwrap();
    let p = &v.parts[0];
    assert_eq!(p.fmt.as_deref(), Some("claude"));
    assert_eq!(p.events.len(), 2);
    assert_eq!(p.events[0].at, T0 + 2 * S, "没带时间的用服务端时间");
    assert!(p.events[1].ok.is_none() && p.events[1].call.is_none(), "调用标识不给界面");
    let since = v.runs;
    assert!(since > rev0, "有事件时 runs 加一");
    // 第二批：结果合进同一条，不另占一行；增量取只拿到新增、改过的
    push_events(&mut s, T0 + 4 * S, d.id, Some("claude"), &[tool_done("t1", "test result: ok", true), ev("result", "完成")]);
    let all = s.read(|r| r.session(d.id)).unwrap().parts[0].events.clone();
    assert_eq!(all.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["message", "tool", "result"]);
    assert_eq!((all[1].tool.as_deref(), all[1].out.as_deref(), all[1].ok), (Some("Bash"), Some("test result: ok"), Some(true)));
    let inc = s.read(|r| r.session_since(d.id, since)).unwrap();
    assert_eq!(inc.parts[0].events.iter().map(|e| e.id).collect::<Vec<_>>(), [all[1].id, all[2].id], "增量：改过的工具调用和新的结果");
    assert_eq!(inc.parts[0].events_from, all[0].id);
    // 推送里带上有新事件的派发
    assert_eq!(s.read(|r| r.dispatches_with_events_since(since)).unwrap(), [d.id]);
    // 不是变更：不推整个工作区
    let rev = s.rev().unwrap();
    push_events(&mut s, T0 + 5 * S, d.id, None, &[ev("text", "warning: x")]);
    assert_eq!(s.rev().unwrap(), rev);
    // 不存在的派发
    let e = s.write(T0, Origin::cli(), |w| w.dispatch_events_append(9999, None, &[ev("text", "x")])).unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[test]
fn only_the_newest_2000_events_are_kept_and_big_ones_are_cut() {
    let mut s = store();
    let (_, d) = running_dispatch(&mut s);
    let batch: Vec<RunEvent> = (0..1500).map(|i| ev("text", &format!("第 {i} 行"))).collect();
    push_events(&mut s, T0 + 2 * S, d.id, Some("dsh"), &batch);
    let batch: Vec<RunEvent> = (1500..2100).map(|i| ev("text", &format!("第 {i} 行"))).collect();
    push_events(&mut s, T0 + 3 * S, d.id, None, &batch);
    let p = s.read(|r| r.session(d.id)).unwrap().parts.remove(0);
    assert_eq!(p.events.len(), 2000);
    assert_eq!(p.events_dropped, 100);
    assert_eq!(p.events[0].text.as_deref(), Some("第 100 行"), "删的是最早的");
    assert_eq!(p.events_from, p.events[0].id);
    assert_eq!(p.fmt.as_deref(), Some("dsh"), "没带格式时不改");
    // 单条超过 4 KB：截断并标明
    push_events(&mut s, T0 + 4 * S, d.id, None, &[ev("message", &"长".repeat(3000))]);
    let p = s.read(|r| r.session(d.id)).unwrap().parts.remove(0);
    let last = p.events.last().unwrap();
    assert!(last.cut);
    assert!(serde_json::to_string(last).unwrap().len() <= runlog::EVENT_MAX + 64);
    // 只有原始输出的 agent：格式是空字符串
    let (_, d2) = {
        let b = new_card(&mut s, T0, "B");
        let d2 = dispatch(&mut s, T0, b, "mbp", None).unwrap();
        (b, d2)
    };
    push_events(&mut s, T0 + 5 * S, d2.id, Some(""), &[]);
    assert_eq!(s.read(|r| r.session(d2.id)).unwrap().parts[0].fmt.as_deref(), Some(""));
}

#[test]
fn events_older_than_30_days_keep_results_and_the_last_50() {
    let mut s = store();
    let (_, d) = running_dispatch(&mut s);
    let mut batch: Vec<RunEvent> = vec![ev("result", "第一张的结果")];
    batch.extend((0..200).map(|i| ev("text", &format!("第 {i} 行"))));
    push_events(&mut s, T0 + 2 * S, d.id, Some("claude"), &batch);
    end(&mut s, T0 + 3 * S, d.id, Some(0), &[]);
    // 不到 30 天：不动
    let n = s.write(T0 + 29 * 24 * 3600 * S, Origin::system(), |w| w.prune_run_events()).unwrap();
    assert_eq!(n, 0);
    let n = s.write(T0 + 31 * 24 * 3600 * S, Origin::system(), |w| w.prune_run_events()).unwrap();
    assert_eq!(n, 150);
    let p = s.read(|r| r.session(d.id)).unwrap().parts.remove(0);
    assert_eq!(p.events.len(), 51);
    assert_eq!(p.events[0].kind, "result", "结果留着");
    assert_eq!(p.events[1].text.as_deref(), Some("第 150 行"));
    assert_eq!(p.events_dropped, 150);
    // 只清一次
    assert_eq!(s.write(T0 + 32 * 24 * 3600 * S, Origin::system(), |w| w.prune_run_events()).unwrap(), 0);
}

#[test]
fn crash_comment_shows_the_last_events_and_folds_the_raw_output() {
    let mut s = store();
    let (a, d) = running_dispatch(&mut s);
    let tail: Vec<String> = vec!["raw 1".into(), "raw 2".into()];
    let events: Vec<String> = vec!["▸ Bash  cargo test  ✗ test failed（退出码 101）".into(), "agent：测试没过，我看看。".into()];
    s.write(T0 + 9 * S, Origin::cli(), |w| w.dispatch_end_with(d.id, Some(1), &tail, &events, EndHow::Exit, None)).unwrap();
    let c = card(&s, a);
    let t = &c.activity.iter().rev().find(|x| x.kind == "comment").unwrap().text;
    assert!(t.starts_with("agent 异常退出（退出码 1）"), "{t}");
    assert!(t.contains("最后 2 个事件：\n\n```\n▸ Bash  cargo test  ✗ test failed（退出码 101）\nagent：测试没过，我看看。\n```"), "{t}");
    assert!(t.contains("<details><summary>原始输出最后 2 行</summary>\n\n```\nraw 1\nraw 2\n```\n\n</details>"), "{t}");
    assert!(!t.contains("日志最后"), "{t}");
}
