//! `board acp`（board host 和 agent 之间讲 ACP 的那一层）：用一个假的 ACP agent（sh 脚本，按顺序读请求、回固定的帧）
//! 走一遍完整的一轮、被取消、agent 提前退出。不执行真实的 agent。
#![cfg(unix)]

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
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
        .stdin(Stdio::null())
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
    // prompt 之后一直不结束，收到 session/cancel 才回 cancelled
    let script = r#"read l; echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
read l; echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s1"}}'
read l; echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}}}'
read l; echo "got: $l" >&2; echo '{"jsonrpc":"2.0","id":2,"result":{"stopReason":"cancelled"}}'
read l; exit 0
"#;
    let c = bridge(d.path(), script);
    // 等它发了 prompt（日志里有 working 之前不会有取消可发）
    std::thread::sleep(Duration::from_millis(800));
    unsafe {
        libc::kill(c.id() as i32, libc::SIGTERM);
    }
    let (code, out, err) = finish(c, Duration::from_secs(15));
    assert_eq!(code, 130, "out:\n{out}\nerr:\n{err}");
    assert!(err.contains("session/cancel"), "{err}");
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
