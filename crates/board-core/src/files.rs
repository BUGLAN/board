//! 卡片附件：图片和常见文件（PRD §5）。
//!
//! - 文件内容放在数据库旁边的 `files/` 目录，以 SHA-256 命名（内容相同只存一份，写入后不再改动）；
//!   `attachments` 表记录所属白板、卡片、文件名、类型、大小、上传者
//! - 单个文件不超过 [`MAX_BYTES`]；只接受 [`kind_of`] 里的类型，图片会核对文件头
//! - 界面新建卡片时先上传、后建卡片：这时附件还没有卡片（`card` 为空），新建、编辑描述、评论时
//!   文字里引用到的 `/api/files/<id>` 归到这张卡片；24 小时后仍没有卡片、也没被白板说明引用的附件被清理
//! - 删除卡片（或白板、项目）时附件记录一起删除，不再被引用的文件由清理收走
//! - 分两层：目录里的文件归 [`FileStore`]（内容寻址、原子写、上传中的预约），记录归数据库（[`Store::write`]）。
//!   服务端的全局 Store 锁只管数据库：上传落盘、清理扫目录和删文件都在锁外（卡片 #426）。
//!   上传 = [`Store::attach_check`]（只读，先挡掉明显不行的）→ [`FileStore::put`]（锁外落盘，拿着预约）
//!   → [`Store::attach_record`]（写事务里重新校验、记一条，`meta.rev` 加一）→ 记完丢掉 [`Put`]；
//!   清理 = [`Store::gc_records`]（写事务里删过期记录，`meta.rev` 加一）→ [`FileStore::collect`]（锁外删文件）

use crate::error::{BoardError, ErrorCode, Result};
use crate::model::{Actor, ActorKind, BoardRef, Origin};
use crate::store::{Store, R, W};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// 单个附件的大小上限：10 MB
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
/// 允许的类型（错误提示里用）
pub const ALLOWED: &str = "PNG、JPEG、GIF、WebP 图片，PDF、TXT、Markdown、日志、JSON、CSV、ZIP";
/// 没有归到卡片的附件保留多久
const UNBOUND_KEEP_MS: i64 = 24 * 3600 * 1000;
/// 目录里没有记录的文件多久后清理。正在上传的文件靠预约保护（[`FileStore`]），这只是兜底
pub const ORPHAN_GRACE: std::time::Duration = std::time::Duration::from_secs(3600);

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

/// 附件能不能收：文件名、类型、内容和扩展名对得上、不是空的、不超过 [`MAX_BYTES`]。
/// 返回 (整理过的文件名, MIME, 是不是图片)。服务端收附件时检查；CLI 建卡带附件时先在本地检查一遍，有一个不行就不上传、不建卡
pub fn check(name: &str, bytes: &[u8]) -> Result<(String, &'static str, bool)> {
    let name = clean_name(name);
    if name.is_empty() {
        return Err(BoardError::usage("附件需要文件名"));
    }
    let Some((mime, image)) = kind_of(&name) else {
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
    Ok((name, mime, image))
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
        created_by: Actor { kind: ActorKind::parse(&r.get::<_, String>(6)?), name: r.get(7)?, agent: None },
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

/* ------------------------------ 文件 ------------------------------ */

/// 附件目录里的文件，加上「正在上传」的预约。不碰数据库，也不在 Store 锁里用。
///
/// - 上传：[`FileStore::put`] 先预约内容的 SHA，再原子落盘（已有就不写）；调用方拿着返回的 [`Put`]，
///   等记录提交（或失败）之后才丢掉，预约随之释放
/// - 清理：[`FileStore::collect`] 在锁外扫目录，挑出没被引用、超过 [`ORPHAN_GRACE`] 没改过的文件；
///   跳过正在上传的，把剩下的标成「正在删」，再查一次数据库确认没被引用才删。标着「正在删」的 SHA，
///   新的上传先等它删完再预约、落盘，所以查完到删完之间不会有新记录指向这些文件
/// - 预约和「正在删」用同一把小锁，谁都只拿一下、不拿着它等别的锁：[`Put`] 在哪里丢掉都行（包括写事务里）
pub struct FileStore {
    dir: PathBuf,
    pending: Mutex<Pending>,
    /// 「正在删」清掉时叫醒等着的上传
    freed: Condvar,
    /// 临时文件名的序号：同样的内容同时上传时各写各的临时文件
    seq: AtomicU64,
    /// 测试用：预约之后、落盘之前调用（模拟慢盘）
    #[cfg(any(test, feature = "test-hooks"))]
    hook: Mutex<Option<PutHook>>,
}

#[derive(Default)]
struct Pending {
    /// 正在上传的 SHA（已经预约、记录还没提交）→ 几个上传在用
    reserved: HashMap<String, usize>,
    /// 清理正在确认、删除的 SHA：这些内容的上传先等着
    collecting: HashSet<String>,
}

/// 测试用的落盘钩子，参数是这次上传的 SHA
#[cfg(any(test, feature = "test-hooks"))]
pub type PutHook = Arc<dyn Fn(&str) + Send + Sync>;

/// 一次上传的预约：[`FileStore::put`] 返回，落盘后到记录提交（或失败）之前拿着，丢掉时释放
#[must_use = "要拿到记录提交之后再丢掉"]
pub struct Put {
    files: Arc<FileStore>,
    sha: String,
}

impl Put {
    /// 落盘的内容的 SHA-256（目录里的文件名，记录里的 `sha256`）
    pub fn sha(&self) -> &str {
        &self.sha
    }
}

impl Drop for Put {
    fn drop(&mut self) {
        self.files.release(&self.sha);
    }
}

impl FileStore {
    pub fn new(dir: PathBuf) -> FileStore {
        FileStore {
            dir,
            pending: Mutex::new(Pending::default()),
            freed: Condvar::new(),
            seq: AtomicU64::new(0),
            #[cfg(any(test, feature = "test-hooks"))]
            hook: Mutex::new(None),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 这份内容的文件
    pub fn path(&self, sha: &str) -> PathBuf {
        self.dir.join(sha)
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 测试用：每次上传预约之后、落盘之前调用 `hook`（`None` 去掉）
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_put_hook(&self, hook: Option<PutHook>) {
        *self.hook.lock().unwrap_or_else(|e| e.into_inner()) = hook;
    }

    /// 存一份内容：先预约它的 SHA（清理正在删同样的内容时等它删完），再原子写进目录（已有就不再写）。
    /// 返回的预约拿到记录提交之后再丢掉；写失败时预约已经释放
    pub fn put(self: &Arc<Self>, bytes: &[u8]) -> Result<Put> {
        let sha = hex(&Sha256::digest(bytes));
        {
            let mut p = self.pending();
            while p.collecting.contains(&sha) {
                p = self.freed.wait(p).unwrap_or_else(|e| e.into_inner());
            }
            *p.reserved.entry(sha.clone()).or_default() += 1;
        }
        let put = Put { files: Arc::clone(self), sha };
        #[cfg(any(test, feature = "test-hooks"))]
        {
            let hook = self.hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(h) = hook {
                h(&put.sha);
            }
        }
        self.write(&put.sha, bytes)?;
        Ok(put)
    }

    fn release(&self, sha: &str) {
        let mut p = self.pending();
        if let Some(n) = p.reserved.get_mut(sha) {
            *n -= 1;
            if *n == 0 {
                p.reserved.remove(sha);
            }
        }
    }

    /// 原子写：先写临时文件、`sync_all`，再改名成 SHA；已经有了就不写（内容相同）
    fn write(&self, sha: &str, bytes: &[u8]) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(sha);
        if path.exists() {
            return Ok(());
        }
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let tmp = self.dir.join(format!(".tmp-{sha}-{}-{n}", std::process::id()));
        let out = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, &path)
        })();
        if let Err(e) = out {
            let _ = std::fs::remove_file(&tmp);
            // 同样的内容另一个上传先写好了（Windows 上目标被打开时改名会失败）：内容一样，算成功
            if !path.exists() {
                return Err(e.into());
            }
        }
        Ok(())
    }

    /// 清理目录里不再被引用的文件，返回删了几个。`used` 是数据库里还在引用的 SHA（[`Store::gc_records`] 取的），
    /// `in_use` 删之前再查一次数据库：给出的文件名里现在还被引用的。
    ///
    /// 都不拿 Store 锁：扫目录、看修改时间；跳过正在上传的，剩下的标成「正在删」（同样内容的上传先等着），
    /// `in_use` 查到的跳过，其余删掉，最后清掉标记、叫醒等着的上传
    pub fn collect(&self, used: &HashSet<String>, in_use: impl FnOnce(&[String]) -> Result<HashSet<String>>) -> Result<usize> {
        if !self.dir.is_dir() {
            return Ok(0);
        }
        let mut old = vec![];
        for e in std::fs::read_dir(&self.dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().to_string();
            if used.contains(&name) || !e.file_type()?.is_file() {
                continue;
            }
            if e.metadata()?.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d >= ORPHAN_GRACE) {
                old.push(name);
            }
        }
        {
            let mut p = self.pending();
            old.retain(|n| !p.reserved.contains_key(n) && !p.collecting.contains(n));
            p.collecting.extend(old.iter().cloned());
        }
        if old.is_empty() {
            return Ok(0);
        }
        // 出错、panic 也要清掉标记，不然同样内容的上传会一直等
        let _marked = Marked { files: self, shas: &old };
        let still = in_use(&old)?;
        let mut removed = 0;
        for name in old.iter().filter(|n| !still.contains(*n)) {
            match std::fs::remove_file(self.dir.join(name)) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(removed)
    }
}

/// 清理标成「正在删」的 SHA：丢掉时清掉标记，叫醒等着的上传
struct Marked<'a> {
    files: &'a FileStore,
    shas: &'a [String],
}

impl Drop for Marked<'_> {
    fn drop(&mut self) {
        let mut p = self.files.pending();
        for sha in self.shas {
            p.collecting.remove(sha);
        }
        drop(p);
        self.files.freed.notify_all();
    }
}

/// 已经落盘的附件，记录要的字段：[`check`] 整理过的文件名和类型、大小、SHA-256
#[derive(Debug, Clone)]
pub struct Stored {
    pub name: String,
    pub mime: &'static str,
    pub size: i64,
    pub sha: String,
}

/// 卡片在这块白板上、没被删除；不在时 `NOT_FOUND`
fn card_alive(conn: &Connection, board: i64, id: i64) -> Result<()> {
    let n: i64 =
        conn.query_row("SELECT COUNT(*) FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![board, id], |r| r.get(0))?;
    if n == 0 {
        return Err(BoardError::new(ErrorCode::NotFound, format!("卡片 #{id} 不存在")));
    }
    Ok(())
}

/// 白板说明、定时建卡模板里引用着的附件（不属于卡片，`card` 为空）：清理时留着
fn kept_ids(r: &R) -> Result<Vec<i64>> {
    let mut keep: Vec<i64> = vec![];
    let mut texts: Vec<(i64, String)> = vec![];
    {
        // 白板说明里贴的图片
        let mut st = r.conn.prepare("SELECT id, readme FROM boards WHERE readme LIKE '%/api/files/%'")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            texts.push(row?);
        }
    }
    // 定时建卡的模板描述里贴的图片（每次建卡复制一份归到新卡）
    texts.extend(r.card_cron_files()?);
    for (board, text) in texts {
        for id in referenced_ids(&text) {
            let mine: i64 = r.conn.query_row("SELECT COUNT(*) FROM attachments WHERE id = ?1 AND board = ?2", params![id, board], |r| r.get(0))?;
            if mine > 0 {
                keep.push(id);
            }
        }
    }
    Ok(keep)
}

impl Store {
    fn files(&self) -> Result<Arc<FileStore>> {
        self.file_store().ok_or_else(|| BoardError::usage("这个数据库不支持附件"))
    }

    /// 上传附件，一次做完（测试和单线程用）。`card` 为空时先不归到卡片（界面新建卡片时）。来源按上传者算（人是界面，agent 是命令行）
    pub fn attach(&mut self, now: i64, board: &BoardRef, card: Option<i64>, by: &Actor, name: &str, bytes: &[u8]) -> Result<Attachment> {
        let origin = if by.is_human() { Origin::ui(None) } else { Origin::cli() };
        self.attach_with(now, origin, card, by, name, bytes, |_| Ok(board.clone()))
    }

    /// 上传附件，一次做完：[`Store::attach_check`] → [`FileStore::put`] → [`Store::attach_record`]。
    /// `board` 定位白板（调用方要的检查也放在这里，例如 agent 名字在线），检查和记录时各调一次。
    /// 服务端不用它：落盘要在 Store 锁外（见 board-server 的上传接口）
    #[allow(clippy::too_many_arguments)]
    pub fn attach_with(
        &mut self,
        now: i64,
        origin: Origin,
        card: Option<i64>,
        by: &Actor,
        name: &str,
        bytes: &[u8],
        board: impl Fn(&R) -> Result<BoardRef>,
    ) -> Result<Attachment> {
        let (name, mime, _) = check(name, bytes)?;
        let files = self.files()?;
        self.attach_check(card, &board)?;
        let put = files.put(bytes)?;
        let stored = Stored { name, mime, size: bytes.len() as i64, sha: put.sha().to_string() };
        self.attach_record(now, origin, card, by, &stored, |w| board(&w.read()))
    }

    /// 上传第一步（只读，很快）：定位白板、卡片还在。落盘之前做，明显不行的上传（卡片不在、名字不在线）不留文件
    pub fn attach_check(&self, card: Option<i64>, board: impl FnOnce(&R) -> Result<BoardRef>) -> Result<BoardRef> {
        self.files()?;
        self.read(|r| {
            let b = board(r)?;
            if let Some(id) = card {
                card_alive(r.conn, b.id, id)?;
            }
            Ok(b)
        })
    }

    /// 上传最后一步：文件已经落盘（调用方拿着它的 [`Put`]，记完再丢掉），在写事务里重新定位白板、确认卡片还在，再记一条，
    /// `meta.rev` 加一。失败时什么都不记，落盘的文件没有记录，之后被清理
    pub fn attach_record(
        &mut self,
        now: i64,
        origin: Origin,
        card: Option<i64>,
        by: &Actor,
        file: &Stored,
        board: impl FnOnce(&W) -> Result<BoardRef>,
    ) -> Result<Attachment> {
        self.write(now, origin, |w| {
            let b = board(w)?;
            if let Some(id) = card {
                card_alive(w.tx, b.id, id)?;
            }
            w.tx.execute(
                "INSERT INTO attachments (board, card, name, mime, size, sha256, created_at, created_by_kind, created_by_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![b.id, card, file.name, file.mime, file.size, file.sha, now, by.kind.as_str(), by.name],
            )?;
            let id = w.tx.last_insert_rowid();
            w.changed = true;
            Ok(w.tx.query_row(&format!("SELECT {COLS} FROM attachments WHERE id = ?1"), [id], row)?)
        })
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
        Ok(Some((a, board, self.files()?.path(&sha))))
    }

    /// 清理第一步（写事务里）：删掉 24 小时还没归到卡片、也没被白板说明和定时建卡模板引用的附件记录，删了就 `meta.rev` 加一。
    /// 返回删了几条，和现在还被引用的 SHA（交给 [`FileStore::collect`]）
    pub fn gc_records(&mut self, now: i64) -> Result<(usize, HashSet<String>)> {
        self.write(now, Origin::system(), |w| {
            let keep = kept_ids(&w.read())?.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",");
            let rows = w.tx.execute(
                &format!("DELETE FROM attachments WHERE card IS NULL AND created_at < ?1 AND id NOT IN ({keep})"),
                [now - UNBOUND_KEEP_MS],
            )?;
            if rows > 0 {
                w.changed = true;
            }
            let used: HashSet<String> =
                w.tx.prepare("SELECT DISTINCT sha256 FROM attachments")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            Ok((rows, used))
        })
    }

    /// 给出的 SHA 里现在还有附件记录引用的（清理删文件前再查一次）
    pub fn files_in_use(&self, shas: &[String]) -> Result<HashSet<String>> {
        let mut st = self.conn().prepare("SELECT 1 FROM attachments WHERE sha256 = ?1 LIMIT 1")?;
        let mut out = HashSet::new();
        for sha in shas {
            if st.exists([sha])? {
                out.insert(sha.clone());
            }
        }
        Ok(out)
    }

    /// 清理，一次做完（测试和单线程用）：[`Store::gc_records`] → [`FileStore::collect`]。返回 (记录数, 文件数)。
    /// 服务端把删文件放在 Store 锁外（见 board-server 的定时任务）
    pub fn gc_files(&mut self, now: i64) -> Result<(usize, usize)> {
        let (rows, used) = self.gc_records(now)?;
        let Some(files) = self.file_store() else { return Ok((rows, 0)) };
        let removed = files.collect(&used, |c| self.files_in_use(c))?;
        Ok((rows, removed))
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
