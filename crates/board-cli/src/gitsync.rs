//! board host 这一侧的 Git 凭据同步（PRD §10.16「共用配置」，服务端见 board-core 的 `gitcred`）：
//! 服务端设了一份 https 远端的令牌、这台机器选了「用服务器的」，就把它配进 Board 管的仓库（工作空间里每个项目的目录），
//! board host 备代码（`git fetch`、`git clone`）和 agent 推集成分支时直接用，不用每台机器手动配 credential helper。
//!
//! - **令牌放哪**：`~/.board/host/board-git-credentials`，git 自带的 `credential-store` 的格式（一行
//!   `https://<用户名>:<令牌>@<主机>`），只有本人能读写（0600）。git 配置里只写 helper 的命令，不写令牌；
//!   board 的输出、日志、上报里也没有令牌，只有主机和用户名。
//! - **配进哪**：每个仓库的本地配置（`git config --local`，同一个仓库的 worktree 共用），只配凭据的那个主机：
//!   `credential.https://<主机>.helper` 先写一个空值（清掉系统、全局配置里的 helper，免得 Git Credential Manager 弹窗、
//!   钥匙串里旧的令牌抢先），再写 `store --file '<上面的文件>'`。用户的全局 `credential.helper` 不碰。
//! - **还原**：选了「不改」、服务端删了凭据时，去掉配进去的这两项（仓库里用户自己写的同名项留着）、删掉文件；
//!   配过哪些仓库记在 `~/.board/host/git-sync.json`，工作空间换了、项目不在了也能找回去。
//! - **之后才有的仓库**：同步之后才克隆、才建的仓库，克隆时用 `git -c` 带上同样的 helper，开始派发前（[`ensure`]）补配。
//! - 只管 https 远端：远端是 SSH（`git@…`）、主机和凭据的不一样时不配；主机不一样算没配上（同步失败，写明是哪几个仓库）。

use board_core::gitcred::{https_host, GitCred};
use board_core::GitSync;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// 令牌文件名（helper 的命令里带着它，据此认出哪些配置项是 Board 写的）
const FILE: &str = "board-git-credentials";
/// 配过哪些仓库
const RECORD: &str = "git-sync.json";
const QUICK: Duration = Duration::from_secs(10);

/// 同步（后台线程）和开始派发前补配（主线程）都会改记录，一次只让一个做
static LOCK: Mutex<()> = Mutex::new(());

/// 配过哪些仓库、配的是哪个主机；主机为空表示现在没在用
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    host: String,
    #[serde(default)]
    repos: Vec<String>,
}

fn load(dir: &Path) -> Record {
    std::fs::read_to_string(dir.join(RECORD)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(dir: &Path, r: &Record) {
    if r.host.is_empty() && r.repos.is_empty() {
        let _ = std::fs::remove_file(dir.join(RECORD));
    } else {
        let _ = std::fs::write(dir.join(RECORD), serde_json::to_string_pretty(r).unwrap_or_default());
    }
}

fn key(host: &str) -> String {
    format!("credential.https://{host}.helper")
}

/// helper 的命令：`store --file '<文件>'`。git 用 sh 跑它（Windows 上是 Git 自带的 sh），路径换成 `/`、用单引号括起来
fn helper(file: &Path) -> String {
    let p = file.to_string_lossy().replace('\\', "/");
    format!("store --file '{}'", p.replace('\'', r"'\''"))
}

/// 这个值是 Board 写的：清空用的空值，或指向 Board 的令牌文件的 `store`
fn ours(v: &str) -> bool {
    v.is_empty() || (v.starts_with("store --file") && v.contains(FILE))
}

/// 百分号编码（credential-store 的文件里用户名、令牌按 URL 写）
fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 写令牌文件：先写临时文件再换上，只有本人能读写
fn write_file(path: &Path, c: &GitCred, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&tmp)?;
    f.write_all(format!("https://{}:{}@{}\n", enc(&c.username), enc(token), c.host).as_bytes())?;
    f.sync_all()?;
    drop(f);
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(path);
        std::fs::rename(&tmp, path)?;
    }
    Ok(())
}

/// 仓库本地配置里这一项的所有值（按顺序；空值也算一个）
fn values(repo: &Path, k: &str) -> Vec<String> {
    match crate::code::git(repo, &["config", "--local", "--null", "--get-all", k], QUICK) {
        Ok(out) => {
            let mut v: Vec<String> = out.split('\0').map(str::to_string).collect();
            if v.last().is_some_and(String::is_empty) {
                v.pop();
            }
            v
        }
        Err(_) => vec![],
    }
}

/// 把这一项改成 `want`（按顺序）；已经是了就不动
fn set_values(repo: &Path, k: &str, now: &[String], want: &[String]) -> Result<bool, String> {
    if now == want {
        return Ok(false);
    }
    if !now.is_empty() {
        crate::code::git(repo, &["config", "--local", "--unset-all", k], QUICK)?;
    }
    for v in want {
        crate::code::git(repo, &["config", "--local", "--add", k, v], QUICK)?;
    }
    Ok(true)
}

/// 配进一个仓库：用户自己写的同名项留在前面，后面接 Board 的空值和 helper
fn configure(repo: &Path, host: &str, file: &Path) -> Result<bool, String> {
    let k = key(host);
    let now = values(repo, &k);
    let mut want: Vec<String> = now.iter().filter(|v| !ours(v)).cloned().collect();
    want.push(String::new());
    want.push(helper(file));
    set_values(repo, &k, &now, &want)
}

/// 从一个仓库里去掉 Board 写的（用户自己写的留着）；仓库不在了、不是仓库了就算了
fn unconfigure(repo: &Path, host: &str) -> Result<bool, String> {
    if host.is_empty() || !repo.is_dir() {
        return Ok(false);
    }
    let k = key(host);
    let now = values(repo, &k);
    let keep: Vec<String> = now.iter().filter(|v| !ours(v)).cloned().collect();
    set_values(repo, &k, &now, &keep)
}

/// 仓库的 `origin`；不是仓库、没有 origin 时 None
fn origin(repo: &Path) -> Option<String> {
    if !repo.is_dir() {
        return None;
    }
    crate::code::git(repo, &["remote", "get-url", "origin"], QUICK).ok().filter(|u| !u.is_empty())
}

fn shown(p: &Path) -> String {
    p.display().to_string()
}

/// 一次同步的结果和要打印的变化（不含令牌）
pub struct Out {
    pub sync: GitSync,
    pub changes: Vec<String>,
}

/// 去掉记录里所有仓库（加上这一次的仓库）里 Board 写的、删掉令牌文件
fn clean(dir: &Path, rec: &Record, repos: &[PathBuf], changes: &mut Vec<String>) {
    let mut all: Vec<PathBuf> = rec.repos.iter().map(PathBuf::from).collect();
    all.extend(repos.iter().cloned());
    all.sort();
    all.dedup();
    for r in &all {
        if let Ok(true) = unconfigure(r, &rec.host) {
            changes.push(format!("Git 凭据：从 {} 里去掉了（{}）", shown(r), rec.host));
        }
    }
    if dir.join(FILE).exists() {
        let _ = std::fs::remove_file(dir.join(FILE));
        changes.push("Git 凭据：删掉了本机的令牌文件".into());
    }
    save(dir, &Record::default());
}

/// 同步：`on` 是机器上选了「用服务器的」，`cred` 是服务端的（没设时 None），`repos` 是 Board 管的仓库（工作空间里每个项目的目录），
/// `dir` 是 `~/.board/host`
pub fn sync(on: bool, cred: Option<&GitCred>, repos: &[PathBuf], dir: &Path) -> Out {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rec = load(dir);
    let mut changes = vec![];
    let token = cred.and_then(|c| c.token.as_deref()).map(str::trim).filter(|t| !t.is_empty());
    let (Some(c), Some(token), true) = (cred, token, on) else {
        clean(dir, &rec, repos, &mut changes);
        let sync = match (on, cred) {
            (false, _) => GitSync { state: "off".into(), ..Default::default() },
            (true, None) => GitSync { state: "none".into(), ..Default::default() },
            (true, Some(c)) => GitSync { state: "failed".into(), host: c.host.clone(), why: Some("服务端的 Git 凭据没有令牌".into()), ..Default::default() },
        };
        return Out { sync, changes };
    };
    let file = dir.join(FILE);
    if let Err(e) = write_file(&file, c, token) {
        clean(dir, &rec, repos, &mut changes);
        return Out { sync: GitSync { state: "failed".into(), host: c.host.clone(), why: Some(format!("写不了 {}：{e}", shown(&file))), ..Default::default() }, changes };
    }
    // 主机换了：先把旧主机的那一项去掉
    if !rec.host.is_empty() && rec.host != c.host {
        for r in &rec.repos {
            let _ = unconfigure(Path::new(r), &rec.host);
        }
    }
    let mut ok: Vec<String> = vec![];
    let mut skipped: Vec<String> = vec![];
    let mut mismatch: Vec<String> = vec![];
    let mut errs: Vec<String> = vec![];
    for r in repos {
        let Some(url) = origin(r) else { continue };
        match https_host(&url) {
            Some(h) if h == c.host => match configure(r, &c.host, &file) {
                Ok(changed) => {
                    if changed {
                        changes.push(format!("Git 凭据：配进了 {}（{}）", shown(r), c.host));
                    }
                    ok.push(shown(r));
                }
                Err(e) => errs.push(format!("{}：{e}", shown(r))),
            },
            Some(h) => mismatch.push(format!("{}（远端是 {h}）", shown(r))),
            None => skipped.push(format!("{}：远端不是 https 地址，用本机自己的凭据", shown(r))),
        }
    }
    // 记录里有、这次没配的（远端换了、项目不在了）：去掉
    for r in rec.repos.iter().filter(|r| !ok.contains(r)) {
        if let Ok(true) = unconfigure(Path::new(r), &c.host) {
            changes.push(format!("Git 凭据：从 {r} 里去掉了"));
        }
    }
    save(dir, &Record { host: c.host.clone(), repos: ok.clone() });
    let mut why = vec![];
    if !mismatch.is_empty() {
        why.push(format!("主机不匹配：{} 和凭据的 {} 不一样", mismatch.join("、"), c.host));
    }
    if !errs.is_empty() {
        why.push(format!("git 配置没写进去：{}", errs.join("；")));
    }
    let sync = GitSync {
        state: if why.is_empty() { "synced" } else { "failed" }.into(),
        host: c.host.clone(),
        repos: ok,
        skipped,
        why: (!why.is_empty()).then(|| why.join("；")),
    };
    Out { sync, changes }
}

/// 开始派发前、克隆完之后：在用服务端的凭据、这个仓库的远端是凭据的主机，就补配上（已经配好时不动）。
/// 返回配上的那一句（打印用）
pub fn ensure(repo: &Path, dir: &Path) -> Option<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rec = load(dir);
    let file = dir.join(FILE);
    if rec.host.is_empty() || !file.exists() {
        return None;
    }
    if https_host(&origin(repo)?).as_deref() != Some(rec.host.as_str()) {
        return None;
    }
    let changed = configure(repo, &rec.host, &file).ok()?;
    let s = shown(repo);
    if !rec.repos.contains(&s) {
        rec.repos.push(s.clone());
        save(dir, &rec);
    }
    changed.then(|| format!("Git 凭据：配进了 {s}（{}）", rec.host))
}

/// `git clone` 前面加的参数：在用服务端的凭据、地址是凭据的主机时，用 `-c` 带上同样的 helper（克隆出来之后再 [`ensure`]）
pub fn clone_args(url: &str, dir: &Path) -> Vec<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rec = load(dir);
    let file = dir.join(FILE);
    if rec.host.is_empty() || !file.exists() || https_host(url).as_deref() != Some(rec.host.as_str()) {
        return vec![];
    }
    let k = key(&rec.host);
    vec!["-c".into(), format!("{k}="), "-c".into(), format!("{k}={}", helper(&file))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn repo(root: &Path, name: &str, url: &str) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q"]);
        git(&d, &["remote", "add", "origin", url]);
        d
    }

    fn cred(host: &str, token: &str) -> GitCred {
        GitCred { host: host.into(), username: "octo cat".into(), token: Some(token.into()), ..Default::default() }
    }

    #[test]
    fn helper_quotes_windows_paths_and_spots_its_own_values() {
        assert_eq!(helper(Path::new(r"C:\Users\it's me\.board\host\board-git-credentials")), r"store --file 'C:/Users/it'\''s me/.board/host/board-git-credentials'");
        assert!(ours("") && ours("store --file '/home/x/.board/host/board-git-credentials'"));
        assert!(!ours("osxkeychain") && !ours("store --file /home/x/.git-credentials"));
        assert_eq!(enc("a b:c@d/é"), "a%20b%3Ac%40d%2F%C3%A9");
    }

    #[test]
    fn configures_matching_repos_reports_mismatch_and_cleans_up() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let gh = repo(t.path(), "gh", "https://github.com/me/board.git");
        let ssh = repo(t.path(), "ssh", "git@github.com:me/x.git");
        let lab = repo(t.path(), "lab", "https://gitlab.com/me/y.git");
        // 用户自己在仓库里写了一个同名的：留着，排在前面
        git(&gh, &["config", "--local", "--add", "credential.https://github.com.helper", "mine"]);
        let repos = vec![gh.clone(), ssh.clone(), lab.clone(), t.path().join("missing")];

        let out = sync(true, Some(&cred("github.com", "tok/en")), &repos, &host_dir);
        assert_eq!(out.sync.state, "failed", "gitlab 的仓库主机不匹配");
        assert!(out.sync.why.as_deref().unwrap().contains("gitlab.com"), "{:?}", out.sync.why);
        assert_eq!(out.sync.repos, vec![shown(&gh)]);
        assert_eq!(out.sync.skipped.len(), 1, "SSH 的不配");
        let k = "credential.https://github.com.helper";
        assert_eq!(values(&gh, k), vec!["mine".to_string(), String::new(), helper(&host_dir.join(FILE))]);
        assert!(values(&ssh, k).is_empty() && values(&lab, k).is_empty());
        let text = std::fs::read_to_string(host_dir.join(FILE)).unwrap();
        assert_eq!(text, "https://octo%20cat:tok%2Fen@github.com\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(host_dir.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // 输出里没有令牌
        assert!(!out.changes.join("\n").contains("tok"), "{:?}", out.changes);
        assert!(!serde_json::to_string(&out.sync).unwrap().contains("tok/en"));

        // git 真的用它：credential fill 拿到的是服务端的用户名和令牌
        let fill = Command::new("git")
            .arg("-C")
            .arg(&gh)
            .args(["credential", "fill"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(b"protocol=https\nhost=github.com\n\n")?;
                c.wait_with_output()
            })
            .unwrap();
        let got = String::from_utf8_lossy(&fill.stdout);
        assert!(got.contains("username=octo cat") && got.contains("password=tok/en"), "{got}");

        // 再同步一次：没有变化
        let again = sync(true, Some(&cred("github.com", "tok/en")), &repos[..1], &host_dir);
        assert_eq!(again.sync.state, "synced");
        assert!(again.changes.is_empty(), "{:?}", again.changes);

        // 之后克隆、新建的仓库：克隆参数带上 helper，ensure 补配
        assert_eq!(clone_args("https://github.com/me/new.git", &host_dir).len(), 4);
        assert!(clone_args("https://gitlab.com/me/new.git", &host_dir).is_empty());
        let later = repo(t.path(), "later", "https://github.com/me/later.git");
        assert!(ensure(&later, &host_dir).is_some());
        assert!(ensure(&later, &host_dir).is_none(), "已经配好时不动");
        assert!(ensure(&lab, &host_dir).is_none());

        // 关掉：去掉 Board 写的、用户自己的留着，令牌文件删掉，记录里后来补配的也去掉
        let off = sync(false, Some(&cred("github.com", "tok/en")), &repos[..1], &host_dir);
        assert_eq!(off.sync.state, "off");
        assert_eq!(values(&gh, k), vec!["mine".to_string()]);
        assert!(values(&later, k).is_empty());
        assert!(!host_dir.join(FILE).exists() && !host_dir.join(RECORD).exists());
        assert!(clone_args("https://github.com/me/new.git", &host_dir).is_empty());
        assert!(ensure(&later, &host_dir).is_none());
    }

    #[test]
    fn missing_token_or_credential_is_reported_not_silent() {
        let t = tempfile::tempdir().unwrap();
        let gh = repo(t.path(), "gh", "https://github.com/me/board.git");
        let none = sync(true, None, std::slice::from_ref(&gh), t.path());
        assert_eq!(none.sync.state, "none");
        let empty = sync(true, Some(&cred("github.com", " ")), std::slice::from_ref(&gh), t.path());
        assert_eq!((empty.sync.state.as_str(), empty.sync.why.as_deref()), ("failed", Some("服务端的 Git 凭据没有令牌")));
        // 主机换了：旧主机的那一项去掉，配上新的
        sync(true, Some(&cred("github.com", "a")), std::slice::from_ref(&gh), t.path());
        git(&gh, &["remote", "set-url", "origin", "https://git.example.com:8443/me/board.git"]);
        let moved = sync(true, Some(&cred("git.example.com:8443", "b")), std::slice::from_ref(&gh), t.path());
        assert_eq!(moved.sync.state, "synced", "{:?}", moved.sync.why);
        assert!(values(&gh, "credential.https://github.com.helper").is_empty());
        assert_eq!(values(&gh, "credential.https://git.example.com:8443.helper").len(), 2);
    }
}
