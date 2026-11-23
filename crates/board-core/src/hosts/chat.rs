//! 机器对话（PRD §9.1「机器对话」，default/main#591）：人在一台机器上和 agent 来回说话，做不挂在卡片上的事
//! （清理空间、装软件、改配置）。
//!
//! - **不是派发**：没有卡片、没有白板，不占会话位（不算进「最多几个会话」，不受接单时段、暂停接单、连续异常退出暂停的
//!   限制，也不影响自动领取），不出现在白板、卡片动态、大盘和角色成绩里。存在自己的表（`chats`、`chat_events`）。
//! - **一直留着**：没有结束，没有超时；随时能接着发。board host 上的 agent 进程空闲一阵会退出，下一条消息来时重新启动，
//!   用记下的 ACP 会话（`acp`）接回同一个会话（`board acp` 先 `session/resume`，不支持再 `session/load`）。
//! - **一轮一轮**：人发一条（[`W::chat_prompt`]），排在 `prompts` 里、`turn` 变成 `busy`；board host 随任务取到
//!   （[`HostWork::chats`]），交给 agent 后回报收下到第几条（`took`），这一轮结束回报 `idle`。这一轮没结束不能发下一条；
//!   可以「停止这一轮」（[`W::chat_cancel`]）：还没交给 board host 的直接撤回，交了的由 board host 发 `session/cancel`。
//! - **只能在登录后的界面上开、发、停**（服务端接口用令牌调时 `NOT_LOGGED_IN`）；board host 用自己的令牌回报（[`W::chat_report`]）。
//! - 人发的每一条消息记成一个事件（`kind: prompt`，原样）；agent 的输出、事件、脱敏、条数上限和派发一样（`runlog.rs`）。
//! - 机器的操作记录写「开始对话（<角色或 agent>）」。机器掉线、board host 重启时还没回答完的那一轮记一条出错，对话留着。
//! - 移除机器时一起删掉；机器改名时跟着改。

use super::*;
use crate::runlog::RunEvent;

pub(crate) const SCHEMA: &str = r#"
-- 机器对话（hosts/chat.rs）：没有卡片、一直留着
CREATE TABLE IF NOT EXISTS chats (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  host_id     INTEGER NOT NULL,
  host        TEXT NOT NULL,
  agent       TEXT NOT NULL,              -- agent 标识：claude …
  agent_label TEXT NOT NULL,              -- agent 名称：Claude Code …
  profile     TEXT NOT NULL DEFAULT '',   -- 按哪个角色开的（角色标识）；空 = 直接选的 agent
  profile_name TEXT NOT NULL DEFAULT '',
  provider    TEXT NOT NULL DEFAULT '',   -- 角色用的供应商；空 = 本机登录
  model       TEXT,
  effort      TEXT,
  env         TEXT,                       -- 启动环境标识（开始时按机器设置定）
  env_name    TEXT,
  title       TEXT NOT NULL,              -- 第一条消息（截到 200 字）
  by_name     TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  last_at     INTEGER NOT NULL,           -- 最近一次有动静：发消息、一轮结束
  turn        TEXT NOT NULL DEFAULT 'idle', -- busy 这一轮在跑（含还没交给 board host 的）| idle 等人发消息
  acp         TEXT,                       -- agent 的 ACP 会话标识：重新启动时接回
  prompts     TEXT NOT NULL DEFAULT '[]', -- 还没交给 board host 的消息（JSON：[{seq, text}]）
  seq         INTEGER NOT NULL DEFAULT 0, -- 一共发过几条
  cancel      INTEGER NOT NULL DEFAULT 0, -- 停止这一轮的请求（> 0 时等 board host 处理）
  log         TEXT NOT NULL DEFAULT '',   -- 输出的最后 500 行
  fmt         TEXT,
  ev_dropped  INTEGER NOT NULL DEFAULT 0,
  project     TEXT NOT NULL DEFAULT '',   -- 在哪个项目的目录里跑（项目标识）；空 = 主目录（#624）
  cwd         TEXT                        -- 开始时按工作空间算出的路径（~ 不展开）；空 = 主目录
);
CREATE INDEX IF NOT EXISTS chats_host ON chats (host_id, last_at);
CREATE TABLE IF NOT EXISTS chat_events (
  id    INTEGER PRIMARY KEY AUTOINCREMENT,
  chat  INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
  kind  TEXT NOT NULL,
  call  TEXT,
  rev   INTEGER NOT NULL,
  data  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS chat_events_chat ON chat_events (chat, id);
CREATE INDEX IF NOT EXISTS chat_events_call ON chat_events (chat, call);
CREATE INDEX IF NOT EXISTS chat_events_rev ON chat_events (rev);
"#;

/// 一条消息最长多少字节
pub const CHAT_TEXT_MAX: usize = 32 * 1024;

/// 一个机器对话（界面「对话」页签的一行）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub id: i64,
    pub host_id: i64,
    pub host: String,
    pub agent: String,
    pub agent_label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub env: Option<String>,
    #[serde(default)]
    pub env_name: Option<String>,
    /// 第一条消息
    pub title: String,
    pub by: String,
    pub created_at: i64,
    /// 最近一次有动静：发消息、一轮结束
    pub last_at: i64,
    /// `busy` 这一轮在跑 | `idle` 等人发消息
    pub turn: String,
    /// 在哪个项目的目录里跑（项目标识）；空 = 主目录
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    /// 实际路径（开始时按这台机器的工作空间算出，`~` 不展开）；空 = 主目录
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// 发给 board host 的一个对话：还没交给它的消息、停止这一轮的请求，以及启动 agent 要用的
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTask {
    pub id: i64,
    pub agent: String,
    #[serde(default)]
    pub env: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub profile_name: String,
    #[serde(default)]
    pub provider: String,
    /// agent 的 ACP 会话标识：有就接回
    #[serde(default)]
    pub acp: Option<String>,
    /// 还没交给它的消息，按先后
    #[serde(default)]
    pub prompts: Vec<ChatPrompt>,
    /// 停止这一轮的请求（> 0 时要停）
    #[serde(default)]
    pub cancel: i64,
    /// 这一轮在跑（含还没交的消息）
    #[serde(default)]
    pub busy: bool,
    /// 在哪个目录里启动（开始时存下的路径，`~` 不展开）；空 = 启动环境里的用户主目录
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatPrompt {
    pub seq: i64,
    pub text: String,
}

/// board host 的回报（`POST /api/chats/{ID}/report`）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReport {
    /// 交给 agent 的消息到第几条（含）
    #[serde(default)]
    pub took: Option<i64>,
    /// agent 的 ACP 会话标识（开了新会话、接回时）
    #[serde(default)]
    pub acp: Option<String>,
    /// 这一轮结束了：结束原因（`end_turn`、`cancelled`、`max_tokens` …）
    #[serde(default)]
    pub idle: Option<String>,
    /// 处理过的停止请求（`ChatTask.cancel`）
    #[serde(default)]
    pub canceled: Option<i64>,
    /// 新增的输出（已脱敏）
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default)]
    pub fmt: Option<String>,
    #[serde(default)]
    pub events: Vec<RunEvent>,
    /// agent 进程退出了（退出码）；这一轮还在跑时记一条出错，对话留着
    #[serde(default)]
    pub exit: Option<i64>,
    /// agent 进程起不来（原因）：这一轮记一条出错，对话留着
    #[serde(default)]
    pub error: Option<String>,
}

/// 一个对话的全部内容（界面右栏）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatView {
    pub chat: Chat,
    pub log: Vec<String>,
    pub fmt: Option<String>,
    pub events: Vec<RunEvent>,
    pub events_from: i64,
    pub events_dropped: i64,
    /// 事件的版本（meta.runs）：下次带上 `since` 只取之后新增、改过的
    pub runs: i64,
}

/// 事件存在哪张表（派发、机器对话共用 [`W::events_append`]）
#[derive(Clone, Copy)]
pub(super) struct EvTable {
    pub owner: &'static str,
    pub events: &'static str,
    pub col: &'static str,
    pub what: &'static str,
}

impl EvTable {
    pub(super) const DISPATCH: EvTable = EvTable { owner: "dispatches", events: "dispatch_events", col: "dispatch", what: "派发" };
    pub(super) const CHAT: EvTable = EvTable { owner: "chats", events: "chat_events", col: "chat", what: "对话" };
}

const CHAT_COLS: &str = "id, host_id, host, agent, agent_label, profile, profile_name, provider, model, effort, env, env_name, title, by_name, created_at, last_at, turn, project, cwd";

fn chat_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Chat> {
    Ok(Chat {
        id: r.get(0)?,
        host_id: r.get(1)?,
        host: r.get(2)?,
        agent: r.get(3)?,
        agent_label: r.get(4)?,
        profile: r.get(5)?,
        profile_name: r.get(6)?,
        provider: r.get(7)?,
        model: r.get(8)?,
        effort: r.get(9)?,
        env: r.get(10)?,
        env_name: r.get(11)?,
        title: r.get(12)?,
        by: r.get(13)?,
        created_at: r.get(14)?,
        last_at: r.get(15)?,
        turn: r.get(16)?,
        project: r.get(17)?,
        cwd: r.get(18)?,
    })
}

fn no_such_chat(id: i64) -> BoardError {
    BoardError::new(ErrorCode::NotFound, format!("对话 #{id} 不存在"))
}

/// 消息：去掉首尾空白，不能为空、不能太长
fn clean_text(text: &str) -> Result<String> {
    let t = text.trim();
    if t.is_empty() {
        return Err(BoardError::usage("消息是空的"));
    }
    if t.len() > CHAT_TEXT_MAX {
        return Err(BoardError::usage(format!("消息太长：最多 {} KB", CHAT_TEXT_MAX / 1024)));
    }
    Ok(t.to_string())
}

/// 机器在线才能开对话、发消息（不排队）
fn need_online(h: &HostRow, now: i64) -> Result<()> {
    if h.online(now) {
        return Ok(());
    }
    Err(BoardError::usage(format!("机器「{}」离线：上线后才能对话", h.name)).with_detail(json!({ "host": h.name, "offline": true })))
}

impl R<'_> {
    /// 所有机器上的对话（工作区里带着，界面「对话」页签按机器筛）
    pub fn chats(&self) -> Result<Vec<Chat>> {
        let mut st = self.conn.prepare(&format!("SELECT {CHAT_COLS} FROM chats ORDER BY last_at DESC, id DESC"))?;
        let rows = st.query_map([], chat_row)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn chat(&self, id: i64) -> Result<Chat> {
        self.conn
            .query_row(&format!("SELECT {CHAT_COLS} FROM chats WHERE id = ?1"), [id], chat_row)
            .optional()?
            .ok_or_else(|| no_such_chat(id))
    }

    /// 一个对话的全部内容：输出（最后 500 行）、事件（只取 rev 大于 since 的；since = 0 取全部）
    pub fn chat_view(&self, id: i64, since: i64) -> Result<ChatView> {
        let chat = self.chat(id)?;
        let (log, fmt, events_dropped): (String, Option<String>, i64) =
            self.conn.query_row("SELECT log, fmt, ev_dropped FROM chats WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let mut st = self.conn.prepare("SELECT id, data FROM chat_events WHERE chat = ?1 AND rev > ?2 ORDER BY id")?;
        let rows = st.query_map(params![id, since], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut events = vec![];
        for row in rows {
            let (eid, data) = row?;
            let mut e: RunEvent = serde_json::from_str(&data).unwrap_or_else(|_| RunEvent::text("text", data));
            e.id = eid;
            e.call = None;
            events.push(e);
        }
        let events_from: i64 = self.conn.query_row("SELECT COALESCE(MIN(id), 0) FROM chat_events WHERE chat = ?1", [id], |r| r.get(0))?;
        let log = if log.is_empty() { vec![] } else { log.split('\n').map(str::to_string).collect() };
        Ok(ChatView { chat, log, fmt, events, events_from, events_dropped, runs: self.runs_rev()? })
    }

    /// 发给 board host 的对话：有还没交给它的消息、有停止请求，或者这一轮在跑的
    pub fn chat_tasks(&self, host_id: i64) -> Result<Vec<ChatTask>> {
        let mut st = self.conn.prepare(
            "SELECT id, agent, env, model, effort, profile, profile_name, provider, acp, prompts, cancel, turn, cwd FROM chats
             WHERE host_id = ?1 AND (turn = 'busy' OR cancel > 0 OR prompts != '[]') ORDER BY id",
        )?;
        let rows = st.query_map([host_id], |r| {
            let prompts: String = r.get(9)?;
            let turn: String = r.get(11)?;
            Ok(ChatTask {
                id: r.get(0)?,
                agent: r.get(1)?,
                env: r.get(2)?,
                model: r.get(3)?,
                effort: r.get(4)?,
                profile: r.get(5)?,
                profile_name: r.get(6)?,
                provider: r.get(7)?,
                acp: r.get(8)?,
                prompts: serde_json::from_str(&prompts).unwrap_or_default(),
                cancel: r.get(10)?,
                busy: turn == "busy",
                cwd: r.get(12)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 实时推送：事件版本 since 之后有新事件的对话
    pub fn chats_with_events_since(&self, since: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT DISTINCT chat FROM chat_events WHERE rev > ?1 ORDER BY chat")?;
        let rows = st.query_map([since], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

impl W<'_> {
    /// 在一台机器上开一个对话，第一条消息随之发出（只能在登录后的界面上开）。
    /// `profile`：按角色开（这台机器跑不了时 `AGENT_NOT_ON_HOST`，写明原因）；没有时用 `agent`（空 = 机器的默认 agent）。
    /// 机器离线时 `USAGE`，不排队
    pub fn chat_start(&mut self, host: &str, profile: Option<&str>, agent: Option<&str>, project: Option<&str>, text: &str, by: &str) -> Result<Chat> {
        let text = clean_text(text)?;
        let h = self.host_row(host)?.ok_or_else(|| no_such_host(host))?;
        need_online(&h, self.now)?;
        let (project, cwd) = match project.map(str::trim).filter(|s| !s.is_empty()) {
            Some(p) => {
                let path = self.chat_dir(&h, p)?;
                (p.to_string(), Some(path))
            }
            None => (String::new(), None),
        };
        let role = match profile.map(str::trim).filter(|s| !s.is_empty()) {
            Some(key) => Some(self.read().profile_by(key)?.ok_or_else(|| crate::profiles::no_such_profile(key))?),
            None => None,
        };
        if let Some(p) = &role {
            if let Err(why) = crate::profiles::fit(p, &h) {
                return Err(BoardError::new(ErrorCode::AgentNotOnHost, format!("机器「{host}」跑不了角色「{}」：{why}", p.name))
                    .with_detail(json!({ "host": host, "profile": p.id })));
            }
        }
        let a = self.dispatch_agent(&h, role.as_ref().map(|p| p.agent.as_str()).or(agent))?;
        // 模型、强度：角色的（角色没指定模型时用机器上这个 agent 现在的）；直接选 agent 的用机器上它现在的
        let (mut model, mut effort) = h.run_opts(&a.id);
        if let Some(p) = &role {
            if !p.model.is_empty() {
                model = Some(p.model.clone());
            }
            effort = Some(p.effort.clone()).filter(|e| !e.is_empty());
        }
        let env = h.agents.iter().find(|x| x.id == a.id).and_then(|x| h.pick(x)).map(|c| c.env.clone());
        let env_name = env.as_deref().map(|e| h.env_name(e));
        let title: String = text.chars().take(200).collect();
        let by_name = if by.trim().is_empty() { "人".to_string() } else { by.trim().to_string() };
        let (pid, pname, provider) = role.as_ref().map_or_else(Default::default, |p| (p.id.clone(), p.name.clone(), p.provider.clone()));
        self.tx.execute(
            "INSERT INTO chats (host_id, host, agent, agent_label, profile, profile_name, provider, model, effort, env, env_name, title, by_name, created_at, last_at, project, cwd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14, ?15, ?16)",
            params![h.id, h.name, a.id, a.name, pid, pname, provider, model, effort, env, env_name, title, by_name, self.now, project, cwd],
        )?;
        let id = self.tx.last_insert_rowid();
        let who = role.as_ref().map_or_else(|| a.name.clone(), |p| p.name.clone());
        let at = if project.is_empty() { String::new() } else { format!("，{project}") };
        self.host_op(h.id, &by_name, &format!("开始对话（{who}{at}）"))?;
        self.queue_prompt(id, &text)?;
        self.read().chat(id)
    }

    /// 对话在项目 `p` 的目录里跑：这台机器上这个项目的目录（board host 上报过的，没报过时按工作空间派生），`~` 不展开。
    /// 项目不存在 `BOARD_NOT_FOUND`；机器没配工作空间、也没单独指定这个项目的目录 `USAGE`；board host 报过目录不存在 `USAGE`。
    /// 不是 git 仓库的照样能用（对话常用来做不碰仓库的事）
    fn chat_dir(&mut self, h: &HostRow, p: &str) -> Result<String> {
        self.project_exists(p)?;
        let info = h.info(self.now);
        let mine = |d: &HostDir| d.board.split_once('/').map(|x| x.0) == Some(p);
        let dir = match info.dirs.iter().find(|d| mine(d)).cloned() {
            Some(d) => Some(d),
            None => {
                let first: Option<String> =
                    self.tx.query_row("SELECT key FROM boards WHERE project = ?1 ORDER BY sort, created_at, key LIMIT 1", [p], |r| r.get(0)).optional()?;
                first.and_then(|b| info.workspace.as_ref().and_then(|w| derive_dir(w, &format!("{p}/{b}"), &h.os)))
            }
        };
        let Some(dir) = dir else {
            return Err(BoardError::usage(format!("机器「{}」没配工作空间：只能在主目录里跑", h.name)).with_detail(json!({ "host": h.name, "project": p })));
        };
        if dir.status.as_deref() == Some("missing") {
            return Err(BoardError::usage(format!("目录不存在：{}", dir.path)).with_detail(json!({ "host": h.name, "project": p, "path": dir.path })));
        }
        Ok(dir.path)
    }

    /// 在对话里发一条（只能在登录后的界面上发）：这一轮还没结束时 `USAGE`；机器离线时 `USAGE`，不排队
    pub fn chat_prompt(&mut self, id: i64, text: &str) -> Result<Chat> {
        let text = clean_text(text)?;
        let c = self.read().chat(id)?;
        let h = self.host_row(&c.host)?.ok_or_else(|| no_such_host(&c.host))?;
        need_online(&h, self.now)?;
        if c.turn == "busy" {
            return Err(BoardError::usage("这一轮还没结束：等它回答完，或者先停止这一轮"));
        }
        self.queue_prompt(id, &text)?;
        self.read().chat(id)
    }

    /// 记一条人发的消息：排给 board host，这一轮开始，事件里记一条 `prompt`
    fn queue_prompt(&mut self, id: i64, text: &str) -> Result<()> {
        let (seq, raw): (i64, String) = self.tx.query_row("SELECT seq, prompts FROM chats WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut list: Vec<ChatPrompt> = serde_json::from_str(&raw).unwrap_or_default();
        list.push(ChatPrompt { seq: seq + 1, text: text.to_string() });
        self.tx.execute(
            "UPDATE chats SET seq = ?2, prompts = ?3, turn = 'busy', last_at = ?4 WHERE id = ?1",
            params![id, seq + 1, serde_json::to_string(&list).unwrap_or_default(), self.now],
        )?;
        self.events_append(EvTable::CHAT, id, None, &[RunEvent::text("prompt", text)])?;
        // 和「列目录」一样：board host 马上重新取任务
        self.touch_hosts();
        Ok(())
    }

    /// 停止这一轮（只能在登录后的界面上停）：还没交给 board host 的消息直接撤回，这一轮结束；交了的由 board host 发
    /// `session/cancel`，结束后回报 `idle`。这一轮没在跑时什么也不做
    pub fn chat_cancel(&mut self, id: i64) -> Result<Chat> {
        let c = self.read().chat(id)?;
        if c.turn != "busy" {
            return Ok(c);
        }
        let (raw, seq): (String, i64) = self.tx.query_row("SELECT prompts, seq FROM chats WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let waiting: Vec<ChatPrompt> = serde_json::from_str(&raw).unwrap_or_default();
        if !waiting.is_empty() {
            self.tx.execute("UPDATE chats SET prompts = '[]', turn = 'idle', last_at = ?2 WHERE id = ?1", params![id, self.now])?;
            self.events_append(EvTable::CHAT, id, None, &[RunEvent { kind: "result".into(), ok: Some(false), stopped: true, ..Default::default() }])?;
        } else {
            self.tx.execute("UPDATE chats SET cancel = ?2 WHERE id = ?1", params![id, seq])?;
        }
        self.touch_hosts();
        self.read().chat(id)
    }

    /// board host 的回报（只认这台机器自己的令牌，由服务端接口检查）：收下到第几条、ACP 会话、输出和事件、这一轮结束、
    /// 处理过的停止请求、agent 进程退出或起不来
    pub fn chat_report(&mut self, id: i64, host: &str, rep: &ChatReport) -> Result<Chat> {
        let c = self.read().chat(id)?;
        let h = self.host_row(host)?.ok_or_else(|| no_such_host(host))?;
        if c.host_id != h.id {
            return Err(no_such_chat(id));
        }
        let mut changed = false;
        if let Some(took) = rep.took {
            let raw: String = self.tx.query_row("SELECT prompts FROM chats WHERE id = ?1", [id], |r| r.get(0))?;
            let mut list: Vec<ChatPrompt> = serde_json::from_str(&raw).unwrap_or_default();
            let n = list.len();
            list.retain(|p| p.seq > took);
            if list.len() != n {
                self.tx.execute("UPDATE chats SET prompts = ?2 WHERE id = ?1", params![id, serde_json::to_string(&list).unwrap_or_default()])?;
            }
        }
        if let Some(acp) = rep.acp.as_deref().map(str::trim).filter(|s| !s.is_empty() && s.len() <= 200) {
            self.tx.execute("UPDATE chats SET acp = ?2 WHERE id = ?1", params![id, acp])?;
        }
        if let Some(n) = rep.canceled {
            self.tx.execute("UPDATE chats SET cancel = 0 WHERE id = ?1 AND cancel <= ?2", params![id, n])?;
        }
        if !rep.lines.is_empty() {
            let log: String = self.tx.query_row("SELECT log FROM chats WHERE id = ?1", [id], |r| r.get(0))?;
            let mut all: Vec<&str> = if log.is_empty() { vec![] } else { log.split('\n').collect() };
            all.extend(rep.lines.iter().map(|l| l.trim_end_matches(['\r', '\n'])));
            let keep = all.len().saturating_sub(LOG_KEEP);
            self.tx.execute("UPDATE chats SET log = ?2 WHERE id = ?1", params![id, all[keep..].join("\n")])?;
        }
        if rep.fmt.is_some() || !rep.events.is_empty() {
            self.events_append(EvTable::CHAT, id, rep.fmt.as_deref(), &rep.events)?;
        }
        // 这一轮的结束事件（含被停止的，`stopped`）由 board host 解析 agent 的输出得到，随 events 一起报
        if rep.idle.is_some() {
            if self.is_busy(id)? {
                self.turn_idle(id)?;
                changed = true;
            }
        }
        if let Some(why) = rep.error.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            self.events_append(EvTable::CHAT, id, None, &[RunEvent::text("error", crate::runlog::clip(why, 2000))])?;
            if self.is_busy(id)? {
                self.turn_idle(id)?;
                changed = true;
            }
        }
        if let Some(code) = rep.exit {
            if self.is_busy(id)? && self.taken(id)? {
                let text = format!("agent 进程退出了（退出码 {code}），这一轮没做完；接着发消息会重新启动它，接回这个对话");
                self.events_append(EvTable::CHAT, id, None, &[RunEvent::text("error", text)])?;
                self.turn_idle(id)?;
                changed = true;
            }
            self.tx.execute("UPDATE chats SET cancel = 0 WHERE id = ?1", [id])?;
        }
        if changed {
            self.touch_hosts();
        }
        self.read().chat(id)
    }

    fn is_busy(&self, id: i64) -> Result<bool> {
        Ok(self.read().chat(id)?.turn == "busy")
    }

    /// 发给 board host 的消息都已经交给它了（没有排着的）
    fn taken(&self, id: i64) -> Result<bool> {
        let raw: String = self.tx.query_row("SELECT prompts FROM chats WHERE id = ?1", [id], |r| r.get(0))?;
        Ok(serde_json::from_str::<Vec<ChatPrompt>>(&raw).unwrap_or_default().is_empty())
    }

    fn turn_idle(&mut self, id: i64) -> Result<()> {
        self.tx.execute("UPDATE chats SET turn = 'idle', cancel = 0, last_at = ?2 WHERE id = ?1", params![id, self.now])?;
        Ok(())
    }

    /// 这一轮在跑、消息已经交给 board host、但那里已经没有它的进程了（board host 重启、机器掉线）：
    /// 这一轮记一条出错、结束，对话留着。`why` 写给人看的原因
    pub(super) fn chats_lost(&mut self, host_id: i64, alive: Option<&[i64]>, why: &str) -> Result<()> {
        let ids: Vec<i64> = self.collect("SELECT id FROM chats WHERE host_id = ?1 AND turn = 'busy' AND prompts = '[]' ORDER BY id", [host_id], |r| r.get(0))?;
        for id in ids {
            if alive.is_some_and(|a| a.contains(&id)) {
                continue;
            }
            self.events_append(EvTable::CHAT, id, None, &[RunEvent::text("error", format!("{why}，这一轮没做完；接着发消息会重新启动 agent，接回这个对话"))])?;
            self.turn_idle(id)?;
            self.touch_hosts();
        }
        Ok(())
    }

    /// 移除机器时：它的对话一起删掉
    pub(super) fn chats_drop(&mut self, host_id: i64) -> Result<()> {
        self.tx.execute("DELETE FROM chats WHERE host_id = ?1", [host_id])?;
        Ok(())
    }
}
