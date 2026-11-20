//! 派发的日志和结构化事件：读新增的行、脱敏、上传、截断和按天清理。

use super::*;

/// 读日志里 from 之后新增的完整行，返回（行，新的位置）；last 为 true 时最后不完整的一行也算
pub(super) fn new_lines(path: &Path, from: u64, last: bool) -> (Vec<String>, u64) {
    let Ok(mut f) = std::fs::File::open(path) else { return (vec![], from) };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len <= from {
        return (vec![], from);
    }
    let start = from.max(len.saturating_sub(LOG_CHUNK));
    if f.seek(SeekFrom::Start(start)).is_err() {
        return (vec![], from);
    }
    let mut buf = Vec::new();
    if f.take(len - start).read_to_end(&mut buf).is_err() {
        return (vec![], from);
    }
    // 跳过了一部分：从下一个整行开始
    let mut skip = 0;
    if start > from {
        skip = buf.iter().position(|b| *b == b'\n').map(|i| i + 1).unwrap_or(buf.len());
    }
    let end = if last { buf.len() } else { buf.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0) };
    if end <= skip {
        return (vec![], if start > from { start + skip as u64 } else { from });
    }
    let text = String::from_utf8_lossy(&buf[skip..end]);
    let lines = text.lines().map(|l| l.trim_end_matches('\r').to_string()).collect();
    (lines, start + end as u64)
}

/// 登录 shell 自己打的启动、退出提示（见 [`board_core::hosts::shell_noise`]）：不是 agent 的输出
pub(super) fn shell_noise(line: &str) -> bool {
    board_core::hosts::shell_noise(line)
}

/// 日志最后 n 行（转写之后的）
pub(super) fn tail_lines(path: &Path, n: usize, fmt: Option<Stream>) -> Vec<String> {
    let (lines, _) = new_lines(path, 0, true);
    let lines: Vec<String> = logfmt::render(fmt, lines).into_iter().filter(|l| !l.trim().is_empty() && !shell_noise(l)).collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

/// 要上传的行：转写，只留最后 500 行（服务端本来也只保留这么多）
pub(super) fn upload_lines(lines: Vec<String>, fmt: Option<Stream>) -> Vec<String> {
    let lines: Vec<String> = logfmt::render(fmt, lines).into_iter().filter(|l| !shell_noise(l)).collect();
    lines[lines.len().saturating_sub(LOG_KEEP)..].to_vec()
}

/// 要上传的行：转写，只留最后 500 行，再脱敏（令牌、密钥不上传到服务端）
pub(super) fn upload_lines_redacted(lines: Vec<String>, fmt: Option<Stream>, secrets: &[&str]) -> Vec<String> {
    upload_lines(lines, fmt).iter().map(|l| redact::redact(l, secrets)).collect()
}

/// 解析日志里新增的行，得到结构化事件（只有原始输出的 agent 只挪位置）。last：最后不完整的一行也算（agent 已经退出）。
/// 事件里的文字先脱敏，再放进待上传的队列和「最后几个事件」。登录 shell 自己打的提示不是 agent 的输出，直接丢掉
pub(super) fn read_events(r: &mut Run, last: bool, secrets: &[&str]) {
    let (lines, to) = new_lines(&r.log, r.ev_read, last);
    r.ev_read = to;
    let Some(p) = r.parser.as_mut() else { return };
    let at = now_ms();
    for line in lines.into_iter().filter(|l| !shell_noise(l)) {
        for mut e in p.feed(&line, at) {
            redact_event(&mut e, secrets);
            remember(&mut r.ev_recent, &e);
            r.ev_queue.push(e);
        }
    }
    let keep = board_core::runlog::EVENTS_KEEP as usize;
    if r.ev_queue.len() > keep {
        let drop = r.ev_queue.len() - keep;
        r.ev_queue.drain(..drop);
    }
}

/// 事件里的文字脱敏（令牌、密钥不上传到服务端，也不写进评论）
pub(super) fn redact_event(e: &mut RunEvent, secrets: &[&str]) {
    for f in [&mut e.text, &mut e.tool, &mut e.args, &mut e.out] {
        if let Some(x) = f.as_mut() {
            *x = redact::redact(x, secrets);
        }
    }
}

/// 记进「最后几个事件」：工具结果合进同一个调用
pub(super) fn remember(recent: &mut VecDeque<RunEvent>, e: &RunEvent) {
    if e.kind == "tool" && e.call.is_some() {
        if let Some(prev) = recent.iter_mut().rev().find(|p| p.kind == "tool" && p.call == e.call) {
            prev.merge(e);
            return;
        }
    }
    recent.push_back(e.clone());
    while recent.len() > board_core::runlog::EVENTS_TAIL {
        recent.pop_front();
    }
}

/// 异常退出评论里的「最后 10 个事件」
pub(super) fn recent_lines(r: &Run) -> Vec<String> {
    r.ev_recent.iter().map(RunEvent::line).collect()
}

/// 日志文件的权限设成 0600（只有自己能读）。Windows 上不设：ACL 不是这个模型，用户目录本来就只有本人能进
pub(super) fn restrict_log(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// 新建（或清空）一个日志文件，权限 0600。返回标准输出、标准错误两份句柄
pub(super) fn create_log(path: &Path) -> std::io::Result<(std::fs::File, std::fs::File)> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    #[cfg(unix)]
    let f = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?
    };
    #[cfg(not(unix))]
    let f = std::fs::File::create(path)?;
    restrict_log(path);
    let err = f.try_clone()?;
    Ok((f, err))
}

/// 太大了只留开头和结尾：超过 `max` 字节时，留下开头 `head` 字节、最后 `tail` 字节，中间换成一行省略说明。
/// 返回截断后的长度（没截断时 None）
pub(super) fn truncate_log(path: &Path, head: u64, tail: u64, max: u64) -> std::io::Result<Option<u64>> {
    let len = std::fs::metadata(path)?.len();
    if len <= max {
        return Ok(None);
    }
    let mut f = std::fs::File::open(path)?;
    let mut first = Vec::new();
    (&mut f).take(head).read_to_end(&mut first)?;
    f.seek(SeekFrom::End(-(tail as i64)))?;
    let mut last = Vec::new();
    f.read_to_end(&mut last)?;
    // 别从半行开始、也别在半行结束
    if let Some(i) = first.iter().rposition(|b| *b == b'\n') {
        first.truncate(i + 1);
    }
    if let Some(i) = last.iter().position(|b| *b == b'\n') {
        last.drain(..=i);
    }
    let note = format!("\n[日志太大（{} 字节），中间 {} 字节已省略：只保留开头 {} KB 和最后 {} KB]\n", len, len - first.len() as u64 - last.len() as u64, head / 1024, tail / 1024);
    let mut out = Vec::with_capacity(first.len() + note.len() + last.len());
    out.extend_from_slice(&first);
    out.extend_from_slice(note.as_bytes());
    out.extend_from_slice(&last);
    std::fs::write(path, &out)?;
    restrict_log(path);
    Ok(Some(out.len() as u64))
}

/// 本机派发日志清理：`~/.board/host/logs` 里结束超过 `days` 天的删掉。`keep` 是正在跑的派发 ID，
/// 它们的日志不删（改过文件时间的、还在写的都按结束时间算：看修改时间）。
/// 返回删掉的个数。日志文件名是派发 ID 或 `ready-*`、`clone-*`、`update` 这类
pub(super) fn clean_logs(dir: &Path, days: i64, keep: &[i64], now: std::time::SystemTime) -> usize {
    let cutoff = now.checked_sub(Duration::from_secs((days.max(1) as u64) * 24 * 3600));
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".log") {
            continue;
        }
        // 正在跑的派发：文件名就是派发 ID
        let id: Option<i64> = name.trim_end_matches(".log").parse().ok();
        if id.is_some_and(|id| keep.contains(&id)) {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let Ok(modified) = m.modified() else { continue };
        if cutoff.is_some_and(|c| modified < c) && std::fs::remove_file(&p).is_ok() {
            removed += 1;
        }
    }
    removed
}

impl Host<'_> {
    /// 清理本机派发日志：结束超过 `log_days` 天的删掉，正在跑的留着；留下来的单个超过 50 MB 的只保留开头和结尾
    /// （只动已经结束的：agent 可能还拿着文件句柄，换掉内容会在它下次写入时留一段空洞）
    pub(super) fn clean_logs(&mut self) {
        let keep: Vec<i64> = self.runs.iter().map(|r| r.d.id).collect();
        let days = self.log_days;
        let removed = clean_logs(&self.logs, days, &keep, std::time::SystemTime::now());
        if removed > 0 {
            self.say(format!("{} 删掉了 {removed} 个结束超过 {days} 天的本机日志", self.p.dim("·")));
        }
        let Ok(rd) = std::fs::read_dir(&self.logs) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".log") {
                continue;
            }
            let id: Option<i64> = name.trim_end_matches(".log").parse().ok();
            if id.is_some_and(|id| keep.contains(&id)) {
                continue;
            }
            if let Ok(Some(_)) = truncate_log(&e.path(), LOG_HEAD, LOG_TAIL, LOG_MAX) {
                self.say(format!("{} {name} 超过 50 MB，只留了开头 1 MB 和最后 10 MB", self.p.dim("·")));
            }
        }
    }

    /// 这次派发注入给 agent 的密钥值（当前令牌、`BOARD_TOKEN` 这类环境变量）：上传日志、写异常退出评论之前
    /// 换成 `<已隐藏>`，本机日志保留原文
    pub(super) fn secrets(&self) -> Vec<String> {
        let mut out = vec![self.rm().token().to_string()];
        for k in ["BOARD_TOKEN", "BOARD_HOST_TOKEN"] {
            if let Some(v) = std::env::var(k).ok().filter(|v| !v.trim().is_empty()) {
                out.push(v);
            }
        }
        out.sort();
        out.dedup();
        out
    }

    pub(super) fn upload_logs(&mut self) {
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let mut sent: Vec<(usize, u64)> = vec![];
        for (i, r) in self.runs.iter().enumerate() {
            // 这里只会有还在跑的派发（结束的已经从 runs 里移走并截断过）：不截断正在写的日志，
            // agent 还拿着它的句柄，换掉文件内容会在它下次写入时从旧位置接着写、中间留一段空洞
            let (lines, to) = new_lines(&r.log, r.sent, false);
            let lines = upload_lines_redacted(lines, r.fmt, &secrets);
            if lines.is_empty() {
                if to != r.sent {
                    sent.push((i, to));
                }
                continue;
            }
            if self.rm().post::<Value>(&format!("/api/dispatches/{}/log", r.d.id), json!({ "lines": lines })).is_ok() {
                sent.push((i, to));
            }
        }
        for (i, to) in sent {
            self.runs[i].sent = to;
        }
    }

    /// 上传一次派发剩下的日志（结束时）
    pub(super) fn flush_log(&self, r: &Run, last: bool) {
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let (lines, _) = new_lines(&r.log, r.sent, last);
        let lines = upload_lines_redacted(lines, r.fmt, &secrets);
        if !lines.is_empty() {
            let _ = self.rm().post::<Value>(&format!("/api/dispatches/{}/log", r.d.id), json!({ "lines": lines }));
        }
    }

    /// 每 2 秒：解析每个在跑的派发新增的输出，把新的结构化事件传上去
    pub(super) fn upload_events(&mut self) {
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let mut runs = std::mem::take(&mut self.runs);
        for r in runs.iter_mut() {
            read_events(r, false, &secrets);
            self.post_events(r);
        }
        self.runs = runs;
    }

    /// 一次派发结束（last：agent 已经退出，最后不完整的一行也读）或会话换到下一张之前：读完新增的输出，把事件都传上去
    pub(super) fn flush_events(&self, r: &mut Run, last: bool) {
        let secrets = self.secrets();
        let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
        read_events(r, last, &secrets);
        self.post_events(r);
    }

    /// 传事件：带上格式（只有原始输出的 agent 是空字符串）。连不上、服务端出错（5xx）时留到下一次；其他拒绝（旧版服务端
    /// 没有这个接口、令牌不让调、派发不存在）就丢掉，不反复重试
    pub(super) fn post_events(&self, r: &mut Run) {
        if r.ev_queue.is_empty() && !r.fmt_due {
            return;
        }
        let events = board_core::runlog::merge_calls(std::mem::take(&mut r.ev_queue));
        let fmt = r.fmt.map(Stream::key).unwrap_or("");
        match self.rm().post::<Value>(&format!("/api/dispatches/{}/events", r.d.id), json!({ "fmt": fmt, "events": events })) {
            Ok(_) => r.fmt_due = false,
            Err(e) if matches!(e.code, ErrorCode::Unreachable | ErrorCode::Internal) => r.ev_queue = events,
            Err(_) => r.fmt_due = false,
        }
    }
}
