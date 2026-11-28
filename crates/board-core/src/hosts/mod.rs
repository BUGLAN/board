//! 任务下发（board host）：机器与派发（任务下发需求 §三–§十）。
//!
//! - **机器**：每台干活的机器上常驻一个 `board host`。它每 10 秒上报一次（[`W::host_beat`]）：机器名、系统、
//!   board 版本、本机 agent、白板与目录，以及本机正在跑的派发；连续 5 次没收到算离线。机器名正被另一个在线的
//!   board host 使用时 `HOST_NAME_TAKEN`；同一台机器上 board host 异常退出后重启，带上上一次的标识（resume）可以接回名字。
//! - **机器设置**（最多几个会话、暂停接单、默认 agent、接单时段、每个 agent 的启动环境、skills / MCP / Git 凭据用不用服务器的）
//!   存在服务端，界面上修改，host 通过推送实时拿到。机器第一次连上时按默认值建立（启动环境、skills、MCP、Git 凭据都是「不改」）；
//!   被移除后再连上来，按新机器处理。
//! - **同步配置**（见 [`ConfigSync`]）：服务器上的 skills 库、MCP、Git 凭据、每个 agent 的启动环境、机器上的这几个选择一改，
//!   服务端算出每台机器要写成的样子（[`R::config_key`]），和它上次写好的不一样就下发「同步配置」请求；board host 写完上报
//!   skills、MCP、Git 凭据各一行，记进操作记录。board host 启动时也写一次。
//! - **启动环境**（启动环境与桌面端需求 §二）：在哪种 shell 里启动 agent 的命令（直接启动、登录 shell、cmd、
//!   PowerShell、Git Bash、WSL 的发行版 …）。board host 启动时发现本机有哪些，并在每个 agent 支持的环境里各检查
//!   一次命令找不找得到；每个 agent 用哪个由机器设置决定，没设置（或设置的环境本机已经没有）时取支持顺序里第一个
//!   找得到的（[`HostRow::pick`]）。派发开始时把用的环境写进派发记录，host 按它启动。旧版 board host 不上报启动环境，
//!   按直接启动。
//! - **派发**：一次「在某台机器上用某种 agent 做某张卡」。手动派发（界面「派发」或 `board dispatch`）和定时领取
//!   都会产生派发记录：排队（queued）→ 在跑（running）→ 结束（done，带结果）。
//! - **开始**（[`W::dispatch_begin`]、定时领取 [`W::dispatch_pull`]）在一个写事务里完成：生成 agent 名字
//!   `<机器名>/<模型名>-<推理强度>`（例如 `win/opus-5.5-high`，见 [`dispatch_agent_name`]）、check 这个名字、
//!   以它领取卡片。领不到时这次派发结束，结果「没领到」。之后 host 每次上报时替这些名字发 agent 级心跳，直到派发结束。
//! - **结束**（[`W::dispatch_end`]）：agent 退出后，卡片仍由这个名字持有就发 final、沿连线移到「错误」节点（没有时去需要人处理的状态）
//!   并评论（异常退出 / 已停止 / 超时）；否则看卡片在哪：交审状态是「已交审」，其他是「做不下去」。
//! - **兜底**：「单次最长运行」和「没输出多久算卡住」（分钟，0 = 不限）所有机器一个值，存在 `sched` 表里（`sched.rs`）。board host 到时间后按「人停止」的流程
//!   结束整个进程树，上报 `how: "timeout"` 和原因，服务端记结果「超时」、按异常退出的规则处理卡片。
//! - **占用**：派发排队时卡片沿连线进入领取时会进的锁定状态（「进行中」），没有持有者、显示排队（[`W::reserve`]）；
//!   开始时在那里原地领取，没开始就取消（或没领到）时回到原来的可领取状态最上面（[`W::unreserve`]）。
//!   别的 agent `board claim <ID>` 报 `DISPATCHED`，也不能移动它；人把它拖出那个状态时这次派发自动取消。
//! - **掉线**：机器掉线超过 50 秒，名字跟着离线，卡片按现有的心跳超时规则释放，在跑的派发结束（机器掉线），见 [`W::sweep_hosts`]。
//!
//! - **拆解**（`kind = split`，PRD §9.1）：派发「未整理」里的条目，让 agent 把它拆成几张卡、记回「未整理」（`board new --inbox --from`）。
//!   不领取、不移动这条，也不占用它（人照样能编辑、整理）；开始时只生成名字、check，host 替这个名字发心跳；
//!   结束时按这次派发期间建了几张「拆自」是它的卡记「已拆解」或「不用拆」，退出码非 0 记「异常退出」并评论，人停止、超时记「已停止」并评论。
//!   和普通派发一样占机器的一个会话位。
//!
//! 派发、取消、开始、结束都在卡片动态里追加一条，写明机器、agent、来源和结果。
//!
//! 这个目录里：`mod.rs` 是建表和迁移、上限和默认值、机器名和 agent 名字的规则；
//! `model.rs` 数据类型和机器信息；`read.rs` 库里的一行、领取规则的匹配、查询；`report.rs` 机器上报和掉线清扫；
//! `settings.rs` 机器设置、工作空间、移除和改名；`manage.rs` 远程管理、自检、自动升级；`sync.rs` 同步配置和导入 skills；
//! `dispatch.rs` 派发的建立、任意机器、改派、接回、取消；`run.rs` 排队、会话、开始、结束、日志和事件。
//! 测试在 `tests_hosts.rs`。

use crate::engine::At;
use crate::error::{BoardError, ErrorCode, Result};
use crate::model::*;
use crate::store::{flow_of, R, W};
use crate::Cmd;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

mod chat;
mod dispatch;
mod manage;
mod model;
mod read;
mod report;
mod run;
mod settings;
mod sync;

pub use chat::*;
pub use dispatch::*;
pub use manage::*;
pub use model::*;
pub(crate) use read::*;
pub use read::{HostSecret, SecretAccess};
pub use report::*;
use run::*;
pub use settings::*;
pub use sync::*;

pub(crate) const SCHEMA: &str = r#"
-- 机器（board host）。设置（max … cron）在界面上修改；被移除时整行删除，再连上来按新机器建立
CREATE TABLE IF NOT EXISTS hosts (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  name     TEXT NOT NULL UNIQUE,
  token    INTEGER,                   -- 最近一次上报用的访问令牌 ID
  instance TEXT NOT NULL,             -- 正在运行的 board host 进程的标识（每次启动随机生成）
  os       TEXT NOT NULL DEFAULT '',
  version  TEXT NOT NULL DEFAULT '',  -- board 版本
  agents   TEXT NOT NULL DEFAULT '[]',-- 本机 agent（JSON，按默认顺序；每个带它在各启动环境里的检查结果）
  envs     TEXT NOT NULL DEFAULT '[]',-- 启动环境（JSON，按发现的顺序）；旧版 board host 不上报，为空
  run      TEXT NOT NULL DEFAULT '{}',-- 每个 agent 的运行设置（JSON：{"<agent>": {"env": "<启动环境>"}}）
  workspace TEXT,                     -- 工作空间（JSON，见 HostWorkspace）：每台设备一个目录，每块白板的目录由它派生
  dirs     TEXT NOT NULL DEFAULT '[]',-- 旧的白板与目录（JSON，按 --dir 的顺序）：只用来导出、迁移，新数据不再写
  dirs_rev INTEGER NOT NULL DEFAULT 0,-- 工作空间的版本：服务端和 board host 用它分清远程修改和本机配置修改
  dirs_at  INTEGER,                   -- 最近一次改目录设置的时间
  dirs_by  TEXT,                      -- 最近一次改目录设置的人；本机 board host 上报的是「本机」
  max      INTEGER NOT NULL DEFAULT 1,-- 最多几个会话（同时在跑的 agent 进程数）
  per_session INTEGER NOT NULL DEFAULT 4,-- 定时领取启动的会话一个最多做几张（board next）
  max_run  INTEGER NOT NULL DEFAULT 180,-- 单次最长运行（分钟，0 = 不限）
  idle_run INTEGER NOT NULL DEFAULT 20,-- 多久没有输出算卡住（分钟，0 = 不限）
  rules    TEXT NOT NULL DEFAULT '[]',-- 定时领取的领取规则（JSON，有顺序：[{"label": "claude", "agent": "claude"}]）
  fallback TEXT,                      -- 兜底：不带任何规则标签的卡片由这个 agent 做；空表示不领这些卡片
  paused   INTEGER NOT NULL DEFAULT 0,-- 暂停接单
  agent    TEXT,                      -- 默认 agent；空表示本机 agent 里第一个找到的
  cron_on  INTEGER NOT NULL DEFAULT 0,-- 定时领取开关
  cron     TEXT NOT NULL DEFAULT '[]',-- 定时领取的时间点（JSON，HH:MM，机器本地时区）
  round_at INTEGER,                   -- 正在进行的一轮领取从什么时候开始；没有时为空
  rr       INTEGER NOT NULL DEFAULT 0,-- 定时领取轮到第几块白板
  seq      INTEGER NOT NULL DEFAULT 0,-- （不再使用：派发名字不再用序号计数，保留列免得旧库迁移）
  first_at INTEGER NOT NULL,
  beat_at  INTEGER NOT NULL,
  off_at   INTEGER,                   -- 离线的时间；在线时为空
  online_at INTEGER,                  -- 这次上线的时间（离线变在线、换了 board host 进程时更新，取当时和那次上报里最早的目录检查时间中较早的）：界面据此认目录结果是不是重连之后报的
  ask      TEXT,                      -- 等 board host 处理的请求（JSON：重新检查 / 重启 / 升级，见 HostAsk）；没有时为空
  ask_seq  INTEGER NOT NULL DEFAULT 0,-- 请求的序号（board host 处理完带着它回报）
  drain_at INTEGER,                   -- 排空开始的时间（暂停接单，等在跑的派发结束）；不在排空时为空
  up_pause INTEGER NOT NULL DEFAULT 0,-- 1 = 为升级暂停了接单：升级完成或失败后恢复
  agent_auto INTEGER NOT NULL DEFAULT 0,-- 1 = 内置 agent 有新版本且没有在跑的派发时自动更新
  rechecked_at INTEGER,               -- 最近一次「重新检查」做完的时间
  selftest TEXT,                      -- 自检（JSON，见 SelfTest）：等 board host 做的请求和最近一次的结果；没做过时为空
  skills   TEXT,                      -- （不再使用：以前按 agent × skill 上报的同步状态）
  mcp      TEXT,                      -- （不再使用：以前按 agent × 服务器上报的同步状态）
  providers TEXT,                     -- （不再使用：以前按 agent 上报的写入状态，现在在 config_sync 里）
  skill_import TEXT,                  -- 从这台机器导入 skills（JSON，见 SkillImport）：本机的 skill、等 board host 做的请求、最近一次的结果
  sync_skills INTEGER NOT NULL DEFAULT 0,-- skills 用服务器的（1）还是不改（0，只装 board-skills）
  sync_mcp INTEGER NOT NULL DEFAULT 0,-- MCP 用服务器的（1）还是不改（0）
  sync_git INTEGER NOT NULL DEFAULT 0,-- Git 凭据用服务器的（1）还是不改（0，新机器默认）
  config_sync TEXT,                   -- 同步配置（JSON，见 ConfigSync）：等 board host 做的请求、最近一次的结果
  res      TEXT                       -- 资源（JSON，见 HostRes）：最近一次采样和最近几次的使用率；旧版 board host 不报，为空
);

-- 添加机器时选了「上线后照 <机器> 配」（hostcopy.rs）：按生成命令时新建的访问令牌记下来源机器，
-- 用这个令牌的机器第一次上线时照它配，用过就删；吊销令牌时一起删
CREATE TABLE IF NOT EXISTS host_presets (
  token INTEGER PRIMARY KEY,         -- 访问令牌 ID
  src   TEXT NOT NULL,               -- 来源机器名
  by    TEXT NOT NULL,               -- 谁加的
  at    INTEGER NOT NULL
);

-- 机器的操作记录（远程管理：重新检查、升级、重启、排空……），机器详情「最近」里和派发一起显示；移除机器时一起删
CREATE TABLE IF NOT EXISTS host_ops (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  host_id INTEGER NOT NULL,
  at      INTEGER NOT NULL,
  by_name TEXT NOT NULL,
  text    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS host_ops_host ON host_ops (host_id, id);

-- 派发。记录一直保留（机器被移除后显示为「已移除的机器」）；白板删除时一起删除
CREATE TABLE IF NOT EXISTS dispatches (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  board       INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  card        INTEGER NOT NULL,
  title       TEXT NOT NULL,          -- 派发时的标题（卡片删除后显示它）
  host_id     INTEGER NOT NULL,
  host        TEXT NOT NULL,
  agent       TEXT NOT NULL,          -- agent 标识：claude …
  agent_label TEXT NOT NULL,          -- agent 名称：Claude Code …
  name        TEXT,                   -- 开始时生成的 agent 名字
  env         TEXT,                   -- 开始时用的启动环境标识；旧版 board host 为空（直接启动）
  env_name    TEXT,                   -- 启动环境的名称：Git Bash、WSL（Ubuntu）…
  source      TEXT NOT NULL,          -- manual 手动 | cron 定时
  by_kind     TEXT NOT NULL,
  by_name     TEXT NOT NULL,
  state       TEXT NOT NULL,          -- queued 排队 | running 在跑 | done 结束
  result      TEXT,                   -- review | stuck | crashed | stopped | timeout | canceled | lost | unclaimed
  created_at  INTEGER NOT NULL,
  started_at  INTEGER,
  ended_at    INTEGER,
  exit_code   INTEGER,
  stop_by     TEXT,                   -- 有人请求停止：谁
  session     INTEGER,                -- 同一个 agent 进程做的几张卡：第一张的派发 ID（开会话时排进去、board next 接着做的那几张都写它）；空表示就是自己
  log         TEXT NOT NULL DEFAULT '', -- 日志的最后 500 行
  model       TEXT,                   -- 开始时按机器设置传给 agent 的模型；没设（agent 自己的默认）、旧版 board host 为空
  effort      TEXT,                   -- 开始时传给 agent 的思考强度；同上
  kind        TEXT NOT NULL DEFAULT '' -- 空 = 做这张卡（领取）；split = 拆解「未整理」里的这条（不领取、不移动）
);
CREATE INDEX IF NOT EXISTS dispatches_card ON dispatches (board, card, state);
CREATE INDEX IF NOT EXISTS dispatches_host ON dispatches (host_id, state, id);
CREATE INDEX IF NOT EXISTS dispatches_host_agent_end ON dispatches (host_id, agent, state, ended_at);

-- agent 运行过程的结构化事件（会话「过程」，见 runlog.rs）：按派发存，每个派发最多 2000 条；派发删除时一起删
CREATE TABLE IF NOT EXISTS dispatch_events (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  dispatch INTEGER NOT NULL REFERENCES dispatches(id) ON DELETE CASCADE,
  kind     TEXT NOT NULL,             -- message | tool | error | result | text
  call     TEXT,                      -- 工具调用的标识：结果后到时合进这一条
  rev      INTEGER NOT NULL,          -- 写入（或合进结果）时 meta.runs 的值：界面按它增量取
  data     TEXT NOT NULL              -- 事件本身（JSON，不含 id、call）
);
CREATE INDEX IF NOT EXISTS dispatch_events_dispatch ON dispatch_events (dispatch, id);
CREATE INDEX IF NOT EXISTS dispatch_events_call ON dispatch_events (dispatch, call);
CREATE INDEX IF NOT EXISTS dispatch_events_rev ON dispatch_events (rev);
"#;

/// 旧数据库补上启动环境的几列
/// 已经移除的内置 agent（pi，见 [`crate::agentdef::REMOVED_IDS`]）留在库里的东西，升级后第一次启动时清一次（#484）：
/// - 每台机器存着的上报（hosts.agents）里没有 custom 标记的那一项（旧数据都没有标记，按内置算）；自定义的同名 agent 留着
/// - 运行设置（hosts.run）里的那一项、默认 agent 设成它的（回到「本机第一个找到的」）——这台机器上有自定义的同名 agent 时不动
/// - 项目里名字就是它、「仅人可打」、没有任何卡片用着的标签（以前给它建的）；别的标签不动
/// 做过一次记在 meta 的 removed_agents 里，之后人自己再建同名的标签不会被删
fn drop_removed_agents(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let done: i64 = conn.query_row("SELECT COALESCE((SELECT value FROM meta WHERE key = 'removed_agents'), 0)", [], |r| r.get(0))?;
    if done >= 1 {
        return Ok(());
    }
    let rows: Vec<(i64, String, String, Option<String>)> = conn
        .prepare("SELECT id, agents, run, agent FROM hosts")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, agents_raw, run_raw, agent) in rows {
        let Ok(mut agents) = serde_json::from_str::<Vec<serde_json::Value>>(&agents_raw) else { continue };
        let removed = |a: &serde_json::Value| {
            crate::agentdef::is_removed(a["id"].as_str().unwrap_or_default(), a["custom"].as_bool().unwrap_or(false))
        };
        let n = agents.len();
        agents.retain(|a| !removed(a));
        if agents.len() != n {
            conn.execute("UPDATE hosts SET agents = ?2 WHERE id = ?1", params![id, serde_json::to_string(&agents).unwrap_or_default()])?;
        }
        let kept = |k: &str| agents.iter().any(|a| a["id"].as_str() == Some(k));
        let mut run: BTreeMap<String, serde_json::Value> = serde_json::from_str(&run_raw).unwrap_or_default();
        let before = run.len();
        run.retain(|k, _| !crate::agentdef::REMOVED_IDS.contains(&k.as_str()) || kept(k));
        if run.len() != before {
            conn.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![id, serde_json::to_string(&run).unwrap_or_default()])?;
        }
        if agent.as_deref().is_some_and(|a| crate::agentdef::REMOVED_IDS.contains(&a) && !kept(a)) {
            conn.execute("UPDATE hosts SET agent = NULL WHERE id = ?1", [id])?;
        }
    }
    for name in crate::agentdef::REMOVED_IDS {
        conn.execute(
            "DELETE FROM labels WHERE name = ?1 AND human_only = 1 AND NOT EXISTS (SELECT 1 FROM card_labels cl WHERE cl.label = labels.id)",
            [name],
        )?;
    }
    conn.execute("INSERT INTO meta (key, value) VALUES ('removed_agents', 1) ON CONFLICT(key) DO UPDATE SET value = 1", [])?;
    Ok(())
}

pub(crate) fn migrate(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let has = |table: &str, col: &str| -> rusqlite::Result<bool> {
        conn.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"), [col], |r| r.get::<_, i64>(0))
            .map(|n| n > 0)
    };
    for (table, col, def) in [
        ("hosts", "envs", "TEXT NOT NULL DEFAULT '[]'"),
        ("hosts", "run", "TEXT NOT NULL DEFAULT '{}'"),
        ("dispatches", "env", "TEXT"),
        ("dispatches", "env_name", "TEXT"),
        ("hosts", "per_session", "INTEGER NOT NULL DEFAULT 4"),
        ("hosts", "rules", "TEXT NOT NULL DEFAULT '[]'"),
        ("hosts", "fallback", "TEXT"),
        ("hosts", "workspace", "TEXT"),
        ("hosts", "dirs_rev", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "dirs_at", "INTEGER"),
        ("hosts", "dirs_by", "TEXT"),
        ("dispatches", "session", "INTEGER"),
        ("hosts", "ask", "TEXT"),
        ("hosts", "ask_seq", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "agent_auto", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "drain_at", "INTEGER"),
        ("hosts", "up_pause", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "rechecked_at", "INTEGER"),
        ("hosts", "online_at", "INTEGER"),
        ("dispatches", "model", "TEXT"),
        ("dispatches", "any_host", "INTEGER NOT NULL DEFAULT 0"),
        ("dispatches", "any_agent", "INTEGER NOT NULL DEFAULT 0"),
        ("dispatches", "effort", "TEXT"),
        // 角色（profiles.rs）：派发时按角色定 agent、供应商、模型、强度；记下标识和当时的名字、供应商、附加说明
        ("dispatches", "profile", "TEXT NOT NULL DEFAULT ''"),
        ("dispatches", "profile_name", "TEXT NOT NULL DEFAULT ''"),
        ("dispatches", "provider", "TEXT NOT NULL DEFAULT ''"),
        ("dispatches", "note", "TEXT NOT NULL DEFAULT ''"),
        ("hosts", "max_run", "INTEGER NOT NULL DEFAULT 180"),
        ("hosts", "idle_run", "INTEGER NOT NULL DEFAULT 20"),
        ("hosts", "selftest", "TEXT"),
        ("hosts", "skills", "TEXT"),
        ("hosts", "mcp", "TEXT"),
        ("hosts", "providers", "TEXT"),
        ("hosts", "skill_import", "TEXT"),
        // 结构化事件（runlog.rs）：agent 的输出格式（空 = 只有原始输出，没有 = 旧版 board host 没报）、删掉了几条、结束 30 天后清过没有
        ("dispatches", "fmt", "TEXT"),
        ("dispatches", "ev_dropped", "INTEGER NOT NULL DEFAULT 0"),
        ("dispatches", "ev_old", "INTEGER NOT NULL DEFAULT 0"),
        ("dispatches", "kind", "TEXT NOT NULL DEFAULT ''"),
        // 开始这次派发时那块白板在这台机器上的目录（工作空间里的 `<工作空间>/<项目>`）
        ("dispatches", "path", "TEXT"),
        ("hosts", "sync_mcp", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "config_sync", "TEXT"),
        ("hosts", "res", "TEXT"),
    ] {
        if !has(table, col)? {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"))?;
        }
    }
    // Git 凭据（gitcred.rs）：新机器默认不同步——打开后写用户级 git 配置（`git config --global`）里该主机的
    // credential helper，凭据带提交身份时还写 user.name / user.email；要人去界面打开。
    // 第一次补上这一列时，已有的机器保持当时的效果（#386 起默认用服务器的），和 skills、MCP 的迁移一样；
    // 之后新建的机器在 INSERT 里显式写 0（0.1.34 的库里这一列是 ALTER 补的，表上的默认值还是 1）
    if !has("hosts", "sync_git")? {
        conn.execute_batch("ALTER TABLE hosts ADD COLUMN sync_git INTEGER NOT NULL DEFAULT 0")?;
        conn.execute_batch("UPDATE hosts SET sync_git = 1")?;
    }
    // 配置同步改成「服务器存一份、机器上选用不用」（机器详情与配置同步重构）：已有的机器保持原来的效果——
    // skills、MCP 以前都同步，记成「用服务器的」。只在第一次补上 sync_skills 这一列时做；之后新建的机器默认全部「不改」。
    // （以前这里还把每个 agent 的供应商记成「不改 / 指定」：#576 去掉了机器上的模型设置，见下面）
    if !has("hosts", "sync_skills")? {
        conn.execute_batch("ALTER TABLE hosts ADD COLUMN sync_skills INTEGER NOT NULL DEFAULT 0")?;
        conn.execute_batch("UPDATE hosts SET sync_skills = 1, sync_mcp = 1")?;
    }
    // #576：机器上不再给 agent 指定供应商、模型（用哪个模型按角色定，派发时带给这一个进程）。存着的「不改 / 指定」那几项
    // （mode、provider、model、effort、protocol）清掉，只留启动环境和恢复时间；board host 写进 agent 配置文件的不恢复
    let rows: Vec<(i64, String)> = conn.prepare("SELECT id, run FROM hosts")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (id, raw) in rows {
        let Ok(mut run) = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&raw) else { continue };
        let mut touched = false;
        for r in run.values_mut() {
            if let Some(o) = r.as_object_mut() {
                for k in ["mode", "provider", "model", "effort", "protocol"] {
                    touched |= o.remove(k).is_some();
                }
            }
        }
        if touched {
            conn.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![id, serde_json::to_string(&run).unwrap_or_default()])?;
        }
    }
    conn.execute_batch("DROP TABLE IF EXISTS provider_agents")?;
    // agent 模板删掉了（本机配置文件里的 [agents.<标识>] 还在）
    conn.execute_batch("DROP TABLE IF EXISTS agent_templates")?;
    // 会话里排着的派发按会话找（session 列是后加的，索引放在补列之后）
    conn.execute_batch("CREATE INDEX IF NOT EXISTS dispatches_session ON dispatches (session, id)")?;
    // 领取规则原来是「单个标签 → agent」：换成等价的 in [标签]
    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT id, rules FROM hosts")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, raw) in rows {
        let list: Vec<ClaimRule> = serde_json::from_str(&raw).unwrap_or_default();
        if list.iter().any(|r| r.label.is_some()) {
            let new: Vec<serde_json::Value> = list.into_iter().map(|r| r.upgraded().stored()).collect();
            conn.execute("UPDATE hosts SET rules = ?2 WHERE id = ?1", params![id, serde_json::to_string(&new).unwrap_or_default()])?;
        }
    }
    drop_removed_agents(conn)?;
    // 定时领取原来是 HH:MM 时间点：换成等价的 cron 表达式（「M H * * *」），不丢已有配置
    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT id, cron FROM hosts")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, raw) in rows {
        let list: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
        if list.iter().any(|t| parse_hhmm(t).is_some()) {
            let new: Vec<String> = list.iter().map(|t| crate::cron::normalize(t).unwrap_or_else(|_| t.clone())).collect();
            conn.execute("UPDATE hosts SET cron = ?2 WHERE id = ?1", params![id, serde_json::to_string(&new).unwrap_or_default()])?;
        }
    }
    // 机器对话（chat.rs）
    conn.execute_batch(chat::SCHEMA)?;
    // 对话的运行目录（#624）：旧对话两列为空 = 主目录
    for (col, def) in [("project", "TEXT NOT NULL DEFAULT ''"), ("cwd", "TEXT")] {
        if !has("chats", col)? {
            conn.execute_batch(&format!("ALTER TABLE chats ADD COLUMN {col} {def}"))?;
        }
    }
    Ok(())
}

/// 机器名最长多少个字符
pub const HOST_NAME_MAX: usize = 24;
/// 「最多几个会话」（同时在跑的 agent 进程数）的上限
pub const HOST_MAX_LIMIT: i64 = 8;
/// 一个会话最多做几张的上限（1–8，默认 4）
pub const PER_SESSION_LIMIT: i64 = 8;
/// 单次最长运行的默认值（分钟，0 = 不限）
pub const MAX_RUN_DEFAULT: i64 = 180;
/// 没输出多久算卡住的默认值（分钟，0 = 不限）
pub const IDLE_RUN_DEFAULT: i64 = 20;
/// 单次最长运行最多几分钟（12 小时）；0 表示不限。测试里可以设得很小。
/// 界面上能选的档位（`web/src/components/hosts/HostModal.tsx` 的 `MAX_RUN_CHOICES`）只在这张范围里，
/// 最高一档就是这个上限；服务端只校验范围，选不选得到哪几档由界面决定
pub const MAX_RUN_LIMIT: i64 = 720;
/// 没输出多久算卡住最多几分钟（2 小时）；0 表示不限。界面的 `IDLE_RUN_CHOICES` 同上
pub const IDLE_RUN_LIMIT: i64 = 120;
/// 领取规则最多几条
pub const RULES_LIMIT: usize = 20;
/// 发给旧版 board host 的「模型不改」（`HostAgent::run_mode`）：机器上不再指定模型（#576），总是它，
/// 旧版 board host 收到后不再按机器上的设置写 agent 的配置文件
pub const MODE_OFF: &str = "off";
/// 每个派发保留的日志行数
pub const LOG_KEEP: usize = 500;
/// 机器多久没上报算离线：与 agent 相同，连续 5 次（50 秒）
pub const HOST_GAP_MS: i64 = BEAT_MS * BEAT_MISSES;

/// Store::sweep 的预检查：要标为离线的机器、名字已经离线的在跑派发（?1 = 现在，?2 = 允许的间隔）
pub(crate) const DUE_COUNT: &str = "(SELECT COUNT(*) FROM hosts WHERE off_at IS NULL AND beat_at + ?2 <= ?1) + \
    (SELECT COUNT(*) FROM dispatches d LEFT JOIN agents a ON a.name = d.name \
     WHERE d.state = 'running' AND (a.name IS NULL OR a.off_at IS NOT NULL OR a.beat_at + ?2 <= ?1))";

/// 机器名：小写字母、数字、`.`、`-`，以字母或数字开头，最长 24 个字符
pub fn check_host_name(name: &str) -> std::result::Result<(), String> {
    if matches!(name, "mine" | "pause-all" | "stop-all") {
        return Err(format!("机器名「{name}」是保留名（接口路径在用），换一个"));
    }
    let ok = !name.is_empty()
        && name.len() <= HOST_NAME_MAX
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!("机器名「{name}」不对：只能用小写字母、数字、. 和 -，以字母或数字开头，最长 {HOST_NAME_MAX} 个字符"))
    }
}

/// 本机主机名（Windows 上是 `COMPUTERNAME`，其他系统用 `hostname`）：`board check --agent` 在本机模式下认机器时用
pub(crate) fn machine_hostname() -> Result<String> {
    if cfg!(windows) {
        if let Ok(n) = std::env::var("COMPUTERNAME") {
            if !n.trim().is_empty() {
                return Ok(n);
            }
        }
    }
    let out = std::process::Command::new("hostname")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| BoardError::usage(format!("取不到本机主机名：{e}")))?;
    let n = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if n.is_empty() {
        return Err(BoardError::usage("取不到本机主机名"));
    }
    Ok(n)
}

/// 本机主机名转成机器名：转成小写，非法字符换成 `-`，去掉开头不是字母或数字的部分，最长 24 个字符
pub fn host_name_from(raw: &str) -> String {
    let s: String = raw
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' { c } else { '-' })
        .collect();
    let s = s.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
    let s: String = s.chars().take(HOST_NAME_MAX).collect();
    let s = s.trim_end_matches(['-', '.']).to_string();
    if s.is_empty() { "host".into() } else { s }
}

/// 一次派发的 agent 名字：`<机器名>/<模型名>-<推理强度>`，例如 `win/opus-5.5-high`；
/// 模型有思考强度时接 `-<强度>`，没有时省略；同一台机器、同一模型、同一强度有多个并发派发时末尾加 `-<序号>`，
/// 例如 `win/opus-5.5-high-1`、`win/opus-5.5-high-2`（seq 0 表示不带）。模型名来自 board host 上报的 agent 数据
/// （配置文件 `[agents.<标识>] model`，内置模板有默认值）；没有时用 agent 标识。转成名字能用的样子
/// （小写、空白和非法字符换成 `-`、去掉开头结尾的 `-`），放不下时截断，保证总长不超过 48 个字符，
/// 能通过 `board check` 的名字格式校验（[`crate::check_agent_name`]）
pub fn dispatch_agent_name(agent: &str, host: &str, model: &str, effort: &str, seq: u32) -> String {
    let first = name_segment(host, HOST_NAME_MAX);
    let first = if first.is_empty() { name_segment(agent, HOST_NAME_MAX) } else { first };
    let suffix = if seq > 0 { format!("-{seq}") } else { String::new() };
    // 序号不截断：模型名先让出序号的位置
    let room = AGENT_NAME_MAX.saturating_sub(first.chars().count() + 1 + suffix.len()).max(1);
    let model = model.trim();
    let effort = effort.trim();
    let raw = if model.is_empty() {
        agent.to_string()
    } else if effort.is_empty() {
        model.to_string()
    } else {
        format!("{model}-{effort}")
    };
    let mut seg = name_segment(&raw, room);
    if seg.is_empty() {
        seg = name_segment(agent, room);
    }
    format!("{first}/{seg}{suffix}")
}

/// 没有机器（没跑 board host、白板不在这台机器）时的 agent 名字：`<agent 标识>/<模型名>`，例如 `codex/codex`、`dsh/deepseek-v4.1-flash`。
/// 规则同 [`dispatch_agent_name`] 的第二段（转成名字能用的样子，放不下时截断），`modeled` 说明第二段是不是真的模型名
pub fn plain_agent_name(agent: &str, model: &str) -> (String, bool) {
    let room = AGENT_NAME_MAX.saturating_sub(agent.chars().count() + 1).max(1);
    let mut seg = name_segment(model, room);
    let modeled = !seg.is_empty();
    if seg.is_empty() {
        seg = name_segment(agent, room);
    }
    (format!("{agent}/{seg}"), modeled)
}

/// 名字的一段（机器名、模型名）：小写，空白和非法字符换成 `-`，去掉开头结尾的 `-`，最长 max 个字符
fn name_segment(raw: &str, max: usize) -> String {
    let s: String = raw
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' { c } else { '-' })
        .collect();
    let s = s.trim_matches(['-', '.']);
    s.chars().take(max).collect::<String>().trim_matches(['-', '.']).to_string()
}

/// agent 标识：小写字母、数字、`.`、`-`，以字母开头，最长 16 个字符（名字 `<机器名>/<模型名>-<强度>` 不超过 48 个字符）
pub fn check_agent_id(id: &str) -> std::result::Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 16
        && id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!("agent 标识「{id}」不对：只能用小写字母、数字、. 和 -，以字母开头，最长 16 个字符"))
    }
}

/// 定时领取的时间点：`H:MM` 或 `HH:MM`，统一成 `HH:MM`
pub fn parse_hhmm(raw: &str) -> Option<String> {
    let (h, m) = raw.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && raw.trim().len() <= 5).then(|| format!("{h:02}:{m:02}"))
}
