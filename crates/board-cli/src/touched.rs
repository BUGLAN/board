//! `board touched`：推送前检查这次提交有没有碰到别的卡最近的改动（default/main#343）。
//!
//! 在旧代码上开工、rebase 冲突时整份取了自己那一版，会悄悄撤回别的卡刚做的改动（#328 带回了 #320 删掉的下载按钮，
//! 还把 #320 的回归测试整份换掉，检查照样通过）。这里只用 git，不连服务端：
//!
//! - **这次的改动**：要推的提交（默认 HEAD）相对集成分支（默认 `origin/HEAD` 指的分支）的差别；提交已经在集成分支上时
//!   （回头查历史提交）就是它相对第一个父提交的差别。
//! - **最近的改动**：开工时的基点之后、集成分支上别的卡的提交（不含合并提交；卡片行和这次相同的不算）。基点按顺序认：
//!   `--base`；否则看本 worktree 的 HEAD reflog（每个 worktree 各一份），从新到旧找第一条：`git switch` / `git checkout`
//!   （work.md「开工」切到 origin/main 那一下，board host 复用 worktree 时也是切过去），或者落在集成分支上的 `commit`
//!   （上一张卡推上去的提交：HEAD 已经是最新的时候开工那一下 switch 不动，reflog 里不记）。rebase、reset 记的位置不算：
//!   解冲突就发生在那里。一条都没有时是建好 worktree（或克隆）的地方；再没有就往前 [`FALLBACK_COMMITS`] 个提交。
//! - **碰到**：这次删掉的行，`git blame` 出来是最近那些提交加的；或者这次加的行，是最近那些提交删掉的（集成分支上现在没有这一行，
//!   也没有和它差不多的行）。加回的行按词比：一模一样，或者词够多、换了个变量名这类小改（[`similar`]）都算。
//!   太短的行（`}`、`</div>` 这类，字母数字不到 [`MIN_SIGNIFICANT`] 个）不算。
//!
//! 有碰到的就按卡列出来，退出码 1：agent 逐条确认是有意的（写进交审评论），不是有意的把别的卡的改动找回来。
//! 同一个功能接着改（上一张卡刚加、这一张有意改掉）也会列出来，由人一眼判断。

use crate::versions::card_refs;
use crate::{Output, Paint};
use board_core::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::Command;

/// 找不到开工时的基点时往前查几个提交
pub const FALLBACK_COMMITS: usize = 20;
/// 一行里至少有几个字母、数字（含汉字）才算数：`}`、`</div>`、`} else {` 这类到处都有的行不算
pub const MIN_SIGNIFICANT: usize = 5;
/// 每个文件、每个方向最多举几行例子
const SAMPLES: usize = 2;
/// reflog 最多往回看几条
const REFLOG_LIMIT: usize = 500;

pub struct Opts<'a> {
    pub commit: Option<&'a str>,
    pub base: Option<&'a str>,
    pub upstream: Option<&'a str>,
    pub card: Option<&'a str>,
    pub dir: Option<&'a str>,
}

fn git(dir: Option<&str>, args: &[&str]) -> Result<String> {
    let mut c = Command::new("git");
    if let Some(d) = dir {
        c.arg("-C").arg(d);
    }
    c.args(["-c", "core.quotePath=false"]);
    let out = c.args(args).output().map_err(|e| BoardError::usage(format!("运行 git 失败：{e}（board touched 要在装了 git 的机器上、代码仓库里运行）")))?;
    if !out.status.success() {
        return Err(BoardError::usage(format!("git {} 失败：{}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// a 是不是 b 的祖先（或就是 b）
fn is_ancestor(dir: Option<&str>, a: &str, b: &str) -> bool {
    let mut c = Command::new("git");
    if let Some(d) = dir {
        c.arg("-C").arg(d);
    }
    c.args(["merge-base", "--is-ancestor", a, b]).output().is_ok_and(|o| o.status.success())
}

fn rev(dir: Option<&str>, r: &str) -> Result<String> {
    Ok(git(dir, &["rev-parse", "--verify", "--quiet", &format!("{r}^{{commit}}")]).map_err(|_| BoardError::usage(format!("认不出提交「{r}」")))?.trim().to_string())
}

fn short(h: &str) -> &str {
    &h[..h.len().min(7)]
}

/// 集成分支：`--upstream`，否则 `origin/HEAD` 指的分支，再否则 `origin/main`
fn upstream(dir: Option<&str>, given: Option<&str>) -> Result<(String, String)> {
    if let Some(u) = given.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok((u.to_string(), rev(dir, u)?));
    }
    let name = git(dir, &["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "origin/main".into());
    let h = rev(dir, &name).map_err(|_| BoardError::usage(format!("找不到集成分支「{name}」：用 --upstream <分支> 指定（例如 origin/main）")))?;
    Ok((name, h))
}

/// 开工时的基点（见模块说明）和它是怎么认出来的
fn base(dir: Option<&str>, given: Option<&str>, head: bool, mine_from: &str) -> Result<(String, String)> {
    if let Some(b) = given.map(str::trim).filter(|s| !s.is_empty()) {
        let h = rev(dir, b)?;
        let mb = git(dir, &["merge-base", &h, mine_from])?.trim().to_string();
        return Ok((mb, format!("--base {b}")));
    }
    if head {
        // 本 worktree 的 HEAD reflog（见模块说明）。git worktree add 记的那条没有说明，克隆记的是 clone:
        if let Ok(log) = git(dir, &["reflog", "show", "--format=%H%x09%gs", "HEAD"]) {
            let entries: Vec<(&str, &str)> = log.lines().filter_map(|l| l.split_once('\t')).collect();
            for (h, what) in entries.iter().take(REFLOG_LIMIT) {
                if what.starts_with("checkout: moving from") {
                    if let Ok(mb) = git(dir, &["merge-base", h, mine_from]) {
                        return Ok((mb.trim().to_string(), format!("开工时切到的 {}", short(h))));
                    }
                }
                if what.starts_with("commit") && is_ancestor(dir, h, mine_from) {
                    return Ok((h.to_string(), format!("开工时所在的提交 {}", short(h))));
                }
            }
            if let Some((h, _)) = entries.last().filter(|(_, what)| what.trim().is_empty() || what.starts_with("clone:")) {
                if let Ok(mb) = git(dir, &["merge-base", h, mine_from]) {
                    return Ok((mb.trim().to_string(), format!("建好 worktree 时的 {}", short(h))));
                }
            }
        }
    }
    let list = git(dir, &["rev-list", "--first-parent", &format!("--max-count={}", FALLBACK_COMMITS + 1), mine_from])?;
    let last = list.lines().last().unwrap_or(mine_from).to_string();
    Ok((last, format!("没找到开工时的基点，往前查 {FALLBACK_COMMITS} 个提交（可以用 --base 指定）")))
}

/// 一行算不算数：字母、数字（含汉字）够多
fn significant(line: &str) -> bool {
    line.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_SIGNIFICANT
}

/// 一行里的词（连续的字母、数字、汉字、下划线）
fn words(line: &str) -> HashSet<&str> {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect()
}

/// 两行差不多：一模一样；或者都有至少 5 个不同的词、词的重合（交集 / 并集）不低于 3/4。
/// 加回别的卡删掉的代码时常常顺手换了变量名（#328 把 `href={img.src}` 加回成 `href={stage.src}`）
pub(crate) fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if a == b {
        return true;
    }
    let (x, y) = (words(a), words(b));
    if x.len() < 5 || y.len() < 5 {
        return false;
    }
    let common = x.intersection(&y).count();
    common * 4 >= (x.len() + y.len() - common) * 3
}

#[derive(Default, Debug)]
struct FileDiff {
    added: Vec<String>,
    removed: Vec<String>,
    /// 删掉的行在旧版本里的范围（起始行号、行数）
    old_ranges: Vec<(u64, u64)>,
}

/// 解析 `git diff -U0` 的输出：每个文件加了、删了哪些行
fn parse_diff(text: &str) -> BTreeMap<String, FileDiff> {
    let mut out: BTreeMap<String, FileDiff> = BTreeMap::new();
    let mut old: Option<String> = None;
    let mut cur: Option<String> = None;
    let mut in_hunk = false;
    for l in text.lines() {
        if l.starts_with("diff --git ") {
            old = None;
            cur = None;
            in_hunk = false;
            continue;
        }
        if !in_hunk {
            if let Some(p) = l.strip_prefix("--- ") {
                old = (p != "/dev/null").then(|| p.strip_prefix("a/").unwrap_or(p).to_string());
                continue;
            }
            if let Some(p) = l.strip_prefix("+++ ") {
                let new = (p != "/dev/null").then(|| p.strip_prefix("b/").unwrap_or(p).to_string());
                cur = new.or_else(|| old.clone());
                continue;
            }
        }
        if let Some(h) = l.strip_prefix("@@ ") {
            in_hunk = true;
            // @@ -a[,b] +c[,d] @@
            if let (Some(path), Some(o)) = (&cur, h.split_whitespace().next().and_then(|s| s.strip_prefix('-'))) {
                let (a, b) = match o.split_once(',') {
                    Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
                    None => (o.parse().unwrap_or(0), 1),
                };
                if b > 0 {
                    out.entry(path.clone()).or_default().old_ranges.push((a, b));
                }
            }
            continue;
        }
        if !in_hunk {
            continue;
        }
        let Some(path) = &cur else { continue };
        if let Some(t) = l.strip_prefix('+') {
            out.entry(path.clone()).or_default().added.push(t.to_string());
        } else if let Some(t) = l.strip_prefix('-') {
            out.entry(path.clone()).or_default().removed.push(t.to_string());
        }
    }
    out
}

/// 最近的一个提交
struct Recent {
    sha: String,
    subject: String,
    card: Option<String>,
    diff: BTreeMap<String, FileDiff>,
}

/// 碰到的一个文件：删掉了它加的、加回了它删的
#[derive(Default)]
struct Hit {
    removed: Vec<String>,
    readded: Vec<String>,
}

pub fn run(o: &Opts, json: bool, p: &Paint) -> Result<Output> {
    let dir = o.dir;
    let commit = o.commit.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("HEAD");
    let c = rev(dir, commit)?;
    let head = rev(dir, "HEAD").ok().as_deref() == Some(c.as_str());
    let (up_name, up) = upstream(dir, o.upstream)?;
    // 这次的改动从哪里算起：已经在集成分支上的提交（回头查历史）按它自己；否则从和集成分支的分叉点算起
    let mb = git(dir, &["merge-base", &c, &up])?.trim().to_string();
    let from = if mb == c { rev(dir, &format!("{c}^"))? } else { mb };
    let (b, how) = base(dir, o.base, head, &from)?;

    // 这次是哪张卡：--card，否则这次的提交说明里写的
    let mine: HashSet<String> = match o.card.map(str::trim).filter(|s| !s.is_empty()) {
        Some(card) => {
            let refs = card_refs(&format!("卡片：{card}"));
            if refs.is_empty() {
                return Err(BoardError::usage(format!("认不出卡片「{card}」：写成 <项目>/<白板>#<ID>")));
            }
            refs.into_iter().map(|(p, b, id)| format!("{p}/{b}#{id}")).collect()
        }
        None => card_refs(&git(dir, &["log", "--format=%B", &format!("{from}..{c}")])?).into_iter().map(|(p, b, id)| format!("{p}/{b}#{id}")).collect(),
    };

    let mine_diff = parse_diff(&git(dir, &["diff", "--no-renames", "--no-color", "--no-ext-diff", "-U0", &from, &c])?);
    let paths: Vec<&str> = mine_diff.keys().map(String::as_str).collect();

    // 最近的提交：只看动过这次改到的文件的
    let mut recent: Vec<Recent> = vec![];
    if !paths.is_empty() && b != from {
        let mut args = vec![
            "log",
            "--no-merges",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
            "-U0",
            "--format=%x1e%H%x1f%s%x1f%B%x1d",
        ];
        let range = format!("{b}..{from}");
        args.push(&range);
        args.push("--");
        args.extend(paths.iter().copied());
        let text = git(dir, &args)?;
        for chunk in text.split('\x1e').filter(|s| !s.trim().is_empty()) {
            let Some((head, diff)) = chunk.split_once('\x1d') else { continue };
            let mut f = head.splitn(3, '\x1f');
            let sha = f.next().unwrap_or_default().trim().to_string();
            let subject = f.next().unwrap_or_default().trim().to_string();
            let body = f.next().unwrap_or_default();
            let card = card_refs(body).into_iter().map(|(p, b, id)| format!("{p}/{b}#{id}")).next();
            if card.as_ref().is_some_and(|c| mine.contains(c)) {
                continue;
            }
            recent.push(Recent { sha, subject, card, diff: parse_diff(diff) });
        }
    }
    let by_sha: HashMap<&str, usize> = recent.iter().enumerate().map(|(i, r)| (r.sha.as_str(), i)).collect();

    // 碰到的：(提交下标, 文件) → 行
    let mut hits: BTreeMap<(usize, String), Hit> = BTreeMap::new();
    for (path, d) in &mine_diff {
        // 删掉的行：blame 出是谁加的
        if !d.old_ranges.is_empty() && !recent.is_empty() {
            let mut args: Vec<String> = vec!["blame".into(), "--line-porcelain".into()];
            for (a, n) in &d.old_ranges {
                args.push("-L".into());
                args.push(format!("{a},+{n}"));
            }
            args.extend([from.clone(), "--".into(), path.clone()]);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            if let Ok(text) = git(dir, &refs) {
                let mut who: Option<usize> = None;
                for l in text.lines() {
                    if let Some(t) = l.strip_prefix('\t') {
                        if let Some(i) = who.take() {
                            if significant(t) {
                                hits.entry((i, path.clone())).or_default().removed.push(t.trim().to_string());
                            }
                        }
                    } else if l.len() > 40 && l.as_bytes()[40] == b' ' && l[..40].bytes().all(|x| x.is_ascii_hexdigit()) {
                        who = by_sha.get(&l[..40]).copied();
                    }
                }
            }
        }
        // 加回的行：最近哪个提交删掉了它，而集成分支上现在没有这一行
        if d.added.is_empty() {
            continue;
        }
        let now: Vec<String> = git(dir, &["show", &format!("{from}:{path}")]).map(|t| t.lines().map(|l| l.trim().to_string()).collect()).unwrap_or_default();
        let exact: HashSet<&str> = now.iter().map(String::as_str).collect();
        for (i, r) in recent.iter().enumerate() {
            let Some(rd) = r.diff.get(path) else { continue };
            // 它删掉、集成分支上现在也没有（连差不多的都没有）的行
            let gone: Vec<&str> = rd
                .removed
                .iter()
                .map(|l| l.trim())
                .filter(|l| significant(l) && !exact.contains(l) && !now.iter().any(|n| similar(n, l)))
                .collect();
            if gone.is_empty() {
                continue;
            }
            for l in &d.added {
                let t = l.trim();
                if significant(t) && gone.iter().any(|g| similar(g, t)) {
                    hits.entry((i, path.clone())).or_default().readded.push(t.to_string());
                }
            }
        }
    }

    // 按卡分组（没有卡片行的提交各自一组），卡按最早碰到的提交排
    struct Group {
        key: String,
        card: Option<String>,
        commits: Vec<usize>,
        files: BTreeMap<String, Hit>,
    }
    let mut groups: Vec<Group> = vec![];
    for ((i, path), h) in hits {
        let r = &recent[i];
        let key = r.card.clone().unwrap_or_else(|| r.sha.clone());
        let g = match groups.iter_mut().position(|g| g.key == key) {
            Some(k) => &mut groups[k],
            None => {
                groups.push(Group { key: key.clone(), card: r.card.clone(), commits: vec![], files: BTreeMap::new() });
                groups.last_mut().expect("刚加的")
            }
        };
        if !g.commits.contains(&i) {
            g.commits.push(i);
        }
        let f = g.files.entry(path).or_default();
        f.removed.extend(h.removed);
        f.readded.extend(h.readded);
    }
    // git log 从新到旧：下标大的更早
    for g in &mut groups {
        g.commits.sort_by(|a, b| b.cmp(a));
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.commits.first().copied().unwrap_or(0)));

    // 没有卡片行的提交按提交号认
    let who = |g: &Group| g.card.clone().unwrap_or_else(|| format!("提交 {}", short(&g.key)));
    let label = |g: &Group| g.card.clone().unwrap_or_else(|| format!("提交 {}（没有卡片行）", short(&g.key)));
    let base_name = |f: &str| f.rsplit('/').next().unwrap_or(f).to_string();
    let summary = if groups.is_empty() {
        "碰到的别的卡：无".to_string()
    } else {
        let parts: Vec<String> = groups.iter().map(|g| format!("{}（{}）", who(g), g.files.keys().map(|f| base_name(f)).collect::<Vec<_>>().join("、"))).collect();
        format!("碰到的别的卡：{}", parts.join("；"))
    };
    let code = if groups.is_empty() { 0 } else { 1 };

    if json {
        let list: Vec<Value> = groups
            .iter()
            .map(|g| {
                json!({
                    "card": g.card,
                    "commits": g.commits.iter().map(|&i| json!({ "sha": recent[i].sha, "subject": recent[i].subject })).collect::<Vec<_>>(),
                    "files": g.files.iter().map(|(path, h)| json!({
                        "path": path,
                        "removed": h.removed.len(),
                        "readded": h.readded.len(),
                        "removed_samples": h.removed.iter().take(SAMPLES).collect::<Vec<_>>(),
                        "readded_samples": h.readded.iter().take(SAMPLES).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let v = json!({
            "commit": c,
            "upstream": up_name,
            "from": from,
            "base": b,
            "base_from": how,
            "mine": mine.iter().collect::<Vec<_>>(),
            "checked": recent.len(),
            "touched": list,
            "summary": summary,
        });
        return Ok(Output::Exit(code, Box::new(Output::Json(v))));
    }

    let mut s = String::new();
    let span = format!("基点 {}（{how}）之后、动过这些文件的 {} 个别的提交", short(&b), recent.len());
    if groups.is_empty() {
        s.push_str(&format!("{} 没有碰到别的卡最近的改动：{span}\n", p.green("✓")));
    } else {
        s.push_str(&format!("{} 这次提交碰到了 {} 处别的卡最近的改动（{span}）：\n", p.yellow("!"), groups.len()));
        let cut = |l: &str| crate::text::truncate(l, 80);
        for g in &groups {
            s.push('\n');
            s.push_str(&format!("{}\n", p.bold(&label(g))));
            for &i in &g.commits {
                s.push_str(&format!("  {} {}\n", p.dim(short(&recent[i].sha)), recent[i].subject));
            }
            for (path, h) in &g.files {
                let mut what = vec![];
                if !h.removed.is_empty() {
                    what.push(format!("删掉了它加的 {} 行", h.removed.len()));
                }
                if !h.readded.is_empty() {
                    what.push(format!("加回了它删的 {} 行", h.readded.len()));
                }
                s.push_str(&format!("  {}  {}\n", p.cyan(path), what.join("，")));
                for l in h.removed.iter().take(SAMPLES) {
                    s.push_str(&format!("      - {}\n", cut(l)));
                }
                for l in h.readded.iter().take(SAMPLES) {
                    s.push_str(&format!("      + {}\n", cut(l)));
                }
            }
        }
        s.push_str("\n逐条确认：有意改的，在交审评论「碰到的别的卡」一行写明为什么；不是有意的（rebase 解冲突时取了自己那一版），把它的改动找回来，重新跑检查再推。\n");
    }
    s.push_str(&format!("\n{summary}\n"));
    Ok(Output::Exit(code, Box::new(Output::Text(s))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn significant_lines() {
        assert!(!significant("}"));
        assert!(!significant("</div>"));
        assert!(!significant("} else {"));
        assert!(significant("return null;"));
        assert!(significant("去掉下载按钮"));
        assert!(!significant("下载"), "两个字的行到处都有");
        assert!(significant("<a download={src}>"));
    }

    #[test]
    fn similar_lines() {
        // #328 加回 #320 删掉的下载按钮时换了变量名
        assert!(similar(r#"<a className="btn sm" href={img.src} download={img.alt || true}>"#, r#"<a className="btn sm" href={stage.src} download={stage.alt || true}>"#));
        assert!(similar("  return null;", "return null;"));
        assert!(!similar("return null;", "return undefined;"), "词太少只认一模一样");
        assert!(!similar(
            "{/* 底部条：文件名、下载，多张时右边是切换；关闭靠 Esc 或点遮罩 */}",
            "{/* 底部条：文件名，多张时右边是切换；关闭靠 Esc 或点遮罩。不放下载按钮（default/main#320）：要存右键图片另存为 */}"
        ));
    }

    #[test]
    fn parses_unified_zero_diff() {
        let d = "diff --git a/a.ts b/a.ts\nindex 1..2 100644\n--- a/a.ts\n+++ b/a.ts\n@@ -3,2 +3 @@ x\n-old one\n--- removed comment\n+new one\n@@ -9,0 +9,1 @@\n+++ added line\ndiff --git a/gone.ts b/gone.ts\ndeleted file mode 100644\n--- a/gone.ts\n+++ /dev/null\n@@ -1 +0,0 @@\n-only line\n";
        let m = parse_diff(d);
        let a = &m["a.ts"];
        assert_eq!(a.removed, vec!["old one", "-- removed comment"]);
        assert_eq!(a.added, vec!["new one", "++ added line"]);
        assert_eq!(a.old_ranges, vec![(3, 2)]);
        assert_eq!(m["gone.ts"].removed, vec!["only line"]);
        assert_eq!(m["gone.ts"].old_ranges, vec![(1, 1)]);
    }
}
