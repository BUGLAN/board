//! 桌面端 GitHub 登录：在系统默认浏览器里登录，用上浏览器里已有的 GitHub 登录状态。
//!
//! 1. 在 127.0.0.1 的随机端口上监听，生成一个随机串 nonce
//! 2. 用默认浏览器打开 `<BOARD_URL>/auth/github?desktop=<端口>&nonce=<随机串>`
//! 3. 服务端完成 GitHub 登录后，把浏览器送到 `http://127.0.0.1:<端口>/callback?state=<随机串>&code=<一次性登录码>`
//!    （失败时是 `&error=<原因>`）
//! 4. 把登录码交给发起登录的窗口，由窗口打开 `/auth/desktop?code=…` 换取会话 Cookie
//!
//! 同一时间只等一个登录：再点一次登录会取消上一次。

use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tauri::Url;

/// 等浏览器登录的最长时间
const WAIT: Duration = Duration::from_secs(10 * 60);

static CURRENT: AtomicU64 = AtomicU64::new(0);

/// 登录没有完成的原因（界面据此显示提示）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginError {
    /// 服务端给的：cancelled / denied / state / github；
    /// 桌面端的：superseded（又点了一次登录）/ timeout / browser（打不开浏览器）/ io
    pub reason: String,
    /// denied 时是被拒绝的 GitHub 账号
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl LoginError {
    fn new(reason: &str, message: impl Into<Option<String>>) -> Self {
        LoginError { reason: reason.into(), login: None, message: message.into() }
    }

    /// 还没连服务端（显示「连接服务端」页时没有登录地址）
    pub fn not_connected() -> Self {
        LoginError::new("no_server", Some("还没有连接服务端：重开窗口，在「连接服务端」页填地址".into()))
    }
}

impl From<std::io::Error> for LoginError {
    fn from(e: std::io::Error) -> Self {
        LoginError::new("io", e.to_string())
    }
}

fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("系统随机数不可用");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// 在浏览器里登录，返回一次性登录码。阻塞直到登录完成、失败、超时或被新的登录取代
pub fn run(board: &Url) -> Result<String, LoginError> {
    run_with(board, |url| open::that_detached(url))
}

/// `open`：在浏览器里打开登录地址（测试里换成模拟的浏览器）
fn run_with(board: &Url, open: impl FnOnce(&str) -> std::io::Result<()>) -> Result<String, LoginError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let nonce = random_hex(16);

    let mut url = board.join("/auth/github").map_err(|e| LoginError::new("io", e.to_string()))?;
    url.query_pairs_mut().append_pair("desktop", &port.to_string()).append_pair("nonce", &nonce);
    open(url.as_str()).map_err(|e| LoginError::new("browser", e.to_string()))?;
    // 浏览器打开了才算发起新的登录（取代还在等的上一次）
    let id = CURRENT.fetch_add(1, Ordering::SeqCst) + 1;

    // 每个连接单独处理：浏览器可能预先建好空连接，不能让它挡住真正的回调
    let (tx, rx) = mpsc::channel();
    let deadline = Instant::now() + WAIT;
    loop {
        if CURRENT.load(Ordering::SeqCst) != id {
            return Err(LoginError::new("superseded", None));
        }
        if Instant::now() >= deadline {
            return Err(LoginError::new("timeout", None));
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let (tx, nonce) = (tx.clone(), nonce.clone());
                std::thread::spawn(move || {
                    if let Some(r) = handle(stream, &nonce) {
                        let _ = tx.send(r);
                    }
                });
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        if let Ok(r) = rx.recv_timeout(Duration::from_millis(100)) {
            return r;
        }
    }
}

/// 处理浏览器的一个请求。是这次登录的回调就返回结果，其他请求返回 None
fn handle(stream: TcpStream, nonce: &str) -> Option<Result<String, LoginError>> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    // 读完请求头再回复，否则关闭连接时浏览器可能收到 RST 而看不到页面
    let mut header = String::new();
    for _ in 0..100 {
        header.clear();
        match reader.read_line(&mut header) {
            Ok(0) => break,
            Ok(_) if header.trim().is_empty() => break,
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    let target = line.split_whitespace().nth(1)?;
    let url = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    if url.path() != "/callback" {
        respond(&stream, "404 Not Found", &page("找不到这个页面", "这是 Board 桌面端登录用的本机地址。"));
        return None;
    }
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if q.get("state").map(String::as_str) != Some(nonce) {
        respond(&stream, "400 Bad Request", &page("登录链接已失效", "请回到 Board 桌面端重新点击「用 GitHub 登录」。"));
        return None;
    }
    if let Some(code) = q.get("code").filter(|c| !c.is_empty()) {
        respond(&stream, "200 OK", &page("已登录 Board", "可以回到 Board 桌面端了，这个页面可以关掉。"));
        return Some(Ok(code.clone()));
    }
    let reason = q.get("error").cloned().unwrap_or_else(|| "state".into());
    respond(&stream, "200 OK", &page("登录没有完成", "回到 Board 桌面端查看原因。"));
    Some(Err(LoginError { reason, login: q.get("login").cloned(), message: None }))
}

fn respond(mut stream: &TcpStream, status: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

/// 浏览器里显示的结果页（文字是固定的，不含任何请求里的内容）
fn page(title: &str, text: &str) -> String {
    format!(
        r#"<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} · Board</title>
<style>
:root{{color-scheme:light dark;--bg:#f4f2ee;--card:#fff;--text:#1b1b1f;--sub:#6b6b73;--line:rgba(0,0,0,.08)}}
@media (prefers-color-scheme:dark){{:root{{--bg:#151518;--card:#1f1f23;--text:#ededf0;--sub:#9a9aa3;--line:rgba(255,255,255,.08)}}}}
body{{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--bg);color:var(--text);
font:14px/1.6 -apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC","Microsoft YaHei",sans-serif}}
main{{width:min(360px,calc(100% - 32px));padding:32px 28px;border-radius:20px;background:var(--card);box-shadow:0 0 0 1px var(--line),0 24px 48px -24px rgba(0,0,0,.25);text-align:center}}
.logo{{display:inline-flex;gap:3px;width:48px;height:48px;padding:11px 10px;box-sizing:border-box;border-radius:14px;background:#18181a;align-items:flex-start}}
.logo i{{flex:1;border-radius:2px}}
h1{{margin:16px 0 6px;font-size:20px}}p{{margin:0;color:var(--sub)}}
</style>
<main><div class="logo"><i style="height:100%;background:#6e7891"></i><i style="height:70%;background:#f07a12"></i><i style="height:46%;background:#1d8fe0"></i><i style="height:82%;background:#12a06a"></i></div>
<h1>{title}</h1><p>{text}</p></main></html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn get(port: u16, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/html\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    /// 起一个只处理回调的监听（不打开浏览器），返回端口和结果通道
    fn listen(nonce: &'static str) -> (u16, mpsc::Receiver<Result<String, LoginError>>) {
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for s in l.incoming() {
                if let Some(r) = handle(s.unwrap(), nonce) {
                    let _ = tx.send(r);
                    return;
                }
            }
        });
        (port, rx)
    }

    #[test]
    fn callback_with_code() {
        let (port, rx) = listen("abc123");
        // 其他路径、state 不对：不算数
        assert!(get(port, "/favicon.ico").starts_with("HTTP/1.1 404"));
        assert!(get(port, "/callback?state=nope&code=x").starts_with("HTTP/1.1 400"));
        let res = get(port, "/callback?state=abc123&code=c0de");
        assert!(res.starts_with("HTTP/1.1 200"), "{res}");
        assert!(res.contains("已登录 Board"));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap(), "c0de");
    }

    /// 完整走一遍：登录地址带上端口和随机串；模拟的浏览器被服务端送回本机地址
    #[test]
    fn run_opens_login_url_and_waits_for_callback() {
        let board: Url = "https://board.example.com/some/page".parse().unwrap();
        let code = run_with(&board, |url| {
            let u = Url::parse(url).unwrap();
            assert_eq!(u.origin(), board.origin());
            assert_eq!(u.path(), "/auth/github");
            let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
            let port: u16 = q["desktop"].parse().unwrap();
            let nonce = q["nonce"].clone();
            assert_eq!(nonce.len(), 32);
            std::thread::spawn(move || {
                // 浏览器预先建好的空连接不能挡住回调
                let _idle = TcpStream::connect(("127.0.0.1", port)).unwrap();
                assert!(get(port, &format!("/callback?state={nonce}&code=one-time")).contains("已登录 Board"));
            });
            Ok(())
        });
        assert_eq!(code.unwrap(), "one-time");
    }

    #[test]
    fn run_reports_browser_failure() {
        let board: Url = "https://board.example.com".parse().unwrap();
        let e = run_with(&board, |_| Err(std::io::Error::other("没有浏览器"))).unwrap_err();
        assert_eq!((e.reason.as_str(), e.message.as_deref()), ("browser", Some("没有浏览器")));
    }

    #[test]
    fn callback_with_error() {
        let (port, rx) = listen("abc123");
        let res = get(port, "/callback?state=abc123&error=denied&login=someone-else");
        assert!(res.contains("登录没有完成"));
        let e = rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err();
        assert_eq!((e.reason.as_str(), e.login.as_deref()), ("denied", Some("someone-else")));
    }
}
