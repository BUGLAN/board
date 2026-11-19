//! Board 作为 git askpass 被启动时必须立即失败，不能进入正常命令或弹出窗口。

use std::process::Command;

#[test]
fn askpass_mode_exits_one_without_output() {
    let out = Command::new(env!("CARGO_BIN_EXE_board"))
        .arg("Username for 'https://example.com':")
        .env("BOARD_INTERNAL_GIT_ASKPASS", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty() && out.stderr.is_empty(), "stdout={:?} stderr={:?}", out.stdout, out.stderr);
}
