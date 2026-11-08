//! 集成测试的公共部分：每个测试起一个线上配置（GitHub 登录）的 board-server，令牌直接写进它的数据库，
//! CLI 通过 BOARD_URL + BOARD_TOKEN 连它。HOME 指向临时目录，测试不会碰开发者的 ~/.board。
#![allow(dead_code)]

use board_server::auth::{self, Auth, GitHub};
use board_server::{bind, serve_in_thread, AppState, Config};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Run {
    pub code: i32,
    pub out: String,
    pub err: String,
}

impl Run {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.out).unwrap_or_else(|e| panic!("不是合法 JSON：{e}\n{}{}", self.out, self.err))
    }
    pub fn error_code(&self) -> String {
        self.json()["error"]["code"].as_str().unwrap_or_default().to_string()
    }
}

pub struct Server {
    pub url: String,
    /// 服务端的数据库（测试里直接写，例如新建标签、吊销令牌）
    pub db: PathBuf,
    pub token: String,
    /// CLI 进程的 HOME
    pub home: PathBuf,
    _dir: tempfile::TempDir,
}

pub fn start() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("server.db");
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
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
    Server { url, db, token, home, _dir: dir }
}

impl Server {
    pub fn env(&self) -> Vec<(&'static str, String)> {
        vec![("BOARD_URL", self.url.clone()), ("BOARD_TOKEN", self.token.clone())]
    }
    /// 以 agent `who` 在白板 default 上执行
    pub fn run(&self, who: &str, args: &[&str]) -> Run {
        let mut env = self.env();
        env.push(("BOARD_AS", who.into()));
        env.push(("BOARD", "default".into()));
        board(&self.home, &env, args)
    }
    /// 只带给定的环境变量执行（连接服务端的变量也要自己给）
    pub fn raw(&self, env: &[(&str, String)], args: &[&str]) -> Run {
        board(&self.home, env, args)
    }
    /// 直接在服务端数据库里新建标签（标签只能在界面上新建）
    pub fn create_label(&self, name: &str) {
        let mut store = board_core::Store::open(&self.db).unwrap();
        store.write(board_core::now_ms(), board_core::Origin::ui(None), |w| w.create_label("default", name, None).map(|_| ())).unwrap();
    }
}

/// 去掉开发者环境里连接线上服务端的变量，免得测试写到真实白板上
pub fn board(home: &Path, env: &[(&str, String)], args: &[&str]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_board"));
    cmd.args(args)
        .env("HOME", home)
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
    Run {
        code: o.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&o.stdout).into(),
        err: String::from_utf8_lossy(&o.stderr).into(),
    }
}
