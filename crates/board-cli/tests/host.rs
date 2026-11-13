//! `board host` / `board dispatch` 端到端（任务下发需求 §十二）：起一个服务端，在后台跑真的 board host，
//! 「agent」是一个 shell 脚本（按 mode 文件决定交审、崩溃还是一直睡），用配置文件的 `[agents.claude]` 换掉内置模板。
#![cfg(unix)]

mod common;
use common::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const FAKE: &str = r#"#!/bin/sh
# 假 agent：$1 是提示词；按 $FAKE_DIR/mode 决定做什么
if [ "$1" = "--version" ]; then echo "9.9.9 (fake)"; exit 0; fi
# claude mcp add-json --scope user <名称> <JSON> / claude mcp remove --scope user <名称>：改 ~/.claude.json 的 mcpServers
if [ "$1" = "mcp" ]; then
  python3 -c '
import json, os, sys
p = os.path.expanduser("~/.claude.json")
d = json.load(open(p)) if os.path.exists(p) else {}
a = sys.argv[1:]
d.setdefault("mcpServers", {})
if a[1] == "add-json":
    d["mcpServers"][a[4]] = json.loads(a[5])
elif a[1] == "remove":
    d["mcpServers"].pop(a[4], None)
json.dump(d, open(p, "w"))
' "$@"
  exit $?
fi
# 自检问的那一句：答出 skill 的名字；mode 是 nologin 时像没登录的 Claude Code 那样报错退出
case "$1" in *不要执行任何命令*)
  if [ "$(cat "$FAKE_DIR/mode" 2>/dev/null)" = "nologin" ]; then echo "Not logged in · Please run /login"; exit 1; fi
  printf 'board-skills\n参数\n'; exit 0 ;;
esac
# 一行都不输出（Claude Code -p 运行中就是这样）：不该按「没输出多久算卡住」停止。放在前面，下面几行也不打
if [ "$(cat "$FAKE_DIR/mode")" = "quiet" ]; then
  echo $$ > "$FAKE_DIR/agent.pid"
  exec sleep 120
fi
echo "prompt: $1"
echo "as: $BOARD_AS host: $BOARD_HOST board: $BOARD"
[ -f "$BOARD_CONFIG" ] && echo "config: ok"
[ -x "$BOARD_EXE" ] && echo "exe: ok"
id=$(echo "$1" | sed 's/.*#\([0-9]*\)任务.*/\1/')
# 拆解「未整理」里的条目：提示词是「/board-skills 拆解 …#<ID>需求（…）」
need=$(echo "$1" | sed -n 's/^\/board-skills 拆解 .*#\([0-9]*\)需求.*/\1/p')
case "$(cat "$FAKE_DIR/mode")" in
  split2)
    # 按提示词拆出两张，记回未整理、挂在原需求下面，在原条目上评论后退出 0
    env -u BOARD_TOKEN "$BOARD_BIN" new "拆出的一" --inbox --from "$need" >/dev/null || exit 9
    env -u BOARD_TOKEN "$BOARD_BIN" new "拆出的二" --inbox --from "$need" >/dev/null || exit 9
    env -u BOARD_TOKEN "$BOARD_BIN" comment "$need" "拆成了两张" >/dev/null
    echo "拆完了 #$need"
    exit 0 ;;
  split0)
    echo "没拆出来 #$need"
    exit 0 ;;
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
  leak)
    # 像 agent 执行 env、报错时带出请求头那样，把令牌和其他密钥打到标准输出/标准错误
    echo "BOARD_TOKEN=$BOARD_TOKEN"
    echo "Authorization: Bearer $BOARD_TOKEN"
    echo "key ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" >&2
    echo "working"
    exit 3 ;;
  orphan)
    # 留下一个忽略 SIGTERM 的子进程（同一进程组），自己先退出
    ( trap '' TERM; exec sleep 60 ) &
    echo $! > "$FAKE_DIR/child.pid"
    echo "left a child"
    exit 0 ;;
  grandchild)
    # 先留下一个忽略 SIGTERM 的子进程（同一进程组），自己接着睡：board host 退出时两个都要结束（卡片 #251）。
    # 子进程忽略 SIGTERM，只有 `kill_rest()` 的 SIGKILL 能结束它
    ( trap '' TERM; exec sleep 60 ) &
    echo $! > "$FAKE_DIR/child.pid"
    echo "started a child"
    echo $$ > "$FAKE_DIR/agent.pid"
    exec sleep 60 ;;
  stream)
    # 像 Claude Code 的 stream-json 那样一行一个事件：先说一句话、调一个工具，等测试写 go 文件再出结果、异常退出
    echo '{"type":"system","subtype":"init","cwd":"/repo","model":"m","claude_code_version":"9.9.9"}'
    echo '{"type":"assistant","message":{"content":[{"type":"text","text":"先领卡，再跑测试。"}]},"parent_tool_use_id":null}'
    echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]},"parent_tool_use_id":null}'
    touch "$FAKE_DIR/streamed"
    i=0; while [ ! -f "$FAKE_DIR/go" ] && [ $i -lt 300 ]; do sleep 0.1; i=$((i+1)); done
    echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"test failed","is_error":true}]},"parent_tool_use_id":null}'
    echo '{"type":"result","subtype":"error_during_execution","is_error":true,"duration_ms":5000,"num_turns":2,"total_cost_usd":0.5,"usage":{"input_tokens":100,"output_tokens":10},"errors":["测试没过"]}'
    exit 4 ;;
  chatty)
    # 输出一行就再也不输出：到「没输出多久算卡住」的时间该被停止
    echo "working"
    echo $$ > "$FAKE_DIR/agent.pid"
    exec sleep 120 ;;
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

    /// board host 启动时读的 `BOARD_HOST_MINUTE_MS`：兜底相关的测试先写 `minute-ms` 文件把它缩短，
    /// 其余按默认的 60 秒
    fn minute_ms(&self) -> String {
        std::fs::read_to_string(self.fake.parent().unwrap().join("minute-ms"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "60000".into())
    }

    fn host(&self, name: &str) -> Child {
        self.host_with(&["host", "--name", name, "--dir", &format!("default/main={}", self.repo.display())])
    }

    fn host_with(&self, args: &[&str]) -> Child {
        self.spawn_host(args, false)
    }

    /// `board host --service …`（守护进程）：board host 异常退出后重启的等待缩短到 0.5 秒
    fn service(&self, name: &str) -> Child {
        let dir = format!("default/main={}", self.repo.display());
        self.spawn_host_env(&["host", "--service", "--name", name, "--dir", &dir], false, &[("BOARD_HOST_RESTART_MS", "500")])
    }

    /// 像桌面端那样运行：`BOARD_HOST_STDIN=1`，标准输入接管道（写一行 reload 通知重新读配置，关掉就退出）
    fn host_desktop(&self) -> Child {
        self.spawn_host(&["host"], true)
    }

    fn spawn_host(&self, args: &[&str], desktop: bool) -> Child {
        self.spawn_host_env(args, desktop, &[])
    }

    fn spawn_host_env(&self, args: &[&str], desktop: bool, extra: &[(&str, &str)]) -> Child {
        let mut c = Command::new(env!("CARGO_BIN_EXE_board"));
        c.envs(extra.iter().copied());
        c.args(args)
            .env("HOME", &self.s.home)
            .env("USERPROFILE", &self.s.home)
            .env("NO_COLOR", "1")
            .env("BOARD_URL", &self.s.url)
            .env("BOARD_TOKEN", &self.s.token)
            .env("BOARD_HOST_INTERVAL", "0.5")
            // 兜底设置的最小档是分钟：测试里一分钟 = 1 秒（`minute-ms` 文件，见 Rig::minute_ms），
            // 30 分钟、20 分钟才等得起。其他测试用默认的 60 秒，没有影响
            .env("BOARD_HOST_MINUTE_MS", self.minute_ms())
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

    /// 像界面一样 GET（会话 Cookie）：会话页这类只给界面的接口
    fn ui_get(&self, path: &str) -> Value {
        let sid = self.ui_session();
        let mut res = ureq::get(&format!("{}{path}", self.s.url)).header("cookie", format!("board_session={sid}")).call().unwrap();
        serde_json::from_str(&res.body_mut().read_to_string().unwrap()).unwrap()
    }

    /// 界面的登录会话：直接写进服务端的库，返回 Cookie 里的值
    fn ui_session(&self) -> &'static str {
        let sid = "test-ui-session";
        let hash: String = Sha256::digest(sid.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let store = board_core::Store::open(&self.s.db).unwrap();
        let now = board_core::now_ms();
        store
            .conn()
            .execute(
                "INSERT OR REPLACE INTO auth_sessions (hash, login, name, avatar, created_at, expires_at) VALUES (?1, 'buglan', 'buglan', '', ?2, ?3)",
                rusqlite::params![hash, now, now + 86_400_000],
            )
            .unwrap();
        sid
    }

    /// 像界面一样 POST（GitHub 会话 Cookie + `x-board-client`），只给界面上才有的接口（全部暂停、全部停止）用。
    /// 会话直接写进服务端的库：测试不经过 GitHub 登录
    fn ui_post(&self, path: &str, body: Value) -> Value {
        let sid = "test-ui-session";
        let hash: String = Sha256::digest(sid.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let store = board_core::Store::open(&self.s.db).unwrap();
        let now = board_core::now_ms();
        store
            .conn()
            .execute(
                "INSERT OR REPLACE INTO auth_sessions (hash, login, name, avatar, created_at, expires_at) VALUES (?1, 'buglan', 'buglan', '', ?2, ?3)",
                rusqlite::params![hash, now, now + 86_400_000],
            )
            .unwrap();
        drop(store);
        let mut res = ureq::post(&format!("{}{path}", self.s.url))
            .header("cookie", format!("board_session={sid}"))
            .header("x-board-client", "test")
            .send_json(body)
            .unwrap();
        serde_json::from_str(&res.body_mut().read_to_string().unwrap()).unwrap_or(Value::Null)
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

/// 用访问令牌 GET 一个接口：返回（状态码，正文）
fn get_raw(url: &str, token: &str) -> (u16, String) {
    let a: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut res = a.get(url).header("authorization", format!("Bearer {token}")).call().unwrap();
    (res.status().as_u16(), res.body_mut().read_to_string().unwrap())
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

    // 最多 1 个会话：#3 在跑时派发 #4 排队，别的 agent 领 #4 报 DISPATCHED
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

/// 未整理里的条目派给 agent 拆解（PRD §9.1）：不领取、不移动；按建出的子卡数记「已拆解」「没拆出卡」，退出码非 0 记「异常退出」
#[test]
fn host_splits_inbox_entries() {
    let rig = Rig::new();
    for t in ["需求一", "需求二", "需求三"] {
        assert_eq!(rig.board(&["new", t, "--inbox"]).code, 0);
    }
    let host = rig.host("mbp");
    wait_until("mbp 上线", Duration::from_secs(20), || {
        rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true))
    });
    let split_ended = |id: i64, what: &str| activity(&rig.show(id)).iter().any(|t| t.starts_with(&format!("拆解结束：{what}")));

    // 拆出两张：#1 一直在未整理、没人持有；两张新条目拆自 #1
    rig.mode("split2");
    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["kind"], "split");
    let d1 = r.json()["id"].as_i64().unwrap();
    wait_until("#1 已拆解", Duration::from_secs(15), || split_ended(1, "已拆解"));
    let c1 = rig.show(1);
    assert_eq!((c1["column"].as_str(), c1["holder"].is_null()), (Some("inbox"), true), "拆解不领取、不移动");
    assert_eq!(c1["split_into"], serde_json::json!([4, 5]));
    for id in [4, 5] {
        let c = rig.show(id);
        assert_eq!((c["column"].as_str(), c["split_from"].as_i64()), (Some("inbox"), Some(1)));
    }
    assert!(activity(&c1).iter().any(|t| t == "拆成了两张"), "{:?}", activity(&c1));
    let local = std::fs::read_to_string(log_file(&rig.s.home, d1)).unwrap();
    assert!(local.contains("prompt: /board-skills 拆解 default项目之中的main白板中的#1需求（board host 派发：BOARD_AS=claude-mbp/opus-5.5，BOARD_HOST=mbp"), "{local}");

    // 一张也没建、退出 0：没拆出卡
    rig.mode("split0");
    assert_eq!(rig.board(&["dispatch", "2", "--host", "mbp"]).code, 0);
    wait_until("#2 没拆出卡", Duration::from_secs(15), || split_ended(2, "没拆出卡"));
    assert_eq!(rig.show(2)["column"], "inbox");

    // 退出 3：异常退出，在原条目上评论退出码和最后几行输出，条目仍在未整理
    rig.mode("crash");
    assert_eq!(rig.board(&["dispatch", "3", "--host", "mbp"]).code, 0);
    wait_until("#3 异常退出", Duration::from_secs(15), || split_ended(3, "异常退出"));
    let c3 = rig.show(3);
    assert_eq!((c3["column"].as_str(), c3["holder"].is_null()), (Some("inbox"), true));
    assert!(activity(&c3).iter().any(|t| t.contains("退出码 3") && t.contains("boom")), "{:?}", activity(&c3));

    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
}

/// 定时领取的会话接着做（board next）：N = 4、6 张卡、最多 1 个会话 → 第一个进程做 4 张，第二个做 2 张；每张各一条派发
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

/// 卡片 #199：agent 把令牌和密钥打到标准输出、标准错误里，上传到服务端的日志和异常退出评论里都只有
/// `<已隐藏>`，本机日志文件里是原文
#[test]
fn tokens_are_redacted_before_upload_but_kept_locally() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let host = rig.host("mbp");
    wait_until("mbp 上线", Duration::from_secs(20), || {
        rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true))
    });
    rig.mode("leak");
    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let id = r.json()["id"].as_i64().unwrap();
    wait_until("#1 异常退出", Duration::from_secs(20), || ended(&rig.show(1), "异常退出"));
    let token = rig.s.token.clone();
    let gh = "ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    // 服务端保存的日志：令牌原文一处不留
    let log = rig.dispatch_log(id);
    assert!(log.iter().any(|l| l == "BOARD_TOKEN=<已隐藏>"), "{log:?}");
    assert!(log.iter().any(|l| l == "Authorization: Bearer <已隐藏>"), "{log:?}");
    assert!(log.iter().any(|l| l == "key <已隐藏>"), "ghp_ 也要隐藏：{log:?}");
    assert!(!log.iter().any(|l| l.contains(&token) || l.contains(gh)), "服务端日志里不能有令牌、密钥原文：{log:?}");
    // 异常退出评论里的「日志最后 20 行」同样脱敏
    let c = rig.show(1);
    let texts = activity(&c);
    // 「开始第 1 次（……判定异常退出）」也带这几个字：找 agent 异常退出的那条评论
    let comment = texts.iter().find(|t| t.starts_with("agent 异常退出")).unwrap_or_else(|| panic!("{texts:?}"));
    assert!(!comment.contains(&token) && !comment.contains(gh), "评论里不能有令牌、密钥原文：{comment}");
    assert!(comment.contains("<已隐藏>"), "{comment}");
    // 本机日志保留原文
    let local = std::fs::read_to_string(log_file(&rig.s.home, id)).unwrap();
    assert!(local.contains(&format!("BOARD_TOKEN={token}")), "本机日志保留原文：{local}");
    assert!(local.contains(gh), "{local}");
    // 本机日志只有自己能读（0600）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(log_file(&rig.s.home, id)).unwrap().permissions().mode() & 0o777, 0o600);
    }
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

/// 卡片 #251：board host 退出时结束的不只是 agent 主进程，它起的子进程（同一进程组）也要结束。
/// 和 `agent_exit_ends_its_process_group` 一起守住 `finish_run()` 里 `kill_rest()` 那一行
#[test]
fn host_exit_ends_the_whole_agent_process_group() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let host = rig.host("mbp");
    rig.online("mbp");
    rig.mode("grandchild");
    assert_eq!(rig.board(&["dispatch", "1", "--host", "mbp"]).code, 0);
    let agent = rig.pid("agent.pid");
    let child = rig.pid("child.pid");
    assert!(running(agent) && running(child));
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    wait_until("agent 和它的子进程都被结束", Duration::from_secs(15), || !running(agent) && !running(child));
    assert!(ended(&rig.show(1), "已停止"), "{:?}", activity(&rig.show(1)));
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

/* ------------------------------ 兜底：单次最长运行、没输出多久算卡住 ------------------------------ */

impl Rig {
    /// 在界面上改这台机器的一项设置（走服务端，和界面一样）
    fn set_host(&self, name: &str, patch: serde_json::Value) {
        let mut store = board_core::Store::open(&self.s.db).unwrap();
        let p: board_core::HostPatch = serde_json::from_value(patch).unwrap();
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.host_update(name, &p)).unwrap();
    }

    /// 在这块白板上加一个错误节点（「搁置」，从 doing 连过去），超时的卡片移到这里；返回它的标识
    fn add_error_status(&self) -> String {
        let mut store = board_core::Store::open(&self.s.db).unwrap();
        let now = board_core::now_ms();
        let st = store
            .write(now, board_core::Origin::ui(None), |w| w.create_status("default/main", "搁置", None))
            .unwrap();
        store
            .write(now, board_core::Origin::ui(None), |w| {
                w.update_edges("default/main", &[(String::from("doing"), st.key.clone())], &[])
            })
            .unwrap();
        store
            .write(now, board_core::Origin::ui(None), |w| {
                w.update_status("default/main", &st.key, board_core::StatusPatch { err: Some(true), ..Default::default() })
            })
            .unwrap();
        st.key
    }
}

/// #198 验收 1：单次最长运行到时间后，进程树按「人停止」的流程被结束，结果「超时」，
/// 卡片移到错误节点，评论写明原因和日志最后 20 行
#[test]
fn max_run_stops_agent_and_records_timeout() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let error = rig.add_error_status();
    rig.minute_ms_short();
    let host = rig.host("mbp");
    rig.online("mbp");
    // 界面上把单次最长运行改成 30 分钟、关掉「没输出多久算卡住」（假 agent 输出一行后就睡，不然它先到）；
    // 不重启 board host
    rig.set_host("mbp", serde_json::json!({ "maxRun": 30, "idleRun": 0 }));
    rig.mode("sleep");
    let d = rig.dispatch_running(1);
    let agent = rig.pid("agent.pid");
    assert!(running(agent));
    // 30 分钟 × 1 秒/分钟：到时间后 host 自己结束它
    wait_until("#1 超时", Duration::from_secs(45), || ended(&rig.show(1), "超时"));
    let c = rig.show(1);
    assert_eq!(c["column"], error, "超时按异常退出的规则移到错误节点");
    let texts = activity(&c);
    assert!(texts.iter().any(|t| t.contains("运行超过 30 分钟，board host 停止了它")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("sleeping")), "评论里附上日志最后几行：{texts:?}");
    assert!(!running(agent), "进程树要被结束");
    // 派发记录的结果
    let store = board_core::Store::open(&rig.s.db).unwrap();
    let d1 = store.read(|r| r.dispatch(d)).unwrap();
    assert_eq!(d1.result.as_deref(), Some("timeout"));
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
}

/// #198 验收 2：输出过内容后长时间没有输出的假 agent 到时间被停止；一行都没输出过的（Claude Code -p）不受影响
#[test]
fn idle_run_stops_only_after_output() {
    let rig = Rig::new();
    for t in ["一", "二"] {
        assert_eq!(rig.board(&["new", t]).code, 0);
    }
    rig.minute_ms_short();
    let host = rig.host("mbp");
    rig.online("mbp");
    // 最多 2 个会话；没输出 5 分钟算卡住；单次最长运行不限。
    // 用直接启动：登录 shell 自己会往标准错误里写几行（「no job control…」），那样就没有「一行都没输出」的派发了
    rig.set_host("mbp", serde_json::json!({ "max": 2, "idleRun": 5, "maxRun": 0, "env": { "agent": "claude", "env": "direct" } }));
    // 一：输出一行后再也不输出 → 该被停止
    rig.mode("chatty");
    rig.dispatch_running(1);
    let chatty = rig.pid("agent.pid");
    wait_until("#1 超时", Duration::from_secs(40), || ended(&rig.show(1), "超时"));
    assert!(activity(&rig.show(1)).iter().any(|t| t.contains("5 分钟没有任何输出，board host 停止了它")));
    assert!(!running(chatty));
    // 二：一行都不输出 → 不按这条算，接着跑
    std::fs::remove_file(rig.fake.parent().unwrap().join("agent.pid")).unwrap();
    rig.mode("quiet");
    assert_eq!(rig.board(&["dispatch", "2", "--host", "mbp"]).code, 0);
    wait_until("#2 开始", Duration::from_secs(15), || rig.show(2)["holder"] == "claude-mbp/opus-5.5");
    let quiet = rig.pid("agent.pid");
    std::thread::sleep(Duration::from_secs(12));
    assert!(running(quiet), "一行都没输出过的派发不受「没输出多久算卡住」影响");
    assert_eq!(rig.show(2)["column"], "doing");
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(ended(&rig.show(2), "已停止"));
}

impl Rig {
    /// 兜底相关的测试：一分钟 = 1 秒（必须在启动 board host 之前写）
    fn minute_ms_short(&self) {
        std::fs::write(self.fake.parent().unwrap().join("minute-ms"), "1000").unwrap();
    }
}

/// #198 验收 3：改了兜底设置，board host 不重启就按新值算
#[test]
fn run_limits_apply_without_restart() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    rig.minute_ms_short();
    let host = rig.host("mbp");
    rig.online("mbp");
    rig.mode("sleep");
    rig.dispatch_running(1);
    // 默认 3 小时（180 秒）：等 10 秒还在跑
    std::thread::sleep(Duration::from_secs(10));
    assert_eq!(rig.show(1)["column"], "doing", "默认 3 小时，10 秒不该停");
    // 改成 30 分钟（30 秒）：同一个进程、不重启，接下来按新值算。关掉「没输出多久算卡住」，不然默认的 20 分钟先到
    rig.set_host("mbp", serde_json::json!({ "maxRun": 30, "idleRun": 0 }));
    wait_until("#1 超时", Duration::from_secs(45), || ended(&rig.show(1), "超时"));
    assert!(activity(&rig.show(1)).iter().any(|t| t.contains("运行超过 30 分钟")));
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
}

/// #198 验收 4：两台机器都在跑时点「全部停止」：确认后（界面上）两边的派发都在 10 秒内结束，
/// 两台机器都变成暂停；「全部恢复」后恢复接单。排队的派发继续排队
#[test]
fn pause_all_and_stop_all_from_ui() {
    let rig = Rig::new();
    for t in ["一", "二"] {
        assert_eq!(rig.board(&["new", t]).code, 0);
    }
    let a = rig.host("mbp");
    rig.online("mbp");
    let b = rig.host("desk");
    rig.online("desk");
    rig.mode("sleep");
    assert_eq!(rig.board(&["dispatch", "1", "--host", "mbp"]).code, 0);
    assert_eq!(rig.board(&["dispatch", "2", "--host", "desk"]).code, 0);
    wait_until("#1 开始", Duration::from_secs(15), || rig.show(1)["holder"] == "claude-mbp/opus-5.5");
    wait_until("#2 开始", Duration::from_secs(15), || rig.show(2)["holder"] == "claude-desk/opus-5.5");

    // 全部暂停：两台都暂停接单，在跑的不受影响
    let hosts = rig.ui_post("/api/hosts/pause-all", serde_json::json!({ "paused": true }));
    assert!(hosts.as_array().is_some_and(|h| h.iter().all(|x| x["paused"] == true)), "{hosts}");
    assert_eq!(rig.workspace()["dispatches"][0]["state"], "running");

    // 全部恢复
    let hosts = rig.ui_post("/api/hosts/pause-all", serde_json::json!({ "paused": false }));
    assert!(hosts.as_array().is_some_and(|h| h.iter().all(|x| x["paused"] == false)), "{hosts}");

    // 全部停止：两边的派发 10 秒内结束，两台都暂停
    let hosts = rig.ui_post("/api/hosts/stop-all", serde_json::json!({}));
    assert!(hosts.as_array().is_some_and(|h| h.iter().all(|x| x["paused"] == true)), "{hosts}");
    wait_until("#1 已停止", Duration::from_secs(10), || ended(&rig.show(1), "已停止"));
    wait_until("#2 已停止", Duration::from_secs(10), || ended(&rig.show(2), "已停止"));
    let texts = activity(&rig.show(1));
    assert!(texts.iter().any(|t| t.contains("buglan 停止了这次派发")), "停止评论写是谁停的：{texts:?}");
    let ws = rig.workspace();
    assert!(ws["hosts"].as_array().unwrap().iter().all(|h| h["paused"] == true));

    // 暂停时新的派发排队
    assert_eq!(rig.board(&["new", "三"]).code, 0);
    assert_eq!(rig.board(&["dispatch", "3", "--host", "mbp"]).code, 0);
    std::thread::sleep(Duration::from_secs(2));
    assert!(rig.show(3)["holder"].is_null(), "全部停止之后暂停接单，新的派发排队");

    // 恢复接单后，排队的接着开始
    rig.ui_post("/api/hosts/pause-all", serde_json::json!({ "paused": false }));
    wait_until("#3 开始", Duration::from_secs(15), || rig.show(3)["holder"] == "claude-mbp/opus-5.5");

    let (code, out) = stop(a);
    assert_eq!(code, 0, "{out}");
    let (code, out) = stop(b);
    assert_eq!(code, 0, "{out}");
}

/* ------------------------------ 自检（PRD §10.12「自检」） ------------------------------ */

/// #191：界面上请求自检，board host 在后台用假 agent 问一句、对白板目录做 git fetch 和 push --dry-run，回报结果；
/// 假 agent 说没登录、origin 指向不存在的地址时对应的项失败并写明原因，远端不会出现 board-selftest 分支
#[test]
fn selftest_checks_agent_git_and_version() {
    let rig = Rig::new();
    let root = rig.repo.parent().unwrap().to_path_buf();
    let sh = |script: &str| {
        let out = Command::new("sh").arg("-c").arg(script).current_dir(&root).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    sh("git init -q --bare origin.git && cd repo && git init -q && git -c user.email=t@t -c user.name=t commit -q --allow-empty -m init && git remote add origin ../origin.git && git push -q origin HEAD:refs/heads/main");
    rig.mode("review");
    let host = rig.host("mbp");
    let online = |rig: &Rig| rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true));
    wait_until("mbp 上线", Duration::from_secs(20), || online(&rig));
    let ask = |agent: Option<&str>| {
        let mut store = board_core::Store::open(&rig.s.db).unwrap();
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.host_selftest("mbp", agent, "octocat")).unwrap()
    };
    let done = |rig: &Rig| {
        let h = rig.workspace()["hosts"][0].clone();
        (h["selftest"]["ask"].is_null() && h["selftest"]["items"].as_array().is_some_and(|i| !i.is_empty())).then_some(h)
    };
    let item = |h: &Value, kind: &str| h["selftest"]["items"].as_array().unwrap().iter().find(|i| i["kind"] == kind).cloned().unwrap_or(Value::Null);
    ask(None);
    let begun = Instant::now();
    while done(&rig).is_none() && begun.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let Some(h) = done(&rig) else {
        let ws = rig.workspace();
        let (_, out) = stop(host);
        panic!("自检没做完：{}\n{out}", ws["hosts"][0]["selftest"]);
    };
    assert_eq!(item(&h, "agent")["key"], "claude", "{h}");
    assert_eq!(item(&h, "agent")["ok"], true, "{h}");
    assert_eq!(item(&h, "git")["ok"], true, "{h}");
    assert_eq!(item(&h, "version")["ok"], true, "{h}");
    assert!(item(&h, "agent")["at"].as_i64().unwrap() > 0);
    let agents: Vec<&Value> = h["selftest"]["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "agent").collect();
    assert_eq!(agents.len(), 1, "找不到命令的 dsh、pi、codex 不查：{h}");

    // 没登录、origin 指向不存在的地址
    rig.mode("nologin");
    sh("cd repo && git remote set-url origin /nope/missing.git");
    ask(None);
    wait_until("第二次自检做完", Duration::from_secs(30), || done(&rig).is_some_and(|h| item(&h, "agent")["ok"] == false));
    let h = done(&rig).unwrap();
    assert!(item(&h, "agent")["why"].as_str().unwrap().starts_with("没有登录：Not logged in"), "{h}");
    let git = item(&h, "git");
    assert_eq!(git["ok"], false, "{h}");
    assert!(git["why"].as_str().unwrap().contains("远端仓库不存在"), "{git}");
    let refs = Command::new("git").args(["-C", root.join("origin.git").to_str().unwrap(), "for-each-ref"]).output().unwrap();
    assert!(!String::from_utf8_lossy(&refs.stdout).contains("board-selftest"), "dry-run 不会推上去");
    let ops: Vec<String> = h["ops"].as_array().unwrap().iter().map(|o| o["text"].as_str().unwrap().to_string()).collect();
    assert!(ops.iter().any(|t| t.starts_with("自检通过")) && ops.iter().any(|t| t.contains("没通过")), "{ops:?}");
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("请求自检") && out.contains("自检做完"), "{out}");
}

/* ------------------------------ 装成服务（#190）：守护进程 ------------------------------ */

/// `board host --service`（`board host --install` 装的服务里运行的就是它）：board host 被 kill -9 后很快重新上线
/// （测试里等 0.5 秒，默认 10 秒），输出写进 ~/.board/host/host.log；停止服务（SIGTERM）时 board host 收尾、机器离线，守护进程退出码 0
#[test]
fn service_restarts_host_after_kill9() {
    let rig = Rig::new();
    rig.mode("review");
    let sup = rig.service("mbp");
    let online = |rig: &Rig| rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true));
    wait_until("mbp 上线", Duration::from_secs(20), || online(&rig));
    let pidfile = rig.s.home.join(".board").join("host").join("mbp.pid");
    let field = |i: usize| std::fs::read_to_string(&pidfile).ok().and_then(|s| s.split_whitespace().nth(i).map(str::to_string));
    let (pid, before) = (field(0).unwrap(), field(2).unwrap());
    assert_ne!(pid.parse::<u32>().unwrap(), sup.id(), "board host 是守护进程的子进程");
    unsafe {
        libc_kill(pid.parse().unwrap(), 9);
    }
    // 10 秒内（这里 0.5 秒）重新启动，以新进程上线（带上一次的标识接回名字）
    wait_until("被 kill -9 后重新上线", Duration::from_secs(15), || field(2).is_some_and(|i| i != before) && online(&rig));
    let log = rig.s.home.join(".board").join("host").join("host.log");
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("启动 board host（第 1 次") && text.contains("异常退出（被信号 9 结束）") && text.contains("启动 board host（第 2 次"), "{text}");
    assert!(text.contains("mbp 已上线"), "board host 的输出写进 host.log：{text}");
    // 停止服务：board host 收尾（机器离线），守护进程退出码 0
    let (code, out) = stop(sup);
    assert_eq!(code, 0, "{out}");
    wait_until("mbp 离线", Duration::from_secs(10), || !online(&rig));
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("收到停止信号") && text.contains("守护进程退出"), "{text}");
    assert!(!rig.s.home.join(".board").join("host").join("service.pid").exists());
}

/* ------------------------------ skills 共用库（#186） ------------------------------ */

/// A 上 `board skills push`：在线的 B（这里的 board host）几秒内装进它每个找到的 agent 的 skill 目录，上报「已同步」；
/// 改了再推换成新版本；本机原来就有的同名目录不覆盖；`rm` 之后同步装的那份删掉。
/// 随服务端版本的 board-skills 自动装上，bin/<平台>/board 是 board host 自己的
#[test]
fn skills_push_sync_update_and_remove() {
    let rig = Rig::new();
    let skills = rig.s.home.join(".claude").join("skills");
    // B 上本来就有一个同名的 mine（不是同步装的）
    std::fs::create_dir_all(skills.join("mine")).unwrap();
    std::fs::write(skills.join("mine").join("SKILL.md"), "local").unwrap();
    let host = rig.host("mbp");
    let state = |rig: &Rig, skill: &str| -> Option<(String, Option<String>)> {
        let h = rig.workspace()["hosts"][0].clone();
        h["skills"].as_array()?.iter().find(|s| s["skill"] == skill && s["agent"] == "claude").map(|s| (s["state"].as_str().unwrap_or("").to_string(), s["version"].as_str().map(str::to_string)))
    };
    wait_until("board-skills 自动同步", Duration::from_secs(20), || state(&rig, "board-skills").is_some_and(|(s, _)| s == "synced"));
    let bs = skills.join("board-skills");
    assert!(bs.join("SKILL.md").is_file() && bs.join("scripts").join("board").is_file());
    assert!(bs.join(".board-sync").is_file());
    let plat = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", _) => "darwin-x64",
        ("linux", "aarch64") => "linux-arm64",
        _ => "linux-x64",
    };
    let own = std::fs::metadata(env!("CARGO_BIN_EXE_board")).unwrap().len();
    assert_eq!(std::fs::metadata(bs.join("bin").join(plat).join("board")).unwrap().len(), own, "bin 里是 board host 自己的 board");

    // A 上 push 一个 skill
    let src = rig.repo.parent().unwrap().join("notes");
    std::fs::create_dir_all(src.join("ref")).unwrap();
    std::fs::write(src.join("SKILL.md"), "# notes v1").unwrap();
    std::fs::write(src.join("ref").join("a.md"), "a").unwrap();
    let r = rig.board(&["skills", "push", src.to_str().unwrap(), "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let v1 = r.json()["skills"][0]["version"].as_str().unwrap().to_string();
    wait_until("B 装上 notes", Duration::from_secs(10), || state(&rig, "notes").is_some_and(|(s, v)| s == "synced" && v.as_deref() == Some(v1.as_str())));
    assert_eq!(std::fs::read_to_string(skills.join("notes").join("ref").join("a.md")).unwrap(), "a");
    // 同名的本机目录：没覆盖，写明
    let r = rig.board(&["skills", "push", skills.join("mine").to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    wait_until("mine 报「本机有同名」", Duration::from_secs(10), || state(&rig, "mine").is_some_and(|(s, _)| s == "local"));
    assert_eq!(std::fs::read_to_string(skills.join("mine").join("SKILL.md")).unwrap(), "local");
    // 改了再推：更新到新版本
    std::fs::write(src.join("SKILL.md"), "# notes v2").unwrap();
    let r = rig.board(&["skills", "push", src.to_str().unwrap(), "--json"]);
    let v2 = r.json()["skills"][0]["version"].as_str().unwrap().to_string();
    assert_ne!(v1, v2);
    wait_until("B 更新 notes", Duration::from_secs(10), || state(&rig, "notes").is_some_and(|(_, v)| v.as_deref() == Some(v2.as_str())));
    assert_eq!(std::fs::read_to_string(skills.join("notes").join("SKILL.md")).unwrap(), "# notes v2");
    // ls 能看到库和本机状态
    let r = rig.board(&["skills", "ls"]);
    assert!(r.out.contains("board-skills") && r.out.contains("notes") && r.out.contains("已同步"), "{}", r.out);
    // board-skills 不能删；rm notes：B 上同步装的删掉，本机的 mine 留着
    assert_ne!(rig.board(&["skills", "rm", "board-skills"]).code, 0);
    let r = rig.board(&["skills", "rm", "notes"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    wait_until("B 删掉 notes", Duration::from_secs(10), || !skills.join("notes").exists());
    assert!(skills.join("mine").join("SKILL.md").is_file());
    // board host 在跑时 sync 交给它
    let r = rig.board(&["skills", "sync"]);
    assert!(r.out.contains("由它来做"), "{}", r.out);
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("skills：装上") && out.contains("notes"), "{out}");
}

/// 超过 5 MB 的 skill 被拒绝，并说明原因（命令行先拦，服务端也拦）
#[test]
fn skills_push_rejects_big_skill() {
    let rig = Rig::new();
    let big = rig.repo.parent().unwrap().join("big");
    std::fs::create_dir_all(&big).unwrap();
    std::fs::write(big.join("SKILL.md"), "x").unwrap();
    std::fs::write(big.join("blob.bin"), vec![0u8; 5 * 1024 * 1024 + 1]).unwrap();
    let r = rig.board(&["skills", "push", big.to_str().unwrap()]);
    assert_ne!(r.code, 0);
    assert!(r.err.contains("超过 5 MB"), "{}", r.err);
    // 绕过命令行直接调接口：服务端同样拒绝
    let data = board_core::skills::b64(&vec![0u8; 5 * 1024 * 1024 + 1]);
    let res = ureq::post(format!("{}/api/skills", rig.s.url))
        .header("authorization", format!("Bearer {}", rig.s.token))
        .config()
        .http_status_as_error(false)
        .build()
        .send_json(serde_json::json!({ "name": "big", "files": [{ "path": "SKILL.md", "data": "eA==" }, { "path": "blob.bin", "data": data }] }))
        .unwrap();
    let mut res = res;
    let body = res.body_mut().read_to_string().unwrap();
    assert!(body.contains("超过 5 MB"), "{body}");
}

/* ------------------------------ MCP 共用配置（#187） ------------------------------ */

/// 在服务端加一个 stdio MCP 服务器（带一个密钥）：B 上 Claude Code（经 claude mcp add-json）、Codex（config.toml）、pi（mcp.json）
/// 的用户级配置里都出现；列表接口、board host 的输出里看不到密钥原文；B 上已有同名的不覆盖；删掉后同步加的删掉、用户自己的留着。
/// 三个 agent 都是假脚本，不执行本机真实的 claude、codex、pi
#[test]
fn mcp_servers_sync_to_claude_codex_pi() {
    let rig = Rig::new();
    let home = rig.s.home.clone();
    // codex、pi 也换成假脚本（找得到命令才同步）；dsh 仍是不存在的命令
    let conf = home.join(".board").join("config.toml");
    let fake = rig.fake.display().to_string();
    let text = std::fs::read_to_string(&conf).unwrap();
    let text = text.replace("[agents.pi]\nname = \"pi\"\ncmd = [\"board-test-no-such-command\"", &format!("[agents.pi]\nname = \"pi\"\ncmd = [\"{fake}\""));
    let text = text.replace("[agents.codex]\ncmd = [\"board-test-no-such-command\"", &format!("[agents.codex]\ncmd = [\"{fake}\""));
    std::fs::write(&conf, text).unwrap();
    // B 上 pi 本来就有一个同名的 shared（用户自己加的），Codex 有一个用户自己的 mine
    std::fs::create_dir_all(home.join(".pi").join("agent")).unwrap();
    std::fs::write(home.join(".pi/agent/mcp.json"), r#"{"mcpServers":{"shared":{"command":"user-own"}}}"#).unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(home.join(".codex/config.toml"), "[mcp_servers.mine]\ncommand = \"mine\"\n").unwrap();
    let host = rig.host("mbp");
    let online = |rig: &Rig| rig.workspace()["hosts"].as_array().is_some_and(|h| h.iter().any(|x| x["name"] == "mbp" && x["online"] == true));
    wait_until("mbp 上线", Duration::from_secs(20), || online(&rig));
    let secret = "sk-live-0123456789abcdef";
    let put = |name: &str| {
        let mut store = board_core::Store::open(&rig.s.db).unwrap();
        let srv: board_core::mcp::McpServer = serde_json::from_value(serde_json::json!({
            "name": name, "kind": "stdio", "command": "npx", "args": ["-y", "db-mcp"],
            "env": [{ "key": "DB_URL", "value": "postgres://db" }, { "key": "DB_TOKEN", "value": secret, "secret": true }],
        }))
        .unwrap();
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.mcp_put(srv, "octocat", "界面")).unwrap();
    };
    put("db");
    put("shared");
    let state = |rig: &Rig, server: &str, agent: &str| -> String {
        let h = rig.workspace()["hosts"][0].clone();
        h["mcp"].as_array().and_then(|a| a.iter().find(|s| s["server"] == server && s["agent"] == agent)).map(|s| s["state"].as_str().unwrap_or("").to_string()).unwrap_or_default()
    };
    wait_until("db 同步到三个 agent", Duration::from_secs(15), || ["claude", "codex", "pi"].iter().all(|a| state(&rig, "db", a) == "synced"));
    let claude: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(claude["mcpServers"]["db"]["command"], "npx");
    assert_eq!(claude["mcpServers"]["db"]["env"]["DB_TOKEN"], secret);
    let codex = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(codex.contains("[mcp_servers.db]") && codex.contains(secret) && codex.contains("[mcp_servers.mine]"), "{codex}");
    let pi: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".pi/agent/mcp.json")).unwrap()).unwrap();
    assert_eq!(pi["mcpServers"]["db"]["args"], serde_json::json!(["-y", "db-mcp"]));
    // pi 上同名的 shared 是用户自己的：没覆盖，写明；其他 agent 照常加上
    assert_eq!(pi["mcpServers"]["shared"]["command"], "user-own");
    assert_eq!(state(&rig, "shared", "pi"), "local");
    assert_eq!(state(&rig, "shared", "claude"), "synced");
    // dsh 找不到命令，不报；列表接口（令牌也能读）只给末尾 4 位
    let mut res = ureq::get(format!("{}/api/mcp", rig.s.url)).header("authorization", format!("Bearer {}", rig.s.token)).call().unwrap();
    let list = res.body_mut().read_to_string().unwrap();
    assert!(!list.contains(secret) && list.contains("cdef"), "{list}");
    // 密钥原文只给 board host 的令牌接口，而且只给这台机器上报时用的令牌
    let (code, body) = get_raw(&format!("{}/api/hosts/mbp/mcp", rig.s.url), &rig.s.token);
    assert_eq!(code, 200, "{body}");
    assert!(body.contains(secret));
    // 另一个令牌（没绑到这台机器）、机器名不对：都拿不到原文
    let other = {
        let store = board_core::Store::open(&rig.s.db).unwrap();
        board_server::auth::create_token(store.conn(), "agent 用的", board_core::now_ms()).unwrap().1
    };
    let (code, body) = get_raw(&format!("{}/api/hosts/mbp/mcp", rig.s.url), &other);
    assert_eq!(code, 401, "别的令牌不能取原文：{body}");
    assert!(body.contains("NOT_LOGGED_IN") && !body.contains(secret), "{body}");
    let (code, body) = get_raw(&format!("{}/api/hosts/nope/mcp", rig.s.url), &rig.s.token);
    assert_eq!(code, 401, "机器不存在：{body}");
    assert!(!body.contains(secret), "{body}");
    // 删掉 db：同步加的删掉，用户自己的 mine、shared（pi）留着
    {
        let mut store = board_core::Store::open(&rig.s.db).unwrap();
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.mcp_rm("db")).unwrap();
    }
    wait_until("db 从三个 agent 删掉", Duration::from_secs(15), || {
        let pi = std::fs::read_to_string(home.join(".pi/agent/mcp.json")).unwrap_or_default();
        let codex = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap_or_default();
        let claude = std::fs::read_to_string(home.join(".claude.json")).unwrap_or_default();
        !pi.contains("\"db\"") && !codex.contains("mcp_servers.db") && !claude.contains("\"db\"")
    });
    assert!(std::fs::read_to_string(home.join(".codex/config.toml")).unwrap().contains("[mcp_servers.mine]"));
    assert!(std::fs::read_to_string(home.join(".pi/agent/mcp.json")).unwrap().contains("user-own"));
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("MCP：加上 claude 的 MCP 服务器 db"), "{out}");
    assert!(!out.contains(secret), "board host 的输出里没有密钥原文：{out}");
    // 本机的 board host 日志、派发日志目录里也没有
    for e in std::fs::read_dir(home.join(".board").join("host").join("logs")).unwrap().flatten() {
        assert!(!std::fs::read_to_string(e.path()).unwrap_or_default().contains(secret), "{}", e.path().display());
    }
}


/* ------------------------------ 结构化事件（会话「过程」，#184） ------------------------------ */

/// #184 验收：内置模板用结构化输出时，正在跑的派发 5 秒内在会话里看得到第一条消息和工具调用，不用等它结束；
/// 有新事件时推送带上派发 ID；工具结果后到时合进同一条；结束后有用量；异常退出的评论里写最后几个事件、原始输出折叠在后面；
/// 「原始输出」照旧是转写后的一行一行
#[test]
fn structured_events_reach_the_session_while_running() {
    let rig = Rig::new();
    // Claude Code 带上事件流参数（假 agent 只看第一个参数）：board host 按 Claude Code 的格式解析
    let conf = rig.s.home.join(".board").join("config.toml");
    let text = std::fs::read_to_string(&conf).unwrap();
    let fake = rig.fake.display().to_string();
    let text = text.replace(
        &format!("cmd = [\"{fake}\", \"{{prompt}}\"]"),
        &format!("cmd = [\"{fake}\", \"{{prompt}}\", \"--output-format\", \"stream-json\", \"--verbose\"]"),
    );
    assert!(text.contains("stream-json"), "{text}");
    std::fs::write(&conf, text).unwrap();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let host = rig.host("mbp");
    rig.online("mbp");
    rig.mode("stream");

    // 像界面一样订阅推送
    let sid = rig.ui_session();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let url = format!("{}/api/stream", rig.s.url);
    std::thread::spawn(move || {
        use std::io::BufRead;
        let Ok(res) = ureq::get(&url).header("cookie", format!("board_session={sid}")).call() else { return };
        for line in std::io::BufReader::new(res.into_body().into_reader()).lines() {
            let Ok(line) = line else { return };
            if tx.send(line).is_err() {
                return;
            }
        }
    });

    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let d = r.json()["id"].as_i64().unwrap();
    let streamed = rig.fake.parent().unwrap().join("streamed");
    wait_until("agent 写出前两个事件", Duration::from_secs(15), || streamed.exists());
    let begun = Instant::now();
    let session = |q: &str| rig.ui_get(&format!("/api/sessions/{d}{q}"));
    // 假 agent 先打的几行（提示词、环境变量）不是 JSON：记成 text，原样保留
    let structured = |v: &Value| -> Vec<Value> {
        v["parts"][0]["events"].as_array().map(|e| e.iter().filter(|x| x["kind"] != "text").cloned().collect()).unwrap_or_default()
    };
    wait_until("会话里有了消息和工具调用", Duration::from_secs(5), || structured(&session("")).len() >= 2);
    assert!(begun.elapsed() < Duration::from_secs(5));
    let v = session("");
    let part = &v["parts"][0];
    assert_eq!(part["fmt"], "claude");
    // 登录 shell 自己打的提示（作业控制、sudo 提示）不是 agent 的输出：不进会话的过程（#232）
    let texts: Vec<&str> = part["events"].as_array().unwrap().iter().filter_map(|e| e["text"].as_str()).collect();
    assert!(texts.iter().all(|t| !t.contains("no job control") && !t.contains("sudo")), "{part}");
    assert!(part["events"][0]["kind"] == "text" && part["events"][0]["text"].as_str().unwrap().starts_with("prompt: "), "{part}");
    let evs = structured(&v);
    assert_eq!((evs[0]["kind"].as_str(), evs[0]["text"].as_str()), (Some("message"), Some("先领卡，再跑测试。")));
    assert_eq!((evs[1]["kind"].as_str(), evs[1]["tool"].as_str(), evs[1]["args"].as_str()), (Some("tool"), Some("Bash"), Some("cargo test")));
    assert!(evs[1]["ok"].is_null(), "还没有结果：{evs:?}");
    assert!(evs[1].get("call").is_none(), "调用标识不给界面");
    let runs = v["runs"].as_i64().unwrap();
    // 推送：带上有新事件的派发
    let pushed = format!("\"ran\":[{d}]");
    let mut seen = false;
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(200)) {
            if line.contains(&pushed) {
                seen = true;
                break;
            }
        }
    }
    assert!(seen, "推送里有 {pushed}");

    // 出结果、异常退出
    std::fs::write(rig.fake.parent().unwrap().join("go"), "").unwrap();
    wait_until("#1 异常退出", Duration::from_secs(15), || ended(&rig.show(1), "异常退出"));
    let inc = session(&format!("?since={runs}"));
    let evs = inc["parts"][0]["events"].as_array().unwrap().clone();
    assert_eq!(evs.iter().map(|e| e["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["tool", "result"], "增量：改过的工具调用和新的结果：{evs:?}");
    assert_eq!((evs[0]["ok"].as_bool(), evs[0]["out"].as_str()), (Some(false), Some("test failed")));
    assert_eq!((evs[1]["ok"].as_bool(), evs[1]["cost"].as_f64(), evs[1]["tokensIn"].as_i64(), evs[1]["tokensOut"].as_i64()), (Some(false), Some(0.5), Some(100), Some(10)));
    assert_eq!(structured(&session("")).len(), 3, "工具结果合进同一条");
    // 评论：最后几个事件，原始输出折叠
    let c = activity(&rig.show(1));
    let comment = c.iter().find(|t| t.contains("agent 异常退出（退出码 4）")).unwrap_or_else(|| panic!("{c:?}"));
    assert!(comment.contains("个事件：") && comment.contains("▸ Bash  cargo test  ✗ test failed"), "{comment}");
    assert!(comment.contains("agent：先领卡，再跑测试。") && comment.contains("■ 结束 · 出错：测试没过"), "{comment}");
    assert!(comment.contains("<details><summary>原始输出最后"), "{comment}");
    // 原始输出照旧是转写后的一行一行
    let log = rig.dispatch_log(d);
    assert!(log.iter().any(|l| l == "▸ Bash  cargo test"), "{log:?}");
    assert!(log.iter().any(|l| l == "  ✗ test failed"), "{log:?}");
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
}

/// 只有原始输出的 agent（配置文件里换掉了命令）：格式报成空字符串，没有事件，评论照旧写「日志最后 N 行」
#[test]
fn raw_only_agents_report_an_empty_format() {
    let rig = Rig::new();
    assert_eq!(rig.board(&["new", "一"]).code, 0);
    let host = rig.host("mbp");
    rig.online("mbp");
    rig.mode("crash");
    let r = rig.board(&["dispatch", "1", "--host", "mbp", "--json"]);
    let d = r.json()["id"].as_i64().unwrap();
    wait_until("#1 异常退出", Duration::from_secs(15), || ended(&rig.show(1), "异常退出"));
    wait_until("格式报上来", Duration::from_secs(5), || rig.ui_get(&format!("/api/sessions/{d}"))["parts"][0]["fmt"] == "");
    let v = rig.ui_get(&format!("/api/sessions/{d}"));
    assert!(v["parts"][0]["events"].as_array().unwrap().is_empty());
    let c = activity(&rig.show(1));
    assert!(c.iter().any(|t| t.contains("日志最后") && t.contains("boom") && !t.contains("<details>")), "{c:?}");
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
}


/* ------------------------------ 共用配置的 agent 模板（#188） ------------------------------ */

/// #188 验收：在共用配置里新加一个 agent 模板（命令是假脚本），在线的机器 10 秒内上报它，不用重启 board host；
/// 本机配置文件里有同标识的 `[agents.*]` 时用本机的，上报里标「本机配置覆盖」；删掉模板后它不再出现
#[test]
fn shared_agent_templates_reach_online_hosts_without_restart() {
    let rig = Rig::new();
    let host = rig.host("mbp");
    rig.online("mbp");
    let fake = rig.fake.display().to_string();
    let put = |id: &str, cmd: Vec<String>| {
        let mut store = board_core::Store::open(&rig.s.db).unwrap();
        let t = board_core::AgentTemplate { id: id.into(), name: format!("{id} 模板"), cmd, ..Default::default() };
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.template_put(t, "buglan")).unwrap();
    };
    let agent = |id: &str| -> Option<Value> { rig.workspace()["hosts"][0]["agents"].as_array()?.iter().find(|a| a["id"] == id).cloned() };
    // 新加：命令是假脚本，找得到
    put("kimi", vec![fake.clone(), "{prompt}".into()]);
    let begun = Instant::now();
    wait_until("mbp 上报 kimi", Duration::from_secs(10), || agent("kimi").is_some_and(|a| a["found"] == true));
    assert!(begun.elapsed() < Duration::from_secs(10));
    let a = agent("kimi").unwrap();
    assert_eq!((a["name"].as_str(), a["version"].as_str()), (Some("kimi 模板"), Some("9.9.9 (fake)")));
    assert!(a.get("local_override").is_none());
    // 共用模板覆盖 claude，本机配置文件里也有 [agents.claude]：用本机的，标出来
    put("claude", vec!["/board-test-no-such-command".into(), "{prompt}".into()]);
    wait_until("claude 标出本机配置覆盖", Duration::from_secs(10), || agent("claude").is_some_and(|a| a["local_override"] == true));
    assert_eq!(agent("claude").unwrap()["found"], true, "用的是本机配置文件里的假脚本");
    // 删掉 kimi：不再出现
    {
        let mut store = board_core::Store::open(&rig.s.db).unwrap();
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.template_rm("kimi")).unwrap();
    }
    wait_until("kimi 不再上报", Duration::from_secs(10), || agent("kimi").is_none());
    let (code, out) = stop(host);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("共用配置的 agent 模板：kimi"), "{out}");
}
