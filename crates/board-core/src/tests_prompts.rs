//! 快捷提示词（#572）：整列读写、和界面一致的校验。

use crate::prompts::Prompt;
use crate::*;

const T0: i64 = 1_700_000_000_000;

fn p(id: &str, title: &str, text: &str) -> Prompt {
    Prompt { id: id.into(), title: title.into(), text: text.into() }
}

fn put(s: &mut Store, list: Vec<Prompt>) -> Result<Vec<Prompt>> {
    s.write(T0, Origin::ui(None), |w| w.prompts_put(list))
}

fn list(s: &Store) -> Vec<Prompt> {
    s.read(|r| r.prompts()).unwrap()
}

#[test]
fn empty_at_first_then_whole_list_replaced_in_order() {
    let mut s = Store::open_in_memory().unwrap();
    assert_eq!(list(&s), vec![]);

    let out = put(&mut s, vec![p("p-2", "重启", "重启一下"), p("p-1", "等发版", "等发版")]).unwrap();
    assert_eq!(out, vec![p("p-2", "重启", "重启一下"), p("p-1", "等发版", "等发版")]);
    assert_eq!(list(&s), out, "按写入的顺序读出，不按 id 排");

    // 整列替换：少掉的删掉，顺序照新的
    put(&mut s, vec![p("p-1", "等发版 ✅", "等发版"), p("p-3", "新", "x")]).unwrap();
    assert_eq!(list(&s), vec![p("p-1", "等发版 ✅", "等发版"), p("p-3", "新", "x")]);

    put(&mut s, vec![]).unwrap();
    assert_eq!(list(&s), vec![]);
}

#[test]
fn normalizes_like_the_ui() {
    let mut s = Store::open_in_memory().unwrap();
    let out = put(&mut s, vec![p(" p-1 ", "  等发版 ", "第一行\r\n第二行\r第三行")]).unwrap();
    assert_eq!(out, vec![p("p-1", "等发版", "第一行\n第二行\n第三行")]);
    assert_eq!(list(&s), out);
}

#[test]
fn rejects_bad_entries_and_keeps_the_old_list() {
    let mut s = Store::open_in_memory().unwrap();
    put(&mut s, vec![p("p-1", "等发版", "等发版")]).unwrap();
    let cases = [
        (vec![p("", "标题", "内容")], "id"),
        (vec![p("p-2", "  ", "内容")], "标题"),
        (vec![p("p-2", "标题", " \n ")], "内容"),
        (vec![p("p-2", "a", "x"), p("p-2", "b", "y")], "重复"),
    ];
    for (bad, word) in cases {
        let e = put(&mut s, bad).unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage);
        assert!(e.message.contains(word), "{}", e.message);
    }
    assert_eq!(list(&s), vec![p("p-1", "等发版", "等发版")], "被拒绝时整列不动");
}
