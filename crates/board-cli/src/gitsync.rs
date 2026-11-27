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
//! - **失败不丢恢复信息**（卡片 #427）：git 配置只通过 [`ConfigBackend`] 读写，读分清「没有这一项」和「读失败」，
//!   读失败就不写；改一整项、中途失败时写回原值。记录先写后改（write-ahead）：改配置之前先把「要写什么、原来是什么」
//!   原子地存进记录，改不成再退回去；关掉时逐项恢复，恢复成一项清一项，失败的留着下次再试，都报 `failed`。

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

/// 读记录。没有记录文件是空记录；有但读不了、解析不了时报错——不能当成空记录，Board 写过什么、原来是什么全靠它
fn load(dir: &Path) -> Result<Record, String> {
    let path = dir.join(RECORD);
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("记录 {} 坏了（{e}），Board 改过的 git 配置靠它恢复，先修好或删掉它", shown(&path))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Record::default()),
        Err(e) => Err(format!("读不了记录 {}：{e}", shown(&path))),
    }
}

/// 存记录：先写临时文件、`sync_all`，再改名换上（中途崩溃也不会留下半截的记录）；记录空了就删掉文件
fn save(dir: &Path, r: &Record) -> Result<(), String> {
    use std::io::Write;
    let path = dir.join(RECORD);
    let fail = |e: std::io::Error| format!("记录 {} 没存上：{e}", shown(&path));
    if r.host.is_empty() && r.repos.is_empty() && r.commit_name.is_empty() && r.commit_email.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(fail(e)),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(dir).map_err(fail)?;
    let tmp = dir.join(format!("{RECORD}.tmp"));
    let out = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(serde_json::to_string_pretty(r).unwrap_or_default().as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &path)
    })();
    if let Err(e) = out {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(e));
    }
    Ok(())
}

fn key(host: &str) -> String {
    format!("credential.https://{host}.helper")
}

/* ------------------------------ git 配置 ------------------------------ */

/// git 配置的读写。同步逻辑只通过它碰配置；生产用 git 命令（[`GitConfig`]），测试换成能注入失败的实现
trait ConfigBackend {
    /// 这一项的所有值（按顺序，空值也算一个）。没有这一项时 `Ok(None)`；读失败（超时、配置文件坏了、git 不在）时 `Err`，
    /// 不能当成没有——不然会在原来的值后面再追加一份、或把原值记成空的
    fn read(&self, key: &str) -> Result<Option<Vec<String>>, String>;
    /// 把这一项整个换成 `values`（空的就去掉这一项）。中途失败时尽量写回原来的值，再返回错误
    fn replace(&self, key: &str, values: &[String]) -> Result<(), String>;
}

/// 配置写在哪：生产用用户级（`--global`），旧版按仓库配的在仓库本地；测试用单独的文件，不碰跑测试的人自己的 `~/.gitconfig`
#[derive(Clone)]
enum Scope {
    Global,
    #[cfg_attr(not(test), allow(dead_code))]
    File(PathBuf),
    Local,
}

/// 用 `git config` 读写：`base` 是跑 git 用的目录（用户级用 `~/.board/host`，本地配置用仓库）
struct GitConfig {
    base: PathBuf,
    scope: Scope,
}

impl GitConfig {
    fn global(dir: &Path) -> GitConfig {
        GitConfig { base: dir.to_path_buf(), scope: Scope::Global }
    }

    fn local(repo: &Path) -> GitConfig {
        GitConfig { base: repo.to_path_buf(), scope: Scope::Local }
    }

    #[cfg(test)]
    fn file(dir: &Path, file: &Path) -> GitConfig {
        GitConfig { base: dir.to_path_buf(), scope: Scope::File(file.to_path_buf()) }
    }

    fn run(&self, args: &[&str]) -> Result<std::process::Output, String> {
        let mut all: Vec<String> = vec!["config".into()];
        match &self.scope {
            Scope::Global => all.push("--global".into()),
            Scope::File(p) => {
                all.push("--file".into());
                all.push(p.to_string_lossy().into_owned());
            }
            Scope::Local => all.push("--local".into()),
        }
        all.extend(args.iter().map(|a| (*a).to_string()));
        let refs: Vec<&str> = all.iter().map(String::as_str).collect();
        crate::code::git_output(&self.base, &refs, QUICK)
    }

    fn set(&self, args: &[&str]) -> Result<(), String> {
        let out = self.run(args)?;
        if out.status.success() {
            Ok(())
        } else {
            Err(crate::code::git_error(&out))
        }
    }

    /// 写成 `values`：一条用 `--replace-all` 一次换掉（git 自己原子地改文件），多条再逐个 `--add`；空的就 `--unset-all`
    fn write(&self, k: &str, values: &[String]) -> Result<(), String> {
        let Some((first, rest)) = values.split_first() else {
            let out = self.run(&["--unset-all", k])?;
            // 退出码 5：本来就没有这一项
            return if out.status.success() || out.status.code() == Some(5) { Ok(()) } else { Err(crate::code::git_error(&out)) };
        };
        self.set(&["--replace-all", k, first])?;
        for v in rest {
            self.set(&["--add", k, v])?;
        }
        Ok(())
    }
}

impl ConfigBackend for GitConfig {
    fn read(&self, k: &str) -> Result<Option<Vec<String>>, String> {
        let out = self.run(&["--null", "--get-all", k])?;
        if out.status.success() {
            let mut v: Vec<String> = String::from_utf8_lossy(&out.stdout).split('\0').map(str::to_string).collect();
            if v.last().is_some_and(String::is_empty) {
                v.pop();
            }
            return Ok(Some(v));
        }
        // 退出码 1、没有报错：这一项不存在。别的（配置文件坏了是 128 之类）都算读失败
        if out.status.code() == Some(1) && String::from_utf8_lossy(&out.stderr).trim().is_empty() {
            return Ok(None);
        }
        Err(crate::code::git_error(&out))
    }

    fn replace(&self, k: &str, values: &[String]) -> Result<(), String> {
        let before = self.read(k)?.unwrap_or_default();
        replace_or_restore(&before, values, |v| self.write(k, v))
    }
}

/// 把 `before` 换成 `want`；`write` 中途失败时用它把 `before` 写回去，再返回原来的错误
fn replace_or_restore(before: &[String], want: &[String], write: impl Fn(&[String]) -> Result<(), String>) -> Result<(), String> {
    if before == want {
        return Ok(());
    }
    if let Err(e) = write(want) {
        return Err(match write(before) {
            Ok(()) => e,
            Err(back) => format!("{e}；写回原来的值也失败：{back}"),
        });
    }
    Ok(())
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

/// 配进用户级（或测试用的文件）：先写空值清掉系统里别的 helper，再留用户自己写的同名项，最后是 Board 的 helper
fn configure(cfg: &dyn ConfigBackend, host: &str, file: &Path) -> Result<bool, String> {
    let k = key(host);
    let now = cfg.read(&k)?.unwrap_or_default();
    let mut want: Vec<String> = vec![String::new()];
    want.extend(now.iter().filter(|v| !ours(v)).cloned());
    want.push(helper(file));
    if now == want {
        return Ok(false);
    }
    cfg.replace(&k, &want)?;
    Ok(true)
}

/// 去掉 Board 写的那一项（用户自己写的留着）
fn unconfigure(cfg: &dyn ConfigBackend, host: &str) -> Result<bool, String> {
    if host.is_empty() {
        return Ok(false);
    }
    let k = key(host);
    let Some(now) = cfg.read(&k)? else { return Ok(false) };
    let keep: Vec<String> = now.iter().filter(|v| !ours(v)).cloned().collect();
    if keep == now {
        return Ok(false);
    }
    cfg.replace(&k, &keep)?;
    Ok(true)
}

fn same_value(now: &[String], want: &str) -> bool {
    now.len() == 1 && now[0] == want
}

/// 提交身份的一项：配置里的键，记录里 Board 写的值、原来的值
#[derive(Clone, Copy)]
enum Ident {
    Name,
    Email,
}

impl Ident {
    fn key(self) -> &'static str {
        match self {
            Ident::Name => "user.name",
            Ident::Email => "user.email",
        }
    }

    fn slot(self, rec: &mut Record) -> (&mut String, &mut Vec<String>) {
        match self {
            Ident::Name => (&mut rec.commit_name, &mut rec.name_before),
            Ident::Email => (&mut rec.commit_email, &mut rec.email_before),
        }
    }
}

/// 写一项提交身份。已经是要的值就不动；用户自己改过时，把用户改的值记成新的原值，之后关掉同步也不会覆盖它。
/// 先把「要写什么、原来是什么」存进记录再改配置：记录存不上就不改；配置改不成就把记录退回去
fn apply_identity(cfg: &dyn ConfigBackend, dir: &Path, rec: &mut Record, which: Ident, want: &str) -> Result<bool, String> {
    let now = cfg.read(which.key())?.unwrap_or_default();
    if same_value(&now, want) {
        return Ok(false);
    }
    let prev = rec.clone();
    {
        let (written, before) = which.slot(rec);
        // 配置里还是 Board 上次写的值：原值不变；否则（第一次写，或用户改过）现在的值就是原值
        if written.is_empty() || !same_value(&now, written) {
            *before = now;
        }
        *written = want.to_string();
    }
    if let Err(e) = save(dir, rec) {
        *rec = prev;
        return Err(e);
    }
    if let Err(e) = cfg.replace(which.key(), &[want.to_string()]) {
        // 配置没改成（一条值是 git 一次原子地改）：记录退回去。退不回也不要紧，配置里不是 Board 写的值，恢复时当成用户的
        *rec = prev;
        let _ = save(dir, rec);
        return Err(e);
    }
    Ok(true)
}

/// 只在配置里还是 Board 写的值时恢复原值；用户后来自己改过就保留用户的。读失败、恢复失败时记录不动，下次再试
fn restore_identity(cfg: &dyn ConfigBackend, dir: &Path, rec: &mut Record, which: Ident) -> Result<bool, String> {
    let (written, before) = which.slot(rec);
    if written.is_empty() {
        return Ok(false);
    }
    let (written, before) = (written.clone(), before.clone());
    let now = cfg.read(which.key())?.unwrap_or_default();
    let changed = same_value(&now, &written) && now != before;
    if changed {
        cfg.replace(which.key(), &before)?;
    }
    let (w, b) = which.slot(rec);
    w.clear();
    b.clear();
    save(dir, rec)?;
    Ok(changed)
}

/// 同步（或恢复）提交身份：`name`、`email` 都有就写，否则恢复 Board 写过的。两项各做各的，一项失败不影响另一项。
/// 返回有没有改过配置，和失败的原因
fn configure_identity(cfg: &dyn ConfigBackend, dir: &Path, rec: &mut Record, name: &str, email: &str) -> (bool, Vec<String>) {
    let mut changed = false;
    let mut errors = vec![];
    for (which, want) in [(Ident::Name, name), (Ident::Email, email)] {
        let out = if name.is_empty() || email.is_empty() {
            restore_identity(cfg, dir, rec, which)
        } else {
            apply_identity(cfg, dir, rec, which, want)
        };
        match out {
            Ok(c) => changed |= c,
            Err(e) => errors.push(format!("{}：{e}", which.key())),
        }
    }
    (changed, errors)
}

fn shown(p: &Path) -> String {
    p.display().to_string()
}

/// 一次同步的结果和要打印的变化（不含令牌）
pub struct Out {
    pub sync: GitSync,
    pub changes: Vec<String>,
}

fn failed(why: String, host: &str) -> GitSync {
    GitSync { state: "failed".into(), why: Some(why), detail: GitDetail { host: host.to_string(), ..Default::default() }, ..Default::default() }
}

/// 目录是不是 git 仓库的工作区：不是（空目录、普通目录）返回 false；git 跑不起来、超时返回错误，
/// 不能当成不是——记录要留着下次再清
fn is_work_tree(dir: &Path) -> Result<bool, String> {
    let out = crate::code::git_output(dir, &["rev-parse", "--is-inside-work-tree"], QUICK)?;
    Ok(out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true")
}

/// 去掉旧版按仓库配过的项（记录里的和这一次的仓库），返回没去掉的（留在记录里下次再试）；
/// 仓库不在了、还不是 git 仓库（项目没设仓库地址时 board host 建的空目录）就算了——没有本地配置可清
fn clean_repos(recorded: &[String], repos: &[PathBuf], host: &str, changes: &mut Vec<String>, errors: &mut Vec<String>) -> Vec<String> {
    let mut all: Vec<PathBuf> = recorded.iter().map(PathBuf::from).collect();
    all.extend(repos.iter().cloned());
    all.sort();
    all.dedup();
    let mut left = vec![];
    for r in &all {
        if host.is_empty() || !r.is_dir() {
            continue;
        }
        match is_work_tree(r) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                errors.push(format!("{} 里旧版按仓库配的项：{e}", shown(r)));
                left.push(shown(r));
                continue;
            }
        }
        match unconfigure(&GitConfig::local(r), host) {
            Ok(true) => changes.push(format!("Git 凭据：从 {} 里去掉了旧版按仓库配的项（{host}）", shown(r))),
            Ok(false) => {}
            Err(e) => {
                errors.push(format!("{} 里旧版按仓库配的项：{e}", shown(r)));
                left.push(shown(r));
            }
        }
    }
    left
}

/// 去掉 Board 写在用户级配置里的那一项、旧版按仓库配过的项，恢复 Board 写过的提交身份，再删掉令牌文件。
/// 一步一步来，哪一步没做成就把它要的记录留着（主机、仓库、提交身份），下次同步再试；最后存记录
fn clean(dir: &Path, cfg: &dyn ConfigBackend, rec: &mut Record, repos: &[PathBuf], changes: &mut Vec<String>) -> Result<(), String> {
    let mut errors = vec![];
    let host = rec.host.clone();
    let global = match unconfigure(cfg, &host) {
        Ok(changed) => {
            if changed {
                changes.push(format!("Git 凭据：从用户级 git 配置里去掉了（{host}）"));
            }
            true
        }
        Err(e) => {
            errors.push(format!("用户级 git 配置里的 {host}：{e}"));
            false
        }
    };
    let left = clean_repos(&rec.repos, repos, &host, changes, &mut errors);
    // 令牌文件：helper 去掉了才删。反过来 helper 还在、文件没了，git 会一直找不到凭据
    let file = dir.join(FILE);
    let mut file_gone = !file.exists();
    if global && !file_gone {
        match std::fs::remove_file(&file) {
            Ok(()) => {
                file_gone = true;
                changes.push("Git 凭据：删掉了本机的令牌文件".into());
            }
            Err(e) => errors.push(format!("删不掉 {}：{e}", shown(&file))),
        }
    }
    // 都清干净了才忘掉主机：没清完的下次还要按它找
    if global && file_gone && left.is_empty() {
        rec.host.clear();
    }
    rec.repos = left;
    let (restored, errs) = configure_identity(cfg, dir, rec, "", "");
    if restored {
        changes.push("Git 提交身份：恢复了用户级 git 配置里原来的 user.name / user.email".into());
    }
    errors.extend(errs.into_iter().map(|e| format!("提交身份没恢复（{e}）")));
    if let Err(e) = save(dir, rec) {
        errors.push(e);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

/// 同步：`on` 是机器上选了「用服务器的」，`cred` 是服务端的（没设时 None），`repos` 是 Board 管的仓库（只用来清旧版按仓库配的项），
/// `dir` 是 `~/.board/host`
pub fn sync(on: bool, cred: Option<&GitCred>, repos: &[PathBuf], dir: &Path) -> Out {
    sync_with(on, cred, repos, dir, &GitConfig::global(dir))
}

/// 同 [`sync`]，用户级配置通过 `cfg` 读写（测试用单独的文件、注入失败）
fn sync_with(on: bool, cred: Option<&GitCred>, repos: &[PathBuf], dir: &Path, cfg: &dyn ConfigBackend) -> Out {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut changes = vec![];
    let cred_host = cred.map(|c| c.host.as_str()).unwrap_or_default();
    // 记录读不了：什么都不改，不然恢复信息就丢了
    let mut rec = match load(dir) {
        Ok(r) => r,
        Err(e) => return Out { sync: failed(e, cred_host), changes },
    };
    let token = cred.and_then(|c| c.token.as_deref()).map(str::trim).filter(|t| !t.is_empty());
    let (Some(c), Some(token), true) = (cred, token, on) else {
        let host = rec.host.clone();
        let cleaned = clean(dir, cfg, &mut rec, repos, &mut changes);
        let sync = match (on, cred, cleaned) {
            (true, Some(c), cleaned) => {
                let mut why = "服务端的 Git 凭据没有令牌".to_string();
                if let Err(e) = cleaned {
                    why.push_str(&format!("；Board 写过的 git 配置没清理完：{e}"));
                }
                failed(why, &c.host)
            }
            (_, _, Err(e)) => failed(format!("Board 写过的 git 配置没清理完：{e}"), &host),
            (false, _, Ok(())) => GitSync { state: "off".into(), ..Default::default() },
            (true, None, Ok(())) => GitSync { state: "none".into(), ..Default::default() },
        };
        return Out { sync, changes };
    };
    let mut errors = vec![];
    // 主机换了：先把旧主机的那一项去掉，再换令牌文件；没去掉就停在这里（旧主机照旧能用），记录里留着旧主机，下次再试
    let old = rec.host.clone();
    if !old.is_empty() && old != c.host {
        match unconfigure(cfg, &old) {
            Ok(true) => changes.push(format!("Git 凭据：从用户级 git 配置里去掉了（{old}）")),
            Ok(false) => {}
            Err(e) => return Out { sync: failed(format!("旧主机 {old} 的 git 配置没去掉：{e}"), &c.host), changes },
        }
    }
    // 旧版按仓库配过的项（记录里的，加上这次看到的仓库）：去掉，用户自己的留着；没去掉的留在记录里
    let left = clean_repos(&rec.repos, repos, &old, &mut changes, &mut errors);
    let file = dir.join(FILE);
    if let Err(e) = write_file(&file, c, token) {
        let mut why = format!("写不了 {}：{e}", shown(&file));
        if let Err(e) = clean(dir, cfg, &mut rec, repos, &mut changes) {
            why.push_str(&format!("；Board 写过的 git 配置没清理完：{e}"));
        }
        return Out { sync: failed(why, &c.host), changes };
    }
    // 先把这次用的主机记下来再改配置：中途失败、崩溃，之后清理也知道去哪个主机清
    rec.host = c.host.clone();
    rec.repos = left;
    if let Err(e) = save(dir, &rec) {
        return Out { sync: failed(e, &c.host), changes };
    }
    match configure(cfg, &c.host, &file) {
        Ok(true) => changes.push(format!("Git 凭据：写进了用户级 git 配置（{}）", c.host)),
        Ok(false) => {}
        Err(e) => errors.push(format!("git 配置没写进去：{e}")),
    }
    let (configured, errs) = configure_identity(cfg, dir, &mut rec, &c.commit_name, &c.commit_email);
    errors.extend(errs.into_iter().map(|e| format!("Git 提交身份没写进去（{e}）")));
    if configured {
        if c.commit_name.is_empty() {
            changes.push("Git 提交身份：恢复了用户级 git 配置里原来的 user.name / user.email".into());
        } else {
            changes.push(format!("Git 提交身份：写进了用户级 git 配置（{} <{}>）", c.commit_name, c.commit_email));
        }
    }
    let sync = if errors.is_empty() {
        GitSync { state: "synced".into(), count: 1, why: None, detail: GitDetail { host: c.host.clone(), ..Default::default() } }
    } else {
        GitSync { count: 1, ..failed(errors.join("；"), &c.host) }
    };
    Out { sync, changes }
}

/// 开始派发前、克隆完之后：在用服务端的凭据，就确认用户级配置里那一项还在（已经配好时不动）。
/// 全局配置按主机生效，不再按仓库补配；只补凭据 helper，不碰提交身份（用户可能自己改过）。
/// 返回写进去的那一句（打印用）
pub fn ensure(_repo: &Path, dir: &Path) -> Option<String> {
    ensure_with(dir, &GitConfig::global(dir))
}

fn ensure_with(dir: &Path, cfg: &dyn ConfigBackend) -> Option<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rec = load(dir).ok()?;
    let file = dir.join(FILE);
    if rec.host.is_empty() || !file.exists() {
        return None;
    }
    let changed = configure(cfg, &rec.host, &file).ok()?;
    changed.then(|| format!("Git 凭据：写进了用户级 git 配置（{}）", rec.host))
}

/// `git clone` 前面加的参数：在用服务端的凭据、地址是凭据的主机时，用 `-c` 带上同样的 helper（克隆出来之后再 [`ensure`]）
pub fn clone_args(url: &str, dir: &Path) -> Vec<String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(rec) = load(dir) else { return vec![] };
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
    use std::cell::RefCell;
    /// 用测试的配置文件同步（不碰跑测试的人自己的 `~/.gitconfig`）
    fn sync_at(on: bool, cred: Option<&GitCred>, repos: &[PathBuf], dir: &Path, cfg: &Path) -> Out {
        sync_with(on, cred, repos, dir, &GitConfig::file(dir, cfg))
    }

    fn ensure_at(dir: &Path, cfg: &Path) -> Option<String> {
        ensure_with(dir, &GitConfig::file(dir, cfg))
    }

    /// 仓库本地配置里这一项的值
    fn local_values(repo: &Path, k: &str) -> Vec<String> {
        GitConfig::local(repo).read(k).unwrap().unwrap_or_default()
    }

    /// 包一层真实的配置文件：指定的键读或写时失败（模拟 git 超时、配置文件坏了、写不进去）
    struct Faulty {
        inner: GitConfig,
        bad_read: RefCell<Vec<&'static str>>,
        bad_write: RefCell<Vec<&'static str>>,
    }

    impl Faulty {
        fn new(dir: &Path, cfg: &Path) -> Faulty {
            Faulty { inner: GitConfig::file(dir, cfg), bad_read: RefCell::default(), bad_write: RefCell::default() }
        }
        fn heal(&self) {
            self.bad_read.borrow_mut().clear();
            self.bad_write.borrow_mut().clear();
        }
    }

    impl ConfigBackend for Faulty {
        fn read(&self, k: &str) -> Result<Option<Vec<String>>, String> {
            if self.bad_read.borrow().contains(&k) {
                return Err("模拟：读配置超时".into());
            }
            self.inner.read(k)
        }
        fn replace(&self, k: &str, values: &[String]) -> Result<(), String> {
            if self.bad_write.borrow().contains(&k) {
                return Err("模拟：写不进去".into());
            }
            self.inner.replace(k, values)
        }
    }

    fn record(dir: &Path) -> Record {
        load(dir).unwrap()
    }

    fn git(cfg: &Path, dir: &Path, args: &[&str]) -> String {
        let out = crate::gitcmd::command()
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", cfg)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn repo(root: &Path, cfg: &Path, name: &str, url: &str) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        git(cfg, &d, &["init", "-q"]);
        git(cfg, &d, &["remote", "add", "origin", url]);
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
        let out = crate::gitcmd::command().arg("config").arg("--file").arg(cfg).args(args).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
        assert!(out.status.success(), "git config --file {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn cfg_values(cfg: &Path, k: &str) -> Vec<String> {
        let out =
            crate::gitcmd::command().arg("config").arg("--file").arg(cfg).args(["--null", "--get-all", k]).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
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
        let out = crate::gitcmd::command()
            .arg("-C")
            .arg(repo)
            .args(["credential", "fill"])
            .env("GIT_CONFIG_GLOBAL", cfg)
            .env("GIT_CONFIG_NOSYSTEM", "1")
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
        let gh = repo(t.path(), &cfg, "gh", "https://github.com/me/board.git");
        let ssh = repo(t.path(), &cfg, "ssh", "git@github.com:me/x.git");
        let lab = repo(t.path(), &cfg, "lab", "https://gitlab.com/me/y.git");
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
            git(&cfg, &gh, &["config", "--local", "--add", k, v]);
        }
        std::fs::write(host_dir.join(RECORD), serde_json::to_string(&Record { host: "github.com".into(), repos: vec![shown(&gh)], ..Default::default() }).unwrap()).unwrap();

        let c = ident("github.com", "tok/en", "BUGLAN", "me@example.com");
        let out = sync_at(true, Some(&c), &repos, &host_dir, &cfg);
        assert_eq!(out.sync.state, "synced", "{:?}", out.sync.why);
        assert_eq!(out.sync.detail.host, "github.com");
        assert_eq!(out.sync.detail.repos, Vec::<String>::new(), "不再按仓库报数量");
        assert_eq!(out.sync.count, 1);
        assert_eq!(cfg_values(&cfg, k), vec![String::new(), mine.to_string(), helper(&host_dir.join(FILE))]);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["me@example.com"]);
        assert!(local_values(&gh, k).is_empty(), "旧版按仓库配的已经去掉");
        assert!(local_values(&ssh, k).is_empty() && local_values(&lab, k).is_empty());
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
        let later = repo(t.path(), &cfg, "later", "https://github.com/me/later.git");
        let got = fill(&later, &cfg, "github.com").expect("新仓库直接用全局凭据");
        assert!(got.contains("username=octo cat") && got.contains("password=tok/en"), "{got}");
        let lab2 = repo(t.path(), &cfg, "lab2", "https://gitlab.com/me/z.git");
        assert!(fill(&lab2, &cfg, "gitlab.com").is_none(), "别的 https 主机不配");

        // 再同步一次：没有变化
        let again = sync_at(true, Some(&c), &repos[..1], &host_dir, &cfg);
        assert_eq!(again.sync.state, "synced");
        assert!(again.changes.is_empty(), "{:?}", again.changes);

        // 克隆参数照样带上 helper（全局配置还没写时也能用）
        assert_eq!(clone_args("https://github.com/me/new.git", &host_dir).len(), 4);
        assert!(clone_args("https://gitlab.com/me/new.git", &host_dir).is_empty());

        // 用户后来自己改了用户名：关掉时只恢复邮箱，用户名保留用户改的
        cfg_git(&cfg, &["user.name", "alice-local"]);
        // 关掉：去掉 Board 写的、用户自己的留着，提交身份只恢复还是 Board 写的项，令牌文件删掉，旧记录也清掉
        let off = sync_at(false, Some(&c), &repos[..1], &host_dir, &cfg);
        assert_eq!(off.sync.state, "off");
        assert_eq!(cfg_values(&cfg, k), vec![mine.to_string()]);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice-local"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
        assert!(!host_dir.join(FILE).exists() && !host_dir.join(RECORD).exists());
        assert!(clone_args("https://github.com/me/new.git", &host_dir).is_empty());
        assert!(fill(&later, &cfg, "github.com").is_none(), "关掉后新仓库也用不到了");
    }

    #[test]
    fn empty_project_dirs_do_not_fail_legacy_cleanup() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        // 项目没设仓库地址时 board host 只建一个空目录；同步不该拿它当旧版配过的仓库去清
        let empty = t.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(
            host_dir.join(RECORD),
            serde_json::to_string(&Record { host: "github.com".into(), repos: vec![shown(&empty)], ..Default::default() }).unwrap(),
        )
        .unwrap();

        let out = sync_at(true, Some(&cred("github.com", "tok")), &[empty.clone()], &host_dir, &cfg);
        assert_eq!(out.sync.state, "synced", "{:?}", out.sync.why);
        assert!(!out.changes.iter().any(|c| c.contains("旧版按仓库")), "{:?}", out.changes);
        let saved = record(&host_dir);
        assert_eq!(saved.host, "github.com");
        assert!(saved.repos.is_empty(), "不是 git 仓库的空目录不用记着清：{:?}", saved.repos);
    }

    #[test]
    fn empty_commit_identity_keeps_existing_global_identity() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        cfg_git(&cfg, &["--add", "user.email", "alice@example.com"]);

        let on = sync_at(true, Some(&cred("github.com", "tok")), &[], &host_dir, &cfg);
        assert_eq!(on.sync.state, "synced");
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
        sync_at(false, Some(&cred("github.com", "tok")), &[], &host_dir, &cfg);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
    }

    #[test]
    fn commit_identity_is_cleared_or_restored_when_sync_turns_off() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        let c = ident("github.com", "tok", "BUGLAN", "me@example.com");

        sync_at(true, Some(&c), &[], &host_dir, &cfg);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["me@example.com"]);
        sync_at(false, Some(&c), &[], &host_dir, &cfg);
        assert!(cfg_values(&cfg, "user.name").is_empty() && cfg_values(&cfg, "user.email").is_empty(), "原来没有就清掉");

        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        sync_at(true, Some(&c), &[], &host_dir, &cfg);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        sync_at(false, Some(&c), &[], &host_dir, &cfg);
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
        let c = ident("github.com", "tok", "BUGLAN", "me@example.com");
        sync_at(true, Some(&c), &[], &host_dir, &cfg);

        cfg_git(&cfg, &["--unset-all", k]);
        cfg_git(&cfg, &["user.name", "alice"]);
        cfg_git(&cfg, &["user.email", "alice@example.com"]);
        assert_eq!(ensure_at(&host_dir, &cfg).as_deref(), Some("Git 凭据：写进了用户级 git 配置（github.com）"));
        assert_eq!(cfg_values(&cfg, k), vec![String::new(), helper(&host_dir.join(FILE))]);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"], "派发前不覆盖用户改过的提交身份");
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
        assert!(ensure_at(&host_dir, &cfg).is_none(), "配好之后不再重复写");
    }

    #[test]
    fn missing_token_or_credential_is_reported_not_silent() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        let none = sync_at(true, None, &[], &host_dir, &cfg);
        assert_eq!(none.sync.state, "none");
        let empty = sync_at(true, Some(&cred("github.com", " ")), &[], &host_dir, &cfg);
        assert_eq!((empty.sync.state.as_str(), empty.sync.why.as_deref()), ("failed", Some("服务端的 Git 凭据没有令牌")));
        // 主机换了：旧主机的那一项去掉，配上新的
        sync_at(true, Some(&cred("github.com", "a")), &[], &host_dir, &cfg);
        let moved = sync_at(true, Some(&cred("git.example.com:8443", "b")), &[], &host_dir, &cfg);
        assert_eq!(moved.sync.state, "synced", "{:?}", moved.sync.why);
        assert!(cfg_values(&cfg, "credential.https://github.com.helper").is_empty());
        assert_eq!(cfg_values(&cfg, "credential.https://git.example.com:8443.helper").len(), 2);
    }

    /// 读配置：没有这一项是 `None`；配置文件坏了是读失败，不当成没有，也不写（卡片 #427）
    #[test]
    fn read_tells_missing_from_failed() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("gitconfig");
        let g = GitConfig::file(t.path(), &cfg);
        assert_eq!(g.read("user.name").unwrap(), None, "文件都没有：没有这一项");
        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        assert_eq!(g.read("user.name").unwrap(), Some(vec!["alice".to_string()]));
        assert_eq!(g.read("user.email").unwrap(), None);
        g.replace("user.name", &[]).unwrap();
        assert_eq!(g.read("user.name").unwrap(), None);
        g.replace("user.name", &[]).unwrap();

        std::fs::write(&cfg, "[user\n\tname = alice\n").unwrap();
        assert!(g.read("user.name").is_err(), "配置文件坏了：读失败");
        assert!(g.replace("user.name", &["BUGLAN".to_string()]).is_err());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "[user\n\tname = alice\n", "读失败就不写");
    }

    /// 换一整项中途失败：写回原来的值，再报原来的错
    #[test]
    fn replace_restores_old_values_when_a_write_fails_midway() {
        let state = RefCell::new(vec!["".to_string(), "mine".to_string()]);
        let before = state.borrow().clone();
        let want = vec!["".to_string(), "mine".to_string(), "board".to_string()];
        // 模拟 git：先整项换成第一个值，再逐个追加；要写的值里有 "board" 时，追加它失败
        let write = |v: &[String]| -> Result<(), String> {
            let mut st = state.borrow_mut();
            st.clear();
            for x in v {
                if x == "board" {
                    return Err("写到一半失败".into());
                }
                st.push(x.clone());
            }
            Ok(())
        };
        assert_eq!(replace_or_restore(&before, &want, write).unwrap_err(), "写到一半失败");
        assert_eq!(*state.borrow(), before, "原来的值写回去了");
        let fail_all = |_: &[String]| -> Result<(), String> { Err("坏了".into()) };
        assert_eq!(replace_or_restore(&before, &want, fail_all).unwrap_err(), "坏了；写回原来的值也失败：坏了");
        assert!(replace_or_restore(&before, &before, fail_all).is_ok(), "已经是了就不写");
    }

    /// 第一次同步时读 user.name 失败：不写这一项、不把原值记成空，报 failed；好了之后再同步、关掉能恢复 alice（卡片 #427）
    #[test]
    fn first_sync_read_failure_keeps_original_identity() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        cfg_git(&cfg, &["--add", "user.email", "alice@example.com"]);
        let c = ident("github.com", "tok", "BUGLAN", "b@example.com");
        let f = Faulty::new(&host_dir, &cfg);

        f.bad_read.borrow_mut().push("user.name");
        let out = sync_with(true, Some(&c), &[], &host_dir, &f);
        assert_eq!(out.sync.state, "failed");
        assert!(out.sync.why.as_deref().unwrap().contains("user.name"), "{:?}", out.sync.why);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"], "没写、也没多出一条");
        let r = record(&host_dir);
        assert!(r.commit_name.is_empty() && r.name_before.is_empty(), "{r:?}");
        assert_eq!((r.commit_email.as_str(), r.email_before.clone()), ("b@example.com", vec!["alice@example.com".to_string()]), "另一项照常写");

        f.heal();
        assert_eq!(sync_with(true, Some(&c), &[], &host_dir, &f).sync.state, "synced");
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        assert_eq!(record(&host_dir).name_before, vec!["alice"]);
        assert_eq!(sync_with(false, Some(&c), &[], &host_dir, &f).sync.state, "off");
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
    }

    /// 关掉时读失败：Board 写的值和原值都留在记录里，报 failed；下次能正常恢复（卡片 #427）
    #[test]
    fn restore_read_failure_keeps_the_record_for_retry() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        let c = ident("github.com", "tok", "BUGLAN", "b@example.com");
        let f = Faulty::new(&host_dir, &cfg);
        assert_eq!(sync_with(true, Some(&c), &[], &host_dir, &f).sync.state, "synced");

        f.bad_read.borrow_mut().push("user.name");
        let off = sync_with(false, Some(&c), &[], &host_dir, &f);
        assert_eq!(off.sync.state, "failed", "清理没做完不能报 off");
        assert!(off.sync.why.as_deref().unwrap().contains("user.name"), "{:?}", off.sync.why);
        let r = record(&host_dir);
        assert_eq!((r.commit_name.as_str(), r.name_before.clone()), ("BUGLAN", vec!["alice".to_string()]));
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        assert!(cfg_values(&cfg, "user.email").is_empty(), "另一项照常恢复（原来没有）");

        f.heal();
        assert_eq!(sync_with(false, Some(&c), &[], &host_dir, &f).sync.state, "off");
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"]);
        assert!(!host_dir.join(RECORD).exists());
    }

    /// 一项写成功、另一项写失败：失败的那项配置和记录都不动，成功的那项有记录，关掉时能恢复（卡片 #427）
    #[test]
    fn one_identity_field_failing_leaves_a_recoverable_record() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        cfg_git(&cfg, &["--add", "user.email", "alice@example.com"]);
        let c = ident("github.com", "tok", "BUGLAN", "b@example.com");
        let f = Faulty::new(&host_dir, &cfg);

        f.bad_write.borrow_mut().push("user.email");
        let out = sync_with(true, Some(&c), &[], &host_dir, &f);
        assert_eq!(out.sync.state, "failed");
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"], "没被改成空");
        let r = record(&host_dir);
        assert_eq!((r.commit_name.as_str(), r.name_before.clone()), ("BUGLAN", vec!["alice".to_string()]));
        assert!(r.commit_email.is_empty() && r.email_before.is_empty(), "写失败的那项记录退回去了：{r:?}");

        f.heal();
        assert_eq!(sync_with(false, Some(&c), &[], &host_dir, &f).sync.state, "off");
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"]);
        assert_eq!(cfg_values(&cfg, "user.email"), vec!["alice@example.com"]);
    }

    /// 记录存不上（这里让临时文件的位置被一个目录占着）、记录坏了：不改任何配置，报 failed（卡片 #427）
    #[test]
    fn record_save_or_load_failure_changes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        cfg_git(&cfg, &["--add", "user.name", "alice"]);
        let c = ident("github.com", "tok", "BUGLAN", "b@example.com");
        let blocker = host_dir.join(format!("{RECORD}.tmp"));
        std::fs::create_dir_all(blocker.join("x")).unwrap();

        let out = sync_at(true, Some(&c), &[], &host_dir, &cfg);
        assert_eq!(out.sync.state, "failed");
        assert!(out.sync.why.as_deref().unwrap().contains("没存上"), "{:?}", out.sync.why);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["alice"]);
        assert!(cfg_values(&cfg, "credential.https://github.com.helper").is_empty(), "凭据也没写");
        std::fs::remove_dir_all(&blocker).unwrap();
        assert_eq!(sync_at(true, Some(&c), &[], &host_dir, &cfg).sync.state, "synced");

        // 记录坏了：不当成空记录，什么都不改（关掉也不动），等人修好
        std::fs::write(host_dir.join(RECORD), "{ 半截").unwrap();
        let off = sync_at(false, Some(&c), &[], &host_dir, &cfg);
        assert_eq!(off.sync.state, "failed");
        assert!(off.sync.why.as_deref().unwrap().contains("坏了"), "{:?}", off.sync.why);
        assert_eq!(cfg_values(&cfg, "user.name"), vec!["BUGLAN"]);
        assert!(host_dir.join(FILE).exists());
        assert!(ensure_at(&host_dir, &cfg).is_none() && clone_args("https://github.com/me/x.git", &host_dir).is_empty());
    }

    /// 关掉时去不掉凭据那一项：令牌文件、主机、提交身份的记录都留着，报 failed；好了之后再关能清干净（卡片 #427）
    #[test]
    fn unconfigure_failure_keeps_token_file_and_host() {
        let t = tempfile::tempdir().unwrap();
        let host_dir = t.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let cfg = host_dir.join("gitconfig");
        let k = "credential.https://github.com.helper";
        let c = ident("github.com", "tok", "BUGLAN", "b@example.com");
        let f = Faulty::new(&host_dir, &cfg);
        assert_eq!(sync_with(true, Some(&c), &[], &host_dir, &f).sync.state, "synced");

        f.bad_read.borrow_mut().push(k);
        let off = sync_with(false, Some(&c), &[], &host_dir, &f);
        assert_eq!(off.sync.state, "failed");
        assert_eq!(off.sync.detail.host, "github.com");
        assert!(off.sync.why.as_deref().unwrap().contains("github.com"), "{:?}", off.sync.why);
        assert!(host_dir.join(FILE).exists(), "helper 还在，令牌文件不能删");
        assert_eq!(record(&host_dir).host, "github.com");
        assert_eq!(cfg_values(&cfg, k).len(), 2);
        assert!(cfg_values(&cfg, "user.name").is_empty(), "提交身份照常恢复（原来没有）");

        // 主机换了、旧主机去不掉：停下来，记录里还是旧主机
        let moved = sync_with(true, Some(&ident("git.example.com", "t2", "BUGLAN", "b@example.com")), &[], &host_dir, &f);
        assert_eq!(moved.sync.state, "failed");
        assert_eq!(record(&host_dir).host, "github.com");
        assert!(std::fs::read_to_string(host_dir.join(FILE)).unwrap().ends_with("@github.com\n"), "令牌文件还是旧主机的，旧主机照旧能用");

        f.heal();
        assert_eq!(sync_with(false, Some(&c), &[], &host_dir, &f).sync.state, "off");
        assert!(cfg_values(&cfg, k).is_empty() && !host_dir.join(FILE).exists() && !host_dir.join(RECORD).exists());
    }
}
