//! 「Agent 接入」里新建令牌后，一键把服务端地址和令牌写进 board 命令行的配置文件 `~/.board/config.toml`
//! （Windows：`%USERPROFILE%\.board\config.toml`），效果和 `board login --url <地址> --token <令牌>` 相同。
//!
//! - 服务端地址用外壳自己打开的地址（不由页面传入）；页面只传令牌，必须是 `bd_` 加十六进制
//! - 读写用 board_core::config（与 board 命令行同一份）：目录权限 0700、文件权限 0600；改写时保留其他配置项和注释，和 board login 同时写不会互相覆盖。不改 shell 配置文件，也不改系统环境变量
//! - 令牌刚在界面上新建，这里不再向服务端校验；写完 board 命令马上就能用，不用新开终端

use board_core::config;
use serde::Serialize;

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

pub fn write(url: &str, token: &str) -> Result<Written, String> {
    if !valid_token(token) {
        return Err("令牌格式不对".into());
    }
    config::update(|c| {
        c.set(config::URL, url);
        c.set(config::TOKEN, token);
        Ok(((), true))
    })
    .map_err(|e| e.message)?;
    Ok(Written { target: config::shown() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "bd_0123456789abcdef0123456789abcdef0123456789abcdef";

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
        let u: tauri::Url = "https://site.example.com/some/path?q=1".parse().unwrap();
        assert_eq!(origin_of(&u).unwrap(), "https://site.example.com");
        let u: tauri::Url = "http://localhost:1420/".parse().unwrap();
        assert_eq!(origin_of(&u).unwrap(), "http://localhost:1420");
    }
    // 写配置文件本身的测试在 board_core::config
}
