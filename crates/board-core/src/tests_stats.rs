//! 大盘的验收标准（大盘需求「验收标准」，PRD §13）：流转记录的回填与写入、统计口径。

use crate::stats::{Scope, Stats, StatsQuery};
use crate::*;

const MIN: i64 = 60 * 1000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;
/// 2023-11-14 00:00:00 UTC（整天）
const BASE: i64 = 1_699_920_000_000;
const B: &str = "default/main";

fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.ensure_default(BASE - 30 * DAY).unwrap();
    s
}

fn exec(s: &mut Store, path: &str, who: &Actor, at: i64, cmd: Cmd) -> CardDetail {
    try_exec(s, path, who, at, cmd).unwrap()
}

fn try_exec(s: &mut Store, path: &str, who: &Actor, at: i64, cmd: Cmd) -> Result<CardDetail> {
    let origin = if who.is_human() { Origin::ui(None) } else { Origin::cli() };
    s.write(at, origin, |w| {
        let b = w.resolve(path)?;
        w.exec(&b, who, cmd)
    })
}

fn new(s: &mut Store, path: &str, at: i64, title: &str) -> i64 {
    exec(s, path, &Actor::human(), at, Cmd::New { title: title.into(), body: None, column: None, top: false, labels: vec![] }).card.id
}

fn mv(s: &mut Store, path: &str, who: &Actor, at: i64, id: i64, col: &str) {
    exec(s, path, who, at, Cmd::Move { id, column: col.into(), top: false, index: None, order: None, before: None, after: None });
}

/// agent 领取、每 40 秒一次心跳（50 秒没有心跳会被释放）、final、交审
fn work(s: &mut Store, path: &str, who: &str, id: i64, start: i64, end: i64) {
    let a = Actor::agent(who);
    exec(s, path, &a, start, Cmd::Claim { id: Some(id) });
    let mut t = start;
    while t < end {
        exec(s, path, &a, t, Cmd::Beat { id, done: false });
        t += 40_000;
    }
    exec(s, path, &a, end, Cmd::Beat { id, done: true });
    mv(s, path, &a, end, id, "review");
}

fn stats(s: &Store, scope: Scope, path: Option<&str>, from: i64, to: i64, tz: &str, now: i64) -> Stats {
    let q = StatsQuery { scope, path: path.map(String::from), from, to, tz: tz.into() };
    s.read(|r| r.stats(&q, now)).unwrap()
}

fn count(s: &Store, sql: &str) -> i64 {
    s.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

/// 验收 1：回填后 moves 里「移动」的行数等于 activity 里 action = 'move' 的行数；改过名的状态回填后 key 为 NULL、名称保留
#[test]
fn backfill_parses_every_move_and_keeps_renamed_names() {
    let mut s = store();
    let h = Actor::human();
    let a = new(&mut s, B, BASE, "一");
    let b = new(&mut s, B, BASE, "二");
    exec(&mut s, B, &h, BASE, Cmd::New { title: "三".into(), body: None, column: Some("inbox".into()), top: false, labels: vec![] });
    mv(&mut s, B, &h, BASE + MIN, 3, "todo"); // 整理好了，从「未整理」移到「待办」
    work(&mut s, B, "alpha", a, BASE + 2 * MIN, BASE + 10 * MIN);
    mv(&mut s, B, &h, BASE + 11 * MIN, a, "done");
    exec(&mut s, B, &Actor::agent("beta"), BASE + 12 * MIN, Cmd::Claim { id: Some(b) });
    exec(&mut s, B, &h, BASE + 12 * MIN + 30_000, Cmd::Release { id: b }); // 强制释放（50 秒内，还没被超时释放）
    // 删除状态引起的移动
    let key = s.write(BASE + 14 * MIN, Origin::ui(None), |w| w.create_status(B, "测试", None)).unwrap().key;
    mv(&mut s, B, &h, BASE + 15 * MIN, b, &key);
    s.write(BASE + 16 * MIN, Origin::ui(None), |w| w.delete_status(B, &key)).unwrap();
    // 把「已完成」改名：回填时按名称找不到「已完成」
    s.write(BASE + 17 * MIN, Origin::ui(None), |w| w.update_status(B, "done", StatusPatch { name: Some("完成了".into()), ..Default::default() }))
        .unwrap();

    let written = count(&s, "SELECT COUNT(*) FROM moves");
    assert!(written >= 7, "新写入的流转记录：{written}");
    // 当作旧数据库重新回填
    s.conn().execute_batch("DELETE FROM moves; DELETE FROM meta WHERE key = 'moves';").unwrap();
    crate::moves::backfill(s.conn()).unwrap();
    assert_eq!(
        count(&s, "SELECT COUNT(*) FROM moves WHERE kind = 'move'"),
        count(&s, "SELECT COUNT(*) FROM activity WHERE action = 'move'"),
        "三种写法都解析成功"
    );
    assert_eq!(count(&s, "SELECT COUNT(*) FROM moves"), written, "领取、释放也回填");
    let (key, name): (Option<String>, String) = s
        .conn()
        .query_row("SELECT to_key, to_name FROM moves WHERE card = ?1 AND to_name = '已完成'", [a], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((key, name.as_str()), (None, "已完成"));
    let (fk, fname): (Option<String>, Option<String>) = s
        .conn()
        .query_row("SELECT from_key, from_name FROM moves WHERE card = ?1 AND kind = 'release'", [b], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((fk.as_deref(), fname.as_deref()), (Some("doing"), Some("进行中")), "释放的原来状态从上一条推出");
    // 只跑一次
    crate::moves::backfill(s.conn()).unwrap();
    assert_eq!(count(&s, "SELECT COUNT(*) FROM moves"), written);
}

/// 验收 2：新的移动（含删除状态引起的移动）写入 moves
#[test]
fn every_status_change_is_logged() {
    let mut s = store();
    let h = Actor::human();
    let id = new(&mut s, B, BASE, "x");
    let key = s.write(BASE, Origin::ui(None), |w| w.create_status(B, "测试", None)).unwrap().key;
    mv(&mut s, B, &h, BASE + MIN, id, &key);
    s.write(BASE + 2 * MIN, Origin::ui(None), |w| w.delete_status(B, &key)).unwrap();
    let rows: Vec<(String, Option<String>, Option<String>, String)> = s
        .conn()
        .prepare("SELECT kind, from_key, to_key, from_name FROM moves ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1], ("move".into(), Some(key), Some("todo".into()), "测试".into()));
}

/// 验收 5：交付周期 1、2、3、10 小时：中位 3 小时、P85 10 小时；其中 1 张返工过：一次通过率 75%。
/// 验收 3：区间覆盖全部历史、没有卡片移出过终点时，完成等于终点状态的卡片数
#[test]
fn lead_time_quantiles_and_first_pass() {
    let mut s = store();
    let h = Actor::human();
    let ids: Vec<i64> = (0..4).map(|i| new(&mut s, B, BASE, &format!("卡片 {i}"))).collect();
    for (i, hours) in [1, 2, 3].iter().enumerate() {
        work(&mut s, B, &format!("a{i}"), ids[i], BASE + 10 * MIN, BASE + 30 * MIN);
        mv(&mut s, B, &h, BASE + hours * HOUR, ids[i], "done");
    }
    // 第 4 张：交审后被打回到「返工」，再做一次
    work(&mut s, B, "a3", ids[3], BASE + MIN, BASE + 20 * MIN);
    mv(&mut s, B, &h, BASE + HOUR, ids[3], "rework");
    work(&mut s, B, "a3", ids[3], BASE + 2 * HOUR, BASE + 3 * HOUR);
    mv(&mut s, B, &h, BASE + 10 * HOUR, ids[3], "done");

    let now = BASE + 2 * DAY;
    let st = stats(&s, Scope::Board, Some(B), BASE - DAY, BASE + DAY, "UTC", now);
    assert_eq!(st.cur.done, 4);
    assert_eq!(st.cur.lead_p50, Some(3 * HOUR));
    assert_eq!(st.cur.lead_p85, Some(10 * HOUR));
    assert_eq!(st.cur.rework, 1);
    assert_eq!(st.cur.first_pass, Some(0.75));
    assert_eq!(st.cur.claims, 5);
    // 1 小时内、1–2、2–4、4–8、8–24 小时、1–3 天、3 天以上
    assert_eq!(st.cur.lead_hist, vec![0, 1, 2, 0, 1, 0, 0]);
    assert_eq!(st.bucket, "hour");
    assert_eq!(st.series.iter().map(|b| b.done).sum::<i64>(), 4);
    // 第 4 张被打回那一次领取是「被打回」，第二次是「交审」
    let r4: Vec<&str> = st.runs.iter().filter(|r| r.card == ids[3]).map(|r| r.result).collect();
    assert_eq!(r4, vec!["rework", "review"]);
    // 卡片流转明细：最近完成的在前，分段从新建一直到完成
    assert_eq!(st.lives[0].card, ids[3]);
    assert_eq!(st.lives[0].reworks, 1);
    assert_eq!(st.lives[0].segs.first().map(|x| x.from), Some(BASE));
    assert_eq!(st.lives[0].segs.last().map(|x| x.to), Some(BASE + 10 * HOUR));
    // 上一周期没有数据：比例为空，不是 NaN
    assert_eq!(st.prev.first_pass, None);
    assert_eq!(st.prev.lead_p50, None);
    // 验收 3
    let all = stats(&s, Scope::Board, Some(B), BASE - 30 * DAY, now, "UTC", now);
    let finals = count(&s, "SELECT COUNT(*) FROM cards WHERE col = 'done'");
    assert_eq!(all.cur.done, finals);
    assert_eq!(all.bucket, "day");
    let life = all.life.unwrap();
    assert_eq!((life.cards, life.finals, life.reworks, life.agents), (4, 4, 1, 4));
}

/// 验收 4：同一区间 UTC 与 Asia/Shanghai 的分桶整体错开 8 小时，时段热力图也错开
#[test]
fn buckets_follow_the_time_zone() {
    let mut s = store();
    let id = new(&mut s, B, BASE + 2 * HOUR, "x");
    exec(&mut s, B, &Actor::human(), BASE + 2 * HOUR, Cmd::Comment { id, text: "看一下".into() });
    let now = BASE + 3 * DAY;
    let utc = stats(&s, Scope::Board, Some(B), BASE, BASE + 7 * DAY, "UTC", now);
    let sh = stats(&s, Scope::Board, Some(B), BASE, BASE + 7 * DAY, "Asia/Shanghai", now);
    assert_eq!(utc.bucket, "day");
    assert_eq!(utc.series[0].t, BASE);
    assert_eq!(sh.series[0].t, BASE - 8 * HOUR);
    assert!(utc.series.iter().zip(&sh.series).all(|(a, b)| a.t - b.t == 8 * HOUR));
    // 2023-11-14 是周二：UTC 2 点，上海 10 点
    assert_eq!(utc.hour_heat[1][2], 2);
    assert_eq!(sh.hour_heat[1][10], 2);
    // 小时桶：上海整点和 UTC 整点是同一批时刻，标签由界面按时区显示
    let h1 = stats(&s, Scope::Board, Some(B), BASE, BASE + DAY, "Asia/Shanghai", now);
    assert_eq!(h1.bucket, "hour");
    assert_eq!(h1.series.len(), 24);
    assert_eq!(h1.series[2].created, 1);
    // 日历热力图：26 周，周一开始
    assert_eq!(utc.day_heat.len(), 26 * 7);
    let first = jiff::Timestamp::from_millisecond(utc.day_heat[0].t).unwrap().to_zoned(jiff::tz::TimeZone::UTC);
    assert_eq!(first.weekday(), jiff::civil::Weekday::Monday);
    assert_eq!(utc.day_heat.iter().map(|d| d.v).sum::<i64>(), 2);
    let bad = s.read(|r| r.stats(&StatsQuery { scope: Scope::Board, path: Some(B.into()), from: BASE, to: BASE + DAY, tz: "Mars/Olympus".into() }, now));
    assert_eq!(bad.unwrap_err().code, ErrorCode::Usage);
}

/// 验收 6：可领取状态有卡、30 分钟没有任何领取：空转 30 分钟；只空 10 分钟时不计
#[test]
fn idle_counts_gaps_of_fifteen_minutes_or_more() {
    let mut s = store();
    let a = new(&mut s, B, BASE, "一");
    // 一建好 10 分钟就有人领：不计
    work(&mut s, B, "alpha", a, BASE + 10 * MIN, BASE + 20 * MIN);
    // 第二张 BASE+20 建好，30 分钟后才被领
    let b = new(&mut s, B, BASE + 20 * MIN, "二");
    work(&mut s, B, "alpha", b, BASE + 50 * MIN, BASE + 60 * MIN);
    let now = BASE + DAY;
    let st = stats(&s, Scope::Board, Some(B), BASE, BASE + 2 * HOUR, "UTC", now);
    assert_eq!(st.cur.idle, 30 * MIN);
    assert_eq!(st.idle.len(), 1);
    assert_eq!((st.idle[0].from, st.idle[0].to), (BASE + 20 * MIN, BASE + 50 * MIN));
}

/// 验收 7：项目的「完成」等于其下各白板之和，全部项目等于各项目之和
#[test]
fn project_and_all_add_up() {
    let mut s = store();
    s.write(BASE, Origin::ui(None), |w| {
        w.create_board("default", "第二块", "second", "", None)?;
        w.create_project("别的项目", "other", "", None, None, NewBoard { name: "主".into(), key: "main".into(), desc: String::new(), pos: None })
    })
    .unwrap();
    let h = Actor::human();
    for (path, n) in [(B, 2), ("default/second", 1), ("other/main", 3)] {
        for i in 0..n {
            let id = new(&mut s, path, BASE, &format!("{path} {i}"));
            work(&mut s, path, &format!("w{i}"), id, BASE + MIN, BASE + 5 * MIN);
            mv(&mut s, path, &h, BASE + 10 * MIN, id, "done");
        }
    }
    let now = BASE + DAY;
    let p = stats(&s, Scope::Project, Some("default"), BASE, BASE + HOUR, "UTC", now);
    assert_eq!(p.cur.done, 3);
    assert_eq!(p.boards_in_scope, vec!["default/main", "default/second"]);
    let all = stats(&s, Scope::All, None, BASE, BASE + HOUR, "UTC", now);
    assert_eq!(all.cur.done, 6);
    let by: std::collections::HashMap<&str, i64> = all.boards.iter().map(|b| (b.key.as_str(), b.done)).collect();
    assert_eq!((by["default/main"], by["default/second"], by["other/main"]), (2, 1, 3));
    let pj: std::collections::HashMap<&str, i64> = all.projects.iter().map(|b| (b.key.as_str(), b.done)).collect();
    assert_eq!((pj["default"], pj["other"], all.all.done), (3, 3, 6));
    assert!(all.life.is_none());
    assert!(p.life.is_some());
}

/// 验收 11、12：派发启动的 claude/mbp-1、claude/mbp-2 合并成一行 claude/mbp；心跳中断的那次是异常退出
#[test]
fn dispatch_names_merge_and_lost_runs() {
    let mut s = store();
    let a = new(&mut s, B, BASE, "一");
    let b = new(&mut s, B, BASE, "二");
    work(&mut s, B, "claude/mbp-1", a, BASE + MIN, BASE + 10 * MIN);
    // 第二张：开始心跳后就断了
    let x = Actor::agent("claude/mbp-2");
    exec(&mut s, B, &x, BASE + MIN, Cmd::Claim { id: Some(b) });
    exec(&mut s, B, &x, BASE + MIN + 30_000, Cmd::Beat { id: b, done: false });
    s.sweep(BASE + 10 * MIN).unwrap();
    for (name, card) in [("claude/mbp-1", a), ("claude/mbp-2", b)] {
        s.conn()
            .execute(
                "INSERT INTO dispatches (board, card, title, host_id, host, agent, agent_label, name, source, by_kind, by_name, state, created_at)
                 VALUES (1, ?1, 't', 1, 'mbp', 'claude', 'Claude Code', ?2, 'manual', 'human', '', 'done', 0)",
                rusqlite::params![card, name],
            )
            .unwrap();
    }
    let st = stats(&s, Scope::Board, Some(B), BASE, BASE + HOUR, "UTC", BASE + DAY);
    assert_eq!(st.agents.len(), 1);
    assert_eq!(st.agents[0].agent, "claude/mbp");
    assert_eq!((st.agents[0].claims, st.agents[0].lost), (2, 1));
    let lost: Vec<&str> = st.runs.iter().filter(|r| r.card == b).map(|r| r.result).collect();
    assert_eq!(lost, vec!["lost"]);
    assert_eq!(st.runs.iter().find(|r| r.card == a).map(|r| (r.agent.as_str(), r.name.as_str())), Some(("claude/mbp", "claude/mbp-1")));
}

/// 验收 13 的数据：待复盘最久的一张等了多久；正在卡着的卡按停的时间排
#[test]
fn present_reports_the_oldest_waits() {
    let mut s = store();
    let a = new(&mut s, B, BASE, "一");
    let b = new(&mut s, B, BASE + HOUR, "二");
    exec(&mut s, B, &Actor::human(), BASE + 2 * HOUR, Cmd::Comment { id: b, text: "\n  缺少线上配置的权限\n第二行".into() });
    work(&mut s, B, "alpha", a, BASE + HOUR, BASE + 2 * HOUR);
    let now = BASE + 11 * HOUR;
    let st = stats(&s, Scope::Board, Some(B), now - DAY, now, "UTC", now);
    assert_eq!((st.present.review.n, st.present.review.oldest), (1, Some(9 * HOUR)));
    assert_eq!((st.present.todo.n, st.present.todo.oldest), (1, Some(10 * HOUR)));
    assert_eq!(st.present.doing.n, 0);
    assert_eq!(st.present.stuck.iter().map(|x| x.card).collect::<Vec<_>>(), vec![b, a]);
    assert_eq!(st.present.stuck[0].note.as_deref(), Some("缺少线上配置的权限"));
    let counts: Vec<(&str, i64)> = st.present.statuses.iter().map(|x| (x.key.as_str(), x.n)).collect();
    assert_eq!(&counts[..3], &[("inbox", 0), ("todo", 1), ("doing", 0)]);
}

/// 验收 9：没有任何动态的区间：数字都是 0 或空，序列化出来没有 NaN
#[test]
fn empty_range_has_no_nan() {
    let mut s = store();
    new(&mut s, B, BASE, "一");
    let st = stats(&s, Scope::Board, Some(B), BASE + 10 * DAY, BASE + 11 * DAY, "UTC", BASE + 20 * DAY);
    assert_eq!(st.activity, 0);
    let json = serde_json::to_string(&st).unwrap();
    assert!(!json.contains("NaN") && !json.contains("Infinity"));
    assert_eq!(st.cur.first_pass, None);
}

/// 白板说明：保存、清空；说明里引用的附件不会被 24 小时清理
#[test]
fn readme_is_saved_and_keeps_its_images() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("b.db")).unwrap();
    s.ensure_default(BASE).unwrap();
    let b = s.read(|r| r.resolve(B)).unwrap();
    let png = b"\x89PNG\r\n\x1a\nxxxx";
    let att = s.attach(BASE, &b, None, &Actor::human(), "a.png", png).unwrap();
    let other = s.attach(BASE, &b, None, &Actor::human(), "b.png", b"\x89PNG\r\n\x1a\nyyyy").unwrap();
    let text = format!("# 说明\n\n![a]({})", crate::files::url_of(att.id));
    s.write(BASE, Origin::ui(None), |w| w.update_board(B, None, None, Some(&text))).unwrap();
    let ws = s.read(|r| r.workspace(BASE)).unwrap();
    assert_eq!(ws.projects[0].boards[0].readme, text);
    s.gc_files(BASE + 2 * DAY).unwrap();
    assert!(s.attachment(att.id).unwrap().is_some(), "说明引用的图片留着");
    assert!(s.attachment(other.id).unwrap().is_none(), "没被引用的照常清理");
    s.write(BASE, Origin::ui(None), |w| w.update_board(B, None, None, Some(""))).unwrap();
    assert_eq!(s.read(|r| r.workspace(BASE)).unwrap().projects[0].boards[0].readme, "");
}
