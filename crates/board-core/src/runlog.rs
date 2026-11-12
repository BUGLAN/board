//! agent 运行过程的结构化事件（卡片 default/main#184，PRD §9.1「执行」）。
//!
//! `board host` 用各个 agent 的结构化输出（Claude Code 的 `stream-json`、Codex 的 `exec --json`、pi 的 `--mode json`、
//! dsh 的 `--json`）启动它，逐行解析成统一的事件，每 2 秒上传一次；服务端按派发保存，界面「会话」的「过程」按它画：
//!
//! | kind | 是什么 | 用到的字段 |
//! |---|---|---|
//! | `message` | agent 说的话（Markdown） | `text` |
//! | `tool` | 一次工具调用 | `tool` 工具名、`args` 参数摘要、`out` 结果摘要、`ok` 成功或失败（还没有结果时为空）、`code` 退出码、`ms` 用时 |
//! | `error` | 出错信息（接口重试、agent 报的错） | `text` |
//! | `result` | 结束：结果文字、用时、输入 / 输出 tokens、费用（agent 输出里有才记） | `text`、`ok`、`ms`、`turns`、`tokensIn`、`tokensOut`、`cost` |
//! | `text` | 解析不了的行（标准错误里的提示），原样保留 | `text` |
//!
//! - 工具调用和它的结果可能分两次上传：两条都带着 agent 给的调用标识 `call`，服务端把结果合进先到的那条，一次调用只占一行。
//! - 每个派发最多留 [`EVENTS_KEEP`] 条（多出来的删最早的，记下删了几条）；单条超过 [`EVENT_MAX`] 字节就截断，标上 `cut`；
//!   派发结束 30 天后只留 `result` 和最后 [`EVENTS_OLD_KEEP`] 条。

use serde::{Deserialize, Serialize};

/// 每个派发最多保留多少条事件
pub const EVENTS_KEEP: i64 = 2000;
/// 单条事件最多多少字节（JSON），超过就截断
pub const EVENT_MAX: usize = 4096;
/// 派发结束多久之后只留 `result` 和最后几条
pub const EVENTS_OLD_MS: i64 = 30 * 24 * 3600 * 1000;
/// 结束 30 天后留下最后几条
pub const EVENTS_OLD_KEEP: i64 = 50;
/// 异常退出的评论里带最后几个事件
pub const EVENTS_TAIL: usize = 10;

/// 一条事件。字段都可以没有，按 `kind` 用其中几项（见模块说明）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    /// 服务端给的序号（按先后）；host 上传时为 0
    #[serde(default, skip_serializing_if = "is_zero")]
    pub id: i64,
    /// 什么时候（毫秒）：host 读到这一行的时间
    #[serde(default, skip_serializing_if = "is_zero")]
    pub at: i64,
    /// `message` | `tool` | `error` | `result` | `text`
    pub kind: String,
    /// 工具调用的标识（agent 给的）：调用和结果分两次上传时据此合成一条。只在 host 和服务端之间用，界面拿不到
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    /// 工具：成功 / 失败（还没有结果时为空）；结束：成功 / 出错
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// 命令的退出码
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
    /// 用时（毫秒）：工具这一次、结束时整次运行
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<i64>,
    /// 结束时一共几轮
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<i64>,
    /// 费用（美元）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    /// 子 agent（Claude Code 的 Task 工具）里的事件
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sub: bool,
    /// 超过 4 KB 截断过
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// 事件的种类
pub const KINDS: [&str; 5] = ["message", "tool", "error", "result", "text"];

impl RunEvent {
    pub fn new(kind: &str) -> RunEvent {
        RunEvent { kind: kind.into(), ..Default::default() }
    }

    pub fn text(kind: &str, text: impl Into<String>) -> RunEvent {
        RunEvent { kind: kind.into(), text: Some(text.into()), ..Default::default() }
    }

    /// 序列化后的字节数
    pub fn size(&self) -> usize {
        serde_json::to_string(self).map(|s| s.len()).unwrap_or(0)
    }

    /// 截到 [`EVENT_MAX`] 字节以内：每次把最长的文字字段砍掉多出来的部分（末尾写 …），标上 `cut`
    pub fn fit(&mut self) {
        self.fit_to(EVENT_MAX);
    }

    pub fn fit_to(&mut self, max: usize) {
        for _ in 0..8 {
            let size = self.size();
            if size <= max {
                return;
            }
            let over = size - max;
            let fields: [&mut Option<String>; 5] = [&mut self.text, &mut self.out, &mut self.args, &mut self.tool, &mut self.call];
            let Some(f) = fields.into_iter().filter_map(|f| f.as_mut()).max_by_key(|s| s.len()) else { break };
            if f.is_empty() {
                break;
            }
            // JSON 里转义会让字符串变长：按多出来的字节再多砍一些
            let keep = f.len().saturating_sub(over + 16);
            let mut cut = keep.min(f.len());
            while cut > 0 && !f.is_char_boundary(cut) {
                cut -= 1;
            }
            f.truncate(cut);
            f.push('…');
            self.cut = true;
        }
        // 还是太大（字段很多、都不长）：只留种类和第一段文字的开头
        if self.size() > max {
            let text = self.text.take().or_else(|| self.out.take()).map(|t| t.chars().take(200).collect::<String>() + "…");
            *self = RunEvent { id: self.id, at: self.at, kind: self.kind.clone(), call: None, text, cut: true, ..Default::default() };
        }
    }

    /// 工具调用的结果合进先到的调用：结果、成败、退出码、用时补上；调用里没有的工具名、参数也补上
    pub fn merge(&mut self, o: &RunEvent) {
        fn fill<T: Clone>(a: &mut Option<T>, b: &Option<T>) {
            if b.is_some() {
                *a = b.clone();
            }
        }
        if self.tool.is_none() {
            self.tool = o.tool.clone();
        }
        if self.args.is_none() {
            self.args = o.args.clone();
        }
        fill(&mut self.out, &o.out);
        fill(&mut self.ok, &o.ok);
        fill(&mut self.code, &o.code);
        fill(&mut self.ms, &o.ms);
        fill(&mut self.text, &o.text);
        self.cut |= o.cut;
        self.sub |= o.sub;
    }

    /// 一行文字（异常退出评论里的「最后 10 个事件」）：工具名加结果摘要、agent 说的话
    pub fn line(&self) -> String {
        let one = |s: &str, n: usize| {
            let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
            clip(&flat, n)
        };
        let t = self.text.as_deref().unwrap_or_default();
        let pad = if self.sub { "  ┆ " } else { "" };
        let body = match self.kind.as_str() {
            "message" => format!("agent：{}", one(t, 300)),
            "tool" => {
                let mut s = format!("▸ {}", self.tool.as_deref().unwrap_or("工具"));
                if let Some(a) = self.args.as_deref().filter(|a| !a.is_empty()) {
                    s += &format!("  {}", one(a, 120));
                }
                match self.ok {
                    Some(ok) => {
                        let mark = if ok { "↳" } else { "✗" };
                        let out = self.out.as_deref().map(|o| one(o.lines().find(|l| !l.trim().is_empty()).unwrap_or_default(), 160)).unwrap_or_default();
                        let code = self.code.filter(|c| *c != 0).map(|c| format!("（退出码 {c}）")).unwrap_or_default();
                        let out = if out.is_empty() { if ok { "完成".to_string() } else { "出错".to_string() } } else { out };
                        s += &format!("  {mark} {out}{code}");
                    }
                    None => s += "  …（没有结果）",
                }
                s
            }
            "error" => format!("✗ {}", one(t, 300)),
            "result" => {
                let mut s = format!("■ 结束 · {}", if self.ok == Some(false) { "出错" } else { "成功" });
                if !t.trim().is_empty() {
                    s += &format!("：{}", one(t, 200));
                }
                s
            }
            _ => one(t, 300),
        };
        format!("{pad}{body}")
    }
}

/// 同一批里，工具的结果合进前面同一个调用（调用和结果在一次上传里时只占一条）
pub fn merge_calls(events: Vec<RunEvent>) -> Vec<RunEvent> {
    let mut out: Vec<RunEvent> = Vec::with_capacity(events.len());
    for e in events {
        if e.kind == "tool" {
            if let Some(call) = e.call.as_deref() {
                if let Some(prev) = out.iter_mut().rev().find(|p| p.kind == "tool" && p.call.as_deref() == Some(call)) {
                    prev.merge(&e);
                    continue;
                }
            }
        }
        out.push(e);
    }
    out
}

/// 截到 max 个字符，多出来的写 …
pub fn clip(x: &str, max: usize) -> String {
    if x.chars().count() <= max {
        x.to_string()
    } else {
        let mut s: String = x.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_events_are_cut_to_4kb_and_marked() {
        let mut e = RunEvent { kind: "tool".into(), tool: Some("Bash".into()), args: Some("ls".into()), out: Some("字".repeat(5000)), ok: Some(true), ..Default::default() };
        e.fit();
        assert!(e.size() <= EVENT_MAX, "{}", e.size());
        assert!(e.cut);
        assert!(e.out.as_deref().unwrap().ends_with('…'));
        assert_eq!(e.tool.as_deref(), Some("Bash"), "短的字段不动");
        // 要转义的内容（引号、换行）也压得下来
        let mut e = RunEvent::text("message", "\"\n".repeat(4000));
        e.fit();
        assert!(e.size() <= EVENT_MAX && e.cut);
        // 本来就小的不动
        let mut e = RunEvent::text("message", "好");
        e.fit();
        assert!(!e.cut);
        assert_eq!(e.text.as_deref(), Some("好"));
    }

    #[test]
    fn a_call_and_its_result_become_one_event() {
        let call = RunEvent { kind: "tool".into(), call: Some("t1".into()), tool: Some("Bash".into()), args: Some("cargo test".into()), ..Default::default() };
        let done = RunEvent { kind: "tool".into(), call: Some("t1".into()), out: Some("ok".into()), ok: Some(true), code: Some(0), ..Default::default() };
        let other = RunEvent::text("message", "好了");
        let out = merge_calls(vec![call, other.clone(), done]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].tool.as_deref(), Some("Bash"));
        assert_eq!(out[0].args.as_deref(), Some("cargo test"));
        assert_eq!(out[0].out.as_deref(), Some("ok"));
        assert_eq!(out[0].ok, Some(true));
        assert_eq!(out[1], other);
    }

    #[test]
    fn lines_say_what_the_agent_was_doing() {
        let t = RunEvent { kind: "tool".into(), tool: Some("Bash".into()), args: Some("cargo test".into()), out: Some("\nerror[E0425]: cannot find value\nmore".into()), ok: Some(false), code: Some(101), ..Default::default() };
        assert_eq!(t.line(), "▸ Bash  cargo test  ✗ error[E0425]: cannot find value（退出码 101）");
        let t = RunEvent { kind: "tool".into(), tool: Some("Read".into()), args: Some("a.rs".into()), ..Default::default() };
        assert_eq!(t.line(), "▸ Read  a.rs  …（没有结果）");
        assert_eq!(RunEvent::text("message", "先领卡，\n再看代码。").line(), "agent：先领卡， 再看代码。");
        assert_eq!(RunEvent::text("error", "rate_limit").line(), "✗ rate_limit");
        let r = RunEvent { kind: "result".into(), ok: Some(false), text: Some("Reached maximum number of turns".into()), ..Default::default() };
        assert_eq!(r.line(), "■ 结束 · 出错：Reached maximum number of turns");
    }
}
