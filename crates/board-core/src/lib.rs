//! Board 核心：数据模型、SQLite 存储与规则引擎。
//!
//! 网页端（board-server）、桌面端（Tauri）与 CLI（board）都通过这个 crate 读写同一个数据库。

pub mod agents;
#[cfg(feature = "config")]
pub mod config;
pub mod cron;
pub mod engine;
pub mod error;
pub mod files;
pub mod flow;
pub mod hosts;
pub mod model;
pub mod moves;
pub mod search;
pub mod seed;
pub mod stats;
pub mod store;

pub use agents::{BeatBy, NameOwner, NAME_GAP_MS};
pub use engine::{purge_trash, Cmd, TRASH_KEEP_MS};
pub use error::{BoardError, ErrorCode, Result};
pub use files::Attachment;
pub use flow::StatusPatch;
pub use search::{SearchHit, SEARCH_LIMIT};
pub use hosts::{
    AgentRun, ClaimRule, Dispatch, EnvCheck, EnvPatch, HostAgent, HostClone, HostDir, HostEnv, HostInfo, HostPatch, HostReport, HostWork,
};
pub use model::*;
pub use store::{hhmm, now_ms, valid_key, NewBoard, Store, R, W};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_hosts;
#[cfg(test)]
mod tests_stats;
#[cfg(test)]
mod tests_search;
