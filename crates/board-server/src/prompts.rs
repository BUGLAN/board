//! 快捷提示词的接口（#572，board-core 的 `prompts`）：整个服务器一份列表，网页、桌面端、手机共用。
//!
//! - `GET /api/prompts`：`{prompts: [{id, title, text}]}`，按列表顺序
//! - `PUT /api/prompts {prompts}`：整列保存，返回整理后的整列；不合规的整列拒绝（`USAGE`）
//!
//! 和其他界面设置（标签、布局）一样只给登录后的界面：访问令牌两个都用不了（`auth::guard` 不放行，`NOT_LOGGED_IN`）。
//! 保存不推送 rev：别的窗口在打开新建卡片时重新取

use crate::{client_of, parse, ApiResult, AppState};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use board_core::prompts::Prompt;
use board_core::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PromptList {
    prompts: Vec<Prompt>,
}

pub(crate) async fn get(State(s): State<AppState>) -> ApiResult<PromptList> {
    let prompts = s.db(|st| st.read(|r| r.prompts())).await?;
    Ok(Json(PromptList { prompts }))
}

pub(crate) async fn put(State(s): State<AppState>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<PromptList> {
    let body: PromptList = parse(v)?;
    let client = client_of(&h);
    let prompts = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.prompts_put(body.prompts))).await?;
    Ok(Json(PromptList { prompts }))
}
