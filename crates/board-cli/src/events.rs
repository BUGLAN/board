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
//!   `rawOutput` 里有退出码时记下。
//! - `result`：prompt 的回应（`stopReason`）。回应里带了 `usage`（输入 / 输出 tokens）时记下；费用取最后一次
//!   `usage_update` 里的 `cost`（累计值）。
//! - 出错的回应 → `error`。

use crate::logfmt::{self, clip, s, Stream};
use board_core::runlog::RunEvent;
use serde_json::Value;

/// agent 说的一段话最多多少个字符（服务端单条 4 KB 截断，这里先收一些）
const TEXT_MAX: usize = 3000;
/// 工具结果最多留几行
const OUT_LINES: usize = 20;
/// 工具结果最多多少个字符
const OUT_MAX: usize = 1500;

/// 一个派发的解析器：费用要记住最后一次 `usage_update`
pub(crate) struct Parser {
    kind: Stream,
    cost: Option<f64>,
}

impl Parser {
    pub(crate) fn new(kind: Stream) -> Parser {
        Parser { kind, cost: None }
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
                "tool_call" => {
                    let (name, arg) = logfmt::tool_of(u);
                    let mut out = vec![RunEvent { kind: "tool".into(), call: some_text(id), tool: Some(name), args: some_text(&arg), ..Default::default() }];
                    out.extend(tool_done(u, id));
                    out
                }
                "tool_call_update" => tool_done(u, id).into_iter().collect(),
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
        let u = &r["usage"];
        vec![RunEvent {
            kind: "result".into(),
            ok: Some(reason == "end_turn"),
            text: some_text(logfmt::stop_words(reason)),
            tokens_in: u["inputTokens"].as_i64(),
            tokens_out: u["outputTokens"].as_i64(),
            cost: self.cost.take(),
            ..Default::default()
        }]
    }
}

/// 工具做完了（completed / failed）时的结果事件
fn tool_done(u: &Value, id: &str) -> Option<RunEvent> {
    let ok = logfmt::finished(u)?;
    let mut e = done(id, &logfmt::tool_output(u), ok);
    e.code = logfmt::tool_code(u);
    Some(e)
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

    #[test]
    fn stopped_turns_are_not_ok() {
        let mut p = Parser::new(Stream::Acp);
        let e = p.feed(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"cancelled"}}"#, 5);
        assert_eq!((e[0].kind.as_str(), e[0].ok, e[0].text.as_deref(), e[0].at), ("result", Some(false), Some("已停止"), 5));
        assert!(p.feed(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#, 1).is_empty(), "board acp 自己发的请求不记");
        assert!(p.feed("   ", 1).is_empty());
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
}
