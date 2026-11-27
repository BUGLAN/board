use board_core::hosts::{Quota, QuotaWindow, WINDOW_FIVE_HOUR, WINDOW_SEVEN_DAY};
use serde_json::Value;

/* ------------------------------ 额度（#668） ------------------------------ */

/// 额度用完的报错文字以这些开头：Claude Agent SDK 的 `USAGE_LIMIT_ERROR_PREFIXES`（0.3.331），Codex 的「You've hit your
/// usage limit」也以第一个开头
const USAGE_LIMIT_PREFIXES: &[&str] = &[
    "You've hit your",
    "You've reached your",
    "You're out of usage credits",
    "Your org is out of usage · add funds to continue",
    "Your org is out of usage · contact your admin",
    "Your seat type doesn't include usage credits",
    "Your seat type doesn't include usage",
    "Your usage allocation has been disabled by your admin",
    "Your group's usage limit is set to $0",
    "Fable 5 requires usage credits",
    "You're out of extra usage",
    "Your seat type doesn't include extra usage",
];

/// Agent SDK 的 `rate_limit_info`（`{status, resetsAt, rateLimitType, utilization, …}`）→ 额度；不是对象、没有状态时为 None。
/// `resetsAt` 是 Unix 秒（看着像毫秒的照用），`utilization` 是 0–1 的比例（大于 1 的当百分数）。现在的 Claude Code 把每个
/// 窗口的用量放在 `unifiedWindows.<窗口>` 里（`{status, resetsAt, utilization}`），顶层没有 `utilization`：顶层没有的从
/// `rateLimitType` 那个窗口里取（本机端到端实测）
pub(super) fn rate_of(v: &Value) -> Option<Quota> {
    let status = v["status"].as_str().filter(|s| !s.is_empty())?;
    let window = v["rateLimitType"].as_str().unwrap_or_default().to_string();
    let unified = &v["unifiedWindows"][window.as_str()];
    let num = |k: &str| v[k].as_f64().or_else(|| unified[k].as_f64());
    let resets_at = reset_ms(num("resetsAt"));
    let used = num("utilization").filter(|u| u.is_finite() && *u >= 0.0).map(|u| if u <= 1.0 { (u * 100.0).round() as i64 } else { u.round() as i64 });
    // 5 小时、本周两个窗口：`unifiedWindows` 里的 utilization 总是比例（用超了会大于 1）
    let windows = [WINDOW_FIVE_HOUR, WINDOW_SEVEN_DAY]
        .into_iter()
        .filter_map(|k| {
            let w = &v["unifiedWindows"][k];
            let used = w["utilization"].as_f64().filter(|u| u.is_finite() && *u >= 0.0).map(|u| (u * 100.0).round() as i64);
            w.is_object().then(|| QuotaWindow { key: k.into(), used, resets_at: reset_ms(w["resetsAt"].as_f64()), status: w["status"].as_str().unwrap_or_default().into() })
        })
        .collect();
    Some(Quota { status: status.to_string(), window, used, resets_at, windows, ..Default::default() })
}


/// 重置时间：Unix 秒（看着像毫秒的照用）→ 毫秒
fn reset_ms(t: Option<f64>) -> Option<i64> {
    t.filter(|t| *t > 0.0).map(|t| if t < 1e11 { (t * 1000.0) as i64 } else { t as i64 })
}

/* ------------------------------ 查额度（`--usage`） ------------------------------ */

/// 本地命令查到的额度文字 → 5 小时、本周两个窗口。认的行（只认这两个窗口，按模型的周额度不要）：
/// - Claude Code 的 `/usage`：「Current session: 43% used · resets Nov 25 at 10:59pm (Europe/Berlin)」
///   「Current week (all models): 58% used · resets Nov 27 at 5:59pm (Europe/Berlin)」
/// - Codex（codex-acp）的 `/status`：「**codex 5h limit:** 88% left (resets 15:30)」「**codex Weekly limit:** 14% left (resets 15:30 on 3 Dec)」
///
/// 时间按本机时区读（agent 和 board acp 在同一台机器上）。一个窗口都认不出时为 None
pub(super) fn usage_of_text(text: &str, now: chrono::DateTime<chrono::Local>) -> Option<Quota> {
    let mut windows: Vec<QuotaWindow> = vec![];
    for line in text.lines() {
        let l = line.replace("**", "").to_lowercase();
        let key = if l.contains("current session") || l.contains("5h limit") || l.contains("5-hour limit") {
            WINDOW_FIVE_HOUR
        } else if l.contains("current week (all models)") || l.contains("weekly limit") || l.contains("weekly · all models") {
            WINDOW_SEVEN_DAY
        } else {
            continue;
        };
        if windows.iter().any(|w| w.key == key) {
            continue;
        }
        let Some(used) = pct_of(&l) else { continue };
        let resets_at = l.find("resets ").and_then(|i| l[i + "resets ".len()..].split(['(', ')']).next()).and_then(|r| reset_at(r, now));
        windows.push(QuotaWindow { key: key.into(), used: Some(used), resets_at, status: String::new() });
    }
    windows.sort_by_key(|w| w.key != WINDOW_FIVE_HOUR);
    let first = windows.first()?.clone();
    Some(Quota { window: first.key, used: first.used, resets_at: first.resets_at, windows, ..Default::default() })
}


/// 「43% used」→ 43；「88% left」→ 12
fn pct_of(l: &str) -> Option<i64> {
    for (pat, left) in [("% used", false), ("% left", true)] {
        if let Some(i) = l.find(pat) {
            let start = l[..i].rfind(|c: char| !(c.is_ascii_digit() || c == '.')).map_or(0, |j| j + 1);
            let n = l[start..i].parse::<f64>().ok()?.round() as i64;
            return Some(if left { 100 - n } else { n }.max(0));
        }
    }
    None
}


/// 重置时间的文字 → 毫秒时间戳（本机时区）：「nov 25 at 10:59pm」「10:59pm」「5pm」「15:30」「15:30 on 3 dec」「15:30 on dec 3」
/// 「15:30 on 12月3日」。只有时刻的是今天的，已经过了就当明天的；有月日没年份、算出来已经过去一天以上的当明年的
fn reset_at(s: &str, now: chrono::DateTime<chrono::Local>) -> Option<i64> {
    use chrono::{Datelike, NaiveDate, NaiveTime, TimeZone, Timelike};
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let (mut time, mut month, mut day, mut year): (Option<NaiveTime>, Option<u32>, Option<u32>, Option<i32>) = (None, None, None, None);
    for tok in s.replace([',', '·'], " ").split_whitespace() {
        match tok {
            "at" | "on" => {}
            "pm" => time = time.map(|t| if t.hour() < 12 { t + chrono::Duration::hours(12) } else { t }),
            "am" => time = time.map(|t| if t.hour() == 12 { t - chrono::Duration::hours(12) } else { t }),
            _ if tok.contains('月') => {
                let (m, d) = tok.split_once('月')?;
                month = m.parse().ok();
                day = d.trim_end_matches('日').parse().ok();
            }
            _ => {
                if let Some(t) = clock_of(tok) {
                    time = Some(t);
                } else if let Some(m) = MONTHS.iter().position(|m| tok.starts_with(m)) {
                    month = Some(m as u32 + 1);
                } else if let Ok(n) = tok.trim_end_matches(|c: char| c.is_alphabetic()).parse::<u32>() {
                    if n >= 1000 {
                        year = Some(n as i32);
                    } else if (1..=31).contains(&n) {
                        day = Some(n);
                    }
                }
            }
        }
    }
    let time = time?;
    let local = |d: NaiveDate| chrono::Local.from_local_datetime(&d.and_time(time)).earliest().map(|t| t.timestamp_millis());
    let now_ms = now.timestamp_millis();
    match (month, day) {
        (Some(m), Some(d)) => {
            let ms = local(NaiveDate::from_ymd_opt(year.unwrap_or(now.year()), m, d)?)?;
            if year.is_none() && ms < now_ms - 24 * 3600 * 1000 {
                return local(NaiveDate::from_ymd_opt(now.year() + 1, m, d)?);
            }
            Some(ms)
        }
        _ => {
            let ms = local(now.date_naive())?;
            Some(if ms < now_ms { ms + 24 * 3600 * 1000 } else { ms })
        }
    }
}


/// 一个时刻：「10:59pm」「5pm」「15:30」「10:59」（后面单独的 am / pm 另算）
fn clock_of(tok: &str) -> Option<chrono::NaiveTime> {
    let (body, pm) = match (tok.strip_suffix("pm"), tok.strip_suffix("am")) {
        (Some(b), _) => (b, Some(true)),
        (_, Some(b)) => (b, Some(false)),
        _ => (tok, None),
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None if pm.is_some() => (body.parse::<u32>().ok()?, 0),
        None => return None,
    };
    let h = match pm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    chrono::NaiveTime::from_hms_opt(h, m, 0)
}


/// 这一轮的错误是不是额度用完；是的话返回记上用完、重置时间的额度（`seen` 是之前 `usage_update` 带的）。
/// 认的是：Claude 的 `data.errorKind` 为 `rate_limit` 且额度状态是 `rejected`（一时的 429 是 `allowed`，不算）；
/// Codex 的 `data.codexErrorInfo` 为 `usageLimitExceeded`（Zed 版：`codex_error_info` 为 `usage_limit_exceeded`）；
/// 或者报错文字以 [`USAGE_LIMIT_PREFIXES`] 开头。
/// 重置时间先用额度信息里的，没有时从「Try again at …」里读（Codex，本机时区）
pub(super) fn quota_of(error: &Value, seen: Option<&Quota>, now: chrono::DateTime<chrono::Local>) -> Option<Quota> {
    let data = &error["data"];
    let texts: Vec<&str> = [error["message"].as_str(), data["message"].as_str(), data.as_str()].into_iter().flatten().map(str::trim).filter(|t| !t.is_empty()).collect();
    let rate = rate_of(&data["rateLimit"]).or_else(|| seen.cloned());
    let rejected = rate.as_ref().is_some_and(|r| r.status == "rejected");
    let claude = data["errorKind"].as_str() == Some("rate_limit") && rejected;
    let codex = data["codexErrorInfo"].as_str() == Some("usageLimitExceeded") || data["codex_error_info"].as_str() == Some("usage_limit_exceeded");
    let words = texts.iter().any(|t| USAGE_LIMIT_PREFIXES.iter().any(|p| t.starts_with(p)));
    if !(claude || codex || words) {
        return None;
    }
    // 给人看的原文：最长的那段（Claude 是 message，Codex 是 data.message；message 有时只是「Internal error」）
    let text = texts.iter().max_by_key(|t| t.chars().count()).map(|t| t.to_string()).unwrap_or_default();
    let mut q = rate.filter(|_| rejected).unwrap_or_default();
    q.exhausted = true;
    if q.resets_at.is_none() {
        q.resets_at = texts.iter().find_map(|t| try_again_at(t, now));
    }
    q.text = text.chars().take(300).collect();
    Some(q)
}


/// Codex 的「… Try again at 3:15 PM.」「… or try again at Nov 26th, 2026 3:15 PM.」→ 毫秒时间戳（本机时区，和 Codex
/// 写的时候一样）。只有时刻的是今天的，已经过了就当明天的
fn try_again_at(text: &str, now: chrono::DateTime<chrono::Local>) -> Option<i64> {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
    let lower = text.to_ascii_lowercase();
    let at = lower.find("try again at ")? + "try again at ".len();
    let rest = text.get(at..)?.trim();
    let rest = rest.split_once(". ").map_or(rest, |(a, _)| a).trim_end_matches('.').trim();
    let local = |n: NaiveDateTime| chrono::Local.from_local_datetime(&n).earliest().map(|t| t.timestamp_millis());
    if let Ok(t) = NaiveTime::parse_from_str(rest, "%I:%M %p") {
        let today = now.date_naive().and_time(t);
        let ms = local(today)?;
        return Some(if ms < now.timestamp_millis() { ms + 24 * 3600 * 1000 } else { ms });
    }
    // 「Nov 26th, 2026 3:15 PM」：去掉日子后面的 st / nd / rd / th
    let (date, time) = rest.split_once(", ").and_then(|(md, yt)| yt.split_once(' ').map(|(y, t)| (format!("{} {y}", md.trim_end_matches(char::is_alphabetic)), t.to_string())))?;
    let d = NaiveDate::parse_from_str(&date, "%b %d %Y").ok()?;
    let t = NaiveTime::parse_from_str(&time, "%I:%M %p").ok()?;
    local(d.and_time(t))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::client::{exit_for, Client, Outcome};
    use crate::acp::testutil::{reply, s, state};
    use crate::acp::{parse, EXIT_QUOTA};
    use chrono::TimeZone;
    use serde_json::json;

    fn usage(session: &str, rate: Value) -> Value {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": { "sessionUpdate": "usage_update", "used": 10, "size": 200000, "_meta": { "_claude/rateLimit": rate } } } })
    }


    /// 走到这一轮开始干活：v2（Claude Code）或 v1（Codex）
    fn working(v2: bool) -> Client {
        let mut c = Client::new("/w", "p", None);
        c.start();
        c.on_frame(&reply(0, json!({ "protocolVersion": if v2 { 2 } else { 1 } })));
        c.on_frame(&reply(1, json!({ "sessionId": "s1" })));
        if v2 {
            c.on_frame(&reply(2, json!({ "messageId": "m1" })));
            c.on_frame(&state("s1", json!({ "state": "running" })));
        }
        c
    }

    /// 额度（#668）：Claude 跑的时候 usage_update 带的额度记下来；额度用完（errorKind rate_limit、状态 rejected）退出码 4，
    /// 带上重置时间；一时的 429（状态 allowed）照旧算出错
    #[test]
    fn claude_usage_limit_is_its_own_outcome() {
        let mut c = working(true);
        c.on_frame(&usage("s1", json!({ "status": "allowed_warning", "resetsAt": 1795000000, "rateLimitType": "five_hour", "utilization": 0.91 })));
        let q = c.quota.clone().unwrap();
        assert_eq!((q.status.as_str(), q.window.as_str(), q.used, q.resets_at, q.exhausted), ("allowed_warning", "five_hour", Some(91), Some(1_795_000_000_000), false));
        let rate = json!({ "status": "rejected", "resetsAt": 1795003600, "rateLimitType": "five_hour", "utilization": 1.0 });
        let err = json!({ "code": -32603, "message": "You've hit your session limit · resets 3pm", "data": { "errorKind": "rate_limit", "rateLimit": rate } });
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": err })));
        assert!(matches!(c.outcome, Some(Outcome::Quota(ref w)) if w.contains("You've hit your session limit")), "{:?}", c.outcome);
        assert_eq!(exit_for(c.outcome.as_ref().unwrap()), EXIT_QUOTA);
        let q = c.quota.clone().unwrap();
        assert_eq!((q.exhausted, q.resets_at, q.used, q.text.as_str()), (true, Some(1_795_003_600_000), Some(100), "You've hit your session limit · resets 3pm"));
        // 现在的 Claude Code：用量在 unifiedWindows.<窗口> 里（本机端到端实测的样子）
        let mut c = working(true);
        c.on_frame(&usage("s1", json!({ "status": "allowed", "rateLimitType": "five_hour", "resetsAt": 1795676400, "overageStatus": "rejected",
            "unifiedWindows": { "five_hour": { "status": "allowed", "resetsAt": 1795676400, "utilization": 0.3 } } })));
        let q = c.quota.clone().unwrap();
        assert_eq!((q.window.as_str(), q.used, q.resets_at), ("five_hour", Some(30), Some(1_795_676_400_000)));
        // 两个窗口都记下（2026-11-25 一次真实派发推来的样子）；用超了（utilization 大于 1）照比例算
        let mut c = working(true);
        c.on_frame(&usage("s1", json!({ "status": "allowed", "rateLimitType": "five_hour", "resetsAt": 1795676400,
            "unifiedWindows": { "five_hour": { "status": "allowed", "resetsAt": 1795676400, "utilization": 0.39 },
                "seven_day": { "status": "allowed_warning", "resetsAt": 1795831200, "utilization": 1.02 },
                "seven_day_overage_included": { "resetsAt": 1795831200, "utilization": 0.03 } } })));
        let w: Vec<(String, Option<i64>, Option<i64>, String)> = c.quota.clone().unwrap().windows.into_iter().map(|w| (w.key, w.used, w.resets_at, w.status)).collect();
        assert_eq!(w, vec![
            ("five_hour".into(), Some(39), Some(1_795_676_400_000), "allowed".into()),
            ("seven_day".into(), Some(102), Some(1_795_831_200_000), "allowed_warning".into()),
        ]);
        // 一时的 429：状态 allowed，文字也不是额度用完
        let mut c = working(true);
        let err = json!({ "code": -32603, "message": "API Error: 429 overloaded", "data": { "errorKind": "rate_limit", "rateLimit": { "status": "allowed" } } });
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": err })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(_))), "{:?}", c.outcome);
        assert_eq!(c.quota, None);
        // 没带 data、只有文字（旧版适配器）：照样认；重置时间用之前 usage_update 报的 rejected
        let mut c = working(true);
        c.on_frame(&usage("s1", json!({ "status": "rejected", "resetsAt": 1795007200 })));
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": { "code": -32603, "message": "You've reached your weekly limit" } })));
        assert!(matches!(c.outcome, Some(Outcome::Quota(_))));
        assert_eq!(c.quota.as_ref().and_then(|q| q.resets_at), Some(1_795_007_200_000));
    }


    /// Codex：Zed 版（v1）session/prompt 回错、data 里 codex_error_info；codex-acp 2.7.0（v2）这一轮以 error 结束、data 里
    /// codexErrorInfo。重置时间从「try again at」读
    #[test]
    fn codex_usage_limit_reads_the_reset_time_from_the_text() {
        let mut c = working(false);
        let text = "You've hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at Nov 26th, 2099 3:15 PM.";
        let err = json!({ "code": -32603, "message": "Internal error", "data": { "message": text, "codex_error_info": "usage_limit_exceeded" } });
        c.on_frame(&json!({ "jsonrpc": "2.0", "id": 2, "error": err }));
        assert!(matches!(c.outcome, Some(Outcome::Quota(ref w)) if w.starts_with("session/prompt 失败：Internal error")), "{:?}", c.outcome);
        let q = c.quota.clone().unwrap();
        let want = chrono::Local.with_ymd_and_hms(2099, 11, 26, 15, 15, 0).unwrap().timestamp_millis();
        assert_eq!((q.exhausted, q.resets_at, q.text.as_str(), q.status.as_str()), (true, Some(want), text, ""));
        // codex-acp 2.7.0（v2）：这一轮以 error 结束，data 里是 codexErrorInfo
        let mut c = working(true);
        let err = json!({ "code": -32603, "message": "Internal error", "data": { "message": "You've hit your usage limit. Try again later.", "codexErrorInfo": "usageLimitExceeded" } });
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": err })));
        assert!(matches!(c.outcome, Some(Outcome::Quota(_))), "{:?}", c.outcome);
        assert_eq!(c.quota.as_ref().map(|q| (q.exhausted, q.resets_at)), Some((true, None)));
        let mut c = working(true);
        let err = json!({ "code": -32603, "message": "Internal error", "data": { "message": "rate limited, retrying failed", "codexErrorInfo": "rateLimitExceeded" } });
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "error", "error": err })));
        assert!(matches!(c.outcome, Some(Outcome::Failed(_))), "一时的限流不算：{:?}", c.outcome);
        // 别的出错不算
        let mut c = working(false);
        c.on_frame(&json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": -32603, "message": "Internal error", "data": { "message": "stream disconnected", "codex_error_info": "response_stream_disconnected" } } }));
        assert!(matches!(c.outcome, Some(Outcome::Failed(_))));
        assert_eq!(c.quota, None);
    }


    #[test]
    fn try_again_at_reads_codex_times() {
        let now = chrono::Local.with_ymd_and_hms(2026, 11, 25, 16, 0, 0).unwrap();
        let at = |y, m, d, h, mi| chrono::Local.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().timestamp_millis();
        assert_eq!(try_again_at("You've hit your usage limit. Try again at 5:30 PM.", now), Some(at(2026, 11, 25, 17, 30)));
        // 只有时刻、已经过了：明天的
        assert_eq!(try_again_at("You've hit your usage limit. Try again at 9:05 AM.", now), Some(at(2026, 11, 26, 9, 5)));
        assert_eq!(try_again_at("… or try again at Dec 1st, 2026 12:00 AM.", now), Some(at(2026, 12, 1, 0, 0)));
        assert_eq!(try_again_at("… or try again at Nov 22nd, 2026 11:45 PM.", now), Some(at(2026, 11, 22, 23, 45)));
        assert_eq!(try_again_at("You've hit your usage limit. Try again later.", now), None);
        assert_eq!(try_again_at("boom", now), None);
        // 没有重置时间：额度用完，时间为空（服务端按 1 小时算）
        let e = json!({ "message": "Internal error", "data": { "message": "You've hit your usage limit. Try again later.", "codex_error_info": "usage_limit_exceeded" } });
        let q = quota_of(&e, None, now).unwrap();
        assert_eq!((q.exhausted, q.resets_at), (true, None));
    }

    /// 查额度的文字：Claude Code 的 `/usage`（2026-11-25 本机实测的原文）、Codex 的 `/status`（codex-acp 2.12 的格式）
    #[test]
    fn usage_text_gives_two_windows() {
        let now = chrono::Local.with_ymd_and_hms(2026, 11, 25, 21, 0, 0).unwrap();
        let at = |y, m, d, h, mi| chrono::Local.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().timestamp_millis();
        let claude = "You are currently using your subscription to power your Claude Code usage\n\n\
            Current session: 43% used · resets Nov 25 at 10:59pm (Europe/Berlin)\n\
            Current week (all models): 58% used · resets Nov 27 at 5:59pm (Europe/Berlin)\n\
            Current week (Fable): 3% used · resets Nov 27 at 5:59pm (Europe/Berlin)\n\n\
            What's contributing to your usage?\n  94%  Start fresh when you switch tasks · run /clear";
        let q = usage_of_text(claude, now).unwrap();
        let w: Vec<(&str, Option<i64>, Option<i64>)> = q.windows.iter().map(|w| (w.key.as_str(), w.used, w.resets_at)).collect();
        assert_eq!(w, vec![("five_hour", Some(43), Some(at(2026, 11, 25, 22, 59))), ("seven_day", Some(58), Some(at(2026, 11, 27, 17, 59)))]);
        assert_eq!((q.window.as_str(), q.used), ("five_hour", Some(43)), "顶层跟着 5 小时（旧界面看它）");
        let codex = "**Model:** gpt-5.2  \n**Account:** dev@example.com (Plus)  \n\n**codex 5h limit:** 88% left (resets 23:30)  \n**codex Weekly limit:** 14% left (resets 09:15 on 3 Dec)";
        let w: Vec<(String, Option<i64>, Option<i64>)> = usage_of_text(codex, now).unwrap().windows.into_iter().map(|w| (w.key, w.used, w.resets_at)).collect();
        assert_eq!(w, vec![("five_hour".into(), Some(12), Some(at(2026, 11, 25, 23, 30))), ("seven_day".into(), Some(86), Some(at(2026, 12, 3, 9, 15)))]);
        // 没有额度的输出（中转、API key）
        assert_eq!(usage_of_text("**Limits:** data not available yet", now), None);
        assert_eq!(usage_of_text("", now), None);
    }


    #[test]
    fn reset_times_in_words() {
        let now = chrono::Local.with_ymd_and_hms(2026, 11, 25, 21, 0, 0).unwrap();
        let at = |y, m, d, h, mi| chrono::Local.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().timestamp_millis();
        assert_eq!(reset_at("nov 25 at 10:59pm", now), Some(at(2026, 11, 25, 22, 59)));
        assert_eq!(reset_at("10:59pm", now), Some(at(2026, 11, 25, 22, 59)));
        // 只有时刻、已经过了：明天的
        assert_eq!(reset_at("9am", now), Some(at(2026, 11, 26, 9, 0)));
        assert_eq!(reset_at("12:30 am", now), Some(at(2026, 11, 26, 0, 30)));
        assert_eq!(reset_at("15:30 on dec 3", now), Some(at(2026, 12, 3, 15, 30)));
        assert_eq!(reset_at("15:30 on 12月3日", now), Some(at(2026, 12, 3, 15, 30)));
        // 跨年：1 月的是明年
        assert_eq!(reset_at("jan 2 at 1:00am", now), Some(at(2027, 1, 2, 1, 0)));
        assert_eq!(reset_at("dec 1, 2026 at 12:00am", now), Some(at(2026, 12, 1, 0, 0)));
        assert_eq!(reset_at("soon", now), None);
    }

    /// 查额度（`--usage`）：开会话后发这条本地命令（不带配置、不切模式），收下 agent 说的话；公布的命令里没有它就不发
    #[test]
    fn usage_mode_sends_only_the_local_command() {
        let o = parse(&s(&["--env", "direct", "--cwd", "/w", "--usage", "/usage", "--", "x"])).unwrap();
        assert_eq!(o.usage.as_deref(), Some("/usage"));
        assert!(parse(&s(&["--env", "direct", "--cwd", "/w", "--usage", "usage", "--", "x"])).unwrap_err().contains("--usage"));
        let init = json!({ "protocolVersion": 2, "capabilities": {} });
        let cmds = |names: &[&str]| json!({ "sessionId": "s1", "availableCommands": names.iter().map(|n| json!({ "name": n })).collect::<Vec<_>>() });
        let mut c = Client::new("/w", "", None).with_usage("/usage");
        c.start();
        assert_eq!(c.on_frame(&reply(0, init.clone()))[0]["method"], "session/new");
        let f = c.on_frame(&reply(1, cmds(&["context", "usage"])));
        assert_eq!((f[0]["method"].as_str(), f[0]["params"]["prompt"][0]["text"].as_str()), (Some("session/prompt"), Some("/usage")));
        let say = |text: &str| json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s1", "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } } } });
        c.on_frame(&say("Current session: 43% used"));
        c.on_frame(&say(" · resets 10:59pm\n"));
        c.on_frame(&state("s1", json!({ "state": "idle", "stopReason": "end_turn" })));
        assert_eq!(c.outcome, Some(Outcome::Stopped("end_turn".into())));
        assert_eq!(c.usage_text, "Current session: 43% used · resets 10:59pm\n");
        // 旧版 agent：公布的命令里没有，不发（免得当成提示词交给模型、花额度）
        let mut c = Client::new("/w", "", None).with_usage("/usage");
        c.start();
        c.on_frame(&reply(0, init));
        assert!(c.on_frame(&reply(1, cmds(&["context"]))).is_empty());
        assert_eq!(c.outcome, Some(Outcome::Failed("这个版本的 agent 没有 /usage 命令，查不了额度".into())));
    }
}
