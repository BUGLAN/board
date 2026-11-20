//! 本机的 agent：内置模板和配置文件里的 agent、在各启动环境里找命令和版本、安装和更新（npm、Homebrew、官方脚本）。

use super::*;

/// 一次正在跑的 agent 安装/更新（后台子进程，不占派发会话位）
pub(super) struct AgentJob {
    pub(super) ask: i64,
    pub(super) agent: String,
    pub(super) name: String,
    pub(super) target: String,
    /// 装进用户目录时，装完要写进登录 shell PATH 的命令目录（`<prefix>/bin`）
    pub(super) bin_dir: Option<PathBuf>,
    /// 用户目录版本要盖住的旧命令目录（npm 全局 prefix 下的 `bin`）：PATH 里它在前面时把用户目录挪到前面
    pub(super) shadow_dir: Option<PathBuf>,
    /// npm 先装进的临时 prefix：成功后换进真正的 prefix，失败、超时只删它，旧版本不动
    pub(super) stage: Option<Stage>,
    pub(super) child: Child,
    pub(super) started: Instant,
    /// 最多跑多久（`[host] agent_job_minutes`，默认 [`AGENT_JOB_LIMIT`]）
    pub(super) limit: Duration,
    pub(super) log: PathBuf,
}

/// 一次 agent 安装/更新怎么执行：环境、命令、参数，以及完成后要不要调整登录 shell 的 PATH
pub(super) struct AgentPlan {
    pub(super) env: Env,
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) bin_dir: Option<PathBuf>,
    pub(super) shadow_dir: Option<PathBuf>,
    pub(super) stage: Option<Stage>,
}

/// npm 装进 `--prefix <目录>` 时，先装进这个目录里的临时 prefix，装好了再换进去（#536）。
/// npm 在 reify 开头就把旧包和 `bin` 里的软链挪成隐藏文件，被中途结束（超时）时不会挪回来，
/// 旧命令就没了；先装到别处，旧版本在成功之前一直能用，失败、超时只删临时目录
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Stage {
    /// 临时 prefix：放在真正的 prefix 里面，同一个文件系统，换进去只是改名
    pub(super) dir: PathBuf,
    /// 真正的 prefix
    pub(super) prefix: PathBuf,
    /// npm 包名（`@openai/codex`）
    pub(super) package: String,
}

/* ------------------------------ 本机 agent ------------------------------ */

pub(super) struct Spec {
    pub(super) id: String,
    pub(super) name: String,
    /// 配置文件 `[agents.<标识>] model`：内置 agent 读不到它自己配置文件里的模型时、配置文件里新加的 agent 上报它
    /// （派发名字 `<机器名>/<模型名>-<强度>` 的第二段）
    pub(super) model: String,
    /// agent 自己的命令：在每个启动环境里找它、取版本号（内置的是 `AgentDef::bin`，配置文件里新加的是 ACP 入口的第一个）
    pub(super) bin: String,
    /// ACP 入口：`board acp` 在启动环境里执行的命令（`{bin}`、`{patch}` 见 `AgentDef::acp`）
    pub(super) acp: Vec<String>,
    /// 告诉适配器用哪个 agent 的环境变量、另外的环境变量、会话开始后切到的模式（见 `AgentDef`）
    pub(super) acp_bin_env: String,
    pub(super) acp_env: Vec<(String, String)>,
    pub(super) acp_mode: String,
    /// 配置文件里写的是旧格式（命令里带 `{prompt}`，以前一次性命令的写法）：不能用，派发时报错说明
    pub(super) legacy: bool,
    /// 读用户级 skill 的目录（内置模板才有）
    pub(super) skills: Vec<String>,
    /// 在仓库目录里读 skill 的相对目录（内置模板才有）
    pub(super) project: Vec<String>,
    /// 在它支持、本机有的每个启动环境里的检查结果，按默认选择的顺序
    pub(super) checks: Vec<Check>,
    /// 找到了命令、却看不到 `board-skills` 的启动环境 → 该装到哪里（启动时由 [`Spec::check_skills`] 检查一次）
    pub(super) no_skill: BTreeMap<String, String>,
    /// 这个内置 agent 的模板；配置文件里新加的为空
    pub(super) builtin: Option<&'static Builtin>,
    /// 能不能由 board host 代装、代升（配置文件覆盖了内置命令时为 false）
    pub(super) managed: bool,
    /// npm 上查到的最新版本、检查时间；没查到时为空
    pub(super) latest: String,
    pub(super) checked_at: i64,
    /// 正在安装/更新，或上一次失败
    pub(super) manage_state: String,
    pub(super) manage_error: String,
}

/// 版本号前面的数字：`0.2.2-rc.1` → [0, 2, 2]、`2.1.311 (Claude Code)` → [2, 1, 311]；没有数字时为空
pub(super) fn version_nums(v: &str) -> Vec<u64> {
    let start = v.find(|c: char| c.is_ascii_digit()).unwrap_or(v.len());
    let head: String = v[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    head.split('.').filter_map(|x| x.parse().ok()).collect()
}

/// `have` 比 `need` 旧（只比数字部分；看不出版本时当作够新）
pub(super) fn older_than(have: &str, need: &str) -> bool {
    let (a, b) = (version_nums(have), version_nums(need));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let n = a.len().max(b.len());
    let pad = |v: &[u64]| (0..n).map(|i| v.get(i).copied().unwrap_or(0)).collect::<Vec<_>>();
    pad(&a) < pad(&b)
}

/// agent 最新版本比较（含 `-rc.1` 这类预发布）：和 board 自己的版本规则一致
pub(super) fn manage_older(have: &str, need: &str) -> bool {
    board_core::hosts::version_lt(have, need)
}

/// npm registry 的 `/<包>/<dist-tag>` 返回里取 `version`
pub(super) fn npm_latest(json: &str) -> std::result::Result<String, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("版本信息不是 JSON：{e}"))?;
    let version = v.get("version").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    version.map(str::to_string).ok_or_else(|| "版本信息里没有 version".into())
}

/// 查一个 npm 包的最新版本（每天一次，超时 15 秒；不影响机器心跳）
pub(super) fn fetch_npm_latest(package: &str, tag: &str) -> std::result::Result<String, String> {
    let base = std::env::var("BOARD_AGENT_REGISTRY").unwrap_or_else(|_| "https://registry.npmjs.org".into());
    let url = format!("{}/{}/{}", base.trim_end_matches('/'), package.replace('/', "%2f"), tag);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(AGENT_CHECK_TIMEOUT))
        .http_status_as_error(false)
        .user_agent(concat!("board-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut res = agent.get(&url).call().map_err(|e| format!("连不上 {url}：{e}"))?;
    let status = res.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("查 {url} 返回 HTTP {status}"));
    }
    let body = res
        .body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_to_string()
        .map_err(|e| format!("读 {url} 失败：{e}"))?;
    npm_latest(&body)
}

/// 命令的实际路径（解析软链接），统一成 `/` 分隔；保留大小写，执行命令时用
pub(super) fn resolved_path(prog: Option<&str>) -> String {
    let Some(p) = prog.map(expand) else { return String::new() };
    p.canonicalize().unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// 命令的实际路径统一成小写，用来判断安装方式（macOS、Windows 的路径不区分大小写）
pub(super) fn real_path(prog: Option<&str>) -> String {
    resolved_path(prog).to_ascii_lowercase()
}

/// npm 全局安装的 prefix：Unix 在 `<prefix>/lib/node_modules`，Windows 在 `<prefix>/node_modules`
pub(super) fn npm_global_prefix(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    ["/lib/node_modules/", "/node_modules/"].iter().find_map(|marker| {
        let prefix = path.split_once(marker)?.0;
        (!prefix.is_empty()).then(|| prefix.to_string())
    })
}

/// 没安装时界面上的命令。Unix 不往 `/usr/local` 这类需要管理员的目录写，装进用户目录；
/// board host 也会在 `~/.local/bin` 找命令，所以复制这条命令单独执行也能用
pub(super) fn install_command(b: &Builtin) -> String {
    #[cfg(unix)]
    if let Some(rest) = b.install.strip_prefix("npm install -g ") {
        return format!("npm install -g --prefix ~/.local {rest}");
    }
    b.install.to_string()
}

/// board host 实际执行的安装命令：Unix 上 npm 装到 `~/.local`，不需要 sudo
pub(super) fn install_plan(b: &Builtin) -> std::result::Result<(String, Vec<String>), String> {
    let mut it = b.install.split_whitespace();
    let program = it.next().ok_or_else(|| "没有安装命令".to_string())?;
    let mut args: Vec<String> = it.map(str::to_string).collect();
    #[cfg(unix)]
    if program == "npm" {
        if let Some(i) = args.iter().position(|a| a == "-g") {
            args.splice(i + 1..i + 1, ["--prefix".to_string(), expand("~/.local").display().to_string()]);
        }
    }
    Ok((program.to_string(), args))
}

/// npm 更新参数：沿实际安装的 prefix 更新，不能改用 npm 当前的全局配置（两者可能不一样）
pub(super) fn npm_update_args(prefix: Option<&str>, package: &str, target: &str) -> Vec<String> {
    let mut args = vec!["install".to_string(), "-g".to_string()];
    if let Some(prefix) = prefix.filter(|p| !p.is_empty()) {
        args.push("--prefix".to_string());
        args.push(prefix.to_string());
    }
    args.push(format!("{package}@{}", if target.is_empty() { "latest" } else { target }));
    args
}

/// Unix 上 npm 要求 prefix 下面的 `lib` 已经存在；用户目录第一次安装时先建好
#[cfg(unix)]
pub(super) fn prepare_npm_install(program: &str, args: &[String]) -> std::result::Result<(), String> {
    if program != "npm" {
        return Ok(());
    }
    let Some(i) = args.iter().position(|a| a == "--prefix") else { return Ok(()) };
    let Some(prefix) = args.get(i + 1) else { return Err("npm 安装参数里缺 --prefix 的目录".into()) };
    std::fs::create_dir_all(Path::new(prefix).join("lib")).map_err(|e| format!("没法创建 npm 用户目录 {prefix}/lib：{e}"))
}

#[cfg(not(unix))]
pub(super) fn prepare_npm_install(_program: &str, _args: &[String]) -> std::result::Result<(), String> {
    Ok(())
}

/// npm 装到 `--prefix <目录>` 时命令放在 `<目录>/bin`（用来把用户目录加进登录 shell 的 PATH）
pub(super) fn npm_bin_dir(program: &str, args: &[String]) -> Option<PathBuf> {
    if program != "npm" {
        return None;
    }
    let i = args.iter().position(|a| a == "--prefix")?;
    args.get(i + 1).map(|p| Path::new(p).join("bin"))
}

/// npm 装进 `--prefix <目录>` 时改成先装进 `<目录>/.board-stage-<agent>`：返回改好的参数和要换进去的 [`Stage`]。
/// 没写 `--prefix`（沿 npm 当前的全局配置）时不知道装到哪，照原样装
pub(super) fn stage_npm(program: &str, mut args: Vec<String>, package: &str, agent: &str) -> (Vec<String>, Option<Stage>) {
    if program != "npm" || package.is_empty() {
        return (args, None);
    }
    let Some(i) = args.iter().position(|a| a == "--prefix") else { return (args, None) };
    let Some(prefix) = args.get(i + 1).map(PathBuf::from) else { return (args, None) };
    let dir = prefix.join(format!(".board-stage-{agent}"));
    args[i + 1] = dir.display().to_string();
    (args, Some(Stage { dir, prefix, package: package.to_string() }))
}

/// 找到过的命令路径现在不在了（绝对路径、跟着软链找不到文件）
pub(super) fn path_gone(path: &str) -> bool {
    let p = Path::new(path);
    p.is_absolute() && !p.exists()
}

/// npm 全局包在 prefix 下的目录：Unix 是 `<prefix>/lib/node_modules/<包>`，Windows 是 `<prefix>/node_modules/<包>`
pub(super) fn npm_pkg_dir(prefix: &Path, package: &str, windows: bool) -> PathBuf {
    let base = if windows { prefix.join("node_modules") } else { prefix.join("lib").join("node_modules") };
    package.split('/').fold(base, |p, part| p.join(part))
}

/// npm 全局命令放在哪：Unix 是 `<prefix>/bin`，Windows 直接在 prefix 下（`codex.cmd`、`codex.ps1`…）
pub(super) fn npm_cmd_dir(prefix: &Path, windows: bool) -> PathBuf {
    if windows {
        prefix.to_path_buf()
    } else {
        prefix.join("bin")
    }
}

/// 删掉目录（不存在不算错）
pub(super) fn remove_tree(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

impl Stage {
    /// 开始前：删掉上一次（host 被结束、崩溃）留下的临时目录，建好 npm 要的 `lib`
    pub(super) fn reset(&self, windows: bool) -> std::result::Result<(), String> {
        remove_tree(&self.dir).map_err(|e| format!("删不掉上次留下的临时目录 {}：{e}", self.dir.display()))?;
        let need = if windows { self.dir.clone() } else { self.dir.join("lib") };
        std::fs::create_dir_all(&need).map_err(|e| format!("没法创建临时目录 {}：{e}", need.display()))
    }

    /// 失败、超时：只删临时目录，真正的 prefix 里的旧版本原样留着
    pub(super) fn discard(&self) {
        let _ = remove_tree(&self.dir);
    }

    /// 装好了：把包换进真正的 prefix（旧包先改名备份，换不进去就改回来），再把命令挪过去，最后删掉备份和临时目录
    pub(super) fn commit(&self, windows: bool) -> std::result::Result<(), String> {
        let fresh = npm_pkg_dir(&self.dir, &self.package, windows);
        if !fresh.join("package.json").is_file() {
            self.discard();
            return Err(format!("npm 报告成功，但临时目录里没有装好的 {}：旧版本没动", self.package));
        }
        let target = npm_pkg_dir(&self.prefix, &self.package, windows);
        let parent = target.parent().map(Path::to_path_buf).unwrap_or_else(|| self.prefix.clone());
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let backup = parent.join(format!(".{name}-board-old"));
        let swap = || -> std::result::Result<bool, String> {
            std::fs::create_dir_all(&parent).map_err(|e| format!("没法创建 {}：{e}", parent.display()))?;
            remove_tree(&backup).map_err(|e| format!("删不掉上次留下的备份 {}：{e}", backup.display()))?;
            let had_old = target.symlink_metadata().is_ok();
            if had_old {
                std::fs::rename(&target, &backup).map_err(|e| format!("没法把旧版本挪开（可能正在运行）：{e}"))?;
            }
            if let Err(e) = std::fs::rename(&fresh, &target) {
                if had_old {
                    let _ = std::fs::rename(&backup, &target);
                }
                return Err(format!("没法把新版本换进 {}：{e}", target.display()));
            }
            Ok(had_old)
        };
        let had_old = match swap() {
            Ok(x) => x,
            Err(why) => {
                self.discard();
                return Err(format!("{why}；旧版本没动"));
            }
        };
        let moved = self.move_cmds(windows);
        if had_old {
            let _ = remove_tree(&backup);
        }
        self.discard();
        moved
    }

    /// 把临时 prefix 里的命令（npm 建的软链、Windows 的 `.cmd` / `.ps1`）挪进真正的 prefix，同名的直接替换。
    /// npm 的软链是相对路径（`../lib/node_modules/<包>/…`），挪过去照样指向换进去的包；指到临时目录的绝对路径改指真正的 prefix
    pub(super) fn move_cmds(&self, windows: bool) -> std::result::Result<(), String> {
        let from = npm_cmd_dir(&self.dir, windows);
        let to = npm_cmd_dir(&self.prefix, windows);
        let Ok(entries) = std::fs::read_dir(&from) else { return Ok(()) };
        std::fs::create_dir_all(&to).map_err(|e| format!("没法创建 {}：{e}", to.display()))?;
        for e in entries.flatten() {
            let Ok(meta) = e.path().symlink_metadata() else { continue };
            if meta.is_dir() {
                continue;
            }
            let dest = to.join(e.file_name());
            #[cfg(unix)]
            if meta.file_type().is_symlink() {
                if let Ok(link) = std::fs::read_link(e.path()) {
                    if let Ok(rest) = link.strip_prefix(&self.dir) {
                        let tmp = to.join(format!(".{}-board-new", e.file_name().to_string_lossy()));
                        let _ = std::fs::remove_file(&tmp);
                        std::os::unix::fs::symlink(self.prefix.join(rest), &tmp)
                            .and_then(|_| std::fs::rename(&tmp, &dest))
                            .map_err(|err| format!("没法建命令 {}：{err}", dest.display()))?;
                        continue;
                    }
                }
            }
            std::fs::rename(e.path(), &dest).map_err(|err| format!("没法把命令放进 {}：{err}", dest.display()))?;
        }
        Ok(())
    }
}

/// 命令目录在用户目录下：装完要写进登录 shell 的 PATH；系统目录由系统管理，不去改
pub(super) fn under_home(dir: &Path) -> bool {
    dir.starts_with(expand("~"))
}

/// 登录 shell 的配置文件：zsh 读 `~/.zshrc`，bash 登录读 `~/.bash_profile`（没有时 `~/.bash_login`、`~/.profile`），
/// fish 读 `~/.config/fish/config.fish`，其他用 `~/.profile`。返回 (文件, 是不是 fish)
#[cfg(any(unix, test))]
pub(super) fn login_rc(env: &Env) -> (PathBuf, bool) {
    let shell = env.prog.file_name().and_then(|s| s.to_str()).unwrap_or_default().to_ascii_lowercase();
    if shell.contains("zsh") {
        (expand("~/.zshrc"), false)
    } else if shell.contains("bash") {
        (bash_rc(&expand("~")), false)
    } else if shell.contains("fish") {
        (expand("~/.config/fish/config.fish"), true)
    } else {
        (expand("~/.profile"), false)
    }
}

/// bash 登录 shell 读 `~/.bash_profile`、`~/.bash_login`、`~/.profile` 里第一个存在的，都没有时写 `~/.bash_profile`
#[cfg(any(unix, test))]
pub(super) fn bash_rc(home: &Path) -> PathBuf {
    [".bash_profile", ".bash_login", ".profile"].iter().map(|n| home.join(*n)).find(|p| p.is_file()).unwrap_or_else(|| home.join(".bash_profile"))
}

/// 写进 shell 配置的那一行
#[cfg(any(unix, test))]
pub(super) fn path_line(dir: &Path, fish: bool) -> String {
    let dir = dir.display().to_string().replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`");
    if fish {
        format!("fish_add_path \"{dir}\"")
    } else {
        format!("export PATH=\"{dir}:$PATH\"")
    }
}

/// 把目录写进 shell 配置的 PATH；同一行已经在文件里时不重复追加（`allow_duplicate` 时除外：
/// PATH 里已有这个目录、但被前面的目录盖住时，要在文件末尾再放一次，结束时的 PATH 才会以它开头）。返回是否写了
#[cfg(any(unix, test))]
pub(super) fn append_path_line(rc: &Path, dir: &Path, fish: bool, allow_duplicate: bool) -> std::io::Result<bool> {
    let line = path_line(dir, fish);
    let old = match std::fs::read_to_string(rc) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if !allow_duplicate && old.lines().any(|l| l.trim() == line) {
        return Ok(false);
    }
    if let Some(d) = rc.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(rc)?;
    if !old.is_empty() && !old.ends_with('\n') {
        writeln!(f)?;
    }
    writeln!(f, "{line}")?;
    Ok(true)
}

/// 这条 PATH 里 `dir` 有没有出现、并且在 `before` 前面（`before` 为空时只要求出现）
#[cfg(unix)]
pub(super) fn path_has_dir_before(path: &str, dir: &Path, before: Option<&Path>) -> bool {
    let dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    let Some(at) = dirs.iter().position(|p| p == dir) else { return false };
    match before.and_then(|b| dirs.iter().position(|p| p == b)) {
        Some(before_at) => at <= before_at,
        None => true,
    }
}

/// 登录 shell 读启动脚本之后的 PATH（不借任何回退位置）
#[cfg(unix)]
pub(super) fn login_path(env: &Env) -> Option<String> {
    let args = ["-c".to_string(), "printf __BOARD_PATH__%s \"$PATH\"".to_string()];
    let out = envs::run_out(env.command("/bin/sh", &args, None, false), Duration::from_secs(15))?;
    let path = out.stdout.rsplit_once("__BOARD_PATH__").map(|(_, p)| p.trim().to_string()).unwrap_or_default();
    out.ok.then_some(path)
}

/// 把用户目录的命令目录写进登录 shell 的 PATH（只在登录 shell 的 PATH 里没有、或它在 `shadow` 后面时）。
/// board host 自己会去 `~/.local/bin` 找命令，但用户新开终端也要能直接敲命令。
#[cfg(unix)]
pub(super) fn ensure_login_path(env: &Env, dir: &Path, shadow: Option<&Path>) -> std::io::Result<bool> {
    let current = login_path(env);
    if current.as_deref().is_some_and(|path| path_has_dir_before(path, dir, shadow)) {
        return Ok(false);
    }
    let present = current.as_deref().is_some_and(|path| std::env::split_paths(path).any(|p| p.as_path() == dir));
    let (rc, fish) = login_rc(env);
    append_path_line(&rc, dir, fish, present)
}

#[cfg(not(unix))]
pub(super) fn ensure_login_path(_env: &Env, _dir: &Path, _shadow: Option<&Path>) -> std::io::Result<bool> {
    Ok(false)
}

/// 当前用户能不能写 npm 的全局目录；目录还不存在时看最近一个已有的上级目录
#[cfg(unix)]
pub(super) fn writable_dir(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let mut path = path.to_path_buf();
    while !path.exists() {
        if !path.pop() {
            return false;
        }
    }
    if !path.is_dir() {
        return false;
    }
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else { return false };
    unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

/// 安装/更新子进程失败时，优先说清真正的原因；npm 最后一行通常只是日志路径
pub(super) fn agent_job_error(code: i32, tail: &[String]) -> String {
    if let Some(eacces) = tail.iter().rev().find(|line| line.contains("EACCES") || line.to_ascii_lowercase().contains("permission denied")) {
        return format!("退出码 {code}：没有权限写 npm 全局目录（{eacces}）；请改用用户目录安装，或在有权限的终端里手动升级");
    }
    let detail = tail
        .iter()
        .rev()
        .map(|line| line.trim())
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("A complete log of this run can be found in:")
                && !line.starts_with("The operation was rejected by your operating system")
                && !line.contains("npm/_logs/")
        })
        .unwrap_or("没有输出");
    format!("退出码 {code}：{detail}")
}

pub(super) fn brew_formula(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    parts.iter().position(|x| *x == "cellar").and_then(|i| parts.get(i + 1)).map(|s| s.to_string()).filter(|s| !s.is_empty())
}

impl Spec {
    /// 这个启动环境里的安装方式、最新版本和能否代管。配置文件里新加/覆盖了命令的不代管；WSL 也不代管。
    pub(super) fn manage_for(&self, check: Option<&Check>) -> AgentManage {
        let Some(b) = self.builtin.filter(|_| self.managed) else {
            return AgentManage {
                method: "config".into(),
                method_label: "配置文件".into(),
                reason: "这个 agent 来自配置文件（或改了内置命令）：Board 只显示版本，不代装、不代升".into(),
                latest_version: self.latest.clone(),
                checked_at: self.checked_at,
                state: self.manage_state.clone(),
                error: self.manage_error.clone(),
                ..Default::default()
            };
        };
        let mut m = AgentManage {
            latest_version: self.latest.clone(),
            checked_at: self.checked_at,
            state: self.manage_state.clone(),
            error: self.manage_error.clone(),
            ..Default::default()
        };
        if check.is_some_and(|c| c.env.starts_with("wsl:")) {
            m.method = "wsl".into();
            m.method_label = "WSL".into();
            m.reason = "WSL 里的 agent 由发行版自己管理：Board 只显示版本，不代装、不代升".into();
            m.update_available = check.is_some_and(|c| c.found && !self.latest.is_empty() && manage_older(&c.version, &self.latest));
            return m;
        }
        let Some(c) = check else {
            m.method = "missing".into();
            m.method_label = "没安装".into();
            m.install_command = install_command(b);
            m.installable = true;
            return m;
        };
        // 装了 agent、缺 ACP 入口要的命令（Node.js）：不是没安装，别让人去重装 agent（#458）
        if let Some(what) = &c.missing {
            m.method = "needs".into();
            m.method_label = format!("缺 {what}");
            m.reason = format!("找到了 {}，但这个启动环境里没有 {what}（ACP 入口要用它）：装上后在「设置」里点「重新检查」", self.name);
            return m;
        }
        if !c.found {
            m.method = "missing".into();
            m.method_label = "没安装".into();
            m.install_command = install_command(b);
            m.installable = true;
            return m;
        }
        let path = real_path(c.path.as_deref().or(c.prog.as_deref()));
        m.update_available = !self.latest.is_empty() && manage_older(&c.version, &self.latest);
        if path.contains(".app/contents/") {
            m.method = "app".into();
            m.method_label = "桌面 app 自带".into();
            m.reason = "命令行随桌面 app 安装：在 app 里更新，Board 不单独升级".into();
        } else if path.contains("/node_modules/") || path.contains("/.npm-global/") || path.contains("/npm/") {
            m.method = "npm-global".into();
            m.method_label = "npm 全局".into();
            m.updatable = true;
        } else if let Some(f) = brew_formula(&path) {
            m.method = "brew".into();
            m.method_label = format!("Homebrew（{f}）");
            m.updatable = true;
        } else if path.contains("/.local/bin/") || path.contains("/.local/share/") || path.contains("/.codex/bin/") {
            m.method = "script".into();
            m.method_label = "官方安装脚本".into();
            // Claude Code 的官方安装脚本自带 update；其他脚本装的仍用 npm 包升级
            m.updatable = true;
        } else {
            m.method = "unknown".into();
            m.method_label = "其他方式".into();
            m.reason = "看不出是哪种常见安装方式：请在那台机器上手动升级".into();
        }
        m
    }

    /// 默认的启动环境：按顺序第一个找到命令的
    pub(super) fn default_check(&self) -> Option<&Check> {
        self.checks.iter().find(|c| c.found)
    }

    /// 在这个启动环境里的 ACP 入口：（程序、参数）。`{bin}` 换成找到的 agent 命令，`{patch}` 换成补丁路径
    pub(super) fn entry(&self, check: &Check, patch: &str) -> std::result::Result<(String, Vec<String>), String> {
        if self.legacy {
            return Err(format!(
                "配置文件里 [agents.{}] cmd 是旧写法（带 {{prompt}}）：board host 现在用 ACP 启动 agent，cmd 要写成 ACP 入口命令（不带 {{prompt}}），见 docs/agents.md",
                self.id
            ));
        }
        let bin = check.prog.clone().unwrap_or_else(|| self.bin.clone());
        let mut it = self.acp.iter().map(|a| a.replace("{bin}", &bin).replace("{patch}", patch));
        let prog = it.next().filter(|p| !p.trim().is_empty()).ok_or_else(|| format!("{} 没有 ACP 入口命令", self.name))?;
        Ok((prog, it.collect()))
    }

    /// 传给 ACP 入口的环境变量：适配器用哪个 agent（找到的路径，见 [`adapter_bin`]），加上内置描述里另外的
    pub(super) fn acp_vars(&self, check: &Check) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.acp_env.clone();
        if !self.acp_bin_env.is_empty() {
            let path = check.path.clone().or_else(|| check.prog.clone()).unwrap_or_default();
            let windows = cfg!(windows) && !check.env.starts_with("wsl:");
            let npm_exe = self.builtin.map(|b| b.npm_exe).unwrap_or_default();
            if let Some(bin) = adapter_bin(&path, windows, npm_exe) {
                out.push((self.acp_bin_env.clone(), bin));
            }
        }
        out
    }

    /// 上报的样子。`now` 是这个 agent 配置文件里现在的模型和强度（内置 agent 读得到时，见 [`providersync::current`]）；
    /// 读不到时模型用配置文件 `[agents.<标识>] model`
    pub(super) fn report(&self, now: Option<&(String, String)>) -> HostAgent {
        let def = self.default_check();
        // 哪个环境都用不了、其中有装了 agent 只是缺 Node.js 的：按它报（版本、缺什么、管理方式），不报成没安装（#458）
        let short = def.is_none().then(|| self.checks.iter().find(|c| c.missing.is_some())).flatten();
        let (model, effort) = match now {
            Some((m, e)) if !m.is_empty() => (m.clone(), e.clone()),
            _ => (self.model.clone(), String::new()),
        };
        let manage = def.or(short).or_else(|| self.checks.first()).map(|c| self.manage_for(Some(c)));
        HostAgent {
            id: self.id.clone(),
            name: self.name.clone(),
            version: def.or(short).map(|c| c.version.clone()).unwrap_or_default(),
            found: def.is_some(),
            missing: short.and_then(|c| c.missing.clone()),
            envs: self
                .checks
                .iter()
                .map(|c| EnvCheck { no_skill: self.no_skill.get(&c.env).cloned(), manage: Some(self.manage_for(Some(c))), ..c.report() })
                .collect(),
            env: None,
            no_skill: def.and_then(|c| self.no_skill.get(&c.env).cloned()),
            model,
            effort,
            manage,
            // 配置文件里新加的（不是内置的）：服务端据此不把它当成已经移除的内置 agent 丢掉（#484）
            custom: self.builtin.is_none(),
            ..Default::default()
        }
    }

    /// 在每个找到了命令的启动环境里看一次 `board-skills`，看不到的记下该装到哪里（终端和界面上的提醒用同一句）
    pub(super) fn check_skills(&mut self, envs: &[Env], dirs: &[(String, PathBuf)]) {
        self.no_skill = envs
            .iter()
            .filter(|e| self.misses_skill(e, dirs))
            .map(|e| {
                let user: Vec<String> = self.skills.iter().map(|d| format!("{d}/{SKILL}")).collect();
                let place = if e.wsl().is_some() { format!("{}里的 ", e.name) } else { String::new() };
                let repo: Vec<String> = self.project.iter().map(|d| format!("仓库的 {d}/{SKILL}")).collect();
                (e.id.clone(), format!("{place}{}", [user, repo].concat().join(" 或 ")))
            })
            .collect();
    }

    /// 在 env 里找到了、但它读 skill 的目录里都没有 `board-skills/SKILL.md`。
    /// 用户级目录来自内置模板（WSL 里看 WSL 用户目录下的）；项目级目录看每块白板对应的仓库目录里这个 agent 会读的那几个
    /// （例如 Claude Code 只读 .claude/skills，不读 .agents/skills）
    pub(super) fn misses_skill(&self, env: &Env, dirs: &[(String, PathBuf)]) -> bool {
        if self.skills.is_empty() || !self.checks.iter().any(|c| c.env == env.id && c.found) {
            return false;
        }
        let has = |d: &Path| d.join(SKILL).join("SKILL.md").is_file();
        !envs::has_skill(env, &self.skills, SKILL) && !dirs.iter().any(|(_, p)| self.project.iter().any(|s| has(&p.join(s))))
    }
}

/// 一个模板（内置的，或配置文件里的）
pub(super) struct Template {
    pub(super) id: String,
    pub(super) name: String,
    /// 配置文件里写的模型名（派发名字的第二段，读不到 agent 配置文件里的模型时用）
    pub(super) model: String,
    pub(super) bin: String,
    pub(super) acp: Vec<String>,
    pub(super) acp_bin_env: String,
    pub(super) acp_env: Vec<(String, String)>,
    pub(super) acp_mode: String,
    pub(super) legacy: bool,
    /// 命令不在 PATH 里时再看的位置（只在直接启动、登录 shell 里用）
    pub(super) also: Vec<String>,
    /// ACP 入口另外要的命令和缺了时的称呼（内置描述的 `needs`，例如 npx）：每个启动环境里一起找，缺了这个环境就不可用（#458）
    pub(super) needs: Vec<(String, String)>,
    pub(super) skills: Vec<String>,
    pub(super) project: Vec<String>,
    /// 支持的启动环境，按默认选择的顺序
    pub(super) order: Vec<String>,
    pub(super) builtin: Option<&'static Builtin>,
    pub(super) managed: bool,
}

/// 配置文件里的命令是以前一次性命令的写法（带 `{prompt}`）
pub(super) fn legacy_cmd(cmd: &[String]) -> bool {
    cmd.iter().any(|a| a.contains("{prompt}"))
}

/// 告诉适配器用哪个 agent 时传的路径（`CLAUDE_CODE_EXECUTABLE`、`CODEX_PATH`）。`windows`：agent 在 Windows 上跑（不在 WSL 里）。
///
/// Windows 上 `PATH` 里找到的常常是 npm 给全局命令生成的垫片：无扩展名的 sh 脚本（Git Bash 的 `command -v` 找到它）、
/// `.cmd`、`.ps1`。适配器直接 spawn 它们会失败（Claude Code 的适配器用 `execFile`，报 `spawn … ENOENT`，#456），
/// 所以按内置描述的 `npm_exe` 在 npm 的全局前缀（垫片所在的目录）里找包里真正的程序传过去：还是机器上这一份，
/// 版本和机器详情里的一样。找不到时不传，适配器用它自带的那份。`.exe` 和别的系统上的路径原样传
pub(super) fn adapter_bin(path: &str, windows: bool, npm_exe: &[&str]) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    if !windows || path.to_ascii_lowercase().ends_with(".exe") {
        return Some(path.to_string());
    }
    let prefix = Path::new(path).parent()?;
    npm_exe.iter().find_map(|rel| glob_file(prefix, rel).filter(|p| is_pe(p))).map(|p| p.display().to_string())
}

/// `rel` 按 `/` 分段接在 `dir` 后面；含 `*` 的一段匹配那一层里名字对得上的（`*` 前后的部分）。返回第一个存在的文件
pub(super) fn glob_file(dir: &Path, rel: &str) -> Option<PathBuf> {
    let mut at = vec![dir.to_path_buf()];
    for seg in rel.split('/') {
        at = match seg.split_once('*') {
            None => at.into_iter().map(|d| d.join(seg)).collect(),
            Some((head, tail)) => at
                .into_iter()
                .flat_map(|d| {
                    let mut hit: Vec<PathBuf> = std::fs::read_dir(&d)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter(|e| {
                            let n = e.file_name().to_string_lossy().into_owned();
                            n.len() >= head.len() + tail.len() && n.starts_with(head) && n.ends_with(tail)
                        })
                        .map(|e| e.path())
                        .collect();
                    hit.sort();
                    hit
                })
                .collect(),
        };
    }
    at.into_iter().find(|p| p.is_file())
}

/// Windows 的可执行文件（PE）以 `MZ` 开头。Claude Code 的 npm 包里 `bin/claude.exe` 起初是一段 sh 占位脚本，
/// postinstall 换成原生程序；没换（`--ignore-scripts`、没装上平台包）时不算
pub(super) fn is_pe(p: &Path) -> bool {
    let mut head = [0u8; 2];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut head)).is_ok() && &head == b"MZ"
}

/// npx 跑适配器时给 npm 的设置：不查 npm 自己有没有新版本、不打 fund / audit 的提示。这些提示打在标准错误上，
/// 会混进派发日志，挤掉异常退出评论里的「最后 N 个事件」（#459）
pub(super) const NPX_QUIET: &[(&str, &str)] = &[("npm_config_update_notifier", "false"), ("npm_config_fund", "false"), ("npm_config_audit", "false")];

/// `board acp` 要带的环境变量：适配器要的（[`Spec::acp_vars`]）；入口是 npx 时加上 [`NPX_QUIET`]，npm 缓存放在
/// board host 自己的目录里（npm_cache，不受用户 ~/.npm 权限出错的影响；WSL 里用它自己的）。
/// WSL 里这些都经 WSLENV 传进去（BOARD_CONFIG、BOARD_PIDFILE 换成 WSL 路径，不传 BOARD_EXE；这些原样传）
pub(super) fn launch_env(mut vars: Vec<(String, String)>, prog: &str, wsl: bool, npm_cache: &Path) -> Vec<(String, std::ffi::OsString)> {
    let npx = prog == "npx";
    if npx {
        vars.extend(NPX_QUIET.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    }
    let mut out: Vec<(String, std::ffi::OsString)> = vars.iter().map(|(k, v)| (k.clone(), v.into())).collect();
    if wsl {
        let extra: Vec<String> = vars.iter().map(|(k, _)| format!("{k}/u")).collect();
        out.push(("WSLENV".into(), envs::wslenv(std::env::var("WSLENV").ok().as_deref(), &extra).into()));
    } else if npx {
        out.push(("npm_config_cache".into(), npm_cache.into()));
    }
    out
}

/// 内置模板、配置文件里的模板，后面的覆盖前面同标识的；顺序：内置的在前，配置文件里新加的按文件顺序排在后面。
/// 配置文件里的 `cmd` 是 ACP 入口命令。同标识覆盖内置模板时：第一个是 `npx` 或用了 `{bin}`（换适配器的版本、参数）时
/// 照旧找内置的 agent 命令、交给适配器；否则第一个就是要找的命令（自己装的 ACP 入口），Board 不再代装、代升它。
/// 以前的写法（带 `{prompt}` 的一次性命令）覆盖内置模板时不用，照内置的跑；新加的 agent 是这种写法时派发报错说明。
/// 每个 agent 在它支持、本机有的启动环境里各检查一次
pub(super) fn agent_specs(conf: &[AgentConf], envs: &[Env]) -> Result<Vec<Spec>> {
    let strs = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let order = |win: &[&str]| if cfg!(windows) { strs(win) } else { strs(envs::UNIX_ORDER) };
    let mut out: Vec<Template> = BUILTIN
        .iter()
        .map(|b| Template {
            id: b.id.to_string(),
            name: b.name.to_string(),
            builtin: Some(b),
            managed: true,
            model: String::new(),
            bin: b.bin.to_string(),
            acp: strs(b.acp),
            acp_bin_env: b.acp_bin_env.to_string(),
            acp_env: b.acp_env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            acp_mode: b.acp_mode.to_string(),
            legacy: false,
            also: strs(b.also),
            needs: b.needs.iter().map(|(c, w)| (c.to_string(), w.to_string())).collect(),
            skills: strs(b.skills),
            project: strs(b.project),
            order: order(b.win),
        })
        .collect();
    for a in conf {
        check_agent_id(&a.id).map_err(BoardError::usage)?;
        match out.iter_mut().find(|x| x.id == a.id) {
            Some(x) => {
                // 只有以前一次性命令的写法：不用它，照内置的 ACP 入口跑
                if !legacy_cmd(&a.cmd) {
                    x.acp = a.cmd.clone();
                    let first = a.cmd.first().cloned().unwrap_or_default();
                    if first != "npx" && !a.cmd.iter().any(|c| c.contains("{bin}")) {
                        x.bin = first;
                        x.acp_bin_env.clear();
                        x.also.clear();
                        x.needs.clear();
                        x.managed = false;
                    }
                }
                if let Some(n) = &a.name {
                    x.name = n.clone();
                }
                if let Some(m) = &a.model {
                    x.model = m.clone();
                }
            }
            None => out.push(Template {
                id: a.id.clone(),
                name: a.name.clone().unwrap_or_else(|| a.id.clone()),
                builtin: None,
                managed: false,
                model: a.model.clone().unwrap_or_default(),
                bin: a.cmd.first().cloned().unwrap_or_default(),
                acp: a.cmd.clone(),
                acp_bin_env: String::new(),
                acp_env: vec![],
                acp_mode: String::new(),
                legacy: legacy_cmd(&a.cmd),
                also: vec![],
                // 新加的 agent 找的就是入口的第一个命令（是 npx 时找的就是 npx）
                needs: vec![],
                skills: vec![],
                project: vec![],
                order: order(envs::CONF_WIN_ORDER),
            }),
        }
    }
    // 每个 agent 在每个环境里的检查一起跑：一个慢的不拖住启动
    Ok(std::thread::scope(|s| {
        let jobs: Vec<_> = out
            .into_iter()
            .map(|t| {
                let checks: Vec<_> = envs::supported(&t.order, envs)
                    .into_iter()
                    .map(|e| {
                        let (cmd, also, needs) = (t.bin.clone(), t.also.clone(), t.needs.clone());
                        s.spawn(move || envs::probe(e, &cmd, &also).needs(&needs, |c| envs::has(e, c)))
                    })
                    .collect();
                (t, checks)
            })
            .collect();
        jobs.into_iter()
            .map(|(t, checks)| Spec {
                id: t.id,
                name: t.name,
                model: t.model,
                bin: t.bin,
                acp: t.acp,
                acp_bin_env: t.acp_bin_env,
                acp_env: t.acp_env,
                acp_mode: t.acp_mode,
                legacy: t.legacy,
                skills: t.skills,
                project: t.project,
                checks: checks.into_iter().filter_map(|j| j.join().ok()).collect(),
                no_skill: BTreeMap::new(),
                builtin: t.builtin,
                managed: t.managed,
                latest: String::new(),
                checked_at: 0,
                manage_state: String::new(),
                manage_error: String::new(),
            })
            .collect()
    }))
}

/* ------------------------------ 同步配置 ------------------------------ */

/// 每个找到的内置 agent 配置文件里现在的模型和强度（启动时、重新检查后读一次；同步配置之后跟着更新）。
/// 按默认的启动环境找配置文件，WSL 里的读不到
pub(super) fn models_now(agents: &[Spec], envs: &[Env]) -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    for a in agents {
        let Some(c) = a.default_check() else { continue };
        let Some(env) = envs.iter().find(|e| e.id == c.env) else { continue };
        if let Some(now) = providersync::files_of(&a.id, env).and_then(|f| providersync::current(&f)) {
            out.insert(a.id.clone(), now);
        }
    }
    out
}

impl Host<'_> {
    pub(super) fn begin_agent_ask(&mut self, ask: &HostAsk) -> (bool, bool) {
        let Some(id) = ask.agent.as_deref() else { return (false, false) };
        let Some(i) = self.agents.iter().position(|a| a.id == id) else { return (false, false) };
        let install = ask.kind == "agent-install";
        let target = ask.to.clone().unwrap_or_default();
        let name = self.agents[i].name.clone();
        let plan = self.agent_command(&self.agents[i], &target, install).and_then(|plan| {
            if let Some(stage) = &plan.stage {
                stage.reset(cfg!(windows))?;
            }
            prepare_npm_install(&plan.program, &plan.args)?;
            Ok(plan)
        });
        let AgentPlan { env, program, args, bin_dir, shadow_dir, stage } = match plan {
            Ok(x) => x,
            Err(why) => {
                self.agents[i].manage_state = "failed".into();
                self.agents[i].manage_error = why.clone();
                self.ask_error = Some(format!("{} {} 失败：{why}", if install { "安装" } else { "更新" }, name));
                self.asked = Some(ask.id);
                return (true, false);
            }
        };
        self.agents[i].manage_state = "running".into();
        self.agents[i].manage_error.clear();
        self.ask_error = None;
        self.asked = Some(ask.id);
        // 先让服务端清掉请求；界面从 HostAgent.manage.state 看忙碌状态
        match self.beat(false) {
            Ok(w) => self.set_work(w),
            Err(_) => {
                self.agents[i].manage_state.clear();
                self.asked = None;
                return (false, false);
            }
        }
        let _ = std::fs::create_dir_all(&self.logs);
        let log = self.logs.join(format!("agent-{}-{}.log", ask.id, id));
        let started = create_log(&log).and_then(|(out, err)| {
            let mut c = env.command(&program, &args, None, false);
            c.stdin(Stdio::null()).stdout(out).stderr(err);
            envs::detach(&mut c);
            c.spawn()
        });
        match started {
            Ok(child) => {
                let limit = config::File::load()
                    .and_then(|f| f.host())
                    .ok()
                    .and_then(|c| c.agent_job_minutes)
                    .map_or(AGENT_JOB_LIMIT, |m| Duration::from_secs(m as u64 * 60));
                self.say(format!(
                    "{} {}{}：{} {}",
                    self.p.yellow("↑"),
                    if install { "安装" } else { "更新" },
                    name,
                    program,
                    args.join(" ")
                ));
                self.agent_job =
                    Some(AgentJob { ask: ask.id, agent: id.into(), name, target, bin_dir, shadow_dir, stage, child, started: Instant::now(), limit, log });
                (false, false)
            }
            Err(e) => {
                if let Some(stage) = &stage {
                    stage.discard();
                }
                let why = format!("没法运行 {program}：{e}");
                self.agents[i].manage_state = "failed".into();
                self.agents[i].manage_error = why.clone();
                self.ask_error = Some(format!("{} {} 失败：{why}", if install { "安装" } else { "更新" }, self.agents[i].name));
                (true, false)
            }
        }
    }

    /// 实际安装/更新命令：按这个 agent 现在用的启动环境、实际命令路径判断安装方式
    pub(super) fn agent_command(&self, spec: &Spec, target: &str, install: bool) -> std::result::Result<AgentPlan, String> {
        let check = spec.default_check();
        let env_id = self
            .work
            .as_ref()
            .and_then(|w| w.host.agents.iter().find(|a| a.id == spec.id))
            .and_then(|a| a.env.clone())
            .or_else(|| check.map(|c| c.env.clone()));
        let env = env_id
            .as_deref()
            .and_then(|id| self.envs.iter().find(|e| e.id == id))
            .cloned()
            .or_else(|| self.envs.first().cloned())
            .ok_or_else(|| "找不到可用的启动环境".to_string())?;
        if env.wsl().is_some() {
            return Err("WSL 里的 agent 不代装、不代升".into());
        }
        let b = spec.builtin.ok_or_else(|| "配置文件里的 agent 不代装、不代升".to_string())?;
        if !spec.managed {
            return Err("配置文件覆盖了这个 agent 的启动命令：不代装、不代升".into());
        }
        if install {
            let (program, args) = install_plan(b)?;
            let bin_dir = npm_bin_dir(&program, &args).filter(|d| under_home(d));
            let (args, stage) = stage_npm(&program, args, b.package, &spec.id);
            return Ok(AgentPlan { env, program, args, bin_dir, shadow_dir: None, stage });
        }
        let m = spec.manage_for(check);
        match m.method.as_str() {
            "npm-global" => {
                let path = resolved_path(check.and_then(|c| c.path.as_deref().or(c.prog.as_deref())));
                let prefix = npm_global_prefix(&path);
                let shadow_dir = prefix.as_deref().map(|p| Path::new(p).join("bin"));
                let mut update_prefix = prefix;
                #[cfg(unix)]
                if let Some(prefix) = update_prefix.as_deref() {
                    if !writable_dir(&Path::new(prefix).join("lib").join("node_modules")) {
                        let user = expand("~/.local");
                        if !writable_dir(&user.join("lib").join("node_modules")) {
                            return Err(format!(
                                "npm 全局目录 {prefix} 和用户目录 {} 当前用户都不能写：请在有权限的终端里升级",
                                user.display()
                            ));
                        }
                        // 系统目录的旧版本不动；新版本装到用户目录，board host 和新终端都优先用它
                        update_prefix = Some(user.to_string_lossy().into_owned());
                    }
                }
                let args = npm_update_args(update_prefix.as_deref(), b.package, target);
                let bin_dir = npm_bin_dir("npm", &args).filter(|d| under_home(d));
                let (args, stage) = stage_npm("npm", args, b.package, &spec.id);
                Ok(AgentPlan { env, program: "npm".into(), args, bin_dir, shadow_dir, stage })
            }
            "brew" => {
                let path = real_path(check.and_then(|c| c.path.as_deref().or(c.prog.as_deref())));
                let formula = brew_formula(&path).ok_or_else(|| "看不出 Homebrew 的 formula 名".to_string())?;
                Ok(AgentPlan { env, program: "brew".into(), args: vec!["upgrade".into(), formula], bin_dir: None, shadow_dir: None, stage: None })
            }
            "script" => {
                let prog = check.and_then(|c| c.prog.clone()).unwrap_or_else(|| spec.bin.clone());
                Ok(AgentPlan { env, program: prog, args: vec!["update".into()], bin_dir: None, shadow_dir: None, stage: None })
            }
            _ => Err(if m.reason.trim().is_empty() { "这个 agent 不能由 board host 更新".into() } else { m.reason }),
        }
    }

    /// agent 的安装/更新子进程跑完了没有。成功就重新检查版本并清掉忙碌状态；失败把原因留在卡片上，可以重试。
    pub(super) fn poll_agent_job(&mut self) -> bool {
        let Some(job) = self.agent_job.as_mut() else { return false };
        let status = job.child.try_wait().ok().flatten();
        if status.is_none() && job.started.elapsed() < job.limit {
            return false;
        }
        let job = self.agent_job.take().expect("上面取过");
        let mut job = job;
        let tail = tail_lines(&job.log, 20, None);
        let detail = tail.last().cloned().unwrap_or_default();
        let out = match status {
            // 先装进临时 prefix 的：换进真正的 prefix 才算装好
            Some(s) if s.success() => job.stage.as_ref().map_or(Ok(()), |stage| stage.commit(cfg!(windows))),
            Some(s) => Err(agent_job_error(s.code().unwrap_or(-1), &tail)),
            None => {
                kill_tree(job.child.id(), true);
                let _ = job.child.kill();
                let _ = job.child.wait();
                let kept = if job.stage.is_some() { "；旧版本没动" } else { "" };
                Err(format!(
                    "超过 {} 分钟没跑完，已结束{kept}（时限用配置文件 [host] agent_job_minutes 改）",
                    job.limit.as_secs() / 60
                ))
            }
        };
        if out.is_err() {
            if let Some(stage) = &job.stage {
                stage.discard();
            }
        }
        match out {
            Ok(()) => {
                if let Some(dir) = job.bin_dir.as_deref() {
                    if let Some(env) = self.envs.iter().find(|e| e.id == "login") {
                        match ensure_login_path(env, dir, job.shadow_dir.as_deref()) {
                            Ok(true) => self.say(format!(
                                "{} 已把 {} 写进登录 shell 的 PATH（新开终端或 source 后生效）",
                                self.p.green("✓"),
                                dir.display()
                            )),
                            Ok(false) => {}
                            Err(e) => self.say(format!("{} 没能把 {} 写进登录 shell 的 PATH：{e}", self.p.yellow("!"), dir.display())),
                        }
                    }
                }
                self.recheck();
                let now = self.agents.iter().find(|a| a.id == job.agent).and_then(|a| a.default_check()).map(|c| c.version.clone()).unwrap_or_default();
                if !job.target.is_empty() && manage_older(&now, &job.target) {
                    let why = format!("安装/更新命令成功，但版本还是 {}（目标 {}）；可能需要重开终端或检查 PATH", if now.is_empty() { "未知" } else { &now }, job.target);
                    if let Some(a) = self.agents.iter_mut().find(|a| a.id == job.agent) {
                        a.manage_state = "failed".into();
                        a.manage_error = why.clone();
                    }
                    let msg = format!("更新 {} 失败：{why}", job.name);
                    self.say(format!("{} {msg}", self.p.red("✗")));
                    self.ask_error = Some(msg);
                } else {
                    self.say(format!(
                        "{} {} 更新好了{}（请求 #{}，目标 {}，现在是 {}）",
                        self.p.green("✓"),
                        job.name,
                        if detail.is_empty() { String::new() } else { format!("：{detail}") },
                        job.ask,
                        job.target,
                        now
                    ));
                }
            }
            Err(why) => {
                // 失败也重新找一遍命令：不在原处了（被 npm 挪走、装了一半）的不再拿缓存里的路径去派发
                self.recheck();
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == job.agent) {
                    a.manage_state = "failed".into();
                    a.manage_error = why.clone();
                }
                let msg = format!("更新 {} 失败：{why}", job.name);
                self.say(format!("{} {msg}", self.p.red("✗")));
                self.ask_error = Some(msg);
            }
        }
        self.beat_soon = true;
        true
    }

    /// 到点就在后台查内置 agent 的最新版本（每天一次）。网络慢、失败都不影响心跳。
    pub(super) fn poll_agent_latest(&mut self) -> bool {
        if self.agent_latest.as_ref().is_some_and(|j| j.is_finished()) {
            let done = self.agent_latest.take().expect("上面取过").join().unwrap_or_default();
            let at = now_ms();
            for (id, r) in done {
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == id) {
                    a.checked_at = at;
                    if let Ok(v) = r {
                        a.latest = v;
                    }
                }
            }
            return true;
        }
        if self.agent_latest.is_some() || Instant::now() < self.next_agent_check {
            return false;
        }
        let want: Vec<(String, &'static str, &'static str)> = self
            .agents
            .iter()
            .filter_map(|a| a.builtin.filter(|_| a.managed).map(|b| (a.id.clone(), b.package, b.tag)))
            .collect();
        self.next_agent_check = Instant::now() + AGENT_CHECK_EVERY;
        if want.is_empty() {
            return false;
        }
        self.agent_latest = Some(std::thread::spawn(move || {
            want.into_iter().map(|(id, pkg, tag)| (id, fetch_npm_latest(pkg, tag))).collect()
        }));
        false
    }

    /// 这个 agent 找到的命令有不在原处的（软链断了、文件被删）：重新检查一次，马上上报
    pub(super) fn recheck_if_gone(&mut self, agent: &str) {
        let Some(spec) = self.agents.iter().find(|a| a.id == agent) else { return };
        let Some(gone) = spec.checks.iter().find_map(|c| c.path.as_deref().filter(|p| c.found && !c.env.starts_with("wsl:") && path_gone(p))) else {
            return;
        };
        self.say(format!("{} {} 的命令 {gone} 不在了：重新检查本机 agent", self.p.yellow("!"), spec.name));
        self.recheck();
        self.beat_soon = true;
    }

    /// 重新检查（远程请求）：重新找启动环境、本机 agent（找不找得到、版本号）、board-skills。
    /// 配置文件读不了、agent 配置不对时保留原来的 agent
    pub(super) fn recheck(&mut self) {
        let latest: BTreeMap<String, (String, i64)> = self.agents.iter().map(|a| (a.id.clone(), (a.latest.clone(), a.checked_at))).collect();
        self.envs = envs::discover();
        match config::File::load().and_then(|f| f.host()).and_then(|c| agent_specs(&c.agents, &self.envs)) {
            Ok(a) => self.agents = a,
            Err(e) => self.say(format!("{} 重新检查 agent 没成：{}（接着用原来的）", self.p.yellow("!"), e.message)),
        }
        let pairs = ws_pairs(&self.ws, &self.boards, &self.os);
        for a in &mut self.agents {
            a.check_skills(&self.envs, &pairs);
            if let Some((v, at)) = latest.get(&a.id) {
                a.latest = v.clone();
                a.checked_at = *at;
            }
        }
        self.agent_now = models_now(&self.agents, &self.envs);
        // 目录状态也重新看一遍（平时每 5 分钟看一次）：刚把目录修好、克隆好的，点「重新检查」马上就能接卡
        self.dir_checks.clear();
        // 拉取失败的项目目录下一次检查时马上重试，不等 5 分钟
        for x in self.fetch_errs.iter_mut() {
            x.2 = Instant::now() - DIR_CHECK_EVERY;
        }
        let found: Vec<String> = self.agents.iter().filter(|a| a.default_check().is_some()).map(|a| a.id.clone()).collect();
        self.say(format!(
            "{} 重新检查完了：启动环境 {} 个，找到 agent {}",
            self.p.green("✓"),
            self.envs.len(),
            if found.is_empty() { "（没有）".to_string() } else { found.join("、") }
        ));
    }

    /// 重新在每个启动环境里看一次 `board-skills`（桌面端一键安装之后）。返回检查结果有没有变；新看得到的打印一行
    pub(super) fn recheck_skills(&mut self) -> bool {
        let mut changed = false;
        let mut fixed = Vec::new();
        let pairs = ws_pairs(&self.ws, &self.boards, &self.os);
        for a in &mut self.agents {
            let before = a.no_skill.clone();
            a.check_skills(&self.envs, &pairs);
            if before != a.no_skill {
                changed = true;
                if a.no_skill.len() < before.len() {
                    fixed.push(a.id.clone());
                }
            }
        }
        if !fixed.is_empty() {
            self.say(format!("{} {} 看得到 {SKILL} 了", self.p.green("✓"), fixed.join("、")));
        }
        changed
    }
}
