//! Unix 上的私有文件权限：目录 0700，文件 0600。其他平台保持系统默认权限。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

#[cfg(unix)]
fn wrong_kind(path: &Path, want: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("{} 不是{want}", path.display()))
}

/// 创建目录（含父目录），Unix 上新建的目录为 0700；已有目录不改。
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

/// 收紧一个已有目录的权限；Unix 上为 0700。
pub fn set_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !std::fs::symlink_metadata(path)?.file_type().is_dir() {
            return Err(wrong_kind(path, "目录"));
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// 打开或新建可读写文件；Unix 上新建时为 0600（已有文件由 [`set_file`] 收紧）。
pub fn open_rw(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        match std::fs::symlink_metadata(path) {
            Ok(m) if !m.file_type().is_file() => return Err(wrong_kind(path, "普通文件")),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        options.mode(0o600);
    }
    options.open(path)
}

/// 新建文件；Unix 上权限固定为 0600，文件已存在时返回 `AlreadyExists`。
pub fn create_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = file.set_permissions(std::fs::Permissions::from_mode(0o600)) {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(e);
        }
    }
    Ok(file)
}

/// 新建并写入一个私有文件；Unix 上为 0600。
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_new(path)?;
    if let Err(e) = file.write_all(bytes) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// 收紧一个已有文件的权限；Unix 上为 0600。
pub fn set_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !std::fs::symlink_metadata(path)?.file_type().is_file() {
            return Err(wrong_kind(path, "普通文件"));
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// 权限收紧失败时输出一条可读警告，不影响调用方继续读取。
pub fn warn(path: &Path, e: &io::Error) {
    eprintln!("警告：无法收紧 {} 的权限：{e}", path.display());
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::*;

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn private_dirs_and_files_are_tightened() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a").join("b");
        create_dir_all(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);

        let file = dir.join("secret");
        write_new(&file, b"secret").unwrap();
        assert_eq!(mode(&file), 0o600);

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_dir_all(&dir).unwrap();
        assert_eq!(mode(&dir), 0o755, "已有目录由调用方决定是否收紧");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        set_dir(&dir).unwrap();
        set_file(&file).unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&file), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn create_new_does_not_follow_a_preexisting_symlink() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::write(&target, b"untouched").unwrap();
        let link = tmp.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(create_new(&link).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
    }

    #[cfg(unix)]
    #[test]
    fn hardening_rejects_files_and_symlinks_without_touching_targets() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        std::fs::write(&file, b"not a directory").unwrap();
        let link = tmp.path().join("link");
        symlink(&file, &link).unwrap();
        let dir = tmp.path().join("dir");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(set_dir(&file).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(set_dir(&link).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(set_file(&dir).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(set_file(&link).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(mode(&file), 0o644);
        assert_eq!(mode(&dir), 0o755);

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(open_rw(&link).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(mode(&file), 0o644, "不能顺着文件符号链接打开或改权限");
    }
}
