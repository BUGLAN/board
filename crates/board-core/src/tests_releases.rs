//! 里程碑（PRD §10.15）：版本记录的同步（board versions sync）和里程碑页的分组。

use crate::releases::{Group, ReleaseCommit, ReleaseSync, NONE_LIMIT};
use crate::*;

const T0: i64 = 1_700_000_000_000;
const H: i64 = 3_600_000;

fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.ensure_default(T0).unwrap();
    s
}

fn new_card(s: &mut Store, at: i64, title: &str) -> i64 {
    s.write(at, Origin::ui(None), |w| {
        let b = w.resolve("default/main")?;
        w.exec(&b, &Actor::human(), Cmd::New { title: title.into(), body: None, column: None, top: false, labels: vec![], from: None })
            .map(|d| d.card.id)
    })
    .unwrap()
}

fn mv(s: &mut Store, at: i64, id: i64, col: &str) {
    s.write(at, Origin::ui(None), |w| {
        let b = w.resolve("default/main")?;
        w.exec(&b, &Actor::human(), Cmd::Move { id, column: col.into(), top: false, index: None, order: None, before: None, after: None })
            .map(|_| ())
    })
    .unwrap();
}

fn commit(h: &str, at: i64, cards: &[&str]) -> ReleaseCommit {
    ReleaseCommit { h: h.into(), at, s: format!("feat: {h}"), cards: cards.iter().map(|c| c.to_string()).collect() }
}

fn sync(s: &mut Store, at: i64, name: &str, commits: Vec<ReleaseCommit>) -> Result<releases::SyncResult> {
    s.write(at, Origin::cli(), |w| w.sync_release(ReleaseSync { project: "default".into(), name: name.into(), released_at: at, commits }))
}

fn labels_of(s: &Store, id: i64) -> Vec<String> {
    s.read(|r| {
        let b = r.resolve("default/main")?;
        let d = r.card(&b, id)?;
        let all = r.workspace(T0)?.projects.into_iter().find(|p| p.key == "default").unwrap().labels;
        Ok(d.card.labels.iter().filter_map(|l| all.iter().find(|x| x.id == *l).map(|x| x.name.clone())).collect())
    })
    .unwrap()
}

#[test]
fn sync_creates_label_and_tags_cards() {
    let mut s = store();
    let a = new_card(&mut s, T0, "做完的需求");
    let b = new_card(&mut s, T0, "另一张");
    let r = sync(&mut s, T0 + H, "v0.1.0", vec![commit("aaa", T0, &[&format!("main#{a}")]), commit("bbb", T0, &[]), commit("ccc", T0, &["main#999", "nope#1"])]).unwrap();
    assert!(r.created);
    assert_eq!(r.label.name, "v0.1.0");
    assert_eq!(r.label.color, releases::VERSION_COLOR);
    assert_eq!(r.added, vec![format!("main#{a}")]);
    assert_eq!(r.missing, vec!["main#999".to_string(), "nope#1".to_string()]);
    assert_eq!(r.label.uses, 1);
    assert_eq!(labels_of(&s, a), vec!["v0.1.0"]);
    assert!(labels_of(&s, b).is_empty());
    // 系统打的标签，动态里写「添加了标签「v0.1.0」」
    let log = s.read(|r| r.card(&r.resolve("default/main")?, a)).unwrap().activity;
    assert!(log.iter().any(|e| e.actor.kind == ActorKind::System && e.text.contains("添加了标签") && e.text.contains("v0.1.0")), "{log:?}");

    // 再同步一次：标签不重复建、不重复打；记录换成新的
    let r = sync(&mut s, T0 + 2 * H, "V0.1.0", vec![commit("aaa", T0, &[&format!("main#{a}")]), commit("ddd", T0 + H, &[&format!("main#{b}")])]).unwrap();
    assert!(!r.created);
    assert_eq!(r.already, 1);
    assert_eq!(r.added, vec![format!("main#{b}")]);
    let m = s.read(|r| r.milestones("default")).unwrap();
    assert_eq!(m.versions.len(), 1);
    assert_eq!(m.versions[0].released_at, Some(T0 + 2 * H));
    assert_eq!(m.versions[0].commits.iter().map(|c| c.h.as_str()).collect::<Vec<_>>(), vec!["ddd", "aaa"]);
}

#[test]
fn sync_rejects_bad_names_and_projects() {
    let mut s = store();
    assert!(sync(&mut s, T0, "release-1", vec![]).is_err());
    let e = s
        .write(T0, Origin::cli(), |w| w.sync_release(ReleaseSync { project: "nope".into(), name: "v1".into(), released_at: T0, commits: vec![] }))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::BoardNotFound);
}

#[test]
fn milestones_groups_cards() {
    let mut s = store();
    // 发版前做完的：一张写了卡片行（进 v0.1.0），一张没写（没有版本）
    let shipped = new_card(&mut s, T0, "发了的");
    let untraced = new_card(&mut s, T0, "没写卡片行的");
    mv(&mut s, T0 + H, shipped, "done");
    mv(&mut s, T0 + H, untraced, "done");
    sync(&mut s, T0 + 2 * H, "v0.1.0", vec![commit("aaa", T0 + H, &[&format!("main#{shipped}")])]).unwrap();
    // 发版之后：做完的、进行中的、还在待办的、未整理里的
    let after = new_card(&mut s, T0 + 3 * H, "发版之后做完的");
    mv(&mut s, T0 + 4 * H, after, "done");
    // 锁定状态只能由 agent 领取进入：用「待审核」代表已经开始、还没做完
    let doing = new_card(&mut s, T0 + 3 * H, "在审的");
    mv(&mut s, T0 + 4 * H, doing, "review");
    let todo = new_card(&mut s, T0 + 3 * H, "还没开始的");
    let inbox = new_card(&mut s, T0 + 3 * H, "想法");
    mv(&mut s, T0 + 4 * H, inbox, INBOX);
    // 返工也是可领取状态：只有人把卡片打回来才会在里面
    let rework = new_card(&mut s, T0 + 3 * H, "打回来的");
    mv(&mut s, T0 + 4 * H, rework, "rework");
    // 手动建的下一个版本标签（还没打 tag）
    s.write(T0, Origin::ui(None), |w| w.create_label("default", "v0.2.0", None, false).map(|_| ())).unwrap();

    let m = s.read(|r| r.milestones("default")).unwrap();
    assert_eq!(m.versions.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), vec!["v0.2.0", "v0.1.0"]);
    assert_eq!(m.versions[0].released_at, None);
    assert_eq!(m.last_release, Some(T0 + 2 * H));
    let group = |id: i64| m.cards.iter().find(|c| c.id == id).map(|c| c.group);
    assert_eq!(group(shipped), Some(Group::Version));
    assert_eq!(group(untraced), Some(Group::None));
    assert_eq!(group(after), Some(Group::Next));
    assert_eq!(group(doing), Some(Group::Next));
    // 待办、返工（可领取状态）里的也是这一轮的活儿：算「未发版」，一处都不能漏（default/main#313）
    assert_eq!(group(todo), Some(Group::Next));
    assert_eq!(group(rework), Some(Group::Next));
    assert_eq!(group(inbox), None);
    // 终点状态里的卡片带着移入时间（= 做完的时间）
    assert_eq!(m.cards.iter().find(|c| c.id == shipped).unwrap().moved_at, Some(T0 + H));
}

#[test]
fn milestones_without_releases_and_limits() {
    let mut s = store();
    for i in 0..(NONE_LIMIT + 5) {
        let id = new_card(&mut s, T0, &format!("老卡 {i}"));
        mv(&mut s, T0 + i as i64, id, "done");
    }
    // 还没有任何版本记录：终点里的都算「没有版本」，最多 NONE_LIMIT 张，最近做完的在前
    let m = s.read(|r| r.milestones("default")).unwrap();
    assert!(m.versions.is_empty());
    assert_eq!(m.last_release, None);
    let none: Vec<_> = m.cards.iter().filter(|c| c.group == Group::None).collect();
    assert_eq!(none.len(), NONE_LIMIT);
    assert!(none[0].moved_at > none[1].moved_at);
    assert!(s.read(|r| r.milestones("nope")).is_err());
}
