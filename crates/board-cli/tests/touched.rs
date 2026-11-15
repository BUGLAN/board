//! `board touched`（default/main#343）：推送前查出这次提交悄悄撤回了别的卡最近的改动。
//! 不连服务端，只在临时的 git 仓库里跑：一个裸仓库当远端，「别人」先推一张卡，agent 在旧代码上开工、rebase 冲突时整份取自己那一版。

use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_EDITOR", "true")
        .env_remove("GIT_DIR")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}：{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// 跑 git，不管成败（rebase 冲突时会失败）
fn git_try(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_EDITOR", "true")
        .env_remove("GIT_DIR")
        .output()
        .unwrap()
        .status
        .success()
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.out).unwrap_or_else(|e| panic!("不是 JSON（{e}）：{}{}", self.out, self.err))
    }
}

fn touched(dir: &Path, args: &[&str]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_board"))
        .arg("touched")
        .args(args)
        .args(["--dir", dir.to_str().unwrap()])
        .env("NO_COLOR", "1")
        .env_remove("BOARD_URL")
        .env_remove("BOARD_TOKEN")
        .env_remove("BOARD_AS")
        .env_remove("BOARD")
        .output()
        .unwrap();
    Run { code: out.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&out.stdout).into_owned(), err: String::from_utf8_lossy(&out.stderr).into_owned() }
}

const VIEW: &str = "export function Lightbox() {
  const title = 'lightbox title here';
  return (
    <div className=\"lightbox\">
      <img src={img.src} alt={img.alt} />
      <a className=\"btn sm\" href={img.src} download={img.alt || true}>
        下载
      </a>
    </div>
  );
}
";

/// 别的卡（#320）去掉下载按钮后的样子
const VIEW_320: &str = "export function Lightbox() {
  const title = 'lightbox title here';
  return (
    <div className=\"lightbox\">
      <img src={img.src} alt={img.alt} />
    </div>
  );
}
";

/// 这次的卡（#328）在旧代码上改的：换了变量名，下载按钮还在
const VIEW_328: &str = "export function Lightbox() {
  const title = 'lightbox title here';
  return (
    <div className=\"lightbox\">
      <img src={stage.src} alt={stage.alt} />
      <a className=\"btn sm\" href={stage.src} download={stage.alt || true}>
        下载
      </a>
    </div>
  );
}
";

const TEST_320: &str = "import { describe, it, expect } from 'vitest';
describe('lightbox regression', () => {
  it('has no download button anymore', () => {
    expect(document.querySelector('a[download]')).toBeNull();
  });
});
";

const TEST_328: &str = "import { describe, it, expect } from 'vitest';
describe('lightbox flicker', () => {
  it('keeps the previous image while loading', () => {
    expect(true).toBe(true);
  });
});
";

/// 远端（裸仓库）上有基础版本；返回（临时目录，远端，agent 的克隆）
fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let up = root.path().join("up.git");
    std::fs::create_dir_all(&up).unwrap();
    git(&up, &["init", "-q", "--bare", "-b", "main"]);
    let seed = root.path().join("seed");
    git(root.path(), &["clone", "-q", up.to_str().unwrap(), seed.to_str().unwrap()]);
    git(&seed, &["switch", "-q", "-c", "main"]);
    std::fs::write(seed.join("Lightbox.tsx"), VIEW).unwrap();
    std::fs::write(seed.join("other.ts"), "export const answer = 42;\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "feat: 图片预览\n\n卡片：default/main#300"]);
    git(&seed, &["push", "-q", "-u", "origin", "main"]);
    let work = root.path().join("work");
    git(root.path(), &["clone", "-q", up.to_str().unwrap(), work.to_str().unwrap()]);
    (root, up, work)
}

/// 「别人」推一张卡：去掉下载按钮、新建回归测试
fn push_320(root: &Path, up: &Path) {
    let other = root.join("other");
    git(root, &["clone", "-q", up.to_str().unwrap(), other.to_str().unwrap()]);
    std::fs::write(other.join("Lightbox.tsx"), VIEW_320).unwrap();
    std::fs::write(other.join("Lightbox.test.tsx"), TEST_320).unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-q", "-m", "fix(web): 去掉下载按钮\n\n卡片：default/main#320"]);
    git(&other, &["push", "-q", "origin", "main"]);
}

/// agent 按 work.md 开工：fetch、切到 origin/main
fn start_card(work: &Path) {
    git(work, &["fetch", "-q", "origin"]);
    git(work, &["switch", "-q", "--detach", "origin/main"]);
}

/// rebase 到最新的 origin/main 后压成一个提交
fn rebase_and_squash(work: &Path, msg: &str, take_mine: &[&str]) {
    git(work, &["fetch", "-q", "origin"]);
    if !git_try(work, &["rebase", "origin/main"]) {
        // 冲突：整份取自己那一版（rebase 里 theirs 是正在重放的自己的提交）
        for f in take_mine {
            git(work, &["checkout", "--theirs", "--", f]);
            git(work, &["add", "--", f]);
        }
        git(work, &["rebase", "--continue"]);
    }
    git(work, &["reset", "-q", "--soft", "origin/main"]);
    git(work, &["commit", "-q", "-m", msg]);
}

/// 验收：在旧代码上开工、rebase 冲突时取了自己那一版，推送前检查能拦下并指出被撤回的卡
#[test]
fn reports_changes_of_other_cards_reverted_by_conflict_resolution() {
    let (root, up, work) = setup();
    start_card(&work);
    // 开工之后别人推了 #320
    push_320(root.path(), &up);
    std::fs::write(work.join("Lightbox.tsx"), VIEW_328).unwrap();
    std::fs::write(work.join("Lightbox.test.tsx"), TEST_328).unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "fix(web): 切图不闪"]);
    rebase_and_squash(&work, "fix(web): 切图不闪\n\n卡片：default/main#328", &["Lightbox.tsx", "Lightbox.test.tsx"]);

    let r = touched(&work, &["--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    let j = r.json();
    assert!(j["base_from"].as_str().unwrap().starts_with("开工时切到的"), "{j}");
    assert_eq!(j["mine"], serde_json::json!(["default/main#328"]));
    let list = j["touched"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{j}");
    assert_eq!(list[0]["card"], "default/main#320");
    assert_eq!(list[0]["commits"][0]["subject"], "fix(web): 去掉下载按钮");
    let file = |name: &str| list[0]["files"].as_array().unwrap().iter().find(|f| f["path"] == name).cloned().unwrap_or_else(|| panic!("没有 {name}：{j}"));
    // 换了变量名加回的下载按钮也认得出
    let view = file("Lightbox.tsx");
    assert_eq!(view["readded"], 1, "{view}");
    assert!(view["readded_samples"][0].as_str().unwrap().contains("download={stage.alt"), "{view}");
    // 整份换掉的回归测试：它加的、这次删掉了（两边一样的 import 行不算）
    let test = file("Lightbox.test.tsx");
    assert_eq!(test["removed"], 3, "{test}");
    assert_eq!(j["summary"], "碰到的别的卡：default/main#320（Lightbox.test.tsx、Lightbox.tsx）");

    // 文字输出：卡号、提交、文件、例子，最后一行给交审评论用
    let r = touched(&work, &[]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("default/main#320") && r.out.contains("去掉下载按钮"), "{}", r.out);
    assert!(r.out.contains("Lightbox.tsx  加回了它删的 1 行"), "{}", r.out);
    assert!(r.out.contains("Lightbox.test.tsx  删掉了它加的 3 行"), "{}", r.out);
    assert_eq!(r.out.trim_end().lines().last(), Some("碰到的别的卡：default/main#320（Lightbox.test.tsx、Lightbox.tsx）"));
}

/// 正常的情况：开工后别人改了别的文件、或者这次改的是自己的行，没有碰到
#[test]
fn quiet_when_nothing_of_other_cards_is_touched() {
    let (root, up, work) = setup();
    start_card(&work);
    push_320(root.path(), &up);
    std::fs::write(work.join("other.ts"), "export const answer = 43;\nexport const question = 'what do you get';\n").unwrap();
    git(&work, &["commit", "-q", "-am", "feat: 改答案"]);
    rebase_and_squash(&work, "feat: 改答案\n\n卡片：default/main#330", &[]);
    let r = touched(&work, &["--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let j = r.json();
    assert!(j["touched"].as_array().unwrap().is_empty(), "{j}");
    assert_eq!(j["summary"], "碰到的别的卡：无");
    let r = touched(&work, &[]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("没有碰到别的卡最近的改动"), "{}", r.out);
    assert_eq!(r.out.trim_end().lines().last(), Some("碰到的别的卡：无"));
}

/// 同一张卡之前推过的提交（返工）不算；--card 指定这次的卡；回头查已经推上去的提交用 --base
#[test]
fn own_card_commits_and_history_mode() {
    let (root, up, work) = setup();
    start_card(&work);
    push_320(root.path(), &up);
    // 返工：#320 自己接着改，把下载按钮加回来（复盘说还是要）——是自己的提交，不算
    git(&work, &["fetch", "-q", "origin"]);
    git(&work, &["switch", "-q", "--detach", "origin/main"]);
    std::fs::write(work.join("Lightbox.tsx"), VIEW).unwrap();
    git(&work, &["commit", "-q", "-am", "fix(web): 下载按钮加回来\n\n卡片：default/main#320"]);
    let r = touched(&work, &["--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    // 换成别的卡（--card）再看：就是加回了 #320 删的
    let r = touched(&work, &["--card", "default/main#999", "--json"]);
    assert_eq!(r.code, 0, "开工之后没有别的提交：{}", r.out);
    let r = touched(&work, &["--card", "default/main#999", "--base", "HEAD~2", "--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    assert_eq!(r.json()["touched"][0]["card"], "default/main#320");
    assert_eq!(r.json()["touched"][0]["files"][0]["readded"], 1);

    // 已经推上去的提交：按它相对父提交算，--base 给出那时的基点
    git(&work, &["push", "-q", "origin", "HEAD:main"]);
    git(&work, &["fetch", "-q", "origin"]);
    let r = touched(&work, &["HEAD", "--card", "default/main#999", "--base", "HEAD~2", "--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    assert_eq!(r.json()["from"], git(&work, &["rev-parse", "HEAD~1"]).trim());

    // 写错的卡片引用、认不出的提交
    let r = touched(&work, &["--card", "main#1", "--json"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.json()["error"]["code"], "USAGE");
    let r = touched(&work, &["no-such-rev", "--json"]);
    assert_eq!(r.json()["error"]["code"], "USAGE");
}

/// board host 新建的 worktree（`git worktree add --detach`，reflog 里没有 checkout）：按建好时的地方认基点
#[test]
fn fresh_worktree_uses_where_it_was_created() {
    let (root, up, work) = setup();
    git(&work, &["fetch", "-q", "origin"]);
    let wt = root.path().join("wt");
    git(&work, &["worktree", "add", "-q", "--detach", wt.to_str().unwrap(), "origin/main"]);
    push_320(root.path(), &up);
    std::fs::write(wt.join("Lightbox.tsx"), VIEW_328).unwrap();
    git(&wt, &["commit", "-q", "-am", "fix(web): 切图不闪"]);
    rebase_and_squash(&wt, "fix(web): 切图不闪\n\n卡片：default/main#328", &["Lightbox.tsx"]);
    let r = touched(&wt, &["--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    let j = r.json();
    assert!(j["base_from"].as_str().unwrap().starts_with("建好 worktree 时的"), "{j}");
    assert_eq!(j["touched"][0]["card"], "default/main#320");
}

/// 一个 worktree 接着做下一张卡：上一张刚推完，开工时 HEAD 已经是最新的，switch 不动、reflog 里不记。
/// 基点是上一张卡推上去的提交，不把上一张卡和更早的提交算进「最近」
#[test]
fn next_card_in_same_worktree_starts_from_previous_push() {
    let (root, up, work) = setup();
    start_card(&work);
    // 上一张卡（#310）：改了 Lightbox，推上去
    std::fs::write(work.join("Lightbox.tsx"), VIEW.replace("lightbox title here", "lightbox title changed by 310")).unwrap();
    git(&work, &["commit", "-q", "-am", "fix: 标题\n\n卡片：default/main#310"]);
    git(&work, &["push", "-q", "origin", "HEAD:main"]);
    // 下一张卡（#311）：开工，HEAD 已经是 origin/main
    start_card(&work);
    std::fs::write(work.join("Lightbox.tsx"), VIEW.replace("lightbox title here", "lightbox title rewritten by 311")).unwrap();
    git(&work, &["commit", "-q", "-am", "fix: 再改标题"]);
    rebase_and_squash(&work, "fix: 再改标题\n\n卡片：default/main#311", &[]);
    let r = touched(&work, &["--json"]);
    assert_eq!(r.code, 0, "上一张卡的改动是看着改的，不算：{}{}", r.out, r.err);
    assert!(r.json()["base_from"].as_str().unwrap().starts_with("开工时所在的提交"), "{}", r.out);
    // 开工之后别人推了 #320，这次又解错了冲突：照样报出来
    let (root2, up2, work2) = setup();
    let _ = (&root, &up);
    start_card(&work2);
    std::fs::write(work2.join("other.ts"), "export const answer = 41;\n").unwrap();
    git(&work2, &["commit", "-q", "-am", "fix: 答案\n\n卡片：default/main#310"]);
    git(&work2, &["push", "-q", "origin", "HEAD:main"]);
    start_card(&work2);
    push_320(root2.path(), &up2);
    std::fs::write(work2.join("Lightbox.tsx"), VIEW_328).unwrap();
    git(&work2, &["commit", "-q", "-am", "fix(web): 切图不闪"]);
    rebase_and_squash(&work2, "fix(web): 切图不闪\n\n卡片：default/main#328", &["Lightbox.tsx"]);
    let r = touched(&work2, &["--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    assert_eq!(r.json()["touched"][0]["card"], "default/main#320");
}
