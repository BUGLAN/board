//! `board host` / `board dispatch` 端到端（任务下发需求 §十二）：起一个服务端，在后台跑真的 board host，
//! 「agent」是一个 shell 脚本（按 mode 文件决定交审、崩溃还是一直睡），用配置文件的 `[agents.claude]` 换掉内置模板。
#![cfg(unix)]

mod common;
use common::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const FAKE: &str = r#"#!/bin/sh
# 假 agent：$1 是提示词；按 $FAKE_DIR/mode 决定做什么
if [ "$1" = "--version" ]; then echo "9.9.9 (fake)"; exit 0; fi
echo "prompt: $1"
echo "as: $BOARD_AS host: $BOARD_HOST board: $BOARD"
id=$(echo "$1" | sed 's/.*#\([0-9]*\)任务.*/\1/')
case "$(cat "$FAKE_DIR/mode")" in
  review)
    "$BOARD_BIN" beat "$id" --final >/dev/null && "$BOARD_BIN" move "$id" review >/dev/null
    echo "交审了"
    exit 0 ;;
  crash)
    echo "working"
    echo "boom"
    exit 3 ;;
  *)
    echo "sleeping"
    exec sleep 60 ;;
esac
"#;

struct Rig {
    s: Server,
    fake: PathBuf,
    repo: PathBuf,
    _tmp: tempfile::TempDir,
}

impl Rig {
    fn new() -> Rig {
        let s = start();
        let tmp = tempfile::tempdir().unwrap();
        let fake = tmp.path().join("fake-agent.sh");
        std::fs::write(&fake, FAKE).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let conf = s.home.join(".board");
        std::fs::create_dir_all(&conf).unwrap();
        std::fs::write(
            conf.join("config.toml"),
            format!(
                "[agents.claude]\ncmd = [\"{}\", \"{{prompt}}\"]\n\n[agents.pi]\nname = \"pi\"\ncmd = [\"board-test-no-such-command\", \"{{prompt}}\"]\n",
                fake.display()
            ),
        )
        .unwrap();
        Rig { s, fake, repo, _tmp: tmp }
    }

    fn mode(&self, m: &str) {
        std::fs::write(self.fake.parent().unwrap().join("mode"), m).unwrap();
    }

    fn host(&self, name: &str) -> Child {
        let mut c = Command::new(env!("CARGO_BIN_EXE_board"));
        c.args(["host", "--name", name, "--dir", &format!("default/main={}", self.repo.display())])
            .env("HOME", &self.s.home)
            .env("USERPROFILE", &self.s.home)
            .env("NO_COLOR", "1")
            .env("BOARD_URL", &self.s.url)
            .env("BOARD_TOKEN", &self.s.token)
            .env("BOARD_HOST_INTERVAL", "0.5")
            .env("BOARD_BIN", env!("CARGO_BIN_EXE_board"))
            .env("FAKE_DIR", self.fake.parent().unwrap())
            .env_remove("BOARD")
            .env_remove("BOARD_AS")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c.spawn().unwrap()
    }

    /// 以 `test/maker` 执行（新建卡片、领取等）
    fn board(&self, args: &[&str]) -> Run {
        self.s.run("test/maker", args)
    }

    fn workspace(&self) -> Value {
        let mut res = ureq::get(format!("{}/api/workspace", self.s.url))
            .header("authorization", format!("Bearer {}", self.s.token))
            .call()
            .unwrap();
        serde_json::from_str(&res.body_mut().read_to_string().unwrap()).unwrap()
    }

    fn show(&self, id: i64) -> Value {
        let r = self.board(&["show", &id.to_string(), "--json"]);
        assert_eq!(r.code, 0, "{}{}", r.out, r.err);
        r.json()
    }

    fn dispatch_log(&self, id: i64) -> Vec<String> {
        let store = board_core::Store::open(&self.s.db).unwrap();
        store.read(|r| r.dispatch_log(id)).unwrap()
    }
}

fn wait_until(what: &str, limit: Duration, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < limit, "{what}：{limit:?} 内没有发生");
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn activity(card: &Value) -> Vec<String> {
    card["activity"].as_array().unwrap().iter().map(|a| a["text"].as_str().unwrap_or_default().to_string()).collect()
}

fn ended(card: &Value, result: &str) -> bool {
    activity(card).iter().any(|t| t.starts_with(&format!("派发结束：{result}")))
}

fn stop(mut c: Child) -> (i32, String) {
    unsafe {
        libc_kill(c.id() as i32, 15);
    }
    let o = wait_out(&mut c, Duration::from_secs(25));
    o
}

extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

fn wait_out(c: &mut Child, limit: Duration) -> (i32, String) {
    let t = Instant::now();
    loop {
        if let Some(s) = c.try_wait().unwrap() {
            let mut out = String::new();
            use std::io::Read;
            if let Some(mut o) = c.stdout.take() {
                let _ = o.read_to_string(&mut out);
            }
            if let Some(mut e) = c.stderr.take() {
                let _ = e.read_to_string(&mut out);
            }
            return (s.code().unwrap_or(-1), out);
        }
        assert!(t.elapsed() < limit, "board host 没有在 {limit:?} 内退出");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn log_file(home: &Path, id: i64) -> PathBuf {
    home.join(".board").join("host").join("logs").join(format!("{id}.log"))
}

#[test]
fn host_runs_dispatches_end_to_end() {
    let rig = Rig::new();
    for t in ["一", "二", "三", "四", "五"] {
        assert_eq!(rig.board(&["new", t]).code, 0);
    }
    let host = rig.host("mbp");
    wait_until("mbp 上线", Duration::from_secs(20), || {
        rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true))
    });
    let ws = rig.workspace();
    let h = &ws["hosts"][0];
    assert_eq!(h["max"], 1);
    let agents = h["agents"].as_array().unwrap();
    assert!(agents.iter().any(|a| a["id"] == "claude" && a["found"] == true && a["version"] == "9.9.9 (fake)"), "{agents:?}");
    assert!(agents.iter().any(|a| a["id"] == "pi" && a["found"] == false), "PATH 里没有的 agent 显示为没找到");

    // 同名的第二个 host：HOST_NAME_TAKEN
    let mut second = rig.host("mbp");
    let (code, out) = wait_out(&mut second, Duration::from_secs(10));
    assert_eq!(code, 1);
    assert!(out.contains("HOST_NAME_TAKEN"), "{out}");

    // 校验
    let r = rig.board(&["dispatch", "5", "--host", "mbp", "--agent", "pi", "--json"]);
    assert_eq!(r.error_code(), "AGENT_NOT_ON_HOST");
    let r = rig.board(&["dispatch", "5", "--host", "nope", "--json"]);
    assert_eq!(r.error_code(), "NO_SUCH_HOST");

    // 交审：持有者是 claude/mbp-1，日志写到 ~/.board/host/logs/，上传到服务端
    rig.mode("review");
    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let d1 = r.json()["id"].as_i64().unwrap();
    wait_until("#1 交审", Duration::from_secs(15), || ended(&rig.show(1), "已交审"));
    let c1 = rig.show(1);
    assert_eq!(c1["column"], "review");
    assert!(activity(&c1).iter().any(|t| t.contains("以 claude/mbp-1 领取")), "{:?}", activity(&c1));
    let local = std::fs::read_to_string(log_file(&rig.s.home, d1)).unwrap();
    assert!(local.contains("prompt: /board-skills 开始执行 default项目之中的main白板中的#1任务"), "{local}");
    assert!(local.contains("as: claude/mbp-1 host: mbp board: default/main"), "{local}");
    assert!(rig.dispatch_log(d1).iter().any(|l| l == "交审了"));

    // 崩溃：移到需人工校验，评论里有退出码和日志最后几行
    rig.mode("crash");
    assert_eq!(rig.board(&["dispatch", "2", "--host", "mbp"]).code, 0);
    wait_until("#2 异常退出", Duration::from_secs(15), || ended(&rig.show(2), "异常退出"));
    let c2 = rig.show(2);
    assert_eq!(c2["column"], "check");
    assert!(activity(&c2).iter().any(|t| t.contains("退出码 3") && t.contains("boom")), "{:?}", activity(&c2));

    // 同时跑 1 个：#3 在跑时派发 #4 排队，别的 agent 领 #4 报 DISPATCHED
    rig.mode("sleep");
    assert_eq!(rig.board(&["dispatch", "3", "--host", "mbp"]).code, 0);
    wait_until("#3 开始", Duration::from_secs(10), || rig.show(3)["holder"] == "claude/mbp-3");
    assert_eq!(rig.board(&["dispatch", "4", "--host", "mbp"]).code, 0);
    assert_eq!(rig.board(&["claim", "4", "--json"]).error_code(), "DISPATCHED");
    std::thread::sleep(Duration::from_secs(1));
    assert!(rig.show(4)["holder"].is_null(), "没有空位，#4 排队");

    // 停止 #3：进程结束，#3 移到需人工校验；#4 接着开始
    let r = rig.board(&["dispatch", "3", "--cancel"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    wait_until("#3 已停止", Duration::from_secs(15), || ended(&rig.show(3), "已停止"));
    assert_eq!(rig.show(3)["column"], "check");
    wait_until("#4 开始", Duration::from_secs(10), || rig.show(4)["holder"] == "claude/mbp-4");

    // Ctrl+C（SIGTERM）：先停掉 #4，再离线
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(ended(&rig.show(4), "已停止"), "{:?}", activity(&rig.show(4)));
    assert!(rig.workspace()["hosts"][0]["online"] == false);
}
