//! 定时领取的 cron 表达式（任务下发：定时领取）：标准 5 段「分 时 日 月 周」，支持 `*`、`*/n`、`a`、`a-b`、`a-b/n`
//! 和逗号列表；星期 0 和 7 都是周日。按 board host 机器的本地时间判断。
//!
//! 日和周都写了具体值（都不以 `*` 开头）时，任一个对上就算到点（和常见的 cron 一致）。

/// 解析后的一条表达式：每一段是允许的取值集合（位图）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    min: u64,
    hour: u64,
    dom: u64,
    mon: u64,
    dow: u64,
    dom_any: bool,
    dow_any: bool,
}

const FIELDS: [(&str, u32, u32); 5] = [("分", 0, 59), ("时", 0, 23), ("日", 1, 31), ("月", 1, 12), ("周", 0, 7)];

impl Cron {
    /// 解析；不对时返回写明哪一段不对的原因
    pub fn parse(raw: &str) -> Result<Cron, String> {
        let parts: Vec<&str> = raw.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!("cron 表达式要有 5 段（分 时 日 月 周），「{}」是 {} 段", raw.trim(), parts.len()));
        }
        let mut bits = [0u64; 5];
        for (i, p) in parts.iter().enumerate() {
            let (name, lo, hi) = FIELDS[i];
            bits[i] = field(p, lo, hi).ok_or_else(|| format!("第 {} 段（{name}）不对：「{p}」，要在 {lo}–{hi} 之间", i + 1))?;
        }
        // 周：7 也是周日
        if bits[4] & (1 << 7) != 0 {
            bits[4] = (bits[4] & !(1 << 7)) | 1;
        }
        Ok(Cron {
            min: bits[0],
            hour: bits[1],
            dom: bits[2],
            mon: bits[3],
            dow: bits[4],
            dom_any: parts[2].starts_with('*'),
            dow_any: parts[4].starts_with('*'),
        })
    }

    /// 这一分钟到点了吗。dow：0 是周日
    pub fn matches(&self, min: u32, hour: u32, dom: u32, mon: u32, dow: u32) -> bool {
        let has = |b: u64, v: u32| v < 64 && b & (1 << v) != 0;
        if !(has(self.min, min) && has(self.hour, hour) && has(self.mon, mon)) {
            return false;
        }
        let (d, w) = (has(self.dom, dom), has(self.dow, dow % 7));
        if self.dom_any || self.dow_any {
            d && w
        } else {
            d || w
        }
    }
}

/// 一段：逗号分开的若干项，每项是 `*`、`a`、`a-b`，可以带 `/n`
fn field(raw: &str, lo: u32, hi: u32) -> Option<u64> {
    let mut out = 0u64;
    for item in raw.split(',') {
        let (base, step) = match item.split_once('/') {
            Some((b, s)) => (b, s.parse::<u32>().ok().filter(|n| *n > 0)?),
            None => (item, 1),
        };
        let (a, b) = if base == "*" {
            (lo, hi)
        } else if let Some((a, b)) = base.split_once('-') {
            (a.parse().ok()?, b.parse().ok()?)
        } else {
            let a: u32 = base.parse().ok()?;
            // 「a/n」：从 a 到最大值，每隔 n
            (a, if item.contains('/') { hi } else { a })
        };
        if a < lo || b > hi || a > b {
            return None;
        }
        let mut v = a;
        while v <= b {
            out |= 1 << v;
            v += step;
        }
    }
    Some(out)
}

/// 界面、配置里的一条定时：cron 表达式原样（多余的空白合并），旧的 `HH:MM` 换成等价的 `M H * * *`
pub fn normalize(raw: &str) -> Result<String, String> {
    if let Some(hm) = crate::hosts::parse_hhmm(raw) {
        let (h, m) = hm.split_once(':').unwrap_or_default();
        return Ok(format!("{} {} * * *", m.trim_start_matches('0').parse::<u32>().unwrap_or(0), h.parse::<u32>().unwrap_or(0)));
    }
    Cron::parse(raw)?;
    Ok(raw.split_whitespace().collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_rejects_with_reason() {
        assert!(Cron::parse("*/5 * * * *").is_ok());
        assert!(Cron::parse("0 9 * * 1-5").is_ok());
        assert!(Cron::parse("0,30 8-18/2 1,15 */3 0,7").is_ok());
        let e = Cron::parse("61 * * * *").unwrap_err();
        assert!(e.contains("第 1 段（分）") && e.contains("61"), "{e}");
        let e = Cron::parse("* * *").unwrap_err();
        assert!(e.contains("5 段") && e.contains("3 段"), "{e}");
        for bad in ["* 24 * * *", "* * 0 * *", "* * * 13 *", "* * * * 8", "*/0 * * * *", "5-1 * * * *", "a * * * *"] {
            assert!(Cron::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn matches_minutes_hours_and_weekdays() {
        let every5 = Cron::parse("*/5 * * * *").unwrap();
        assert!(every5.matches(0, 3, 1, 1, 0) && every5.matches(55, 23, 31, 12, 6));
        assert!(!every5.matches(7, 3, 1, 1, 0));
        let workday = Cron::parse("0 9 * * 1-5").unwrap();
        assert!(workday.matches(0, 9, 10, 11, 1), "周一 9 点");
        assert!(!workday.matches(0, 9, 15, 11, 6), "周六不触发");
        assert!(!workday.matches(1, 9, 10, 11, 1));
        // 7 和 0 都是周日
        assert!(Cron::parse("0 0 * * 7").unwrap().matches(0, 0, 9, 11, 0));
        // 日和周都写了：任一个对上就算
        let either = Cron::parse("0 0 1 * 1").unwrap();
        assert!(either.matches(0, 0, 1, 5, 3) && either.matches(0, 0, 9, 5, 1));
        assert!(!either.matches(0, 0, 9, 5, 3));
    }

    #[test]
    fn old_hhmm_becomes_daily_cron() {
        assert_eq!(normalize("03:00").unwrap(), "0 3 * * *");
        assert_eq!(normalize("23:45").unwrap(), "45 23 * * *");
        assert_eq!(normalize("  */5   *  * * * ").unwrap(), "*/5 * * * *");
        assert!(normalize("61 * * * *").is_err());
    }
}
