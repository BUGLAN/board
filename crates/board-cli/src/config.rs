//! 命令行的配置文件 `~/.board/config.toml`（Windows：`%USERPROFILE%\.board\config.toml`，PRD §9）。
//!
//! - `url`（服务端地址）和 `token`（访问令牌），由 `board login` 写入、`board logout` 删掉令牌。
//! - `[host]`（机器名、白板与目录）和 `[agents.<标识>]`（agent 命令模板），给 `board host` 用；
//!   桌面端「设置 → 本机」也写这两节。agent 名字（`BOARD_AS`）和白板（`BOARD`）不写进这个文件。
//! - 读取顺序：环境变量 `BOARD_URL` / `BOARD_TOKEN` 优先，配置文件其次（两项分别看）。
//! - 目录权限 0700，文件权限 0600；改写时保留其他配置项和注释。

use board_core::{BoardError, Result};
use std::path::PathBuf;
use toml_edit::{value, DocumentMut};

/// 配置文件里的项
pub const URL: &str = "url";
pub const TOKEN: &str = "token";

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

/// 配置文件的位置（给人看的写法）
pub fn shown() -> &'static str {
    if cfg!(windows) {
        r"%USERPROFILE%\.board\config.toml"
    } else {
        "~/.board/config.toml"
    }
}

/// 用户目录：Windows 上是 USERPROFILE，其他系统是 HOME（测试把它指向临时目录）
fn home() -> Option<PathBuf> {
    let keys: [&str; 2] = if cfg!(windows) { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
    keys.iter().find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
}

pub fn path() -> Result<PathBuf> {
    home()
        .map(|h| h.join(".board").join("config.toml"))
        .ok_or_else(|| BoardError::usage("找不到用户目录（HOME / USERPROFILE），没法读写配置文件"))
}

/// 读进来的配置文件；文件不存在时是空的
pub struct File {
    path: PathBuf,
    doc: DocumentMut,
}

impl File {
    pub fn load() -> Result<File> {
        let path = path()?;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(BoardError::usage(format!("读不了 {}：{e}", path.display()))),
        };
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| BoardError::usage(format!("{} 不是合法的 TOML：{}", path.display(), e.to_string().trim())))?;
        Ok(File { path, doc })
    }

    /// 一项字符串配置；没有或为空时是 None
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        match self.doc.get(key) {
            None => Ok(None),
            Some(item) => match item.as_str() {
                Some(s) => Ok(Some(s.trim().to_string()).filter(|s| !s.is_empty())),
                None => Err(BoardError::usage(format!("{} 里的 {key} 应该是字符串", self.path.display()))),
            },
        }
    }

    pub fn set(&mut self, key: &str, v: &str) {
        self.doc[key] = value(v);
    }

    /// 删掉一项，返回原来有没有
    pub fn remove(&mut self, key: &str) -> bool {
        self.doc.remove(key).is_some()
    }

    /// 写回：目录 0700、文件 0600。先写临时文件再改名，写到一半不会留下残缺的配置
    pub fn save(&self) -> Result<()> {
        let fail = |e: std::io::Error| BoardError::usage(format!("写入 {} 失败：{e}", self.path.display()));
        let dir = self.path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&dir).map_err(fail)?;
        let tmp = dir.join(".config.toml.tmp");
        {
            use std::io::Write;
            let mut o = std::fs::OpenOptions::new();
            o.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            let mut f = o.open(&tmp).map_err(fail)?;
            f.write_all(self.doc.to_string().as_bytes()).map_err(fail)?;
            f.sync_all().map_err(fail)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(fail)?;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(fail)?;
        }
        std::fs::rename(&tmp, &self.path).map_err(fail)
    }
}

/// `[host]` 与 `[agents.*]`：
///
/// ```toml
/// [host]
/// name = "macbook"
/// dirs = { "default/main" = "~/repo/board", "mini/main" = "~/repo/mini" }
///
/// [agents.claude]
/// cmd = ["claude", "-p", "{prompt}", "--dangerously-skip-permissions"]
/// ```
#[derive(Debug, Clone, Default)]
pub struct HostConf {
    pub name: Option<String>,
    /// 白板与目录，按文件里的顺序
    pub dirs: Vec<(String, String)>,
    pub agents: Vec<AgentConf>,
}

#[derive(Debug, Clone)]
pub struct AgentConf {
    pub id: String,
    /// 名称（界面上显示）；不写时用标识
    pub name: Option<String>,
    pub cmd: Vec<String>,
}

impl File {
    pub fn host(&self) -> Result<HostConf> {
        let bad = |what: &str| BoardError::usage(format!("{} 里的 {what} 格式不对", self.path.display()));
        let mut c = HostConf::default();
        if let Some(h) = self.doc.get("host") {
            let t = h.as_table_like().ok_or_else(|| bad("[host]"))?;
            if let Some(n) = t.get("name") {
                c.name = Some(n.as_str().ok_or_else(|| bad("host.name"))?.trim().to_string()).filter(|s| !s.is_empty());
            }
            if let Some(d) = t.get("dirs") {
                let d = d.as_table_like().ok_or_else(|| bad("host.dirs"))?;
                for (k, v) in d.iter() {
                    c.dirs.push((k.to_string(), v.as_str().ok_or_else(|| bad("host.dirs"))?.to_string()));
                }
            }
        }
        if let Some(a) = self.doc.get("agents") {
            let t = a.as_table_like().ok_or_else(|| bad("[agents]"))?;
            for (id, v) in t.iter() {
                let at = v.as_table_like().ok_or_else(|| bad(&format!("[agents.{id}]")))?;
                let cmd = at
                    .get("cmd")
                    .and_then(|x| x.as_array())
                    .ok_or_else(|| bad(&format!("agents.{id}.cmd（要写成字符串数组）")))?
                    .iter()
                    .map(|x| x.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| bad(&format!("agents.{id}.cmd（要写成字符串数组）")))?;
                let name = at.get("name").and_then(|x| x.as_str()).map(str::to_string);
                c.agents.push(AgentConf { id: id.to_string(), name, cmd });
            }
        }
        Ok(c)
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
