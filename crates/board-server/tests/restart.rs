//! 服务端重启：停机时间不计入心跳超时（#95）。

use board_core::*;
use board_server::auth::Auth;
use board_server::{AppState, Config};
use rusqlite::params;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const S: i64 = 1000;
const B: &str = "default/main";

fn report(running: Vec<i64>) -> HostReport {
    HostReport {
        name: "mbp".into(),
        instance: "i1".into(),
        os: "Windows".into(),
        version: "0.1.6".into(),
        agents: vec![HostAgent { id: "claude".into(), name: "Claude Code".into(), version: "2.4.1".into(), found: true, model: "opus-5.5".into(), ..Default::default() }],
        workspace: Some(HostWorkspace { path: "E:\\repo".into(), ..Default::default() }),
        dirs: vec![HostDir { board: B.into(), path: "E:\\repo\\default\\main".into(), ..Default::default() }],
        running,
        config_sync: true,
        ..Default::default()
    }
}

fn new_card(s: &mut Store, at: i64, title: &str) -> i64 {
    s.write(at, Origin::ui(None), |w| {
        let b = w.resolve(B)?;
        w.exec(&b, &Actor::human(), Cmd::New { title: title.into(), body: None, column: None, top: false, labels: vec![], from: None })
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
    s.write(at, Origin::cli(), |w| w.agent_check_owned("claude/alpha", Some(1), NameOwner::Anyone)).unwrap();
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
            w.dispatch_create(&br, b, "mbp", None, None, &Actor::human())
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
    assert_eq!(card(&s, 2).holder.as_deref(), Some("mbp/opus-5.5"));
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

fn enlarge_db(db: &Path) {
    let s = Store::open(db).unwrap();
    let conn = s.conn();
    conn.execute_batch("CREATE TABLE backup_bulk (id INTEGER PRIMARY KEY, body BLOB NOT NULL)").unwrap();
    let chunk = vec![0x5au8; 64 * 1024];
    for id in 0..96 {
        conn.execute("INSERT INTO backup_bulk (id, body) VALUES (?1, ?2)", params![id, &chunk]).unwrap();
    }
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    drop(s);
    let len = std::fs::metadata(db).unwrap().len();
    assert!(len >= 5 * 1024 * 1024, "测试库只有 {len} 字节，需要至少 5 MB");
}

fn read_all(mut reader: impl Read) -> String {
    let mut out = String::new();
    reader.read_to_string(&mut out).unwrap();
    out
}

fn run_with_timeout(mut cmd: Command, timeout: Duration) -> std::process::Output {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || read_all(out));
    let err = std::thread::spawn(move || read_all(err));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let stderr = err.join().unwrap();
            panic!("命令超过 {timeout:?} 没有结束：{stderr}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    std::process::Output { status, stdout: out.join().unwrap().into_bytes(), stderr: err.join().unwrap().into_bytes() }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_listening(child: &mut Child, port: u16) {
    let addr = ([127, 0, 0, 1], port).into();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("服务端提前退出：{status}");
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() {
            return;
        }
        assert!(Instant::now() < until, "服务端 10 秒内没有开始监听 127.0.0.1:{port}");
        std::thread::sleep(Duration::from_millis(20));
    }
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
    let dest = dir.path().join("backup").join("copy.db");
    let out = Command::new(exe).arg("backup").arg(&dest).arg("--db").arg(&db).output().unwrap();
    assert!(out.status.success(), "backup 失败：{}", String::from_utf8_lossy(&out.stderr));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dest.parent().unwrap()), 0o700, "新建的备份目录");
        assert_eq!(mode(&dest), 0o600, "备份数据库");
    }
    assert_eq!(beats(&db), before, "token ls / backup 不能改动心跳时间");
}

#[test]
fn backup_completes_while_server_keeps_writing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("b.db");
    seed(&db, board_core::now_ms());
    enlarge_db(&db);

    let port = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let exe = env!("CARGO_BIN_EXE_board-server");
    let server = Command::new(exe)
        .arg("--local")
        .arg("--db")
        .arg(&db)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .env_remove("BOARD_PUBLIC_URL")
        .env_remove("BOARD_DB")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut server = ChildGuard(server);
    wait_listening(&mut server.0, port);

    let stop = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicUsize::new(0));
    let writer = {
        let db = db.clone();
        let stop = Arc::clone(&stop);
        let writes = Arc::clone(&writes);
        std::thread::spawn(move || {
            let mut store = Store::open(&db).unwrap();
            while !stop.load(Ordering::Relaxed) {
                store
                    .write(board_core::now_ms(), Origin::cli(), |w| w.agent_beat_by("claude/alpha", false, BeatBy::Cli(None)))
                    .unwrap();
                writes.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(50));
            }
        })
    };
    let until = Instant::now() + Duration::from_secs(2);
    while writes.load(Ordering::Relaxed) == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(writes.load(Ordering::Relaxed) > 0, "并发写入没有开始");

    let dest = dir.path().join("backup.db");
    let started = Instant::now();
    let out = run_with_timeout(
        {
            let mut cmd = Command::new(exe);
            cmd.arg("backup").arg(&dest).arg("--db").arg(&db).env_remove("BOARD_DB");
            cmd
        },
        Duration::from_secs(8),
    );
    let elapsed = started.elapsed();
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();

    assert!(out.status.success(), "并发写入时备份失败：{}", String::from_utf8_lossy(&out.stderr));
    assert!(elapsed < Duration::from_secs(8), "备份用了 {elapsed:?}");
    let copy = rusqlite::Connection::open(&dest).unwrap();
    let integrity: String = copy.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
    assert_eq!(integrity, "ok");
    let cards: i64 = copy.query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0)).unwrap();
    assert_eq!(cards, 2);
}

#[test]
fn backup_and_token_refuse_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("missing.db");
    let dest = dir.path().join("copy.db");
    let exe = env!("CARGO_BIN_EXE_board-server");

    let out = Command::new(exe).arg("backup").arg(&dest).arg("--db").arg(&db).env_remove("BOARD_DB").output().unwrap();
    assert!(!out.status.success());
    assert!(!db.exists(), "backup 不能新建源库");
    assert!(!dest.exists(), "源库不存在时不能留下空备份");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--db"));

    let out = Command::new(exe).args(["token", "new", "ci", "--db"]).arg(&db).env_remove("BOARD_DB").output().unwrap();
    assert!(!out.status.success());
    assert!(!db.exists(), "token 不能新建源库");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--db"));
}
