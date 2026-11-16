//! board host 这一侧的 Git 凭据同步（PRD §10.16「共用配置」，服务端见 board-core 的 `gitcred`）：
//! 服务端设了一份 https 远端的令牌、这台机器选了「用服务器的」，就把它写进这台机器的用户级 git 配置，
//! 之后新建、克隆的仓库只要远端是同一个 https 主机就自动生效，board host 备代码（`git fetch`、`git clone`）
//! 和 agent 推集成分支时直接用，不用每台机器手动配 credential helper。凭据里填了提交身份时，同一套同步
//! 还写 `user.name` / `user.email`，新建提交的作者、提交者就不再用系统账号和主机名兜底。
//!
//! - **令牌放哪**：`~/.board/host/board-git-credentials`，git 自带的 `credential-store` 的格式（一行
//!   `https://<用户名>:<令牌>@<主机>`），只有本人能读写（0600）。git 配置里只写 helper 的命令，不写令牌；
//!   board 的输出、日志、上报里也没有令牌，只有主机和用户名。
//! - **配进哪**：用户级配置（`git config --global`），只配凭据的那个主机：`credential.https://<主机>.helper`
//!   先写一个空值（清掉系统里别的 helper，免得 Git Credential Manager 弹窗、钥匙串里旧的令牌抢先），
//!   留用户自己为同一主机写的同名项，最后写 `store --file '<上面的文件>'`；其它主机、其它 helper 不碰。
//! - **还原**：选了「不改」、服务端删了凭据时，去掉 Board 写的这一项（用户自己写的留着）、删掉文件；
//!   旧版按仓库配过的项从 `~/.board/host/git-sync.json` 里的记录找回去，一并清掉。
//! - **之后才有的仓库**：全局配置按主机生效，新建、克隆的仓库不用再补配；克隆时仍用 `git -c` 带上同样的 helper，
//!   开始派发前（[`ensure`]）再确认一次全局项还在。
//! - 只管 https 远端：远端是 SSH（`git@…`）的仓库不用这一项，照旧用本机的 key；别的 https 主机不受影响。

use board_core::gitcred::{https_host, GitCred};
use board_core::{GitDetail, GitSync};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// 令牌文件名（helper 的命令里带着它，据此认出哪些配置项是 Board 写的）
const FILE: &str = "board-git-credentials";
/// 记录：用的是哪个主机、旧版按仓库配过哪些仓库（只为了升级后清掉），以及 Board 写过的提交身份和原来的值
const RECORD: &str = "git-sync.json";
const QUICK: Duration = Duration::from_secs(10);

/// 同步（后台线程）和开始派发前补配（主线程）都会改记录，一次只让一个做
static LOCK: Mutex<()> = Mutex::new(());

/// 配的是哪个主机、旧版按仓库配过哪些仓库、Board 写过的提交身份；主机为空表示凭据现在没在用
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    host: String,
    #[serde(default)]
    repos: Vec<String>,
    /// Board 最后写进用户级 git 配置的 `user.name`；空表示 Board 没写过
    #[serde(default)]
    commit_name: String,
    /// Board 最后写进用户级 git 配置的 `user.email`；空表示 Board 没写过
    #[serde(default)]
    commit_email: String,
    /// 第一次由 Board 覆盖前，用户级配置里的 `user.name`（可能有多条，也可能没有）
    #[serde(default)]
    name_before: Vec<String>,
    /// 第一次由 Board 覆盖前，用户级配置里的 `user.email`（可能有多条，也可能没有）
    #[serde(default)]
    email_before: Vec<String>,
}

fn load(dir: &Path) -> Record {
    std::fs::read_to_string(dir.join(RECORD)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(dir: &Path, r: &Record) {
    if r.host.is_empty() && r.repos.is_empty() && r.commit_name.is_empty() && r.commit_email.is_empty() {
        let _ = std::fs::remove_file(dir.join(RECORD));
    } else {
        let _ = std::fs::write(dir.join(RECORD), serde_json::to_string_pretty(r).unwrap_or_default());
    }
}

fn key(host: &str) -> String {
    format!("credential.https://{host}.helper")
}

/// 配置写在哪：生产用用户级（`--global`）；测试用单独的文件或仓库本地配置，不碰跑测试的人自己的 `~/.gitconfig`
#[derive(Clone, Copy)]
#[cfg_attr(not(test), allow(dead_code))]
enum Scope<'a> {
    Global,
    File(&'a Path),
    Local,
}

/// 读、写这项配置：`base` 是跑 git 用的目录（用户级用 `~/.board/host`，本地配置用仓库）
fn with_config(base: &Path, scope: Scope<'_>, args: &[&str]) -> Result<String, String> {
    let mut all: Vec<String> = vec!["config".into()];
    match scope {
        Scope::Global => all.push("--global".into()),
        Scope::File(p) => {
            all.push("--file".into());
            all.push(p.to_string_lossy().into_owned());
        }
        Scope::Local => all.push("--local".into()),
    }
    all.extend(args.iter().map(|a| (*a).to_string()));
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    crate::code::git(base, &refs, QUICK)
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

/// 这项配置的所有值（按顺序；空值也算一个）
fn values(base: &Path, scope: Scope<'_>, k: &str) -> Vec<String> {
    match with_config(base, scope, &["--null", "--get-all", k]) {
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
fn set_values(base: &Path, scope: Scope<'_>, k: &str, now: &[String], want: &[String]) -> Result<bool, String> {
    if now == want {
        return Ok(false);
    }
    if !now.is_empty() {
        with_config(base, scope, &["--unset-all", k])?;
    }
    for v in want {
        with_config(base, scope, &["--add", k, v])?;
    }
    Ok(true)
}

/// 配进用户级（或测试用的文件）：先写空值清掉系统里别的 helper，再留用户自己写的同名项，最后是 Board 的 helper
fn configure(base: &Path, scope: Scope<'_>, host: &str, file: &Path) -> Result<bool, String> {
    let k = key(host);
    let now = values(base, scope, &k);
    let mut want: Vec<String> = vec![String::new()];
    want.extend(now.iter().filter(|v| !ours(v)).cloned());
    want.push(helper(file));
    set_values(base, scope, &k, &now, &want)
}

fn same_value(now: &[String], want: &str) -> bool {
    now.len() == 1 && now[0] == want
}

/// 写一项提交身份。已经是要的值就不动；用户自己改过时，把用户改的值记成新的原值，之后关掉同步也不会覆盖它。
fn apply_identity(base: &Path, scope: Scope<'_>, key: &str, want: &str, written: &mut String, before: &mut Vec<String>) -> Result<bool, String> {
    let now = values(base, scope, key);
    if same_value(&now, want) {
        return Ok(false);
    }
    let backup = if !written.is_empty() && same_value(&now, written) {
        before.clone()
    } else {
        now.clone()
    };
    set_values(base, scope, key, &now, &[want.to_string()])?;
    *written = want.to_string();
    *before = backup;
    Ok(true)
}

/// 只在配置里还是 Board 写的值时恢复原值；用户后来自己改过就保留用户的。
fn restore_identity_value(base: &Path, scope: Scope<'_>, key: &str, written: &mut String, before: &mut Vec<String>) -> Result<bool, String> {
    if written.is_empty() {
        return Ok(false);
    }
    let now = values(base, scope, key);
    let changed = if same_value(&now, written) { set_values(base, scope, key, &now, before)? } else { false };
    written.clear();
    before.clear();
    Ok(changed)
}

/// 同步（或恢复）用户级 git 配置里的提交身份
fn configure_identity(base: &Path, scope: Scope<'_>, rec: &mut Record, name: &str, email: &str) -> Result<bool, String> {
    if name.is_empty() || email.is_empty() {
        let name_changed = restore_identity_value(base, scope, "user.name", &mut rec.commit_name, &mut rec.name_before)?;
        let email_changed = restore_identity_value(base, scope, "user.email", &mut rec.commit_email, &mut rec.email_before)?;
        return Ok(name_changed || email_changed);
    }
    let name_changed = apply_identity(base, scope, "user.name", name, &mut rec.commit_name, &mut rec.name_before)?;
    let email_changed = apply_identity(base, scope, "user.email", email, &mut rec.commit_email, &mut rec.email_before)?;
    Ok(name_changed || email_changed)
}

/// 从用户级（或测试用的本地配置、文件）里去掉 Board 写的（用户自己写的留着）；仓库不在了就算了
fn unconfigure(base: &Path, scope: Scope<'_>, host: &str) -> Result<bool, String> {
    if host.is_empty() || matches!(scope, Scope::Local) && !base.is_dir() {
        return Ok(false);
    }
    let k = key(host);
    let now = values(base, scope, &k);
    let keep: Vec<String> = now.iter().filter(|v| !ours(v)).cloned().collect();
    set_values(base, scope, &k, &now, &keep)
}

fn shown(p: &Path) -> String {
    p.display().to_string()
}

/// 一次同步的结果和要打印的变化（不含令牌）
pub struct Out {
    pub sync: GitSync,
    pub changes: Vec<String>,
}

/// 去掉 Board 写在用户级配置里的那一项、旧版按仓库配过的项（记录里和这一次的仓库），恢复 Board 写过的提交身份，再删掉令牌文件
fn clean(dir: &Path, scope: Scope<'_>, rec: &Record, repos: &[PathBuf], changes: &mut Vec<String>) {
    if let Ok(true) = unconfigure(dir, scope, &rec.host) {
        changes.push(format!("Git 凭据：从用户级 git 配置里去掉了（{}）", rec.host));
    }
    let mut all: Vec<PathBuf> = rec.repos.iter().map(PathBuf::from).collect();
    all.extend(repos.iter().cloned());
    all.sort();
    all.dedup();
    for r in &all {
        if let Ok(true) = unconfigure(r, Scope::Local, &rec.host) {
            changes.push(format!("Git 凭据：从 {} 里去掉了旧版按仓库配的项（{}）", shown(r), rec.host));
        }
    }
    if dir.join(FILE).exists() {
        let _ = std::fs::remove_file(dir.join(FILE));
        changes.push("Git 凭据：删掉了本机的令牌文件".into());
    }
    let mut left = rec.clone();
    left.host.clear();
    left.repos.clear();
    match configure_identity(dir, scope, &mut left, "", "") {
        Ok(true) => changes.push("Git 提交身份：恢复了用户级 git 配置里原来的 user.name / user.email".into()),
        Ok(false) => {}
        Err(e) => {
            changes.push(format!("Git 提交身份：没恢复原来的 user.name / user.email：{e}"));
        }
    }
    save(dir, &left);
}

/// 同步：`on` 是机器上选了「用服务器的」，`cred` 是服务端的（没设时 None），`repos` 是 Board 管的仓库（只用来清旧版按仓库配的项），
/// `dir` 是 `~/.board/host`
pub fn sync(on: bool, cred: Option<&GitCred>, repos: &[PathBuf], dir: &Path) -> Out {
    sync_at(on, cred, repos, dir, Scope::Global)
}

/// 同 [`sync`]，配置写在哪由 `scope` 决定（测试用单独的文件）
fn sync_at(on: bool, cred: Option<&GitCred>, repos: &[PathBuf], dir: &Path, scope: Scope<'_>) -> Out {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rec = load(dir);
    let mut changes = vec![];
    let token = cred.and_then(|c| c.token.as_deref()).map(str::trim).filter(|t| !t.is_empty());
    let (Some(c), Some(token), true) = (cred, token, on) else {
        clean(dir, scope, &rec, repos, &mut changes);
        let sync = match (on, cred) {
            (false, _) => GitSync { state: "off".into(), ..Default::default() },
            (true, None) => GitSync { state: "none".into(), ..Default::default() },
            (true, Some(c)) => GitSync { state: "failed".into(), why: Some("服务端的 Git 凭据没有令牌".into()), detail: GitDetail { host: c.host.clone(), ..Default::default() }, ..Default::default() },
        };
        return Out { sync, changes };
    };
    let file = dir.join(FILE);
    if let Err(e) = write_file(&file, c, token) {
        clean(dir, scope, &rec, repos, &mut changes);
        return Out { sync: GitSync { state: "failed".into(), why: Some(format!("写不了 {}：{e}", shown(&file))), detail: GitDetail { host: c.host.clone(), ..Default::default() }, ..Default::default() }, changes };
    }
    // 主机换了：先把旧主机的那一项去掉
    if !rec.host.is_empty() && rec.host != c.host {
        if let Ok(true) = unconfigure(dir, scope, &rec.host) {
            changes.push(format!("Git 凭据：从用户级 git 配置里去掉了（{}）", rec.host));
        }
    }
    // 旧版按仓库配过的项（记录里的，加上这次看到的仓库）：去掉，用户自己的留着
    let mut old: Vec<PathBuf> = rec.repos.iter().map(PathBuf::from).collect();
    old.extend(repos.iter().cloned());
    old.sort();
    old.dedup();
    for r in &old {
        if let Ok(true) = unconfigure(r, Scope::Local, &rec.host) {
            changes.push(format!("Git 凭据：从 {} 里去掉了旧版按仓库配的项", shown(r)));
        }
    }
    let mut errors = vec![];
    match configure(dir, scope, &c.host, &file) {
        Ok(true) => {
            changes.push(format!("Git 凭据：写进了用户级 git 配置（{}）", c.host));
        }
        Ok(false) => {}
        Err(e) => errors.push(format!("git 配置没写进去：{e}")),
    }
    let mut configured = false;
    match configure_identity(dir, scope, &mut rec, &c.commit_name, &c.commit_email) {
        Ok(changed) => configured |= changed,
        Err(e) => errors.push(format!("Git 提交身份没写进去：{e}")),
    }
    if configured {
        if c.commit_name.is_empty() {
            changes.push("Git 提交身份：恢复了用户级 git 配置里原来的 user.name / user.email".into());
        } else {
            changes.push(format!("Git 提交身份：写进了用户级 git 配置（{} <{}>）", c.commit_name, c.commit_email));
        }
    }
    rec.host = c.host.clone();
    rec.repos.clear();
    save(dir, &rec);
    let (state, why) = if errors.is_empty() {
        ("synced".to_string(), None)
    } else {
        ("failed".to_string(), Some(errors.join("；")))
    };
    let sync = GitSync {
        state,
        count: 1,
        why,
        detail: GitDetail { host: c.host.clone(), ..Default::default() },
    };
    Out { sync, changes }
}

/// 开始派发前、克隆完之后：在用服务端的凭据，就确认用户级配置里那一项还在（已经配好时不动）。
/// 全局配置按主机生效，不再按仓库补配；只补凭据 helper，不碰提交身份（用户可能自己改过）。
/// 返回写进去的那一句（打印用）
pub fn ensure(_repo: &Path, dir: &Path) -> Option<String> {
    ensure_at(dir, Scope::Global)
}

fn ensure_at(dir: &Path, scope: Scope<'_>) -> Option<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rec = load(dir);
    let file = dir.join(FILE);
    if rec.host.is_empty() || !file.exists() {
        return None;
    }
    let changed = configure(dir, scope, &rec.host, &file).ok()?;
    changed.then(|| format!("Git 凭据：写进了用户级 git 配置（{}）", rec.host))
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

    fn ident(host: &str, token: &str, name: &str, email: &str) -> GitCred {
        GitCred { commit_name: name.into(), commit_email: email.into(), ..cred(host, token) }
    }

    /// 测试用的用户级配置文件（不碰跑测试的人自己的 `~/.gitconfig`）
    fn cfg_git(cfg: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("config").arg("--file").arg(cfg).args(args).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
        assert!(out.status.success(), "git config --file {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn cfg_values(cfg: &Path, k: &str) -> Vec<String> {
        let out = Command::new("git").arg("config").arg("--file").arg(cfg).args(["--null", "--get-all", k]).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
        if !out.status.success() {
            return vec![];
        }
        let mut v: Vec<String> = String::from_utf8_lossy(&out.stdout).split('\0').map(str::to_string).collect();
        if v.last().is_some_and(String::is_empty) {
            v.pop();
        }
        v
    }

    /// 在这个仓库里 `credential fill`：用测试的用户级配置文件，拿到凭据时返回输出的全文
    fn fill(repo: &Path, cfg: &Path, host: &str) -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["credential", "fill"])
            .env("GIT_CONFIG_GLOBAL", cfg)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(format!("protocol=https\nhost={host}\n\n").as_bytes())?;
                c.wait_with_output()
            })
            .unwrap();
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    #[test]
    fn helper_quotes_windows_paths_and_spots_its_own_values() {
        assert_eq!(helper(Path::new(r"C:\Users\it's me\.board\host\board-git-credentials")), r"store --file 'C:/Users/it'\''s me/.board/host/board-git-credentials'");
        assert!(ours("") && ours("store --file '/home/x/.board/host/board-git-credentials'"));
        assert!(!ours("osxkeychain") && !ours("store --file /home/x/.git-credentials"));
        assert_eq!(enc("a b:c@d/é"), "a%20b%3Ac%40d%2F%C3%A9");
    }

    #[test]
    fn writes_user_global_config_for_new_repos_and_cleans_old_repo_entries() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        let gh = repo(t.path(), "gh", "https://github.com/me/board.git");
        let ssh = repo(t.path(), "ssh", "git@github.com:me/x.git");
        let lab = repo(t.path(), "lab", "https://gitlab.com/me/y.git");
        let repos = vec![gh.clone(), ssh.clone(), lab.clone(), t.path().join("missing")];
        let k = "credential.https://github.com.helper";
        // 用户自己为同一主机写的 helper 留着，排在前面
        let mine = "store --file '/tmp/board-test-user-credentials'";
        cfg_git(&cfg, &["--add", k, mine]);
        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        cfg_git(&cfg, &["--add", "user.email", "alice@example.com"]);
        // 旧版按仓库配过的：记录里有，这次同步要去掉
        let old = helper(&host_dir.join(FILE));
        for v in ["", old.as_str()] {
            git(&gh, &["config", "--local", "--add", k, v]);
        }
        std::fs::write(host_dir.join(RECORD), serde_json::to_string(&Record { host: "github.com".into(), repos: vec![shown(&gh)], ..Default::default() }).unwrap()).unwrap();

        let c = ident("github.com", "tok/en", "BUGLAN", "1831353087@qq.com");
        let out = sync_at(true, Some(&c), &repos, &host_dir, Scope::File(&cfg));
        assert_eq!(out.sync.state, "synced", "{:?}", out.sync.why);
        assert_eq!(out.sync.detail.host, "github.com");
        assert_eq!(out.sync.detail.repos, Vec::<String>::new(), "不再按仓库报数量");
        assert_eq!(out.sync.count, 1);
        assert_eq!(cfg_values(&cfg, k), vec![String::new(), mine.to_string(), helper(&host_dir.join(FILE))]);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["1831353087@qq.com"]);
        assert!(values(&gh, Scope::Local, k).is_empty(), "旧版按仓库配的已经去掉");
        assert!(values(&ssh, Scope::Local, k).is_empty() && values(&lab, Scope::Local, k).is_empty());
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

        // 同步之后才建的仓库也自动用它；别的 https 主机不受影响
        let later = repo(t.path(), "later", "https://github.com/me/later.git");
        let got = fill(&later, &cfg, "github.com").expect("新仓库直接用全局凭据");
        assert!(got.contains("username=octo cat") && got.contains("password=tok/en"), "{got}");
        let lab2 = repo(t.path(), "lab2", "https://gitlab.com/me/z.git");
        assert!(fill(&lab2, &cfg, "gitlab.com").is_none(), "别的 https 主机不配");

        // 再同步一次：没有变化
        let again = sync_at(true, Some(&c), &repos[..1], &host_dir, Scope::File(&cfg));
        assert_eq!(again.sync.state, "synced");
        assert!(again.changes.is_empty(), "{:?}", again.changes);

        // 克隆参数照样带上 helper（全局配置还没写时也能用）
        assert_eq!(clone_args("https://github.com/me/new.git", &host_dir).len(), 4);
        assert!(clone_args("https://gitlab.com/me/new.git", &host_dir).is_empty());

        // 用户后来自己改了用户名：关掉时只恢复邮箱，用户名保留用户改的
        cfg_git(&cfg, &["user.name", "alice-local"]);
        // 关掉：去掉 Board 写的、用户自己的留着，提交身份只恢复还是 Board 写的项，令牌文件删掉，旧记录也清掉
        let off = sync_at(false, Some(&c), &repos[..1], &host_dir, Scope::File(&cfg));
        assert_eq!(off.sync.state, "off");
        assert_eq!(cfg_values(&cfg, k), vec![mine.to_string()]);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice-local"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
        assert!(!host_dir.join(FILE).exists() && !host_dir.join(RECORD).exists());
        assert!(clone_args("https://github.com/me/new.git", &host_dir).is_empty());
        assert!(fill(&later, &cfg, "github.com").is_none(), "关掉后新仓库也用不到了");
    }

    #[test]
    fn empty_commit_identity_keeps_existing_global_identity() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        cfg_git(&cfg, &["--add", "user.email", "alice@example.com"]);

        let on = sync_at(true, Some(&cred("github.com", "tok")), &[], &host_dir, Scope::File(&cfg));
        assert_eq!(on.sync.state, "synced");
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
        sync_at(false, Some(&cred("github.com", "tok")), &[], &host_dir, Scope::File(&cfg));
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
    }

    #[test]
    fn commit_identity_is_cleared_or_restored_when_sync_turns_off() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        let c = ident("github.com", "tok", "BUGLAN", "1831353087@qq.com");

        sync_at(true, Some(&c), &[], &host_dir, Scope::File(&cfg));
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["1831353087@qq.com"]);
        sync_at(false, Some(&c), &[], &host_dir, Scope::File(&cfg));
        assert!(cfg_values(&cfg, "user.name").is_empty() && cfg_values(&cfg, "user.email").is_empty(), "原来没有就清掉");

        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        sync_at(true, Some(&c), &[], &host_dir, Scope::File(&cfg));
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        sync_at(false, Some(&c), &[], &host_dir, Scope::File(&cfg));
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"], "用户原来的名字恢复");
        assert!(cfg_values(&cfg, "user.email").is_empty());
    }

    #[test]
    fn ensure_only_restores_helper_and_keeps_user_changed_identity() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        let k = "credential.https://github.com.helper";
        let c = ident("github.com", "tok", "BUGLAN", "1831353087@qq.com");
        sync_at(true, Some(&c), &[], &host_dir, Scope::File(&cfg));

        cfg_git(&cfg, &["--unset-all", k]);
        cfg_git(&cfg, &["user.name", "alice"]);
        cfg_git(&cfg, &["user.email", "alice@example.com"]);
        assert_eq!(ensure_at(&host_dir, Scope::File(&cfg)).as_deref(), Some("Git 凭据：写进了用户级 git 配置（github.com）"));
        assert_eq!(cfg_values(&cfg, k), vec![String::new(), helper(&host_dir.join(FILE))]);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"], "派发前不覆盖用户改过的提交身份");
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
        assert!(ensure_at(&host_dir, Scope::File(&cfg)).is_none(), "配好之后不再重复写");
    }

    #[test]
    fn missing_token_or_credential_is_reported_not_silent() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        let none = sync_at(true, None, &[], &host_dir, Scope::File(&cfg));
        assert_eq!(none.sync.state, "none");
        let empty = sync_at(true, Some(&cred("github.com", " ")), &[], &host_dir, Scope::File(&cfg));
        assert_eq!((empty.sync.state.as_str(), empty.sync.why.as_deref()), ("failed", Some("服务端的 Git 凭据没有令牌")));
        // 主机换了：旧主机的那一项去掉，配上新的
        sync_at(true, Some(&cred("github.com", "a")), &[], &host_dir, Scope::File(&cfg));
        let moved = sync_at(true, Some(&cred("git.example.com:8443", "b")), &[], &host_dir, Scope::File(&cfg));
        assert_eq!(moved.sync.state, "synced", "{:?}", moved.sync.why);
        assert!(cfg_values(&cfg, "credential.https://github.com.helper").is_empty());
        assert_eq!(cfg_values(&cfg, "credential.https://git.example.com:8443.helper").len(), 2);
    }
}
