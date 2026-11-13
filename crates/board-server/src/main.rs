//! `board-server` —— Board 服务端：HTTP API + 实时推送，并托管界面。
//!
//! ```text
//! board-server [--host 127.0.0.1] [--port 7420] [--db <路径>] [--demo] [--local]
//! board-server token new <名字> | token ls | token revoke <ID>
//! board-server backup <目标文件>
//! board-server healthcheck
//! ```
//!
//! 线上用 GitHub 登录，配置见 USAGE 里的环境变量；`--local` 是本机模式：只监听本机地址，不需要登录。

use board_core::Store;
use board_server::auth::{self, Auth, GitHub};
use board_server::{bind, serve, AppState, Config};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

const USAGE: &str = "用法：
  board-server [--host 127.0.0.1] [--port 7420] [--db <路径>] [--demo] [--local]
  board-server token new <名字>       新建 agent 访问令牌（原文只显示这一次）
  board-server token ls               列出访问令牌
  board-server token revoke <ID>      吊销访问令牌
  board-server backup <目标文件>      把数据库在线备份到一个新文件；附件补到它旁边的 files/
  board-server healthcheck            检查本机服务是否正常（容器健康检查用）

参数：
  --host   监听地址，默认 $BOARD_HOST 或 127.0.0.1
  --port   端口，默认 $BOARD_PORT 或 7420
  --db     数据库文件，默认 $BOARD_DB 或 ~/.board/board.db（只有服务端读写它）
  --demo   数据库为空时写入示例项目
  --local  本机模式：只能监听 127.0.0.1 / ::1，不需要登录

环境变量（线上，用 GitHub 登录）：
  BOARD_PUBLIC_URL            对外地址，例如 https://board.example.com
  BOARD_GITHUB_CLIENT_ID      GitHub OAuth App 的 Client ID
  BOARD_GITHUB_CLIENT_SECRET  GitHub OAuth App 的 Client secret
  BOARD_GITHUB_USER           允许登录的 GitHub 账号
  RUST_LOG                    日志级别，默认 info";

fn fail(msg: &str) -> ! {
    eprintln!("{msg}\n\n{USAGE}");
    std::process::exit(2);
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

struct Opts {
    host: Option<IpAddr>,
    port: u16,
    db: PathBuf,
    demo: bool,
    local: bool,
    rest: Vec<String>,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts {
        host: None,
        port: env("BOARD_PORT").map(|p| p.parse().unwrap_or_else(|_| fail("BOARD_PORT 需要一个端口号"))).unwrap_or(7420),
        db: Store::default_path(),
        demo: false,
        local: false,
        rest: vec![],
    };
    let mut i = 0;
    while i < args.len() {
        let next = |i: usize| args.get(i + 1).cloned().unwrap_or_else(|| fail(&format!("{} 需要一个值", args[i])));
        match args[i].as_str() {
            "--host" => {
                o.host = Some(next(i).parse().unwrap_or_else(|_| fail("--host 需要一个 IP 地址")));
                i += 1;
            }
            "--port" => {
                o.port = next(i).parse().unwrap_or_else(|_| fail("--port 需要一个端口号"));
                i += 1;
            }
            "--db" => {
                o.db = PathBuf::from(next(i));
                i += 1;
            }
            "--demo" => o.demo = true,
            "--local" => o.local = true,
            "-h" | "--help" | "help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other if other.starts_with("--") => fail(&format!("未知参数 {other}")),
            other => o.rest.push(other.to_string()),
        }
        i += 1;
    }
    o
}

fn main() {
    use std::io::IsTerminal;
    tracing_subscriber::fmt()
        .with_ansi(std::io::stdout().is_terminal())
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = parse(&args);
    match o.rest.first().map(String::as_str) {
        None => run(o),
        Some("token") => token(&o),
        Some("backup") => backup(&o),
        Some("healthcheck") => healthcheck(&o),
        Some(other) => fail(&format!("未知命令 {other}")),
    }
}

/* ------------------------------ 服务 ------------------------------ */

fn github_from_env() -> GitHub {
    let need = [
        "BOARD_PUBLIC_URL",
        "BOARD_GITHUB_CLIENT_ID",
        "BOARD_GITHUB_CLIENT_SECRET",
        "BOARD_GITHUB_USER",
    ];
    let missing: Vec<&str> = need.iter().copied().filter(|k| env(k).is_none()).collect();
    if !missing.is_empty() {
        fail(&format!(
            "没有配置 GitHub 登录，缺少环境变量：{}\n只在本机使用时加 --local（不需要登录，只监听本机地址）",
            missing.join("、")
        ));
    }
    let public_url = env("BOARD_PUBLIC_URL").unwrap_or_default().trim_end_matches('/').to_string();
    if !public_url.starts_with("https://") && !public_url.starts_with("http://") {
        fail("BOARD_PUBLIC_URL 需要以 https:// 或 http:// 开头");
    }
    GitHub {
        client_id: env("BOARD_GITHUB_CLIENT_ID").unwrap_or_default(),
        client_secret: env("BOARD_GITHUB_CLIENT_SECRET").unwrap_or_default(),
        user: env("BOARD_GITHUB_USER").unwrap_or_default(),
        public_url,
        // 只在测试时改：指向模拟的 GitHub 授权服务
        oauth_url: env("BOARD_GITHUB_OAUTH_URL").unwrap_or_else(|| "https://github.com".into()),
        api_url: env("BOARD_GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into()),
    }
}

fn run(o: Opts) {
    let default_host: IpAddr = [127, 0, 0, 1].into();
    let host = o
        .host
        .or_else(|| env("BOARD_HOST").map(|h| h.parse().unwrap_or_else(|_| fail("BOARD_HOST 需要一个 IP 地址"))))
        .unwrap_or(default_host);
    let auth = if o.local {
        if !host.is_loopback() {
            fail("本机模式（--local）只能监听 127.0.0.1 或 ::1；要在网络上提供服务，请配置 GitHub 登录");
        }
        // 本机来的请求没有令牌：`GET /api/hosts/mine`（board check --agent 自己拼名字时用）按本机认机器
        std::env::set_var("BOARD_LOCAL", "1");
        Auth::Local
    } else {
        Auth::GitHub(github_from_env())
    };

    let cfg = Config { db: o.db.clone(), demo: o.demo, serve_web: true, auth: auth.clone() };
    let state = AppState::open(&cfg).unwrap_or_else(|e| fail(&format!("打开数据库失败：{e}")));
    let addr = SocketAddr::new(host, o.port);
    let listener = bind(addr).unwrap_or_else(|e| fail(&format!("无法监听 {addr}：{e}")));

    let shown = if host.is_unspecified() { format!("http://127.0.0.1:{}", o.port) } else { format!("http://{addr}") };
    println!("Board 服务端：{shown}");
    println!("数据库：{}", o.db.display());
    match &auth {
        Auth::Local => println!("本机模式：不需要登录，只接受本机访问"),
        Auth::GitHub(gh) => println!("登录：GitHub，只允许 {}；对外地址 {}", gh.user, gh.public_url),
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio");
    rt.block_on(async move {
        tokio::select! {
            r = serve(state, cfg, listener) => if let Err(e) = r { eprintln!("服务异常退出：{e}"); std::process::exit(1); },
            _ = shutdown() => tracing::info!("收到退出信号，服务停止"),
        }
    });
}

/// Ctrl-C 或 SIGTERM（docker stop）
async fn shutdown() {
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let on_signal = notify.clone();
    ctrlc::set_handler(move || on_signal.notify_one()).expect("信号处理");
    notify.notified().await;
}

/* ------------------------------ 子命令 ------------------------------ */

fn open_store(o: &Opts) -> Store {
    let store = Store::open(&o.db).unwrap_or_else(|e| fail(&format!("打开数据库 {} 失败：{e}", o.db.display())));
    auth::setup(store.conn()).unwrap_or_else(|e| fail(&format!("初始化登录数据失败：{e}")));
    store
}

fn ts(ms: Option<i64>) -> String {
    use chrono::TimeZone;
    ms.and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single())
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

fn token(o: &Opts) {
    let store = open_store(o);
    let conn = store.conn();
    match o.rest.get(1).map(String::as_str) {
        Some("new") => {
            let name = o.rest[2..].join(" ");
            let (info, raw) = auth::create_token(conn, &name, board_core::now_ms()).unwrap_or_else(|e| fail(&e.message));
            println!("已新建令牌 #{} {}：", info.id, info.name);
            println!("{raw}");
            println!("这是唯一一次显示原文。agent 使用：export BOARD_TOKEN={raw}");
        }
        Some("ls") => {
            let list = auth::list_tokens(conn).unwrap_or_else(|e| fail(&e.message));
            if list.is_empty() {
                println!("（还没有访问令牌）");
            }
            for t in list {
                println!("#{:<4}{:<20}{:<14}创建 {}  最近使用 {}", t.id, t.name, t.hint, ts(Some(t.created_at)), ts(t.last_used_at));
            }
        }
        Some("revoke") => {
            let id: i64 = o.rest.get(2).and_then(|s| s.trim_start_matches('#').parse().ok()).unwrap_or_else(|| fail("token revoke 需要令牌 ID"));
            auth::revoke_token(conn, id).unwrap_or_else(|e| fail(&e.message));
            println!("已吊销令牌 #{id}");
        }
        _ => fail("token 后面需要 new、ls 或 revoke"),
    }
}

/// 用 `VACUUM INTO` 得到一份一致的快照，服务运行时也可以执行
fn backup(o: &Opts) {
    let Some(dest) = o.rest.get(1) else { fail("backup 需要目标文件路径") };
    let dest = PathBuf::from(dest);
    if dest.exists() {
        fail(&format!("{} 已存在，请换一个文件名", dest.display()));
    }
    if let Some(dir) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(&format!("无法创建目录 {}：{e}", dir.display())));
    }
    let conn = rusqlite::Connection::open(&o.db).unwrap_or_else(|e| fail(&format!("打开数据库失败：{e}")));
    let _ = conn.busy_timeout(std::time::Duration::from_secs(30));
    conn.execute("VACUUM INTO ?1", [dest.to_string_lossy()])
        .unwrap_or_else(|e| fail(&format!("备份失败：{e}")));
    println!("已备份到 {}", dest.display());
    // 附件：备份旁边的 files/ 里补上还没有的文件（文件按内容命名、不会改动，所有备份共用一份）
    let (src, dst) = (board_core::files::dir_for(&o.db), board_core::files::dir_for(&dest));
    match board_core::files::copy_new(&src, &dst) {
        Ok(n) => println!("附件：新复制 {n} 个文件到 {}", dst.display()),
        Err(e) => fail(&format!("备份附件失败：{e}")),
    }
}

fn healthcheck(o: &Opts) {
    let url = format!("http://127.0.0.1:{}/api/health", o.port);
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(3))).build().into();
    match agent.get(&url).call() {
        Ok(r) if r.status().is_success() => println!("ok"),
        Ok(r) => {
            eprintln!("{url} 返回 {}", r.status());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{url} 连接失败：{e}");
            std::process::exit(1);
        }
    }
}
