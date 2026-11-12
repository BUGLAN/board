//! skills 共用库的接口（PRD §10.12「共用配置」、§9「board skills」）：
//!
//! - `GET /api/skills`：库里的 skill（不带内容），第一项是随服务端版本的 board-skills
//! - `GET /api/skills/{名称}`：连同内容（board host、`board skills sync` 下载用）
//! - `POST /api/skills {name, files, source?}`：放进库里，同名的换成新版本（界面、`board skills push`）
//! - `DELETE /api/skills/{名称}`：删掉；board-skills 不能删、不能改
//!
//! 访问令牌都能调用（`board skills` 命令）。board-skills 的内容在编译时打包进来（build.rs，跳过 bin/）。

use crate::auth::Caller;
use crate::{client_of, parse, ApiError, ApiResult, AppState};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::{Extension, Json};
use board_core::skills::{self, SkillFile, SkillInfo, SkillPack, BUILTIN};
use board_core::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/board_skills.rs"));

/// 随服务端版本的 board-skills（没有打包进来时为空）
pub fn builtin() -> Option<&'static SkillPack> {
    static PACK: OnceLock<Option<SkillPack>> = OnceLock::new();
    PACK.get_or_init(|| {
        if !BOARD_SKILLS.iter().any(|(p, _, _)| *p == "SKILL.md") {
            return None;
        }
        let files: Vec<(String, bool, Vec<u8>)> = BOARD_SKILLS.iter().map(|(p, x, d)| (p.to_string(), *x, d.to_vec())).collect();
        let version = skills::pack_version(&files);
        let size: usize = files.iter().map(|f| f.2.len()).sum();
        Some(SkillPack {
            info: SkillInfo {
                name: BUILTIN.into(),
                version,
                size: size as i64,
                files: files.len() as i64,
                updated_at: 0,
                by: "服务端".into(),
                source: format!("随服务端 {}", crate::VERSION),
                builtin: true,
            },
            files: files.iter().map(|(p, x, d)| SkillFile { path: p.clone(), exec: *x, data: skills::b64(d) }).collect(),
        })
    })
    .as_ref()
}

/// 库的索引：board-skills 在前
pub fn index(mut list: Vec<SkillInfo>) -> Vec<SkillInfo> {
    if let Some(b) = builtin() {
        list.insert(0, b.info.clone());
    }
    list
}

pub(crate) async fn list(State(s): State<AppState>) -> ApiResult<Vec<SkillInfo>> {
    Ok(Json(index(s.db(|st| st.read(|r| r.skills())).await?)))
}

pub(crate) async fn get(State(s): State<AppState>, Path(name): Path<String>) -> ApiResult<SkillPack> {
    if name == BUILTIN {
        return builtin().cloned().map(Json).ok_or_else(|| ApiError(BoardError::new(ErrorCode::NotFound, "这个服务端没有打包 board-skills")));
    }
    let n = name.clone();
    s.db(move |st| st.read(|r| r.skill(&n))).await?.map(Json).ok_or_else(|| ApiError(BoardError::new(ErrorCode::NotFound, format!("库里没有 skill「{name}」"))))
}

#[derive(Deserialize)]
pub(crate) struct PutBody {
    name: String,
    #[serde(default)]
    files: Vec<SkillFile>,
    /// 从哪里上传：机器名；界面上传时不给
    #[serde(default)]
    source: Option<String>,
}

fn who(c: &Caller, conn_name: Option<String>) -> String {
    match c {
        Caller::Human { login, .. } => login.clone().unwrap_or_else(|| "人".into()),
        Caller::Agent { .. } => conn_name.unwrap_or_else(|| "令牌".into()),
    }
}

pub(crate) async fn put(State(s): State<AppState>, Extension(caller): Extension<Caller>, h: HeaderMap, Json(v): Json<Value>) -> ApiResult<SkillInfo> {
    let b: PutBody = parse(v)?;
    let token = match &caller {
        Caller::Agent { token } => Some(*token),
        _ => None,
    };
    let source = match (&caller, b.source.as_deref().map(str::trim).filter(|x| !x.is_empty())) {
        (Caller::Agent { .. }, Some(src)) => src.chars().take(64).collect(),
        (Caller::Agent { .. }, None) => "命令行".to_string(),
        (Caller::Human { .. }, _) => "界面".to_string(),
    };
    let client = client_of(&h);
    let out = s
        .db(move |st| {
            let name = match token {
                Some(t) => crate::auth::token_name(st.conn(), t)?,
                None => None,
            };
            let by = who(&caller, name);
            let origin = if client.is_some() { Origin::ui(client) } else { Origin::cli() };
            st.write(now_ms(), origin, |w| w.skill_put(&b.name, &b.files, &by, &source))
        })
        .await?;
    s.poke();
    Ok(Json(out))
}

pub(crate) async fn remove(State(s): State<AppState>, h: HeaderMap, Path(name): Path<String>) -> ApiResult<Value> {
    let client = client_of(&h);
    s.db(move |st| {
        let origin = if client.is_some() { Origin::ui(client) } else { Origin::cli() };
        st.write(now_ms(), origin, |w| w.skill_rm(&name))
    })
    .await?;
    s.poke();
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_skills_is_packed_without_bin() {
        let b = builtin().expect("board-skills 打包进来了");
        let paths: Vec<&str> = b.files.iter().map(|f| f.path.as_str()).collect();
        for p in ["SKILL.md", "work.md", "plan.md", "host.md", "reference.md", "scripts/board", "scripts/board.cmd"] {
            assert!(paths.contains(&p), "缺 {p}：{paths:?}");
        }
        assert!(!paths.iter().any(|p| p.starts_with("bin/")), "bin/ 由各台 board host 填：{paths:?}");
        assert!(b.files.iter().find(|f| f.path == "scripts/board").unwrap().exec, "scripts/board 可执行");
        assert!(b.info.builtin && b.info.name == BUILTIN && b.info.version.len() == 12);
        assert_eq!(index(vec![])[0].name, BUILTIN);
    }
}
