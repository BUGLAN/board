//! 命令行的配置文件 `~/.board/config.toml`（Windows：`%USERPROFILE%\.board\config.toml`，PRD §9）。
//!
//! - `url`（服务端地址）和 `token`（访问令牌），由 `board login` 写入、`board logout` 删掉令牌。
//! - `[host]`（机器名、白板与目录）和 `[agents.<标识>]`（agent 命令模板），给 `board host` 用；
//!   桌面端「设置 → 本机」也写这两节。agent 名字（`BOARD_AS`）和白板（`BOARD`）不写进这个文件。
//! - 读取顺序：环境变量 `BOARD_URL` / `BOARD_TOKEN` 优先，配置文件其次（两项分别看）。
//! - 环境变量 `BOARD_CONFIG` 指定另一个配置文件代替 `~/.board/config.toml`：board host 启动 agent 时用它传连接——
//!   有的 agent（例如 dsh）不把名字像密钥的环境变量（带 TOKEN、KEY 等）传给它执行的命令，`BOARD_TOKEN` 到不了 `board`。
//! - 读写在 `board_core::config`（和桌面端共用）：`~/.board` 目录 0700，文件 0600；改写时保留其他配置项和注释，同时写不会互相覆盖。

use board_core::Result;

// 读写配置文件只有 board-core 里的一份（桌面端也用它）：加锁读改写、原子写回、权限
pub use board_core::config::{board_dir, shown, update, AgentConf, Config as File, TOKEN, URL};

/// 一项配置从哪里来
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 环境变量 BOARD_URL / BOARD_TOKEN
    Env,
    /// ~/.board/config.toml
    File,
}

impl Source {
    /// JSON 里用的标识
    pub fn key(self) -> &'static str {
        match self {
            Source::Env => "env",
            Source::File => "file",
        }
    }
}

/// 环境变量（去掉首尾空白，空的算没有）
pub fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// 连接服务端用的地址和令牌，以及各自从哪里来
#[derive(Debug, Clone, Default)]
pub struct Conn {
    pub url: Option<(String, Source)>,
    pub token: Option<(String, Source)>,
    /// 这次读连接时从派发的配置文件（`BOARD_CONFIG` 的 `[agent_env]`）补了哪些 `BOARD_*`（见 [`from_agent_env`]）。
    /// 补上的变量就和环境变量一样参与下面的读取，所以地址、令牌可能正是从这里来的
    pub filled: Vec<String>,
}

/// 派发的配置文件（`BOARD_CONFIG`）里有没有 `[agent_env]`：有就把其中的 `BOARD_*` 设进本进程的环境。
///
/// board host 启动 agent 时把连接和名字放在它的环境变量里，但 agent 会过滤传给命令的环境变量（Codex 只在 shell 的
/// 环境策略放行的那几个里传，`BOARD_*` 不在里面；dsh 去掉名字里带 `TOKEN` 的），agent 执行 `board` 时可能看不到。
/// 所以 board host 把它们也写进这次派发的配置文件的 `[agent_env]`，`scripts/board`（带 `BOARD_AGENT_ENV=1`）和
/// `board` 自己都从这里补上。已经在环境里的不覆盖。返回补上了哪几个（没补的、或没有这一节时为空）
pub fn from_agent_env() -> Vec<String> {
    let Some(path) = env("BOARD_CONFIG") else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    apply_agent_env(&text)
}

/// [`from_agent_env`] 的实体：把配置文本里 `[agent_env]` 的 `BOARD_*` 设进环境（有的就不动），返回设了哪几个
fn apply_agent_env(text: &str) -> Vec<String> {
    let mut filled = Vec::new();
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else { return filled };
    let Some(t) = doc.get("agent_env").and_then(|t| t.as_table_like()) else { return filled };
    for (k, v) in t.iter() {
        if !k.starts_with("BOARD_") {
            continue;
        }
        let Some(s) = v.as_str() else { continue };
        if env(k).is_some() {
            continue;
        }
        std::env::set_var(k, s);
        filled.push(k.to_string());
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 派发的配置文件里 `[agent_env]` 的 `BOARD_*` 补进环境，返回补了哪几个；环境里已经有的不覆盖、也不算补过
    #[test]
    fn agent_env_section_fills_the_environment() {
        // 名字带用例自己的后缀：几个用例在同一个进程里跑，别互相抢变量
        let text = "[agent_env]\nBOARD_AS = \"codex-win/codex-test\"\nBOARD = \"oss/main\"\nNOT_BOARD = \"x\"\n";
        std::env::remove_var("BOARD_AS");
        std::env::set_var("BOARD", "already/here");
        assert_eq!(apply_agent_env(text), ["BOARD_AS"]);
        assert_eq!(std::env::var("BOARD_AS").ok().as_deref(), Some("codex-win/codex-test"));
        assert_eq!(std::env::var("BOARD").ok().as_deref(), Some("already/here"), "环境里已经有的不覆盖");
        assert!(std::env::var("NOT_BOARD").is_err(), "只补 BOARD_ 开头的");
        // 同样的文本再来一次：环境里都有了，什么也不补
        assert!(apply_agent_env(text).is_empty());
        std::env::remove_var("BOARD_AS");
        std::env::remove_var("BOARD");
        // 没有这一节：什么也不做
        assert!(apply_agent_env("url = \"http://127.0.0.1:1\"\n").is_empty());
    }

    /// `board help` 的「数据」一行：从派发配置补上的地址说清楚来自哪，没补过时照旧写环境变量或配置文件
    #[test]
    fn describe_says_where_the_url_came_from() {
        let url = "http://127.0.0.1:7443";
        assert_eq!(describe(url, Source::Env, &[]), format!("服务端 {url}（地址来自环境变量 BOARD_URL）"));
        assert_eq!(describe(url, Source::File, &[]), format!("服务端 {url}（地址来自{}）", shown()));
        assert_eq!(
            describe(url, Source::Env, &["BOARD_URL".to_string()]),
            format!("服务端 {url}（从派发的配置 {} 的 [agent_env] 补过 BOARD_URL）", shown())
        );
        // 补过的是别的变量：地址还是来自环境变量或配置文件
        assert_eq!(
            describe(url, Source::Env, &["BOARD_AS".to_string(), "BOARD".to_string()]),
            format!("服务端 {url}（地址来自环境变量 BOARD_URL）")
        );
    }
}

impl Conn {
    /// 环境变量优先，配置文件其次。两项环境变量都有时不读配置文件
    pub fn resolve() -> Result<Conn> {
        let (url_env, token_env) = (env("BOARD_URL"), env("BOARD_TOKEN"));
        let file = if url_env.is_some() && token_env.is_some() { None } else { Some(File::load()?) };
        let from_file = |k: &str| -> Result<Option<(String, Source)>> {
            Ok(match &file {
                Some(f) => f.get(k)?.map(|v| (v, Source::File)),
                None => None,
            })
        };
        Ok(Conn {
            url: match url_env {
                Some(u) => Some((u, Source::Env)),
                None => from_file(URL)?,
            },
            token: match token_env {
                Some(t) => Some((t, Source::Env)),
                None => from_file(TOKEN)?,
            },
            filled: Vec::new(),
        })
    }

    /// 从派发的配置文件补上 `BOARD_*` 之后读连接（[`crate::backend::Backend`] 用这个，见 [`from_agent_env`]）
    pub fn resolve_for_agent() -> Result<Conn> {
        let filled = from_agent_env();
        let mut c = Conn::resolve()?;
        c.filled = filled;
        Ok(c)
    }
}

/// `board help` 里「数据」一行的说法：地址和它从哪来。`filled` 是从派发配置补上的变量（[`Conn::filled`]），
/// 地址正是从那里补来的时说清楚来自派发配置，而不是只说「环境变量」（补上之后它确实也在环境里，看不出是补的）。
pub fn describe(url: &str, src: Source, filled: &[String]) -> String {
    if filled.iter().any(|k| k == "BOARD_URL") {
        format!("服务端 {url}（从派发的配置 {} 的 [agent_env] 补过 BOARD_URL）", shown())
    } else {
        format!("服务端 {url}（地址来自{}）", origin(src, "BOARD_URL"))
    }
}

/// 「来自哪里」的说法
pub fn origin(src: Source, var: &str) -> String {
    match src {
        Source::Env => format!("环境变量 {var}"),
        Source::File => shown().to_string(),
    }
}

/// 本机配置的机器名（`~/.board/config.toml` 的 `[host] name`，桌面端「设置 → 本机」或 `board host --name` 写的）：
/// `board check --agent <标识>` 拿它告诉服务端是哪台机器，名字由服务端按这台机器的 agent 设置生成（PRD §3.2）。
/// 没写、配置文件读不了时为空（服务端再按令牌认机器）
pub fn host_name() -> Option<String> {
    env("BOARD_HOST_NAME").or_else(|| File::load().ok()?.host().ok()?.name)
}
