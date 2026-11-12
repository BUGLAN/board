//! skills 共用库（PRD §10.12「共用配置」、§9「board skills」）：服务端存一份 skill，各台机器的 board host 同步到本机 agent 读 skill 的目录。
//!
//! - 每个 skill 是一个目录：`SKILL.md` 加上它带的文件，合计最大 [`MAX_BYTES`]。传输时每个文件一项：相对路径、可不可执行、base64 内容。
//! - 版本号是内容的哈希（[`pack_version`]：按路径排序后对路径、可执行位、内容做 SHA-256，取前 12 位），同样的内容各处算出来一样。
//! - 记录更新时间、谁上传的、从哪里（哪台机器、界面）上传的。
//! - `board-skills` 是固定的一项：内容随服务端版本（编译服务端时打包进去，见 board-server），不存进这张表，不能删、不能手动改。
//! - 库变了算机器有变化（hosts 版本加一）：在线的 board host 马上重新取任务，看到新的库就同步。

use crate::store::{R, W};
use crate::{BoardError, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const SCHEMA: &str = r#"
-- skills 共用库（board-skills 不在这里：它随服务端版本，见 board-server）
CREATE TABLE IF NOT EXISTS skills (
  name       TEXT PRIMARY KEY,
  version    TEXT NOT NULL,             -- 内容的哈希（pack_version）
  size       INTEGER NOT NULL,          -- 所有文件的字节数
  files      TEXT NOT NULL,             -- JSON：[{path, exec, data(base64)}]，按路径排序
  updated_at INTEGER NOT NULL,
  by_name    TEXT NOT NULL,             -- 谁上传的：登录的账号、令牌名称
  source     TEXT NOT NULL              -- 从哪里：机器名、「界面」
);
"#;

/// 一个 skill 合计最大多少字节
pub const MAX_BYTES: usize = 5 * 1024 * 1024;
/// 随服务端版本的那一个
pub const BUILTIN: &str = "board-skills";
/// 同步装上的 skill 目录里放的标记文件（记下名称和版本）：没有它的同名目录是本机自己的，不覆盖
pub const MARK: &str = ".board-sync";

/// 一个文件
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFile {
    /// 相对 skill 目录的路径，用 `/` 分隔
    pub path: String,
    /// 可执行（scripts/board 这类）
    #[serde(default)]
    pub exec: bool,
    /// base64 编码的内容
    pub data: String,
}

/// 库里的一个 skill（不带内容）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    pub version: String,
    pub size: i64,
    /// 文件数
    pub files: i64,
    pub updated_at: i64,
    pub by: String,
    pub source: String,
    /// board-skills：随服务端版本，不能删、不能改
    #[serde(default)]
    pub builtin: bool,
}

/// 一个 skill 连同内容（`GET /api/skills/{名称}`）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPack {
    #[serde(flatten)]
    pub info: SkillInfo,
    #[serde(rename = "content")]
    pub files: Vec<SkillFile>,
}

/// 一台机器上一个 agent 目录里一个 skill 的同步状态（board host 上报）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillState {
    pub skill: String,
    /// agent 标识（几个 agent 共用一个目录时各报一行）
    pub agent: String,
    /// 启动环境（WSL 里的写 wsl:<发行版>）
    #[serde(default)]
    pub env: String,
    /// skill 目录（显示用）
    pub dir: String,
    /// synced 已同步 | behind 版本落后 | local 本机有同名、不是同步装的，没有覆盖 | failed 失败
    pub state: String,
    /// 本机现在的版本（同步装的才有）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

pub fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn unb64(s: &str) -> Result<Vec<u8>> {
    STANDARD.decode(s.trim()).map_err(|e| BoardError::usage(format!("文件内容不是 base64：{e}")))
}

/// skill 名称：小写字母、数字、`.`、`_`、`-`，以字母或数字开头，最长 64 个字符（也是目录名）
pub fn check_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(BoardError::usage(format!("skill 名称「{name}」不对：只能用小写字母、数字、.、_、-，以字母或数字开头，最长 64 个字符（就是目录名）")))
    }
}

/// 文件路径：相对路径、`/` 分隔、没有 `..` 和空段、不是标记文件
pub fn check_path(path: &str) -> Result<()> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.split('/').any(|s| s.is_empty() || s == "." || s == "..")
        || path == MARK;
    if bad {
        Err(BoardError::usage(format!("文件路径不对：{path}")))
    } else {
        Ok(())
    }
}

/// 内容的版本号：按路径排序后对路径、可执行位、内容做 SHA-256，取前 12 位
pub fn pack_version(files: &[(String, bool, Vec<u8>)]) -> String {
    let mut sorted: Vec<&(String, bool, Vec<u8>)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (p, x, d) in sorted {
        h.update(p.as_bytes());
        h.update([0, *x as u8, 0]);
        h.update((d.len() as u64).to_le_bytes());
        h.update(d);
    }
    h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// 校验一组文件（名称、路径、有 SKILL.md、不超过上限），解出内容，算版本号
pub fn decode(name: &str, files: &[SkillFile]) -> Result<(Vec<(String, bool, Vec<u8>)>, String, usize)> {
    check_name(name)?;
    let mut out = vec![];
    let mut size = 0usize;
    for f in files {
        check_path(&f.path)?;
        if out.iter().any(|(p, _, _): &(String, bool, Vec<u8>)| *p == f.path) {
            return Err(BoardError::usage(format!("文件写了两次：{}", f.path)));
        }
        let data = unb64(&f.data)?;
        size += data.len();
        if size > MAX_BYTES {
            return Err(BoardError::usage(format!("skill「{name}」超过 {} MB：放进库里的 skill 合计不能超过 {} MB（大文件不要放进 skill 目录）", MAX_BYTES / 1024 / 1024, MAX_BYTES / 1024 / 1024)));
        }
        out.push((f.path.clone(), f.exec, data));
    }
    if !out.iter().any(|(p, _, _)| p == "SKILL.md") {
        return Err(BoardError::usage(format!("skill「{name}」里没有 SKILL.md：上传的是 skill 目录本身（里面直接有 SKILL.md）")));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    let v = pack_version(&out);
    Ok((out, v, size))
}

fn info_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(SkillInfo, String)> {
    let files: String = r.get(3)?;
    let n = serde_json::from_str::<Vec<SkillFile>>(&files).map(|v| v.len()).unwrap_or(0);
    Ok((
        SkillInfo {
            name: r.get(0)?,
            version: r.get(1)?,
            size: r.get(2)?,
            files: n as i64,
            updated_at: r.get(4)?,
            by: r.get(5)?,
            source: r.get(6)?,
            builtin: false,
        },
        files,
    ))
}

const COLS: &str = "name, version, size, files, updated_at, by_name, source";

impl R<'_> {
    /// 库里的 skill（不含 board-skills），按名称
    pub fn skills(&self) -> Result<Vec<SkillInfo>> {
        let mut st = self.conn.prepare(&format!("SELECT {COLS} FROM skills ORDER BY name"))?;
        let rows = st.query_map([], info_row)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?.into_iter().map(|(i, _)| i).collect())
    }

    pub fn skill(&self, name: &str) -> Result<Option<SkillPack>> {
        let row = self.conn.query_row(&format!("SELECT {COLS} FROM skills WHERE name = ?1"), [name], info_row).optional()?;
        Ok(row.map(|(info, files)| SkillPack { info, files: serde_json::from_str(&files).unwrap_or_default() }))
    }
}

impl W<'_> {
    /// 放进库里（同名的换成新版本）。内容没变时不动
    pub fn skill_put(&mut self, name: &str, files: &[SkillFile], by: &str, source: &str) -> Result<SkillInfo> {
        if name == BUILTIN {
            return Err(BoardError::usage("board-skills 随服务端版本，不能上传：升级服务端就会换成新版本"));
        }
        let (decoded, version, size) = decode(name, files)?;
        if let Some(old) = self.read().skill(name)? {
            if old.info.version == version {
                return Ok(old.info);
            }
        }
        let stored: Vec<SkillFile> = decoded.iter().map(|(p, x, d)| SkillFile { path: p.clone(), exec: *x, data: b64(d) }).collect();
        let by = if by.trim().is_empty() { "人" } else { by.trim() };
        let source = if source.trim().is_empty() { "界面" } else { source.trim() };
        self.tx.execute(
            "INSERT INTO skills (name, version, size, files, updated_at, by_name, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(name) DO UPDATE SET version = ?2, size = ?3, files = ?4, updated_at = ?5, by_name = ?6, source = ?7",
            params![name, version, size as i64, serde_json::to_string(&stored).unwrap_or_default(), self.now, by, source],
        )?;
        self.hosts_changed = true;
        self.changed = true;
        Ok(SkillInfo { name: name.into(), version, size: size as i64, files: stored.len() as i64, updated_at: self.now, by: by.into(), source: source.into(), builtin: false })
    }

    /// 从库里删掉；各台机器上同步装的那份随后被删掉
    pub fn skill_rm(&mut self, name: &str) -> Result<()> {
        if name == BUILTIN {
            return Err(BoardError::usage("board-skills 是固定的一项，不能删"));
        }
        if self.tx.execute("DELETE FROM skills WHERE name = ?1", [name])? == 0 {
            return Err(BoardError::new(crate::ErrorCode::NotFound, format!("库里没有 skill「{name}」")));
        }
        self.hosts_changed = true;
        self.changed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Origin, Store};

    fn f(path: &str, data: &[u8]) -> SkillFile {
        SkillFile { path: path.into(), exec: false, data: b64(data) }
    }

    #[test]
    fn names_paths_and_versions() {
        assert!(check_name("my-skill_2.x").is_ok());
        assert!(check_name("My Skill").is_err() && check_name("-x").is_err() && check_name("").is_err());
        for bad in ["/abs", "../x", "a/../b", "a//b", "a\\b", "C:x", MARK] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
        assert!(check_path("scripts/board").is_ok());
        // 同样的内容不管顺序，版本一样；可执行位、内容不同时不一样
        let a = pack_version(&[("SKILL.md".into(), false, b"x".to_vec()), ("a".into(), true, b"y".to_vec())]);
        let b = pack_version(&[("a".into(), true, b"y".to_vec()), ("SKILL.md".into(), false, b"x".to_vec())]);
        let c = pack_version(&[("a".into(), false, b"y".to_vec()), ("SKILL.md".into(), false, b"x".to_vec())]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 12);
    }

    #[test]
    fn put_replace_remove_and_limits() {
        let mut s = Store::open_in_memory().unwrap();
        let put = |s: &mut Store, name: &str, files: Vec<SkillFile>| s.write(1000, Origin::cli(), |w| w.skill_put(name, &files, "tok", "mbp"));
        let v1 = put(&mut s, "notes", vec![f("SKILL.md", b"# notes"), f("ref/a.md", b"a")]).unwrap();
        assert_eq!((v1.files, v1.size, v1.source.as_str(), v1.by.as_str()), (2, 8, "mbp", "tok"));
        // 同样内容再传：版本不变
        assert_eq!(put(&mut s, "notes", vec![f("ref/a.md", b"a"), f("SKILL.md", b"# notes")]).unwrap().version, v1.version);
        let v2 = put(&mut s, "notes", vec![f("SKILL.md", b"# notes v2")]).unwrap();
        assert_ne!(v2.version, v1.version);
        let pack = s.read(|r| r.skill("notes")).unwrap().unwrap();
        assert_eq!(pack.info.version, v2.version);
        assert_eq!(unb64(&pack.files[0].data).unwrap(), b"# notes v2");
        assert_eq!(s.read(|r| r.skills()).unwrap().len(), 1);
        // 没有 SKILL.md、超过 5 MB、board-skills、坏路径：拒绝
        assert!(put(&mut s, "x", vec![f("README.md", b"a")]).unwrap_err().message.contains("SKILL.md"));
        let big = vec![0u8; MAX_BYTES + 1];
        let e = put(&mut s, "big", vec![f("SKILL.md", b"a"), f("blob", &big)]).unwrap_err();
        assert!(e.message.contains("超过 5 MB"), "{}", e.message);
        assert!(put(&mut s, BUILTIN, vec![f("SKILL.md", b"a")]).is_err());
        assert!(put(&mut s, "x", vec![f("SKILL.md", b"a"), f("../evil", b"a")]).is_err());
        // 删掉
        s.write(2000, Origin::cli(), |w| w.skill_rm("notes")).unwrap();
        assert!(s.read(|r| r.skill("notes")).unwrap().is_none());
        assert_eq!(s.write(2000, Origin::cli(), |w| w.skill_rm("notes")).unwrap_err().code, crate::ErrorCode::NotFound);
        assert!(s.write(2000, Origin::cli(), |w| w.skill_rm(BUILTIN)).is_err());
    }
}
