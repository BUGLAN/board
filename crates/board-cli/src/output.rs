//! 输出：结果（文字 / JSON）、怎么打印；卡片、附件、名字的文字和 JSON 格式；终端上色

use crate::text::{pad, truncate, width};
use board_core::*;
use chrono::TimeZone;
use serde_json::{json, Value};

pub(crate) enum Output {
    Text(String),
    Json(Value),
    /// 照常打印，但退出码不是 0（`board doctor` 有检查没通过）
    Exit(i32, Box<Output>),
}

/// 打印结果并返回退出码。标准输出写不进去（管道读端已关）时不 panic：
/// 例如 board host 运行中输出被关掉，前面没写出去的内容还在缓冲里，这里再打印会再失败一次
pub(crate) fn emit(out: Output) -> i32 {
    use std::io::Write;
    match out {
        Output::Text(s) => {
            let mut o = std::io::stdout();
            let _ = write!(o, "{s}").and_then(|_| o.flush());
            0
        }
        Output::Json(v) => {
            let _ = writeln!(std::io::stdout(), "{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            0
        }
        Output::Exit(code, body) => {
            let _ = emit(*body);
            code
        }
    }
}

/* ------------------------------ 输出 ------------------------------ */

pub(crate) struct Fmt<'a> {
    pub(crate) p: &'a Paint,
    pub(crate) label_name: &'a dyn Fn(i64) -> String,
    pub(crate) label_priority: &'a dyn Fn(i64) -> bool,
    pub(crate) path: &'a str,
    /// 卡片的网址（服务端地址 + 路径）
    pub(crate) url_of: &'a dyn Fn(i64) -> String,
    pub(crate) now: i64,
    pub(crate) flow: &'a Flow,
}

pub(crate) fn iso(ms: Option<i64>) -> Value {
    match ms.and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single()) {
        Some(t) => Value::String(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)),
        None => Value::Null,
    }
}

/// 耗时：12:34 / 1:02:03
pub(crate) fn clock(ms: i64) -> String {
    let t = ms.max(0) / 1000;
    let (h, m, s) = (t / 3600, t % 3600 / 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

impl Fmt<'_> {
    /// 按状态的规则上色：可领取白、锁定黄、终点绿、错误节点红、未整理紫、其他蓝
    pub(crate) fn col(&self, key: &str, s: &str) -> String {
        let code = if key == INBOX {
            "35"
        } else if self.flow.is_error(key) {
            "31"
        } else if self.flow.is_lock(key) {
            "33"
        } else if self.flow.is_final(key) {
            "32"
        } else if self.flow.is_claim(key) {
            "37"
        } else {
            "34"
        };
        self.p.wrap(code, s)
    }

    fn labels(&self, c: &Card) -> Vec<String> {
        c.labels.iter().map(|l| (self.label_name)(*l)).filter(|n| !n.is_empty()).collect()
    }

    /// 卡片上的优先标签（不带 ID 领取时这张先被领取）
    fn priority_labels(&self, c: &Card) -> Vec<String> {
        c.labels.iter().filter(|l| (self.label_priority)(**l)).map(|l| (self.label_name)(*l)).filter(|n| !n.is_empty()).collect()
    }

    pub(crate) fn card(&self, c: &Card) -> Value {
        json!({
            "id": c.id,
            "column": c.column,
            "title": c.title,
            "holder": c.holder,
            "labels": self.labels(c),
            "priority_labels": self.priority_labels(c),
        })
    }

    pub(crate) fn full(&self, d: &CardDetail) -> Value {
        let c = &d.card;
        json!({
            "id": c.id,
            "board": self.path,
            "url": (self.url_of)(c.id),
            "column": c.column,
            "title": c.title,
            "body": c.body,
            "holder": c.holder,
            "claimed_at": iso(c.claimed_at),
            "beat_at": iso(c.beat_at),
            "run_started_at": iso(c.run_started_at),
            "work": {
                "total_ms": c.work_ms + c.run_started_at.map(|t| (self.now - t).max(0)).unwrap_or(0),
                "runs": c.runs,
                "lost": c.runs_lost,
                "beats": c.beats,
            },
            "labels": self.labels(c),
            "priority_labels": self.priority_labels(c),
            "split_from": c.split_from,
            "split_into": d.split_into,
            "attachments": d.attachments.iter().map(attachment_json).collect::<Vec<_>>(),
            "created_at": iso(Some(c.created_at)),
            "updated_at": iso(Some(c.updated_at)),
            "activity": d.activity.iter().map(|a| json!({
                "at": iso(Some(a.at)),
                "actor": a.actor.display(),
                "actor_kind": a.actor.kind,
                "type": a.kind,
                "action": a.action,
                "text": a.text,
            })).collect::<Vec<_>>(),
        })
    }

    /// 自己持有的卡片：追加「本次 12:34」
    pub(crate) fn renewed(&self, d: &CardDetail, me: &str) -> String {
        match (&d.card.holder, d.card.run_started_at) {
            (Some(h), Some(t)) if h == me => self.p.dim(&format!("  本次 {}", clock(self.now - t))),
            _ => String::new(),
        }
    }

    pub(crate) fn list(&self, cards: &[Card]) -> String {
        let p = self.p;
        if cards.is_empty() {
            return format!("{}\n", p.dim("（没有匹配的卡片）"));
        }
        let tw = cards.iter().map(|c| width(&c.title)).max().unwrap_or(0).min(56) + 2;
        let tags: Vec<String> = cards.iter().map(|c| self.tags(c)).collect();
        let lw = tags.iter().map(|t| width(t)).max().unwrap_or(0) + 2;
        let cw = cards.iter().map(|c| width(&c.column)).max().unwrap_or(0).max(6) + 2;
        let mut s = String::new();
        for (c, tags) in cards.iter().zip(&tags) {
            let holder = c.holder.as_deref().unwrap_or("");
            let mut line = format!("{}{}", p.cyan(&pad(&format!("#{}", c.id), 6)), self.col(&c.column, &pad(&c.column, cw)));
            let title = truncate(&c.title, 56);
            if tags.is_empty() && holder.is_empty() {
                line += &title;
            } else {
                line += &pad(&title, tw);
                if holder.is_empty() {
                    line += &p.magenta(tags);
                } else {
                    line += &format!("{}{}", p.magenta(&pad(tags, lw)), p.yellow(holder));
                }
            }
            s += &line;
            s.push('\n');
        }
        s
    }

    fn tags(&self, c: &Card) -> String {
        self.labels(c).iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ")
    }

    pub(crate) fn show(&self, d: &CardDetail) -> String {
        let p = self.p;
        let c = &d.card;
        let mut s = String::new();
        let row = |k: &str| p.dim(&pad(k, 10));
        s += &format!("{}  {}\n", p.cyan(&format!("#{}", c.id)), p.bold(&c.title));
        s += &format!("{}{}\n", row("网址"), (self.url_of)(c.id));
        s += &format!("{}{} {}\n", row("状态"), self.flow.name(&c.column), self.col(&c.column, &format!("({})", c.column)));
        if !c.labels.is_empty() {
            s += &format!("{}{}\n", row("标签"), p.magenta(&self.tags(c)));
        }
        if let Some(f) = c.split_from {
            s += &format!("{}{}\n", row("拆自"), p.cyan(&format!("#{f}")));
        }
        if !d.split_into.is_empty() {
            let ids: Vec<String> = d.split_into.iter().map(|i| format!("#{i}")).collect();
            s += &format!("{}{}\n", row("拆出"), p.cyan(&ids.join(" ")));
        }
        if let Some(h) = &c.holder {
            s += &format!("{}{}\n", row("持有者"), p.yellow(h));
            match c.run_started_at {
                Some(t) => s += &format!("{}{}{}\n", row("本次"), clock(self.now - t), p.dim(&format!("  第 {} 次，第一次心跳 {}", c.runs, hhmm(t)))),
                None => s += &format!("{}{}\n", row("心跳"), p.red("还没开始：领取后 50 秒内没收到心跳会被释放")),
            }
        }
        if c.runs > 0 {
            let total = c.work_ms + c.run_started_at.map(|t| (self.now - t).max(0)).unwrap_or(0);
            let lost = if c.runs_lost > 0 { format!("，其中 {} 次异常退出", c.runs_lost) } else { String::new() };
            s += &format!("{}{}{}\n", row("累计"), clock(total), p.dim(&format!("  共 {} 次{lost}，心跳 {} 次", c.runs, c.beats)));
        }
        if c.body.is_empty() {
            s += &format!("{}{}\n", row("描述"), p.dim("（空）"));
        } else {
            s += &format!("{}\n", p.dim("描述"));
            for l in c.body.lines() {
                s += &format!("  {l}\n");
            }
        }
        if !d.attachments.is_empty() {
            s += &format!("{}{}\n", p.dim("附件"), p.dim(&format!("  board files {} 下载到本地查看", c.id)));
            for f in &d.attachments {
                s += &format!("  {}  {}  {}\n", p.dim(&format!("{}", f.id)), f.name, p.dim(&human_size(f.size)));
            }
        }
        s += &format!("{}\n", p.dim("动态"));
        let aw = d.activity.iter().map(|a| width(a.actor.display())).max().unwrap_or(0).max(6) + 2;
        for a in &d.activity {
            let who = pad(a.actor.display(), aw);
            let who = match a.actor.kind {
                ActorKind::Agent => p.yellow(&who),
                ActorKind::Human => p.green(&who),
                ActorKind::System => p.red(&who),
            };
            let text = if a.kind == "comment" { format!("💬 {}", a.text) } else { p.dim(&a.text) };
            s += &format!("  {}  {}{}\n", p.dim(&hhmm(a.at)), who, text);
        }
        s
    }
}

/// `board check` / 不带 ID 的 `board beat` 的 JSON
pub(crate) fn agent_json(a: &AgentInfo) -> Value {
    json!({
        "name": a.name,
        "checked_at": iso(Some(a.checked_at)),
        "beat_at": iso(Some(a.beat_at)),
        "cards": a.cards.iter().map(|h| json!({
            "board": h.board,
            "id": h.card.id,
            "title": h.card.title,
            "column": h.card.column,
            "run_started_at": iso(h.card.run_started_at),
            "beats": h.card.beats,
        })).collect::<Vec<_>>(),
        // 名字由 Board 生成时才有：名字里的模型和这台机器、这个 agent 的设置是否一致（见 check --agent）
        "modeled": a.modeled,
    })
}

pub(crate) fn saved_json(f: &Attachment, to: &std::path::Path) -> Value {
    let mut v = attachment_json(f);
    v["path"] = json!(to.display().to_string());
    v
}

/* ------------------------------ 文本工具 ------------------------------ */

pub(crate) fn attachment_json(f: &Attachment) -> Value {
    json!({
        "id": f.id,
        "name": f.name,
        "mime": f.mime,
        "size": f.size,
        "image": f.image,
        "url": f.url,
        "created_at": iso(Some(f.created_at)),
        "created_by": f.created_by.display(),
    })
}

/// 状态规则的一行文字，例如「可领取 · 锁定」；`board flow` 用来算列宽和显示
pub(crate) fn rule_of(st: &Status) -> String {
    let mut rules = vec![];
    if st.claim {
        rules.push("可领取");
    }
    if st.lock {
        rules.push("锁定");
    }
    if st.fin {
        rules.push("终点");
    }
    if st.err {
        rules.push("错误节点");
    }
    rules.join(" · ")
}

pub(crate) fn human_size(n: i64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{:.1} MB", n as f64 / 1048576.0),
    }
}

pub(crate) struct Paint {
    pub(crate) on: bool,
}

impl Paint {
    fn wrap(&self, code: &str, s: &str) -> String {
        if self.on && !s.is_empty() { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
    }
    pub(crate) fn dim(&self, s: &str) -> String {
        self.wrap("2", s)
    }
    pub(crate) fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }
    pub(crate) fn red(&self, s: &str) -> String {
        self.wrap("31", s)
    }
    pub(crate) fn red_bold(&self, s: &str) -> String {
        self.wrap("1;31", s)
    }
    pub(crate) fn green(&self, s: &str) -> String {
        self.wrap("32", s)
    }
    pub(crate) fn yellow(&self, s: &str) -> String {
        self.wrap("33", s)
    }
    pub(crate) fn magenta(&self, s: &str) -> String {
        self.wrap("35", s)
    }
    pub(crate) fn cyan(&self, s: &str) -> String {
        self.wrap("36", s)
    }
}
