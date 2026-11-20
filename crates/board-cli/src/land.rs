//! `board land <ID>`：把推送里机械的几步按顺序做完（default/main#553）。
//!
//! skill「代码 → 推送」原来要 agent 一条条照着做：压成一个提交、`board touched`、`board hold`、推集成分支、删 wip 分支。
//! 顺序错了后果很重（没 hold 就推，集成分支上会有同一张卡的两份改动），这里按顺序做完，有一步不行就停在那一步：
//!
//! 1. **前提**（不满足就报错，什么都不改）：工作区没有没提交的改动；`git fetch origin` 之后 HEAD 在最新的
//!    `origin/<集成分支>` 之上；相对它有改动。rebase 和跑检查不在这里做：冲突要 agent 读懂两边再合，检查是仓库自己规定的。
//! 2. **压成一个提交**：`git reset --soft origin/<集成分支>` 后用 `--message-file` 的内容提交，最后一行补上
//!    `卡片：<项目>/<白板>#<ID>`。已经是压好的那一个（上一次停在了 touched）时不重复压。
//! 3. **`board touched`**：碰到别的卡时退出码 1、不推；确认是有意的之后加 `--touched-ok` 重来。
//! 4. **`board hold`**：领取已经失效（`NOT_HOLDER`）、连不上服务端（`UNREACHABLE`）时不推，原样报错。
//! 5. **推集成分支**：被拒（别人先推了）时退出码 1，本地压好的提交留着；不 `--force`、不自动 rebase（rebase 之后要重新跑检查）。
//! 6. **删 wip 分支**：卡片评论里有 `未完成的改动：wip/…`、远端还有它（这张卡是从它接着做的）时删掉。
//! 7. 输出交审评论最后的四行。
//!
//! `--wip`（做不下去）：只把没提交的改动提交、推到 `wip/<白板标识>/<ID>`（已经有时覆盖），输出 `未完成的改动：<分支>` 一行；
//! 不查 touched、不 hold、不碰集成分支。

use crate::args::{file_arg, Args};
use crate::backend::{env, Backend, Want};
use crate::code::{git, git_output, integration_branch, is_repo, unpushed};
use crate::output::{Output, Paint};
use crate::touched;
use crate::versions::card_refs;
use board_core::*;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

/// 本地的 git 命令（status、rev-parse 这类）最多等多久
const QUICK: Duration = Duration::from_secs(30);
/// 连远端（fetch、push）最多等多久
const NET: Duration = Duration::from_secs(300);
/// 提交（可能跑仓库的钩子）最多等多久
const COMMIT: Duration = Duration::from_secs(600);

pub fn run(a: &Args, name: &str, be: &mut Backend, json: bool, p: &Paint) -> Result<Output> {
    let id = a.id("land")?;
    let raw = a.value("--board").map(|s| s.to_string()).or_else(|| env("BOARD")).filter(|s| !s.trim().is_empty());
    let Some(raw) = raw else {
        return Err(BoardError::code(ErrorCode::NoBoard));
    };
    let dir = Path::new(a.value("--dir").unwrap_or("."));
    if !is_repo(dir) {
        return Err(BoardError::usage(format!("{} 不是 git 仓库：在自己的 worktree 里运行，或用 --dir <仓库> 指定", dir.display())));
    }
    if a.flag("--wip") {
        return wip(&raw, id, dir, be, json, p);
    }

    // 参数：提交说明、跑了哪些检查（命令不替 agent 跑检查，但要它说出来，原样进交审评论）
    let message = file_arg(a, "--message-file")?.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        BoardError::usage("board land 要 --message-file <提交说明文件>（第一行按仓库的提交规范写；最后一行的「卡片：」没写会补上）")
    })?;
    let checked = a.value("--checked").map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| {
        BoardError::usage("board land 要 --checked \"<跑了哪些检查、结果>\"：在 rebase 之后的代码上跑完仓库规定的全部检查再来")
    })?;

    // 白板：路径和项目设置里的集成分支（只取工作流，不取整块白板的卡片）
    let d = be.board(&raw, Want::Brief, false)?;
    let card = format!("{}#{id}", d.path);
    let msg = with_card_line(&message, &d.path, id)?;

    // 1. 前提
    git(dir, &["fetch", "--quiet", "origin"], NET).map_err(|e| BoardError::usage(format!("git fetch origin 失败：{e}。什么都没改")))?;
    let want = a.value("--branch").map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or(d.branch.clone());
    let branch = integration_branch(dir, &want).map_err(BoardError::usage)?;
    let up = format!("origin/{branch}");
    let dirty = git(dir, &["status", "--porcelain"], QUICK).map_err(BoardError::usage)?;
    let dirty = dirty.lines().filter(|l| !l.trim().is_empty()).count();
    if dirty > 0 {
        return Err(BoardError::usage(format!(
            "工作区有 {dirty} 个没提交的文件：先提交，再 git rebase {up}、在 rebase 之后的代码上跑检查，然后 board land {id}。什么都没改"
        )));
    }
    if !ancestor(dir, &up, "HEAD") {
        return Err(BoardError::usage(format!(
            "HEAD 不在最新的 {up} 之上（集成分支有新提交，或者还没 rebase）：先 git rebase {up}、在 rebase 之后的代码上重新跑检查，再 board land {id}。什么都没改"
        )));
    }
    let same = git_output(dir, &["diff", "--quiet", &up, "HEAD"], QUICK).map_err(BoardError::usage)?;
    if same.status.success() {
        return Err(BoardError::usage(format!("相对 {up} 没有改动：没有要推的")));
    }

    // 2. 压成一个提交（上一次已经压好、停在了 touched 时不重复压）
    let ahead: usize = git(dir, &["rev-list", "--count", &format!("{up}..HEAD")], QUICK).ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let before = git(dir, &["rev-parse", "--short", "HEAD"], QUICK).map_err(BoardError::usage)?;
    let current = git(dir, &["log", "-1", "--format=%B", "HEAD"], QUICK).unwrap_or_default();
    if !(ahead == 1 && current.trim_end() == msg.trim_end()) {
        git(dir, &["reset", "--quiet", "--soft", &up], QUICK).map_err(|e| BoardError::usage(format!("git reset --soft {up} 失败：{e}")))?;
        git(dir, &["commit", "--quiet", "-m", &msg], COMMIT).map_err(|e| {
            BoardError::usage(format!(
                "压成一个提交时 git commit 失败：{e}。改动都在暂存区里；原来的提交是 {before}，要回去用 git reset --soft {before}"
            ))
        })?;
    }
    let sha = git(dir, &["rev-parse", "--short", "HEAD"], QUICK).map_err(BoardError::usage)?;
    let squashed = if ahead > 1 { format!("{ahead} 个提交压成了 1 个") } else { "1 个提交".to_string() };

    // 3. 有没有碰到别的卡
    let dir_arg = a.value("--dir");
    let t = touched::check(&touched::Opts { commit: None, base: None, upstream: Some(&up), card: Some(&card), dir: dir_arg }, json, p)?;
    if t.code != 0 && !a.flag("--touched-ok") {
        let hint = format!(
            "没有推，压好的提交 {sha} 留着。逐条确认：不是有意的，把别的卡的改动找回来，重新跑检查再 board land {id}；都是有意的，加 --touched-ok 再执行一次，交审评论「碰到的别的卡」一行后面逐条写为什么"
        );
        return Ok(Output::Exit(
            1,
            Box::new(match t.out {
                Output::Exit(_, body) if json => match *body {
                    Output::Json(v) => Output::Json(json!({ "pushed": false, "reason": "touched", "commit": sha, "branch": branch, "touched": t.summary, "report": v, "hint": hint })),
                    other => other,
                },
                Output::Exit(_, body) => match *body {
                    Output::Text(s) => Output::Text(format!("{s}\n{} {hint}\n", p.yellow("!"))),
                    other => other,
                },
                other => other,
            }),
        ));
    }

    // 4. 推之前确认领取仍有效（连不上时重试，等不到就 UNREACHABLE）
    let wait = match a.value("--wait") {
        Some(v) => v.trim().parse::<u64>().map_err(|_| BoardError::usage("board land 的 --wait 需要是秒数（整数）"))?,
        None => crate::HOLD_WAIT_SECS,
    };
    let every = crate::hold_every();
    let (flow, detail) = crate::hold_retry(
        || {
            let d = be.board(&raw, Want::Brief, false)?;
            let c = be.card(&d.path, id)?;
            Ok((d.flow, c))
        },
        Duration::from_secs(wait),
        every,
        |left| eprintln!("{} 连不上服务端，{} 秒后再确认 #{id} 的领取（还能等 {} 秒）", p.yellow("!"), every.as_secs_f64().round(), left.as_secs()),
    )?;
    crate::hold_check(id, flow.name(&detail.card.column), flow.is_lock(&detail.card.column), detail.card.holder.as_deref(), name)?;

    // 5. 推集成分支
    let pushed = git_output(dir, &["push", "--porcelain", "origin", &format!("HEAD:refs/heads/{branch}")], NET).map_err(BoardError::usage)?;
    if !pushed.status.success() {
        let out = String::from_utf8_lossy(&pushed.stdout);
        // --porcelain：被拒的引用那一行以 ! 开头，例如 `!	HEAD:refs/heads/main	[rejected] (fetch first)`
        let Some(line) = out.lines().find(|l| l.starts_with('!')) else {
            return Err(BoardError::usage(format!("git push 失败：{}。集成分支没有变化", crate::code::git_error(&pushed))));
        };
        let why = line.split('\t').nth(2).unwrap_or(line).trim().to_string();
        let hint = format!(
            "推送被拒（{why}）：多半是别人先推了。本地压好的提交 {sha} 还在：git fetch origin、git rebase {up}、在 rebase 之后的代码上重新跑检查，再 board land {id}。不要 --force"
        );
        return Ok(Output::Exit(
            1,
            Box::new(if json {
                Output::Json(json!({ "pushed": false, "reason": "rejected", "commit": sha, "branch": branch, "detail": why, "hint": hint }))
            } else {
                Output::Text(format!("{} {hint}\n", p.red("✗")))
            }),
        ));
    }
    // 让本地认得刚推上去的提交：之后这个 worktree 就算干净的（提交都在远端）
    let _ = git(dir, &["fetch", "--quiet", "origin", &format!("+refs/heads/{branch}:refs/remotes/origin/{branch}")], NET);

    // 6. 从 wip 分支接着做的：推上去之后删掉它
    let mut deleted: Option<String> = None;
    let mut kept: Option<String> = None;
    if let Some(w) = wip_branch(&detail.activity) {
        let on_remote = git(dir, &["ls-remote", "--heads", "origin", &format!("refs/heads/{w}")], NET).is_ok_and(|s| !s.trim().is_empty());
        if on_remote {
            match git(dir, &["push", "--quiet", "origin", "--delete", &w], NET) {
                Ok(_) => deleted = Some(w),
                Err(e) => kept = Some(format!("{w}（删不掉：{e}，请手动 git push origin --delete {w}）")),
            }
        }
    }

    // 7. 交审评论最后的四行
    let lines = format!("提交：{sha}\n集成分支：{branch}\n检查：{checked}\n{}", t.summary);
    if json {
        return Ok(Output::Json(json!({
            "pushed": true,
            "commit": sha,
            "branch": branch,
            "checked": checked,
            "touched": t.summary,
            "wip_deleted": deleted,
            "wip_kept": kept,
            "review_lines": lines,
        })));
    }
    let mut s = format!("{} #{id} 已推到 {branch}：{sha}{}\n", p.green("✓"), p.dim(&format!("（{squashed}）")));
    if let Some(w) = &deleted {
        s += &format!("  删掉了接着做的 wip 分支 {w}\n");
    }
    if let Some(w) = &kept {
        s += &format!("{} wip 分支 {w}\n", p.yellow("!"));
    }
    if t.code != 0 {
        s += &format!("{} 碰到了别的卡（--touched-ok）：交审评论「碰到的别的卡」一行后面逐条写为什么是有意的\n", p.yellow("!"));
    }
    s += &format!("\n{}\n{lines}\n", p.dim("交审评论最后四行（照抄）："));
    Ok(Output::Text(s))
}

/// `board land <ID> --wip`：没做完的改动推到 `wip/<白板标识>/<ID>`（见模块说明）
fn wip(raw: &str, id: i64, dir: &Path, be: &mut Backend, json: bool, p: &Paint) -> Result<Output> {
    // 白板写全了（<项目>/<白板>）就不用连服务端：做不下去的原因可能正是连不上
    let path = if raw.trim().contains('/') { raw.trim().to_string() } else { be.board(raw, Want::Brief, false)?.path };
    let board = path.rsplit('/').next().unwrap_or(&path).to_string();
    let branch = format!("wip/{board}/{id}");
    git(dir, &["fetch", "--quiet", "origin"], NET).map_err(|e| BoardError::usage(format!("git fetch origin 失败：{e}。什么都没改")))?;
    if unpushed(dir).is_none() {
        return Err(BoardError::usage("没有没推的改动（文件都提交了、提交都在远端）：不用推 wip 分支"));
    }
    let dirty = git(dir, &["status", "--porcelain"], QUICK).map_err(BoardError::usage)?;
    if dirty.lines().any(|l| !l.trim().is_empty()) {
        git(dir, &["add", "-A"], QUICK).map_err(BoardError::usage)?;
        // 半成品：不跑仓库的提交钩子（检查不过正是做不下去的原因之一）
        let msg = format!("wip: 未完成的改动\n\n卡片：{path}#{id}");
        git(dir, &["commit", "--quiet", "--no-verify", "-m", &msg], COMMIT).map_err(|e| BoardError::usage(format!("提交没完成的改动失败：{e}")))?;
    }
    let exists = git(dir, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")], QUICK).is_ok();
    let target = format!("HEAD:refs/heads/{branch}");
    let mut args = vec!["push", "--quiet"];
    // 已经有这条分支（接着做的卡又做不下去）：覆盖，但只覆盖刚 fetch 到的那一版，别人刚推的不盖掉
    if exists {
        args.push("--force-with-lease");
    }
    args.extend(["origin", target.as_str()]);
    git(dir, &args, NET).map_err(|e| BoardError::usage(format!("推到 {branch} 失败：{e}。改动还在本地（已经提交）")))?;
    let _ = git(dir, &["fetch", "--quiet", "origin", &format!("+refs/heads/{branch}:refs/remotes/origin/{branch}")], NET);
    let sha = git(dir, &["rev-parse", "--short", "HEAD"], QUICK).map_err(BoardError::usage)?;
    let line = format!("未完成的改动：{branch}");
    if json {
        return Ok(Output::Json(json!({ "branch": branch, "commit": sha, "line": line })));
    }
    Ok(Output::Text(format!(
        "{} 没做完的改动推到了 {branch}（{sha}），集成分支没有变化\n\n{}\n{line}\n",
        p.green("✓"),
        p.dim("board giveup 的评论里带上这一行：")
    )))
}

/// a 是不是 b 的祖先（或就是 b）
fn ancestor(dir: &Path, a: &str, b: &str) -> bool {
    git_output(dir, &["merge-base", "--is-ancestor", a, b], QUICK).is_ok_and(|o| o.status.success())
}

/// 提交说明：最后一行是 `卡片：<path>#<id>`。没写就补上；写在了中间就挪到最后；写的是别的卡时拒绝
fn with_card_line(message: &str, path: &str, id: i64) -> Result<String> {
    let want = format!("卡片：{path}#{id}");
    let mine = |r: &(String, String, i64)| format!("{}/{}#{}", r.0, r.1, r.2) == format!("{path}#{id}");
    if let Some(other) = card_refs(message).into_iter().find(|r| !mine(r)) {
        return Err(BoardError::usage(format!(
            "提交说明里写的是「卡片：{}/{}#{}」，和这张卡（{path}#{id}）对不上：改成这张卡，或者删掉那一行（会自动补上）",
            other.0, other.1, other.2
        )));
    }
    // 去掉已经写了的那一行（可能不在最后，或者冒号是半角），统一补在最后
    let kept: Vec<&str> = message.lines().filter(|l| card_refs(l).is_empty()).collect();
    let body = kept.join("\n");
    let body = body.trim_end();
    if body.trim().is_empty() {
        return Err(BoardError::usage("提交说明是空的：第一行写这次改了什么（按仓库的提交规范）"));
    }
    Ok(format!("{body}\n\n{want}\n"))
}

/// 卡片评论里最后一条 `未完成的改动：wip/…`：这张卡是从这条分支接着做的。只认 `wip/` 开头的分支，别的不删
fn wip_branch(activity: &[Activity]) -> Option<String> {
    let mut found = None;
    for a in activity.iter().filter(|a| a.kind == "comment") {
        for l in a.text.lines() {
            let l = l.trim().trim_start_matches(['-', '*', ' ']);
            let Some(rest) = l.strip_prefix("未完成的改动：").or_else(|| l.strip_prefix("未完成的改动:")) else { continue };
            let b = rest.trim().trim_matches('`').split_whitespace().next().unwrap_or("").trim_matches('`');
            if b.starts_with("wip/") && !b.contains("..") {
                found = Some(b.to_string());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_line_is_appended_or_moved_to_the_end() {
        assert_eq!(with_card_line("feat: 加按钮\n\n- 改了 x\n", "default/main", 7).unwrap(), "feat: 加按钮\n\n- 改了 x\n\n卡片：default/main#7\n");
        assert_eq!(with_card_line("feat: 加按钮\n\n卡片：default/main#7", "default/main", 7).unwrap(), "feat: 加按钮\n\n卡片：default/main#7\n");
        // 写在中间、冒号半角：挪到最后
        assert_eq!(with_card_line("feat: x\n\n卡片: default/main#7\nCo-Authored-By: a", "default/main", 7).unwrap(), "feat: x\n\nCo-Authored-By: a\n\n卡片：default/main#7\n");
        let e = with_card_line("feat: x\n\n卡片：default/main#8", "default/main", 7).unwrap_err();
        assert!(e.message.contains("default/main#8"), "{}", e.message);
        assert!(with_card_line("卡片：default/main#7", "default/main", 7).is_err(), "只有卡片行，没有说明");
    }

    #[test]
    fn wip_branch_comes_from_the_last_comment() {
        let c = |kind: &str, text: &str| Activity {
            id: 0,
            at: 0,
            actor: Actor::agent("a/b"),
            kind: kind.into(),
            action: String::new(),
            text: text.into(),
            edited_at: None,
        };
        assert_eq!(wip_branch(&[]), None);
        let acts = vec![
            c("comment", "做不下去：检查不过\n\n未完成的改动：wip/main/21"),
            c("system", "未完成的改动：wip/main/99"),
            c("comment", "又停了\n- 未完成的改动：`wip/main/21-win`"),
        ];
        assert_eq!(wip_branch(&acts).as_deref(), Some("wip/main/21-win"));
        // 不是 wip/ 开头的分支不删
        assert_eq!(wip_branch(&[c("comment", "未完成的改动：main")]), None);
    }
}
