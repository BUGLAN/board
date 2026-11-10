//! 多行文字一字不丢（卡片 default/main#147）：Windows 上 skill 的入口 board.cmd 要经过 cmd.exe，
//! 参数里的换行会把命令截断在第一行，所以多行内容用 --body-file / --comment-file 从文件传。
//! 这里核对：文件（含 BOM、UTF-16、CRLF）和标准输入传进去的内容与原文逐字一致；skill 的入口脚本把参数原样交给 board。
//! Windows 上 CI 也跑这个文件（.github/workflows/deploy.yml 的 heartbeat-windows），连同 board.cmd。

mod common;

use common::{board_stdin, start, Server};
use std::path::{Path, PathBuf};
use std::process::Command;

const ALPHA: &str = "test/alpha";

/// 换行、空行、列表、代码块、引号、中文标点、行尾空格都要原样保存
const BODY: &str = "来源：「样式动画更新」第 3 条\n\n## 背景\n\n- [ ] 第一条，带 `代码` 和 \"双引号\"、'单引号'\n- [ ] 第二条：100% 完成！\n\n```sh\necho \"a  b\" | grep -v '^#'\n```\n\n> 引用；最后一行  \n结尾";

fn body_of(s: &Server, id: u64) -> String {
    s.run(ALPHA, &["show", &id.to_string(), "--json"]).json()["body"].as_str().unwrap().to_string()
}

fn last_comment(s: &Server, id: u64) -> String {
    let c = s.run(ALPHA, &["show", &id.to_string(), "--json"]).json();
    let acts = c["activity"].as_array().unwrap();
    acts.iter().rev().find(|a| a["type"] == "comment").unwrap()["text"].as_str().unwrap().to_string()
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn body_and_comment_files_keep_every_line() {
    let s = start();
    let dir = tempfile::tempdir().unwrap();
    let utf8 = write(dir.path(), "body.md", BODY.as_bytes());

    // new --body-file
    let c = s.run(ALPHA, &["new", "多行", "--body-file", &utf8, "--json"]).json();
    let id = c["id"].as_u64().unwrap();
    assert_eq!(c["body"], BODY);

    // edit --body-file：带 BOM、CRLF 的文件（Windows 记事本、PowerShell 的 Set-Content -Encoding UTF8）
    let crlf = format!("\u{feff}{}", BODY.replace('\n', "\r\n"));
    let f = write(dir.path(), "crlf.md", crlf.as_bytes());
    assert_eq!(s.run(ALPHA, &["edit", &id.to_string(), "--body-file", &f]).code, 0);
    assert_eq!(body_of(&s, id), BODY);

    // UTF-16LE 带 BOM（Windows PowerShell 5.1 的 `>`、Out-File 默认）
    let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain(BODY.encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
    let f = write(dir.path(), "utf16.md", &le);
    s.run(ALPHA, &["edit", &id.to_string(), "--body", "x"]);
    assert_eq!(s.run(ALPHA, &["edit", &id.to_string(), "--body-file", &f]).code, 0);
    assert_eq!(body_of(&s, id), BODY);

    // comment --comment-file
    let r = s.run(ALPHA, &["comment", &id.to_string(), "--comment-file", &utf8]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(last_comment(&s, id), BODY);

    // attach --comment-file：说明在前，附件的 Markdown 跟在后面
    let png = write(dir.path(), "a.txt", b"log");
    let r = s.run(ALPHA, &["attach", &id.to_string(), &png, "--comment-file", &utf8]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(last_comment(&s, id).starts_with(BODY.trim()), "{}", last_comment(&s, id));

    // - 读标准输入
    let mut env = s.env();
    env.push(("BOARD_AS", ALPHA.into()));
    env.push(("BOARD", "default".into()));
    let r = board_stdin(&s.home, &env, &["new", "标准输入", "--body-file", "-", "--json"], Some(BODY));
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(r.json()["body"], BODY);
}

#[test]
fn text_and_file_cannot_both_be_given() {
    let s = start();
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "b.md", b"x");
    s.run(ALPHA, &["new", "一"]);
    for args in [
        vec!["new", "二", "--body", "a", "--body-file", f.as_str(), "--json"],
        vec!["edit", "1", "--body", "a", "--body-file", f.as_str(), "--json"],
        vec!["comment", "1", "进展", "--comment-file", f.as_str(), "--json"],
        vec!["new", "三", "--body-file", "/no/such/file", "--json"],
    ] {
        let r = s.run(ALPHA, &args);
        assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"), "{args:?}: {}{}", r.out, r.err);
    }
    // GBK 之类的非 UTF-8 文件：拒绝，不乱码写进去
    let gbk = write(dir.path(), "gbk.md", &[0xC4, 0xE3, 0xBA, 0xC3]);
    let r = s.run(ALPHA, &["edit", "1", "--body-file", &gbk, "--json"]);
    assert_eq!((r.code, r.error_code().as_str()), (1, "USAGE"));
}

/// skill 的入口脚本（scripts/board、Windows 上 scripts\board.cmd）：BOARD_EXE 指向测试编出的 board
fn skill_script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.agents/skills/board-skills/scripts").join(name)
}

fn via_script(s: &Server, script: &Path, args: &[&str]) -> (i32, String, String) {
    let mut cmd = if cfg!(windows) {
        // Rust 调 .cmd 时自己经过 cmd.exe，参数按批处理的规则转义（带换行的参数它会直接拒绝）
        Command::new(script)
    } else {
        let mut c = Command::new("sh");
        c.arg(script);
        c
    };
    let o = cmd
        .args(args)
        .env("BOARD_EXE", env!("CARGO_BIN_EXE_board"))
        .env("HOME", &s.home)
        .env("USERPROFILE", &s.home)
        .env("NO_COLOR", "1")
        .env("BOARD_URL", &s.url)
        .env("BOARD_TOKEN", &s.token)
        .env("BOARD_AS", ALPHA)
        .env("BOARD", "default")
        .output()
        .unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

#[test]
fn skill_entry_passes_body_file_through() {
    let s = start();
    s.check(ALPHA);
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "body.md", BODY.as_bytes());
    let script = skill_script(if cfg!(windows) { "board.cmd" } else { "board" });
    let (code, out, err) = via_script(&s, &script, &["new", "via-entry-script", "--body-file", &f, "--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let c: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(c["body"], BODY);
    // 版本号也原样交过去（发版后核对 skill 里的 board）
    let (code, out, _) = via_script(&s, &script, &["--version"]);
    assert_eq!((code, out.trim().to_string()), (0, format!("board {}", env!("CARGO_PKG_VERSION"))));
}

/// sh 入口：直接写在参数里的多行正文也原样交过去（macOS、Linux、Git Bash 用 scripts/board 时）
#[cfg(unix)]
#[test]
fn sh_entry_keeps_inline_multiline_body() {
    let s = start();
    s.check(ALPHA);
    let (code, out, err) = via_script(&s, &skill_script("board"), &["new", "直接写", "--body", BODY, "--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let c: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(c["body"], BODY);
}
