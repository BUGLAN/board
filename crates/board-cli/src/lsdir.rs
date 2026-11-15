//! 列目录（界面选工作空间时，服务端转来的「列目录」请求）：只列文件夹，不读文件内容。
//!
//! - 空路径是最上面一层：Windows 列盘符（`C:\`、`D:\` …），macOS / Linux 是用户目录
//! - 每个子文件夹写：名称、完整路径、它本身是不是 git 仓库（里面有 `.git`）、它下面一层有几个 git 仓库
//! - `chain` 是从最上面一层到它自己的每一级：界面据此一层一列展开到现在的工作空间
//! - 隐藏的（`.` 开头的）不列；一个文件夹最多列 [`LIMIT`] 个，数仓库时每个子文件夹最多看 [`LIMIT`] 项

use board_core::{DirEntry, DirList};
use std::path::{Path, PathBuf};

/// 一层最多列几个
pub const LIMIT: usize = 500;

/// 用户目录（不带结尾的分隔符）
fn home() -> PathBuf {
    let h = crate::envs::expand("~").display().to_string();
    let t = h.trim_end_matches(['/', '\\']);
    PathBuf::from(if t.is_empty() { h.as_str() } else { t })
}

/// `~` 开头的展开成用户目录；去掉结尾多余的分隔符（盘符的 `C:\` 留着）
fn expand(path: &str) -> String {
    let p = path.trim();
    if p.is_empty() {
        return String::new();
    }
    let full = if p == "~" || p.starts_with("~/") || p.starts_with("~\\") { crate::envs::expand(p).display().to_string() } else { p.to_string() };
    let trimmed = full.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return "/".into();
    }
    if trimmed.len() == 2 && trimmed.ends_with(':') {
        return format!("{trimmed}\\");
    }
    trimmed.to_string()
}

/// 从最上面一层（空）到它自己的每一级
fn chain_of(path: &str, home: &Path, win: bool) -> Vec<String> {
    let mut out = vec![String::new()];
    if path.is_empty() {
        return out;
    }
    if win {
        let parts: Vec<&str> = path.split('\\').filter(|x| !x.is_empty()).collect();
        let mut cur = String::new();
        for (i, x) in parts.iter().enumerate() {
            cur = if i == 0 { format!("{x}\\") } else if cur.ends_with('\\') { format!("{cur}{x}") } else { format!("{cur}\\{x}") };
            out.push(cur.clone());
        }
        return out;
    }
    let h = home.display().to_string();
    let (start, rest) = match path.strip_prefix(&h) {
        Some(r) if r.is_empty() || r.starts_with('/') => (h.clone(), r.to_string()),
        _ => ("/".to_string(), path.trim_start_matches('/').to_string()),
    };
    out.push(start.clone());
    let mut cur = start;
    for x in rest.split('/').filter(|x| !x.is_empty()) {
        cur = if cur.ends_with('/') { format!("{cur}{x}") } else { format!("{cur}/{x}") };
        out.push(cur.clone());
    }
    out
}

fn is_repo(p: &Path) -> bool {
    p.join(".git").exists()
}

/// 下面一层有几个 git 仓库
fn repos_in(p: &Path) -> i64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten().take(LIMIT).filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()) && is_repo(&e.path())).count() as i64
}

fn entry(name: String, path: &Path) -> DirEntry {
    let git = is_repo(path);
    DirEntry { name, path: path.display().to_string(), git, repos: if git { 0 } else { repos_in(path) } }
}

/// 列一个文件夹的子文件夹
pub fn list(path: &str) -> DirList {
    list_with(path, &home(), cfg!(windows))
}

fn list_with(path: &str, home: &Path, win: bool) -> DirList {
    let p = expand(path);
    let chain = chain_of(&p, home, win);
    if p.is_empty() {
        let entries = if win {
            ('A'..='Z')
                .map(|c| format!("{c}:\\"))
                .filter(|d| Path::new(d).is_dir())
                .map(|d| DirEntry { name: d.clone(), path: d.clone(), git: false, repos: repos_in(Path::new(&d)) })
                .collect()
        } else {
            vec![entry(home.display().to_string(), home)]
        };
        return DirList { path: p, chain, entries, error: None };
    }
    let dir = Path::new(&p);
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            let why = if e.kind() == std::io::ErrorKind::NotFound { "这个文件夹不存在".to_string() } else { format!("读不了：{e}") };
            return DirList { path: p, chain, entries: vec![], error: Some(why) };
        }
    };
    let mut entries: Vec<DirEntry> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter(|e| e.path().is_dir())
        .take(LIMIT)
        .map(|e| entry(e.file_name().to_string_lossy().into_owned(), &e.path()))
        .collect();
    entries.sort_by_key(|e| e.name.to_lowercase());
    DirList { path: p, chain, entries, error: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_repos_and_chain() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("me");
        let repo = home.join("repo");
        for x in ["board/.git", "notes/.git", "board.wt/host-detail/.git", "plain", ".hidden"] {
            std::fs::create_dir_all(repo.join(x)).unwrap();
        }
        std::fs::write(repo.join("README.md"), "x").unwrap();
        let out = list_with(&repo.display().to_string(), &home, false);
        assert_eq!(out.error, None);
        let got: Vec<(String, bool, i64)> = out.entries.iter().map(|e| (e.name.clone(), e.git, e.repos)).collect();
        assert_eq!(got, vec![("board".into(), true, 0), ("board.wt".into(), false, 1), ("notes".into(), true, 0), ("plain".into(), false, 0)], "只列文件夹，隐藏的不列");
        // 面包屑按平台的写法拼：Windows 上按 `\` 一级一级往下（见 windows_chain_and_drive_root）
        if !cfg!(windows) {
            let h = home.display().to_string();
            assert_eq!(out.chain, vec![String::new(), h.clone(), format!("{h}/repo")]);
        }
        let h = home.display().to_string();
        // 最上面一层：用户目录，带它下面的仓库数
        let top = list_with("", &home, false);
        assert_eq!((top.entries.len(), top.entries[0].path.clone(), top.chain.clone()), (1, h.clone(), vec![String::new()]));
        // 用户目录外面的：从 / 开始
        assert_eq!(chain_of("/opt/x", &home, false), vec!["", "/", "/opt", "/opt/x"]);
        // 不存在
        assert!(list_with(&format!("{h}/nope"), &home, false).error.unwrap().contains("不存在"));
    }

    #[test]
    fn windows_chain_and_drive_root() {
        assert_eq!(chain_of("D:\\repo\\board.wt", Path::new("C:\\Users\\me"), true), vec!["", "D:\\", "D:\\repo", "D:\\repo\\board.wt"]);
        assert_eq!(expand("D:"), "D:\\");
        assert_eq!(expand("D:\\repo\\"), "D:\\repo");
    }
}
