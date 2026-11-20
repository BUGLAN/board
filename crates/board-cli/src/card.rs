//! 读写一张卡的命令：show、edit、move、claim、next、comment、beat <ID>、files，以及 start、submit / giveup（在 session.rs）。
//!
//! 它们不先取白板快照：服务端在卡片命令、读一张卡的返回里带上白板的工作流、仓库、时间和标签（`CardResp.board`），
//! 输出就用它画，一条命令一个请求（default/main#549；线上一个请求要 2–3 秒）。旧服务端的返回里没有，
//! 再取一次快照（`Backend::view`），输出一样，只是多一个请求。

use crate::args::{file_arg, text_arg, Args};
use crate::backend::{env, needs_labels, Backend, View};
use crate::output::{clock, human_size, saved_json, Fmt, Output, Paint};
use crate::session::{save_files, start_cmd, submit_cmd};
use board_core::*;
use serde_json::Value;

/// 读写一张卡的命令要的东西。`raw` 是命令行里写的白板（可以只写项目标识），服务端认
pub(crate) struct Ctx<'a> {
    pub(crate) a: &'a Args,
    pub(crate) sub: &'a str,
    pub(crate) json: bool,
    pub(crate) p: &'a Paint,
    pub(crate) raw: &'a str,
    pub(crate) name: &'a str,
    pub(crate) actor: &'a Actor,
    /// 服务端地址（卡片网址的前半）
    pub(crate) origin: &'a str,
}

impl Ctx<'_> {
    /// 旧服务端补取快照时要不要标签
    pub(crate) fn labels(&self) -> bool {
        needs_labels(self.sub, self.json)
    }
}

/// 用 `v` 画输出：卡片的网址、标签名、状态名都按它
pub(crate) fn render<T>(v: &View, origin: &str, p: &Paint, f: impl FnOnce(&Fmt) -> T) -> T {
    let label_name = |id: i64| v.labels.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_default();
    let label_priority = |id: i64| v.labels.iter().any(|l| l.id == id && l.priority);
    let url_of = |id: i64| format!("{origin}/{}/{id}", v.path);
    let fmt = Fmt { p, label_name: &label_name, label_priority: &label_priority, path: &v.path, url_of: &url_of, now: v.now, flow: &v.flow };
    f(&fmt)
}

pub(crate) fn run(c: &Ctx, be: &mut Backend, started: &Option<(String, bool)>) -> Result<Output> {
    let Ctx { a, sub, json, p, raw, name, actor, origin } = *c;
    // 写命令：执行，用返回画「已更新」这类一行字，或整张卡的 JSON
    let wrote = |be: &mut Backend, cmd: Cmd, line: &dyn Fn(&CardDetail, &View, &Fmt) -> String| -> Result<Output> {
        let d = be.exec_full(raw, actor, cmd)?;
        let v = be.view(raw, &d, c.labels())?;
        Ok(render(&v, origin, p, |fmt| if json { Output::Json(fmt.full(&d)) } else { Output::Text(line(&d.detail, &v, fmt)) }))
    };
    match sub {
        "show" => {
            let id = a.id(sub)?;
            let d = be.card(raw, id)?;
            let v = be.view(raw, &d, c.labels())?;
            Ok(render(&v, origin, p, |fmt| if json { Output::Json(fmt.full(&d)) } else { Output::Text(fmt.show(&d)) }))
        }
        "edit" => {
            let id = a.id(sub)?;
            let body = text_arg(a, "--body", "--body-file")?;
            if a.value("--title").is_none() && body.is_none() {
                return Err(BoardError::usage("board edit 需要 --title、--body 或 --body-file"));
            }
            wrote(be, Cmd::Edit { id, title: a.value("--title").map(|s| s.to_string()), body }, &|d, _, fmt| {
                format!("{} 已更新 {}{}\n", p.green("✓"), p.cyan(&format!("#{}", d.card.id)), fmt.renewed(d, name))
            })
        }
        "move" => {
            let id = a.id(sub)?;
            let col = a.pos.get(2).ok_or_else(|| BoardError::usage("board move 需要 <状态>"))?;
            let top = a.flag("--top");
            wrote(be, Cmd::Move { id, column: col.clone(), top, index: None, order: None, before: None, after: None }, &|d, v, _| {
                format!(
                    "{} {} → {} ({}){}\n",
                    p.green("✓"),
                    p.cyan(&format!("#{}", d.card.id)),
                    v.flow.name(&d.card.column),
                    d.card.column,
                    p.dim(if top { "  最上面" } else { "" })
                )
            })
        }
        "claim" => {
            let id = if a.pos.len() > 1 { Some(a.id(sub)?) } else { None };
            wrote(be, Cmd::Claim { id }, &|d, v, fmt| {
                format!(
                    "{} {}  {}\n{}\n\n{}",
                    p.green("✓"),
                    p.bold("已领取"),
                    p.dim(&format!("进入「{}」", v.flow.name(&d.card.column))),
                    p.yellow(&format!(
                        "后台心跳会维持这张卡片；做完用 board submit {}，做不下去用 board giveup {}",
                        d.card.id,
                        d.card.id
                    )),
                    fmt.show(d)
                )
            })
        }
        // board next：只在 board host 启动的会话里（环境里有 BOARD_HOST），由服务端结算上一张、领下一张
        "next" => {
            if env("BOARD_HOST").is_none() {
                return Err(BoardError::usage("board next 只能在 board host 启动的会话里用（环境里没有 BOARD_HOST）"));
            }
            let d = be.next(name)?;
            let v = be.view(raw, &d, c.labels())?;
            Ok(render(&v, origin, p, |fmt| {
                if json {
                    Output::Json(fmt.full(&d))
                } else {
                    Output::Text(format!(
                        "{} {}  {}\n{}\n\n{}",
                        p.green("✓"),
                        p.bold("接着做下一张"),
                        p.dim(&format!("已领取，进入「{}」", v.flow.name(&d.card.column))),
                        p.yellow(&format!("上一张的派发已结算；这张做完后 board submit {}（做不下去 board giveup {}），再 board next", d.card.id, d.card.id)),
                        fmt.show(&d)
                    ))
                }
            }))
        }
        "comment" => {
            let id = a.id(sub)?;
            let text = match file_arg(a, "--comment-file")? {
                Some(_) if a.pos.len() > 2 => return Err(BoardError::usage("评论内容和 --comment-file 只能给一个")),
                Some(t) => t,
                None => a.pos[2..].join(" "),
            };
            wrote(be, Cmd::Comment { id, text }, &|d, _, fmt| format!("{} 已评论 {}{}\n", p.green("✓"), p.cyan(&format!("#{}", d.card.id)), fmt.renewed(d, name)))
        }
        "beat" => {
            let id = a.id(sub)?;
            let done = a.flag("--final");
            wrote(be, Cmd::Beat { id, done }, &|d, _, fmt| {
                if done {
                    format!(
                        "{} {} 心跳已结束{}\n",
                        p.green("✓"),
                        p.cyan(&format!("#{}", d.card.id)),
                        if d.card.runs > 0 { p.dim(&format!("  累计 {}（{} 次）", clock(d.card.work_ms), d.card.runs)) } else { String::new() }
                    )
                } else {
                    format!("{} {} 心跳{}\n", p.green("✓"), p.cyan(&format!("#{}", d.card.id)), fmt.renewed(d, name))
                }
            })
        }
        // 附件下载：输出里用不到工作流，不画
        "files" => {
            let id = a.id(sub)?;
            let d = be.card(raw, id)?;
            let (dir, saved) = save_files(be, &d, a.value("--out"))?;
            if json {
                Ok(Output::Json(Value::Array(saved.iter().map(|(f, to)| saved_json(f, to)).collect())))
            } else if saved.is_empty() {
                Ok(Output::Text(format!("{}\n", p.dim(&format!("#{id} 没有附件")))))
            } else {
                let mut s = format!("{} 已下载 {} 个附件到 {}\n", p.green("✓"), saved.len(), dir.as_ref().unwrap().display());
                for (f, to) in &saved {
                    s += &format!("  {}  {}\n", to.display(), p.dim(&human_size(f.size)));
                }
                Ok(Output::Text(s))
            }
        }
        "start" => start_cmd(c, be, started),
        "giveup" | "submit" => submit_cmd(c, be),
        other => Err(BoardError::usage(format!("命令「{other}」还没有实现"))),
    }
}
