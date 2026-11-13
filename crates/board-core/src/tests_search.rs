//! 跨白板搜索（PRD §9 `board search`、§10.1 命令面板）：文字、`#ID`、`项目/白板#ID`、`label:标签`。

use crate::*;
use std::time::Instant;

const T0: i64 = 1_700_000_000_000;

fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.ensure_default(T0).unwrap();
    s
}

fn new_card(s: &mut Store, path: &str, title: &str, body: &str) -> i64 {
    s.write(T0, Origin::ui(None), |w| {
        let b = w.resolve(path)?;
        w.exec(&b, &Actor::human(), Cmd::New { title: title.into(), body: Some(body.into()), column: None, top: false, labels: vec![] })
            .map(|d| d.card.id)
    })
    .unwrap()
}

fn comment(s: &mut Store, path: &str, id: i64, text: &str) {
    s.write(T0, Origin::ui(None), |w| {
        let b = w.resolve(path)?;
        w.exec(&b, &Actor::human(), Cmd::Comment { id, text: text.into() }).map(|_| ())
    })
    .unwrap();
}

fn search(s: &Store, q: &str) -> Vec<SearchHit> {
    s.read(|r| r.search(q, SEARCH_LIMIT)).unwrap()
}

/// 多一块白板：default/other
fn other_board(s: &mut Store) {
    s.write(T0, Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None)).unwrap();
}

#[test]
fn text_search_covers_every_board_and_the_inbox() {
    let mut s = store();
    other_board(&mut s);
    let a = new_card(&mut s, "default/main", "登录按钮圆角", "");
    new_card(&mut s, "default/other", "设置页按钮圆角", "");
    let inbox = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve("default/main")?;
            w.exec(&b, &Actor::human(), Cmd::New { title: "按钮的说明".into(), body: None, column: Some(INBOX.into()), top: false, labels: vec![] })
                .map(|d| d.card.id)
        })
        .unwrap();
    let hits = search(&s, "圆角");
    let boards: Vec<(String, i64)> = hits.iter().map(|h| (h.board.clone(), h.id)).collect();
    assert_eq!(boards.len(), 2, "{boards:?}");
    assert!(boards.contains(&("default/main".into(), a)));
    assert!(boards.contains(&("default/other".into(), 1)));
    assert!(hits.iter().all(|h| h.matched == "title" && h.score == 100));

    // 「未整理」也在搜索范围里，状态名称是「未整理」
    let hits = search(&s, "的说明");
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].column.as_str(), hits[0].status.as_str()), (INBOX, INBOX_NAME));
    assert_eq!(hits[0].id, inbox);
}

#[test]
fn search_matches_body_and_comments_and_ignores_case() {
    let mut s = store();
    let c = new_card(&mut s, "default/main", "标题", "描述里写了 GH#140");
    let d = new_card(&mut s, "default/main", "另一个", "");
    comment(&mut s, "default/main", d, "评论里提到 Token 过期");
    let hits = search(&s, "gh#140");
    assert_eq!(hits.len(), 1, "描述命中");
    assert_eq!((hits[0].id, hits[0].matched.as_str(), hits[0].score), (c, "body", 40));
    let hits = search(&s, "token");
    assert_eq!(hits.len(), 1, "评论命中，大小写不敏感");
    assert_eq!((hits[0].id, hits[0].matched.as_str(), hits[0].score), (d, "comment", 25));
}

#[test]
fn title_hits_rank_before_body_hits_and_limit_is_50() {
    let mut s = store();
    for i in 0..60 {
        new_card(&mut s, "default/main", &format!("题 {i}"), "正文里提到圆角");
    }
    let c = new_card(&mut s, "default/main", "圆角", "");
    let hits = search(&s, "圆角");
    // 再加一张就是 61 张：最多只返回 SEARCH_LIMIT 条
    assert_eq!(hits.len(), SEARCH_LIMIT, "最多 {SEARCH_LIMIT} 条");
    assert_eq!(hits[0].id, c, "标题命中排在前面");
    assert_eq!(hits[0].matched, "title");
    assert!(hits[1..].iter().all(|h| h.score == 40));
    // limit 可以被参数压小
    assert_eq!(s.read(|r| r.search("圆角", 3)).unwrap().len(), 3);
}

#[test]
fn multiple_words_need_all_first_then_fall_back_to_any() {
    let mut s = store();
    let both = new_card(&mut s, "default/main", "登录按钮", "");
    let one = new_card(&mut s, "default/main", "按钮太大", "");
    let hits = search(&s, "登录 按钮");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, both);
    // 全命中没有结果时退回任一词命中
    let hits = search(&s, "登录 圆角");
    let ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
    assert_eq!(ids, vec![both]);
    assert_eq!(hits[0].matched, "any", "只命中了其中一部分词");
    let hits = search(&s, "登录 完全没有");
    assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), vec![both]);
    let hits = search(&s, "按钮 圆角");
    let ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
    assert!(ids.contains(&one) && ids.contains(&both), "{ids:?}");
    assert!(hits.iter().all(|h| h.matched == "any" && h.score == 10));
    // 没有结果时是空数组
    assert!(search(&s, "根本不存在").is_empty());
}

#[test]
fn id_forms_locate_cards_across_boards() {
    let mut s = store();
    other_board(&mut s);
    new_card(&mut s, "default/main", "主白板第一条", "");
    let b = new_card(&mut s, "default/other", "其他白板第一条", "");
    // 不同白板的编号各自从头开始：#1 有两张
    let hits = search(&s, "#1");
    let boards: Vec<&str> = hits.iter().map(|h| h.board.as_str()).collect();
    assert_eq!(boards, vec!["default/main", "default/other"]);
    assert!(hits.iter().all(|h| h.id == 1 && h.matched == "ref"));
    // 白板路径 + 编号：直接定位
    let hits = search(&s, "default/other#1");
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].board.as_str(), hits[0].id, hits[0].title.as_str()), ("default/other", b, "其他白板第一条"));
    assert_eq!(hits[0].id, b);
    // 白板不存在时明确报错
    let e = s.read(|r| r.search("nope/main#1", 10)).unwrap_err();
    assert_eq!(e.code, ErrorCode::BoardNotFound);
    // 编号不是数字
    let e = s.read(|r| r.search("default/main#abc", 10)).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
}

#[test]
fn label_form_finds_cards_with_that_label() {
    let mut s = store();
    let (bug, ui) = s
        .write(T0, Origin::ui(None), |w| {
            Ok((w.create_label("default", "bug", None, false)?.id, w.create_label("default", "ui", None, false)?.id))
        })
        .unwrap();
    let c = s
        .write(T0, Origin::ui(None), |w| {
            let b = w.resolve("default/main")?;
            let d = w.exec(
                &b,
                &Actor::human(),
                Cmd::New { title: "带 bug 的卡".into(), body: None, column: None, top: false, labels: vec![bug, ui] },
            )?;
            Ok(d.card.id)
        })
        .unwrap();
    new_card(&mut s, "default/main", "没有标签", "");
    let hits = search(&s, "label:bug");
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].id, hits[0].matched.as_str()), (c, "label"));
    assert_eq!(hits[0].labels, vec!["bug".to_string(), "ui".to_string()], "结果带标签名");
    assert!(search(&s, "label:BUG").len() == 1, "标签名不区分大小写");
    assert!(search(&s, "label:没有这个标签").is_empty());
    let e = s.read(|r| r.search("label:", 10)).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
}

#[test]
fn empty_query_is_rejected() {
    let s = store();
    assert_eq!(s.read(|r| r.search("   ", 10)).unwrap_err().code, ErrorCode::Usage);
}

/// 验收标准：1 万张卡片时一次搜索在 200 毫秒内。
///
/// 这是性能用例，不进普通 `cargo test`：`#[ignore]` 之后要单独跑 `cargo test -- --ignored search_is_fast`。
/// 1 万张卡片的建库本身就很慢，200 毫秒的阈值在 debug 构建、并行跑测试的机器上会随机失败。
/// 阈值按 release 构建定，debug 构建放宽（SQL 本身的扫描量没变，只是慢几倍）。
#[test]
#[ignore = "性能用例：1 万张卡片，`cargo test -- --ignored` 里跑"]
fn search_is_fast_with_ten_thousand_cards() {
    let mut s = store();
    other_board(&mut s);
    s.write(T0, Origin::ui(None), |w| {
        let b = w.resolve("default/main")?;
        let other = w.resolve("default/other")?;
        for i in 0..5000 {
            let title = format!("卡片 {i} 搜索");
            w.exec(&b, &Actor::human(), Cmd::New { title: title.clone(), body: Some(format!("第 {i} 张，正文里也提到关键词")), column: None, top: false, labels: vec![] })?;
            w.exec(&other, &Actor::human(), Cmd::New { title: format!("其他 {i}"), body: Some("无关".into()), column: None, top: false, labels: vec![] })?;
        }
        Ok(())
    })
    .unwrap();
    let n: i64 = s.read(|r| Ok(r.conn.query_row("SELECT COUNT(*) FROM cards", [], |x| x.get(0))?)).unwrap();
    assert_eq!(n, 10_000);
    // release 构建按验收标准的 200 毫秒；debug 构建放开到 2 秒，阈值抖动时不误报
    let limit = if cfg!(debug_assertions) { 2000 } else { 200 };
    for q in ["搜索", "#4321", "default/other#77", "卡片 4999"] {
        let t = Instant::now();
        let hits = search(&s, q);
        let ms = t.elapsed().as_millis();
        assert!(!hits.is_empty(), "搜「{q}」应该有结果");
        assert!(ms < limit, "搜「{q}」用了 {ms} 毫秒（上限 {limit}）");
    }
}
