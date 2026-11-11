//! 派发日志的转写（卡片 default/main#211）。
//!
//! Claude Code 的 `-p`、pi 的 `-p` 都只在结束时把最终回答写到标准输出，运行中日志一直是空的，人看不出 agent 在干什么。
//! 内置模板因此改用它们的事件流：Claude Code `--output-format stream-json --verbose`、pi `--mode json`，运行中每一步都写
//! 一行 JSON（JSONL）。本机的日志文件 `~/.board/host/logs/<派发 ID>.log` 里存原样的事件；上传到服务端（界面「日志」）和
//! 异常退出评论里的「日志最后 20 行」之前，用这里的函数转成一行一行能读的内容：
//!
//! ```text
//! ● 开始 · Claude Code 2.1.311 · claude-opus-5-5 · /Users/me/repo/board
//! 先领卡，再看代码。
//! ▸ Bash  board show 211 --json
//!   ↳ {"activity": [  （共 120 行）
//! ▸ Edit  crates/board-cli/src/host.rs
//!   ✗ String to replace not found in file.
//! ■ 结束 · 成功 · 用时 16 分 57 秒 · 42 轮 · $3.12
//! ```
//!
//! - 不是 JSON 的行（标准错误里的提示、shell 的警告，例如 `[claude-code:unrecognized_model] …`）原样保留；
//!   配置文件里把命令换回文本输出的，整份日志都原样保留。
//! - 认得出、但对人没用的事件（流式片段、状态变化、钩子）不显示。
//! - 每一行最长 [`LINE_MAX`] 个字符，一段话最多 [`TEXT_LINES`] 行，工具结果只写第一行和总行数。
//!
//! 事件格式的出处：Claude Code 是 Claude Agent SDK 的 `SDKMessage`（`sdk.d.ts`：`system`/`init`、`assistant`、
//! `user` 里的 `tool_result`、`result`、`system`/`api_retry` …）；pi 是它文档的 `docs/json.md`、`docs/message-types.md`
//! （`session`、`message_end`、`tool_execution_start`/`end`、`auto_retry_*`、`compaction_*`、`agent_settled` …）。

use serde_json::Value;

/// 一行最长多少个字符（多出来的截掉，末尾写 …）
pub(crate) const LINE_MAX: usize = 400;
/// agent 说的一段话最多保留几行
const TEXT_LINES: usize = 40;
/// 工具参数、结果摘要最长多少个字符
const BRIEF: usize = 160;

/// 哪一种事件流
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    /// Claude Code `--output-format stream-json --verbose`
    Claude,
    /// pi `--mode json`
    Pi,
}

/// 这个 agent 的日志按哪种事件流转写：内置的 Claude Code、pi；其他（dsh、Codex 运行中本来就写过程）不转
pub(crate) fn stream_of(agent: &str) -> Option<Stream> {
    match agent {
        "claude" => Some(Stream::Claude),
        "pi" => Some(Stream::Pi),
        _ => None,
    }
}

/// 把日志里的原始行转成上传的行。kind 为空、或这一行不是 JSON 对象时原样保留
pub(crate) fn render(kind: Option<Stream>, raw: Vec<String>) -> Vec<String> {
    let Some(kind) = kind else { return raw };
    let mut out = vec![];
    for line in raw {
        let t = line.trim();
        let v = if t.starts_with('{') { serde_json::from_str::<Value>(t).ok().filter(Value::is_object) } else { None };
        match v {
            Some(v) => match kind {
                Stream::Claude => claude(&v, &mut out),
                Stream::Pi => pi(&v, &mut out),
            },
            None => out.push(line),
        }
    }
    out.into_iter().map(|l| clip(&l, LINE_MAX)).collect()
}

/* ------------------------------ Claude Code ------------------------------ */

fn claude(v: &Value, out: &mut Vec<String>) {
    // 子 agent（Task 工具）里的事件：缩进一格，和主线分开
    let sub = !v["parent_tool_use_id"].is_null();
    let pad = if sub { "  ┆ " } else { "" };
    let mut push = |s: String| out.push(format!("{pad}{s}"));
    match (s(&v["type"]), s(&v["subtype"])) {
        ("system", "init") => {
            let ver = s(&v["claude_code_version"]);
            let mut head = "● 开始 · Claude Code".to_string();
            if !ver.is_empty() {
                head += &format!(" {ver}");
            }
            for x in [s(&v["model"]), s(&v["cwd"])] {
                if !x.is_empty() {
                    head += &format!(" · {x}");
                }
            }
            push(head);
        }
        ("system", "api_retry") => {
            let mut t = format!("↻ 接口重试 第 {}/{} 次", n(&v["attempt"]), n(&v["max_retries"]));
            let why = [s(&v["error"]).to_string(), v["error_status"].as_i64().map(|c| c.to_string()).unwrap_or_default()]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !why.is_empty() {
                t += &format!("：{why}");
            }
            push(t);
        }
        ("system", "compact_boundary") => {
            let m = &v["compact_metadata"];
            let how = if s(&m["trigger"]) == "manual" { "手动" } else { "自动" };
            push(format!("● 压缩上下文（{how}，压缩前 {} tokens）", n(&m["pre_tokens"])));
        }
        ("system", "task_started") => push(format!("▸ 子任务  {}", brief(s(&v["description"])))),
        ("assistant", _) => {
            if let Some(e) = v["error"].as_str() {
                push(format!("✗ 出错：{e}"));
            }
            for c in arr(&v["message"]["content"]) {
                match s(&c["type"]) {
                    "text" => text(s(&c["text"]), "", &mut push),
                    "thinking" => thinking(s(&c["thinking"]), &mut push),
                    "tool_use" => push(tool_line(s(&c["name"]), &c["input"])),
                    _ => {}
                }
            }
        }
        ("user", _) => {
            // 提示词、skill 内容这些人写的文字不显示，只显示工具结果
            for c in arr(&v["message"]["content"]) {
                if s(&c["type"]) == "tool_result" {
                    push(result_line(&tool_result_text(&c["content"]), c["is_error"].as_bool() == Some(true), None));
                }
            }
        }
        ("result", sub_type) => {
            let ok = sub_type == "success" && v["is_error"].as_bool() != Some(true);
            let mut t = format!("■ 结束 · {}", if ok { "成功".to_string() } else { format!("出错（{sub_type}）") });
            if let Some(ms) = v["duration_ms"].as_i64() {
                t += &format!(" · 用时 {}", dur(ms));
            }
            if let Some(k) = v["num_turns"].as_i64() {
                t += &format!(" · {k} 轮");
            }
            if let Some(c) = v["total_cost_usd"].as_f64() {
                t += &format!(" · ${c:.2}");
            }
            push(t);
            text(s(&v["result"]), "", &mut push);
            for e in arr(&v["errors"]) {
                if let Some(e) = e.as_str() {
                    push(format!("✗ {}", clip(e, BRIEF)));
                }
            }
        }
        _ => {}
    }
}

/// tool_result 的 content：字符串，或 `[{type: text, text}]`。Claude Code 把工具的报错包在 `<tool_use_error>` 里，去掉这层
fn tool_result_text(c: &Value) -> String {
    let t = match c {
        Value::String(x) => x.clone(),
        Value::Array(xs) => xs.iter().filter(|x| s(&x["type"]) == "text").map(|x| s(&x["text"])).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    };
    t.replace("<tool_use_error>", "").replace("</tool_use_error>", "")
}

/* ------------------------------ pi ------------------------------ */

fn pi(v: &Value, out: &mut Vec<String>) {
    let mut push = |x: String| out.push(x);
    match s(&v["type"]) {
        "session" => {
            let mut head = "● 开始 · pi".to_string();
            if !s(&v["cwd"]).is_empty() {
                head += &format!(" · {}", s(&v["cwd"]));
            }
            push(head);
        }
        "message_end" => {
            let m = &v["message"];
            if s(&m["role"]) != "assistant" {
                return;
            }
            for c in arr(&m["content"]) {
                match s(&c["type"]) {
                    "text" => text(s(&c["text"]), "", &mut push),
                    "thinking" => thinking(s(&c["thinking"]), &mut push),
                    // 工具调用看 tool_execution_start（带参数，执行前就写）
                    _ => {}
                }
            }
            match s(&m["stopReason"]) {
                "error" => push(format!("✗ 出错：{}", clip(s(&m["errorMessage"]), BRIEF))),
                "aborted" => push("✗ 已中止".into()),
                _ => {}
            }
        }
        "tool_execution_start" => push(tool_line(s(&v["toolName"]), &v["args"])),
        "tool_execution_end" => {
            let r = &v["result"];
            let body = match &r["content"] {
                Value::Array(_) => tool_result_text(&r["content"]),
                Value::String(x) => x.clone(),
                _ => String::new(),
            };
            push(result_line(&body, v["isError"].as_bool() == Some(true), v["durationMs"].as_i64()));
        }
        "auto_retry_start" => {
            let mut t = format!("↻ 重试 第 {}/{} 次", n(&v["attempt"]), n(&v["maxAttempts"]));
            if !s(&v["errorMessage"]).is_empty() {
                t += &format!("：{}", clip(s(&v["errorMessage"]), BRIEF));
            }
            push(t);
        }
        "auto_retry_end" if v["success"].as_bool() == Some(false) => push(format!("✗ 重试失败：{}", clip(s(&v["finalError"]), BRIEF))),
        "compaction_start" => push(format!("● 压缩上下文（{}）", s(&v["reason"]))),
        "compaction_end" if !s(&v["errorMessage"]).is_empty() => push(format!("✗ 压缩失败：{}", clip(s(&v["errorMessage"]), BRIEF))),
        "agent_settled" => push(if v["aborted"].as_bool() == Some(true) { "■ 结束 · 已中止".into() } else { "■ 结束".into() }),
        _ => {}
    }
}

/* ------------------------------ 共用 ------------------------------ */

/// 一次工具调用：`▸ Bash  git status`
fn tool_line(name: &str, input: &Value) -> String {
    let arg = tool_arg(name, input);
    if arg.is_empty() {
        format!("▸ {name}")
    } else {
        format!("▸ {name}  {arg}")
    }
}

/// 工具参数的摘要：命令、文件、搜索词这些最能说明在干什么的那一项；都没有时写紧凑的 JSON
fn tool_arg(name: &str, input: &Value) -> String {
    if name == "TodoWrite" {
        let todos = arr(&input["todos"]);
        let now = todos.iter().find(|t| s(&t["status"]) == "in_progress").map(|t| s(&t["content"])).unwrap_or_default();
        return if now.is_empty() { format!("{} 项", todos.len()) } else { format!("{} 项 · 正在做：{}", todos.len(), brief(now)) };
    }
    for k in ["command", "file_path", "pattern", "path", "url", "query", "description", "skill", "prompt"] {
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

/// 工具结果：`  ↳ 第一行  （共 N 行 · 3.2 秒）`；出错时 `  ✗ …`
fn result_line(body: &str, err: bool, ms: Option<i64>) -> String {
    let lines: Vec<&str> = body.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    let first = lines.first().map(|l| brief(l.trim())).unwrap_or_else(|| if err { "出错".into() } else { "（没有输出）".into() });
    let mut more = vec![];
    if lines.len() > 1 {
        more.push(format!("共 {} 行", lines.len()));
    }
    if let Some(ms) = ms.filter(|ms| *ms >= 1000) {
        more.push(format!("{:.1} 秒", ms as f64 / 1000.0));
    }
    let tail = if more.is_empty() { String::new() } else { format!("  （{}）", more.join(" · ")) };
    format!("  {} {first}{tail}", if err { "✗" } else { "↳" })
}

/// agent 说的一段话：去掉空行，最多 TEXT_LINES 行
fn text(t: &str, prefix: &str, push: &mut impl FnMut(String)) {
    let lines: Vec<&str> = t.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    for l in lines.iter().take(TEXT_LINES) {
        push(format!("{prefix}{l}"));
    }
    if lines.len() > TEXT_LINES {
        push(format!("{prefix}…（还有 {} 行）", lines.len() - TEXT_LINES));
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
fn brief(x: &str) -> String {
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
fn clip(x: &str, max: usize) -> String {
    if x.chars().count() <= max {
        x.to_string()
    } else {
        let mut s: String = x.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

/// 用时：12 秒、3 分 5 秒、1 小时 2 分
fn dur(ms: i64) -> String {
    let t = ms.max(0) / 1000;
    let (h, m, sec) = (t / 3600, (t % 3600) / 60, t % 60);
    if h > 0 {
        format!("{h} 小时 {m} 分")
    } else if m > 0 {
        format!("{m} 分 {sec} 秒")
    } else {
        format!("{sec} 秒")
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn n(v: &Value) -> String {
    v.as_i64().map(|x| x.to_string()).unwrap_or_else(|| "?".into())
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    /// Claude Code `--output-format stream-json --verbose` 的一次运行（按 SDKMessage 的格式写的样例）
    const CLAUDE: &str = include_str!("../tests/fixtures/claude-stream.jsonl");
    /// pi `--mode json` 的一次运行（按 docs/json.md 的格式写的样例）
    const PI: &str = include_str!("../tests/fixtures/pi-events.jsonl");

    #[test]
    fn claude_stream_becomes_readable_lines() {
        let out = render(Some(Stream::Claude), lines(CLAUDE));
        let want = vec![
            "[claude-code:unrecognized_model] {\"model\":\"group/4.1f-routing-group[1m]\",\"query_source\":\"sdk\"}",
            "● 开始 · Claude Code 2.1.311 · claude-opus-5-5 · /Users/me/repo/board",
            "（思考）先领卡，再看代码 …",
            "先领卡、读描述。",
            "▸ Bash  board show 211 --json",
            "  ↳ {  （共 3 行）",
            "▸ Read  crates/board-cli/src/host.rs",
            "  ↳ 1\t//! `board host`  （共 2 行）",
            "▸ TodoWrite  2 项 · 正在做：排查日志",
            "  ↳ Todos have been modified successfully.",
            "▸ Edit  crates/board-cli/src/host.rs",
            "  ✗ String to replace not found in file.",
            "▸ Grep  stream-json  （crates）",
            "  ↳ No matches found",
            "▸ Bash  cargo test -p board-cli \\ …",
            "  ↳ test result: ok. 3 passed  （共 2 行）",
            "▸ 子任务  查 sysinfo 的接口",
            "  ┆ ▸ Read  sysinfo/src/lib.rs",
            "↻ 接口重试 第 1/10 次：rate_limit 429",
            "● 压缩上下文（自动，压缩前 160000 tokens）",
            "✗ 出错：max_output_tokens",
            "■ 结束 · 成功 · 用时 16 分 57 秒 · 42 轮 · $3.12",
            "删掉了状态块右边缘悬浮时出现的竖条。",
            "提交：a5ea001",
            "■ 结束 · 出错（error_max_turns） · 用时 5 秒 · 3 轮",
            "✗ Reached maximum number of turns",
        ];
        assert_eq!(out, want);
    }

    #[test]
    fn pi_events_become_readable_lines() {
        let out = render(Some(Stream::Pi), lines(PI));
        let want = vec![
            "● 开始 · pi · /home/me/repo/board",
            "（思考）用户要我执行 #178 …",
            "先读卡片。",
            "▸ bash  board show 178 --json",
            "  ↳ {\"id\":178}  （1.2 秒）",
            "▸ read  web/src/components/stats/Overview.tsx",
            "  ↳ import { useState } from 'react';  （共 2 行）",
            "▸ edit  web/src/components/stats/Overview.tsx",
            "  ✗ Could not find the exact text",
            "↻ 重试 第 1/3 次：529 overloaded",
            "✗ 重试失败：529 overloaded",
            "● 压缩上下文（threshold）",
            "✗ 出错：provider error",
            "■ 结束",
            "■ 结束 · 已中止",
        ];
        assert_eq!(out, want);
    }

    #[test]
    fn other_agents_and_text_lines_stay_as_they_are() {
        let raw = lines("dsh: reasoning:\n{\"type\":\"system\",\"subtype\":\"init\"}\nplain");
        assert_eq!(render(None, raw.clone()), raw, "dsh、Codex、配置文件里的 agent 不转写");
        assert_eq!(stream_of("dsh"), None);
        assert_eq!(stream_of("codex"), None);
        // 配置文件把 Claude Code 换回文本输出：结果是普通文字，原样保留；坏掉的 JSON、JSON 数组也原样保留
        let txt = lines("删掉了竖条。\n{\"type\": \"assistant\", \"message\": \n[1, 2]");
        assert_eq!(render(Some(Stream::Claude), txt.clone()), txt);
    }

    #[test]
    fn long_lines_and_long_text_are_clipped() {
        let long = "字".repeat(LINE_MAX + 50);
        let ev = serde_json::json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": long }] }, "parent_tool_use_id": null });
        let out = render(Some(Stream::Claude), vec![ev.to_string()]);
        assert_eq!(out[0].chars().count(), LINE_MAX);
        assert!(out[0].ends_with('…'));
        let many: String = (0..60).map(|i| format!("第 {i} 行\n")).collect();
        let ev = serde_json::json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": many }] }, "parent_tool_use_id": null });
        let out = render(Some(Stream::Claude), vec![ev.to_string()]);
        assert_eq!(out.len(), TEXT_LINES + 1);
        assert_eq!(out.last().unwrap(), "…（还有 20 行）");
        // 非 JSON 的长行同样截断
        let out = render(Some(Stream::Pi), vec!["x".repeat(LINE_MAX * 2)]);
        assert_eq!(out[0].chars().count(), LINE_MAX);
    }
}
