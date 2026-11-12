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
[ -f "$BOARD_CONFIG" ] && echo "config: ok"
[ -x "$BOARD_EXE" ] && echo "exe: ok"
id=$(echo "$1" | sed 's/.*#\([0-9]*\)任务.*/\1/')
case "$(cat "$FAKE_DIR/mode")" in
  review)
    # 像 dsh 那样拿不到名字像密钥的环境变量：令牌只能从 BOARD_CONFIG 指向的文件里读
    env -u BOARD_TOKEN "$BOARD_BIN" beat "$id" --final >/dev/null && env -u BOARD_TOKEN "$BOARD_BIN" move "$id" review >/dev/null
    echo "交审了"
    exit 0 ;;
  session)
    # 定时领取的会话：交审后 board next 接着做，直到 SESSION_DONE
    while :; do
      env -u BOARD_TOKEN "$BOARD_BIN" beat "$id" --final >/dev/null && env -u BOARD_TOKEN "$BOARD_BIN" move "$id" review >/dev/null
      echo "交审了 #$id"
      out=$(env -u BOARD_TOKEN "$BOARD_BIN" next --json) || { echo "会话结束"; exit 0; }
      id=$(echo "$out" | sed -n 's/^  "id": \([0-9]*\),*$/\1/p' | head -1)
      # 真的 agent 做一张要几分钟；这里等一下，让 board host 先换到新的派发再写下一张的输出
      sleep 1
    done ;;
  crash)
    echo "working"
    echo "boom"
    exit 3 ;;
  orphan)
    # 留下一个忽略 SIGTERM 的子进程（同一进程组），自己先退出
    ( trap '' TERM; exec sleep 60 ) &
    echo $! > "$FAKE_DIR/child.pid"
    echo "left a child"
    exit 0 ;;
  *)
    echo "sleeping"
    echo $$ > "$FAKE_DIR/agent.pid"
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
                // 机器上真装了 dsh、pi、codex 也不用它们：都换成找不到的命令（默认 agent 就是 claude）。
                // 不换的话启动时会真的执行它们的 --version（codex 要一两秒，机器忙时更久），同名第二个 host 来不及在时限内退出
                "[agents.dsh]\ncmd = [\"board-test-no-such-command\", \"{{prompt}}\"]\n\n[agents.claude]\ncmd = [\"{}\", \"{{prompt}}\"]\n\n[agents.pi]\nname = \"pi\"\ncmd = [\"board-test-no-such-command\", \"{{prompt}}\"]\n\n[agents.codex]\ncmd = [\"board-test-no-such-command\", \"{{prompt}}\"]\n",
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
        self.host_with(&["host", "--name", name, "--dir", &format!("default/main={}", self.repo.display())])
    }

    fn host_with(&self, args: &[&str]) -> Child {
        self.spawn_host(args, false)
    }

    /// 像桌面端那样运行：`BOARD_HOST_STDIN=1`，标准输入接管道（写一行 reload 通知重新读配置，关掉就退出）
    fn host_desktop(&self) -> Child {
        self.spawn_host(&["host"], true)
    }

    fn spawn_host(&self, args: &[&str], desktop: bool) -> Child {
        let mut c = Command::new(env!("CARGO_BIN_EXE_board"));
        c.args(args)
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
            .env_remove("BOARD_CONFIG")
            .stdin(if desktop { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if desktop {
            c.env("BOARD_HOST_STDIN", "1");
        }
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
    assert_eq!(h["agent"], "claude", "dsh 没找到时，默认 agent 是下一个找到的");

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

    // 交审：持有者是 claude-mbp/opus-5.5（<agent 标识>-<机器名>/<模型名>，模型名来自内置模板的默认值），
    // 日志写到 ~/.board/host/logs/，上传到服务端
    rig.mode("review");
    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let d1 = r.json()["id"].as_i64().unwrap();
    wait_until("#1 交审", Duration::from_secs(15), || ended(&rig.show(1), "已交审"));
    let c1 = rig.show(1);
    assert_eq!(c1["column"], "review");
    assert!(activity(&c1).iter().any(|t| t.contains("以 claude-mbp/opus-5.5 领取")), "{:?}", activity(&c1));
    let local = std::fs::read_to_string(log_file(&rig.s.home, d1)).unwrap();
    assert!(local.contains("prompt: /board-skills 开始执行 default项目之中的main白板中的#1任务"), "{local}");
    assert!(local.contains("BOARD_AS=claude-mbp/opus-5.5，BOARD_HOST=mbp"), "派发提示词里带上 board host 身份：{local}");
    assert!(local.contains("as: claude-mbp/opus-5.5 host: mbp board: default/main"), "{local}");
    assert!(rig.dispatch_log(d1).iter().any(|l| l == "交审了"));
    assert!(local.contains("config: ok"), "BOARD_CONFIG 指向这次派发的配置文件：{local}");
    assert!(local.contains("exe: ok"), "BOARD_EXE 是 board 自己的路径：{local}");
    let run = rig.s.home.join(".board").join("host").join("run").join("mbp");
    assert!(!run.join(format!("{d1}.toml")).exists(), "派发结束后删掉它的配置文件");

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
    wait_until("#3 开始", Duration::from_secs(10), || rig.show(3)["holder"] == "claude-mbp/opus-5.5");
    assert_eq!(rig.board(&["dispatch", "4", "--host", "mbp"]).code, 0);
    assert_eq!(rig.board(&["claim", "4", "--json"]).error_code(), "DISPATCHED");
    std::thread::sleep(Duration::from_secs(1));
    assert!(rig.show(4)["holder"].is_null(), "没有空位，#4 排队");

    // 停止 #3：进程结束，#3 移到需人工校验；#4 接着开始
    let r = rig.board(&["dispatch", "3", "--cancel"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    wait_until("#3 已停止", Duration::from_secs(15), || ended(&rig.show(3), "已停止"));
    assert_eq!(rig.show(3)["column"], "check");
    // 上一个会话离线后名字复用
    wait_until("#4 开始", Duration::from_secs(10), || rig.show(4)["holder"] == "claude-mbp/opus-5.5");

    // Ctrl+C（SIGTERM）：先停掉 #4，再离线
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(ended(&rig.show(4), "已停止"), "{:?}", activity(&rig.show(4)));
    assert!(rig.workspace()["hosts"][0]["online"] == false);
}

/// 定时领取的会话接着做（board next）：N = 4、6 张卡、同时跑 1 个 → 第一个进程做 4 张，第二个做 2 张；每张各一条派发
#[test]
fn cron_session_does_several_cards() {
    let rig = Rig::new();
    for i in 1..=6 {
        assert_eq!(rig.board(&["new", &format!("卡{i}")]).code, 0);
    }
    rig.mode("session");
    let host = rig.host("mbp");
    wait_until("mbp 上线", Duration::from_secs(20), || {
        rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true))
    });
    assert_eq!(rig.workspace()["hosts"][0]["perSession"], 4);
    // 没有领取规则时不自动领取：加一条命中全部卡片的规则（不带「跳过」→ claude）
    let rule = board_core::ClaimRule { op: "not_in".into(), labels: vec!["跳过".into()], agent: "claude".into(), ..Default::default() };
    let patch = board_core::HostPatch { rules: Some(vec![rule]), ..Default::default() };
    let mut store = board_core::Store::open(&rig.s.db).unwrap();
    store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.create_label("default", "跳过", None, false)).unwrap();
    store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.host_update("mbp", &patch)).unwrap();
    ureq::post(format!("{}/api/hosts/mbp/round", rig.s.url))
        .header("authorization", format!("Bearer {}", rig.s.token))
        .send_json(serde_json::json!({}))
        .unwrap();
    wait_until("6 张都交审", Duration::from_secs(60), || (1..=6).all(|i| rig.show(i)["column"] == "review"));
    wait_until("派发都结束", Duration::from_secs(20), || {
        let store = board_core::Store::open(&rig.s.db).unwrap();
        store.read(|r| r.active_dispatches()).unwrap().is_empty()
    });
    let store = board_core::Store::open(&rig.s.db).unwrap();
    let mut all = store.read(|r| r.host_recent(1, 50)).unwrap();
    all.sort_by_key(|d| d.id);
    assert_eq!(all.len(), 6, "每张卡各一条派发");
    assert!(all.iter().all(|d| d.result.as_deref() == Some("review")), "{all:?}");
    let sessions: Vec<i64> = all.iter().map(|d| d.session).collect();
    assert_eq!(sessions, vec![all[0].id, all[0].id, all[0].id, all[0].id, all[4].id, all[4].id], "4 张一个进程，剩下 2 张换新进程");
    // 进程的输出记到当时正在做的那张卡的派发上
    assert!(rig.dispatch_log(all[1].id).iter().any(|l| l == &format!("交审了 #{}", all[1].card)), "{:?}", rig.dispatch_log(all[1].id));
    assert!(activity(&rig.show(all[1].card)).iter().any(|t| t.contains("接着做（定时，这个会话的第 2 张）")));
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn end_response_failure_is_retried_with_real_exit_code() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let host = rig.host("mbp");
    wait_until("mbp 上线", Duration::from_secs(20), || {
        rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true))
    });
    rig.mode("crash");
    // 第一次 /end 返回 502（服务端重启、反向代理抖动）：host 不能丢掉这次结束，
    // 下一次上报仍然要把真实退出码和日志最后 20 行报上去，而不是「没有找到 agent 进程」。
    rig.s.state.fail_next_end();
    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let id = r.json()["id"].as_i64().unwrap();
    wait_until("#1 异常退出", Duration::from_secs(20), || ended(&rig.show(1), "异常退出"));
    let c = rig.show(1);
    assert_eq!(c["column"], "check");
    let texts = activity(&c);
    assert!(texts.iter().any(|t| t.contains("退出码 3") && t.contains("boom")), "要按真实退出码结束：{texts:?}");
    assert!(!texts.iter().any(|t| t.contains("没有找到 agent 进程")), "不能按「找不到进程」结束：{texts:?}");
    assert!(rig.dispatch_log(id).iter().any(|l| l == "boom"), "日志最后 20 行要上报");
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
}

/* ------------------------------ 不留下失控的 agent ------------------------------ */

/// 进程还在跑（僵尸算不在：容器里的 1 号进程不一定回收孤儿）
fn running(pid: i32) -> bool {
    let o = Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    !s.is_empty() && !s.starts_with('Z')
}

impl Rig {
    /// 假 agent 写下的进程号（`agent.pid`、`child.pid`）
    fn pid(&self, file: &str) -> i32 {
        let p = self.fake.parent().unwrap().join(file);
        wait_until(&format!("{file} 写好"), Duration::from_secs(15), || {
            std::fs::read_to_string(&p).is_ok_and(|s| s.trim().parse::<i32>().is_ok())
        });
        std::fs::read_to_string(&p).unwrap().trim().parse().unwrap()
    }

    fn online(&self, name: &str) {
        wait_until(&format!("{name} 上线"), Duration::from_secs(20), || {
            self.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == name && x["online"] == true))
        });
    }

    /// 派发 #id 给 mbp，等 agent 开始，返回派发 ID
    fn dispatch_running(&self, id: i64) -> i64 {
        let r = self.board(&["dispatch", &id.to_string(), "--host", "mbp", "--json"]);
        assert_eq!(r.code, 0, "{}{}", r.out, r.err);
        wait_until(&format!("#{id} 开始"), Duration::from_secs(15), || self.show(id)["holder"] == "claude-mbp/opus-5.5");
        r.json()["id"].as_i64().unwrap()
    }
}

/// 标准输出的读端已经关了（`board host | head -n 1`、终端已关）：打印失败不能 panic，
/// SIGTERM 时照样结束 agent、上报「已停止」、离线
#[test]
fn closed_stdout_does_not_leave_agents() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let mut host = rig.host("mbp");
    rig.online("mbp");
    drop(host.stdout.take());
    rig.mode("sleep");
    rig.dispatch_running(1);
    let agent = rig.pid("agent.pid");
    assert!(running(agent));
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("panicked"), "{out}");
    assert!(ended(&rig.show(1), "已停止"), "{:?}", activity(&rig.show(1)));
    assert!(!running(agent), "agent 要被结束");
    assert!(rig.workspace()["hosts"][0]["online"] == false);
}

/// host 被 kill -9 后同名重启：上一个进程留下的 agent 在启动后被结束
#[test]
fn restart_after_kill9_ends_left_agents() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let mut host = rig.host("mbp");
    rig.online("mbp");
    rig.mode("sleep");
    rig.dispatch_running(1);
    let agent = rig.pid("agent.pid");
    unsafe {
        libc_kill(host.id() as i32, 9);
    }
    host.wait().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(running(agent), "kill -9 之后 agent 还在跑（没人管）");
    let again = rig.host("mbp");
    wait_until("留下的 agent 被结束", Duration::from_secs(20), || !running(agent));
    let (code, out) = stop(again);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("结束上一次留下的 agent：default/main#1"), "{out}");
}

/// 有派发在跑时同名再启动一个：第二个报 HOST_NAME_TAKEN，不删第一个的派发配置文件
#[test]
fn second_host_keeps_first_hosts_run_files() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let host = rig.host("mbp");
    rig.online("mbp");
    rig.mode("sleep");
    let d = rig.dispatch_running(1);
    let run = rig.s.home.join(".board").join("host").join("run").join("mbp");
    // 服务端记下持有者之后 host 才启动 agent：等它写好
    wait_until("派发的配置和进程文件写好", Duration::from_secs(10), || {
        run.join(format!("{d}.toml")).exists() && run.join(format!("{d}.proc")).exists()
    });
    let mut second = rig.host("mbp");
    let (code, out) = wait_out(&mut second, Duration::from_secs(10));
    assert_eq!(code, 1);
    assert!(out.contains("HOST_NAME_TAKEN"), "{out}");
    assert!(run.join(format!("{d}.toml")).exists(), "第二个 host 不能删掉正在跑的派发的配置文件");
    assert!(run.join(format!("{d}.proc")).exists());
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(ended(&rig.show(1), "已停止"));
}

/// agent 主进程退出后，同一进程组里剩下的（忽略 SIGTERM 的）子进程也被结束
#[test]
fn agent_exit_ends_its_process_group() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let host = rig.host("mbp");
    rig.online("mbp");
    rig.mode("orphan");
    assert_eq!(rig.board(&["dispatch", "1", "--host", "mbp"]).code, 0);
    let child = rig.pid("child.pid");
    wait_until("#1 的派发结束", Duration::from_secs(15), || {
        activity(&rig.show(1)).iter().any(|t| t.starts_with("派发结束"))
    });
    wait_until("留下的子进程被结束", Duration::from_secs(10), || !running(child));
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
}

/// 桌面端「设置 → 本机」保存（改配置文件的 [host]，再往 board host 的标准输入写一行 reload）：board host 不重启，
/// 换上新的机器名、白板与目录。不通知就不重新读配置。在跑的 agent 不被停止，接着由同一台机器（改名后的记录）续心跳；
/// 机器列表里始终只有一台
#[test]
fn host_follows_config_without_restart() {
    let rig = Rig::new();
    for t in ["一", "二"] {
        assert_eq!(rig.board(&["new", t]).code, 0);
    }
    let cfg = rig.s.home.join(".board").join("config.toml");
    let base = std::fs::read_to_string(&cfg).unwrap();
    let host_conf = |name: &str, dir: &Path| format!("{base}\n[host]\nname = \"{name}\"\n\n[host.dirs]\n\"default/main\" = \"{}\"\n", dir.display());
    std::fs::write(&cfg, host_conf("mbp", &rig.repo)).unwrap();
    // 像桌面端那样运行，不带 --name、--dir：都跟着配置文件
    let mut host = rig.host_desktop();
    let mut stdin = host.stdin.take().unwrap();
    let mut reload = || {
        use std::io::Write;
        stdin.write_all(b"reload\n").unwrap();
        stdin.flush().unwrap();
    };
    wait_until("mbp 上线", Duration::from_secs(20), || {
        rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true))
    });
    let id = rig.workspace()["hosts"][0]["id"].clone();
    // 一个 agent 在跑
    rig.mode("sleep");
    assert_eq!(rig.board(&["dispatch", "1", "--host", "mbp"]).code, 0);
    wait_until("#1 开始", Duration::from_secs(10), || rig.show(1)["holder"] == "claude-mbp/opus-5.5");
    let pidfile = rig.fake.parent().unwrap().join("agent.pid");
    wait_until("agent 写出进程号", Duration::from_secs(10), || std::fs::read_to_string(&pidfile).is_ok_and(|s| !s.trim().is_empty()));
    let pid = std::fs::read_to_string(&pidfile).unwrap();

    // 桌面端保存：先在服务端改名（board host rename），再写配置文件；目录也换一个
    let other = rig.repo.with_file_name("repo2");
    std::fs::create_dir(&other).unwrap();
    let mut env = rig.s.env();
    env.retain(|(k, _)| *k != "BOARD_AS" && *k != "BOARD");
    let r = rig.s.raw(&env, &["host", "rename", "win", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    std::fs::write(&cfg, host_conf("win", &other)).unwrap();
    // 没有通知：不去读配置，目录还是原来的
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(rig.workspace()["hosts"][0]["dirs"][0]["path"], rig.repo.display().to_string());
    reload();
    wait_until("board host 换上新目录", Duration::from_secs(10), || {
        let ws = rig.workspace();
        let hs = ws["hosts"].as_array().unwrap();
        hs.len() == 1 && hs[0]["name"] == "win" && hs[0]["dirs"][0]["path"] == other.display().to_string()
    });
    // 再过几次上报：还是一台机器（同一条记录），agent 没被停止，卡片仍由它持有
    std::thread::sleep(Duration::from_secs(2));
    let ws = rig.workspace();
    let hs = ws["hosts"].as_array().unwrap();
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!((hs[0]["id"].clone(), hs[0]["online"].clone()), (id, Value::Bool(true)));
    assert_eq!(rig.show(1)["holder"], "claude-mbp/opus-5.5");
    assert!(Command::new("kill").args(["-0", pid.trim()]).status().unwrap().success(), "在跑的 agent 没有被停止");
    let home = rig.s.home.join(".board").join("host");
    assert!(home.join("win.pid").exists() && !home.join("mbp.pid").exists(), "pid 文件换成新名字");

    // 新派发用新名字、新目录
    let r = rig.board(&["dispatch", "1", "--cancel"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    wait_until("#1 已停止", Duration::from_secs(15), || ended(&rig.show(1), "已停止"));
    rig.mode("review");
    let r = rig.board(&["dispatch", "2", "--host", "win", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let d2 = r.json()["id"].as_i64().unwrap();
    wait_until("#2 交审", Duration::from_secs(15), || ended(&rig.show(2), "已交审"));
    let local = std::fs::read_to_string(log_file(&rig.s.home, d2)).unwrap();
    assert!(local.contains("as: claude-win/opus-5.5 host: win"), "{local}");

    // 目录不存在不算配置不对（#185）：照样换上，上报「不存在」，界面上可以克隆
    let nope = rig.repo.with_file_name("nope");
    std::fs::write(&cfg, host_conf("win", &nope)).unwrap();
    reload();
    wait_until("board host 换上不存在的目录并上报", Duration::from_secs(10), || {
        let d = rig.workspace()["hosts"][0]["dirs"][0].clone();
        d["path"] == nope.display().to_string() && d["status"] == "missing"
    });

    // 配置不对（白板没写成 <项目>/<白板>）：打印一行，接着用原来的
    std::fs::write(&cfg, format!("{base}\n[host]\nname = \"win\"\n\n[host.dirs]\n\"nope\" = \"{}\"\n", other.display())).unwrap();
    reload();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(rig.workspace()["hosts"][0]["dirs"][0]["path"], nope.display().to_string());

    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("机器名已改为 win"), "{out}");
    assert!(out.contains("白板与目录没有更新"), "{out}");
}

/// 白板的环境检查（PRD §9.1）：board host 在白板目录里跑检查命令、上报结果；没通过的机器派发被拒（BOARD_NOT_READY），
/// 命令改了马上重跑，通过之后恢复
#[test]
fn ready_check_runs_in_board_dir_and_gates_dispatch() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let host = rig.host("mbp");
    wait_until("mbp 上线", Duration::from_secs(20), || {
        rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true))
    });
    // 白板设置只有人能改（界面）：直接写数据库，board host 下一次上报就拿到新命令
    let patch = |cmd: &str| {
        let mut store = board_core::Store::open(&rig.s.db).unwrap();
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.set_ready_check("default/main", Some(cmd), None)).unwrap();
    };
    let dir = |ws: &Value| ws["hosts"][0]["dirs"][0].clone();
    // 检查命令在白板目录里执行：目录里没有 ok.txt，不通过，输出的最后一行写进派发的拒绝原因
    patch("echo checking; test -f ok.txt || { echo 'missing ok.txt'; exit 4; }");
    wait_until("检查没通过", Duration::from_secs(20), || dir(&rig.workspace())["ready"]["ok"] == false && dir(&rig.workspace())["ready"]["cmd"].as_str().is_some_and(|c| c.contains("ok.txt")));
    let d = dir(&rig.workspace());
    assert_eq!(d["ready"]["code"], 4, "{d}");
    assert_eq!(d["ready"]["tail"], serde_json::json!(["checking", "missing ok.txt"]), "{d}");
    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    assert_eq!(r.error_code(), "BOARD_NOT_READY", "{}{}", r.out, r.err);
    assert!(r.out.contains("missing ok.txt") || r.err.contains("missing ok.txt"), "{}{}", r.out, r.err);
    // 补上文件、改命令（命令变了马上重跑）：通过，可以派发
    std::fs::write(rig.repo.join("ok.txt"), "").unwrap();
    patch("test -f ok.txt");
    wait_until("检查通过", Duration::from_secs(20), || {
        let d = dir(&rig.workspace());
        d["ready"]["ok"] == true && d["ready"]["cmd"] == "test -f ok.txt"
    });
    rig.mode("review");
    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    wait_until("#1 交审", Duration::from_secs(15), || ended(&rig.show(1), "已交审"));
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("环境检查没通过：missing ok.txt") && out.contains("环境检查通过"), "{out}");
}

/// 远程管理（PRD §9.1）：界面上请求「重新检查」，board host 重新找 agent（新配的 agent 出现）；
/// 「重启 board host」后原地换成新进程（进程号不变）、重新上线
#[test]
fn remote_recheck_and_restart() {
    let rig = Rig::new();
    let mut host = rig.host("mbp");
    let online = |rig: &Rig| rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true));
    wait_until("mbp 上线", Duration::from_secs(20), || online(&rig));
    let ask = |kind: &str| {
        let mut store = board_core::Store::open(&rig.s.db).unwrap();
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.host_ask("mbp", kind, false, "octocat")).unwrap()
    };
    let has_agent = |rig: &Rig, id: &str| rig.workspace()["hosts"][0]["agents"].as_array().unwrap().iter().any(|a| a["id"] == id && a["found"] == true);
    assert!(!has_agent(&rig, "newbie"));
    // 在这台机器上配一个新的 agent，再从界面请求重新检查：10 秒内出现，带版本号
    let conf = rig.s.home.join(".board").join("config.toml");
    let mut text = std::fs::read_to_string(&conf).unwrap();
    text += &format!("\n[agents.newbie]\nname = \"Newbie\"\ncmd = [\"{}\", \"{{prompt}}\"]\n", rig.fake.display());
    std::fs::write(&conf, text).unwrap();
    ask("recheck");
    wait_until("新 agent 出现", Duration::from_secs(10), || has_agent(&rig, "newbie"));
    let h = rig.workspace()["hosts"][0].clone();
    assert!(h["ask"].is_null(), "{h}");
    assert!(h["recheckedAt"].is_i64(), "{h}");
    // 重启：先离线（final），再以新进程上线；进程号不变（原地换进程）。新进程的标识写在 pid 文件里：
    // 看它换了（离线到上线可能快得轮询看不到）
    let pid = host.id();
    let pidfile = rig.s.home.join(".board").join("host").join("mbp.pid");
    let instance = |p: &Path| std::fs::read_to_string(p).ok().and_then(|s| s.split_whitespace().nth(2).map(str::to_string));
    let before = instance(&pidfile).expect("pid 文件");
    ask("restart");
    wait_until("重启成新进程", Duration::from_secs(20), || instance(&pidfile).is_some_and(|i| i != before));
    wait_until("重启后上线", Duration::from_secs(20), || online(&rig));
    assert!(host.try_wait().unwrap().is_none(), "还是同一个进程 {pid}");
    let ops: Vec<String> = rig.workspace()["hosts"][0]["ops"].as_array().unwrap().iter().map(|o| o["text"].as_str().unwrap().to_string()).collect();
    assert!(ops.iter().any(|t| t == "正在重启 board host"), "{ops:?}");
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("重新检查完了") && out.contains("请求重启 board host") && out.contains("重新启动 board host"), "{out}");
}

/// 机器设置里的模型、思考强度（PRD §9.1）：不重启 board host，下一次派发就按模板把参数传给 agent，
/// agent 名字第二段用模型，卡片动态的「派发开始」写着模型和强度
#[test]
fn model_and_effort_reach_the_agent() {
    let rig = Rig::new();
    // claude 换成一个把参数打印出来的假脚本（内置模板的 --model / --effort 照样插在提示词前面）
    let printer = rig.fake.parent().unwrap().join("printer.sh");
    std::fs::write(&printer, "#!/bin/sh\necho \"args: $*\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&printer, std::fs::Permissions::from_mode(0o755)).unwrap();
    let conf = rig.s.home.join(".board").join("config.toml");
    let text = std::fs::read_to_string(&conf).unwrap().replace(&rig.fake.display().to_string(), &printer.display().to_string());
    std::fs::write(&conf, text).unwrap();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let host = rig.host("mbp");
    wait_until("mbp 上线", Duration::from_secs(20), || {
        rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true))
    });
    let a = rig.workspace()["hosts"][0]["agents"].as_array().unwrap().iter().find(|a| a["id"] == "claude").cloned().unwrap();
    assert_eq!(a["opts"]["efforts"], serde_json::json!(["low", "medium", "high", "xhigh", "max"]), "{a}");
    {
        let mut store = board_core::Store::open(&rig.s.db).unwrap();
        let run = board_core::RunPatch { agent: "claude".into(), model: Some("sonnet".into()), effort: Some("high".into()) };
        let p = board_core::HostPatch { run: Some(run), ..Default::default() };
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.host_update("mbp", &p)).unwrap();
    }
    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let d = r.json()["id"].as_i64().unwrap();
    let log = log_file(&rig.s.home, d);
    wait_until("假 agent 打印参数", Duration::from_secs(15), || std::fs::read_to_string(&log).is_ok_and(|s| s.contains("args:")));
    let out = std::fs::read_to_string(&log).unwrap();
    assert!(out.contains("args: --model sonnet --effort high /board-skills 开始执行"), "{out}");
    let acts = activity(&rig.show(1));
    assert!(acts.iter().any(|t| t.contains("以 claude-mbp/sonnet 领取") && t.contains("模型 sonnet，强度 high")), "{acts:?}");
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
}
