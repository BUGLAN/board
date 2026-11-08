//! \`board doctor\` 的验收：配置正确时逐项通过、退出 0；PATH、服务端地址、令牌、服务端
//! 四种问题都能给出具体原因和下一步，退出码非 0；--json 每项有明确的 key 和布尔状态；
//! 地址和令牌写明来自环境变量还是 ~/.board/config.toml，两处都有时 warn；名字、白板没提供时跳过。
//!
//! 每个用例启动一个线上配置（GitHub 登录）的 board-server，令牌直接写进它的数据库。
//! HOME 指向临时目录，不会读开发者的 ~/.board/config.toml。

use board_server::auth::{self, Auth, GitHub};
use board_server::{bind, serve_in_thread, AppState, Config};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.out).unwrap_or_else(|e| panic!("不是合法 JSON：{e}\n{}{}", self.out, self.err))
    }
}

fn check<'a>(v: &'a Value, key: &str) -> &'a Value {
    v["checks"]
        .as_array()
        .unwrap_or_else(|| panic!("输出里没有 checks：{v}"))
        .iter()
        .find(|c| c["key"] == key)
        .unwrap_or_else(|| panic!("缺少检查项 {key}：{v}"))
}

/// 配置路径文案：Windows 是 %USERPROFILE%\.board\config.toml，其他系统是 ~/.board/config.toml
fn config_shown() -> &'static str {
    if cfg!(windows) {
        r"%USERPROFILE%\.board\config.toml"
    } else {
        "~/.board/config.toml"
    }
}

struct Server {
    url: String,
    db: PathBuf,
    token: String,
    _dir: tempfile::TempDir,
}

fn start() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("server.db");
    let cfg = Config {
        db: db.clone(),
        demo: false,
        serve_web: false,
        // 令牌认证不会访问 GitHub
        auth: Auth::GitHub(GitHub {
            client_id: "cid".into(),
            client_secret: "secret".into(),
            user: "buglan".into(),
            public_url: "https://board.test".into(),
            oauth_url: "http://127.0.0.1:9".into(),
            api_url: "http://127.0.0.1:9".into(),
        }),
    };
    let state = AppState::open(&cfg).unwrap();
    let l = bind(([127, 0, 0, 1], 0).into()).unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    serve_in_thread(state, cfg, l);
    let token = {
        let store = board_core::Store::open(&db).unwrap();
        auth::create_token(store.conn(), "test", board_core::now_ms()).unwrap().1
    };
    Server { url, db, token, _dir: dir }
}

impl Server {
    /// 连线上服务端的配置，PATH 指向 board 所在的目录
    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("BOARD_URL", self.url.clone()),
            ("BOARD_TOKEN", self.token.clone()),
            ("BOARD_AS", "test/alpha".into()),
            ("BOARD", "default".into()),
        ]
    }
    fn doctor(&self, args: &[&str]) -> Run {
        board(&self.env(), args)
    }
}

/// board 可执行文件所在的目录：把它当 PATH，等于「board 已经装好」
fn exe_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_board")).parent().unwrap().to_path_buf()
}

fn board(env: &[(&str, String)], args: &[&str]) -> Run {
    board_with_path(&exe_dir(), env, args)
}

/// 没有配置文件的 HOME（一个新的空目录）
fn empty_home() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn board_with_path(path: &Path, env: &[(&str, String)], args: &[&str]) -> Run {
    let home = empty_home();
    board_in(home.path(), path, env, args)
}

fn board_in(home: &Path, path: &Path, env: &[(&str, String)], args: &[&str]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_board"));
    // 去掉开发者环境里的变量，免得太空或太全影响检查结果
    cmd.args(args)
        .env("PATH", path)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("NO_COLOR", "1")
        .env_remove("BOARD")
        .env_remove("BOARD_AS")
        .env_remove("BOARD_URL")
        .env_remove("BOARD_TOKEN")
        .env_remove("BOARD_DB");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().unwrap();
    Run { code: o.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&o.stdout).into(), err: String::from_utf8_lossy(&o.stderr).into() }
}

/* ------------------------------ 用例 ------------------------------ */

/// 配置正确：逐项打勾、退出 0，服务端版本来自 /api/health，不打印令牌原文
#[test]
fn doctor_all_green_against_live_server() {
    let s = start();
    let env = s.env();
    // 名字要在线
    let r = board(&env, &["check", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let r = board(&env, &["doctor", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let v = r.json();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["summary"]["failed"].as_u64(), Some(0), "{v}");
    for key in ["board_path", "path", "board_url", "server", "board_token", "board_as", "board"] {
        let c = check(&v, key);
        assert_eq!(c["ok"], true, "{key} 没通过：{v}");
        assert_eq!(c["status"], "ok", "{key}：{v}");
    }
    assert_eq!(v["server"]["reachable"], true);
    assert!(!v["server"]["version"].as_str().unwrap_or_default().is_empty(), "要带服务端版本：{v}");
    assert!(!v["version"].as_str().unwrap_or_default().is_empty(), "要带 CLI 版本：{v}");
    assert!(!r.out.contains(&s.token), "不能打印令牌原文");
    assert!(!r.out.contains("Bearer"), "输出里不该出现 Authorization 头");

    // 人读版本：逐项打勾 + 一段结论
    let r = board(&env, &["doctor"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains('✓') && r.out.contains("全部通过"), "{}", r.out);
    assert!(r.out.contains("服务端版本"), "{}", r.out);
    assert!(!r.out.contains(&s.token), "{}", r.out);
}

/// PATH 里找不到 board：说清原因和怎么加进 PATH，其他项照常检查
#[test]
fn doctor_reports_missing_board_on_path() {
    let s = start();
    let empty = tempfile::tempdir().unwrap();
    let env = s.env();
    let r = board_with_path(empty.path(), &env, &["doctor", "--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    let v = r.json();
    let c = check(&v, "path");
    assert_eq!(c["ok"], false);
    assert_eq!(c["status"], "fail");
    assert!(c["detail"].as_str().unwrap_or_default().contains("PATH"), "{c}");
    let next = c["next"].as_str().unwrap_or_default();
    assert!(next.contains("PATH") && next.contains("export"), "下一步要能照着做：{next}");
    // 其他配置都是好的
    assert_eq!(check(&v, "board_url")["ok"], true);
    assert_eq!(check(&v, "board_token")["ok"], true);
    // 文本模式同样有原因和下一步
    let r = board_with_path(empty.path(), &env, &["doctor"]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("PATH 里找不到 board") && r.out.contains("下一步："), "{}", r.out);
}

/// 没设 BOARD_URL：board 连不上白板，要指出去哪里配置
#[test]
fn doctor_reports_missing_board_url() {
    let env = vec![("BOARD_AS", "test/alpha".to_string()), ("BOARD", "default".to_string())];
    let r = board(&env, &["doctor", "--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    let v = r.json();
    let c = check(&v, "board_url");
    assert_eq!(c["ok"], false);
    assert_eq!(c["status"], "fail");
    assert!(c["detail"].as_str().unwrap_or_default().contains("没有配置"), "{c}");
    assert!(c["next"].as_str().unwrap_or_default().contains("board login --url"), "{c}");
    // 依赖服务端地址的检查明确标成跳过
    assert_eq!(check(&v, "server")["status"], "skip");
    assert_eq!(check(&v, "board_token")["status"], "skip");
    // 名字格式对，但没法检查是否在线；白板已经在
    assert_eq!(check(&v, "board_as")["status"], "skip");
    assert_eq!(check(&v, "board")["ok"], true);
}

/// 令牌失效：写错的令牌与被吊销的令牌都要报「令牌」问题，且不能回显令牌
#[test]
fn doctor_reports_invalid_and_revoked_token() {
    let s = start();
    let mut env = s.env();
    for (k, v) in env.iter_mut() {
        if *k == "BOARD_TOKEN" {
            *v = "bd_nope".to_string();
        }
    }
    let r = board(&env, &["doctor", "--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    let v = r.json();
    let c = check(&v, "board_token");
    assert_eq!(c["ok"], false);
    assert_eq!(c["status"], "fail");
    assert!(c["detail"].as_str().unwrap_or_default().contains("令牌"), "{c}");
    assert!(c["next"].as_str().unwrap_or_default().contains("Agent 接入"), "{c}");
    assert!(!r.out.contains("bd_nope"), "令牌原文（哪怕是错的）也不该出现");

    // 令牌被吊销
    {
        let store = board_core::Store::open(&s.db).unwrap();
        let id = auth::list_tokens(store.conn()).unwrap()[0].id;
        auth::revoke_token(store.conn(), id).unwrap();
    }
    let r = s.doctor(&["doctor", "--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    let v = r.json();
    assert_eq!(check(&v, "board_token")["status"], "fail", "{v}");
    assert!(!r.out.contains(&s.token), "不能打印令牌原文");
}

/// 服务端连不上：报 UNREACHABLE 的实话，并给出排查方向
#[test]
fn doctor_reports_unreachable_server() {
    let env = vec![
        ("BOARD_URL", "http://127.0.0.1:9".to_string()),
        ("BOARD_TOKEN", "bd_secret".to_string()),
        ("BOARD_AS", "test/alpha".to_string()),
        ("BOARD", "default".to_string()),
    ];
    let r = board(&env, &["doctor", "--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    let v = r.json();
    let c = check(&v, "server");
    assert_eq!(c["ok"], false);
    assert_eq!(c["status"], "fail");
    assert!(c["detail"].as_str().unwrap_or_default().contains("连不上"), "{c}");
    assert!(c["next"].as_str().unwrap_or_default().contains("127.0.0.1:9"), "下一步要指向这个地址：{c}");
    // 服务端不通就没法验令牌
    assert_eq!(check(&v, "board_token")["status"], "skip");
    // 前端检查（路径、URL 格式）仍然是好的
    assert_eq!(check(&v, "board_path")["ok"], true);
    assert_eq!(check(&v, "board_url")["ok"], true);
    assert!(!r.out.contains("bd_secret"), "不能打印令牌原文");
}

/// doctor 不需要身份和白板也能跑，并在 stderr/stdout 里说清缺什么；help 里列出来
#[test]
fn doctor_does_not_require_identity_and_is_in_help() {
    let r = board(&[], &["doctor", "--json"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    let v = r.json();
    assert!(v["checks"].is_array(), "doctor 不该返回 NO_IDENTITY/NO_BOARD 错误：{v}");
    // 名字和白板没提供时跳过，不判失败
    assert_eq!(check(&v, "board_as")["status"], "skip");
    assert_eq!(check(&v, "board")["status"], "skip");

    let r = board(&[], &["help"]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("board doctor"), "{}", r.out);
    assert!(r.out.contains("BOARD_URL/BOARD_TOKEN"), "{}", r.out);
}

/// 拼错的参数仍然按 USAGE 拒绝
#[test]
fn doctor_rejects_unknown_flags() {
    let r = board(&[], &["doctor", "--nope", "--json"]);
    assert_eq!(r.code, 1);
    assert_eq!(r.json()["error"]["code"], "USAGE");
}

/// 配置文件（board login 写的）：来源写明是 ~/.board/config.toml；环境变量也有时 warn；
/// 名字通过 --as 给出时检查是否在线
#[test]
fn doctor_reports_config_file_source_and_env_override() {
    let s = start();
    let dir = empty_home();
    let home = dir.path().to_path_buf();
    let r = board_in(&home, &exe_dir(), &[], &["login", "--url", &s.url, "--token", &s.token, "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);

    // 只有配置文件
    let r = board_in(&home, &exe_dir(), &[], &["doctor", "--json"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let v = r.json();
    for key in ["board_url", "board_token"] {
        let c = check(&v, key);
        assert_eq!((c["status"].as_str(), c["source"].as_str()), (Some("ok"), Some("file")), "{c}");
        assert!(c["detail"].as_str().unwrap().contains(config_shown()), "{c}");
    }
    assert_eq!(check(&v, "board_as")["status"], "skip");
    assert_eq!(check(&v, "board")["status"], "skip");

    // 名字：格式不对 fail；没 check 不在线 fail；check 之后通过
    let r = board_in(&home, &exe_dir(), &[], &["doctor", "--as", "claude", "--json"]);
    assert_eq!(check(&r.json(), "board_as")["status"], "fail");
    let r = board_in(&home, &exe_dir(), &[], &["doctor", "--as", "test/alpha", "--board", "default", "--json"]);
    let c = check(&r.json(), "board_as").clone();
    assert_eq!(c["status"], "fail");
    assert!(c["next"].as_str().unwrap().contains("board check --as test/alpha"), "{c}");
    assert_eq!(board_in(&home, &exe_dir(), &[], &["check", "--as", "test/alpha", "--json"]).code, 0);
    let r = board_in(&home, &exe_dir(), &[], &["doctor", "--as", "test/alpha", "--board", "default", "--json"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(check(&r.json(), "board_as")["status"], "ok");
    assert_eq!(check(&r.json(), "board")["status"], "ok");

    // 环境变量和配置文件都有：warn，说明环境变量覆盖了配置文件
    let env = vec![("BOARD_URL", s.url.clone()), ("BOARD_TOKEN", s.token.clone())];
    let r = board_in(&home, &exe_dir(), &env, &["doctor", "--json"]);
    assert_eq!(r.code, 0, "warn 不算失败：{}", r.out);
    let v = r.json();
    for key in ["board_url", "board_token"] {
        let c = check(&v, key);
        assert_eq!((c["status"].as_str(), c["source"].as_str()), (Some("warn"), Some("env")), "{c}");
        assert!(c["detail"].as_str().unwrap().contains("覆盖"), "{c}");
    }
    let r = board_in(&home, &exe_dir(), &env, &["doctor"]);
    assert!(r.out.contains("环境变量 BOARD_URL") && r.out.contains('!'), "{}", r.out);
    assert!(!r.out.contains(&s.token), "不能打印令牌原文");
}
