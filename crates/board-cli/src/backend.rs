//! 数据从哪来：本地数据库，或 Board 服务端（PRD §9）。
//!
//! - 没有设置 `BOARD_URL`：直接读写本地 SQLite（默认 `~/.board/board.db`，`BOARD_DB` 可指定）。
//! - 设置了 `BOARD_URL`：通过 HTTP API 访问服务端，用 `BOARD_TOKEN` 认证。规则在服务端执行，
//!   被拒绝时服务端返回同样的错误码，这里原样还原。
//!
//! 两种模式共用同一套命令和输出。

use board_core::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::time::Duration;

/// 一块白板上命令需要的数据
pub struct BoardData {
    pub path: String,
    pub flow: Flow,
    /// 按工作流和状态内顺序排列（「未整理」在最前）
    pub cards: Vec<Card>,
    /// 白板所属项目的标签
    pub labels: Vec<Label>,
    pub now: i64,
}

pub enum Backend {
    Local(Store),
    Remote(Remote),
}

pub fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl Backend {
    pub fn open() -> Result<Backend> {
        match env("BOARD_URL") {
            Some(url) => Ok(Backend::Remote(Remote::new(&url)?)),
            None => {
                let now = now_ms();
                let mut store = Store::open(&Store::default_path())?;
                store.ensure_default(now)?;
                store.sweep(now)?;
                Ok(Backend::Local(store))
            }
        }
    }

    /// `board help` 里「数据」一行
    pub fn describe() -> String {
        match env("BOARD_URL") {
            Some(url) => format!("服务端 {url}（BOARD_URL，令牌 BOARD_TOKEN）"),
            None => format!("{}（BOARD_DB 可指定；设置 BOARD_URL 和 BOARD_TOKEN 改为连接服务端）", Store::default_path().display()),
        }
    }

    pub fn workspace(&mut self) -> Result<Workspace> {
        match self {
            Backend::Local(s) => s.read(|r| r.workspace(now_ms())),
            Backend::Remote(r) => r.get("/api/workspace", &[]),
        }
    }

    pub fn board(&mut self, raw: &str) -> Result<BoardData> {
        match self {
            Backend::Local(s) => s.read(|r| {
                let b = r.resolve(raw)?;
                Ok(BoardData {
                    path: b.path(),
                    flow: r.flow(&b)?,
                    cards: r.cards(&b)?,
                    labels: r.labels(&b.project)?,
                    now: now_ms(),
                })
            }),
            Backend::Remote(r) => {
                let mut snap: BoardSnapshot = r.get("/api/board", &[("path", raw)])?;
                // 与本地 ls 相同的顺序：未整理，再按工作流顺序，状态内按位置
                let mut order: Vec<String> = vec![INBOX.to_string()];
                order.extend(snap.statuses.iter().map(|s| s.key.clone()));
                order.extend(snap.cols.keys().filter(|k| !order.contains(k)).cloned().collect::<Vec<_>>());
                let mut cards = vec![];
                for col in &order {
                    for id in snap.cols.get(col).into_iter().flatten() {
                        if let Some(c) = snap.cards.remove(id) {
                            cards.push(c);
                        }
                    }
                }
                let project = snap.path.split('/').next().unwrap_or_default().to_string();
                let ws: Workspace = r.get("/api/workspace", &[])?;
                let labels = ws.projects.into_iter().find(|p| p.key == project).map(|p| p.labels).unwrap_or_default();
                Ok(BoardData { path: snap.path, flow: Flow(snap.statuses), cards, labels, now: snap.now })
            }
        }
    }

    pub fn card(&mut self, path: &str, id: i64) -> Result<CardDetail> {
        match self {
            Backend::Local(s) => s.read(|r| {
                let b = r.resolve(path)?;
                r.card(&b, id)
            }),
            Backend::Remote(r) => r.get("/api/card", &[("path", path), ("id", &id.to_string())]),
        }
    }

    /// 上传附件到卡片
    pub fn attach(&mut self, path: &str, id: i64, actor: &Actor, name: &str, bytes: &[u8]) -> Result<Attachment> {
        match self {
            Backend::Local(s) => {
                let b = s.read(|r| r.resolve(path))?;
                s.attach(now_ms(), &b, Some(id), actor, name, bytes)
            }
            Backend::Remote(r) => {
                let id = id.to_string();
                r.upload(&[("board", path), ("card", &id), ("name", name), ("as", &actor.name)], bytes)
            }
        }
    }

    /// 读取附件内容
    pub fn download(&mut self, path: &str, a: &Attachment) -> Result<Vec<u8>> {
        match self {
            Backend::Local(s) => {
                let b = s.read(|r| r.resolve(path))?;
                match s.attachment(a.id)? {
                    Some((_, board, file)) if board == b.id => Ok(std::fs::read(file)?),
                    _ => Err(BoardError::new(ErrorCode::NotFound, format!("附件 {} 不存在", a.id))),
                }
            }
            Backend::Remote(r) => r.bytes(&a.url),
        }
    }

    pub fn exec(&mut self, path: &str, actor: &Actor, cmd: Cmd) -> Result<CardDetail> {
        match self {
            Backend::Local(s) => s.write(now_ms(), Origin::cli(), |w| {
                let b = w.resolve(path)?;
                w.exec(&b, actor, cmd)
            }),
            Backend::Remote(r) => {
                let mut body = cmd_json(&cmd);
                body["board"] = json!(path);
                body["as"] = json!(actor.name);
                r.post("/api/cmd", body)
            }
        }
    }
}

/// 卡片命令的 JSON（与界面发给服务端的格式相同）
fn cmd_json(cmd: &Cmd) -> Value {
    match cmd {
        Cmd::New { title, body, column, top, labels } => {
            json!({ "op": "new", "title": title, "body": body, "column": column, "top": top, "labels": labels })
        }
        Cmd::Edit { id, title, body } => json!({ "op": "edit", "id": id, "title": title, "body": body }),
        Cmd::Move { id, column, top, index } => json!({ "op": "move", "id": id, "column": column, "top": top, "index": index }),
        Cmd::Claim { id } => json!({ "op": "claim", "id": id }),
        Cmd::Comment { id, text } => json!({ "op": "comment", "id": id, "text": text }),
        Cmd::Label { id, add, remove } => json!({ "op": "label", "id": id, "add": add, "remove": remove }),
        Cmd::Release { id } => json!({ "op": "release", "id": id }),
        Cmd::Delete { id } => json!({ "op": "delete", "id": id }),
    }
}

/* ------------------------------ 远程 ------------------------------ */

pub struct Remote {
    url: String,
    token: String,
    agent: ureq::Agent,
}

impl Remote {
    fn new(url: &str) -> Result<Remote> {
        let url = url.trim_end_matches('/').to_string();
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(BoardError::usage(format!("BOARD_URL 需要以 https:// 或 http:// 开头：{url}")));
        }
        let Some(token) = env("BOARD_TOKEN") else {
            return Err(BoardError::new(
                ErrorCode::NotLoggedIn,
                "设置了 BOARD_URL 但没有 BOARD_TOKEN：在界面「设置 → Agent 接入」新建访问令牌，再 export BOARD_TOKEN=<令牌>",
            ));
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(concat!("board-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Ok(Remote { url, token, agent })
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let mut req = self.agent.get(format!("{}{path}", self.url)).header("authorization", format!("Bearer {}", self.token));
        for (k, v) in query {
            req = req.query(*k, *v);
        }
        self.handle(req.call())
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: Value) -> Result<T> {
        let req = self.agent.post(format!("{}{path}", self.url)).header("authorization", format!("Bearer {}", self.token));
        self.handle(req.send_json(body))
    }

    /// 上传附件：请求体是文件内容
    fn upload(&self, query: &[(&str, &str)], bytes: &[u8]) -> Result<Attachment> {
        let mut req = self.agent.post(format!("{}/api/files", self.url)).header("authorization", format!("Bearer {}", self.token));
        for (k, v) in query {
            req = req.query(*k, *v);
        }
        self.handle(req.send(bytes))
    }

    /// 读取附件内容（地址是服务端给的 /api/files/<id>）
    fn bytes(&self, path: &str) -> Result<Vec<u8>> {
        let res = self.agent.get(format!("{}{path}", self.url)).header("authorization", format!("Bearer {}", self.token)).call();
        let mut res = res.map_err(|e| BoardError::new(ErrorCode::Unreachable, format!("连不上 {}：{e}", self.url)))?;
        if !res.status().is_success() {
            return self.fail(res);
        }
        res.body_mut()
            .with_config()
            .limit(files::MAX_BYTES as u64 + 1024)
            .read_to_vec()
            .map_err(|e| BoardError::new(ErrorCode::Unreachable, format!("下载 {path} 失败：{e}")))
    }

    fn handle<T: DeserializeOwned>(&self, res: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<T> {
        let mut res = res.map_err(|e| BoardError::new(ErrorCode::Unreachable, format!("连不上 {}：{e}", self.url)))?;
        if !res.status().is_success() {
            return self.fail(res);
        }
        let text = res
            .body_mut()
            .read_to_string()
            .map_err(|e| BoardError::new(ErrorCode::Unreachable, format!("读取 {} 的响应失败：{e}", self.url)))?;
        serde_json::from_str(&text).map_err(|e| BoardError::new(ErrorCode::Internal, format!("服务端返回的数据无法解析：{e}")))
    }

    /// 服务端拒绝时还原错误码
    fn fail<T>(&self, mut res: ureq::http::Response<ureq::Body>) -> Result<T> {
        let status = res.status();
        let text = res.body_mut().read_to_string().unwrap_or_default();
        // 服务端的拒绝：{"error": {"code", "message", "detail"}}
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let e = &v["error"];
        match e["code"].as_str().and_then(ErrorCode::parse) {
            Some(code) => {
                let mut err = BoardError::new(code, e["message"].as_str().unwrap_or(code.message()));
                if !e["detail"].is_null() {
                    err = err.with_detail(e["detail"].clone());
                }
                Err(err)
            }
            // 不是 Board 的响应（例如反向代理返回 502）
            None => Err(BoardError::new(ErrorCode::Unreachable, format!("{} 返回 HTTP {status}", self.url))),
        }
    }
}
