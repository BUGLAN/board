//! `board-server` —— 网页端。在浏览器里打开它打印的地址即可。
//!
//! ```text
//! board-server [--host 127.0.0.1] [--port 7420] [--db <路径>] [--demo]
//! ```
//! 手机浏览器访问：`--host 0.0.0.0`，再用电脑的局域网地址打开（没有账号与登录，只在可信网络里这样做）。

use board_core::Store;
use board_server::{bind, serve, AppState, Config};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

const USAGE: &str = "用法：board-server [--host 127.0.0.1] [--port 7420] [--db <路径>] [--demo]

  --host   监听地址。手机访问时用 0.0.0.0
  --port   端口，默认 7420
  --db     数据库文件，默认 $BOARD_DB 或 ~/.board/board.db（与 board 命令行共用）
  --demo   数据库为空时写入示例项目";

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let mut host: IpAddr = [127, 0, 0, 1].into();
    let mut port: u16 = 7420;
    let mut db = Store::default_path();
    let mut demo = false;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let fail = |msg: &str| -> ! {
        eprintln!("{msg}\n\n{USAGE}");
        std::process::exit(2);
    };
    while i < args.len() {
        let next = |i: usize| args.get(i + 1).cloned().unwrap_or_else(|| fail(&format!("{} 需要一个值", args[i])));
        match args[i].as_str() {
            "--host" => {
                host = next(i).parse().unwrap_or_else(|_| fail("--host 需要一个 IP 地址"));
                i += 1;
            }
            "--port" => {
                port = next(i).parse().unwrap_or_else(|_| fail("--port 需要一个端口号"));
                i += 1;
            }
            "--db" => {
                db = PathBuf::from(next(i));
                i += 1;
            }
            "--demo" => demo = true,
            "-h" | "--help" | "help" => {
                println!("{USAGE}");
                return;
            }
            other => fail(&format!("未知参数 {other}")),
        }
        i += 1;
    }

    let cfg = Config { db: db.clone(), demo, serve_web: true, cors_origins: vec![] };
    let state = match AppState::open(&cfg) {
        Ok(s) => s,
        Err(e) => fail(&format!("打开数据库失败：{e}")),
    };
    let addr = SocketAddr::new(host, port);
    let listener = match bind(addr) {
        Ok(l) => l,
        Err(e) => fail(&format!("无法监听 {addr}：{e}")),
    };
    let shown = if host.is_unspecified() { format!("http://127.0.0.1:{port}") } else { format!("http://{addr}") };
    println!("Board 网页端：{shown}");
    println!("数据库：{}", db.display());
    if host.is_unspecified() {
        println!("已监听所有网卡：同一网络里的手机可以用这台电脑的局域网地址访问（没有登录，请只在可信网络中使用）");
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio");
    rt.block_on(async move {
        tokio::select! {
            r = serve(state, cfg, listener) => if let Err(e) = r { eprintln!("服务异常退出：{e}"); std::process::exit(1); },
            _ = tokio::signal::ctrl_c() => {}
        }
    });
}
