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
        })
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
