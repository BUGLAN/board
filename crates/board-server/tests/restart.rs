//! 服务端重启：停机时间不计入心跳超时（#95）。

use board_core::*;
use board_server::auth::Auth;
use board_server::{AppState, Config};
use std::path::Path;
use std::process::Command;

const S: i64 = 1000;
const B: &str = "default/main";

fn report(running: Vec<i64>) -> HostReport {
    HostReport {
        name: "mbp".into(),
        instance: "i1".into(),
        resume: None,
        os: "Windows".into(),
        version: "0.1.6".into(),
        agents: vec![HostAgent { id: "claude".into(), name: "Claude Code".into(), version: "2.4.1".into(), found: true, model: "opus-5.5".into(), ..Default::default() }],
        envs: vec![],
        dirs: vec![HostDir { board: B.into(), path: "E:\\repo\\board".into(), ..Default::default() }],
        dirs_rev: 0,
        running,
        done: false,
    }
}

fn new_card(s: &mut Store, at: i64, title: &str) -> i64 {
    s.write(at, Origin::ui(None), |w| {
        let b = w.resolve(B)?;
        w.exec(&b, &Actor::human(), Cmd::New { title: title.into(), body: None, column: None, top: false, labels: vec![] })
    })
    .unwrap()
    .card
    .id
}

/// 停机前的状态：名字 check 过、有领取和心跳、机器在线、一张卡由派发在跑
fn seed(path: &Path, at: i64) {
    let mut s = Store::open(path).unwrap();
    s.ensure_default(at).unwrap();
    s.write(at, Origin::cli(), |w| w.host_beat(&report(vec![]), Some(1))).unwrap();
    s.write(at, Origin::cli(), |w| w.agent_check("claude/alpha", Some(1))).unwrap();
    let a = new_card(&mut s, at, "A");
    s.write(at, Origin::cli(), |w| {
        let b = w.resolve(B)?;
        w.exec(&b, &Actor::agent("claude/alpha"), Cmd::Claim { id: Some(a) })
    })
    .unwrap();
    s.write(at + S, Origin::cli(), |w| {
        let b = w.resolve(B)?;
        w.exec(&b, &Actor::agent("claude/alpha"), Cmd::Beat { id: a, done: false })
    })
    .unwrap();
    let b = new_card(&mut s, at, "B");
    let d = s
        .write(at, Origin::ui(None), |w| {
            let br = w.resolve(B)?;
            w.dispatch_create(&br, b, "mbp", None, &Actor::human())
        })
        .unwrap();
    let d = s.write(at + S, Origin::cli(), |w| w.dispatch_begin("mbp", "i1", d.id, Some(1))).unwrap();
    assert_eq!(d.state, "running");
    s.write(at + 2 * S, Origin::cli(), |w| w.host_beat(&report(vec![d.id]), Some(1))).unwrap();
}

fn card(s: &Store, id: i64) -> Card {
    s.read(|r| {
        let b = r.resolve(B)?;
        Ok(r.card(&b, id)?.card)
    })
    .unwrap()
}

#[test]
fn open_after_downtime_keeps_claims_and_running_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
    let old = board_core::now_ms() - 10 * 60 * 1000;
    seed(&db, old);
    // 模拟停机 10 分钟后启动服务：AppState::open 做启动顺延
    let cfg = Config { db: db.clone(), demo: false, serve_web: false, auth: Auth::Local };
    let state = AppState::open(&cfg).unwrap();
    drop(state);
    let s = Store::open(&db).unwrap();
    let now = board_core::now_ms();
    assert_eq!(card(&s, 1).holder.as_deref(), Some("claude/alpha"), "卡片仍由原名字持有");
    assert_eq!(card(&s, 2).holder.as_deref(), Some("claude-mbp/opus-5.5"));
    assert!(s.read(|r| r.agents(now)).unwrap().iter().any(|a| a.name == "claude/alpha"), "名字仍在线");
    assert!(s.read(|r| r.host("mbp", now)).unwrap().online, "机器仍在线");
    assert_eq!(s.read(|r| r.dispatch(1)).unwrap().state, "running", "在跑的派发仍然是 running");
}

fn beats(db: &Path) -> (i64, i64, Option<i64>) {
    let s = Store::open(db).unwrap();
    let a = s.read(|r| r.agent("claude/alpha")).unwrap().unwrap().beat_at;
    let h = s.read(|r| r.host("mbp", 0)).unwrap().beat_at;
    (a, h, card(&s, 1).beat_at)
}

#[test]
fn token_and_backup_do_not_touch_heartbeats() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
    let at = 1_700_000_000_000i64;
    seed(&db, at);
    let before = beats(&db);
    let exe = env!("CARGO_BIN_EXE_board-server");
    // board-server token ls：只读令牌，不改心跳
    let out = Command::new(exe).arg("token").arg("ls").arg("--db").arg(&db).output().unwrap();
    assert!(out.status.success(), "token ls 失败：{}", String::from_utf8_lossy(&out.stderr));
    // board-server backup：在线备份，不改心跳
    let dest = dir.path().join("copy.db");
    let out = Command::new(exe).arg("backup").arg(&dest).arg("--db").arg(&db).output().unwrap();
    assert!(out.status.success(), "backup 失败：{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(beats(&db), before, "token ls / backup 不能改动心跳时间");
}
