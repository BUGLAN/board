//! `board` —— agent 用的命令行（PRD §9）。
//!
//! 身份：`--as <名字>` 或环境变量 `BOARD_AS`；白板：`--board <项目>/<白板>` 或环境变量 `BOARD`。
//! 所有命令支持 `--json`。成功退出码 0；被拒绝退出码 1，并给出错误码和原因。
//! 数据：通过服务端读写，必须设置 `BOARD_URL` 和 `BOARD_TOKEN`（界面「设置 → Agent 接入」新建令牌），
//! 没有配置时直接报错。见 backend.rs。

mod backend;
mod doctor;

use backend::{env, Backend, BoardData};
use board_core::*;
use chrono::TimeZone;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use unicode_width::UnicodeWidthStr;

const VALUE_FLAGS: [&str; 9] = ["--as", "--board", "--column", "--search", "--label", "--body", "--title", "--comment", "--out"];
const BOOL_FLAGS: [&str; 5] = ["--json", "--top", "--help", "-h", "--final"];
const COMMANDS: [&str; 16] =
    ["help", "doctor", "boards", "flow", "ls", "show", "new", "edit", "move", "claim", "comment", "beat", "label", "labels", "attach", "files"];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let p = Paint { on: !json && std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() };
    match run(&args, &p) {
        Ok(out) => {
            let code = emit(out);
            if code != 0 {
                std::process::exit(code);
            }
        }
        Err(e) => {
            if json {
                let mut err = json!({ "code": e.code, "message": e.message });
                if let Some(d) = e.detail {
                    err["detail"] = d;
                }
                println!("{}", serde_json::to_string_pretty(&json!({ "error": err })).unwrap_or_default());
            } else {
                let pe = Paint { on: std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none() };
                eprintln!("{} {}  {}", pe.red("✗"), pe.red_bold(e.code.as_str()), e.message);
            }
            std::process::exit(1);
        }
    }
}

enum Output {
    Text(String),
    Json(Value),
    /// 照常打印，但退出码不是 0（`board doctor` 有检查没通过）
    Exit(i32, Box<Output>),
}

/// 打印结果并返回退出码
fn emit(out: Output) -> i32 {
    match out {
        Output::Text(s) => {
            print!("{s}");
            0
        }
        Output::Json(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            0
        }
        Output::Exit(code, body) => {
            let _ = emit(*body);
            code
        }
    }
}

/* ------------------------------ 参数 ------------------------------ */

struct Args {
    values: HashMap<String, String>,
    bools: HashSet<String>,
    pos: Vec<String>,
}

impl Args {
    fn parse(tokens: &[String]) -> Result<Args> {
        let mut a = Args { values: HashMap::new(), bools: HashSet::new(), pos: vec![] };
        let mut i = 0;
        while i < tokens.len() {
            let t = &tokens[i];
            if t.starts_with("--") && t.contains('=') {
                let (k, v) = t.split_once('=').unwrap_or_default();
                if !VALUE_FLAGS.contains(&k) {
                    return Err(BoardError::usage(format!("未知参数 {k}")));
                }
                a.values.insert(k.to_string(), v.to_string());
            } else if VALUE_FLAGS.contains(&t.as_str()) {
                let v = tokens.get(i + 1).ok_or_else(|| BoardError::usage(format!("{t} 需要一个值")))?;
                a.values.insert(t.clone(), v.clone());
                i += 1;
            } else if BOOL_FLAGS.contains(&t.as_str()) {
                a.bools.insert(t.clone());
            } else if t.starts_with("--") {
                return Err(BoardError::usage(format!("未知参数 {t}，运行 board help 查看用法")));
            } else {
                a.pos.push(t.clone());
            }
            i += 1;
        }
        Ok(a)
    }
    fn value(&self, k: &str) -> Option<&str> {
        self.values.get(k).map(|s| s.as_str())
    }
    fn flag(&self, k: &str) -> bool {
        self.bools.contains(k)
    }
    fn id(&self, sub: &str) -> Result<i64> {
        let raw = self.pos.get(1).ok_or_else(|| BoardError::usage(format!("board {sub} 需要 <ID>")))?;
        raw.trim_start_matches('#')
            .parse::<i64>()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| BoardError::usage(format!("无效的 ID「{raw}」")))
    }
}

/* ------------------------------ 执行 ------------------------------ */

fn run(tokens: &[String], p: &Paint) -> Result<Output> {
    let a = Args::parse(tokens)?;
    let json = a.flag("--json");
    let sub = a.pos.first().map(|s| s.as_str());
    let sub = match sub {
        None => return Ok(Output::Text(help(p))),
        Some(_) if a.flag("--help") || a.flag("-h") => return Ok(Output::Text(help(p))),
        Some("help") => return Ok(Output::Text(help(p))),
        Some(s) if !COMMANDS.contains(&s) => {
            return Err(BoardError::usage(format!("未知命令「{s}」，运行 board help 查看用法")));
        }
        Some(s) => s,
    };

    // 自检只检查环境本身：不需要身份和白板
    if sub == "doctor" {
        return Ok(doctor::run(json, p));
    }

    // 身份（PRD §3）
    let name = a.value("--as").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).or_else(|| env("BOARD_AS"));
    let Some(name) = name else {
        return Err(BoardError::code(ErrorCode::NoIdentity));
    };
    let actor = Actor::agent(name.clone());

    let mut be = Backend::open()?;

    if sub == "boards" {
        let ws = be.workspace()?;
        let boards: Vec<(&ProjectInfo, &BoardInfo)> =
            ws.projects.iter().flat_map(|pr| pr.boards.iter().map(move |b| (pr, b))).collect();
        if json {
            return Ok(Output::Json(Value::Array(
                boards
                    .iter()
                    .map(|(pr, b)| json!({ "board": b.path, "project": pr.name, "name": b.name, "counts": b.counts, "flow": b.statuses.iter().map(|s| &s.key).collect::<Vec<_>>() }))
                    .collect(),
            )));
        }
        let w = boards.iter().map(|(_, b)| width(&b.path)).max().unwrap_or(0) + 2;
        let nw = boards.iter().map(|(pr, b)| width(&format!("{} / {}", pr.name, b.name))).max().unwrap_or(0) + 2;
        let mut s = String::new();
        for (pr, b) in boards {
            let n = |k: &str| b.counts.get(k).copied().unwrap_or(0);
            let mut parts = vec![format!("{INBOX_NAME} {}", n(INBOX))];
            parts.extend(b.statuses.iter().map(|st| format!("{} {}", st.name, n(&st.key))));
            s += &format!("{}{}{}\n", p.cyan(&pad(&b.path, w)), pad(&format!("{} / {}", pr.name, b.name), nw), p.dim(&parts.join(" · ")));
        }
        return Ok(Output::Text(s));
    }

    // 白板（PRD §3）
    let raw = a.value("--board").map(|s| s.to_string()).or_else(|| env("BOARD"));
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Err(BoardError::code(ErrorCode::NoBoard));
    };
    let BoardData { path, flow, cards: all, labels, now } = be.board(&raw)?;
    let label_name = |id: i64| labels.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_default();
    let find_label = |name: &str| -> Result<i64> {
        let q = name.trim().to_lowercase();
        labels.iter().find(|l| l.name.to_lowercase() == q).map(|l| l.id).ok_or_else(|| {
            BoardError::usage(format!(
                "项目里没有标签「{name}」，可选：{}",
                if labels.is_empty() {
                    "（这个项目还没有标签，在界面上新建）".to_string()
                } else {
                    labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join("、")
                }
            ))
        })
    };
    let mut exec = |cmd: Cmd| -> Result<CardDetail> { be.exec(&path, &actor, cmd) };
    let fmt = Fmt { p, label_name: &label_name, path: &path, now, flow: &flow };

    let out = match sub {
        "flow" => {
            let cards = &all;
            let count = |k: &str| cards.iter().filter(|c| c.column == k).count();
            if json {
                Output::Json(Value::Array(
                    flow.0
                        .iter()
                        .map(|st| json!({ "key": st.key, "name": st.name, "claim": st.claim, "lock": st.lock, "final": st.fin, "to": st.to, "cards": count(&st.key) }))
                        .collect(),
                ))
            } else {
                let kw = flow.0.iter().map(|st| width(&st.key)).max().unwrap_or(0).max(5) + 2;
                let nw = flow.0.iter().map(|st| width(&st.name)).max().unwrap_or(0) + 2;
                let mut s = String::new();
                let rule_of = |st: &Status| {
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
                    rules.join(" · ")
                };
                let rw = flow.0.iter().map(|st| width(&rule_of(st))).max().unwrap_or(0) + 2;
                for (i, st) in flow.0.iter().enumerate() {
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
                    let to: Vec<&str> = st.to.iter().map(|k| flow.name(k)).collect();
                    s += &format!(
                        "{} {}{}{}{}{}\n",
                        p.dim(&format!("{}.", i + 1)),
                        fmt.col(&st.key, &pad(&st.key, kw)),
                        pad(&st.name, nw),
                        p.dim(&pad(&format!("{} 张", count(&st.key)), 8)),
                        p.yellow(&pad(&rules.join(" · "), rw)),
                        if to.is_empty() { p.dim("→ （没有连线）") } else { format!("→ {}", to.join("、")) }
                    );
                }
                s += &format!("{} {}{}{}\n", p.dim("·"), p.magenta(&pad(INBOX, kw)), pad(INBOX_NAME, nw), p.dim(&format!("{} 张  不属于工作流", count(INBOX))));
                Output::Text(s)
            }
        }
        "ls" => {
            let column = match a.value("--column") {
                Some(c) => Some(
                    flow.resolve(c)
                        .ok_or_else(|| BoardError::usage(format!("未知的状态「{}」，可选：{}", c.trim(), flow.choices())))?,
                ),
                None => None,
            };
            let label = a.value("--label").map(find_label).transpose()?;
            let q = a.value("--search").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
            let cards: Vec<Card> = all
                .into_iter()
                .filter(|c| column.as_ref().is_none_or(|col| &c.column == col))
                .filter(|c| label.is_none_or(|l| c.labels.contains(&l)))
                .filter(|c| q.as_ref().is_none_or(|q| format!("{}\n{}", c.title, c.body).to_lowercase().contains(q)))
                .collect();
            if json {
                Output::Json(Value::Array(cards.iter().map(|c| fmt.card(c)).collect()))
            } else {
                Output::Text(fmt.list(&cards))
            }
        }
        "show" => {
            let id = a.id(sub)?;
            let d = be.card(&path, id)?;
            if json { Output::Json(fmt.full(&d)) } else { Output::Text(fmt.show(&d)) }
        }
        "new" => {
            let title = a.pos[1..].join(" ");
            let top = a.flag("--top");
            let d = exec(Cmd::New {
                title,
                body: a.value("--body").map(|s| s.to_string()),
                column: None,
                top,
                labels: vec![],
            })?;
            if json {
                Output::Json(fmt.full(&d))
            } else {
                Output::Text(format!(
                    "{} 已新建 {}  {}{}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{}", d.card.id)),
                    d.card.title,
                    p.dim(&format!("  → {}最{}", flow.home().name, if top { "上面" } else { "下面" }))
                ))
            }
        }
        "edit" => {
            let id = a.id(sub)?;
            if a.value("--title").is_none() && a.value("--body").is_none() {
                return Err(BoardError::usage("board edit 需要 --title 或 --body"));
            }
            let d = exec(Cmd::Edit {
                id,
                title: a.value("--title").map(|s| s.to_string()),
                body: a.value("--body").map(|s| s.to_string()),
            })?;
            if json {
                Output::Json(fmt.full(&d))
            } else {
                Output::Text(format!("{} 已更新 {}{}\n", p.green("✓"), p.cyan(&format!("#{}", d.card.id)), fmt.renewed(&d, &name)))
            }
        }
        "move" => {
            let id = a.id(sub)?;
            let col = a.pos.get(2).ok_or_else(|| BoardError::usage("board move 需要 <状态>"))?;
            let top = a.flag("--top");
            let d = exec(Cmd::Move { id, column: col.clone(), top, index: None })?;
            if json {
                Output::Json(fmt.full(&d))
            } else {
                Output::Text(format!(
                    "{} {} → {} ({}){}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{}", d.card.id)),
                    flow.name(&d.card.column),
                    d.card.column,
                    p.dim(if top { "  最上面" } else { "" })
                ))
            }
        }
        "claim" => {
            let id = if a.pos.len() > 1 { Some(a.id(sub)?) } else { None };
            let d = exec(Cmd::Claim { id })?;
            if json {
                Output::Json(fmt.full(&d))
            } else {
                Output::Text(format!(
                    "{} {}  {}\n{}\n\n{}",
                    p.green("✓"),
                    p.bold("已领取"),
                    p.dim(&format!("进入「{}」", flow.name(&d.card.column))),
                    p.yellow(&format!(
                        "马上开始心跳（skill 里的 scripts/heartbeat.sh start {}）：{} 秒内没收到心跳，领取会被释放",
                        d.card.id,
                        BEAT_MS * BEAT_MISSES / 1000
                    )),
                    fmt.show(&d)
                ))
            }
        }
        "comment" => {
            let id = a.id(sub)?;
            let text = a.pos[2..].join(" ");
            let d = exec(Cmd::Comment { id, text })?;
            if json {
                Output::Json(fmt.full(&d))
            } else {
                Output::Text(format!("{} 已评论 {}{}\n", p.green("✓"), p.cyan(&format!("#{}", d.card.id)), fmt.renewed(&d, &name)))
            }
        }
        "beat" => {
            let id = a.id(sub)?;
            let done = a.flag("--final");
            let d = exec(Cmd::Beat { id, done })?;
            if json {
                Output::Json(fmt.full(&d))
            } else if done {
                let total = d.card.work_ms;
                Output::Text(format!(
                    "{} {} 心跳已结束{}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{}", d.card.id)),
                    if d.card.runs > 0 { p.dim(&format!("  累计 {}（{} 次）", clock(total), d.card.runs)) } else { String::new() }
                ))
            } else {
                Output::Text(format!("{} {} 心跳{}\n", p.green("✓"), p.cyan(&format!("#{}", d.card.id)), fmt.renewed(&d, &name)))
            }
        }
        "label" => {
            let id = a.id(sub)?;
            let ops = &a.pos[2..];
            if ops.is_empty() {
                return Err(BoardError::usage("用法：board label <ID> +<标签> -<标签> …"));
            }
            let (mut add, mut remove) = (vec![], vec![]);
            for o in ops {
                if let Some(n) = o.strip_prefix('-') {
                    remove.push(find_label(n)?);
                } else {
                    add.push(find_label(o.strip_prefix('+').unwrap_or(o))?);
                }
            }
            let d = exec(Cmd::Label { id, add, remove })?;
            if json {
                Output::Json(fmt.full(&d))
            } else {
                let ls: Vec<String> = d.card.labels.iter().map(|l| p.magenta(&format!("#{}", label_name(*l)))).collect();
                Output::Text(format!(
                    "{} {}  标签：{}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{}", d.card.id)),
                    if ls.is_empty() { p.dim("（无）") } else { ls.join(" ") }
                ))
            }
        }
        "attach" => {
            let id = a.id(sub)?;
            let paths = &a.pos[2..];
            if paths.is_empty() {
                return Err(BoardError::usage("用法：board attach <ID> <文件>… [--comment \"<说明>\"]"));
            }
            let mut done: Vec<Attachment> = vec![];
            for f in paths {
                let bytes = std::fs::read(f).map_err(|e| BoardError::usage(format!("读不到文件 {f}：{e}")))?;
                let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
                done.push(be.attach(&path, id, &actor, &name, &bytes)?);
            }
            // 附件放进一条评论里，界面的动态里就能看到（图片直接显示）
            let mut text = a.value("--comment").map(|s| s.trim().to_string()).unwrap_or_default();
            for f in &done {
                text += &format!("{}{}", if text.is_empty() { "" } else { "\n\n" }, f.markdown);
            }
            let d = be.exec(&path, &actor, Cmd::Comment { id, text })?;
            if json {
                Output::Json(json!({ "attachments": done.iter().map(attachment_json).collect::<Vec<_>>(), "card": fmt.full(&d) }))
            } else {
                let mut s = format!("{} 已上传 {} 个附件到 {}{}\n", p.green("✓"), done.len(), p.cyan(&format!("#{id}")), fmt.renewed(&d, &name));
                for f in &done {
                    s += &format!("  {}  {}  {}\n", p.dim(&format!("{}", f.id)), f.name, p.dim(&human_size(f.size)));
                }
                Output::Text(s)
            }
        }
        "files" => {
            let id = a.id(sub)?;
            let d = be.card(&path, id)?;
            let dir = match a.value("--out") {
                Some(o) => std::path::PathBuf::from(o),
                None => std::env::temp_dir().join("board-files").join(format!("{}-{id}", path.replace('/', "-"))),
            };
            let mut saved = vec![];
            if !d.attachments.is_empty() {
                std::fs::create_dir_all(&dir).map_err(|e| BoardError::usage(format!("无法创建目录 {}：{e}", dir.display())))?;
            }
            for f in &d.attachments {
                let bytes = be.download(f)?;
                let to = dir.join(format!("{}-{}", f.id, f.name));
                std::fs::write(&to, bytes).map_err(|e| BoardError::usage(format!("写入 {} 失败：{e}", to.display())))?;
                saved.push((f, to));
            }
            if json {
                Output::Json(Value::Array(
                    saved
                        .iter()
                        .map(|(f, to)| {
                            let mut v = attachment_json(f);
                            v["path"] = json!(to.display().to_string());
                            v
                        })
                        .collect(),
                ))
            } else if saved.is_empty() {
                Output::Text(format!("{}\n", p.dim(&format!("#{id} 没有附件"))))
            } else {
                let mut s = format!("{} 已下载 {} 个附件到 {}\n", p.green("✓"), saved.len(), dir.display());
                for (f, to) in &saved {
                    s += &format!("  {}  {}\n", to.display(), p.dim(&human_size(f.size)));
                }
                Output::Text(s)
            }
        }
        "labels" => {
            if json {
                Output::Json(Value::Array(
                    labels.iter().map(|l| json!({ "name": l.name, "color": l.color, "cards": l.uses })).collect(),
                ))
            } else if labels.is_empty() {
                Output::Text(format!("{}\n", p.dim("（这个项目还没有标签，在界面上新建）")))
            } else {
                let w = labels.iter().map(|l| width(&l.name)).max().unwrap_or(0) + 4;
                Output::Text(
                    labels
                        .iter()
                        .map(|l| format!("{}{}\n", p.magenta(&pad(&format!("#{}", l.name), w)), p.dim(&format!("{} 张卡片", l.uses))))
                        .collect(),
                )
            }
        }
        _ => unreachable!(),
    };
    Ok(out)
}

/* ------------------------------ 输出 ------------------------------ */

struct Fmt<'a> {
    p: &'a Paint,
    label_name: &'a dyn Fn(i64) -> String,
    path: &'a str,
    now: i64,
    flow: &'a Flow,
}

fn iso(ms: Option<i64>) -> Value {
    match ms.and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single()) {
        Some(t) => Value::String(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)),
        None => Value::Null,
    }
}

/// 耗时：12:34 / 1:02:03
fn clock(ms: i64) -> String {
    let t = ms.max(0) / 1000;
    let (h, m, s) = (t / 3600, t % 3600 / 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

impl Fmt<'_> {
    /// 按状态的规则上色：可领取白、锁定黄、终点绿、未整理紫、其他蓝
    fn col(&self, key: &str, s: &str) -> String {
        let code = if key == INBOX {
            "35"
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

    fn card(&self, c: &Card) -> Value {
        json!({
            "id": c.id,
            "column": c.column,
            "title": c.title,
            "holder": c.holder,
            "labels": self.labels(c),
        })
    }

    fn full(&self, d: &CardDetail) -> Value {
        let c = &d.card;
        json!({
            "id": c.id,
            "board": self.path,
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
    fn renewed(&self, d: &CardDetail, me: &str) -> String {
        match (&d.card.holder, d.card.run_started_at) {
            (Some(h), Some(t)) if h == me => self.p.dim(&format!("  本次 {}", clock(self.now - t))),
            _ => String::new(),
        }
    }

    fn list(&self, cards: &[Card]) -> String {
        let p = self.p;
        if cards.is_empty() {
            return format!("{}\n", p.dim("（没有匹配的卡片）"));
        }
        let tw = cards.iter().map(|c| width(&c.title)).max().unwrap_or(0).min(56) + 2;
        let lw = cards.iter().map(|c| width(&self.tags(c))).max().unwrap_or(0) + 2;
        let mut s = String::new();
        for c in cards {
            let tags = self.tags(c);
            let holder = c.holder.as_deref().unwrap_or("");
            let cw = cards.iter().map(|c| width(&c.column)).max().unwrap_or(0).max(6) + 2;
            let mut line = format!("{}{}", p.cyan(&pad(&format!("#{}", c.id), 6)), self.col(&c.column, &pad(&c.column, cw)));
            let title = truncate(&c.title, 56);
            if tags.is_empty() && holder.is_empty() {
                line += &title;
            } else {
                line += &pad(&title, tw);
                if holder.is_empty() {
                    line += &p.magenta(&tags);
                } else {
                    line += &format!("{}{}", p.magenta(&pad(&tags, lw)), p.yellow(holder));
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

    fn show(&self, d: &CardDetail) -> String {
        let p = self.p;
        let c = &d.card;
        let mut s = String::new();
        let row = |k: &str| p.dim(&pad(k, 10));
        s += &format!("{}  {}\n", p.cyan(&format!("#{}", c.id)), p.bold(&c.title));
        s += &format!("{}{} {}\n", row("状态"), self.flow.name(&c.column), self.col(&c.column, &format!("({})", c.column)));
        if !c.labels.is_empty() {
            s += &format!("{}{}\n", row("标签"), p.magenta(&self.tags(c)));
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

fn help(p: &Paint) -> String {
    let rows: [(&str, &str, &str); 16] = [
        ("doctor", "", "环境自检：board 路径、PATH、BOARD_URL/BOARD_TOKEN、BOARD_AS/BOARD、服务端版本；有问题退出码非 0"),
        ("boards", "", "列出所有白板"),
        ("flow", "", "列出白板的工作流：状态的顺序、标识、名称、规则，以及每个状态能去哪里"),
        ("ls", "[--column <状态>] [--search <文字>] [--label <标签>]", "列出卡片"),
        ("show", "<ID>", "显示卡片的全部字段和动态"),
        ("new", "\"<标题>\" [--body \"<描述>\"] [--top]", "在首个可领取状态新建卡片"),
        ("edit", "<ID> [--title \"<标题>\"] [--body \"<描述>\"]", "修改标题或描述"),
        ("move", "<ID> <状态> [--top]", "把卡片移到指定状态（标识或名称）；只能沿 board flow 里的连线移动"),
        ("claim", "[<ID>]", "领取；不带 ID 时按工作流顺序领可领取状态最上面的卡片"),
        ("comment", "<ID> \"<内容>\"", "追加评论"),
        ("beat", "<ID> [--final]", "心跳：领取后每 10 秒一次，50 秒没收到就释放；--final 正常结束并记下本次耗时。由 skill 里的脚本调用"),
        ("label", "<ID> +<标签> -<标签> …", "加标签（+ 或不带前缀）、去标签（-）"),
        ("labels", "", "列出项目的标签，以及每个标签被多少张卡片使用"),
        ("attach", "<ID> <文件>… [--comment \"<说明>\"]", "上传附件（图片、日志等），连同说明作为一条评论"),
        ("files", "<ID> [--out <目录>]", "把卡片的附件下载到本地（默认临时目录），列出路径"),
        ("help", "", "显示这段说明"),
    ];
    let aw = rows.iter().map(|r| width(r.1)).max().unwrap_or(0) + 2;
    let mut out = vec![
        format!("{} {}", p.bold("board"), p.dim("— 看板式白板的命令行。人在界面上操作，agent 通过它读写同一块白板。")),
        String::new(),
    ];
    for (cmd, args, desc) in rows {
        out.push(format!("  board {} {}{}", p.cyan(&pad(cmd, 8)), p.dim(&pad(args, aw)), desc));
    }
    out.extend([
        String::new(),
        format!("  {}默认是 todo  doing  review  done  rework  check；每块白板可以自定义，用 board flow 查看  {}", p.dim("状态："), p.dim("（未整理：inbox）")),
        format!("  {}--as <名字>，或 export BOARD_AS=<名字>", p.dim("身份：")),
        format!("  {}--board <项目>/<白板>，或 export BOARD=<项目>/<白板>", p.dim("白板：")),
        format!("  {}export BOARD_URL=<服务端地址> BOARD_TOKEN=<访问令牌>（必填，令牌在界面「设置 → Agent 接入」新建）", p.dim("连接：")),
        format!("  {}所有命令都支持 --json；被拒绝时退出码为 1，并给出错误码", p.dim("输出：")),
        format!("  {}{}", p.dim("数据："), Backend::describe()),
        String::new(),
    ]);
    out.join("\n")
}

/* ------------------------------ 文本工具 ------------------------------ */

fn attachment_json(f: &Attachment) -> Value {
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

fn human_size(n: i64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{:.1} MB", n as f64 / 1048576.0),
    }
}

fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn pad(s: &str, w: usize) -> String {
    let n = width(s);
    if n >= w { s.to_string() } else { format!("{s}{}", " ".repeat(w - n)) }
}

fn truncate(s: &str, w: usize) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    let mut out = String::new();
    for ch in s.chars() {
        if width(&out) + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) > w - 1 {
            break;
        }
        out.push(ch);
    }
    out + "…"
}

struct Paint {
    on: bool,
}

impl Paint {
    fn wrap(&self, code: &str, s: &str) -> String {
        if self.on && !s.is_empty() { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
    }
    fn dim(&self, s: &str) -> String {
        self.wrap("2", s)
    }
    fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }
    fn red(&self, s: &str) -> String {
        self.wrap("31", s)
    }
    fn red_bold(&self, s: &str) -> String {
        self.wrap("1;31", s)
    }
    fn green(&self, s: &str) -> String {
        self.wrap("32", s)
    }
    fn yellow(&self, s: &str) -> String {
        self.wrap("33", s)
    }
    fn magenta(&self, s: &str) -> String {
        self.wrap("35", s)
    }
    fn cyan(&self, s: &str) -> String {
        self.wrap("36", s)
    }
}
