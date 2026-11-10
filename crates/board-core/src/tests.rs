//! 规则层面的验收标准（PRD §13）。CLI 层面的（退出码、--json、环境变量）在 board-cli 的测试里。

use crate::*;

const T0: i64 = 1_700_000_000_000;
const MIN: i64 = 60 * 1000;

fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.ensure_default(T0).unwrap();
    s
}

fn exec(s: &mut Store, path: &str, who: &Actor, at: i64, cmd: Cmd) -> Result<CardDetail> {
    let origin = if who.is_human() { Origin::ui(None) } else { Origin::cli() };
    s.write(at, origin, |w| {
        let b = w.resolve(path)?;
        w.exec(&b, who, cmd)
    })
}

fn new(title: &str) -> Cmd {
    Cmd::New { title: title.into(), body: None, column: None, top: false, labels: vec![] }
}
fn claim(id: Option<i64>) -> Cmd {
    Cmd::Claim { id }
}
fn mv(id: i64, col: &str) -> Cmd {
    Cmd::Move { id, column: col.into(), top: false, index: None, order: None }
}
fn alpha() -> Actor {
    Actor::agent("alpha")
}
fn beta() -> Actor {
    Actor::agent("beta")
}
fn human() -> Actor {
    Actor::human()
}
fn code(r: Result<CardDetail>) -> ErrorCode {
    r.expect_err("应当被拒绝").code
}
fn snapshot(s: &Store, path: &str, now: i64) -> BoardSnapshot {
    s.read(|r| {
        let b = r.resolve(path)?;
        r.board_snapshot(&b, now)
    })
    .unwrap()
}
fn card(s: &Store, path: &str, id: i64) -> CardDetail {
    s.read(|r| {
        let b = r.resolve(path)?;
        r.card(&b, id)
    })
    .unwrap()
}

const B: &str = "default/main";

#[test]
fn ac1_claim_takes_top_of_todo() {
    let mut s = store();
    exec(&mut s, B, &alpha(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, new("二")).unwrap();
    let c = exec(&mut s, B, &alpha(), T0, claim(None)).unwrap().card;
    assert_eq!(c.id, 1);
    assert_eq!(c.column, "doing");
    assert_eq!(c.holder.as_deref(), Some("alpha"));
    assert_eq!(c.claimed_at, Some(T0));
    assert_eq!((c.runs, c.run_started_at), (0, None), "第一次心跳才开始计时");
}

#[test]
fn ac2_concurrent_claim_only_one_wins() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        exec(&mut s, B, &human(), T0, new("x")).unwrap();
    }
    let handles: Vec<_> = ["alpha", "beta", "gamma", "delta"]
        .into_iter()
        .map(|name| {
            let path = path.clone();
            std::thread::spawn(move || {
                let mut s = Store::open(&path).unwrap();
                exec(&mut s, B, &Actor::agent(name), T0, claim(Some(1))).map(|_| ()).map_err(|e| e.code)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "{results:?}");
    assert!(results.iter().filter_map(|r| r.err()).all(|c| c == ErrorCode::NotInTodo), "{results:?}");
}

#[test]
fn ac3_one_card_per_agent() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &human(), T0, new("二")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, claim(None))), ErrorCode::AlreadyHolding);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, claim(Some(2)))), ErrorCode::AlreadyHolding);
}

#[test]
fn ac4_only_holder_edits_others_comment() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    let r = exec(&mut s, B, &beta(), T0, Cmd::Edit { id: 1, title: Some("x".into()), body: None });
    assert_eq!(code(r), ErrorCode::NotHolder);
    exec(&mut s, B, &beta(), T0, Cmd::Comment { id: 1, text: "…".into() }).unwrap();
    // 人也只能评论
    let r = exec(&mut s, B, &human(), T0, Cmd::Edit { id: 1, title: Some("x".into()), body: None });
    assert_eq!(code(r), ErrorCode::NotHolder);
}

#[test]
fn ac5_comment_does_not_keep_claim() {
    // 评论、编辑不再续期：只看心跳
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    exec(&mut s, B, &alpha(), T0 + 40 * SEC, Cmd::Comment { id: 1, text: "进展".into() }).unwrap();
    assert_eq!(s.sweep(T0 + 50 * SEC).unwrap(), 1);
}

#[test]
fn ac6_no_heartbeat_after_claim() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &human(), T0, new("二")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(Some(1))).unwrap();
    // 服务端定时扫描：领取后 50 秒内没有第一次心跳
    assert_eq!(s.sweep(T0 + 49 * SEC).unwrap(), 0);
    assert_eq!(s.sweep(T0 + 50 * SEC).unwrap(), 1);
    let snap = snapshot(&s, B, T0 + 50 * SEC);
    assert_eq!(snap.cols["todo"], vec![1, 2], "回到「待办」最上面");
    let c = card(&s, B, 1);
    assert_eq!(c.card.holder, None);
    assert_eq!(c.card.runs, 0);
    assert!(c.activity.iter().any(|a| a.text.contains("领取后 50 秒内没有开始心跳")));
    exec(&mut s, B, &beta(), T0 + 60 * SEC, claim(Some(1))).unwrap();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0 + 61 * SEC, mv(1, "review"))), ErrorCode::NotHolder);
}

fn beat(id: i64, done: bool) -> Cmd {
    Cmd::Beat { id, done }
}
/// who 在 [from, to] 之间每 10 秒给 id 发一次心跳（最后一次正好在 to）
fn beat_span(s: &mut Store, who: &Actor, id: i64, from: i64, to: i64) {
    let mut t = from;
    loop {
        exec(s, B, who, t, beat(id, false)).unwrap();
        if t >= to {
            break;
        }
        t = (t + BEAT_MS).min(to);
    }
}
const SEC: i64 = 1000;

#[test]
fn heartbeat_keeps_claim_and_starts_run_once() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    let first = T0 + 2 * SEC;
    let mut t = first;
    exec(&mut s, B, &alpha(), t, beat(1, false)).unwrap();
    for _ in 0..200 {
        t += BEAT_MS;
        let c = exec(&mut s, B, &alpha(), t, beat(1, false)).unwrap().card;
        assert_eq!(c.beat_at, Some(t));
        assert_eq!(s.sweep(t + 49 * SEC).unwrap(), 0);
    }
    // 200 次心跳 ≈ 33 分钟，没有被释放；这一次从第一次心跳算起
    let c = card(&s, B, 1);
    assert_eq!(c.card.holder.as_deref(), Some("alpha"));
    assert_eq!((c.card.runs, c.card.run_started_at, c.card.work_ms), (1, Some(first), 0));
    assert_eq!(c.activity.iter().filter(|a| a.action == "beat").count(), 1, "只记开始那一次");
    assert!(c.activity.iter().any(|a| a.text.contains("开始第 1 次")));
}

#[test]
fn heartbeat_lost_after_five_misses() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &human(), T0, new("二")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(Some(2))).unwrap();
    exec(&mut s, B, &alpha(), T0 + SEC, beat(2, false)).unwrap();
    let last = T0 + 10 * MIN + SEC;
    let mut t = T0 + SEC;
    while t < last {
        t += BEAT_MS;
        exec(&mut s, B, &alpha(), t.min(last), beat(2, false)).unwrap();
    }
    assert_eq!(s.sweep(last + BEAT_MS * BEAT_MISSES - 1).unwrap(), 0, "第 5 次还没到");
    assert_eq!(s.sweep(last + BEAT_MS * BEAT_MISSES).unwrap(), 1, "连续 5 次没收到");
    let snap = snapshot(&s, B, last + 50 * SEC);
    assert_eq!(snap.cols["todo"], vec![2, 1], "回到「待办」最上面");
    let c = card(&s, B, 2);
    assert_eq!(c.card.holder, None);
    assert_eq!(c.card.beat_at, None);
    // 这一次以最后一次心跳为结束：10 分钟
    assert_eq!((c.card.runs, c.card.runs_lost, c.card.work_ms, c.card.run_started_at), (1, 1, 10 * MIN, None));
    assert!(c.activity.iter().any(|a| a.action == "lost" && a.text.contains("alpha") && a.text.contains("异常退出") && a.text.contains("第 1 次耗时约 10 分 0 秒")));
    assert_eq!(code(exec(&mut s, B, &alpha(), last + 60 * SEC, beat(2, false))), ErrorCode::NotHolder);
}

#[test]
fn heartbeat_lost_without_server_sweep() {
    // 本机模式没有服务端扫描：下一条命令执行前同样判定
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    exec(&mut s, B, &alpha(), T0, beat(1, false)).unwrap();
    let c = exec(&mut s, B, &beta(), T0 + 50 * SEC, claim(None)).unwrap();
    assert_eq!(c.card.holder.as_deref(), Some("beta"));
}

#[test]
fn heartbeat_final_is_normal_exit() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    beat_span(&mut s, &alpha(), 1, T0 + SEC, T0 + 3 * MIN);
    let end = T0 + 3 * MIN + 5 * SEC;
    let c = exec(&mut s, B, &alpha(), end, beat(1, true)).unwrap();
    assert_eq!(c.card.holder.as_deref(), Some("alpha"), "final 不释放领取");
    assert_eq!((c.card.run_started_at, c.card.work_ms, c.card.runs_lost), (None, 3 * MIN + 4 * SEC, 0));
    assert!(c.activity.iter().any(|a| a.action == "beat" && a.text.contains("正常退出，第 1 次耗时 3 分 4 秒")));
    // 心跳次数：T0+1s 起每 10 秒一次到 3 分（19 次）+ final
    assert_eq!(c.card.beats, 20);
    // final 之后 50 秒内要交审或放回
    assert_eq!(s.sweep(end + 49 * SEC).unwrap(), 0);
    exec(&mut s, B, &alpha(), end + 10 * SEC, mv(1, "review")).unwrap();
    let c = exec(&mut s, B, &alpha(), end + 11 * SEC, beat(1, true)).unwrap();
    assert_eq!(c.card.column, "review", "已交审后再发 final：什么也不做");
    assert_eq!(c.card.runs, 1);
    assert_eq!(c.card.beats, 20, "不是持有者时的 final 不计数");
}

#[test]
fn heartbeat_after_final_does_not_start_new_run() {
    // #48：final 之后、交审之前，还在途的一次普通心跳到了服务端（脚本 stop 时后台循环的 board beat 晚到）
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    beat_span(&mut s, &alpha(), 1, T0 + SEC, T0 + 2 * MIN);
    let end = T0 + 2 * MIN + 5 * SEC;
    exec(&mut s, B, &alpha(), end, beat(1, true)).unwrap();
    let stray = exec(&mut s, B, &alpha(), end + SEC, beat(1, false));
    assert_eq!(code(stray), ErrorCode::Usage, "这次已经 final：普通心跳被拒绝");
    let c = card(&s, B, 1);
    assert_eq!((c.card.runs, c.card.run_started_at, c.card.work_ms), (1, None, 2 * MIN + 4 * SEC), "不新开第 2 次");
    assert!(!c.activity.iter().any(|a| a.text.contains("开始第 2 次")), "没有幽灵记录");
    // T0+1s 起每 10 秒一次到 2 分（13 次）+ final
    assert_eq!(c.card.beats, 14, "幽灵心跳不计数");
    // 不延长 50 秒释放窗口：从 final 算起
    assert_eq!(s.sweep(end + 50 * SEC).unwrap(), 1);
    // 交审时也不会出现「第 2 次 · 0 秒」
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    exec(&mut s, B, &alpha(), T0 + SEC, beat(1, false)).unwrap();
    exec(&mut s, B, &alpha(), T0 + 9 * SEC, beat(1, true)).unwrap();
    let _ = exec(&mut s, B, &alpha(), T0 + 10 * SEC, beat(1, false));
    let c = exec(&mut s, B, &alpha(), T0 + 11 * SEC, mv(1, "review")).unwrap();
    assert!(c.activity.iter().any(|a| a.action == "move" && a.text.ends_with("领取已释放")), "{:?}", c.activity.last().map(|a| &a.text));
    assert_eq!(c.card.runs, 1);
    // 重新领取后，下一次心跳正常开始第 2 次
    exec(&mut s, B, &human(), T0 + MIN, mv(1, "rework")).unwrap();
    exec(&mut s, B, &alpha(), T0 + 2 * MIN, claim(None)).unwrap();
    let c = exec(&mut s, B, &alpha(), T0 + 2 * MIN + SEC, beat(1, false)).unwrap();
    assert_eq!((c.card.runs, c.card.run_started_at), (2, Some(T0 + 2 * MIN + SEC)));
    assert!(c.activity.iter().any(|a| a.text.contains("开始第 2 次")));
}

#[test]
fn heartbeat_final_then_idle_is_released() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    exec(&mut s, B, &alpha(), T0 + SEC, beat(1, false)).unwrap();
    exec(&mut s, B, &alpha(), T0 + 9 * SEC, beat(1, true)).unwrap();
    assert_eq!(s.sweep(T0 + 59 * SEC).unwrap(), 1);
    let c = card(&s, B, 1);
    assert_eq!((c.card.holder, c.card.runs_lost), (None, 0), "已经正常结束，不算异常退出");
    assert!(c.activity.iter().any(|a| a.text.contains("结束心跳后 50 秒内没有交审或放回")));
}

#[test]
fn runs_accumulate_across_claims() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    // 第 1 次：alpha 做了 2 分钟，交审时结束（没先发 final）
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    beat_span(&mut s, &alpha(), 1, T0 + SEC, T0 + 2 * MIN);
    let c = exec(&mut s, B, &alpha(), T0 + 2 * MIN + SEC, mv(1, "review")).unwrap();
    assert!(c.activity.iter().any(|a| a.action == "move" && a.text.contains("领取已释放，第 1 次耗时 2 分 0 秒")));
    // 打回，第 2 次：beta 做了 30 秒后被人强制释放
    exec(&mut s, B, &human(), T0 + 3 * MIN, mv(1, "todo")).unwrap();
    exec(&mut s, B, &beta(), T0 + 4 * MIN, claim(None)).unwrap();
    exec(&mut s, B, &beta(), T0 + 4 * MIN, beat(1, false)).unwrap();
    exec(&mut s, B, &beta(), T0 + 4 * MIN + 20 * SEC, beat(1, false)).unwrap();
    let c = exec(&mut s, B, &human(), T0 + 4 * MIN + 30 * SEC, Cmd::Release { id: 1 }).unwrap();
    assert!(c.activity.iter().any(|a| a.action == "release" && a.text.contains("第 2 次耗时 30 秒")));
    // 第 3 次：alpha 进行中
    exec(&mut s, B, &alpha(), T0 + 5 * MIN, claim(None)).unwrap();
    exec(&mut s, B, &alpha(), T0 + 5 * MIN + 2 * SEC, beat(1, false)).unwrap();
    let c = card(&s, B, 1).card;
    assert_eq!((c.runs, c.runs_lost, c.work_ms, c.run_started_at), (3, 0, 2 * MIN + 30 * SEC, Some(T0 + 5 * MIN + 2 * SEC)));
}

#[test]
fn heartbeat_rules() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, beat(1, false))), ErrorCode::NotHolder, "没领取不能发心跳");
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    assert_eq!(code(exec(&mut s, B, &beta(), T0, beat(1, false))), ErrorCode::NotHolder, "别人的卡片");
    assert_eq!(code(exec(&mut s, B, &human(), T0, beat(1, false))), ErrorCode::Usage, "人不发心跳");
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, beat(9, false))), ErrorCode::NotFound);
}

#[test]
fn ac6_timeout_without_server_sweep() {
    // 没有服务端时，任何一条命令执行前也会先释放到期的领取
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    let c = exec(&mut s, B, &beta(), T0 + 31 * MIN, claim(None)).unwrap();
    assert_eq!(c.card.id, 1);
    assert_eq!(c.card.holder.as_deref(), Some("beta"));
}

#[test]
fn ac7_done_belongs_to_humans() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, mv(1, "done"))), ErrorCode::ForbiddenColumn);
    let c = exec(&mut s, B, &alpha(), T0, mv(1, "review")).unwrap();
    assert_eq!(c.card.holder, None);
    assert_eq!(c.card.beat_at, None);
    let c = exec(&mut s, B, &human(), T0, mv(1, "done")).unwrap();
    assert_eq!(c.card.column, "done");
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, mv(1, "todo"))), ErrorCode::ForbiddenColumn);
}

#[test]
fn ac8_doing_only_by_claim() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &human(), T0, new("二")).unwrap();
    assert_eq!(code(exec(&mut s, B, &human(), T0, mv(1, "doing"))), ErrorCode::ForbiddenColumn);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, mv(2, "doing"))), ErrorCode::ForbiddenColumn);
    let snap = snapshot(&s, B, T0);
    assert_eq!(snap.cols["todo"], vec![1, 2]);
}

#[test]
fn ac9_force_release() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &human(), T0, new("二")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(Some(2))).unwrap();
    let c = exec(&mut s, B, &human(), T0, Cmd::Release { id: 2 }).unwrap();
    assert_eq!(c.card.holder, None);
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![2, 1]);
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    // 人不能把「进行中」的卡片拖走
    assert_eq!(code(exec(&mut s, B, &human(), T0, mv(2, "review"))), ErrorCode::NotHolder);
}

#[test]
fn ac10_rev_increments() {
    let mut s = store();
    let r0 = s.rev().unwrap();
    exec(&mut s, B, &alpha(), T0, new("x")).unwrap();
    assert!(s.rev().unwrap() > r0);
    // 被拒绝的命令不改变 rev
    let r1 = s.rev().unwrap();
    let _ = exec(&mut s, B, &alpha(), T0, mv(1, "doing"));
    assert_eq!(s.rev().unwrap(), r1);
}

fn oss(s: &mut Store) {
    s.write(T0, Origin::ui(None), |w| {
        w.create_project(
            "开源",
            "oss",
            "",
            None,
            None,
            NewBoard { name: "主白板".into(), key: "main".into(), desc: String::new(), pos: None },
        )?;
        w.create_board("oss", "发布", "release", "", None)?;
        w.create_project(
            "官网",
            "site",
            "",
            None,
            None,
            NewBoard { name: "上线".into(), key: "launch".into(), desc: String::new(), pos: None },
        )?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn ac13_board_paths() {
    let mut s = store();
    oss(&mut s);
    let e = s.read(|r| r.resolve("oss")).unwrap_err();
    assert_eq!(e.code, ErrorCode::BoardNotFound);
    assert!(e.message.contains("main") && e.message.contains("release"), "{}", e.message);
    assert_eq!(s.read(|r| r.resolve("oss/nope")).unwrap_err().code, ErrorCode::BoardNotFound);
    assert_eq!(s.read(|r| r.resolve("site")).unwrap(), s.read(|r| r.resolve("site/launch")).unwrap());
    assert_eq!(s.read(|r| r.resolve("")).unwrap_err().code, ErrorCode::NoBoard);
}

#[test]
fn ac14_ids_per_board() {
    let mut s = store();
    oss(&mut s);
    exec(&mut s, "oss/main", &alpha(), T0, new("a")).unwrap();
    exec(&mut s, "oss/main", &alpha(), T0, new("b")).unwrap();
    assert_eq!(exec(&mut s, "oss/release", &alpha(), T0, new("x")).unwrap().card.id, 1);
}

#[test]
fn ac15_holding_is_per_board() {
    let mut s = store();
    oss(&mut s);
    exec(&mut s, "oss/main", &human(), T0, new("a")).unwrap();
    exec(&mut s, "oss/release", &human(), T0, new("b")).unwrap();
    exec(&mut s, "oss/main", &alpha(), T0, claim(None)).unwrap();
    exec(&mut s, "oss/release", &alpha(), T0, claim(None)).unwrap();
}

#[test]
fn ac17_deleted_board() {
    let mut s = store();
    oss(&mut s);
    s.write(T0, Origin::ui(None), |w| w.delete_board("oss/release")).unwrap();
    assert_eq!(s.read(|r| r.resolve("oss/release")).unwrap_err().code, ErrorCode::BoardNotFound);
    // 只剩一块白板时只写项目标识也可以
    assert!(s.read(|r| r.resolve("oss")).is_ok());
}

#[test]
fn keep_at_least_one_board() {
    let mut s = store();
    assert_eq!(s.write(T0, Origin::ui(None), |w| w.delete_board(B)).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(s.write(T0, Origin::ui(None), |w| w.delete_project("default")).unwrap_err().code, ErrorCode::Usage);
    oss(&mut s);
    s.write(T0, Origin::ui(None), |w| w.delete_project("oss")).unwrap();
    assert!(s.read(|r| r.resolve("oss/main")).is_err());
}

#[test]
fn ac19_21_inbox() {
    let mut s = store();
    for _ in 0..4 {
        exec(&mut s, B, &human(), T0, new("x")).unwrap();
    }
    for id in 1..=4 {
        exec(&mut s, B, &human(), T0, Cmd::Delete { id }).unwrap();
    }
    let c = exec(&mut s, B, &human(), T0, Cmd::New { title: "想法".into(), body: None, column: Some("inbox".into()), top: false, labels: vec![] }).unwrap();
    assert_eq!(c.card.id, 5);
    assert_eq!(c.card.column, "inbox");
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, claim(None))), ErrorCode::EmptyTodo);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, claim(Some(5)))), ErrorCode::NotInTodo);
    // 拖到「进行中」被拒绝
    assert_eq!(code(exec(&mut s, B, &human(), T0, mv(5, "doing"))), ErrorCode::ForbiddenColumn);
    // 整理到待办 → 最下面
    exec(&mut s, B, &human(), T0, new("y")).unwrap();
    exec(&mut s, B, &human(), T0, mv(5, "todo")).unwrap();
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![6, 5]);
    // agent/CLI 也可以在「未整理」里新建，放在最下面，持有者为空
    let a = exec(&mut s, B, &alpha(), T0, Cmd::New { title: "agent 记的".into(), body: None, column: Some("inbox".into()), top: false, labels: vec![] }).unwrap();
    assert_eq!(a.card.column, "inbox");
    assert_eq!(a.card.holder, None);
    let b = exec(&mut s, B, &alpha(), T0, Cmd::New { title: "agent 又记的".into(), body: None, column: Some("inbox".into()), top: false, labels: vec![] }).unwrap();
    assert_eq!(b.card.column, "inbox");
    assert_eq!(snapshot(&s, B, T0).cols["inbox"], vec![7, 8]);
}

#[test]
fn ac22_25_labels() {
    let mut s = store();
    let bug = s.write(T0, Origin::ui(None), |w| w.create_label("default", "bug", None, false)).unwrap();
    assert_eq!(
        s.write(T0, Origin::ui(None), |w| w.create_label("default", "BUG", None, false)).unwrap_err().code,
        ErrorCode::Usage,
        "名称不区分大小写"
    );
    exec(&mut s, B, &human(), T0, new("a")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    let c = exec(&mut s, B, &beta(), T0 + 5 * SEC, Cmd::Label { id: 1, add: vec![bug.id], remove: vec![] }).unwrap();
    assert_eq!(c.card.labels, vec![bug.id]);
    assert_eq!(c.card.holder.as_deref(), Some("alpha"), "别人持有的卡片也能加标签");
    assert!(c.activity.iter().any(|a| a.action == "label" && a.text.contains("bug")));
    // 不存在的标签
    assert_eq!(code(exec(&mut s, B, &beta(), T0, Cmd::Label { id: 1, add: vec![999], remove: vec![] })), ErrorCode::Usage);
    // 删除标签 → 从卡片上移除
    s.write(T0, Origin::ui(None), |w| w.delete_label("default", bug.id)).unwrap();
    assert!(card(&s, B, 1).card.labels.is_empty());
}

/// 仅人可打的标签：agent 加、去、新建时带上都被拒绝（USAGE）；人不受限，可以改标签类型
#[test]
fn human_only_labels() {
    let mut s = store();
    let mine = ui(&mut s, |w| w.create_label("default", "人定", None, true)).unwrap();
    assert!(mine.human_only);
    let any = ui(&mut s, |w| w.create_label("default", "通用", None, false)).unwrap();
    exec(&mut s, B, &human(), T0, new("a")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();

    // agent 加：拒绝，原因里写明标签名
    let e = exec(&mut s, B, &beta(), T0, Cmd::Label { id: 1, add: vec![mine.id], remove: vec![] }).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("人定") && e.message.contains("只有人能打"), "{}", e.message);
    // agent 可打的标签照常
    exec(&mut s, B, &alpha(), T0, Cmd::Label { id: 1, add: vec![any.id], remove: vec![] }).unwrap();
    // agent 新建时带上：拒绝
    let e = exec(&mut s, B, &beta(), T0, Cmd::New { title: "b".into(), body: None, column: None, top: false, labels: vec![mine.id] })
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);

    // 人：任意状态、任意持有者的卡片都能加、去
    let c = exec(&mut s, B, &human(), T0, Cmd::Label { id: 1, add: vec![mine.id], remove: vec![] }).unwrap();
    assert_eq!(c.card.holder.as_deref(), Some("alpha"));
    assert!(c.card.labels.contains(&mine.id));
    // agent 去：同样拒绝，卡片上的标签不变
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, Cmd::Label { id: 1, add: vec![], remove: vec![mine.id] })), ErrorCode::Usage);
    assert!(card(&s, B, 1).card.labels.contains(&mine.id));
    exec(&mut s, B, &human(), T0, Cmd::Label { id: 1, add: vec![], remove: vec![mine.id] }).unwrap();
    assert!(!card(&s, B, 1).card.labels.contains(&mine.id));

    // 改类型：改成 agent 可打后 agent 就能打
    ui(&mut s, |w| w.update_label("default", mine.id, None, None, Some(false))).unwrap();
    let ls = s.read(|r| r.labels("default")).unwrap();
    assert!(!ls.iter().find(|l| l.id == mine.id).unwrap().human_only);
    exec(&mut s, B, &alpha(), T0, Cmd::Label { id: 1, add: vec![mine.id], remove: vec![] }).unwrap();
}

/// 旧数据库的标签没有类型列：迁移后都是 agent 可打
#[test]
fn old_labels_default_to_agent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        ui(&mut s, |w| w.create_label("default", "旧", None, true)).unwrap();
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("ALTER TABLE labels DROP COLUMN human_only").unwrap();
    }
    let s = Store::open(&path).unwrap();
    let ls = s.read(|r| r.labels("default")).unwrap();
    assert_eq!(ls.len(), 1);
    assert!(!ls[0].human_only);
}

#[test]
fn drag_index_and_reorder() {
    let mut s = store();
    for t in ["a", "b", "c"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    exec(&mut s, B, &human(), T0, Cmd::Move { id: 3, column: "todo".into(), top: false, index: Some(0), order: None }).unwrap();
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![3, 1, 2]);
    exec(&mut s, B, &human(), T0, Cmd::Move { id: 3, column: "review".into(), top: false, index: Some(0), order: None }).unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Move { id: 1, column: "review".into(), top: true, index: None, order: None }).unwrap();
    let snap = snapshot(&s, B, T0);
    assert_eq!(snap.cols["todo"], vec![2]);
    assert_eq!(snap.cols["review"], vec![1, 3]);
    exec(&mut s, B, &alpha(), T0, Cmd::New { title: "top".into(), body: None, column: None, top: true, labels: vec![] }).unwrap();
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![4, 2]);
}

#[test]
fn drag_with_order_saves_whole_column() {
    let mut s = store();
    for t in ["a", "b", "c", "d"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    exec(&mut s, B, &human(), T0, mv(4, "review")).unwrap();
    // 界面按编号降序显示 [3, 2, 1]，把 #1 拖到最前：整块按看到的顺序保存
    let mv_order = |id: i64, index: usize, order: Vec<i64>| Cmd::Move { id, column: "todo".into(), top: false, index: Some(index), order: Some(order) };
    exec(&mut s, B, &human(), T0, mv_order(1, 0, vec![1, 3, 2])).unwrap();
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![1, 3, 2]);
    let d = card(&s, B, 1);
    assert!(d.activity.iter().any(|a| a.action == "reorder" && a.text.contains("整块的优先级")));
    // 不在这个状态的 id 忽略，没列出的卡片按原顺序补在后面；别的状态不受影响
    exec(&mut s, B, &human(), T0, mv_order(2, 0, vec![2, 4, 99])).unwrap();
    let snap = snapshot(&s, B, T0);
    assert_eq!(snap.cols["todo"], vec![2, 1, 3]);
    assert_eq!(snap.cols["review"], vec![4]);
}

#[test]
fn empty_title_and_comment() {
    let mut s = store();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, new("  "))), ErrorCode::EmptyTitle);
    exec(&mut s, B, &alpha(), T0, new("a")).unwrap();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, Cmd::Edit { id: 1, title: Some("".into()), body: None })), ErrorCode::EmptyTitle);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, Cmd::Comment { id: 1, text: " ".into() })), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, Cmd::Comment { id: 9, text: "x".into() })), ErrorCode::NotFound);
}

#[test]
fn edit_own_comment() {
    let mut s = store();
    exec(&mut s, B, &alpha(), T0, new("a")).unwrap();
    let d = exec(&mut s, B, &human(), T0, Cmd::Comment { id: 1, text: "打错字了".into() }).unwrap();
    let mine = d.activity.last().unwrap().id;
    let d = exec(&mut s, B, &alpha(), T0, Cmd::Comment { id: 1, text: "agent 的评论".into() }).unwrap();
    let theirs = d.activity.last().unwrap().id;
    let sys = d.activity[0].id;
    let n = d.activity.len();
    let edit = |activity: i64, text: &str| Cmd::EditComment { id: 1, activity, text: text.into() };

    // 人改自己的评论：原位更新、记下修改时间，不新增动态
    let d = exec(&mut s, B, &human(), T0 + MIN, edit(mine, "  改好了  ")).unwrap();
    assert_eq!(d.activity.len(), n);
    let a = d.activity.iter().find(|a| a.id == mine).unwrap();
    assert_eq!((a.text.as_str(), a.edited_at), ("改好了", Some(T0 + MIN)));
    assert_eq!(card(&s, B, 1).activity.iter().find(|a| a.id == mine).unwrap().text, "改好了");
    assert_eq!(snapshot(&s, B, T0 + MIN).cards[&1].comments, 2);

    // 别人的评论、agent 改人的评论：NOT_AUTHOR；系统动态、空内容：USAGE；不存在：NOT_FOUND
    assert_eq!(code(exec(&mut s, B, &human(), T0, edit(theirs, "x"))), ErrorCode::NotAuthor);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, edit(mine, "x"))), ErrorCode::NotAuthor);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, edit(theirs, "x"))), ErrorCode::NotAuthor);
    assert_eq!(code(exec(&mut s, B, &human(), T0, edit(sys, "x"))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, edit(mine, "  "))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, edit(9999, "x"))), ErrorCode::NotFound);
    assert_eq!(card(&s, B, 1).activity.iter().find(|a| a.id == theirs).unwrap().edited_at, None);
}

#[test]
fn demo_seed() {
    let mut s = store();
    assert!(seed::seed_demo(&mut s, T0).unwrap());
    let ws = s.read(|r| r.workspace(T0)).unwrap();
    assert_eq!(ws.projects.len(), 3);
    assert_eq!(ws.projects.iter().map(|p| p.boards.len()).sum::<usize>(), 5);
    let main = &ws.projects[0].boards[0];
    assert_eq!(main.path, "oss/main");
    assert_eq!(main.counts["inbox"], 4);
    assert_eq!(main.counts["doing"], 2);
    assert!(!seed::seed_demo(&mut s, T0).unwrap(), "已有数据时不再写入");
}

/* ------------------------------ 工作流（PRD §7、§13 26–29） ------------------------------ */

fn ui<T>(s: &mut Store, f: impl FnOnce(&mut W) -> Result<T>) -> Result<T> {
    s.write(T0, Origin::ui(None), f)
}
fn flow(s: &Store) -> Flow {
    s.read(|r| {
        let b = r.resolve(B)?;
        r.flow(&b)
    })
    .unwrap()
}
fn keys(s: &Store) -> Vec<String> {
    flow(s).0.into_iter().map(|x| x.key).collect()
}

fn outs(s: &Store, k: &str) -> Vec<String> {
    flow(s).get(k).unwrap().to.clone()
}
fn edge(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}
fn err(r: Result<CardDetail>) -> BoardError {
    r.expect_err("应当被拒绝")
}

#[test]
fn default_flow_and_custom_status() {
    let mut s = store();
    assert_eq!(keys(&s), ["todo", "doing", "review", "done", "rework", "check"]);
    assert_eq!(outs(&s, "todo"), ["doing"]);
    assert_eq!(outs(&s, "doing"), ["review", "check"]);
    assert_eq!(outs(&s, "review"), ["done", "rework"]);
    assert_eq!(outs(&s, "rework"), ["doing"]);
    assert_eq!(outs(&s, "check"), ["todo"]);
    assert!(flow(&s).get("rework").unwrap().claim);
    let st = ui(&mut s, |w| w.create_status(B, "设计中", None)).unwrap();
    assert_eq!(st.key, "s1");
    // 重名（不区分大小写）、保留名称
    assert_eq!(ui(&mut s, |w| w.create_status(B, "设计中", None)).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(ui(&mut s, |w| w.create_status(B, "未整理", None)).unwrap_err().code, ErrorCode::Usage);
    let order: Vec<String> = ["todo", "s1", "doing", "review", "done", "rework", "check"].map(String::from).to_vec();
    ui(&mut s, |w| w.reorder_statuses(B, &order)).unwrap();
    assert_eq!(keys(&s), order);
    // §13 26：连线「待办 → 设计中」后，agent 才能这样移动
    ui(&mut s, |w| w.update_edges(B, &[edge("todo", "s1")], &[])).unwrap();
    assert_eq!(outs(&s, "todo"), ["s1", "doing"]);
    for t in ["a", "b", "c"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    // 标识和名称都能用
    exec(&mut s, B, &alpha(), T0, mv(2, "设计中")).unwrap();
    exec(&mut s, B, &alpha(), T0, mv(1, "s1")).unwrap();
    assert_eq!(snapshot(&s, B, T0).cols["s1"], vec![2, 1]);
    let e = err(exec(&mut s, B, &alpha(), T0, mv(3, "review")));
    assert_eq!(e.code, ErrorCode::ForbiddenColumn);
    assert!(e.message.contains("「设计中」「进行中」"), "{}", e.message);
    // 人不受连线限制
    exec(&mut s, B, &human(), T0, mv(3, "review")).unwrap();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, mv(1, "不存在"))), ErrorCode::Usage);
    // 连线：不能连到自己、不能连到不存在的状态；去掉连线
    assert_eq!(ui(&mut s, |w| w.update_edges(B, &[edge("todo", "todo")], &[])).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(ui(&mut s, |w| w.update_edges(B, &[edge("todo", "nope")], &[])).unwrap_err().code, ErrorCode::Usage);
    ui(&mut s, |w| w.update_edges(B, &[], &[edge("todo", "s1")])).unwrap();
    assert_eq!(outs(&s, "todo"), ["doing"]);
}

#[test]
fn claim_order_follows_flow() {
    let mut s = store();
    // §13 27：「待办」为空时领「返工」
    exec(&mut s, B, &human(), T0, new("a")).unwrap();
    exec(&mut s, B, &human(), T0, mv(1, "rework")).unwrap();
    let c = exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    assert_eq!((c.card.id, c.card.column.as_str()), (1, "doing"));
    // 都有卡片时按工作流顺序：先「待办」
    exec(&mut s, B, &human(), T0, new("b")).unwrap();
    exec(&mut s, B, &human(), T0, new("c")).unwrap();
    exec(&mut s, B, &human(), T0, mv(3, "rework")).unwrap();
    assert_eq!(exec(&mut s, B, &beta(), T0, claim(None)).unwrap().card.id, 2);
    // 把「返工」排到「待办」之前：先领「返工」
    exec(&mut s, B, &human(), T0, new("d")).unwrap();
    let order: Vec<String> = ["rework", "todo", "doing", "review", "done", "check"].map(String::from).to_vec();
    ui(&mut s, |w| w.reorder_statuses(B, &order)).unwrap();
    assert_eq!(exec(&mut s, B, &Actor::agent("gamma"), T0, claim(None)).unwrap().card.id, 3);
    // 首个可领取状态变成「返工」：新建放进这里
    assert_eq!(exec(&mut s, B, &human(), T0, new("e")).unwrap().card.column, "rework");
    // 可领取可以有多个，但至少一个
    ui(&mut s, |w| w.update_status(B, "rework", StatusPatch { claim: Some(false), ..Default::default() })).unwrap();
    assert_eq!(
        ui(&mut s, |w| w.update_status(B, "todo", StatusPatch { claim: Some(false), ..Default::default() })).unwrap_err().code,
        ErrorCode::Usage
    );
    // 「返工」不再可领取：带 ID 领取里面的卡片被拒绝
    assert_eq!(code(exec(&mut s, B, &Actor::agent("delta"), T0, claim(Some(5)))), ErrorCode::NotInTodo);
}

#[test]
fn agent_moves_follow_edges() {
    let mut s = store();
    for t in ["a", "b", "c"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    exec(&mut s, B, &alpha(), T0, claim(Some(1))).unwrap();
    // §13 31：做不下去不能丢回「待办」，要沿连线去「需人工校验」
    let e = err(exec(&mut s, B, &alpha(), T0, mv(1, "todo")));
    assert_eq!(e.code, ErrorCode::ForbiddenColumn);
    assert!(e.message.contains("「待审核」「需人工校验」"), "{}", e.message);
    let c = exec(&mut s, B, &alpha(), T0, mv(1, "需人工校验")).unwrap();
    assert!(c.card.holder.is_none());
    // 人不受连线限制
    exec(&mut s, B, &human(), T0, mv(1, "review")).unwrap();
    // 同一状态内调整顺序不受连线影响
    exec(&mut s, B, &alpha(), T0, Cmd::Move { id: 3, column: "todo".into(), top: true, index: None, order: None }).unwrap();
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![3, 2]);
    // §13 34：「未整理」只归人管
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, mv(2, "inbox"))), ErrorCode::ForbiddenColumn);
    exec(&mut s, B, &human(), T0, mv(2, "inbox")).unwrap();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, mv(2, "todo"))), ErrorCode::ForbiddenColumn);
}

#[test]
fn release_returns_to_claimed_status() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("a")).unwrap();
    exec(&mut s, B, &human(), T0, new("b")).unwrap();
    exec(&mut s, B, &human(), T0, mv(1, "rework")).unwrap();
    // §13 32：超时回到「返工」最上面，不回「待办」
    exec(&mut s, B, &alpha(), T0, claim(Some(1))).unwrap();
    s.sweep(T0 + 31 * MIN).unwrap();
    let snap = snapshot(&s, B, T0 + 31 * MIN);
    assert_eq!((snap.cols["rework"].clone(), snap.cols["todo"].clone()), (vec![1], vec![2]));
    // 心跳中断同样
    exec(&mut s, B, &beta(), T0 + 32 * MIN, claim(Some(1))).unwrap();
    exec(&mut s, B, &beta(), T0 + 32 * MIN, Cmd::Beat { id: 1, done: false }).unwrap();
    s.sweep(T0 + 34 * MIN).unwrap();
    assert_eq!(snapshot(&s, B, T0 + 34 * MIN).cols["rework"], vec![1]);
    // 强制释放同样
    exec(&mut s, B, &alpha(), T0 + 35 * MIN, claim(Some(1))).unwrap();
    exec(&mut s, B, &human(), T0 + 35 * MIN, Cmd::Release { id: 1 }).unwrap();
    assert_eq!(snapshot(&s, B, T0 + 35 * MIN).cols["rework"], vec![1]);
    // 来源状态不再可领取时，回首个可领取状态
    exec(&mut s, B, &alpha(), T0 + 36 * MIN, claim(Some(1))).unwrap();
    ui(&mut s, |w| w.update_status(B, "rework", StatusPatch { claim: Some(false), ..Default::default() })).unwrap();
    exec(&mut s, B, &human(), T0 + 36 * MIN, Cmd::Release { id: 1 }).unwrap();
    assert_eq!(snapshot(&s, B, T0 + 36 * MIN).cols["todo"], vec![1, 2]);
}

#[test]
fn claim_needs_edge_to_locked_status() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("a")).unwrap();
    exec(&mut s, B, &human(), T0, mv(1, "rework")).unwrap();
    // §13 33：去掉「返工 → 进行中」后不能从「返工」领取，不带 ID 时跳过它
    ui(&mut s, |w| w.update_edges(B, &[], &[edge("rework", "doing")])).unwrap();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, claim(Some(1)))), ErrorCode::ForbiddenColumn);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, claim(None))), ErrorCode::EmptyTodo);
    ui(&mut s, |w| w.update_edges(B, &[edge("rework", "doing")], &[])).unwrap();
    assert_eq!(exec(&mut s, B, &alpha(), T0, claim(None)).unwrap().card.id, 1);
}

#[test]
fn multiple_locked_statuses() {
    let mut s = store();
    let t = ui(&mut s, |w| w.create_status(B, "测试中", None)).unwrap();
    ui(&mut s, |w| w.update_status(B, &t.key, StatusPatch { lock: Some(true), ..Default::default() })).unwrap();
    ui(&mut s, |w| w.update_edges(B, &[edge("doing", &t.key)], &[])).unwrap();
    for x in ["a", "b"] {
        exec(&mut s, B, &human(), T0, new(x)).unwrap();
    }
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    // 持有者沿连线在锁定状态之间移动：领取保持，这一次继续计时
    exec(&mut s, B, &alpha(), T0 + SEC, beat(1, false)).unwrap();
    let c = exec(&mut s, B, &alpha(), T0 + 10 * SEC, mv(1, "测试中")).unwrap();
    assert_eq!(c.card.holder.as_deref(), Some("alpha"));
    assert_eq!((c.card.runs, c.card.run_started_at), (1, Some(T0 + SEC)));
    // 别人不能把卡片直接移进锁定状态
    assert_eq!(code(exec(&mut s, B, &human(), T0, mv(2, "测试中"))), ErrorCode::ForbiddenColumn);
    assert_eq!(code(exec(&mut s, B, &beta(), T0, mv(1, "review"))), ErrorCode::NotHolder);
    // 「测试中」还没有连到「待审核」
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, mv(1, "review"))), ErrorCode::ForbiddenColumn);
    ui(&mut s, |w| w.update_edges(B, &[edge(&t.key, "review")], &[])).unwrap();
    // 离开锁定状态即释放
    let c = exec(&mut s, B, &alpha(), T0, mv(1, "review")).unwrap();
    assert!(c.card.holder.is_none());
}

#[test]
fn unlock_and_delete_status_release_cards() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("a")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    // 关闭「进行中」的锁定：卡片留在原处，领取释放
    ui(&mut s, |w| w.update_status(B, "doing", StatusPatch { lock: Some(false), ..Default::default() })).unwrap();
    let c = card(&s, B, 1);
    assert_eq!((c.card.column.as_str(), c.card.holder.is_none()), ("doing", true));
    // 可领取状态可以删除，但最后一个不能
    ui(&mut s, |w| w.delete_status(B, "rework")).unwrap();
    assert!(!outs(&s, "review").contains(&"rework".to_string()), "相关连线一起删除");
    assert_eq!(ui(&mut s, |w| w.delete_status(B, "todo")).unwrap_err().code, ErrorCode::Usage);
    // 删除其他状态时卡片移到首个可领取状态最下面
    exec(&mut s, B, &human(), T0, new("b")).unwrap();
    ui(&mut s, |w| w.delete_status(B, "doing")).unwrap();
    assert_eq!(keys(&s), ["todo", "review", "done", "check"]);
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![2, 1]);
    assert!(outs(&s, "todo").is_empty());
    // 没有连到锁定状态时不能领取
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, claim(None))), ErrorCode::EmptyTodo);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, claim(Some(1)))), ErrorCode::ForbiddenColumn);
}

#[test]
fn card_view_defaults_by_final_and_can_be_set() {
    let mut s = store();
    let view = |s: &Store, k: &str| flow(s).get(k).unwrap().view;
    // 没设置过：终点用磁贴，其他用卡片
    assert_eq!(view(&s, "done"), CardView::Tile);
    assert_eq!(view(&s, "todo"), CardView::Card);
    // 设置后按设置的来
    ui(&mut s, |w| w.update_status(B, "done", StatusPatch { view: Some(CardView::Line), ..Default::default() })).unwrap();
    assert_eq!(view(&s, "done"), CardView::Line);
    // 没设置过的状态改成终点后跟着变成磁贴；设置过的不变
    let t = ui(&mut s, |w| w.create_status(B, "已发布", None)).unwrap();
    assert_eq!(t.view, CardView::Card);
    ui(&mut s, |w| w.update_status(B, &t.key, StatusPatch { fin: Some(true), ..Default::default() })).unwrap();
    assert_eq!(view(&s, &t.key), CardView::Tile);
    ui(&mut s, |w| w.update_status(B, "review", StatusPatch { view: Some(CardView::Card), ..Default::default() })).unwrap();
    ui(&mut s, |w| w.update_status(B, "review", StatusPatch { fin: Some(true), ..Default::default() })).unwrap();
    assert_eq!(view(&s, "review"), CardView::Card);
    // 界面传来的 JSON
    let p: StatusPatch = serde_json::from_value(serde_json::json!({ "view": "tile" })).unwrap();
    assert_eq!(p.view, Some(CardView::Tile));
    assert!(serde_json::from_value::<StatusPatch>(serde_json::json!({ "view": "grid" })).is_err());
    assert_eq!(serde_json::to_value(flow(&s).get("todo").unwrap()).unwrap()["view"], "card");
}

#[test]
fn old_database_gets_card_view_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("ALTER TABLE statuses DROP COLUMN view").unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    assert_eq!(flow(&s).get("done").unwrap().view, CardView::Tile);
    assert_eq!(flow(&s).get("doing").unwrap().view, CardView::Card);
    ui(&mut s, |w| w.update_status(B, "doing", StatusPatch { view: Some(CardView::Tile), ..Default::default() })).unwrap();
    assert_eq!(flow(&s).get("doing").unwrap().view, CardView::Tile);
}

#[test]
fn old_database_gets_chain_edges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
    }
    // 模拟升级前的数据库：只有 4 个状态，没有连线
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "DELETE FROM transitions; DELETE FROM meta WHERE key = 'edges';
             DELETE FROM statuses WHERE key IN ('rework', 'check');",
        )
        .unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(outs(&s, "todo"), ["doing"]);
    assert_eq!(outs(&s, "doing"), ["review"]);
    assert_eq!(outs(&s, "review"), ["done"]);
    assert!(outs(&s, "done").is_empty());
}

#[test]
fn final_status_is_human_only_and_layout_saved() {
    let mut s = store();
    let t = ui(&mut s, |w| w.create_status(B, "已发布", Some("#EC4899"))).unwrap();
    ui(&mut s, |w| w.update_status(B, &t.key, StatusPatch { fin: Some(true), ..Default::default() })).unwrap();
    exec(&mut s, B, &human(), T0, new("a")).unwrap();
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, mv(1, "已发布"))), ErrorCode::ForbiddenColumn);
    exec(&mut s, B, &human(), T0, mv(1, "已发布")).unwrap();
    // 布局
    let mut tiles = std::collections::BTreeMap::new();
    tiles.insert("todo".to_string(), Tile { x: 0, w: 6, y: 0.0 });
    ui(&mut s, |w| w.set_flow_layout(B, &tiles)).unwrap();
    assert_eq!(flow(&s).get("todo").unwrap().layout, Some(Tile { x: 0, w: 6, y: 0.0 }));
    tiles.insert("todo".to_string(), Tile { x: 8, w: 6, y: 0.0 });
    assert_eq!(ui(&mut s, |w| w.set_flow_layout(B, &tiles)).unwrap_err().code, ErrorCode::Usage);
    ui(&mut s, |w| w.set_flow_layout(B, &Default::default())).unwrap();
    assert!(flow(&s).0.iter().all(|x| x.layout.is_none()));
    // 新建白板自带默认工作流
    ui(&mut s, |w| w.create_board("default", "第二块", "second", "", None)).unwrap();
    let second = s.read(|r| r.resolve("default/second").and_then(|b| r.flow(&b))).unwrap();
    assert_eq!(second.0.len(), 6);
}

/* ---------------- 附件（PRD §5） ---------------- */

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

#[test]
fn attachments_upload_bind_delete_and_gc() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let files = dir.path().join("files");

    // 类型、大小、文件头
    let e = s.attach(T0, &b, None, &human(), "x.svg", b"<svg/>").unwrap_err();
    assert!(e.message.contains("不支持"), "{}", e.message);
    let e = s.attach(T0, &b, None, &human(), "x.png", b"<html>").unwrap_err();
    assert!(e.message.contains("对不上"), "{}", e.message);
    let big = vec![b'a'; files::MAX_BYTES + 1];
    let e = s.attach(T0, &b, None, &human(), "big.txt", &big).unwrap_err();
    assert!(e.message.contains("不能超过 10 MB"), "{}", e.message);
    assert_eq!(s.attach(T0, &b, Some(99), &human(), "a.png", PNG).unwrap_err().code, ErrorCode::NotFound);

    // 新建弹窗：先上传（没有卡片），新建时描述里引用到就归到这张卡片
    let a = s.attach(T0, &b, None, &human(), "C:\\截图.png", PNG).unwrap();
    assert_eq!((a.card_id, a.name.as_str(), a.image), (None, "截图.png", true));
    assert_eq!(a.markdown, format!("![截图.png](/api/files/{})", a.id));
    let body = format!("看图：{}", a.markdown);
    let c = exec(&mut s, B, &human(), T0, Cmd::New { title: "有图".into(), body: Some(body), column: None, top: false, labels: vec![] })
        .unwrap();
    assert_eq!(c.attachments.len(), 1);
    assert_eq!(c.attachments[0].card_id, Some(c.card.id));

    // agent 直接传到卡片上；同样的内容只存一份文件
    let log = s.attach(T0, &b, Some(c.card.id), &alpha(), "run.log", b"ok\n").unwrap();
    s.attach(T0, &b, Some(c.card.id), &alpha(), "again.png", PNG).unwrap();
    assert_eq!(std::fs::read_dir(&files).unwrap().count(), 2);
    let c = card(&s, B, c.card.id);
    assert_eq!(c.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["截图.png", "run.log", "again.png"]);
    assert_eq!(c.attachments[1].created_by, alpha());
    let (got, board, path) = s.attachment(log.id).unwrap().unwrap();
    assert_eq!((got.name.as_str(), board), ("run.log", b.id));
    assert_eq!(std::fs::read(path).unwrap(), b"ok\n");

    // 评论里引用到的未归属附件也归到卡片；别的白板、已归属的不受影响
    let c2 = exec(&mut s, B, &human(), T0, new("第二张")).unwrap().card.id;
    let pending = s.attach(T0, &b, None, &human(), "p.pdf", b"%PDF-1.7").unwrap();
    let text = format!("附上 {} 和 {}", pending.markdown, log.markdown);
    exec(&mut s, B, &human(), T0, Cmd::Comment { id: c2, text }).unwrap();
    assert_eq!(card(&s, B, c2).attachments.iter().map(|a| a.id).collect::<Vec<_>>(), [pending.id]);
    assert_eq!(s.attachment(log.id).unwrap().unwrap().0.card_id, Some(c.card.id));

    // 24 小时没归属的记录被清理；删除卡片后记录随之删除，不再被引用的文件被清理
    let orphan = s.attach(T0, &b, None, &human(), "o.txt", b"orphan").unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: c.card.id }).unwrap();
    assert!(s.attachment(log.id).unwrap().is_none());
    let (rows, _) = s.gc_files(T0 + 25 * 3600 * 1000).unwrap();
    assert_eq!(rows, 1);
    assert!(s.attachment(orphan.id).unwrap().is_none());
    // 文件要超过 1 小时没被引用才删：把它们的修改时间调早
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    for e in std::fs::read_dir(&files).unwrap() {
        std::fs::File::options().write(true).open(e.unwrap().path()).unwrap().set_modified(old).unwrap();
    }
    let (_, removed) = s.gc_files(T0 + 25 * 3600 * 1000).unwrap();
    assert_eq!(removed, 3, "截图/again 共用的 PNG、run.log、o.txt；p.pdf 还在用");
    assert!(s.attachment(pending.id).unwrap().unwrap().2.exists());

    // 备份：只复制还没有的文件
    let backup = dir.path().join("backups/files");
    assert_eq!(files::copy_new(&files, &backup).unwrap(), 1);
    assert_eq!(files::copy_new(&files, &backup).unwrap(), 0);
}

#[test]
fn in_memory_store_has_no_attachments() {
    let mut s = store();
    let b = s.read(|r| r.resolve(B)).unwrap();
    assert!(s.attach(T0, &b, None, &human(), "a.png", PNG).is_err());
}

#[test]
fn project_color_palette_or_any_hex() {
    let mut s = store();
    let nb = || NewBoard { name: "主白板".into(), key: "main".into(), desc: String::new(), pos: None };
    s.write(T0, Origin::ui(None), |w| w.create_project("甲", "a", "", Some("#4db5ff"), None, nb())).unwrap();
    s.write(T0, Origin::ui(None), |w| w.create_project("乙", "b", "", Some("#12ab9f"), None, nb())).unwrap();
    let r = s.write(T0, Origin::ui(None), |w| w.create_project("丙", "c", "", Some("teal"), None, nb()));
    assert_eq!(r.expect_err("不是颜色").code, ErrorCode::Usage);
    let r = s.write(T0, Origin::ui(None), |w| w.create_project("丙", "c", "", Some("#12ab9"), None, nb()));
    assert_eq!(r.expect_err("位数不对").code, ErrorCode::Usage);
    s.write(T0, Origin::ui(None), |w| w.update_project("a", None, None, Some("#00ff7f"))).unwrap();
    let ws = s.read(|r| r.workspace(T0)).unwrap();
    let color = |k: &str| ws.projects.iter().find(|p| p.key == k).unwrap().color.clone();
    assert_eq!(color("a"), "#00FF7F");
    assert_eq!(color("b"), "#12AB9F");
}

/* ------------------------------ 动态分页（#55） ------------------------------ */

#[test]
fn events_page_pages_backwards_without_gaps_or_duplicates() {
    let mut s = store();
    for i in 0..25 {
        exec(&mut s, B, &alpha(), T0 + i, new(&format!("卡{i}"))).unwrap();
    }
    let b = s.read(|r| r.resolve(B)).unwrap();
    let ids = |v: &[Event]| v.iter().map(|e| e.id).collect::<Vec<_>>();

    // 不带 before：最新的一页，id 倒序
    let all = s.read(|r| r.events_page(&b, None, 1000)).unwrap();
    assert!(all.len() >= 25, "25 张卡片至少有 25 条动态：{}", all.len());
    assert!(all.windows(2).all(|w| w[0].id > w[1].id), "新的在前");

    // 每页 5 条往前翻，拼起来要正好等于全部：不重不漏
    let all_ids = ids(&all);
    let mut got: Vec<i64> = Vec::new();
    let mut before = None;
    loop {
        let page = s.read(|r| r.events_page(&b, before, 5)).unwrap();
        if page.is_empty() {
            break;
        }
        let page_ids = ids(&page);
        assert!(page_ids.len() <= 5);
        assert!(page_ids.windows(2).all(|w| w[0] > w[1]));
        before = page_ids.last().copied();
        got.extend(page_ids);
        assert!(got.len() <= all_ids.len(), "翻页应当终止");
    }
    assert_eq!(got, all_ids);
}

#[test]
fn events_page_before_out_of_range() {
    let mut s = store();
    exec(&mut s, B, &alpha(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0 + 1, new("二")).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let ids = |v: &[Event]| v.iter().map(|e| e.id).collect::<Vec<_>>();
    let all = s.read(|r| r.events_page(&b, None, 100)).unwrap();
    let all_ids = ids(&all);
    let newest = *all_ids.first().unwrap();
    let oldest = *all_ids.last().unwrap();

    // 比所有 id 都大：当作从最新一条开始
    let page = s.read(|r| r.events_page(&b, Some(newest + 1000), 3)).unwrap();
    assert_eq!(ids(&page), all_ids[..3.min(all_ids.len())].to_vec());

    // 比最老的还老（或 0）：翻到底，返回空数组而不是报错
    assert!(s.read(|r| r.events_page(&b, Some(oldest - 1), 3)).unwrap().is_empty());
    assert!(s.read(|r| r.events_page(&b, Some(oldest), 3)).unwrap().is_empty());
    assert!(s.read(|r| r.events_page(&b, Some(0), 3)).unwrap().is_empty());
}

/* ------------------------------ 标签顺序（#58） ------------------------------ */

fn label_names(s: &Store, project: &str) -> Vec<String> {
    s.read(|r| r.labels(project)).unwrap().into_iter().map(|l| l.name).collect()
}

#[test]
fn labels_reorder_and_new_labels_go_last() {
    let mut s = store();
    let a = ui(&mut s, |w| w.create_label("default", "甲", None, false)).unwrap();
    let b = ui(&mut s, |w| w.create_label("default", "乙", None, false)).unwrap();
    let c = ui(&mut s, |w| w.create_label("default", "丙", None, false)).unwrap();
    assert_eq!(label_names(&s, "default"), ["甲", "乙", "丙"]);

    // 只给出部分顺序：没提到的按原顺序补在后面
    ui(&mut s, |w| w.reorder_labels("default", &[c.id, a.id])).unwrap();
    assert_eq!(label_names(&s, "default"), ["丙", "甲", "乙"]);

    // 新建标签排在最后，不会插到中间
    let d = ui(&mut s, |w| w.create_label("default", "丁", None, false)).unwrap();
    assert_eq!(label_names(&s, "default"), ["丙", "甲", "乙", "丁"]);

    // 全部重排：顺序以传入的 ids 为准
    ui(&mut s, |w| w.reorder_labels("default", &[b.id, c.id, a.id, d.id])).unwrap();
    assert_eq!(label_names(&s, "default"), ["乙", "丙", "甲", "丁"]);
    // board labels --json 读的就是这个顺序（labels_of 按 sort 排）
    let labels = s.read(|r| r.labels("default")).unwrap();
    assert_eq!(labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["乙", "丙", "甲", "丁"]);
}

#[test]
fn labels_reorder_rejects_illegal_ids_without_changing_order() {
    let mut s = store();
    let a = ui(&mut s, |w| w.create_label("default", "甲", None, false)).unwrap();
    let b = ui(&mut s, |w| w.create_label("default", "乙", None, false)).unwrap();
    // 另一个项目里的标签
    let other = ui(&mut s, |w| {
        w.create_project(
            "乙项目",
            "b",
            "",
            None,
            None,
            NewBoard { name: "主白板".into(), key: "main".into(), desc: String::new(), pos: None },
        )?;
        w.create_label("b", "别处", None, false)
    })
    .unwrap();
    assert_eq!(label_names(&s, "default"), ["甲", "乙"]);

    // 混入别的项目的标签
    let e = ui(&mut s, |w| w.reorder_labels("default", &[b.id, other.id])).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("属于项目"), "{}", e.message);
    // id 不存在
    let e = ui(&mut s, |w| w.reorder_labels("default", &[a.id, 9999])).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("9999"), "{}", e.message);
    // 重复的 id
    let e = ui(&mut s, |w| w.reorder_labels("default", &[a.id, a.id])).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("两次"), "{}", e.message);
    // 项目不存在
    assert_eq!(ui(&mut s, |w| w.reorder_labels("nope", &[])).unwrap_err().code, ErrorCode::BoardNotFound);

    // 出错时顺序一点没变
    assert_eq!(label_names(&s, "default"), ["甲", "乙"]);
}

/* ------------------------------ agent 名字与 agent 级心跳（PRD §3） ------------------------------ */

const NAME: &str = "claude/opus-5.5";

fn check(s: &mut Store, name: &str, at: i64) -> Result<AgentInfo> {
    s.write(at, Origin::cli(), |w| w.agent_check(name, Some(1)))
}
fn agent_beat(s: &mut Store, name: &str, at: i64, done: bool) -> Result<AgentInfo> {
    s.write(at, Origin::cli(), |w| w.agent_beat(name, done))
}
fn online(s: &Store, at: i64) -> Vec<String> {
    s.read(|r| r.agents(at)).unwrap().into_iter().map(|a| a.name).collect()
}

#[test]
fn agent_name_format() {
    let long = format!("a/{}", "b".repeat(47));
    for bad in ["claude", "Claude/opus", "a/b/c", "claude/opus 5.5", "claude/", "/opus", "-x/y", "x/.y", "", long.as_str()] {
        let e = s_check(bad);
        assert_eq!(e.code, ErrorCode::Usage, "{bad}");
        assert!(e.message.contains("<运行环境>/<模型>"), "要给出格式说明：{}", e.message);
    }
    let max = format!("a/{}", "b".repeat(46));
    for good in [NAME, "dsh/deepseek-v4.1-flash", "claude/opus-5.5-login", "codex/gpt-5", max.as_str()] {
        check(&mut store(), good, T0).unwrap_or_else(|e| panic!("{good}: {e}"));
    }
}
fn s_check(name: &str) -> BoardError {
    check(&mut store(), name, T0).expect_err("应当被拒绝")
}

#[test]
fn agent_check_takes_name_until_offline() {
    let mut s = store();
    let r0 = s.rev().unwrap();
    let a = check(&mut s, NAME, T0).unwrap();
    assert_eq!((a.checked_at, a.beat_at, a.cards.len()), (T0, T0, 0));
    assert!(s.rev().unwrap() > r0, "上线要推送给界面");
    assert_eq!(online(&s, T0), [NAME]);
    // 在线时再 check：NAME_TAKEN，detail 带名字
    let e = check(&mut s, NAME, T0 + SEC).unwrap_err();
    assert_eq!(e.code, ErrorCode::NameTaken);
    assert_eq!(e.detail.unwrap()["name"], NAME);
    // 加后缀可以
    check(&mut s, "claude/opus-5.5-login", T0 + SEC).unwrap();
    // check 之后不发心跳：50 秒后离线
    assert_eq!(check(&mut s, NAME, T0 + 49 * SEC).unwrap_err().code, ErrorCode::NameTaken);
    assert!(s.read(|r| r.require_online(NAME, T0 + 49 * SEC)).is_ok());
    let e = s.read(|r| r.require_online(NAME, T0 + 50 * SEC)).unwrap_err();
    assert_eq!(e.code, ErrorCode::NameOffline);
    assert!(e.message.contains(&format!("board check --as {NAME}")), "{}", e.message);
    assert_eq!(agent_beat(&mut s, NAME, T0 + 50 * SEC, false).unwrap_err().code, ErrorCode::NameOffline, "离线后心跳被拒绝");
    let r1 = s.rev().unwrap();
    assert_eq!(s.sweep(T0 + 50 * SEC).unwrap(), 1, "扫描把它标为离线");
    assert!(s.rev().unwrap() > r1, "下线要推送给界面");
    assert_eq!(online(&s, T0 + 50 * SEC), ["claude/opus-5.5-login"]);
    // 离线后同一个名字可以再次 check；首次 check 时间保留
    let a = check(&mut s, NAME, T0 + 60 * SEC).unwrap();
    assert_eq!((a.checked_at, a.beat_at), (T0, T0 + 60 * SEC));
    // 没 check 过的名字不在线
    assert_eq!(s.read(|r| r.require_online("codex/gpt-5", T0)).unwrap_err().code, ErrorCode::NameOffline);
    assert_eq!(agent_beat(&mut s, "codex/gpt-5", T0, true).unwrap_err().code, ErrorCode::NameOffline);
}

#[test]
fn agent_beat_keeps_cards_on_all_boards() {
    let mut s = store();
    oss(&mut s);
    let me = Actor::agent(NAME);
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, "oss/main", &human(), T0, new("甲")).unwrap();
    check(&mut s, NAME, T0).unwrap();
    exec(&mut s, B, &me, T0, claim(None)).unwrap();
    exec(&mut s, "oss/main", &me, T0, claim(None)).unwrap();

    // 第一次 agent 级心跳：两张卡片都开始第 1 次
    let r0 = s.rev().unwrap();
    let a = agent_beat(&mut s, NAME, T0 + 2 * SEC, false).unwrap();
    assert_eq!(a.cards.iter().map(|c| (c.board.as_str(), c.card.id)).collect::<Vec<_>>(), [(B, 1), ("oss/main", 1)]);
    assert!(a.cards.iter().all(|c| c.card.run_started_at == Some(T0 + 2 * SEC) && c.card.runs == 1));
    assert!(s.rev().unwrap() > r0, "开始第 1 次记了动态，要推送");

    // 之后只发 agent 级心跳：卡片不释放，心跳次数增长，rev 不变（不推送）
    let r1 = s.rev().unwrap();
    let mut t = T0 + 2 * SEC;
    for _ in 0..60 {
        t += BEAT_MS;
        agent_beat(&mut s, NAME, t, false).unwrap();
        assert_eq!(s.sweep(t + 49 * SEC).unwrap(), 0);
    }
    assert_eq!(s.rev().unwrap(), r1, "只更新心跳时间不推送");
    for path in [B, "oss/main"] {
        let c = card(&s, path, 1);
        assert_eq!(c.card.holder.as_deref(), Some(NAME));
        assert_eq!((c.card.beat_at, c.card.beats, c.card.runs), (Some(t), 61, 1));
        assert_eq!(c.activity.iter().filter(|a| a.text.contains("开始第 1 次")).count(), 1);
    }

    // final 一张：之后 agent 级心跳不再刷新它；50 秒内没移走就释放；之后的 board beat <ID> 被拒绝
    let tf = t + SEC;
    exec(&mut s, B, &me, tf, beat(1, true)).unwrap();
    let a = agent_beat(&mut s, NAME, tf + 9 * SEC, false).unwrap();
    let main = &a.cards.iter().find(|c| c.board == B).unwrap().card;
    assert_eq!((main.beat_at, main.beats), (Some(tf), 62), "final 之后不再被刷新");
    assert_eq!(code(exec(&mut s, B, &me, tf + 10 * SEC, beat(1, false))), ErrorCode::Usage);
    for k in 2..=5 {
        agent_beat(&mut s, NAME, tf + 9 * SEC + k * BEAT_MS, false).unwrap();
    }
    assert_eq!(s.sweep(tf + 50 * SEC).unwrap(), 1, "final 之后 50 秒没移走：释放");
    assert_eq!(card(&s, B, 1).card.holder, None);
    assert_eq!(card(&s, "oss/main", 1).card.holder.as_deref(), Some(NAME), "另一张照常持有");

    // 名字下线（heartbeat.sh stop → board beat --final）：立刻离线；它持有的卡片 50 秒内释放，记「心跳中断」
    let last = tf + 9 * SEC + 5 * BEAT_MS;
    let r2 = s.rev().unwrap();
    agent_beat(&mut s, NAME, last + SEC, true).unwrap();
    assert!(online(&s, last + SEC).is_empty());
    assert!(s.rev().unwrap() > r2, "下线要推送");
    assert_eq!(agent_beat(&mut s, NAME, last + 2 * SEC, false).unwrap_err().code, ErrorCode::NameOffline);
    agent_beat(&mut s, NAME, last + 2 * SEC, true).unwrap();
    assert_eq!(s.sweep(last + 49 * SEC).unwrap(), 0);
    assert_eq!(s.sweep(last + 50 * SEC).unwrap(), 1);
    let snap = snapshot(&s, "oss/main", last + 50 * SEC);
    assert_eq!(snap.cols["todo"], vec![1], "回到领取起点最上面");
    let c = card(&s, "oss/main", 1);
    assert_eq!((c.card.holder, c.card.runs_lost), (None, 1));
    assert!(c.activity.iter().any(|a| a.action == "lost" && a.text.contains("心跳中断")), "{:?}", c.activity.last());
    // 本次耗时：第一次心跳到最后一次心跳
    assert_eq!(c.card.work_ms, last - (T0 + 2 * SEC));
}
