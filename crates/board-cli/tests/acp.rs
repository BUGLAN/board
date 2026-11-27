//! `board acp`（board host 和 agent 之间讲 ACP 的那一层）：用一个假的 ACP agent（sh 脚本，按顺序读请求、回固定的帧）
//! 走一遍完整的一轮、被取消、agent 提前退出；续轮（#454）连一个真的服务端，看名字还持有没持有卡片。不执行真实的 agent。
#![cfg(unix)]

mod common;
use common::{isolate, start, Server};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

const UPDATE: &str = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":"#;

fn write(dir: &Path, name: &str, text: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p.display().to_string()
}

/// 启动 `board acp`，agent 是 `sh <script>`
fn bridge(dir: &Path, script: &str) -> std::process::Child {
    let fake = write(dir, "fake.sh", script);
    let prompt = write(dir, "prompt.txt", "/board-skills 开始执行 default项目之中的main白板中的#1任务");
    let d = dir.display().to_string();
    Command::new(env!("CARGO_BIN_EXE_board"))
        .args(["acp", "--env", "direct", "--cwd", &d, "--prompt-file", &prompt, "--mode", "bypassPermissions", "--", "sh", &fake])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn finish(mut c: std::process::Child, limit: Duration) -> (i32, String, String) {
    let until = Instant::now() + limit;
    let status = loop {
        if let Some(s) = c.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < until, "board acp 没在 {limit:?} 内退出");
        std::thread::sleep(Duration::from_millis(50));
    };
    let (mut out, mut err) = (String::new(), String::new());
    c.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    c.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    (status.code().unwrap_or(-1), out, err)
}

struct ChatBridge {
    child: std::process::Child,
    stdin: Option<ChildStdin>,
    state: PathBuf,
}

impl ChatBridge {
    fn send(&mut self, line: &str) {
        let w = self.stdin.as_mut().expect("标准输入还开着");
        writeln!(w, "{line}").unwrap();
        w.flush().unwrap();
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }

    fn wait_exit(&mut self, limit: Duration) {
        let until = Instant::now() + limit;
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < until, "board acp 没在 {limit:?} 内退出");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn finish(mut self, limit: Duration) -> (i32, String, String) {
        self.stdin.take();
        finish(self.child, limit)
    }
}

fn chat_bridge(dir: &Path, script: &str, more: &[&str]) -> ChatBridge {
    let fake = write(dir, "fake-chat.sh", script);
    let state = dir.join("chat.state");
    let cwd = dir.display().to_string();
    let state_arg = state.display().to_string();
    let mut c = Command::new(env!("CARGO_BIN_EXE_board"));
    c.args(["acp", "--env", "direct", "--cwd", &cwd, "--chat", &state_arg]).args(more).args(["--", "sh", &fake]);
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    let stdin = child.stdin.take();
    ChatBridge { child, stdin, state }
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(15);
    while !ok() {
        assert!(Instant::now() < until, "15 秒内没有等到：{what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn chat_state(path: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn wait_state(path: &Path, what: &str, mut ok: impl FnMut(&serde_json::Value) -> bool) -> serde_json::Value {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(v) = chat_state(path).filter(|v| ok(v)) {
            return v;
        }
        assert!(Instant::now() < until, "15 秒内没有等到状态：{what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn a_whole_turn_is_logged_frame_by_frame() {
    let d = tempfile::tempdir().unwrap();
    // 会话配置里有 mode：board acp 要先切到 bypassPermissions（id 2），再发 prompt（id 3）
    let script = format!(
        r#"read l; echo '{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":1,"agentInfo":{{"name":"fake-agent","version":"1.2.3"}}}}}}'
read l; echo "$l" | grep -q '"mcpServers":\[\]' || echo "no mcpServers" >&2
echo '{{"jsonrpc":"2.0","id":1,"result":{{"sessionId":"s1","configOptions":[{{"id":"mode","category":"mode","options":[{{"value":"default"}},{{"value":"bypassPermissions"}}]}}]}}}}'
read l; echo "mode: $l" >&2; echo '{{"jsonrpc":"2.0","id":2,"result":{{}}}}'
read l; echo "prompt: $l" >&2
echo '{UPDATE}{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"先领卡，"}}}}}}}}'
echo '{UPDATE}{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"再看代码。"}}}}}}}}'
echo '{{"jsonrpc":"2.0","id":"perm-1","method":"session/request_permission","params":{{"sessionId":"s1","toolCall":{{"toolCallId":"t1"}},"options":[{{"optionId":"once","kind":"allow_once"}},{{"optionId":"always","kind":"allow_always"}}]}}}}'
read l; echo "permission: $l" >&2
echo '{UPDATE}{{"sessionUpdate":"tool_call","toolCallId":"t1","title":"git status","kind":"execute","status":"pending","rawInput":{{"command":"git status"}}}}}}}}'
echo '{UPDATE}{{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","rawOutput":{{"output":"On branch main","exit_code":0}}}}}}}}'
echo '{{"jsonrpc":"2.0","id":3,"result":{{"stopReason":"end_turn"}}}}'
read l; exit 0
"#
    );
    let (code, out, err) = finish(bridge(d.path(), &script), Duration::from_secs(20));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
    assert!(!err.contains("no mcpServers"), "{err}");
    assert!(err.lines().any(|l| l.starts_with("mode: ") && l.contains("session/set_config_option") && l.contains("bypassPermissions")), "{err}");
    assert!(err.contains("prompt:") && err.contains("开始执行"), "提示词从文件读、作为一条文本发出去：{err}");
    assert!(err.contains(r#""optionId":"always""#) && err.contains(r#""id":"perm-1""#), "权限自动选「总是允许」：{err}");
    let lines: Vec<&str> = out.lines().collect();
    // 日志一行一帧：发出去的请求也在里面；两段消息片段合成一行
    assert!(lines.iter().any(|l| l.contains(r#""method":"initialize""#)), "{out}");
    assert!(lines.iter().any(|l| l.contains("fake-agent")), "{out}");
    let merged: Vec<&&str> = lines.iter().filter(|l| l.contains("agent_message_chunk")).collect();
    assert_eq!(merged.len(), 1, "{out}");
    assert!(merged[0].contains("先领卡，再看代码。"), "{out}");
    assert!(lines.iter().any(|l| l.contains(r#""stopReason":"end_turn""#)), "{out}");
    for l in &lines {
        serde_json::from_str::<serde_json::Value>(l).unwrap_or_else(|e| panic!("日志里每一行都是 JSON：{l}（{e}）"));
    }
}

#[test]
fn sigterm_cancels_the_turn() {
    let d = tempfile::tempdir().unwrap();
    // prompt 之后一直不结束，收到 session/cancel 才回 cancelled；收到 prompt 时在工作目录里留个记号（prompted）
    let script = r#"read l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
read l; echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}}}'; touch prompted
read l; echo "got: $l" >&2; echo '{"jsonrpc":"2.0","id":2,"result":{"stopReason":"cancelled"}}'
read l; exit 0
"#;
    let c = bridge(d.path(), script);
    // 等它发了 prompt 再停（之前没有可取消的）。不用固定的等待：机器忙时（同一批测试在起服务端）board acp 起得慢，
    // 信号处理还没装好就收到 SIGTERM 会直接被结束
    let mark = d.path().join("prompted");
    let until = Instant::now() + Duration::from_secs(15);
    while !mark.exists() {
        assert!(Instant::now() < until, "15 秒内 agent 没收到 prompt");
        std::thread::sleep(Duration::from_millis(50));
    }
    unsafe {
        libc::kill(c.id() as i32, libc::SIGTERM);
    }
    let (code, out, err) = finish(c, Duration::from_secs(15));
    assert_eq!(code, 130, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("session/cancel"), "{err}");
    assert!(out.contains(r#""stopReason":"cancelled""#), "{out}");
}

/// 派发时 board host 只拿着标准输入的写端：host 退出、管道关闭后，board acp 也要像收到 SIGTERM 一样取消
#[test]
fn closed_parent_stdin_cancels_the_turn() {
    let d = tempfile::tempdir().unwrap();
    let script = r#"read l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
read l; echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}}}'; touch prompted
read l; echo "got: $l" >&2; echo '{"jsonrpc":"2.0","id":2,"result":{"stopReason":"cancelled"}}'
read l; exit 0
"#;
    let mut c = bridge(d.path(), script);
    let mark = d.path().join("prompted");
    let until = Instant::now() + Duration::from_secs(15);
    while !mark.exists() {
        assert!(Instant::now() < until, "15 秒内 agent 没收到 prompt");
        std::thread::sleep(Duration::from_millis(25));
    }
    drop(c.stdin.take());
    let (code, out, err) = finish(c, Duration::from_secs(15));
    assert_eq!(code, 130, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("got: ") && err.contains("session/cancel"), "{err}");
    assert!(out.contains(r#""stopReason":"cancelled""#), "{out}");
}

#[test]
fn an_agent_that_dies_early_is_reported() {
    let d = tempfile::tempdir().unwrap();
    // 握手之后就退出（例如没登录、崩了）：退出码 0 也记成 1
    let script = r#"read l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
read l; echo "Error: not logged in" >&2; exit 0
"#;
    let (code, _out, err) = finish(bridge(d.path(), script), Duration::from_secs(15));
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("not logged in") && err.contains("这一轮结束之前就退出了"), "{err}");

    // 握手就出错：退出码 2，原因写进标准错误（也就是日志）
    let script = r#"read l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read l; echo '{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required"}}'
read l; exit 0
"#;
    let (code, _out, err) = finish(bridge(d.path(), script), Duration::from_secs(15));
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("agent 要求先登录：Authentication required"), "{err}");

    // 命令不存在：退出码 2
    let prompt = write(d.path(), "p.txt", "x");
    let o = Command::new(env!("CARGO_BIN_EXE_board"))
        .args(["acp", "--env", "direct", "--cwd", &d.path().display().to_string(), "--prompt-file", &prompt, "--", "/board-test-no-such-acp-agent"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("启动"), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn provider_file_is_deleted_even_when_the_agent_cannot_start() {
    let d = tempfile::tempdir().unwrap();
    let prompt = write(d.path(), "p.txt", "x");
    let provider = write(d.path(), "provider.json", r#"{"apiType":"anthropic","headers":{"Authorization":"Bearer sk-secret"}}"#);
    let cwd = d.path().display().to_string();
    let o = Command::new(env!("CARGO_BIN_EXE_board"))
        .args(["acp", "--env", "direct", "--cwd", &cwd, "--prompt-file", &prompt, "--provider-file", &provider, "--", "/board-test-no-such-acp-agent"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(!Path::new(&provider).exists(), "agent 起不来也要先删掉含 key 的文件");

    let state = write(d.path(), "chat.state", "");
    let provider = write(d.path(), "chat-provider.json", r#"{"apiType":"anthropic"}"#);
    let o = Command::new(env!("CARGO_BIN_EXE_board"))
        .args(["acp", "--env", "direct", "--cwd", &cwd, "--chat", &state, "--provider-file", &provider, "--", "/board-test-no-such-acp-agent"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(!Path::new(&provider).exists(), "对话起不来也要先删掉含 key 的文件");
}

/// 对话：同一个 ACP 会话里连发两轮，状态文件的 busy / seq / stop 跟着每一轮变化；`session/new` 只开一次
#[test]
fn chat_reuses_one_session_for_two_rounds() {
    let d = tempfile::tempdir().unwrap();
    let script = r#"read -r l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read -r l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'; echo "session/new" >> sessions
n=0
while read -r l; do
  case "$l" in
    *'"session/prompt"'*)
      n=$((n+1))
      pid=$(printf '%s\n' "$l" | sed 's/.*"id":\([0-9]*\).*/\1/')
      : > "got$n"
      i=0; while [ ! -f "go$n" ] && [ $i -lt 300 ]; do sleep 0.05; i=$((i+1)); done
      echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"s1\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"round $n\"}}}}"
      echo "{\"jsonrpc\":\"2.0\",\"id\":$pid,\"result\":{\"stopReason\":\"end_turn\"}}"
      ;;
  esac
done
"#;
    let mut b = chat_bridge(d.path(), script, &[]);
    wait_state(&b.state, "会话开好", |v| v["session"] == "s1" && v["busy"] == false && v["seq"] == 0);

    b.send(r#"{"seq":1,"prompt":"第一轮"}"#);
    wait_for("agent 收到第一轮", || d.path().join("got1").exists());
    wait_state(&b.state, "第一轮运行中", |v| v["session"] == "s1" && v["busy"] == true && v["seq"] == 1 && v["stop"].is_null());
    std::fs::write(d.path().join("go1"), "").unwrap();
    wait_state(&b.state, "第一轮结束", |v| v["busy"] == false && v["seq"] == 1 && v["stop"] == "end_turn");

    b.send(r#"{"seq":2,"prompt":"第二轮"}"#);
    wait_for("agent 收到第二轮", || d.path().join("got2").exists());
    wait_state(&b.state, "第二轮运行中", |v| v["session"] == "s1" && v["busy"] == true && v["seq"] == 2 && v["stop"].is_null());
    std::fs::write(d.path().join("go2"), "").unwrap();
    wait_state(&b.state, "第二轮结束", |v| v["busy"] == false && v["seq"] == 2 && v["stop"] == "end_turn");

    assert_eq!(std::fs::read_to_string(d.path().join("sessions")).unwrap().lines().count(), 1);
    let (code, out, err) = b.finish(Duration::from_secs(15));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
}

/// 对话：收到 `{"cancel":true}` 时给 agent 发 `session/cancel`，状态文件的 stop 变成 cancelled
#[test]
fn chat_cancel_ends_the_round_as_cancelled() {
    let d = tempfile::tempdir().unwrap();
    let script = r#"read -r l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read -r l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
read -r l; printf '%s\n' "$l" > prompt
pid=$(printf '%s\n' "$l" | sed 's/.*"id":\([0-9]*\).*/\1/')
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}}}'
read -r l; printf '%s\n' "$l" > cancel
echo "{\"jsonrpc\":\"2.0\",\"id\":$pid,\"result\":{\"stopReason\":\"cancelled\"}}"
while read -r l; do :; done
"#;
    let mut b = chat_bridge(d.path(), script, &[]);
    wait_state(&b.state, "会话开好", |v| v["session"] == "s1");
    b.send(r#"{"seq":1,"prompt":"跑一条长命令"}"#);
    wait_state(&b.state, "这一轮运行中", |v| v["busy"] == true && v["seq"] == 1);
    b.send(r#"{"cancel":true}"#);
    wait_state(&b.state, "取消结束", |v| v["busy"] == false && v["seq"] == 1 && v["stop"] == "cancelled");
    let cancel = std::fs::read_to_string(d.path().join("cancel")).unwrap();
    assert!(cancel.contains("session/cancel") && cancel.contains(r#""sessionId":"s1""#), "{cancel}");
    let (code, out, err) = b.finish(Duration::from_secs(15));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
}

/// 对话：没有新消息超过 `--idle-exit` 秒就自己退出（标准输入还开着）
#[test]
fn chat_exits_after_the_idle_timeout() {
    let d = tempfile::tempdir().unwrap();
    let script = r#"read -r l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read -r l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
while read -r l; do :; done
"#;
    let mut b = chat_bridge(d.path(), script, &["--idle-exit", "1"]);
    wait_state(&b.state, "会话开好", |v| v["session"] == "s1");
    b.wait_exit(Duration::from_secs(10));
    let (code, out, err) = b.finish(Duration::from_secs(5));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
}

/// 对话：board host 关掉标准输入时退出
#[test]
fn chat_exits_when_stdin_closes() {
    let d = tempfile::tempdir().unwrap();
    let script = r#"read -r l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read -r l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
while read -r l; do :; done
"#;
    let mut b = chat_bridge(d.path(), script, &[]);
    wait_state(&b.state, "会话开好", |v| v["session"] == "s1");
    b.close_stdin();
    b.wait_exit(Duration::from_secs(10));
    let (code, out, err) = b.finish(Duration::from_secs(5));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
}

/// 对话：agent 既不支持 session/resume 也不支持 session/load 时开新会话，并在标准错误里说明之前的对话没了
#[test]
fn chat_starts_a_new_session_when_resume_is_not_supported() {
    let d = tempfile::tempdir().unwrap();
    let script = r#"read -r l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
read -r l; printf '%s\n' "$l" > method; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s2"}}'
while read -r l; do
  case "$l" in
    *'"session/prompt"'*)
      pid=$(printf '%s\n' "$l" | sed 's/.*"id":\([0-9]*\).*/\1/')
      echo "{\"jsonrpc\":\"2.0\",\"id\":$pid,\"result\":{\"stopReason\":\"end_turn\"}}"
      ;;
  esac
done
"#;
    let mut b = chat_bridge(d.path(), script, &["--resume", "s1"]);
    wait_state(&b.state, "新会话开好", |v| v["session"] == "s2");
    let method = std::fs::read_to_string(d.path().join("method")).unwrap();
    assert!(method.contains("session/new") && !method.contains("session/resume") && !method.contains("session/load"), "{method}");
    b.send(r#"{"seq":1,"prompt":"还记得刚才吗"}"#);
    wait_state(&b.state, "新会话第一轮结束", |v| v["busy"] == false && v["seq"] == 1 && v["stop"] == "end_turn");
    let (code, out, err) = b.finish(Duration::from_secs(15));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("agent 不支持接回会话") && err.contains("开了新会话") && err.contains("之前"), "{err}");
}

/// 续轮用的假 agent：答完握手，之后每条 prompt 都以 end_turn 结束这一轮，收到的 prompt 打到标准错误（`prompt <id>: <帧>`）。
/// 请求 id 是环境变量 HAND_OVER 的那一轮里先像 board-skills 那样交审 #1（final、移到待审核），再结束这一轮
const TURNS: &str = r#"read l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
n=2
while read -r l; do
  echo "prompt $n: $l" >&2
  if [ "$n" = "$HAND_OVER" ]; then "$BOARD_BIN" beat 1 --final >/dev/null && "$BOARD_BIN" move 1 review >/dev/null && echo "交审了" >&2; fi
  echo "{\"jsonrpc\":\"2.0\",\"id\":$n,\"result\":{\"stopReason\":\"end_turn\"}}"
  n=$((n+1))
done
"#;

/// 服务端上名字 who 持有 default/main#1（check、新建、领取）
fn holder(who: &str) -> Server {
    let s = start();
    assert_eq!(s.run(who, &["new", "做卡"]).code, 0);
    let r = s.run(who, &["claim", "1"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    s
}

/// 像 board host 那样启动 `board acp`：连接、名字、白板在环境变量里（续轮靠它们查名字还持有没持有卡片），
/// HOME 和临时目录和 check 时一样（假 agent 交审时用得上 check 的会话）。agent 是只会 v1 的 TURNS
fn turns(s: &Server, dir: &Path, who: &str, extra: &[&str], env: &[(&str, &str)]) -> std::process::Child {
    turns_with(s, dir, who, TURNS, extra, env)
}

/// 同 turns，agent 换成 script
fn turns_with(s: &Server, dir: &Path, who: &str, script: &str, extra: &[&str], env: &[(&str, &str)]) -> std::process::Child {
    let fake = write(dir, "turns.sh", script);
    let prompt = write(dir, "prompt.txt", "/board-skills 开始执行 default项目之中的main白板中的#1任务");
    let d = dir.display().to_string();
    let mut c = Command::new(env!("CARGO_BIN_EXE_board"));
    isolate(&mut c, &s.home);
    c.args(["acp", "--env", "direct", "--cwd", &d, "--prompt-file", &prompt]).args(extra).args(["--", "sh", &fake]);
    c.env("BOARD_URL", &s.url).env("BOARD_TOKEN", &s.token).env("BOARD_AS", who).env("BOARD", "default/main").env("BOARD_BIN", env!("CARGO_BIN_EXE_board"));
    for (k, v) in env {
        c.env(k, v);
    }
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()
}

/// 卡片 #454：这一轮结束时名字还持有卡片（没交审、没交给人）→ 在同一个会话里续一轮；续的这一轮交审了 → 收尾
#[test]
fn a_turn_that_ends_while_holding_the_card_is_continued() {
    let s = holder("acp/fake");
    let d = tempfile::tempdir().unwrap();
    let c = turns(&s, d.path(), "acp/fake", &["--extra-turns", "3"], &[("HAND_OVER", "3")]);
    let (code, out, err) = finish(c, Duration::from_secs(30));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("↻ 第 1 次续轮（最多 3 次）：default/main#1 还由 acp/fake 持有"), "{err}");
    let again = err.lines().find(|l| l.starts_with("prompt 3: ")).unwrap_or_else(|| panic!("{err}"));
    assert!(again.contains("session/prompt") && again.contains(r#""sessionId":"s1""#), "同一个会话：{again}");
    assert!(again.contains("default/main#1「做卡」") && again.contains("第 1 次续轮，最多 3 次"), "{again}");
    assert!(err.contains("交审了") && !err.contains("prompt 4: ") && !err.contains("第 2 次续轮"), "交审之后不再续：{err}");
    // 续轮发出去的提示词也是日志里的一帧
    assert_eq!(out.lines().filter(|l| l.contains(r#""method":"session/prompt""#)).count(), 2, "{out}");
    assert_eq!(s.run("acp/fake", &["show", "1", "--json"]).json()["column"], "review");
}

/// 续到上限还持有：不再续，写明续了几轮，照常以 0 退出（board host 按卡片还在它手里记成异常退出）
#[test]
fn turns_stop_at_the_limit() {
    let s = holder("acp/fake");
    let d = tempfile::tempdir().unwrap();
    let (code, out, err) = finish(turns(&s, d.path(), "acp/fake", &["--extra-turns", "2"], &[]), Duration::from_secs(30));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("第 1 次续轮") && err.contains("第 2 次续轮") && !err.contains("第 3 次续轮"), "{err}");
    assert!(err.contains("prompt 4: ") && !err.contains("prompt 5: "), "第一轮加续的两轮：{err}");
    assert!(err.contains("✗ 续了 2 轮，default/main#1 还由 acp/fake 持有：不再续，收尾"), "{err}");
}

/// 不带 --extra-turns（拆解、自检）不查也不续；连不上服务端、没有名字时不续，照旧收尾
#[test]
fn no_turns_without_the_flag_or_an_answer() {
    let s = holder("acp/fake");
    let d = tempfile::tempdir().unwrap();
    let (code, _out, err) = finish(turns(&s, d.path(), "acp/fake", &[], &[]), Duration::from_secs(20));
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("prompt 3: ") && !err.contains("续轮") && !err.contains("查不到"), "{err}");
    let (code, _out, err) = finish(turns(&s, d.path(), "acp/fake", &["--extra-turns", "3"], &[("BOARD_URL", "http://127.0.0.1:9")]), Duration::from_secs(30));
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("✗ 查不到 acp/fake 还持有没持有卡片") && !err.contains("prompt 3: "), "{err}");
    let (code, _out, err) = finish(turns(&s, d.path(), "acp/fake", &["--extra-turns", "3"], &[("BOARD_AS", "")]), Duration::from_secs(20));
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("没有 BOARD_AS、BOARD") && !err.contains("prompt 3: "), "{err}");
}

/// 讲 ACP v2 的假 agent（#461）：握手回 protocolVersion 2；prompt 的回应只带 messageId，这一轮的进展用 state_update 报。
/// 收到的 prompt 打到标准错误（`prompt <第几条>: <帧>`），session/close 打成 `close: <帧>`。按 MODE：
/// - `bg`：第一轮 idle 时后台还有一个终端在跑（background 不空），4 秒后后台跑完，自己又报 running（不等 prompt）、
///   交审 #1、再 idle
/// - `err`：这一轮出错（stopReason error）
/// - 其他：每条 prompt 都 idle、后台为空；第 HAND_OVER 条 prompt 的那一轮里先交审 #1
const V2: &str = r#"read -r l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":2,"info":{"name":"fake-v2","version":"1"},"capabilities":{}}}'
read -r l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
st() { echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"s1\",\"update\":{\"sessionUpdate\":\"state_update\",$1}}}"; }
hand_over() { "$BOARD_BIN" beat 1 --final >/dev/null && "$BOARD_BIN" move 1 review >/dev/null && echo "交审了" >&2; }
n=0
while read -r l; do
  case "$l" in
    *'"session/close"'*) echo "close: $l" >&2; continue ;;
    *'"session/prompt"'*) ;;
    *) continue ;;
  esac
  n=$((n+1))
  pid=$(printf '%s\n' "$l" | sed 's/.*"id":\([0-9]*\).*/\1/')
  echo "prompt $n: $l" >&2
  echo "{\"jsonrpc\":\"2.0\",\"id\":$pid,\"result\":{\"messageId\":\"m$n\"}}"
  st '"state":"running"'
  if [ "$MODE" = bg ] && [ $n = 1 ]; then
    st '"state":"idle","stopReason":"end_turn","background":[{"type":"terminal","terminalId":"bg"}]'
    sleep 4
    st '"state":"running","background":[]'
    hand_over
    st '"state":"idle","stopReason":"end_turn","background":[]'
  elif [ "$MODE" = err ]; then
    st '"state":"idle","stopReason":"error","error":{"code":-32603,"message":"rate limited"}'
  else
    [ "$n" = "$HAND_OVER" ] && hand_over
    st '"state":"idle","stopReason":"end_turn","background":[]'
  fi
done
"#;

/// 卡片 #461：v2 的 agent 这一轮做完时后台还有东西在跑（idle 带着 background）→ 不续轮、不收尾，等它跑完；
/// agent 自己又开一轮、交审、idle → 收尾。收尾时发 session/close
#[test]
fn v2_waits_for_background_work_instead_of_nudging() {
    let s = holder("acp/fake");
    let d = tempfile::tempdir().unwrap();
    let c = turns_with(&s, d.path(), "acp/fake", V2, &["--extra-turns", "3"], &[("MODE", "bg")]);
    let (code, out, err) = finish(c, Duration::from_secs(40));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("⋯ 等后台：default/main#1 还由 acp/fake 持有，后台还有 1 项在跑"), "{err}");
    assert!(err.contains("prompt 1: ") && !err.contains("prompt 2: ") && !err.contains("续轮"), "后台在跑：不续轮：{err}");
    assert!(err.contains("交审了") && err.contains("close: ") && err.contains(r#""sessionId":"s1""#), "收尾发 session/close：{err}");
    assert_eq!(s.run("acp/fake", &["show", "1", "--json"]).json()["column"], "review");
}

/// v2：这一轮做完、后台为空，卡片还在手里 → 同一会话续轮；续的这一轮交审 → 收尾。这一轮出错 → 退出码 2
#[test]
fn v2_nudges_when_quiet_and_reports_errors() {
    let s = holder("acp/fake");
    let d = tempfile::tempdir().unwrap();
    let c = turns_with(&s, d.path(), "acp/fake", V2, &["--extra-turns", "3"], &[("HAND_OVER", "2")]);
    let (code, out, err) = finish(c, Duration::from_secs(40));
    assert_eq!(code, 0, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("↻ 第 1 次续轮（最多 3 次）：default/main#1 还由 acp/fake 持有") && !err.contains("等后台"), "{err}");
    let again = err.lines().find(|l| l.starts_with("prompt 2: ")).unwrap_or_else(|| panic!("{err}"));
    assert!(again.contains("default/main#1「做卡」") && again.contains(r#""sessionId":"s1""#), "{again}");
    assert!(err.contains("交审了") && !err.contains("prompt 3: "), "{err}");
    let d = tempfile::tempdir().unwrap();
    let (code, _out, err) = finish(turns_with(&s, d.path(), "acp/fake", V2, &["--extra-turns", "3"], &[("MODE", "err")]), Duration::from_secs(30));
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("✗ 这一轮出错：rate limited") && !err.contains("续轮"), "出错不续轮：{err}");
}

/// v2：干活时收到 SIGTERM → session/cancel，agent 以 cancelled 回到 idle → 退出码 130
#[test]
fn v2_sigterm_cancels_the_work() {
    let d = tempfile::tempdir().unwrap();
    // prompt 之后一直在干活，收到 session/cancel 才回 idle（cancelled）；收到 prompt 时在工作目录里留个记号
    let script = r#"read l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":2,"info":{"name":"fake-v2"},"capabilities":{}}}'
read l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
read l; echo '{"jsonrpc":"2.0","id":2,"result":{"messageId":"m1"}}'
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"state_update","state":"running"}}}'; touch prompted
read l; echo "got: $l" >&2
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"state_update","state":"idle","stopReason":"cancelled"}}}'
read l; exit 0
"#;
    let c = bridge(d.path(), script);
    let mark = d.path().join("prompted");
    let until = Instant::now() + Duration::from_secs(15);
    while !mark.exists() {
        assert!(Instant::now() < until, "15 秒内 agent 没收到 prompt");
        std::thread::sleep(Duration::from_millis(50));
    }
    unsafe {
        libc::kill(c.id() as i32, libc::SIGTERM);
    }
    let (code, out, err) = finish(c, Duration::from_secs(15));
    assert_eq!(code, 130, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("got: ") && err.contains("session/cancel"), "{err}");
}

/// 额度用完（#668）：usage_update 带着额度（rejected），这一轮以 rate_limit 出错 → 退出码 4，标准错误写明什么时候恢复，
/// 看到的额度写进 `--quota-file`（board host 随派发结束上报）
#[test]
fn usage_limit_exits_4_and_writes_the_quota_file() {
    let d = tempfile::tempdir().unwrap();
    let script = r#"read l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":2,"info":{"name":"fake-v2"},"capabilities":{}}}'
read l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
read l; echo '{"jsonrpc":"2.0","id":2,"result":{"messageId":"m1"}}'
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"state_update","state":"running"}}}'
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"usage_update","used":5,"size":200000,"_meta":{"_claude/rateLimit":{"status":"rejected","resetsAt":4102444800,"rateLimitType":"five_hour","utilization":1}}}}}'
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"state_update","state":"idle","stopReason":"error","error":{"code":-32603,"message":"usage limit reached","data":{"errorKind":"rate_limit"}}}}}'
read l; exit 0
"#;
    let fake = write(d.path(), "fake.sh", script);
    let prompt = write(d.path(), "prompt.txt", "做卡");
    let quota = d.path().join("1.quota");
    let dir = d.path().display().to_string();
    let c = Command::new(env!("CARGO_BIN_EXE_board"))
        .args(["acp", "--env", "direct", "--cwd", &dir, "--prompt-file", &prompt, "--quota-file", &quota.display().to_string(), "--", "sh", &fake])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (code, out, err) = finish(c, Duration::from_secs(30));
    assert_eq!(code, 4, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("✗ 额度用完，") && err.contains("这一轮出错：usage limit reached"), "{err}");
    let q: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&quota).unwrap()).unwrap();
    assert_eq!(q, serde_json::json!({ "exhausted": true, "resetsAt": 4102444800000i64, "status": "rejected", "window": "five_hour", "used": 100, "text": "usage limit reached", "at": 0 }));
}
