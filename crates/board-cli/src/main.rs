//! `board` —— agent 用的命令行（PRD §9）。
//!
//! 身份：先 `board check --as <名字>`，之后每条命令都带 `--as <名字>`（或环境变量 `BOARD_AS`）；
//! 白板：`--board <项目>/<白板>` 或环境变量 `BOARD`。
//! 所有命令支持 `--json`。成功退出码 0；被拒绝退出码 1，并给出错误码和原因。
//! 数据：通过服务端读写。服务端地址和访问令牌由 `board login` 写进 `~/.board/config.toml`，
//! 环境变量 `BOARD_URL`、`BOARD_TOKEN` 优先；没有配置时直接报错。见 backend.rs、config.rs。
//! 任务下发：`board host` 在这台机器上常驻接派发（host.rs），`board dispatch` 把卡片派给一台机器。

mod backend;
mod config;
mod doctor;
mod envs;
mod heartbeat;
mod host;

use backend::{env, Backend, BoardData, Remote};
use board_core::*;
use chrono::TimeZone;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use unicode_width::UnicodeWidthStr;

const VALUE_FLAGS: [&str; 19] = [
    "--as", "--board", "--column", "--search", "--label", "--body", "--title", "--comment", "--out", "--url", "--token", "--name", "--dir",
    "--host", "--agent", "--body-file", "--comment-file", "--image", "--file",
];
const BOOL_FLAGS: [&str; 9] = ["--json", "--top", "--inbox", "--help", "-h", "--final", "--cancel", "--version", "-V"];
/// 全部命令：（命令、参数、说明）。认不认识一个命令、board help 的内容都只看这张表；
/// 新加命令时在这里加一行，再在 run() 里处理（测试 every_command_is_dispatched 会检查）
const COMMANDS: [(&str, &str, &str); 23] = [
        ("login", "[--url <地址>] [--token <令牌>]", "校验令牌后把服务端地址和令牌写进 ~/.board/config.toml；不带 --token 时从标准输入读一行"),
        ("logout", "", "从 ~/.board/config.toml 删掉令牌，保留服务端地址"),
        ("doctor", "[--as <名字>] [--board <白板>]", "环境自检：board 路径、PATH、服务端地址和令牌（以及来自哪里）、服务端版本、名字是否在线；有问题退出码非 0"),
        ("check", "--as <名字>", "会话开始时占用名字（<运行环境>/<模型>[-后缀]）；名字正被在线 agent 使用时 NAME_TAKEN"),
        ("heartbeat", "start | stop | final <ID>", "start：check 之后启动后台心跳（每 10 秒一次 board beat，跟随 agent 进程），立即返回；stop：停掉后台心跳再发 final，名字立刻离线；final <ID>：结束这张卡片这一次的计时"),
        ("beat", "[<ID>] [--final]", "不带 ID：agent 级心跳，维持名字在线和持有的卡片；--final 名字立刻离线。带 ID：这张卡片的心跳；--final 结束这一次计时。一般用 board heartbeat"),
        ("boards", "", "列出所有白板"),
        ("flow", "", "列出白板的工作流：状态的顺序、标识、名称、规则，以及每个状态能去哪里"),
        ("ls", "[--column <状态>] [--search <文字>] [--label <标签>]", "列出卡片"),
        ("show", "<ID>", "显示卡片的全部字段和动态"),
        ("new", "\"<标题>\" [--body \"<描述>\" | --body-file <文件>] [--image <图片>]… [--file <文件>]… [--top] [--inbox] [--label <标签>]…", "在首个可领取状态新建卡片；--inbox 记到「未整理」最下面；--label 可以写多次，带上项目里已有的标签；多行描述用 --body-file（- 读标准输入）；--image、--file 可以写多次，按顺序附在描述后面（图片直接显示，文件是下载链接）"),
        ("edit", "<ID> [--title \"<标题>\"] [--body \"<描述>\" | --body-file <文件>]", "修改标题或描述"),
        ("move", "<ID> <状态> [--top]", "把卡片移到指定状态（标识或名称）；只能沿 board flow 里的连线移动"),
        ("claim", "[<ID>]", "领取；不带 ID 时按工作流顺序领可领取状态最上面的卡片"),
        ("next", "", "只在 board host 启动的会话里用：上一张交审或交给人之后领下一张接着做；做满了、没有可做的返回 SESSION_DONE"),
        ("comment", "<ID> \"<内容>\" | <ID> --comment-file <文件>", "追加评论；多行内容用 --comment-file（- 读标准输入）"),
        ("label", "<ID> +<标签> -<标签> …", "加标签（+ 或不带前缀）、去标签（-）；仅人可打的标签不能加、去"),
        ("labels", "", "列出项目的标签、类型（agent 可打 / 仅人可打）、是否优先，以及每个标签被多少张卡片使用"),
        ("attach", "<ID> <文件>… [--comment \"<说明>\" | --comment-file <文件>]", "上传附件（图片、日志等），连同说明作为一条评论"),
        ("files", "<ID> [--out <目录>]", "把卡片的附件下载到本地（默认临时目录），列出路径"),
        ("host", "[run] [--name <机器名>] [--dir <项目>/<白板>=<路径>]… | rename <新机器名>", "run（默认）：在这台机器上常驻接任务，派来的卡片在对应目录里启动本机 agent，Ctrl+C 退出（机器名、白板与目录也可写在 ~/.board/config.toml 的 [host]）；rename：机器改名，服务端上的记录原地改名，设置和历史派发记录保留（桌面端「设置 → 本机」保存机器名时调用）"),
        ("dispatch", "<ID> --host <机器名> [--agent <标识>] | <ID> --cancel", "把卡片派给一台机器上的 agent（不带 --agent 用默认的）；--cancel 取消排队或停止正在跑的派发"),
        ("help", "", "显示这段说明"),
];

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

/// 打印结果并返回退出码。标准输出写不进去（管道读端已关）时不 panic：
/// 例如 board host 运行中输出被关掉，前面没写出去的内容还在缓冲里，这里再打印会再失败一次
fn emit(out: Output) -> i32 {
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

/* ------------------------------ 参数 ------------------------------ */

struct Args {
    values: HashMap<String, String>,
    /// 可以写多次的参数（--dir）：按出现的顺序
    many: Vec<(String, String)>,
    bools: HashSet<String>,
    pos: Vec<String>,
}

impl Args {
    fn parse(tokens: &[String]) -> Result<Args> {
        let mut a = Args { values: HashMap::new(), many: vec![], bools: HashSet::new(), pos: vec![] };
        let mut i = 0;
        while i < tokens.len() {
            let t = &tokens[i];
            if t.starts_with("--") && t.contains('=') {
                let (k, v) = t.split_once('=').unwrap_or_default();
                if !VALUE_FLAGS.contains(&k) {
                    return Err(BoardError::usage(format!("未知参数 {k}")));
                }
                a.values.insert(k.to_string(), v.to_string());
                a.many.push((k.to_string(), v.to_string()));
            } else if VALUE_FLAGS.contains(&t.as_str()) {
                let v = tokens.get(i + 1).ok_or_else(|| BoardError::usage(format!("{t} 需要一个值")))?;
                a.values.insert(t.clone(), v.clone());
                a.many.push((t.clone(), v.clone()));
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
    /// 写了多次的参数的全部值
    fn all(&self, k: &str) -> Vec<String> {
        self.many.iter().filter(|(x, _)| x == k).map(|(_, v)| v.clone()).collect()
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
    // 版本号（发版附件里的安装包、skill 自带的 board 都能这样核对）
    if a.flag("--version") || a.flag("-V") {
        let v = env!("CARGO_PKG_VERSION");
        return Ok(if json { Output::Json(json!({ "version": v })) } else { Output::Text(format!("board {v}")) });
    }
    let sub = a.pos.first().map(|s| s.as_str());
    let sub = match sub {
        None => return Ok(Output::Text(help(p))),
        Some(_) if a.flag("--help") || a.flag("-h") => return Ok(Output::Text(help(p))),
        Some("help") => return Ok(Output::Text(help(p))),
        Some(s) if !COMMANDS.iter().any(|c| c.0 == s) => {
            return Err(BoardError::usage(format!("未知命令「{s}」，运行 board help 查看用法")));
        }
        Some(s) => s,
    };

    // 自检只检查环境本身：名字和白板没提供时跳过
    if sub == "doctor" {
        return Ok(doctor::run(json, p, a.value("--as"), a.value("--board")));
    }
    // 登录、退出：只读写配置文件，不需要名字和白板
    if sub == "login" {
        return login(&a, json, p);
    }
    if sub == "logout" {
        return logout(json, p);
    }

    // 任务下发：board host 在前台一直运行；board host rename 不需要名字（带了也只当参数）；board dispatch 不需要名字（带了就记为发起人）
    if sub == "host" {
        if a.pos.get(1).map(String::as_str) == Some("rename") {
            let to = a
                .pos
                .get(2)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| BoardError::usage("用法：board host rename <新机器名>"))?;
            return host::rename(to, json, p);
        }
        host::run(host::Opts { name: a.value("--name").map(str::to_string), dirs: a.all("--dir") }, p)?;
        return Ok(Output::Text(String::new()));
    }
    if sub == "dispatch" {
        return dispatch(&a, json, p);
    }

    // 连接（PRD §9）：环境变量优先，其次 ~/.board/config.toml；都没有时 NOT_LOGGED_IN
    let mut be = Backend::open()?;

    // 身份（PRD §3）
    let name = a.value("--as").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).or_else(|| env("BOARD_AS"));
    let Some(name) = name else {
        return Err(BoardError::code(ErrorCode::NoIdentity));
    };
    let actor = Actor::agent(name.clone());

    // 名字（PRD §3）：check 占用名字；不带 ID 的 beat 是 agent 级心跳。都不需要白板
    if sub == "check" {
        check_agent_name(&name).map_err(BoardError::usage)?;
        let info = be.check(&name)?;
        return Ok(if json {
            Output::Json(agent_json(&info))
        } else {
            Output::Text(format!(
                "{} {} 已在线\n{}\n",
                p.green("✓"),
                p.bold(&name),
                p.yellow(&format!(
                    "马上启动心跳：board heartbeat start --as {name}（后台每 {} 秒发一次心跳；{} 秒没收到就离线）",
                    BEAT_MS / 1000,
                    NAME_GAP_MS / 1000
                ))
            ))
        });
    }
    if sub == "heartbeat" {
        return heartbeat_cmd(&a, &name, json, p);
    }
    if sub == "beat" && a.pos.len() == 1 {
        let done = a.flag("--final");
        let info = be.agent_beat(&name, done)?;
        if json {
            return Ok(Output::Json(agent_json(&info)));
        }
        let cards: Vec<String> = info.cards.iter().map(|h| format!("{}#{}", h.board, h.card.id)).collect();
        return Ok(Output::Text(if done {
            format!(
                "{} {} 已离线{}\n",
                p.green("✓"),
                p.bold(&name),
                if cards.is_empty() {
                    String::new()
                } else {
                    p.dim(&format!("  持有的 {} 收不到心跳，{} 秒内释放", cards.join("、"), NAME_GAP_MS / 1000))
                }
            )
        } else {
            let mut s = format!("{} {} 心跳\n", p.green("✓"), p.bold(&name));
            for h in &info.cards {
                let run = match h.card.run_started_at {
                    Some(t) => format!("本次 {}", clock(info.beat_at - t)),
                    None => "已 final：50 秒内沿连线移走".to_string(),
                };
                s += &format!("  {} {}  {}\n", p.cyan(&format!("{}#{}", h.board, h.card.id)), h.card.title, p.dim(&run));
            }
            s
        }));
    }

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
    // board next：只在 board host 启动的会话里（环境里有 BOARD_HOST），由服务端结算上一张、领下一张
    let next = if sub == "next" {
        if env("BOARD_HOST").is_none() {
            return Err(BoardError::usage("board next 只能在 board host 启动的会话里用（环境里没有 BOARD_HOST）"));
        }
        Some(be.next(&name)?)
    } else {
        None
    };
    let mut exec = |cmd: Cmd| -> Result<CardDetail> { be.exec(&path, &actor, cmd) };
    let label_priority = |id: i64| labels.iter().any(|l| l.id == id && l.priority);
    let fmt = Fmt { p, label_name: &label_name, label_priority: &label_priority, path: &path, now, flow: &flow };

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
            let inbox = a.flag("--inbox");
            let top = a.flag("--top") && !inbox;
            // --label 可以写多次，按名称匹配（不区分大小写），同一个只算一次；不存在、仅人可打的都在新建前拒绝
            let mut with: Vec<i64> = vec![];
            for n in a.all("--label") {
                let id = find_label(&n)?;
                if !with.contains(&id) {
                    with.push(id);
                }
            }
            if let Some(l) = labels.iter().find(|l| l.human_only && with.contains(&l.id)) {
                return Err(BoardError::usage(format!("标签「{}」只有人能打（仅人可打），agent 建卡时不能带上", l.name)));
            }
            let mut body = text_arg(&a, "--body", "--body-file")?;
            // --image / --file：先在本地全部检查一遍（不存在、类型不支持、超过 10 MB 都在上传前拒绝，不建卡），
            // 再逐个上传成还没归属的附件，按参数顺序把它们的 Markdown 接在描述后面；建卡时服务端按引用把附件归到这张卡片
            let files = new_files(&a, title.trim().is_empty())?;
            if !files.is_empty() {
                let mut refs = vec![];
                for (name, bytes) in &files {
                    refs.push(be.upload_unbound(&path, &actor, name, bytes)?.markdown);
                }
                let text = body.take().unwrap_or_default();
                let text = text.trim_end();
                body = Some(format!("{text}{}{}", if text.is_empty() { "" } else { "\n\n" }, refs.join("\n\n")));
            }
            let d = be.exec(&path, &actor, Cmd::New {
                title,
                body,
                column: inbox.then(|| INBOX.to_string()),
                top,
                labels: with,
            })?;
            if json {
                Output::Json(fmt.full(&d))
            } else {
                let ls: Vec<String> = d.card.labels.iter().map(|l| p.magenta(&format!("#{}", label_name(*l)))).collect();
                Output::Text(format!(
                    "{} 已新建 {}  {}{}{}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{}", d.card.id)),
                    d.card.title,
                    if ls.is_empty() { String::new() } else { format!("  {}", ls.join(" ")) },
                    p.dim(&format!("  → {}最{}", if inbox { INBOX_NAME } else { flow.home().name.as_str() }, if top { "上面" } else { "下面" }))
                ))
            }
        }
        "edit" => {
            let id = a.id(sub)?;
            let body = text_arg(&a, "--body", "--body-file")?;
            if a.value("--title").is_none() && body.is_none() {
                return Err(BoardError::usage("board edit 需要 --title、--body 或 --body-file"));
            }
            let d = exec(Cmd::Edit { id, title: a.value("--title").map(|s| s.to_string()), body })?;
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
            let d = exec(Cmd::Move { id, column: col.clone(), top, index: None, order: None, before: None, after: None })?;
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
                        "会话开始时启动的心跳（board heartbeat start）会维持这张卡片；做完后 board heartbeat final {}，再在 {} 秒内沿连线移走",
                        d.card.id,
                        BEAT_MS * BEAT_MISSES / 1000
                    )),
                    fmt.show(&d)
                ))
            }
        }
        "next" => {
            let Some(d) = next else { return Err(BoardError::usage("board next 没有结果")) };
            if json {
                Output::Json(fmt.full(&d))
            } else {
                Output::Text(format!(
                    "{} {}  {}\n{}\n\n{}",
                    p.green("✓"),
                    p.bold("接着做下一张"),
                    p.dim(&format!("已领取，进入「{}」", flow.name(&d.card.column))),
                    p.yellow(&format!("上一张的派发已结算；这张做完后 board heartbeat final {}，沿连线移走，再 board next", d.card.id)),
                    fmt.show(&d)
                ))
            }
        }
        "comment" => {
            let id = a.id(sub)?;
            let text = match file_arg(&a, "--comment-file")? {
                Some(_) if a.pos.len() > 2 => return Err(BoardError::usage("评论内容和 --comment-file 只能给一个")),
                Some(t) => t,
                None => a.pos[2..].join(" "),
            };
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
            // 仅人可打的标签只有人能在界面上加、去；服务端同样会拒绝，这里先拦下，省一次请求
            if let Some(l) = labels.iter().find(|l| l.human_only && (add.contains(&l.id) || remove.contains(&l.id))) {
                return Err(BoardError::usage(format!("标签「{}」只有人能打（仅人可打），agent 不能加、去", l.name)));
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
            let mut text = text_arg(&a, "--comment", "--comment-file")?.map(|s| s.trim().to_string()).unwrap_or_default();
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
                    labels
                        .iter()
                        .map(|l| json!({ "name": l.name, "color": l.color, "cards": l.uses, "human_only": l.human_only, "priority": l.priority }))
                        .collect(),
                ))
            } else if labels.is_empty() {
                Output::Text(format!("{}\n", p.dim("（这个项目还没有标签，在界面上新建）")))
            } else {
                let w = labels.iter().map(|l| width(&l.name)).max().unwrap_or(0) + 4;
                Output::Text(
                    labels
                        .iter()
                        .map(|l| {
                            let kind = if l.human_only { "仅人可打" } else { "agent 可打" };
                            let prio = if l.priority { p.yellow("优先") } else { String::new() };
                            format!(
                                "{}{}{}{}\n",
                                p.magenta(&pad(&format!("#{}", l.name), w)),
                                pad(kind, 12),
                                p.dim(&pad(&format!("{} 张卡片", l.uses), 12)),
                                prio
                            )
                        })
                        .collect(),
                )
            }
        }
        // 命令表里有、上面却没处理：返回错误而不是 panic（测试 every_command_is_dispatched 防止发生）
        other => return Err(BoardError::usage(format!("命令「{other}」还没有实现"))),
    };
    Ok(out)
}

/* ------------------------------ 派发 ------------------------------ */

/// `board dispatch <ID> --host <机器名> [--agent <标识>]`、`board dispatch <ID> --cancel`（任务下发需求 §五）
fn dispatch(a: &Args, json: bool, p: &Paint) -> Result<Output> {
    let id = a.id("dispatch")?;
    let raw = a.value("--board").map(|s| s.to_string()).or_else(|| env("BOARD"));
    let Some(path) = raw.filter(|s| !s.trim().is_empty()) else {
        return Err(BoardError::code(ErrorCode::NoBoard));
    };
    let who = a.value("--as").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).or_else(|| env("BOARD_AS"));
    let mut be = Backend::open()?;
    if a.flag("--cancel") {
        let d = be.cancel_dispatch(&path, id, who.as_deref())?;
        if json {
            return Ok(Output::Json(serde_json::to_value(&d).unwrap_or_default()));
        }
        let what = if d.state == "done" {
            format!("派发已取消（{} · {}）", d.host, d.agent_label)
        } else {
            format!("已通知 {} 停止这次派发：结束 agent 进程后卡片移到需要人处理的状态", d.host)
        };
        return Ok(Output::Text(format!("{} {}  {what}\n", p.green("✓"), p.cyan(&format!("#{id}")))));
    }
    let host = a.value("--host").map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| {
        BoardError::usage("用法：board dispatch <ID> --host <机器名> [--agent <标识>]，或 board dispatch <ID> --cancel")
    })?;
    let d = be.dispatch(&path, id, host, a.value("--agent"), who.as_deref())?;
    if json {
        return Ok(Output::Json(serde_json::to_value(&d).unwrap_or_default()));
    }
    Ok(Output::Text(format!(
        "{} {} 已派发到 {} · {}  {}\n",
        p.green("✓"),
        p.cyan(&format!("#{}", d.card)),
        p.bold(&d.host),
        d.agent_label,
        p.dim("排队，机器有空位时开始")
    )))
}

/* ------------------------------ 登录 ------------------------------ */

/// 设置了环境变量时提醒：它们会覆盖配置文件
fn env_overrides(keys: &[&'static str]) -> Vec<&'static str> {
    keys.iter().copied().filter(|k| env(k).is_some()).collect()
}

fn override_hint(p: &Paint, vars: &[&str]) -> String {
    if vars.is_empty() {
        return String::new();
    }
    let exports: Vec<String> = vars.iter().map(|v| format!("export {v}")).collect();
    format!(
        "{} 环境变量 {} 会覆盖配置文件：从 shell 配置文件（例如 ~/.zshrc、~/.bashrc）里删掉对应的 {}\n",
        p.yellow("!"),
        vars.join("、"),
        exports.join("、")
    )
}

/// `board login [--url <地址>] [--token <令牌>]`：用 `GET /api/workspace` 校验令牌，通过后写进配置文件
fn login(a: &Args, json: bool, p: &Paint) -> Result<Output> {
    let file = config::File::load()?;
    let url = match a.value("--url").map(str::trim).filter(|s| !s.is_empty()) {
        Some(u) => u.trim_end_matches('/').to_string(),
        None => file.get(config::URL)?.ok_or_else(|| {
            BoardError::usage(format!("{} 里还没有服务端地址：board login --url <服务端地址> --token <令牌>", config::shown()))
        })?,
    };
    if !backend::is_http(&url) {
        return Err(BoardError::usage(format!("服务端地址需要以 https:// 或 http:// 开头：{url}")));
    }
    let token = match a.value("--token") {
        Some(t) => t.trim().to_string(),
        None => {
            if std::io::stdin().is_terminal() {
                eprint!("粘贴访问令牌（在界面「设置 → Agent 接入」新建）后回车：");
            }
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).map_err(|e| BoardError::usage(format!("读不到令牌：{e}")))?;
            line.trim().to_string()
        }
    };
    if token.is_empty() {
        return Err(BoardError::usage("没有令牌：board login --token <令牌>，或从标准输入给出一行令牌"));
    }
    // 校验：令牌错误、连不上时按服务端的错误码报错，不写文件
    Remote::new(&url, &token)?.get::<Workspace>("/api/workspace", &[])?;
    // 写的时候重新读一遍（校验期间别处可能改了配置文件，例如桌面端保存了本机设置）
    config::update(|f| {
        f.set(config::URL, &url);
        f.set(config::TOKEN, &token);
        Ok(((), true))
    })?;
    let vars = env_overrides(&["BOARD_URL", "BOARD_TOKEN"]);
    if json {
        return Ok(Output::Json(json!({ "ok": true, "url": url, "config": config::shown(), "env_overrides": vars })));
    }
    Ok(Output::Text(format!(
        "{} 已登录 {}：服务端地址和令牌写进了 {}（不显示令牌原文）\n{}",
        p.green("✓"),
        p.cyan(&url),
        config::shown(),
        override_hint(p, &vars)
    )))
}

/// `board logout`：从配置文件删掉令牌，保留服务端地址
fn logout(json: bool, p: &Paint) -> Result<Output> {
    let removed = config::update(|f| {
        let removed = f.remove(config::TOKEN);
        Ok((removed, removed))
    })?;
    let vars = env_overrides(&["BOARD_TOKEN"]);
    if json {
        return Ok(Output::Json(json!({ "ok": true, "removed": removed, "config": config::shown(), "env_overrides": vars })));
    }
    let head = if removed {
        format!("{} 已退出：从 {} 删掉了令牌，保留服务端地址\n", p.green("✓"), config::shown())
    } else {
        format!("{} {} 里没有令牌\n", p.dim("·"), config::shown())
    };
    Ok(Output::Text(head + &override_hint(p, &vars)))
}

/// `board check` / 不带 ID 的 `board beat` 的 JSON
fn agent_json(a: &AgentInfo) -> Value {
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
    })
}

/* ------------------------------ 输出 ------------------------------ */

struct Fmt<'a> {
    p: &'a Paint,
    label_name: &'a dyn Fn(i64) -> String,
    label_priority: &'a dyn Fn(i64) -> bool,
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

    /// 卡片上的优先标签（不带 ID 领取时这张先被领取）
    fn priority_labels(&self, c: &Card) -> Vec<String> {
        c.labels.iter().filter(|l| (self.label_priority)(**l)).map(|l| (self.label_name)(*l)).filter(|n| !n.is_empty()).collect()
    }

    fn card(&self, c: &Card) -> Value {
        json!({
            "id": c.id,
            "column": c.column,
            "title": c.title,
            "holder": c.holder,
            "labels": self.labels(c),
            "priority_labels": self.priority_labels(c),
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
            "priority_labels": self.priority_labels(c),
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

/* ------------------------------ board new 带附件 ------------------------------ */

/// `board new` 的 `--image` / `--file`（按命令行里的顺序）：读出来并按服务端的规则检查，返回 (文件名, 内容)。
/// 有一个不行就报错，什么也不上传；`--image` 只收图片。标题为空时不读文件（服务端会拒绝建卡，不留没用的附件）
fn new_files(a: &Args, no_title: bool) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = vec![];
    for (flag, f) in a.many.iter().filter(|(k, _)| k == "--image" || k == "--file") {
        if no_title {
            return Err(BoardError::new(ErrorCode::EmptyTitle, "标题不能为空"));
        }
        let bytes = std::fs::read(f).map_err(|e| BoardError::usage(format!("{flag}：读不到文件 {f}：{e}")))?;
        let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
        let (name, _, image) = board_core::files::check(&name, &bytes).map_err(|e| BoardError::usage(format!("{flag} {f}：{}", e.message)))?;
        if flag == "--image" && !image {
            return Err(BoardError::usage(format!("--image {f}：不是图片（PNG、JPEG、GIF、WebP）；其他文件用 --file")));
        }
        out.push((name, bytes));
    }
    Ok(out)
}

/* ------------------------------ 从文件读多行文字 ------------------------------ */

/// `--body` / `--comment` 的值：直接写的，或者 `--body-file` / `--comment-file` 从文件读的（两个只能给一个）。
/// Windows 上 skill 的入口 board.cmd 要经过 cmd.exe，参数里的换行会把整条命令截断在第一行（批处理里补不回来），
/// 多行内容只能这样传
fn text_arg(a: &Args, flag: &str, file_flag: &str) -> Result<Option<String>> {
    match (a.value(flag), file_arg(a, file_flag)?) {
        (Some(_), Some(_)) => Err(BoardError::usage(format!("{flag} 和 {file_flag} 只能给一个"))),
        (Some(v), None) => Ok(Some(v.to_string())),
        (None, t) => Ok(t),
    }
}

/// `--xxx-file <文件>` 的内容；`-` 读标准输入
fn file_arg(a: &Args, file_flag: &str) -> Result<Option<String>> {
    let Some(f) = a.value(file_flag) else { return Ok(None) };
    let bytes = if f == "-" {
        let mut b = vec![];
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut b).map_err(|e| BoardError::usage(format!("读不到标准输入：{e}")))?;
        b
    } else {
        std::fs::read(f).map_err(|e| BoardError::usage(format!("{file_flag}：读不到文件 {f}：{e}")))?
    };
    decode_text(&bytes).map(Some).map_err(|e| BoardError::usage(format!("{file_flag} {f}：{e}")))
}

/// 文件内容按 UTF-8 读：去掉 BOM；带 BOM 的 UTF-16（Windows PowerShell 5.1 的 `>`、Out-File 默认写成这样）也认；
/// 换行统一成 \n，别的字符原样保留
pub(crate) fn decode_text(b: &[u8]) -> std::result::Result<String, String> {
    let utf16 = |be: bool| {
        let units: Vec<u16> = b[2..].chunks(2).map(|c| if be { u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) } else { u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)]) }).collect();
        String::from_utf16(&units).map_err(|_| "不是合法的 UTF-16".to_string())
    };
    let s = match b {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).map_err(|_| "不是合法的 UTF-8".to_string())?,
        [0xFF, 0xFE, ..] => utf16(false)?,
        [0xFE, 0xFF, ..] => utf16(true)?,
        _ => String::from_utf8(b.to_vec()).map_err(|_| "不是 UTF-8 编码（另存为 UTF-8 再试）".to_string())?,
    };
    Ok(s.replace("\r\n", "\n"))
}

fn help(p: &Paint) -> String {
    let aw = COMMANDS.iter().map(|r| width(r.1)).max().unwrap_or(0) + 2;
    let mut out = vec![
        format!("{} {}", p.bold("board"), p.dim("— 看板式白板的命令行。人在界面上操作，agent 通过它读写同一块白板。")),
        String::new(),
    ];
    for (cmd, args, desc) in COMMANDS {
        out.push(format!("  board {} {}{}", p.cyan(&pad(cmd, 8)), p.dim(&pad(args, aw)), desc));
    }
    out.extend([
        String::new(),
        format!("  {}默认是 todo  doing  review  done  rework  check；每块白板可以自定义，用 board flow 查看  {}", p.dim("状态："), p.dim("（未整理：inbox）")),
        format!(
            "  {}先 board check --as <名字>（例如 claude/opus-5.5），之后每条命令都带 --as <名字>，或前缀 BOARD_AS=<名字>",
            p.dim("身份：")
        ),
        format!("  {}--board <项目>/<白板>，或前缀 BOARD=<项目>/<白板>", p.dim("白板：")),
        format!(
            "  {}board login --url <服务端地址> --token <令牌>，写进 {}（令牌在界面「设置 → Agent 接入」新建）；环境变量 BOARD_URL/BOARD_TOKEN 优先",
            p.dim("连接："),
            config::shown()
        ),
        format!("  {}所有命令都支持 --json；被拒绝时退出码为 1，并给出错误码", p.dim("输出：")),
        format!("  {}board --version", p.dim("版本：")),
        format!("  {}{}", p.dim("数据："), Backend::describe()),
        String::new(),
    ]);
    out.join("\n")
}

/* ------------------------------ 心跳 ------------------------------ */

/// `board heartbeat start | stop | final <ID>`（见 heartbeat.rs）
fn heartbeat_cmd(a: &Args, name: &str, json: bool, p: &Paint) -> Result<Output> {
    const USAGE: &str = "用法：board heartbeat start | stop | final <ID>";
    match a.pos.get(1).map(|s| s.as_str()) {
        Some("start") => {
            let s = heartbeat::start(name)?;
            Ok(if json {
                Output::Json(json!({
                    "name": name,
                    "loop_pid": s.loop_pid,
                    "agent_pid": s.agent.map(|x| x.pid),
                    "interval_ms": s.interval.as_millis() as u64,
                }))
            } else {
                Output::Text(format!(
                    "{} {} 心跳已开始：每 {} 秒一次（后台进程 {}{}）\n",
                    p.green("✓"),
                    p.bold(name),
                    s.interval.as_secs_f64(),
                    s.loop_pid,
                    match s.agent {
                        Some(x) => format!("，跟随 agent 进程 {}", x.pid),
                        None => "，没找到 agent 进程，会一直跑到 stop".to_string(),
                    }
                ))
            })
        }
        Some("stop") => {
            let info = heartbeat::stop(name)?;
            Ok(match (json, info) {
                (true, Some(info)) => Output::Json(agent_json(&info)),
                (true, None) => Output::Json(json!({ "name": name, "already_offline": true })),
                (false, Some(info)) => {
                    let cards: Vec<String> = info.cards.iter().map(|h| format!("{}#{}", h.board, h.card.id)).collect();
                    Output::Text(format!(
                        "{} {} 心跳已停止，已离线{}\n",
                        p.green("✓"),
                        p.bold(name),
                        if cards.is_empty() {
                            String::new()
                        } else {
                            p.dim(&format!("  持有的 {} 收不到心跳，{} 秒内释放", cards.join("、"), NAME_GAP_MS / 1000))
                        }
                    ))
                }
                (false, None) => Output::Text(format!("{} {} 心跳已停止（名字之前已经离线）\n", p.green("✓"), p.bold(name))),
            })
        }
        // 等同 board beat <ID> --final
        Some("final") => {
            let id = a.pos.get(2).ok_or_else(|| BoardError::usage("用法：board heartbeat final <ID>"))?;
            let mut t = vec!["beat".to_string(), id.clone(), "--final".to_string()];
            for (k, v) in &a.values {
                t.push(k.clone());
                t.push(v.clone());
            }
            if json {
                t.push("--json".into());
            }
            run(&t, p)
        }
        // 后台循环：只由 start 启动，不在 help 里列出
        Some("_loop") => {
            heartbeat::run_loop(name, a.pos.get(2).and_then(|s| heartbeat::Proc::parse(s)));
            Ok(Output::Text(String::new()))
        }
        _ => Err(BoardError::usage(USAGE)),
    }
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

pub(crate) struct Paint {
    on: bool,
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
    fn magenta(&self, s: &str) -> String {
        self.wrap("35", s)
    }
    pub(crate) fn cyan(&self, s: &str) -> String {
        self.wrap("36", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 命令表里的每个命令在 run() 里都有处理（help 在前面单独处理）
    #[test]
    fn every_command_is_dispatched() {
        let src = include_str!("main.rs");
        let body = &src[src.find("fn run(").unwrap()..src.find("fn help(").unwrap()];
        for (cmd, _, _) in COMMANDS {
            let handled = [format!("Some(\"{cmd}\")"), format!("sub == \"{cmd}\""), format!("\"{cmd}\" =>")].iter().any(|k| body.contains(k.as_str()));
            assert!(handled, "命令 {cmd} 在命令表里，但 run() 里没有处理");
        }
        let mut names: Vec<_> = COMMANDS.iter().map(|c| c.0).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), COMMANDS.len(), "命令表里有重复");
    }

    #[test]
    fn help_lists_every_command() {
        let h = help(&Paint { on: false });
        for (cmd, _, _) in COMMANDS {
            assert!(h.contains(&format!("board {cmd}")), "help 里没有 {cmd}");
        }
    }

    #[test]
    fn decode_text_handles_bom_utf16_and_crlf() {
        assert_eq!(decode_text("一\r\n\r\n二".as_bytes()).unwrap(), "一\n\n二");
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, b'a', b'\n']).unwrap(), "a\n");
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("中\r\nb".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        assert_eq!(decode_text(&le).unwrap(), "中\nb");
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("中".encode_utf16().flat_map(|u| u.to_be_bytes())).collect();
        assert_eq!(decode_text(&be).unwrap(), "中");
        assert!(decode_text(&[0xC4, 0xE3]).is_err(), "GBK 不当成 UTF-8 乱读");
    }

    #[test]
    fn version_flag_prints_cargo_version() {
        let p = Paint { on: false };
        let v = env!("CARGO_PKG_VERSION");
        for flag in ["--version", "-V"] {
            match run(&[flag.to_string()], &p) {
                Ok(Output::Text(t)) => assert_eq!(t, format!("board {v}")),
                _ => panic!("{flag} 应该打印版本号"),
            }
        }
        match run(&["--version".to_string(), "--json".to_string()], &p) {
            Ok(Output::Json(j)) => assert_eq!(j["version"], v),
            _ => panic!("--version --json 应该输出 JSON"),
        }
    }

    /// `board host rename` 缺新名字时报用法错（不会真的去常驻或改名）
    #[test]
    fn host_rename_requires_a_name() {
        let p = Paint { on: false };
        match run(&["host".into(), "rename".into()], &p) {
            Err(e) => assert!(e.message.contains("用法"), "{}", e.message),
            Ok(_) => panic!("缺新名字时应当报用法错"),
        }
    }
}
