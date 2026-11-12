//! `board search`（PRD §9）：不需要白板，一次在所有白板里找。

mod common;

use common::start;

const ALPHA: &str = "test/alpha";

fn hits(r: &common::Run) -> Vec<serde_json::Value> {
    r.json().as_array().unwrap().clone()
}

#[test]
fn search_crosses_boards_without_board_env() {
    let s = start();
    // 先让 ALPHA 这个名字上线（s.run 会替它 check），在主白板上建一张卡
    s.run(ALPHA, &["new", "登录按钮圆角", "--body", "主白板的卡"]);
    // 再加一块白板：default/other（项目里有两块白板了，BOARD 要写全路径）
    board_core::Store::open(&s.db)
        .unwrap()
        .write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.create_board("default", "其他", "other", "", None))
        .unwrap();
    // search 需要名字（--as / BOARD_AS），但不需要白板
    let mut me = s.env();
    me.push(("BOARD_AS", ALPHA.into()));
    let mut other = s.env();
    other.push(("BOARD_AS", ALPHA.into()));
    other.push(("BOARD", "default/other".into()));
    let mut main = me.clone();
    main.push(("BOARD", "default/main".into()));
    let r = s.raw(&other, &["new", "设置页按钮圆角", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);

    // 不带 BOARD 也能搜：两块白板的卡片都在结果里，每条带白板路径、状态、标签、更新时间
    let r = s.raw(&me, &["search", "圆角", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let list = hits(&r);
    let boards: Vec<&str> = list.iter().map(|h| h["board"].as_str().unwrap()).collect();
    assert_eq!(boards.len(), 2, "{list:?}");
    assert!(boards.contains(&"default/main") && boards.contains(&"default/other"));
    for k in ["board", "id", "column", "status", "title", "labels", "updated_at", "matched"] {
        assert!(list[0].get(k).is_some(), "结果里没有 {k}：{}", list[0]);
    }

    // 项目/白板#ID：直接定位
    let r = s.raw(&me, &["search", "default/other#1", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let list = hits(&r);
    assert_eq!(list.len(), 1);
    assert_eq!((list[0]["id"].as_i64().unwrap(), list[0]["matched"].as_str().unwrap()), (1, "ref"));

    // #ID：所有白板里编号相同的那几张（两块白板各自从头编号）
    let r = s.raw(&me, &["search", "#1", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(hits(&r).len(), 2);

    // label:标签
    s.create_label("bug");
    let r = s.raw(&main, &["label", "1", "+bug", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let r = s.raw(&me, &["search", "label:bug", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let list = hits(&r);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["labels"][0], "bug");

    // --limit
    assert_eq!(hits(&s.raw(&me, &["search", "圆角", "--limit", "1", "--json"])).len(), 1);

    // 文本模式：写出白板路径、ID、状态、标题
    let r = s.raw(&me, &["search", "圆角"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("default/main#1"), "{}", r.out);
    assert!(r.out.contains("待办"), "{}", r.out);
    assert!(r.out.contains("登录按钮圆角"), "{}", r.out);
    assert!(r.out.contains("default/other#1"), "{}", r.out);

    // 空关键字被拒绝；没有结果时是空列表（文本模式给一句说明）
    let r = s.raw(&me, &["search", "  ", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = s.raw(&me, &["search", "根本没有这个", "--json"]);
    assert_eq!(r.code, 0);
    assert!(hits(&r).is_empty());
    let r = s.raw(&me, &["search", "根本没有这个"]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("没有找到"), "{}", r.out);
}
