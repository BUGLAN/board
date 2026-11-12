//! 上传前的脱敏（卡片 #199）：board host 把 agent 的输出上传到服务端（日志、异常退出评论），
//! 服务端上任何登录的人都能看到。agent 执行 `env`、打印配置文件、报错时带出请求头，令牌和其他密钥
//! 就会跟着进服务端。上传之前把下面这些换成 `<已隐藏>`：
//!
//! - 当前令牌的原文（board host 自己用的这个）；
//! - 这次派发注入的密钥值（`BOARD_TOKEN`、`BOARD_HOST_TOKEN` 等环境变量的值，调用方一并传进来）；
//! - 常见格式的密钥：`sk-…`、`ghp_…`、`github_pat_…`、`xox…-…`、`AKIA…`、整段 `-----BEGIN … PRIVATE KEY-----`
//!   块（到对应的 `-----END … PRIVATE KEY-----` 为止），以及 `Authorization: Bearer …` 的凭据部分。
//!
//! 本机的日志文件保留原文：脱敏只在上传（和写进卡片评论的那份摘要）之前做。
//!
//! 普通的十六进制提交号（40 位十六进制、`abcdef1234567890` 这种短的）不能被误伤：`sk-`、`ghp_` 这些
//! 前缀后面要求至少 16 位，而且只认字母和数字，不会匹配到普通的英文单词。

/// 换上去的文字
pub const HIDDEN: &str = "<已隐藏>";

/// 一个密钥格式：前缀 + 后面至少多少位「token 字符」（字母、数字、`_`、`-`）
struct Rule {
    prefix: &'static str,
    /// 前缀后面至少这么多位才算（太短的是普通单词，不是密钥）
    min_rest: usize,
}

/// 常见格式的密钥前缀。顺序不影响结果（都在原文上从左到右扫，取最长的覆盖）
const RULES: &[Rule] = &[
    // OpenAI、Anthropic 等：sk-…、sk-ant-…、sk-proj-…
    Rule { prefix: "sk-", min_rest: 16 },
    // GitHub：旧版个人令牌 ghp_、新版细粒度令牌 github_pat_
    Rule { prefix: "ghp_", min_rest: 16 },
    Rule { prefix: "github_pat_", min_rest: 16 },
    // Slack：xoxb-、xoxp-、xoxa-、xoxr-、xoxs-
    Rule { prefix: "xoxb-", min_rest: 10 },
    Rule { prefix: "xoxp-", min_rest: 10 },
    Rule { prefix: "xoxa-", min_rest: 10 },
    Rule { prefix: "xoxr-", min_rest: 10 },
    Rule { prefix: "xoxs-", min_rest: 10 },
    // AWS access key ID：AKIA 加 16 位大写字母、数字
    Rule { prefix: "AKIA", min_rest: 16 },
];

/// token 里会出现的字符：字母、数字、`_`、`-`。遇到别的（空白、引号、`:`、`/`）就到此为止
fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// `Authorization: Bearer …`（大小写不敏感）里凭据的起点：冒号后面先跳过空白，再看是不是 Bearer
fn bearer_rest(s: &str, from: usize) -> Option<usize> {
    let rest = &s[from..];
    let lower = rest.to_ascii_lowercase();
    if !lower.starts_with("authorization") {
        return None;
    }
    let after = &rest["authorization".len()..];
    let after = after.trim_start_matches([' ', '\t']);
    let after = after.strip_prefix(':')?;
    let after = after.trim_start_matches([' ', '\t']);
    let lower = after.to_ascii_lowercase();
    let after = &after[..];
    if !lower.starts_with("bearer") {
        return None;
    }
    // Bearer 后面要有空白，不能是 Basic
    let after = &after["bearer".len()..];
    if !after.starts_with([' ', '\t']) {
        return None;
    }
    let lead = after.len() - after.trim_start_matches([' ', '\t']).len();
    let cred = &after[lead..];
    if cred.is_empty() {
        return None;
    }
    Some(from + s[from..].len() - cred.len())
}

/// 从 `from` 开始的一个 `-----BEGIN … PRIVATE KEY-----` 块的结尾（含 `-----END …-----` 那一行）。
/// 没有对应的 END 时返回 None（不吞掉后面的日志）
fn private_key_end(s: &str, from: usize) -> Option<usize> {
    let line_end = s[from..].find('\n').map(|i| from + i).unwrap_or(s.len());
    let head = &s[from..line_end];
    if !head.starts_with("-----BEGIN ") || !head.contains("PRIVATE KEY-----") {
        return None;
    }
    let kind = head.strip_prefix("-----BEGIN ").and_then(|r| r.strip_suffix("-----"))?;
    let end = format!("-----END {kind}-----");
    let at = s[line_end..].find(&end)? + line_end;
    // 一行结束：带上 END 后面的换行前的剩余字符
    let stop = s[at..].find('\n').map(|i| at + i).unwrap_or(s.len());
    Some(stop)
}

/// 脱敏：`secrets` 是这次派发注入的密钥原文（当前令牌、`BOARD_TOKEN` 之类的值），
/// 出现的每一处都换成 `<已隐藏>`，再按常见格式扫一遍。返回脱敏后的文字
pub fn redact(s: &str, secrets: &[&str]) -> String {
    if s.is_empty() {
        return String::new();
    }
    // 先换成隐藏：按长度从长到短，免得短的先换掉一个长密钥的一部分
    let mut out = s.to_string();
    let mut list: Vec<&str> = secrets.iter().copied().filter(|x| !x.trim().is_empty()).collect();
    list.sort_by_key(|x| std::cmp::Reverse(x.len()));
    list.dedup();
    for secret in list {
        if out.contains(secret) {
            out = out.replace(secret, HIDDEN);
        }
    }
    // 再按格式扫一遍：认出 [start, end) 的每一段，合成结果
    let mut found: Vec<(usize, usize)> = Vec::new();
    let bytes_at = |i: usize| out.get(i..).and_then(|r| r.chars().next());
    let mut i = 0;
    while i < out.len() {
        if !out.is_char_boundary(i) {
            i += 1;
            continue;
        }
        // Authorization: Bearer …（在 Authorization 处处理，跳过整段凭据）
        if let Some(start) = bearer_rest(&out, i).filter(|_| out[i..].len() >= 13) {
            let send = out[start..].find(|c: char| !token_char(c) && c != '.' && c != '~' && c != '+' && c != '/').map(|k| start + k).unwrap_or(out.len());
            if send > start {
                found.push((start, send));
                i = send;
                continue;
            }
        }
        // PRIVATE KEY 块
        if let Some(end) = private_key_end(&out, i) {
            found.push((i, end));
            i = end;
            continue;
        }
        // 前缀格式
        for rule in RULES {
            if !out[i..].starts_with(rule.prefix) {
                continue;
            }
            // AKIA 后面只认大写字母和数字（AWS 的格式）；其他认一般的 token 字符
            let ok = |c: char| if rule.prefix == "AKIA" { c.is_ascii_uppercase() || c.is_ascii_digit() } else { token_char(c) };
            let mut end = i + rule.prefix.len();
            for (k, c) in out[end..].char_indices() {
                if !ok(c) {
                    break;
                }
                end = i + rule.prefix.len() + k + c.len_utf8();
            }
            if end - i - rule.prefix.len() >= rule.min_rest {
                found.push((i, end));
                i = end;
            }
            break;
        }
        i += bytes_at(i).map(char::len_utf8).unwrap_or(1);
    }
    if found.is_empty() {
        return out;
    }
    found.sort();
    let mut res = String::with_capacity(out.len());
    let mut at = 0;
    for (start, end) in found {
        if start < at {
            continue;
        }
        res.push_str(&out[at..start]);
        res.push_str(HIDDEN);
        at = end;
    }
    res.push_str(&out[at..]);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_the_token_itself_and_injected_secrets() {
        let s = "BOARD_TOKEN=bd_abcdefghijklmnop\nenv: BOARD_HOST_TOKEN=bd_abcdefghijklmnop\nother bd_zzzzzzzzzzzzzzzz";
        let got = redact(s, &["bd_abcdefghijklmnop", "bd_zzzzzzzzzzzzzzzz"]);
        assert_eq!(got, "BOARD_TOKEN=<已隐藏>\nenv: BOARD_HOST_TOKEN=<已隐藏>\nother <已隐藏>", "令牌原文一处不留");
        assert!(!got.contains("bd_abc") && !got.contains("bd_zzz"));
        // 空白的密钥不算、不会把所有空白都换掉
        assert_eq!(redact("a b", &["", "  "]), "a b");
        assert_eq!(redact("", &["x"]), "");
    }

    #[test]
    fn hides_common_key_formats() {
        let gh = format!("ghp_{}", "a1B2".repeat(9));
        let pat = format!("github_pat_{}", "x9".repeat(11));
        let sk = format!("sk-{}", "Ab3".repeat(10));
        let sl = format!("xoxb-{}-{}", "1234567890", "abcdefghij");
        let aws = format!("AKIA{}", "ABCDEFGHIJKLMNOP");
        let s = format!("A {gh}\nB {pat}\nC {sk}\nD {sl}\nE {aws}\n");
        let got = redact(&s, &[]);
        for part in [&gh, &pat, &sk, &sl, &aws] {
            assert!(!got.contains(part.as_str()), "要隐藏 {part}：{got}");
        }
        assert_eq!(got.matches(HIDDEN).count(), 5, "{got}");
        assert!(got.starts_with(&format!("A {HIDDEN}\nB {HIDDEN}")), "{got}");
        assert!(got.contains(&format!("\nE {HIDDEN}\n")), "AKIA 后面的 16 位一起隐藏：{got}");
    }

    #[test]
    fn hides_private_key_blocks_as_a_whole() {
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nAAAABG5vbmUAAAAE\n-----END OPENSSH PRIVATE KEY-----";
        let s = format!("before\n{key}\nafter\n");
        let got = redact(&s, &[]);
        assert_eq!(got, format!("before\n{HIDDEN}\nafter\n"), "整段换成一处");
        // 没有 END 的不吞掉后面的日志
        let cut = "-----BEGIN RSA PRIVATE KEY-----\nAAAA\nboom\n";
        assert_eq!(redact(cut, &[]), cut);
        // 不是密钥的 BEGIN 不动
        let cert = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----";
        assert_eq!(redact(cert, &[]), cert);
    }

    #[test]
    fn hides_bearer_credentials_only() {
        let got = redact("Authorization: Bearer abc.def-123\nnext line", &[]);
        assert_eq!(got, format!("Authorization: Bearer {HIDDEN}\nnext line"));
        // 大小写、空白
        let got = redact("authorization:   bearer   tok3n", &[]);
        assert_eq!(got, format!("authorization:   bearer   {HIDDEN}"));
        // Basic 认证不动（不是这个格式）
        let basic = "Authorization: Basic dXNlcjpwYXNz";
        assert_eq!(redact(basic, &[]), basic);
        // 没有 Authorization 的 Bearer 不动
        let plain = "Bearer abc";
        assert_eq!(redact(plain, &[]), plain);
    }

    /// 普通的十六进制提交号、短单词不能被误伤（卡片 #199 的验收标准）
    #[test]
    fn keeps_ordinary_hex_and_words() {
        let lines = [
            "commit 271b3c8f5a3f7cb38844cb988ffe88aa6a402c5d",
            "abcdef1234567890",
            "sk- short",
            "task-runner: restarting",
            "risk-free",
            "xoxo",
            "AKIA",
            "mask-1234",
        ];
        for line in lines {
            assert_eq!(redact(line, &[]), line, "不能误伤：{line}");
        }
    }

    /// 例子里的每一种格式各来一处，和令牌原文混在一起
    #[test]
    fn every_format_in_one_line() {
        let token = "bd_0123456789abcdef0123456789abcdef";
        let gh = format!("ghp_{}", "Z".repeat(36));
        let pat = format!("github_pat_{}", "1".repeat(30));
        let sk = format!("sk-{}", "Q".repeat(20));
        let sl = format!("xoxp-{}-{}", "9".repeat(12), "8".repeat(12));
        let aws = format!("AKIA{}", "7".repeat(16));
        let key = "-----BEGIN PRIVATE KEY-----\nMIIBVwIBADAN\n-----END PRIVATE KEY-----";
        let s = format!("token={token} gh={gh} pat={pat} sk={sk} slack={sl} aws={aws} key={key}\nauth: Authorization: Bearer {token}");
        let got = redact(&s, &[token]);
        for part in [token, &gh, &pat, &sk, &sl, &aws] {
            assert!(!got.contains(part), "没隐藏 {part}：{got}");
        }
        assert!(!got.contains("MIIBVwIBADAN"), "{got}");
        assert!(!got.contains("-----BEGIN PRIVATE KEY-----"), "{got}");
        assert_eq!(got.matches(HIDDEN).count(), 8, "{got}");
    }
}
