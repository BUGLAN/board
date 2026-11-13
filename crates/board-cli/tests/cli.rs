//! CLI 层面的验收标准（PRD §13）：身份与白板定位、退出码、--json、连接配置。
//! 所有命令都经测试服务端执行（tests/common）。

mod common;

use common::{start, Server};

const ALPHA: &str = "test/alpha";

#[test]
fn ac11_json_and_exit_codes() {
    let s = start();
    for args in [
        vec!["new", "x", "--json"],
        vec!["ls", "--json"],
        vec!["show", "1", "--json"],
        vec!["claim", "--json"],
        vec!["comment", "1", "进展", "--json"],
        vec!["edit", "1", "--body", "b", "--json"],
        vec!["move", "1", "review", "--json"],
        vec!["boards", "--json"],
        vec!["labels", "--json"],
    ] {
        let r = s.run(ALPHA, &args);
        assert_eq!(r.code, 0, "{args:?}: {}{}", r.out, r.err);
        r.json();
    }
    // 被拒绝：退出码 1，JSON 带错误码
    let r = s.run(ALPHA, &["move", "1", "doing", "--json"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.error_code(), "FORBIDDEN_COLUMN");
    let r = s.run(ALPHA, &["show", "99", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_FOUND"));
    let r = s.run(ALPHA, &["frobnicate", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = s.run(ALPHA, &["move", "1", "nowhere", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = s.run(ALPHA, &["comment", "1", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    // 文本模式：错误码写在 stderr
    let r = s.run(ALPHA, &["move", "1", "doing"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("FORBIDDEN_COLUMN"), "{}", r.err);
    // help 不需要身份
    assert_eq!(s.raw(&[], &["help"]).code, 0);
}

#[test]
fn ac1_claim_json_returns_full_card() {
    let s = start();
    s.run(ALPHA, &["new", "一"]);
    s.run(ALPHA, &["new", "二", "--body", "gh#140"]);
    let c = s.run(ALPHA, &["claim", "--json"]).json();
    assert_eq!(c["id"], 1);
    assert_eq!(c["column"], "doing");
    assert_eq!(c["holder"], "test/alpha");
    assert!(c["claimed_at"].is_string());
    assert!(c["run_started_at"].is_null(), "第一次心跳才开始计时");
    // 心跳：开始第 1 次
    let r = s.run(ALPHA, &["beat", "1", "--json"]);
    assert!(r.json()["run_started_at"].is_string());
    assert_eq!(r.json()["work"]["runs"], 1);
    assert!(c["activity"].as_array().unwrap().len() >= 2);
    // --search 在描述里查找
    let r = s.run(ALPHA, &["ls", "--search", "GH#140", "--json"]);
    let ids: Vec<i64> = r.json().as_array().unwrap().iter().map(|c| c["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn new_inbox_creates_unfiled_entry() {
    let s = start();
    let r = s.run(ALPHA, &["new", "想法", "--body", "待细化", "--inbox", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let c = r.json();
    assert_eq!(c["column"], "inbox");
    assert!(c["holder"].is_null());
    let r = s.run(ALPHA, &["ls", "--column", "inbox", "--json"]);
    let ids: Vec<i64> = r.json().as_array().unwrap().iter().map(|c| c["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![1]);
    // 未整理仍不能领取
    let r = s.run(ALPHA, &["claim", "1", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_IN_TODO"));
    // 不带 --inbox 时行为不变：进入首个可领取状态最上面
    let r = s.run(ALPHA, &["new", "任务", "--top", "--json"]);
    assert_eq!(r.json()["column"], "todo");
    // 已有卡片仍不能移入未整理
    let r = s.run(ALPHA, &["move", "2", "inbox", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "FORBIDDEN_COLUMN"));
}

/// 拆解：board new --inbox --from <ID> 记下拆自哪张，board show 两边都显示（PRD §9）
#[test]
fn new_from_shows_split_relation() {
    let s = start();
    assert_eq!(s.run(ALPHA, &["new", "原始需求", "--inbox", "--json"]).json()["id"], 1);
    let r = s.run(ALPHA, &["new", "a", "--inbox", "--from", "1", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let c = r.json();
    assert_eq!((c["column"].as_str(), c["split_from"].as_i64()), (Some("inbox"), Some(1)));
    let r = s.run(ALPHA, &["new", "b", "--inbox", "--from", "#1"]);
    assert!(r.out.contains("拆自 #1"), "{}", r.out);
    let p = s.run(ALPHA, &["show", "1", "--json"]).json();
    assert_eq!(p["split_into"], serde_json::json!([2, 3]));
    assert!(p["split_from"].is_null());
    let t = s.run(ALPHA, &["show", "1"]).out;
    assert!(t.contains("拆出") && t.contains("#2 #3"), "{t}");
    assert!(s.run(ALPHA, &["show", "2"]).out.contains("拆自"), "子卡显示拆自");
    let r = s.run(ALPHA, &["new", "c", "--inbox", "--from", "9999", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_FOUND"));
    let r = s.run(ALPHA, &["new", "c", "--from", "abc", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
}

#[test]
fn ac12_identity_and_board_required() {
    let s = start();
    let with = |extra: &[(&'static str, &str)]| {
        let mut env = s.env();
        env.extend(extra.iter().map(|(k, v)| (*k, v.to_string())));
        env
    };
    let r = s.raw(&with(&[]), &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NO_IDENTITY"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("board check --as"));
    let r = s.raw(&with(&[("BOARD_AS", "test/alpha")]), &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NO_BOARD"));
    // --as / --board 与环境变量等价
    let r = s.raw(&with(&[]), &["ls", "--as", "test/alpha", "--board", "default/main", "--json"]);
    assert_eq!(r.code, 0, "{}", r.out);
    // boards 只需要身份
    let r = s.raw(&with(&[("BOARD_AS", "test/alpha")]), &["boards", "--json"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.json()[0]["board"], "default/main");
    let r = s.raw(&with(&[("BOARD_AS", "test/alpha"), ("BOARD", "nope")]), &["ls", "--json"]);
    assert_eq!(r.error_code(), "BOARD_NOT_FOUND");
}

#[test]
fn ac23_24_labels() {
    let s = start();
    s.run(ALPHA, &["new", "a"]);
    s.create_label("bug");
    s.run(ALPHA, &["new", "b"]);
    let r = s.run(ALPHA, &["label", "1", "+bug", "--json"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(r.json()["labels"][0], "bug");
    let r = s.run(ALPHA, &["label", "1", "+不存在的标签", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("bug"), "原因里列出可用的标签");
    let r = s.run(ALPHA, &["ls", "--label", "BUG", "--json"]);
    let ids: Vec<i64> = r.json().as_array().unwrap().iter().map(|c| c["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![1]);
    let r = s.run(ALPHA, &["labels", "--json"]);
    assert_eq!(r.json()[0]["cards"], 1);
    let r = s.run(ALPHA, &["label", "1", "-bug", "--json"]);
    assert!(r.json()["labels"].as_array().unwrap().is_empty());
}

/// 仅人可打的标签：board labels 能区分；agent 加、去被拒绝（USAGE），原因里说明只有人能打
#[test]
fn human_only_labels() {
    let s = start();
    s.run(ALPHA, &["new", "a"]);
    s.create_label("bug");
    s.create_label_as("人定", true);
    let r = s.run(ALPHA, &["labels", "--json"]);
    assert_eq!(r.json()[0]["human_only"], false);
    assert_eq!(r.json()[1]["human_only"], true);
    let r = s.run(ALPHA, &["labels"]);
    assert!(r.out.contains("仅人可打") && r.out.contains("agent 可打"), "{}", r.out);
    let r = s.run(ALPHA, &["label", "1", "+人定", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("只有人能打"));
    let r = s.run(ALPHA, &["label", "1", "+bug", "--json"]);
    assert_eq!(r.code, 0, "{}", r.out);
}

/// board new --label：可以写多次、不区分大小写、重复只算一次；不存在、仅人可打的拒绝且不建卡；能和 --inbox、--top 一起用
#[test]
fn new_with_labels() {
    let s = start();
    s.create_label("功能");
    s.create_label("UI");
    s.create_label_as("人定", true);
    let r = s.run(ALPHA, &["new", "x", "--label", "功能", "--label", "ui", "--label", "UI", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["labels"], serde_json::json!(["功能", "UI"]));
    let r = s.run(ALPHA, &["new", "y", "--label", "功能"]);
    assert!(r.out.contains("#功能"), "文字输出带上标签：{}", r.out);
    let count = || s.run(ALPHA, &["ls", "--json"]).json().as_array().unwrap().len();
    let before = count();
    let r = s.run(ALPHA, &["new", "z", "--label", "不存在的标签", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("功能"), "原因里列出可用的标签");
    let r = s.run(ALPHA, &["new", "z", "--label", "人定", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    assert_eq!(count(), before, "没有多出卡片");
    // 和 --inbox、--top 一起用
    let r = s.run(ALPHA, &["new", "收件", "--inbox", "--label", "UI", "--json"]);
    assert_eq!((r.json()["column"].as_str(), r.json()["labels"].clone()), (Some("inbox"), serde_json::json!(["UI"])));
    let r = s.run(ALPHA, &["new", "置顶", "--top", "--label", "UI", "--json"]);
    assert_eq!(r.json()["labels"], serde_json::json!(["UI"]));
    // 不带 --label 时和原来一样
    assert_eq!(s.run(ALPHA, &["new", "无标签", "--json"]).json()["labels"], serde_json::json!([]));
    assert!(s.run(ALPHA, &["help"]).out.contains("--label <标签>"));
}

/// board next 只在 board host 启动的会话里能用：环境里没有 BOARD_HOST 时 USAGE；有但名字没有在跑的派发时也是 USAGE
#[test]
fn next_only_in_host_sessions() {
    let s = start();
    s.run(ALPHA, &["new", "a"]);
    let r = s.run(ALPHA, &["next", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("BOARD_HOST"));
}

/// 优先标签：labels、ls、show 的 --json 里能看出是否优先；不带 ID 领取先领带优先标签的卡片
#[test]
fn priority_labels() {
    let s = start();
    for t in ["a", "b"] {
        s.run(ALPHA, &["new", t]);
    }
    s.create_label("急");
    s.create_label("bug");
    let r = s.run(ALPHA, &["labels", "--json"]);
    assert_eq!((r.json()[0]["priority"].clone(), r.json()[1]["priority"].clone()), (false.into(), false.into()), "默认不是优先");
    s.set_priority("急");
    s.run(ALPHA, &["label", "2", "+急", "+bug"]);
    let r = s.run(ALPHA, &["labels", "--json"]);
    assert_eq!(r.json()[0]["priority"], true);
    assert!(s.run(ALPHA, &["labels"]).out.contains("优先"));
    let r = s.run(ALPHA, &["ls", "--json"]);
    let b = r.json().as_array().unwrap().iter().find(|c| c["id"] == 2).unwrap().clone();
    assert_eq!(b["priority_labels"], serde_json::json!(["急"]));
    assert_eq!(s.run(ALPHA, &["show", "1", "--json"]).json()["priority_labels"], serde_json::json!([]));
    // 不带 ID 领取：b 带优先标签，先领 b
    assert_eq!(s.run(ALPHA, &["claim", "--json"]).json()["id"], 2);
}

/// 附件：上传成一条评论、show 列出、files 下载到本地
#[test]
fn attach_and_download_files() {
    let s = start();
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("截图.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
    let log = dir.path().join("run.log");
    std::fs::write(&log, "ok\n").unwrap();
    s.run(ALPHA, &["new", "带附件"]);

    let r = s.run(ALPHA, &["attach", "1", png.to_str().unwrap(), log.to_str().unwrap(), "--comment", "复现截图和日志", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let v = r.json();
    assert_eq!(v["attachments"][0]["name"], "截图.png");
    assert_eq!(v["attachments"][0]["image"], true);
    let comment = v["card"]["activity"].as_array().unwrap().last().unwrap()["text"].as_str().unwrap().to_string();
    assert!(comment.starts_with("复现截图和日志\n\n![截图.png](/api/files/1)"), "{comment}");
    assert!(comment.ends_with("[run.log](/api/files/2)"), "{comment}");

    // show 列出附件，提示怎么下载
    let r = s.run(ALPHA, &["show", "1"]);
    assert!(r.out.contains("附件  board files 1 下载到本地查看"), "{}", r.out);
    assert!(r.out.contains("截图.png"));
    assert_eq!(s.run(ALPHA, &["show", "1", "--json"]).json()["attachments"][1]["name"], "run.log");

    // files 下载到指定目录
    let out = dir.path().join("dl");
    let r = s.run(ALPHA, &["files", "1", "--out", out.to_str().unwrap(), "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json().as_array().unwrap().len(), 2);
    assert_eq!(std::fs::read(out.join("2-run.log")).unwrap(), b"ok\n");
    assert!(out.join("1-截图.png").exists());

    // 错误：不支持的类型、文件不存在、卡片不存在、没有附件
    let svg = dir.path().join("x.svg");
    std::fs::write(&svg, "<svg/>").unwrap();
    let r = s.run(ALPHA, &["attach", "1", svg.to_str().unwrap(), "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
    let r = s.run(ALPHA, &["attach", "1", "nope.png", "--json"]);
    assert_eq!(r.error_code(), "USAGE");
    let r = s.run(ALPHA, &["attach", "9", png.to_str().unwrap(), "--json"]);
    assert_eq!(r.error_code(), "NOT_FOUND");
    s.run(ALPHA, &["new", "没附件"]);
    assert!(s.run(ALPHA, &["files", "2"]).out.contains("#2 没有附件"));
}

/// board new 带图片、文件（卡片 default/main#148）：按参数顺序接在描述后面，建卡时归到这张卡片；有一个不行就什么都不做
#[test]
fn new_with_images_and_files() {
    let s = start();
    s.create_label("功能");
    let dir = tempfile::tempdir().unwrap();
    let put = |name: &str, bytes: &[u8]| {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p.to_string_lossy().into_owned()
    };
    let p1 = put("1.png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
    let p2 = put("2.jpg", b"\xFF\xD8\xFF\xE0jfif");
    let log = put("run.log", b"ok\n");

    // 图片、文件混着写，顺序保持；--label、--json 照常
    let r = s.run(ALPHA, &["new", "带图", "--body", "描述", "--image", &p1, "--file", &log, "--image", &p2, "--label", "功能", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let c = r.json();
    assert_eq!(c["body"], "描述\n\n![1.png](/api/files/1)\n\n[run.log](/api/files/2)\n\n![2.jpg](/api/files/3)");
    let names: Vec<&str> = c["attachments"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["1.png", "run.log", "2.jpg"]);
    assert_eq!(c["labels"], serde_json::json!(["功能"]));
    // 只有图片、没有描述；--inbox 照常
    let c = s.run(ALPHA, &["new", "只有图", "--image", &p1, "--inbox", "--json"]).json();
    assert_eq!(c["body"], "![1.png](/api/files/4)");
    assert_eq!(c["column"], "inbox");
    assert_eq!(c["attachments"].as_array().unwrap().len(), 1);

    // 错误都在上传前报出来：不建卡，也不上传任何附件
    let big = put("big.png", &[b"\x89PNG\r\n\x1a\n".as_slice(), &vec![0u8; 10 * 1024 * 1024]].concat());
    let svg = put("x.svg", b"<svg/>");
    let fake = put("fake.png", b"<html>");
    for (args, code) in [
        (vec!["new", "x", "--image", &p1, "--file", "/no/such.log"], "USAGE"),
        (vec!["new", "x", "--image", &p1, "--file", &svg], "USAGE"),
        (vec!["new", "x", "--image", &log], "USAGE"),
        (vec!["new", "x", "--image", &fake], "USAGE"),
        (vec!["new", "x", "--file", &big], "USAGE"),
        (vec!["new", "", "--image", &p1], "EMPTY_TITLE"),
    ] {
        let mut args = args;
        args.push("--json");
        let r = s.run(ALPHA, &args);
        assert_eq!((r.code, r.error_code().as_str()), (1, code), "{args:?}: {}{}", r.out, r.err);
    }
    let all = s.run(ALPHA, &["ls", "--json"]).json();
    assert!(all.as_array().unwrap().iter().all(|c| c["title"] != "x"), "失败的都没有建卡：{all}");
    let store = board_core::Store::open(&s.db).unwrap();
    let n: i64 = store.conn().query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 4, "失败的都没有上传附件");
}

/// 没有配置服务端：直接失败，提示运行 board login；不会在 HOME 下建本地数据库
#[test]
fn requires_login() {
    let s: Server = start();
    let who = [("BOARD_AS", "test/alpha".to_string()), ("BOARD", "default".to_string())];

    // 环境变量和配置文件都没有
    for args in [vec!["ls", "--json"], vec!["boards", "--json"], vec!["claim", "--json"], vec!["check", "--json"], vec!["beat", "--json"]] {
        let r = s.raw(&who, &args);
        assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_LOGGED_IN"), "{args:?}");
        let msg = r.json()["error"]["message"].as_str().unwrap().to_string();
        assert!(msg.contains("board login") && msg.contains("设置 → Agent 接入"), "{msg}");
    }
    let r = s.raw(&who, &["ls"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("NOT_LOGGED_IN") && r.err.contains("board login"), "{}", r.err);
    // 即使给了 BOARD_DB 也不再读写本地数据库
    let db = s.home.join("x.db");
    let mut env = who.to_vec();
    env.push(("BOARD_DB", db.to_string_lossy().into()));
    assert_eq!(s.raw(&env, &["ls", "--json"]).error_code(), "NOT_LOGGED_IN");
    assert!(!db.exists());

    // 有地址但没有令牌
    let mut env = who.to_vec();
    env.push(("BOARD_URL", s.url.clone()));
    let r = s.raw(&env, &["ls", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_LOGGED_IN"));
    let msg = r.json()["error"]["message"].as_str().unwrap().to_string();
    assert!(msg.contains("board login") && msg.contains("设置 → Agent 接入"), "{msg}");

    // help 照常可用，并说明怎么配置
    let r = s.raw(&[], &["help"]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("board login") && r.out.contains("设置 → Agent 接入") && !r.out.contains("本地数据库"), "{}", r.out);

    assert!(!s.home.join(".board").exists(), "不应该在 HOME 下创建 ~/.board");
}

/// board host rename（桌面端「设置 → 本机」保存机器名时调用）：服务端把机器记录原地改名，
/// 设置和历史派发记录保留；机器没记录时 NO_SUCH_HOST，新名字被在线机器占用时 HOST_NAME_TAKEN，名字没变时不动
#[test]
fn host_rename_renames_the_machine_record() {
    let s: Server = start();
    let now = board_core::now_ms();
    let mut st = board_core::Store::open(&s.db).unwrap();
    let mut rep = board_core::HostReport {
        name: "mbp".into(),
        instance: "i1".into(),
        resume: None,
        os: "test".into(),
        version: "0.0.0".into(),
        agents: vec![],
        envs: vec![],
        workspace: Some(board_core::HostWorkspace { path: "/tmp".into(), ..Default::default() }),
        dirs: vec![board_core::HostDir { board: "default/main".into(), path: "/tmp/default/main".into(), ..Default::default() }],
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
    };
    st.write(now, board_core::Origin::cli(), |w| w.host_beat(&rep, Some(1))).unwrap();
    // 机器名写在配置文件 [host] 里（和桌面端保存的一样）
    let cfg = s.home.join(".board");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("config.toml"), format!("url = \"{}\"\ntoken = \"{}\"\n\n[host]\nname = \"mbp\"\n", s.url, s.token)).unwrap();
    let r = s.raw(&s.env(), &["host", "rename", "win", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["name"], "win");
    assert!(st.read(|x| x.host("win", now)).is_ok(), "新名字有机器");
    assert!(st.read(|x| x.host("mbp", now)).is_err(), "旧名字不再有机器");
    // 配置里还是旧名字、服务端上已经没有它：NO_SUCH_HOST（桌面端当作没有要改的）
    let r = s.raw(&s.env(), &["host", "rename", "desk", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NO_SUCH_HOST"), "{}{}", r.out, r.err);
    // 新名字被另一台在线机器占用：HOST_NAME_TAKEN
    rep.name = "desk".into();
    rep.instance = "j1".into();
    st.write(now, board_core::Origin::cli(), |w| w.host_beat(&rep, Some(1))).unwrap();
    std::fs::write(cfg.join("config.toml"), format!("url = \"{}\"\ntoken = \"{}\"\n\n[host]\nname = \"win\"\n", s.url, s.token)).unwrap();
    let r = s.raw(&s.env(), &["host", "rename", "desk", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "HOST_NAME_TAKEN"), "{}{}", r.out, r.err);
    assert!(st.read(|x| x.host("win", now)).is_ok(), "机器记录没有被改动");
    // 名字没变：不动服务端
    let r = s.raw(&s.env(), &["host", "rename", "win", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["unchanged"], true);
}

/// board hold：推集成分支前确认领取仍有效。仍由自己持有、在锁定状态时成功；被人强制释放后 NOT_HOLDER
#[test]
fn hold_confirms_the_claim_before_pushing() {
    let s = start();
    s.run(ALPHA, &["new", "a"]);
    assert_eq!(s.run(ALPHA, &["claim", "1"]).code, 0);
    let r = s.run(ALPHA, &["hold", "1", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!((r.json()["ok"].as_bool(), r.json()["column"].as_str()), (Some(true), Some("doing")));
    // 人在界面上强制释放：卡片回到待办，领取失效
    let mut store = board_core::Store::open(&s.db).unwrap();
    store
        .write(board_core::now_ms(), board_core::Origin::ui(None), |w| {
            let b = w.resolve("default/main")?;
            w.exec(&b, &board_core::Actor::human(), board_core::Cmd::Release { id: 1 }).map(|_| ())
        })
        .unwrap();
    let r = s.run(ALPHA, &["hold", "1", "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "NOT_HOLDER"));
    let msg = r.json()["error"]["message"].as_str().unwrap().to_string();
    assert!(msg.contains("领取已经失效") && msg.contains("「待办」") && msg.contains("wip 分支"), "{msg}");
    // 别人领走了：也是 NOT_HOLDER，写明是谁
    s.run("test/beta", &["claim", "1"]);
    let r = s.run(ALPHA, &["hold", "1", "--json"]);
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("由 test/beta 持有"), "{}", r.out);
}

/// board hold：连不上服务端时按间隔重试（测试里把 30 秒缩短成 0.2 秒），恢复后接着确认；一直连不上就 UNREACHABLE
#[test]
fn hold_waits_for_the_server() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    let s = start();
    s.run(ALPHA, &["new", "a"]);
    assert_eq!(s.run(ALPHA, &["claim", "1"]).code, 0);
    // 一个先没人监听、1 秒后才开始转发到服务端的端口：模拟服务端停了一会儿又恢复
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let target = s.url.trim_start_matches("http://").to_string();
    let env = |url: String| {
        vec![
            ("BOARD_URL", url),
            ("BOARD_TOKEN", s.token.clone()),
            ("BOARD_AS", ALPHA.to_string()),
            ("BOARD", "default".to_string()),
            ("BOARD_HOLD_INTERVAL", "0.2".to_string()),
        ]
    };
    // 一直连不上：等满 --wait 就 UNREACHABLE，写明改推 wip 分支。
    // Windows 上连接被拒绝要 ~2 秒才返回，--wait 1 时第一次还没回来就等满了，不会打印重试提示：多等几秒
    let wait = if cfg!(windows) { 6 } else { 1 };
    let begun = std::time::Instant::now();
    let r = s.raw(&env(format!("http://127.0.0.1:{port}")), &["hold", "1", "--wait", &wait.to_string(), "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "UNREACHABLE"), "{}{}", r.out, r.err);
    assert!(r.json()["error"]["message"].as_str().unwrap().contains("wip 分支"));
    assert!(begun.elapsed() < std::time::Duration::from_secs(15));
    assert!(r.err.contains("连不上服务端"), "重试时在标准错误里提示：{}", r.err);
    // 停了一会儿又恢复：等到恢复后确认成功
    let home = s.home.clone();
    let e = env(format!("http://127.0.0.1:{port}"));
    let cli = std::thread::spawn(move || common::board(&home, &e, &["hold", "1", "--wait", "20", "--json"]));
    // 等第一次确认确实失败了再开始转发（Windows 上连接被拒绝要 ~2 秒才返回，太快接上就看不到重试提示）
    std::thread::sleep(std::time::Duration::from_secs(if cfg!(windows) { 4 } else { 1 }));
    let l = TcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let t = target.clone();
            std::thread::spawn(move || {
                let Ok(up) = TcpStream::connect(&t) else { return };
                let (mut a, mut b) = (c.try_clone().unwrap(), up.try_clone().unwrap());
                let (mut c2, mut up2) = (c, up);
                std::thread::spawn(move || {
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = a.read(&mut buf) {
                        if n == 0 || up2.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                    let _ = up2.shutdown(std::net::Shutdown::Write);
                });
                let mut buf = [0u8; 8192];
                while let Ok(n) = b.read(&mut buf) {
                    if n == 0 || c2.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                let _ = c2.shutdown(std::net::Shutdown::Write);
            });
        }
    });
    let r = cli.join().unwrap();
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["ok"].as_bool(), Some(true));
    assert!(r.err.contains("连不上服务端"), "{}", r.err);
}
