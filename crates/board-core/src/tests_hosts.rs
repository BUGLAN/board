//! 任务下发（board host）的规则：机器上报、手动派发的校验、开始、结束、占用、掉线、定时领取、移除（任务下发需求 §十二）。

use crate::hosts::{EndHow, AUTO_UPGRADE_RETRY_MS, AUTO_UPGRADE_TRIES, IDLE_RUN_DEFAULT, IDLE_RUN_LIMIT, MAX_RUN_DEFAULT, NO_SELF_UPDATE};
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

fn managed(version: &str, latest: &str, state: &str) -> HostAgent {
    HostAgent {
        id: "claude".into(),
        name: "Claude Code".into(),
        version: version.into(),
        found: true,
        manage: Some(AgentManage {
            method: "npm-global".into(),
            method_label: "npm 全局".into(),
            latest_version: latest.into(),
            update_available: crate::hosts::version_lt(version, latest),
            updatable: true,
            state: state.into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 一台机器的上报：工作空间 `/tmp`（每个项目的目录是 `/tmp/<项目>`，项目下的白板共用），`dirs` 是它派生、检查过的白板目录。
/// 旧版 board host（只报白板与目录、没有 workspace）用 [`legacy_report`]
fn report(name: &str, instance: &str, running: Vec<i64>) -> HostReport {
    HostReport {
        name: name.into(),
        instance: instance.into(),
        resume: None,
        os: "macOS 15.4".into(),
        version: "0.1.1".into(),
        agents: vec![claude(true), HostAgent { id: "mycli".into(), name: "mycli".into(), version: String::new(), found: false, ..Default::default() }],
        envs: vec![],
        workspace: Some(HostWorkspace { path: "/tmp".into(), ..Default::default() }),
        dirs: vec![HostDir { board: B.into(), path: "/tmp/default".into(), ..Default::default() }],
        dirs_rev: 0,
        running,
        done: false,
        asked: None,
        ask_error: None,
        selftest: true,
        config_sync: true,
        local_skills: None,
        res: None,
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
    exec(s, at, &Actor::human(), Cmd::New { title: title.into(), body: None, column: None, top: false, labels: vec![], from: None }).unwrap().card.id
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
    assert!(h.agents.iter().any(|a| a.id == "mycli" && !a.found), "没找到的 agent 也上报");
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
fn host_workspace_is_server_authoritative_and_keep_reported_checks() {
    let mut s = store();
    let w = beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!(w.host.workspace.as_ref().map(|x| x.path.as_str()), Some("/tmp"));
    assert_eq!((w.host.dirs_rev, w.host.dirs[0].path.as_str()), (1, "/tmp/default"), "目录由工作空间派生：<工作空间>/<项目>");

    // 在别的设备上把工作空间改掉：服务端升版本，同一块白板的目录跟着换
    let h = s
        .write(T0 + S, Origin::ui(None), |w| w.host_update_as("mbp", &HostPatch { workspace: Some(HostWorkspace { path: "/tmp/new".into(), ..Default::default() }), ..Default::default() }, "buglan"))
        .unwrap();
    assert_eq!((h.dirs_rev, h.dirs[0].path.as_str(), h.dirs_changed_by.as_deref()), (2, "/tmp/new/default", Some("buglan")));

    // 旧版本（还是版本 1 的本机配置）上报的检查结果不挂在新的目录上
    let mut stale = report("mbp", "i1", vec![]);
    stale.dirs[0].status = Some("missing".into());
    stale.dirs[0].checked_at = Some(T0 + 2 * S);
    let w = beat(&mut s, T0 + 2 * S, &stale).unwrap();
    assert_eq!(w.host.dirs[0].path, "/tmp/new/default", "本机上报的是旧的工作空间，不能覆盖服务端设置");

    // 应用了服务端版本后，同一块白板的检查结果合进来
    let mut checked = report("mbp", "i1", vec![]);
    checked.dirs_rev = 2;
    checked.workspace = Some(HostWorkspace { path: "/tmp/new".into(), ..Default::default() });
    checked.dirs[0] = HostDir {
        board: B.into(),
        path: "/tmp/new/default".into(),
        status: Some("ok".into()),
        origin: Some("git@github.com:buglan/board.git".into()),
        branch: Some("main".into()),
        checked_at: Some(T0 + 3 * S),
        ..Default::default()
    };
    let w = beat(&mut s, T0 + 3 * S, &checked).unwrap();
    assert_eq!(w.host.dirs[0].status.as_deref(), Some("ok"));
    assert_eq!(w.host.dirs[0].origin.as_deref(), Some("git@github.com:buglan/board.git"));
    assert_eq!(w.host.dirs[0].branch.as_deref(), Some("main"));
    assert_eq!(w.host.dirs_rev, 2);

    // 本机配置先改了（版本还是 2）：服务端收下，记成「本机」改的
    let mut local = checked;
    local.workspace = Some(HostWorkspace { path: "/tmp/local".into(), ..Default::default() });
    local.dirs[0].path = "/tmp/local/default".into();
    let w = beat(&mut s, T0 + 4 * S, &local).unwrap();
    assert_eq!(
        (w.host.dirs_rev, w.host.workspace.as_ref().map(|x| x.path.as_str()), w.host.dirs[0].path.as_str(), w.host.dirs_changed_by.as_deref()),
        (3, Some("/tmp/local"), "/tmp/local/default", Some("本机"))
    );
}

/// 一台只报白板与目录的旧版 board host（`--dir` 写的白板标识）：升级前照样能派卡
fn legacy_report(name: &str, instance: &str) -> HostReport {
    let mut r = report(name, instance, vec![]);
    r.workspace = None;
    r.dirs = vec![
        HostDir { board: "main".into(), path: "/tmp/board".into(), fixed: true, ..Default::default() },
        HostDir { board: "other".into(), path: "/elsewhere/repo".into(), fixed: true, ..Default::default() },
    ];
    r
}


#[test]
fn legacy_host_dirs_still_work_and_upgrade_to_a_workspace() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None)).unwrap();
    // 旧版 board host 上报白板标识、不报工作空间：导出不了工作空间，但存着的目录照样让它派卡
    let w = beat(&mut s, T0, &legacy_report("mbp", "i1")).unwrap();
    assert!(w.host.workspace.as_ref().is_none_or(|x| x.path.is_empty()), "白板标识不是路径，导出不了工作空间");
    let a = new_card(&mut s, T0, "A");
    assert!(dispatch(&mut s, T0, a, "mbp", None).is_ok(), "旧版按白板标识存着目录，照样能派");

    // 老数据（升级前逐块配的 `[host] dirs`）：重新连上时导出成工作空间，几块白板取共同的上两级
    let mut s2 = store();
    s2.write(T0, Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None)).unwrap();
    let mut rep = report("old", "i1", vec![]);
    rep.workspace = None;
    rep.dirs = vec![
        HostDir { board: B.into(), path: "/repo/default/main".into(), ..Default::default() },
        HostDir { board: "default/other".into(), path: "/repo/default/other".into(), ..Default::default() },
    ];
    let w = beat(&mut s2, T0, &rep).unwrap();
    assert_eq!(w.host.workspace.as_ref().map(|x| x.path.as_str()), Some("/repo"));
    assert_eq!(w.host.dirs.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(), vec!["/repo/default/main", "/repo/default/other"]);
    // 旧的逐块目录（`<工作空间>/<项目>/<白板>`）不是现在的 `<工作空间>/<项目>`：留成单独覆盖，仓库还在原处，照样能接卡
    assert!(w.host.dirs.iter().all(|d| d.fixed));
    // 服务端按导出的工作空间记着：之后用旧版 board host 继续上报也不丢
    let w = beat(&mut s2, T0 + S, &rep).unwrap();
    assert_eq!(w.host.workspace.as_ref().map(|x| x.path.as_str()), Some("/repo"));

    // 服务端已经按工作空间记着的机器（新版）：上报的白板标识不合规则时拒绝，不改存着的目录
    let mut s3 = store();
    beat(&mut s3, T0, &report("mbp", "i1", vec![])).unwrap();
    let mut rep2 = report("mbp", "i1", vec![]);
    rep2.workspace = None;
    rep2.dirs = vec![HostDir { board: "main".into(), path: "/tmp/board".into(), ..Default::default() }];
    let e = beat(&mut s3, T0 + S, &rep2).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("<项目>/<白板>"), "{}", e.message);
    assert_eq!(s3.read(|r| r.host("mbp", T0 + S)).unwrap().workspace.as_ref().map(|w| w.path.as_str()), Some("/tmp"), "拒绝的上报不改存着的目录");
}

#[test]
fn workspace_keeps_dir_overrides_and_rejects_their_removal() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None)).unwrap();
    let mut rep = report("mbp", "i1", vec![]);
    rep.workspace = Some(HostWorkspace {
        path: "/tmp".into(),
        created: false,
        overrides: vec![HostDirOverride { board: "default/other".into(), path: "/elsewhere/repo".into() }],
        ..Default::default()
    });
    beat(&mut s, T0, &rep).unwrap();
    let h = host(&s, T0);
    assert_eq!(
        h.dirs.iter().map(|d| (d.board.as_str(), d.path.as_str(), d.fixed)).collect::<Vec<_>>(),
        vec![(B, "/tmp/default", false), ("default/other", "/elsewhere/repo", true)]
    );

    // --dir 覆盖的目录不能从界面上改掉（没带上的补回去，带了别的路径就拒绝）
    let h = s
        .write(T0 + S, Origin::ui(None), |w| {
            w.host_update("mbp", &HostPatch { workspace: Some(HostWorkspace { path: "/tmp".into(), ..Default::default() }), ..Default::default() })
        })
        .unwrap();
    assert_eq!(h.workspace.as_ref().map(|w| w.overrides.clone()).unwrap_or_default(), vec![HostDirOverride { board: "default/other".into(), path: "/elsewhere/repo".into() }]);
    let e = s
        .write(T0 + S, Origin::ui(None), |w| {
            let p = HostPatch {
                workspace: Some(HostWorkspace {
                    path: "/tmp".into(),
                    created: false,
                    overrides: vec![HostDirOverride { board: "default/other".into(), path: "/tmp/other".into() }],
                    ..Default::default()
                }),
                ..Default::default()
            };
            w.host_update("mbp", &p)
        })
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("--dir"), "{}", e.message);

    // 带上覆盖的照常改：覆盖的路径不变，其他白板的目录跟着工作空间换
    let h = s
        .write(T0 + S, Origin::ui(None), |w| {
            w.host_update(
                "mbp",
                &HostPatch {
                    workspace: Some(HostWorkspace {
                        path: "/tmp/new".into(),
                        created: false,
                        overrides: vec![HostDirOverride { board: "default/other".into(), path: "/elsewhere/repo".into() }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    assert_eq!(h.dirs.iter().map(|d| (d.board.as_str(), d.path.as_str())).collect::<Vec<_>>(), vec![
        (B, "/tmp/new/default"),
        ("default/other", "/elsewhere/repo")
    ]);
    assert_eq!(h.dirs_changed_by.as_deref(), Some("人"));

    // 不配工作空间：清掉，派发时按「还没有配工作空间」拒绝
    let h = s.write(T0 + 2 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { workspace: Some(HostWorkspace::default()), ..Default::default() })).unwrap();
    assert!(h.workspace.is_none() && h.dirs.is_empty());
    let a = new_card(&mut s, T0, "A");
    assert_eq!(dispatch(&mut s, T0, a, "mbp", None).unwrap_err().code, ErrorCode::BoardNotOnHost);
}

#[test]
fn board_dir_uses_the_machines_separator() {
    // 一个项目一个目录：项目下的几块白板共用 `<工作空间>/<项目>`
    assert_eq!(hosts::board_dir("/repo", "default/main", "macOS 15.4").as_deref(), Some("/repo/default"));
    assert_eq!(hosts::board_dir("/repo", "default/other", "macOS 15.4").as_deref(), Some("/repo/default"));
    assert_eq!(hosts::board_dir("~/repo/", "default/main", "Linux").as_deref(), Some("~/repo/default"));
    assert_eq!(hosts::board_dir("C:\\repo\\", "default/main", "Windows 11 Pro").as_deref(), Some("C:\\repo\\default"));
    assert_eq!(hosts::board_dir("", "default/main", "macOS").as_deref(), None);
    assert_eq!(hosts::board_dir("/repo", "nope", "macOS").as_deref(), None);
}

/// 项目目录（#488）：项目设置里填了的用 `<工作空间>/<项目目录>`，没填（或空白）用项目标识；`--dir` 覆盖的优先；分隔符按机器
#[test]
fn derive_dir_uses_the_project_dir() {
    let ws = |projects: &[(&str, &str)]| HostWorkspace {
        path: "/repo".into(),
        projects: projects.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        ..Default::default()
    };
    let path = |w: &HostWorkspace, board: &str, os: &str| hosts::derive_dir(w, board, os).map(|d| d.path);
    assert_eq!(path(&ws(&[]), "default/main", "macOS").as_deref(), Some("/repo/default"), "没填：项目标识");
    assert_eq!(path(&ws(&[("default", "board")]), "default/main", "macOS").as_deref(), Some("/repo/board"));
    assert_eq!(path(&ws(&[("default", "board")]), "default/other", "macOS").as_deref(), Some("/repo/board"), "项目下的白板共用");
    assert_eq!(path(&ws(&[("default", "board")]), "site/main", "macOS").as_deref(), Some("/repo/site"), "别的项目不受影响");
    assert_eq!(path(&ws(&[("default", "  ")]), "default/main", "macOS").as_deref(), Some("/repo/default"), "空白当没填");
    let win = HostWorkspace { path: "C:\\work\\".into(), ..ws(&[("default", "board")]) };
    assert_eq!(path(&win, "default/main", "Windows 11 Pro").as_deref(), Some("C:\\work\\board"));
    // `--dir` 覆盖的优先
    let fixed = HostWorkspace { overrides: vec![HostDirOverride { board: "default".into(), path: "/elsewhere".into() }], ..ws(&[("default", "board")]) };
    let d = hosts::derive_dir(&fixed, "default/main", "macOS").unwrap();
    assert_eq!((d.path.as_str(), d.fixed), ("/elsewhere", true));
    assert_eq!(path(&ws(&[("default", "board")]), "nope", "macOS"), None);
    assert_eq!(hosts::project_dir("", "board", "macOS"), None);
}

/// 项目目录只能是一个合法的目录名
#[test]
fn project_dir_must_be_one_directory_name() {
    for ok in ["", "  ", "board", "my.repo-2", "仓库"] {
        assert!(crate::store::check_project_dir(ok).is_ok(), "{ok}");
    }
    assert_eq!(crate::store::check_project_dir("  board ").unwrap(), "board");
    for bad in ["a/b", "a\\b", ".", "..", "C:", "C:\\repo", "~repo", "a|b", "a?b", "a*", "a<b", "a\"b", "repo.", "a\u{1}b", &"x".repeat(65)] {
        let e = crate::store::check_project_dir(bad).unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{bad}");
    }
}

/// 项目设置里改项目目录（#488）：工作区里带上，机器的目录跟着派生（旧路径的检查结果不再算），
/// 发给 board host 的工作空间带上项目目录、存着的不带；board host 按新路径报上来的检查结果合得上；
/// 不同项目不能用同一个目录；清空回到项目标识
#[test]
fn project_dir_moves_the_machines_directory() {
    let mut s = store();
    let mut rep = report("mbp", "i1", vec![]);
    rep.dirs[0].status = Some("not_git".into());
    rep.dirs[0].checked_at = Some(T0);
    beat(&mut s, T0, &rep).unwrap();
    let h = s.read(|r| r.host("mbp", T0)).unwrap();
    assert_eq!((h.dirs[0].path.as_str(), h.dirs[0].status.as_deref()), ("/tmp/default", Some("not_git")));

    let ui = |s: &mut Store, f: &dyn Fn(&mut W) -> Result<()>| s.write(T0 + S, Origin::ui(None), |w| f(w));
    ui(&mut s, &|w| w.set_project_dir("default", " board ")).unwrap();
    let wsp = s.read(|r| r.workspace(T0 + S)).unwrap();
    assert_eq!(wsp.projects.iter().find(|p| p.key == "default").unwrap().dir, "board");
    let h = s.read(|r| r.host("mbp", T0 + S)).unwrap();
    assert_eq!(h.dirs[0].path, "/tmp/board", "按项目目录派生");
    assert_eq!(h.dirs[0].status, None, "旧路径的检查结果不算");
    assert_eq!(h.workspace.as_ref().unwrap().projects.get("default").map(String::as_str), Some("board"));
    let w = s.read(|r| r.host_work("mbp", T0 + S)).unwrap();
    assert_eq!(w.host.workspace.as_ref().unwrap().projects.get("default").map(String::as_str), Some("board"), "board host 拿得到");
    // board host 按新目录检查后报上来：合得上；存着的工作空间里没有项目目录
    let mut rep = report("mbp", "i1", vec![]);
    rep.workspace.as_mut().unwrap().projects = w.host.workspace.as_ref().unwrap().projects.clone();
    rep.dirs = vec![HostDir { board: B.into(), path: "/tmp/board".into(), status: Some("ok".into()), origin: Some("https://github.com/o/board.git".into()), checked_at: Some(T0 + 2 * S), ..Default::default() }];
    let w = beat(&mut s, T0 + 2 * S, &rep).unwrap();
    assert_eq!((w.host.dirs[0].path.as_str(), w.host.dirs[0].status.as_deref()), ("/tmp/board", Some("ok")));
    let raw: String = s.conn().query_row("SELECT workspace FROM hosts WHERE name = 'mbp'", [], |r| r.get(0)).unwrap();
    assert!(!raw.contains("projects"), "{raw}");
    // 白板路径、BOARD 不受影响
    assert_eq!(s.read(|r| r.resolve(B)).unwrap().path(), B);

    // 不同项目不能用同一个目录；项目标识也不能和别的项目填的目录重名
    ui(&mut s, &|w| w.create_project("站点", "site", "", None, None, NewBoard { name: "主".into(), key: "main".into(), desc: String::new(), pos: None }).map(|_| ())).unwrap();
    let e = ui(&mut s, &|w| w.set_project_dir("site", "Board")).unwrap_err();
    assert!(e.message.contains("default"), "{}", e.message);
    let e = ui(&mut s, &|w| w.set_project_dir("default", "site")).unwrap_err();
    assert!(e.message.contains("site"), "{}", e.message);
    ui(&mut s, &|w| w.create_project("板", "board", "", None, None, NewBoard { name: "主".into(), key: "main".into(), desc: String::new(), pos: None }).map(|_| ())).unwrap();
    let e = ui(&mut s, &|w| w.set_project_dir("board", "")).unwrap_err();
    assert!(e.message.contains("default"), "{}", e.message);
    // 非法的不改
    assert_eq!(ui(&mut s, &|w| w.set_project_dir("default", "../x")).unwrap_err().code, ErrorCode::Usage);
    // 清空：回到项目标识（填成项目标识本身也一样）
    ui(&mut s, &|w| w.set_project_dir("default", "default")).unwrap();
    let wsp = s.read(|r| r.workspace(T0 + S)).unwrap();
    assert_eq!(wsp.projects.iter().find(|p| p.key == "default").unwrap().dir, "");
    assert_eq!(s.read(|r| r.host("mbp", T0 + S)).unwrap().dirs[0].path, "/tmp/default");
}

#[test]
fn fixed_dirs_are_read_only_and_missing_dir_can_be_cloned() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "第三块", "third", "", None)).unwrap();
    // `--dir default/main=<路径>` 单独覆盖一块白板，其他白板由工作空间派生
    let mut rep = report("mbp", "i1", vec![]);
    rep.workspace = Some(HostWorkspace {
        path: "/tmp".into(),
        created: false,
        overrides: vec![HostDirOverride { board: B.into(), path: "/tmp/board".into() }],
        ..Default::default()
    });
    rep.dirs = vec![HostDir {
        board: B.into(),
        path: "/tmp/board".into(),
        fixed: true,
        status: Some("missing".into()),
        checked_at: Some(T0),
        ..Default::default()
    }];
    beat(&mut s, T0, &rep).unwrap();

    // `--dir` 覆盖的白板不能从界面上删掉（提交里没带上的补回去）
    let h = s
        .write(T0 + S, Origin::ui(None), |w| {
            w.host_update("mbp", &HostPatch { workspace: Some(HostWorkspace { path: "/tmp".into(), ..Default::default() }), ..Default::default() })
        })
        .unwrap();
    assert_eq!(h.workspace.as_ref().map(|w| w.overrides.clone()).unwrap_or_default(), vec![HostDirOverride { board: B.into(), path: "/tmp/board".into() }]);

    // 换工作空间：覆盖的那块不变，别的白板由新的工作空间派生；旧界面把几块白板放在同一个工作空间里的照常收
    let h = s
        .write(T0 + S, Origin::ui(None), |w| {
            w.host_update(
                "mbp",
                &HostPatch {
                    workspace: Some(HostWorkspace {
                        path: "/repo".into(),
                        created: false,
                        overrides: vec![HostDirOverride { board: B.into(), path: "/tmp/board".into() }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    // `--dir` 覆盖的白板用覆盖的路径、只读；服务端上别的白板按工作空间派生（default/main#330）
    assert_eq!(h.dirs.iter().filter(|d| d.fixed).map(|d| (d.board.as_str(), d.path.as_str())).collect::<Vec<_>>(), vec![(B, "/tmp/board")]);
    assert!(h.dirs.iter().filter(|d| !d.fixed).all(|d| d.path == "/repo/default"), "{:?}", h.dirs.iter().map(|d| (&d.board, &d.path)).collect::<Vec<_>>());
    let h = s
        .write(T0 + 2 * S, Origin::ui(None), |w| {
            let p = HostPatch {
                dirs: Some(vec![
                    HostDir { board: "default/other".into(), path: "/repo/default/other".into(), ..Default::default() },
                    HostDir { board: "default/third".into(), path: "/repo/default/third".into(), ..Default::default() },
                ]),
                ..Default::default()
            };
            w.host_update("mbp", &p)
        })
        .unwrap();
    assert_eq!(h.workspace.as_ref().map(|w| w.path.as_str()), Some("/repo"));
    // 旧界面按白板提交的 `<工作空间>/<项目>/<白板>` 不是现在派生的目录：留成单独覆盖
    assert_eq!(
        h.dirs.iter().map(|d| (d.board.as_str(), d.path.as_str(), d.fixed)).collect::<Vec<_>>(),
        vec![(B, "/tmp/board", true), ("default/other", "/repo/default/other", true), ("default/third", "/repo/default/third", true)]
    );

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

    // 没配工作空间：报错写项目的目录 `<工作空间>/<项目>`，不是整个白板路径
    let mut s3 = store();
    let mut bare = report("bare", "i1", vec![]);
    bare.workspace = None;
    bare.dirs = vec![];
    beat(&mut s3, T0, &bare).unwrap();
    let e = s3
        .write(T0 + S, Origin::ui(None), |w| w.host_clone_request("bare", B, "git@github.com:buglan/board.git", "buglan"))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("default 项目的目录是 <工作空间>/default"), "{}", e.message);
    assert!(!e.message.contains(B), "{}", e.message);
}

#[test]
fn dispatch_validation_follows_the_table() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    // 不在可领取状态：agent 不能派发（人可以，见 humans_dispatch_from_any_unlocked_status_in_one_step）
    exec(&mut s, T0, &Actor::human(), Cmd::Move { id: b, column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    let by_agent = s.write(T0, Origin::cli(), |w| {
        let r = w.resolve(B)?;
        w.dispatch_create(&r, b, "mbp", None, &Actor::agent("beta/x"))
    });
    assert_eq!(by_agent.unwrap_err().code, ErrorCode::NotInTodo);
    // 机器不存在
    assert_eq!(dispatch(&mut s, T0, a, "nope", None).unwrap_err().code, ErrorCode::NoSuchHost);
    // agent 没找到 / 没有这种 agent
    assert_eq!(dispatch(&mut s, T0, a, "mbp", Some("mycli")).unwrap_err().code, ErrorCode::AgentNotOnHost);
    assert_eq!(dispatch(&mut s, T0, a, "mbp", Some("codex")).unwrap_err().code, ErrorCode::AgentNotOnHost);
    // 有工作空间的机器就视为有这块白板的目录：没在它上面见到的白板也派得出去
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None)).unwrap();
    let c = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve("default/other")?;
            w.exec(&b, &Actor::human(), Cmd::New { title: "C".into(), body: None, column: None, top: false, labels: vec![], from: None })
        })
        .unwrap()
        .card
        .id;
    let d = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve("default/other")?;
            w.dispatch_create(&b, c, "mbp", None, &Actor::human())
        })
        .unwrap();
    assert_eq!(d.board, "default/other");
    s.write(T0, Origin::ui(None), |w| w.dispatch_cancel(d.id, &Actor::human(), "人")).unwrap();
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
    // 名字：<机器名>/<模型名>-<强度>，模型名来自 board host 上报的 agent 数据
    assert_eq!(d.name.as_deref(), Some("mbp/opus-5.5"));
    check_agent_name(d.name.as_deref().unwrap()).unwrap();
    let c = card(&s, a);
    assert_eq!(c.card.holder.as_deref(), Some("mbp/opus-5.5"));
    assert_eq!(c.card.holder_agent.as_deref(), Some("claude"), "卡片上带出持有者的 agent 标识");
    assert_eq!(c.card.column, "doing");
    let info = s.read(|r| r.agent("mbp/opus-5.5")).unwrap().unwrap();
    assert_eq!(info.agent.as_deref(), Some("claude"), "在线列表里带出 agent 标识");
    // host 上报时替名字发心跳：开始第 1 次
    beat(&mut s, T0 + 10 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    assert!(card(&s, a).card.run_started_at.is_some());
    // agent 按 skill 交审后退出
    let me = Actor::agent("mbp/opus-5.5");
    exec(&mut s, T0 + 20 * S, &me, Cmd::Beat { id: a, done: true }).unwrap();
    exec(&mut s, T0 + 21 * S, &me, Cmd::Move { id: a, column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    let d = end(&mut s, T0 + 22 * S, d.id, Some(0), &[]);
    assert_eq!((d.state.as_str(), d.result.as_deref()), ("done", Some("review")));
    let acts: Vec<String> = card(&s, a).activity.iter().filter(|x| x.action == "dispatch").map(|x| x.text.clone()).collect();
    assert!(acts.iter().any(|t| t.starts_with("派发开始")), "{acts:?}");
    assert!(acts.iter().any(|t| t.starts_with("派发结束：已交审")), "{acts:?}");
    // 名字只给这次派发用，结束后离线；下一次派发复用这个名字（序号只在并发时加）
    assert!(s.read(|r| r.require_online("mbp/opus-5.5", T0 + 23 * S)).is_err());
    let b = new_card(&mut s, T0, "B");
    let d2 = dispatch(&mut s, T0 + 30 * S, b, "mbp", None).unwrap();
    assert_eq!(begin(&mut s, T0 + 31 * S, d2.id).unwrap().name.as_deref(), Some("mbp/opus-5.5"));
}

/// 派发的 agent 名字：`<机器名>/<模型名>-<强度>`（例如 `win/opus-5.5-high`）
#[test]
fn dispatch_agent_name_format() {
    // 基本格式：<机器名>/<模型名>-<强度>；强度没有时省略，序号加在末尾
    assert_eq!(hosts::dispatch_agent_name("dsh", "win", "opus-5.5", "high", 0), "win/opus-5.5-high");
    assert_eq!(hosts::dispatch_agent_name("dsh", "win", "opus-5.5", "high", 1), "win/opus-5.5-high-1");
    assert_eq!(hosts::dispatch_agent_name("dsh", "win", "opus-5.5", "", 0), "win/opus-5.5");
    assert_eq!(hosts::dispatch_agent_name("dsh", "win", "deepseek-v4.1-flash", "medium", 2), "win/deepseek-v4.1-flash-medium-2");
    assert_eq!(hosts::dispatch_agent_name("mycli", "mbp", "", "", 2), "mbp/mycli-2");
    // 模型名转成小写、空白和非法字符换成 -；没有模型时（旧版 board host、没配置）用 agent 标识
    assert_eq!(hosts::dispatch_agent_name("claude", "mbp", "Opus 5.5", "HIGH", 0), "mbp/opus-5.5-high");
    assert_eq!(hosts::dispatch_agent_name("dsh", "win", "  DeepSeek V4.1 Flash ", "  ", 0), "win/deepseek-v4.1-flash");
    assert_eq!(hosts::dispatch_agent_name("mycli", "mbp", "", "", 0), "mbp/mycli");
    assert_eq!(hosts::dispatch_agent_name("codex", "mbp", "---", "", 0), "mbp/codex");
    // 名字都能通过 board check 的格式校验；长机器名、长标识、多位的序号也一样（序号只在并发时加，
    // 最多「最多几个会话」（≤8）个，实际是个位数；模型名放不下时截断）
    let cases = [
        ("dsh", "win", "deepseek-v4.1-flash", "high", 0u32),
        ("dsh", "win", "deepseek-v4.1-flash", "xhigh", 12),
        ("my-agent-16chars", "host-abc1234567890123456", "a-model", "high", 9),
        ("dsh", "win", "gpt-5.1-codex-max-preview-very-long-model-name", "high", 8),
    ];
    for (a, h, m, e, seq) in cases {
        let n = hosts::dispatch_agent_name(a, h, m, e, seq);
        check_agent_name(&n).unwrap_or_else(|e| panic!("{n}：{e}"));
        assert!(n.chars().count() <= 48, "{n}");
    }
    // 模型名被截断到放得下为止，序号保留在最后
    assert_eq!(
        hosts::dispatch_agent_name("my-agent-16chars", "host-abc1234567890123456", "a-model", "high", 9),
        "host-abc1234567890123456/a-model-high-9"
    );
    assert_eq!(
        hosts::dispatch_agent_name("dsh", "win", "gpt-5.1-codex-max-preview-very-long-model-name", "high", 8),
        "win/gpt-5.1-codex-max-preview-very-long-model-8"
    );
}

/// 旧格式名字按已知的内置 agent 解析出标识；认不出的保持空（界面上退回字母头像）
#[test]
fn legacy_names_map_to_agent_ids() {
    use crate::agents::legacy_agent_id;
    assert_eq!(legacy_agent_id("dsh-win/deepseek-v4.1-flash").as_deref(), Some("dsh"));
    assert_eq!(legacy_agent_id("claude-mbp/opus-5.5").as_deref(), Some("claude"));
    assert_eq!(legacy_agent_id("pi/pi"), None, "已经不内置的 agent（pi）认不出");
    assert_eq!(legacy_agent_id("Claude/opus").as_deref(), Some("claude"));
    assert_eq!(legacy_agent_id("win/opus-5.5-high"), None);
    assert_eq!(legacy_agent_id("my-dsh/1"), None);
    assert_eq!(legacy_agent_id("alpha"), None);
    assert_eq!(legacy_agent_id(""), None);
}

/// 升级旧库：名字表里已有的名字按旧格式解析一次补上 agent；新格式、认不出的保持空
#[test]
fn old_agent_names_get_agent_id_on_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        s.write(T0, Origin::cli(), |w| w.agent_check("dsh-win/deepseek-v4.1-flash", None)).unwrap();
        s.write(T0, Origin::cli(), |w| w.agent_check("win/opus-5.5-high", None)).unwrap();
    }
    rusqlite::Connection::open(&path).unwrap().execute_batch("ALTER TABLE agents DROP COLUMN agent").unwrap();
    let s = Store::open(&path).expect("旧库要能打开");
    assert_eq!(s.read(|r| r.agent("dsh-win/deepseek-v4.1-flash")).unwrap().unwrap().agent.as_deref(), Some("dsh"));
    assert_eq!(s.read(|r| r.agent("win/opus-5.5-high")).unwrap().unwrap().agent, None);
}

/* ------------------------------ 已经移除的内置 agent（#484） ------------------------------ */

fn pi(custom: bool) -> HostAgent {
    HostAgent { id: "pi".into(), name: "pi".into(), version: "1.5.0".into(), found: true, custom, ..Default::default() }
}

fn run_keys(s: &Store, host: &str) -> Vec<String> {
    let raw: String = s.conn().query_row("SELECT run FROM hosts WHERE name = ?1", [host], |r| r.get(0)).unwrap();
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw).unwrap().keys().cloned().collect()
}

/// 旧版 board host 还上报 pi（没有 custom）：服务端丢掉，机器信息、取到的任务里都没有，运行设置里的 pi 也删掉；
/// 新版 board host 上报配置文件里显式写的 [agents.pi]（custom）照常显示
#[test]
fn removed_builtin_agent_is_dropped_but_custom_one_stays() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    s.conn().execute("UPDATE hosts SET run = '{\"pi\":{\"env\":\"login\"},\"claude\":{\"env\":\"login\"}}' WHERE name = 'mbp'", []).unwrap();
    let mut rep = report("mbp", "i1", vec![]);
    rep.agents.push(pi(false));
    let w = beat(&mut s, T0 + S, &rep).unwrap();
    assert!(!w.host.agents.iter().any(|a| a.id == "pi"), "{:?}", w.host.agents.iter().map(|a| &a.id).collect::<Vec<_>>());
    assert!(!s.read(|r| r.host("mbp", T0 + S)).unwrap().agents.iter().any(|a| a.id == "pi"));
    assert_eq!(run_keys(&s, "mbp"), vec!["claude".to_string()], "运行设置里的 pi 删掉，别的留着");
    // 自定义的同名 agent：留着
    rep.agents.pop();
    rep.agents.push(pi(true));
    let w = beat(&mut s, T0 + 2 * S, &rep).unwrap();
    assert!(w.host.agents.iter().any(|a| a.id == "pi" && a.custom && a.found));
    let v = serde_json::to_value(&w.host.agents).unwrap();
    assert!(v.as_array().unwrap().iter().any(|a| a["id"] == "pi" && a["custom"] == true));
    assert!(v.as_array().unwrap().iter().all(|a| a["id"] == "pi" || a.get("custom").is_none()), "内置的不带 custom：{v}");
}

/// 升级后第一次启动：存着的机器上报、运行设置、默认 agent 里的 pi 清掉，没人用的「仅人可打」pi 标签删掉；
/// 自定义的 pi、有卡片用着的标签、别的标签不动；只做一次（之后人自己建的 pi 标签不会被删）
#[test]
fn removed_builtin_agent_leftovers_are_cleaned_once_on_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let reopen = |path: &std::path::Path, reset: bool| {
        if reset {
            rusqlite::Connection::open(path).unwrap().execute("DELETE FROM meta WHERE key = 'removed_agents'", []).unwrap();
        }
        Store::open(path).unwrap()
    };
    let (card, board) = {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
        let mut b = report("win", "i2", vec![]);
        b.agents.push(pi(true));
        beat(&mut s, T0, &b).unwrap();
        // 旧数据：mbp 上报里有内置的 pi（没有 custom），运行设置、默认 agent 都是它
        let mut agents: Vec<serde_json::Value> = serde_json::from_str(&s.conn().query_row("SELECT agents FROM hosts WHERE name = 'mbp'", [], |r| r.get::<_, String>(0)).unwrap()).unwrap();
        agents.push(serde_json::json!({ "id": "pi", "name": "pi", "version": "1.5.0", "found": true }));
        s.conn()
            .execute(
                "UPDATE hosts SET agents = ?1, run = '{\"pi\":{\"env\":\"login\"},\"claude\":{\"env\":\"login\"}}', agent = 'pi' WHERE name = 'mbp'",
                [serde_json::to_string(&agents).unwrap()],
            )
            .unwrap();
        s.conn().execute("UPDATE hosts SET run = '{\"pi\":{\"env\":\"login\"}}', agent = 'pi' WHERE name = 'win'", []).unwrap();
        s.write(T0, Origin::ui(None), |w| w.create_label("default", "pi", None, true)).unwrap();
        s.write(T0, Origin::ui(None), |w| w.create_label("default", "急", None, true)).unwrap();
        let card = new_card(&mut s, T0, "用着 pi 标签");
        let board: i64 = s.conn().query_row("SELECT board FROM cards WHERE id = ?1", [card], |r| r.get(0)).unwrap();
        (card, board)
    };
    let labels = |s: &Store| -> Vec<String> {
        s.conn().prepare("SELECT name FROM labels ORDER BY name").unwrap().query_map([], |r| r.get::<_, String>(0)).unwrap().map(|x| x.unwrap()).collect()
    };
    let pi_label = |s: &Store| -> i64 { s.conn().query_row("SELECT id FROM labels WHERE name = 'pi'", [], |r| r.get(0)).unwrap() };
    // 有卡片用着 pi 标签：标签留着
    {
        let s = Store::open(&path).unwrap();
        s.conn().execute("INSERT INTO card_labels (board, card, label) VALUES (?1, ?2, ?3)", rusqlite::params![board, card, pi_label(&s)]).unwrap();
    }
    let s = reopen(&path, true);
    assert!(labels(&s).contains(&"pi".to_string()), "有卡片用着的不删");
    let mbp = s.read(|r| r.host("mbp", T0)).unwrap();
    assert!(!mbp.agents.iter().any(|a| a.id == "pi"));
    assert_eq!(run_keys(&s, "mbp"), vec!["claude".to_string()]);
    let agent: Option<String> = s.conn().query_row("SELECT agent FROM hosts WHERE name = 'mbp'", [], |r| r.get(0)).unwrap();
    assert_eq!(agent, None, "默认 agent 回到本机第一个找到的");
    // win 上的 pi 是自定义的：上报、运行设置、默认 agent 都不动
    let win = s.read(|r| r.host("win", T0)).unwrap();
    assert!(win.agents.iter().any(|a| a.id == "pi" && a.custom));
    assert_eq!(run_keys(&s, "win"), vec!["pi".to_string()]);
    let agent: Option<String> = s.conn().query_row("SELECT agent FROM hosts WHERE name = 'win'", [], |r| r.get(0)).unwrap();
    assert_eq!(agent.as_deref(), Some("pi"));
    // 没卡片用了：再升级一次（清掉标记）时删掉；别的「仅人可打」标签不动
    s.conn().execute("DELETE FROM card_labels", []).unwrap();
    drop(s);
    let s = reopen(&path, false);
    assert!(labels(&s).contains(&"pi".to_string()), "做过一次就不再做");
    drop(s);
    let s = reopen(&path, true);
    let names = labels(&s);
    assert!(!names.contains(&"pi".to_string()) && names.contains(&"急".to_string()), "{names:?}");
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
    // 两个并发派发：第一个不带序号，第二个在模型名后面加 -1
    let da = dispatch(&mut s, T0, a, "mbp", Some("dsh")).unwrap();
    let db = dispatch(&mut s, T0, b, "mbp", Some("dsh")).unwrap();
    let da = begin(&mut s, T0 + S, da.id).unwrap();
    assert_eq!(da.name.as_deref(), Some("mbp/deepseek-v4.1-flash"));
    check_agent_name(da.name.as_deref().unwrap()).unwrap();
    let db = begin(&mut s, T0 + S, db.id).unwrap();
    assert_eq!(db.name.as_deref(), Some("mbp/deepseek-v4.1-flash-1"), "并发派发在名字末尾加序号");
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
    assert_eq!(dp.name.as_deref(), Some("mbp/deepseek-v4.1-flash"), "离线后的名字可以复用");
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

fn mv(id: i64, column: &str, index: Option<usize>) -> Cmd {
    Cmd::Move { id, column: column.into(), top: false, index, order: None, before: None, after: None }
}

fn col(s: &Store, key: &str) -> Vec<i64> {
    s.read(|r| {
        let b = r.resolve(B)?;
        Ok(r.board_snapshot(&b, T0, true)?.cols.get(key).cloned().unwrap_or_default())
    })
    .unwrap()
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
    // 人把它拖出它排队的状态：派发自动取消
    exec(&mut s, T0 + 2 * S, &Actor::human(), mv(a, "review", None)).unwrap();
    assert_eq!(get(&s, d.id).result.as_deref(), Some("canceled"));
}

/// 派发排队时卡片就进了「进行中」：没有持有者、显示排队，别的 agent 领不了、移不了；
/// 开始时原地领取；没开始就取消时回到原来的可领取状态最上面；人把它拖出进行中等于取消派发
#[test]
fn queued_dispatch_waits_in_the_lock_status() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    let c = new_card(&mut s, T0, "C");
    let da = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let ca = card(&s, a);
    assert_eq!((ca.card.column.as_str(), ca.card.holder.as_deref(), ca.card.claimed_from.as_deref()), ("doing", None, Some("todo")));
    let text = &ca.activity.iter().rev().find(|x| x.text.starts_with("派发到")).unwrap().text;
    assert_eq!(text, "派发到 mbp · Claude Code（手动），在「进行中」排队");
    assert_eq!(col(&s, "todo"), vec![b, c]);
    // 流转记录：派发排队算一次换状态（不算领取、不算返工）
    let kinds: Vec<(String, Option<String>, Option<String>)> = s
        .conn()
        .prepare("SELECT kind, from_key, to_key FROM moves WHERE card = ?1 ORDER BY id")
        .unwrap()
        .query_map([a], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(kinds, vec![("dispatch".into(), Some("todo".into()), Some("doing".into()))]);

    // 别的 agent：带 ID 领、沿连线移都报 DISPATCHED；不带 ID 领到 B，排在排队的 A 前面
    s.write(T0, Origin::cli(), |w| w.agent_check("alpha/x", None)).unwrap();
    let alpha = Actor::agent("alpha/x");
    let e = exec(&mut s, T0 + S, &alpha, Cmd::Claim { id: Some(a) }).unwrap_err();
    assert_eq!(e.code, ErrorCode::Dispatched);
    assert!(e.message.contains("在「进行中」排队"), "{}", e.message);
    assert_eq!(exec(&mut s, T0 + S, &alpha, mv(a, "review", None)).unwrap_err().code, ErrorCode::Dispatched);
    assert_eq!(exec(&mut s, T0 + S, &alpha, Cmd::Claim { id: None }).unwrap().card.id, b);
    assert_eq!(col(&s, "doing"), vec![b, a], "被持有的排在排队的前面");
    // 手动派发报「已经派发」，而不是「不在可领取状态」
    assert_eq!(dispatch(&mut s, T0 + S, a, "mbp", None).unwrap_err().code, ErrorCode::AlreadyDispatched);

    // 开始：原地领取，claimed_from 仍是原来的可领取状态（异常退出、释放时回那里）
    let da = begin(&mut s, T0 + 2 * S, da.id).unwrap();
    let ca = card(&s, a);
    assert_eq!((ca.card.column.as_str(), ca.card.holder.as_deref(), ca.card.claimed_from.as_deref()), ("doing", da.name.as_deref(), Some("todo")));
    assert!(ca.activity.iter().any(|x| x.action == "claim" && x.text == "领取了卡片（派发排队时已在「进行中」）"));

    // C 排队后被取消：回到待办最上面
    let dc = dispatch(&mut s, T0 + 3 * S, c, "mbp", None).unwrap();
    assert_eq!(col(&s, "doing"), vec![b, a, c]);
    s.write(T0 + 4 * S, Origin::ui(None), |w| w.dispatch_cancel(dc.id, &Actor::human(), "buglan")).unwrap();
    let cc = card(&s, c);
    assert_eq!((cc.card.column.as_str(), cc.card.claimed_from.as_deref()), ("todo", None));
    assert!(cc.activity.iter().any(|x| x.text.starts_with("取消了派发") && x.text.ends_with("，卡片回到「待办」最上面")));
    // 人把排队的卡片拖回待办（拖到哪就在哪）：派发自动取消
    new_card(&mut s, T0 + 4 * S, "D");
    let dc = dispatch(&mut s, T0 + 5 * S, c, "mbp", None).unwrap();
    exec(&mut s, T0 + 6 * S, &Actor::human(), mv(c, "todo", Some(1))).unwrap();
    assert_eq!(get(&s, dc.id).result.as_deref(), Some("canceled"));
    assert_eq!(col(&s, "todo")[1], c);
    assert_eq!(card(&s, c).card.claimed_from, None);
    // 人在进行中里调整排队卡片的顺序：派发不受影响
    let dc = dispatch(&mut s, T0 + 7 * S, c, "mbp", None).unwrap();
    exec(&mut s, T0 + 7 * S, &Actor::human(), mv(c, "doing", Some(0))).unwrap();
    assert_eq!(get(&s, dc.id).state, "queued");
    // 删除后恢复：回到排队前的可领取状态
    exec(&mut s, T0 + 8 * S, &Actor::human(), Cmd::Delete { id: c }).unwrap();
    assert_eq!(get(&s, dc.id).result.as_deref(), Some("canceled"));
    let r = exec(&mut s, T0 + 9 * S, &Actor::human(), Cmd::Restore { id: c }).unwrap();
    assert_eq!((r.card.column.as_str(), r.card.holder.as_deref()), ("todo", None));
}

/// 会话结束、机器移除时排着的回到待办，仍按原来的先后排在最上面
#[test]
fn canceled_queue_goes_back_in_order() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let x = new_card(&mut s, T0, "X");
    let a = new_card(&mut s, T0, "A");
    let b = new_card(&mut s, T0, "B");
    dispatch(&mut s, T0, a, "mbp", None).unwrap();
    dispatch(&mut s, T0, b, "mbp", None).unwrap();
    assert_eq!((col(&s, "todo"), col(&s, "doing")), (vec![x], vec![a, b]));
    s.write(T0 + S, Origin::ui(None), |w| w.host_remove("mbp", &Actor::human(), "buglan")).unwrap();
    assert_eq!((col(&s, "todo"), col(&s, "doing")), (vec![a, b, x], vec![]));
    assert!(card(&s, a).activity.iter().any(|t| t.text.starts_with("移除了机器 mbp") && t.text.ends_with("，卡片回到「待办」最上面")));
}

/// 优先标签（PRD §8 规则 1）：有排队派发的优先卡片照样被跳过；定时领取一轮里优先卡片先被派发；手动派发不受影响
#[test]
fn priority_labels_in_queue_and_cron() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let p = s.write(T0, Origin::ui(None), |w| w.create_label("default", "急", None, false)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.update_label("default", p.id, None, None, None, Some(true))).unwrap();
    new_card(&mut s, T0, "A");
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
    assert_eq!(begin(&mut s, T0 + S, db.id).unwrap_err().code, ErrorCode::Usage, "最多 1 个会话：没有空位");
    let w = s.read(|r| r.host_work("mbp", T0 + S)).unwrap();
    assert_eq!((w.running.len(), w.queued.len()), (1, 1));
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(2), paused: Some(true), ..Default::default() })).unwrap();
    assert_eq!(begin(&mut s, T0 + 2 * S, db.id).unwrap_err().code, ErrorCode::Usage, "暂停接单");
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { paused: Some(false), ..Default::default() })).unwrap();
    assert_eq!(begin(&mut s, T0 + 3 * S, db.id).unwrap().state, "running");
    // 设置校验
    let e = s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(9), ..Default::default() })).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    let e = s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { agent: Some("mycli".into()), ..Default::default() })).unwrap_err();
    assert_eq!(e.code, ErrorCode::AgentNotOnHost);
    // 自动接单和接单时段：时段排好、挨着的并成一段；旧界面的「定时领取」开关当成自动接单的开关，时间表不再存
    assert!(!host(&s, T0).auto, "新机器默认不自动接单");
    assert_eq!(host(&s, T0).windows, vec![[0, 1440]], "默认全天");
    let h = s
        .write(T0, Origin::ui(None), |w| {
            w.host_update("mbp", &HostPatch { auto: Some(true), windows: Some(vec![[1320, 1440], [0, 240], [240, 480]]), ..Default::default() })
        })
        .unwrap();
    assert!(h.auto);
    assert_eq!(h.windows, vec![[0, 480], [1320, 1440]]);
    let e = s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { windows: Some(vec![[600, 600]]), ..Default::default() })).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    let h = s
        .write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { cron_on: Some(false), cron: Some(vec!["3:00".into()]), ..Default::default() }))
        .unwrap();
    assert!(!h.auto && !h.cron_on && h.cron.is_empty());
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
    assert_eq!(card(&s, 2).card.holder.as_deref(), Some("mbp/opus-5.5"));
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
            w.exec(&b, &Actor::human(), Cmd::New { title: "O".into(), body: None, column: None, top: false, labels: vec![], from: None })
        })
        .unwrap()
        .card
        .id;
    // 规则「全部」→ claude：两块白板的卡片都不带它，靠「其余卡片 →」兜底给 claude，都能领
    s.write(T0, Origin::ui(None), |w| w.create_label("default", "全部", None, false)).unwrap();
    // 一个会话只做一张（N = 1）：这里测的是占空位、多块白板轮流；会话排几张见 session_* 的用例
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { rules: Some(vec![cond("in", &["全部"], "claude")]), fallback: Some("claude".into()), max: Some(2), per_session: Some(1), ..Default::default() })).unwrap();
    let pull = |s: &mut Store, at: i64| s.write(at, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap();
    // 没开自动接单时不领
    assert!(pull(&mut s, T0 + S).is_none());
    assert!(!host(&s, T0 + S).round);
    s.write(T0 + S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { auto: Some(true), ..Default::default() })).unwrap();
    let d1 = pull(&mut s, T0 + 2 * S).unwrap();
    let d2 = pull(&mut s, T0 + 2 * S).unwrap();
    assert_eq!((d1.board.as_str(), d1.card, d1.source.as_str()), (B, a, "cron"));
    assert_eq!((d2.board.as_str(), d2.card), ("default/other", o), "一块白板领一张再换下一块");
    assert!(pull(&mut s, T0 + 2 * S).is_none(), "没有空位");
    assert!(host(&s, T0 + 2 * S).round, "在接单时段里");
    // 一张结束后马上领下一张
    end(&mut s, T0 + 3 * S, d2.id, Some(0), &[]);
    let d3 = pull(&mut s, T0 + 4 * S).unwrap();
    assert_eq!(d3.card, b);
    end(&mut s, T0 + 5 * S, d1.id, Some(1), &[]);
    // 都领完了：没有卡可领，但还在接单时段里，有新卡再来领
    assert!(pull(&mut s, T0 + 6 * S).is_none());
    assert!(host(&s, T0 + 6 * S).round);
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

/// 资源（PRD §10.12「资源」）：存最近一次和趋势，随机器信息给界面；每次都在变，不算机器有变化；旧版 board host 没有
#[test]
fn host_resources_are_stored_without_bumping_rev() {
    use crate::hostres::{ResCpu, ResDisk, ResGpu, ResMem};
    let mut s = store();
    let res = |cpu: u8| HostRes {
        age: Some(2 * S),
        cpu: Some(ResCpu { pct: cpu, cores: 12, load: Some([1.5, 1.0, 0.5]), model: "Apple M3 Pro".into() }),
        mem: Some(ResMem { total: 32 << 30, used: 16 << 30 }),
        gpus: Some(vec![ResGpu { name: "Apple M3 Pro".into(), vendor: "Apple".into(), pct: Some(12), ..Default::default() }]),
        disks: Some(vec![ResDisk { mount: "/".into(), label: "Macintosh HD".into(), total: 1000, avail: 80, ws: true }]),
        ..Default::default()
    };
    // 新机器第一次上报就带着
    beat(&mut s, T0, &HostReport { res: Some(res(20)), ..report("mbp", "i1", vec![]) }).unwrap();
    let h = host(&s, T0);
    let r = h.res.as_ref().expect("存了资源");
    assert_eq!(r.at, T0 - 2 * S, "采样时间按 board host 报的「过了多久」往前推");
    assert_eq!(r.hist.cpu, vec![20]);
    assert_eq!(r.hist.mem, vec![50]);
    // 之后每次上报：接上趋势，不推送（机器没有别的变化）
    let rev = s.rev().unwrap();
    beat(&mut s, T0 + 10 * S, &HostReport { res: Some(res(40)), ..report("mbp", "i1", vec![]) }).unwrap();
    assert_eq!(s.rev().unwrap(), rev, "资源变了不算机器有变化");
    let h = host(&s, T0 + 10 * S);
    assert_eq!(h.res.as_ref().unwrap().hist.cpu, vec![20, 40]);
    assert_eq!(h.res.as_ref().unwrap().cpu.as_ref().unwrap().pct, 40);
    // 列表里也有（界面每 10 秒取一次机器列表）
    let list = s.read(|r| r.hosts(T0 + 10 * S)).unwrap();
    assert_eq!(list.iter().find(|x| x.name == "mbp").unwrap().res.as_ref().unwrap().hist.gpu, vec![12, 12]);
    // 离线后留着最后一次（界面画成离线前的数据）
    let later = s.read(|r| r.host("mbp", T0 + 600 * S)).unwrap();
    assert!(!later.online && later.res.is_some());
    // 换成旧版 board host（不报资源）：清掉，界面写「不上报」
    beat(&mut s, T0 + 20 * S, &report("mbp", "i1", vec![])).unwrap();
    assert!(host(&s, T0 + 20 * S).res.is_none());
    let json = serde_json::to_value(host(&s, T0 + 20 * S)).unwrap();
    assert!(json.get("res").is_none(), "没有资源时不带这个字段");
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
    assert!(s.write(T0 + 61 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none(), "这一段接单里没做成的卡先不再领");
    // 这一段接单结束（关掉自动接单再打开，或出了接单时段再进来）之后可以重新领
    let auto = |s: &mut Store, at: i64, on: bool| s.write(at, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { auto: Some(on), ..Default::default() })).unwrap();
    auto(&mut s, T0 + 62 * S, false);
    assert!(s.write(T0 + 62 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none());
    auto(&mut s, T0 + 62 * S, true);
    let d2 = s.write(T0 + 63 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().expect("新的一段接单应该可以再领");
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
    assert_eq!(card(&s, a).card.holder.as_deref(), Some("mbp/opus-5.5"));
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
fn pausing_stops_auto_claim_and_resuming_continues() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    new_card(&mut s, T0, "A");
    lead_round(&mut s);
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(host(&s, T0 + S).round);
    let h = s.write(T0 + 2 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { paused: Some(true), ..Default::default() })).unwrap();
    assert!(!h.round, "暂停接单时不自动领");
    let t = new_card_tagged(&mut s, "T");
    assert!(s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none());
    let h = s.write(T0 + 3 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { paused: Some(false), ..Default::default() })).unwrap();
    assert!(h.round, "恢复接单后还在接单时段里：接着领");
    assert_eq!(s.write(T0 + 4 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().map(|d| d.card), Some(t));
    // 不在接单时段里也不领：把时段设成不含现在的那一段
    let m = crate::sched::server_minute(T0 + 5 * S);
    let away = if m < 720 { [900, 960] } else { [60, 120] };
    let h = s.write(T0 + 5 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { windows: Some(vec![away]), ..Default::default() })).unwrap();
    assert!(!h.round, "现在不在接单时段里");
    let h = s.write(T0 + 5 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { windows: Some(vec![[m, m + 1]]), ..Default::default() })).unwrap();
    assert!(h.round, "现在在接单时段里");
    let h = s.write(T0 + 5 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { windows: Some(vec![]), ..Default::default() })).unwrap();
    assert!(!h.round, "一段都没有：不接单");
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
    exec(&mut s, T0 + 2 * S, &Actor::agent("mbp/opus-5.5"), Cmd::Move { id: a, column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    let d = end(&mut s, T0 + 3 * S, d.id, Some(0), &[]);
    assert_eq!(d.result.as_deref(), Some("review"));
    assert_eq!(card(&s, a).card.column, "review");
}

/* ---------------- 启动环境（启动环境与桌面端需求 §二） ---------------- */

fn env(id: &str, name: &str) -> HostEnv {
    HostEnv { id: id.into(), name: name.into(), path: String::new() }
}

fn check(env: &str, found: bool, version: &str) -> EnvCheck {
    EnvCheck { env: env.into(), found, version: version.into(), ..Default::default() }
}

/// 一台 Windows 机器：cmd、PowerShell 7、Git Bash、WSL（Ubuntu）。mycli 不支持 cmd；WSL 里没装 mycli
fn win_report(instance: &str) -> HostReport {
    let mut r = report("mbp", instance, vec![]);
    r.envs = vec![env("cmd", "cmd"), env("pwsh", "PowerShell 7"), env("git-bash", "Git Bash"), env("wsl:Ubuntu", "WSL（Ubuntu）")];
    r.agents = vec![
        HostAgent {
            id: "mycli".into(),
            name: "mycli".into(),
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
    let mycli = h.agents.iter().find(|a| a.id == "mycli").unwrap();
    assert_eq!((mycli.env.as_deref(), mycli.found, mycli.version.as_str()), (Some("git-bash"), true, "1.5.0"));
    let claude = h.agents.iter().find(|a| a.id == "claude").unwrap();
    assert_eq!((claude.env.as_deref(), claude.found, claude.version.as_str()), (Some("wsl:Ubuntu"), true, "2.5.0"), "按顺序跳过没找到的");
    // 派发记录写启动环境，动态里写明
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", Some("mycli")).unwrap();
    assert_eq!(d.env, None, "排队时还没定");
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!((d.env.as_deref(), d.env_name.as_deref()), (Some("git-bash"), Some("Git Bash")));
    let acts: Vec<String> = card(&s, a).activity.into_iter().map(|x| x.text).collect();
    assert!(acts.iter().any(|t| t.starts_with("派发开始：mbp 上的 mycli（Git Bash）以 mbp/mycli 领取")), "{acts:?}");
}

#[test]
fn missing_skill_follows_the_env_in_use() {
    let mut s = store();
    // Git Bash 里看不到 board-skills，PowerShell 7 里看得到
    let mut r = win_report("i1");
    let mycli = r.agents.iter_mut().find(|a| a.id == "mycli").unwrap();
    mycli.no_skill = Some("~/.agents/skills/board-skills".into());
    mycli.envs[0].no_skill = Some("~/.agents/skills/board-skills".into());
    let h = beat(&mut s, T0, &r).unwrap().host;
    let get = |h: &HostInfo, id: &str| h.agents.iter().find(|a| a.id == id).unwrap().no_skill.clone();
    assert_eq!(get(&h, "mycli").as_deref(), Some("~/.agents/skills/board-skills"), "默认用 Git Bash：提醒");
    assert_eq!(get(&h, "claude"), None, "装了的不标");
    let h = set_env(&mut s, "mycli", "pwsh").unwrap();
    assert_eq!(get(&h, "mycli"), None, "换到看得到的启动环境：不提醒");
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
    assert_eq!(set_env(&mut s, "mycli", "cmd").unwrap_err().code, ErrorCode::Usage, "mycli 不支持 cmd");
    assert_eq!(set_env(&mut s, "mycli", "powershell").unwrap_err().code, ErrorCode::Usage, "本机没有 Windows PowerShell");
    let e = set_env(&mut s, "mycli", "wsl:Ubuntu").unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("WSL（Ubuntu）") && e.message.contains("没找到"), "{}", e.message);
    assert_eq!(set_env(&mut s, "nope", "pwsh").unwrap_err().code, ErrorCode::Usage);
    // 改成 PowerShell 7：下一次派发就用它
    let h = set_env(&mut s, "mycli", "pwsh").unwrap();
    assert_eq!(h.agents.iter().find(|a| a.id == "mycli").unwrap().env.as_deref(), Some("pwsh"));
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", Some("mycli")).unwrap();
    assert_eq!(begin(&mut s, T0 + S, d.id).unwrap().env.as_deref(), Some("pwsh"));
    // 设置的环境本机已经没有了（卸载了 PowerShell 7）：按默认规则选
    let mut r = win_report("i1");
    r.envs.retain(|e| e.id != "pwsh");
    for a in r.agents.iter_mut() {
        a.envs.retain(|c| c.env != "pwsh");
    }
    let h = beat(&mut s, T0 + 2 * S, &r).unwrap().host;
    assert_eq!(h.agents.iter().find(|a| a.id == "mycli").unwrap().env.as_deref(), Some("git-bash"));
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
    // 默认 agent 也跟着变：mycli
    assert_eq!(h.agent.as_deref(), Some("mycli"));
}

/// 装了 agent、启动环境里缺 Node.js（npx）：board host 报成没找到并写明缺什么；没设启动环境时用找得到的那个环境，
/// 设了的照设的算、派发报错写明缺什么（#458）
#[test]
fn agent_missing_node_is_not_usable_and_says_why() {
    let mut s = store();
    let mut r = win_report("i1");
    let no_node = || EnvCheck { missing: Some("Node.js（npx）".into()), ..check("pwsh", false, "2.4.0") };
    // 哪个环境都用不了：按缺 Node.js 的那个报，界面写缺什么，不写「没找到」
    r.agents[1].envs = vec![check("git-bash", false, ""), no_node()];
    let c = claude_of(&beat(&mut s, T0, &r).unwrap().host);
    assert_eq!((c.env.as_deref(), c.found, c.missing.as_deref(), c.version.as_str()), (Some("pwsh"), false, Some("Node.js（npx）"), "2.4.0"));
    r.agents[1].envs = vec![no_node(), check("git-bash", true, "2.4.0")];
    let h = beat(&mut s, T0, &r).unwrap().host;
    let c = claude_of(&h);
    assert_eq!((c.env.as_deref(), c.found, c.missing.as_deref()), (Some("git-bash"), true, None), "没设启动环境：用找得到的 Git Bash");
    // 选了 PowerShell 7 之后，那里的 Node.js 没了（升级、卸载）：照选的算
    r.agents[1].envs = vec![check("pwsh", true, "2.4.0"), check("git-bash", true, "2.4.0")];
    beat(&mut s, T0, &r).unwrap();
    set_env(&mut s, "claude", "pwsh").unwrap();
    r.agents[1].envs = vec![no_node(), check("git-bash", true, "2.4.0")];
    let h = beat(&mut s, T0 + S, &r).unwrap().host;
    let c = claude_of(&h);
    assert_eq!((c.env.as_deref(), c.found, c.missing.as_deref(), c.version.as_str()), (Some("pwsh"), false, Some("Node.js（npx）"), "2.4.0"));
    let a = new_card(&mut s, T0, "A");
    let e = dispatch(&mut s, T0 + S, a, "mbp", Some("claude")).unwrap_err();
    assert_eq!(e.code, ErrorCode::AgentNotOnHost);
    assert!(e.message.contains("在PowerShell 7里缺 Node.js（npx）"), "{}", e.message);
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

/* ---------------- 定时领取：--dir 覆盖的目录 ---------------- */

/// `--dir` 覆盖（或旧版 board host 上报）的目录 `fixed: true`：只是目录由启动参数固定，照样按顺序参与定时领取；
/// 白板标识不是 `<项目>/<白板>` 的旧目录仍跳过（win2 的 `D:\repo\board` 被标成 fixed，导致 default/main 一轮都领不到）
#[test]
fn pull_includes_fixed_dirs_and_skips_legacy_board_ids() {
    let mut s = store();
    // 旧版 board host：只有一个 `--dir` 目录、`board` 一列是老的白板标识，工作空间从它导出
    let mut rep = report("mbp", "i1", vec![]);
    rep.workspace = None;
    rep.dirs = vec![
        HostDir { board: "main".into(), path: "/repo/board".into(), fixed: true, ..Default::default() },
        HostDir { board: B.into(), path: "/repo/board".into(), fixed: true, ..Default::default() },
    ];
    beat(&mut s, T0, &rep).unwrap();
    let h = host(&s, T0);
    assert!(h.dirs.iter().any(|d| d.board == B && d.fixed), "default/main 是 --dir 覆盖的目录：{:?}", h.dirs.iter().map(|d| (&d.board, d.fixed)).collect::<Vec<_>>());
    lead_round(&mut s);
    let a = new_card_tagged(&mut s, "A");
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", Some(1))).unwrap().expect("--dir 覆盖的 default/main 也要参与定时领取");
    assert_eq!(d.card, a);
    assert_eq!(d.state, "running");
    assert_eq!(d.path.as_deref(), Some("/repo/board"), "派发在 --dir 指定的目录里做");
    assert_eq!(card(&s, a).card.holder.as_deref(), Some("mbp/opus-5.5"));

    // 白板标识不是 `<项目>/<白板>` 的旧目录（`main` 这种）仍跳过：一块白板都领不到，这一轮照常结束
    let mut s = store();
    let mut rep = report("mbp", "i1", vec![]);
    rep.workspace = None;
    rep.dirs = vec![HostDir { board: "main".into(), path: "/repo/board".into(), fixed: true, ..Default::default() }];
    beat(&mut s, T0, &rep).unwrap();
    // 规则原样写进库（不经过 host_update 的校验：没有目录能派生时它不给加规则）
    new_card(&mut s, T0, "B");
    s.write(T0, Origin::cli(), |w| {
        let rules = serde_json::json!([{ "op": "in", "labels": ["deepseek"], "agent": "claude" }]).to_string();
        w.tx.execute("UPDATE hosts SET rules = ?1 WHERE name = 'mbp'", rusqlite::params![rules])?;
        Ok(())
    })
    .unwrap();
    assert!(host(&s, T0).dirs.iter().all(|d| !crate::valid_board(&d.board)), "只有旧的 `main`：{:?}", host(&s, T0).dirs.iter().map(|d| &d.board).collect::<Vec<_>>());
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none(), "非法的旧目录不会被领取");
    assert!(!s.read(|r| r.host("mbp", T0 + 2 * S)).unwrap().round, "没有可领的白板，这一轮结束");
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
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { rules: Some(vec![cond("in", &["deepseek"], "claude")]), fallback: Some(String::new()), auto: Some(true), ..Default::default() })).unwrap();
}

fn session_of(s: &Store, session: i64) -> Vec<Dispatch> {
    s.read(|r| r.host_recent(1, 50)).unwrap().into_iter().chain(s.read(|r| r.active_dispatches()).unwrap()).filter(|d| d.session == session).collect()
}

/// 「最多几个会话」限的是同时在跑的会话（agent 进程）数，不是派发条数：
/// N = 2、最多 2 个会话、6 张卡。第一个会话在跑；第二个会话的卡片在会话里排着，不额外占会话位；
/// 两个会话占满 2 个位置后这一轮不再开新会话，第三个会话要等有会话结束后才开
#[test]
fn session_capacity_counts_sessions_not_dispatches() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    lead_round(&mut s);
    set_host(&mut s, HostPatch { max: Some(2), per_session: Some(2), ..Default::default() }).unwrap();
    for t in ["A", "B", "C", "D", "E", "F"] {
        new_card_tagged(&mut s, t);
    }
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    // 第一个会话：1 张在跑 + 1 张排着，只占 1 个会话位
    let d1 = pull(&mut s, T0 + 2 * S).unwrap();
    assert_eq!((d1.state.as_str(), d1.seq, d1.total), ("running", 1, 2));
    // 第二个会话照常开得起来（会话位还剩 1 个），它排着的卡不占位
    let d2 = pull(&mut s, T0 + 2 * S).expect("还有 1 个会话位");
    assert_eq!((d2.state.as_str(), d2.seq, d2.total), ("running", 1, 2));
    let running = |s: &Store| s.read(|r| r.active_dispatches()).unwrap().into_iter().filter(|d| d.state == "running").collect::<Vec<_>>();
    assert_eq!(running(&s).len(), 2, "两个会话各一张在跑");
    let queued = |s: &Store| s.read(|r| r.active_dispatches()).unwrap().into_iter().filter(|d| d.state == "queued").collect::<Vec<_>>();
    assert_eq!(queued(&s).len(), 2, "两个会话各排着 1 张，不占会话位");
    // 两个会话位都占满：这一轮不开新会话（E、F 还没派）
    assert!(pull(&mut s, T0 + 2 * S).is_none(), "会话位满了");
    assert_eq!(col(&s, "todo").len(), 2, "E、F 留在待办");
    // 手动派发的另一张卡同样等会话位
    let q0 = queued(&s)[0].card;
    assert_eq!(dispatch(&mut s, T0 + 3 * S, q0, "mbp", None).unwrap_err().code, ErrorCode::AlreadyDispatched, "排着派的卡片不能再派");
    // 第一个会话做完两张、进程退出：空出一个会话位，接着开新会话
    let name = d1.name.clone().unwrap();
    let mut cur = d1.card;
    for k in 0..2 {
        submit(&mut s, T0 + (4 + k) * S, &name, cur);
        match next(&mut s, T0 + (4 + k) * S, &name) {
            Ok(c) => cur = c.card.id,
            Err(e) => {
                assert_eq!(e.code, ErrorCode::SessionDone);
                assert!(e.message.contains("排着的卡片都开始过或取消了"), "{}", e.message);
            }
        }
    }
    end(&mut s, T0 + 7 * S, d1.id, Some(0), &[]);
    let d3 = pull(&mut s, T0 + 8 * S).expect("空出一个会话位：开新会话");
    assert_eq!((d3.state.as_str(), d3.total), ("running", 2));
    // 会话位不够时的错误提示按会话口径写：新卡手派到这台机器上先排队，begin 报没有空位
    let e5 = new_card(&mut s, T0 + 9 * S, "G");
    let extra = dispatch(&mut s, T0 + 9 * S, e5, "mbp", None).unwrap();
    let err = begin(&mut s, T0 + 9 * S, extra.id).unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("最多 2 个会话同时跑"), "{}", err.message);
    assert_eq!(get(&s, extra.id).state, "queued");
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

/// N = 4、6 张同标签的卡片、最多 1 个会话：第一个会话做 4 张后 SESSION_DONE，第二个会话做剩下 2 张；每张卡各一条派发
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
    // 上一张还由这个名字持有：ALREADY_HOLDING，提示用 board submit / giveup 交出去（default/main#557）
    let e = next(&mut s, T0 + S, &name).unwrap_err();
    assert_eq!(e.code, ErrorCode::AlreadyHolding);
    assert_eq!(e.message, "#1 还由你持有：先 board submit 1（做不下去 board giveup 1），再 board next");
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

/// 一轮领取开几个会话（最多 4 个会话）：先把一个会话排满再开下一个。返回每个会话（开始的那张 + 排着的）
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

/// 最多 4 个会话、N = 4：4、6、16、20 张卡分别开 1、2（4 + 2）、4、4 个会话；两个会话不会排到同一张卡；N = 1 时 4 张开 4 个会话
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

/// N = 4、6 张同标签的卡、最多 1 个会话：第 1 张在跑，第 2–4 张排在会话里（会话第 N/4 张）；第 5、6 张没有派发；
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
    assert_eq!(a.text, format!("派发到 mbp · Claude Code（定时），会话第 3/4 张，在「进行中」排队等 {name} 接着做"));
    // 排着的卡片在「进行中」，没有持有者；第 1 张被持有、排在它们前面
    assert_eq!(col(&s, "doing"), cards[..4].to_vec());
    assert!(cards[1..4].iter().all(|c| card(&s, *c).card.holder.is_none()));
    assert_eq!(col(&s, "todo"), cards[4..].to_vec());
    assert!(s.read(|r| r.card_dispatch(&r.resolve(B)?, cards[4])).unwrap().is_none(), "第 5 张没有派发");
    // board host 拿到的排队里没有会话里排着的，也不能用 begin 开始它们
    let w = beat(&mut s, T0 + S, &report("mbp", "i1", vec![d1.id])).unwrap();
    assert!(w.queued.is_empty(), "{:?}", w.queued);
    assert_eq!(begin(&mut s, T0 + S, queued[0].id).unwrap_err().code, ErrorCode::Usage);
    assert!(pull(&mut s, T0 + S).is_none(), "最多 1 个会话：没有空位");
    // board next 按排好的顺序开始
    let mut t = T0 + 2 * S;
    for (k, want) in cards[1..4].iter().enumerate() {
        let cur = s.read(|r| r.active_dispatches()).unwrap().into_iter().find(|d| d.state == "running").unwrap();
        submit(&mut s, t, &name, cur.card);
        let c = next(&mut s, t, &name).unwrap();
        assert_eq!((c.card.id, c.card.holder.as_deref()), (*want, Some(name.as_str())));
        let started = c.activity.iter().rev().find(|a| a.text.starts_with("派发开始")).unwrap();
        // 派发记录里的模型是 board host 上报的、配置文件里现在的
        assert!(started.text.ends_with(&format!("接着做（定时，这个会话的第 {} 张，模型 opus-5.5）", k + 2)), "{}", started.text);
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
    // 没有派发的那张一直在待办：别人领得走，不会被这一轮的会话排进去
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
    // 人取消第 2 张、把第 3 张拖出待办：都「已取消」，board next 领第 4 张
    s.write(T0 + 2 * S, Origin::ui(None), |w| w.dispatch_cancel(queued[0].id, &Actor::human(), "buglan")).unwrap();
    exec(&mut s, T0 + 2 * S, &Actor::human(), Cmd::Move { id: cards[2], column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    assert_eq!(get(&s, queued[0].id).result.as_deref(), Some("canceled"));
    assert_eq!(get(&s, queued[1].id).result.as_deref(), Some("canceled"));
    submit(&mut s, T0 + 3 * S, &name, d1.card);
    assert_eq!(next(&mut s, T0 + 3 * S, &name).unwrap().card.id, cards[3]);
    assert_eq!(card(&s, free).card.column, "todo", "没有派发的卡片这一轮不动");
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
        assert_eq!(col(&s, "todo"), vec![cards[2], cards[3]], "回到待办最上面，仍是排好的顺序");
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
            auto: Some(true),
            ..Default::default()
        },
    )
}

/// 一轮领取能派发的全部（最多 8 个会话，派发完为止）：（卡片，agent）
fn round_all(s: &mut Store, at: i64) -> Vec<(i64, String)> {
    // 这里只看规则把卡片交给谁：一个会话一张（N = 1），每张卡都由 pull 开始
    set_host(s, HostPatch { max: Some(8), per_session: Some(1), auto: Some(true), ..Default::default() }).unwrap();
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
    // 不带标签的卡片没有规则，留在待办（这一轮不派）
    new_card(&mut s, T0, "无标签");
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
}

/// 没有一条领取规则时不自动领取：「现在领一轮」被拒绝，定时到点也不领卡；兜底单独设了也不算规则。
/// 手动派发、`board claim` 不受影响；加一条规则后照常领取
#[test]
fn no_rules_never_auto_claims() {
    let mut s = store();
    beat(&mut s, T0, &report_cd("i1")).unwrap();
    new_card(&mut s, T0, "A");
    // 开了自动接单、但一条规则也没有：不算在自动接单
    set_host(&mut s, HostPatch { auto: Some(true), ..Default::default() }).unwrap();
    assert!(!host(&s, T0 + S).round);
    // 只设了兜底也一样
    set_host(&mut s, HostPatch { fallback: Some("dsh".into()), ..Default::default() }).unwrap();
    assert!(host(&s, T0 + S).rules.is_empty());
    assert!(!host(&s, T0 + S).round);
    // 以前的「现在领一轮」接口留给旧版 board host 调：什么也不做，不报错
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(!host(&s, T0 + S).round);
    // 来领卡时也不领
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
    // 加一条规则后开始自动领取，只领规则命中的卡片
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
    // 这台机器上没有 mycli：拒绝
    assert_eq!(rules(&mut s, &[("claude", "mycli")], None).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(rules(&mut s, &[], Some("mycli")).unwrap_err().code, ErrorCode::Usage);
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

/// 两台都装了 claude、dsh 的机器：`mbp`（i1）和 `desk`（j1）；`desk` 的 id 是 2
fn rep_mbp() -> HostReport {
    report_cd("i1")
}

fn rep_desk() -> HostReport {
    let mut r = report("desk", "j1", vec![]);
    r.agents = vec![claude(true), dsh(true)];
    r
}

/// 改过名的 mbp（同一个进程 i1）
fn rep_win() -> HostReport {
    let mut r = report_cd("i1");
    r.name = "win".into();
    r
}

fn two_hosts(s: &mut Store) {
    beat(s, T0, &rep_mbp()).unwrap();
    beat(s, T0, &rep_desk()).unwrap();
    for name in ["mbp", "desk"] {
        s.write(T0, Origin::ui(None), |w| w.host_update(name, &HostPatch { max: Some(4), per_session: Some(1), auto: Some(true), ..Default::default() }))
            .unwrap();
    }
}

/// 每台机器来领一次，返回它领到的卡片（`report` 是它的心跳：每台机器一个进程标识，来领卡也要带着）
fn pull2(s: &mut Store, at: i64, rep: &HostReport) -> Option<i64> {
    beat(s, at, rep).unwrap();
    s.write(at, Origin::cli(), |w| w.dispatch_pull(&rep.name, &rep.instance, Some(1))).unwrap().map(|d| d.card)
}

#[test]
fn pick_off_machine_does_not_auto_claim_that_agent() {
    let mut s = store();
    two_hosts(&mut s);
    let lc = label(&mut s, "claude");
    let c = new_card(&mut s, T0, "C");
    tag(&mut s, c, lc);
    rules(&mut s, &[("claude", "claude")], None).unwrap();
    // 没设置过：两台都参与（和以前一样），第一台照常领
    let desk = s.read(|r| r.host("desk", T0)).unwrap().id;
    assert_eq!(pull2(&mut s, T0 + S, &rep_mbp()), Some(c));
    let d = s.read(|r| r.active_dispatches()).unwrap();
    assert_eq!(d.len(), 1);
    // 勾掉 mbp（只留 desk 一台）：mbp 不领 claude 的卡
    let mbp = host(&s, T0).id;
    let sc = s.write(T0 + 2 * S, Origin::ui(None), |w| w.sched_pick("claude", &[desk], &[mbp], false)).unwrap();
    assert_eq!(sc.picks.iter().find(|p| p.agent == "claude").map(|p| (p.hosts.clone(), p.none, p.unset)), Some((vec![desk], false, false)));
    let c2 = new_card(&mut s, T0 + 2 * S, "C2");
    tag(&mut s, c2, lc);
    assert_eq!(pull2(&mut s, T0 + 3 * S, &rep_mbp()), None, "没勾的机器有空位也不领");
}

#[test]
fn pick_none_means_agent_never_auto_claims() {
    let mut s = store();
    two_hosts(&mut s);
    let lc = label(&mut s, "claude");
    let c = new_card(&mut s, T0, "C");
    tag(&mut s, c, lc);
    rules(&mut s, &[("claude", "claude")], None).unwrap();
    // 两台都取消：一台都没勾，两台都不领（换新的进程标识也一样），卡片留在待办；手动派发照常
    let ids: Vec<i64> = s.read(|r| r.hosts(T0)).unwrap().iter().map(|h| h.id).collect();
    let sc = s.write(T0 + S, Origin::ui(None), |w| w.sched_pick("claude", &[], &ids, false)).unwrap();
    let p = sc.picks.iter().find(|p| p.agent == "claude").unwrap();
    assert!(p.none && p.hosts.is_empty() && !p.unset, "{p:?}");
    assert_eq!(pull2(&mut s, T0 + 2 * S, &rep_mbp()), None);
    assert_eq!(pull2(&mut s, T0 + 2 * S, &rep_desk()), None);
    assert_eq!(card(&s, c).card.column, "todo");
    assert_eq!(dispatch(&mut s, T0 + 3 * S, c, "mbp", Some("claude")).unwrap().agent, "claude");
    // 恢复全选（所有装了它的机器）：先取消手动派发（卡片回到待办）
    s.write(T0 + 4 * S, Origin::ui(None), |w| w.sched_pick("claude", &[], &[], true)).unwrap();
    let sc = s.read(|r| r.sched(T0 + 4 * S)).unwrap();
    let p = sc.picks.iter().find(|p| p.agent == "claude").unwrap();
    let ids: Vec<i64> = s.read(|r| r.hosts(T0)).unwrap().iter().map(|h| h.id).collect();
    assert!(p.unset && !p.none && p.hosts.len() == ids.len(), "{p:?}");
    let back = s.read(|r| r.active_dispatches()).unwrap();
    for d in back {
        s.write(T0 + 5 * S, Origin::ui(None), |w| w.dispatch_cancel(d.id, &Actor::human(), "不要了")).unwrap();
    }
    // 这一段接单里 mbp 先跳过它（手动派发过又取消）：换新的一段接单后照常领
    s.write(T0 + 6 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { paused: Some(true), ..Default::default() })).unwrap();
    s.write(T0 + 6 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { paused: Some(false), ..Default::default() })).unwrap();
    assert_eq!(pull2(&mut s, T0 + 7 * S, &rep_mbp()), Some(c), "新的一段接单照常领");
}

#[test]
fn picks_are_per_agent_and_survive_rename() {
    let mut s = store();
    two_hosts(&mut s);
    let lc = label(&mut s, "claude");
    let ld = label(&mut s, "deepseek");
    let c = new_card(&mut s, T0, "C");
    tag(&mut s, c, lc);
    let d = new_card(&mut s, T0, "D");
    tag(&mut s, d, ld);
    rules(&mut s, &[("claude", "claude"), ("deepseek", "dsh")], None).unwrap();
    // 只勾掉 claude 在 mbp 上的参与：dsh 的卡不变
    let desk = s.read(|r| r.host("desk", T0)).unwrap().id;
    let mbp = s.read(|r| r.host("mbp", T0)).unwrap().id;
    s.write(T0 + S, Origin::ui(None), |w| w.sched_pick("claude", &[desk], &[mbp], false)).unwrap();
    // 改名：存的是机器 id，选择不受影响
    s.write(T0 + 2 * S, Origin::cli(), |w| w.host_rename("mbp", "win")).unwrap();
    let sc = s.read(|r| r.sched(T0 + 2 * S)).unwrap();
    assert_eq!(sc.picks.iter().find(|p| p.agent == "claude").unwrap().hosts, vec![desk]);
    let pd = sc.picks.iter().find(|p| p.agent == "dsh").unwrap();
    assert!(pd.unset && pd.hosts.contains(&mbp) && pd.hosts.contains(&desk), "dsh 没设置过 = 全都参与");
    // 改名后还是那台机器：win 不领 claude 的卡（claude 的卡留给勾着的 desk），dsh 的卡它自己领走
    let first = pull2(&mut s, T0 + 3 * S, &rep_win());
    assert_ne!(first, Some(c), "win（原来的 mbp）不领 claude 的卡");
    assert_eq!(first, Some(d), "它照常领自己参与的 dsh 卡");
    assert_eq!(pull2(&mut s, T0 + 4 * S, &rep_desk()), Some(c));
    // 规则整个保存（界面改标签）：没带 hosts 时保留原来的选择
    let next = vec![rule("claude", "claude"), rule("deepseek", "dsh")];
    s.write(T0 + 5 * S, Origin::ui(None), |w| w.sched_update(Some(&next), None)).unwrap();
    let sc = s.read(|r| r.sched(T0 + 5 * S)).unwrap();
    assert_eq!(sc.picks.iter().find(|p| p.agent == "claude").unwrap().hosts, vec![desk], "整个列表替换时选择不丢");
}

#[test]
fn removed_or_new_host_and_agent() {
    let mut s = store();
    two_hosts(&mut s);
    let lc = label(&mut s, "claude");
    let c = new_card(&mut s, T0, "C");
    tag(&mut s, c, lc);
    rules(&mut s, &[("claude", "claude")], None).unwrap();
    let desk = s.read(|r| r.host("desk", T0)).unwrap().id;
    let mbp = s.read(|r| r.host("mbp", T0)).unwrap().id;
    // 勾掉的 mbp 仍然不参与；移除勾着的 desk 不会把 mbp 重新选回来
    s.write(T0 + S, Origin::ui(None), |w| w.sched_pick("claude", &[desk], &[mbp], false)).unwrap();
    s.write(T0 + 2 * S, Origin::ui(None), |w| w.host_remove("desk", &Actor::human(), "buglan")).unwrap();
    let sc = s.read(|r| r.sched(T0 + 2 * S)).unwrap();
    let p = sc.picks.iter().find(|p| p.agent == "claude").unwrap();
    assert!(p.hosts.is_empty() && p.none && !p.unset, "{p:?}");
    assert_eq!(pull2(&mut s, T0 + 3 * S, &rep_mbp()), None, "被排除的 mbp 没因为另一台被移除而恢复");
    // 移除被排除的 mbp：排除列表清空；之后新装 claude 的机器默认参与
    s.write(T0 + 4 * S, Origin::ui(None), |w| w.host_remove("mbp", &Actor::human(), "buglan")).unwrap();
    let sc = s.read(|r| r.sched(T0 + 4 * S)).unwrap();
    let p = sc.picks.iter().find(|p| p.agent == "claude").unwrap();
    assert!(p.hosts.is_empty() && p.unset, "排除列表清完 = 默认全选");
    let mut air = report("air", "k1", vec![]);
    air.agents = vec![claude(true)];
    beat(&mut s, T0 + 5 * S, &air).unwrap();
    s.write(T0 + 5 * S, Origin::ui(None), |w| w.host_update("air", &HostPatch { auto: Some(true), max: Some(1), ..Default::default() })).unwrap();
    assert_eq!(pull2(&mut s, T0 + 6 * S, &air), Some(c), "后装上这个 agent 的机器默认参与");
}

/// 只有兜底、没有规则行的 agent 也能单独勾机器；没勾的机器不领规则没命中的卡片
#[test]
fn fallback_only_agent_can_pick_machines() {
    let mut s = store();
    two_hosts(&mut s);
    label(&mut s, "claude");
    let other = new_card(&mut s, T0, "其余");
    rules(&mut s, &[("claude", "claude")], Some("dsh")).unwrap();
    let desk = s.read(|r| r.host("desk", T0)).unwrap().id;
    let mbp = host(&s, T0).id;
    let sc = s.write(T0 + S, Origin::ui(None), |w| w.sched_pick("dsh", &[], &[mbp], false)).unwrap();
    let p = sc.picks.iter().find(|p| p.agent == "dsh").unwrap();
    assert_eq!((p.hosts.clone(), p.none, p.unset), (vec![desk], false, false));
    assert_eq!(pull2(&mut s, T0 + 2 * S, &rep_mbp()), None, "兜底 agent 在 mbp 上被勾掉");
    assert_eq!(pull2(&mut s, T0 + 3 * S, &rep_desk()), Some(other), "desk 仍然接兜底卡片");
}

/// 会话中途把机器从「由这些机器做」里点掉：board next 不再把排着的卡交给它，卡片释放给别的机器
#[test]
fn next_stops_when_machine_is_unpicked_mid_session() {
    let mut s = store();
    two_hosts(&mut s);
    let lc = label(&mut s, "claude");
    let first = new_card(&mut s, T0, "第一张");
    tag(&mut s, first, lc);
    let second = new_card(&mut s, T0, "第二张");
    tag(&mut s, second, lc);
    rules(&mut s, &[("claude", "claude")], None).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { per_session: Some(2), ..Default::default() })).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = pull(&mut s, T0 + 2 * S).unwrap();
    assert_eq!(d.card, first);
    let name = d.name.unwrap();
    let mbp = host(&s, T0).id;
    s.write(T0 + 3 * S, Origin::ui(None), |w| w.sched_pick("claude", &[], &[mbp], false)).unwrap();
    submit(&mut s, T0 + 3 * S, &name, first);
    assert_eq!(next(&mut s, T0 + 4 * S, &name).unwrap_err().code, ErrorCode::SessionDone);
    assert_eq!(card(&s, second).card.column, "todo", "排着的那张已经放回待办");
    assert_eq!(pull2(&mut s, T0 + 5 * S, &rep_desk()), Some(second), "仍然勾着的 desk 接着领");
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

/* ---------------- 升级：每台机器的规则和定时领取 → 全局规则、自动接单和接单时段 ---------------- */

/// 升级前的库：规则、兜底、定时领取都存在机器上。升级后规则并成全局的一份（一模一样的只留一条）；
/// 旧的定时领取不迁移——每台机器的自动接单都是关着的、接单时段全天，由人在「调度」里重新设
#[test]
fn per_host_rules_migrate_to_global_rules_and_old_cron_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        beat(&mut s, T0, &report_cd("i1")).unwrap();
        let mut win = report_cd("w1");
        win.name = "win".into();
        beat(&mut s, T0, &win).unwrap();
        let mut air = report_cd("a1");
        air.name = "air".into();
        beat(&mut s, T0, &air).unwrap();
    }
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            r#"DELETE FROM sched;
               UPDATE hosts SET rules = '[{"op":"in","labels":["定时"],"agent":"dsh"},{"label":"claude","agent":"claude"}]', fallback = 'dsh', cron_on = 1, cron = '["*/5 * * * *"]' WHERE name = 'mbp';
               UPDATE hosts SET rules = '[{"op":"in","labels":["CLAUDE"],"agent":"claude"},{"op":"none","labels":[],"agent":"dsh"}]', cron_on = 1, cron = '["0 23 * * *"]' WHERE name = 'win';
               UPDATE hosts SET rules = '[{"op":"in","labels":["定时"],"agent":"dsh"}]', cron_on = 0, cron = '["*/5 * * * *"]' WHERE name = 'air';"#,
        )
        .unwrap();
    let s = Store::open(&path).unwrap();
    let sched = s.read(|r| r.sched(T0)).unwrap();
    let got: Vec<(String, Vec<String>, String)> = sched.rules.iter().map(|r| (r.op.clone(), r.labels.clone(), r.agent.clone())).collect();
    let want = vec![("in", vec!["定时"], "dsh"), ("in", vec!["claude"], "claude"), ("none", vec![], "dsh")];
    assert_eq!(got, want.into_iter().map(|(o, l, a)| (o.to_string(), l.into_iter().map(String::from).collect(), a.to_string())).collect::<Vec<_>>(), "按机器建立的先后接上，一模一样的（不分大小写）只留一条");
    assert_eq!(sched.fallback.as_deref(), Some("dsh"));
    let h = |n: &str| s.read(|r| r.host(n, T0)).unwrap();
    for n in ["mbp", "win", "air"] {
        assert!(!h(n).auto, "{n}：旧的定时领取不迁移，自动接单关着，打开之前不会自己领卡");
        assert_eq!(h(n).windows, vec![[0, 1440]], "{n}：接单时段全天");
        assert!(h(n).ops.is_empty(), "{n}：不写操作记录");
        assert!(!h(n).round);
    }
    // 每台机器读到的都是全局这一份；旧的定时领取字段不再有值
    assert_eq!(h("air").rules.len(), 3);
    assert!(!h("mbp").cron_on && h("mbp").cron.is_empty());
    drop(s);
    // 再打开一次不重复迁移
    let s = Store::open(&path).unwrap();
    assert_eq!(s.read(|r| r.sched(T0)).unwrap().rules.len(), 3);
}

/// 领取规则是全局的：在任一处改，每台机器都按它领；校验看的是所有项目的标签、所有机器的 agent
#[test]
fn rules_are_global_and_validated_against_all_hosts() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let mut win = report("win", "w1", vec![]);
    win.agents = vec![dsh(true)];
    beat(&mut s, T0, &win).unwrap();
    label(&mut s, "定时");
    let set = |s: &mut Store, rules: Vec<ClaimRule>, fb: Option<&str>| s.write(T0, Origin::ui(None), |w| w.sched_update(Some(&rules), fb));
    // dsh 只在 win 上：照样能写进规则；mbp 上这条不生效，win 上生效
    let out = set(&mut s, vec![cond("in", &["定时"], "dsh")], Some("claude")).unwrap();
    assert!(out.rules[0].active && out.fallback_active);
    let active = |s: &Store, n: &str| s.read(|r| r.host(n, T0)).unwrap().rules[0].active;
    assert!(!active(&s, "mbp") && active(&s, "win"));
    // 没有哪个项目有的标签、没有机器找得到的 agent 拒绝
    assert!(set(&mut s, vec![cond("in", &["没有的"], "dsh")], None).unwrap_err().message.contains("没有哪个项目里有标签"));
    assert!(set(&mut s, vec![cond("in", &["定时"], "mycli")], None).unwrap_err().message.contains("没有机器上找得到"));
    // 旧界面从机器设置里改规则：改的是全局这一份
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { rules: Some(vec![]), ..Default::default() })).unwrap();
    assert!(s.read(|r| r.sched(T0)).unwrap().rules.is_empty());
    assert!(s.read(|r| r.host("win", T0)).unwrap().rules.is_empty());
}

/// 还没有机器（或哪台都没上报找得到的 agent）时：领取规则能选内置的 agent，存下来但不生效；有机器上报之后照旧只认找得到的
#[test]
fn rules_accept_builtin_agents_before_any_host() {
    let mut s = store();
    label(&mut s, "bug");
    let set = |s: &mut Store, rules: Vec<ClaimRule>, fb: Option<&str>| s.write(T0, Origin::ui(None), |w| w.sched_update(Some(&rules), fb));
    let out = set(&mut s, vec![cond("in", &["bug"], "claude")], Some("codex")).unwrap();
    assert_eq!((out.rules[0].agent.as_str(), out.rules[0].active), ("claude", false));
    assert_eq!((out.fallback.as_deref(), out.fallback_active), (Some("codex"), false));
    // 不是内置的 agent 照样拒绝，原因里列出能选的
    let err = set(&mut s, vec![cond("in", &["bug"], "nope")], None).unwrap_err().message;
    assert!(err.contains("没有机器上找得到 agent「nope」") && err.contains("dsh、claude、codex"), "{err}");
    // 有机器上报了 agent：规则对它生效；新加的只能选找得到的（dsh 不在这台机器上）
    let mut win = report("win", "w1", vec![]);
    win.agents = vec![dsh(false), claude(true)];
    beat(&mut s, T0, &win).unwrap();
    assert!(s.read(|r| r.sched(T0)).unwrap().rules[0].active);
    assert!(set(&mut s, vec![cond("in", &["bug"], "claude"), cond("none", &[], "dsh")], None).unwrap_err().message.contains("没有机器上找得到 agent「dsh」"));
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
    assert_eq!(set_conds(&mut s, vec![cond("in", &["claude"], "mycli")], None).unwrap_err().code, ErrorCode::Usage);
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
    rusqlite::Connection::open(&path).unwrap().execute_batch(r#"UPDATE sched SET rules = '[{"label":"claude","agent":"claude"}]'"#).unwrap();
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
    assert_eq!(d.name.as_deref(), Some("mbp/opus-5.5"));
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

/// board host 级别的心跳 / final 只对自己派发生成的名字有效：
/// 不能给另一个会话（新版 CLI check 的）或另一台机器派发的名字发心跳、让它离线
#[test]
fn host_cannot_beat_or_final_another_sessions_name() {
    use crate::agents::{BeatBy, NameOwner};
    let mut s = store();
    // 一个新版 CLI 会话 check 的名字
    s.write(T0, Origin::cli(), |w| w.agent_check_owned("claude-mbp/opus-5.5", None, NameOwner::Session("abc"))).unwrap();
    let agent = "claude-mbp/opus-5.5";
    // board host 的心跳、final 都被拒，名字的状态不变
    let e = s.write(T0 + S, Origin::cli(), |w| w.agent_beat_by(agent, false, BeatBy::Host)).unwrap_err();
    assert_eq!(e.code, ErrorCode::NameTaken);
    assert!(e.message.contains("board host"), "{}", e.message);
    let e = s.write(T0 + S, Origin::cli(), |w| w.agent_beat_by(agent, true, BeatBy::Host)).unwrap_err();
    assert_eq!(e.code, ErrorCode::NameTaken);
    s.read(|r| r.require_online(agent, T0 + S)).expect("被拒之后名字仍在线");
    // 它自己的会话照常被接受
    s.write(T0 + 2 * S, Origin::cli(), |w| w.agent_beat_by(agent, false, BeatBy::Cli(Some("abc")))).unwrap();
    // 另一个 host 派发生成的名字：别的 host 也不能替它心跳，只有它自己的 host 可以
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    let name = d.name.clone().unwrap();
    // 名字归属由 board host 的派发维持；这里直接发一次 board host 级心跳，自己的 host 能发
    s.write(T0 + 2 * S, Origin::cli(), |w| w.agent_beat_by(&name, false, BeatBy::Host)).unwrap();
    s.read(|r| r.require_online(&name, T0 + 2 * S)).expect("board host 能给自己派发的名字发心跳");
}

/* ------------------------------ 登录 shell 的提示 ------------------------------ */

/// 卡片 #232：登录 shell 自己打的作业控制提示、`logout`、sudo 提示不是输出；真正的错误要留着
#[test]
fn login_shell_noise_is_recognized() {
    for line in [
        "logout",
        " bash: no job control in this shell ",
        "bash: cannot set terminal process group (-1): Inappropriate ioctl for device",
        "stty: standard input: Inappropriate ioctl for device",
        // Ubuntu/Debian 的 /etc/bash.bashrc 打的 sudo 提示（CI 的 ubuntu-latest 就有，卡片 #232 第二次失败）
        "To run a command as administrator (user \"root\"), use \"sudo <command>\".",
        "See \"man sudo_root\" for details.",
    ] {
        assert!(hosts::shell_noise(line), "{line}");
    }
    for line in [
        "",
        "zsh:1: command not found: cargo",
        "logout finished",
        "missing ok.txt",
        "checking",
        "stty -a failed",
        // 不是这两句固定形状的都不算：命令自己打的同类文字要留着
        "To run a command as administrator",
        "See \"man sudo_root\" for details",
    ] {
        assert!(!hosts::shell_noise(line), "{line}");
    }
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
fn repo_belongs_to_the_project_and_all_its_boards_share_it() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| {
        w.create_board("default", "文档", "docs", "", None)
    })
    .unwrap();
    let ws = |s: &Store| s.read(|r| r.workspace(T0)).unwrap().projects[0].clone();
    // 按项目设：项目和它下面每块白板都是这一个仓库
    ui(&mut s, |w| {
        w.set_repo_branch("default", Some("git@github.com:o/r.git"), Some("main"))
    })
    .unwrap();
    let p = ws(&s);
    assert_eq!(
        (p.repo.as_str(), p.branch.as_str()),
        ("git@github.com:o/r.git", "main")
    );
    assert!(
        p.boards
            .iter()
            .all(|b| b.repo == "git@github.com:o/r.git" && b.branch == "main"),
        "{:?}",
        p.boards
    );
    // 旧客户端按白板改：改的是它所在的项目，另一块白板跟着变
    ui(&mut s, |w| {
        w.set_repo_branch("default/docs", Some("git@github.com:o/x.git"), None)
    })
    .unwrap();
    let p = ws(&s);
    assert_eq!(p.repo, "git@github.com:o/x.git");
    assert!(p.boards.iter().all(|b| b.repo == "git@github.com:o/x.git"));
    let snap = s
        .read(|r| r.resolve(B).and_then(|b| r.board_snapshot(&b, T0, false)))
        .unwrap();
    assert_eq!(
        snap.repo, "git@github.com:o/x.git",
        "白板快照里带着项目的仓库"
    );
    // 不存在的项目、白板
    assert_eq!(
        ui(&mut s, |w| w.set_repo_branch("nope", Some(""), None))
            .unwrap_err()
            .code,
        ErrorCode::BoardNotFound
    );
    assert_eq!(
        ui(&mut s, |w| w.set_repo_branch(
            "default/nope",
            Some(""),
            None
        ))
        .unwrap_err()
        .code,
        ErrorCode::BoardNotFound
    );
}

#[test]
fn old_database_moves_board_repo_to_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        s.write(T0, Origin::ui(None), |w| {
            w.create_board("default", "文档", "docs", "", None)
        })
        .unwrap();
    }
    {
        // 升级前：仓库地址、集成分支设在白板上（第一块没设地址、第二块设了；分支只有第一块设了）
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "ALTER TABLE projects DROP COLUMN repo; ALTER TABLE projects DROP COLUMN branch;
             UPDATE boards SET repo = '', branch = 'dev' WHERE key = 'main';
             UPDATE boards SET repo = 'git@github.com:o/r.git', branch = '' WHERE key = 'docs';",
        )
        .unwrap();
    }
    let s = Store::open(&path).unwrap();
    let p = s.read(|r| r.workspace(T0)).unwrap().projects[0].clone();
    assert_eq!(
        (p.repo.as_str(), p.branch.as_str()),
        ("git@github.com:o/r.git", "dev"),
        "各取第一块设了的白板"
    );
    assert!(p
        .boards
        .iter()
        .all(|b| b.repo == p.repo && b.branch == p.branch));
}

#[test]
fn dir_override_can_cover_a_whole_project() {
    let ws = HostWorkspace {
        path: "/ws".into(),
        created: false,
        overrides: vec![
            HostDirOverride {
                board: "a".into(),
                path: "/srv/a".into(),
            },
            HostDirOverride {
                board: "b/old".into(),
                path: "/legacy/b-old".into(),
            },
        ],
        ..Default::default()
    };
    let dir = |b: &str| hosts::derive_dir(&ws, b, "Linux").map(|d| (d.path, d.fixed));
    // `--dir a=<路径>`：项目 a 的所有白板都用它
    assert_eq!(dir("a/main"), Some(("/srv/a".into(), true)));
    assert_eq!(dir("a/docs"), Some(("/srv/a".into(), true)));
    // 旧写法 `--dir b/old=<路径>` 只管那一块；同项目的其他白板用 `<工作空间>/<项目>`
    assert_eq!(dir("b/old"), Some(("/legacy/b-old".into(), true)));
    assert_eq!(dir("b/main"), Some(("/ws/b".into(), false)));
    assert!(
        board_core_valid("a")
            && board_core_valid("a/b")
            && !board_core_valid("A")
            && !board_core_valid("a/b/c")
    );
}

fn board_core_valid(k: &str) -> bool {
    crate::valid_dir_key(k)
}

#[test]
fn old_database_boards_get_no_repo_or_branch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.db");
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
fn agent_update_is_handed_to_the_host_and_failure_can_retry() {
    let mut s = store();
    let mut rep = report("mbp", "i1", vec![]);
    rep.agents[0] = managed("2.1.300", "2.1.325", "");
    beat(&mut s, T0, &rep).unwrap();

    let h = s.write(T0 + S, Origin::ui(None), |w| w.host_agent_ask("mbp", "agent-update", "claude", Some("2.1.325"), "octocat")).unwrap();
    let ask = h.ask.clone().unwrap();
    assert_eq!((ask.kind.as_str(), ask.agent.as_deref(), ask.to.as_deref()), ("agent-update", Some("claude"), Some("2.1.325")));
    assert_eq!(h.ops[0].text, "请求更新 Claude Code 到 2.1.325");

    // board host 开始处理：请求清掉，之后上报失败原因；状态留在卡片上，人可以重试
    let w = acked(&mut s, T0 + 2 * S, "i1", ask.id);
    assert!(w.host.ask.is_none());
    let mut bad = rep.clone();
    bad.agents[0] = managed("2.1.300", "2.1.325", "failed");
    bad.agents[0].manage.as_mut().unwrap().error = "npm 退出码 1".into();
    bad.ask_error = Some("更新 Claude Code 失败：npm 退出码 1".into());
    let w = beat(&mut s, T0 + 3 * S, &bad).unwrap();
    assert!(w.host.ops[0].text.contains("npm 退出码 1"), "{:?}", w.host.ops);
    let retry = s.write(T0 + 4 * S, Origin::ui(None), |w| w.host_agent_ask("mbp", "agent-update", "claude", Some("2.1.325"), "octocat")).unwrap();
    let retry = retry.ask.unwrap();
    assert_eq!(retry.agent.as_deref(), Some("claude"));
    acked(&mut s, T0 + 5 * S, "i1", retry.id);

    // 本机还在跑同一个安装/更新：不再接新的请求
    let mut busy = report("mbp", "i1", vec![]);
    busy.agents[0] = managed("2.1.300", "2.1.325", "running");
    beat(&mut s, T0 + 6 * S, &busy).unwrap();
    assert!(s.write(T0 + 6 * S, Origin::ui(None), |w| w.host_agent_ask("mbp", "agent-update", "claude", None, "octocat")).unwrap_err().message.contains("正在更新"));

    // 不支持代管、已经是最新、离线：都不发请求
    let mut cfg = report("mbp", "i1", vec![]);
    cfg.agents[0].manage = Some(AgentManage { method: "config".into(), reason: "配置文件".into(), latest_version: "2.1.325".into(), ..Default::default() });
    beat(&mut s, T0 + 7 * S, &cfg).unwrap();
    assert!(s.write(T0 + 8 * S, Origin::ui(None), |w| w.host_agent_ask("mbp", "agent-update", "claude", None, "octocat")).unwrap_err().message.contains("配置文件"));
}

#[test]
fn agent_auto_update_waits_for_no_running_dispatch_and_does_not_loop_after_failure() {
    let mut s = store();
    let mut rep = report("mbp", "i1", vec![]);
    rep.agents[0] = managed("2.1.300", "2.1.325", "");
    beat(&mut s, T0, &rep).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { agent_auto: Some(true), ..Default::default() })).unwrap();

    // 有派发（哪怕是刚结束、本机还报着的）时不自动更新
    let w = beat(&mut s, T0 + S, &HostReport { running: vec![999], ..rep.clone() }).unwrap();
    assert!(w.host.ask.is_none());

    // 没在跑了：发自动更新请求，操作记录写明「自动更新」
    let w = beat(&mut s, T0 + 2 * S, &rep).unwrap();
    let ask = w.host.ask.clone().unwrap();
    assert_eq!((ask.kind.as_str(), ask.by.as_str(), ask.agent.as_deref()), ("agent-update", "自动更新", Some("claude")));
    assert_eq!(w.host.ops[0].text, "自动更新 Claude Code 到 2.1.325");
    acked(&mut s, T0 + 3 * S, "i1", ask.id);

    // 失败后状态是 failed：不每一轮都重试；人点重试仍然可以
    let mut bad = rep.clone();
    bad.agents[0] = managed("2.1.300", "2.1.325", "failed");
    bad.ask_error = Some("更新 Claude Code 失败：网络".into());
    let w = beat(&mut s, T0 + 4 * S, &bad).unwrap();
    assert!(w.host.ask.is_none());
    let again = beat(&mut s, T0 + 5 * S, &bad).unwrap();
    assert!(again.host.ask.is_none());

    // 关掉开关：有新版本也不自动更新
    s.write(T0 + 6 * S, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { agent_auto: Some(false), ..Default::default() })).unwrap();
    let w = beat(&mut s, T0 + 7 * S, &rep).unwrap();
    assert!(w.host.ask.is_none());
}

#[test]
fn only_the_token_a_live_host_reports_may_take_secrets() {
    // board host 的令牌接口（GET /api/hosts/{名字}/mcp）只给这台机器上报时用的令牌：
    // 别的令牌、机器不存在、离线之后都拿不到密钥原文（卡片 #236）
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let own = |s: &Store, name: &str, token: i64, at: i64| s.read(|r| r.host_uses_token(name, token, at)).unwrap();
    assert!(own(&s, "mbp", 1, T0 + S));
    // 另一个令牌（没绑到这台机器）：不行
    assert!(!own(&s, "mbp", 2, T0 + S));
    // 机器名不对、机器不存在：不行
    assert!(!own(&s, "nope", 1, T0 + S));
    // 机器改名或换成另一个令牌上报后，旧令牌不再能用
    beat(&mut s, T0 + 2 * S, &report("mbp", "i1", vec![])).unwrap();
    assert!(own(&s, "mbp", 1, T0 + 3 * S));
    // 机器离线（50 秒没有上报）：不行
    assert!(!own(&s, "mbp", 1, T0 + 2 * S + 60 * S));
}

/// 令牌绑定机器（卡片 #379）：board host 的接口只认这台机器上报时用的令牌；派发的接口只认派发所在机器的令牌；
/// 绑着另一个还没吊销的令牌时，别的令牌上报不能接管（离线、resume 也一样），原来的令牌吊销后才能换绑
#[test]
fn host_and_dispatch_calls_need_the_token_the_host_reports_with() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| {
        w.tx.execute_batch(
            "CREATE TABLE auth_tokens (id INTEGER PRIMARY KEY, name TEXT NOT NULL, hash TEXT NOT NULL, hint TEXT NOT NULL, created_at INTEGER NOT NULL, last_used_at INTEGER, host TEXT);
             INSERT INTO auth_tokens VALUES (1, 'mbp 的令牌', 'h1', 'bd_1', 0, NULL, 'mbp'), (2, 'win 的令牌', 'h2', 'bd_2', 0, NULL, 'win'),
               (3, '重装 mbp 的令牌', 'h3', 'bd_3', 0, NULL, 'mbp'), (4, 'old 的令牌', 'h4', 'bd_4', 0, NULL, 'old');",
        )?;
        Ok(())
    })
    .unwrap();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let owned = |s: &Store, token: Option<i64>| s.read(|r| r.host_owned_by("mbp", token));
    owned(&s, Some(1)).unwrap();
    // 界面、本机模式（没有令牌）不查
    owned(&s, None).unwrap();
    // 别的令牌：和机器不存在一样
    assert_eq!(owned(&s, Some(2)).unwrap_err().code, ErrorCode::NoSuchHost);
    assert_eq!(s.read(|r| r.host_owned_by("nope", Some(1))).unwrap_err().code, ErrorCode::NoSuchHost);

    // 派发：在 mbp 上的派发只有 mbp 的令牌能结束、写日志
    let id = new_card(&mut s, T0, "派给 mbp 的卡");
    let d = dispatch(&mut s, T0, id, "mbp", None).unwrap();
    begin(&mut s, T0 + S, d.id).unwrap();
    let d_owned = |s: &Store, id: i64, token: Option<i64>| s.read(|r| r.dispatch_owned_by(id, token));
    d_owned(&s, d.id, Some(1)).unwrap();
    d_owned(&s, d.id, None).unwrap();
    assert_eq!(d_owned(&s, d.id, Some(2)).unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(d_owned(&s, 999, Some(1)).unwrap_err().code, ErrorCode::NotFound);

    // 令牌 2 冒充 mbp 上报：在线时、离线后、带 resume 都不行，mbp 仍绑着令牌 1
    let other = |s: &mut Store, at: i64, rep: &HostReport| s.write(at, Origin::cli(), |w| w.host_beat(rep, Some(2)));
    let e = other(&mut s, T0 + 2 * S, &report("mbp", "x1", vec![])).unwrap_err();
    assert_eq!(e.code, ErrorCode::HostNameTaken);
    assert!(e.message.contains("mbp 的令牌"), "{}", e.message);
    let mut off = report("mbp", "i1", vec![]);
    off.done = true;
    beat(&mut s, T0 + 3 * S, &off).unwrap();
    assert!(!host(&s, T0 + 4 * S).online);
    let e = other(&mut s, T0 + 5 * S, &report("mbp", "x1", vec![])).unwrap_err();
    assert_eq!(e.code, ErrorCode::HostNameTaken);
    let mut resume = report("mbp", "x2", vec![]);
    resume.resume = Some("i1".into());
    assert_eq!(other(&mut s, T0 + 5 * S, &resume).unwrap_err().code, ErrorCode::HostNameTaken);
    // 带着 mbp 的进程标识、用别的名字上报（改名后认回的那条路）也不行
    assert_eq!(other(&mut s, T0 + 5 * S, &report("mbp2", "i1", vec![])).unwrap_err().code, ErrorCode::HostNameTaken);
    // final 也不能替它下线
    beat(&mut s, T0 + 6 * S, &report("mbp", "i1", vec![])).unwrap();
    let mut bye = report("mbp", "x1", vec![]);
    bye.done = true;
    assert_eq!(other(&mut s, T0 + 7 * S, &bye).unwrap_err().code, ErrorCode::HostNameTaken);
    assert!(host(&s, T0 + 7 * S).online);
    owned(&s, Some(1)).unwrap();
    assert_eq!(owned(&s, Some(2)).unwrap_err().code, ErrorCode::NoSuchHost);

    // 令牌 1 被吊销后（换了令牌重装这台机器）：换绑也算登记（#507），给 win 的令牌 2 不行，
    // 「添加机器」给 mbp 新建的令牌 3 可以，之后只认令牌 3
    s.write(T0, Origin::ui(None), |w| Ok(w.tx.execute("DELETE FROM auth_tokens WHERE id = 1", [])?)).unwrap();
    let e = other(&mut s, T0 + 60 * S, &report("mbp", "x3", vec![])).unwrap_err();
    assert_eq!(e.code, ErrorCode::HostNameTaken);
    assert!(e.message.contains("添加机器"), "{}", e.message);
    s.write(T0 + 60 * S, Origin::cli(), |w| w.host_beat(&report("mbp", "x3", vec![]), Some(3))).unwrap();
    owned(&s, Some(3)).unwrap();
    assert_eq!(owned(&s, Some(1)).unwrap_err().code, ErrorCode::NoSuchHost);
    assert_eq!(owned(&s, Some(2)).unwrap_err().code, ErrorCode::NoSuchHost);
    d_owned(&s, d.id, Some(3)).unwrap();
    // 没绑过令牌的机器（本机模式上报的）：同样只有绑定它名字的令牌能绑上
    s.write(T0, Origin::cli(), |w| w.host_beat(&report("old", "o1", vec![]), None)).unwrap();
    assert_eq!(s.read(|r| r.host_owned_by("old", Some(2))).unwrap_err().code, ErrorCode::NoSuchHost);
    let mut again = report("old", "o1", vec![]);
    again.resume = Some("o1".into());
    assert_eq!(other(&mut s, T0 + S, &again).unwrap_err().code, ErrorCode::HostNameTaken);
    s.write(T0 + S, Origin::cli(), |w| w.host_beat(&again, Some(4))).unwrap();
    s.read(|r| r.host_owned_by("old", Some(4))).unwrap();
}

/// 登记新机器（#507）：普通访问令牌、绑定了别的机器名的令牌都不能用一个没人用过的名字自注册（不建记录，
/// 之后也读不到只发给机器的密钥）；「添加机器」新建的、绑定这个名字的机器令牌可以。已经绑着普通令牌的机器
/// （升级前连上来的）照常上报；改名后机器令牌绑定的名字跟着改，移除后再连上来照样能登记
#[test]
fn only_a_machine_token_registers_a_new_host() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| {
        w.tx.execute_batch(
            "CREATE TABLE auth_tokens (id INTEGER PRIMARY KEY, name TEXT NOT NULL, hash TEXT NOT NULL, hint TEXT NOT NULL, created_at INTEGER NOT NULL, last_used_at INTEGER, host TEXT);
             INSERT INTO auth_tokens VALUES (1, 'ci', 'h1', 'bd_1', 0, NULL, NULL), (2, 'devbox', 'h2', 'bd_2', 0, NULL, 'devbox'), (3, 'mbp', 'h3', 'bd_3', 0, NULL, 'mbp');",
        )?;
        Ok(())
    })
    .unwrap();
    let up = |s: &mut Store, at: i64, name: &str, inst: &str, t: i64| s.write(at, Origin::cli(), |w| w.host_beat(&report(name, inst, vec![]), Some(t)));
    let count = |s: &Store| s.read(|r| Ok(r.conn.query_row("SELECT COUNT(*) FROM hosts", [], |x| x.get::<_, i64>(0))?)).unwrap();

    // 普通访问令牌：随便一个名字自注册不行，不建记录，也读不到密钥
    let e = up(&mut s, T0, "evil-1", "e1", 1).unwrap_err();
    assert_eq!(e.code, ErrorCode::HostNameTaken);
    assert!(e.message.contains("普通访问令牌") && e.message.contains("机器 → 添加机器"), "{}", e.message);
    assert_eq!(count(&s), 0);
    assert!(!s.read(|r| r.host_uses_token("evil-1", 1, T0)).unwrap());
    // 绑定了别的名字的机器令牌：不行
    let e = up(&mut s, T0, "evil-2", "e2", 2).unwrap_err();
    assert_eq!(e.code, ErrorCode::HostNameTaken);
    assert!(e.message.contains("devbox"), "{}", e.message);
    assert_eq!(count(&s), 0);
    // 结束上报：照旧是机器不存在
    let mut bye = report("evil-3", "e3", vec![]);
    bye.done = true;
    assert_eq!(s.write(T0, Origin::cli(), |w| w.host_beat(&bye, Some(1))).unwrap_err().code, ErrorCode::NoSuchHost);

    // 名字一致的机器令牌：登记，之后能取自己的配置；A 的令牌读不了 B
    up(&mut s, T0, "devbox", "d1", 2).unwrap();
    up(&mut s, T0, "mbp", "m1", 3).unwrap();
    assert!(s.read(|r| r.host_uses_token("devbox", 2, T0)).unwrap());
    assert!(!s.read(|r| r.host_uses_token("mbp", 2, T0)).unwrap());
    assert!(!s.read(|r| r.host_uses_token("devbox", 3, T0)).unwrap());

    // 升级前用普通令牌连上来的机器（已经绑着令牌 1）：照常上报
    s.write(T0, Origin::ui(None), |w| Ok(w.tx.execute("UPDATE hosts SET token = 1 WHERE name = 'mbp'", [])?)).unwrap();
    up(&mut s, T0 + S, "mbp", "m1", 1).unwrap();
    assert!(s.read(|r| r.host_uses_token("mbp", 1, T0 + S)).unwrap());

    // 改名：机器令牌绑定的名字跟着改；移除后用新名字再连上来照样能登记，旧名字不行
    s.write(T0 + S, Origin::ui(None), |w| w.host_rename("devbox", "devbox2")).unwrap();
    let bound: Option<String> = s.read(|r| Ok(r.conn.query_row("SELECT host FROM auth_tokens WHERE id = 2", [], |x| x.get(0))?)).unwrap();
    assert_eq!(bound.as_deref(), Some("devbox2"));
    s.write(T0 + S, Origin::ui(None), |w| w.host_remove("devbox2", &Actor::human(), "人")).unwrap();
    assert_eq!(up(&mut s, T0 + 2 * S, "devbox", "d2", 2).unwrap_err().code, ErrorCode::HostNameTaken);
    up(&mut s, T0 + 2 * S, "devbox2", "d2", 2).unwrap();
}

#[test]
fn host_token_is_shown_and_can_be_revoked_on_remove() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| {
        w.tx.execute_batch(
            "CREATE TABLE auth_tokens (id INTEGER PRIMARY KEY, name TEXT NOT NULL, hash TEXT NOT NULL, hint TEXT NOT NULL, created_at INTEGER NOT NULL, last_used_at INTEGER, host TEXT);
             INSERT INTO auth_tokens VALUES (1, 'win2 的令牌', 'h1', 'brd_ab12', 0, 123, 'mbp'), (2, 'win 的令牌', 'h2', 'brd_cd34', 0, NULL, 'win');",
        )?;
        Ok(())
    })
    .unwrap();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let t = host(&s, T0).token.unwrap();
    assert_eq!((t.id, t.name.as_str(), t.hint.as_str(), t.last_used_at), (1, "win2 的令牌", "brd_ab12", Some(123)));
    // 另一台在线的机器也用这个令牌（升级前两台共用一个普通令牌连上来的）：不能一起吊销
    s.write(T0, Origin::cli(), |w| w.host_beat(&report("win", "w1", vec![]), Some(2))).unwrap();
    s.write(T0, Origin::ui(None), |w| Ok(w.tx.execute("UPDATE hosts SET token = 1 WHERE name = 'win'", [])?)).unwrap();
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
    let left: i64 = s.read(|r| Ok(r.conn.query_row("SELECT COUNT(*) FROM auth_tokens WHERE id = 1", [], |x| x.get(0))?)).unwrap();
    assert_eq!(left, 0);
    assert!(s.read(|r| r.host("mbp", T0)).is_err());
}

#[test]
fn version_order() {
    assert!(hosts::version_lt("0.1.9", "0.1.19"));
    assert!(hosts::version_lt("v0.1.19", "0.2.0"));
    assert!(hosts::version_lt("2.1.300 (Claude Code)", "2.1.325"));
    assert!(hosts::version_lt("0.2.2-rc.3", "0.2.2"));
    assert!(!hosts::version_lt("0.1.19", "0.1.19"));
    assert!(!hosts::version_lt("1.0.0", "0.9.9"));
}

/* ------------------------------ 模型：不改 / 指定（机器详情与配置同步重构） ------------------------------ */

/// 两个供应商：p1（Anthropic，model-a 标了 low、high；model-b 没标）、chat（只有 OpenAI Chat）
fn providers(s: &mut Store) {
    let ep = |i: &str| crate::providers::ProviderEndpoint { interface: i.into(), url: "https://api.example.com".into() };
    let p1 = crate::providers::Provider {
        name: "p1".into(),
        endpoints: vec![ep("anthropic")],
        api_key: Some("sk-1".into()),
        models: vec!["model-a".into(), "model-b".into()],
        efforts: [("model-a".to_string(), vec!["low".to_string(), "high".into()])].into(),
        ..Default::default()
    };
    let chat = crate::providers::Provider { name: "chat".into(), endpoints: vec![ep("openai-chat")], api_key: Some("sk-2".into()), models: vec!["c1".into()], ..Default::default() };
    s.write(T0, Origin::ui(None), |w| {
        w.provider_put(p1, "buglan", "")?;
        w.provider_put(chat, "buglan", "")
    })
    .unwrap();
}

fn set_run(s: &mut Store, v: serde_json::Value) -> Result<HostInfo> {
    let run: RunPatch = serde_json::from_value(v).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { run: Some(run), ..Default::default() }))
}

fn claude_of(h: &HostInfo) -> HostAgent {
    h.agents.iter().find(|a| a.id == "claude").cloned().unwrap()
}

#[test]
fn model_modes_default_to_off_and_are_validated() {
    let mut s = store();
    providers(&mut s);
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    // 新机器：全部不改（给旧版 board host 的写法是 @local）
    let h = s.read(|r| r.host("mbp", T0)).unwrap();
    let a = claude_of(&h);
    assert_eq!((a.run_mode.as_deref(), a.run_provider.as_deref()), (Some("off"), Some("@local")));
    assert!(!h.sync_skills && !h.sync_mcp, "skills、MCP 也是不改");
    // 没有「用服务器的」了
    assert!(set_run(&mut s, serde_json::json!({ "agent": "claude", "mode": "server" })).unwrap_err().message.contains("只能是"));
    // 指定：模型留空用第一个；强度要是模型标了的、claude 能写的
    let a = claude_of(&set_run(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "p1" })).unwrap());
    assert_eq!((a.run_mode.as_deref(), a.run_provider.as_deref(), a.run_model.as_deref(), a.run_effort.as_deref()), (Some("own"), Some("p1"), Some("model-a"), None));
    let a = claude_of(&set_run(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "p1", "model": "model-a", "effort": "high" })).unwrap());
    assert_eq!(a.run_effort.as_deref(), Some("high"));
    // 只改强度：供应商、模型不动
    let a = claude_of(&set_run(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "effort": "low" })).unwrap());
    assert_eq!((a.run_provider.as_deref(), a.run_model.as_deref(), a.run_effort.as_deref()), (Some("p1"), Some("model-a"), Some("low")));
    let bad = |s: &mut Store, v: serde_json::Value| set_run(s, v).unwrap_err().message;
    assert!(bad(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "p1", "model": "model-b", "effort": "high" })).contains("不能选强度"), "model-b 没标强度");
    assert!(bad(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "p1", "model": "model-a", "effort": "max" })).contains("low、high"));
    assert!(bad(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "chat" })).contains("接口"), "claude 只认 Anthropic");
    assert!(bad(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "nope" })).contains("没有模型供应商"));
    assert!(bad(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "p1", "model": "zzz" })).contains("没有模型"));
    assert!(bad(&mut s, serde_json::json!({ "agent": "claude", "mode": "later" })).contains("只能是"));
    assert!(bad(&mut s, serde_json::json!({ "agent": "kimi", "mode": "off" })).contains("内置"));
    // 旧界面只带供应商：空、@local = 不改
    let a = claude_of(&set_run(&mut s, serde_json::json!({ "agent": "claude", "provider": "@local" })).unwrap());
    assert_eq!((a.run_mode.as_deref(), a.run_model.as_deref()), (Some("off"), None), "改成不改：指定的清掉");
    set_run(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "p1" })).unwrap();
    assert_eq!(claude_of(&set_run(&mut s, serde_json::json!({ "agent": "claude", "provider": "" })).unwrap()).run_mode.as_deref(), Some("off"));
}

#[test]
fn the_interface_can_be_chosen_when_agent_and_provider_both_have_several() {
    let mut s = store();
    providers(&mut s);
    let mut rep = report("mbp", "i1", vec![]);
    rep.agents.push(dsh(true));
    beat(&mut s, T0, &rep).unwrap();
    let ep = |i: &str, u: &str| crate::providers::ProviderEndpoint { interface: i.into(), url: u.into() };
    let both = crate::providers::Provider {
        name: "both".into(),
        endpoints: vec![ep("anthropic", "https://relay.example.com"), ep("openai-chat", "https://relay.example.com/v1")],
        api_key: Some("sk-3".into()),
        models: vec!["b1".into()],
        ..Default::default()
    };
    s.write(T0, Origin::ui(None), |w| w.provider_put(both, "buglan", "")).unwrap();
    let dsh_of = |h: &HostInfo| h.agents.iter().find(|a| a.id == "dsh").cloned().unwrap();
    // 不指定：空，board host 用第一个对得上的
    let a = dsh_of(&set_run(&mut s, serde_json::json!({ "agent": "dsh", "mode": "own", "provider": "both" })).unwrap());
    assert_eq!(a.run_protocol, None);
    // 指定 OpenAI Chat；只改强度、模型时留着
    let a = dsh_of(&set_run(&mut s, serde_json::json!({ "agent": "dsh", "mode": "own", "provider": "both", "protocol": "openai-chat" })).unwrap());
    assert_eq!(a.run_protocol.as_deref(), Some("openai-chat"));
    let a = dsh_of(&set_run(&mut s, serde_json::json!({ "agent": "dsh", "mode": "own", "model": "b1", "effort": "" })).unwrap());
    assert_eq!(a.run_protocol.as_deref(), Some("openai-chat"));
    // 供应商没给的、agent 不认的：拒绝
    let bad = |s: &mut Store, v: serde_json::Value| set_run(s, v).unwrap_err().message;
    assert!(bad(&mut s, serde_json::json!({ "agent": "dsh", "mode": "own", "provider": "both", "protocol": "openai-responses" })).contains("接口只能是"));
    assert!(bad(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "both", "protocol": "openai-chat" })).contains("Anthropic Messages"));
    // 换供应商：原来指定的接口清掉；空 = 回到第一个对得上的
    let a = dsh_of(&set_run(&mut s, serde_json::json!({ "agent": "dsh", "mode": "own", "provider": "chat" })).unwrap());
    assert_eq!((a.run_provider.as_deref(), a.run_protocol.as_deref()), (Some("chat"), None));
    set_run(&mut s, serde_json::json!({ "agent": "dsh", "mode": "own", "provider": "both", "protocol": "anthropic" })).unwrap();
    let a = dsh_of(&set_run(&mut s, serde_json::json!({ "agent": "dsh", "mode": "own", "provider": "both", "protocol": "" })).unwrap());
    assert_eq!(a.run_protocol, None);
    // 改成不改：清掉
    set_run(&mut s, serde_json::json!({ "agent": "dsh", "mode": "own", "provider": "both", "protocol": "openai-chat" })).unwrap();
    let key = s.read(|r| r.host_work("mbp", T0)).unwrap().config_key;
    set_run(&mut s, serde_json::json!({ "agent": "dsh", "mode": "own", "provider": "both", "protocol": "anthropic" })).unwrap();
    assert_ne!(s.read(|r| r.host_work("mbp", T0)).unwrap().config_key, key, "换接口要重新写");
    let a = dsh_of(&set_run(&mut s, serde_json::json!({ "agent": "dsh", "mode": "off" })).unwrap());
    assert_eq!((a.run_mode.as_deref(), a.run_protocol.as_deref()), (Some("off"), None));
}

#[test]
fn dispatch_records_the_model_the_agent_config_has() {
    let mut s = store();
    // board host 上报：claude 配置文件里现在是 sonnet、强度 high（同步写进去的，或人配的）
    let mut rep = report("mbp", "i1", vec![]);
    rep.agents[0].model = "sonnet".into();
    rep.agents[0].effort = "high".into();
    beat(&mut s, T0, &rep).unwrap();
    let id = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, id, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!((d.model.as_deref(), d.effort.as_deref()), (Some("sonnet"), Some("high")));
    assert_eq!(d.name.as_deref(), Some("mbp/sonnet-high"), "名字第二段是配置文件里的模型");
    let acts = card(&s, id).activity;
    assert!(acts.iter().any(|a| a.text.contains("派发开始") && a.text.contains("模型 sonnet，强度 high")), "{acts:?}");
    end(&mut s, T0 + 2 * S, d.id, Some(0), &[]);
    // 读不到模型（没配）：名字只写 agent 标识
    let mut rep = report("mbp", "i1", vec![]);
    rep.agents[0].model = String::new();
    beat(&mut s, T0 + 3 * S, &rep).unwrap();
    let id2 = new_card(&mut s, T0, "B");
    let d2 = dispatch(&mut s, T0 + 3 * S, id2, "mbp", None).unwrap();
    let d2 = begin(&mut s, T0 + 3 * S, d2.id).unwrap();
    assert_eq!((d2.model, d2.effort, d2.name.as_deref()), (None, None, Some("mbp/claude")));
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
    assert_eq!(selftest(&mut s, T0, Some("mycli")).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(selftest(&mut s, T0, Some("codex")).unwrap_err().code, ErrorCode::Usage);
    let h = selftest(&mut s, T0 + S, None).unwrap();
    let ask = h.selftest.asks.first().cloned().expect("记下请求");
    assert_eq!((ask.agent.as_deref(), ask.by.as_str()), (None, "octocat"));
    // 取任务时 board host 能看到请求
    let w = s.read(|r| r.host_work("mbp", T0 + 2 * S)).unwrap();
    assert_eq!(w.host.selftest.asks.iter().map(|a| a.id).collect::<Vec<_>>(), vec![ask.id]);
    // 整台机器的自检没做完时不能再单查一个 agent
    let e = selftest(&mut s, T0 + 2 * S, Some("claude")).unwrap_err();
    assert!(e.message.contains("正在整台自检"), "{}", e.message);
    // 别的进程、序号对不上的回报不算
    let h = selftest_done(&mut s, T0 + 3 * S, "i2", ask.id, vec![item("agent", "claude", true, None)]);
    assert!(h.selftest.asks.len() == 1 && h.selftest.items.is_empty());
    let h = selftest_done(&mut s, T0 + 3 * S, "i1", ask.id + 5, vec![item("agent", "claude", true, None)]);
    assert_eq!(h.selftest.asks.len(), 1);
    // 做完：agent、git 两项 + 服务端补上版本（0.1.1 落后于服务端）
    let h = selftest_done(
        &mut s,
        T0 + 4 * S,
        "i1",
        ask.id,
        vec![item("agent", "claude", true, None), item("git", B, false, Some("没有推送权限")), item("bogus", "x", true, None)],
    );
    assert!(h.selftest.asks.is_empty());
    let kinds: Vec<(&str, bool)> = h.selftest.items.iter().map(|i| (i.kind.as_str(), i.ok)).collect();
    assert_eq!(kinds, vec![("agent", true), ("git", false), ("version", false)]);
    assert!(h.selftest.items.iter().all(|i| i.at == T0 + 4 * S), "检查时间是服务端收到的时间");
    let v = h.selftest.items.iter().find(|i| i.kind == "version").unwrap();
    assert!(v.why.as_deref().unwrap().contains("0.1.1") && v.why.as_deref().unwrap().contains(env!("CARGO_PKG_VERSION")));
    assert!(h.ops.iter().any(|o| o.text.contains("没通过") && o.text.contains(B)), "{:?}", h.ops);
    // 只查一个 agent：只换它那一项，git 的结果保留到下一次整机自检
    let ask = selftest(&mut s, T0 + 5 * S, Some("claude")).unwrap().selftest.asks[0].clone();
    assert_eq!(ask.agent.as_deref(), Some("claude"));
    let h = selftest_done(&mut s, T0 + 6 * S, "i1", ask.id, vec![item("agent", "claude", false, Some("没有登录")), item("git", B, true, None)]);
    let got: Vec<(&str, bool, i64)> = h.selftest.items.iter().map(|i| (i.kind.as_str(), i.ok, i.at)).collect();
    assert_eq!(got, vec![("agent", false, T0 + 6 * S), ("git", false, T0 + 4 * S), ("version", false, T0 + 6 * S)]);
}

#[test]
fn selftest_needs_online_host_and_stale_request_can_be_replaced() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let first = selftest(&mut s, T0, None).unwrap().selftest.asks[0].clone();
    // 一直没做完（board host 卡住、重启丢了）：超过期限后可以重新请求
    beat(&mut s, T0 + hosts::SELFTEST_STALE_MS, &report("mbp", "i1", vec![])).unwrap();
    let again = selftest(&mut s, T0 + hosts::SELFTEST_STALE_MS + S, None).unwrap().selftest;
    assert_eq!(again.asks.len(), 1, "过期的请求换掉了：{:?}", again.asks);
    assert!(again.asks[0].id > first.id);
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

/// 同一台机器同时查几个不同的 agent（default/main#487）：请求都记下，结果各自回来只换各自那一项；
/// 同一个 agent 不能重复请求；有单查没做完时不能整台自检；旧版 board host 看到的 `ask` 是最早的一个
#[test]
fn selftest_agents_run_concurrently() {
    let mut s = store();
    let mut rep = report("mbp", "i1", vec![]);
    rep.agents.push(HostAgent { id: "codex".into(), name: "Codex".into(), ..claude(true) });
    beat(&mut s, T0, &rep).unwrap();
    // 先整台做一次：两个 agent、git 都有结果
    let all = selftest(&mut s, T0, None).unwrap().selftest.asks[0].id;
    selftest_done(&mut s, T0 + S, "i1", all, vec![item("agent", "claude", true, None), item("agent", "codex", true, None), item("git", B, true, None)]);

    let a = selftest(&mut s, T0 + 2 * S, Some("claude")).unwrap().selftest.asks[0].clone();
    let h = selftest(&mut s, T0 + 3 * S, Some("codex")).unwrap();
    let ids: Vec<(i64, Option<&str>)> = h.selftest.asks.iter().map(|x| (x.id, x.agent.as_deref())).collect();
    assert_eq!(ids.len(), 2, "两个请求同时在：{ids:?}");
    let b = h.selftest.asks[1].clone();
    assert_eq!((a.agent.as_deref(), b.agent.as_deref()), (Some("claude"), Some("codex")));
    // 同一个 agent 正在自检：拒绝
    let e = selftest(&mut s, T0 + 4 * S, Some("claude")).unwrap_err();
    assert!(e.message.contains("正在自检 Claude Code"), "{}", e.message);
    // 有单查没做完：不能整台自检，写明在查哪几个
    let e = selftest(&mut s, T0 + 4 * S, None).unwrap_err();
    assert!(e.message.contains("Claude Code、Codex"), "{}", e.message);
    // 发给 board host、界面的 JSON：asks 两个，ask 是最早的那个（旧版 board host 一次做一个）
    let v = serde_json::to_value(&h.selftest).unwrap();
    assert_eq!(v["asks"].as_array().map(Vec::len), Some(2));
    assert_eq!(v["ask"]["id"], a.id);
    // 后请求的先做完：只换 codex 那一项，claude、git 的旧结果留着，claude 的请求还在
    let h = selftest_done(&mut s, T0 + 5 * S, "i1", b.id, vec![item("agent", "codex", false, Some("没有登录"))]);
    assert_eq!(h.selftest.asks.iter().map(|x| x.id).collect::<Vec<_>>(), vec![a.id]);
    let got = |h: &HostInfo| -> Vec<(String, String, bool, i64)> { h.selftest.items.iter().map(|i| (i.kind.clone(), i.key.clone(), i.ok, i.at)).collect() };
    let row = |k: &str, key: &str, ok: bool, at: i64| (k.to_string(), key.to_string(), ok, at);
    assert_eq!(
        got(&h),
        vec![row("agent", "claude", true, T0 + S), row("agent", "codex", false, T0 + 5 * S), row("git", B, true, T0 + S), row("version", "board", false, T0 + 5 * S)]
    );
    let h = selftest_done(&mut s, T0 + 6 * S, "i1", a.id, vec![item("agent", "claude", false, Some("额度用完"))]);
    assert!(h.selftest.asks.is_empty());
    assert_eq!(
        got(&h),
        vec![row("agent", "claude", false, T0 + 6 * S), row("agent", "codex", false, T0 + 5 * S), row("git", B, true, T0 + S), row("version", "board", false, T0 + 6 * S)]
    );
    // 都做完了：又能整台自检
    assert!(selftest(&mut s, T0 + 7 * S, None).is_ok());
}

/// 旧数据（只有一个 `ask`）读出来是一个请求；没有请求时 JSON 里不带 asks、ask
#[test]
fn selftest_reads_old_single_ask() {
    let st: hosts::SelfTest = serde_json::from_value(serde_json::json!({ "able": true, "ask": { "id": 7, "agent": "claude", "by": "octocat", "at": 1 }, "items": [] })).unwrap();
    assert_eq!(st.asks.iter().map(|a| (a.id, a.agent.as_deref())).collect::<Vec<_>>(), vec![(7, Some("claude"))]);
    let v = serde_json::to_value(hosts::SelfTest { able: true, ..Default::default() }).unwrap();
    assert!(v.get("asks").is_none() && v.get("ask").is_none(), "{v}");
    // 往返一次还是同样的请求（存进数据库再读出来不会变成两个）
    let back: hosts::SelfTest = serde_json::from_value(serde_json::to_value(&st).unwrap()).unwrap();
    assert_eq!(back, st);
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

/* ------------------------------ 同步配置（机器详情与配置同步重构 §三） ------------------------------ */

fn sync_of(s: &Store, name: &str) -> ConfigSync {
    s.read(|r| r.host(name, T0)).unwrap().sync
}

fn synced(s: &mut Store, name: &str, instance: &str, id: Option<i64>, key: &str, agents: Vec<crate::providers::ProviderState>) -> Result<HostInfo> {
    let res = SyncResult {
        id,
        key: key.into(),
        agents,
        skills: SkillsSync { state: "synced".into(), count: 3, detail: SkillsDetail { replaced: vec!["frontend-design".into()], ..Default::default() }, ..Default::default() },
        mcp: McpSync { state: "synced".into(), count: 2, detail: McpDetail { agents: vec!["Claude Code".into()], ..Default::default() }, ..Default::default() },
        ..Default::default()
    };
    s.write(T0, Origin::cli(), |w| w.host_synced(name, instance, res))
}

#[test]
fn config_changes_ask_affected_hosts_to_sync_and_reports_clear_it() {
    let mut s = store();
    providers(&mut s);
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let mut old = report("old", "i2", vec![]);
    old.config_sync = false;
    beat(&mut s, T0, &old).unwrap();
    // board host 启动时自己写一次：带着任务里的 key 回报
    let key = s.read(|r| r.host_work("mbp", T0)).unwrap().config_key;
    synced(&mut s, "mbp", "i1", None, &key, vec![]).unwrap();
    let cs = sync_of(&s, "mbp");
    assert!(cs.ask.is_none() && cs.last.as_ref().is_some_and(|l| l.key == key));
    // 都是不改：改供应商、skills 库不影响它
    s.write(T0, Origin::ui(None), |w| w.skill_put("demo", &[crate::skills::SkillFile { path: "SKILL.md".into(), exec: false, data: crate::skills::b64(b"# demo") }], "buglan", "")).unwrap();
    assert!(sync_of(&s, "mbp").ask.is_none(), "skills 不改：库变了不用同步");
    // 机器改成 skills 用服务器的：下发请求
    s.write(T0, Origin::ui(None), |w| w.host_update_as("mbp", &HostPatch { sync_skills: Some(true), ..Default::default() }, "buglan")).unwrap();
    let ask = sync_of(&s, "mbp").ask.expect("要同步");
    assert_eq!(ask.by, "buglan");
    assert!(sync_of(&s, "old").ask.is_none(), "旧版 board host 不发请求");
    // 再改一个：等着的请求换成新的样子
    let chat = crate::providers::Provider {
        name: "chat".into(),
        endpoints: vec![crate::providers::ProviderEndpoint { interface: "openai-chat".into(), url: "https://api.example.com/v1".into() }],
        models: vec!["c1".into(), "c2".into()],
        ..Default::default()
    };
    s.write(T0, Origin::ui(None), |w| w.provider_put(chat, "buglan", "")).unwrap();
    assert_eq!(sync_of(&s, "mbp").ask.unwrap().key, ask.key, "claude 不改：供应商变了不影响");
    set_run(&mut s, serde_json::json!({ "agent": "claude", "mode": "own", "provider": "p1" })).unwrap();
    let ask2 = sync_of(&s, "mbp").ask.unwrap();
    assert_ne!(ask2.key, ask.key);
    assert!(ask2.id > ask.id);
    // 回报：清掉请求，记一条操作记录
    let st = crate::providers::ProviderState { agent: "claude".into(), state: "synced".into(), provider: "p1".into(), model: "model-a".into(), ..Default::default() };
    let h = synced(&mut s, "mbp", "i1", Some(ask2.id), &ask2.key, vec![st]).unwrap();
    assert!(h.sync.ask.is_none());
    assert_eq!(h.sync.last.as_ref().unwrap().agents.len(), 1);
    let op = &h.ops[0];
    assert_eq!(op.text, "同步配置：1 个 agent 已写入；3 个 skill、2 个 MCP 已写入，替换了本机同名的 frontend-design");
    assert_eq!(op.by, "人", "改模型的是界面上的人");
    // 改供应商的强度：用着它的机器要重新写
    let ep = crate::providers::ProviderEndpoint { interface: "anthropic".into(), url: "https://api.example.com".into() };
    s.write(T0, Origin::ui(None), |w| w.provider_put(crate::providers::Provider { name: "p1".into(), endpoints: vec![ep], api_key: None, models: vec!["model-a".into()], ..Default::default() }, "buglan", "")).unwrap();
    let ask3 = sync_of(&s, "mbp").ask.expect("供应商变了");
    // 写的时候又改了：结果的 key 对不上，再发一次
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { sync_skills: Some(false), ..Default::default() })).unwrap();
    let ask4 = sync_of(&s, "mbp").ask.unwrap();
    assert_ne!(ask4.key, ask3.key);
    let h = synced(&mut s, "mbp", "i1", Some(ask3.id), &ask3.key, vec![]).unwrap();
    assert!(h.sync.ask.as_ref().is_some_and(|a| a.key == ask4.key), "还要按最新的再写一次");
    // 换了进程的旧结果不收
    assert!(synced(&mut s, "mbp", "i9", Some(ask4.id), &ask4.key, vec![]).is_err());
    // 重试：样子没变也发
    let h = s.write(T0, Origin::ui(None), |w| w.host_sync_retry("mbp", "buglan")).unwrap();
    assert!(h.sync.ask.unwrap().id > ask4.id);
    assert!(s.write(T0, Origin::ui(None), |w| w.host_sync_retry("old", "buglan")).unwrap_err().message.contains("旧版"));
}

/// 升级：已有的机器保持原来的效果（skills、MCP 用服务器的；跟共用配置的 agent 指定共用配置里给它的那一组、没配的不改，
/// 本机自己管的不改，单独指定了供应商的指定），派发时的模型、强度清掉；跑多久停取机器里最常见的；agent 模板的表删掉
#[test]
fn old_hosts_keep_syncing_after_upgrade_and_run_limits_become_global() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        providers(&mut s);
        for n in ["mbp", "win", "air"] {
            let mut rep = report(n, &format!("i-{n}"), vec![]);
            rep.agents.push(dsh(true));
            rep.agents.push(HostAgent { id: "kimi".into(), name: "kimi".into(), found: true, ..Default::default() });
            beat(&mut s, T0, &rep).unwrap();
        }
    }
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            r#"ALTER TABLE hosts DROP COLUMN sync_skills;
               ALTER TABLE sched DROP COLUMN max_run;
               ALTER TABLE sched DROP COLUMN idle_run;
               CREATE TABLE agent_templates (id TEXT PRIMARY KEY, spec TEXT NOT NULL, version TEXT NOT NULL, updated_at INTEGER NOT NULL, by_name TEXT NOT NULL);
               CREATE TABLE provider_agents (agent TEXT PRIMARY KEY, spec TEXT NOT NULL, updated_at INTEGER NOT NULL, by_name TEXT NOT NULL);
               INSERT INTO provider_agents VALUES ('claude', '{"agent":"claude","provider":"p1","model":"model-a","effort":"low"}', 0, 'buglan');
               UPDATE hosts SET max_run = 60, idle_run = 15 WHERE name IN ('mbp', 'win');
               UPDATE hosts SET max_run = 240, idle_run = 30 WHERE name = 'air';
               UPDATE hosts SET run = '{"claude":{"env":"login","model":"sonnet","effort":"high"},"dsh":{"provider":"@local"},"kimi":{"model":"k2"}}' WHERE name = 'mbp';
               UPDATE hosts SET run = '{"claude":{"provider":"p1","model":"model-b"}}' WHERE name = 'win';"#,
        )
        .unwrap();
    let s = Store::open(&path).unwrap();
    let h = |n: &str| s.read(|r| r.host(n, T0)).unwrap();
    let mode = |h: &HostInfo, a: &str| h.agents.iter().find(|x| x.id == a).and_then(|x| x.run_mode.clone()).unwrap();
    for n in ["mbp", "win", "air"] {
        assert!(h(n).sync_skills && h(n).sync_mcp, "{n}：以前都同步，记成用服务器的");
    }
    let mbp = h("mbp");
    assert_eq!((mode(&mbp, "claude").as_str(), mode(&mbp, "dsh").as_str()), ("own", "off"));
    let c = mbp.agents.iter().find(|x| x.id == "claude").unwrap();
    assert_eq!(
        (c.run_env.as_deref(), c.run_provider.as_deref(), c.run_model.as_deref(), c.run_effort.as_deref()),
        (Some("login"), Some("p1"), Some("model-a"), Some("low")),
        "启动环境留着；跟共用配置的指定成那一组（派发时的 sonnet、high 不要了）"
    );
    assert_eq!(mode(&mbp, "kimi").as_str(), "off", "配置文件里新加的 agent 不按供应商写");
    let win = h("win");
    let c = win.agents.iter().find(|x| x.id == "claude").unwrap();
    assert_eq!((c.run_mode.as_deref(), c.run_provider.as_deref(), c.run_model.as_deref()), (Some("own"), Some("p1"), Some("model-b")));
    assert_eq!(mode(&win, "dsh").as_str(), "off", "没设过的跟共用配置，共用配置里没给 dsh 配：不改");
    let gone: i64 = s.conn().query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'provider_agents'", [], |r| r.get(0)).unwrap();
    assert_eq!(gone, 0, "服务器上的分配用完就删");
    let sched = s.read(|r| r.sched(T0)).unwrap();
    assert_eq!((sched.max_run, sched.idle_run), (60, 15), "两台是 60 / 15");
    assert_eq!(h("air").max_run, 60);
    let gone: i64 = s.conn().query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'agent_templates'", [], |r| r.get(0)).unwrap();
    assert_eq!(gone, 0);
    drop(s);
    // 再打开一次不重复迁移：新设的不改
    let mut s = Store::open(&path).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { sync_skills: Some(false), ..Default::default() })).unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    assert!(!s.read(|r| r.host("mbp", T0)).unwrap().sync_skills);
}

/// 去掉「用服务器的」：选了它的 agent 改成指定服务器上那一组（机器上的效果不变），服务器上没配的改成不改；
/// 表删掉，再打开不重复
#[test]
fn server_mode_becomes_own_with_the_old_assignment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v029.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        providers(&mut s);
        let mut rep = report("mbp", "i1", vec![]);
        rep.agents.push(dsh(true));
        beat(&mut s, T0, &rep).unwrap();
    }
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            r#"CREATE TABLE provider_agents (agent TEXT PRIMARY KEY, spec TEXT NOT NULL, updated_at INTEGER NOT NULL, by_name TEXT NOT NULL);
               INSERT INTO provider_agents VALUES ('claude', '{"agent":"claude","provider":"p1","model":"model-a","effort":"high"}', 0, 'buglan');
               INSERT INTO provider_agents VALUES ('dsh', '{"agent":"dsh","provider":"","model":"","effort":""}', 0, 'buglan');
               UPDATE hosts SET run = '{"claude":{"mode":"server","env":"login"},"dsh":{"mode":"server"}}' WHERE name = 'mbp';"#,
        )
        .unwrap();
    let s = Store::open(&path).unwrap();
    let h = s.read(|r| r.host("mbp", T0)).unwrap();
    let c = claude_of(&h);
    assert_eq!(
        (c.run_mode.as_deref(), c.run_provider.as_deref(), c.run_model.as_deref(), c.run_effort.as_deref(), c.run_env.as_deref()),
        (Some("own"), Some("p1"), Some("model-a"), Some("high"), Some("login"))
    );
    let d = h.agents.iter().find(|a| a.id == "dsh").unwrap();
    assert_eq!((d.run_mode.as_deref(), d.run_provider.as_deref()), (Some("off"), Some("@local")), "服务器上没给它配：不改");
    let left: i64 = s.conn().query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'provider_agents'", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0);
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(claude_of(&s.read(|r| r.host("mbp", T0)).unwrap()).run_model.as_deref(), Some("model-a"), "再打开不变");
}

#[test]
fn sync_words_name_failures_and_off() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let res = SyncResult {
        agents: vec![
            crate::providers::ProviderState { agent: "claude".into(), state: "synced".into(), ..Default::default() },
            crate::providers::ProviderState { agent: "mycli".into(), state: "failed".into(), why: Some("x".into()), ..Default::default() },
            crate::providers::ProviderState { agent: "dsh".into(), state: "off".into(), ..Default::default() },
        ],
        skills: SkillsSync { state: "off".into(), count: 1, ..Default::default() },
        mcp: McpSync { state: "off".into(), ..Default::default() },
        ..Default::default()
    };
    let h = s.write(T0, Origin::cli(), |w| w.host_synced("mbp", "i1", res)).unwrap();
    assert_eq!(h.ops[0].text, "同步配置：1 个 agent 已写入，mycli 失败；只装 board-skills");

    // 三类都失败：各自的说法，Git 凭据单独一段；没报原因时用各自的默认说法
    let res = SyncResult {
        skills: SkillsSync { state: "failed".into(), why: Some("没有权限".into()), ..Default::default() },
        mcp: McpSync { state: "failed".into(), ..Default::default() },
        git: GitSync { state: "failed".into(), ..Default::default() },
        ..Default::default()
    };
    let h = s.write(T0, Origin::cli(), |w| w.host_synced("mbp", "i1", res)).unwrap();
    assert_eq!(h.ops[0].text, "同步配置：agent 的配置都不改；skills 失败（没有权限）、MCP 失败（没写进去）；Git 凭据失败（没配上）");

    // MCP 换掉了本机同名的、skills 不改：替换的照样写在库那一段末尾；Git 凭据服务端没设时不写
    let res = SyncResult {
        skills: SkillsSync { state: "off".into(), count: 1, ..Default::default() },
        mcp: McpSync { state: "synced".into(), count: 2, detail: McpDetail { replaced: vec!["github".into()], ..Default::default() }, ..Default::default() },
        git: GitSync { state: "none".into(), ..Default::default() },
        ..Default::default()
    };
    let h = s.write(T0, Origin::cli(), |w| w.host_synced("mbp", "i1", res)).unwrap();
    assert_eq!(h.ops[0].text, "同步配置：agent 的配置都不改；只装 board-skills、2 个 MCP 已写入，替换了本机同名的 github");
}

#[test]
fn sync_result_json_keeps_field_names() {
    // 旧版 board host 的上报：skills、MCP 的字段平铺在一层，没有 Git 凭据这一行
    let old = r#"{"key":"k","agents":[],
        "skills":{"state":"synced","count":3,"dirs":["~/.claude/skills"],"replaced":["design"],"links":["dev"]},
        "mcp":{"state":"failed","count":1,"agents":["Claude Code"],"replaced":[],"why":"x"}}"#;
    let r: SyncResult = serde_json::from_str(old).unwrap();
    assert_eq!((r.skills.state.as_str(), r.skills.count), ("synced", 3));
    assert_eq!(r.skills.detail, SkillsDetail { dirs: vec!["~/.claude/skills".into()], replaced: vec!["design".into()], links: vec!["dev".into()] });
    assert_eq!((r.mcp.state.as_str(), r.mcp.why.as_deref()), ("failed", Some("x")));
    assert_eq!(r.mcp.detail.agents, vec!["Claude Code".to_string()]);
    assert_eq!(r.git, GitSync::default(), "旧版不报 Git 凭据：状态是空的");
    // 拆开之前的 board host 报的 Git 凭据没有数量：按 0 读，明细照样在
    let g: GitSync = serde_json::from_str(r#"{"state":"synced","host":"github.com","repos":["/w/b"],"skipped":[]}"#).unwrap();
    assert_eq!((g.count, g.detail.host.as_str(), g.detail.repos.len()), (0, "github.com", 1));

    // 新版的上报：明细和共同的部分平铺在同一层，字段名和拆开之前一样（旧服务端按原来的结构读）
    let git = GitSync { state: "synced".into(), count: 1, why: None, detail: GitDetail { host: "github.com".into(), repos: vec!["/w/b".into()], skipped: vec![] } };
    let v = serde_json::to_value(SyncResult { git, ..r }).unwrap();
    assert_eq!(v["skills"], serde_json::json!({ "state": "synced", "count": 3, "dirs": ["~/.claude/skills"], "replaced": ["design"], "links": ["dev"] }));
    assert_eq!(v["mcp"], serde_json::json!({ "state": "failed", "count": 1, "agents": ["Claude Code"], "replaced": [], "why": "x" }));
    assert_eq!(v["git"], serde_json::json!({ "state": "synced", "count": 1, "host": "github.com", "repos": ["/w/b"], "skipped": [] }));
}

#[test]
fn run_limits_are_one_value_for_every_host() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    beat(&mut s, T0, &report("win", "i2", vec![])).unwrap();
    let w = s.read(|r| r.host_work("mbp", T0)).unwrap();
    assert_eq!((w.host.max_run, w.host.idle_run), (MAX_RUN_DEFAULT, IDLE_RUN_DEFAULT));
    let sched = s.write(T0, Origin::ui(None), |w| w.set_run_limits(Some(30), Some(5))).unwrap();
    assert_eq!((sched.max_run, sched.idle_run), (30, 5));
    for name in ["mbp", "win"] {
        let w = s.read(|r| r.host_work(name, T0)).unwrap();
        assert_eq!((w.host.max_run, w.host.idle_run), (30, 5), "{name}：board host 照旧从机器设置里读");
    }
    // 旧界面从机器设置里改：改的是全局的
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max_run: Some(0), ..Default::default() })).unwrap();
    assert_eq!(s.read(|r| r.host("win", T0)).unwrap().max_run, 0);
    assert!(s.write(T0, Origin::ui(None), |w| w.set_run_limits(Some(-1), None)).is_err());
    assert!(s.write(T0, Origin::ui(None), |w| w.set_run_limits(None, Some(IDLE_RUN_LIMIT + 1))).is_err());
}

/* ------------------------------ 拆解「未整理」里的条目（PRD §9.1） ------------------------------ */

fn inbox_card(s: &mut Store, at: i64, who: &Actor, title: &str, from: Option<i64>) -> Result<CardDetail> {
    exec(s, at, who, Cmd::New { title: title.into(), body: None, column: Some(INBOX.into()), top: false, labels: vec![], from })
}

#[test]
fn new_from_records_split_relation_both_ways() {
    let mut s = store();
    let a = inbox_card(&mut s, T0, &Actor::human(), "原始需求", None).unwrap().card.id;
    let x = inbox_card(&mut s, T0 + S, &Actor::agent("mbp/opus-5.5"), "拆出来的一", Some(a)).unwrap();
    assert_eq!((x.card.column.as_str(), x.card.split_from), (INBOX, Some(a)), "子卡在未整理，记着拆自");
    assert!(x.activity.last().unwrap().text.contains(&format!("拆自 #{a}")));
    let y = exec(&mut s, T0 + 2 * S, &Actor::human(), Cmd::New { title: "拆出来的二".into(), body: None, column: None, top: false, labels: vec![], from: Some(a) })
        .unwrap()
        .card
        .id;
    assert_eq!(card(&s, a).split_into, vec![x.card.id, y], "原卡列出拆出的卡，按编号");
    assert!(card(&s, x.card.id).split_into.is_empty());
    // 不在这块白板上：NOT_FOUND，不建卡
    let e = inbox_card(&mut s, T0, &Actor::human(), "没有原卡", Some(9999)).unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    // 原卡进回收站：照样能 --from；子卡留着编号，原卡的「拆出」不含回收站里的
    exec(&mut s, T0 + 3 * S, &Actor::human(), Cmd::Delete { id: a }).unwrap();
    let z = inbox_card(&mut s, T0 + 4 * S, &Actor::human(), "回收站里的原卡也行", Some(a)).unwrap();
    assert_eq!(z.card.split_from, Some(a));
    exec(&mut s, T0 + 5 * S, &Actor::human(), Cmd::Delete { id: y }).unwrap();
    exec(&mut s, T0 + 6 * S, &Actor::human(), Cmd::Purge { id: a }).unwrap();
    assert_eq!(card(&s, x.card.id).card.split_from, Some(a), "原卡永久删除后子卡还留着编号");
}

#[test]
fn inbox_count_leaves_out_entries_already_split() {
    let mut s = store();
    let inbox = |s: &Store| s.read(|r| r.workspace(T0)).unwrap().projects.iter().flat_map(|p| p.boards.iter()).map(|b| b.counts[INBOX]).sum::<i64>();
    let a = inbox_card(&mut s, T0, &Actor::human(), "原始需求", None).unwrap().card.id;
    inbox_card(&mut s, T0, &Actor::human(), "别的一条", None).unwrap();
    assert_eq!(inbox(&s), 2);
    let x = inbox_card(&mut s, T0 + S, &Actor::agent("mbp/opus-5.5"), "拆出来的一", Some(a)).unwrap().card.id;
    let y = inbox_card(&mut s, T0 + S, &Actor::agent("mbp/opus-5.5"), "拆出来的二", Some(a)).unwrap().card.id;
    assert_eq!(inbox(&s), 3, "原条目并进拆出的那组，不单独算");
    exec(&mut s, T0 + 2 * S, &Actor::human(), Cmd::Move { id: x, column: "todo".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    assert_eq!(inbox(&s), 2, "拆出的卡整理走了，原条目照样不算");
    // 撤销拆解 = 删掉拆出的卡：原条目回来
    exec(&mut s, T0 + 3 * S, &Actor::human(), Cmd::Delete { id: x }).unwrap();
    exec(&mut s, T0 + 3 * S, &Actor::human(), Cmd::Delete { id: y }).unwrap();
    assert_eq!(inbox(&s), 2);
}

#[test]
fn split_dispatch_keeps_card_in_inbox_and_counts_children() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = inbox_card(&mut s, T0, &Actor::human(), "一句话需求", None).unwrap().card.id;
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    assert_eq!((d.kind.as_str(), d.state.as_str()), (hosts::KIND_SPLIT, "queued"));
    let c = card(&s, a).card;
    assert_eq!((c.column.as_str(), c.holder.as_deref()), (INBOX, None), "排队时不动这一条");
    // 同一条已经在排队：ALREADY_DISPATCHED
    assert_eq!(dispatch(&mut s, T0 + S, a, "mbp", None).unwrap_err().code, ErrorCode::AlreadyDispatched);
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    assert_eq!(d.state, "running");
    let name = d.name.clone().unwrap();
    let c = card(&s, a).card;
    assert_eq!((c.column.as_str(), c.holder.as_deref()), (INBOX, None), "开始时不领取、不移动");
    assert!(card(&s, a).activity.iter().any(|x| x.text.starts_with("拆解开始")));
    // 正在拆解：人照样能编辑这一条；再派一次 ALREADY_DISPATCHED
    exec(&mut s, T0 + 2 * S, &Actor::human(), Cmd::Edit { id: a, title: Some("一句话需求（补充）".into()), body: None }).unwrap();
    assert_eq!(dispatch(&mut s, T0 + 2 * S, a, "mbp", None).unwrap_err().code, ErrorCode::AlreadyDispatched);
    // 占一个会话位：最多 1 个会话时，别的派发排着
    let b = new_card(&mut s, T0, "B");
    let db = dispatch(&mut s, T0 + 2 * S, b, "mbp", None).unwrap();
    assert_eq!(begin(&mut s, T0 + 2 * S, db.id).unwrap_err().code, ErrorCode::Usage, "拆解占着会话位");
    // host 替这个名字发心跳；agent 按提示词建两张拆自它的卡
    beat(&mut s, T0 + 3 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    let agent = Actor::agent(name.clone());
    let x = inbox_card(&mut s, T0 + 4 * S, &agent, "子卡一", Some(a)).unwrap().card.id;
    let y = inbox_card(&mut s, T0 + 5 * S, &agent, "子卡二", Some(a)).unwrap().card.id;
    let d = end(&mut s, T0 + 6 * S, d.id, Some(0), &[]);
    assert_eq!(d.result.as_deref(), Some("split"), "已拆解");
    assert_eq!(hosts::result_words("split"), "已拆解");
    let c = card(&s, a);
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), (INBOX, None), "结束后还在未整理");
    assert_eq!(c.split_into, vec![x, y]);
    assert!(c.activity.iter().any(|x| x.text.starts_with("拆解结束：已拆解") && x.text.contains("拆出 2 条")), "{:?}", c.activity.last());
    for id in [x, y] {
        assert_eq!(card(&s, id).card.column, INBOX, "拆出的卡在未整理");
    }
    // 空出了会话位
    assert_eq!(begin(&mut s, T0 + 7 * S, db.id).unwrap().state, "running");
}

#[test]
fn split_dispatch_results_without_cards_crash_stop_and_timeout() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(4), ..Default::default() })).unwrap();
    let run = |s: &mut Store, title: &str| -> (i64, Dispatch) {
        let a = inbox_card(s, T0, &Actor::human(), title, None).unwrap().card.id;
        let d = dispatch(s, T0, a, "mbp", None).unwrap();
        (a, begin(s, T0 + S, d.id).unwrap())
    };
    // 一张也没建，退出 0：粒度预检判过了，不用拆（default/main#306）
    let (a, d) = run(&mut s, "拆不出来");
    // 拆解开始之前建的、拆自别的条目的都不算
    inbox_card(&mut s, T0 + 2 * S, &Actor::human(), "别的", None).unwrap();
    assert_eq!(end(&mut s, T0 + 3 * S, d.id, Some(0), &[]).result.as_deref(), Some("fine"));
    assert_eq!(hosts::result_words("fine"), "不用拆");
    // 旧版服务端记的 nosplit 照样有中文
    assert_eq!(hosts::result_words("nosplit"), "没拆出卡");
    let c = card(&s, a);
    assert_eq!(c.card.column, INBOX);
    assert!(c.activity.iter().any(|x| x.text.starts_with("拆解结束：不用拆")), "{:?}", c.activity.last());
    // 退出 1：异常退出，在原条目上评论退出码和最后的输出，卡片仍在未整理
    let (a, d) = run(&mut s, "会出错");
    let tail: Vec<String> = vec!["读需求".into()];
    let events: Vec<String> = vec!["工具 Bash：board show".into()];
    let d = s.write(T0 + 3 * S, Origin::cli(), |w| w.dispatch_end_with(d.id, Some(1), &tail, &events, EndHow::Exit, None)).unwrap();
    assert_eq!(d.result.as_deref(), Some("crashed"));
    let c = card(&s, a);
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), (INBOX, None));
    let comment = c.activity.iter().find(|x| x.kind == "comment").expect("异常退出要评论");
    assert!(comment.text.contains("退出码 1") && comment.text.contains("board show"), "{}", comment.text);
    // 人停止：已停止，评论是谁停的
    let (a, d) = run(&mut s, "人停了");
    s.write(T0 + 2 * S, Origin::ui(None), |w| w.dispatch_cancel(d.id, &Actor::human(), "buglan")).unwrap();
    assert_eq!(end(&mut s, T0 + 3 * S, d.id, Some(143), &[]).result.as_deref(), Some("stopped"));
    let c = card(&s, a);
    assert_eq!(c.card.column, INBOX);
    assert!(c.activity.iter().any(|x| x.kind == "comment" && x.text.contains("buglan 停止了这次拆解")));
    // 兜底（超时）：也记已停止，评论原因
    let (a, d) = run(&mut s, "卡住了");
    let d = s.write(T0 + 3 * S, Origin::cli(), |w| w.dispatch_end_with(d.id, None, &[], &[], EndHow::Timeout, Some("20 分钟没有输出"))).unwrap();
    assert_eq!(d.result.as_deref(), Some("stopped"));
    let c = card(&s, a);
    assert_eq!(c.card.column, INBOX);
    assert!(c.activity.iter().any(|x| x.kind == "comment" && x.text.contains("20 分钟没有输出")));
    // 排队中取消：条目不动
    let a = inbox_card(&mut s, T0, &Actor::human(), "取消", None).unwrap().card.id;
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.dispatch_cancel(d.id, &Actor::human(), "buglan")).unwrap();
    assert_eq!(get(&s, d.id).result.as_deref(), Some("canceled"));
    assert_eq!(card(&s, a).card.column, INBOX);
}

#[test]
fn split_dispatch_checks_host_and_cron_never_takes_inbox() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = inbox_card(&mut s, T0, &Actor::human(), "需求", None).unwrap().card.id;
    assert_eq!(dispatch(&mut s, T0, a, "nope", None).unwrap_err().code, ErrorCode::NoSuchHost);
    assert_eq!(dispatch(&mut s, T0, a, "mbp", Some("mycli")).unwrap_err().code, ErrorCode::AgentNotOnHost);
    // 人把排队中的条目整理进工作流：拆解自动取消
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    exec(&mut s, T0 + S, &Actor::human(), Cmd::Move { id: a, column: "todo".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    assert_eq!(get(&s, d.id).result.as_deref(), Some("canceled"));
    // 定时领取不碰未整理：规则命中所有卡片（兜底），白板上只有未整理的条目
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    inbox_card(&mut s, T0, &Actor::human(), "只在未整理", None).unwrap();
    s.write(T0, Origin::ui(None), |w| w.create_label("default", "全部", None, false)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { rules: Some(vec![cond("in", &["全部"], "claude")]), fallback: Some("claude".into()), ..Default::default() })).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none(), "定时领取不会派发未整理里的条目");
}

/* ------------------------------ 从机器导入 skills ------------------------------ */

fn local(name: &str, dirs: &[&str], why: Option<&str>) -> crate::skills::LocalSkill {
    crate::skills::LocalSkill { name: name.into(), version: "abc123abc123".into(), files: 2, size: 900, agents: vec!["claude".into()], dirs: dirs.iter().map(|d| d.to_string()).collect(), why: why.map(str::to_string) }
}

fn pick(name: &str, dir: &str) -> SkillImportPick {
    SkillImportPick { name: name.into(), dir: dir.into() }
}

fn import(s: &mut Store, at: i64, picks: Vec<SkillImportPick>) -> Result<HostInfo> {
    s.write(at, Origin::ui(None), |w| w.host_skill_import("mbp", picks, "octocat"))
}

#[test]
fn skill_import_lists_local_skills_and_runs_once() {
    let mut s = store();
    // 旧版 board host 不报本机的 skill：不能导入，写要升级
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let e = import(&mut s, T0, vec![pick("design", "~/.claude/skills")]).unwrap_err();
    assert!(e.message.contains("升级"), "{}", e.message);
    // 新版带上本机的 skill
    let rep = HostReport { local_skills: Some(vec![local("design", &["~/.claude/skills"], None), local("huge", &["~/.claude/skills"], Some("超过 5 MB"))]), ..report("mbp", "i1", vec![]) };
    let w = beat(&mut s, T0 + S, &rep).unwrap();
    assert!(w.host.skill_import.able);
    assert_eq!(w.host.skill_import.local.len(), 2);
    // 只能选它报的、能导入的
    assert_eq!(import(&mut s, T0 + S, vec![]).unwrap_err().code, ErrorCode::Usage);
    assert!(import(&mut s, T0 + S, vec![pick("nope", "~/.claude/skills")]).unwrap_err().message.contains("没有"));
    assert!(import(&mut s, T0 + S, vec![pick("huge", "~/.claude/skills")]).unwrap_err().message.contains("5 MB"));
    assert!(import(&mut s, T0 + S, vec![pick("board-skills", "~/.claude/skills")]).unwrap_err().message.contains("board-skills"));
    let h = import(&mut s, T0 + 2 * S, vec![pick("design", "~/.claude/skills")]).unwrap();
    let ask = h.skill_import.ask.clone().expect("记下请求");
    assert_eq!((ask.picks.len(), ask.by.as_str()), (1, "octocat"));
    assert_eq!(h.ops[0].text, "请求导入 skills：design");
    // 取任务时 board host 能看到请求；同时只做一次
    let w = s.read(|r| r.host_work("mbp", T0 + 3 * S)).unwrap();
    assert_eq!(w.host.skill_import.ask.as_ref().map(|a| a.id), Some(ask.id));
    assert!(import(&mut s, T0 + 3 * S, vec![pick("design", "~/.claude/skills")]).unwrap_err().message.contains("正在导入"));
    // 别的进程、序号对不上的回报不算
    let h = s.write(T0 + 4 * S, Origin::cli(), |w| w.host_skill_imported("mbp", "i2", ask.id, vec!["design".into()], vec![])).unwrap();
    assert!(h.skill_import.ask.is_some());
    // 做完：记下结果、清掉请求、记操作；上报里的本机 skill 不被冲掉
    let fail = SkillImportFail { name: "x".into(), why: "读不了".into() };
    let h = s.write(T0 + 5 * S, Origin::cli(), |w| w.host_skill_imported("mbp", "i1", ask.id, vec!["design".into()], vec![fail])).unwrap();
    assert!(h.skill_import.ask.is_none());
    let last = h.skill_import.last.clone().unwrap();
    assert_eq!((last.ok.clone(), last.failed.len(), last.by.as_str()), (vec!["design".to_string()], 1, "octocat"));
    assert!(h.ops[0].text.starts_with("导入了 skills：design；x 没传成"), "{:?}", h.ops);
    assert_eq!(h.skill_import.local.len(), 2);
    // 离线不能导入；超过时间没做完的请求可以重新发
    let h = import(&mut s, T0 + 6 * S, vec![pick("design", "~/.claude/skills")]).unwrap();
    assert!(h.skill_import.ask.is_some());
    assert!(import(&mut s, T0 + 6 * S + SKILL_IMPORT_STALE_MS + S, vec![pick("design", "~/.claude/skills")]).unwrap_err().message.contains("离线"));
    beat(&mut s, T0 + 6 * S + SKILL_IMPORT_STALE_MS + S, &rep).unwrap();
    assert!(import(&mut s, T0 + 6 * S + SKILL_IMPORT_STALE_MS + 2 * S, vec![pick("design", "~/.claude/skills")]).is_ok());
}

/* ------------------------------ agent 名字由 Board 生成（PRD §3.2，board check --agent） ------------------------------ */

fn check_gen(s: &mut Store, at: i64, host: Option<&str>, agent: &str, token: Option<i64>) -> Result<AgentInfo> {
    s.write(at, Origin::cli(), |w| w.agent_check_generated(host, agent, token, crate::agents::NameOwner::Session("s1")))
}

/// 不带名字的 check：名字按这台机器的 agent 设置生成（和派发一致），没配模型时第二段是 agent 标识
#[test]
fn check_generates_name_from_host_agent_settings() {
    let mut s = store();
    let mut rep = report("win", "i1", vec![]);
    rep.agents.push(HostAgent { id: "dsh".into(), name: "DeepSeek Harness".into(), version: "1.0".into(), found: true, ..Default::default() });
    beat(&mut s, T0, &rep).unwrap();
    // mycli 没配模型：win/mycli
    let info = check_gen(&mut s, T0 + S, Some("win"), "mycli", Some(1)).unwrap();
    assert_eq!(info.name, "win/mycli");
    assert_eq!(info.modeled, Some(false), "没配模型：第二段是 agent 标识，不是模型");
    check_agent_name(&info.name).unwrap();
    // dsh 没有模型（board host 没上报）：同样退回 agent 标识
    assert_eq!(check_gen(&mut s, T0 + S, Some("win"), "dsh", Some(1)).unwrap().name, "win/dsh");
    // claude 配了 opus-5.5：win/opus-5.5，和派发出来的名字一致
    assert_eq!(check_gen(&mut s, T0 + S, Some("win"), "claude", Some(1)).unwrap().name, "win/opus-5.5");
}

/// 名字第二段是 board host 上报的、配置文件里现在的模型，和派发一致
#[test]
fn check_uses_the_model_the_agent_config_has() {
    let mut s = store();
    let mut rep = report("mbp", "i1", vec![]);
    rep.agents[0].model = "sonnet".into();
    beat(&mut s, T0, &rep).unwrap();
    assert_eq!(check_gen(&mut s, T0 + S, Some("mbp"), "claude", Some(1)).unwrap().name, "mbp/sonnet");
}

/// 同一机器同一 agent 已经有在线会话时自动加 -1、-2 …，不报 NAME_TAKEN；离线后复用
#[test]
fn check_generated_name_adds_sequence_instead_of_name_taken() {
    let mut s = store();
    beat(&mut s, T0, &report("win", "i1", vec![])).unwrap();
    assert_eq!(check_gen(&mut s, T0 + S, Some("win"), "claude", Some(1)).unwrap().name, "win/opus-5.5");
    assert_eq!(check_gen(&mut s, T0 + S, Some("win"), "claude", Some(1)).unwrap().name, "win/opus-5.5-1");
    assert_eq!(check_gen(&mut s, T0 + S, Some("win"), "claude", Some(1)).unwrap().name, "win/opus-5.5-2");
    // 名字太长的：先截断模型名，序号保留在最后，总长不超过 48
    let mut rep = report("win", "i1", vec![]);
    rep.agents[0].model = "gpt-5.1-codex-max-preview-very-long-model-name".into();
    beat(&mut s, T0 + 2 * S, &rep).unwrap();
    // 换了模型（现在是这个名字）：算新名字，从第一个空闲的序号开始
    let a = check_gen(&mut s, T0 + 2 * S, Some("win"), "claude", Some(1)).unwrap().name;
    let b = check_gen(&mut s, T0 + 2 * S, Some("win"), "claude", Some(1)).unwrap().name;
    for n in [&a, &b] {
        check_agent_name(n).unwrap();
        assert!(n.chars().count() <= 48, "{n}");
    }
    assert!(b.ends_with("-1") && !a.ends_with("-1"), "{a} {b}");
    // 不带序号的名字离线后可以重新用
    assert!(a.starts_with("win/gpt-5.1-codex-max-preview-"), "{a}");
    s.write(T0 + 3 * S, Origin::cli(), |w| w.agent_beat_by(&a, true, BeatBy::Cli(Some("s1")))).unwrap();
    assert_eq!(check_gen(&mut s, T0 + 3 * S, Some("win"), "claude", Some(1)).unwrap().name, a);
}

/// 查不到这台机器（没跑 board host、机器名不存在、离线）时退回 <agent 标识>/<模型>，仍然合法
#[test]
fn check_falls_back_to_agent_slash_model_without_host() {
    let mut s = store();
    let info = check_gen(&mut s, T0, Some("nope"), "mycli", Some(1)).unwrap();
    assert_eq!(info.name, "mycli/mycli");
    assert_eq!(info.modeled, Some(false));
    check_agent_name(&info.name).unwrap();
    // 不带机器名、令牌也不是任何机器的：一样退回
    assert_eq!(check_gen(&mut s, T0, None, "dsh", None).unwrap().name, "dsh/dsh");
    // 机器存在但离线：按查不到处理
    beat(&mut s, T0, &report("win", "i1", vec![])).unwrap();
    let later = T0 + hosts::HOST_GAP_MS + S;
    assert_eq!(check_gen(&mut s, later, Some("win"), "claude", Some(1)).unwrap().name, "claude/claude");
}

/// 没有机器（没跑 board host）时也不报 NAME_TAKEN：<标识>/<标识> 被在线会话占用时加 -1、-2（default/main#539）；
/// 前面的会话下线后，新会话又能拿到不带序号的名字
#[test]
fn check_generated_without_a_host_adds_sequence_too() {
    let mut s = store();
    let names: Vec<String> = (0..3).map(|_| check_gen(&mut s, T0 + S, None, "claude", None).unwrap().name).collect();
    assert_eq!(names, vec!["claude/claude", "claude/claude-1", "claude/claude-2"]);
    for n in &names {
        check_agent_name(n).unwrap();
    }
    // 第一个会话下线
    s.write(T0 + 2 * S, Origin::cli(), |w| w.agent_beat_by("claude/claude", true, crate::agents::BeatBy::Cli(Some("s1")))).unwrap();
    assert_eq!(check_gen(&mut s, T0 + 3 * S, None, "claude", None).unwrap().name, "claude/claude");
}

/// 没带机器名时按令牌认机器：令牌是它的 board host 上报用的那个
#[test]
fn check_finds_the_host_by_token() {
    let mut s = store();
    beat(&mut s, T0, &report("win", "i1", vec![])).unwrap();
    assert_eq!(check_gen(&mut s, T0 + S, None, "claude", Some(1)).unwrap().name, "win/opus-5.5");
    // 别的令牌（它不是任何机器上报用的）：退回
    assert_eq!(check_gen(&mut s, T0 + S, None, "claude", Some(9)).unwrap().name, "claude/claude");
}

/// 带了名字（--as）的 check 不受影响：照写的名字 check，被占用时 NAME_TAKEN，不加序号
#[test]
fn check_with_an_explicit_name_is_unchanged() {
    let mut s = store();
    beat(&mut s, T0, &report("win", "i1", vec![])).unwrap();
    s.write(T0, Origin::cli(), |w| w.agent_check("win/opus-5.5", Some(1))).unwrap();
    let e = s.write(T0 + S, Origin::cli(), |w| w.agent_check("win/opus-5.5", Some(1))).unwrap_err();
    assert_eq!(e.code, ErrorCode::NameTaken);
    // board host 派发还是用自己生成的名字：同一机器的同一 agent 加序号
    let id = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, id, "win", Some("claude")).unwrap();
    let d = s.write(T0 + S, Origin::cli(), |w| w.dispatch_begin("win", "i1", d.id, Some(1))).unwrap();
    assert_eq!(d.name.as_deref(), Some("win/opus-5.5-1"));
}

/// agent 标识、机器名不合法时报 USAGE（不带名字的 check 的输入要校验）
#[test]
fn check_generated_validates_agent_and_host() {
    let mut s = store();
    beat(&mut s, T0, &report("win", "i1", vec![])).unwrap();
    assert_eq!(check_gen(&mut s, T0 + S, Some("win"), "Claude Code", Some(1)).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(check_gen(&mut s, T0 + S, Some("Win 1"), "claude", Some(1)).unwrap_err().code, ErrorCode::Usage);
}

/* ------------------------------ 能不能接（fit.rs） ------------------------------ */

fn dir_report(status: &str, origin: Option<&str>) -> HostReport {
    let mut rep = report("mbp", "i1", vec![]);
    rep.dirs[0].status = Some(status.into());
    rep.dirs[0].origin = origin.map(str::to_string);
    rep.dirs[0].checked_at = Some(T0);
    rep
}

fn blocks(s: &Store) -> Vec<(String, String, String)> {
    s.read(|r| r.blocks()).unwrap().into_iter().map(|b| (b.host, b.board, b.code)).collect()
}

/// 项目设了仓库地址：机器上它的目录不存在、不是 git 仓库、origin 不是项目的仓库时，这台机器接不了这个项目的卡——
/// 手动派发被拒、定时领取跳过、工作区数据的 `blocks` 里写明原因。项目没设仓库时不看目录
#[test]
fn project_repo_must_be_cloned_right_before_the_host_takes_its_cards() {
    let mut s = store();
    beat(&mut s, T0, &dir_report("missing", None)).unwrap();
    lead_round(&mut s);
    let id = new_card_tagged(&mut s, "A");
    // 没设仓库的项目用空目录，board host 开始派发时自己建：不拦
    assert!(blocks(&s).is_empty(), "{:?}", blocks(&s));
    ui(&mut s, |w| w.set_repo_branch("default", Some("git@github.com:o/r.git"), None)).unwrap();
    let e = dispatch(&mut s, T0, id, "mbp", None).unwrap_err();
    assert_eq!(e.code, ErrorCode::BoardNotOnHost);
    assert!(e.message.contains("/tmp/default 还不存在"), "{}", e.message);
    assert_eq!(blocks(&s), vec![("mbp".to_string(), B.to_string(), "BOARD_NOT_ON_HOST".to_string())]);
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(pull(&mut s, T0 + S).is_none(), "目录不存在的项目不领");
    // 同一个项目的另一块白板共用这个目录：它还没被这台机器报过，也按这个目录的状态算
    ui(&mut s, |w| w.create_board("default", "文档", "docs", "", None)).unwrap();
    assert_eq!(blocks(&s).len(), 2, "{:?}", blocks(&s));
    // 不是 git 仓库
    beat(&mut s, T0 + 2 * S, &dir_report("not_git", None)).unwrap();
    let e = dispatch(&mut s, T0 + 2 * S, id, "mbp", None).unwrap_err();
    assert!(e.code == ErrorCode::BoardNotOnHost && e.message.contains("不是 git 仓库"), "{}", e.message);
    // 是别的仓库
    beat(&mut s, T0 + 3 * S, &dir_report("ok", Some("git@github.com:o/other.git"))).unwrap();
    let e = dispatch(&mut s, T0 + 3 * S, id, "mbp", None).unwrap_err();
    assert!(e.code == ErrorCode::BoardNotOnHost && e.message.contains("origin 是 git@github.com:o/other.git"), "{}", e.message);
    // 就是项目的仓库（写法不同也算）：照常领
    beat(&mut s, T0 + 4 * S, &dir_report("ok", Some("https://github.com/O/R.git"))).unwrap();
    assert!(blocks(&s).is_empty(), "{:?}", blocks(&s));
    s.write(T0 + 4 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    let d = pull(&mut s, T0 + 5 * S).unwrap();
    assert_eq!((d.card, d.state.as_str()), (id, "running"));
}

/// 没有工作空间、目录不对从同一个判断出来，错误码和手动派发的一致
#[test]
fn blocks_list_every_host_board_pair_that_cannot_take_cards() {
    let mut s = store();
    let mut rep = report("mbp", "i1", vec![]);
    rep.workspace = None;
    rep.dirs.clear();
    beat(&mut s, T0, &rep).unwrap();
    let ws = s.read(|r| r.workspace(T0)).unwrap();
    assert_eq!(ws.blocks.len(), 1);
    assert_eq!((ws.blocks[0].code.as_str(), ws.blocks[0].why.as_str()), ("BOARD_NOT_ON_HOST", "还没有配工作空间"));
    s.write(T0, Origin::ui(None), |w| {
        w.create_board("default", "文档", "docs", "", None)
    })
    .unwrap();
    let ws = s.read(|r| r.workspace(T0)).unwrap();
    assert_eq!(ws.blocks.len(), 2);
    assert!(ws.blocks.iter().all(|b| b.code == "BOARD_NOT_ON_HOST" && b.why == "还没有配工作空间"));
    beat(&mut s, T0 + S, &report("mbp", "i1", vec![])).unwrap();
    assert!(blocks(&s).is_empty());
}

fn crash_once(s: &mut Store, t: i64) {
    let c = new_card(s, t, "x");
    let d = dispatch(s, t, c, "mbp", None).unwrap();
    let d = begin(s, t + S, d.id).unwrap();
    beat(s, t + 2 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    assert_eq!(end(s, t + 3 * S, d.id, Some(1), &["Not logged in"]).result.as_deref(), Some("crashed"));
}

fn tripped(s: &Store, at: i64) -> bool {
    host(s, at).agents.iter().find(|a| a.id == "claude").unwrap().tripped
}

/// 一个 agent 在一台机器上连续 3 次异常退出就暂停接单：手动派发被拒、定时领取不领交给它的卡，记一条操作记录；
/// 人点「恢复」之后重新数
#[test]
fn agent_pauses_after_three_crashes_in_a_row_until_resumed() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    lead_round(&mut s);
    let mut t = T0;
    for i in 0..3 {
        crash_once(&mut s, t);
        assert_eq!(tripped(&s, t + 3 * S), i == 2, "第 {} 次", i + 1);
        t += 10 * S;
    }
    let ops = host(&s, t).ops;
    assert_eq!(ops.iter().filter(|o| o.text.contains("连续 3 次异常退出")).count(), 1, "{:?}", ops.iter().map(|o| &o.text).collect::<Vec<_>>());
    // 手动派发被拒，写明怎么恢复
    let c = new_card_tagged(&mut s, "T");
    let e = dispatch(&mut s, t, c, "mbp", None).unwrap_err();
    assert_eq!(e.code, ErrorCode::AgentNotOnHost);
    assert!(e.message.contains("暂停接单") && e.message.contains("恢复"), "{}", e.message);
    // 定时领取不领交给它的卡
    beat(&mut s, t, &report("mbp", "i1", vec![])).unwrap();
    s.write(t, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(pull(&mut s, t + S).is_none());
    assert_eq!(card(&s, c).card.column, "todo");
    // 恢复：重新数，照常派
    let p = HostPatch { resume: Some("claude".into()), ..Default::default() };
    s.write(t + 2 * S, Origin::ui(None), |w| w.host_update_as("mbp", &p, "lan")).unwrap();
    assert!(!tripped(&s, t + 2 * S));
    assert!(host(&s, t + 2 * S).ops[0].text.contains("恢复了 Claude Code 接单"));
    // 再点一次不重复记
    s.write(t + 3 * S, Origin::ui(None), |w| w.host_update_as("mbp", &p, "lan")).unwrap();
    assert_eq!(host(&s, t + 3 * S).ops.iter().filter(|o| o.text.contains("恢复了")).count(), 1);
    s.write(t + 3 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert_eq!(pull(&mut s, t + 4 * S).map(|d| d.card), Some(c));
    // 恢复之后的一次异常退出只算一次
    let t2 = t + 20 * S;
    beat(&mut s, t2, &report("mbp", "i1", vec![])).unwrap();
    crash_once(&mut s, t2);
    assert!(!tripped(&s, t2 + 3 * S));
}

/// Windows 机器的目录用 `\` 拼：服务端派生的路径和机器上报的一样，检查结果才对得上
#[test]
fn windows_dirs_keep_their_reported_status() {
    let mut s = store();
    let mut rep = report("mbp", "i1", vec![]);
    rep.os = "Windows 11 Pro".into();
    rep.workspace = Some(HostWorkspace { path: "D:\\repo".into(), ..Default::default() });
    rep.dirs = vec![HostDir { board: B.into(), path: "D:\\repo\\default".into(), status: Some("ok".into()), checked_at: Some(T0), ..Default::default() }];
    beat(&mut s, T0, &rep).unwrap();
    let d = &host(&s, T0).dirs[0];
    assert_eq!((d.path.as_str(), d.status.as_deref()), ("D:\\repo\\default", Some("ok")));
}

/* ------------------------------ 任意机器、改派 ------------------------------ */

fn report_as(name: &str, instance: &str, running: Vec<i64>) -> HostReport {
    report(name, instance, running)
}

fn any(s: &mut Store, at: i64, card: i64, agent: Option<&str>) -> Result<Dispatch> {
    s.write(at, Origin::ui(None), |w| {
        let b = w.resolve(B)?;
        w.dispatch_create_any(&b, card, agent, &Actor::human())
    })
}

fn begin_on(s: &mut Store, at: i64, host: &str, instance: &str, id: i64) -> Result<Dispatch> {
    s.write(at, Origin::cli(), |w| w.dispatch_begin(host, instance, id, Some(1)))
}

fn set_on(s: &mut Store, at: i64, host: &str, p: HostPatch) {
    s.write(at, Origin::ui(None), |w| w.host_update(host, &p)).unwrap();
}

/// 派给任意机器：服务端挑空位最多的；一样多时轮流；挑中的机器照常开始
#[test]
fn any_host_dispatch_goes_to_the_freest_host_and_rotates() {
    let mut s = store();
    beat(&mut s, T0, &report_as("mbp", "i1", vec![])).unwrap();
    beat(&mut s, T0, &report_as("win", "w1", vec![])).unwrap();
    set_on(&mut s, T0, "win", HostPatch { max: Some(2), ..Default::default() });
    // win 有 2 个空位，mbp 1 个：先给 win
    let a = new_card(&mut s, T0, "A");
    let d = any(&mut s, T0 + S, a, None).unwrap();
    assert!(d.any);
    assert_eq!((d.host.as_str(), d.agent.as_str(), d.state.as_str()), ("win", "claude", "queued"));
    assert_eq!(card(&s, a).card.column, "doing", "和指定机器的派发一样，排队时就进锁定状态");
    let log = card(&s, a).activity.iter().map(|x| x.text.clone()).collect::<Vec<_>>().join("\n");
    assert!(log.contains("派发到任意机器") && log.contains("任意机器：分给 win · Claude Code"), "{log}");
    // 现在各剩 1 个：轮到最久没接过的 mbp
    let b = new_card(&mut s, T0, "B");
    assert_eq!(any(&mut s, T0 + 2 * S, b, None).unwrap().host, "mbp");
    // 再来一张：win 还有 1 个空位，mbp 没有了
    let c = new_card(&mut s, T0, "C");
    assert_eq!(any(&mut s, T0 + 3 * S, c, None).unwrap().host, "win");
    // 都排满了：没有机器，接着排
    let e = new_card(&mut s, T0, "E");
    let waiting = any(&mut s, T0 + 4 * S, e, None).unwrap();
    assert_eq!((waiting.host.as_str(), waiting.host_id, waiting.agent.as_str(), waiting.agent_label.as_str()), ("", 0, "", hosts::ANY_AGENT_LABEL));
    // 分到的机器照常开始；随它的任务一起发下去
    let w = beat(&mut s, T0 + 5 * S, &report_as("win", "w1", vec![])).unwrap();
    assert_eq!(w.queued.iter().map(|x| x.card).collect::<Vec<_>>(), vec![a, c]);
    let run = begin_on(&mut s, T0 + 6 * S, "win", "w1", d.id).unwrap();
    assert_eq!((run.state.as_str(), run.host.as_str(), run.agent.as_str()), ("running", "win", "claude"));
}

/// 分到的机器开始不了（暂停接单、掉线、被移除）时收回，换一台；都不行就等，有机器能接了再分
#[test]
fn any_host_dispatch_is_taken_back_when_its_host_cannot_start_it() {
    let mut s = store();
    beat(&mut s, T0, &report_as("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = any(&mut s, T0, a, None).unwrap();
    assert_eq!(d.host, "mbp");
    // 暂停接单：收回，没有别的机器，等着
    set_on(&mut s, T0 + S, "mbp", HostPatch { paused: Some(true), ..Default::default() });
    assert_eq!(get(&s, d.id).host, "");
    assert_eq!(get(&s, d.id).state, "queued");
    // 另一台机器上线：它一上报就分给它
    let w = beat(&mut s, T0 + 2 * S, &report_as("win", "w1", vec![])).unwrap();
    assert_eq!(w.queued.iter().map(|x| x.id).collect::<Vec<_>>(), vec![d.id]);
    assert_eq!(get(&s, d.id).host, "win");
    // 它被移除：这条派发不跟着取消，回去等
    s.write(T0 + 3 * S, Origin::ui(None), |w| w.host_remove("win", &Actor::human(), "人")).unwrap();
    let g = get(&s, d.id);
    assert_eq!((g.host.as_str(), g.state.as_str()), ("", "queued"));
    assert_eq!(card(&s, a).card.column, "doing");
    // mbp 恢复接单：分回它
    set_on(&mut s, T0 + 4 * S, "mbp", HostPatch { paused: Some(false), ..Default::default() });
    assert_eq!(get(&s, d.id).host, "mbp");
    // 掉线超过 50 秒：收回
    s.sweep(T0 + 4 * S + hosts::HOST_GAP_MS + S).unwrap();
    assert_eq!(get(&s, d.id).host, "");
    // 人取消：卡片回到可领取状态
    s.write(T0 + 100 * S, Origin::ui(None), |w| w.dispatch_cancel(d.id, &Actor::human(), "人")).unwrap();
    assert_eq!(card(&s, a).card.column, "todo");
}

/// 「任意机器」创建时就拒绝的情况：一台机器都没有、没有机器接得了这块白板、能接的机器上没有这个 agent
#[test]
fn any_host_dispatch_needs_some_host_that_could_take_it() {
    let mut s = store();
    let a = new_card(&mut s, T0, "A");
    assert_eq!(any(&mut s, T0, a, None).unwrap_err().code, ErrorCode::NoSuchHost);
    beat(&mut s, T0, &report_as("mbp", "i1", vec![])).unwrap();
    // 指定的 agent 没找到命令
    let e = any(&mut s, T0, a, Some("mycli")).unwrap_err();
    assert_eq!(e.code, ErrorCode::AgentNotOnHost);
    assert!(e.message.contains("mbp") && e.message.contains("mycli"), "{}", e.message);
    // 机器离线也能派：看的是能不能接，不看在不在线
    s.sweep(T0 + hosts::HOST_GAP_MS + S).unwrap();
    let d = any(&mut s, T0 + hosts::HOST_GAP_MS + 2 * S, a, Some("claude")).unwrap();
    assert_eq!((d.host.as_str(), d.agent.as_str(), d.state.as_str()), ("", "claude", "queued"));
    // 已经有派发的卡不能再派
    assert_eq!(any(&mut s, T0 + hosts::HOST_GAP_MS + 3 * S, a, None).unwrap_err().code, ErrorCode::AlreadyDispatched);
}

/// 改派：排队中的派发换到另一台机器，或交给任意机器；已经开始的不能改
#[test]
fn queued_dispatch_can_be_moved_to_another_host_or_to_any() {
    let mut s = store();
    beat(&mut s, T0, &report_as("mbp", "i1", vec![])).unwrap();
    let mut win = report_as("win", "w1", vec![]);
    win.agents = vec![dsh(true)];
    beat(&mut s, T0, &win).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let mv = |s: &mut Store, at: i64, host: Option<&str>, agent: Option<&str>| s.write(at, Origin::ui(None), |w| w.dispatch_move(d.id, host, agent, &Actor::human()));
    // 换到 win：那台没有 Claude Code，换成它的默认 agent
    let m = mv(&mut s, T0 + S, Some("win"), None).unwrap();
    assert_eq!((m.host.as_str(), m.agent.as_str(), m.state.as_str(), m.any), ("win", "dsh", "queued", false));
    assert_eq!(card(&s, a).card.column, "doing", "改派不动卡片");
    assert!(card(&s, a).activity.iter().any(|x| x.text.contains("改派到 win · DeepSeek Harness（原来 mbp · Claude Code")), "{:?}", card(&s, a).activity.iter().map(|x| &x.text).collect::<Vec<_>>());
    // 校验和手动派发一样
    assert_eq!(mv(&mut s, T0 + S, Some("nope"), None).unwrap_err().code, ErrorCode::NoSuchHost);
    assert_eq!(mv(&mut s, T0 + S, Some("win"), Some("claude")).unwrap_err().code, ErrorCode::AgentNotOnHost);
    // 交给任意机器：挑空位多的（win 上排着它自己，收回后两台各 1 个空位，按轮流）
    let m = mv(&mut s, T0 + 2 * S, None, None).unwrap();
    assert!(m.any && !m.host.is_empty(), "{m:?}");
    // 已经开始的不能改
    let host = m.host.clone();
    let inst = if host == "win" { "w1" } else { "i1" };
    begin_on(&mut s, T0 + 3 * S, &host, inst, d.id).unwrap();
    let e = mv(&mut s, T0 + 4 * S, Some("mbp"), None).unwrap_err();
    assert!(e.code == ErrorCode::Usage && e.message.contains("已经开始"), "{}", e.message);
}

/// 定时建卡的「随机」先挑有空位的：空位最多的那台
#[test]
fn candidate_counts_running_and_queued_against_the_session_limit() {
    let mut s = store();
    beat(&mut s, T0, &report_as("mbp", "i1", vec![])).unwrap();
    let free = |s: &mut Store| s.write(T0 + S, Origin::ui(None), |w| { let h = w.host_row("mbp")?.unwrap(); Ok(w.candidate(&h)?.free) }).unwrap();
    assert_eq!(free(&mut s), 1);
    let a = new_card(&mut s, T0, "A");
    dispatch(&mut s, T0, a, "mbp", None).unwrap();
    assert_eq!(free(&mut s), 0, "排着还没开始的也占一个位置");
    set_on(&mut s, T0, "mbp", HostPatch { max: Some(3), ..Default::default() });
    assert_eq!(free(&mut s), 2);
}

/* ------------------------------ 自动领取：全局规则、白板开关、按机器分摊 ------------------------------ */

fn pull_on(s: &mut Store, at: i64, host: &str, instance: &str) -> Option<Dispatch> {
    s.write(at, Origin::cli(), |w| w.dispatch_pull(host, instance, Some(1))).unwrap()
}

/// 一个会话排几张卡按现在也能接的机器分摊：4 张卡、两台机器都在自动接单且有空位，先来的一台只排 2 张，另一台接剩下 2 张
#[test]
fn session_shares_cards_with_other_hosts_that_can_take_them() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    beat(&mut s, T0, &report("win", "w1", vec![])).unwrap();
    lead_round(&mut s);
    set_on(&mut s, T0, "win", HostPatch { auto: Some(true), ..Default::default() });
    let cards: Vec<i64> = (0..4).map(|i| new_card_tagged(&mut s, &format!("c{i}"))).collect();
    let d = pull_on(&mut s, T0 + S, "mbp", "i1").unwrap();
    let mine: Vec<i64> = session_of(&s, d.session).iter().map(|x| x.card).collect();
    assert_eq!(mine.len(), 2, "两台机器分 4 张：{mine:?}");
    let d2 = pull_on(&mut s, T0 + S, "win", "w1").unwrap();
    assert_eq!(d2.host, "win");
    let all: i64 = s.read(|r| r.active_dispatches()).unwrap().len() as i64;
    assert_eq!(all, 4, "四张都排上了：{cards:?}");
    // 另一台不在自动接单时不分摊：一台排满
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    beat(&mut s, T0, &report("win", "w1", vec![])).unwrap();
    lead_round(&mut s);
    for i in 0..4 {
        new_card_tagged(&mut s, &format!("c{i}"));
    }
    let d = pull_on(&mut s, T0 + S, "mbp", "i1").unwrap();
    assert_eq!(session_of(&s, d.session).len(), 4);
}

/// 白板可以不参与自动领取（手动派发照常）；运行中新建的白板不用机器重启、不用它先上报就会被领到
#[test]
fn auto_claim_follows_the_board_switch_and_sees_new_boards() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    lead_round(&mut s);
    let a = new_card_tagged(&mut s, "A");
    s.write(T0, Origin::ui(None), |w| w.set_board_auto(B, false)).unwrap();
    assert!(!s.read(|r| r.workspace(T0)).unwrap().projects[0].boards[0].auto);
    assert!(pull_on(&mut s, T0 + S, "mbp", "i1").is_none(), "不参与的白板不领");
    assert_eq!(dispatch(&mut s, T0 + S, a, "mbp", None).unwrap().state, "queued", "手动派发照常");
    // 机器上报之后才建的白板：没有出现在它上报的目录里，照样被领到
    ui(&mut s, |w| w.create_board("default", "文档", "docs", "", None)).unwrap();
    let l = s.read(|r| r.labels("default")).unwrap().into_iter().find(|l| l.name == "deepseek").unwrap();
    let c = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve("default/docs")?;
            w.exec(&b, &Actor::human(), Cmd::New { title: "D".into(), body: None, column: None, top: false, labels: vec![l.id], from: None })
        })
        .unwrap()
        .card
        .id;
    set_on(&mut s, T0, "mbp", HostPatch { max: Some(2), ..Default::default() });
    let d = pull_on(&mut s, T0 + 2 * S, "mbp", "i1").unwrap();
    assert_eq!((d.board.as_str(), d.card), ("default/docs", c));
}

/// default/main#330：机器上报之后才建的白板、项目，随下一次的任务一起发给它（带派生的目录），
/// 它不用重启就能建目录、克隆；没有工作空间路径的旧机器不派生
#[test]
fn boards_created_after_the_host_started_reach_it_with_their_dirs() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!(host(&s, T0).dirs.iter().map(|d| d.board.as_str()).collect::<Vec<_>>(), vec![B]);
    ui(&mut s, |w| w.create_board("default", "文档", "docs", "", None)).unwrap();
    s.write(T0, Origin::ui(None), |w| w.create_project("新项目", "newp", "", None, None, NewBoard { name: "主白板".into(), key: "main".into(), desc: String::new(), pos: None })).unwrap();
    let w = beat(&mut s, T0 + S, &report("mbp", "i1", vec![])).unwrap();
    let got: Vec<(String, String)> = w.host.dirs.iter().map(|d| (d.board.clone(), d.path.clone())).collect();
    assert!(got.contains(&("default/docs".into(), "/tmp/default".into())), "{got:?}");
    assert!(got.contains(&("newp/main".into(), "/tmp/newp".into())), "{got:?}");
    // 新项目有工作空间里的目录：这块白板能派了
    let id = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve("newp/main")?;
            w.exec(&b, &Actor::human(), Cmd::New { title: "N".into(), body: None, column: None, top: false, labels: vec![], from: None })
        })
        .unwrap()
        .card
        .id;
    let put = |s: &mut Store, at: i64| {
        s.write(at, Origin::ui(None), |w| {
            let b = w.resolve("newp/main")?;
            w.dispatch_create(&b, id, "mbp", None, &Actor::human())
        })
    };
    assert_eq!(put(&mut s, T0 + S).unwrap().state, "queued");
    // 旧版 board host（没有工作空间路径）：不替它派生
    let mut s = store();
    beat(&mut s, T0, &legacy_report("old", "o1")).unwrap();
    ui(&mut s, |w| w.create_board("default", "文档", "docs", "", None)).unwrap();
    let h = s.read(|r| r.host("old", T0)).unwrap();
    assert!(!h.dirs.iter().any(|d| d.board == "default/docs"), "{:?}", h.dirs.iter().map(|d| &d.board).collect::<Vec<_>>());
}

/// 项目目录没问题、但这台机器拉不到最新代码：先不接这个项目的卡，原因里写明；拉成了（不再报）就恢复
#[test]
fn host_that_cannot_fetch_does_not_take_the_projects_cards() {
    let mut s = store();
    ui(&mut s, |w| w.set_repo_branch("default", Some("git@github.com:o/r.git"), Some("main"))).unwrap();
    let mut rep = dir_report("ok", Some("git@github.com:o/r.git"));
    rep.dirs[0].fetch_error = Some("Permission denied (publickey).".into());
    beat(&mut s, T0, &rep).unwrap();
    let id = new_card(&mut s, T0, "A");
    let e = dispatch(&mut s, T0, id, "mbp", None).unwrap_err();
    assert_eq!(e.code, ErrorCode::BoardNotOnHost);
    assert!(e.message.contains("/tmp/default 拉不到最新代码：Permission denied (publickey)."), "{}", e.message);
    assert_eq!(s.read(|r| r.blocks()).unwrap()[0].why, "/tmp/default 拉不到最新代码：Permission denied (publickey).");
    beat(&mut s, T0 + S, &dir_report("ok", Some("git@github.com:o/r.git"))).unwrap();
    let d = dispatch(&mut s, T0 + S, id, "mbp", None).unwrap();
    // 派发带着项目的仓库和集成分支：board host 据此拉代码、备 worktree
    assert_eq!((d.repo.as_str(), d.branch.as_str()), ("git@github.com:o/r.git", "main"));
}

/* ------------------------------ 掉线后接回、半成品交接 ------------------------------ */

/// 机器掉线超过 50 秒：派发按「机器掉线」结束、卡片回到待办。它恢复连接时还报着这次派发在跑，卡片没被别人领走：
/// 接回，名字重新持有卡片，派发回到在跑；已经被别人领走的不接
#[test]
fn lost_dispatch_is_revived_when_the_host_comes_back_and_the_card_is_still_free() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    let name = d.name.clone().unwrap();
    let gone = T0 + S + hosts::HOST_GAP_MS + 5 * S;
    s.sweep(gone).unwrap();
    assert_eq!(get(&s, d.id).result.as_deref(), Some("lost"));
    assert_eq!((card(&s, a).card.column.as_str(), card(&s, a).card.holder.clone()), ("todo", None));
    // 恢复连接：agent 进程还在，board host 报着这次派发
    let w = beat(&mut s, gone + S, &report("mbp", "i1", vec![d.id])).unwrap();
    assert_eq!(w.running.iter().map(|x| x.id).collect::<Vec<_>>(), vec![d.id], "接回之后随任务发回去，board host 不会结束它");
    let g = get(&s, d.id);
    assert_eq!((g.state.as_str(), g.result.clone(), g.ended_at), ("running", None, None));
    let c = card(&s, a);
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), ("doing", Some(name.as_str())));
    assert!(c.activity.iter().any(|x| x.text.contains("恢复连接") && x.text.contains("接着做")), "{:?}", c.activity.iter().map(|x| &x.text).collect::<Vec<_>>());
    // 照常结束
    assert_eq!(end(&mut s, gone + 2 * S, d.id, Some(1), &["boom"]).result.as_deref(), Some("crashed"));

    // 另一张：掉线期间被别人领走了，不接
    let b = new_card(&mut s, gone + 3 * S, "B");
    let d2 = dispatch(&mut s, gone + 3 * S, b, "mbp", None).unwrap();
    let d2 = begin(&mut s, gone + 4 * S, d2.id).unwrap();
    let gone2 = gone + 4 * S + hosts::HOST_GAP_MS + 5 * S;
    s.sweep(gone2).unwrap();
    s.write(gone2, Origin::cli(), |w| w.agent_check("other/x", None)).unwrap();
    exec(&mut s, gone2 + S, &Actor::agent("other/x"), Cmd::Claim { id: Some(b) }).unwrap();
    let w = beat(&mut s, gone2 + 2 * S, &report("mbp", "i1", vec![d2.id])).unwrap();
    assert!(w.running.is_empty(), "已经被别人领走：不接回，board host 会结束那个 agent");
    assert_eq!(get(&s, d2.id).result.as_deref(), Some("lost"));
    assert_eq!(card(&s, b).card.holder.as_deref(), Some("other/x"));
}

/// 半成品交接：board host 把没推的改动保存成 wip 分支后，在卡片上评论怎么结束的、留下了什么、agent 最后做了什么，
/// 最后一行是 board-skills 认的「未完成的改动：<分支>」；派发已经结束（机器掉线）也能评论
#[test]
fn wip_comment_tells_the_next_agent_where_to_pick_up() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    let d = begin(&mut s, T0 + S, d.id).unwrap();
    beat(&mut s, T0 + 2 * S, &report("mbp", "i1", vec![d.id])).unwrap();
    end(&mut s, T0 + 3 * S, d.id, Some(1), &["boom"]);
    let tail: Vec<String> = (1..=12).map(|i| format!("▸ 第 {i} 步")).collect();
    s.write(T0 + 4 * S, Origin::cli(), |w| w.dispatch_wip(d.id, "wip/main/1", "agent 退出，退出码 1", "2 个没提交的文件（a.rs、b.tsx）", &tail)).unwrap();
    let c = card(&s, a);
    let text = &c.activity.iter().rev().find(|x| x.kind == "comment").unwrap().text;
    assert!(text.contains("board host（mbp）") && text.contains("已经保存到分支 `wip/main/1`"), "{text}");
    assert!(text.contains("怎么结束的：agent 退出，退出码 1") && text.contains("留下了什么：2 个没提交的文件（a.rs、b.tsx）"), "{text}");
    assert!(text.contains("第 12 步") && text.contains("第 3 步") && !text.contains("第 2 步"), "只留最后 10 条：{text}");
    assert_eq!(text.lines().last(), Some("未完成的改动：wip/main/1"), "{text}");
    // 分支名不对的拒绝
    for bad in ["main", "wip/../x", "wip/a b"] {
        assert_eq!(s.write(T0 + 5 * S, Origin::cli(), |w| w.dispatch_wip(d.id, bad, "", "", &[])).unwrap_err().code, ErrorCode::Usage, "{bad}");
    }
}

/// 自动升级：开关开着时，board 比服务端旧的机器一上报就替它发起升级（和人点「升级」一样：暂停接单、等在跑的做完）；
/// 没成时恢复接单，不马上再试（重试见下一个用例）；不比服务端旧的、关着开关的不动
#[test]
fn auto_upgrade_asks_outdated_hosts_once_per_server_version() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    assert!(host(&s, T0).ask.is_none(), "默认关着");
    assert!(!s.read(|r| r.sched(T0)).unwrap().auto_upgrade);
    assert!(s.write(T0, Origin::ui(None), |w| w.set_auto_upgrade(true)).unwrap().auto_upgrade);
    // 这台机器是 0.1.1，比服务端旧：下一次上报就发起升级
    beat(&mut s, T0 + S, &report("mbp", "i1", vec![])).unwrap();
    let h = host(&s, T0 + S);
    let ask = h.ask.clone().expect("应该有升级请求");
    assert_eq!((ask.kind.as_str(), ask.by.as_str(), ask.to.as_deref()), ("upgrade", "自动升级", Some(env!("CARGO_PKG_VERSION"))));
    assert!(h.paused, "升级前先暂停接单");
    // 再上报不重复发起
    beat(&mut s, T0 + 2 * S, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!(host(&s, T0 + 2 * S).ask.map(|a| a.id), Some(ask.id));
    // 机器开始处理、报升级没成：恢复接单，这个版本不再自动试
    let mut rep = report("mbp", "i1", vec![]);
    rep.asked = Some(ask.id);
    beat(&mut s, T0 + 3 * S, &rep).unwrap();
    let mut rep = report("mbp", "i1", vec![]);
    rep.ask_error = Some("升级没成：下载不了".into());
    beat(&mut s, T0 + 4 * S, &rep).unwrap();
    beat(&mut s, T0 + 5 * S, &report("mbp", "i1", vec![])).unwrap();
    let h = host(&s, T0 + 5 * S);
    assert!(h.ask.is_none() && !h.paused, "没成：恢复接单，不马上再试");
    // 版本和服务端一样的机器不动
    let mut cur = report("win", "w1", vec![]);
    cur.version = env!("CARGO_PKG_VERSION").into();
    beat(&mut s, T0 + 5 * S, &cur).unwrap();
    beat(&mut s, T0 + 6 * S, &cur).unwrap();
    assert!(s.read(|r| r.host("win", T0 + 6 * S)).unwrap().ask.is_none());
}

/// 自动升级的一次：机器开始处理（asked），再报结果（`err` 为空时报「升级好了」：换一个进程、版本变成服务端的）
fn auto_upgrade_round(s: &mut Store, at: i64, err: Option<&str>) {
    let ask = host(s, at).ask.expect("应该有升级请求");
    let mut rep = report("mbp", "i1", vec![]);
    rep.asked = Some(ask.id);
    beat(s, at, &rep).unwrap();
    match err {
        Some(e) => {
            let mut rep = report("mbp", "i1", vec![]);
            rep.ask_error = Some(e.into());
            beat(s, at + S, &rep).unwrap();
        }
        None => {
            // 旧进程退出前发 final（机器离线），新版本的进程再上来
            beat(s, at, &HostReport { done: true, ..report("mbp", "i1", vec![]) }).unwrap();
            let mut rep = report("mbp", "i2", vec![]);
            rep.version = env!("CARGO_PKG_VERSION").into();
            beat(s, at + S, &rep).unwrap();
        }
    }
}

/// default/main#352：服务端先部署、发版包晚十几分钟才出来时，第一次自动升级下载不到。
/// 临时失败恢复接单后每隔 10 分钟再试一次，同一个服务端版本最多 5 次；操作记录里写明每一次和结果
#[test]
fn auto_upgrade_retries_temporary_failures_every_10_minutes_up_to_5_times() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    s.write(T0, Origin::ui(None), |w| w.set_auto_upgrade(true)).unwrap();
    let server = env!("CARGO_PKG_VERSION");
    let mut t = T0 + S;
    beat(&mut s, t, &report("mbp", "i1", vec![])).unwrap();
    for i in 1..=AUTO_UPGRADE_TRIES {
        let ask = host(&s, t).ask.unwrap_or_else(|| panic!("第 {i} 次：应该有升级请求"));
        let by = if i == 1 { "自动升级".to_string() } else { format!("自动升级（第 {i} 次）") };
        assert_eq!((ask.by.as_str(), ask.to.as_deref()), (by.as_str(), Some(server)), "第 {i} 次");
        assert!(host(&s, t).paused, "第 {i} 次：升级前先暂停接单");
        auto_upgrade_round(&mut s, t + S, Some("升级失败：没有找到比现在新的桌面端版本"));
        let failed = t + 2 * S;
        let h = host(&s, failed);
        assert!(h.ask.is_none() && !h.paused, "第 {i} 次没成：恢复接单");
        let text = &h.ops[0].text;
        assert!(text.starts_with("升级失败：没有找到比现在新的桌面端版本；已恢复接单"), "{text}");
        if i < AUTO_UPGRADE_TRIES {
            assert!(text.ends_with(&format!("10 分钟后自动再试（第 {}/5 次）", i + 1)), "{text}");
        } else {
            assert!(text.ends_with(&format!("自动升级到 {server} 已经试了 5 次，不再重试，请手动升级")), "{text}");
        }
        // 不到 10 分钟不试
        beat(&mut s, failed + AUTO_UPGRADE_RETRY_MS - S, &report("mbp", "i1", vec![])).unwrap();
        assert!(host(&s, failed + AUTO_UPGRADE_RETRY_MS - S).ask.is_none(), "第 {i} 次没成：不到 10 分钟不再试");
        t = failed + AUTO_UPGRADE_RETRY_MS;
        beat(&mut s, t, &report("mbp", "i1", vec![])).unwrap();
    }
    // 5 次都没成：之后不再自动试，接单不受影响
    assert!(host(&s, t).ask.is_none(), "试满 5 次不再试");
    beat(&mut s, t + 10 * AUTO_UPGRADE_RETRY_MS, &report("mbp", "i1", vec![])).unwrap();
    let h = host(&s, t + 10 * AUTO_UPGRADE_RETRY_MS);
    assert!(h.ask.is_none() && !h.paused, "试满 5 次不再试");
    // 人照样可以手动点「升级」：没成时不再接「自动升级」的半句
    beat(&mut s, t + 11 * AUTO_UPGRADE_RETRY_MS, &report("mbp", "i1", vec![])).unwrap();
    s.write(t + 11 * AUTO_UPGRADE_RETRY_MS, Origin::ui(None), |w| w.host_ask("mbp", "upgrade", false, "人")).unwrap();
    auto_upgrade_round(&mut s, t + 11 * AUTO_UPGRADE_RETRY_MS + S, Some("升级失败：网络不通"));
    let h = host(&s, t + 11 * AUTO_UPGRADE_RETRY_MS + 2 * S);
    assert_eq!(h.ops[0].text, "升级失败：网络不通；已恢复接单");
}

/// 临时失败后再试一次就升上去了：恢复接单、不再发起；CLI 机器下载发版附件 404 同样算临时失败
#[test]
fn auto_upgrade_retry_succeeds_after_release_is_published() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    s.write(T0, Origin::ui(None), |w| w.set_auto_upgrade(true)).unwrap();
    beat(&mut s, T0 + S, &report("mbp", "i1", vec![])).unwrap();
    auto_upgrade_round(&mut s, T0 + 2 * S, Some("升级失败：发版附件里没有 board-windows-x64.zip（https://example/board-windows-x64.zip 404）：这一版可能还没有单独的 board 包"));
    let failed = T0 + 3 * S;
    assert!(host(&s, failed).ops[0].text.ends_with("10 分钟后自动再试（第 2/5 次）"), "{:?}", host(&s, failed).ops);
    let t = failed + AUTO_UPGRADE_RETRY_MS;
    beat(&mut s, t, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!(host(&s, t).ask.map(|a| a.by), Some("自动升级（第 2 次）".into()));
    auto_upgrade_round(&mut s, t + S, None);
    let h = host(&s, t + 2 * S);
    assert_eq!(h.version, env!("CARGO_PKG_VERSION"));
    assert!(h.ask.is_none() && !h.paused, "{:?}", h.ops);
    assert!(h.ops[0].text.starts_with(&format!("升级到 {}，重新上线；已恢复接单", env!("CARGO_PKG_VERSION"))), "{:?}", h.ops);
    let mut cur = report("mbp", "i2", vec![]);
    cur.version = env!("CARGO_PKG_VERSION").into();
    beat(&mut s, t + 2 * S + 10 * AUTO_UPGRADE_RETRY_MS, &cur).unwrap();
    assert!(host(&s, t + 2 * S + 10 * AUTO_UPGRADE_RETRY_MS).ask.is_none());
}

/// 旧版本的 board 没有 `board update`：只提示一次，不反复暂停接单
#[test]
fn auto_upgrade_does_not_retry_hosts_that_cannot_update_themselves() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    s.write(T0, Origin::ui(None), |w| w.set_auto_upgrade(true)).unwrap();
    beat(&mut s, T0 + S, &report("mbp", "i1", vec![])).unwrap();
    let why = format!("升级失败：这个版本的 board 还不能自己升级（{NO_SELF_UPDATE}）：到这台机器上手动升级一次");
    auto_upgrade_round(&mut s, T0 + 2 * S, Some(&why));
    let h = host(&s, T0 + 3 * S);
    assert!(h.ops[0].text.ends_with("这台机器的 board 不会自己升级，自动升级不再重试"), "{:?}", h.ops);
    for k in 1..=6 {
        let at = T0 + 3 * S + k * AUTO_UPGRADE_RETRY_MS;
        beat(&mut s, at, &report("mbp", "i1", vec![])).unwrap();
        let h = host(&s, at);
        assert!(h.ask.is_none() && !h.paused, "第 {k} 个 10 分钟：不再自动试");
    }
}

/* ------------------------------ Git 凭据（gitcred.rs） ------------------------------ */

fn git_cred(host: &str, token: Option<&str>) -> crate::gitcred::GitCred {
    crate::gitcred::GitCred { host: host.into(), username: "buglan".into(), token: token.map(str::to_string), ..Default::default() }
}

#[test]
fn git_credential_is_masked_versioned_and_synced_to_hosts_that_use_it() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    beat(&mut s, T0, &report("win", "i2", vec![])).unwrap();
    // 新机器默认不改；服务端没设时版本是空的，配置里也不带 Git（改了凭据不会让不改的机器同步）
    let h = s.read(|r| r.host("mbp", T0)).unwrap();
    assert!(!h.sync_git, "Git 凭据新机器默认不改");
    assert_eq!(s.read(|r| r.host_work("mbp", T0)).unwrap().git.as_deref(), Some(""));
    // mbp 在界面上打开：记一条操作记录，下发一次同步（服务端没设时版本是空的）
    let h = s.write(T0, Origin::ui(None), |w| w.host_update_as("mbp", &HostPatch { sync_git: Some(true), ..Default::default() }, "buglan")).unwrap();
    assert!(h.sync_git);
    assert!(h.ops.iter().any(|o| o.text.contains("Git 凭据改成用服务器的")), "{:?}", h.ops);
    let on_ask = sync_of(&s, "mbp").ask.expect("开关变了要同步");
    synced(&mut s, "mbp", "i1", Some(on_ask.id), &on_ask.key, vec![]).unwrap();
    assert_eq!(s.read(|r| r.host_work("mbp", T0)).unwrap().git.as_deref(), Some(""));
    let key_win = s.read(|r| r.host_work("win", T0)).unwrap().config_key;
    synced(&mut s, "win", "i2", None, &key_win, vec![]).unwrap();
    // win 打开又改成不改：两次都记一条操作记录，各下发一次同步（关掉时去掉之前配的）
    let h = s.write(T0, Origin::ui(None), |w| w.host_update_as("win", &HostPatch { sync_git: Some(true), ..Default::default() }, "buglan")).unwrap();
    assert!(h.ops.iter().any(|o| o.text.contains("Git 凭据改成用服务器的")), "{:?}", h.ops);
    let win_on = sync_of(&s, "win").ask.expect("打开时同步");
    synced(&mut s, "win", "i2", Some(win_on.id), &win_on.key, vec![]).unwrap();
    let h = s.write(T0, Origin::ui(None), |w| w.host_update_as("win", &HostPatch { sync_git: Some(false), ..Default::default() }, "buglan")).unwrap();
    assert!(!h.sync_git);
    assert!(h.ops.iter().any(|o| o.text.contains("Git 凭据改成不改")), "{:?}", h.ops);
    let win_ask = sync_of(&s, "win").ask.expect("开关变了要同步（去掉之前配的）");
    synced(&mut s, "win", "i2", Some(win_ask.id), &win_ask.key, vec![]).unwrap();

    // 设置：主机规范化、只回末尾 4 位；没有令牌、主机写成 SSH 的拒绝
    assert!(s.write(T0, Origin::ui(None), |w| w.git_cred_put(git_cred("github.com", None), "buglan", "")).unwrap_err().message.contains("令牌"));
    assert!(s.write(T0, Origin::ui(None), |w| w.git_cred_put(git_cred("git@github.com:me/x.git", Some("t")), "buglan", "")).unwrap_err().message.contains("SSH"));
    assert!(s.write(T0, Origin::ui(None), |w| w.git_cred_put(crate::gitcred::GitCred { username: " ".into(), ..git_cred("github.com", Some("t")) }, "buglan", "")).unwrap_err().message.contains("用户名"));
    let out = s.write(T0, Origin::ui(None), |w| w.git_cred_put(git_cred(" https://GitHub.com/ ", Some("ghp_abcd1234")), "buglan", "")).unwrap();
    assert_eq!((out.host.as_str(), out.token.as_deref(), out.tail.as_deref()), ("github.com", None, Some("1234")));
    let full = s.read(|r| r.git_cred()).unwrap().unwrap();
    assert_eq!(full.token.as_deref(), Some("ghp_abcd1234"));
    assert!(!full.version.contains("ghp"));
    // 用服务器的机器：下发同步请求，操作记录里有谁改的、主机、末尾 4 位，没有原文
    let ask = sync_of(&s, "mbp").ask.expect("凭据变了要同步");
    assert_eq!(ask.by, "buglan");
    let h = s.read(|r| r.host("mbp", T0)).unwrap();
    let op = h.ops.iter().find(|o| o.text.contains("Git 凭据")).expect("留痕");
    assert_eq!((op.by.as_str(), op.text.as_str()), ("buglan", "设置了服务端的 Git 凭据：github.com · 用户 buglan · 令牌末尾 1234"));
    assert!(sync_of(&s, "win").ask.is_none(), "不改的机器不受影响");
    assert!(!s.read(|r| r.host("win", T0)).unwrap().ops.iter().any(|o| o.text.contains("服务端的 Git 凭据")));
    assert_eq!(s.read(|r| r.host_work("mbp", T0)).unwrap().git, Some(full.version.clone()));

    // 回报：同步结果里带 Git 这一行，记进操作记录
    let res = SyncResult {
        id: Some(ask.id),
        key: ask.key.clone(),
        git: GitSync { state: "synced".into(), count: 1, detail: GitDetail { host: "github.com".into(), ..Default::default() }, ..Default::default() },
        ..Default::default()
    };
    let h = s.write(T0, Origin::cli(), |w| w.host_synced("mbp", "i1", res)).unwrap();
    assert!(h.ops[0].text.ends_with("；Git 凭据（github.com）已按主机全局生效"), "{}", h.ops[0].text);
    let res = SyncResult { key: ask.key.clone(), git: GitSync { state: "failed".into(), why: Some("git 配置没写进去".into()), ..Default::default() }, ..Default::default() };
    let h = s.write(T0, Origin::cli(), |w| w.host_synced("mbp", "i1", res)).unwrap();
    assert!(h.ops[0].text.ends_with("；Git 凭据失败（git 配置没写进去）"), "{}", h.ops[0].text);

    // 令牌留空：保留原来的，内容没变不算改
    let same = s.write(T0, Origin::ui(None), |w| w.git_cred_put(git_cred("github.com", None), "buglan", "")).unwrap();
    assert_eq!(same.version, full.version);
    assert!(sync_of(&s, "mbp").ask.is_none());
    // 只换令牌：版本变了，再同步
    let v2 = s.write(T0, Origin::ui(None), |w| w.git_cred_put(git_cred("github.com", Some("ghp_new")), "buglan", "")).unwrap().version;
    assert_ne!(v2, full.version);
    assert!(sync_of(&s, "mbp").ask.is_some());
    assert!(s.read(|r| r.host("mbp", T0)).unwrap().ops.iter().any(|o| o.text.starts_with("改了服务端的 Git 凭据")));
    // 删掉：记一条，用服务器的机器再同步（去掉之前配的）；再删报没有
    s.write(T0, Origin::ui(None), |w| w.git_cred_rm("buglan")).unwrap();
    assert!(s.read(|r| r.git_cred()).unwrap().is_none());
    assert_eq!(s.read(|r| r.host_work("mbp", T0)).unwrap().git.as_deref(), Some(""));
    assert!(s.read(|r| r.host("mbp", T0)).unwrap().ops.iter().any(|o| o.text.starts_with("删掉了服务端的 Git 凭据")));
    assert_eq!(s.write(T0, Origin::ui(None), |w| w.git_cred_rm("buglan")).unwrap_err().code, ErrorCode::NotFound);
}

#[test]
fn git_credential_commit_identity_is_paired_masked_and_versioned() {
    let mut s = store();
    let one = crate::gitcred::GitCred { commit_name: "BUGLAN".into(), ..git_cred("github.com", Some("ghp_abcd1234")) };
    assert!(s.write(T0, Origin::ui(None), |w| w.git_cred_put(one, "buglan", "")).unwrap_err().message.contains("一起填"));

    let both = crate::gitcred::GitCred { commit_name: " BUGLAN ".into(), commit_email: " 1831353087@qq.com ".into(), ..git_cred("github.com", Some("ghp_abcd1234")) };
    let bad = crate::gitcred::GitCred { commit_name: "BUGLAN".into(), commit_email: "not-an-email".into(), ..both.clone() };
    assert!(s.write(T0, Origin::ui(None), |w| w.git_cred_put(bad, "buglan", "")).unwrap_err().message.contains("邮箱地址"));
    let out = s.write(T0, Origin::ui(None), |w| w.git_cred_put(both.clone(), "buglan", "")).unwrap();
    assert_eq!((out.commit_name.as_str(), out.commit_email.as_str(), out.token.as_deref(), out.tail.as_deref()), ("BUGLAN", "1831353087@qq.com", None, Some("1234")));
    let full = s.read(|r| r.git_cred()).unwrap().unwrap();
    assert_eq!((full.commit_name.as_str(), full.commit_email.as_str()), ("BUGLAN", "1831353087@qq.com"));
    assert!(!full.version.contains("BUGLAN") && !full.version.contains("1831353087"));

    let same = s.write(T0, Origin::ui(None), |w| w.git_cred_put(crate::gitcred::GitCred { token: None, ..both.clone() }, "buglan", "")).unwrap();
    assert_eq!(same.version, full.version, "留空令牌时提交身份也没变");
    let changed = s.write(T0, Origin::ui(None), |w| w.git_cred_put(crate::gitcred::GitCred { commit_email: "other@example.com".into(), ..both }, "buglan", "")).unwrap();
    assert_ne!(changed.version, full.version, "提交身份改了要重新同步");
}

#[test]
fn existing_hosts_get_the_git_switch_on_after_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
        s.conn().execute_batch("ALTER TABLE hosts DROP COLUMN sync_git").unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    assert!(s.read(|r| r.host("mbp", T0)).unwrap().sync_git, "升级前已有的机器保持原来的效果：用服务器的");
    // 升级之后新连上的机器默认不改；再打开一次不重复补，已有的机器还是原来的值
    beat(&mut s, T0, &report("win", "i2", vec![])).unwrap();
    assert!(!s.read(|r| r.host("win", T0)).unwrap().sync_git, "升级之后的新机器默认不改");
    drop(s);
    let s = Store::open(&path).unwrap();
    assert!(s.read(|r| r.host("mbp", T0)).unwrap().sync_git);
    assert!(!s.read(|r| r.host("win", T0)).unwrap().sync_git);
}

#[test]
fn new_hosts_on_a_0_1_34_database_default_to_no_git_sync() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-default.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
        // 模拟 0.1.34 的库：#386 迁移时用 ALTER 补的列，表上的默认值是 1
        s.conn().execute_batch("ALTER TABLE hosts DROP COLUMN sync_git").unwrap();
        s.conn().execute_batch("ALTER TABLE hosts ADD COLUMN sync_git INTEGER NOT NULL DEFAULT 1").unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    assert!(s.read(|r| r.host("mbp", T0)).unwrap().sync_git, "已有的机器还是用服务器的");
    beat(&mut s, T0, &report("win", "i2", vec![])).unwrap();
    assert!(!s.read(|r| r.host("win", T0)).unwrap().sync_git, "升级后新连上的机器默认不改，不取表上的旧默认值");
}

#[test]
fn existing_hosts_keep_their_git_switch_values_after_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        beat(&mut s, T0, &report("on", "i1", vec![])).unwrap();
        beat(&mut s, T0, &report("off", "i2", vec![])).unwrap();
        s.write(T0, Origin::ui(None), |w| w.host_update_as("on", &HostPatch { sync_git: Some(true), ..Default::default() }, "buglan")).unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert!(s.read(|r| r.host("on", T0)).unwrap().sync_git, "开着的还是开着");
    assert!(!s.read(|r| r.host("off", T0)).unwrap().sync_git, "关着的还是关着");
}

/* ------------------------------ 照另一台机器配（hostcopy.rs，default/main#398） ------------------------------ */

fn named(mut r: HostReport, name: &str, os: &str) -> HostReport {
    r.name = name.into();
    r.os = os.into();
    r.config_sync = true;
    r
}

fn update(s: &mut Store, name: &str, p: HostPatch) -> HostInfo {
    s.write(T0, Origin::ui(None), |w| w.host_update(name, &p)).unwrap()
}

fn copy(s: &mut Store, to: &str, from: &str, dry: bool) -> Result<crate::hostcopy::HostCopy> {
    s.write(T0, Origin::ui(None), |w| w.host_copy(to, from, "buglan", dry))
}

fn run(agent: &str, provider: &str, model: &str, effort: &str) -> RunPatch {
    serde_json::from_value(serde_json::json!({ "agent": agent, "mode": "own", "provider": provider, "model": model, "effort": effort })).unwrap()
}

#[test]
fn host_copy_previews_then_copies_once_and_leaves_workspace_name_and_accept_mode() {
    let mut s = store();
    providers(&mut s);
    beat(&mut s, T0, &named(report("a", "ia", vec![]), "a", "macOS 15.4")).unwrap();
    let mut b = named(report("b", "ib", vec![]), "b", "macOS 26");
    b.workspace = Some(HostWorkspace { path: "/w/b".into(), ..Default::default() });
    beat(&mut s, T0, &b).unwrap();
    // A：claude 指定 p1、MCP 开、最多 3 个会话、一个会话 2 张、9 点到 18 点接单、自动更新 agent、自动领
    update(
        &mut s,
        "a",
        HostPatch {
            run: Some(run("claude", "p1", "model-a", "high")),
            sync_mcp: Some(true),
            max: Some(3),
            per_session: Some(2),
            windows: Some(vec![[540, 1080]]),
            agent_auto: Some(true),
            auto: Some(true),
            ..Default::default()
        },
    );

    // 预览：逐项列出从什么变成什么，没有变化的不列；不写
    let dry = copy(&mut s, "b", "a", true).unwrap();
    assert!(!dry.done);
    let rows: Vec<(&str, &str, &str)> = dry.changes.iter().map(|c| (c.label.as_str(), c.from.as_str(), c.to.as_str())).collect();
    assert_eq!(
        rows,
        vec![
            ("Claude Code 的模型", "不改", "p1 · model-a · 强度 high"),
            ("MCP", "不改", "用服务器的"),
            ("自动更新 agent", "关闭", "自动更新"),
            ("最多几个会话", "1", "3"),
            ("一个会话几张", "4", "2"),
            ("接单时段", "全天", "09:00–18:00"),
        ]
    );
    assert!(dry.notes.is_empty(), "{:?}", dry.notes);
    assert_eq!(s.read(|r| r.host("b", T0)).unwrap().max, 1, "预览不写");

    // 确认：一次写进去，B 收到同步请求，记一条操作记录
    let done = copy(&mut s, "b", "a", false).unwrap();
    assert!(done.done);
    let hb = s.read(|r| r.host("b", T0)).unwrap();
    let c = claude_of(&hb);
    assert_eq!((c.run_mode.as_deref(), c.run_provider.as_deref(), c.run_model.as_deref(), c.run_effort.as_deref()), (Some("own"), Some("p1"), Some("model-a"), Some("high")));
    assert!(hb.sync_mcp && !hb.sync_skills && !hb.sync_git);
    assert_eq!((hb.max, hb.per_session, hb.windows.clone(), hb.agent_auto), (3, 2, vec![[540, 1080]], true));
    // 不复制：工作空间、机器名、接单三态
    assert_eq!(hb.name, "b");
    assert_eq!(hb.workspace.as_ref().map(|w| w.path.as_str()), Some("/w/b"));
    assert!(!hb.auto && !hb.paused, "还是「只做派给它的」");
    assert!(hb.sync.ask.is_some(), "改了会写进 agent 配置的：下发同步请求");
    assert_eq!(hb.ops[0].by, "buglan");
    assert_eq!(hb.ops[0].text, "照 a 配：改了 Claude Code 的模型、MCP、自动更新 agent、最多几个会话、一个会话几张、接单时段");
    // 之后各改各的：再预览就是已经一样；A 再改不影响 B
    assert!(copy(&mut s, "b", "a", true).unwrap().changes.is_empty());
    update(&mut s, "a", HostPatch { max: Some(5), ..Default::default() });
    assert_eq!(s.read(|r| r.host("b", T0)).unwrap().max, 3);
    // 已经一样时确认也不写、不记
    let n = s.read(|r| r.host("b", T0)).unwrap().ops.len();
    update(&mut s, "a", HostPatch { max: Some(3), ..Default::default() });
    assert!(!copy(&mut s, "b", "a", false).unwrap().done);
    assert_eq!(s.read(|r| r.host("b", T0)).unwrap().ops.len(), n);

    assert_eq!(copy(&mut s, "b", "b", true).unwrap_err().code, ErrorCode::Usage, "不能照自己配");
    assert_eq!(copy(&mut s, "b", "nope", true).unwrap_err().code, ErrorCode::NoSuchHost);
}

#[test]
fn host_copy_keeps_env_when_the_systems_differ_or_the_target_cannot_use_it() {
    let mut s = store();
    beat(&mut s, T0, &named(win_report("ia"), "a", "Windows 11")).unwrap();
    beat(&mut s, T0, &named(win_report("ib"), "b", "Windows 10")).unwrap();
    beat(&mut s, T0, &named(win_report("ic"), "c", "macOS 26")).unwrap();
    // A：mycli 在 PowerShell 7 里启动
    update(&mut s, "a", HostPatch { env: Some(EnvPatch { agent: "mycli".into(), env: "pwsh".into() }), ..Default::default() });

    // 同一种系统、B 上 mycli 在 PowerShell 7 里找得到：复制
    let b = copy(&mut s, "b", "a", false).unwrap();
    assert_eq!(b.changes.iter().map(|c| (c.label.as_str(), c.from.as_str(), c.to.as_str())).collect::<Vec<_>>(), vec![("mycli 的启动环境", "默认", "PowerShell 7")]);
    let mycli = s.read(|r| r.host("b", T0)).unwrap().agents.into_iter().find(|a| a.id == "mycli").unwrap();
    assert_eq!(mycli.env.as_deref(), Some("pwsh"));

    // A 是 Windows、C 是 macOS：启动环境不复制，写明
    let c = copy(&mut s, "c", "a", true).unwrap();
    assert!(c.changes.is_empty(), "{:?}", c.changes);
    assert_eq!(c.notes, vec!["启动环境没复制：a 是 Windows、c 是 macOS，两边的启动环境不通用（mycli）".to_string()]);

    // 同一种系统，但这台机器上在那个环境里找不到：保持原来的，写明原因
    let mut d = named(win_report("id"), "d", "Windows 11");
    d.agents.retain(|a| a.id == "claude");
    d.agents.push(HostAgent { id: "mycli".into(), name: "mycli".into(), version: "1.5.0".into(), found: true, envs: vec![check("git-bash", true, "1.5.0"), check("pwsh", false, "")], ..Default::default() });
    beat(&mut s, T0, &d).unwrap();
    let d = copy(&mut s, "d", "a", true).unwrap();
    assert!(d.changes.is_empty());
    assert_eq!(d.notes.len(), 1);
    assert!(d.notes[0].starts_with("mycli 的启动环境没复制：") && d.notes[0].contains("没找到"), "{:?}", d.notes);
}

#[test]
fn host_copy_stores_settings_for_agents_the_target_has_not_found_yet() {
    let mut s = store();
    providers(&mut s);
    let mut a = named(report("a", "ia", vec![]), "a", "macOS 15.4");
    a.agents.push(dsh(true));
    beat(&mut s, T0, &a).unwrap();
    beat(&mut s, T0, &named(report("b", "ib", vec![]), "b", "macOS 15.4")).unwrap();
    update(&mut s, "a", HostPatch { run: Some(run("dsh", "p1", "model-a", "low")), agent: Some("dsh".into()), ..Default::default() });

    let out = copy(&mut s, "b", "a", false).unwrap();
    assert_eq!(out.changes.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), vec!["DeepSeek Harness 的模型"]);
    assert_eq!(
        out.notes,
        vec!["这台机器上还没找到 DeepSeek Harness：它的设置先存下，装上并重新检查后生效".to_string(), "默认 agent 没改：这台机器上还没找到 DeepSeek Harness".to_string()]
    );
    // B 装上 dsh、重新检查（上报里有它）：用的是复制来的设置
    let mut b = named(report("b", "ib", vec![]), "b", "macOS 15.4");
    b.agents.push(dsh(true));
    let h = beat(&mut s, T0 + S, &b).unwrap().host;
    let d = h.agents.iter().find(|x| x.id == "dsh").unwrap();
    assert_eq!((d.run_mode.as_deref(), d.run_provider.as_deref(), d.run_model.as_deref(), d.run_effort.as_deref()), (Some("own"), Some("p1"), Some("model-a"), Some("low")));
}

#[test]
fn host_preset_copies_on_first_connect_only() {
    let mut s = store();
    providers(&mut s);
    beat(&mut s, T0, &named(report("a", "ia", vec![]), "a", "macOS 15.4")).unwrap();
    update(&mut s, "a", HostPatch { run: Some(run("claude", "p1", "model-a", "high")), sync_skills: Some(true), max: Some(2), auto: Some(true), ..Default::default() });
    // 添加机器时选了「上线后照 a 配」：记在新建的令牌（7）上
    assert_eq!(s.write(T0, Origin::ui(None), |w| w.host_preset(7, "nope", "buglan")).unwrap_err().code, ErrorCode::NoSuchHost);
    s.write(T0, Origin::ui(None), |w| w.host_preset(7, "a", "buglan")).unwrap();

    // 用这个令牌的机器第一次上线：照 a 现在的设置配（接单三态、工作空间不复制）
    let h = s.write(T0 + S, Origin::cli(), |w| w.host_beat(&named(report("n", "in", vec![]), "n", "macOS 26"), Some(7))).unwrap().host;
    assert_eq!(claude_of(&h).run_provider.as_deref(), Some("p1"));
    assert!(h.sync_skills && h.max == 2 && !h.auto);
    assert_eq!(h.workspace.as_ref().map(|w| w.path.as_str()), Some("/tmp"));
    assert_eq!(h.ops[0].by, "buglan");
    assert_eq!(h.ops[0].text, "上线后照 a 配：改了 Claude Code 的模型、skills、最多几个会话");
    // 同步请求在这次写完时下发（board host 下一次取任务时拿到）
    assert!(s.read(|r| r.host("n", T0 + S)).unwrap().sync.ask.is_some(), "照着配完下发同步请求");
    // 用过就删：同一个令牌再连上来一台新机器，按默认设置
    let m = s.write(T0 + S, Origin::cli(), |w| w.host_beat(&named(report("m", "im", vec![]), "m", "macOS 26"), Some(7))).unwrap().host;
    assert_eq!((m.max, m.sync_skills), (1, false));
    assert!(m.ops.is_empty(), "{:?}", m.ops);

    // 来源机器到上线时已经不在了：按默认设置建立，记一条
    s.write(T0, Origin::ui(None), |w| w.host_preset(8, "a", "buglan")).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_remove("a", &Actor::human(), "buglan")).unwrap();
    let g = s.write(T0 + S, Origin::cli(), |w| w.host_beat(&named(report("g", "ig", vec![]), "g", "macOS 26"), Some(8))).unwrap().host;
    assert_eq!(g.ops[0].text, "上线后照 a 配：a 已经不在了，按默认设置建立");
}

/* ------------------------------ 「跑第一张卡」清单（firstrun.rs，default/main#397） ------------------------------ */

/// 这块白板的清单：每项 (key, state, go)
fn checklist(s: &Store, at: i64) -> (bool, Vec<(String, String, Option<String>)>) {
    let all = s.read(|r| r.checklist(at)).unwrap();
    let c = all.into_iter().find(|c| c.board == B).expect("每块白板一份");
    (c.done, c.items.into_iter().map(|i| (i.key, i.state, i.go)).collect())
}

fn state_of(s: &Store, at: i64, key: &str) -> (String, Option<String>) {
    checklist(s, at).1.into_iter().find(|(k, ..)| k == key).map(|(_, st, go)| (st, go)).unwrap()
}

fn why_of(s: &Store, at: i64, key: &str) -> String {
    s.read(|r| r.checklist(at)).unwrap().into_iter().find(|c| c.board == B).unwrap().items.into_iter().find(|i| i.key == key).unwrap().why
}

#[test]
fn checklist_without_hosts_points_to_add_host_and_waits_for_the_rest() {
    let s = store();
    let (done, items) = checklist(&s, T0);
    assert!(!done);
    let o = |s: &str| Some(s.to_string());
    assert_eq!(
        items,
        vec![
            ("host".into(), "fail".into(), o("add-host")),
            ("agent".into(), "wait".into(), None),
            ("repo".into(), "fail".into(), o("project-repo")),
            ("push".into(), "wait".into(), None),
            ("auto".into(), "fail".into(), o("rules")),
        ]
    );
    let c = s.read(|r| r.checklist(T0)).unwrap();
    assert!(c[0].items.iter().find(|i| i.key == "auto").unwrap().auto, "第 5 项标明只影响自动领取");
    assert!(why_of(&s, T0, "auto").contains("只影响自动领取"));
    // 工作区数据里带着
    assert!(s.read(|r| r.workspace(T0)).unwrap().checklist.iter().any(|c| c.board == B));
}

#[test]
fn checklist_host_item_needs_an_online_host_that_can_take_the_board() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!(state_of(&s, T0, "host").0, "ok");
    assert!(why_of(&s, T0, "host").starts_with("mbp 在线"));
    // 离线
    assert_eq!(state_of(&s, T0 + 120 * S, "host"), ("fail".into(), Some("hosts".into())));
    assert!(why_of(&s, T0 + 120 * S, "host").contains("机器都不在线"));
    // 在线但接不了：没有工作空间，去处是机器详情
    s.write(T0, Origin::ui(None), |w| {
        w.host_update("mbp", &HostPatch { workspace: Some(HostWorkspace::default()), ..Default::default() })
    })
    .unwrap();
    assert_eq!(state_of(&s, T0, "host"), ("fail".into(), Some("host-settings:mbp".into())));
    assert_eq!(why_of(&s, T0, "host"), "mbp：还没有配工作空间");
}

#[test]
fn checklist_agent_item_follows_skills_and_selftest() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    // 找得到、看得到 board-skills，没自检过：还没确认，去处是自检
    assert_eq!(state_of(&s, T0, "agent"), ("unknown".into(), Some("selftest:mbp".into())));
    // 正在自检
    let ask = selftest(&mut s, T0, None).unwrap().selftest.asks[0].clone();
    assert_eq!(state_of(&s, T0, "agent"), ("unknown".into(), None));
    // 自检通过
    selftest_done(&mut s, T0, "i1", ask.id, vec![item("agent", "claude", true, None)]);
    assert_eq!(state_of(&s, T0, "agent"), ("ok".into(), None));
    // 自检没通过：去机器详情的 agent 页
    let ask = selftest(&mut s, T0, None).unwrap().selftest.asks[0].clone();
    selftest_done(&mut s, T0, "i1", ask.id, vec![item("agent", "claude", false, Some("没登录"))]);
    assert_eq!(state_of(&s, T0, "agent"), ("fail".into(), Some("host-agents:mbp".into())));
    assert_eq!(why_of(&s, T0, "agent"), "mbp 上的 Claude Code 自检没通过：没登录");
    // 看不到 board-skills
    let mut r = report("mbp", "i1", vec![]);
    r.agents = vec![HostAgent { no_skill: Some("~/.claude/skills".into()), ..claude(true) }];
    beat(&mut s, T0 + S, &r).unwrap();
    assert_eq!(state_of(&s, T0 + S, "agent").0, "fail");
    assert!(why_of(&s, T0 + S, "agent").contains("看不到 board-skills"));
    // 装了 agent、缺 Node.js（npx）：说缺什么，不让人去重装 agent（#458）
    r.agents = vec![HostAgent { missing: Some("Node.js（npx）".into()), version: "2.4.1".into(), ..claude(false) }];
    beat(&mut s, T0 + 2 * S, &r).unwrap();
    assert_eq!(state_of(&s, T0 + 2 * S, "agent").0, "fail");
    assert_eq!(why_of(&s, T0 + 2 * S, "agent"), "mbp 上的 Claude Code 缺 Node.js（npx）：装上后点「重新检查」");
}

#[test]
fn checklist_repo_and_push_items() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    assert_eq!(state_of(&s, T0, "repo"), ("fail".into(), Some("project-repo".into())));
    assert_eq!(state_of(&s, T0, "push"), ("wait".into(), None), "没设仓库时推送等前一项");
    ui(&mut s, |w| w.set_repo_branch("default", Some("git@github.com:o/r.git"), None)).unwrap();
    assert_eq!(state_of(&s, T0, "repo").0, "ok");
    // 没自检过：还没确认
    assert_eq!(state_of(&s, T0, "push"), ("unknown".into(), Some("selftest:mbp".into())));
    // 自检能推送（同一个项目的哪块白板都算）
    let ask = selftest(&mut s, T0, None).unwrap().selftest.asks[0].clone();
    selftest_done(&mut s, T0, "i1", ask.id, vec![item("git", B, true, None)]);
    assert_eq!(state_of(&s, T0, "push"), ("ok".into(), None));
    // 推不了：去 Git 凭据
    let ask = selftest(&mut s, T0, None).unwrap().selftest.asks[0].clone();
    selftest_done(&mut s, T0, "i1", ask.id, vec![item("git", B, false, Some("Permission denied"))]);
    assert_eq!(state_of(&s, T0, "push"), ("fail".into(), Some("git".into())));
    assert_eq!(why_of(&s, T0, "push"), "mbp 推不了：Permission denied");
}

#[test]
fn checklist_auto_item_names_what_is_missing() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    // 没有规则、机器不是自动领
    assert_eq!(state_of(&s, T0, "auto"), ("fail".into(), Some("rules".into())));
    assert_eq!(why_of(&s, T0, "auto"), "还没有领取规则或兜底；能接的机器都不是「自动领」：只影响自动领取，手动派发照常");
    // 只有兜底也算规则；机器还是「只做派给它的」
    s.write(T0, Origin::ui(None), |w| w.sched_update(Some(&[]), Some("claude"))).unwrap();
    assert_eq!(state_of(&s, T0, "auto"), ("fail".into(), Some("accept".into())));
    assert_eq!(why_of(&s, T0, "auto"), "能接的机器都不是「自动领」：只影响自动领取，手动派发照常");
    set_host(&mut s, HostPatch { auto: Some(true), ..Default::default() }).unwrap();
    assert_eq!(state_of(&s, T0, "auto"), ("ok".into(), None));
    // 白板不参与自动领取
    s.write(T0, Origin::ui(None), |w| w.set_board_auto(B, false)).unwrap();
    assert_eq!(state_of(&s, T0, "auto"), ("fail".into(), Some("board-auto".into())));
}

#[test]
fn checklist_is_done_once_a_machine_submitted_a_card() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let a = new_card(&mut s, T0, "A");
    let d = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    begin(&mut s, T0 + S, d.id).unwrap();
    assert!(!checklist(&s, T0 + S).0);
    let me = Actor::agent("mbp/opus-5.5");
    exec(&mut s, T0 + 2 * S, &me, Cmd::Beat { id: a, done: true }).unwrap();
    exec(&mut s, T0 + 2 * S, &me, Cmd::Move { id: a, column: "review".into(), top: false, index: None, order: None, before: None, after: None }).unwrap();
    end(&mut s, T0 + 3 * S, d.id, Some(0), &[]);
    assert!(checklist(&s, T0 + 3 * S).0, "有卡被机器交审过");
}

/// 人强制释放后旧派发还在跑（#428，现场 #414）：卡片回到可领取状态，但两台机器的自动领取、不带 ID 的领取都不会再领它，
/// 带 ID 领取报 DISPATCHED，不会出现第二个派发；旧派发结束（已停止）后，这台机器这一段接单里不再领它，别的机器按规则照常领一次
#[test]
fn force_release_does_not_redispatch_while_the_old_dispatch_runs() {
    let mut s = store();
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    beat(&mut s, T0, &report("mac", "i2", vec![])).unwrap();
    lead_round(&mut s);
    // mbp 留出第二个会话位：后面领不到是因为卡片有在跑的派发，不是因为没空位
    s.write(T0, Origin::ui(None), |w| w.host_update("mbp", &HostPatch { max: Some(2), ..Default::default() })).unwrap();
    s.write(T0, Origin::ui(None), |w| w.host_update("mac", &HostPatch { auto: Some(true), ..Default::default() })).unwrap();
    let a = new_card_tagged(&mut s, "A");
    for h in ["mbp", "mac"] {
        s.write(T0 + S, Origin::ui(None), |w| w.host_round(h)).unwrap();
    }
    let d = s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().expect("定时领取应该开始一张");
    assert_eq!((d.card, d.state.as_str()), (a, "running"));

    // 人强制释放：卡片回到待办最上面，旧派发只是请求了停止，还在跑
    exec(&mut s, T0 + 3 * S, &Actor::human(), Cmd::Release { id: a }).unwrap();
    assert_eq!((card(&s, a).card.column.as_str(), card(&s, a).card.holder.clone()), ("todo", None));
    let old = get(&s, d.id);
    assert_eq!((old.state.as_str(), old.stop_by.is_some()), ("running", true));

    // 旧派发结束之前：两台机器都不再派它，agent 也领不到
    for (h, i) in [("mbp", "i1"), ("mac", "i2")] {
        assert!(s.write(T0 + 4 * S, Origin::cli(), |w| w.dispatch_pull(h, i, None)).unwrap().is_none(), "{h} 不该重派");
    }
    let beta = Actor::agent("beta/x");
    assert_eq!(exec(&mut s, T0 + 5 * S, &beta, Cmd::Claim { id: None }).unwrap_err().code, ErrorCode::EmptyTodo);
    let e = exec(&mut s, T0 + 5 * S, &beta, Cmd::Claim { id: Some(a) }).unwrap_err();
    assert_eq!(e.code, ErrorCode::Dispatched, "{}", e.message);
    assert!(e.message.contains("还在跑"), "{}", e.message);
    let all = |s: &Store| s.read(|r| r.card_dispatches(&r.resolve(B)?, a)).unwrap();
    assert_eq!(all(&s).len(), 1, "没有第二个派发");

    // 旧派发结束（已停止）：mbp 这一段接单里不再领它；mac 照常领，只领一次
    let ended = end(&mut s, T0 + 6 * S, d.id, None, &[]);
    assert_eq!(ended.result.as_deref(), Some("stopped"));
    assert!(s.write(T0 + 7 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none(), "mbp 这一段接单里不再领被停下的卡");
    let again = s.write(T0 + 7 * S, Origin::cli(), |w| w.dispatch_pull("mac", "i2", None)).unwrap().expect("旧派发结束后别的机器按规则领");
    assert_eq!((again.card, again.host.as_str()), (a, "mac"));
    assert_eq!(all(&s).len(), 2);
}

/// 错误节点里的卡片（#428）：带着命中领取规则的标签，自动领取、不带 ID 的领取都不领，带 ID 领取 NOT_IN_TODO；agent 也移不出来
#[test]
fn error_status_cards_are_never_claimed_even_with_matching_labels() {
    let mut s = store();
    let key = add_error_status(&mut s);
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    lead_round(&mut s);
    let a = new_card_tagged(&mut s, "A");
    let mv = |id: i64, to: &str| Cmd::Move { id, column: to.into(), top: false, index: None, order: None, before: None, after: None };
    exec(&mut s, T0, &Actor::human(), mv(a, &key)).unwrap();
    s.write(T0 + S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(s.write(T0 + 2 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none(), "错误节点里的卡片不被自动领取");
    let beta = Actor::agent("beta/x");
    assert_eq!(exec(&mut s, T0 + 3 * S, &beta, Cmd::Claim { id: None }).unwrap_err().code, ErrorCode::EmptyTodo);
    assert_eq!(exec(&mut s, T0 + 3 * S, &beta, Cmd::Claim { id: Some(a) }).unwrap_err().code, ErrorCode::NotInTodo);
    assert_eq!(exec(&mut s, T0 + 3 * S, &beta, mv(a, "todo")).unwrap_err().code, ErrorCode::ForbiddenColumn, "错误节点只有人能移出");
    assert_eq!(card(&s, a).card.column, key);
    // 人移回待办后照常领
    exec(&mut s, T0 + 4 * S, &Actor::human(), mv(a, "todo")).unwrap();
    assert_eq!(s.write(T0 + 5 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().map(|d| d.card), Some(a));
}

/// 人从可领取状态以外派发（#498）：错误节点、待审核、终点里的卡片一步进锁定状态排队，不经过可领取状态，
/// 带着命中领取规则的标签也不会先被自动领走；动态写明从哪里派的；排队时取消回到派发前的状态。
/// agent 照旧只能从可领取状态派发；锁定状态里、被持有的卡片照旧拒绝
#[test]
fn humans_dispatch_from_any_unlocked_status_in_one_step() {
    let mut s = store();
    let key = add_error_status(&mut s);
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    lead_round(&mut s);
    let mv = |id: i64, to: &str| Cmd::Move { id, column: to.into(), top: false, index: None, order: None, before: None, after: None };
    let a = new_card_tagged(&mut s, "A");
    exec(&mut s, T0, &Actor::human(), mv(a, &key)).unwrap();
    // agent 不能从错误节点派发
    let by_agent = s.write(T0, Origin::cli(), |w| {
        let r = w.resolve(B)?;
        w.dispatch_create(&r, a, "mbp", None, &Actor::agent("beta/x"))
    });
    assert_eq!(by_agent.unwrap_err().code, ErrorCode::NotInTodo);
    assert_eq!(card(&s, a).card.column, key);
    // 人派发：直接进「进行中」排队，claimed_from 记着错误节点
    let d = dispatch(&mut s, T0 + S, a, "mbp", None).unwrap();
    let c = card(&s, a);
    assert_eq!((d.state.as_str(), c.card.column.as_str(), c.card.holder.as_deref()), ("queued", "doing", None));
    assert_eq!(c.card.claimed_from.as_deref(), Some(key.as_str()));
    let at = c.activity.iter().rposition(|x| x.action == "dispatch").unwrap();
    let text = &c.activity[at].text;
    assert!(text.starts_with("从「搁置」派发到 mbp · ") && text.ends_with("在「进行中」排队"), "{text}");
    assert!(!c.activity[at..].iter().any(|x| x.text.contains("「待办」")), "中间不经过可领取状态");
    // 排队时取消：回到错误节点最上面，不回待办（回去了带标签的卡会被马上自动领走）
    s.write(T0 + 2 * S, Origin::ui(None), |w| w.dispatch_cancel(d.id, &Actor::human(), "buglan")).unwrap();
    let c = card(&s, a);
    assert_eq!((c.card.column.as_str(), c.card.claimed_from.as_deref()), (key.as_str(), None));
    assert!(c.activity.iter().any(|x| x.text.contains("卡片回到「搁置」最上面")), "{:?}", c.activity.iter().map(|x| &x.text).collect::<Vec<_>>());
    s.write(T0 + 3 * S, Origin::ui(None), |w| w.host_round("mbp")).unwrap();
    assert!(s.write(T0 + 3 * S, Origin::cli(), |w| w.dispatch_pull("mbp", "i1", None)).unwrap().is_none(), "回到错误节点后不被自动领取");
    // 终点里的卡片派给任意机器：同样一步进「进行中」，开始后照常领取
    let e = new_card(&mut s, T0, "E");
    exec(&mut s, T0, &Actor::human(), mv(e, "done")).unwrap();
    let de = s
        .write(T0 + 4 * S, Origin::ui(None), |w| {
            let r = w.resolve(B)?;
            w.dispatch_create_any(&r, e, None, &Actor::human())
        })
        .unwrap();
    assert_eq!(card(&s, e).card.column, "doing");
    assert!(card(&s, e).activity.iter().any(|x| x.text.starts_with("从「已完成」派发到任意机器")));
    begin(&mut s, T0 + 5 * S, de.id).unwrap();
    let c = card(&s, e);
    assert_eq!((c.card.column.as_str(), c.card.holder.is_some()), ("doing", true));
    // 锁定状态里被持有的卡片：人也不能派发
    let f = new_card(&mut s, T0, "F");
    exec(&mut s, T0 + 6 * S, &Actor::agent("beta/x"), Cmd::Claim { id: Some(f) }).unwrap();
    let err = dispatch(&mut s, T0 + 6 * S, f, "mbp", None).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotInTodo);
    assert!(err.message.contains("正在被领取"), "{}", err.message);
}

/// 排队的派发被取消时，卡片已经不在它排队的锁定状态里（这里直接把它放进错误节点）：只清掉记录，不把它挪回可领取状态（#428）
#[test]
fn canceling_a_queue_never_moves_a_card_out_of_the_error_status() {
    let mut s = store();
    let key = add_error_status(&mut s);
    beat(&mut s, T0, &report("mbp", "i1", vec![])).unwrap();
    let (a, b) = (new_card(&mut s, T0, "A"), new_card(&mut s, T0, "B"));
    let first = dispatch(&mut s, T0, a, "mbp", None).unwrap();
    begin(&mut s, T0 + S, first.id).unwrap();
    // 最多 1 个会话：第二张排队，进了「进行中」，记着原来的「待办」
    let queued = dispatch(&mut s, T0 + 2 * S, b, "mbp", None).unwrap();
    assert_eq!((queued.state.as_str(), card(&s, b).card.column.as_str()), ("queued", "doing"));
    s.conn().execute("UPDATE cards SET col = ?1 WHERE id = ?2", rusqlite::params![key, b]).unwrap();
    s.write(T0 + 3 * S, Origin::ui(None), |w| w.dispatch_cancel(queued.id, &Actor::human(), "buglan")).unwrap();
    assert_eq!(card(&s, b).card.column, key, "错误节点只有人能移出");
    assert_eq!(card(&s, b).card.claimed_from, None);
}
