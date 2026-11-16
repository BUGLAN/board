//! Git 凭据共用配置（PRD §10.16「共用配置」）：服务端存一份 https 远端的令牌，各台机器的 board host 按设置
//! 配进 Board 管的仓库，board host 备代码、agent 推集成分支时直接用。
//!
//! - 一份：主机（`github.com` 这样的主机名，可以带端口）、用户名、令牌，以及可选的提交身份（`user.name` / `user.email`）。
//!   只管 https 远端；SSH 地址（`git@…`）不管。
//! - 令牌只存在服务端：列表接口只给末尾 4 位（[`GitCred::masked`]），完整的值只经 board host 的令牌接口
//!   （`GET /api/hosts/{名字}/git`，只给这台机器上报时用的令牌）下发。
//! - 版本号是全部内容（含令牌）的哈希前 12 位：令牌改了也会重新同步，哈希不泄露原文。
//! - 每台机器选「用服务器的 / 不改」（机器设置的 `syncGit`，新机器默认不改：它只改 Board 管的仓库的本地 git 配置，
//!   不碰 agent 的配置文件，也不碰用户的全局 git 配置，所以由人在界面上按机器打开）。改了算配置有变化：服务端给受影响的机器下发「同步配置」请求，
//!   并在这些机器的操作记录里记一条（谁改的、主机、用户名、令牌末尾 4 位）。

use crate::store::{R, W};
use crate::{BoardError, ErrorCode, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const SCHEMA: &str = r#"
-- Git 凭据共用配置：只有一行（id = 1）
CREATE TABLE IF NOT EXISTS git_cred (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  spec       TEXT NOT NULL,             -- JSON：GitCred（含令牌原文）
  version    TEXT NOT NULL,             -- 内容的哈希
  updated_at INTEGER NOT NULL,
  by_name    TEXT NOT NULL,
  source     TEXT NOT NULL
);
"#;

/// Git 凭据（PRD §10.16「共用配置」）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCred {
    /// https 远端的主机名（小写，可以带 `:端口`），例如 `github.com`
    pub host: String,
    pub username: String,
    /// 令牌；列表接口里是空的（看 tail），保存时留空表示保留原来的值
    #[serde(default)]
    pub token: Option<String>,
    /// 令牌的末尾 4 位（只在列表接口里有）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<String>,
    /// 提交作者的用户名（用户级 git 配置的 `user.name`）；和提交邮箱都填或都空
    #[serde(default)]
    pub commit_name: String,
    /// 提交作者的邮箱（用户级 git 配置的 `user.email`）；和提交用户名都填或都空
    #[serde(default)]
    pub commit_email: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub source: String,
}

fn tail(v: &str) -> String {
    let c: Vec<char> = v.chars().collect();
    c[c.len().saturating_sub(4)..].iter().collect()
}

/// 规范化主机：去掉首尾空白、`https://`、路径和结尾的 `/`，转小写。`github.com`、`https://github.com/` 都是 `github.com`
pub fn normalize_host(host: &str) -> String {
    let h = host.trim();
    let h = h.strip_prefix("https://").unwrap_or(h);
    let h = h.split('/').next().unwrap_or_default();
    // 写了用户名的（`user@github.com`）只留主机
    let h = h.rsplit('@').next().unwrap_or_default();
    h.trim().to_ascii_lowercase()
}

/// 一个 https 远端地址的主机（小写，带端口时带上）；不是 https 的（SSH、`git@…`、http、本地路径）是 None
pub fn https_host(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("https://")?;
    let host = rest.split('/').next()?.rsplit('@').next()?.to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn check_host(host: &str) -> Result<()> {
    let (name, port) = match host.split_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (host, None),
    };
    let ok_name = !name.is_empty()
        && name.len() <= 253
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !name.starts_with(['.', '-'])
        && !name.ends_with(['.', '-']);
    let ok_port = port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()));
    if ok_name && ok_port {
        Ok(())
    } else {
        Err(BoardError::usage(format!("主机「{host}」不对：只写 https 远端的主机名，例如 github.com（可以带 :端口）；SSH 地址（git@…）不支持")))
    }
}

impl GitCred {
    /// 给界面、命令行看的：令牌只留末尾 4 位
    pub fn masked(&self) -> GitCred {
        GitCred { token: None, tail: self.token.as_deref().map(tail), ..self.clone() }
    }

    /// 内容的版本号（含令牌原文，不含版本号和元数据本身）
    pub fn content_version(&self) -> String {
        let body = serde_json::json!({
            "host": self.host,
            "username": self.username,
            "token": self.token,
            "commitName": self.commit_name,
            "commitEmail": self.commit_email,
        });
        Sha256::digest(body.to_string().as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
    }

    /// 记进操作记录的一句：主机 · 用户名 · 令牌末尾 4 位（不含原文）
    pub fn words(&self) -> String {
        format!("{} · 用户 {} · 令牌末尾 {}", self.host, self.username, self.token.as_deref().map(tail).or_else(|| self.tail.clone()).unwrap_or_default())
    }

    fn check(&self) -> Result<()> {
        if self.host.is_empty() {
            return Err(BoardError::usage("Git 凭据要填主机，例如 github.com"));
        }
        check_host(&self.host)?;
        if self.username.is_empty() {
            return Err(BoardError::usage("Git 凭据要填用户名"));
        }
        if self.username.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(BoardError::usage("用户名里不能有空格、换行"));
        }
        let token = self.token.as_deref().unwrap_or_default();
        if token.is_empty() {
            return Err(BoardError::usage("Git 凭据要填令牌"));
        }
        if token.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(BoardError::usage("令牌里不能有空格、换行"));
        }
        if self.commit_name.is_empty() != self.commit_email.is_empty() {
            return Err(BoardError::usage("提交用户名和提交邮箱要一起填，或都留空"));
        }
        if self.commit_name.chars().any(|c| c.is_control()) {
            return Err(BoardError::usage("提交用户名里不能有换行"));
        }
        if !self.commit_email.is_empty() {
            let (name, domain) = self.commit_email.split_once('@').unwrap_or_default();
            if name.is_empty() || domain.is_empty() || domain.contains('@') || self.commit_email.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(BoardError::usage("提交邮箱要填邮箱地址，例如 name@example.com"));
            }
        }
        Ok(())
    }
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<GitCred> {
    let spec: String = r.get(0)?;
    let mut c: GitCred = serde_json::from_str(&spec).unwrap_or_default();
    c.version = r.get(1)?;
    c.updated_at = r.get(2)?;
    c.by = r.get(3)?;
    c.source = r.get(4)?;
    Ok(c)
}

impl R<'_> {
    /// 服务端存的 Git 凭据（含令牌原文：只给这台机器用的令牌接口，见 [`R::host_uses_token`]；界面用 [`GitCred::masked`]）
    pub fn git_cred(&self) -> Result<Option<GitCred>> {
        Ok(self.conn.query_row("SELECT spec, version, updated_at, by_name, source FROM git_cred WHERE id = 1", [], row).optional()?)
    }

    /// 发给 board host 的版本号：没设时是空的
    pub fn git_version(&self) -> Result<String> {
        Ok(self.conn.query_row("SELECT version FROM git_cred WHERE id = 1", [], |r| r.get(0)).optional()?.unwrap_or_default())
    }
}

impl W<'_> {
    /// 设置或修改。令牌留空时保留原来的值。返回打过码的；用服务器的机器各记一条操作记录
    pub fn git_cred_put(&mut self, mut c: GitCred, by: &str, source: &str) -> Result<GitCred> {
        c.host = normalize_host(&c.host);
        c.username = c.username.trim().to_string();
        c.token = c.token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        c.commit_name = c.commit_name.trim().to_string();
        c.commit_email = c.commit_email.trim().to_string();
        c.tail = None;
        let old = self.read().git_cred()?;
        if c.token.is_none() {
            c.token = old.as_ref().and_then(|o| o.token.clone());
        }
        c.check()?;
        c.version = c.content_version();
        c.updated_at = self.now;
        c.by = if by.trim().is_empty() { "人".into() } else { by.trim().into() };
        c.source = if source.trim().is_empty() { "界面".into() } else { source.trim().into() };
        if old.as_ref().is_some_and(|o| o.version == c.version) {
            return Ok(old.expect("上面看过").masked());
        }
        self.tx.execute(
            "INSERT INTO git_cred (id, spec, version, updated_at, by_name, source) VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET spec = ?1, version = ?2, updated_at = ?3, by_name = ?4, source = ?5",
            params![serde_json::to_string(&c).unwrap_or_default(), c.version, c.updated_at, c.by, c.source],
        )?;
        let text = format!("{}服务端的 Git 凭据：{}", if old.is_some() { "改了" } else { "设置了" }, c.words());
        self.git_ops(&c.by, &text)?;
        self.hosts_changed = true;
        self.config_changed = true;
        self.config_by = c.by.clone();
        self.changed = true;
        Ok(c.masked())
    }

    /// 删掉：用服务器的机器下次同步时去掉之前配进仓库的凭据
    pub fn git_cred_rm(&mut self, by: &str) -> Result<()> {
        let Some(old) = self.read().git_cred()? else {
            return Err(BoardError::new(ErrorCode::NotFound, "服务端没有设 Git 凭据"));
        };
        self.tx.execute("DELETE FROM git_cred WHERE id = 1", [])?;
        let by = if by.trim().is_empty() { "人" } else { by.trim() };
        self.git_ops(by, &format!("删掉了服务端的 Git 凭据（{}）", old.words()))?;
        self.hosts_changed = true;
        self.config_changed = true;
        self.config_by = by.to_string();
        self.changed = true;
        Ok(())
    }
}
