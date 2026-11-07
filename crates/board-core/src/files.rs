//! 卡片附件：图片和常见文件（PRD §5）。
//!
//! - 文件内容放在数据库旁边的 `files/` 目录，以 SHA-256 命名（内容相同只存一份，写入后不再改动）；
//!   `attachments` 表记录所属白板、卡片、文件名、类型、大小、上传者
//! - 单个文件不超过 [`MAX_BYTES`]；只接受 [`kind_of`] 里的类型，图片会核对文件头
//! - 界面新建卡片时先上传、后建卡片：这时附件还没有卡片（`card` 为空），新建、编辑描述、评论时
//!   文字里引用到的 `/api/files/<id>` 归到这张卡片；24 小时后仍没有卡片的附件被清理
//! - 删除卡片（或白板、项目）时附件记录一起删除，不再被引用的文件由 [`Store::gc_files`] 清理

use crate::error::{BoardError, ErrorCode, Result};
use crate::model::{Actor, ActorKind, BoardRef};
use crate::store::{Store, R, W};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 单个附件的大小上限：10 MB
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
/// 允许的类型（错误提示里用）
pub const ALLOWED: &str = "PNG、JPEG、GIF、WebP 图片，PDF、TXT、Markdown、日志、JSON、CSV、ZIP";
/// 没有归到卡片的附件保留多久
const UNBOUND_KEEP_MS: i64 = 24 * 3600 * 1000;
/// 目录里没有记录的文件多久后清理（避免清掉正在上传、还没写记录的文件）
const ORPHAN_GRACE: std::time::Duration = std::time::Duration::from_secs(3600);

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS attachments (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  board           INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  card            INTEGER,
  name            TEXT NOT NULL,
  mime            TEXT NOT NULL,
  size            INTEGER NOT NULL,
  sha256          TEXT NOT NULL,
  created_at      INTEGER NOT NULL,
  created_by_kind TEXT NOT NULL,
  created_by_name TEXT NOT NULL,
  FOREIGN KEY (board, card) REFERENCES cards(board, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS attachments_card ON attachments (board, card);
"#;

/// 附件的类型：按扩展名判断；返回 (MIME, 是不是图片)
pub fn kind_of(name: &str) -> Option<(&'static str, bool)> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => ("image/png", true),
        "jpg" | "jpeg" => ("image/jpeg", true),
        "gif" => ("image/gif", true),
        "webp" => ("image/webp", true),
        "pdf" => ("application/pdf", false),
        "txt" | "log" => ("text/plain; charset=utf-8", false),
        "md" | "markdown" => ("text/markdown; charset=utf-8", false),
        "json" => ("application/json", false),
        "csv" => ("text/csv; charset=utf-8", false),
        "zip" => ("application/zip", false),
        _ => return None,
    })
}

/// 文件头和扩展名对得上（图片、PDF、ZIP）；文本类不检查
fn magic_ok(mime: &str, b: &[u8]) -> bool {
    match mime {
        "image/png" => b.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => b.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a"),
        "image/webp" => b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP",
        "application/pdf" => b.starts_with(b"%PDF"),
        "application/zip" => b.starts_with(b"PK\x03\x04") || b.starts_with(b"PK\x05\x06"),
        _ => true,
    }
}

/// 文件名只保留最后一段，去掉控制字符，最长 120 个字符
pub fn clean_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let s: String = base.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
    if s.chars().count() <= 120 {
        return s;
    }
    // 太长时保留扩展名
    match s.rsplit_once('.') {
        Some((stem, ext)) if ext.chars().count() <= 10 => {
            format!("{}.{ext}", stem.chars().take(119 - ext.chars().count()).collect::<String>())
        }
        _ => s.chars().take(120).collect(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: i64,
    /// 还没归到卡片时为空（界面新建卡片时先上传的图片）
    #[serde(default)]
    pub card_id: Option<i64>,
    pub name: String,
    pub mime: String,
    pub size: i64,
    /// 图片在描述、评论里直接显示，其他文件显示为下载链接
    pub image: bool,
    /// 读取地址（需要登录或访问令牌）
    pub url: String,
    /// 插入描述或评论的 Markdown
    pub markdown: String,
    pub created_at: i64,
    pub created_by: Actor,
}

pub fn url_of(id: i64) -> String {
    format!("/api/files/{id}")
}

/// 插入描述或评论用的 Markdown：图片 `![名](地址)`，其他文件 `[名](地址)`
pub fn markdown_of(id: i64, name: &str, image: bool) -> String {
    let label: String = name.chars().map(|c| if matches!(c, '[' | ']') { ' ' } else { c }).collect();
    format!("{}[{label}]({})", if image { "!" } else { "" }, url_of(id))
}

const COLS: &str = "id, card, name, mime, size, created_at, created_by_kind, created_by_name";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Attachment> {
    let id: i64 = r.get(0)?;
    let name: String = r.get(2)?;
    let mime: String = r.get(3)?;
    let image = mime.starts_with("image/");
    Ok(Attachment {
        id,
        card_id: r.get(1)?,
        url: url_of(id),
        markdown: markdown_of(id, &name, image),
        name,
        mime,
        size: r.get(4)?,
        image,
        created_at: r.get(5)?,
        created_by: Actor { kind: ActorKind::parse(&r.get::<_, String>(6)?), name: r.get(7)? },
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 文字里引用到的附件 id（`/api/files/<id>`）
fn referenced_ids(text: &str) -> Vec<i64> {
    let mut out = vec![];
    let mut rest = text;
    while let Some(i) = rest.find("/api/files/") {
        rest = &rest[i + "/api/files/".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(id) = digits.parse::<i64>() {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// 把 `src` 里还没有的文件复制到 `dst`（备份用；文件内容不会变，同名即相同），返回复制的个数
pub fn copy_new(src: &Path, dst: &Path) -> std::io::Result<usize> {
    if !src.is_dir() {
        return Ok(0);
    }
    std::fs::create_dir_all(dst)?;
    let mut n = 0;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        if !e.file_type()?.is_file() || name.to_string_lossy().starts_with('.') {
            continue;
        }
        let to = dst.join(&name);
        if !to.exists() {
            std::fs::copy(e.path(), &to)?;
            n += 1;
        }
    }
    Ok(n)
}

/// 数据库旁边的附件目录
pub fn dir_for(db: &Path) -> PathBuf {
    db.parent().map(|d| d.join("files")).unwrap_or_else(|| PathBuf::from("files"))
}

impl Store {
    fn files(&self) -> Result<&Path> {
        self.files_dir().ok_or_else(|| BoardError::usage("这个数据库不支持附件"))
    }

    /// 上传附件。`card` 为空时先不归到卡片（界面新建卡片时）
    pub fn attach(&mut self, now: i64, board: &BoardRef, card: Option<i64>, by: &Actor, name: &str, bytes: &[u8]) -> Result<Attachment> {
        let name = clean_name(name);
        if name.is_empty() {
            return Err(BoardError::usage("附件需要文件名"));
        }
        let Some((mime, _)) = kind_of(&name) else {
            return Err(BoardError::usage(format!("不支持这种文件：{name}。支持 {ALLOWED}")));
        };
        if bytes.is_empty() {
            return Err(BoardError::usage(format!("{name} 是空文件")));
        }
        if bytes.len() > MAX_BYTES {
            return Err(BoardError::usage(format!(
                "{name} 有 {:.1} MB，附件不能超过 {} MB",
                bytes.len() as f64 / 1048576.0,
                MAX_BYTES / 1048576
            )));
        }
        if !magic_ok(mime, bytes) {
            return Err(BoardError::usage(format!("{name} 的内容和扩展名对不上")));
        }
        if let Some(id) = card {
            let n: i64 = self.conn().query_row(
                "SELECT COUNT(*) FROM cards WHERE board = ?1 AND id = ?2",
                params![board.id, id],
                |r| r.get(0),
            )?;
            if n == 0 {
                return Err(BoardError::new(ErrorCode::NotFound, format!("卡片 #{id} 不存在")));
            }
        }
        // 先写文件再写记录：中途失败只会留下没有记录的文件，之后被清理
        let sha = hex(&Sha256::digest(bytes));
        let dir = self.files()?.to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(&sha);
        if !path.exists() {
            let tmp = dir.join(format!(".tmp-{sha}-{}", std::process::id()));
            {
                let mut f = std::fs::File::create(&tmp)?;
                f.write_all(bytes)?;
                f.sync_all()?;
            }
            std::fs::rename(&tmp, &path)?;
        }
        self.conn().execute(
            "INSERT INTO attachments (board, card, name, mime, size, sha256, created_at, created_by_kind, created_by_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![board.id, card, name, mime, bytes.len() as i64, sha, now, by.kind.as_str(), by.name],
        )?;
        let id = self.conn().last_insert_rowid();
        Ok(self
            .conn()
            .query_row(&format!("SELECT {COLS} FROM attachments WHERE id = ?1"), [id], row)?)
    }

    /// 读取附件：记录、所属白板 id、文件路径
    pub fn attachment(&self, id: i64) -> Result<Option<(Attachment, i64, PathBuf)>> {
        let found: Option<(Attachment, i64, String)> = self
            .conn()
            .query_row(&format!("SELECT {COLS}, board, sha256 FROM attachments WHERE id = ?1"), [id], |r| {
                Ok((row(r)?, r.get(8)?, r.get(9)?))
            })
            .optional()?;
        let Some((a, board, sha)) = found else { return Ok(None) };
        Ok(Some((a, board, self.files()?.join(sha))))
    }

    /// 清理：24 小时还没归到卡片的附件记录；目录里不再被任何记录引用的文件。返回 (记录数, 文件数)
    pub fn gc_files(&mut self, now: i64) -> Result<(usize, usize)> {
        let rows = self
            .conn()
            .execute("DELETE FROM attachments WHERE card IS NULL AND created_at < ?1", [now - UNBOUND_KEEP_MS])?;
        let Some(dir) = self.files_dir().map(Path::to_path_buf) else { return Ok((rows, 0)) };
        if !dir.is_dir() {
            return Ok((rows, 0));
        }
        let used: HashSet<String> = self
            .conn()
            .prepare("SELECT DISTINCT sha256 FROM attachments")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        let mut files = 0;
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().to_string();
            if used.contains(&name) || !e.file_type()?.is_file() {
                continue;
            }
            let old = e.metadata()?.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d >= ORPHAN_GRACE);
            if old {
                std::fs::remove_file(e.path())?;
                files += 1;
            }
        }
        Ok((rows, files))
    }
}

impl R<'_> {
    /// 卡片的附件，按上传顺序
    pub fn attachments(&self, board: i64, card: i64) -> Result<Vec<Attachment>> {
        attachments_of(self.conn, board, card)
    }
}

pub(crate) fn attachments_of(conn: &Connection, board: i64, card: i64) -> Result<Vec<Attachment>> {
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM attachments WHERE board = ?1 AND card = ?2 ORDER BY id"))?;
    let rows = st.query_map(params![board, card], row)?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

impl W<'_> {
    /// 文字里引用到的、同一块白板上还没归到卡片的附件，归到这张卡片
    pub(crate) fn bind_attachments(&mut self, board: i64, card: i64, text: &str) -> Result<()> {
        for id in referenced_ids(text) {
            self.tx.execute(
                "UPDATE attachments SET card = ?1 WHERE id = ?2 AND board = ?3 AND card IS NULL",
                params![card, id, board],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_names() {
        assert_eq!(kind_of("a.PNG"), Some(("image/png", true)));
        assert_eq!(kind_of("log.txt").map(|k| k.1), Some(false));
        assert_eq!(kind_of("x.svg"), None);
        assert_eq!(kind_of("x.html"), None);
        assert_eq!(kind_of("noext"), None);
        assert_eq!(clean_name("../../etc/passwd.txt"), "passwd.txt");
        assert_eq!(clean_name("C:\\Users\\me\\截图 1.png"), "截图 1.png");
        let long = format!("{}.png", "长".repeat(200));
        let c = clean_name(&long);
        assert_eq!(c.chars().count(), 120);
        assert!(c.ends_with(".png"));
        assert_eq!(markdown_of(3, "a[1].png", true), "![a 1 .png](/api/files/3)");
        assert_eq!(markdown_of(4, "r.pdf", false), "[r.pdf](/api/files/4)");
        assert_eq!(referenced_ids("看 ![a](/api/files/12) 和 [b](/api/files/7)，再看 /api/files/12"), vec![12, 7]);
    }

    #[test]
    fn magic() {
        assert!(magic_ok("image/png", b"\x89PNG\r\n\x1a\nrest"));
        assert!(!magic_ok("image/png", b"<html>"));
        assert!(magic_ok("image/webp", b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(magic_ok("text/plain; charset=utf-8", b"anything"));
    }
}
