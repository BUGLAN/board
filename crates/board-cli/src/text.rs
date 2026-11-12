//! 对齐、截断用的小工具：按看到的宽度算（中文两个字符宽），不按 char 数。

use unicode_width::UnicodeWidthStr;

/// 显示宽度
pub(crate) fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// 补空格到显示宽度 w（已经够宽时原样返回）
pub(crate) fn pad(s: &str, w: usize) -> String {
    let n = width(s);
    if n >= w { s.to_string() } else { format!("{s}{}", " ".repeat(w - n)) }
}

/// 截到显示宽度 w，多出来的写 …（已经够窄时原样返回）
pub(crate) fn truncate(s: &str, w: usize) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    let mut out = String::new();
    for ch in s.chars() {
        if width(&out) + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) > w - 1 {
            break;
        }
        out.push(ch);
    }
    out + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_by_display_width() {
        assert_eq!(width("卡片"), 4);
        assert_eq!(pad("卡片", 6), "卡片  ");
        assert_eq!(pad("abcdef", 3), "abcdef");
    }

    #[test]
    fn truncates_by_display_width() {
        assert_eq!(truncate("abcdef", 6), "abcdef");
        assert_eq!(truncate("abcdefg", 4), "abc…");
        // 中文按两个字符宽算：宽度 5 放得下两个汉字
        assert_eq!(truncate("卡片列表", 5), "卡片…");
    }
}
