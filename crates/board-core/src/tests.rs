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
    Cmd::New { title: title.into(), body: None, column: None, top: false, labels: vec![], from: None }
}
fn claim(id: Option<i64>) -> Cmd {
    Cmd::Claim { id }
}
fn mv(id: i64, col: &str) -> Cmd {
    Cmd::Move { id, column: col.into(), top: false, index: None, order: None, before: None, after: None }
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
        r.board_snapshot(&b, now, true)
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
fn heartbeat_final_ends_heartbeat_without_exit_status() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    beat_span(&mut s, &alpha(), 1, T0 + SEC, T0 + 3 * MIN);
    let end = T0 + 3 * MIN + 5 * SEC;
    let c = exec(&mut s, B, &alpha(), end, beat(1, true)).unwrap();
    assert_eq!(c.card.holder.as_deref(), Some("alpha"), "final 不释放领取");
    assert_eq!((c.card.run_started_at, c.card.work_ms, c.card.runs_lost), (None, 3 * MIN + 4 * SEC, 0));
    assert!(c.activity.iter().any(|a| a.action == "beat" && a.text.contains("结束了心跳，第 1 次耗时 3 分 4 秒")));
    assert!(!c.activity.iter().any(|a| a.text.contains("正常退出")), "final 只知道心跳结束，不知道 agent 的退出码");
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
    // final 之后的心跳被拒绝（这次领取已经结束），也不新开第 2 次
    assert_eq!(code(exec(&mut s, B, &alpha(), T0 + 10 * SEC, beat(1, false))), ErrorCode::Usage);
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

/// 领取后第一次心跳之前就 final（board heartbeat final）、或者没有目标状态的 giveup：同样记下 final。
/// 之后的 agent 级心跳不给它开始计时，50 秒后照常释放；final 之后照常能在 50 秒内移到交审状态
#[test]
fn final_before_the_first_beat_still_ends_the_claim() {
    let mut s = store();
    let me = Actor::agent("test/alpha");
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &human(), T0, new("二")).unwrap();
    exec(&mut s, B, &human(), T0, new("三")).unwrap();
    s.write(T0, Origin::cli(), |w| w.agent_check_with_agent(None, &me.name, None, NameOwner::Anyone)).unwrap();
    // 一：领取后 1 秒就 final，然后 agent 级心跳照常到达
    exec(&mut s, B, &me, T0, claim(Some(1))).unwrap();
    let c = exec(&mut s, B, &me, T0 + SEC, beat(1, true)).unwrap();
    assert_eq!((c.card.runs, c.card.run_started_at), (0, None));
    assert!(!c.activity.iter().any(|a| a.text.contains("结束了心跳")), "没有计时，不记「结束了心跳」");
    for k in 1..=6 {
        s.write(T0 + k * 10 * SEC, Origin::cli(), |w| w.agent_beat(&me.name, false).map(|_| ())).unwrap();
    }
    let c = card(&s, B, 1);
    assert_eq!((c.card.runs, c.card.run_started_at), (0, None), "agent 级心跳不给 final 过的卡开始计时");
    assert!(!c.activity.iter().any(|a| a.text.contains("开始第 1 次")));
    assert_eq!(s.sweep(T0 + 51 * SEC).unwrap(), 1);
    let c = card(&s, B, 1);
    assert_eq!((c.card.holder, c.card.column.as_str(), c.card.runs_lost), (None, "todo", 0));
    assert!(c.activity.iter().any(|a| a.text.contains("结束心跳后 50 秒内没有交审或放回")));

    // 二：没有交审状态时的 giveup（只评论、结束计时）在第一次心跳之前：同样到期释放
    s.write(T0 + 2 * MIN, Origin::cli(), |w| w.agent_check_with_agent(None, &me.name, None, NameOwner::Anyone)).unwrap();
    exec(&mut s, B, &me, T0 + 2 * MIN, claim(Some(2))).unwrap();
    let c = s.write(T0 + 2 * MIN + SEC, Origin::cli(), |w| {
        let b = w.resolve(B)?;
        w.exec(&b, &me, Cmd::Beat { id: 2, done: true })
    });
    assert!(c.is_ok());
    s.write(T0 + 2 * MIN + 20 * SEC, Origin::cli(), |w| w.agent_beat(&me.name, false).map(|_| ())).unwrap();
    assert_eq!(s.sweep(T0 + 2 * MIN + 52 * SEC).unwrap(), 1);
    assert_eq!(card(&s, B, 2).card.holder, None);

    // 三：领取 → 立刻 final → 50 秒内移到交审状态：照常成功
    exec(&mut s, B, &me, T0 + 5 * MIN, claim(Some(3))).unwrap();
    exec(&mut s, B, &me, T0 + 5 * MIN, beat(3, true)).unwrap();
    let c = exec(&mut s, B, &me, T0 + 5 * MIN + 30 * SEC, mv(3, "review")).unwrap();
    assert_eq!((c.card.column.as_str(), c.card.holder), ("review", None));
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

/// 动态里「第 n 次耗时 …」写的秒数（dur_words 的格式：3 分 4 秒 / 8 秒 / 1 小时 5 分）
fn shown_secs(text: &str) -> Option<i64> {
    let rest = text.split_once("次耗时")?.1.trim_start_matches('约');
    let (mut secs, mut num) = (0, 0);
    for tok in rest.split('，').next()?.split_whitespace() {
        match tok {
            "小时" => secs += num * 3600,
            "分" => secs += num * 60,
            "秒" => secs += num,
            n => num = n.parse().ok()?,
        }
    }
    Some(secs)
}

/// #665：每一次都有不足一秒的零头时，累计等于动态里逐次记的整秒耗时之和（每一次先截断再相加），
/// final 后交审、强制释放、心跳中断、不发 final 直接交审都是同一口径；进行中的这一次不并入累计
#[test]
fn work_ms_is_sum_of_whole_seconds_per_run() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    // 第 1 次：48.4 秒，先 final 再交审
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    let t1 = T0 + SEC + 300;
    beat_span(&mut s, &alpha(), 1, t1, t1 + 40 * SEC);
    exec(&mut s, B, &alpha(), t1 + 48 * SEC + 400, beat(1, true)).unwrap();
    exec(&mut s, B, &alpha(), t1 + 50 * SEC, mv(1, "review")).unwrap();
    // 第 2 次：1 分 48.6 秒后被人强制释放
    exec(&mut s, B, &human(), T0 + 2 * MIN, mv(1, "todo")).unwrap();
    exec(&mut s, B, &beta(), T0 + 2 * MIN, claim(None)).unwrap();
    let t2 = T0 + 2 * MIN + 250;
    beat_span(&mut s, &beta(), 1, t2, t2 + 100 * SEC);
    exec(&mut s, B, &human(), t2 + 108 * SEC + 600, Cmd::Release { id: 1 }).unwrap();
    // 第 3 次：10.8 秒后心跳中断，以最后一次心跳为结束
    exec(&mut s, B, &alpha(), T0 + 5 * MIN, claim(None)).unwrap();
    let t3 = T0 + 5 * MIN + 700;
    let last = t3 + 10 * SEC + 800;
    beat_span(&mut s, &alpha(), 1, t3, last);
    assert_eq!(s.sweep(last + BEAT_MS * BEAT_MISSES).unwrap(), 1);
    // 第 4 次：11 分 56.9 秒，没发 final 直接交审
    exec(&mut s, B, &beta(), T0 + 8 * MIN, claim(None)).unwrap();
    let t4 = T0 + 8 * MIN + 100;
    beat_span(&mut s, &beta(), 1, t4, t4 + 716 * SEC);
    let c = exec(&mut s, B, &beta(), t4 + 716 * SEC + 900, mv(1, "review")).unwrap();

    let shown: Vec<i64> = c.activity.iter().filter_map(|a| shown_secs(&a.text)).collect();
    assert_eq!(shown, vec![48, 108, 10, 716], "{:?}", c.activity.iter().map(|a| &a.text).collect::<Vec<_>>());
    // 原始毫秒相加是 884.7 秒（显示 14:44）；每一次先截断再相加是 14:42，和动态里四次相加一样
    assert_eq!((c.card.runs, c.card.runs_lost, c.card.work_ms), (4, 1, shown.iter().sum::<i64>() * SEC));
    assert_eq!(c.card.work_ms, (14 * 60 + 42) * SEC);
    assert_eq!(card(&s, B, 1).card.work_ms, c.card.work_ms, "快照、详情读的是同一个值");

    // 第 5 次进行中：本次按毫秒实时走，不并入累计；结束后同样截断到整秒并入
    exec(&mut s, B, &human(), T0 + 30 * MIN, mv(1, "todo")).unwrap();
    exec(&mut s, B, &alpha(), T0 + 30 * MIN, claim(None)).unwrap();
    let t5 = T0 + 30 * MIN + 450;
    exec(&mut s, B, &alpha(), t5, beat(1, false)).unwrap();
    let c = card(&s, B, 1).card;
    assert_eq!((c.runs, c.run_started_at, c.work_ms), (5, Some(t5), 882 * SEC));
    let c = exec(&mut s, B, &alpha(), t5 + 9 * SEC + 999, beat(1, true)).unwrap().card;
    assert_eq!((c.run_started_at, c.work_ms), (None, 891 * SEC));

    // 只领取一次：显示的秒数不变（原来也是整个截断一次）
    exec(&mut s, B, &human(), T0 + 40 * MIN, new("二")).unwrap();
    exec(&mut s, B, &beta(), T0 + 40 * MIN, claim(Some(2))).unwrap();
    exec(&mut s, B, &beta(), T0 + 40 * MIN + 200, beat(2, false)).unwrap();
    let c = exec(&mut s, B, &beta(), T0 + 40 * MIN + 3 * SEC + 900, beat(2, true)).unwrap().card;
    assert_eq!(c.work_ms, 3 * SEC);
}

/// 完成时间（finished_at）：最近一次领取的结束时间，多次领取取最后一次；异常退出也有；评论、编辑、移动不改它
#[test]
fn finished_at_is_last_run_end() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    assert_eq!(card(&s, B, 1).card.finished_at, None, "没有领取记录时为空");
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    beat_span(&mut s, &alpha(), 1, T0 + SEC, T0 + 2 * MIN);
    assert_eq!(card(&s, B, 1).card.finished_at, None, "进行中时为空");
    exec(&mut s, B, &alpha(), T0 + 2 * MIN, beat(1, true)).unwrap();
    exec(&mut s, B, &alpha(), T0 + 2 * MIN + SEC, mv(1, "review")).unwrap();
    let first = card(&s, B, 1).card.finished_at.expect("交审后有完成时间");
    assert!(first >= T0 + 2 * MIN - SEC && first <= T0 + 2 * MIN + SEC, "{first}");
    // 评论、编辑描述、移动状态都不改它
    exec(&mut s, B, &human(), T0 + 5 * MIN, Cmd::Comment { id: 1, text: "看过了".into() }).unwrap();
    exec(&mut s, B, &human(), T0 + 6 * MIN, Cmd::Edit { id: 1, title: None, body: Some("新描述".into()) }).unwrap();
    exec(&mut s, B, &human(), T0 + 7 * MIN, mv(1, "todo")).unwrap();
    assert_eq!(card(&s, B, 1).card.finished_at, Some(first));
    // 第 2 次：心跳中断（异常退出）也有结束时间，取最后一次
    exec(&mut s, B, &beta(), T0 + 8 * MIN, claim(None)).unwrap();
    exec(&mut s, B, &beta(), T0 + 8 * MIN + SEC, beat(1, false)).unwrap();
    let last = T0 + 8 * MIN + SEC;
    s.sweep(last + BEAT_MS * BEAT_MISSES).unwrap();
    let c = card(&s, B, 1).card;
    assert_eq!(c.runs_lost, 1);
    let second = c.finished_at.expect("异常退出也有结束时间");
    assert!(second > first, "多次领取取最后一次");
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
fn system_identity_is_board() {
    // 系统身份（超时释放、心跳中断这类）在文本里叫 board，不再是「系统」；API 的 actor_kind 仍是 "system"
    let mut s = store();
    assert_eq!(Actor::system().display(), "board");
    assert_eq!(Actor::human().display(), "人");
    assert_eq!(Actor::agent("alpha").display(), "alpha");
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    let c = exec(&mut s, B, &beta(), T0 + 31 * MIN, claim(None)).unwrap();
    let t = c.activity.iter().find(|a| a.action == "timeout").expect("超时释放的动态");
    assert_eq!(t.actor.kind, ActorKind::System);
    assert_eq!(t.actor.display(), "board");
    assert!(t.text.contains("卡片回到「待办」最上面"), "{}", t.text);
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
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, mv(1, "doing"))), ErrorCode::ForbiddenColumn, "直接移进锁定状态");
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
    let c = exec(&mut s, B, &human(), T0, Cmd::New { title: "想法".into(), body: None, column: Some("inbox".into()), top: false, labels: vec![], from: None }).unwrap();
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
    let a = exec(&mut s, B, &alpha(), T0, Cmd::New { title: "agent 记的".into(), body: None, column: Some("inbox".into()), top: false, labels: vec![], from: None }).unwrap();
    assert_eq!(a.card.column, "inbox");
    assert_eq!(a.card.holder, None);
    let b = exec(&mut s, B, &alpha(), T0, Cmd::New { title: "agent 又记的".into(), body: None, column: Some("inbox".into()), top: false, labels: vec![], from: None }).unwrap();
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
    // agent 新建时带上：可以（#422，什么时候带由 board-skills 约束）；之后对这张卡加、去仍被拒绝
    let b = exec(&mut s, B, &beta(), T0, Cmd::New { title: "b".into(), body: None, column: None, top: false, labels: vec![mine.id], from: None })
        .unwrap()
        .card
        .id;
    assert!(card(&s, B, b).card.labels.contains(&mine.id));
    assert_eq!(code(exec(&mut s, B, &beta(), T0, Cmd::Label { id: b, add: vec![], remove: vec![mine.id] })), ErrorCode::Usage);
    assert!(card(&s, B, b).card.labels.contains(&mine.id));

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
    ui(&mut s, |w| w.update_label("default", mine.id, None, None, Some(false), None)).unwrap();
    let ls = s.read(|r| r.labels("default")).unwrap();
    assert!(!ls.iter().find(|l| l.id == mine.id).unwrap().human_only);
    exec(&mut s, B, &alpha(), T0, Cmd::Label { id: 1, add: vec![mine.id], remove: vec![] }).unwrap();
}

/// 优先标签（PRD §8 规则 1）：不带 ID 领取时带优先标签的卡片先被领取；两组内部仍按工作流顺序、状态内从上到下
#[test]
fn priority_labels_claim_first() {
    let mut s = store();
    let p = ui(&mut s, |w| w.create_label("default", "急", None, false)).unwrap();
    assert!(!p.priority, "新建的标签默认不是优先");
    ui(&mut s, |w| w.update_label("default", p.id, None, None, None, Some(true))).unwrap();
    assert!(s.read(|r| r.labels("default")).unwrap()[0].priority, "刷新后保持");
    let gamma = Actor::agent("gamma");
    for t in ["A", "B", "C"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    exec(&mut s, B, &human(), T0, Cmd::Label { id: 3, add: vec![p.id], remove: vec![] }).unwrap();
    // 待办 A、B、C，只有 C 带优先标签：先领 C，再领 A
    assert_eq!(exec(&mut s, B, &alpha(), T0, claim(None)).unwrap().card.id, 3);
    assert_eq!(exec(&mut s, B, &beta(), T0, claim(None)).unwrap().card.id, 1);
    // 带 ID 领取不受影响
    assert_eq!(exec(&mut s, B, &gamma, T0, claim(Some(2))).unwrap().card.id, 2);

    // 「待办」里没有优先的、「返工」里有：领「返工」那张；同一个状态里两张优先的：先领上面那张
    let mut s = store();
    let p = ui(&mut s, |w| w.create_label("default", "急", None, false)).unwrap();
    ui(&mut s, |w| w.update_label("default", p.id, None, None, None, Some(true))).unwrap();
    for t in ["A", "B", "C", "D"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    exec(&mut s, B, &human(), T0, mv(3, "rework")).unwrap();
    exec(&mut s, B, &human(), T0, mv(4, "rework")).unwrap();
    for id in [3, 4] {
        exec(&mut s, B, &human(), T0, Cmd::Label { id, add: vec![p.id], remove: vec![] }).unwrap();
    }
    assert_eq!(exec(&mut s, B, &alpha(), T0, claim(None)).unwrap().card.id, 3);
    assert_eq!(exec(&mut s, B, &beta(), T0, claim(None)).unwrap().card.id, 4);
    // 优先的领完了：回到原来的顺序
    assert_eq!(exec(&mut s, B, &gamma, T0, claim(None)).unwrap().card.id, 1);

    // 取消优先、或从卡片上去掉优先标签：顺序恢复
    let mut s = store();
    let p = ui(&mut s, |w| w.create_label("default", "急", None, false)).unwrap();
    ui(&mut s, |w| w.update_label("default", p.id, None, None, None, Some(true))).unwrap();
    for t in ["A", "B"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    exec(&mut s, B, &human(), T0, Cmd::Label { id: 2, add: vec![p.id], remove: vec![] }).unwrap();
    ui(&mut s, |w| w.update_label("default", p.id, None, None, None, Some(false))).unwrap();
    assert_eq!(exec(&mut s, B, &alpha(), T0, claim(None)).unwrap().card.id, 1);
}

/// 旧数据库的标签没有「优先」列：迁移后都不是优先
#[test]
fn old_labels_are_not_priority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        ui(&mut s, |w| w.create_label("default", "旧", None, false)).unwrap();
    }
    rusqlite::Connection::open(&path).unwrap().execute_batch("ALTER TABLE labels DROP COLUMN priority").unwrap();
    let s = Store::open(&path).unwrap();
    assert!(!s.read(|r| r.labels("default")).unwrap()[0].priority);
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
    exec(&mut s, B, &human(), T0, Cmd::Move { id: 3, column: "todo".into(), top: false, index: Some(0), order: None, before: None, after: None }).unwrap();
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![3, 1, 2]);
    exec(&mut s, B, &human(), T0, Cmd::Move { id: 3, column: "review".into(), top: false, index: Some(0), order: None, before: None, after: None }).unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Move { id: 1, column: "review".into(), top: true, index: None, order: None, before: None, after: None }).unwrap();
    let snap = snapshot(&s, B, T0);
    assert_eq!(snap.cols["todo"], vec![2]);
    assert_eq!(snap.cols["review"], vec![1, 3]);
    exec(&mut s, B, &alpha(), T0, Cmd::New { title: "top".into(), body: None, column: None, top: true, labels: vec![], from: None }).unwrap();
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
    let mv_order = |id: i64, index: usize, order: Vec<i64>| Cmd::Move { id, column: "todo".into(), top: false, index: Some(index), order: Some(order), before: None, after: None };
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
fn delete_own_comment() {
    let mut s = store();
    exec(&mut s, B, &alpha(), T0, new("a")).unwrap();
    let d = exec(&mut s, B, &human(), T0, Cmd::Comment { id: 1, text: "粘贴错了".into() }).unwrap();
    let mine = d.activity.last().unwrap().id;
    let d = exec(&mut s, B, &alpha(), T0, Cmd::Comment { id: 1, text: "agent 的评论".into() }).unwrap();
    let theirs = d.activity.last().unwrap().id;
    let sys = d.activity[0].id;
    assert_eq!(snapshot(&s, B, T0).cards[&1].comments, 2);
    let del = |activity: i64| Cmd::DeleteComment { id: 1, activity };

    // 人删自己的评论：从动态里移除，评论数减一，别人的动态不受影响
    let d = exec(&mut s, B, &human(), T0 + MIN, del(mine)).unwrap();
    assert!(!d.activity.iter().any(|a| a.id == mine));
    assert!(d.activity.iter().any(|a| a.id == theirs));
    assert!(!card(&s, B, 1).activity.iter().any(|a| a.id == mine));
    assert_eq!(snapshot(&s, B, T0 + MIN).cards[&1].comments, 1);
    assert!(snapshot(&s, B, T0 + MIN).log.iter().any(|e| e.op == "delete_comment"));

    // agent 删自己的评论、人删 agent 的评论：NOT_AUTHOR；系统动态：USAGE；不存在：NOT_FOUND
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, del(theirs))), ErrorCode::NotAuthor);
    assert_eq!(code(exec(&mut s, B, &human(), T0, del(theirs))), ErrorCode::NotAuthor);
    assert_eq!(code(exec(&mut s, B, &human(), T0, del(sys))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, del(9999))), ErrorCode::NotFound);
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
    exec(&mut s, B, &alpha(), T0, Cmd::Move { id: 3, column: "todo".into(), top: true, index: None, order: None, before: None, after: None }).unwrap();
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
fn old_database_gets_board_settings_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("ALTER TABLE boards DROP COLUMN saved_layout; ALTER TABLE boards DROP COLUMN keep_order; ALTER TABLE boards DROP COLUMN keep_sorted")
            .unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    let snap = |s: &Store| s.read(|r| r.resolve(B).and_then(|b| r.board_snapshot(&b, T0, false))).unwrap();
    assert!(!snap(&s).keep_sorted, "升级前的白板（保存顺序开着）保留顺序关着：拖到哪就在哪，和原来一样");
    assert!(snap(&s).saved_layout.is_none());
    ui(&mut s, |w| w.set_keep_sorted(B, true)).unwrap();
    assert!(snap(&s).keep_sorted);
}

#[test]
fn old_database_keep_order_off_becomes_keep_sorted_and_moved_at_backfilled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        ui(&mut s, |w| w.create_board("default", "第二块", "second", "", None)).unwrap();
        exec(&mut s, B, &human(), T0, new("a")).unwrap();
        exec(&mut s, B, &human(), T0, new("b")).unwrap();
        exec(&mut s, B, &human(), T0 + 5 * MIN, mv(1, "待审核")).unwrap();
    }
    // 模拟升级前的数据库：没有保留顺序、排序依据、移入时间；主白板关着「保存顺序」
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "ALTER TABLE boards DROP COLUMN keep_sorted; ALTER TABLE statuses DROP COLUMN order_by; ALTER TABLE statuses DROP COLUMN order_desc;
             ALTER TABLE cards DROP COLUMN moved_at; UPDATE boards SET keep_order = 0 WHERE key = 'main';",
        )
        .unwrap();
    }
    let s = Store::open(&path).unwrap();
    let main = snapshot(&s, B, T0);
    assert!(main.keep_sorted, "关着保存顺序（同一块里不能拖动排序）的白板打开保留顺序");
    assert!(!snapshot(&s, "default/second", T0).keep_sorted);
    assert!(main.statuses.iter().all(|x| x.order == Order::default()), "旧状态都按移入时间");
    assert_eq!(main.cards[&1].moved_at, T0 + 5 * MIN, "按最后一次换状态的动态补");
    assert_eq!(main.cards[&2].moved_at, T0);
}

#[test]
fn moved_at_changes_only_when_status_changes() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("a")).unwrap();
    exec(&mut s, B, &human(), T0, new("b")).unwrap();
    let at = |s: &Store, id: i64| snapshot(s, B, T0).cards[&id].moved_at;
    assert_eq!(at(&s, 1), T0, "新建算移入");
    exec(&mut s, B, &human(), T0 + MIN, mv(1, "待审核")).unwrap();
    assert_eq!(at(&s, 1), T0 + MIN);
    // 同一状态里调整顺序不算移入
    exec(&mut s, B, &human(), T0 + 2 * MIN, Cmd::Move { id: 2, column: "todo".into(), top: true, index: None, order: None, before: None, after: None })
        .unwrap();
    assert_eq!(at(&s, 2), T0);
    // 领取进入锁定状态、释放回来都算
    exec(&mut s, B, &alpha(), T0 + 3 * MIN, claim(Some(2))).unwrap();
    assert_eq!(at(&s, 2), T0 + 3 * MIN);
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
    // 白板设置：保存布局、复原、应用；复原不丢已保存的布局；删掉的状态跳过，新状态按默认补位
    let snap = |s: &Store| s.read(|r| r.resolve(B).and_then(|b| r.board_snapshot(&b, T0, false))).unwrap();
    assert!(snap(&s).saved_layout.is_none());
    assert!(!snap(&s).keep_sorted, "新白板保留顺序关着");
    assert_eq!(ui(&mut s, |w| w.apply_saved_layout(B)).unwrap_err().code, ErrorCode::Usage);
    let mut saved = std::collections::BTreeMap::new();
    saved.insert("todo".to_string(), Tile { x: 3, w: 6, y: 0.0 });
    saved.insert(t.key.clone(), Tile { x: 0, w: 3, y: 1.0 });
    ui(&mut s, |w| w.save_flow_layout(B, &saved)).unwrap();
    assert!(flow(&s).0.iter().all(|x| x.layout.is_none()), "保存布局不改当前布局");
    tiles.insert("todo".to_string(), Tile { x: 0, w: 12, y: 0.0 });
    tiles.insert("doing".to_string(), Tile { x: 0, w: 4, y: 5.0 });
    ui(&mut s, |w| w.set_flow_layout(B, &tiles)).unwrap();
    ui(&mut s, |w| w.delete_status(B, &t.key)).unwrap();
    ui(&mut s, |w| w.apply_saved_layout(B)).unwrap();
    assert_eq!(flow(&s).get("todo").unwrap().layout, Some(Tile { x: 3, w: 6, y: 0.0 }));
    assert_eq!(flow(&s).get("doing").unwrap().layout, None, "没保存过位置的状态按默认规则补位");
    assert_eq!(snap(&s).saved_layout.unwrap().len(), 1, "已删除的状态不出现在已保存的布局里");
    ui(&mut s, |w| w.set_flow_layout(B, &Default::default())).unwrap();
    assert!(snap(&s).saved_layout.is_some(), "复原布局不丢已保存的布局");
    ui(&mut s, |w| w.set_keep_sorted(B, true)).unwrap();
    assert!(snap(&s).keep_sorted);
    // 排序依据：每个状态各自保存；按标签排不分方向
    assert_eq!(flow(&s).get("done").unwrap().order, Order { by: OrderBy::Moved, desc: false }, "默认按移入时间，新移入的在最后");
    let order = |by, desc| StatusPatch { order: Some(Order { by, desc }), ..Default::default() };
    ui(&mut s, |w| w.update_status(B, "done", order(OrderBy::Id, true))).unwrap();
    ui(&mut s, |w| w.update_status(B, "todo", order(OrderBy::Labels, true))).unwrap();
    assert_eq!(flow(&s).get("done").unwrap().order, Order { by: OrderBy::Id, desc: true });
    assert_eq!(flow(&s).get("todo").unwrap().order, Order { by: OrderBy::Labels, desc: false });
    assert_eq!(flow(&s).get("review").unwrap().order, Order::default(), "别的状态不变");
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
    let c = exec(&mut s, B, &human(), T0, Cmd::New { title: "有图".into(), body: Some(body), column: None, top: false, labels: vec![], from: None })
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

    // 24 小时没归属的记录被清理；删除卡片后附件跟着进回收站，永久删除后记录随之删除，不再被引用的文件被清理
    let orphan = s.attach(T0, &b, None, &human(), "o.txt", b"orphan").unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: c.card.id }).unwrap();
    assert!(s.attachment(log.id).unwrap().is_some(), "在回收站里，附件还在");
    exec(&mut s, B, &human(), T0, Cmd::Purge { id: c.card.id }).unwrap();
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

/// 上传附件走写事务（卡片 #380）：记一条就把 rev 加一；卡片不存在时 rev 不变、不留记录也不写文件
#[test]
fn attaching_bumps_rev_and_a_missing_card_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let id = exec(&mut s, B, &human(), T0, new("要传附件")).unwrap().card.id;
    let count = |s: &Store| s.conn().query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get::<_, i64>(0)).unwrap();
    let files = || std::fs::read_dir(dir.path().join("files")).map(|d| d.count()).unwrap_or(0);

    let r0 = s.rev().unwrap();
    s.attach(T0, &b, Some(id), &alpha(), "run.log", b"ok\n").unwrap();
    assert_eq!(s.rev().unwrap(), r0 + 1);
    // 先不归到卡片的（新建弹窗里先传）也算有变化
    s.attach(T0, &b, None, &human(), "a.png", PNG).unwrap();
    assert_eq!(s.rev().unwrap(), r0 + 2);
    // 同样的内容再传：各记一条，文件只存一份
    s.attach(T0, &b, Some(id), &alpha(), "again.log", b"ok\n").unwrap();
    assert_eq!((s.rev().unwrap(), count(&s), files()), (r0 + 3, 3, 2));

    // 卡片不存在、已删除：NOT_FOUND，rev、记录、文件都不变
    let gone = exec(&mut s, B, &human(), T0, new("要删的")).unwrap().card.id;
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: gone }).unwrap();
    let r1 = s.rev().unwrap();
    for card in [99, gone] {
        let e = s.attach(T0, &b, Some(card), &alpha(), "new.txt", b"new content").unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
    }
    assert_eq!((s.rev().unwrap(), count(&s), files()), (r1, 3, 2));
    // 定位白板失败（调用方的检查不过）：同样什么都不留
    let e = s.attach_with(T0, Origin::cli(), Some(id), &alpha(), "x.txt", b"x", |_| Err(BoardError::code(ErrorCode::NameOffline))).unwrap_err();
    assert_eq!(e.code, ErrorCode::NameOffline);
    assert_eq!((s.rev().unwrap(), count(&s), files()), (r1, 3, 2));
}

/// 把附件目录里所有文件的修改时间调早：清理只删超过 1 小时没改过的
fn age_files(dir: &std::path::Path) {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    for e in std::fs::read_dir(dir).unwrap() {
        std::fs::File::options().write(true).open(e.unwrap().path()).unwrap().set_modified(old).unwrap();
    }
}

/// 上传分两层（卡片 #426）：先落盘再进写事务。落盘之后卡片被删了：写事务回滚，不记、rev 不变，
/// 只留下没有记录的文件，预约释放后由清理收走
#[test]
fn upload_record_after_card_is_gone_leaves_only_an_orphan_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let id = exec(&mut s, B, &human(), T0, new("要传附件")).unwrap().card.id;
    let files = s.file_store().unwrap();
    let count = |s: &Store| s.conn().query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get::<_, i64>(0)).unwrap();

    let board = |r: &R| r.resolve(B);
    assert_eq!(s.attach_check(Some(id), board).unwrap().id, b.id);
    let put = files.put(b"late\n").unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Delete { id }).unwrap();
    let r0 = s.rev().unwrap();
    let stored = files::Stored { name: "late.log".into(), mime: "text/plain; charset=utf-8", size: 5, sha: put.sha().to_string() };
    let e = s.attach_record(T0, Origin::cli(), Some(id), &alpha(), &stored, |w| w.resolve(B)).unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!((s.rev().unwrap(), count(&s)), (r0, 0));
    let path = files.path(put.sha());
    assert_eq!(std::fs::read(&path).unwrap(), b"late\n", "文件已经落盘，留着");

    // 预约还在：再旧也不删；释放之后才被清理
    age_files(files.dir());
    assert_eq!(s.gc_files(T0).unwrap(), (0, 0));
    assert!(path.exists());
    drop(put);
    assert_eq!(s.gc_files(T0).unwrap(), (0, 1));
    assert!(!path.exists());

    // 检查一步就不过（卡片已删）：不落盘
    assert_eq!(s.attach_check(Some(id), board).unwrap_err().code, ErrorCode::NotFound);
}

/// 清理的记录删除走写事务（卡片 #426）：删了记录 rev 加一，没删就不变
#[test]
fn gc_deletes_records_in_a_write_and_bumps_rev() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let a = s.attach(T0, &b, None, &human(), "o.txt", b"orphan").unwrap();

    let r0 = s.rev().unwrap();
    let (rows, used) = s.gc_records(T0 + 3600 * 1000).unwrap();
    assert_eq!((rows, s.rev().unwrap()), (0, r0), "还没过 24 小时：不删、rev 不变");
    assert_eq!(used.len(), 1);
    let (rows, used) = s.gc_records(T0 + 25 * 3600 * 1000).unwrap();
    assert_eq!((rows, s.rev().unwrap()), (1, r0 + 1));
    assert!(used.is_empty());
    assert!(s.attachment(a.id).unwrap().is_none());
}

/// 删文件在 Store 锁外（卡片 #426）：清理拿的引用快照过时了（扫目录之后又有记录指向这个文件），
/// 删之前再查一次数据库，不删；正在上传（拿着预约）的也不删
#[test]
fn gc_rechecks_references_and_skips_reserved_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let files = s.file_store().unwrap();
    let a = s.attach(T0, &b, None, &human(), "a.png", PNG).unwrap();
    let path = s.attachment(a.id).unwrap().unwrap().2;
    let png_sha = path.file_name().unwrap().to_string_lossy().to_string();
    age_files(files.dir());

    // 快照里还没有它（空集合），数据库里有：不删
    let stale = std::collections::HashSet::new();
    assert_eq!(files.collect(&stale, |c| s.files_in_use(c)).unwrap(), 0);
    assert!(path.exists());

    // 正在上传：快照、数据库里都没有，但有预约，不删，而且不去查数据库
    let put = files.put(b"uploading").unwrap();
    age_files(files.dir());
    let asked = std::cell::Cell::new(false);
    let n = files
        .collect(&[png_sha.clone()].into_iter().collect(), |c| {
            asked.set(true);
            s.files_in_use(c)
        })
        .unwrap();
    assert_eq!((n, asked.get()), (0, false));
    assert!(files.path(put.sha()).exists());
    drop(put);
    assert_eq!(files.collect(&[png_sha.clone()].into_iter().collect(), |c| s.files_in_use(c)).unwrap(), 1);
    assert!(path.exists(), "有记录的还在");
}

/// 清理正在删一个文件时，同样内容的上传等它删完再落盘：上传完文件在，不会被删掉
#[test]
fn put_waits_while_gc_is_deleting_the_same_content() {
    let dir = tempfile::tempdir().unwrap();
    let files = std::sync::Arc::new(files::FileStore::new(dir.path().join("files")));
    let sha = files.put(b"again").unwrap().sha().to_string();
    age_files(files.dir());
    let mut upload = None;
    let n = files
        .collect(&Default::default(), |c| {
            assert_eq!(c, std::slice::from_ref(&sha));
            let f = files.clone();
            let h = std::thread::spawn(move || {
                let put = f.put(b"again").unwrap();
                f.path(put.sha()).exists()
            });
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(!h.is_finished(), "上传应该等清理删完");
            upload = Some(h);
            Ok(Default::default())
        })
        .unwrap();
    assert_eq!(n, 1);
    assert!(upload.unwrap().join().unwrap(), "删完之后重新落盘");
    assert!(files.path(&sha).exists());
}

/// 同样的内容同时上传（不再被 Store 锁排成一个一个）：各写各的临时文件，都成功，文件内容完整，预约都释放
#[test]
fn concurrent_puts_of_the_same_content() {
    let dir = tempfile::tempdir().unwrap();
    let files = std::sync::Arc::new(files::FileStore::new(dir.path().join("files")));
    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let shas: Vec<String> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let (files, body) = (&files, &body);
                sc.spawn(move || {
                    let put = files.put(body).unwrap();
                    put.sha().to_string()
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(shas.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(std::fs::read(files.path(&shas[0])).unwrap(), body);
    assert_eq!(std::fs::read_dir(files.dir()).unwrap().count(), 1, "临时文件都改名或清掉了");
    // 预约都释放了：旧了就能清掉
    age_files(files.dir());
    assert_eq!(files.collect(&Default::default(), |_| Ok(Default::default())).unwrap(), 1);
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

/// 标签颜色和项目一样：色板里的，或取色器选的任意 #RRGGBB（卡片 default/main#150）
#[test]
fn label_color_palette_or_any_hex() {
    let mut s = store();
    let a = ui(&mut s, |w| w.create_label("default", "甲", Some("#12ab9f"), false)).unwrap();
    assert_eq!(a.color, "#12AB9F");
    let b = ui(&mut s, |w| w.create_label("default", "乙", Some("#3b82f6"), false)).unwrap();
    assert_eq!(b.color, "#3B82F6", "色板里的颜色照常");
    assert_eq!(ui(&mut s, |w| w.create_label("default", "丙", Some("teal"), false)).expect_err("不是颜色").code, ErrorCode::Usage);
    assert_eq!(ui(&mut s, |w| w.create_label("default", "丙", Some("#12ab9"), false)).expect_err("位数不对").code, ErrorCode::Usage);
    ui(&mut s, |w| w.update_label("default", a.id, None, Some("#00ff7f"), None, None)).unwrap();
    let ls = s.read(|r| r.labels("default")).unwrap();
    assert_eq!(ls.iter().find(|l| l.id == a.id).unwrap().color, "#00FF7F");
}

/* ---------------- 白板说明（PRD §5） ---------------- */

/// 白板说明：保存、清空；说明里引用的附件不会被 24 小时清理
#[test]
fn readme_is_saved_and_keeps_its_images() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let png = b"\x89PNG\r\n\x1a\nxxxx";
    let att = s.attach(T0, &b, None, &Actor::human(), "a.png", png).unwrap();
    let other = s.attach(T0, &b, None, &Actor::human(), "b.png", b"\x89PNG\r\n\x1a\nyyyy").unwrap();
    let text = format!("# 说明\n\n![a]({})", crate::files::url_of(att.id));
    s.write(T0, Origin::ui(None), |w| w.update_board(B, None, None, Some(&text))).unwrap();
    let ws = s.read(|r| r.workspace(T0)).unwrap();
    assert_eq!(ws.projects[0].boards[0].readme, text);

    // 卡片里引用说明图不会把它「抢」走；永久删除卡片后说明图还在
    let c = exec(&mut s, B, &human(), T0, new("引用说明图")).unwrap().card.id;
    exec(&mut s, B, &human(), T0, Cmd::Comment { id: c, text: format!("再看一遍 {}", crate::files::url_of(att.id)) }).unwrap();
    assert_eq!(s.attachment(att.id).unwrap().unwrap().0.card_id, None, "说明里的图不归到卡片");
    assert!(card(&s, B, c).attachments.is_empty());
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: c }).unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Purge { id: c }).unwrap();
    let (_, _, path) = s.attachment(att.id).unwrap().expect("永久删除卡片后说明图还在");
    assert!(path.exists());

    s.gc_files(T0 + 2 * DAY).unwrap();
    assert!(s.attachment(att.id).unwrap().is_some(), "说明引用的图片留着");
    assert!(s.attachment(other.id).unwrap().is_none(), "没被引用的照常清理");
    s.write(T0, Origin::ui(None), |w| w.update_board(B, None, None, Some(""))).unwrap();
    assert_eq!(s.read(|r| r.workspace(T0)).unwrap().projects[0].boards[0].readme, "");
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

#[test]
fn event_and_activity_actors_include_agent_id_even_offline() {
    let mut s = store();
    let name = "win2/group-4.1fzzz-max";
    s.write(T0, Origin::cli(), |w| w.agent_check_with_agent(Some("codex"), name, Some(1), NameOwner::Anyone)).unwrap();
    let c = exec(&mut s, B, &Actor::agent(name), T0 + 1, new("一")).unwrap().card;
    s.write(T0 + 2 * SEC, Origin::cli(), |w| w.agent_beat(name, true)).unwrap();
    assert!(s.read(|r| r.require_online(name, T0 + 3 * SEC)).is_err(), "用例要在名字离线后读");

    let b = s.read(|r| r.resolve(B)).unwrap();
    let event = s.read(|r| r.events_page(&b, None, 100)).unwrap().into_iter().find(|e| e.actor.name == name).unwrap();
    assert_eq!(event.actor.agent.as_deref(), Some("codex"));
    let activity = card(&s, B, c.id).activity.into_iter().find(|a| a.actor.name == name).unwrap();
    assert_eq!(activity.actor.agent.as_deref(), Some("codex"));
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

/// 终点状态的快照默认只带最近移入的 FINAL_RECENT 张，省略的张数单独给出；full 时是全部；别的状态不受影响
#[test]
fn final_snapshot_keeps_only_recent_cards() {
    let mut s = store();
    let n = FINAL_RECENT as i64 + 10;
    for i in 1..=n {
        exec(&mut s, B, &human(), T0 + i, new(&format!("完成 {i}"))).unwrap();
        exec(&mut s, B, &human(), T0 + i, mv(i, "done")).unwrap();
    }
    for i in 1..=3 {
        exec(&mut s, B, &human(), T0 + n + i, new(&format!("待办 {i}"))).unwrap();
    }
    let part = s
        .read(|r| {
            let b = r.resolve(B)?;
            r.board_snapshot(&b, T0, false)
        })
        .unwrap();
    // 最近移入的在最下面：留下 11..=n（按最后一次移入挑，不看 pos）
    assert_eq!(part.cols["done"], (11..=n).collect::<Vec<_>>());
    assert_eq!(part.omitted.get("done"), Some(&10));
    assert!(!part.cards.contains_key(&1) && !part.cards.contains_key(&10));
    assert!(part.cards.contains_key(&11));
    assert_eq!(part.cols["todo"], vec![n + 1, n + 2, n + 3]);
    assert_eq!(part.omitted.len(), 1);

    let all = snapshot(&s, B, T0);
    assert_eq!(all.cols["done"], (1..=n).collect::<Vec<_>>());
    assert!(all.omitted.is_empty());
    assert_eq!(all.cards.len() as i64, n + 3);
}

/// 拆出的卡都被终点状态的省略挡在快照外时，快照照样告诉界面原条目拆过（split_omitted），界面才不会把它又列回未整理；
/// 进了回收站的拆出卡不算；full 时没有省略，这一项为空（default/main#534）
#[test]
fn final_snapshot_reports_split_kids_it_omitted() {
    let mut s = store();
    let inbox = |title: &str, from: Option<i64>| Cmd::New { title: title.into(), body: None, column: Some(INBOX.into()), top: false, labels: vec![], from };
    let a = exec(&mut s, B, &human(), T0, inbox("原条目", None)).unwrap().card.id;
    let x = exec(&mut s, B, &human(), T0 + 1, inbox("拆出的一", Some(a))).unwrap().card.id;
    let y = exec(&mut s, B, &human(), T0 + 1, inbox("拆出的二", Some(a))).unwrap().card.id;
    let z = exec(&mut s, B, &human(), T0 + 1, inbox("拆出的三", Some(a))).unwrap().card.id;
    for id in [x, y, z] {
        exec(&mut s, B, &human(), T0 + 2, mv(id, "todo")).unwrap();
        exec(&mut s, B, &human(), T0 + 2, mv(id, "done")).unwrap();
    }
    exec(&mut s, B, &human(), T0 + 3, Cmd::Delete { id: z }).unwrap();
    let part = |s: &Store| s.read(|r| { let b = r.resolve(B)?; r.board_snapshot(&b, T0, false) }).unwrap();
    assert!(part(&s).split_omitted.is_empty(), "拆出的卡还在快照里：不用另报");
    // 后来完成的卡把拆出的两张挤出快照
    for i in 0..FINAL_RECENT as i64 {
        let id = exec(&mut s, B, &human(), T0 + 10 + i, new(&format!("完成 {i}"))).unwrap().card.id;
        exec(&mut s, B, &human(), T0 + 10 + i, mv(id, "done")).unwrap();
    }
    let snap = part(&s);
    assert!(!snap.cards.contains_key(&x) && !snap.cards.contains_key(&y));
    assert_eq!(snap.cols[INBOX], vec![a], "原条目还在未整理");
    assert_eq!(snap.split_omitted.get(&a), Some(&2), "回收站里的 #{z} 不算");
    assert_eq!(snap.split_omitted.len(), 1);
    assert!(snapshot(&s, B, T0).split_omitted.is_empty(), "全量没有省略");
}

/// 「最近」按最后一次移入算，不按 pos：人把新完成的放在最上面时，留下的是最上面这些（不是最下面的老卡片）；
/// 留下的卡片按 pos 排，省略的可以夹在中间。拖进这种状态时按看到的相邻卡片（before / after）定位
#[test]
fn final_snapshot_keeps_recently_moved_wherever_they_are() {
    let mut s = store();
    let n = FINAL_RECENT as i64 + 10;
    for i in 1..=n {
        exec(&mut s, B, &human(), T0 + i, new(&format!("完成 {i}"))).unwrap();
        exec(&mut s, B, &human(), T0 + i, Cmd::Move { id: i, column: "done".into(), top: true, index: None, order: None, before: None, after: None }).unwrap();
    }
    // 老卡片 #1 后来又被移出、再移回来，放在最下面：它也算最近的
    exec(&mut s, B, &human(), T0 + n + 1, mv(1, "review")).unwrap();
    exec(&mut s, B, &human(), T0 + n + 2, mv(1, "done")).unwrap();
    let snap = |s: &Store| s.read(|r| { let b = r.resolve(B)?; r.board_snapshot(&b, T0, false) }).unwrap();
    let part = snap(&s);
    let mut want: Vec<i64> = (12..=n).rev().collect();
    want.push(1);
    assert_eq!(part.cols["done"], want);
    assert_eq!(part.omitted.get("done"), Some(&10));
    assert!(!part.cards.contains_key(&2) && !part.cards.contains_key(&11));

    // 看到的 #12 和 #1 之间夹着省略的 #11..#2：放到 #1 前面 = 整列里 #2 后面
    exec(&mut s, B, &human(), T0 + n + 3, new("新卡片")).unwrap();
    let id = n + 1;
    exec(&mut s, B, &human(), T0 + n + 4, Cmd::Move { id, column: "done".into(), top: false, index: Some(0), order: None, before: Some(1), after: None }).unwrap();
    let all = snapshot(&s, B, T0);
    let col = &all.cols["done"];
    assert_eq!(col[col.len() - 2..], [id, 1]);
    // 放到 #12 后面
    exec(&mut s, B, &human(), T0 + n + 5, Cmd::Move { id, column: "done".into(), top: false, index: Some(0), order: None, before: None, after: Some(12) }).unwrap();
    let all = snapshot(&s, B, T0);
    let col = &all.cols["done"];
    let at = col.iter().position(|x| *x == 12).unwrap();
    assert_eq!(col[at + 1], id);
    // 锚点不在目标状态里：按 index
    exec(&mut s, B, &human(), T0 + n + 6, Cmd::Move { id, column: "done".into(), top: false, index: Some(0), order: None, before: Some(9999), after: None }).unwrap();
    assert_eq!(snapshot(&s, B, T0).cols["done"][0], id);
}

/// 放到最下面只改这一张的 pos，不重写整列（终点状态越积越多，重写整列越来越慢）
#[test]
fn place_at_bottom_updates_only_that_card() {
    let mut s = store();
    s.write(T0, Origin::ui(None), |w| {
        let b = w.resolve(B)?;
        for i in 1..=1001 {
            w.exec(&b, &human(), new(&format!("卡片 {i}")))?;
        }
        Ok(())
    })
    .unwrap();
    // 前 1000 张直接放进「待审核」，pos 稀疏：被重写的话会变成 0..999
    s.conn().execute("UPDATE cards SET col = 'review', pos = id * 10 WHERE id <= 1000", []).unwrap();
    exec(&mut s, B, &human(), T0 + 2000, mv(1001, "review")).unwrap();
    let pos: Vec<(i64, i64)> = {
        let mut st = s.conn().prepare("SELECT id, pos FROM cards WHERE col = 'review' ORDER BY pos, id").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|x| x.unwrap()).collect()
    };
    assert_eq!(pos.len(), 1001);
    for (id, p) in &pos[..1000] {
        assert_eq!(*p, id * 10, "#{id} 的 pos 被改了");
    }
    assert_eq!(pos[1000], (1001, 10_001));
    assert_eq!(snapshot(&s, B, T0).cols["review"].last(), Some(&1001));
    // 再放一张到指定位置：按 pos 顺序插入，空位不影响
    exec(&mut s, B, &human(), T0 + 2001, new("插队")).unwrap();
    exec(&mut s, B, &human(), T0 + 2002, Cmd::Move { id: 1002, column: "review".into(), top: false, index: Some(1), order: None, before: None, after: None }).unwrap();
    assert_eq!(&snapshot(&s, B, T0).cols["review"][..3], &[1, 1002, 2]);
}

/// 「未整理」页的合并：只留目标条目，描述接上来源、标签取并集，附件和评论跟过来，来源删除；只有人能合并、只能合并「未整理」里的
#[test]
fn inbox_merge() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let bug = s.write(T0, Origin::ui(None), |w| w.create_label("default", "bug", None, false)).unwrap().id;
    let doc = s.write(T0, Origin::ui(None), |w| w.create_label("default", "文档", None, false)).unwrap().id;
    let inbox = |title: &str, body: &str, labels: Vec<i64>| Cmd::New {
        title: title.into(),
        body: Some(body.into()),
        column: Some("inbox".into()),
        top: false,
        labels,
        from: None,
    };
    let merge = |id: i64, from: Vec<i64>| Cmd::Merge { id, from, title: None, body: None };

    exec(&mut s, B, &human(), T0, inbox("掉线后自动重派", "想法 A", vec![bug])).unwrap();
    exec(&mut s, B, &alpha(), T0, inbox("掉线的卡片别丢进人工校验", "想法 B", vec![doc])).unwrap();
    exec(&mut s, B, &human(), T0, inbox("另一条", "", vec![bug])).unwrap();
    exec(&mut s, B, &human(), T0, new("待办里的")).unwrap();
    let png = s.attach(T0, &b, Some(2), &alpha(), "shot.png", PNG).unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Comment { id: 2, text: "补充一句".into() }).unwrap();

    // 只有人能合并；至少一条别的；都要在「未整理」里；标题不能为空
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, merge(1, vec![2]))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, merge(1, vec![]))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, merge(1, vec![1]))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, merge(1, vec![4]))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, merge(1, vec![99]))), ErrorCode::NotFound);
    let blank = Cmd::Merge { id: 1, from: vec![2], title: Some("  ".into()), body: None };
    assert_eq!(code(exec(&mut s, B, &human(), T0, blank)), ErrorCode::EmptyTitle);
    assert_eq!(snapshot(&s, B, T0).cols["inbox"], vec![1, 2, 3], "被拒绝时什么都不变");

    // 不给描述：目标原文在前，每条来源一节
    let d = exec(&mut s, B, &human(), T0 + MIN, merge(1, vec![2, 3, 2])).unwrap();
    assert_eq!(d.card.title, "掉线后自动重派");
    assert_eq!(
        d.card.body,
        "想法 A\n\n## 掉线的卡片别丢进人工校验\n\n想法 B\n\n> 合并自 #2 · alpha\n\n## 另一条\n\n> 合并自 #3"
    );
    let mut labels = d.card.labels.clone();
    labels.sort();
    assert_eq!(labels, vec![bug, doc]);
    assert_eq!(d.attachments.iter().map(|a| a.id).collect::<Vec<_>>(), [png.id], "来源的附件归到留下的条目");
    assert!(d.activity.iter().any(|a| a.kind == "comment" && a.text == "补充一句"), "来源的评论跟过来");
    let m = d.activity.iter().find(|a| a.action == "merge").expect("留下合并的动态");
    assert_eq!(m.text, "合并了 #2「掉线的卡片别丢进人工校验」、#3「另一条」");
    assert_eq!(snapshot(&s, B, T0).cols["inbox"], vec![1]);
    assert_eq!(s.read(|r| r.card(&b, 2)).unwrap_err().code, ErrorCode::NotFound);
    assert!(s.read(|r| r.trash(&b)).unwrap().is_empty(), "合并掉的条目不进回收站");

    // 给了标题和描述：按给的写
    exec(&mut s, B, &human(), T0, inbox("五", "", vec![])).unwrap();
    exec(&mut s, B, &human(), T0, inbox("六", "", vec![])).unwrap();
    let d = exec(&mut s, B, &human(), T0, Cmd::Merge { id: 6, from: vec![5], title: Some(" 新标题 ".into()), body: Some("新描述\n".into()) }).unwrap();
    assert_eq!((d.card.title.as_str(), d.card.body.as_str()), ("新标题", "新描述"));
    assert!(d.activity.iter().any(|a| a.action == "merge" && a.text == "合并了 #5「五」，标题改为「新标题」"));
    assert_eq!(snapshot(&s, B, T0).cols["inbox"], vec![1, 6]);
}

/* ------------------------------ 复制为新卡片（PRD §10.8） ------------------------------ */

/// 复制为新卡片：新卡干净（不带动态、领取、耗时），描述里引用的附件复制一份、只在评论里的不复制，两边互相记下；
/// 原卡可以一起移走或进回收站，任何一步被拒绝都不留下；只有人能复制
#[test]
fn copy_card() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let bug = s.write(T0, Origin::ui(None), |w| w.create_label("default", "bug", None, false)).unwrap().id;
    exec(&mut s, B, &human(), T0, new("修安装脚本")).unwrap();
    let shot = s.attach(T0, &b, Some(1), &human(), "shot.png", PNG).unwrap();
    let log = s.attach(T0, &b, Some(1), &alpha(), "run.log", b"boom\n").unwrap();
    let body = format!("要做的\n\n![图]({0})\n\n再看一次 ![图]({0})", shot.url);
    exec(&mut s, B, &human(), T0, Cmd::Edit { id: 1, title: None, body: Some(body.clone()) }).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(Some(1))).unwrap();
    exec(&mut s, B, &alpha(), T0, Cmd::Comment { id: 1, text: format!("报错了 [日志]({})", log.url) }).unwrap();
    let copy = |body: &str, files: bool, move_to: Option<&str>, trash: bool| Cmd::Copy {
        id: 1,
        title: " 修安装脚本（重来） ".into(),
        body: Some(body.into()),
        column: Some("inbox".into()),
        labels: vec![bug],
        files,
        move_to: move_to.map(Into::into),
        trash,
    };

    // 只有人能复制；原卡只能二选一；原卡还被持有时不能一起移走，整个复制不留下
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, copy(&body, true, None, false))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, copy(&body, true, Some("done"), true))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, copy(&body, true, Some("done"), false))), ErrorCode::NotHolder);
    assert!(snapshot(&s, B, T0).cols["inbox"].is_empty(), "被拒绝时不建卡");
    assert_eq!(card(&s, B, 1).attachments.len(), 2);

    // 被持有的卡也能复制（原卡不动）：新卡干净，描述里的附件复制一份（引用两次也只复制一份），评论里的日志不复制
    let d = exec(&mut s, B, &human(), T0, copy(&body, true, None, false)).unwrap();
    let id = d.card.id;
    assert_eq!((d.card.title.as_str(), d.card.column.as_str(), d.card.labels.clone()), ("修安装脚本（重来）", "inbox", vec![bug]));
    assert_eq!((d.card.holder.as_deref(), d.card.runs, d.card.copied_from), (None, 0, Some(1)));
    assert_eq!(d.activity.iter().map(|a| a.text.as_str()).collect::<Vec<_>>(), ["从 #1 复制过来"], "不带原卡的动态");
    assert_eq!(d.attachments.len(), 1);
    let a = &d.attachments[0];
    assert_ne!(a.id, shot.id);
    assert_eq!((a.name.as_str(), a.created_by.name.as_str()), ("shot.png", shot.created_by.name.as_str()), "上传者不变");
    assert_eq!(d.card.body, format!("要做的\n\n![图]({0})\n\n再看一次 ![图]({0})", a.url), "引用换成新的");
    let src = card(&s, B, 1);
    assert_eq!((src.card.holder.as_deref(), src.copied_into.clone()), (Some("alpha"), vec![id]));
    assert!(src.activity.iter().any(|x| x.action == "copy" && x.text == format!("复制为 #{id}")));
    assert_eq!(snapshot(&s, B, T0).cards[&id].copied_from, Some(1));

    // 不复制附件：引用原样留着
    let d = exec(&mut s, B, &human(), T0, copy(&body, false, None, false)).unwrap();
    assert!(d.attachments.is_empty());
    assert_eq!(d.card.body, body);

    // 原卡一起移到终点 / 移进回收站（领取已经因为没有心跳释放了）
    let d = exec(&mut s, B, &human(), T0 + 3 * MIN, copy(&body, true, Some("done"), false)).unwrap();
    assert_eq!(card(&s, B, 1).card.column, "done");
    assert_eq!(d.card.column, "inbox");
    let mut c = copy(&body, true, None, true);
    if let Cmd::Copy { column, .. } = &mut c {
        *column = None;
    }
    let d = exec(&mut s, B, &human(), T0 + 4 * MIN, c).unwrap();
    assert_eq!(d.card.column, "todo", "不选「未整理」时放进首个可领取状态");
    assert!(s.read(|r| r.in_trash(&b, 1)).unwrap());
    assert_eq!(code(exec(&mut s, B, &human(), T0 + 5 * MIN, copy(&body, true, None, false))), ErrorCode::InTrash);
    let mut c = copy(&body, true, None, false);
    if let Cmd::Copy { id, .. } = &mut c {
        *id = 99;
    }
    assert_eq!(code(exec(&mut s, B, &human(), T0, c)), ErrorCode::NotFound);
}

/* ------------------------------ 未整理：新条目（PRD §10.5） ------------------------------ */

fn inbox_new(title: &str) -> Cmd {
    Cmd::New { title: title.into(), body: None, column: Some("inbox".into()), top: false, labels: vec![], from: None }
}
fn seen(id: Option<i64>) -> Cmd {
    Cmd::Seen { id }
}

/// 「新」：在「未整理」里、agent 记下、人还没看过
fn is_new(c: &Card) -> bool {
    c.column == INBOX && c.created_by.kind == ActorKind::Agent && c.seen_at.is_none()
}

#[test]
fn inbox_agent_entries_are_new_until_seen() {
    let mut s = store();
    let a = exec(&mut s, B, &alpha(), T0, inbox_new("agent 记的")).unwrap();
    assert!(is_new(&a.card), "agent 记下、人还没看过的算新");
    let h = exec(&mut s, B, &human(), T0, inbox_new("人自己记的")).unwrap();
    assert!(!is_new(&h.card), "人自己记的不算新");
    assert!(h.card.seen_at.is_some());

    // 拖进工作流、改名都不再是「新」；快照里带着看过的状态
    let snap = snapshot(&s, B, T0);
    assert!(is_new(&snap.cards[&a.card.id]));
    assert!(!is_new(&snap.cards[&h.card.id]));
    let b = exec(&mut s, B, &alpha(), T0, inbox_new("agent 又记的")).unwrap();
    exec(&mut s, B, &human(), T0, mv(b.card.id, "todo")).unwrap();
    assert!(!is_new(&snapshot(&s, B, T0).cards[&b.card.id]), "整理过的看过");
}

#[test]
fn inbox_seen_one_and_all() {
    let mut s = store();
    let ids: Vec<i64> = (1..=3).map(|i| exec(&mut s, B, &alpha(), T0, inbox_new(&format!("第 {i} 条"))).unwrap().card.id).collect();
    let mine = exec(&mut s, B, &human(), T0, inbox_new("人记的")).unwrap().card.id;
    let beyond = exec(&mut s, B, &human(), T0, new("工作流里的")).unwrap().card.id;

    // 只有人能标；只能在「未整理」里标；卡片要存在
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, seen(Some(ids[0])))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, seen(Some(beyond)))), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, seen(Some(99)))), ErrorCode::NotFound);

    // 展开一条：只有它不再是「新」，记下时间；不新增动态
    let d = exec(&mut s, B, &human(), T0 + MIN, seen(Some(ids[1]))).unwrap();
    assert_eq!(d.card.seen_at, Some(T0 + MIN));
    assert!(d.activity.iter().all(|x| x.action != "seen"), "「看过」不记动态");
    let snap = snapshot(&s, B, T0 + MIN);
    assert!(is_new(&snap.cards[&ids[0]]));
    assert!(!is_new(&snap.cards[&ids[1]]));

    // 已经看过的不刷新时间（两个窗口都在看时结果一样）
    let again = exec(&mut s, B, &human(), T0 + 5 * MIN, seen(Some(ids[1]))).unwrap();
    assert_eq!(again.card.seen_at, Some(T0 + MIN));

    // 全部标为已读：「未整理」里的都算看过；人自己记的、工作流里的不动
    exec(&mut s, B, &human(), T0 + 2 * MIN, seen(None)).unwrap();
    let snap = snapshot(&s, B, T0 + 2 * MIN);
    assert_eq!(snap.cards[&ids[0]].seen_at, Some(T0 + 2 * MIN));
    assert_eq!(snap.cards[&ids[1]].seen_at, Some(T0 + MIN), "早就看过的时间不刷新");
    for id in &ids {
        assert!(!is_new(&snap.cards[id]), "#{id} 全部标为已读后不再新");
    }
    assert_eq!(snap.cards[&mine].seen_at, Some(T0), "人自己记的时间不变");
    assert_eq!(snap.cards[&beyond].seen_at, None, "工作流里的卡片不记这个");
}

#[test]
fn inbox_edit_merge_mark_seen() {
    let mut s = store();
    let a = exec(&mut s, B, &alpha(), T0, inbox_new("想法 A")).unwrap().card.id;
    let b = exec(&mut s, B, &alpha(), T0, inbox_new("想法 B")).unwrap().card.id;
    let c = exec(&mut s, B, &alpha(), T0, inbox_new("想法 C")).unwrap().card.id;

    // agent 编辑不算人看过
    exec(&mut s, B, &alpha(), T0, Cmd::Edit { id: a, title: Some("想法 A2".into()), body: None }).unwrap();
    assert!(is_new(&snapshot(&s, B, T0).cards[&a]), "agent 改的不算看过");
    // 人在界面上改过就算看过
    exec(&mut s, B, &human(), T0 + MIN, Cmd::Edit { id: a, title: Some("想法 A3".into()), body: None }).unwrap();
    assert!(!is_new(&snapshot(&s, B, T0 + MIN).cards[&a]));

    // 看过的那条并进没看过的那条：留下的也算看过（并进来的已经没有点了）
    exec(&mut s, B, &human(), T0 + 2 * MIN, seen(Some(b))).unwrap();
    exec(&mut s, B, &human(), T0 + 3 * MIN, Cmd::Merge { id: c, from: vec![b], title: None, body: None }).unwrap();
    let snap = snapshot(&s, B, T0 + 3 * MIN);
    assert!(!is_new(&snap.cards[&c]), "并进来的条目看过，留下的这条也看过");

    // 都没看过的合并：留下的也不算看过
    let d = exec(&mut s, B, &alpha(), T0, inbox_new("D")).unwrap().card.id;
    let e = exec(&mut s, B, &alpha(), T0, inbox_new("E")).unwrap().card.id;
    exec(&mut s, B, &human(), T0 + 4 * MIN, Cmd::Merge { id: d, from: vec![e], title: None, body: None }).unwrap();
    assert!(is_new(&snapshot(&s, B, T0 + 4 * MIN).cards[&d]));
}

/* ------------------------------ 回收站（PRD §8 规则 10、§10.3） ------------------------------ */

const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;

/// 各模块的建表语句（SCHEMA）在打开数据库时最先执行，那时旧库还没补上迁移新增的列：
/// SCHEMA 里的索引只要用到「ALTER TABLE … ADD COLUMN」补的列，打开旧库就报 no such column，服务端起不来。
/// 这类索引要放到补完列之后建
#[test]
fn schema_indexes_do_not_use_migrated_columns() {
    let sources = [
        ("store.rs", include_str!("store.rs")),
        ("files.rs", include_str!("files.rs")),
        ("agents.rs", include_str!("agents.rs")),
        ("hosts/mod.rs", include_str!("hosts/mod.rs")),
        ("moves.rs", include_str!("moves.rs")),
        ("skills.rs", include_str!("skills.rs")),
        ("mcp.rs", include_str!("mcp.rs")),
    ];
    let word = |s: &str| s.trim().split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("").to_string();
    // 迁移补的列：(表, 列)
    let mut added = std::collections::HashSet::new();
    for (_, text) in sources {
        for part in text.split("ALTER TABLE ").skip(1) {
            let table = word(part);
            if let Some(rest) = part.trim_start_matches(&table).trim_start().strip_prefix("ADD COLUMN ") {
                added.insert((table, word(rest)));
            }
        }
    }
    // 逐列补的写法：add_cols(conn, "<表>", &[("<列>", "<定义>"), …])
    for (_, text) in sources.iter().chain([("sched.rs", include_str!("sched.rs"))].iter()) {
        for part in text.split("add_cols(").skip(1) {
            let Some(table) = part.split('"').nth(1) else { continue };
            let list = part.split(")]").next().unwrap_or("");
            for item in list.split("(\"").skip(1) {
                added.insert((table.to_string(), word(item)));
            }
        }
    }
    assert!(added.contains(&("cards".to_string(), "deleted_at".to_string())), "应该认出迁移补的列：{added:?}");
    let mut checked = 0;
    for (name, text) in sources {
        for block in text.split("SCHEMA: &str = r#\"").skip(1) {
            let schema = block.split("\"#;").next().unwrap();
            for stmt in schema.split("INDEX IF NOT EXISTS ").skip(1) {
                // <索引名> ON <表> (<列>, …)
                let (head, cols) = stmt.split_once('(').unwrap();
                let table = word(head.split(" ON ").nth(1).unwrap());
                for col in cols.split(')').next().unwrap().split(',') {
                    let col = word(col);
                    assert!(
                        !added.contains(&(table.clone(), col.clone())),
                        "{name} 的 SCHEMA 里有索引用到迁移才补的列 {table}.{col}：{}",
                        stmt.lines().next().unwrap()
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 10, "应该检查到各个 SCHEMA 里的索引（只检查了 {checked} 列）");
}

/// v0.1.19 及更早的数据库：cards 没有回收站的列，也没有 cards_trash 索引。
/// 建表语句里的索引不能先于迁移补列执行，否则打开旧库就报 no such column: deleted_at，服务端起不来（v0.1.20 部署失败）
#[test]
fn old_db_without_trash_columns_opens_and_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        exec(&mut s, B, &human(), T0, new("旧卡")).unwrap();
    }
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP INDEX cards_trash;
             ALTER TABLE cards DROP COLUMN deleted_at; ALTER TABLE cards DROP COLUMN deleted_by_kind;
             ALTER TABLE cards DROP COLUMN deleted_by_name; ALTER TABLE cards DROP COLUMN del_col;
             ALTER TABLE cards DROP COLUMN del_pos; ALTER TABLE cards DROP COLUMN del_from; ALTER TABLE cards DROP COLUMN del_held;",
        )
        .unwrap();
    let mut s = Store::open(&path).expect("旧库要能打开");
    // 守住同一类问题：见 schema_indexes_do_not_use_migrated_columns
    let has_index: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'cards_trash'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(has_index, 1, "迁移后补上回收站的索引");
    // 旧卡不在回收站里，删除、恢复照常
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![1]);
    exec(&mut s, B, &human(), T0 + MIN, Cmd::Delete { id: 1 }).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    assert_eq!(s.read(|r| r.trash(&b)).unwrap().len(), 1);
}

/// 环境检查删除后，旧库里的那几列还在：服务端照样启动、读工作区，也不会再把它们发出去。
#[test]
fn old_db_with_ready_check_columns_opens_and_ignores_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-ready.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        exec(&mut s, B, &human(), T0, new("旧卡")).unwrap();
    }
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "ALTER TABLE projects ADD COLUMN ready_check TEXT NOT NULL DEFAULT '';
             ALTER TABLE projects ADD COLUMN ready_check_win TEXT NOT NULL DEFAULT '';
             ALTER TABLE boards ADD COLUMN ready_check TEXT NOT NULL DEFAULT '';
             ALTER TABLE boards ADD COLUMN ready_check_win TEXT NOT NULL DEFAULT '';
             UPDATE projects SET ready_check = 'exit 1';
             UPDATE boards SET ready_check = 'exit 1';",
        )
        .unwrap();
    let s = Store::open(&path).expect("带旧环境检查列的库要能打开");
    let ws = s.read(|r| r.workspace(T0)).unwrap();
    assert_eq!(ws.projects[0].boards.len(), 1);
    assert_eq!(ws.projects[0].name, "默认项目");
    assert!(!serde_json::to_string(&ws).unwrap().contains("readyCheck"));
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![1]);
}

#[test]
fn trash_delete_hides_restore_and_never_reuses_ids() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    for t in ["一", "二", "三"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    let png = s.attach(T0, &b, Some(2), &human(), "shot.png", PNG).unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Comment { id: 2, text: "看这里".into() }).unwrap();

    // 删除 #2：白板、快照、card 里都没有了，回收站里有它（谁删的、什么时候、原来在哪个状态）
    let d = exec(&mut s, B, &human(), T0 + MIN, Cmd::Delete { id: 2 }).unwrap();
    assert_eq!(d.card.id, 2);
    assert_eq!(snapshot(&s, B, T0 + MIN).cols["todo"], vec![1, 3]);
    assert!(s.read(|r| r.card(&b, 2)).unwrap_err().code == ErrorCode::NotFound);
    assert_eq!(s.read(|r| r.board_snapshot(&b, T0 + MIN, true)).unwrap().trash, 1);
    let t = s.read(|r| r.trash(&b)).unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].card.id, t[0].card.title.as_str(), t[0].col.as_str(), t[0].deleted_at), (2, "二", "todo", T0 + MIN));
    assert!(t[0].deleted_by.is_human());
    let act = card_activity(&s, B, 2).unwrap_err();
    assert_eq!(act.code, ErrorCode::NotFound, "删掉的卡片读不到动态");
    assert!(s.read(|r| r.trash_of(&b, 2)).unwrap().is_some());

    // 新建卡片不复用编号；恢复回到原来状态原来的位置，评论和附件都在
    let c4 = exec(&mut s, B, &human(), T0 + 2 * MIN, new("四")).unwrap().card;
    assert_eq!(c4.id, 4);
    let r = exec(&mut s, B, &human(), T0 + 3 * MIN, Cmd::Restore { id: 2 }).unwrap();
    assert_eq!(r.card.id, 2);
    assert!(r.activity.iter().any(|a| a.kind == "comment" && a.text == "看这里"));
    assert_eq!(r.attachments.iter().map(|a| a.id).collect::<Vec<_>>(), [png.id]);
    assert!(r.activity.iter().any(|a| a.action == "restore" && a.text == "从回收站恢复，放回「待办」"));
    let snap = snapshot(&s, B, T0 + 3 * MIN);
    assert_eq!(snap.cols["todo"], vec![1, 2, 3, 4]);
    assert_eq!(snap.trash, 0);
    assert!(s.read(|r| r.trash(&b)).unwrap().is_empty());

    // 只有人能删除、恢复、永久删除；不在回收站里的不能恢复、不能永久删除
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, Cmd::Delete { id: 1 })), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, Cmd::Restore { id: 2 })), ErrorCode::Usage);
    assert_eq!(code(exec(&mut s, B, &human(), T0, Cmd::Restore { id: 2 })), ErrorCode::NotFound);
    assert_eq!(code(exec(&mut s, B, &human(), T0, Cmd::Purge { id: 2 })), ErrorCode::NotFound);
    // 删除以后也找不到（NOT_FOUND），快照里没有
    exec(&mut s, B, &human(), T0 + 4 * MIN, Cmd::Delete { id: 3 }).unwrap();
    assert_eq!(code(exec(&mut s, B, &human(), T0, Cmd::Comment { id: 3, text: "x".into() })), ErrorCode::NotFound);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, claim(Some(3)))), ErrorCode::NotFound);
    assert_eq!(code(exec(&mut s, B, &human(), T0, mv(3, "review"))), ErrorCode::InTrash);
    assert_eq!(code(exec(&mut s, B, &human(), T0, Cmd::Later { id: 3, on: true })), ErrorCode::InTrash);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, Cmd::Later { id: 3, on: true })), ErrorCode::InTrash);
    // 服务端据此在其他回收站命令上回 IN_TRASH（而不是 NOT_FOUND），界面据此提示「先恢复」
    assert!(s.read(|r| r.in_trash(&b, 3)).unwrap());
    assert!(!s.read(|r| r.in_trash(&b, 1)).unwrap());
    let e = exec(&mut s, B, &human(), T0, Cmd::Purge { id: 3 }).unwrap();
    assert_eq!(e.card.id, 3);
    assert_eq!(snapshot(&s, B, T0).cols["todo"], vec![1, 2, 4]);
    assert_eq!(snapshot(&s, B, T0).trash, 0);
}

/// 标签使用数只算没进回收站的卡片（卡片 default/main#250）
#[test]
fn label_uses_skip_trashed_cards() {
    let mut s = store();
    let bug = ui(&mut s, |w| w.create_label("default", "bug", None, false)).unwrap();
    for t in ["一", "二"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    for id in [1, 2] {
        exec(&mut s, B, &human(), T0, Cmd::Label { id, add: vec![bug.id], remove: vec![] }).unwrap();
    }
    let uses = |s: &Store| s.read(|r| r.labels("default")).unwrap().into_iter().find(|l| l.id == bug.id).unwrap().uses;
    assert_eq!(uses(&s), 2);

    // 删除的卡片不算在里面
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: 1 }).unwrap();
    assert_eq!(uses(&s), 1);
    // 从回收站恢复后又算上
    exec(&mut s, B, &human(), T0, Cmd::Restore { id: 1 }).unwrap();
    assert_eq!(uses(&s), 2);
    // 永久删除后标签行也一起没了
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: 2 }).unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Purge { id: 2 }).unwrap();
    assert_eq!(uses(&s), 1);
}

fn card_activity(s: &Store, path: &str, id: i64) -> Result<CardDetail> {
    s.read(|r| {
        let b = r.resolve(path)?;
        r.card(&b, id)
    })
}

#[test]
fn trash_delete_releases_claim_and_restore_does_not_rekeep_it() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("做一半")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    exec(&mut s, B, &alpha(), T0 + SEC, Cmd::Beat { id: 1, done: false }).unwrap();
    let d = exec(&mut s, B, &human(), T0 + 2 * SEC, Cmd::Delete { id: 1 }).unwrap();
    assert!(d.card.holder.is_none(), "被删时领取已经释放");

    let r = exec(&mut s, B, &human(), T0 + 3 * SEC, Cmd::Restore { id: 1 }).unwrap().card;
    assert_eq!(r.holder, None, "恢复后不再被任何人持有");
    assert_eq!(r.column, "todo", "原来在锁定状态（进行中），回到当初领取它的待办");
    // 领取已经释放：不用再等心跳，看板上的耗时也不再算下去
    assert_eq!(r.beat_at, None, "恢复后没有心跳：这张卡不在任何一次领取里");
    assert_eq!((r.claimed_at, r.run_started_at), (None, None));
}

#[test]
fn trash_restore_from_inbox_goes_back_to_inbox() {
    // 「未整理」不在工作流（statuses 表）里：恢复时不能当成「原来所在的状态已经删除」扔回待办，
    // 否则刚记下、还没整理的条目会直接进可领取状态被领走
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("先记下")).unwrap();
    exec(&mut s, B, &human(), T0, Cmd::New { title: "想法".into(), body: None, column: Some(INBOX.into()), top: false, labels: vec![], from: None })
        .unwrap();
    exec(&mut s, B, &human(), T0, Cmd::New { title: "想法二".into(), body: None, column: Some(INBOX.into()), top: false, labels: vec![], from: None })
        .unwrap();
    assert_eq!(snapshot(&s, B, T0).cols[INBOX], vec![2, 3]);

    exec(&mut s, B, &human(), T0 + SEC, Cmd::Delete { id: 2 }).unwrap();
    let t = s.read(|r| r.trash(&r.resolve(B).unwrap())).unwrap();
    assert_eq!((t[0].col.as_str(), t[0].pos), (INBOX, 0), "回收站里记着原来在未整理");

    let r = exec(&mut s, B, &human(), T0 + 2 * SEC, Cmd::Restore { id: 2 }).unwrap();
    assert_eq!(r.card.column, INBOX, "从「未整理」删除的条目，恢复后回到「未整理」");
    assert!(r.activity.iter().any(|a| a.action == "restore" && a.text == "从回收站恢复，放回「未整理」"));
    assert_eq!(snapshot(&s, B, T0 + 2 * SEC).cols[INBOX], vec![2, 3], "放回原来的位置");
    assert!(snapshot(&s, B, T0 + 2 * SEC).cols["todo"] == vec![1], "没有进待办");
}

#[test]
fn trash_restore_of_held_card_goes_back_to_claimed_from() {
    // 删除会释放领取并清空 claimed_from：恢复要回到「当初领取它的可领取状态」，不能一律回首个可领取状态
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("返工里的")).unwrap();
    exec(&mut s, B, &human(), T0, mv(1, "review")).unwrap();
    exec(&mut s, B, &human(), T0, mv(1, "rework")).unwrap();
    exec(&mut s, B, &alpha(), T0 + SEC, claim(Some(1))).unwrap();
    assert_eq!(card(&s, B, 1).card.claimed_from.as_deref(), Some("rework"));

    exec(&mut s, B, &human(), T0 + 2 * SEC, Cmd::Delete { id: 1 }).unwrap();
    let t = s.read(|r| r.trash(&r.resolve(B).unwrap())).unwrap();
    assert_eq!(t[0].held_from.as_deref(), Some("rework"), "删除前把领取来源存进回收站");

    let r = exec(&mut s, B, &human(), T0 + 3 * SEC, Cmd::Restore { id: 1 }).unwrap();
    assert_eq!(r.card.column, "rework", "回到当初领取它的返工，而不是首个可领取状态的待办");
    assert_eq!(r.card.holder, None);
    assert_eq!(r.card.claimed_from, None, "恢复后不保留领取来源");
    assert!(r.activity.iter().any(|a| a.action == "restore" && a.text == "从回收站恢复，原来在「进行中」（被删时还在做），放回「返工」"));
    assert_eq!(snapshot(&s, B, T0 + 3 * SEC).cols["todo"], Vec::<i64>::new(), "没有扔回待办");

    // 领取它的那个可领取状态后来被删了：退回首个可领取状态
    exec(&mut s, B, &alpha(), T0 + 4 * SEC, claim(Some(1))).unwrap();
    exec(&mut s, B, &human(), T0 + 5 * SEC, Cmd::Delete { id: 1 }).unwrap();
    run(&mut s, |w| w.delete_status(B, "rework"));
    let r = exec(&mut s, B, &human(), T0 + 6 * SEC, Cmd::Restore { id: 1 }).unwrap();
    assert_eq!(r.card.column, "todo");
}

#[test]
fn trash_restore_after_status_deleted_goes_home() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("返工里的")).unwrap();
    exec(&mut s, B, &human(), T0, mv(1, "review")).unwrap();
    exec(&mut s, B, &human(), T0, mv(1, "rework")).unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: 1 }).unwrap();
    run(&mut s, |w| w.delete_status(B, "rework"));
    let r = exec(&mut s, B, &human(), T0 + MIN, Cmd::Restore { id: 1 }).unwrap();
    assert_eq!(r.card.column, "todo");
    assert!(r.activity.iter().any(|a| a.action == "restore" && a.text.contains("原来所在的状态已经删除")));
}

fn run(s: &mut Store, f: impl FnOnce(&mut W) -> Result<()>) {
    s.write(T0 + 500, Origin::ui(None), |w| f(w)).unwrap();
}

/// 回收站不自动清理（default/main#486）：删掉的卡片、附件一年后还在，附件清理也不动它们；
/// 只有人「永久删除」后卡片、动态、附件记录才删掉，文件由附件清理去掉。张数也不设上限
#[test]
fn trash_is_kept_until_purged_by_human() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(T0).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let files = dir.path().join("files");
    exec(&mut s, B, &human(), T0, new("要清的")).unwrap();
    let a = s.attach(T0, &b, Some(1), &human(), "log.txt", b"old").unwrap();
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: 1 }).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    let age = || {
        for e in std::fs::read_dir(&files).unwrap() {
            std::fs::File::options().write(true).open(e.unwrap().path()).unwrap().set_modified(old).unwrap();
        }
    };
    // 一年后：卡片、附件都还在，附件清理不删被回收站卡片引用的文件
    age();
    let (_, removed) = s.gc_files(T0 + 365 * DAY).unwrap();
    assert_eq!(removed, 0, "回收站卡片的附件不清理");
    assert_eq!(s.read(|r| r.trash(&b)).unwrap().len(), 1);
    assert_eq!(s.attachment(a.id).unwrap().unwrap().0.card_id, Some(1), "在回收站里附件还在");
    // agent 不能永久删除；人永久删除后卡片、附件记录一起删，文件由附件清理去掉
    assert_eq!(code(exec(&mut s, B, &alpha(), T0 + 365 * DAY, Cmd::Purge { id: 1 })), ErrorCode::Usage);
    exec(&mut s, B, &human(), T0 + 365 * DAY, Cmd::Purge { id: 1 }).unwrap();
    assert!(s.read(|r| r.trash(&b)).unwrap().is_empty());
    assert!(s.attachment(a.id).unwrap().is_none());
    age();
    let (_, removed) = s.gc_files(T0 + 365 * DAY).unwrap();
    assert_eq!(removed, 1, "附件文件也删掉了");
    // 编号不复用
    assert_eq!(exec(&mut s, B, &human(), T0 + 366 * DAY, new("新的")).unwrap().card.id, 2);
}

#[test]
fn trash_has_no_size_limit() {
    let mut s = store();
    let n = 1002;
    // 一个写事务里做完，比每张一个事务快得多；被测的是回收站不自动清，不是事务
    s.write(T0, Origin::ui(None), |w| {
        let b = w.resolve(B)?;
        for _ in 0..n {
            w.exec(&b, &human(), new("x"))?;
        }
        Ok(())
    })
    .unwrap();
    s.write(T0 + HOUR, Origin::ui(None), |w| {
        let b = w.resolve(B)?;
        for id in 1..=n {
            w.exec(&b, &human(), Cmd::Delete { id })?;
        }
        Ok(())
    })
    .unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let t = s.read(|r| r.trash(&b)).unwrap();
    assert_eq!(t.len() as i64, n, "删得再多也都留着");
    assert_eq!(t[0].card.id, 1);
}

/// 复盘台的「打回」「回复并放回」（#194）：评论和移动在一个命令、一个写事务里，要么都发生，要么都不发生
#[test]
fn reply_comments_and_moves_in_one_write() {
    let mut s = store();
    let id = exec(&mut s, B, &human(), T0, new("A")).unwrap().card.id;
    exec(&mut s, B, &alpha(), T0, claim(Some(id))).unwrap();
    exec(&mut s, B, &alpha(), T0, Cmd::Beat { id, done: true }).unwrap();
    exec(&mut s, B, &alpha(), T0, mv(id, "review")).unwrap();
    let before = card(&s, B, id).activity.len();
    // 打回：意见作为评论发出，卡片移到返工（rework）
    let c = exec(&mut s, B, &human(), T0 + MIN, Cmd::Reply { id, text: "  按钮颜色不对，改成品牌色  ".into(), column: "返工".into() }).unwrap();
    assert_eq!(c.card.column, "rework");
    let d = card(&s, B, id);
    let new_items: Vec<_> = d.activity[before..].iter().map(|a| (a.kind.clone(), a.text.clone())).collect();
    assert_eq!(new_items[0], ("comment".to_string(), "按钮颜色不对，改成品牌色".to_string()));
    assert!(new_items[1].0 == "system" && new_items[1].1.contains("返工"), "{new_items:?}");
    // 没写意见：拒绝，什么也不发生
    let n = card(&s, B, id).activity.len();
    assert_eq!(code(exec(&mut s, B, &human(), T0, Cmd::Reply { id, text: "  ".into(), column: "todo".into() })), ErrorCode::Usage);
    // 移动被拒绝（agent 不能移出可领取状态以外的地方 / 状态不存在）：评论也不留下
    assert!(exec(&mut s, B, &human(), T0, Cmd::Reply { id, text: "放回去".into(), column: "nope".into() }).is_err());
    assert_eq!(card(&s, B, id).activity.len(), n, "整个写回滚：没有留下评论");
    assert_eq!(card(&s, B, id).card.column, "rework");
}

/* ---------------- 隐藏的标签（default/main#324） ---------------- */

fn hidden_of(s: &Store, key: &str) -> Vec<String> {
    s.read(|r| r.workspace(T0)).unwrap().projects.into_iter().find(|p| p.key == key).unwrap().hidden_labels
}

fn project_hidden(s: &Store, key: &str) -> bool {
    s.read(|r| r.workspace(T0)).unwrap().projects.into_iter().find(|p| p.key == key).unwrap().hidden
}

/// 新项目默认隐藏版本标签（`^v\d`）；改规则是整组替换，去掉空行、首尾空白和重复的
#[test]
fn hidden_labels_default_and_update() {
    let mut s = store();
    assert_eq!(hidden_of(&s, "default"), vec![r"^v\d".to_string()]);
    let rules: Vec<String> = vec![" ^v\\d ".into(), "".into(), "^tmp-".into(), "^v\\d".into()];
    s.write(T0, Origin::ui(None), |w| w.update_project_with("default", store::ProjectPatch { hidden_labels: Some(&rules), ..Default::default() }))
        .unwrap();
    assert_eq!(hidden_of(&s, "default"), vec![r"^v\d".to_string(), "^tmp-".to_string()]);
    // 只改名字不动规则
    s.write(T0, Origin::ui(None), |w| w.update_project("default", Some("改名"), None, None)).unwrap();
    assert_eq!(hidden_of(&s, "default").len(), 2);
    // 清空：一条都不隐藏（不会回到默认）
    s.write(T0, Origin::ui(None), |w| w.update_project_with("default", store::ProjectPatch { hidden_labels: Some(&[]), ..Default::default() }))
        .unwrap();
    assert!(hidden_of(&s, "default").is_empty());
}

/// 条数、长度有上限，超了拒绝（USAGE），原来的规则不变
#[test]
fn hidden_labels_limits() {
    let mut s = store();
    let many: Vec<String> = (0..=HIDDEN_LABELS_MAX).map(|i| format!("^x{i}")).collect();
    let r = s.write(T0, Origin::ui(None), |w| w.update_project_with("default", store::ProjectPatch { hidden_labels: Some(&many), ..Default::default() }));
    assert_eq!(r.expect_err("太多条").code, ErrorCode::Usage);
    let long = vec!["a".repeat(HIDDEN_LABEL_LEN + 1)];
    let r = s.write(T0, Origin::ui(None), |w| w.update_project_with("default", store::ProjectPatch { hidden_labels: Some(&long), ..Default::default() }));
    assert_eq!(r.expect_err("太长").code, ErrorCode::Usage);
    assert_eq!(hidden_of(&s, "default"), vec![r"^v\d".to_string()]);
    assert_eq!(store::check_hidden_labels(&many[..HIDDEN_LABELS_MAX]).unwrap().len(), HIDDEN_LABELS_MAX);
}

/// 旧数据库的项目没有隐藏规则列：迁移后按默认隐藏版本标签
#[test]
fn old_projects_hide_version_labels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
    }
    rusqlite::Connection::open(&path).unwrap().execute_batch("ALTER TABLE projects DROP COLUMN hidden_labels").unwrap();
    let s = Store::open(&path).unwrap();
    assert_eq!(hidden_of(&s, "default"), vec![r"^v\d".to_string()]);
}

/// 旧服务端的工作区没有 hiddenLabels：按默认处理
#[test]
fn project_info_without_hidden_labels_uses_default() {
    let p: ProjectInfo = serde_json::from_value(serde_json::json!({
        "key": "a", "name": "甲", "desc": "", "color": "#000000", "pos": { "x": 0.0, "y": 0.0 }, "createdAt": 0, "labels": [], "boards": []
    }))
    .unwrap();
    assert_eq!(p.hidden_labels, vec![r"^v\d".to_string()]);
    assert!(!p.hidden, "旧服务端没有 hidden：按显示");
}

/// 项目默认显示；可以隐藏，刷新、重开数据库后仍然隐藏，只改别的字段不动它
#[test]
fn project_hidden_defaults_visible_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        assert!(!project_hidden(&s, "default"), "新项目默认显示");
        s.write(T0, Origin::ui(None), |w| w.update_project_with("default", store::ProjectPatch { hidden: Some(true), ..Default::default() }))
            .unwrap();
        assert!(project_hidden(&s, "default"));
        s.write(T0, Origin::ui(None), |w| w.update_project("default", Some("改名"), None, None)).unwrap();
        assert!(project_hidden(&s, "default"), "只改名字不动隐藏");
    }
    let s = Store::open(&path).unwrap();
    let p = s.read(|r| r.workspace(T0)).unwrap().projects.into_iter().find(|p| p.key == "default").unwrap();
    assert!(p.hidden);
    assert_eq!(serde_json::to_value(p).unwrap()["hidden"], serde_json::json!(true));
}

/// 旧数据库的项目没有 hidden 列：迁移后都显示
#[test]
fn old_projects_are_visible_in_side_panel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
    }
    rusqlite::Connection::open(&path).unwrap().execute_batch("ALTER TABLE projects DROP COLUMN hidden").unwrap();
    let s = Store::open(&path).unwrap();
    assert!(!project_hidden(&s, "default"));
}

/* ---------------- 稍后再做（default/main#303） ---------------- */

fn later(id: i64, on: bool) -> Cmd {
    Cmd::Later { id, on }
}

/// 收起、放回：只有人能做、只在「未整理」里；状态、位置、标签都不动；重复执行结果相同；记一条动态
#[test]
fn inbox_later_on_and_off() {
    let mut s = store();
    let a = exec(&mut s, B, &alpha(), T0, inbox_new("先放一放")).unwrap().card.id;
    let b = exec(&mut s, B, &alpha(), T0, inbox_new("第二条")).unwrap().card.id;
    let beyond = exec(&mut s, B, &human(), T0, new("工作流里的")).unwrap().card.id;
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, later(a, true))), ErrorCode::Usage, "agent 不能收");
    assert_eq!(code(exec(&mut s, B, &human(), T0, later(beyond, true))), ErrorCode::Usage, "工作流里的不能收");
    assert_eq!(code(exec(&mut s, B, &human(), T0, later(99, true))), ErrorCode::NotFound);

    let before = snapshot(&s, B, T0).cols[INBOX].clone();
    let d = exec(&mut s, B, &human(), T0 + 5, later(a, true)).unwrap();
    assert_eq!(d.card.later_at, Some(T0 + 5));
    assert_eq!(d.card.column, INBOX, "还在「未整理」，只是收起来");
    assert!(d.activity.iter().any(|x| x.text.contains("稍后再做")));
    // 再收一次不变（时间不刷新、不再记动态）
    let n = d.activity.len();
    let d2 = exec(&mut s, B, &human(), T0 + 9, later(a, true)).unwrap();
    assert_eq!(d2.card.later_at, Some(T0 + 5));
    assert_eq!(d2.activity.len(), n);
    assert_eq!(snapshot(&s, B, T0).cols[INBOX], before, "顺序不动");

    let d = exec(&mut s, B, &human(), T0 + 10, later(a, false)).unwrap();
    assert_eq!(d.card.later_at, None);
    assert_eq!(snapshot(&s, B, T0).cols[INBOX], before, "放回原来的位置");
    assert_eq!(snapshot(&s, B, T0).cards[&b].later_at, None);
}

/// 「稍后再做」的条目不占「未整理」计数，单独算在 later 里；白板的卡片总数不变（default/main#585）
#[test]
fn inbox_counts_leave_out_later_entries() {
    let mut s = store();
    let a = exec(&mut s, B, &alpha(), T0, inbox_new("先放一放")).unwrap().card.id;
    exec(&mut s, B, &alpha(), T0, inbox_new("第二条")).unwrap();
    let counts = |s: &Store| {
        let ws = s.read(|r| r.workspace(T0)).unwrap();
        let b = &ws.projects[0].boards[0];
        (b.counts[INBOX], b.counts[LATER], b.counts.values().sum::<i64>())
    };
    assert_eq!(counts(&s), (2, 0, 2));
    exec(&mut s, B, &human(), T0 + 5, later(a, true)).unwrap();
    assert_eq!(counts(&s), (1, 1, 2), "收起的从 inbox 挪到 later，总数不变");
    exec(&mut s, B, &human(), T0 + 10, later(a, false)).unwrap();
    assert_eq!(counts(&s), (2, 0, 2), "放回后加回来");

    // 拆解后又被收起：两项条件同时成立时，inbox 只减一次；收起的原条目仍算进 later
    exec(
        &mut s,
        B,
        &human(),
        T0 + 15,
        Cmd::New { title: "拆出来".into(), body: None, column: Some(INBOX.into()), top: false, labels: vec![], from: Some(a) },
    )
    .unwrap();
    assert_eq!(counts(&s), (2, 0, 2), "拆解后原条目由拆出的条目表示");
    exec(&mut s, B, &human(), T0 + 20, later(a, true)).unwrap();
    assert_eq!(counts(&s), (2, 1, 3), "拆解 + 收起重合时，inbox 不会减两次");
}

/// 收起的条目被整理进工作流时清掉标记；放回「未整理」（回收站恢复之外）不会自己变回收起
#[test]
fn inbox_later_cleared_when_leaving_inbox() {
    let mut s = store();
    let a = exec(&mut s, B, &alpha(), T0, inbox_new("收起后整理")).unwrap().card.id;
    exec(&mut s, B, &human(), T0, later(a, true)).unwrap();
    exec(&mut s, B, &human(), T0, mv(a, "todo")).unwrap();
    let c = &snapshot(&s, B, T0).cards[&a];
    assert_eq!(c.column, "todo");
    assert_eq!(c.later_at, None);
}

/// 旧数据库的卡片没有 later_at 列：迁移后都没收起
#[test]
fn old_cards_are_not_later() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        exec(&mut s, B, &alpha(), T0, inbox_new("旧条目")).unwrap();
    }
    rusqlite::Connection::open(&path).unwrap().execute_batch("ALTER TABLE cards DROP COLUMN later_at").unwrap();
    let s = Store::open(&path).unwrap();
    assert!(snapshot(&s, B, T0).cards.values().all(|c| c.later_at.is_none()));
}

/// 迁移不是原子时留下的半升级库（#509）：旧版同一批加 repo、branch，只加了 repo 就断掉——下次打开逐列补上 branch，
/// 项目、白板上已经设的仓库地址不动；sched 只有 max_run 没有 idle_run 时同样补上
#[test]
fn half_migrated_columns_are_completed_without_touching_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("half.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "UPDATE projects SET repo = 'https://example.com/r.git', branch = 'dev';
             UPDATE boards SET repo = 'https://example.com/b.git';
             ALTER TABLE projects DROP COLUMN branch;
             ALTER TABLE boards DROP COLUMN branch;
             ALTER TABLE sched DROP COLUMN idle_run;",
        )
        .unwrap();
    }
    let s = Store::open(&path).unwrap();
    let (repo, branch): (String, String) = s.conn().query_row("SELECT repo, branch FROM projects WHERE key = 'default'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((repo.as_str(), branch.as_str()), ("https://example.com/r.git", ""), "补上 branch，仓库地址保留（白板上没有 branch 可回填）");
    let (brepo, bbranch): (String, String) = s.conn().query_row("SELECT repo, branch FROM boards LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((brepo.as_str(), bbranch.as_str()), ("https://example.com/b.git", ""));
    let idle: i64 = s.conn().query_row("SELECT idle_run FROM sched", [], |r| r.get(0)).unwrap();
    assert_eq!(idle, 20);
}

/// 旧版「加列 + 回填」只加了列、没来得及回填就断掉（#509）：没有步骤标记的库下次打开补上还是 NULL 的，
/// 已经有值的不覆盖，没看过的「未整理」条目照旧没看过；补过一次就记下标记，不再重复
#[test]
fn backfill_left_undone_by_an_old_crash_runs_once_on_next_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fill.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
        exec(&mut s, B, &human(), T0, new("待办里的")).unwrap();
        exec(&mut s, B, &human(), T0, new("已经看过的")).unwrap();
        exec(&mut s, B, &alpha(), T0, Cmd::New { title: "未整理".into(), body: None, column: Some("inbox".into()), top: false, labels: vec![], from: None }).unwrap();
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "UPDATE cards SET moved_at = NULL, seen_at = NULL WHERE id IN (1, 3);
             UPDATE cards SET seen_at = 42 WHERE id = 2;
             DELETE FROM meta WHERE key LIKE 'step:%';",
        )
        .unwrap();
    }
    let s = Store::open(&path).unwrap();
    let get = |id: i64| -> (Option<i64>, Option<i64>) { s.conn().query_row("SELECT moved_at, seen_at FROM cards WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap() };
    assert_eq!(get(1), (Some(T0), Some(T0)), "补上了");
    assert_eq!(get(2).1, Some(42), "已经有值的不覆盖");
    assert_eq!(get(3), (Some(T0), None), "agent 记的未整理条目还是没看过");
    let steps: i64 = s.conn().query_row("SELECT COUNT(*) FROM meta WHERE key IN ('step:moved_at', 'step:seen_at')", [], |r| r.get(0)).unwrap();
    assert_eq!(steps, 2);
    drop(s);
    // 记了标记：之后变回 NULL 的（例如新的未整理条目）不会被再补
    rusqlite::Connection::open(&path).unwrap().execute_batch("UPDATE cards SET seen_at = NULL WHERE id = 1").unwrap();
    let s = Store::open(&path).unwrap();
    assert_eq!(s.conn().query_row("SELECT seen_at FROM cards WHERE id = 1", [], |r| r.get::<_, Option<i64>>(0)).unwrap(), None);
}

/// 迁移中途失败（#509）：整个回滚，数据库还是升级前的样子，打开报错写明原因；下一次打开从头再来、升级完整
#[test]
fn a_failed_migration_rolls_back_and_the_next_open_finishes_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fail.db");
    {
        let mut s = Store::open(&path).unwrap();
        s.ensure_default(T0).unwrap();
    }
    let cols = |p: &std::path::Path| -> Vec<String> {
        let c = rusqlite::Connection::open(p).unwrap();
        let mut st = c.prepare("SELECT name FROM pragma_table_info('boards')").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect()
    };
    rusqlite::Connection::open(&path).unwrap().execute_batch("ALTER TABLE boards DROP COLUMN branch; ALTER TABLE boards DROP COLUMN readme;").unwrap();
    for at in ["boards", "end"] {
        crate::store::FAIL_AT.with(|f| f.set(Some(at)));
        let e = Store::open(&path).err().expect("迁移失败时打不开");
        crate::store::FAIL_AT.with(|f| f.set(None));
        assert!(e.message.contains("已经回滚") && e.message.contains(at), "{}", e.message);
        let now = cols(&path);
        assert!(!now.contains(&"branch".to_string()) && !now.contains(&"readme".to_string()), "回滚了：{at} 之前补的列也不在 {now:?}");
    }
    let s = Store::open(&path).unwrap();
    drop(s);
    let now = cols(&path);
    assert!(now.contains(&"branch".to_string()) && now.contains(&"readme".to_string()), "{now:?}");
}

/* ---------------- 批量打标签（default/main#531） ---------------- */

fn label_batch(s: &mut Store, who: &Actor, ids: &[i64], add: &[i64], remove: &[i64]) -> Result<LabelBatch> {
    let origin = if who.is_human() { Origin::ui(None) } else { Origin::cli() };
    s.write(T0, origin, |w| {
        let b = w.resolve(B)?;
        w.label_batch(&b, who, ids, add, remove)
    })
}
fn label_acts(s: &Store, id: i64) -> usize {
    card(s, B, id).activity.iter().filter(|a| a.action == "label").count()
}

/// 一次写事务打完，rev 只加一次；已经带着 / 本来没有的不算改动、不记动态；重复执行结果相同
#[test]
fn label_batch_all_in_one_write() {
    let mut s = store();
    let bug = ui(&mut s, |w| w.create_label("default", "bug", None, false)).unwrap();
    for i in 0..100 {
        exec(&mut s, B, &human(), T0, new(&format!("卡{i}"))).unwrap();
    }
    exec(&mut s, B, &human(), T0, Cmd::Label { id: 3, add: vec![bug.id], remove: vec![] }).unwrap();
    let ids: Vec<i64> = (1..=100).collect();

    let rev = s.rev().unwrap();
    let r = label_batch(&mut s, &human(), &ids, &[bug.id], &[]).unwrap();
    assert_eq!(s.rev().unwrap(), rev + 1, "一次批量写入只让 rev 加一");
    assert_eq!(r.changed.len(), 99, "#3 本来就有，不算");
    assert!(!r.changed.contains(&3) && r.failed.is_empty());
    assert!(ids.iter().all(|id| card(&s, B, *id).card.labels == vec![bug.id]));
    assert_eq!(label_acts(&s, 3), 1, "本来就有的不重复记动态");
    assert_eq!(label_acts(&s, 50), 1);

    // 重复执行：什么也不变，rev 不动
    let rev = s.rev().unwrap();
    let r = label_batch(&mut s, &human(), &ids, &[bug.id], &[]).unwrap();
    assert!(r.changed.is_empty() && r.failed.is_empty());
    assert_eq!(s.rev().unwrap(), rev);
    assert_eq!(label_acts(&s, 50), 1);

    // 去掉：同样一次；重复的 id 只算一次
    let mut twice = ids.clone();
    twice.extend([1, 2]);
    let r = label_batch(&mut s, &human(), &twice, &[], &[bug.id]).unwrap();
    assert_eq!(r.changed, ids);
    assert!(ids.iter().all(|id| card(&s, B, *id).card.labels.is_empty()));
    assert_eq!(label_acts(&s, 1), 2);
}

/// 一张不存在、一张在回收站里：记进 failed，其他照常
#[test]
fn label_batch_partial_failure() {
    let mut s = store();
    let bug = ui(&mut s, |w| w.create_label("default", "bug", None, false)).unwrap();
    for t in ["一", "二", "三"] {
        exec(&mut s, B, &human(), T0, new(t)).unwrap();
    }
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: 2 }).unwrap();
    let r = label_batch(&mut s, &human(), &[1, 2, 3, 99], &[bug.id], &[]).unwrap();
    assert_eq!(r.changed, vec![1, 3]);
    let failed: Vec<(i64, ErrorCode)> = r.failed.iter().map(|f| (f.id, f.code)).collect();
    assert_eq!(failed, vec![(2, ErrorCode::InTrash), (99, ErrorCode::NotFound)]);
    assert!(r.failed[1].message.contains("#99"), "{}", r.failed[1].message);
    assert_eq!(card(&s, B, 1).card.labels, vec![bug.id]);
    assert_eq!(card(&s, B, 3).card.labels, vec![bug.id]);
}

/// 整批拒绝、什么也不改：超过上限、标签不在项目里、agent 调用
#[test]
fn label_batch_rejections() {
    let mut s = store();
    let bug = ui(&mut s, |w| w.create_label("default", "bug", None, false)).unwrap();
    let mine = ui(&mut s, |w| w.create_label("default", "人定", None, true)).unwrap();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();

    let many: Vec<i64> = (1..=(BATCH_MAX as i64 + 1)).collect();
    let e = label_batch(&mut s, &human(), &many, &[bug.id], &[]).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("500"), "{}", e.message);
    // 正好上限：可以（不存在的记进 failed）
    let r = label_batch(&mut s, &human(), &many[..BATCH_MAX], &[bug.id], &[]).unwrap();
    assert_eq!((r.changed.len(), r.failed.len()), (1, BATCH_MAX - 1));

    let rev = s.rev().unwrap();
    assert_eq!(label_batch(&mut s, &human(), &[1], &[999], &[]).unwrap_err().code, ErrorCode::Usage);
    // agent 不能用批量接口（仅人可打的、普通的都一样），单卡命令照旧
    assert_eq!(label_batch(&mut s, &alpha(), &[1], &[bug.id], &[]).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(label_batch(&mut s, &alpha(), &[1], &[], &[mine.id]).unwrap_err().code, ErrorCode::Usage);
    assert_eq!(s.rev().unwrap(), rev);
    assert_eq!(card(&s, B, 1).card.labels, vec![bug.id]);

    // 仅人可打：人可以批量加、去
    let r = label_batch(&mut s, &human(), &[1], &[mine.id], &[]).unwrap();
    assert_eq!(r.changed, vec![1]);
    assert!(card(&s, B, 1).card.labels.contains(&mine.id));
    let r = label_batch(&mut s, &human(), &[1], &[], &[mine.id]).unwrap();
    assert_eq!(r.changed, vec![1]);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, Cmd::Label { id: 1, add: vec![mine.id], remove: vec![] })), ErrorCode::Usage);
}

/* ---------------- 批量卡片操作（default/main#595） ---------------- */

fn card_batch(s: &mut Store, who: &Actor, op: &CardBatchOp, ids: &[i64]) -> Result<CardBatch> {
    let origin = if who.is_human() { Origin::ui(None) } else { Origin::cli() };
    s.write(T0, origin, |w| {
        let b = w.resolve(B)?;
        w.card_batch(&b, who, op, ids)
    })
}

/// 按传入顺序逐张执行，重复 id 只做一次；一次批量写入只让 rev 加一
#[test]
fn card_batch_move_runs_in_order_once() {
    let mut s = store();
    for title in ["一", "二", "三"] {
        exec(&mut s, B, &human(), T0, new(title)).unwrap();
    }
    let rev = s.rev().unwrap();
    let r = card_batch(&mut s, &human(), &CardBatchOp::Move { column: "review".into() }, &[3, 1, 2, 3]).unwrap();
    assert_eq!(r.ok, vec![3, 1, 2]);
    assert!(r.failed.is_empty());
    assert_eq!(snapshot(&s, B, T0).cols["review"], vec![3, 1, 2]);
    assert_eq!(s.rev().unwrap(), rev + 1, "一次批量写入只让 rev 加一");
}

/// 其他批量操作也走同一批单卡命令
#[test]
fn card_batch_covers_later_and_trash_operations() {
    let mut s = store();
    for title in ["一", "二"] {
        exec(&mut s, B, &human(), T0, inbox_new(title)).unwrap();
    }
    card_batch(&mut s, &human(), &CardBatchOp::Later { on: true }, &[1, 2]).unwrap();
    assert!(snapshot(&s, B, T0).cards.values().all(|c| c.later_at.is_some()));
    card_batch(&mut s, &human(), &CardBatchOp::Later { on: false }, &[1, 2]).unwrap();
    assert!(snapshot(&s, B, T0).cards.values().all(|c| c.later_at.is_none()));

    card_batch(&mut s, &human(), &CardBatchOp::Delete, &[1, 2]).unwrap();
    assert!(snapshot(&s, B, T0).cols[INBOX].is_empty());
    assert_eq!(snapshot(&s, B, T0).trash, 2);

    card_batch(&mut s, &human(), &CardBatchOp::Restore, &[1, 2]).unwrap();
    assert_eq!(snapshot(&s, B, T0).cols[INBOX], vec![1, 2]);
    assert_eq!(snapshot(&s, B, T0).trash, 0);

    card_batch(&mut s, &human(), &CardBatchOp::Delete, &[1, 2]).unwrap();
    card_batch(&mut s, &human(), &CardBatchOp::Purge, &[1, 2]).unwrap();
    assert_eq!(snapshot(&s, B, T0).trash, 0);
    assert_eq!(code(exec(&mut s, B, &human(), T0, Cmd::Restore { id: 1 })), ErrorCode::NotFound);
}

/// 一张失败只回滚这一张，前后的卡片照常执行；rev 仍只加一次
#[test]
fn card_batch_partial_failure_isolated() {
    let mut s = store();
    for title in ["一", "二", "三"] {
        exec(&mut s, B, &human(), T0, new(title)).unwrap();
    }
    exec(&mut s, B, &human(), T0, Cmd::Delete { id: 2 }).unwrap();
    let rev = s.rev().unwrap();
    let r = card_batch(&mut s, &human(), &CardBatchOp::Move { column: "review".into() }, &[1, 2, 3, 99]).unwrap();
    assert_eq!(r.ok, vec![1, 3]);
    let failed: Vec<(i64, ErrorCode)> = r.failed.iter().map(|f| (f.id, f.code)).collect();
    assert_eq!(failed, vec![(2, ErrorCode::InTrash), (99, ErrorCode::NotFound)]);
    assert_eq!(snapshot(&s, B, T0).cols["review"], vec![1, 3]);
    assert_eq!(snapshot(&s, B, T0).trash, 1, "失败那张仍在回收站，没有留下半步修改");
    assert_eq!(s.rev().unwrap(), rev + 1);
}

/// 批量卡片操作仅人可用；去重后超过上限整批拒绝
#[test]
fn card_batch_rejections() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    let rev = s.rev().unwrap();
    assert_eq!(
        card_batch(&mut s, &alpha(), &CardBatchOp::Move { column: "review".into() }, &[1]).unwrap_err().code,
        ErrorCode::Usage
    );

    let many: Vec<i64> = (1..=(BATCH_MAX as i64 + 1)).collect();
    let e = card_batch(&mut s, &human(), &CardBatchOp::Delete, &many).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("500"), "{}", e.message);
    assert_eq!(s.rev().unwrap(), rev);
    assert!(!s.read(|r| r.in_trash(&r.resolve(B).unwrap(), 1)).unwrap());
}

/* ---------------- 交审、做不下去一步完成（board submit / board giveup） ---------------- */

fn submit(id: i64, text: &str) -> Cmd {
    Cmd::Submit { id, text: text.into() }
}
fn giveup(id: i64, text: &str) -> Cmd {
    Cmd::GiveUp { id, text: text.into() }
}

/// 交审：评论、结束计时、移到交审状态，一条命令做完；只有持有者能做，要写评论
#[test]
fn submit_comments_finals_and_moves_to_review() {
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    // 还没领取、不是持有者
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, submit(1, "做完了"))), ErrorCode::NotHolder);
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    assert_eq!(code(exec(&mut s, B, &beta(), T0, submit(1, "做完了"))), ErrorCode::NotHolder);
    assert_eq!(code(exec(&mut s, B, &human(), T0, submit(1, "做完了"))), ErrorCode::NotHolder);
    assert_eq!(code(exec(&mut s, B, &alpha(), T0, submit(1, "  "))), ErrorCode::Usage);
    assert!(card(&s, B, 1).activity.iter().all(|a| a.action != "comment"), "被拒绝时评论不留下");

    beat_span(&mut s, &alpha(), 1, T0 + SEC, T0 + 2 * MIN);
    let c = exec(&mut s, B, &alpha(), T0 + 2 * MIN + SEC, submit(1, "改了登录页\n提交：abc1234")).unwrap();
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), ("review", None));
    assert_eq!((c.card.runs, c.card.runs_lost, c.card.run_started_at), (1, 0, None), "这一次正常结束，不算异常退出");
    let acts: Vec<(&str, &str)> = c.activity.iter().map(|a| (a.action.as_str(), a.text.as_str())).collect();
    let at = |action: &str, has: &str| acts.iter().position(|(a, t)| *a == action && t.contains(has)).unwrap_or_else(|| panic!("{action} {has}: {acts:?}"));
    // 顺序：评论 → 结束心跳 → 移动
    assert!(at("comment", "提交：abc1234") < at("beat", "结束了心跳"));
    assert!(at("beat", "结束了心跳") < at("move", "待审核"));
    // 交审之后不会因为没移走被释放
    assert_eq!(s.sweep(T0 + 10 * MIN).unwrap(), 0);
    assert_eq!(card(&s, B, 1).card.column, "review");
}

/// 交审状态看状态设置里的开关（default/main#541）：名字不带关键词的「待复盘」打开开关后，board submit 把卡移过去；
/// 默认工作流的「待审核」新建时就打开；一块白板上都没打开时按名称里的关键词认（旧白板）
#[test]
fn review_status_follows_the_switch_then_falls_back_to_names() {
    let mut s = store();
    let flow = |s: &Store| s.read(|r| r.flow(&r.resolve(B)?)).unwrap();
    assert!(flow(&s).get("review").unwrap().review, "默认工作流的「待审核」是交审状态");
    // 进行中 → 待复盘 → 已完成，去掉 进行中 → 待审核
    let t = ui(&mut s, |w| w.create_status(B, "待复盘", None)).unwrap();
    ui(&mut s, |w| w.update_edges(B, &[("doing".into(), t.key.clone()), (t.key.clone(), "done".into())], &[("doing".into(), "review".into())])).unwrap();
    assert!(flow(&s).review_after("doing").is_none(), "没打开开关、名字也不带关键词：认不出");
    ui(&mut s, |w| w.update_status(B, &t.key, StatusPatch { review: Some(true), ..Default::default() })).unwrap();
    let f = flow(&s);
    assert_eq!(f.review_after("doing").map(|x| x.key.as_str()), Some(t.key.as_str()));
    assert!(f.is_review(&t.key));
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    let c = exec(&mut s, B, &alpha(), T0 + SEC, submit(1, "做完了")).unwrap();
    assert_eq!((c.card.column.as_str(), c.card.holder), (t.key.as_str(), None));

    // 开关和名称都在时先看开关：连回 进行中 → 待审核，但「待审核」关掉开关，「待复盘」开着
    ui(&mut s, |w| w.update_edges(B, &[("doing".into(), "review".into())], &[])).unwrap();
    ui(&mut s, |w| w.update_status(B, "review", StatusPatch { review: Some(false), ..Default::default() })).unwrap();
    assert_eq!(flow(&s).review_after("doing").map(|x| x.key.clone()), Some(t.key.clone()));
    // 都没打开（旧白板）：按名称里的关键词
    ui(&mut s, |w| w.update_status(B, &t.key, StatusPatch { review: Some(false), ..Default::default() })).unwrap();
    assert_eq!(flow(&s).review_after("doing").map(|x| x.key.as_str()), Some("review"));
}

/// 交审状态的开关只能开在不是可领取、锁定、终点、错误节点的状态上；打开那几项之一时同时关掉它
#[test]
fn review_switch_only_on_plain_statuses() {
    let mut s = store();
    let on = StatusPatch { review: Some(true), ..Default::default() };
    for key in ["todo", "doing", "done"] {
        let e = ui(&mut s, |w| w.update_status(B, key, on.clone())).unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{key}");
        assert!(e.message.contains("交审状态"), "{}", e.message);
    }
    let t = ui(&mut s, |w| w.create_status(B, "错误", None)).unwrap();
    ui(&mut s, |w| w.update_status(B, &t.key, StatusPatch { err: Some(true), ..Default::default() })).unwrap();
    assert_eq!(ui(&mut s, |w| w.update_status(B, &t.key, on.clone())).unwrap_err().code, ErrorCode::Usage);
    // 同一次把锁定关掉、交审打开：可以
    let u = ui(&mut s, |w| w.create_status(B, "测试中", None)).unwrap();
    ui(&mut s, |w| w.update_status(B, &u.key, StatusPatch { lock: Some(true), ..Default::default() })).unwrap();
    ui(&mut s, |w| w.update_status(B, &u.key, StatusPatch { lock: Some(false), review: Some(true), ..Default::default() })).unwrap();
    let flow = |s: &Store| s.read(|r| r.flow(&r.resolve(B)?)).unwrap();
    assert!(flow(&s).get(&u.key).unwrap().review);
    // 打开终点：交审一起关掉
    ui(&mut s, |w| w.update_status(B, &u.key, StatusPatch { fin: Some(true), ..Default::default() })).unwrap();
    let st = flow(&s).get(&u.key).unwrap().clone();
    assert!(st.fin && !st.review);
}

/// 白板没有交审状态：拒绝，评论和 final 都不留下，卡片还归持有者
#[test]
fn submit_without_review_status_changes_nothing() {
    let mut s = store();
    ui(&mut s, |w| w.delete_status(B, "review")).unwrap();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    exec(&mut s, B, &alpha(), T0 + SEC, beat(1, false)).unwrap();
    let e = exec(&mut s, B, &alpha(), T0 + 2 * SEC, submit(1, "做完了")).unwrap_err();
    assert_eq!(e.code, ErrorCode::ForbiddenColumn);
    assert!(e.message.contains("board giveup 1"), "{}", e.message);
    let c = card(&s, B, 1);
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), ("doing", Some("alpha")));
    assert!(c.card.run_started_at.is_some(), "没有 final");
    assert!(c.activity.iter().all(|a| a.action != "comment"));
    // 一级锁定、真的没有交审状态：没有下一个锁定状态，照旧建议 giveup
    assert!(flow(&s).next_lock("doing").is_none());
    assert!(!e.message.contains("board move"), "{}", e.message);
}

/// 两级锁定（进行中 → 测试中 → 待审核）：刚领到的卡在「进行中」，连不到交审状态。board submit 拒绝时提示先
/// board move 到下一个锁定状态，不建议 giveup；移过去再交审成功（default/main#542）
#[test]
fn submit_from_first_of_two_locks_points_to_next_lock() {
    let mut s = store();
    // 默认工作流（一级锁定）：没有下一个锁定状态
    assert!(flow(&s).next_lock("doing").is_none());
    assert!(flow(&s).next_lock_hint(1, "doing").is_none());
    let t = ui(&mut s, |w| w.create_status(B, "测试中", None)).unwrap();
    ui(&mut s, |w| w.update_status(B, &t.key, StatusPatch { lock: Some(true), ..Default::default() })).unwrap();
    let add = [("doing".to_string(), t.key.clone()), (t.key.clone(), "review".into()), (t.key.clone(), "check".into())];
    ui(&mut s, |w| w.update_edges(B, &add, &[("doing".into(), "review".into()), ("doing".into(), "check".into())])).unwrap();
    let f = flow(&s);
    assert_eq!(f.next_lock("doing").map(|x| x.key.as_str()), Some(t.key.as_str()));
    assert!(f.next_lock(&t.key).is_none(), "第二级连出去没有锁定状态");
    assert!(f.next_lock("todo").is_none(), "不是锁定状态");
    assert!(f.next_lock_hint(1, &t.key).is_none(), "第二级连得到交审状态：不用提示");

    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    let e = exec(&mut s, B, &alpha(), T0 + SEC, submit(1, "做完了")).unwrap_err();
    assert_eq!(e.code, ErrorCode::ForbiddenColumn);
    assert_eq!(e.message, "#1 在「进行中」，下一步是锁定状态「测试中」：先 board move 1 测试中，到那里再交审");
    let c = card(&s, B, 1);
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), ("doing", Some("alpha")));
    assert!(c.activity.iter().all(|a| a.action != "comment"), "被拒绝时评论不留下");

    exec(&mut s, B, &alpha(), T0 + 2 * SEC, mv(1, &t.key)).unwrap();
    let c = exec(&mut s, B, &alpha(), T0 + 3 * SEC, submit(1, "做完了")).unwrap();
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), ("review", None));

    // 名称带空白：命令里写标识，复制出去不会断开
    ui(&mut s, |w| w.update_status(B, &t.key, StatusPatch { name: Some("测试 中".into()), ..Default::default() })).unwrap();
    let hint = flow(&s).next_lock_hint(2, "doing").unwrap();
    assert!(hint.contains(&format!("先 board move 2 {}，", t.key)), "{hint}");
}

/// 做不下去：先去错误节点，没有时去需要人处理的状态；两个都没有时只评论、结束计时，50 秒后回到领取前的状态
#[test]
fn giveup_goes_to_error_node_then_human_status() {
    // 默认工作流没有错误节点：去「需人工校验」
    let mut s = store();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    assert_eq!(code(exec(&mut s, B, &beta(), T0, giveup(1, "卡住了"))), ErrorCode::NotHolder);
    let c = exec(&mut s, B, &alpha(), T0 + SEC, giveup(1, "需要确认：接口要不要兼容旧版")).unwrap();
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), ("check", None));
    assert!(c.activity.iter().any(|a| a.action == "comment" && a.text.contains("需要确认")));

    // 有错误节点：先去错误节点
    let mut s = store();
    let st = ui(&mut s, |w| w.create_status(B, "搁置", None)).unwrap();
    ui(&mut s, |w| w.update_edges(B, &[(String::from("doing"), st.key.clone())], &[])).unwrap();
    ui(&mut s, |w| w.update_status(B, &st.key, StatusPatch { err: Some(true), ..Default::default() })).unwrap();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    let c = exec(&mut s, B, &alpha(), T0 + SEC, giveup(1, "检查不过")).unwrap();
    assert_eq!(c.card.column, st.key);

    // 两个都没有：评论、final，卡片留在原处，50 秒后领取释放、回到待办
    let mut s = store();
    ui(&mut s, |w| w.delete_status(B, "check")).unwrap();
    exec(&mut s, B, &human(), T0, new("一")).unwrap();
    exec(&mut s, B, &alpha(), T0, claim(None)).unwrap();
    exec(&mut s, B, &alpha(), T0 + SEC, beat(1, false)).unwrap();
    let c = exec(&mut s, B, &alpha(), T0 + 2 * SEC, giveup(1, "卡住了")).unwrap();
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref(), c.card.run_started_at), ("doing", Some("alpha"), None));
    assert!(c.activity.iter().any(|a| a.action == "comment" && a.text.contains("卡住了")));
    s.sweep(T0 + 2 * SEC + 51 * SEC).unwrap();
    let c = card(&s, B, 1);
    assert_eq!((c.card.column.as_str(), c.card.holder.as_deref()), ("todo", None));
}
