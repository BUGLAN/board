//! Board 内部执行 git 时共用的非交互环境。
//!
//! `GIT_TERMINAL_PROMPT=0` 只关闭终端提问；Windows 上的 Git Credential Manager 和 askpass
//! 仍可能打开图形窗口。所有 Board 自己跑的 git 都用 [`command`] 创建，把 askpass 指向一个
//! 立即退出 1 的临时脚本；脚本创建不了时退回 board 自身，`main` 看到 [`ASKPASS_MARKER`]
//! 后立即退出 1。

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// 区分“作为 askpass 被 git 启动”和正常执行 board。只在 Board 创建 git 命令时设置
pub const ASKPASS_MARKER: &str = "BOARD_INTERNAL_GIT_ASKPASS";

/// 创建一个不会停下来等人的 git 命令
pub fn command() -> Command {
    let mut c = Command::new("git");
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("SSH_ASKPASS_REQUIRE", "force");
    let helper = askpass();
    c.env("GIT_ASKPASS", &helper).env("SSH_ASKPASS", helper).env(ASKPASS_MARKER, "1");
    c
}

fn askpass() -> PathBuf {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER
        .get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("board-git-askpass-{}", std::process::id()));
            let name = if cfg!(windows) { "askpass.cmd" } else { "askpass.sh" };
            let path = dir.join(name);
            let body = if cfg!(windows) { "@exit /b 1\r\n" } else { "#!/bin/sh\nexit 1\n" };
            let made = std::fs::create_dir_all(&dir)
                .and_then(|_| std::fs::write(&path, body))
                .and_then(|_| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
                    }
                    Ok(())
                })
                .is_ok();
            if made {
                return path;
            }
            // 脚本也建不了时仍要压住系统 askpass：让 board 自己作为 helper 立即失败
            std::env::current_exe().unwrap_or_else(|_| {
                std::env::temp_dir().join(format!("board-git-askpass-missing-{}", std::process::id()))
            })
        })
        .clone()
}

/// 当前进程是不是 git 启动的 askpass helper
pub fn is_askpass() -> bool {
    std::env::var_os(ASKPASS_MARKER).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::ffi::OsStr;

    fn env(c: &Command, key: &str) -> Option<OsString> {
        c.get_envs().find(|(k, _)| *k == OsStr::new(key)).and_then(|(_, v)| v).map(OsStr::to_os_string)
    }

    #[test]
    fn every_board_git_command_disables_interactive_prompts() {
        let c = command();
        assert_eq!(env(&c, "GIT_TERMINAL_PROMPT").as_deref(), Some(OsStr::new("0")));
        assert_eq!(env(&c, "GCM_INTERACTIVE").as_deref(), Some(OsStr::new("never")));
        assert_eq!(env(&c, "SSH_ASKPASS_REQUIRE").as_deref(), Some(OsStr::new("force")));
        assert_eq!(env(&c, ASKPASS_MARKER).as_deref(), Some(OsStr::new("1")));
        assert_eq!(env(&c, "GIT_ASKPASS"), env(&c, "SSH_ASKPASS"));
        let helper = env(&c, "GIT_ASKPASS").expect("askpass helper");
        let body = std::fs::read_to_string(&helper).expect("临时 askpass 脚本");
        assert!(body.contains("exit") && body.contains('1'), "{body:?}");
    }
}
