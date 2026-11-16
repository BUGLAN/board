//! Board 核心：数据模型、SQLite 存储与规则引擎。
//!
//! 网页端（board-server）、桌面端（Tauri）与 CLI（board）都通过这个 crate 读写同一个数据库。

pub mod agents;
pub mod card_crons;
#[cfg(feature = "config")]
pub mod config;
pub mod cron;
pub mod engine;
pub mod error;
pub mod files;
pub mod fit;
pub mod gitcred;
pub mod flow;
pub mod hostres;
pub mod hosts;
pub mod model;
pub mod releases;
pub mod moves;
pub mod providers;
pub mod runlog;
pub mod sched;
pub mod search;
pub mod seed;
pub mod service;
pub mod mcp;
pub mod skills;
pub mod stats;
pub mod store;

pub use agents::{BeatBy, NameOwner, NAME_GAP_MS};
pub use engine::{purge_trash, Cmd, TRASH_KEEP_MS};
pub use error::{BoardError, ErrorCode, Result};
pub use files::Attachment;
pub use flow::StatusPatch;
pub use search::{SearchHit, SEARCH_LIMIT};
pub use hostres::HostRes;
pub use hosts::{
    AgentManage, AgentOpts, AgentRun, ClaimRule, Dispatch, EnvCheck, EnvPatch, HostAgent, HostClone, HostDir, HostDirOverride, HostEnv, HostInfo, HostPatch, HostReport,
    HostWork, HostWorkspace, ConfigSync, SyncAsk, SyncResult, SkillsSync, McpSync, GitSync, LsAsk, DirEntry, DirList,
    RunPatch, SelfTest, SelfTestAsk, SelfTestItem, SessionPart, SessionView, SkillImport, SkillImportAsk, SkillImportFail, SkillImportPick, SkillImportResult, SKILL_IMPORT_STALE_MS,
};
pub use model::*;
pub use runlog::RunEvent;
pub use store::{hhmm, now_ms, valid_board, valid_dir_key, valid_key, NewBoard, Store, R, W};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_hosts;
#[cfg(test)]
mod tests_stats;
#[cfg(test)]
mod tests_search;
#[cfg(test)]
mod tests_releases;
