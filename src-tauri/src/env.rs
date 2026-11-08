//! 「Agent 接入」里新建令牌后，一键把服务端地址和令牌写进 board 命令行的配置文件 `~/.board/config.toml`
//! （Windows：`%USERPROFILE%\.board\config.toml`），效果和 `board login --url <地址> --token <令牌>` 相同。
//!
//! - 服务端地址用外壳自己打开的地址（不由页面传入）；页面只传令牌，必须是 `bd_` 加十六进制
//! - 目录权限 0700、文件权限 0600；改写时保留其他配置项和注释。不改 shell 配置文件，也不改系统环境变量
//! - 令牌刚在界面上新建，这里不再向服务端校验；写完 board 命令马上就能用，不用新开终端

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct Written {
    /// 写到了哪里，给人看的：`~/.board/config.toml`
    pub target: String,
}

/// 令牌只允许 `bd_` 加十六进制
pub fn valid_token(t: &str) -> bool {
    t.strip_prefix("bd_").is_some_and(|h| (16..=128).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// 地址只保留 origin，并且只允许常见字符
pub fn origin_of(url: &tauri::Url) -> Result<String, String> {
    let o = url.origin().ascii_serialization();
    let ok = (o.starts_with("https://") || o.starts_with("http://"))
        && o.bytes().all(|b| b.is_ascii_alphanumeric() || b".:/-[]".contains(&b));
    if ok {
        Ok(o)
    } else {
        Err(format!("服务端地址不能写进配置文件：{o}"))
    }
}

/// 配置文件的位置：Windows 上在 USERPROFILE 下，其他系统在 HOME 下（与 board 命令行相同）
fn config_path() -> Result<PathBuf, String> {
    let keys: [&str; 2] = if cfg!(windows) { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
    keys.iter()
        .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
        .map(|h| h.join(".board").join("config.toml"))
        .ok_or_else(|| "找不到用户目录".to_string())
}

/// 给人看的写法
fn shown() -> &'static str {
    if cfg!(windows) {
        r"%USERPROFILE%\.board\config.toml"
    } else {
        "~/.board/config.toml"
    }
}

/// 写入 url、token 两项，保留文件里的其他配置项；目录 0700、文件 0600，先写临时文件再改名
pub fn upsert_config(path: &Path, url: &str, token: &str) -> Result<(), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("原来的配置文件不是合法的 TOML：{e}"))?;
    doc["url"] = toml_edit::value(url);
    doc["token"] = toml_edit::value(token);
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
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
        let mut f = o.open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(doc.to_string().as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn write(url: &str, token: &str) -> Result<Written, String> {
    if !valid_token(token) {
        return Err("令牌格式不对".into());
    }
    let path = config_path()?;
    upsert_config(&path, url, token).map_err(|e| format!("写入 {} 失败：{e}", shown()))?;
    Ok(Written { target: shown().into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "bd_0123456789abcdef0123456789abcdef0123456789abcdef";

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("board-env-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn token_format() {
        assert!(valid_token(TOKEN));
        assert!(!valid_token("bd_123"));
        assert!(!valid_token("bd_zz0123456789abcdef0123456789ab"));
        assert!(!valid_token("bd_0123456789abcdef0123456789abcdef0123456789abcdef'; rm -rf ~; '"));
        assert!(!valid_token("0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn origin_only() {
        let u: tauri::Url = "https://board.buglan.com/some/path?q=1".parse().unwrap();
        assert_eq!(origin_of(&u).unwrap(), "https://board.buglan.com");
        let u: tauri::Url = "http://localhost:1420/".parse().unwrap();
        assert_eq!(origin_of(&u).unwrap(), "http://localhost:1420");
    }

    #[test]
    fn upsert_writes_and_keeps_other_items() {
        let d = tmp("upsert");
        let f = d.join(".board").join("config.toml");
        // 文件不存在时新建
        upsert_config(&f, "http://localhost:1420", TOKEN).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), format!("url = \"http://localhost:1420\"\ntoken = \"{TOKEN}\"\n"));
        // 改写时保留其他配置项和注释，重复写不会重复
        std::fs::write(&f, "# 我的配置\neditor = \"vim\"\nurl = \"http://old\"\ntoken = \"bd_old\"\n").unwrap();
        upsert_config(&f, "https://board.buglan.com", TOKEN).unwrap();
        upsert_config(&f, "https://board.buglan.com", TOKEN).unwrap();
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            format!("# 我的配置\neditor = \"vim\"\nurl = \"https://board.buglan.com\"\ntoken = \"{TOKEN}\"\n")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(f.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        std::fs::remove_dir_all(&d).ok();
    }
}
