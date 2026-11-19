//! 机器详情与配置同步重构：同步配置的「重试」和回报、选工作空间时的「列目录」（界面 → 服务端 → board host → 界面）。
//! 起一个本机模式的 board-server，用 HTTP 扮演界面和 board host 两边。

mod common;

use board_server::auth::Auth;
use common::{send, Server, UI};
use serde_json::{json, Value};

fn report(name: &str, instance: &str, sync: bool) -> Value {
    json!({
        "name": name,
        "instance": instance,
        "os": "macOS 15.4",
        "version": env!("CARGO_PKG_VERSION"),
        "agents": [{ "id": "claude", "name": "Claude Code", "version": "2.4.1", "found": true }],
        "selftest": true,
        "config_sync": sync,
    })
}

#[test]
fn list_dir_goes_through_the_board_host_and_comes_back() {
    let s = Server::start(Auth::Local);
    let r = send("POST", &format!("{}/api/hosts/mbp/beat", s.url), &[UI], Some(report("mbp", "i1", true)));
    assert_eq!(r.status, 200, "{}", r.text);
    // 界面在问（会一直等到 board host 回传）
    let url = s.url.clone();
    let ui = std::thread::spawn(move || send("POST", &format!("{url}/api/hosts/mbp/ls"), &[UI], Some(json!({ "path": "" }))));
    // board host：重新取任务，里面有这个请求
    let mut ask = Value::Null;
    for _ in 0..100 {
        let w = send("GET", &format!("{}/api/hosts/mbp/work", s.url), &[UI], None);
        if let Some(a) = w.body["ls"].as_array().and_then(|l| l.first()) {
            ask = a.clone();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    assert_eq!(ask["path"], json!(""), "任务里带着要列的目录");
    let list = json!({ "path": "", "chain": [""], "entries": [{ "name": "/Users/me", "path": "/Users/me", "git": false, "repos": 3 }] });
    let r = send("POST", &format!("{}/api/hosts/mbp/ls/{}", s.url, ask["id"]), &[UI], Some(list.clone()));
    assert_eq!(r.status, 200, "{}", r.text);
    let got = ui.join().unwrap();
    assert_eq!(got.status, 200, "{}", got.text);
    assert_eq!(got.body["entries"], list["entries"]);
    // 回传过的不再出现在任务里
    let w = send("GET", &format!("{}/api/hosts/mbp/work", s.url), &[UI], None);
    assert!(w.body.get("ls").is_none(), "{}", w.text);

    // 旧版 board host（不会列）：直接说
    send("POST", &format!("{}/api/hosts/old/beat", s.url), &[UI], Some(report("old", "i2", false)));
    let r = send("POST", &format!("{}/api/hosts/old/ls", s.url), &[UI], Some(json!({ "path": "" })));
    assert_eq!(r.code(), "USAGE");
    assert!(r.body["error"]["message"].as_str().unwrap().contains("旧版"), "{}", r.text);
    // 离线：不等
    let mut bye = report("mbp", "i1", true);
    bye["final"] = json!(true);
    send("POST", &format!("{}/api/hosts/mbp/beat", s.url), &[UI], Some(bye));
    let r = send("POST", &format!("{}/api/hosts/mbp/ls", s.url), &[UI], Some(json!({ "path": "" })));
    assert!(r.body["error"]["message"].as_str().unwrap().contains("离线"), "{}", r.text);
}

#[test]
fn sync_retry_asks_again_and_the_report_clears_it() {
    let s = Server::start(Auth::Local);
    let w = send("POST", &format!("{}/api/hosts/mbp/beat", s.url), &[UI], Some(report("mbp", "i1", true)));
    let key = w.body["config_key"].as_str().unwrap().to_string();
    assert!(!key.is_empty());
    let r = send("POST", &format!("{}/api/hosts/mbp/sync", s.url), &[UI], None);
    assert_eq!(r.status, 200, "{}", r.text);
    let ask = r.body["sync"]["ask"].clone();
    assert_eq!(ask["key"], json!(key));
    // board host 写完回报
    let result = json!({ "id": ask["id"], "key": key, "agents": [{ "agent": "claude", "state": "off" }], "skills": { "state": "off", "count": 1 }, "mcp": { "state": "off" } });
    let r = send("POST", &format!("{}/api/hosts/mbp/synced", s.url), &[UI], Some(json!({ "instance": "i1", "result": result })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(r.body["sync"].get("ask").is_none(), "{}", r.text);
    assert_eq!(r.body["sync"]["last"]["agents"][0]["state"], json!("off"));
    assert_eq!(r.body["ops"][0]["text"], json!("同步配置：agent 的配置都不改；只装 board-skills"));
    // 跑多久停：所有机器一个值
    let r = send("PATCH", &format!("{}/api/sched", s.url), &[UI], Some(json!({ "maxRun": 60, "idleRun": 10 })));
    assert_eq!((r.body["maxRun"].clone(), r.body["idleRun"].clone()), (json!(60), json!(10)), "{}", r.text);
    let w = send("GET", &format!("{}/api/hosts/mbp/work", s.url), &[UI], None);
    assert_eq!((w.body["host"]["maxRun"].clone(), w.body["host"]["idleRun"].clone()), (json!(60), json!(10)));
}

/// 「由这些机器做」（default/main#406）：界面点掉一台机器 → `PATCH /api/sched {agent, remove}` 存到服务端，
/// 之后这台机器来领卡不领；别的 agent 不受影响；一台都没选时也照常存
#[test]
fn sched_pick_add_remove_and_all_through_the_api() {
    let s = Server::start(Auth::Local);
    for (n, i) in [("a", "ia"), ("b", "ib")] {
        let mut rep = report(n, i, true);
        rep["agents"] = json!([
            { "id": "claude", "name": "Claude Code", "version": "2.4.1", "found": true },
            { "id": "dsh", "name": "DeepSeek Harness", "version": "1.0", "found": true }
        ]);
        rep["dirs"] = json!([{ "board": "default/main", "path": format!("/tmp/{n}") }]);
        let r = send("POST", &format!("{}/api/hosts/{n}/beat", s.url), &[UI], Some(rep));
        assert_eq!(r.status, 200, "{}", r.text);
    }
    let hosts = send("GET", &format!("{}/api/hosts", s.url), &[UI], None);
    let host_id = |name: &str| hosts.body["hosts"].as_array().unwrap().iter().find(|h| h["name"] == name).unwrap()["id"].as_i64().unwrap();
    let (a, b) = (host_id("a"), host_id("b"));
    // 先有标签「claude」和带它的卡片，再加规则：claude → claude
    let mut store = board_core::Store::open(&s.db).unwrap();
    let l = store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.create_label("default", "claude", None, false)).unwrap();
    let r = send("PATCH", &format!("{}/api/sched", s.url), &[UI], Some(json!({ "rules": [{ "op": "in", "labels": ["claude"], "agent": "claude" }] })));
    assert_eq!(r.status, 200, "{}", r.text);
    let card = store
        .write(board_core::now_ms(), board_core::Origin::ui(None), |w| {
            let b = w.resolve("default/main")?;
            w.exec(&b, &board_core::Actor::human(), board_core::Cmd::New { title: "C".into(), body: None, column: None, top: false, labels: vec![], from: None })
        })
        .unwrap();
    store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| {
        let b = w.resolve("default/main")?;
        w.exec(&b, &board_core::Actor::human(), board_core::Cmd::Label { id: card.card.id, add: vec![l.id], remove: vec![] })
    })
    .unwrap();
    // 没设置过：a 先来领，照常领到
    for (n, i, path) in [("a", "ia", "/tmp/a"), ("b", "ib", "/tmp/b")] {
        let mut rep = report(n, i, true);
        rep["agents"] = json!([{ "id": "claude", "name": "Claude Code", "version": "2.4.1", "found": true }]);
        rep["dirs"] = json!([{ "board": "default/main", "path": path }]);
        send("POST", &format!("{}/api/hosts/{n}/beat", s.url), &[UI], Some(rep));
    }
    // 自动接单的开关和时段：开了才会来领
    for n in ["a", "b"] {
        let r = send("PATCH", &format!("{}/api/hosts/{n}", s.url), &[UI], Some(json!({ "auto": true, "max": 2 })));
        assert_eq!(r.status, 200, "{}", r.text);
    }
    let r = send("POST", &format!("{}/api/hosts/a/pull", s.url), &[UI], Some(json!({ "instance": "ia" })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(!r.body.is_null(), "没设置过时 a 照常领：{}", r.text);
    // 点掉 a：a 就不领了（牌子上还没有这一张的时候也一样：这里卡片已经被 a 领走，换成新的一张看）
    let r = send("PATCH", &format!("{}/api/sched", s.url), &[UI], Some(json!({ "agent": "claude", "remove": [a] })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["picks"], json!([{ "agent": "claude", "hosts": [b], "none": false, "unset": false }]), "{}", r.text);
    let card2 = store
        .write(board_core::now_ms(), board_core::Origin::ui(None), |w| {
            let b = w.resolve("default/main")?;
            w.exec(&b, &board_core::Actor::human(), board_core::Cmd::New { title: "C2".into(), body: None, column: None, top: false, labels: vec![], from: None })
        })
        .unwrap();
    store
        .write(board_core::now_ms(), board_core::Origin::ui(None), |w| {
            let b = w.resolve("default/main")?;
            w.exec(&b, &board_core::Actor::human(), board_core::Cmd::Label { id: card2.card.id, add: vec![l.id], remove: vec![] })
        })
        .unwrap();
    let r = send("POST", &format!("{}/api/hosts/a/pull", s.url), &[UI], Some(json!({ "instance": "ia" })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(r.body.is_null(), "a 已经被点掉，不领这个 agent 的卡了：{}", r.text);
    // 再把 b 也点掉：一台都没选，b 也不领
    let r = send("PATCH", &format!("{}/api/sched", s.url), &[UI], Some(json!({ "agent": "claude", "remove": [b] })));
    assert_eq!(r.body["picks"], json!([{ "agent": "claude", "hosts": [], "none": true, "unset": false }]), "{}", r.text);
    let r = send("POST", &format!("{}/api/hosts/b/pull", s.url), &[UI], Some(json!({ "instance": "ib" })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(r.body.is_null(), "一台都没选时 b 也不领：{}", r.text);
    // 回到全都参与
    let r = send("PATCH", &format!("{}/api/sched", s.url), &[UI], Some(json!({ "agent": "claude", "all": true })));
    assert_eq!(r.body["picks"], json!([{ "agent": "claude", "hosts": [a, b], "none": false, "unset": true }]), "{}", r.text);
    // 不存在的机器：拒绝
    let r = send("PATCH", &format!("{}/api/sched", s.url), &[UI], Some(json!({ "agent": "claude", "add": [999] })));
    assert_eq!(r.code(), "USAGE", "{}", r.text);
}
#[test]
fn copy_settings_from_another_host_is_ui_only_and_presets_ride_on_the_new_token() {
    let s = Server::start(Auth::Local);
    for (n, i) in [("a", "ia"), ("b", "ib")] {
        let r = send("POST", &format!("{}/api/hosts/{n}/beat", s.url), &[UI], Some(report(n, i, true)));
        assert_eq!(r.status, 200, "{}", r.text);
    }
    let r = send("PATCH", &format!("{}/api/hosts/a", s.url), &[UI], Some(json!({ "syncMcp": true, "max": 3 })));
    assert_eq!(r.status, 200, "{}", r.text);

    // 访问令牌调不了
    let store = board_core::Store::open(&s.db).unwrap();
    let tok = board_server::auth::create_token(store.conn(), "b 的令牌", board_core::now_ms()).unwrap().1;
    let bearer = format!("Bearer {tok}");
    let r = send("POST", &format!("{}/api/hosts/b/copy", s.url), &[("authorization", bearer.as_str())], Some(json!({ "from": "a", "dry": true })));
    assert_eq!((r.status, r.code().as_str()), (401, "NOT_LOGGED_IN"), "{}", r.text);

    // 界面：先预览（不写），再确认
    let r = send("POST", &format!("{}/api/hosts/b/copy", s.url), &[UI], Some(json!({ "from": "a", "dry": true })));
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["done"], json!(false));
    let labels: Vec<&str> = r.body["changes"].as_array().unwrap().iter().map(|c| c["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["MCP", "最多几个会话"]);
    assert_eq!(r.body["changes"][1], json!({ "key": "max", "label": "最多几个会话", "from": "1", "to": "3" }));
    let r = send("POST", &format!("{}/api/hosts/b/copy", s.url), &[UI], Some(json!({ "from": "a" })));
    assert_eq!(r.body["done"], json!(true), "{}", r.text);
    let hosts = send("GET", &format!("{}/api/hosts", s.url), &[UI], None);
    let b = hosts.body["hosts"].as_array().unwrap().iter().find(|h| h["name"] == "b").unwrap().clone();
    assert_eq!((b["max"].as_i64(), b["syncMcp"].as_bool()), (Some(3), Some(true)));
    assert_eq!(b["ops"][0]["text"], json!("照 a 配：改了 MCP、最多几个会话"));
    assert!(b["sync"]["ask"].is_object(), "{b}");

    // 添加机器：生成命令时带「上线后照 a 配」；来源机器不在时不建令牌
    let r = send("POST", &format!("{}/api/tokens", s.url), &[UI], Some(json!({ "name": "c", "copyFrom": "nope" })));
    assert_eq!(r.code(), "NO_SUCH_HOST", "{}", r.text);
    let r = send("POST", &format!("{}/api/tokens", s.url), &[UI], Some(json!({ "name": "c", "copyFrom": "a", "host": "c" })));
    assert_eq!(r.status, 200, "{}", r.text);
    let raw = r.body["token"].as_str().unwrap().to_string();
    let bearer = format!("Bearer {raw}");
    let w = send("POST", &format!("{}/api/hosts/c/beat", s.url), &[("authorization", bearer.as_str())], Some(report("c", "ic", true)));
    assert_eq!(w.status, 200, "{}", w.text);
    assert_eq!((w.body["host"]["max"].as_i64(), w.body["host"]["syncMcp"].as_bool()), (Some(3), Some(true)), "{}", w.text);
    assert_eq!(w.body["host"]["ops"][0]["text"], json!("上线后照 a 配：改了 MCP、最多几个会话"));
}
