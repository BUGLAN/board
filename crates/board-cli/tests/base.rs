//! `board base`（default/main#552）：开工一条命令做完。临时的裸仓库当远端、测试服务端上领好卡片，
//! 覆盖卡片验收的几种情况：干净的仓库、现场不干净、从 wip 分支接着做（含冲突）、已经推过的卡、不是自己持有，
//! 以及 `board touched` 仍然认得出这次开工的基点。

mod common;

use common::Server;
use serde_json::Value;
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
    up: PathBuf,
    /// 项目的主仓库（agent 在这里执行 board base）
    main: PathBuf,
}

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
    let main = r.join("proj");
    git(&r, &["clone", "-q", up.to_str().unwrap(), main.to_str().unwrap()]);
    git(&main, &["config", "user.name", "agent"]);
    git(&main, &["config", "user.email", "agent@example.com"]);
    Repo { _root: root, root: r, up, main }
}

impl Repo {
    /// 「别人」在另一个克隆里推一个提交到 `branch`（从 `from` 开始）
    fn push(&self, clone: &str, from: &str, branch: &str, file: &str, text: &str, msg: &str) -> String {
        let other = self.root.join(clone);
        if !other.exists() {
            git(&self.root, &["clone", "-q", self.up.to_str().unwrap(), other.to_str().unwrap()]);
        }
        git(&other, &["fetch", "-q", "origin"]);
        git(&other, &["checkout", "-q", "--detach", from]);
        std::fs::write(other.join(file), text).unwrap();
        git(&other, &["add", "."]);
        git(&other, &["commit", "-q", "-m", msg]);
        git(&other, &["push", "-q", "origin", &format!("HEAD:refs/heads/{branch}")]);
        git(&other, &["rev-parse", "HEAD"])
    }
    fn tip(&self) -> String {
        git(&self.up, &["rev-parse", "refs/heads/main"])
    }
}

/// 同一个目录（macOS 的临时目录经过 /var → /private/var 的软链接，Windows 的规范路径带 \\?\ 前缀：两边都规范化再比）
fn same(a: &Path, b: &Path) -> bool {
    a.canonicalize().unwrap() == b.canonicalize().unwrap()
}

fn server() -> Server {
    let s = common::start();
    assert_eq!(s.run(ALPHA, &["new", "第一张", "--json"]).code, 0);
    assert_eq!(s.run(ALPHA, &["claim", "1", "--json"]).code, 0);
    s
}

fn base(s: &Server, dir: &Path) -> (i32, Value) {
    let r = s.run(ALPHA, &["base", "1", "--dir", dir.to_str().unwrap(), "--json"]);
    (r.code, serde_json::from_str(&r.out).unwrap_or_else(|e| panic!("{e}：{}{}", r.out, r.err)))
}

/// 干净的仓库：建好自己的 worktree（锁着名字、board host 那套目录名），停在最新的 origin/main，下一步直接改代码；
/// 再执行一次接着用；当前目录就是锁着自己名字的 worktree（board host 备好的）时用它
#[test]
fn creates_or_reuses_the_session_worktree() {
    let s = server();
    let r = repo();
    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 0, "{v}");
    assert_eq!((v["step"].as_str(), v["created"].as_bool(), v["branch"].as_str()), (Some("code"), Some(true), Some("main")), "{v}");
    let wt = PathBuf::from(v["worktree"].as_str().unwrap());
    assert!(same(wt.parent().unwrap(), &r.root.join("proj.wt")), "{}", wt.display());
    assert!(wt.file_name().unwrap().to_string_lossy().starts_with("test-alpha-"), "{}", wt.display());
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), r.tip());
    assert!(git(&r.main, &["worktree", "list", "--porcelain"]).contains("locked test/alpha"));
    assert!(v["next"].as_str().unwrap().contains("直接改代码"));

    // 再来一次：接着用；集成分支有了新提交，切到最新的
    let tip = r.push("other", "origin/main", "main", "b.txt", "pushed by someone else\n", "feat: 别的卡\n\n卡片：default/main#91");
    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["created"].as_bool(), Some(false));
    assert!(same(Path::new(v["worktree"].as_str().unwrap()), &wt), "{v}");
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), tip);

    // board host 备好的 worktree（别的目录名，锁着这个名字）：在它里面执行就用它
    let hosted = r.root.join("proj.wt").join("hosted");
    git(&r.main, &["worktree", "add", "-q", "--detach", "--lock", "--reason", ALPHA, hosted.to_str().unwrap(), "origin/main~1"]);
    let (code, v) = base(&s, &hosted);
    assert_eq!(code, 0, "{v}");
    assert!(same(Path::new(v["worktree"].as_str().unwrap()), &hosted), "{v}");
    assert_eq!(git(&hosted, &["rev-parse", "HEAD"]), tip);
}

/// 本地误建了同名分支 origin/main：base 仍切到远端最新提交
#[test]
fn uses_the_remote_ref_when_a_local_origin_branch_exists() {
    let s = server();
    let r = repo();
    let tip = r.push("other", "origin/main", "main", "b.txt", "pushed by someone else\n", "feat: 别的卡\n\n卡片：default/main#91");
    let old = git(&r.main, &["rev-parse", "refs/remotes/origin/main"]);
    git(&r.main, &["branch", "origin/main", &old]);

    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["base"].as_str().map(|b| tip.starts_with(b)), Some(true), "{v}");
    let wt = PathBuf::from(v["worktree"].as_str().unwrap());
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), tip);
}

/// 锁着自己名字、目录被人删了：先解掉旧登记，再新建 worktree，不能一直失败
#[test]
fn recovers_when_a_locked_worktree_directory_was_deleted() {
    let s = server();
    let r = repo();
    let (_, v) = base(&s, &r.main);
    let wt = PathBuf::from(v["worktree"].as_str().unwrap());
    std::fs::remove_dir_all(&wt).unwrap();

    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["created"], true, "{v}");
    assert!(same(Path::new(v["worktree"].as_str().unwrap()), &wt), "{v}");
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), r.tip());
    let list = git(&r.main, &["worktree", "list", "--porcelain"]);
    assert_eq!(list.matches(&format!("worktree {}", wt.display())).count(), 1, "{list}");
}

/// 用完留着、下一个会话接手（default/main#622）：另一个会话还在线、锁着它的 worktree 时不碰它的，自己新建；
/// 那个会话结束、解锁之后，再来的会话（换了名字）接手它留下的 worktree：锁换成自己，停在最新的集成分支上，
/// 里面的依赖、构建产物（被忽略的文件）都还在
#[test]
fn adopts_the_idle_worktree_another_session_left() {
    const GAMMA: &str = "test/gamma";
    let s = server();
    let r = repo();
    let (_, v) = base(&s, &r.main);
    let wt = PathBuf::from(v["worktree"].as_str().unwrap());
    assert_eq!((v["created"].as_bool(), v["adopted"].as_bool()), (Some(true), Some(false)), "{v}");
    std::fs::write(r.main.join(".git/info/exclude"), "target/\n").unwrap();
    std::fs::create_dir_all(wt.join("target")).unwrap();
    std::fs::write(wt.join("target/built"), "x").unwrap();

    // alpha 还在线：beta 不接手，新建自己的
    let run = |who: &str, id: &str| {
        assert_eq!(s.run(who, &["new", "又一张", "--json"]).code, 0);
        assert_eq!(s.run(who, &["claim", id, "--json"]).code, 0);
        let out = s.run(who, &["base", id, "--dir", r.main.to_str().unwrap(), "--json"]);
        (out.code, serde_json::from_str::<Value>(&out.out).unwrap_or_else(|e| panic!("{e}：{}{}", out.out, out.err)))
    };
    let (code, v) = run(BETA, "2");
    assert_eq!(code, 0, "{v}");
    assert_eq!((v["created"].as_bool(), v["adopted"].as_bool()), (Some(true), Some(false)), "{v}");
    assert!(!same(Path::new(v["worktree"].as_str().unwrap()), &wt));

    // alpha 结束、解锁；集成分支又有了新提交。gamma 接手 alpha 留下的
    git(&r.main, &["worktree", "unlock", wt.to_str().unwrap()]);
    let tip = r.push("other", "origin/main", "main", "b.txt", "pushed by someone else\n", "feat: 别的卡\n\n卡片：default/main#91");
    let (code, v) = run(GAMMA, "3");
    assert_eq!(code, 0, "{v}");
    assert_eq!((v["created"].as_bool(), v["adopted"].as_bool(), v["step"].as_str()), (Some(false), Some(true), Some("code")), "{v}");
    assert!(same(Path::new(v["worktree"].as_str().unwrap()), &wt), "{v}");
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), tip);
    assert!(wt.join("target/built").exists(), "构建产物还在");
    assert!(git(&r.main, &["worktree", "list", "--porcelain"]).contains("locked test/gamma"));
    assert_eq!(v["cleaned"], serde_json::json!([]), "{v}");
}

/// worktree 里有没提交的改动、没推的提交：退出码 1，不切换、不丢改动，列出来并写明怎么办
#[test]
fn keeps_unsaved_changes_and_does_not_switch() {
    let s = server();
    let r = repo();
    let (_, v) = base(&s, &r.main);
    let wt = PathBuf::from(v["worktree"].as_str().unwrap());
    std::fs::write(wt.join("half.txt"), "half done\n").unwrap();
    r.push("other", "origin/main", "main", "b.txt", "pushed by someone else\n", "feat: 别的卡\n\n卡片：default/main#91");
    let head = git(&wt, &["rev-parse", "HEAD"]);
    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["step"], "save");
    assert!(v["dirty"].as_array().unwrap().iter().any(|f| f.as_str().unwrap().contains("half.txt")), "{v}");
    let next = v["next"].as_str().unwrap();
    assert!(next.contains("board land <上一张卡 ID> --wip") && next.contains("board land 1 --wip") && next.contains("presync"), "{next}");
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), head, "没有切换");
    assert!(wt.join("half.txt").exists(), "改动还在");

    // 只有没推的提交也一样
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "local only"]);
    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 1);
    assert_eq!(v["unpushed"].as_array().unwrap().len(), 1, "{v}");
}

/// 卡片评论里有「未完成的改动：wip/main/1」、远端有这条分支：切过去、rebase 到最新集成分支；冲突时停在冲突上
#[test]
fn continues_from_the_wip_branch() {
    let s = server();
    let r = repo();
    let start = r.tip();
    r.push("wip", &start, "wip/main/1", "w.txt", "half done by the last agent\n", "wip: 未完成的改动");
    let tip = r.push("other", "origin/main", "main", "b.txt", "pushed by someone else\n", "feat: 别的卡\n\n卡片：default/main#91");
    assert_eq!(s.run(ALPHA, &["comment", "1", "检查不过\n\n未完成的改动：wip/main/1", "--json"]).code, 0);
    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 0, "{v}");
    assert_eq!((v["step"].as_str(), v["from_wip"].as_str()), (Some("wip"), Some("wip/main/1")), "{v}");
    let wt = PathBuf::from(v["worktree"].as_str().unwrap());
    assert_eq!(git(&wt, &["rev-parse", "HEAD~1"]), tip, "半成品 rebase 到了最新的集成分支上");
    assert!(wt.join("w.txt").exists() && wt.join("b.txt").exists());
}

#[test]
fn stops_on_wip_rebase_conflicts() {
    let s = server();
    let r = repo();
    let start = r.tip();
    r.push("wip", &start, "wip/main/1", "a.txt", "first line changed by the wip\nsecond line of the file\n", "wip: 未完成的改动");
    r.push("other", "origin/main", "main", "a.txt", "first line changed on main\nsecond line of the file\n", "feat: 别的卡\n\n卡片：default/main#91");
    assert_eq!(s.run(ALPHA, &["comment", "1", "未完成的改动：wip/main/1", "--json"]).code, 0);
    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["step"], "resolve");
    assert_eq!(v["conflicts"], serde_json::json!(["a.txt"]));
    assert!(v["next"].as_str().unwrap().contains("git rebase --continue"));
    let wt = PathBuf::from(v["worktree"].as_str().unwrap());
    let head = git(&wt, &["rev-parse", "HEAD"]);

    // 再跑一次 base：认得出还在 rebase 中途，不切换
    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["step"], "resolve", "{v}");
    let next = v["next"].as_str().unwrap();
    assert!(next.contains("rebase") && next.contains("git rebase --continue") && next.contains("git rebase --abort"), "{next}");
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), head, "没有切换");
    git(&wt, &["rebase", "--abort"]);
}

/// 评论里是别的卡号的 wip 分支：不切过去
#[test]
fn ignores_a_wip_branch_for_another_card() {
    let s = server();
    let r = repo();
    let start = r.tip();
    r.push("wip", &start, "wip/main/2", "wrong.txt", "别的卡\n", "wip: 未完成的改动");
    assert_eq!(s.run(ALPHA, &["comment", "1", "未完成的改动：wip/main/2", "--json"]).code, 0);
    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 0, "{v}");
    assert_eq!((v["step"].as_str(), v["from_wip"].as_str()), (Some("code"), None), "{v}");
    let wt = PathBuf::from(v["worktree"].as_str().unwrap());
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), r.tip());
    assert!(!wt.join("wrong.txt").exists(), "没有切到别的卡的分支");
}

/// 集成分支上已经有这张卡的提交：动态里没有「提交：」行是推过没交审，有就是被打回接着修
#[test]
fn reports_commits_already_on_the_integration_branch() {
    let s = server();
    let r = repo();
    let sha = r.push("other", "origin/main", "main", "b.txt", "done\n", "feat: 第一张\n\n卡片：default/main#1");
    let (code, v) = base(&s, &r.main);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["step"], "submit", "{v}");
    assert!(sha.starts_with(v["landed"][0]["commit"].as_str().unwrap()), "{v}");
    assert_eq!(v["landed"][0]["reported"], false);

    let comment = format!("做完了\n\n提交：{}\n集成分支：main\n检查：过了\n碰到的别的卡：无", &sha[..7]);
    assert_eq!(s.run(ALPHA, &["comment", "1", &comment, "--json"]).code, 0);
    let (_, v) = base(&s, &r.main);
    assert_eq!(v["step"], "rework", "{v}");
    assert_eq!(v["landed"][0]["reported"], true);
}

/// 卡片不是自己持有：NOT_HOLDER，什么都不动（不建 worktree）
#[test]
fn refuses_cards_held_by_someone_else() {
    let s = common::start();
    assert_eq!(s.run(ALPHA, &["new", "第一张", "--json"]).code, 0);
    assert_eq!(s.run(BETA, &["claim", "1", "--json"]).code, 0);
    let r = repo();
    let out = s.run(ALPHA, &["base", "1", "--dir", r.main.to_str().unwrap(), "--json"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.error_code(), "NOT_HOLDER", "{}", out.out);
    assert!(!r.root.join("proj.wt").exists(), "没有建 worktree");
}

/// 同一个 worktree 接着做下一张：board base 切到最新集成分支那一下，board touched 认得出是开工时的基点
#[test]
fn touched_finds_the_base_it_switched_to() {
    let s = server();
    let r = repo();
    let (_, v) = base(&s, &r.main);
    let wt = PathBuf::from(v["worktree"].as_str().unwrap());
    let tip = r.push("other", "origin/main", "main", "b.txt", "pushed by someone else\n", "feat: 别的卡\n\n卡片：default/main#91");
    let (_, v) = base(&s, &r.main);
    assert_eq!(v["base"].as_str().map(|b| tip.starts_with(b)), Some(true), "{v}");
    std::fs::write(wt.join("c.txt"), "the change of this card\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "feat: 第一张\n\n卡片：default/main#1"]);
    let out = s.raw(&[], &["touched", "--dir", wt.to_str().unwrap(), "--json"]);
    let t: Value = serde_json::from_str(&out.out).unwrap_or_else(|e| panic!("{e}：{}{}", out.out, out.err));
    assert_eq!(t["base"].as_str(), Some(tip.as_str()), "{t}");
    assert!(t["base_from"].as_str().unwrap().starts_with("开工时切到的"), "{t}");
}
