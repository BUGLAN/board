//! `board base <ID>`：把开工做完（default/main#552），输出下一步该做什么。
//!
//! skill「代码 → 会话的 worktree」「开工」原来是十来条 git 命令加一页规则，做错了后果很重
//! （在旧代码上开工，rebase 时悄悄撤回别的卡的改动，#328）。这里按顺序做完，用的是 board host 备代码的同一套规则（code.rs）：
//!
//! 1. 卡片要由自己持有（否则 `NOT_HOLDER`，什么都不动），免得在没领到的卡上开工。
//! 2. 集成分支：`--branch` → 项目设置里的 → 远端默认分支；`git fetch origin`。
//! 3. 会话的 worktree（[`code::session_worktree`]）：当前目录就是锁着自己名字的 worktree（board host 备好的）就用它，
//!    `.wt/` 里有锁着自己名字的就接着用，没有就接手一个别的会话留下的空闲 worktree（#622：依赖、构建产物都在），
//!    都没有才新建。
//! 4. 保护现场：worktree 里有没提交的文件、没推到远端的提交时**不切换**，退出码 1，列出来。
//!    改动属于哪张卡要看内容才知道，不替 agent 判断、不自动保存。
//! 5. 这张卡在集成分支上有没有提交（提交说明里 `卡片：<项目>/<白板>#<ID>`），卡片动态里有没有对应的「提交：」行。
//! 6. 卡片评论里最后一条 `未完成的改动：wip/…`、远端还有这条分支：切过去，rebase 到最新的集成分支；冲突时停在冲突上，退出码 1。
//! 7. 否则切到最新的集成分支（`git checkout --detach`：reflog 里记一条切换，`board touched` 靠它认开工时的基点）。
//! 8. 确认基点包含最新的集成分支。
//! 9. 清理空闲的 worktree（[`code::sweep`]，和 board host 定时做的是同一套规则）：超过 36 小时没用的、多出 4 个的删掉。

use crate::args::Args;
use crate::backend::{env, Backend, Want};
use crate::code::{self, git, git_output, integration_branch, is_repo, unpushed};
use crate::land::wip_branch;
use crate::output::{Output, Paint};
use board_core::*;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, SystemTime};

const QUICK: Duration = Duration::from_secs(30);
const NET: Duration = Duration::from_secs(300);
/// rebase 可能跑仓库的钩子
const REBASE: Duration = Duration::from_secs(600);
/// 现场不干净时最多列几行
const SHOWN: usize = 20;

pub fn run(a: &Args, name: &str, be: &mut Backend, json: bool, p: &Paint) -> Result<Output> {
    let id = a.id("base")?;
    let raw = a.value("--board").map(|s| s.to_string()).or_else(|| env("BOARD")).filter(|s| !s.trim().is_empty());
    let Some(raw) = raw else {
        return Err(BoardError::code(ErrorCode::NoBoard));
    };
    let dir = Path::new(a.value("--dir").unwrap_or("."));
    if !is_repo(dir) {
        return Err(BoardError::usage(format!("{} 不是 git 仓库：在项目的仓库（主仓库或任意 worktree）里运行，或用 --dir <仓库> 指定", dir.display())));
    }

    // 1. 卡片要由自己持有
    let board = be.board(&raw, Want::Brief, false)?;
    let card = be.card(&board.path, id)?;
    if card.card.holder.as_deref() != Some(name) {
        let who = match &card.card.holder {
            Some(h) => format!("由 {h} 持有"),
            None => "没人持有".to_string(),
        };
        return Err(BoardError::new(
            ErrorCode::NotHolder,
            format!("#{id} 现在在「{}」，{who}：先 board start {id} 领到再开工。什么都没动", board.flow.name(&card.card.column)),
        ));
    }

    // 2. 集成分支、拉最新
    git(dir, &["fetch", "--quiet", "--prune", "origin"], NET).map_err(|e| BoardError::usage(format!("git fetch origin 失败：{e}。什么都没动")))?;
    let want = a.value("--branch").map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or(board.branch.clone());
    let branch = integration_branch(dir, &want).map_err(BoardError::usage)?;
    let up = format!("origin/{branch}");

    // 3. 会话的 worktree
    // 谁在线：判断别的会话留下的 worktree 是不是空闲。查不到（旧服务端、网络）时锁着的一律当成有人在用
    let online = be.online().ok();
    let (wt, got) = code::session_worktree(dir, dir, name, &up, online.as_deref()).map_err(BoardError::usage)?;
    let wt_s = wt.display().to_string();
    let mut v = json!({
        "worktree": wt_s,
        "created": got == code::Got::Created,
        "adopted": got == code::Got::Adopted,
        "branch": branch,
        "base": Value::Null,
        "from_wip": Value::Null,
        "landed": [],
    });

    // 4. 保护现场：有没推的改动就不切换
    if let Some(what) = unpushed(&wt) {
        let files: Vec<String> = git(&wt, &["status", "--porcelain"], QUICK).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
        let commits: Vec<String> =
            git(&wt, &["log", "--oneline", "--no-decorate", &format!("--max-count={SHOWN}"), "HEAD", "--not", "--remotes"], QUICK).unwrap_or_default().lines().map(str::to_string).collect();
        let board_key = board.path.rsplit('/').next().unwrap_or(&board.path).to_string();
        let next = format!(
            "先保存现场：worktree 里有没推的改动（{what}），没有切换。上一张卡没收尾的：board land <上一张卡 ID> --wip，在那张卡上评论它输出的一行；就是这张卡从 wip 分支接着做的：board land {id} --wip；分不清属于哪张卡：git add -A、git commit 后推成快照 git push origin HEAD:refs/heads/wip/{board_key}/{id}-presync-<短提交号>，在这张卡上评论保存到了哪。不要 git stash 后丢掉、不要 reset --hard / checkout -- . / clean。保存好再 board base {id}"
        );
        v["step"] = json!("save");
        v["next"] = json!(next);
        v["dirty"] = json!(files);
        v["unpushed"] = json!(commits);
        if json {
            return Ok(Output::Exit(1, Box::new(Output::Json(v))));
        }
        let mut s = format!("{} {wt_s} 里有没推的改动，没有切换\n", p.yellow("!"));
        for l in files.iter().take(SHOWN) {
            s += &format!("  {l}\n");
        }
        if files.len() > SHOWN {
            s += &format!("  {}\n", p.dim(&format!("… 共 {} 个文件", files.len())));
        }
        for l in &commits {
            s += &format!("  {} {l}\n", p.dim("没推的提交"));
        }
        s += &format!("\n{} {next}\n", p.bold("下一步："));
        return Ok(Output::Exit(1, Box::new(Output::Text(s))));
    }

    // 5. 这张卡在集成分支上已经有的提交；动态里有没有交审时写的「提交：」行
    let mine = format!("{}#{id}", board.path);
    let log = git(&wt, &["log", &up, "--format=%h%x09%s", "--fixed-strings", &format!("--grep=卡片：{mine}")], QUICK).unwrap_or_default();
    let reported: Vec<String> = card
        .activity
        .iter()
        .filter(|a| a.kind == "comment")
        .flat_map(|a| a.text.lines().map(str::trim).filter_map(|l| l.strip_prefix("提交：").or_else(|| l.strip_prefix("提交:"))).map(|r| r.trim().trim_matches('`').to_string()).collect::<Vec<_>>())
        .filter(|r| r.len() >= 7)
        .collect();
    let mut landed: Vec<(String, String, bool)> = vec![];
    for l in log.lines() {
        let Some((sha, subject)) = l.split_once('\t') else { continue };
        // 提交说明的最后一行才算（--grep 只能找到包含它的）
        let body = git(&wt, &["log", "-1", "--format=%B", sha], QUICK).unwrap_or_default();
        if body.lines().map(str::trim).rfind(|l| !l.is_empty()) != Some(format!("卡片：{mine}").as_str()) {
            continue;
        }
        let full = git(&wt, &["rev-parse", sha], QUICK).unwrap_or_default();
        let seen = reported.iter().any(|r| full.starts_with(r.as_str()));
        landed.push((sha.to_string(), subject.to_string(), seen));
    }
    v["landed"] = Value::Array(landed.iter().map(|(sha, subject, seen)| json!({ "commit": sha, "subject": subject, "reported": seen })).collect());

    // 6. 从 wip 分支接着做：切过去，rebase 到最新的集成分支
    let wip = wip_branch(&card.activity).filter(|w| git(&wt, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{w}")], QUICK).is_ok());
    if let Some(w) = &wip {
        v["from_wip"] = json!(w);
        git(&wt, &["checkout", "--quiet", "--detach", &format!("origin/{w}")], QUICK).map_err(|e| BoardError::usage(format!("切不到 origin/{w}：{e}")))?;
        let r = git_output(&wt, &["rebase", "--quiet", &up], REBASE).map_err(BoardError::usage)?;
        if !r.status.success() {
            let conflicts: Vec<String> = git(&wt, &["diff", "--name-only", "--diff-filter=U"], QUICK).unwrap_or_default().lines().map(str::to_string).collect();
            if conflicts.is_empty() {
                let _ = git(&wt, &["rebase", "--abort"], QUICK);
                return Err(BoardError::usage(format!("把 {w} rebase 到 {up} 失败：{}。已经 rebase --abort", code::git_error(&r))));
            }
            let next = format!(
                "先解冲突：从 {w} 接着做，rebase 到 {up} 时冲突（{}）。按 work.md「推送」第 1 步的规则逐块合（保留上游和半成品两边的改动），git add 后 git rebase --continue；合不了就 git rebase --abort，按「做不下去」交给人",
                conflicts.join("、")
            );
            v["step"] = json!("resolve");
            v["next"] = json!(next);
            v["conflicts"] = json!(conflicts);
            return Ok(Output::Exit(
                1,
                Box::new(if json { Output::Json(v) } else { Output::Text(format!("{} {wt_s}：{w} rebase 到 {up} 时冲突\n\n{} {next}\n", p.yellow("!"), p.bold("下一步："))) }),
            ));
        }
    } else {
        // 7. 切到最新的集成分支
        git(&wt, &["checkout", "--quiet", "--detach", &up], QUICK).map_err(|e| BoardError::usage(format!("{wt_s} 切不到 {up}：{e}")))?;
    }

    // 8. 基点包含最新的集成分支
    let ok = git_output(&wt, &["merge-base", "--is-ancestor", &up, "HEAD"], QUICK).is_ok_and(|o| o.status.success());
    if !ok {
        return Err(BoardError::usage(format!("{wt_s} 的 HEAD 不包含最新的 {up}：重新 board base {id}")));
    }
    let base = git(&wt, &["rev-parse", "--short", "HEAD"], QUICK).map_err(BoardError::usage)?;
    v["base"] = json!(base);

    // 9. 清理空闲的 worktree：只报删掉的（留着的由 board host 的清理去说）
    let cleaned: Vec<String> = code::sweep(dir, std::slice::from_ref(&wt), online.as_deref(), code::WT_UNUSED, code::WT_IDLE_MAX, SystemTime::now())
        .into_iter()
        .filter(|(_, s)| matches!(s, code::Swept::Removed | code::Swept::Extra))
        .map(|(p, _)| p.display().to_string())
        .collect();
    v["cleaned"] = json!(cleaned);

    let unreported: Vec<&str> = landed.iter().filter(|l| !l.2).map(|l| l.0.as_str()).collect();
    let all: Vec<&str> = landed.iter().map(|l| l.0.as_str()).collect();
    let (step, next) = if let Some(w) = &wip {
        ("wip", format!("从 {w} 接着做，已经 rebase 到 {up}：先看 git diff {up}...HEAD 和卡片动态里上一次留下的说明，弄清做到哪了再接着改，不要从头重做"))
    } else if !unreported.is_empty() {
        ("submit", format!("这张卡已经推到集成分支（{}），动态里还没有交审：不重做，确认提交满足卡片后直接 board submit {id}（「提交：」写这个提交号）", unreported.join("、")))
    } else if !all.is_empty() {
        ("rework", format!(
            "这张卡交审过（{}），又回到了你手里：按动态里人的意见在这里接着修，再提交一次；不改写已经推上去的提交，要回滚时 git revert --no-commit <提交号>",
            all.join("、")
        ))
    } else {
        ("code", format!("直接改代码：cd {wt_s}"))
    };
    v["step"] = json!(step);
    v["next"] = json!(next);
    if json {
        return Ok(Output::Json(v));
    }
    let how = match got {
        code::Got::Created => "（新建）",
        code::Got::Adopted => "（接手空闲的）",
        code::Got::Mine => "（接着用）",
    };
    let mut s = format!("{} #{id} 开工：{wt_s}{}\n", p.green("✓"), p.dim(how));
    s += &format!("  {}  {branch}，基点 {base}{}\n", p.dim("集成分支"), if wip.is_some() { String::new() } else { format!("（{up}）") });
    if let Some(w) = &wip {
        s += &format!("  {}  {w}，已经 rebase 到 {up}\n", p.dim("接着做  "));
    }
    for (sha, subject, seen) in &landed {
        s += &format!("  {}  {sha} {subject}{}\n", p.dim("已推过  "), p.dim(if *seen { "（交审过）" } else { "（动态里没有交审）" }));
    }
    for c in &cleaned {
        s += &format!("  {}  {c}\n", p.dim("清理空闲"));
    }
    s += &format!("\n{} {next}\n", p.bold("下一步："));
    Ok(Output::Text(s))
}
