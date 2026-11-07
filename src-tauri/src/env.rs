//! 「Agent 接入」里新建令牌后，一键把 `BOARD_URL`、`BOARD_TOKEN` 写成长期生效的环境变量。
//!
//! - 服务端地址用外壳自己打开的地址（不由页面传入）；页面只传令牌，必须是 `bd_` 加十六进制
//! - macOS / Linux：写进登录 shell 的配置文件（zsh → ~/.zshrc；bash → ~/.bashrc，macOS 上 ~/.bash_profile）。
//!   先删掉文件里旧的这两行再追加，重复写不会重复；直接写回原路径，配置文件是软链接时写到它指向的文件
//! - Windows：`setx` 写入用户环境变量（不经过 shell，参数原样传入）
//!
//! 已经打开的终端不会生效，要新开一个。

use serde::Serialize;
use std::path::Path;
#[cfg(any(not(target_os = "windows"), test))]
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct Written {
    /// 写到了哪里，给人看的：`~/.zshrc`、`用户环境变量`
    pub target: String,
}

/// 令牌只允许 `bd_` 加十六进制，写进配置文件时不会带进引号、换行之类的字符
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
        Err(format!("服务端地址不能写进环境变量：{o}"))
    }
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
/// 把这两行写进 shell 配置文件：去掉旧的 `export BOARD_URL=`、`export BOARD_TOKEN=` 行，在末尾追加
pub fn upsert_rc(path: &Path, url: &str, token: &str) -> std::io::Result<()> {
    let old = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let mut out: String = old
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("export BOARD_URL=") && !t.starts_with("export BOARD_TOKEN=")
        })
        .map(|l| format!("{l}\n"))
        .collect();
    out.push_str(&format!("export BOARD_URL='{url}'\nexport BOARD_TOKEN='{token}'\n"));
    std::fs::write(path, out)
}

/// 当前用户的登录 shell 对应的配置文件
#[cfg(not(target_os = "windows"))]
fn rc_file() -> Result<(PathBuf, String), String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("找不到 HOME 目录")?;
    let shell = std::env::var("SHELL").unwrap_or_default();
    let name = Path::new(&shell).file_name().and_then(|s| s.to_str()).unwrap_or_default();
    let name = match name {
        "" if cfg!(target_os = "macos") => "zsh",
        "" => "bash",
        n => n,
    };
    let file = match name {
        "zsh" => ".zshrc",
        "bash" if cfg!(target_os = "macos") => ".bash_profile",
        "bash" => ".bashrc",
        other => return Err(format!("不支持自动写入 {other} 的配置，请复制下面的命令手动设置")),
    };
    Ok((home.join(file), format!("~/{file}")))
}

pub fn write(url: &str, token: &str) -> Result<Written, String> {
    if !valid_token(token) {
        return Err("令牌格式不对".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        for (k, v) in [("BOARD_URL", url), ("BOARD_TOKEN", token)] {
            let status = std::process::Command::new("setx")
                .args([k, v])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map_err(|e| format!("运行 setx 失败：{e}"))?;
            if !status.success() {
                return Err(format!("setx {k} 失败（退出码 {:?}）", status.code()));
            }
        }
        Ok(Written { target: "用户环境变量".into() })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let (path, shown) = rc_file()?;
        upsert_rc(&path, url, token).map_err(|e| format!("写入 {shown} 失败：{e}"))?;
        Ok(Written { target: shown })
    }
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
    fn upsert_replaces_and_keeps_other_lines() {
        let d = tmp("upsert");
        let f = d.join(".zshrc");
        std::fs::write(&f, "alias ll='ls -l'\nexport BOARD_TOKEN=bd_old\n  export BOARD_URL=http://old\nexport PATH=$PATH:/x").unwrap();
        upsert_rc(&f, "https://board.buglan.com", TOKEN).unwrap();
        upsert_rc(&f, "https://board.buglan.com", TOKEN).unwrap();
        let s = std::fs::read_to_string(&f).unwrap();
        assert_eq!(
            s,
            format!("alias ll='ls -l'\nexport PATH=$PATH:/x\nexport BOARD_URL='https://board.buglan.com'\nexport BOARD_TOKEN='{TOKEN}'\n")
        );
        // 文件不存在时新建
        let g = d.join(".bashrc");
        upsert_rc(&g, "http://localhost:1420", TOKEN).unwrap();
        assert_eq!(std::fs::read_to_string(&g).unwrap(), format!("export BOARD_URL='http://localhost:1420'\nexport BOARD_TOKEN='{TOKEN}'\n"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[test]
    fn upsert_follows_symlink() {
        let d = tmp("link");
        let real = d.join("dotfiles-zshrc");
        std::fs::write(&real, "export EDITOR=vim\n").unwrap();
        let link = d.join(".zshrc");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        upsert_rc(&link, "https://board.buglan.com", TOKEN).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "仍是软链接");
        assert!(std::fs::read_to_string(&real).unwrap().contains("export BOARD_TOKEN="));
        std::fs::remove_dir_all(&d).ok();
    }
}
