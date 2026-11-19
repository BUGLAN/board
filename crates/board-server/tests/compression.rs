//! 响应压缩（default/main#504）：/api 的 JSON 按 Accept-Encoding 压缩；/api/stream 的 SSE 不压缩。
//!
//! 用原始 TCP 发请求、只看响应头：HTTP 客户端库可能自己加 Accept-Encoding、自己解压，看不出服务端做了什么。

mod common;

use board_server::auth::Auth;
use common::Server;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// 发一个 GET，读到响应头结束为止，返回小写的响应头文本
fn head(s: &Server, path: &str, accept: Option<&str>) -> String {
    let addr = s.url.trim_start_matches("http://");
    let mut c = TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let ae = accept.map(|a| format!("Accept-Encoding: {a}\r\n")).unwrap_or_default();
    write!(c, "GET {path} HTTP/1.1\r\nHost: {addr}\r\n{ae}Connection: close\r\n\r\n").unwrap();
    let mut buf = Vec::new();
    let mut b = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = c.read(&mut b).unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&b[..n]);
    }
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_lowercase()
}

#[test]
fn json_api_is_compressed_when_client_accepts_it() {
    let s = Server::start(Auth::Local);
    let h = head(&s, "/api/workspace", Some("gzip"));
    assert!(h.starts_with("http/1.1 200"), "{h}");
    assert!(h.contains("content-encoding: gzip"), "{h}");
    let h = head(&s, "/api/board?path=default/main", Some("br, gzip"));
    assert!(h.contains("content-encoding: br"), "{h}");
}

#[test]
fn not_compressed_without_accept_encoding() {
    let s = Server::start(Auth::Local);
    let h = head(&s, "/api/workspace", None);
    assert!(h.starts_with("http/1.1 200"), "{h}");
    assert!(!h.contains("content-encoding"), "{h}");
}

#[test]
fn sse_stream_is_not_compressed() {
    let s = Server::start(Auth::Local);
    let h = head(&s, "/api/stream", Some("gzip, br"));
    assert!(h.starts_with("http/1.1 200"), "{h}");
    assert!(h.contains("content-type: text/event-stream"), "{h}");
    assert!(!h.contains("content-encoding"), "{h}");
}
