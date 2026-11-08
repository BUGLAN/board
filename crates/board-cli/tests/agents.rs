//! agent 名字与 agent 级心跳（PRD §3）：check 的格式与查重、严格模式、`board beat`（不带 ID）维持名字和持有的卡片、
//! `board beat --final` 让名字立刻离线。按时间离线（50 秒）的规则在 board-core 的测试里用模拟时间验证。

mod common;

use common::{board, start, Run};

const ME: &str = "test/alpha";

#[test]
fn check_name_format_and_strict_mode() {
    let s = start();
    let long = format!("a/{}", "b".repeat(47));
    for bad in ["claude", "Claude/opus", "a/b/c", long.as_str()] {
        let r = board(&s.home, &s.env(), &["check", "--as", bad, "--json"]);
        assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"), "{bad}");
        assert!(r.json()["error"]["message"].as_str().unwrap().contains("<运行环境>/<模型>"), "要给出格式说明");
    }
    // 没 check 过的名字：卡片命令被拒绝；读命令不检查名字
    let r = s.run_unchecked(ME, &["new", "x", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NAME_OFFLINE"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("board check --as test/alpha"));
    assert_eq!(s.run_unchecked(ME, &["ls", "--json"]).code, 0);
    // check 之后可以；文本输出提示启动心跳
    let r = board(&s.home, &s.env(), &["check", "--as", ME]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("已在线") && r.out.contains("board heartbeat start"), "{}", r.out);
    assert_eq!(s.run_unchecked(ME, &["new", "x", "--json"]).code, 0);
    // 在线时再 check：NAME_TAKEN
    let r = board(&s.home, &s.env(), &["check", "--as", ME, "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NAME_TAKEN"));
    assert_eq!(r.json()["error"]["detail"]["name"], ME);
}

#[test]
fn concurrent_check_only_one_wins() {
    let s = start();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let (home, env) = (s.home.clone(), s.env());
            std::thread::spawn(move || board(&home, &env, &["check", "--as", "dsh/deepseek-v4.1-flash", "--json"]))
        })
        .collect();
    let runs: Vec<Run> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(runs.iter().filter(|r| r.code == 0).count(), 1, "{:?}", runs.iter().map(|r| &r.out).collect::<Vec<_>>());
    assert!(runs.iter().filter(|r| r.code != 0).all(|r| r.error_code() == "NAME_TAKEN"));
}

#[test]
fn agent_beat_keeps_held_cards_and_final_goes_offline() {
    let s = start();
    s.run(ME, &["new", "一"]);
    assert_eq!(s.run(ME, &["claim", "1", "--json"]).code, 0);
    // 不带 ID 的 beat 不需要白板
    let mut env = s.env();
    env.push(("BOARD_AS", ME.into()));
    let beat = |args: &[&str]| board(&s.home, &env, args);

    let r = beat(&["beat", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let v = r.json();
    assert_eq!((v["cards"][0]["board"].as_str(), v["cards"][0]["id"].as_i64()), (Some("default/main"), Some(1)));
    assert!(v["cards"][0]["run_started_at"].is_string(), "领取后的第一次心跳开始这一次计时");
    let r = beat(&["beat"]);
    assert!(r.out.contains("心跳") && r.out.contains("default/main#1") && r.out.contains("本次"), "{}", r.out);
    let c = s.run(ME, &["show", "1", "--json"]).json();
    assert_eq!((c["holder"].as_str(), c["work"]["runs"].as_i64(), c["work"]["beats"].as_i64()), (Some(ME), Some(1), Some(2)));
    assert!(c["activity"].as_array().unwrap().iter().any(|a| a["text"].as_str().unwrap().contains("开始第 1 次")));

    // final 之后后台心跳不再刷新这张卡片，再发 board beat <ID> 被拒绝
    assert_eq!(s.run(ME, &["beat", "1", "--final", "--json"]).code, 0);
    let v = beat(&["beat", "--json"]).json();
    assert_eq!(v["cards"][0]["beats"], 3, "final 计一次，之后的 agent 级心跳不计");
    assert_eq!(s.run(ME, &["beat", "1", "--json"]).error_code(), "USAGE");
    assert_eq!(s.run(ME, &["move", "1", "review", "--json"]).code, 0);

    // board beat --final：名字立刻离线；之后心跳、卡片命令都是 NAME_OFFLINE；可以重新 check
    let r = beat(&["beat", "--final"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("已离线"), "{}", r.out);
    assert_eq!(beat(&["beat", "--json"]).error_code(), "NAME_OFFLINE");
    assert_eq!(s.run_unchecked(ME, &["comment", "1", "x", "--json"]).error_code(), "NAME_OFFLINE");
    assert_eq!(s.check(ME).code, 0);
    assert_eq!(s.run_unchecked(ME, &["comment", "1", "x", "--json"]).code, 0);
}
