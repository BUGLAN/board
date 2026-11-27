//! `board land`（default/main#553）：推送一条命令做完。临时的裸仓库当远端，测试服务端上领好卡片，
//! 覆盖卡片验收的七种情况：正常推送、不在最新集成分支之上、碰到别的卡、不再持有、推送被拒、删接着做的 wip 分支、--wip。

mod common;

use common::Server;
use std::path::{Path, PathBuf};
use std::process::Command;

const ALPHA: &str = "test/alpha";
const BETA: &str = "test/beta";

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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Repo {
    _root: tempfile::TempDir,
    root: PathBuf,
    /// 远端（裸仓库）
    up: PathBuf,
    /// agent 的工作区（已经按 work.md 开工：切到 origin/main）
    work: PathBuf,
    /// 写好的提交说明
    msg: PathBuf,
}

/// 远端上有基础版本；agent 克隆、开工（切到 origin/main）
fn repo() -> Repo {
    let root = tempfile::tempdir().unwrap();
    let r = root.path().to_path_buf();
    let up = r.join("up.git");
    std::fs::create_dir_all(&up).unwrap();
    git(&up, &["init", "-q", "--bare", "-b", "main"]);
    let seed = r.join("seed");
    git(&r, &["clone", "-q", up.to_str().unwrap(), seed.to_str().unwrap()]);
    git(&seed, &["switch", "-q", "-c", "main"]);
    std::fs::write(seed.join("a.txt"), "first line of the file\nsecond line of the file\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "chore: 起步\n\n卡片：default/main#90"]);
    git(&seed, &["push", "-q", "-u", "origin", "main"]);
    let work = r.join("work");
    git(&r, &["clone", "-q", up.to_str().unwrap(), work.to_str().unwrap()]);
    git(&work, &["config", "user.name", "agent"]);
    git(&work, &["config", "user.email", "agent@example.com"]);
    git(&work, &["switch", "-q", "--detach", "origin/main"]);
    let msg = r.join("msg.txt");
    std::fs::write(&msg, "feat: 第三行\n\n- a.txt 加了一行\n").unwrap();
    Repo { _root: root, root: r, up, work, msg }
}

impl Repo {
    /// agent 在工作区里提交一次
    fn commit(&self, file: &str, text: &str, msg: &str) {
        std::fs::write(self.work.join(file), text).unwrap();
        git(&self.work, &["add", "."]);
        git(&self.work, &["commit", "-q", "-m", msg]);
    }
    /// 「别人」在另一个克隆里推一张卡
    fn push_other(&self, name: &str, file: &str, text: &str, msg: &str) {
        let other = self.root.join(name);
        git(&self.root, &["clone", "-q", self.up.to_str().unwrap(), other.to_str().unwrap()]);
        std::fs::write(other.join(file), text).unwrap();
        git(&other, &["add", "."]);
        git(&other, &["commit", "-q", "-m", msg]);
        git(&other, &["push", "-q", "origin", "main"]);
    }
    /// 远端集成分支的提交
    fn main(&self) -> String {
        git(&self.up, &["rev-parse", "refs/heads/main"])
    }
}

/// 测试服务端上建一张卡，ALPHA 领走
fn server() -> Server {
    let s = common::start();
    assert_eq!(s.run(ALPHA, &["new", "第三行", "--json"]).code, 0);
    assert_eq!(s.run(ALPHA, &["claim", "1", "--json"]).code, 0);
    s
}

fn land(s: &Server, r: &Repo, extra: &[&str]) -> common::Run {
    let mut args = vec!["land", "1", "--dir", r.work.to_str().unwrap(), "--json"];
    args.extend_from_slice(extra);
    s.run(ALPHA, &args)
}

fn normal<'a>(r: &'a Repo, extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec!["--message-file", r.msg.to_str().unwrap(), "--checked", "cargo test 通过"];
    a.extend_from_slice(extra);
    a
}

/// 正常情况：两个提交压成一个，最后一行是卡片，推到集成分支，输出交审评论的四行
#[test]
fn squashes_pushes_and_prints_review_lines() {
    let s = server();
    let r = repo();
    let base = r.main();
    r.commit("a.txt", "first line of the file\nsecond line of the file\nthird line from the agent\n", "wip 1");
    r.commit("b.txt", "another file from the agent\n", "wip 2");
    let out = land(&s, &r, &normal(&r, &[]));
    assert_eq!(out.code, 0, "{}{}", out.out, out.err);
    let v = out.json();
    assert_eq!(v["pushed"], true);
    assert_eq!(v["branch"], "main");
    let head = r.main();
    assert!(head.starts_with(v["commit"].as_str().unwrap()), "{v}");
    // 压成了一个提交，父提交是开工时的集成分支
    assert_eq!(git(&r.up, &["rev-parse", "refs/heads/main^"]), base);
    let body = git(&r.up, &["log", "-1", "--format=%B", "refs/heads/main"]);
    assert!(body.starts_with("feat: 第三行"), "{body}");
    assert_eq!(body.lines().last(), Some("卡片：default/main#1"), "{body}");
    let lines = v["review_lines"].as_str().unwrap();
    assert_eq!(
        lines,
        format!("提交：{}\n集成分支：main\n检查：cargo test 通过\n碰到的别的卡：无", v["commit"].as_str().unwrap())
    );
    // 本地认得刚推上去的提交：worktree 是干净的
    assert!(!git(&r.work, &["branch", "-r", "--contains", "HEAD"]).is_empty());

    // 参数缺了直接拒绝
    let e = land(&s, &r, &["--checked", "x"]);
    assert_eq!(e.error_code(), "USAGE");
    assert!(e.json()["error"]["message"].as_str().unwrap().contains("--message-file"));
    let e = land(&s, &r, &["--message-file", r.msg.to_str().unwrap()]);
    assert!(e.json()["error"]["message"].as_str().unwrap().contains("--checked"));
}

/// 本地误建了同名分支 origin/main：land 仍以远端最新提交为基点，不被旧分支带回去
#[test]
fn uses_the_remote_ref_when_a_local_origin_branch_exists() {
    let s = server();
    let r = repo();
    let old = git(&r.work, &["rev-parse", "HEAD"]);
    r.push_other("other", "c.txt", "pushed by someone else\n", "feat: 别的卡\n\n卡片：default/main#91");
    let latest = r.main();
    git(&r.work, &["fetch", "-q", "origin"]);
    git(&r.work, &["rebase", "-q", "refs/remotes/origin/main"]);
    git(&r.work, &["branch", "origin/main", &old]);
    r.commit("d.txt", "my change\n", "wip");

    let out = land(&s, &r, &normal(&r, &[]));
    assert_eq!(out.code, 0, "{}{}", out.out, out.err);
    assert_eq!(git(&r.up, &["rev-parse", "refs/heads/main^"]), latest);
}

/// HEAD 不在最新的集成分支之上：不压、不推，写明先 rebase、重新跑检查
#[test]
fn refuses_when_not_on_top_of_latest_integration_branch() {
    let s = server();
    let r = repo();
    r.commit("b.txt", "another file from the agent\n", "wip 1");
    r.commit("b.txt", "another file from the agent, changed\n", "wip 2");
    r.push_other("other", "c.txt", "someone else pushed first\n", "feat: 别的卡\n\n卡片：default/main#91");
    let before = git(&r.work, &["rev-parse", "HEAD"]);
    let remote = r.main();
    let out = land(&s, &r, &normal(&r, &[]));
    assert_eq!(out.code, 1);
    assert_eq!(out.error_code(), "USAGE");
    let m = out.json()["error"]["message"].as_str().unwrap().to_string();
    assert!(m.contains("git rebase origin/main") && m.contains("重新跑检查"), "{m}");
    assert_eq!(git(&r.work, &["rev-parse", "HEAD"]), before, "没有压提交");
    assert_eq!(r.main(), remote, "没有推");

    // 工作区有没提交的改动：同样什么都不做
    std::fs::write(r.work.join("b.txt"), "dirty\n").unwrap();
    let out = land(&s, &r, &normal(&r, &[]));
    assert!(out.json()["error"]["message"].as_str().unwrap().contains("没提交的文件"), "{}", out.out);
}

/// 删掉了别的卡刚加的行：退出码 1、没有推；确认有意后 --touched-ok 推上去，最后一行照 touched
#[test]
fn stops_on_touched_until_confirmed() {
    let s = server();
    let r = repo();
    r.push_other("other", "a.txt", "first line of the file\nsecond line of the file\nthird line added by card ninety nine\n", "feat: 加一行\n\n卡片：default/main#99");
    // 开工之后集成分支有了 #99；rebase 上去，然后把它加的那行删了
    git(&r.work, &["fetch", "-q", "origin"]);
    git(&r.work, &["rebase", "-q", "origin/main"]);
    r.commit("a.txt", "first line of the file\nsecond line of the file\nmy own line replaces it all\n", "wip");
    let remote = r.main();
    let out = land(&s, &r, &normal(&r, &[]));
    assert_eq!(out.code, 1, "{}{}", out.out, out.err);
    let v = out.json();
    assert_eq!((v["pushed"].as_bool(), v["reason"].as_str()), (Some(false), Some("touched")), "{v}");
    assert!(v["touched"].as_str().unwrap().contains("default/main#99"), "{v}");
    assert_eq!(r.main(), remote, "没有推");
    let squashed = git(&r.work, &["rev-parse", "HEAD"]);

    let out = land(&s, &r, &normal(&r, &["--touched-ok"]));
    assert_eq!(out.code, 0, "{}{}", out.out, out.err);
    let v = out.json();
    assert_eq!(r.main(), squashed, "压好的提交没有重压");
    assert!(v["touched"].as_str().unwrap().starts_with("碰到的别的卡：default/main#99"), "{v}");
    assert!(v["review_lines"].as_str().unwrap().ends_with(v["touched"].as_str().unwrap()));
}

/// 卡片已经不归自己持有：NOT_HOLDER，没有推
#[test]
fn does_not_push_when_no_longer_holding() {
    let s = common::start();
    assert_eq!(s.run(ALPHA, &["new", "第三行", "--json"]).code, 0);
    assert_eq!(s.run(BETA, &["claim", "1", "--json"]).code, 0);
    let r = repo();
    r.commit("b.txt", "another file from the agent\n", "wip");
    let remote = r.main();
    let out = land(&s, &r, &normal(&r, &[]));
    assert_eq!(out.code, 1);
    assert_eq!(out.error_code(), "NOT_HOLDER", "{}", out.out);
    assert!(out.json()["error"]["message"].as_str().unwrap().contains("wip 分支"));
    assert_eq!(r.main(), remote);
}

/// 推送被拒（别人在 fetch 之后先推了）：退出码 1，本地压好的提交还在，写明先 rebase、重新跑检查
#[test]
fn rejected_push_keeps_the_squashed_commit() {
    let s = server();
    let r = repo();
    r.commit("b.txt", "another file from the agent\n", "wip");
    // 推的地址是另一个已经有新提交的远端：fetch 看到的集成分支是旧的，推的时候被拒
    let ahead = r.root.join("ahead.git");
    git(&r.root, &["clone", "-q", "--bare", r.up.to_str().unwrap(), ahead.to_str().unwrap()]);
    let other = r.root.join("other");
    git(&r.root, &["clone", "-q", ahead.to_str().unwrap(), other.to_str().unwrap()]);
    std::fs::write(other.join("c.txt"), "pushed in between\n").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-q", "-m", "feat: 抢先\n\n卡片：default/main#92"]);
    git(&other, &["push", "-q", "origin", "main"]);
    git(&r.work, &["config", "remote.origin.pushurl", ahead.to_str().unwrap()]);
    let remote = r.main();
    let out = land(&s, &r, &normal(&r, &[]));
    assert_eq!(out.code, 1, "{}{}", out.out, out.err);
    let v = out.json();
    assert_eq!((v["pushed"].as_bool(), v["reason"].as_str()), (Some(false), Some("rejected")), "{v}");
    let hint = v["hint"].as_str().unwrap();
    assert!(hint.contains("git rebase origin/main") && hint.contains("重新跑检查") && hint.contains("--force"), "{hint}");
    assert_eq!(git(&r.work, &["log", "-1", "--format=%B"]).lines().last(), Some("卡片：default/main#1"), "压好的提交还在");
    assert_eq!(r.main(), remote);
}

/// 从 wip 分支接着做的卡：推上去之后远端的 wip 分支被删掉
#[test]
fn deletes_the_wip_branch_it_continued_from() {
    let s = server();
    let r = repo();
    r.commit("b.txt", "half done\n", "wip: 未完成的改动");
    git(&r.work, &["push", "-q", "origin", "HEAD:refs/heads/wip/main/1"]);
    assert_eq!(s.run(ALPHA, &["comment", "1", "检查不过\n\n未完成的改动：wip/main/1", "--json"]).code, 0);
    r.commit("b.txt", "all done now\n", "wip 2");
    let out = land(&s, &r, &normal(&r, &[]));
    assert_eq!(out.code, 0, "{}{}", out.out, out.err);
    assert_eq!(out.json()["wip_deleted"], "wip/main/1");
    assert_eq!(git(&r.up, &["branch", "--list", "wip/main/1"]), "", "远端的 wip 分支删掉了");
}

/// --wip：没做完的改动推到 wip/<白板标识>/<ID>，集成分支不变；再做不下去时覆盖
#[test]
fn wip_saves_unfinished_changes() {
    let s = server();
    let r = repo();
    std::fs::write(r.work.join("a.txt"), "half way there\n").unwrap();
    std::fs::write(r.work.join("new.txt"), "untracked too\n").unwrap();
    let remote = r.main();
    let out = s.run(ALPHA, &["land", "1", "--wip", "--dir", r.work.to_str().unwrap(), "--json"]);
    assert_eq!(out.code, 0, "{}{}", out.out, out.err);
    let v = out.json();
    assert_eq!(v["branch"], "wip/main/1");
    assert_eq!(v["line"], "未完成的改动：wip/main/1");
    assert_eq!(r.main(), remote, "集成分支没有变化");
    assert_eq!(git(&r.up, &["show", "refs/heads/wip/main/1:new.txt"]), "untracked too");
    assert_eq!(git(&r.work, &["status", "--porcelain"]), "");

    // 接着做又做不下去：同一条分支覆盖
    std::fs::write(r.work.join("a.txt"), "still not done\n").unwrap();
    let out = s.run(ALPHA, &["land", "1", "--wip", "--dir", r.work.to_str().unwrap(), "--json"]);
    assert_eq!(out.code, 0, "{}{}", out.out, out.err);
    assert_eq!(git(&r.up, &["show", "refs/heads/wip/main/1:a.txt"]), "still not done");

    // 都推了：没有要保存的
    let out = s.run(ALPHA, &["land", "1", "--wip", "--dir", r.work.to_str().unwrap(), "--json"]);
    assert_eq!(out.error_code(), "USAGE");
}
