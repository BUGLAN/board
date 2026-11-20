//! 会话和名字：check、board start 占用名字并启动后台心跳、board heartbeat；
//! board start / submit / giveup 这三条命令

use crate::args::{text_arg, Args};
use crate::backend::{env, Backend, Want};
use crate::card::{render, Ctx};
use crate::output::{agent_json, attachment_json, human_size, saved_json, Output, Paint};
use crate::{config, heartbeat, run};
use board_core::*;
use serde_json::{json, Value};

/// `board start`：领取（board host 启动的会话只读、不领取）、下载附件，输出卡片和交审、做不下去时去哪。
/// 不先取白板：工作流、仓库在领取的返回里（见 card.rs）
pub(crate) fn start_cmd(c: &Ctx, be: &mut Backend, started: &Option<(String, bool)>) -> Result<Output> {
    let Ctx { a, sub, json, p, raw, name, actor, origin } = *c;
    let id = if a.pos.len() > 1 { Some(a.id(sub)?) } else { None };
    // board host 启动的会话：卡片由 board host 领好了，这里只读、绝不领取（不归自己持有的卡不能顺手领走）
    let (d, resumed) = if env("BOARD_HOST").is_some() {
        let Some(id) = id else {
            return Err(BoardError::usage("board host 启动的会话里 board start 要带 <ID>（提示词里的那张）；接着做下一张用 board next"));
        };
        let d = be.card(raw, id)?;
        if d.card.holder.as_deref() != Some(name) {
            let v = be.view(raw, &d, false)?;
            return Err(BoardError::new(
                ErrorCode::NotHolder,
                format!(
                    "#{id} 现在在「{}」，{}：不要动这张卡片，评论说明后结束（board host 启动的会话不领取卡片）",
                    v.flow.name(&d.card.column),
                    match &d.card.holder {
                        Some(h) => format!("由 {h} 持有"),
                        None => "没人持有".to_string(),
                    }
                ),
            ));
        }
        (d, true)
    } else {
        match be.exec_full(raw, actor, Cmd::Claim { id }) {
            // 领取自己已经持有的卡片照样成功：服务端说这次是不是新领取的（旧服务端没说，按有没有开始过计时、领取之后有没有动态认）
            Ok(d) => {
                let resumed = match &d.board {
                    Some(b) => !b.claimed_now,
                    None => d.card.run_started_at.is_some() || d.activity.iter().any(|x| Some(x.at) > d.card.claimed_at),
                };
                (d, resumed)
            }
            Err(e) if matches!(e.code, ErrorCode::NotInTodo | ErrorCode::AlreadyHolding) => {
                let mine = id.and_then(|id| be.card(raw, id).ok()).filter(|d| d.card.holder.as_deref() == Some(name));
                match mine {
                    Some(d) => (d, true),
                    None => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    };
    let v = be.view(raw, &d, c.labels())?;
    let (dir, saved) = save_files(be, &d, a.value("--out"))?;
    let fresh = started.as_ref().is_some_and(|(_, fresh)| *fresh);
    // 后台心跳跟着 agent 进程：认不出进程时它会一直跑到 heartbeat stop（board host 启动的会话心跳由 board host 发，不用管）
    let hosted = env("BOARD_HOST").is_some();
    let follows = hosted || heartbeat::follows(name);
    let review = v.flow.review_after(&d.card.column);
    let over = v.flow.giveup_after(&d.card.column);
    let (repo, branch) = (&v.repo, &v.branch);
    Ok(render(&v, origin, p, |fmt| {
        if json {
            let role = |s: Option<&Status>| s.map(|s| json!({ "key": s.key, "name": s.name })).unwrap_or(Value::Null);
            let mut out = fmt.full(&d);
            out["session"] = json!({ "name": name, "started": fresh, "resumed": resumed, "follows": follows });
            out["review"] = role(review);
            out["giveup"] = role(over);
            out["repo"] = json!(repo);
            out["branch"] = json!(branch);
            out["files"] = Value::Array(saved.iter().map(|(f, to)| saved_json(f, to)).collect());
            Output::Json(out)
        } else {
            let cid = d.card.id;
            let to = |s: Option<&Status>, none: &str| s.map(|s| format!("→ {}（{}）", s.name, s.key)).unwrap_or_else(|| none.to_string());
            let warn = if fresh && !follows {
                format!("\n{}", p.yellow("没找到 agent 进程：后台心跳会一直跑到 board heartbeat stop，会话结束时一定要执行；这台机器上同时只能开一个这样的会话"))
            } else {
                String::new()
            };
            let mut s = format!(
                "{} {}  {}\n{}{warn}\n\n{}\n",
                p.green("✓"),
                p.bold(if resumed { "接着做" } else { "已领取" }),
                p.dim(&format!(
                    "在「{}」；名字 {}（{}）",
                    v.flow.name(&d.card.column),
                    name,
                    if hosted {
                        "心跳由 board host 发"
                    } else if fresh {
                        "刚占用，后台心跳已启动"
                    } else {
                        "接着用，后台心跳在跑"
                    }
                )),
                p.yellow("后台心跳会维持这张卡片，干多久都不会被释放；做完用 board submit，做不下去用 board giveup"),
                fmt.show(&d)
            );

            s += &format!("{}  board submit {cid} --comment-file <文件>  {}\n", p.dim("交审      "), to(review, "这块白板没有交审状态：用 board giveup"));
            s += &format!(
                "{}  board giveup {cid} --comment-file <文件>  {}\n",
                p.dim("做不下去  "),
                to(over, "没有错误节点和需要人处理的状态：只评论、结束计时，然后告诉用户")
            );
            s += &format!(
                "{}  {}  {} {}\n",
                p.dim("集成分支  "),
                if branch.is_empty() { "（没设：按仓库的 AGENTS.md / CLAUDE.md，没写用远端默认分支）" } else { branch.as_str() },
                p.dim("仓库"),
                if repo.is_empty() { "（没设）" } else { repo.as_str() }
            );
            if !saved.is_empty() {
                s += &format!("{}  已下载 {} 个到 {}\n", p.dim("附件      "), saved.len(), dir.as_ref().unwrap().display());
                for (f, to) in &saved {
                    s += &format!("            {}  {}\n", to.display(), p.dim(&human_size(f.size)));
                }
            }
            Output::Text(s)
        }
    }))
}

/// `board submit` / `board giveup`：上传附件、评论、结束计时、移到交审状态或错误节点。
/// 不先取白板：工作流在返回里（见 card.rs）；交审评论缺固定的四行时才取一次，看项目有没有设代码仓库
pub(crate) fn submit_cmd(c: &Ctx, be: &mut Backend) -> Result<Output> {
    let Ctx { a, sub, json, p, raw, name, actor, origin } = *c;
    let giveup = sub == "giveup";
    let id = a.id(sub)?;
    let what = if giveup { "原因" } else { "交审评论" };
    let Some(mut text) = text_arg(a, "--comment", "--comment-file")?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return Err(BoardError::usage(format!("board {sub} 要写{what}：--comment \"<{what}>\" 或 --comment-file <文件>")));
    };
    // 改了代码的卡片：交审评论最后固定四行，复盘的人靠它们找提交、看检查（项目设了代码仓库才查）
    if !giveup && !a.flag("--no-code") {
        let missing = missing_review_lines(&text);
        if !missing.is_empty() && !be.board(raw, Want::Brief, false)?.repo.is_empty() {
            return Err(BoardError::usage(format!(
                "交审评论缺这几行：{}。改了代码的卡片，评论最后要有「提交：<sha>」「集成分支：<分支>」「检查：<结果>」「碰到的别的卡：<board touched 的最后一行>」四行；这张卡没改代码时加 --no-code",
                missing.iter().map(|m| format!("「{m}：」")).collect::<Vec<_>>().join("")
            )));
        }
    }
    // 附件先全部读进来：有一个读不到就不上传、不评论
    let mut blobs = vec![];
    for f in &a.pos[2..] {
        let bytes = std::fs::read(f).map_err(|e| BoardError::usage(format!("读不到文件 {f}：{e}")))?;
        let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
        blobs.push((name, bytes));
    }
    let mut done: Vec<Attachment> = vec![];
    for (fname, bytes) in &blobs {
        let f = be.attach(raw, id, actor, fname, bytes)?;
        text += &format!("\n\n{}", f.markdown);
        done.push(f);
    }
    let mut brief = None;
    let d = {
        let be = std::cell::RefCell::new(&mut *be);
        hand_over(
            name,
            id,
            text,
            giveup,
            |cmd| {
                let r = be.borrow_mut().exec_full(raw, actor, cmd)?;
                brief = r.board;
                Ok(r.detail)
            },
            |id| be.borrow_mut().card(raw, id).map(|r| r.detail),
            || be.borrow_mut().board(raw, Want::Brief, false).map(|b| b.flow),
        )?
    };
    let d = CardResp { detail: d, board: brief };
    let v = be.view(raw, &d, c.labels())?;
    let moved = !v.flow.is_lock(&d.card.column);
    Ok(render(&v, origin, p, |fmt| {
        if json {
            let mut out = fmt.full(&d);
            out["moved"] = json!(moved);
            out["uploaded"] = Value::Array(done.iter().map(attachment_json).collect());
            Output::Json(out)
        } else {
            let files = if done.is_empty() { String::new() } else { p.dim(&format!("  带 {} 个附件", done.len())) };
            Output::Text(if moved {
                format!(
                    "{} {} {} → {}（{}）{}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{id}")),
                    if giveup { "已交给人处理" } else { "已交审" },
                    v.flow.name(&d.card.column),
                    d.card.column,
                    files
                )
            } else {
                format!(
                    "{} {} 已评论并结束计时{}\n{}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{id}")),
                    files,
                    p.yellow(&format!(
                        "这块白板没有错误节点和需要人处理的状态，卡片还在「{}」：{} 秒后领取释放、回到领取前的状态。停下来告诉用户",
                        v.flow.name(&d.card.column),
                        BEAT_MS * BEAT_MISSES / 1000
                    ))
                )
            })
        }
    }))
}

/* ------------------------------ 名字 ------------------------------ */

/// `--as <名字>` 或 `BOARD_AS`：写了就自己起名字（照旧，被占用时 NAME_TAKEN，不加序号）
pub(crate) fn explicit_name(a: &Args) -> Option<String> {
    a.value("--as").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).or_else(|| env("BOARD_AS"))
}

/// 这条 `board start` 用的名字和是不是这一次新占用的（[`start_session`] 成功后由 run() 记下）：之后领取失败时，
/// 报错里也写上它（名字已经在线、后台心跳已经在跑，不再领卡时要 `board heartbeat stop`），见 main()
pub(crate) static SESSION: std::sync::OnceLock<(String, bool)> = std::sync::OnceLock::new();

/// `board start` 的第一步：让这个会话有一个在线的名字和后台心跳。返回（名字，这一次是不是新占用的）。
/// - board host 启动的会话（环境里有 `BOARD_HOST`）：名字是 `BOARD_AS`，心跳由 board host 发，这里什么也不做；
/// - 这个 agent 进程已经 check 过、后台心跳还在跑：接着用；
/// - 否则占用名字（`--agent <标识>` 由 Board 生成，或 `--as <名字>` 自己起）并启动后台心跳
pub(crate) fn start_session(a: &Args, be: &mut Backend) -> Result<(String, bool)> {
    if env("BOARD_HOST").is_some() {
        return explicit_name(a).map(|n| (n, false)).ok_or_else(|| BoardError::code(ErrorCode::NoIdentity));
    }
    let (name, session, agent, host) = match explicit_name(a) {
        Some(name) => {
            if heartbeat::is_live(&name) {
                return Ok((name, false));
            }
            check_agent_name(&name).map_err(BoardError::usage)?;
            let info = be.check(&name)?;
            (name, info.session, None, None)
        }
        None => {
            let agent = agent_id(a);
            if let Some(name) = heartbeat::live_name(agent.as_deref()) {
                return Ok((name, false));
            }
            let Some(agent) = agent else {
                return Err(BoardError::usage(
                    "board start 在会话里第一次用时要 --agent <标识>（你实际运行的 agent：claude、codex、dsh …），名字由 Board 生成；也可以 --as <名字> 自己起名字",
                ));
            };
            let info = generated_check(be, &agent)?;
            let host = if info.modeled.is_some() { config::host_name() } else { None };
            (info.name.clone(), info.session, Some(agent), host)
        }
    };
    heartbeat::checked_with(&name, session, agent, host)?;
    heartbeat::start(&name, false)?;
    Ok((name, true))
}

/// 交审评论里固定的四行（work.md「处理一张卡片」第 5 步）缺了哪几行。一行以「提交：」这样开头才算，冒号全角、半角都行
fn missing_review_lines(text: &str) -> Vec<&'static str> {
    const LINES: [&str; 4] = ["提交", "集成分支", "检查", "碰到的别的卡"];
    let has = |key: &str| {
        text.lines().any(|l| {
            let l = l.trim().trim_start_matches(['-', '*', ' ']);
            l.strip_prefix(key).is_some_and(|rest| rest.starts_with('：') || rest.starts_with(':'))
        })
    };
    LINES.into_iter().filter(|k| !has(k)).collect()
}

/// `board submit` / `board giveup`：服务端一条命令做完评论、final、移动。旧服务端不认这两条命令
/// （「请求格式不正确：unknown variant」）时在这里拆成三步做：规则一样，只是不在一次写入里
fn hand_over(
    name: &str,
    id: i64,
    text: String,
    giveup: bool,
    mut exec: impl FnMut(Cmd) -> Result<CardDetail>,
    mut card: impl FnMut(i64) -> Result<CardDetail>,
    mut flow_of: impl FnMut() -> Result<Flow>,
) -> Result<CardDetail> {
    let cmd = if giveup { Cmd::GiveUp { id, text: text.clone() } } else { Cmd::Submit { id, text: text.clone() } };
    match exec(cmd) {
        Err(e) if e.code == ErrorCode::Usage && e.message.contains("unknown variant") => {}
        r => return r,
    }
    let cur = card(id)?.card;
    let flow = flow_of()?;
    if cur.holder.as_deref() != Some(name) {
        let msg = match &cur.holder {
            Some(h) => format!("#{id} 由 {h} 持有，你不是持有者"),
            None => format!("#{id} 没有被领取：先 board start {id}"),
        };
        return Err(BoardError::new(ErrorCode::NotHolder, msg));
    }
    let target = if giveup { flow.giveup_after(&cur.column) } else { flow.review_after(&cur.column) }.map(|s| s.key.clone());
    if !giveup && target.is_none() {
        return Err(BoardError::new(
            ErrorCode::ForbiddenColumn,
            format!(
                "#{id} 在「{}」，没有连到交审状态（名称里带「审核」的状态），能去：{}。要交给人处理用 board giveup {id}",
                flow.name(&cur.column),
                flow.outs(&cur.column)
            ),
        ));
    }
    exec(Cmd::Comment { id, text })?;
    let d = exec(Cmd::Beat { id, done: true })?;
    match target {
        Some(column) => exec(Cmd::Move { id, column, top: false, index: None, order: None, before: None, after: None }),
        None => Ok(d),
    }
}

/// 把卡片的附件下载到本地：给了目录就放进去，没给时建一个只有自己能读的临时目录。返回（目录，[(附件, 路径)]）；没有附件时目录为空
pub(crate) fn save_files(be: &mut Backend, d: &CardDetail, out: Option<&str>) -> Result<(Option<std::path::PathBuf>, Vec<(Attachment, std::path::PathBuf)>)> {
    if d.attachments.is_empty() {
        return Ok((None, vec![]));
    }
    let dir = match out {
        Some(o) => {
            let dir = std::path::PathBuf::from(o);
            std::fs::create_dir_all(&dir).map_err(|e| BoardError::usage(format!("无法创建目录 {}：{e}", dir.display())))?;
            dir
        }
        None => {
            let t = tempfile::Builder::new()
                .prefix("board-files-")
                .tempdir_in(std::env::temp_dir())
                .map_err(|e| BoardError::usage(format!("无法创建临时目录：{e}")))?;
            board_core::private::set_dir(t.path()).map_err(|e| BoardError::usage(format!("无法收紧目录 {} 的权限：{e}", t.path().display())))?;
            t.keep()
        }
    };
    let mut saved = vec![];
    for f in &d.attachments {
        let bytes = be.download(f)?;
        let to = dir.join(format!("{}-{}", f.id, f.name));
        let written = if out.is_some() { std::fs::write(&to, bytes) } else { board_core::private::write_new(&to, &bytes) };
        written.map_err(|e| BoardError::usage(format!("写入 {} 失败：{e}", to.display())))?;
        saved.push((f.clone(), to));
    }
    Ok((Some(dir), saved))
}

/// `--agent <标识>` 或 `BOARD_AGENT`：这个会话跑在哪个 agent 里（skill 按自己实际运行的 agent 填）
pub(crate) fn agent_id(a: &Args) -> Option<String> {
    a.value("--agent").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).or_else(|| env("BOARD_AGENT"))
}

/// `board check [--agent <标识>] [--as <名字>]`：占用名字。返回里 `name` 是实际用的名字（不带名字时是 Board 生成的）
pub(crate) fn check(a: &Args, be: &mut Backend, json: bool, p: &Paint) -> Result<Output> {
    let info = match explicit_name(a) {
        Some(name) => {
            check_agent_name(&name).map_err(BoardError::usage)?;
            be.check(&name)?
        }
        None => match agent_id(a) {
            Some(agent) => generated_check(be, &agent)?,
            None => {
                return Err(BoardError::usage(
                    "board check 要 --agent <标识>（名字由 Board 生成，例如 board check --agent dsh）或 --as <名字>（自己起名字）",
                ))
            }
        },
    };
    let name = info.name.clone();
    // 记下这个名字属于调用它的 agent 进程和服务端发的会话标识：之后的心跳带上它，别的会话停不掉。
    // 名字由 Board 生成时连着 agent 标识、机器一起记下，以后换名字了还按它算
    heartbeat::checked_with(&name, info.session.clone(), agent_id(a), if info.modeled.is_some() { config::host_name() } else { None })?;
    if !json {
        return Ok(Output::Text(format!(
            "{} {} 已在线\n{}\n",
            p.green("✓"),
            p.bold(&name),
            p.yellow(&format!(
                "马上启动心跳：board heartbeat start（后台每 {} 秒发一次心跳；{} 秒没收到就离线）",
                BEAT_MS / 1000,
                NAME_GAP_MS / 1000
            ))
        )));
    }
    Ok(Output::Json(agent_json(&info)))
}

/// `board check --agent <标识>`：名字由 Board 生成。
///
/// 新版服务端按机器名 / 令牌认机器、按机器设置生成名字（占用时加序号）。旧服务端不认 `agent`（报 `NO_IDENTITY`）：
/// 先问「这个令牌是哪台机器的」（`GET /api/hosts/mine`）拿到机器设置，自己按同样的规则拼名字再带名字 check
/// （带了机器名，让服务端也按这个名字生成；连着名都不认的旧服务端直接 check 拼出来的名字，占用时自己加序号）。
/// 问不出机器时退回 `<agent 标识>/<模型>`
fn generated_check(be: &mut Backend, agent: &str) -> Result<AgentInfo> {
    board_core::hosts::check_agent_id(agent).map_err(BoardError::usage)?;
    let host = config::host_name();
    let mut info = match be.check_generated(agent, host.as_deref()) {
        Ok(i) => i,
        Err(e) if e.code == ErrorCode::NoIdentity => {
            let mine = be.my_host()?;
            let (name, modeled, query) = match mine {
                Some(h) => {
                    // 名字第二段：机器设置里选的模型、强度 → board host 上报的 → agent 标识（和派发、新版服务端一样）
                    let a = h.agents.iter().find(|a| a.id == agent);
                    let set = a.and_then(|a| a.run_model.clone().filter(|_| a.opts.as_ref().is_some_and(|o| o.model)));
                    let modeled = set.is_some();
                    let model = set.or_else(|| a.map(|a| a.model.clone())).unwrap_or_default();
                    let effort = a.and_then(|a| a.run_effort.clone()).or_else(|| a.map(|a| a.effort.clone())).unwrap_or_default();
                    (board_core::hosts::dispatch_agent_name(agent, &h.name, &model, &effort, 0), modeled, Some(h.name))
                }
                None => {
                    let (name, modeled) = board_core::hosts::plain_agent_name(agent, "");
                    (name, modeled, None)
                }
            };
            // 再问一次：这一次知道是哪台机器了（旧服务端两次都报 NO_IDENTITY，就自己拼名字）
            let info = match be.check_generated(agent, query.as_deref()) {
                Ok(i) => i,
                Err(e) if e.code == ErrorCode::NoIdentity => {
                    // 旧服务端：带名字 check；被占用时在末尾加 -1、-2 …（和派发、新版服务端一样）
                    let (first, seg) = name.split_once('/').map_or((name.clone(), String::new()), |(f, s)| (f.to_string(), s.to_string()));
                    let mut seq = 0u32;
                    loop {
                        let n = if seq == 0 { name.clone() } else { format!("{first}/{seg}-{seq}") };
                        check_agent_name(&n).map_err(BoardError::usage)?;
                        match be.check(&n) {
                            Ok(i) => break i,
                            Err(e) if e.code == ErrorCode::NameTaken => seq += 1,
                            Err(e) => return Err(e),
                        }
                    }
                }
                Err(e) => return Err(e),
            };
            let mut info = info;
            info.modeled = Some(modeled);
            info
        }
        Err(e) => return Err(e),
    };
    info.modeled.get_or_insert(false);
    Ok(info)
}

/// 带名字的命令用的名字是本机另一个还活着的 agent 进程 check 的：提醒的话

/// 本机记下的名字（`board check --agent <标识>` 生成的）不只属于某个进程时，其他命令也可以用它。
/// 用了别人 check 的名字时的提醒（--json 里加 warning 字段）
pub(crate) fn borrowed(tokens: &[String]) -> Option<String> {
    let a = Args::parse(tokens).ok()?;
    let sub = a.pos.first()?.as_str();
    if a.flag("--help") || a.flag("-h") || a.flag("--version") || a.flag("-V") {
        return None;
    }
    if matches!(sub, "check" | "heartbeat" | "login" | "logout" | "update" | "doctor" | "help" | "host" | "dispatch" | "skills" | "versions" | "touched")
        || (sub == "beat" && a.pos.len() == 1)
    {
        return None;
    }
    let name = explicit_name(&a).or_else(heartbeat::current_name)?;
    heartbeat::borrowed_warning(&name)
}

/* ------------------------------ 心跳 ------------------------------ */

/// `board heartbeat start | stop | final <ID>`（见 heartbeat.rs）
pub(crate) fn heartbeat_cmd(a: &Args, name: &str, json: bool, p: &Paint) -> Result<Output> {
    const USAGE: &str = "用法：board heartbeat start | stop | final <ID>";
    match a.pos.get(1).map(|s| s.as_str()) {
        Some("start") => {
            let s = heartbeat::start(name, a.flag("--force"))?;
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
            let info = heartbeat::stop(name, a.flag("--force"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 交审评论固定的四行：缺哪几行；冒号全角半角都认，前面有列表符号也认，写在句子中间的不算
    #[test]
    fn review_lines_are_checked_by_line_start() {
        let ok = "改了登录页的校验。\n\n提交：abc1234\n集成分支：main\n- 检查: npm test 通过\n碰到的别的卡：无";
        assert!(missing_review_lines(ok).is_empty());
        assert_eq!(missing_review_lines("做完了"), vec!["提交", "集成分支", "检查", "碰到的别的卡"]);
        assert_eq!(missing_review_lines("提交：abc\n集成分支：main\n检查：过了"), vec!["碰到的别的卡"]);
        assert_eq!(missing_review_lines("这次的提交：abc，检查：过了\n集成分支：main\n碰到的别的卡：无"), vec!["提交", "检查"]);
    }

    /// board submit / giveup 连旧服务端（不认这两条命令）：拆成评论、final、移动三步，规则和新服务端一样
    #[test]
    fn hand_over_falls_back_to_three_steps_on_old_servers() {
        use std::cell::RefCell;
        const T0: i64 = 1_700_000_000_000;
        let store = RefCell::new({
            let mut s = Store::open_in_memory().unwrap();
            s.ensure_default(T0).unwrap();
            s
        });
        let me = Actor::agent("test/alpha");
        let run = |who: &Actor, cmd: Cmd| {
            store.borrow_mut().write(T0, Origin::cli(), |w| {
                let b = w.resolve("default/main")?;
                w.exec(&b, who, cmd)
            })
        };
        let sent = RefCell::new(Vec::<&'static str>::new());
        // 旧服务端：submit、giveup 解析不了
        let old = |cmd: Cmd| {
            let op = match &cmd {
                Cmd::Submit { .. } => "submit",
                Cmd::GiveUp { .. } => "giveup",
                Cmd::Comment { .. } => "comment",
                Cmd::Beat { .. } => "beat",
                Cmd::Move { .. } => "move",
                _ => "other",
            };
            sent.borrow_mut().push(op);
            if matches!(cmd, Cmd::Submit { .. } | Cmd::GiveUp { .. }) {
                return Err(BoardError::usage("请求格式不正确：unknown variant `submit`, expected one of `new`, `edit`"));
            }
            run(&me, cmd)
        };
        let card = |id: i64| {
            store.borrow().read(|r| {
                let b = r.resolve("default/main")?;
                r.card(&b, id)
            })
        };
        let flow = store.borrow().read(|r| r.flow(&r.resolve("default/main")?)).unwrap();
        let new = |t: &str| Cmd::New { title: t.into(), body: None, column: None, top: false, labels: vec![], from: None };
        run(&Actor::human(), new("一")).unwrap();
        run(&Actor::human(), new("二")).unwrap();

        // 不是持有者：不评论
        let e = hand_over("test/alpha", 1, "做完了".into(), false, old, card, || Ok(flow.clone())).unwrap_err();
        assert_eq!(e.code, ErrorCode::NotHolder);
        assert_eq!(*sent.borrow(), vec!["submit"]);

        run(&me, Cmd::Claim { id: Some(1) }).unwrap();
        sent.borrow_mut().clear();
        let d = hand_over("test/alpha", 1, "做完了".into(), false, old, card, || Ok(flow.clone())).unwrap();
        assert_eq!((d.card.column.as_str(), d.card.holder.as_deref()), ("review", None));
        assert_eq!(*sent.borrow(), vec!["submit", "comment", "beat", "move"]);

        run(&me, Cmd::Claim { id: Some(2) }).unwrap();
        let d = hand_over("test/alpha", 2, "卡住了".into(), true, old, card, || Ok(flow.clone())).unwrap();
        assert_eq!(d.card.column, "check");
        assert!(d.activity.iter().any(|a| a.action == "comment" && a.text == "卡住了"));

        // 别的错误原样返回，不走三步
        sent.borrow_mut().clear();
        let e = hand_over("test/alpha", 1, "x".into(), false, |_| Err(BoardError::code(ErrorCode::NotHolder)), card, || Ok(flow.clone())).unwrap_err();
        assert_eq!(e.code, ErrorCode::NotHolder);
        assert!(sent.borrow().is_empty());
    }
}
