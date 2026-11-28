//! 大盘接口 `GET /api/stats` 的参数检查。
//!
//! 起一个真实的 board-server（后台线程、随机端口），用 HTTP 访问；本机模式（Auth::Local）不需要登录。

mod common;

use std::time::{Duration, Instant};

use board_server::auth::Auth;
use common::{send, Server};

/// #683：`to - from` 会溢出的时间范围直接返回 400，不在全局锁里按小时分桶推到 9999 年
#[test]
fn overflowing_range_is_rejected_quickly() {
    let s = Server::start(Auth::Local);
    for (from, to) in [(-1, i64::MAX), (i64::MIN, i64::MIN + 1)] {
        let url = format!("{}/api/stats?scope=all&from={from}&to={to}&tz=America/New_York", s.url);
        let t = Instant::now();
        let r = send("GET", &url, &[], None);
        let took = t.elapsed();
        assert_eq!(r.status, 400, "from={from} to={to}: {}", r.text);
        assert_eq!(r.code(), "USAGE", "{}", r.text);
        assert!(took < Duration::from_millis(100), "from={from} to={to} 用了 {took:?}");
    }
    // 正常的范围照常返回
    let r = send("GET", &format!("{}/api/stats?scope=all&from=0&to=86400000&tz=UTC", s.url), &[], None);
    assert_eq!(r.status, 200, "{}", r.text);
}
