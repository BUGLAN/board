// 界面构建产物（web/dist）变化时重新编译，让网页端带上最新的界面
// 提交号写进可执行文件（BOARD_COMMIT），`/api/health` 返回它，部署后据此核对线上版本：
// 设置了环境变量 BOARD_COMMIT 时用它（GitHub Actions 部署时设置），
// 否则取 git 当前提交，取不到时是 unknown
// skills 共用库里固定的 board-skills 随服务端版本：把 .agents/skills/board-skills 打包进来（跳过 bin/：各平台的 board
// 由各台 board host 用自己的填上），生成 $OUT_DIR/board_skills.rs（[(相对路径, 可执行, 内容)]）
//
// 打包的字节就是各台机器同步到本机的那份，`scripts/board`、`scripts/heartbeat.sh` 还要在 WSL、Linux 里按
// `#!/bin/sh` 执行：行尾必须是 LF。Windows 上检出（`core.autocrlf=true`）可能是 CRLF，
// `.gitattributes` 之外这里再兜一层——打包前换成 LF，免得一份 CRLF 的 skill 同步下去在 WSL 里报
// 「cannot execute: required file not found」。
use std::path::Path;
use std::process::Command;

fn skill_files(root: &Path, dir: &Path, out: &mut Vec<(String, bool, std::path::PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || rel == "bin" {
            continue;
        }
        if p.is_dir() {
            skill_files(root, &p, out);
        } else {
            #[cfg(unix)]
            let exec = {
                use std::os::unix::fs::PermissionsExt;
                p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
            };
            // Windows 上看不出可执行位：没有扩展名的脚本（scripts/board、heartbeat.sh）算可执行
            #[cfg(not(unix))]
            let exec = rel.starts_with("scripts/") && (!name.contains('.') || name.ends_with(".sh"));
            out.push((rel, exec, p));
        }
    }
}

/// sh 脚本、Markdown 的行尾换成 LF（Windows 上检出可能是 CRLF）。`.cmd`、`.ps1` 原样不动
fn normalize(rel: &str, bytes: Vec<u8>) -> Vec<u8> {
    let sh = rel.starts_with("scripts/") && (!rel.contains('.') || rel.ends_with(".sh"));
    if !(sh || rel.ends_with(".md")) {
        return bytes;
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

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

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.agents/skills/board-skills");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = vec![];
    skill_files(&root, &root, &mut files);
    let mut code = String::from("pub const BOARD_SKILLS: &[(&str, bool, &[u8])] = &[\n");
    let out_dir = Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    for (i, (rel, exec, p)) in files.iter().enumerate() {
        println!("cargo:rerun-if-changed={}", p.display());
        let raw = std::fs::read(p).unwrap_or_default();
        let data = normalize(rel, raw.clone());
        if data == raw {
            code += &format!("    ({rel:?}, {exec}, include_bytes!({:?})),\n", p.canonicalize().unwrap().display().to_string());
        } else {
            // 行尾换过（Win 上检出 CRLF、仓库里是 LF）：先写一份到 OUT_DIR，再 include_bytes!
            let f = out_dir.join(format!("skill-{i}.bin"));
            std::fs::write(&f, &data).unwrap();
            code += &format!("    ({rel:?}, {exec}, include_bytes!({:?})),\n", f.display().to_string());
        }
    }
    code += "];\n";
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("board_skills.rs");
    std::fs::write(out, code).unwrap();
}
