//! `board versions sync`（PRD §9、§10.15）：在代码仓库里从 git tag 和提交历史反推每个版本做了哪些卡片，写进服务端。
//!
//! - 版本 = `v` 加数字开头的 tag，按版本号排；两个相邻 tag 之间的提交（不含合并提交）属于后一个 tag，第一个 tag 是它之前的全部提交。
//! - 提交说明里每一行「卡片：<项目>/<白板>#<ID>」把这个提交连到一张卡片（一个提交可以写多行）。
//! - 发版提交本身（`chore` 开头、说明里写着这个 tag 名）不算进这一版。
//! - 项目：`--project`，不写时取提交里出现最多的项目；连到别的项目的卡片不管。
//! - 每个版本一次 `POST /api/releases`：服务端建好版本标签（不存在时）、给连到的卡片打上、存下这一版的提交。
//!   同一个版本可以反复同步，结果一样；只同步某几个 tag 时写在 sync 后面。

use crate::backend::Remote;
use crate::{Output, Paint};
use board_core::releases::{is_version_name, version_cmp, ReleaseCommit, SyncResult};
use board_core::*;
use serde_json::json;
use std::collections::HashMap;
use std::process::Command;

/// 提交说明里连到卡片的那一行：「卡片：项目/白板#ID」（冒号全角半角都认）
pub(crate) fn card_refs(body: &str) -> Vec<(String, String, i64)> {
    body.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("卡片：").or_else(|| l.strip_prefix("卡片:"))?.trim();
            let (path, id) = rest.rsplit_once('#')?;
            let (project, board) = path.trim().split_once('/')?;
            let id = id.trim().parse::<i64>().ok().filter(|n| *n > 0)?;
            (!project.is_empty() && !board.is_empty() && !board.contains('/')).then(|| (project.to_string(), board.to_string(), id))
        })
        .collect()
}

/// 发版提交本身：`chore` 开头、说明里写着这个 tag 名（例如「chore: 发版 v0.1.21」）
pub(crate) fn is_release_commit(subject: &str, tag: &str) -> bool {
    subject.trim_start().starts_with("chore") && subject.split(|c: char| c.is_whitespace() || c == ':' || c == '：').any(|w| w == tag)
}

struct Tag {
    name: String,
    at: i64,
}

struct Raw {
    h: String,
    at: i64,
    s: String,
    refs: Vec<(String, String, i64)>,
}

fn git(dir: Option<&str>, args: &[&str]) -> Result<String> {
    let mut c = Command::new("git");
    if let Some(d) = dir {
        c.arg("-C").arg(d);
    }
    let out = c.args(args).output().map_err(|e| BoardError::usage(format!("运行 git 失败：{e}（board versions sync 要在装了 git 的机器上、代码仓库里运行）")))?;
    if !out.status.success() {
        return Err(BoardError::usage(format!("git {} 失败：{}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 仓库里的版本 tag，版本号从旧到新
fn tags(dir: Option<&str>) -> Result<Vec<Tag>> {
    let out = git(dir, &["for-each-ref", "--format=%(refname:short)%09%(creatordate:unix)", "refs/tags"])?;
    let mut tags: Vec<Tag> = out
        .lines()
        .filter_map(|l| {
            let (name, at) = l.split_once('\t')?;
            is_version_name(name).then(|| Tag { name: name.to_string(), at: at.trim().parse::<i64>().unwrap_or(0) * 1000 })
        })
        .collect();
    tags.sort_by(|a, b| version_cmp(&a.name, &b.name));
    Ok(tags)
}

/// 一段提交（range 是 `上一个..这个` 或者只有这个 tag）
fn commits(dir: Option<&str>, range: &str, tag: &str) -> Result<Vec<Raw>> {
    let out = git(dir, &["log", "--no-merges", "--format=%h%x1f%ct%x1f%s%x1f%B%x1e", range])?;
    Ok(out
        .split('\u{1e}')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|rec| {
            let mut f = rec.splitn(4, '\u{1f}');
            let h = f.next()?.to_string();
            let at = f.next()?.trim().parse::<i64>().ok()? * 1000;
            let s = f.next()?.to_string();
            let body = f.next().unwrap_or_default();
            Some(Raw { h, at, refs: card_refs(body), s })
        })
        .filter(|c| !is_release_commit(&c.s, tag))
        .collect())
}

pub fn run(pos: &[String], project: Option<&str>, dir: Option<&str>, dry_run: bool, json: bool, p: &Paint, rm: &Remote) -> Result<Output> {
    if pos.get(1).map(String::as_str) != Some("sync") {
        return Err(BoardError::usage("用法：board versions sync [<tag>…] [--project <项目>] [--dir <仓库>] [--dry-run]"));
    }
    let all = tags(dir)?;
    if all.is_empty() {
        return Err(BoardError::usage("仓库里没有 v 开头的 tag（版本 = v 加数字开头的 tag，例如 v0.1.0）"));
    }
    let only: Vec<&str> = pos[2..].iter().map(String::as_str).collect();
    for t in &only {
        if !all.iter().any(|x| x.name == *t) {
            return Err(BoardError::usage(format!("仓库里没有 tag {t}")));
        }
    }

    // 每个 tag 的提交（只同步某几个 tag 时也要读全部：项目按全部提交里出现最多的算）
    let mut per: Vec<(&Tag, Vec<Raw>)> = Vec::new();
    for (i, t) in all.iter().enumerate() {
        let range = if i == 0 { t.name.clone() } else { format!("{}..{}", all[i - 1].name, t.name) };
        if only.is_empty() || only.contains(&t.name.as_str()) || project.is_none() {
            per.push((t, commits(dir, &range, &t.name)?));
        }
    }
    let project = match project {
        Some(p) => p.trim().to_string(),
        None => {
            let mut n: HashMap<&str, usize> = HashMap::new();
            for (_, cs) in &per {
                for c in cs {
                    for (pr, _, _) in &c.refs {
                        *n.entry(pr.as_str()).or_default() += 1;
                    }
                }
            }
            n.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|(k, _)| k.to_string()).ok_or_else(|| {
                BoardError::usage("提交里没有「卡片：<项目>/<白板>#<ID>」，看不出是哪个项目：用 --project <项目> 指定")
            })?
        }
    };
    if !only.is_empty() {
        per.retain(|(t, _)| only.contains(&t.name.as_str()));
    }

    let mut rows = Vec::new();
    let mut text = String::new();
    let mut total_added = 0usize;
    // 新的在前
    for (t, cs) in per.iter().rev() {
        let commits: Vec<ReleaseCommit> = cs
            .iter()
            .map(|c| ReleaseCommit {
                h: c.h.clone(),
                at: c.at,
                s: c.s.clone(),
                cards: c.refs.iter().filter(|(pr, _, _)| *pr == project).map(|(_, b, id)| format!("{b}#{id}")).collect(),
            })
            .collect();
        let mut cards: Vec<&String> = commits.iter().flat_map(|c| &c.cards).collect();
        cards.sort();
        cards.dedup();
        let loose = commits.iter().filter(|c| c.cards.is_empty()).count();
        let res: Option<SyncResult> = if dry_run {
            None
        } else {
            Some(rm.post("/api/releases", json!({ "project": project, "name": t.name, "releasedAt": t.at, "commits": commits }))?)
        };
        if let Some(r) = &res {
            total_added += r.added.len();
        }
        rows.push(json!({
            "name": t.name,
            "releasedAt": t.at,
            "commits": commits.len(),
            "loose": loose,
            "cards": cards.len(),
            "created": res.as_ref().map(|r| r.created),
            "added": res.as_ref().map(|r| r.added.clone()),
            "already": res.as_ref().map(|r| r.already),
            "missing": res.as_ref().map(|r| r.missing.clone()),
        }));
        let mut line = format!("{} 张卡片", cards.len());
        if let Some(r) = &res {
            if !r.added.is_empty() {
                line += &format!("（新打 {}）", r.added.len());
            }
        }
        line += &format!(" · {} 个提交", commits.len());
        if loose > 0 {
            line += &format!("，{loose} 个没写卡片行");
        }
        text += &format!("{} {}  {}\n", if dry_run { p.dim("·") } else { p.green("✓") }, p.cyan(&format!("{:<10}", t.name)), line);
        if let Some(r) = &res {
            if !r.missing.is_empty() {
                text += &format!("  {}\n", p.yellow(&format!("找不到：{}", r.missing.join("、"))));
            }
        }
    }
    if json {
        return Ok(Output::Json(json!({ "project": project, "dryRun": dry_run, "versions": rows })));
    }
    text += &if dry_run {
        p.dim(&format!("没有写入（--dry-run）：项目 {project}，{} 个版本\n", rows.len()))
    } else {
        p.dim(&format!("项目 {project}：同步了 {} 个版本，新打标签 {total_added} 张\n", rows.len()))
    };
    Ok(Output::Text(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_card_lines() {
        let body = "feat: 里程碑\n\n说明\n\n卡片：default/main#263\n卡片: oss/site#7\n卡片：坏的\n不是卡片：default/main#1";
        assert_eq!(
            card_refs(body),
            vec![("default".into(), "main".into(), 263), ("oss".into(), "site".into(), 7)]
        );
        assert!(card_refs("卡片：default#3").is_empty());
        assert!(card_refs("卡片：default/main#0").is_empty());
    }

    #[test]
    fn skips_release_commits() {
        assert!(is_release_commit("chore: 发版 v0.1.21", "v0.1.21"));
        assert!(is_release_commit("chore(release): v1.2.0", "v1.2.0"));
        assert!(!is_release_commit("chore: 发版 v0.1.21", "v0.1.2"));
        assert!(!is_release_commit("feat: 支持 v0.1.21 的格式", "v0.1.21"));
    }
}
