//! `board host` 的启动环境：在哪种 shell 里启动 agent 的命令（启动环境与桌面端需求 §二）。
//!
//! | 标识 | 名称 | 系统 | 怎么发现 | 怎么启动 agent |
//! |---|---|---|---|---|
//! | `direct` | 直接启动 | 所有 | 总有 | 直接执行命令模板，不经过 shell |
//! | `login` | 登录 shell | macOS、Linux | `$SHELL`，没有时 passwd 里的登录 shell，再没有 `/bin/sh` | `$SHELL -l -i -c 'exec "$0" "$@"' <命令> <参数>…` |
//! | `cmd` | cmd | Windows | `%ComSpec%` | `cmd.exe /d /s /c "<命令> <参数>…"`，每个参数加双引号 |
//! | `powershell` | Windows PowerShell | Windows | `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe` | `powershell.exe -NoLogo -NonInteractive -ExecutionPolicy Bypass -Command "& '<命令>' '<参数>' …; exit $LASTEXITCODE"` |
//! | `pwsh` | PowerShell 7 | Windows | `PATH` 里的 `pwsh.exe`，或 `%ProgramFiles%\PowerShell\7\pwsh.exe` | 同上，换成 `pwsh.exe` |
//! | `git-bash` | Git Bash | Windows | `%ProgramFiles%\Git\bin\bash.exe`、`%ProgramFiles(x86)%\Git\bin\bash.exe`，再看 `PATH` 里的 git 旁边 | `bash.exe -l -c 'exec "$0" "$@"' <命令> <参数>…` |
//! | `wsl:<发行版>` | WSL（<发行版>） | Windows | `wsl.exe -l -q` 列出的每个发行版 | `wsl.exe -d <发行版> --cd <目录> --exec setsid -w bash -l -i -c 'exec "$0" "$@"' <命令> <参数>…` |
//!
//! - 登录 shell 和 WSL 加 `-i`：很多人把 `PATH` 写在 `~/.zshrc`、`~/.bashrc` 里，只有交互式 shell 才读。
//!   为了不让交互式 shell 去抢终端，agent 进程（和检查用的进程）都 `setsid` 脱离终端。
//! - 提示词作为独立参数传入，不拼进 shell 命令文本（cmd 除外：加双引号拼进命令行，提示词里没有需要转义的字符）。
//! - 退出码就是 agent 的退出码：sh 类用 `exec`，PowerShell 用 `exit $LASTEXITCODE`。
//! - shell 会先读用户的启动脚本：脚本里设了 `BOARD_URL`、`BOARD_TOKEN` 也不影响，启动 agent 前用 board host 的值
//!   （`BOARD_HOST_URL`、`BOARD_HOST_TOKEN`）再设一遍。
//! - WSL：环境变量经 `WSLENV` 传入，`BOARD_CONFIG` 换成 WSL 路径，不传 `BOARD_EXE`（WSL 里用 skill 自带的 Linux 版
//!   `board`）；agent 的进程号写进 `BOARD_PIDFILE`，停止时在 WSL 里结束它的整个进程组（[`wsl_kill`]）。
//! - 找命令（[`probe`]）：sh 类 `command -v`（WSL 里不算 `/mnt/` 下的 Windows 程序），cmd、PowerShell 用 `where.exe`；
//!   找到了再执行 `<命令> --version` 取第一行。内置模板「PATH 里没有时再看」的位置只在 `direct`、`login` 里用。

use board_core::{EnvCheck, HostEnv};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// 一种启动环境
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Direct,
    Login,
    Cmd,
    PowerShell,
    Pwsh,
    GitBash,
    /// 发行版名
    Wsl(String),
}

/// 本机的一个启动环境
#[derive(Debug, Clone)]
pub struct Env {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// 程序：登录 shell、cmd.exe、powershell.exe、pwsh.exe、bash.exe、wsl.exe；直接启动时为空
    pub prog: PathBuf,
}

/// 内置 agent 在 macOS、Linux 上支持的启动环境（按默认选择的顺序）
pub const UNIX_ORDER: &[&str] = &["login", "direct"];
/// 配置文件里新加的 agent 在 Windows 上支持的启动环境
pub const CONF_WIN_ORDER: &[&str] = &["pwsh", "powershell", "git-bash", "wsl:*", "cmd", "direct"];

impl Env {
    pub fn direct() -> Env {
        Env { id: "direct".into(), name: "直接启动".into(), kind: Kind::Direct, prog: PathBuf::new() }
    }

    fn new(kind: Kind, prog: PathBuf) -> Env {
        let (id, name) = match &kind {
            Kind::Direct => ("direct".to_string(), "直接启动".to_string()),
            Kind::Login => ("login".into(), "登录 shell".into()),
            Kind::Cmd => ("cmd".into(), "cmd".into()),
            Kind::PowerShell => ("powershell".into(), "Windows PowerShell".into()),
            Kind::Pwsh => ("pwsh".into(), "PowerShell 7".into()),
            Kind::GitBash => ("git-bash".into(), "Git Bash".into()),
            Kind::Wsl(d) => (format!("wsl:{d}"), format!("WSL（{d}）")),
        };
        Env { id, name, kind, prog }
    }

    /// 上报给服务端：标识、名称、程序路径（WSL 写发行版名，直接启动为空）
    pub fn report(&self) -> HostEnv {
        let path = match &self.kind {
            Kind::Direct => String::new(),
            Kind::Wsl(d) => d.clone(),
            _ => self.prog.display().to_string(),
        };
        HostEnv { id: self.id.clone(), name: self.name.clone(), path }
    }

    pub fn wsl(&self) -> Option<&str> {
        match &self.kind {
            Kind::Wsl(d) => Some(d),
            _ => None,
        }
    }

    /// 登录 shell 是 fish：脚本要用 fish 的写法（参数在 `$argv` 里）
    fn fish(&self) -> bool {
        self.kind == Kind::Login && self.prog.file_name().is_some_and(|n| n.to_string_lossy().starts_with("fish"))
    }

    /// 在这个环境里执行 `prog args`。run 为真时是派发：启动前再设一遍连接（WSL 还记下进程号、脱离成自己的进程组），
    /// dir 是工作目录（WSL 用 `--cd` 传 Windows 目录，由 wsl.exe 换成 `/mnt/…`）
    pub fn command(&self, prog: &str, args: &[String], dir: Option<&Path>, run: bool) -> Command {
        let mut c = match &self.kind {
            Kind::Direct => {
                let mut c = Command::new(prog);
                c.args(args);
                c
            }
            Kind::Login | Kind::GitBash => {
                let mut c = Command::new(&self.prog);
                c.arg("-l");
                if self.kind == Kind::Login {
                    c.arg("-i");
                }
                if self.fish() {
                    c.arg("-c").arg(fish_script(run)).arg(prog).args(args);
                } else {
                    c.arg("-c").arg(sh_script(run, false)).arg(prog).args(args);
                }
                c
            }
            Kind::Wsl(d) => {
                let mut c = Command::new(&self.prog);
                c.args(["-d", d]);
                if let Some(dir) = dir {
                    c.arg("--cd").arg(dir);
                }
                c.arg("--exec");
                if run {
                    c.args(["setsid", "-w"]);
                }
                c.args(["bash", "-l", "-i", "-c"]).arg(sh_script(run, true)).arg(prog).args(args);
                c
            }
            Kind::Cmd => {
                let mut c = Command::new(&self.prog);
                raw(&mut c, &cmd_line(prog, args));
                c
            }
            Kind::PowerShell | Kind::Pwsh => {
                let mut c = Command::new(&self.prog);
                c.args(["-NoLogo", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"]).arg(ps_script(prog, args, run));
                c
            }
        };
        if let Some(dir) = dir {
            c.current_dir(dir);
        }
        c
    }

    /// 在这个环境里找命令的那条命令：sh 类 `command -v`，cmd、PowerShell 用 `where.exe`
    fn locate(&self, cmd: &str) -> Command {
        match &self.kind {
            Kind::Direct => Command::new(cmd),
            Kind::Login | Kind::GitBash | Kind::Wsl(_) => {
                let mut c = Command::new(&self.prog);
                if let Kind::Wsl(d) = &self.kind {
                    c.args(["-d", d, "--exec", "bash", "-l", "-i"]);
                } else {
                    c.arg("-l");
                    if self.kind == Kind::Login {
                        c.arg("-i");
                    }
                }
                let script = if self.fish() { "command -v $argv[1]" } else { r#"command -v "$0""# };
                c.arg("-c").arg(script).arg(cmd);
                c
            }
            Kind::Cmd => {
                let mut c = Command::new(&self.prog);
                raw(&mut c, &cmd_line("where", &[cmd.to_string()]));
                c
            }
            Kind::PowerShell | Kind::Pwsh => {
                let mut c = Command::new(&self.prog);
                // 和启动 agent 时一样读用户的 profile（profile 里可能改了 PATH）
                c.args(["-NoLogo", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"])
                    .arg(format!("where.exe {}; exit $LASTEXITCODE", ps_quote(cmd)));
                c
            }
        }
    }

    /// 检查用的命令多久没结果算失败：WSL 第一次要启动发行版，慢一些
    fn limit(&self) -> Duration {
        match self.kind {
            Kind::Wsl(_) => Duration::from_secs(30),
            _ => Duration::from_secs(10),
        }
    }
}

/* ------------------------------ 脚本 ------------------------------ */

/// sh 类的包装脚本：`exec "$0" "$@"`；派发时先用 board host 的连接覆盖启动脚本里可能设的，WSL 还要写进程号
pub fn sh_script(run: bool, wsl: bool) -> String {
    let mut s = String::new();
    if run && wsl {
        s += r#"[ -n "$BOARD_PIDFILE" ] && echo $$ > "$BOARD_PIDFILE"; "#;
    }
    if run {
        s += r#"if [ -n "$BOARD_HOST_URL" ]; then export BOARD_URL="$BOARD_HOST_URL" BOARD_TOKEN="$BOARD_HOST_TOKEN"; fi; "#;
    }
    s + r#"exec "$0" "$@""#
}

fn fish_script(run: bool) -> String {
    let mut s = String::new();
    if run {
        s += "if set -q BOARD_HOST_URL; set -gx BOARD_URL $BOARD_HOST_URL; set -gx BOARD_TOKEN $BOARD_HOST_TOKEN; end; ";
    }
    s + "exec $argv"
}

/// PowerShell 的单引号字符串：`'` 写成 `''`
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// PowerShell 的包装脚本：`& '<命令>' '<参数>' …; exit $LASTEXITCODE`。输出按 UTF-8，派发时先覆盖连接
pub fn ps_script(prog: &str, args: &[String], run: bool) -> String {
    let mut s = String::new();
    if run {
        s += "try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}; $OutputEncoding = [System.Text.Encoding]::UTF8; ";
        s += "if ($env:BOARD_HOST_URL) { $env:BOARD_URL = $env:BOARD_HOST_URL; $env:BOARD_TOKEN = $env:BOARD_HOST_TOKEN }; ";
    }
    s += "$LASTEXITCODE = $null; & ";
    s += &ps_quote(prog);
    for a in args {
        s.push(' ');
        s += &ps_quote(a);
    }
    s + "; if ($null -eq $LASTEXITCODE) { exit 1 }; exit $LASTEXITCODE"
}

/// cmd 的命令行：`/d /s /c "<命令> <参数>…"`，每个参数加双引号（/s：去掉最外面一对引号，其余原样）
pub fn cmd_line(prog: &str, args: &[String]) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('"', ""));
    let mut parts = vec![q(prog)];
    parts.extend(args.iter().map(|a| q(a)));
    format!("/d /s /c \"{}\"", parts.join(" "))
}

/// Windows 上原样写进命令行（cmd 自己解析引号，不能用 Rust 的转义）
fn raw(c: &mut Command, line: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.raw_arg(line);
    }
    #[cfg(not(windows))]
    {
        c.arg(line);
    }
}

/// WSL 里要传的环境变量（WSLENV）：`/u` 只从 Windows 传进 WSL，`/p` 再把路径换成 WSL 路径。不传 BOARD_EXE
pub fn wslenv(old: Option<&str>) -> String {
    let ours = "BOARD_URL/u:BOARD_TOKEN/u:BOARD_HOST_URL/u:BOARD_HOST_TOKEN/u:BOARD_AS/u:BOARD_HOST/u:BOARD/u:BOARD_CONFIG/pu:BOARD_PIDFILE/pu";
    let keep: Vec<&str> = old
        .unwrap_or_default()
        .split(':')
        .filter(|x| !x.is_empty())
        .filter(|x| {
            let name = x.split('/').next().unwrap_or_default();
            !name.starts_with("BOARD")
        })
        .collect();
    if keep.is_empty() { ours.to_string() } else { format!("{}:{ours}", keep.join(":")) }
}

/// 在 bash 里结束一个进程组：`bash -c KILL_GROUP <组长进程号> <TERM|KILL>`；组已经没了时退回只结束这个进程
const KILL_GROUP: &str = r#"kill -s "$1" -- "-$0" 2>/dev/null || kill -s "$1" -- "$0" 2>/dev/null"#;

/// 结束 WSL 里 agent 的整个进程组（pid 是写进 BOARD_PIDFILE 的进程号，setsid 之后它就是组长）
pub fn wsl_kill(env: &Env, pid: u32, hard: bool) {
    let Some(d) = env.wsl() else { return };
    let sig = if hard { "KILL" } else { "TERM" };
    let mut c = Command::new(&env.prog);
    c.args(["-d", d, "--exec", "bash", "-c", KILL_GROUP]).arg(pid.to_string()).arg(sig);
    let _ = run_out(c, Duration::from_secs(15));
}

/* ------------------------------ 发现 ------------------------------ */

/// 本机的启动环境：Windows 上 cmd、Windows PowerShell、PowerShell 7、Git Bash、各个 WSL 发行版，其他系统上登录 shell；
/// 最后都有直接启动
pub fn discover() -> Vec<Env> {
    let mut out = vec![];
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let cmd = std::env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| root.join("System32").join("cmd.exe"));
        if cmd.is_file() {
            out.push(Env::new(Kind::Cmd, cmd));
        }
        let ps = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        if ps.is_file() {
            out.push(Env::new(Kind::PowerShell, ps));
        }
        let pwsh = which("pwsh").or_else(|| {
            std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join(r"PowerShell\7\pwsh.exe")).filter(|p| p.is_file())
        });
        if let Some(p) = pwsh {
            out.push(Env::new(Kind::Pwsh, p));
        }
        if let Some(b) = git_bash() {
            out.push(Env::new(Kind::GitBash, b));
        }
        let wsl = root.join("System32").join("wsl.exe");
        if wsl.is_file() {
            for d in wsl_distros(&wsl) {
                out.push(Env::new(Kind::Wsl(d), wsl.clone()));
            }
        }
    } else {
        out.push(Env::new(Kind::Login, login_shell()));
    }
    out.push(Env::direct());
    out
}

/// 登录 shell：`$SHELL`，没有时 passwd 里的，再没有 `/bin/sh`
fn login_shell() -> PathBuf {
    if let Some(s) = std::env::var_os("SHELL").map(PathBuf::from).filter(|p| p.is_absolute() && is_exe(p)) {
        return s;
    }
    #[cfg(unix)]
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if !pw.is_null() && !(*pw).pw_shell.is_null() {
            let s = std::ffi::CStr::from_ptr((*pw).pw_shell).to_string_lossy().into_owned();
            let p = PathBuf::from(s);
            if p.is_absolute() && is_exe(&p) {
                return p;
            }
        }
    }
    PathBuf::from("/bin/sh")
}

/// Git Bash：`%ProgramFiles%\Git\bin\bash.exe`、`%ProgramFiles(x86)%\Git\bin\bash.exe`，再看 PATH 里 git 旁边的
fn git_bash() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|k| std::env::var_os(k))
        .map(|p| PathBuf::from(p).join("Git").join("bin").join("bash.exe"))
        .find(|p| p.is_file())
        .or_else(|| git_bash_dir().map(|d| d.join("bash.exe")))
}

/// `wsl.exe -l -q` 列出的发行版（输出是 UTF-16LE；设了 WSL_UTF8=1 时是 UTF-8）
fn wsl_distros(wsl: &Path) -> Vec<String> {
    let mut c = Command::new(wsl);
    c.args(["-l", "-q"]).env("WSL_UTF8", "1");
    match run_out(c, Duration::from_secs(15)) {
        Some(o) if o.ok => parse_distros(&o.raw),
        _ => vec![],
    }
}

pub fn parse_distros(raw: &[u8]) -> Vec<String> {
    let utf16 = raw.len() >= 2 && raw.iter().skip(1).step_by(2).filter(|b| **b == 0).count() * 2 >= raw.len() / 2;
    let text = if utf16 {
        let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// 一个 agent 支持、本机有的启动环境（按 order 的顺序，`wsl:*` 按发现的顺序展开）
pub fn supported<'e>(order: &[String], envs: &'e [Env]) -> Vec<&'e Env> {
    let mut out: Vec<&Env> = vec![];
    for p in order {
        for e in envs {
            let hit = if p == "wsl:*" { e.wsl().is_some() } else { e.id == *p };
            if hit && !out.iter().any(|x| x.id == e.id) {
                out.push(e);
            }
        }
    }
    out
}

/* ------------------------------ 找命令 ------------------------------ */

/// 一个 agent 在一个启动环境里的检查结果
#[derive(Debug, Clone)]
pub struct Check {
    pub env: String,
    pub found: bool,
    pub version: String,
    /// 在这个环境里执行的命令：一般就是模板里的命令名（由环境去找）；在「PATH 里没有时再看」的位置找到时是那个绝对路径
    pub prog: Option<String>,
}

impl Check {
    pub fn report(&self) -> EnvCheck {
        EnvCheck { env: self.env.clone(), found: self.found, version: self.version.clone() }
    }
}

/// 在 env 里找 cmd（命令模板的第一个元素），找到了再取版本号。also：PATH 里没有时再看的位置（只在 direct、login 里用）
pub fn probe(env: &Env, cmd: &str, also: &[String]) -> Check {
    let prog = locate(env, cmd, also);
    let version = prog.as_deref().map(|p| version_in(env, p)).unwrap_or_default();
    Check { env: env.id.clone(), found: prog.is_some(), version, prog }
}

fn locate(env: &Env, cmd: &str, also: &[String]) -> Option<String> {
    let fallback = || {
        if matches!(env.kind, Kind::Direct | Kind::Login) {
            also.iter().map(|p| expand(p)).find(|p| is_exe(p)).map(|p| p.display().to_string())
        } else {
            None
        }
    };
    let p = expand(cmd);
    let has_dir = p.components().count() > 1;
    match &env.kind {
        Kind::Direct => which(cmd).map(|p| p.display().to_string()).or_else(fallback),
        // 写了绝对路径的 Windows 命令：直接看文件在不在（where.exe 不认路径）
        Kind::Cmd | Kind::PowerShell | Kind::Pwsh if has_dir => is_exe(&p).then(|| p.display().to_string()),
        Kind::Wsl(_) if has_dir && !cmd.starts_with('/') => None,
        _ => {
            let arg = if has_dir { p.display().to_string() } else { cmd.to_string() };
            let o = run_out(env.locate(&arg), env.limit())?;
            let hit = if !o.ok {
                None
            } else if matches!(env.kind, Kind::Cmd | Kind::PowerShell | Kind::Pwsh) {
                o.stdout.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
            } else {
                // sh 类：最后一行是路径（启动脚本可能先打印别的）；别名、函数不算；WSL 里 /mnt/ 下的是 Windows 程序，不算
                o.stdout
                    .lines()
                    .map(str::trim)
                    .rfind(|l| !l.is_empty())
                    .filter(|l| l.starts_with('/') && !(env.wsl().is_some() && l.starts_with("/mnt/")))
                    .map(str::to_string)
            };
            hit.map(|_| arg).or_else(fallback)
        }
    }
}

/// 在 env 里 `<prog> --version` 的第一行（跳过 shell 自己的提示）
fn version_in(env: &Env, prog: &str) -> String {
    match run_out(env.command(prog, &["--version".to_string()], None, false), env.limit()) {
        Some(o) => first_line(&o.stdout).or_else(|| first_line(&o.stderr)).unwrap_or_default(),
        None => String::new(),
    }
}

fn first_line(t: &str) -> Option<String> {
    let noise = |l: &str| {
        l.starts_with("bash: cannot set terminal process group")
            || l.starts_with("bash: no job control")
            || l.starts_with("zsh: ")
            || l.starts_with("stty: ")
    };
    t.lines().map(str::trim).find(|l| !l.is_empty() && !noise(l)).map(|l| l.chars().take(60).collect())
}

/// 看 skill 装没装：dirs 里有没有 `<skill>/SKILL.md`。WSL 里看 WSL 用户目录下的（`~` 是 WSL 的用户目录，
/// `$CODEX_HOME` 在 WSL 里取，默认 `~/.codex`）；其他环境看本机的
pub fn has_skill(env: &Env, dirs: &[String], skill: &str) -> bool {
    match env.wsl() {
        None => dirs.iter().any(|d| expand(d).join(skill).join("SKILL.md").is_file()),
        Some(d) => {
            if dirs.is_empty() {
                return false;
            }
            let list: Vec<String> = dirs.iter().map(|x| format!("\"{}\"", wsl_dir(x))).collect();
            let script = format!(r#"for d in {}; do [ -f "$d/{skill}/SKILL.md" ] && exit 0; done; exit 1"#, list.join(" "));
            let mut c = Command::new(&env.prog);
            c.args(["-d", d, "--exec", "sh", "-c", &script]);
            run_out(c, env.limit()).is_some_and(|o| o.ok)
        }
    }
}

/// 内置模板里的 skill 目录写成 WSL 里 sh 的写法：`~/x` → `$HOME/x`，`$CODEX_HOME/x` → `${CODEX_HOME:-$HOME/.codex}/x`
fn wsl_dir(d: &str) -> String {
    if let Some(rest) = d.strip_prefix("~/") {
        format!("$HOME/{rest}")
    } else if let Some(rest) = d.strip_prefix("$CODEX_HOME") {
        format!("${{CODEX_HOME:-$HOME/.codex}}{rest}")
    } else {
        d.to_string()
    }
}

/* ------------------------------ 进程 ------------------------------ */

/// 一次检查命令的结果
pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub raw: Vec<u8>,
}

/// 执行一条检查用的命令：标准输入为空，脱离终端；limit 内没结束就结束它（连同子进程），算失败
pub fn run_out(mut c: Command, limit: Duration) -> Option<Out> {
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    detach(&mut c);
    let child = c.spawn().ok()?;
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(o)) => Some(Out {
            ok: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            raw: o.stdout,
        }),
        Ok(Err(_)) => None,
        Err(_) => {
            kill_tree(pid, true);
            None
        }
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 不弹控制台窗口
#[cfg(windows)]
fn no_window(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    c.creation_flags(CREATE_NO_WINDOW);
}

/// 脱离终端、自成一组：终端里的 Ctrl+C 不直接打到它身上，交互式 shell 也不会去抢终端；停止时整组结束。
/// Unix 上 setsid（进程号就是组号），Windows 上新的进程组、不弹窗口
pub fn detach(c: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        c.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
}

/// 结束进程树：先 SIGTERM（hard 时 SIGKILL）整个进程组；Windows 直接结束进程树
pub fn kill_tree(pid: u32, hard: bool) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), if hard { libc::SIGKILL } else { libc::SIGTERM });
    }
    #[cfg(windows)]
    {
        let _ = hard;
        let mut c = Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        no_window(&mut c);
        let _ = c.status();
    }
}

/* ------------------------------ 找程序 ------------------------------ */

/// 在 PATH 里找命令（Windows 上按 PATHEXT 补扩展名）；写了路径的直接看文件在不在
pub fn which(cmd: &str) -> Option<PathBuf> {
    let p = expand(cmd);
    if p.components().count() > 1 {
        return is_exe(&p).then_some(p);
    }
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_lowercase())
            .collect()
    } else {
        vec![]
    };
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        if cfg!(windows) {
            for e in &exts {
                let f = dir.join(format!("{cmd}{e}"));
                if f.is_file() {
                    return Some(f);
                }
            }
        } else {
            let f = dir.join(cmd);
            if is_exe(&f) {
                return Some(f);
            }
        }
    }
    None
}

/// Windows 上从 PATH 里的 git 推出 Git Bash 的目录（`<Git>\bin`）。
/// pi 只认 `%ProgramFiles%\Git\bin\bash.exe`，Git 装在 D 盘这类位置时会回落到 PATH 里的
/// `bash.exe`——那常常是 `C:\Windows\System32\bash.exe`（WSL），WSL 没启用时 pi 的命令都跑不了。
pub fn git_bash_dir() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    git_bash_beside(&which("git")?)
}

/// git.exe 的上级或上上级里的 `bin\bash.exe`（`<Git>\bin\git.exe` 或 `<Git>\cmd\git.exe` 两种安装布局）
pub fn git_bash_beside(git: &Path) -> Option<PathBuf> {
    let dir = git.parent()?;
    let mut candidates = vec![dir.to_path_buf()];
    if let Some(root) = dir.parent() {
        candidates.push(root.join("bin"));
    }
    candidates.into_iter().find(|d| d.join("bash.exe").is_file())
}

pub fn is_exe(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// `~` 开头的路径换成用户目录；`$CODEX_HOME` 开头的换成这个环境变量（没设时 `~/.codex`）
pub fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("$CODEX_HOME") {
        let base = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| expand("~/.codex"));
        return base.join(rest.trim_start_matches(['/', '\\']));
    }
    board_core::config::expand(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn args(c: &Command) -> Vec<String> {
        c.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    const PROMPT: &str = "/board-skills 开始执行 default项目之中的main白板中的#3任务";

    #[test]
    fn wraps_commands_per_env() {
        let login = Env::new(Kind::Login, PathBuf::from("/bin/zsh"));
        let c = login.command("pi", &s(&["-p", PROMPT]), None, true);
        assert_eq!(c.get_program(), "/bin/zsh");
        let a = args(&c);
        assert_eq!(&a[..3], &["-l", "-i", "-c"]);
        assert!(a[3].ends_with(r#"exec "$0" "$@""#) && a[3].contains("BOARD_HOST_URL"), "{}", a[3]);
        assert_eq!(&a[4..], &["pi", "-p", PROMPT], "提示词是独立参数");
        // 检查版本号时不覆盖连接
        assert_eq!(args(&login.command("pi", &s(&["--version"]), None, false))[3], r#"exec "$0" "$@""#);

        let wsl = Env::new(Kind::Wsl("Ubuntu".into()), PathBuf::from(r"C:\Windows\System32\wsl.exe"));
        assert_eq!((wsl.id.as_str(), wsl.name.as_str()), ("wsl:Ubuntu", "WSL（Ubuntu）"));
        let a = args(&wsl.command("pi", &s(&["-p", PROMPT]), Some(Path::new(r"D:\repo\board")), true));
        assert_eq!(&a[..9], &["-d", "Ubuntu", "--cd", r"D:\repo\board", "--exec", "setsid", "-w", "bash", "-l"]);
        assert!(a[11].starts_with(r#"[ -n "$BOARD_PIDFILE" ] && echo $$"#), "{}", a[11]);
        assert_eq!(&a[12..], &["pi", "-p", PROMPT]);
        assert_eq!(wsl.report().path, "Ubuntu", "WSL 写发行版名");

        let git = Env::new(Kind::GitBash, PathBuf::from(r"D:\Git\bin\bash.exe"));
        let a = args(&git.command("pi", &s(&["-p", PROMPT]), None, true));
        assert_eq!(&a[..2], &["-l", "-c"]);
        assert_eq!(&a[3..], &["pi", "-p", PROMPT]);

        let direct = Env::direct();
        let c = direct.command("/usr/local/bin/pi", &s(&["-p", PROMPT]), None, true);
        assert_eq!(c.get_program(), "/usr/local/bin/pi");
        assert_eq!(args(&c), &["-p", PROMPT]);
    }

    #[test]
    fn powershell_and_cmd_quoting() {
        let script = ps_script("pi", &s(&["-p", "it's"]), false);
        assert!(script.contains("& 'pi' '-p' 'it''s'; "), "{script}");
        assert!(script.ends_with("exit $LASTEXITCODE"), "退出码就是 agent 的");
        assert!(ps_script("pi", &[], true).contains("$env:BOARD_URL = $env:BOARD_HOST_URL"));
        assert_eq!(cmd_line("dsh", &s(&["--profile", "headless", PROMPT])), format!("/d /s /c \"\"dsh\" \"--profile\" \"headless\" \"{PROMPT}\"\""));
    }

    #[test]
    fn wslenv_passes_board_vars_but_not_exe() {
        let w = wslenv(Some("FOO/p:BOARD_EXE"));
        assert!(w.starts_with("FOO/p:"), "{w}");
        assert!(w.contains("BOARD_CONFIG/pu") && w.contains("BOARD_AS/u") && w.contains("BOARD_PIDFILE/pu"));
        assert!(!w.contains("BOARD_EXE"), "WSL 里用 skill 自带的 Linux 版 board");
        assert!(!wslenv(None).starts_with(':'));
    }

    #[test]
    fn parses_wsl_list_in_utf16() {
        let text = "\u{feff}Ubuntu\r\ndocker-desktop\r\n";
        let raw: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(parse_distros(&raw), vec!["Ubuntu", "docker-desktop"]);
        assert_eq!(parse_distros(b"Ubuntu-24.04\n"), vec!["Ubuntu-24.04"]);
    }

    #[test]
    fn supported_keeps_order_and_expands_wsl() {
        let envs = vec![
            Env::new(Kind::Cmd, PathBuf::from("cmd.exe")),
            Env::new(Kind::Pwsh, PathBuf::from("pwsh.exe")),
            Env::new(Kind::GitBash, PathBuf::from("bash.exe")),
            Env::new(Kind::Wsl("Ubuntu".into()), PathBuf::from("wsl.exe")),
            Env::new(Kind::Wsl("Debian".into()), PathBuf::from("wsl.exe")),
            Env::direct(),
        ];
        let pi = s(&["git-bash", "wsl:*", "pwsh", "powershell"]);
        let ids: Vec<&str> = supported(&pi, &envs).iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["git-bash", "wsl:Ubuntu", "wsl:Debian", "pwsh"], "pi 不支持 cmd；本机没有的跳过");
    }

    #[test]
    fn version_skips_shell_noise() {
        let out = "bash: cannot set terminal process group (-1): Inappropriate ioctl for device\nbash: no job control in this shell\n1.5.0\n";
        assert_eq!(first_line(out).as_deref(), Some("1.5.0"));
    }

    /// WSL 里停止用的脚本：结束整个进程组（这里在本机的 bash 上试）
    #[cfg(unix)]
    #[test]
    fn kill_group_script_ends_the_group() {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", "sleep 30 & sleep 30; wait"]).stdin(Stdio::null());
        detach(&mut c);
        let mut child = c.spawn().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let o = run_out(
            {
                let mut k = Command::new("bash");
                k.args(["-c", KILL_GROUP]).arg(child.id().to_string()).arg("TERM");
                k
            },
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(o.ok);
        let begun = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(begun.elapsed() < Duration::from_secs(5), "进程组没结束");
            std::thread::sleep(Duration::from_millis(50));
        }
        // 组里的另一个 sleep 也结束了：它的父进程先退出，由系统接管、回收，要等一会儿才从进程组里消失
        let begun = std::time::Instant::now();
        while unsafe { libc::kill(-(child.id() as i32), 0) } == 0 {
            assert!(begun.elapsed() < Duration::from_secs(5), "进程组里还有进程");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(unix)]
    #[test]
    fn login_shell_finds_and_runs_commands() {
        let sh = Env::new(Kind::Login, PathBuf::from("/bin/sh"));
        let c = probe(&sh, "sh", &[]);
        assert!(c.found, "登录 shell 里找得到 sh");
        assert_eq!(c.prog.as_deref(), Some("sh"));
        assert!(!probe(&sh, "board-no-such-agent", &[]).found);
        // PATH 里没有时再看的位置
        let c = probe(&sh, "board-no-such-agent", &["/bin/sh".into()]);
        assert_eq!((c.found, c.prog.as_deref()), (true, Some("/bin/sh")));
        // 退出码就是命令的退出码，参数原样传进去
        let o = run_out(sh.command("sh", &s(&["-c", "printf %s \"$1\"; exit 7", "x", PROMPT]), None, true), Duration::from_secs(10)).unwrap();
        assert!(!o.ok);
        assert_eq!(o.stdout, PROMPT);
    }
}
