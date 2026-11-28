//! `board host` 的启动环境：在哪种 shell 里启动 agent 的命令（启动环境与桌面端需求 §二）。
//!
//! | 标识 | 名称 | 系统 | 怎么发现 | 怎么启动 agent |
//! |---|---|---|---|---|
//! | `direct` | 直接启动 | 所有 | 总有 | 直接执行命令，不经过 shell |
//! | `login` | 登录 shell | macOS、Linux | `$SHELL`，没有时 passwd 里的登录 shell，再没有 `/bin/sh` | `$SHELL -l -i -c 'exec "$0" "$@"' <命令> <参数>…` |
//! | `cmd` | cmd | Windows | `%ComSpec%` | `cmd.exe /d /s /c "<命令> <参数>…"`，每个参数加双引号；命令名是普通名字时不加，否则 `.cmd` 里的 `%~dp0` 会变成当前目录 |
//! | `powershell` | Windows PowerShell | Windows | `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe` | `powershell.exe -NoLogo -NonInteractive -ExecutionPolicy Bypass -Command "& '<命令>' '<参数>' …; exit $LASTEXITCODE"`（`npx`、`npm` 写成 `.cmd`，见 [`ps_prog`]） |
//! | `pwsh` | PowerShell 7 | Windows | `PATH` 里的 `pwsh.exe`，或 `%ProgramFiles%\PowerShell\7\pwsh.exe` | 同上，换成 `pwsh.exe` |
//! | `git-bash` | Git Bash | Windows | `%ProgramFiles%\Git\bin\bash.exe`、`%ProgramFiles(x86)%\Git\bin\bash.exe`，再看 `PATH` 里 `git` 所在目录往上三层的 `bin`（见 [`git_bash_beside`]） | `bash.exe -l -c 'exec "$0" "$@"' <命令> <参数>…` |
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

    /// 按上报时的标识和程序还原一个启动环境（`board acp` 用：board host 把派发用的环境传给它）。
    /// 认不出的标识当直接启动
    pub fn from_parts(id: &str, prog: &str) -> Env {
        let kind = match id {
            "login" => Kind::Login,
            "cmd" => Kind::Cmd,
            "powershell" => Kind::PowerShell,
            "pwsh" => Kind::Pwsh,
            "git-bash" => Kind::GitBash,
            _ => match id.strip_prefix("wsl:") {
                Some(d) if !d.is_empty() => Kind::Wsl(d.to_string()),
                _ => return Env::direct(),
            },
        };
        Env::new(kind, PathBuf::from(prog))
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

/// PowerShell 里要换成 `.cmd` 来执行的命令（Windows）。Node.js 自带的 `npx.ps1`、`npm.ps1` 在 `-Command` 方式下
/// 从 `$MyInvocation.Line`（也就是整行包装脚本）里截自己的参数再 `Invoke-Expression`，截出来的不是它的参数，
/// 直接报错退出（#455）；PowerShell 找命令时同目录的 `.ps1` 优先，所以要写明 `.cmd`
const PS_VIA_CMD: &[&str] = &["npx", "npm"];

/// 在 PowerShell 里执行时写的命令名：[`PS_VIA_CMD`] 里的换成 `<命令>.cmd`，其余原样
pub fn ps_prog(prog: &str) -> String {
    if PS_VIA_CMD.iter().any(|c| c.eq_ignore_ascii_case(prog)) {
        format!("{prog}.cmd")
    } else {
        prog.to_string()
    }
}

/// PowerShell 的包装脚本：`& '<命令>' '<参数>' …; exit $LASTEXITCODE`。输出按 UTF-8，派发时先覆盖连接；
/// 命令名经 [`ps_prog`]（`npx`、`npm` 用 `.cmd`）
pub fn ps_script(prog: &str, args: &[String], run: bool) -> String {
    let mut s = String::new();
    if run {
        s += "try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}; $OutputEncoding = [System.Text.Encoding]::UTF8; ";
        s += "if ($env:BOARD_HOST_URL) { $env:BOARD_URL = $env:BOARD_HOST_URL; $env:BOARD_TOKEN = $env:BOARD_HOST_TOKEN }; ";
    }
    s += "$LASTEXITCODE = $null; & ";
    s += &ps_quote(&ps_prog(prog));
    for a in args {
        s.push(' ');
        s += &ps_quote(a);
    }
    s + "; if ($null -eq $LASTEXITCODE) { exit 1 }; exit $LASTEXITCODE"
}

/// cmd 的命令行：`/d /s /c "<命令> <参数>…"`。参数逐个加双引号；命令名是普通名字时不加，含路径、空格等
/// 字符时才加（/s：去掉最外面一对引号，其余原样）。
///
/// 命令名不能一律加引号：找 `.cmd` 批处理（DeepSeek Harness 装的 `dsh.cmd` 就是）时，加了引号的命令在
/// 脚本里 `%~dp0` 变成当前目录，脚本用 `%~dp0..\..\..\..\DeepSeek Harness.exe` 拼的路径就找错了地方，
/// 退出码 9009（#230）。不加引号的名字由 cmd 自己找，`%~dp0` 是命令真正所在的目录。
pub fn cmd_line(prog: &str, args: &[String]) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('"', ""));
    // 只有「普通名字」（字母、数字、`.`、`-`、`_`）不加引号，其余（路径、空格、括号、`&`）都加
    let plain = !prog.is_empty() && prog.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    let mut parts = vec![if plain { prog.to_string() } else { q(prog) }];
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

/// WSL 里要传的环境变量（WSLENV）：`/u` 只从 Windows 传进 WSL，`/p` 再把路径换成 WSL 路径。不传 BOARD_EXE。
/// extra：另外要传的（`名字/u` 这样写好的，例如 ACP 适配器要的 `CLAUDE_CODE_EXECUTABLE/u`）
pub fn wslenv(old: Option<&str>, extra: &[String]) -> String {
    let mut ours = "BOARD_URL/u:BOARD_TOKEN/u:BOARD_HOST_URL/u:BOARD_HOST_TOKEN/u:BOARD_AS/u:BOARD_HOST/u:BOARD/u:BOARD_CONFIG/pu:BOARD_PIDFILE/pu".to_string();
    let names: Vec<&str> = extra.iter().map(|x| x.split('/').next().unwrap_or_default()).collect();
    for x in extra {
        ours = format!("{ours}:{x}");
    }
    let keep: Vec<&str> = old
        .unwrap_or_default()
        .split(':')
        .filter(|x| !x.is_empty())
        .filter(|x| {
            let name = x.split('/').next().unwrap_or_default();
            !name.starts_with("BOARD") && !names.contains(&name)
        })
        .collect();
    if keep.is_empty() { ours } else { format!("{}:{ours}", keep.join(":")) }
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
/// （[`git_bash_beside`]：`<Git>\bin\git.exe`、`<Git>\cmd\git.exe`、`<Git>\mingw64\bin\git.exe` 三种布局都认）
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
#[derive(Debug, Clone, Default)]
pub struct Check {
    pub env: String,
    /// 能用：找到了 agent 命令，ACP 入口另外要的命令（[`Check::needs`]）也都在
    pub found: bool,
    pub version: String,
    /// 在这个环境里执行用的命令：一般就是模板里的命令名；在「PATH 里没有时再看」的位置找到时是绝对路径
    pub prog: Option<String>,
    /// 实际找到的命令路径：判断安装方式用（cmd 里的 `.cmd` 执行时仍要用命令名，不能用带引号的绝对路径）
    pub path: Option<String>,
    /// 找到了 agent 命令、却缺 ACP 入口另外要的命令时，缺的是什么（例如「Node.js（npx）」）。这时 `found` 是 false（#458）
    pub missing: Option<String>,
}

impl Check {
    pub fn report(&self) -> EnvCheck {
        EnvCheck { env: self.env.clone(), found: self.found, version: self.version.clone(), missing: self.missing.clone(), ..Default::default() }
    }

    /// 找到了 agent 命令时，再在 env 里找 ACP 入口另外要的命令（`needs`：命令，缺了时怎么称呼它）；
    /// 缺一个就把这个环境记成不可用，写明缺什么。`has`：在 env 里找得到这个命令（测试里换成假的）
    pub fn needs(mut self, needs: &[(String, String)], has: impl Fn(&str) -> bool) -> Check {
        if self.found {
            if let Some((_, what)) = needs.iter().find(|(cmd, _)| !has(cmd)) {
                self.found = false;
                self.missing = Some(what.clone());
            }
        }
        self
    }
}

/// 在 env 里找得到 cmd（`command -v` / `where.exe`，不取版本号）：ACP 入口另外要的命令（npx）用
pub fn has(env: &Env, cmd: &str) -> bool {
    locate(env, cmd, &[]).is_some()
}

/// 在 env 里找 cmd（agent 命令），找到了再取版本号。also：PATH 里没有时再看的位置（只在 direct、login 里用）；
/// 第一项是 board host 装进用户目录的位置，PATH 里还有一个旧版本时比较版本号，用较新的那个
pub fn probe(env: &Env, cmd: &str, also: &[String]) -> Check {
    let hit = locate(env, cmd, &[]);
    let local = local_copy(env, also.first());
    let (hit, version) = match (hit, local) {
        (Some((prog, path)), Some(local)) => {
            let old = version_in(env, &prog);
            let new = version_in(env, &local);
            if newer_version(&old, &new) { (Some((local.clone(), local)), new) } else { (Some((prog, path)), old) }
        }
        (Some((prog, path)), None) => {
            let version = version_in(env, &prog);
            (Some((prog, path)), version)
        }
        (None, Some(local)) => {
            let version = version_in(env, &local);
            (Some((local.clone(), local)), version)
        }
        (None, None) => match locate(env, cmd, also) {
            Some((prog, path)) => {
                let version = version_in(env, &prog);
                (Some((prog, path)), version)
            }
            None => (None, String::new()),
        },
    };
    Check { env: env.id.clone(), found: hit.is_some(), version, prog: hit.as_ref().map(|(p, _)| p.clone()), path: hit.map(|(_, path)| path), missing: None }
}

/// PATH 里没有时再看的位置里，第一个存在的用户目录安装（只在 direct、login 里用）
fn local_copy(env: &Env, first: Option<&String>) -> Option<String> {
    if !matches!(env.kind, Kind::Direct | Kind::Login) {
        return None;
    }
    let path = expand(first?);
    is_exe(&path).then(|| path.display().to_string())
}

/// 用户目录里的版本比 PATH 里的新；看不出版本时不换，免得把能用的命令换掉
fn newer_version(old: &str, new: &str) -> bool {
    !new.trim().is_empty() && (old.trim().is_empty() || board_core::hosts::version_lt(old, new))
}

fn locate(env: &Env, cmd: &str, also: &[String]) -> Option<(String, String)> {
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
        Kind::Direct => which(cmd).map(|p| p.display().to_string()).or_else(fallback).map(|path| (path.clone(), path)),
        // 写了绝对路径的 Windows 命令：直接看文件在不在（where.exe 不认路径）
        Kind::Cmd | Kind::PowerShell | Kind::Pwsh if has_dir => is_exe(&p).then(|| p.display().to_string()).map(|path| (path.clone(), path)),
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
            match hit {
                Some(path) => {
                    let path = if env.kind == Kind::GitBash { git_bash_path(&path).unwrap_or(path) } else { path };
                    Some((arg, path))
                }
                None => fallback().map(|path| (path.clone(), path)),
            }
        }
    }
}

/// Git Bash 的 `command -v` 给出的是 MSYS 路径（`/c/...`，Cygwin 下也有 `/cygdrive/c/...`）：Windows 的 `Path`
/// 不认它，要换成盘符路径。`/usr/bin/...` 这类 MSYS 根里的路径不能按盘符猜，原样返回 None
fn git_bash_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/cygdrive/").or_else(|| path.strip_prefix('/'))?;
    let (drive, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let mut chars = drive.chars();
    let drive = chars.next()?;
    if chars.next().is_some() || !drive.is_ascii_alphabetic() {
        return None;
    }
    Some(if tail.is_empty() {
        format!("{}:\\", drive.to_ascii_uppercase())
    } else {
        format!("{}:\\{}", drive.to_ascii_uppercase(), tail.replace('/', "\\"))
    })
}

/// 在 env 里 `<prog> --version` 的第一行（跳过 shell 自己的提示）
fn version_in(env: &Env, prog: &str) -> String {
    match run_out(env.command(prog, &["--version".to_string()], None, false), env.limit()) {
        Some(o) => first_line(&o.stdout).or_else(|| first_line(&o.stderr)).unwrap_or_default(),
        None => String::new(),
    }
}

fn first_line(t: &str) -> Option<String> {
    // 登录 shell 自己打的提示（见 board_core::hosts::shell_noise）；`zsh: ` 也跳过：读 `.zshrc` 时的告警不是版本号
    let noise = |l: &str| board_core::hosts::shell_noise(l) || l.starts_with("zsh: ");
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

/// WSL 里的一个用户级目录在 Windows 上怎么访问：在 WSL 里展开（`~`、`$CODEX_HOME`），换成 `\\wsl.localhost\<发行版>\…`。
/// 取不到时为 None
pub fn wsl_unc(env: &Env, dir: &str) -> Option<PathBuf> {
    let d = env.wsl()?;
    let mut c = Command::new(&env.prog);
    c.args(["-d", d, "--exec", "sh", "-c", &format!("printf %s \"{}\"", wsl_dir(dir))]);
    let out = run_out(c, env.limit())?;
    let p = out.stdout.trim();
    if !out.ok || !p.starts_with('/') {
        return None;
    }
    Some(PathBuf::from(format!("\\\\wsl.localhost\\{d}{}", p.replace('/', "\\"))))
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

/// Windows 上从 PATH 里的 git 推出 Git Bash 的目录（`<Git>\bin`）：Git 不在 `%ProgramFiles%` 下（例如装在 D 盘）时靠它找 Git Bash
fn git_bash_dir() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    git_bash_beside(&which("git")?)
}

/// `git.exe` 所在的 Git 安装里带 `bash.exe` 的目录。
///
/// Git for Windows 把 `git.exe` 放出来有三种布局：`<Git>\bin`、`<Git>\cmd`、`<Git>\mingw64\bin`，
/// PATH 里的 `git` 是哪一种取决于安装时勾了什么，`where git` 常常先得到 `<Git>\mingw64\bin\git.exe`。
/// 从 `git.exe` 的目录往上找三层：`<Git>\bin\bash.exe`（Git for Windows 自己的包装）优先，
/// 没有时用 `<Git>\usr\bin\bash.exe`（MSYS2 的原身）。
pub fn git_bash_beside(git: &Path) -> Option<PathBuf> {
    let dir = git.parent()?;
    // `<Git>\bin\git.exe`：git.exe 自己就在 `bin` 里
    if dir.join("bash.exe").is_file() {
        return Some(dir.to_path_buf());
    }
    // `<Git>\cmd\git.exe`、`<Git>\mingw64\bin\git.exe`：往上找到 `<Git>`，再用它下面的 `bin`
    let mut usr = None;
    for d in dir.ancestors().take(3) {
        let bin = d.join("bin");
        if bin.join("bash.exe").is_file() {
            return Some(bin);
        }
        if usr.is_none() {
            let u = d.join("usr").join("bin");
            if u.join("bash.exe").is_file() {
                usr = Some(u);
            }
        }
    }
    usr
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
        let c = login.command("claude", &s(&["-p", PROMPT]), None, true);
        assert_eq!(c.get_program(), "/bin/zsh");
        let a = args(&c);
        assert_eq!(&a[..3], &["-l", "-i", "-c"]);
        assert!(a[3].ends_with(r#"exec "$0" "$@""#) && a[3].contains("BOARD_HOST_URL"), "{}", a[3]);
        assert_eq!(&a[4..], &["claude", "-p", PROMPT], "提示词是独立参数");
        // 检查版本号时不覆盖连接
        assert_eq!(args(&login.command("claude", &s(&["--version"]), None, false))[3], r#"exec "$0" "$@""#);

        let wsl = Env::new(Kind::Wsl("Ubuntu".into()), PathBuf::from(r"C:\Windows\System32\wsl.exe"));
        assert_eq!((wsl.id.as_str(), wsl.name.as_str()), ("wsl:Ubuntu", "WSL（Ubuntu）"));
        let a = args(&wsl.command("claude", &s(&["-p", PROMPT]), Some(Path::new(r"D:\repo\board")), true));
        assert_eq!(&a[..9], &["-d", "Ubuntu", "--cd", r"D:\repo\board", "--exec", "setsid", "-w", "bash", "-l"]);
        assert!(a[11].starts_with(r#"[ -n "$BOARD_PIDFILE" ] && echo $$"#), "{}", a[11]);
        assert_eq!(&a[12..], &["claude", "-p", PROMPT]);
        assert_eq!(wsl.report().path, "Ubuntu", "WSL 写发行版名");

        let git = Env::new(Kind::GitBash, PathBuf::from(r"D:\Git\bin\bash.exe"));
        let a = args(&git.command("claude", &s(&["-p", PROMPT]), None, true));
        assert_eq!(&a[..2], &["-l", "-c"]);
        assert_eq!(&a[3..], &["claude", "-p", PROMPT]);

        let direct = Env::direct();
        let c = direct.command("/usr/local/bin/claude", &s(&["-p", PROMPT]), None, true);
        assert_eq!(c.get_program(), "/usr/local/bin/claude");
        assert_eq!(args(&c), &["-p", PROMPT]);
    }

    #[test]
    fn powershell_and_cmd_quoting() {
        let script = ps_script("claude", &s(&["-p", "it's"]), false);
        assert!(script.contains("& 'claude' '-p' 'it''s'; "), "{script}");
        assert!(script.ends_with("exit $LASTEXITCODE"), "退出码就是 agent 的");
        assert!(ps_script("claude", &[], true).contains("$env:BOARD_URL = $env:BOARD_HOST_URL"));
        assert_eq!(cmd_line("dsh", &s(&["--profile", "headless", PROMPT])), format!("/d /s /c \"dsh \"--profile\" \"headless\" \"{PROMPT}\"\""));
        // 命令名是普通名字时不加引号：加了引号的 .cmd 拿到的 %~dp0 是当前目录，脚本里拼的相对路径会找错
        assert_eq!(cmd_line(r"C:\Program Files\dsh.cmd", &s(&["--version"])), r#"/d /s /c ""C:\Program Files\dsh.cmd" "--version"""#);
        assert_eq!(cmd_line("dsh", &[]), r#"/d /s /c "dsh""#);
        // 普通名字以外的一律加引号（空格、路径、cmd 的语法字符）
        assert_eq!(cmd_line(r"\tools\a&b.cmd", &[]), r#"/d /s /c ""\tools\a&b.cmd"""#);
    }

    /// PowerShell 里 npx、npm 写成 `.cmd`：Node.js 自带的 `.ps1` 从整行包装脚本里截参数，截错了直接报错（#455）
    #[test]
    fn powershell_runs_npx_and_npm_as_cmd() {
        let script = ps_script("npx", &s(&["-y", "@agentclientprotocol/codex-acp@2.7.0"]), true);
        assert!(script.contains("& 'npx.cmd' '-y' '@agentclientprotocol/codex-acp@2.7.0'; "), "{script}");
        assert!(ps_script("npm", &s(&["install", "-g", "@openai/codex@latest"]), false).contains("& 'npm.cmd' 'install' "));
        assert_eq!((ps_prog("NPX").as_str(), ps_prog("claude").as_str(), ps_prog("npx.cmd").as_str()), ("NPX.cmd", "claude", "npx.cmd"));
        // 写了路径的不动：那是人自己指定的
        assert_eq!(ps_prog(r"C:\tools\npx"), r"C:\tools\npx");
        // 别的启动环境不经过它：cmd 按 PATHEXT 找（不认 .ps1），sh 类执行 npm 的 sh 垫片
        assert_eq!(cmd_line("npx", &[]), r#"/d /s /c "npx""#);
    }

    /// 同一目录里有会报错的 `npx.ps1` 和正常的 `npx.cmd`（Node.js 装好后就是这样）：两种 PowerShell 都执行到 `.cmd`（#455）
    #[cfg(windows)]
    #[test]
    fn powershell_skips_npx_ps1() {
        let dir = std::env::temp_dir().join(format!("board-ps-npx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("npx.ps1"), "Write-Output 'ran npx.ps1'\r\nexit 7\r\n").unwrap();
        std::fs::write(dir.join("npx.cmd"), "@echo off\r\necho ran npx.cmd [%1] [%2]\r\n").unwrap();
        let old = std::env::var_os("PATH").unwrap_or_default();
        let mut parts: Vec<std::ffi::OsString> = vec![dir.clone().into_os_string()];
        parts.extend(std::env::split_paths(&old).map(|p| p.into_os_string()));
        let path = std::env::join_paths(parts).unwrap();
        let windows_ps = std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
            .filter(|p| p.is_file());
        let shells: Vec<(Kind, PathBuf)> =
            [which("pwsh").map(|p| (Kind::Pwsh, p)), windows_ps.map(|p| (Kind::PowerShell, p))].into_iter().flatten().collect();
        assert!(!shells.is_empty(), "这台 Windows 上没有 PowerShell");
        for (kind, prog) in shells {
            let mut c = Env::new(kind.clone(), prog).command("npx", &s(&["-y", "pkg@1.0.0"]), None, true);
            c.env("PATH", &path);
            let out = run_out(c, Duration::from_secs(60)).expect("PowerShell 能跑");
            assert!(out.ok, "{kind:?}：{}{}", out.stdout, out.stderr);
            assert!(out.stdout.contains("ran npx.cmd [-y] [pkg@1.0.0]") && !out.stdout.contains("npx.ps1"), "{kind:?}：{}", out.stdout);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `.cmd` 批处理靠 `%~dp0` 找自己旁边的文件：命令名不加引号时 `%~dp0` 才是命令真正所在的目录，
    /// 加了引号会变成当前目录，脚本里拼的相对路径就找错了（DeepSeek Harness 的 `dsh.cmd` 因此退出码 9009，#230）
    #[cfg(windows)]
    #[test]
    fn cmd_runs_dot_cmd_from_path_with_its_own_dir() {
        let dir = std::env::temp_dir().join(format!("board-cmd-dp0-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(dir.join("data.txt"), "found\r\n").unwrap();
        // 和 dsh.cmd 一样用 %~dp0 拼上级目录里的文件
        let script = "@echo off\r\ntype \"%~dp0..\\data.txt\"\r\necho one=[%1]\r\n";
        std::fs::write(bin.join("board-no-such-agent.cmd"), script).unwrap();
        let prog = std::env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("cmd.exe"));
        let mut c = Env::new(Kind::Cmd, prog).command("board-no-such-agent", &s(&["a b"]), None, false);
        let old = std::env::var_os("PATH").unwrap_or_default();
        let mut parts: Vec<std::ffi::OsString> = vec![bin.clone().into_os_string()];
        parts.extend(std::env::split_paths(&old).map(|p| p.into_os_string()));
        c.env("PATH", std::env::join_paths(parts).unwrap());
        let out = run_out(c, Duration::from_secs(20)).expect("cmd 能跑");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(out.ok, "{}{}", out.stdout, out.stderr);
        assert!(out.stdout.contains("found"), "%~dp0 不是命令所在的目录：{}", out.stdout);
        assert!(out.stdout.contains("one=[\"a b\"]"), "参数仍然逐个加引号：{}", out.stdout);
    }

    #[test]
    fn wslenv_passes_board_vars_but_not_exe() {
        let w = wslenv(Some("FOO/p:BOARD_EXE"), &[]);
        assert!(w.starts_with("FOO/p:"), "{w}");
        assert!(w.contains("BOARD_CONFIG/pu") && w.contains("BOARD_AS/u") && w.contains("BOARD_PIDFILE/pu"));
        assert!(!w.contains("BOARD_EXE"), "WSL 里用 skill 自带的 Linux 版 board");
        assert!(!wslenv(None, &[]).starts_with(':'));
        // ACP 适配器要的变量也传进去；原来 WSLENV 里同名的不再重复
        let w = wslenv(Some("CODEX_PATH/p:FOO/p"), &["CODEX_PATH/u".to_string()]);
        assert!(w.ends_with(":CODEX_PATH/u") && w.starts_with("FOO/p:") && w.matches("CODEX_PATH").count() == 1, "{w}");
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
        let order = s(&["git-bash", "wsl:*", "pwsh", "powershell"]);
        let ids: Vec<&str> = supported(&order, &envs).iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["git-bash", "wsl:Ubuntu", "wsl:Debian", "pwsh"], "不在顺序里的（cmd）不选；本机没有的跳过");
    }

    /// Git Bash 报的 MSYS 盘符路径在 Windows 上换成原生路径；MSYS 根里的 `/usr/...` 不能猜成盘符
    #[test]
    fn git_bash_paths_become_windows_paths() {
        assert_eq!(
            git_bash_path("/c/Users/me/AppData/Roaming/npm/codex").as_deref(),
            Some(r"C:\Users\me\AppData\Roaming\npm\codex")
        );
        assert_eq!(
            git_bash_path("/cygdrive/d/Program Files/npm/codex.cmd").as_deref(),
            Some(r"D:\Program Files\npm\codex.cmd")
        );
        assert_eq!(git_bash_path("/c").as_deref(), Some(r"C:\"));
        assert_eq!(git_bash_path("/usr/bin/codex"), None);
        assert_eq!(git_bash_path(r"C:\Users\me\codex"), None);
    }

    /// 真实 Git Bash 里探测一个 Windows 程序：`locate` 要把 `/c/...` 换成盘符路径，
    /// 后面找 npm 包里的 `.exe`、判断安装位置都才拿得到文件
    #[cfg(windows)]
    #[test]
    fn git_bash_probe_reports_a_windows_path() {
        let Some(git) = discover().into_iter().find(|e| e.kind == Kind::GitBash) else { return };
        let c = probe(&git, "where.exe", &[]);
        assert!(c.found, "Git Bash 里应找得到 where.exe");
        let path = c.path.expect("探测结果要带路径");
        assert!(!path.starts_with('/'), "还是 MSYS 路径：{path}");
        assert!(Path::new(&path).is_file(), "不是 Windows 上能直接打开的文件：{path}");
    }

    #[test]
    fn version_skips_shell_noise() {
        let out = "bash: cannot set terminal process group (-1): Inappropriate ioctl for device\nbash: no job control in this shell\n1.5.0\n";
        assert_eq!(first_line(out).as_deref(), Some("1.5.0"));
        // 发行版的 bashrc 打的 sudo 提示也算噪声（#232）：版本号不能显示成它
        let out = "To run a command as administrator (user \"root\"), use \"sudo <command>\".\nSee \"man sudo_root\" for details.\n\n9.9.9 (fake)\n";
        assert_eq!(first_line(out).as_deref(), Some("9.9.9 (fake)"));
    }

    /// PATH 里是系统装的旧版、用户目录里有 board host 刚升级的新版时用新版；看不出版本时不换
    #[test]
    fn a_newer_user_copy_replaces_the_old_path_command() {
        assert!(newer_version("2.1.323 (Claude Code)", "2.1.325"));
        assert!(!newer_version("2.1.325", "2.1.323"));
        assert!(!newer_version("2.1.325", "2.1.325"));
        assert!(newer_version("", "1.2.3"), "PATH 里的版本读不到时用能读到的用户目录版本");
        assert!(!newer_version("1.2.3", ""), "用户目录里的版本读不到时不换掉能用的命令");
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
        assert_eq!(c.prog.as_deref(), Some("sh"), "执行仍用命令名");
        let found = c.path.expect("安装方式判断要拿到实际路径");
        assert!(found.starts_with('/') && Path::new(&found).is_file(), "登录 shell 也要报实际路径：{found}");
        assert!(!probe(&sh, "board-no-such-agent", &[]).found);
        // PATH 里没有时再看的位置
        let c = probe(&sh, "board-no-such-agent", &["/bin/sh".into()]);
        assert_eq!((c.found, c.prog.as_deref()), (true, Some("/bin/sh")));
        // 退出码就是命令的退出码，参数原样传进去
        let o = run_out(sh.command("sh", &s(&["-c", "printf %s \"$1\"; exit 7", "x", PROMPT]), None, true), Duration::from_secs(10)).unwrap();
        assert!(!o.ok);
        assert_eq!(o.stdout, PROMPT);
    }

    /// Windows 进程树回收（`proctree.rs`）：假 board acp 由 PowerShell 启动，再起一个不等它的孙进程后自己退出。
    /// 名字以 `powershell_` 开头，现有的 Windows CI 步骤（`envs::tests::powershell_`）会一起跑。
    #[cfg(windows)]
    fn powershell_job_running(pid: u32) -> bool {
        let pid = sysinfo::Pid::from_u32(pid);
        let Ok(mut sys) = sysinfo::System::new() else { return false };
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        sys.process(pid).is_some_and(|p| {
            !matches!(
                p.status(),
                sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
            )
        })
    }

    #[cfg(windows)]
    fn powershell_job_wait_until(what: &str, mut ok: impl FnMut() -> bool) {
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while !ok() {
            assert!(std::time::Instant::now() < until, "10 秒内没有等到：{what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// 起一个“假 board acp”：稍等 Job 绑定，再起一个不等待它的孙进程，写下孙进程号后自己退出
    #[cfg(windows)]
    fn powershell_job_grandchild(dir: &Path) -> (crate::proctree::Job, std::process::Child, PathBuf) {
        let pidfile = dir.join("grandchild.pid");
        let script = dir.join("fake-acp.ps1");
        let path = pidfile.display().to_string().replace('\'', "''");
        std::fs::write(
            &script,
            format!(
                r#"Start-Sleep -Milliseconds 1000
$ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$grand = Start-Process -FilePath $ps -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60' -PassThru
Set-Content -LiteralPath '{path}' -Value $grand.Id
"#
            ),
        )
        .unwrap();
        let child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let job = crate::proctree::Job::attach(&child).unwrap();
        (job, child, pidfile)
    }

    #[cfg(windows)]
    fn powershell_job_grandchild_pid(
        (job, mut parent, pidfile): (crate::proctree::Job, std::process::Child, PathBuf),
    ) -> (crate::proctree::Job, u32) {
        assert!(parent.wait().unwrap().success(), "假 board acp 没正常退出");
        powershell_job_wait_until("孙进程写下进程号", || pidfile.is_file());
        let pid = std::fs::read_to_string(pidfile)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        powershell_job_wait_until("孙进程还活着", || powershell_job_running(pid));
        assert!(powershell_job_running(pid), "父进程退出后，孙进程仍在 Job 里");
        (job, pid)
    }

    #[cfg(windows)]
    #[test]
    fn powershell_job_object_kills_detached_grandchild() {
        let dir = tempfile::tempdir().unwrap();
        let (job, pid) = powershell_job_grandchild_pid(powershell_job_grandchild(dir.path()));
        job.terminate(true);
        powershell_job_wait_until("TerminateJobObject 结束孙进程", || !powershell_job_running(pid));
    }

    #[cfg(windows)]
    #[test]
    fn powershell_job_object_close_kills_detached_grandchild() {
        let dir = tempfile::tempdir().unwrap();
        let (job, pid) = powershell_job_grandchild_pid(powershell_job_grandchild(dir.path()));
        drop(job);
        powershell_job_wait_until("关闭 Job 句柄后系统结束孙进程", || {
            !powershell_job_running(pid)
        });
    }
}
