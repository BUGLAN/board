//! agent 的结构化输出 → 统一事件（卡片 default/main#184；事件的定义在 board-core 的 runlog.rs）。
//!
//! `board host` 逐行读派发日志，每个在跑的派发一个 [`Parser`]：JSON 行按 agent 的事件格式转成 `message`、`tool`、
//! `error`、`result`，不是 JSON 的行（标准错误里的提示）记成 `text`，认得出、但对人没用的事件（流式片段、状态变化、
//! 思考）不记。格式的出处和 logfmt.rs 一样；每种格式在 `tests/fixtures/` 里有一份样例，单元测试按它核对事件的种类和条数。
//!
//! - 工具调用和结果分成两条事件，带着同一个 `call`（agent 给的调用标识），上传前同一批里合成一条，跨批的由服务端合并。
//! - 参数只留摘要（命令、文件、搜索词，见 logfmt `tool_arg`）；结果留前 [`OUT_LINES`] 行、最多 [`OUT_MAX`] 个字符。
//! - `result`：Claude Code 的 `result` 带用时、轮数、tokens、费用；pi、Codex、dsh 没有汇总，按每条消息 / 每一步 / 每一轮
//!   报的用量加起来，在结束时（pi `agent_settled`、Codex `turn.completed`、dsh `final`）写进 `result`。

use crate::logfmt::{self, arr, clip, s, Stream};
use board_core::runlog::RunEvent;
use serde_json::Value;

/// agent 说的一段话最多多少个字符（服务端单条 4 KB 截断，这里先收一些）
const TEXT_MAX: usize = 3000;
/// 工具结果最多留几行
const OUT_LINES: usize = 20;
/// 工具结果最多多少个字符
const OUT_MAX: usize = 1500;

/// 一个派发的解析器：pi、Codex、dsh 的用量要跨行累加
pub(crate) struct Parser {
    kind: Stream,
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    cost: Option<f64>,
}

impl Parser {
    pub(crate) fn new(kind: Stream) -> Parser {
        Parser { kind, tokens_in: None, tokens_out: None, cost: None }
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
                Stream::Claude => claude(&v),
                Stream::Pi => self.pi(&v),
                Stream::Codex => self.codex(&v),
                Stream::Dsh => self.dsh(&v),
            },
            None => vec![RunEvent::text("text", clip(line.trim_end(), logfmt::LINE_MAX))],
        };
        for e in &mut out {
            e.at = at;
        }
        out
    }

    fn add_usage(&mut self, input: Option<i64>, output: Option<i64>, cost: Option<f64>) {
        if let Some(i) = input {
            self.tokens_in = Some(self.tokens_in.unwrap_or(0) + i);
        }
        if let Some(o) = output {
            self.tokens_out = Some(self.tokens_out.unwrap_or(0) + o);
        }
        if let Some(c) = cost {
            self.cost = Some(self.cost.unwrap_or(0.0) + c);
        }
    }

    /// 结束事件：带上从上一个结束事件以来累加的用量（会话头部把各个结束事件的用量加起来，不重复算）
    fn result(&mut self, ok: bool, text: &str) -> RunEvent {
        RunEvent {
            kind: "result".into(),
            ok: Some(ok),
            text: some_text(&clip(text, TEXT_MAX)),
            tokens_in: self.tokens_in.take(),
            tokens_out: self.tokens_out.take(),
            cost: self.cost.take(),
            ..Default::default()
        }
    }

    /* ------------------------------ pi ------------------------------ */

    fn pi(&mut self, v: &Value) -> Vec<RunEvent> {
        let mut out = vec![];
        match s(&v["type"]) {
            "message_end" => {
                let m = &v["message"];
                if s(&m["role"]) != "assistant" {
                    return out;
                }
                // 每条消息的用量：input + cacheRead + cacheWrite 算输入
                let u = &m["usage"];
                let input = u["input"].as_i64().map(|i| i + u["cacheRead"].as_i64().unwrap_or(0) + u["cacheWrite"].as_i64().unwrap_or(0));
                self.add_usage(input, u["output"].as_i64(), u["cost"]["total"].as_f64());
                for c in arr(&m["content"]) {
                    if s(&c["type"]) == "text" {
                        out.extend(message(s(&c["text"])));
                    }
                }
                match s(&m["stopReason"]) {
                    "error" => out.push(RunEvent::text("error", clip(s(&m["errorMessage"]), TEXT_MAX))),
                    "aborted" => out.push(RunEvent::text("error", "已中止")),
                    _ => {}
                }
            }
            "tool_execution_start" => out.push(call(s(&v["toolCallId"]), s(&v["toolName"]), &v["args"])),
            "tool_execution_end" => {
                let r = &v["result"];
                let body = match &r["content"] {
                    Value::Array(_) => logfmt::tool_result_text(&r["content"]),
                    Value::String(x) => x.clone(),
                    _ => String::new(),
                };
                let mut e = done(s(&v["toolCallId"]), &body, v["isError"].as_bool() != Some(true));
                e.tool = some_text(s(&v["toolName"]));
                e.ms = v["durationMs"].as_i64();
                out.push(e);
            }
            "auto_retry_start" => {
                let mut t = format!("重试 第 {}/{} 次", logfmt::n(&v["attempt"]), logfmt::n(&v["maxAttempts"]));
                if !s(&v["errorMessage"]).is_empty() {
                    t += &format!("：{}", s(&v["errorMessage"]));
                }
                out.push(RunEvent::text("error", clip(&t, TEXT_MAX)));
            }
            "auto_retry_end" if v["success"].as_bool() == Some(false) => {
                out.push(RunEvent::text("error", clip(&format!("重试失败：{}", s(&v["finalError"])), TEXT_MAX)))
            }
            "compaction_start" => out.push(RunEvent::text("text", format!("● 压缩上下文（{}）", s(&v["reason"])))),
            "compaction_end" if !s(&v["errorMessage"]).is_empty() => {
                out.push(RunEvent::text("error", clip(&format!("压缩失败：{}", s(&v["errorMessage"])), TEXT_MAX)))
            }
            "agent_settled" => out.push(self.result(v["aborted"].as_bool() != Some(true), if v["aborted"].as_bool() == Some(true) { "已中止" } else { "" })),
            _ => {}
        }
        out
    }

    /* ------------------------------ Codex ------------------------------ */

    fn codex(&mut self, v: &Value) -> Vec<RunEvent> {
        let mut out = vec![];
        let item = &v["item"];
        let id = s(&item["id"]);
        match (s(&v["type"]), s(&item["type"])) {
            ("item.started", "command_execution") => out.push(call(id, "shell", &json_cmd(&logfmt::codex_command(s(&item["command"]))))),
            ("item.started", "mcp_tool_call") => out.push(call(id, &logfmt::codex_mcp_name(item), &item["arguments"])),
            ("item.completed", "command_execution") => {
                let code = item["exit_code"].as_i64();
                let ok = s(&item["status"]) == "completed" && code.map_or(true, |c| c == 0);
                let mut e = done(id, s(&item["aggregated_output"]), ok);
                e.tool = Some("shell".into());
                e.args = Some(clip(&logfmt::codex_command(s(&item["command"])), logfmt::LINE_MAX));
                e.code = code;
                out.push(e);
            }
            ("item.completed", "mcp_tool_call") => {
                let ok = s(&item["status"]) != "failed";
                let body = if ok { logfmt::json_text(&item["result"]) } else { logfmt::codex_mcp_error(item) };
                let mut e = done(id, &body, ok);
                e.tool = Some(logfmt::codex_mcp_name(item));
                e.args = some_text(&logfmt::tool_arg("", &item["arguments"]));
                out.push(e);
            }
            ("item.completed", "file_change") => {
                let mut e = call(id, "apply_patch", &json_cmd(&logfmt::codex_changes(item)));
                e.ok = Some(s(&item["status"]) != "failed");
                out.push(e);
            }
            ("item.completed", "web_search") => {
                let mut e = call(id, "web_search", &json_cmd(s(&item["query"])));
                e.ok = Some(true);
                out.push(e);
            }
            ("item.completed", "todo_list") => {
                let mut e = call(id, "todo", &json_cmd(&logfmt::codex_todos(item)));
                e.ok = Some(true);
                out.push(e);
            }
            ("item.completed", "agent_message") => out.extend(message(s(&item["text"]))),
            ("item.completed", "error") => out.push(RunEvent::text("error", clip(s(&item["message"]), TEXT_MAX))),
            ("turn.completed", _) => {
                // input_tokens 已经含了 cached_input_tokens
                let u = &v["usage"];
                self.add_usage(u["input_tokens"].as_i64(), u["output_tokens"].as_i64(), None);
                out.push(self.result(true, ""));
            }
            ("turn.failed", _) => {
                out.push(RunEvent::text("error", clip(s(&v["error"]["message"]), TEXT_MAX)));
                out.push(self.result(false, ""));
            }
            ("error", _) => out.push(RunEvent::text("error", clip(s(&v["message"]), TEXT_MAX))),
            _ => {}
        }
        out
    }

    /* ------------------------------ dsh ------------------------------ */

    fn dsh(&mut self, v: &Value) -> Vec<RunEvent> {
        let mut out = vec![];
        match s(&v["type"]) {
            "text" => out.extend(message(s(&v["text"]))),
            "tool_call" => out.push(call(s(&v["callId"]), s(&v["tool"]), &logfmt::dsh_input(&v["input"]))),
            "tool_result" => {
                let mut body = s(&v["result"]).to_string();
                if body.trim().is_empty() {
                    body = logfmt::dsh_error(&v["error"]);
                }
                out.push(done(s(&v["callId"]), &body, s(&v["status"]) != "error"));
            }
            "status" => match s(&v["phase"]) {
                // 每一步的用量（重试的也算在里面）：inputTokens 加缓存命中、写入算输入
                "step_end" => {
                    let u = &v["usage"];
                    let input = u["inputTokens"].as_i64().map(|i| i + u["cacheReadTokens"].as_i64().unwrap_or(0) + u["cacheWriteTokens"].as_i64().unwrap_or(0));
                    self.add_usage(input, u["outputTokens"].as_i64(), None);
                }
                "turn_end" if s(&v["reason"]["kind"]) == "error" => {
                    out.push(RunEvent::text("error", clip(&logfmt::dsh_error(&v["reason"]["error"]), TEXT_MAX)))
                }
                _ => {}
            },
            "final" => {
                let kind = s(&v["reason"]["kind"]);
                out.push(self.result(kind.is_empty() || kind == "completed", s(&v["text"])));
            }
            "error" => out.push(RunEvent::text("error", clip(s(&v["message"]), TEXT_MAX))),
            _ => {}
        }
        out
    }
}

/* ------------------------------ Claude Code ------------------------------ */

fn claude(v: &Value) -> Vec<RunEvent> {
    let mut out = vec![];
    match (s(&v["type"]), s(&v["subtype"])) {
        ("system", "api_retry") => {
            let mut t = format!("接口重试 第 {}/{} 次", logfmt::n(&v["attempt"]), logfmt::n(&v["max_retries"]));
            let why = [s(&v["error"]).to_string(), v["error_status"].as_i64().map(|c| c.to_string()).unwrap_or_default()]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !why.is_empty() {
                t += &format!("：{why}");
            }
            out.push(RunEvent::text("error", t));
        }
        ("system", "compact_boundary") => {
            let m = &v["compact_metadata"];
            let how = if s(&m["trigger"]) == "manual" { "手动" } else { "自动" };
            out.push(RunEvent::text("text", format!("● 压缩上下文（{how}，压缩前 {} tokens）", logfmt::n(&m["pre_tokens"]))));
        }
        ("assistant", _) => {
            if let Some(e) = v["error"].as_str() {
                out.push(RunEvent::text("error", clip(e, TEXT_MAX)));
            }
            for c in arr(&v["message"]["content"]) {
                match s(&c["type"]) {
                    "text" => out.extend(message(s(&c["text"]))),
                    "tool_use" => out.push(call(s(&c["id"]), s(&c["name"]), &c["input"])),
                    _ => {}
                }
            }
        }
        ("user", _) => {
            for c in arr(&v["message"]["content"]) {
                if s(&c["type"]) == "tool_result" {
                    out.push(done(s(&c["tool_use_id"]), &logfmt::tool_result_text(&c["content"]), c["is_error"].as_bool() != Some(true)));
                }
            }
        }
        ("result", sub_type) => {
            let ok = sub_type == "success" && v["is_error"].as_bool() != Some(true);
            let mut text = s(&v["result"]).to_string();
            for e in arr(&v["errors"]).iter().filter_map(Value::as_str) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text += e;
            }
            // usage 是整次运行的：输入算上缓存的读写
            let u = &v["usage"];
            let tokens_in = u["input_tokens"]
                .as_i64()
                .map(|i| i + u["cache_creation_input_tokens"].as_i64().unwrap_or(0) + u["cache_read_input_tokens"].as_i64().unwrap_or(0));
            out.push(RunEvent {
                kind: "result".into(),
                ok: Some(ok),
                text: some_text(&clip(&text, TEXT_MAX)),
                ms: v["duration_ms"].as_i64(),
                turns: v["num_turns"].as_i64(),
                tokens_in,
                tokens_out: u["output_tokens"].as_i64(),
                cost: v["total_cost_usd"].as_f64(),
                ..Default::default()
            });
        }
        _ => {}
    }
    // 子 agent（Task 工具）里的事件
    if !v["parent_tool_use_id"].is_null() {
        for e in &mut out {
            e.sub = true;
        }
    }
    out
}

/* ------------------------------ 共用 ------------------------------ */

/// agent 说的一段话（空的不记）
fn message(t: &str) -> Option<RunEvent> {
    let t = t.trim();
    (!t.is_empty()).then(|| RunEvent::text("message", clip(t, TEXT_MAX)))
}

/// 一次工具调用（还没有结果）
fn call(id: &str, name: &str, input: &Value) -> RunEvent {
    RunEvent {
        kind: "tool".into(),
        call: some_text(id),
        tool: Some(if name.is_empty() { "工具".into() } else { name.to_string() }),
        args: some_text(&logfmt::tool_arg(name, input)),
        ..Default::default()
    }
}

/// 工具的结果：前 OUT_LINES 行，多出来的写一行「…（共 N 行）」
fn done(id: &str, body: &str, ok: bool) -> RunEvent {
    RunEvent { kind: "tool".into(), call: some_text(id), out: Some(summary(body)), ok: Some(ok), ..Default::default() }
}

/// 结果摘要：去掉首尾空行，留前 OUT_LINES 行、最多 OUT_MAX 个字符
fn summary(body: &str) -> String {
    let body = body.replace("<tool_use_error>", "").replace("</tool_use_error>", "");
    let lines: Vec<&str> = body.trim_matches('\n').lines().map(str::trim_end).collect();
    let mut out = lines.iter().take(OUT_LINES).copied().collect::<Vec<_>>().join("\n");
    if lines.len() > OUT_LINES {
        out += &format!("\n…（共 {} 行）", lines.len());
    }
    clip(&out, OUT_MAX)
}

fn json_cmd(x: &str) -> Value {
    serde_json::json!({ "command": x })
}

fn some_text(x: &str) -> Option<String> {
    let x = x.trim();
    (!x.is_empty()).then(|| x.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use board_core::runlog::merge_calls;

    fn parse(kind: Stream, text: &str) -> Vec<RunEvent> {
        let mut p = Parser::new(kind);
        let all: Vec<RunEvent> = text.lines().flat_map(|l| p.feed(l, 1)).collect();
        merge_calls(all)
    }

    fn kinds(evs: &[RunEvent]) -> Vec<&str> {
        evs.iter().map(|e| e.kind.as_str()).collect()
    }

    const CLAUDE: &str = include_str!("../tests/fixtures/claude-stream.jsonl");
    const PI: &str = include_str!("../tests/fixtures/pi-events.jsonl");
    const CODEX: &str = include_str!("../tests/fixtures/codex-events.jsonl");
    const DSH: &str = include_str!("../tests/fixtures/dsh-events.jsonl");

    #[test]
    fn claude_stream_becomes_events() {
        let evs = parse(Stream::Claude, CLAUDE);
        assert_eq!(
            kinds(&evs),
            ["text", "message", "tool", "tool", "tool", "tool", "tool", "tool", "tool", "error", "text", "error", "result", "result"],
            "{evs:#?}"
        );
        assert!(evs.iter().all(|e| e.at == 1));
        assert_eq!(evs[0].text.as_deref(), Some("[claude-code:unrecognized_model] {\"model\":\"group/4.1f-routing-group[1m]\",\"query_source\":\"sdk\"}"));
        assert_eq!(evs[1].text.as_deref(), Some("先领卡、读描述。"));
        // 调用和结果合成一条
        let bash = &evs[2];
        assert_eq!((bash.tool.as_deref(), bash.args.as_deref(), bash.ok), (Some("Bash"), Some("board show 211 --json"), Some(true)));
        assert_eq!(bash.out.as_deref(), Some("{\n  \"id\": 211\n}"));
        let edit = evs.iter().find(|e| e.tool.as_deref() == Some("Edit")).unwrap();
        assert_eq!((edit.ok, edit.out.as_deref()), (Some(false), Some("String to replace not found in file.")));
        let todo = evs.iter().find(|e| e.tool.as_deref() == Some("TodoWrite")).unwrap();
        assert_eq!(todo.args.as_deref(), Some("2 项 · 正在做：排查日志"));
        // 子 agent 里的调用：标上 sub，还没有结果
        let sub = evs.iter().find(|e| e.sub).unwrap();
        assert_eq!((sub.tool.as_deref(), sub.args.as_deref(), sub.ok), (Some("Read"), Some("sysinfo/src/lib.rs"), None));
        assert_eq!(evs[9].text.as_deref(), Some("接口重试 第 1/10 次：rate_limit 429"));
        assert_eq!(evs[11].text.as_deref(), Some("max_output_tokens"));
        let r = &evs[12];
        assert_eq!((r.ok, r.ms, r.turns, r.cost), (Some(true), Some(1_017_000), Some(42), Some(3.1234)));
        assert_eq!((r.tokens_in, r.tokens_out), (Some(1 + 200 + 30_000), Some(2)), "输入算上缓存的读写");
        assert_eq!(r.text.as_deref(), Some("删掉了状态块右边缘悬浮时出现的竖条。\n\n提交：a5ea001"));
        let r = &evs[13];
        assert_eq!((r.ok, r.text.as_deref()), (Some(false), Some("Reached maximum number of turns")));
    }

    #[test]
    fn pi_events_become_events() {
        let evs = parse(Stream::Pi, PI);
        assert_eq!(kinds(&evs), ["message", "tool", "tool", "tool", "error", "error", "text", "error", "result", "result"], "{evs:#?}");
        let bash = &evs[1];
        assert_eq!((bash.tool.as_deref(), bash.args.as_deref(), bash.out.as_deref(), bash.ok, bash.ms), (Some("bash"), Some("board show 178 --json"), Some("{\"id\":178}"), Some(true), Some(1234)));
        assert_eq!(evs[3].ok, Some(false));
        assert_eq!(evs[4].text.as_deref(), Some("重试 第 1/3 次：529 overloaded"));
        assert_eq!(evs[7].text.as_deref(), Some("provider error"));
        // 用量按每条消息累加（样例里第一条消息报了用量）
        let r = &evs[8];
        assert_eq!((r.ok, r.tokens_in, r.tokens_out, r.cost), (Some(true), Some(150), Some(20), Some(0.0123)));
        assert_eq!((evs[9].ok, evs[9].text.as_deref(), evs[9].tokens_in), (Some(false), Some("已中止"), None), "用量只算一次");
    }

    #[test]
    fn codex_events_become_events() {
        let evs = parse(Stream::Codex, CODEX);
        assert_eq!(
            kinds(&evs),
            ["text", "tool", "message", "tool", "tool", "tool", "tool", "tool", "error", "message", "result", "error", "result", "error"],
            "{evs:#?}"
        );
        assert_eq!(evs[0].text.as_deref(), Some("Reading prompt from stdin..."));
        let show = &evs[1];
        assert_eq!((show.tool.as_deref(), show.args.as_deref(), show.ok, show.code), (Some("shell"), Some("board show 184 --json"), Some(true), Some(0)));
        let test = &evs[3];
        assert_eq!((test.args.as_deref(), test.ok, test.code, test.out.as_deref()), (Some("cargo test"), Some(false), Some(101), Some("error[E0425]: cannot find value `x`")));
        assert_eq!(evs[4].args.as_deref(), Some("改了 2 个文件：crates/board-cli/src/host.rs、docs/agents.md"));
        let mcp = &evs[5];
        assert_eq!((mcp.tool.as_deref(), mcp.args.as_deref(), mcp.out.as_deref(), mcp.ok), (Some("docs/search"), Some("exec json"), Some("found 2 pages"), Some(true)));
        assert_eq!((evs[6].tool.as_deref(), evs[6].args.as_deref()), (Some("web_search"), Some("codex exec json events")));
        assert_eq!((evs[7].tool.as_deref(), evs[7].args.as_deref()), (Some("todo"), Some("2 项 · 完成 2 项")));
        let r = &evs[10];
        assert_eq!((r.ok, r.tokens_in, r.tokens_out, r.cost), (Some(true), Some(24763), Some(122), None), "Codex 不报费用");
        assert_eq!(evs[12].ok, Some(false));
    }

    #[test]
    fn dsh_events_become_events() {
        let evs = parse(Stream::Dsh, DSH);
        assert_eq!(kinds(&evs), ["text", "message", "tool", "tool", "tool", "result", "error", "result", "error"], "{evs:#?}");
        let bash = &evs[2];
        assert_eq!((bash.tool.as_deref(), bash.args.as_deref(), bash.out.as_deref(), bash.ok), (Some("bash"), Some("board show 184 --json"), Some("{\"id\":184}"), Some(true)));
        assert_eq!((evs[3].ok, evs[3].out.as_deref()), (Some(false), Some("ENOENT: no such file")));
        assert_eq!(evs[4].args.as_deref(), Some("not json"), "参数不是 JSON 时原样显示");
        let r = &evs[5];
        assert_eq!((r.ok, r.tokens_in, r.tokens_out, r.text.as_deref()), (Some(true), Some(3700), Some(120), Some("改好了。\n\n提交：a5ea001")));
        assert_eq!(evs[6].text.as_deref(), Some("provider_error: 429 rate limited"));
        assert_eq!(evs[7].ok, Some(false));
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
