//! 派发日志的转写（卡片 default/main#211）。
//!
//! board host 通过 `board acp`（acp.rs）用 ACP 驱动 agent，本机日志文件 `~/.board/host/logs/<派发 ID>.log` 里是一行一帧的
//! ACP 消息（JSON-RPC）：`board acp` 发出去的请求、agent 的回应和 `session/update` 通知，还有 agent、`board acp` 写到标准错误
//! 的提示。上传到服务端（界面「原始输出」）和异常退出评论里的「日志最后 20 行」之前，用这里的函数转成一行一行能读的内容：
//!
//! ```text
//! ● 开始 · Claude Agent 0.107.3
//! （思考）用户要我执行 #211，先看卡片。 …
//! 先领卡，再看代码。
//! ▸ shell  board show 211 --json
//!   ↳ {"id":211}
//! ▸ edit  crates/board-cli/src/host.rs
//!   ✗ String to replace not found in file.
//! ■ 结束
//! ```
//!
//! - 不是 JSON 的行（标准错误里的提示）原样保留。
//! - 握手、命令列表、模式和配置的变化、用量这些对人没用的帧不显示。
//! - 每一行最长 [`LINE_MAX`] 个字符，一段话最多 [`TEXT_LINES`] 行，工具结果只写第一行和总行数。
//!
//! 帧的格式按 ACP v1 的 schema（<https://agentclientprotocol.com/protocol/v1/prompt-turn>、`tool-calls`、`agent-plan`）和
//! v2 草案（#461，Claude Code、Codex 走 v2）。v2 的不同：
//! - 没有单独的 `tool_call`，开始和后来的变化都是 `tool_call_update`，每次只带改了的字段（`kind`、`title` 往往只在第一帧，
//!   命令这类参数 `rawInput` 在后面的帧）。一次转写里记下每个调用的这些字段（[`Calls`]），带着参数的那一帧写「▸」。
//! - 命令的输出在终端里：`terminal_output_chunk` 是边跑边来的片段（不逐条写），跑完时 `terminal_update` 带着完整的
//!   `output`（base64）和 `exitStatus`，写成一行摘要；工具调用只引用这个终端，结束时没有文字就不再写一行。
//! - 这一轮的结束在 `state_update`：`idle` 带 `stopReason` 写「■ 结束」，出错的先写原因。计划是 `plan_update`。
//!
//! 同一份日志还由 events.rs 解析成结构化事件（会话「过程」，卡片 #184）；这里的转写是「原始输出」看到的内容。

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// 一行最长多少个字符（多出来的截掉，末尾写 …）
pub(crate) const LINE_MAX: usize = 400;
/// agent 说的一段话最多保留几行
const TEXT_LINES: usize = 40;
/// 工具参数、结果摘要最长多少个字符
const BRIEF: usize = 160;

/// 日志的格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    /// `board acp` 写的 ACP 帧
    Acp,
}

impl Stream {
    /// 报给服务端的格式名（派发的 `fmt`）
    pub(crate) fn key(self) -> &'static str {
        match self {
            Stream::Acp => "acp",
        }
    }
}

/// 把日志里的原始行转成上传的行。kind 为空、或这一行不是 JSON 对象时原样保留
pub(crate) fn render(kind: Option<Stream>, raw: Vec<String>) -> Vec<String> {
    let Some(kind) = kind else { return raw };
    let mut out = vec![];
    let mut calls = Calls::default();
    for line in raw {
        let t = line.trim();
        let v = if t.starts_with('{') { serde_json::from_str::<Value>(t).ok().filter(Value::is_object) } else { None };
        match v {
            Some(v) => match kind {
                Stream::Acp => acp(&v, &mut calls, &mut out),
            },
            None => out.push(line),
        }
    }
    out.into_iter().map(|l| clip(&l, LINE_MAX)).collect()
}

/* ------------------------------ ACP ------------------------------ */

fn acp(v: &Value, calls: &mut Calls, out: &mut Vec<String>) {
    let mut push = |x: String| out.push(x);
    if let Some(e) = error_of(v) {
        push(format!("✗ 出错：{}", clip(&e, BRIEF)));
        return;
    }
    if let Some(u) = update_of(v) {
        match s(&u["sessionUpdate"]) {
            "agent_message_chunk" => text(&content_text(&u["content"]), &mut push),
            "agent_thought_chunk" => thinking(&content_text(&u["content"]), &mut push),
            "tool_call" | "tool_call_update" => {
                let id = s(&u["toolCallId"]);
                let call = calls.merge(u);
                if calls.starts(u, &call) {
                    let (name, arg) = tool_of(&call);
                    push(if arg.is_empty() { format!("▸ {name}") } else { format!("▸ {name}  {arg}") });
                }
                if let Some(ok) = finished(u) {
                    let body = tool_output(u);
                    // v2：命令的输出、退出码在终端里，跑完时已经写过一行（terminal_update），这里没有文字就不再写
                    if !(body.trim().is_empty() && refs_terminal(&call)) {
                        push(result_line(&body, !ok));
                    }
                    calls.done(id);
                }
            }
            "terminal_update" => {
                let code = terminal_code(u);
                let failed = code.is_some_and(|c| c != 0);
                let mark = code.filter(|c| *c != 0).map(|c| format!("（退出码 {c}）")).unwrap_or_default();
                match u["output"]["data"].as_str() {
                    Some(data) => push(result_line(&terminal_text(data), failed) + &mark),
                    None if failed => push(format!("  ✗ 退出码 {}", code.unwrap_or_default())),
                    None => {}
                }
            }
            "state_update" if s(&u["state"]) == "idle" => match u["stopReason"].as_str() {
                Some("error") => {
                    push(format!("✗ 出错：{}", clip(&error_of(u).unwrap_or_else(|| "出错了".into()), BRIEF)));
                    push("■ 结束 · 出错".into());
                }
                Some(reason) => push(end_line(reason)),
                None => {}
            },
            "plan" => push(format!("● {}", plan_brief(u))),
            "plan_update" if !arr(&u["plan"]["entries"]).is_empty() => push(format!("● {}", plan_brief(&u["plan"]))),
            _ => {}
        }
        return;
    }
    match s(&v["method"]) {
        "session/set_mode" => push(format!("● 切换到 {} 模式", s(&v["params"]["modeId"]))),
        "session/set_config_option" => {
            let (id, val) = (s(&v["params"]["configId"]), s(&v["params"]["value"]));
            push(if id == "mode" { format!("● 切换到 {val} 模式") } else { format!("● 设置 {id}：{val}") });
        }
        _ => {}
    }
    let r = &v["result"];
    // initialize 的回应：v1 是 agentInfo，v2 是 info
    let info = r.get("agentInfo").filter(|x| x.is_object()).or_else(|| r.get("info").filter(|x| x.is_object() && r.get("protocolVersion").is_some()));
    if let Some(info) = info {
        let name = [s(&info["title"]), s(&info["name"])].into_iter().find(|x| !x.is_empty()).unwrap_or("agent");
        let mut head = format!("● 开始 · {name}");
        if !s(&info["version"]).is_empty() {
            head += &format!(" {}", s(&info["version"]));
        }
        push(head);
    }
    if let Some(reason) = r["stopReason"].as_str() {
        push(end_line(reason));
    }
}

/// 「■ 结束」，不是正常结束时带上说法
fn end_line(reason: &str) -> String {
    let words = stop_words(reason);
    if words.is_empty() { "■ 结束".to_string() } else { format!("■ 结束 · {words}") }
}

/// 工具调用记下的字段：v2 的更新只带改了的字段，名称（`kind`）、标题、位置、参数、内容并起来才完整（v1 的
/// `tool_call` 第一帧就全）。也记下哪些调用已经写过开始（「▸」或事件），同一个调用只写一次
#[derive(Default)]
pub(crate) struct Calls {
    fields: HashMap<String, Value>,
    started: HashSet<String>,
}

impl Calls {
    /// 把这一帧带的字段并进这个调用记下的，返回并起来的（只含 kind、title、locations、rawInput、content）
    pub(crate) fn merge(&mut self, u: &Value) -> Value {
        let entry = self.fields.entry(s(&u["toolCallId"]).to_string()).or_insert_with(|| json!({}));
        for k in ["kind", "title", "locations", "rawInput", "content"] {
            if let Some(x) = u.get(k).filter(|x| !x.is_null()) {
                entry[k] = x.clone();
            }
        }
        entry.clone()
    }

    /// 这一帧该不该写这个调用的开始：v1 的 `tool_call`；v2 的第一次带上参数（`rawInput`）；参数一直没来、调用就结束了的，
    /// 结束时补上（认得出是什么工具时）。每个调用只返回一次 true
    pub(crate) fn starts(&mut self, u: &Value, call: &Value) -> bool {
        let id = s(&u["toolCallId"]);
        if self.started.contains(id) {
            return false;
        }
        let input = match &call["rawInput"] {
            Value::Object(m) => !m.is_empty(),
            Value::String(x) => !x.trim().is_empty(),
            _ => false,
        };
        let known = !s(&call["title"]).is_empty() || !s(&call["kind"]).is_empty();
        let start = s(&u["sessionUpdate"]) == "tool_call" || input || (finished(u).is_some() && known);
        if start {
            self.started.insert(id.to_string());
        }
        start
    }

    /// 调用结束后不再需要它的字段和「已经写过开始」的标记；长派发里几千次调用不能一直攒着
    pub(crate) fn done(&mut self, id: &str) {
        self.fields.remove(id);
        self.started.remove(id);
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.fields.len().max(self.started.len())
    }
}

/// 工具调用的内容里引用了终端（v2：命令的输出在终端里）
pub(crate) fn refs_terminal(call: &Value) -> bool {
    arr(&call["content"]).iter().any(|c| s(&c["type"]) == "terminal")
}

/// 工具调用引用的终端（v2）
pub(crate) fn terminal_of(call: &Value) -> Option<String> {
    arr(&call["content"]).iter().find(|c| s(&c["type"]) == "terminal").map(|c| s(&c["terminalId"]).to_string()).filter(|x| !x.is_empty())
}

/// v2 终端的退出码（`exitStatus.exitCode`；被信号结束时没有）
pub(crate) fn terminal_code(u: &Value) -> Option<i64> {
    u["exitStatus"]["exitCode"].as_i64()
}

/// v2 终端的数据：base64 编码的字节，像终端那样换行是 \r\n；解不开时原样当文字
pub(crate) fn terminal_text(data: &str) -> String {
    let text = match STANDARD.decode(data.trim()) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(_) => data.to_string(),
    };
    text.replace("\r\n", "\n")
}

/// `session/update` 通知里的 update
pub(crate) fn update_of(v: &Value) -> Option<&Value> {
    (v["method"].as_str() == Some("session/update")).then(|| &v["params"]["update"]).filter(|u| u.is_object())
}

/// 回应里的错误（文字）
pub(crate) fn error_of(v: &Value) -> Option<String> {
    let e = v.get("error").filter(|e| e.is_object())?;
    let msg = s(&e["message"]);
    let data = e["data"].as_str().or_else(|| e["data"]["message"].as_str()).unwrap_or_default();
    Some(match (msg.is_empty(), data.is_empty() || data == msg) {
        (false, true) => msg.to_string(),
        (false, false) => format!("{msg}（{data}）"),
        (true, false) => data.to_string(),
        (true, true) => "出错了".into(),
    })
}

/// 结束原因的说法（`end_turn` 是正常结束，没有说法）
pub(crate) fn stop_words(reason: &str) -> &str {
    match reason {
        "end_turn" => "",
        "cancelled" => "已停止",
        "max_tokens" => "输出长度到了上限",
        "max_turn_requests" => "轮数到了上限",
        "refusal" => "模型拒绝继续",
        r => r,
    }
}

/// 内容块里的文字（只认 text）
pub(crate) fn content_text(c: &Value) -> String {
    match s(&c["type"]) {
        "text" => s(&c["text"]).to_string(),
        "resource_link" => s(&c["uri"]).to_string(),
        _ => String::new(),
    }
}

/// 工具调用的（名称、参数摘要）。名称按 `kind` 取一个短词（shell、read、edit …），认不出时用标题；
/// 参数先看 `rawInput` 里最能说明在干什么的那一项（命令、文件、搜索词），没有时用标题、再没有用第一个位置
pub(crate) fn tool_of(u: &Value) -> (String, String) {
    let title = s(&u["title"]).trim().trim_matches('`').trim();
    let short = match s(&u["kind"]) {
        "execute" => "shell",
        "read" => "read",
        "edit" => "edit",
        "delete" => "delete",
        "move" => "move",
        "search" => "search",
        "fetch" => "fetch",
        "think" => "think",
        "switch_mode" => "mode",
        _ => "",
    };
    let raw = &u["rawInput"];
    let from_raw = if raw.is_object() { tool_arg(raw) } else { String::new() };
    let from_raw = if from_raw.starts_with('{') || from_raw.starts_with('[') { String::new() } else { from_raw };
    let place = u["locations"].as_array().and_then(|l| l.first()).map(|l| s(&l["path"]).to_string()).unwrap_or_default();
    if short.is_empty() {
        let name = if title.is_empty() { "工具".to_string() } else { brief(title) };
        return (name, from_raw);
    }
    let arg = [from_raw, brief(title), brief(&place)].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
    (short.to_string(), arg)
}

/// 工具做完了没有：completed → 成功，failed → 失败；还在跑时 None
pub(crate) fn finished(u: &Value) -> Option<bool> {
    match s(&u["status"]) {
        "completed" => Some(true),
        "failed" => Some(false),
        _ => None,
    }
}

/// 工具的结果文字：`content` 里的文字（去掉包着的 ``` ），改了的文件写「改了 <路径>」；Codex 的终端输出在
/// `_meta.terminal_output_delta`；都没有时看 `rawOutput`
pub(crate) fn tool_output(u: &Value) -> String {
    let mut parts = vec![];
    for c in arr(&u["content"]) {
        match s(&c["type"]) {
            "content" => {
                let t = content_text(&c["content"]);
                if !t.trim().is_empty() {
                    parts.push(unfence(&t));
                }
            }
            "diff" => parts.push(format!("改了 {}", s(&c["path"]))),
            _ => {}
        }
    }
    if !parts.is_empty() {
        return parts.join("\n");
    }
    if let Some(x) = u["_meta"]["terminal_output_delta"]["data"].as_str().filter(|x| !x.is_empty()) {
        return x.to_string();
    }
    let r = &u["rawOutput"];
    match r {
        Value::String(x) => x.clone(),
        Value::Object(_) => ["formatted_output", "aggregated_output", "output", "stdout", "stderr", "result", "error", "message"]
            .iter()
            .find_map(|k| r[*k].as_str().filter(|x| !x.trim().is_empty()))
            .unwrap_or_default()
            .to_string(),
        Value::Null => String::new(),
        _ => r.to_string(),
    }
}

/// 工具的退出码：Codex 的终端在 `_meta.terminal_exit`，其他 agent 在 `rawOutput` 里
pub(crate) fn tool_code(u: &Value) -> Option<i64> {
    let t = &u["_meta"]["terminal_exit"];
    let r = &u["rawOutput"];
    t["exit_code"].as_i64().or_else(|| t["exitCode"].as_i64()).or_else(|| r["exit_code"].as_i64()).or_else(|| r["exitCode"].as_i64())
}

/// 去掉整段包着的 ```（Claude Code 的命令输出这样写）
fn unfence(t: &str) -> String {
    let t = t.trim();
    let Some(rest) = t.strip_prefix("```") else { return t.to_string() };
    let body = rest.split_once('\n').map(|(_, b)| b).unwrap_or_default();
    body.trim_end().strip_suffix("```").unwrap_or(body).trim_matches('\n').to_string()
}

/// 计划一句话：「计划 3 项 · 正在做：改代码」
pub(crate) fn plan_brief(u: &Value) -> String {
    let items = arr(&u["entries"]);
    let now = items.iter().find(|e| s(&e["status"]) == "in_progress").map(|e| s(&e["content"])).unwrap_or_default();
    let done = items.iter().filter(|e| s(&e["status"]) == "completed").count();
    if now.is_empty() {
        format!("计划 {} 项 · 完成 {done} 项", items.len())
    } else {
        format!("计划 {} 项 · 正在做：{}", items.len(), brief(now))
    }
}

/// 计划写成 Markdown 任务列表（「过程」里一条 message）
pub(crate) fn plan_markdown(u: &Value) -> String {
    arr(&u["entries"])
        .iter()
        .map(|e| {
            let mark = if s(&e["status"]) == "completed" { "x" } else { " " };
            let now = if s(&e["status"]) == "in_progress" { "（正在做）" } else { "" };
            format!("- [{mark}] {}{now}", s(&e["content"]))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/* ------------------------------ 共用 ------------------------------ */

/// 工具参数的摘要：命令、文件、搜索词这些最能说明在干什么的那一项；都没有时写紧凑的 JSON
pub(crate) fn tool_arg(input: &Value) -> String {
    for k in ["command", "cmd", "file_path", "path", "pattern", "url", "query", "description", "skill", "prompt"] {
        if let Some(x) = input[k].as_str().filter(|x| !x.trim().is_empty()) {
            let mut a = brief(x);
            // 搜索带上范围
            if matches!(k, "pattern") {
                if let Some(p) = input["path"].as_str().filter(|p| !p.is_empty()) {
                    a += &format!("  （{}）", brief(p));
                }
            }
            return a;
        }
    }
    match input {
        Value::Object(m) if m.is_empty() => String::new(),
        Value::Null => String::new(),
        _ => clip(&input.to_string(), BRIEF),
    }
}

/// 工具结果：`  ↳ 第一行  （共 N 行）`；出错时 `  ✗ …`
fn result_line(body: &str, err: bool) -> String {
    let lines: Vec<&str> = body.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    let first = lines.first().map(|l| brief(l.trim())).unwrap_or_else(|| if err { "出错".into() } else { "（没有输出）".into() });
    let tail = if lines.len() > 1 { format!("  （共 {} 行）", lines.len()) } else { String::new() };
    format!("  {} {first}{tail}", if err { "✗" } else { "↳" })
}

/// agent 说的一段话：去掉空行，最多 TEXT_LINES 行
fn text(t: &str, push: &mut impl FnMut(String)) {
    let lines: Vec<&str> = t.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    for l in lines.iter().take(TEXT_LINES) {
        push(l.to_string());
    }
    if lines.len() > TEXT_LINES {
        push(format!("…（还有 {} 行）", lines.len() - TEXT_LINES));
    }
}

/// 思考：只写第一行（被隐藏的思考没有文字，不写）
fn thinking(t: &str, push: &mut impl FnMut(String)) {
    let mut lines = t.lines().map(str::trim).filter(|l| !l.is_empty());
    if let Some(first) = lines.next() {
        let more = if lines.next().is_some() { " …" } else { "" };
        push(format!("（思考）{}{more}", brief(first)));
    }
}

/// 截到 BRIEF 个字符，只取第一行（多行的命令后面写 …）
pub(crate) fn brief(x: &str) -> String {
    let mut ls = x.trim().lines();
    let first = ls.next().unwrap_or_default().trim();
    let more = ls.next().is_some();
    let c = clip(first, BRIEF);
    if more && !c.ends_with('…') {
        format!("{c} …")
    } else {
        c
    }
}

/// 截到 max 个字符，多出来的写 …
pub(crate) fn clip(x: &str, max: usize) -> String {
    if x.chars().count() <= max {
        x.to_string()
    } else {
        let mut s: String = x.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

pub(crate) fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

pub(crate) fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    /// `board acp` 写的一次运行（按 ACP v1 的 schema 写的样例，带上了 board acp 自己发的请求）
    const ACP: &str = include_str!("../tests/fixtures/acp-session.jsonl");
    const CODEX_TERMINAL: &str = include_str!("../tests/fixtures/codex-terminal.jsonl");

    #[test]
    fn acp_frames_become_readable_lines() {
        let out = render(Some(Stream::Acp), lines(ACP));
        let want = vec![
            "● 开始 · Claude Agent 0.107.3",
            "● 切换到 bypassPermissions 模式",
            "（思考）用户要我执行 #211，先看卡片。 …",
            "先领卡，再看代码。",
            "▸ shell  board show 211 --json",
            "  ↳ {\"id\":211}",
            "▸ edit  crates/board-cli/src/host.rs",
            "  ✗ String to replace not found in file.",
            "● 计划 3 项 · 正在做：改代码",
            "▸ shell  cargo test",
            "  ✗ error[E0425]: cannot find value `x`  （共 2 行）",
            "▸ edit  Write docs/agents.md",
            "  ↳ 改了 docs/agents.md",
            "✗ 出错：Internal error（rate limited）",
            "改完了。",
            "提交：a5ea001",
            "■ 结束",
            "[claude-code:unrecognized_model] {\"model\":\"x\"}",
        ];
        assert_eq!(out, want);
    }

    const ACP_V2: &str = include_str!("../tests/fixtures/acp-v2-session.jsonl");

    /// v2（#461）：工具调用的名称在第一帧、命令在第二帧，并起来写一行「▸」；终端跑完时的完整输出写一行摘要
    /// （带退出码），工具调用结束时没有文字不再写；idle 带 stopReason 写「■ 结束」，重报的 idle、running 不写
    #[test]
    fn acp_v2_frames_become_readable_lines() {
        let out = render(Some(Stream::Acp), lines(ACP_V2));
        let want = vec![
            "● 开始 · Claude Agent 0.107.3",
            "● 切换到 bypassPermissions 模式",
            "先跑测试。",
            "▸ shell  npm test",
            "  ✗ line1  （共 3 行）（退出码 1）",
            "● 计划 2 项 · 正在做：交审",
            "■ 结束",
            "后台跑完了，交审。",
            "■ 结束",
        ];
        assert_eq!(out, want);
        // 出错结束：先写原因
        let err = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"state_update","state":"idle","stopReason":"error","error":{"code":-32603,"message":"rate limited"}}}}"#;
        assert_eq!(render(Some(Stream::Acp), lines(err)), ["✗ 出错：rate limited", "■ 结束 · 出错"]);
        // 终端解不开 base64 时原样当文字
        assert_eq!(terminal_text("not base64!"), "not base64!");
    }

    #[test]
    fn stop_reasons_and_unknown_tools() {
        let end = |r: &str| render(Some(Stream::Acp), vec![serde_json::json!({ "jsonrpc": "2.0", "id": 3, "result": { "stopReason": r } }).to_string()]);
        assert_eq!(end("cancelled"), ["■ 结束 · 已停止"]);
        assert_eq!(end("max_tokens"), ["■ 结束 · 输出长度到了上限"]);
        let set = |id: &str, v: &str| render(Some(Stream::Acp), vec![serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "session/set_config_option", "params": { "sessionId": "s", "configId": id, "value": v } }).to_string()]);
        assert_eq!(set("mode", "bypassPermissions"), ["● 切换到 bypassPermissions 模式"]);
        assert_eq!(set("reasoning_effort", "high"), ["● 设置 reasoning_effort：high"]);
        // 认不出 kind 的工具：名称用标题，参数看 rawInput
        let u = serde_json::json!({ "toolCallId": "x", "title": "mcp__db__query", "kind": "other", "rawInput": { "query": "select 1" } });
        assert_eq!(tool_of(&u), ("mcp__db__query".to_string(), "select 1".to_string()));
        let u = serde_json::json!({ "toolCallId": "x" });
        assert_eq!(tool_of(&u), ("工具".to_string(), String::new()));
        assert_eq!(tool_code(&serde_json::json!({ "rawOutput": { "exitCode": 2 } })), Some(2));
        assert_eq!(unfence("```sh\nls\na\n```"), "ls\na");
        assert_eq!(unfence("plain"), "plain");
    }

    #[test]
    fn codex_terminal_output_and_exit_code_are_read_from_meta() {
        let out = render(Some(Stream::Acp), lines(CODEX_TERMINAL));
        assert_eq!(out, ["▸ shell  sh -c \"exit 7\"", "  ✗ oops"]);
        let u: Value = serde_json::from_str(CODEX_TERMINAL.lines().nth(1).unwrap()).unwrap();
        let u = &u["params"]["update"];
        assert_eq!(tool_output(u), "oops\n");
        assert_eq!(tool_code(u), Some(7));
    }

    #[test]
    fn text_lines_stay_as_they_are() {
        let raw = lines("Error: not logged in\n✗ agent 在这一轮结束之前就退出了（退出码 1）\n[1, 2]\n{broken");
        assert_eq!(render(Some(Stream::Acp), raw.clone()), raw);
        assert_eq!(render(None, raw.clone()), raw);
    }

    #[test]
    fn long_lines_and_long_text_are_clipped() {
        let msg = |t: &str| serde_json::json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s", "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": t } } } }).to_string();
        let long = "字".repeat(LINE_MAX + 50);
        let out = render(Some(Stream::Acp), vec![msg(&long)]);
        assert_eq!(out[0].chars().count(), LINE_MAX);
        assert!(out[0].ends_with('…'));
        let many: String = (0..60).map(|i| format!("第 {i} 行\n")).collect();
        let out = render(Some(Stream::Acp), vec![msg(&many)]);
        assert_eq!(out.len(), TEXT_LINES + 1);
        assert_eq!(out.last().unwrap(), "…（还有 20 行）");
        // 非 JSON 的长行同样截断
        let out = render(Some(Stream::Acp), vec!["x".repeat(LINE_MAX * 2)]);
        assert_eq!(out[0].chars().count(), LINE_MAX);
    }
}
