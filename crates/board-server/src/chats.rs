//! 机器对话（board-core `hosts/chat.rs`，PRD §9.1「机器对话」）的接口。
//!
//! - 界面（只能登录后用，令牌调用时守卫回 `NOT_LOGGED_IN`）：`POST /api/hosts/{名字}/chats` 开一个对话（第一条消息随之发出），
//!   `GET /api/chats/{ID}?since=` 取内容，`POST /api/chats/{ID}/prompt` 发一条，`POST /api/chats/{ID}/cancel` 停止这一轮
//! - board host（用这台机器的令牌）：`POST /api/hosts/{名字}/chats/{ID}` 回报（收下到第几条、ACP 会话、输出和事件、这一轮结束……）

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::{Extension, Json};
use board_core::hosts::{Chat, ChatReport, ChatView};
use board_core::*;
use serde::Deserialize;
use serde_json::Value;

use crate::auth::{human, Caller};
use crate::{client_of, parse, ApiError, AppState};

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

#[derive(Deserialize)]
pub(crate) struct StartBody {
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    text: String,
}

#[derive(Deserialize)]
pub(crate) struct PromptBody {
    text: String,
}

#[derive(Deserialize, Default)]
pub(crate) struct ViewQuery {
    #[serde(default)]
    since: i64,
}

/// 一次回报最多带多少条事件、多少行输出（多的丢最早的）
const REPORT_EVENTS: usize = 500;
const REPORT_LINES: usize = 2000;

pub(crate) async fn start(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Chat> {
    let by = human(&caller)?;
    let b: StartBody = parse(v)?;
    let client = client_of(&h);
    let out = s
        .db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.chat_start(&name, b.profile.as_deref(), b.agent.as_deref(), &b.text, &by)))
        .await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn view(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(id): Path<i64>, Query(q): Query<ViewQuery>) -> ApiResult<ChatView> {
    human(&caller)?;
    Ok(Json(s.db(move |st| st.read(|r| r.chat_view(id, q.since.max(0)))).await?))
}

pub(crate) async fn prompt(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(v): Json<Value>,
) -> ApiResult<Chat> {
    human(&caller)?;
    let b: PromptBody = parse(v)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.chat_prompt(id, &b.text))).await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn cancel(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Path(id): Path<i64>) -> ApiResult<Chat> {
    human(&caller)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.chat_cancel(id))).await?;
    s.poke();
    Ok(Json(out))
}

/// board host 的回报：只认这台机器绑定的令牌（人、本机模式也可以，测试用）
pub(crate) async fn report(
    State(s): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((name, id)): Path<(String, i64)>,
    Json(v): Json<Value>,
) -> ApiResult<Chat> {
    let mut rep: ChatReport = parse(v)?;
    if rep.events.len() > REPORT_EVENTS {
        rep.events.drain(..rep.events.len() - REPORT_EVENTS);
    }
    if rep.lines.len() > REPORT_LINES {
        rep.lines.drain(..rep.lines.len() - REPORT_LINES);
    }
    let token = match &caller {
        Caller::Agent { token } => Some(*token),
        Caller::Human { .. } => None,
    };
    let out = s
        .db(move |st| {
            st.write(now_ms(), Origin::cli(), |w| {
                w.read().host_owned_by(&name, token)?;
                w.chat_report(id, &name, &rep)
            })
        })
        .await?;
    s.poke();
    Ok(Json(out))
}
