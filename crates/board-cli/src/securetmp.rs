//! 本机临时目录：共享临时目录里的固定路径先验属主，host 自己的临时目录放在 `~/.board/host/tmp`。

use board_core::{private, BoardError, Result};
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
fn uid() -> u32 {
    unsafe { libc::geteuid() }
}

fn wrong_kind(path: &Path, what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("{} 不是{what}", path.display()))
}

#[cfg(unix)]
fn ensure_dir_owned(path: &Path, owner: u32) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let md = std::fs::symlink_metadata(path)?;
    if !md.file_type().is_dir() {
        return Err(wrong_kind(path, "目录"));
    }
    if md.uid() != owner {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} 属于 uid {}，不是当前用户 uid {owner}", path.display(), md.uid()),
        ));
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

/// 确认 `path` 是当前用户的目录并收紧到 0700；Symbolic link、普通文件和其他用户的目录都拒绝。
pub(crate) fn ensure_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        ensure_dir_owned(path, uid())
    }
    #[cfg(not(unix))]
    {
        let md = std::fs::symlink_metadata(path)?;
        if !md.file_type().is_dir() {
            return Err(wrong_kind(path, "目录"));
        }
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn create_private_dir_owned(path: &Path, owner: u32) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => ensure_dir_owned(path, owner),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => ensure_dir_owned(path, owner),
        Err(e) => Err(e),
    }
}

/// 新建（或接管已有的）当前用户私有目录，权限 0700。
pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        create_private_dir_owned(path, uid())
    }
    #[cfg(not(unix))]
    {
        if std::fs::symlink_metadata(path).is_ok() {
            return ensure_private_dir(path);
        }
        std::fs::create_dir(path)
    }
}

/// 清空并重建一个当前用户私有目录。
pub(crate) fn reset_private_dir(path: &Path) -> io::Result<()> {
    ensure_private_dir(path)?;
    std::fs::remove_dir_all(path)?;
    create_private_dir(path)
}

fn host_tmp_root() -> Result<PathBuf> {
    let root = board_core::config::board_dir()?.join("host").join("tmp");
    private::create_dir_all(&root).map_err(|e| BoardError::usage(format!("无法创建 {}：{e}", root.display())))?;
    create_private_dir(&root).map_err(|e| BoardError::usage(format!("无法使用 {}：{e}", root.display())))?;
    Ok(root)
}

fn host_tmp_at(root: &Path, name: &str) -> io::Result<PathBuf> {
    private::create_dir_all(root)?;
    create_private_dir(root)?;
    let dir = root.join(name);
    create_private_dir(&dir)?;
    Ok(dir)
}

fn label(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect()
}

fn selftest_dir_at(root: &Path, agent: Option<&str>) -> io::Result<PathBuf> {
    let name = match agent {
        Some(a) => format!("selftest-{}", label(a)),
        None => "selftest".into(),
    };
    host_tmp_at(root, &name)
}

fn usage_dir_at(root: &Path, agent: &str) -> io::Result<PathBuf> {
    host_tmp_at(root, &format!("usage-{}", label(agent)))
}

fn auth_dir_at(root: &Path, agent: &str) -> io::Result<PathBuf> {
    host_tmp_at(root, &format!("auth-{}", label(agent)))
}

pub(crate) fn selftest_dir(agent: Option<&str>) -> Result<PathBuf> {
    selftest_dir_at(&host_tmp_root()?, agent)
        .map_err(|e| BoardError::usage(format!("无法创建自检临时目录：{e}")))
}

pub(crate) fn usage_dir(agent: &str) -> Result<PathBuf> {
    usage_dir_at(&host_tmp_root()?, agent)
        .map_err(|e| BoardError::usage(format!("无法创建查额度临时目录：{e}")))
}

pub(crate) fn auth_dir(agent: &str) -> Result<PathBuf> {
    auth_dir_at(&host_tmp_root()?, agent)
        .map_err(|e| BoardError::usage(format!("无法创建问身份临时目录：{e}")))
}

/// 新建（或清空）日志文件；Unix 上权限 0600，且不跟随已有的符号链接。
pub(crate) fn create_log(path: &Path) -> io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn host_work_dirs_are_under_board_host_and_private() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".board").join("host").join("tmp");
        let dirs = [
            selftest_dir_at(&root, None).unwrap(),
            selftest_dir_at(&root, Some("claude")).unwrap(),
            usage_dir_at(&root, "claude").unwrap(),
            auth_dir_at(&root, "claude").unwrap(),
        ];
        #[cfg(unix)]
        assert_eq!(mode(&root), 0o700);
        for dir in dirs {
            assert!(dir.starts_with(&root), "{} 不在 {} 下", dir.display(), root.display());
            #[cfg(unix)]
            assert_eq!(mode(&dir), 0o700, "{}", dir.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn existing_dir_is_tightened_and_wrong_owner_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("board-heartbeat");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(dir.metadata().unwrap().uid(), uid());
        ensure_private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700, "升级前的目录收紧到 0700");

        let e = ensure_dir_owned(&dir, uid() + 1).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_private_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let link = tmp.path().join("link");
        symlink(&target, &link).unwrap();
        assert_eq!(ensure_private_dir(&link).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn logs_do_not_follow_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::write(&target, b"untouched").unwrap();
        let link = tmp.path().join("log");
        symlink(&target, &link).unwrap();
        assert!(create_log(&link).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
    }
}
