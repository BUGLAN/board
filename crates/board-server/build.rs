// 界面构建产物（web/dist）变化时重新编译，让网页端带上最新的界面
// 提交号写进可执行文件（BOARD_COMMIT），`/api/health` 返回它，部署后据此核对线上版本：
// 设置了环境变量 BOARD_COMMIT 时用它（GitHub Actions 部署时设置），
// 否则取 git 当前提交，取不到时是 unknown
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn main() {
    println!("cargo:rerun-if-changed=../../web/dist");
    println!("cargo:rerun-if-env-changed=BOARD_COMMIT");
    let commit = match std::env::var("BOARD_COMMIT").ok().filter(|s| !s.trim().is_empty()) {
        Some(c) => c.trim().to_string(),
        None => {
            // 换提交、切分支时重新取
            if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
                println!("cargo:rerun-if-changed={dir}/HEAD");
                println!("cargo:rerun-if-changed={dir}/refs/heads");
                println!("cargo:rerun-if-changed={dir}/packed-refs");
            }
            git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".into())
        }
    };
    println!("cargo:rustc-env=BOARD_COMMIT={commit}");
}
