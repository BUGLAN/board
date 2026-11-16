//! Git 凭据共用配置的接口（PRD §10.16「共用配置」，board-core 的 `gitcred`）：
//!
//! - `GET /api/git`：`{cred}`，令牌只给末尾 4 位；没设时 `cred` 是 null（界面、访问令牌都能读）
//! - `POST /api/git {host, username, token}`：设置或修改（只能在登录后的界面上做）；令牌留空保留原来的值
//! - `DELETE /api/git`：删掉（只能在界面上做）；用服务器的机器下次同步时去掉之前写进用户级 git 配置的
//! - `GET /api/hosts/{名字}/git`：`{cred}`，含令牌原文，只给这台机器用的访问令牌；界面、其他机器的令牌都是 `NOT_LOGGED_IN`

use crate::auth::{human, Caller};
use crate::{client_of, parse, ApiError, ApiResult, AppState};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::{Extension, Json};
use board_core::gitcred::GitCred;
use board_core::*;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Serialize)]
pub(crate) struct GitView {
    cred: Option<GitCred>,
}

pub(crate) async fn get(State(s): State<AppState>) -> ApiResult<GitView> {
    let cred = s.db(|st| st.read(|r| r.git_cred())).await?;
    Ok(Json(GitView { cred: cred.as_ref().map(GitCred::masked) }))
}

pub(crate) async fn put(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<GitCred> {
    let by = human(&caller).map_err(ApiError)?;
    let cred: GitCred = parse(v)?;
    let client = client_of(&h);
    let out = s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.git_cred_put(cred, &by, "界面"))).await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn remove(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap) -> ApiResult<Value> {
    let by = human(&caller).map_err(ApiError)?;
    let client = client_of(&h);
    s.db(move |st| st.write(now_ms(), Origin::ui(client), |w| w.git_cred_rm(&by))).await?;
    s.poke();
    Ok(Json(json!({ "ok": true })))
}

/// board host 取完整内容（含令牌原文）：只给这台机器用的访问令牌，别的令牌（界面登录、其他机器、没绑机器的新令牌）
/// 和登录的界面都拿不到原文，用打码的 `GET /api/git`
pub(crate) async fn for_host(State(s): State<AppState>, Extension(caller): Extension<Caller>, Path(name): Path<String>) -> ApiResult<GitView> {
    let Caller::Agent { token } = caller else {
        return Err(ApiError(BoardError::new(ErrorCode::NotLoggedIn, "Git 令牌原文只发给 board host：界面上用 GET /api/git")));
    };
    let name = name.trim().to_string();
    let now = now_ms();
    let own = s.db(move |st| st.read(|r| r.host_uses_token(&name, token, now))).await?;
    if !own {
        return Err(ApiError(BoardError::new(
            ErrorCode::NotLoggedIn,
            "Git 令牌原文只发给这台机器用的令牌：机器不在这台机器上，或它的令牌不是你用的这个；界面上用 GET /api/git（令牌打码）",
        )));
    }
    let cred = s.db(|st| st.read(|r| r.git_cred())).await?;
    Ok(Json(GitView { cred }))
}
