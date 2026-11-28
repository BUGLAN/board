//! Board 内部执行 git 时共用的非交互环境。
//!
//! `GIT_TERMINAL_PROMPT=0` 只关闭终端提问；Windows 上的 Git Credential Manager 和 askpass
//! 仍可能打开图形窗口。所有 Board 自己跑的 git 都用 [`command`] 创建，把 askpass 指向一个随机私有目录里、
//! 立即退出 1 的临时脚本；进程退出时删掉这个目录。脚本创建不了时退回 board 自身，`main` 看到
//! [`ASKPASS_MARKER`] 后立即退出 1。

use board_core::private;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Once, OnceLock};

/// 区分“作为 askpass 被 git 启动”和正常执行 board。只在 Board 创建 git 命令时设置
pub const ASKPASS_MARKER: &str = "BOARD_INTERNAL_GIT_ASKPASS";

/// 创建一个不会停下来等人的 git 命令
pub fn command() -> Command {
    let mut c = Command::new(program());
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("SSH_ASKPASS_REQUIRE", "force");
    let helper = askpass();
    c.env("GIT_ASKPASS", &helper).env("SSH_ASKPASS", helper).env(ASKPASS_MARKER, "1");
    c
}

#[cfg(not(test))]
fn program() -> std::ffi::OsString {
    "git".into()
}

#[cfg(test)]
thread_local! {
    /// 测试里换成假的 git（例如卡住不动的）：只影响当前测试线程
    static PROGRAM: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn program() -> std::ffi::OsString {
    PROGRAM.with(|p| p.borrow().clone()).map(Into::into).unwrap_or_else(|| "git".into())
}

/// 测试用：`f` 里本线程跑的 git 都换成 `git`
#[cfg(test)]
pub fn with_program<T>(git: &Path, f: impl FnOnce() -> T) -> T {
    PROGRAM.with(|p| *p.borrow_mut() = Some(git.to_path_buf()));
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    PROGRAM.with(|p| *p.borrow_mut() = None);
    out.unwrap_or_else(|e| std::panic::resume_unwind(e))
}

fn askpass() -> PathBuf {
    HELPER
        .get_or_init(|| {
            register_cleanup();
            Askpass::new().ok()
        })
        .as_ref()
        .map(|a| a.path.clone())
        .unwrap_or_else(|| {
            // 脚本也建不了时仍要压住系统 askpass：让 board 自己作为 helper 立即失败
            std::env::current_exe().unwrap_or_else(|_| std::env::temp_dir().join("board-git-askpass-missing"))
        })
}

struct Askpass {
    dir: PathBuf,
    path: PathBuf,
}

impl Askpass {
    fn new() -> io::Result<Askpass> {
        Self::new_in(&std::env::temp_dir())
    }

    fn new_in(base: &Path) -> io::Result<Askpass> {
        let temp = tempfile::Builder::new().prefix("board-git-askpass-").tempdir_in(base)?;
        let dir = temp.keep();
        let made = (|| {
            crate::securetmp::ensure_private_dir(&dir)?;
            let name = if cfg!(windows) { "askpass.cmd" } else { "askpass.sh" };
            let path = dir.join(name);
            let body = if cfg!(windows) { "@exit /b 1\r\n" } else { "#!/bin/sh\nexit 1\n" };
            private::write_new(&path, body.as_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(Askpass { dir: dir.clone(), path })
        })();
        if made.is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        made
    }

    fn remove(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for Askpass {
    fn drop(&mut self) {
        self.remove();
    }
}

/// 删除当前进程的 askpass 目录。正常退出时也会由 C 运行库的退出处理再调用一次，重复调用无副作用。
pub(crate) fn cleanup() {
    if let Some(Some(a)) = HELPER.get() {
        a.remove();
    }
}

static HELPER: OnceLock<Option<Askpass>> = OnceLock::new();

#[cfg(unix)]
extern "C" fn cleanup_at_exit() {
    cleanup();
}

fn register_cleanup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        #[cfg(unix)]
        unsafe {
            libc::atexit(cleanup_at_exit);
        }
    });
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

    #[test]
    fn askpass_uses_a_random_private_dir_and_cleans_it_up() {
        let tmp = tempfile::tempdir().unwrap();
        let fixed = tmp.path().join(format!("board-git-askpass-{}", std::process::id()));
        std::fs::create_dir(&fixed).unwrap();
        std::fs::write(fixed.join("askpass.sh"), b"sentinel").unwrap();

        let helper = Askpass::new_in(tmp.path()).unwrap();
        assert_ne!(helper.dir, fixed, "不接收别人预建的固定目录");
        assert!(helper.dir.file_name().unwrap().to_string_lossy().starts_with("board-git-askpass-"));
        assert!(helper.path.is_file());
        assert_eq!(std::fs::read(fixed.join("askpass.sh")).unwrap(), b"sentinel");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&helper.dir).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(std::fs::metadata(&helper.path).unwrap().permissions().mode() & 0o777, 0o700);
        }

        let dir = helper.dir.clone();
        drop(helper);
        assert!(!dir.exists(), "askpass 目录在进程结束时删掉");
    }
}
