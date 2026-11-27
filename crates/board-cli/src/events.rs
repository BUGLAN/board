//! 派发日志 → 统一事件（卡片 default/main#184；事件的定义在 board-core 的 runlog.rs）。
//!
//! `board host` 逐行读派发日志（`board acp` 写的 ACP 帧，见 acp.rs、logfmt.rs），每个在跑的派发一个 [`Parser`]：
//! JSON 行按 ACP 的帧转成 `message`、`tool`、`error`、`result`，不是 JSON 的行（标准错误里的提示）记成 `text`，
//! 对人没用的帧（握手、思考、命令列表、模式和配置的变化）不记。`tests/fixtures/acp-session.jsonl` 是一份样例，
//! 单元测试按它核对事件的种类和条数。
//!
//! - 消息：`agent_message_chunk`（`board acp` 已经把连续的片段合成一帧）→ `message`；`plan` 写成 Markdown 任务列表的 `message`。
//! - 工具：`tool_call` → 调用，`tool_call_update` 到了 completed / failed → 结果，两条带着同一个 `call`（`toolCallId`），
//!   上传前同一批里合成一条，跨批的由服务端合并。参数只留摘要，结果留前 [`OUT_LINES`] 行、最多 [`OUT_MAX`] 个字符；
//!   `_meta` 或 `rawOutput` 里有退出码时记下。
//! - `result`：这一轮的结束——v1 是 prompt 的回应（`stopReason`），v2 是带 `stopReason` 的 `idle`（`state_update`）。
//!   带了 `usage`（这一轮的输入 / 输出 tokens）时记下；费用取最后一次 `usage_update` 里的 `cost`（整个会话的累计值）减去
//!   前面几轮已经记过的：续轮（#454）、v2 的 agent 自己接着做时一个会话有好几条 `result`，界面把它们加起来，每条只记这一轮花的。
//! - 出错的回应、v2 出错结束的 `idle` → `error`。
//!
//! v2（#461，Claude Code、Codex）：工具调用只有 `tool_call_update`，每次只带改了的字段，用 [`logfmt::Calls`] 并起来，
//! 带着参数的那一帧记调用；命令的输出、退出码在它引用的终端里（`terminal_output_chunk` 的片段、跑完时 `terminal_update`
//! 的完整 `output` 和 `exitStatus`），工具调用结束时没有文字就用终端的；计划是 `plan_update`。

use crate::logfmt::{self, clip, s, Stream};
use board_core::runlog::RunEvent;
use serde_json::Value;
use std::collections::HashMap;

/// agent 说的一段话最多多少个字符（服务端单条 4 KB 截断，这里先收一些）
const TEXT_MAX: usize = 3000;
/// 工具结果最多留几行
const OUT_LINES: usize = 20;
/// 工具结果最多多少个字符
const OUT_MAX: usize = 1500;
/// v2 终端的输出最多留多少个字节（摘要只用前 OUT_LINES 行）
const TERM_KEEP: usize = 16 * 1024;

/// 一个派发的解析器：费用要记住最后一次 `usage_update`，以及前面几轮的 `result` 已经记了多少；
/// v2 还要记住工具调用的字段和终端的输出
pub(crate) struct Parser {
    kind: Stream,
    cost: Option<f64>,
    billed: f64,
    calls: logfmt::Calls,
    /// v2 终端：输出（解码后最多 TERM_KEEP 个字节）、退出码
    terms: HashMap<String, (String, Option<i64>)>,
}

impl Parser {
    pub(crate) fn new(kind: Stream) -> Parser {
        Parser { kind, cost: None, billed: 0.0, calls: logfmt::Calls::default(), terms: HashMap::new() }
    }

    /// 一行输出 → 0 到多条事件。`at` 是读到这一行的时间（毫秒）
    pub(crate) fn feed(&mut self, line: &str, at: i64) -> Vec<RunEvent> {
        let t = line.trim();
        if t.is_empty() {
            return vec![];
        }
        let v = if t.starts_with('{') { serde_json::from_str::<Value>(t).ok().filter(Value::is_object) } else { None };
        let mut out = match v {
            Some(v) => match self.kind {
                Stream::Acp => self.acp(&v),
            },
            None => vec![RunEvent::text("text", clip(line.trim_end(), logfmt::LINE_MAX))],
        };
        for e in &mut out {
            e.at = at;
        }
        out
    }

    fn acp(&mut self, v: &Value) -> Vec<RunEvent> {
        if let Some(e) = logfmt::error_of(v) {
            return vec![RunEvent::text("error", clip(&e, TEXT_MAX))];
        }
        if let Some(u) = logfmt::update_of(v) {
            let id = s(&u["toolCallId"]);
            return match s(&u["sessionUpdate"]) {
                "agent_message_chunk" => message(&logfmt::content_text(&u["content"])).into_iter().collect(),
                "plan" => message(&logfmt::plan_markdown(u)).into_iter().collect(),
                "plan_update" => message(&logfmt::plan_markdown(&u["plan"])).into_iter().collect(),
                "tool_call" | "tool_call_update" => {
                    let call = self.calls.merge(u);
                    let mut out = vec![];
                    if self.calls.starts(u, &call) {
                        let (name, arg) = logfmt::tool_of(&call);
                        out.push(RunEvent { kind: "tool".into(), call: some_text(id), tool: Some(name), args: some_text(&arg), ..Default::default() });
                    }
                    out.extend(self.tool_done(u, id, &call));
                    if logfmt::finished(u).is_some() {
                        self.calls.done(id);
                    }
                    out
                }
                "terminal_update" => {
                    let t = self.terms.entry(s(&u["terminalId"]).to_string()).or_default();
                    if let Some(data) = u["output"]["data"].as_str() {
                        t.0 = keep(logfmt::terminal_text(data));
                    }
                    if let Some(c) = logfmt::terminal_code(u) {
                        t.1 = Some(c);
                    }
                    vec![]
                }
                "terminal_output_chunk" => {
                    let t = self.terms.entry(s(&u["terminalId"]).to_string()).or_default();
                    if t.0.len() < TERM_KEEP {
                        t.0 = keep(t.0.clone() + &logfmt::terminal_text(s(&u["data"])));
                    }
                    vec![]
                }
                "state_update" if s(&u["state"]) == "idle" => match u["stopReason"].as_str() {
                    Some("error") => {
                        let why = logfmt::error_of(u).unwrap_or_else(|| "出错了".into());
                        vec![RunEvent::text("error", clip(&why, TEXT_MAX)), self.result("error", &u["usage"], Some("出错"))]
                    }
                    Some(reason) => vec![self.result(reason, &u["usage"], None)],
                    None => vec![],
                },
                "usage_update" => {
                    if let Some(c) = u["cost"]["amount"].as_f64() {
                        self.cost = Some(c);
                    }
                    vec![]
                }
                _ => vec![],
            };
        }
        let r = &v["result"];
        let Some(reason) = r["stopReason"].as_str() else { return vec![] };
        vec![self.result(reason, &r["usage"], None)]
    }

    /// 这一轮结束的事件。`text` 不给时用结束原因的说法
    fn result(&mut self, reason: &str, usage: &Value, text: Option<&str>) -> RunEvent {
        // usage_update 的费用是会话累计的：这一轮只记比上一条 result 多出来的
        let cost = self.cost.take().map(|total| {
            let this = (total - self.billed).max(0.0);
            self.billed = self.billed.max(total);
            this
        });
        RunEvent {
            kind: "result".into(),
            ok: Some(reason == "end_turn"),
            // 人停的（派发的「停止」、对话的「停止这一轮」）：界面上写停止了，不算出错
            stopped: reason == "cancelled",
            text: some_text(text.unwrap_or(logfmt::stop_words(reason))),
            tokens_in: usage["inputTokens"].as_i64(),
            tokens_out: usage["outputTokens"].as_i64(),
            cost,
            ..Default::default()
        }
    }

    /// 工具做完了（completed / failed）时的结果事件。v2 命令的输出、退出码在它引用的终端里（终端名不写时和调用同名）
    fn tool_done(&mut self, u: &Value, id: &str, call: &Value) -> Option<RunEvent> {
        let ok = logfmt::finished(u)?;
        let mut body = logfmt::tool_output(u);
        let mut code = logfmt::tool_code(u);
        let term = logfmt::terminal_of(call).unwrap_or_else(|| id.to_string());
        if let Some((out, c)) = self.terms.remove(&term) {
            if body.trim().is_empty() {
                body = out;
            }
            code = code.or(c);
        }
        let mut e = done(id, &body, ok);
        e.code = code;
        Some(e)
    }
}

/// 终端输出只留前 TERM_KEEP 个字节（按字符边界截）
fn keep(mut t: String) -> String {
    if t.len() > TERM_KEEP {
        let mut end = TERM_KEEP;
        while !t.is_char_boundary(end) {
            end -= 1;
        }
        t.truncate(end);
    }
    t
}

/* ------------------------------ 共用 ------------------------------ */

/// agent 说的一段话（空的不记）
fn message(t: &str) -> Option<RunEvent> {
    let t = t.trim();
    (!t.is_empty()).then(|| RunEvent::text("message", clip(t, TEXT_MAX)))
}

/// 工具的结果：前 OUT_LINES 行，多出来的写一行「…（共 N 行）」
fn done(id: &str, body: &str, ok: bool) -> RunEvent {
    RunEvent { kind: "tool".into(), call: some_text(id), out: Some(summary(body)), ok: Some(ok), ..Default::default() }
}

/// 结果摘要：去掉首尾空行，留前 OUT_LINES 行、最多 OUT_MAX 个字符
fn summary(body: &str) -> String {
    let lines: Vec<&str> = body.trim_matches('\n').lines().map(str::trim_end).collect();
    let mut out = lines.iter().take(OUT_LINES).copied().collect::<Vec<_>>().join("\n");
    if lines.len() > OUT_LINES {
        out += &format!("\n…（共 {} 行）", lines.len());
    }
    clip(&out, OUT_MAX)
}

fn some_text(x: &str) -> Option<String> {
    let x = x.trim();
    (!x.is_empty()).then(|| x.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use board_core::runlog::merge_calls;

    fn parse(text: &str) -> Vec<RunEvent> {
        let mut p = Parser::new(Stream::Acp);
        let all: Vec<RunEvent> = text.lines().flat_map(|l| p.feed(l, 1)).collect();
        merge_calls(all)
    }

    fn kinds(evs: &[RunEvent]) -> Vec<&str> {
        evs.iter().map(|e| e.kind.as_str()).collect()
    }

    const ACP: &str = include_str!("../tests/fixtures/acp-session.jsonl");
    const CODEX_TERMINAL: &str = include_str!("../tests/fixtures/codex-terminal.jsonl");
    /// ACP v2 的一个会话（照 claude-agent-acp 0.107.3 的帧写的）：工具调用分几帧打补丁，命令的输出、退出码在终端里，
    /// 这一轮在 idle 结束、带着 tokens；后台跑完 agent 自己又开了一轮
    const ACP_V2: &str = include_str!("../tests/fixtures/acp-v2-session.jsonl");

    /// v2（#461）：消息、工具（参数在第二帧、输出和退出码从终端来）、计划、两轮的结束（tokens、按轮算的费用）
    #[test]
    fn acp_v2_frames_become_events() {
        let evs = parse(ACP_V2);
        assert_eq!(kinds(&evs), ["message", "tool", "message", "result", "message", "result"], "{evs:#?}");
        assert_eq!(evs[0].text.as_deref(), Some("先跑测试。"));
        let t = &evs[1];
        assert_eq!((t.tool.as_deref(), t.args.as_deref(), t.ok, t.code), (Some("shell"), Some("npm test"), Some(false), Some(1)), "{t:#?}");
        assert_eq!(t.out.as_deref(), Some("line1\nline2\nFAIL src/a.test.ts"), "输出是终端最后的完整输出，\\r\\n 换成 \\n");
        assert_eq!(evs[2].text.as_deref(), Some("- [x] 跑测试\n- [ ] 交审（正在做）"));
        let first = &evs[3];
        assert_eq!((first.ok, first.tokens_in, first.tokens_out, first.cost), (Some(true), Some(1200), Some(80), Some(0.3)));
        assert_eq!(evs[4].text.as_deref(), Some("后台跑完了，交审。"), "agent 自己接着做的那一轮");
        let second = &evs[5];
        assert_eq!((second.ok, second.tokens_in, second.tokens_out), (Some(true), Some(300), Some(20)));
        assert!((second.cost.unwrap() - 0.2).abs() < 1e-9, "费用按轮算：{:?}", second.cost);
    }

    /// v2：出错结束的 idle 记一条 error 和一条失败的 result；只有终端片段、没有完整输出时用片段
    #[test]
    fn acp_v2_errors_and_streamed_output() {
        let mut p = Parser::new(Stream::Acp);
        let upd = |u: &str| format!(r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{u}}}}}"#);
        let mut evs = vec![];
        evs.extend(p.feed(&upd(r#"{"sessionUpdate":"tool_call_update","toolCallId":"t9","kind":"execute","title":"ls","rawInput":{"command":"ls"},"content":[{"type":"terminal","terminalId":"t9"}]}"#), 1));
        evs.extend(p.feed(&upd(r#"{"sessionUpdate":"terminal_output_chunk","terminalId":"t9","data":"YQ0K"}"#), 2));
        evs.extend(p.feed(&upd(r#"{"sessionUpdate":"terminal_output_chunk","terminalId":"t9","data":"Yg0K"}"#), 3));
        evs.extend(p.feed(&upd(r#"{"sessionUpdate":"tool_call_update","toolCallId":"t9","status":"completed"}"#), 4));
        evs.extend(p.feed(&upd(r#"{"sessionUpdate":"state_update","state":"idle","stopReason":"error","error":{"code":-32603,"message":"rate limited"}}"#), 5));
        let evs = merge_calls(evs);
        assert_eq!(kinds(&evs), ["tool", "error", "result"], "{evs:#?}");
        assert_eq!((evs[0].args.as_deref(), evs[0].out.as_deref(), evs[0].ok), (Some("ls"), Some("a\nb"), Some(true)));
        assert_eq!(evs[1].text.as_deref(), Some("rate limited"));
        assert_eq!((evs[2].ok, evs[2].text.as_deref()), (Some(false), Some("出错")));
        // 后台重报的 idle（不带 stopReason）、running：不记
        assert!(p.feed(&upd(r#"{"sessionUpdate":"state_update","state":"idle","background":[]}"#), 6).is_empty());
        assert!(p.feed(&upd(r#"{"sessionUpdate":"state_update","state":"running"}"#), 7).is_empty());
    }

    #[test]
    fn acp_frames_become_events() {
        let evs = parse(ACP);
        assert_eq!(kinds(&evs), ["message", "tool", "tool", "message", "tool", "tool", "error", "message", "result", "text"], "{evs:#?}");
        assert_eq!(evs[0].text.as_deref(), Some("先领卡，再看代码。"));
        let show = &evs[1];
        assert_eq!((show.tool.as_deref(), show.args.as_deref(), show.out.as_deref(), show.ok), (Some("shell"), Some("board show 211 --json"), Some("{\"id\":211}"), Some(true)));
        let edit = &evs[2];
        assert_eq!((edit.tool.as_deref(), edit.args.as_deref(), edit.ok), (Some("edit"), Some("crates/board-cli/src/host.rs"), Some(false)));
        assert_eq!(evs[3].text.as_deref(), Some("- [x] 领卡\n- [ ] 改代码（正在做）\n- [ ] 交审"));
        let test = &evs[4];
        assert_eq!((test.args.as_deref(), test.ok, test.code), (Some("cargo test"), Some(false), Some(101)));
        assert!(test.out.as_deref().unwrap().starts_with("error[E0425]"));
        // 一帧里就带着结果的工具调用
        assert_eq!((evs[5].out.as_deref(), evs[5].ok), (Some("改了 docs/agents.md"), Some(true)));
        assert_eq!(evs[6].text.as_deref(), Some("Internal error（rate limited）"));
        let r = &evs[8];
        assert_eq!((r.ok, r.tokens_in, r.tokens_out, r.cost, r.text.as_deref()), (Some(true), Some(31000), Some(2000), Some(0.42), None));
        assert_eq!(evs[9].text.as_deref(), Some("[claude-code:unrecognized_model] {\"model\":\"x\"}"));
    }

    /// 续轮（#454）：一个会话几条 result，usage_update 的费用是累计值；每条只记这一轮的，加起来是整个会话的
    #[test]
    fn each_turn_records_only_its_own_cost() {
        let mut p = Parser::new(Stream::Acp);
        let usage = |c: f64| format!(r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"usage_update","used":10,"size":100,"cost":{{"amount":{c},"currency":"USD"}}}}}}}}"#);
        let end = |id: i64| format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"stopReason":"end_turn","usage":{{"inputTokens":100,"outputTokens":10}}}}}}"#);
        assert!(p.feed(&usage(0.42), 1).is_empty());
        let first = p.feed(&end(2), 2);
        assert_eq!((first[0].cost, first[0].tokens_in), (Some(0.42), Some(100)));
        p.feed(&usage(0.5), 3);
        let second = p.feed(&end(3), 4);
        assert!((second[0].cost.unwrap() - 0.08).abs() < 1e-9, "{:?}", second[0].cost);
        assert_eq!(second[0].tokens_in, Some(100), "tokens 是这一轮的，照原样记");
        // 这一轮没有新的 usage_update：不记费用
        assert_eq!(p.feed(&end(4), 5)[0].cost, None);
    }

    #[test]
    fn stopped_turns_are_not_ok() {
        let mut p = Parser::new(Stream::Acp);
        let e = p.feed(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"cancelled"}}"#, 5);
        assert_eq!((e[0].kind.as_str(), e[0].ok, e[0].text.as_deref(), e[0].at), ("result", Some(false), Some("已停止"), 5));
        assert!(e[0].stopped, "人停的：对话里画成「这一轮停止了」");
        assert!(p.feed(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#, 1).is_empty(), "board acp 自己发的请求不记");
        assert!(p.feed("   ", 1).is_empty());
    }

    #[test]
    fn codex_terminal_tool_keeps_output_and_exit_code() {
        let evs = parse(CODEX_TERMINAL);
        assert_eq!(kinds(&evs), ["tool"]);
        assert_eq!(
            (evs[0].tool.as_deref(), evs[0].args.as_deref(), evs[0].out.as_deref(), evs[0].ok, evs[0].code),
            (Some("shell"), Some("sh -c \"exit 7\""), Some("oops"), Some(false), Some(7))
        );
    }

    #[test]
    fn long_results_keep_the_first_lines() {
        let body: String = (0..50).map(|i| format!("第 {i} 行\n")).collect();
        let e = done("t", &body, true);
        let out = e.out.unwrap();
        assert_eq!(out.lines().count(), OUT_LINES + 1);
        assert!(out.ends_with("…（共 50 行）"));
        assert!(done("t", &"x".repeat(OUT_MAX * 2), true).out.unwrap().chars().count() <= OUT_MAX);
    }

    /// 长时间派发里几千次工具调用都已结束：调用字段、开始标记和终端缓存都要释放
    #[test]
    fn finished_tool_calls_release_parser_memory() {
        let mut p = Parser::new(Stream::Acp);
        let upd = |u: String| format!(r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{u}}}}}"#);
        for i in 0..10_000 {
            let id = format!("t{i}");
            p.feed(
                &upd(format!(
                    r#"{{"sessionUpdate":"tool_call_update","toolCallId":"{id}","kind":"execute","title":"ls","rawInput":{{"command":"ls"}},"content":[{{"type":"terminal","terminalId":"{id}"}}]}}"#
                )),
                i,
            );
            p.feed(&upd(format!(r#"{{"sessionUpdate":"terminal_output_chunk","terminalId":"{id}","data":""}}"#)), i);
            p.feed(&upd(format!(r#"{{"sessionUpdate":"tool_call_update","toolCallId":"{id}","status":"completed"}}"#)), i);
        }
        assert_eq!(p.calls.len(), 0);
        assert!(p.terms.is_empty());
    }
}
