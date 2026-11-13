//! agent 名字与 agent 级心跳（PRD §3）：check 的格式与查重、严格模式、`board beat`（不带 ID）维持名字和持有的卡片、
//! `board beat --final` 让名字立刻离线。按时间离线（50 秒）的规则在 board-core 的测试里用模拟时间验证。

mod common;

use common::{board, start, Run, Server};

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

/* ------------------------------ 名字由 Board 生成（board check --agent <标识>） ------------------------------ */

use board_core::{HostAgent, HostDir, HostReport};

/// 这台机器（win）上找得到 claude（模型 opus-5.5）、dsh（模型 deepseek-v4.1-flash）、pi（没配模型）
fn report_win(instance: &str) -> HostReport {
    let agent = |id: &str, model: &str| HostAgent { id: id.into(), name: id.into(), version: "1.0".into(), found: true, model: model.into(), ..Default::default() };
    HostReport {
        name: "win".into(),
        instance: instance.into(),
        resume: None,
        os: "Windows 11".into(),
        version: "0.1.24".into(),
        agents: vec![agent("claude", "opus-5.5"), agent("dsh", "deepseek-v4.1-flash"), agent("pi", "")],
        envs: vec![],
        workspace: Some(board_core::HostWorkspace { path: "C:\\repo".into(), ..Default::default() }),
        dirs: vec![HostDir { board: "default/main".into(), path: "C:\\repo\\default\\main".into(), ..Default::default() }],
        dirs_rev: 0,
        running: vec![],
        done: false,
        asked: None,
        ask_error: None,
        selftest: false,
        skills: None,
        mcp: None,
        providers: None,
        local_skills: None,
    }
}

/// 以 `win` 这台机器（board host 用它上报、用 `token` 访问令牌）登记一台在线机器，返回它用的令牌
fn register_win(s: &Server) -> String {
    use board_core::{now_ms, Origin};
    let mut store = board_core::Store::open(&s.db).unwrap();
    let (info, token) = board_server::auth::create_token(store.conn(), "win2", now_ms()).unwrap();
    store.write(now_ms(), Origin::cli(), |w| w.host_beat(&report_win("i1"), Some(info.id))).unwrap();
    token
}

#[test]
fn check_agent_gets_the_name_board_generates_and_remembers_it() {
    let s = start();
    let win = register_win(&s);
    // 本机配置的机器名 win：claude 配了 opus-5.5
    let env = vec![("BOARD_URL", s.url.clone()), ("BOARD_TOKEN", win.clone()), ("BOARD_HOST_NAME", "win".into())];
    let r = board(&s.home, &env, &["check", "--agent", "claude", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["name"], "claude-win/opus-5.5");
    assert_eq!(r.json()["modeled"], true);
    // 同一个 agent 再 check 一个会话（另一个 agent 进程）：自动加 -2，不报 NAME_TAKEN
    let r = board(&s.home, &env, &["check", "--agent", "claude", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["name"], "claude-win/opus-5.5-2");
    // pi 没配模型：pi-win/pi
    assert_eq!(board(&s.home, &env, &["check", "--agent", "pi", "--json"]).json()["name"], "pi-win/pi");
    // 不带 --as 的命令用 check 记下的名字（最近的那个）：持有、心跳、评论都对得上
    let mut no_as = env.clone();
    no_as.push(("BOARD", "default".into()));
    let r = board(&s.home, &no_as, &["new", "一", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(board(&s.home, &no_as, &["claim", "1", "--json"]).code, 0);
    let r = board(&s.home, &no_as, &["beat", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["cards"][0]["id"], 1);
    let c = board(&s.home, &no_as, &["show", "1", "--json"]).json();
    assert_eq!(c["holder"], "pi-win/pi");
    // 带了 --as 的照旧：自己起名字，被占用时 NAME_TAKEN，不加序号
    assert_eq!(board(&s.home, &env, &["check", "--as", "pi-win/pi", "--json"]).error_code(), "NAME_TAKEN");
}

#[test]
fn check_agent_without_a_host_falls_back_to_agent_slash_model() {
    let s = start();
    // 没有机器上报、机器名也不存在：退回 <标识>/<标识>
    let r = board(&s.home, &s.env(), &["check", "--agent", "pi", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["name"], "pi/pi");
    assert_eq!(r.json()["modeled"], false);
    // 机器名不存在时也是（不报 NO_SUCH_HOST）
    let mut env = s.env();
    env.push(("BOARD_HOST_NAME", "nope".into()));
    assert_eq!(board(&s.home, &env, &["check", "--agent", "codex", "--json"]).json()["name"], "codex/codex");
}

#[test]
fn check_agent_requires_agent_or_name() {
    let s = start();
    let r = board(&s.home, &s.env(), &["check", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("--agent"), "{}", r.out);
    let r = board(&s.home, &s.env(), &["check", "--agent", "Claude Code", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
}
