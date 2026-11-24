//! 部署后的健康检查：同时核对 API 模式/版本和首页 HTML。
//!
//! 只依赖 `board-server` 自己的 `/api/health` 与 `/`，不要求服务器上装 curl 或 jq；
//! Docker 的 HEALTHCHECK 和局域网部署脚本都可以直接调用。

use serde_json::Value;
use std::time::{Duration, Instant};

/// 可以额外核对的期望值；不填的项只确认服务端返回了它。
#[derive(Debug, Default, Clone)]
pub struct Expect {
    pub mode: Option<String>,
    pub version: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub mode: String,
    pub version: String,
    pub commit: String,
}

struct Fetched {
    status: u16,
    content_type: String,
    body: String,
}

fn fetch(agent: &ureq::Agent, url: &str) -> Result<Fetched, String> {
    let mut res = agent.get(url).call().map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(Fetched { status, content_type, body })
}

/// 先检查 API，再检查首页；两边都查完才汇总错误，避免一个失败把另一个的结果盖住。
pub fn check(base: &str, expect: &Expect) -> Result<Report, Vec<String>> {
    let base = base.trim_end_matches('/');
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut errors = Vec::new();
    let mut report = None;

    match fetch(&agent, &format!("{base}/api/health")) {
        Err(e) => errors.push(format!("API /api/health 连接失败：{e}")),
        Ok(r) if !(200..300).contains(&r.status) => {
            errors.push(format!("API /api/health 返回 HTTP {}（应为 2xx）", r.status));
        }
        Ok(r) => match serde_json::from_str::<Value>(&r.body) {
            Err(e) => errors.push(format!("API /api/health 不是 JSON：{e}")),
            Ok(v) => {
                if v["ok"] != true {
                    errors.push("API /api/health 没有返回 ok=true".into());
                }
                let field = |name: &str| v[name].as_str().unwrap_or_default().to_string();
                let mode = field("mode");
                let version = field("version");
                let commit = field("commit");
                if mode.is_empty() {
                    errors.push("API /api/health 没有返回 mode（旧版服务端或配置不完整）".into());
                }
                if version.is_empty() {
                    errors.push("API /api/health 没有返回 version".into());
                }
                if commit.is_empty() {
                    errors.push("API /api/health 没有返回 commit".into());
                }
                if v["ui"].as_bool() == Some(false) {
                    errors.push("API /api/health 报告界面没有编译进来（ui=false）".into());
                }
                if let Some(want) = &expect.mode {
                    if mode != *want {
                        errors.push(format!("API 模式是 {mode:?}，应为 {want:?}"));
                    }
                }
                if let Some(want) = &expect.version {
                    if version != *want {
                        errors.push(format!("API 版本是 {version:?}，应为 {want:?}"));
                    }
                }
                if let Some(want) = &expect.commit {
                    if commit != *want {
                        errors.push(format!("API 提交是 {commit:?}，应为 {want:?}"));
                    }
                }
                report = Some(Report { mode, version, commit });
            }
        },
    }

    match fetch(&agent, &format!("{base}/")) {
        Err(e) => errors.push(format!("界面 / 连接失败：{e}")),
        Ok(r) if !(200..300).contains(&r.status) => {
            errors.push(format!("界面 / 返回 HTTP {}（应为 2xx 的 HTML）", r.status));
        }
        Ok(r) => {
            if !r.content_type.to_ascii_lowercase().starts_with("text/html") {
                errors.push(format!("界面 / 的 Content-Type 是 {:?}，应为 text/html", r.content_type));
            }
            if !r.body.to_ascii_lowercase().contains("<html") || !r.body.contains("id=\"root\"") {
                errors.push("界面 / 返回的不是 Board 的 index.html".into());
            }
        }
    }

    let Some(report) = report else { return Err(errors) };
    if errors.is_empty() {
        Ok(report)
    } else {
        Err(errors)
    }
}

/// 部署后服务可能还在重启：在 `wait_secs` 内每秒重试，超时后返回最后一次的错误。
pub fn wait(base: &str, expect: &Expect, wait_secs: u64) -> Result<Report, Vec<String>> {
    let deadline = Instant::now() + Duration::from_secs(wait_secs);
    loop {
        match check(base, expect) {
            Ok(report) => return Ok(report),
            Err(errors) if Instant::now() >= deadline => return Err(errors),
            Err(_) => std::thread::sleep(Duration::from_secs(1)),
        }
    }
}
