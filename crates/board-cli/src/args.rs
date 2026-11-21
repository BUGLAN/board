//! 命令行参数：认哪些参数、怎么解析成 Args；`--body` / `--comment` 这类文字直接写或从文件读

use board_core::*;
use std::collections::{HashMap, HashSet};

pub(crate) const VALUE_FLAGS: [&str; 31] = [
    "--as", "--board", "--column", "--search", "--label", "--body", "--title", "--comment", "--out", "--url", "--token", "--name", "--dir",
    "--host", "--agent", "--profile", "--body-file", "--comment-file", "--image", "--file", "--limit", "--wait", "--from", "--project", "--workspace",
    "--base", "--upstream", "--card", "--message-file", "--checked", "--branch",
];
pub(crate) const BOOL_FLAGS: [&str; 20] = [
    "--json", "--top", "--inbox", "--no-code", "--help", "-h", "--final", "--cancel", "--any", "--move", "--version", "-V", "--force", "--latest", "--install", "--uninstall", "--service",
    "--dry-run", "--touched-ok", "--wip",
];

/* ------------------------------ 参数 ------------------------------ */

pub(crate) struct Args {
    pub(crate) values: HashMap<String, String>,
    /// 可以写多次的参数（--dir）：按出现的顺序
    pub(crate) many: Vec<(String, String)>,
    pub(crate) bools: HashSet<String>,
    pub(crate) pos: Vec<String>,
}

impl Args {
    pub(crate) fn parse(tokens: &[String]) -> Result<Args> {
        let mut a = Args { values: HashMap::new(), many: vec![], bools: HashSet::new(), pos: vec![] };
        let mut i = 0;
        while i < tokens.len() {
            let t = &tokens[i];
            if t.starts_with("--") && t.contains('=') {
                let (k, v) = t.split_once('=').unwrap_or_default();
                if !VALUE_FLAGS.contains(&k) {
                    return Err(BoardError::usage(format!("未知参数 {k}")));
                }
                a.values.insert(k.to_string(), v.to_string());
                a.many.push((k.to_string(), v.to_string()));
            } else if VALUE_FLAGS.contains(&t.as_str()) {
                let v = tokens.get(i + 1).ok_or_else(|| BoardError::usage(format!("{t} 需要一个值")))?;
                a.values.insert(t.clone(), v.clone());
                a.many.push((t.clone(), v.clone()));
                i += 1;
            } else if BOOL_FLAGS.contains(&t.as_str()) {
                a.bools.insert(t.clone());
            } else if t.starts_with("--") {
                return Err(BoardError::usage(format!("未知参数 {t}，运行 board help 查看用法")));
            } else {
                a.pos.push(t.clone());
            }
            i += 1;
        }
        Ok(a)
    }
    pub(crate) fn value(&self, k: &str) -> Option<&str> {
        self.values.get(k).map(|s| s.as_str())
    }
    pub(crate) fn flag(&self, k: &str) -> bool {
        self.bools.contains(k)
    }
    /// 写了多次的参数的全部值
    pub(crate) fn all(&self, k: &str) -> Vec<String> {
        self.many.iter().filter(|(x, _)| x == k).map(|(_, v)| v.clone()).collect()
    }
    pub(crate) fn id(&self, sub: &str) -> Result<i64> {
        let raw = self.pos.get(1).ok_or_else(|| BoardError::usage(format!("board {sub} 需要 <ID>")))?;
        raw.trim_start_matches('#')
            .parse::<i64>()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| BoardError::usage(format!("无效的 ID「{raw}」")))
    }
}

/* ------------------------------ 从文件读多行文字 ------------------------------ */

/// `--body` / `--comment` 的值：直接写的，或者 `--body-file` / `--comment-file` 从文件读的（两个只能给一个）。
/// Windows 上 skill 的入口 board.cmd 要经过 cmd.exe，参数里的换行会把整条命令截断在第一行（批处理里补不回来），
/// 多行内容只能这样传
pub(crate) fn text_arg(a: &Args, flag: &str, file_flag: &str) -> Result<Option<String>> {
    match (a.value(flag), file_arg(a, file_flag)?) {
        (Some(_), Some(_)) => Err(BoardError::usage(format!("{flag} 和 {file_flag} 只能给一个"))),
        (Some(v), None) => Ok(Some(v.to_string())),
        (None, t) => Ok(t),
    }
}

/// `--xxx-file <文件>` 的内容；`-` 读标准输入
pub(crate) fn file_arg(a: &Args, file_flag: &str) -> Result<Option<String>> {
    let Some(f) = a.value(file_flag) else { return Ok(None) };
    let bytes = if f == "-" {
        let mut b = vec![];
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut b).map_err(|e| BoardError::usage(format!("读不到标准输入：{e}")))?;
        b
    } else {
        std::fs::read(f).map_err(|e| BoardError::usage(format!("{file_flag}：读不到文件 {f}：{e}")))?
    };
    decode_text(&bytes).map(Some).map_err(|e| BoardError::usage(format!("{file_flag} {f}：{e}")))
}

/// 文件内容按 UTF-8 读：去掉 BOM；带 BOM 的 UTF-16（Windows PowerShell 5.1 的 `>`、Out-File 默认写成这样）也认；
/// 换行统一成 \n，别的字符原样保留
pub(crate) fn decode_text(b: &[u8]) -> std::result::Result<String, String> {
    let utf16 = |be: bool| {
        let units: Vec<u16> = b[2..].chunks(2).map(|c| if be { u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) } else { u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)]) }).collect();
        String::from_utf16(&units).map_err(|_| "不是合法的 UTF-16".to_string())
    };
    let s = match b {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).map_err(|_| "不是合法的 UTF-8".to_string())?,
        [0xFF, 0xFE, ..] => utf16(false)?,
        [0xFE, 0xFF, ..] => utf16(true)?,
        _ => String::from_utf8(b.to_vec()).map_err(|_| "不是 UTF-8 编码（另存为 UTF-8 再试）".to_string())?,
    };
    Ok(s.replace("\r\n", "\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_text_handles_bom_utf16_and_crlf() {
        assert_eq!(decode_text("一\r\n\r\n二".as_bytes()).unwrap(), "一\n\n二");
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, b'a', b'\n']).unwrap(), "a\n");
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("中\r\nb".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        assert_eq!(decode_text(&le).unwrap(), "中\nb");
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("中".encode_utf16().flat_map(|u| u.to_be_bytes())).collect();
        assert_eq!(decode_text(&be).unwrap(), "中");
        assert!(decode_text(&[0xC4, 0xE3]).is_err(), "GBK 不当成 UTF-8 乱读");
    }
}
