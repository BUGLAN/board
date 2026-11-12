//! 任务下发（board host）：机器与派发（任务下发需求 §三–§十）。
//!
//! - **机器**：每台干活的机器上常驻一个 `board host`。它每 10 秒上报一次（[`W::host_beat`]）：机器名、系统、
//!   board 版本、本机 agent、白板与目录，以及本机正在跑的派发；连续 5 次没收到算离线。机器名正被另一个在线的
//!   board host 使用时 `HOST_NAME_TAKEN`；同一台机器上 board host 异常退出后重启，带上上一次的标识（resume）可以接回名字。
//! - **机器设置**（同时跑几个、暂停接单、默认 agent、定时领取、每个 agent 的启动环境）存在服务端，界面上修改，
//!   host 通过推送实时拿到。机器第一次连上时按默认值建立；被移除后再连上来，按新机器处理。
//! - **启动环境**（启动环境与桌面端需求 §二）：在哪种 shell 里启动 agent 的命令（直接启动、登录 shell、cmd、
//!   PowerShell、Git Bash、WSL 的发行版 …）。board host 启动时发现本机有哪些，并在每个 agent 支持的环境里各检查
//!   一次命令找不找得到；每个 agent 用哪个由机器设置决定，没设置（或设置的环境本机已经没有）时取支持顺序里第一个
//!   找得到的（[`HostRow::pick`]）。派发开始时把用的环境写进派发记录，host 按它启动。旧版 board host 不上报启动环境，
//!   按直接启动。
//! - **派发**：一次「在某台机器上用某种 agent 做某张卡」。手动派发（界面「派发」或 `board dispatch`）和定时领取
//!   都会产生派发记录：排队（queued）→ 在跑（running）→ 结束（done，带结果）。
//! - **开始**（[`W::dispatch_begin`]、定时领取 [`W::dispatch_pull`]）在一个写事务里完成：生成 agent 名字
//!   `<agent 标识>-<机器名>/<模型名>`（例如 `dsh-win/deepseek-v4.1-flash`，见 [`dispatch_agent_name`]）、check 这个名字、
//!   以它领取卡片。领不到时这次派发结束，结果「没领到」。之后 host 每次上报时替这些名字发 agent 级心跳，直到派发结束。
//! - **结束**（[`W::dispatch_end`]）：agent 退出后，卡片仍由这个名字持有就发 final、沿连线移到「错误」节点（没有时去需要人处理的状态）
//!   并评论（异常退出 / 已停止）；否则看卡片在哪：交审状态是「已交审」，其他是「做不下去」。
//! - **占用**：卡片有排队中的派发时，别的 agent `board claim <ID>` 报 `DISPATCHED`，不带 ID 的 `board claim` 跳过它；
//!   人把它拖出可领取状态时这次派发自动取消。
//! - **掉线**：机器掉线超过 50 秒，名字跟着离线，卡片按现有的心跳超时规则释放，在跑的派发结束（机器掉线），见 [`W::sweep_hosts`]。
//!
//! 派发、取消、开始、结束都在卡片动态里追加一条，写明机器、agent、来源和结果。

use crate::error::{BoardError, ErrorCode, Result};
use crate::model::*;
use crate::store::{flow_of, R, W};
use crate::Cmd;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

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
  dirs     TEXT NOT NULL DEFAULT '[]',-- 白板与目录（JSON，按 --dir 的顺序）
  dirs_rev INTEGER NOT NULL DEFAULT 0,-- 目录设置的版本：服务端和 board host 用它分清远程修改和本机配置修改
  dirs_at  INTEGER,                   -- 最近一次改目录设置的时间
  dirs_by  TEXT,                      -- 最近一次改目录设置的人；本机 board host 上报的是「本机」
  max      INTEGER NOT NULL DEFAULT 1,-- 同时跑几个
  per_session INTEGER NOT NULL DEFAULT 4,-- 定时领取启动的会话一个最多做几张（board next）
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
  off_at   INTEGER                    -- 离线的时间；在线时为空
);

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
  result      TEXT,                   -- review | stuck | crashed | stopped | canceled | lost | unclaimed
  created_at  INTEGER NOT NULL,
  started_at  INTEGER,
  ended_at    INTEGER,
  exit_code   INTEGER,
  stop_by     TEXT,                   -- 有人请求停止：谁
  session     INTEGER,                -- 同一个 agent 进程做的几张卡：第一张的派发 ID（board next 接着做的那几张都写它）；空表示就是自己
  log         TEXT NOT NULL DEFAULT '' -- 日志的最后 500 行
);
CREATE INDEX IF NOT EXISTS dispatches_card ON dispatches (board, card, state);
CREATE INDEX IF NOT EXISTS dispatches_host ON dispatches (host_id, state, id);
"#;

/// 旧数据库补上启动环境的几列
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
        ("hosts", "dirs_rev", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "dirs_at", "INTEGER"),
        ("hosts", "dirs_by", "TEXT"),
        ("dispatches", "session", "INTEGER"),
    ] {
        if !has(table, col)? {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"))?;
        }
    }
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
    Ok(())
}

/// 机器名最长多少个字符
pub const HOST_NAME_MAX: usize = 24;
/// 同时跑几个的上限
pub const HOST_MAX_LIMIT: i64 = 8;
/// 一个会话最多做几张的上限（1–8，默认 4）
pub const PER_SESSION_LIMIT: i64 = 8;
/// 领取规则最多几条
pub const RULES_LIMIT: usize = 20;
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

/// 一次派发的 agent 名字：`<agent 标识>-<机器名>/<模型名>`，例如 `dsh-win/deepseek-v4.1-flash`；
/// 同一台机器同一 agent、模型有多个并发派发时模型名后面加 `-<序号>`，例如 `dsh-win/deepseek-v4.1-flash-2`（seq 0 表示不带）。
/// 模型名来自 board host 上报的 agent 数据（配置文件 `[agents.<标识>] model`，内置模板有默认值）；没有时用 agent 标识。
/// 转成名字能用的样子（小写、空白和非法字符换成 `-`、去掉开头结尾的 `-`），放不下时截断，保证总长不超过 48 个字符，
/// 能通过 `board check` 的名字格式校验（[`crate::check_agent_name`]）
pub fn dispatch_agent_name(agent: &str, host: &str, model: &str, seq: u32) -> String {
    let first = format!("{agent}-{host}");
    let suffix = if seq > 0 { format!("-{seq}") } else { String::new() };
    // 序号不截断：模型名先让出序号的位置
    let room = AGENT_NAME_MAX.saturating_sub(first.chars().count() + 1 + suffix.len()).max(1);
    let mut seg = name_segment(model, room);
    if seg.is_empty() {
        seg = name_segment(agent, room);
    }
    format!("{first}/{seg}{suffix}")
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

/// agent 标识：小写字母、数字、`.`、`-`，以字母开头，最长 16 个字符（名字 `<标识>-<机器名>/<模型名>` 不超过 48 个字符）
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

/* ------------------------------ 数据 ------------------------------ */

/// 本机 agent：board host 启动时按配置文件和内置模板逐个检查（任务下发需求 §三）。
///
/// board host 上报的 `found`、`version` 是它在默认启动环境里的结果；服务端发出去的（[`HostInfo::agents`]）
/// 换成按机器设置实际用的启动环境（`env`）里的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostAgent {
    /// 标识：claude、dsh、pi …
    pub id: String,
    /// 名称：Claude Code …
    pub name: String,
    /// `<命令> --version` 的第一行；没找到时为空
    #[serde(default)]
    pub version: String,
    /// 命令找到了
    pub found: bool,
    /// 这个 agent 支持、本机有的启动环境，按默认选择的顺序，各自的检查结果；旧版 board host 不上报
    #[serde(default)]
    pub envs: Vec<EnvCheck>,
    /// 实际用的启动环境（服务端按机器设置算出来；board host 上报时为空，旧版 board host 也为空）
    #[serde(default)]
    pub env: Option<String>,
    /// 在实际用的启动环境里找到了命令、却看不到 `board-skills` 时，写该装到哪里（board host 上报时是默认启动环境里的）。
    /// 看得到、没找到命令、配置文件里的 agent（不检查）、旧版 board host 都为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_skill: Option<String>,
    /// 这个 agent 在这台机器上实际配的模型名：派发名字的第二段（`<agent 标识>-<机器名>/<模型名>`）。
    /// 来源是 board host 的配置文件 `[agents.<标识>] model`，内置模板有默认值；没配置、旧版 board host 为空，
    /// 生成名字时用 agent 标识代替
    #[serde(default)]
    pub model: String,
}

/// 一个 agent 在一个启动环境里的检查结果
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvCheck {
    /// 启动环境标识
    pub env: String,
    /// 在这个环境里找到了命令（`command -v` / `where`）
    pub found: bool,
    /// 在这个环境里 `<命令> --version` 的第一行
    #[serde(default)]
    pub version: String,
    /// 找到了命令、却看不到 `board-skills` 时，写该装到哪里；其余情况（含旧版 board host）为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_skill: Option<String>,
}

/// 启动环境：board host 在哪种 shell 里启动 agent 的命令（启动环境与桌面端需求 §二）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEnv {
    /// `direct`、`login`、`cmd`、`powershell`、`pwsh`、`git-bash`、`wsl:<发行版>`
    pub id: String,
    /// 名称：直接启动、登录 shell、Git Bash、WSL（Ubuntu）…
    pub name: String,
    /// 程序路径（WSL 写发行版名，直接启动为空）
    #[serde(default)]
    pub path: String,
}

/// 机器设置里一个 agent 的运行设置
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRun {
    /// 启动环境；没有时按默认规则选
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
}

/// 白板与目录：派到这台机器的卡片，agent 在对应目录里启动
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDir {
    /// `<项目>/<白板>`
    pub board: String,
    /// 配置里写的路径（`~` 不展开）
    pub path: String,
    /// 命令行用 `--dir` 固定：界面上只读，要改启动参数
    #[serde(default)]
    pub fixed: bool,
    /// ok | missing | not_git；旧版 board host 不上报
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// 是 git 仓库时的 `origin`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// 是 git 仓库时的当前分支
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// board host 最近一次检查的时间
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    /// 目录不存在时，在这台机器上克隆的状态
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clone: Option<HostClone>,
}

/// 在 board host 那台机器上克隆一块白板的目录
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostClone {
    /// queued | running | done | failed
    pub state: String,
    /// 仓库地址
    pub url: String,
    /// 正在做什么 / 失败原因
    #[serde(default)]
    pub message: String,
    /// 状态最近一次变化的时间
    pub updated_at: i64,
    /// 谁发起的
    #[serde(default)]
    pub by: String,
}

/// 只比较目录设置本身（白板、路径、是否 --dir 固定），不看检查结果和克隆状态
fn same_dir_settings(a: &[HostDir], b: &[HostDir]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| x.board == y.board && x.path == y.path && x.fixed == y.fixed)
}

fn valid_board_ref(board: &str) -> bool {
    matches!(board.split_once('/'), Some((project, key)) if crate::valid_key(project) && crate::valid_key(key))
}

/// 把 board host 上报或界面提交的目录整理成设置：去掉首尾空白，保留同一块白板已有的检查结果
fn clean_dirs(raw: &[HostDir], old: &[HostDir]) -> Vec<HostDir> {
    raw.iter()
        .map(|d| {
            let board = d.board.trim().to_string();
            let path = d.path.trim().to_string();
            let mut out = HostDir {
                board: board.clone(),
                path: path.clone(),
                fixed: d.fixed,
                status: d.status.clone(),
                origin: d.origin.clone(),
                branch: d.branch.clone(),
                checked_at: d.checked_at,
                clone: d.clone.clone(),
            };
            if let Some(prev) = old.iter().find(|x| x.board == board && x.path == path) {
                out.status = prev.status.clone();
                out.origin = prev.origin.clone();
                out.branch = prev.branch.clone();
                out.checked_at = prev.checked_at;
                out.clone = prev.clone.clone();
            }
            out
        })
        .collect()
}

/// 服务端设置不变时，把 board host 刚检查到的目录状态合进来；旧版 board host 没有状态，保留原值
fn merge_dir_checks(settings: &mut [HostDir], checks: &[HostDir]) {
    for s in settings {
        let Some(r) = checks.iter().find(|r| r.board == s.board && r.path == s.path) else { continue };
        if r.checked_at.is_some() || r.status.is_some() {
            s.status = r.status.clone();
            s.origin = r.origin.clone();
            s.branch = r.branch.clone();
            s.checked_at = r.checked_at;
        }
        // 远程发起的克隆以服务端为准；board host 只上报 running / done / failed 覆盖排队状态
        if let Some(run) = &r.clone {
            match &s.clone {
                Some(old) if old.url == run.url => {
                    if run.state != "queued" {
                        s.clone = Some(run.clone());
                    }
                }
                Some(_) => {}
                None => s.clone = Some(run.clone()),
            }
        }
    }
}

/// 定时领取的一条领取规则：卡片的标签满足条件就由这个 agent 做。标签按名称匹配，不区分大小写
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimRule {
    /// `in` 带其中任一个 | `all` 全都带（多出的标签不管） | `not_in` 一个都不带 | `equal` 标签正好是这几个（顺序无关）
    /// | `only` 标签都在这几个里面、没有别的（至少带一个） | `none` 没有标签（不带标签列表）
    #[serde(default = "op_in")]
    pub op: String,
    #[serde(default)]
    pub labels: Vec<String>,
    /// 旧数据（单个标签 → agent）：读入时换成 `in [标签]`
    #[serde(default, skip_serializing)]
    pub label: Option<String>,
    pub agent: String,
    /// 服务端发出去时才有：agent 在这台机器上找得到，规则生效
    #[serde(default)]
    pub active: bool,
    /// 服务端发出去时才有：这台机器有目录的项目里都没有的标签（界面上标出来）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

fn op_in() -> String {
    "in".into()
}

/// 领取规则的条件
pub const RULE_OPS: [&str; 6] = ["in", "all", "not_in", "equal", "only", "none"];

/// 不带标签列表的条件：「没有标签」只看卡片有没有标签
fn op_needs_labels(op: &str) -> bool {
    op != "none"
}

impl ClaimRule {
    /// 旧数据的单个标签换成 `in [标签]`；标签去掉空白、按名称（不区分大小写）去重
    fn upgraded(mut self) -> ClaimRule {
        if let Some(l) = self.label.take() {
            if self.labels.is_empty() {
                self.labels = vec![l];
                self.op = "in".into();
            }
        }
        let mut seen: Vec<String> = vec![];
        self.labels = self
            .labels
            .into_iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && {
                let k = l.to_lowercase();
                !seen.contains(&k) && {
                    seen.push(k);
                    true
                }
            })
            .collect();
        // 「没有标签」不看标签列表：存着也只会让界面上多出没用的标签
        if !op_needs_labels(&self.op) {
            self.labels.clear();
        }
        self
    }
    /// 存进数据库的样子
    fn stored(&self) -> serde_json::Value {
        json!({ "op": self.op, "labels": self.labels, "agent": self.agent })
    }
}

/// 一台机器（界面「机器」页）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub id: i64,
    pub name: String,
    pub os: String,
    pub version: String,
    /// 本机 agent（`found`、`version`、`env` 按实际用的启动环境）
    pub agents: Vec<HostAgent>,
    /// 启动环境（按发现的顺序）；旧版 board host 不上报，为空（旧版服务端没有这一项）
    #[serde(default)]
    pub envs: Vec<HostEnv>,
    pub dirs: Vec<HostDir>,
    /// 服务端目录设置的版本
    #[serde(default)]
    pub dirs_rev: i64,
    /// 最近一次改目录设置的时间、是谁改的
    #[serde(default)]
    pub dirs_changed_at: Option<i64>,
    #[serde(default)]
    pub dirs_changed_by: Option<String>,
    /// 同时跑几个（1–8）
    pub max: i64,
    /// 定时领取启动的会话一个最多做几张（1–8，默认 4）；旧版服务端没有这一项
    #[serde(default = "default_per_session")]
    pub per_session: i64,
    /// 定时领取的领取规则（有顺序）；没有规则时定时领取不领卡、也不能开始一轮
    #[serde(default)]
    pub rules: Vec<ClaimRule>,
    /// 兜底：不带任何规则标签的卡片由这个 agent 做；没设时不领这些卡片
    #[serde(default)]
    pub fallback: Option<String>,
    /// 兜底的 agent 在这台机器上找得到
    #[serde(default)]
    pub fallback_active: bool,
    /// 有规则、没设兜底时：这台机器有目录的白板上，可领取、没有匹配任何规则的卡片数
    #[serde(default)]
    pub unmatched: i64,
    /// 暂停接单
    pub paused: bool,
    /// 默认 agent：设置过且找到了就是它，否则是本机 agent 里第一个找到的；都没找到时为空
    pub agent: Option<String>,
    /// 定时领取开关和 cron 表达式（5 段「分 时 日 月 周」，机器本地时区）
    pub cron_on: bool,
    pub cron: Vec<String>,
    /// 正在进行一轮领取（定时到点或「现在领一轮」）
    pub round: bool,
    pub online: bool,
    pub first_at: i64,
    pub beat_at: i64,
    pub off_at: Option<i64>,
}

fn default_per_session() -> i64 {
    4
}

/// board host 每 10 秒的上报（`POST /api/hosts/beat`）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostReport {
    pub name: String,
    /// 这个 board host 进程的标识
    pub instance: String,
    /// 同一台机器上一次异常退出的 board host 的标识：用来接回还在线的名字
    #[serde(default)]
    pub resume: Option<String>,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub agents: Vec<HostAgent>,
    /// 启动环境（旧版 board host 不上报）
    #[serde(default)]
    pub envs: Vec<HostEnv>,
    #[serde(default)]
    pub dirs: Vec<HostDir>,
    /// board host 已经应用的服务端目录版本；本机配置先改了时还是旧版本
    #[serde(default)]
    pub dirs_rev: i64,
    /// 本机正在跑（有进程）的派发
    #[serde(default)]
    pub running: Vec<i64>,
    /// board host 退出：机器马上离线
    #[serde(default, rename = "final")]
    pub done: bool,
}

/// 发给一台 board host 的任务：设置、在跑的派发（含停止请求）、排队的派发（按创建先后）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostWork {
    pub host: HostInfo,
    pub running: Vec<Dispatch>,
    pub queued: Vec<Dispatch>,
    pub now: i64,
}

/// 界面上修改机器设置
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPatch {
    #[serde(default)]
    pub max: Option<i64>,
    #[serde(default)]
    pub per_session: Option<i64>,
    /// 领取规则（整个列表替换）
    #[serde(default)]
    pub rules: Option<Vec<ClaimRule>>,
    /// 兜底的 agent；空字符串表示取消兜底
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub cron_on: Option<bool>,
    #[serde(default)]
    pub cron: Option<Vec<String>>,
    /// 一个 agent 的启动环境
    #[serde(default)]
    pub env: Option<EnvPatch>,
    /// 白板与目录（整个列表替换）
    #[serde(default)]
    pub dirs: Option<Vec<HostDir>>,
}

/// 改一个 agent 的启动环境
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EnvPatch {
    pub agent: String,
    pub env: String,
}

/// 一次派发
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dispatch {
    pub id: i64,
    /// 白板路径
    pub board: String,
    pub card: i64,
    pub title: String,
    pub host_id: i64,
    pub host: String,
    /// agent 标识与名称
    pub agent: String,
    pub agent_label: String,
    /// 开始时生成的 agent 名字
    pub name: Option<String>,
    /// 开始时用的启动环境标识与名称；旧版 board host（直接启动）、还没开始时为空
    pub env: Option<String>,
    pub env_name: Option<String>,
    /// manual 手动 | cron 定时
    pub source: String,
    /// 发起人
    pub by: Actor,
    /// queued 排队 | running 在跑 | done 结束
    pub state: String,
    /// review 已交审 | stuck 做不下去 | crashed 异常退出 | stopped 已停止 | canceled 已取消 | lost 机器掉线 | unclaimed 没领到
    pub result: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
    /// 有人请求停止（还在跑时）：谁
    pub stop_by: Option<String>,
    /// 持有者最近一次心跳（在跑时）
    pub beat_at: Option<i64>,
    /// 同一个 agent 进程做的几张卡共用的会话标识：第一张的派发 ID（旧数据、还没开始时就是自己的 ID）
    #[serde(default)]
    pub session: i64,
}

/// 结果的中文
pub fn result_words(r: &str) -> &'static str {
    match r {
        "review" => "已交审",
        "stuck" => "做不下去",
        "crashed" => "异常退出",
        "stopped" => "已停止",
        "canceled" => "已取消",
        "lost" => "机器掉线",
        "unclaimed" => "没领到",
        _ => "已结束",
    }
}

fn source_words(s: &str) -> &'static str {
    if s == "cron" { "定时" } else { "手动" }
}

/// 派发怎么结束（host 上报）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndHow {
    /// agent 进程退出（任何退出码；人停止时也是这样上报）
    Exit,
    /// board host 退出前结束了 agent 进程
    HostExit,
}

/* ------------------------------ 读 ------------------------------ */

struct HostRow {
    id: i64,
    name: String,
    instance: String,
    os: String,
    version: String,
    agents: Vec<HostAgent>,
    envs: Vec<HostEnv>,
    run: std::collections::BTreeMap<String, AgentRun>,
    dirs: Vec<HostDir>,
    dirs_rev: i64,
    dirs_at: Option<i64>,
    dirs_by: Option<String>,
    max: i64,
    per_session: i64,
    rules: Vec<ClaimRule>,
    fallback: Option<String>,
    paused: bool,
    agent: Option<String>,
    cron_on: bool,
    cron: Vec<String>,
    round_at: Option<i64>,
    rr: i64,
    first_at: i64,
    beat_at: i64,
    off_at: Option<i64>,
    agents_raw: String,
    envs_raw: String,
    dirs_raw: String,
}

const HOST_COLS: &str =
    "id, name, instance, os, version, agents, dirs, max, paused, agent, cron_on, cron, round_at, rr, first_at, beat_at, off_at, envs, run, per_session, rules, fallback, dirs_rev, dirs_at, dirs_by";

fn host_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HostRow> {
    let agents_raw: String = r.get(5)?;
    let dirs_raw: String = r.get(6)?;
    let cron: String = r.get(11)?;
    let envs_raw: String = r.get(17)?;
    let run: String = r.get(18)?;
    Ok(HostRow {
        id: r.get(0)?,
        name: r.get(1)?,
        instance: r.get(2)?,
        os: r.get(3)?,
        version: r.get(4)?,
        agents: serde_json::from_str(&agents_raw).unwrap_or_default(),
        envs: serde_json::from_str(&envs_raw).unwrap_or_default(),
        run: serde_json::from_str(&run).unwrap_or_default(),
        dirs: serde_json::from_str(&dirs_raw).unwrap_or_default(),
        dirs_rev: r.get(22)?,
        dirs_at: r.get(23)?,
        dirs_by: r.get(24)?,
        max: r.get(7)?,
        per_session: r.get(19)?,
        rules: serde_json::from_str::<Vec<ClaimRule>>(&r.get::<_, String>(20)?).unwrap_or_default().into_iter().map(ClaimRule::upgraded).collect(),
        fallback: r.get(21)?,
        paused: r.get::<_, i64>(8)? != 0,
        agent: r.get(9)?,
        cron_on: r.get::<_, i64>(10)? != 0,
        cron: serde_json::from_str(&cron).unwrap_or_default(),
        round_at: r.get(12)?,
        rr: r.get(13)?,
        first_at: r.get(14)?,
        beat_at: r.get(15)?,
        off_at: r.get(16)?,
        agents_raw,
        envs_raw,
        dirs_raw,
    })
}

impl HostRow {
    fn online(&self, now: i64) -> bool {
        self.off_at.is_none() && self.beat_at + HOST_GAP_MS > now
    }
    /// 一个 agent 实际用的启动环境（启动环境与桌面端需求 §2.4）：设置过、本机还有（在它支持的环境里）就用它，
    /// 找没找到都按它算；否则按支持的顺序取第一个找到命令的。都找不到时为空。
    /// 旧版 board host 不上报启动环境：返回 None，找没找到按它上报的
    fn pick<'h>(&self, a: &'h HostAgent) -> Option<&'h EnvCheck> {
        if self.envs.is_empty() {
            return None;
        }
        let set = self.run.get(&a.id).and_then(|r| r.env.as_deref());
        set.and_then(|e| a.envs.iter().find(|c| c.env == e)).or_else(|| a.envs.iter().find(|c| c.found))
    }
    /// 本机 agent，`found`、`version`、`env` 换成实际用的启动环境里的结果
    fn resolved(&self) -> Vec<HostAgent> {
        self.agents
            .iter()
            .map(|a| {
                let mut out = a.clone();
                if !self.envs.is_empty() {
                    let c = self.pick(a);
                    out.found = c.is_some_and(|c| c.found);
                    out.version = c.map(|c| c.version.clone()).unwrap_or_default();
                    out.env = c.map(|c| c.env.clone());
                    out.no_skill = c.filter(|c| c.found).and_then(|c| c.no_skill.clone());
                }
                out
            })
            .collect()
    }
    /// 启动环境的名称
    fn env_name(&self, id: &str) -> String {
        self.envs.iter().find(|e| e.id == id).map(|e| e.name.clone()).unwrap_or_else(|| id.to_string())
    }
    /// 这台机器上找得到这个 agent
    fn has_agent(&self, id: &str) -> bool {
        self.resolved().iter().any(|a| a.id == id && a.found)
    }
    /// 解析后的领取规则；一条规则也没有时为 None：定时领取不领卡（兜底只处理规则没命中的卡片，不算规则）
    fn claim_rules(&self) -> Option<Rules> {
        if self.rules.is_empty() {
            return None;
        }
        Some(Rules {
            list: self
                .rules
                .iter()
                .map(|r| Cond {
                    op: r.op.clone(),
                    labels: r.labels.iter().map(|l| l.trim().to_lowercase()).collect(),
                    agent: r.agent.clone(),
                    ok: self.has_agent(&r.agent),
                })
                .collect(),
            fallback: self.fallback.clone().map(|a| {
                let ok = self.has_agent(&a);
                (a, ok)
            }),
        })
    }
    /// 有没有至少一条领取规则（兜底不算）。没有时定时领取不领卡，「现在领一轮」也不能开始
    fn has_rules(&self) -> bool {
        !self.rules.is_empty()
    }
    /// 默认 agent：设置过且找到了就是它，否则第一个找到的
    fn default_agent(&self) -> Option<HostAgent> {
        let all = self.resolved();
        let found = |id: &str| all.iter().find(|a| a.id == id && a.found).cloned();
        self.agent.as_deref().and_then(found).or_else(|| all.iter().find(|a| a.found).cloned())
    }
    fn info(&self, now: i64) -> HostInfo {
        HostInfo {
            id: self.id,
            name: self.name.clone(),
            os: self.os.clone(),
            version: self.version.clone(),
            agents: self.resolved(),
            envs: self.envs.clone(),
            dirs: self.dirs.clone(),
            dirs_rev: self.dirs_rev,
            dirs_changed_at: self.dirs_at,
            dirs_changed_by: self.dirs_by.clone(),
            max: self.max,
            per_session: self.per_session,
            rules: self.rules.iter().map(|x| ClaimRule { active: self.has_agent(&x.agent), ..x.clone() }).collect(),
            fallback: self.fallback.clone(),
            fallback_active: self.fallback.as_deref().is_some_and(|a| self.has_agent(a)),
            unmatched: 0,
            paused: self.paused,
            agent: self.default_agent().map(|a| a.id.clone()),
            cron_on: self.cron_on,
            cron: self.cron.clone(),
            round: self.round_at.is_some(),
            online: self.online(now),
            first_at: self.first_at,
            beat_at: self.beat_at,
            off_at: self.off_at,
        }
    }
}

const DISPATCH_SELECT: &str = "SELECT d.id, b.project || '/' || b.key, d.card, COALESCE(c.title, d.title), d.host_id, d.host, d.agent,
       d.agent_label, d.name, d.source, d.by_kind, d.by_name, d.state, d.result, d.created_at, d.started_at, d.ended_at,
       d.exit_code, d.stop_by, CASE WHEN c.holder = d.name THEN c.beat_at END, d.env, d.env_name, COALESCE(d.session, d.id)
  FROM dispatches d JOIN boards b ON b.id = d.board LEFT JOIN cards c ON c.board = d.board AND c.id = d.card";

fn dispatch_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Dispatch> {
    Ok(Dispatch {
        id: r.get(0)?,
        board: r.get(1)?,
        card: r.get(2)?,
        title: r.get(3)?,
        host_id: r.get(4)?,
        host: r.get(5)?,
        agent: r.get(6)?,
        agent_label: r.get(7)?,
        name: r.get(8)?,
        env: r.get(20)?,
        env_name: r.get(21)?,
        source: r.get(9)?,
        by: Actor { kind: ActorKind::parse(&r.get::<_, String>(10)?), name: r.get(11)? },
        state: r.get(12)?,
        result: r.get(13)?,
        created_at: r.get(14)?,
        started_at: r.get(15)?,
        ended_at: r.get(16)?,
        exit_code: r.get(17)?,
        stop_by: r.get(18)?,
        beat_at: r.get(19)?,
        session: r.get(22)?,
    })
}

/// 一条解析后的领取规则
struct Cond {
    op: String,
    /// 标签名小写
    labels: Vec<String>,
    agent: String,
    /// agent 在这台机器上找得到
    ok: bool,
}

impl Cond {
    /// 卡片的标签（小写）满足条件吗
    fn matches(&self, card: &[String]) -> bool {
        match self.op.as_str() {
            // 空规则不匹配任何卡片（保存时会被拦住，这里兜住手写的数据）
            "all" => !self.labels.is_empty() && self.labels.iter().all(|l| card.contains(l)),
            "not_in" => !self.labels.iter().any(|l| card.contains(l)),
            "equal" => {
                let mut a: Vec<&String> = card.iter().collect();
                a.sort();
                a.dedup();
                let mut b: Vec<&String> = self.labels.iter().collect();
                b.sort();
                b.dedup();
                a == b
            }
            // 标签都在这几个里面、没有别的，至少要有一个（空规则不匹配任何卡片）
            "only" => !self.labels.is_empty() && !card.is_empty() && card.iter().all(|l| self.labels.contains(l)),
            "none" => card.is_empty(),
            _ => self.labels.iter().any(|l| card.contains(l)),
        }
    }
}

/// 解析后的领取规则（任务下发：定时领取按标签条件筛卡片、选 agent）
struct Rules {
    list: Vec<Cond>,
    /// 兜底的 agent，找得到
    fallback: Option<(String, bool)>,
}

impl Rules {
    /// 一张卡片（标签名小写）交给哪个 agent：按顺序第一条命中、生效的规则；先命中的是不生效的规则（agent 找不到）时不领它；
    /// 一条都没命中时用兜底；都没有时不领它
    fn agent_for(&self, labels: &[String]) -> Option<&str> {
        let hit = self.list.iter().find(|c| c.matches(labels))?;
        if hit.ok {
            return Some(&hit.agent);
        }
        None
    }
    /// 同上，算上兜底
    fn route(&self, labels: &[String]) -> Option<&str> {
        if self.matches_any(labels) {
            return self.agent_for(labels);
        }
        self.fallback.as_ref().filter(|(_, ok)| *ok).map(|(a, _)| a.as_str())
    }
    /// 有一条规则命中（不管生效没有）
    fn matches_any(&self, labels: &[String]) -> bool {
        self.list.iter().any(|c| c.matches(labels))
    }
    /// 规则里用到的标签（小写）
    fn labels(&self) -> Vec<String> {
        let mut out: Vec<String> = self.list.iter().flat_map(|c| c.labels.clone()).collect();
        out.sort();
        out.dedup();
        out
    }
}

/// 卡片的标签名（小写）
fn card_label_names(conn: &rusqlite::Connection, b: i64, card: i64) -> Result<Vec<String>> {
    let mut st = conn.prepare("SELECT lower(l.name) FROM card_labels cl JOIN labels l ON l.id = cl.label WHERE cl.board = ?1 AND cl.card = ?2")?;
    let out = st.query_map(params![b, card], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(out)
}

fn no_such_host(name: &str) -> BoardError {
    BoardError::new(ErrorCode::NoSuchHost, format!("机器「{name}」不存在：在那台机器上运行 board host，界面「机器」里能看到所有机器"))
        .with_detail(json!({ "host": name }))
}

impl<'a> R<'a> {
    fn host_row(&self, name: &str) -> Result<Option<HostRow>> {
        Ok(self.conn.query_row(&format!("SELECT {HOST_COLS} FROM hosts WHERE name = ?1"), [name], host_row).optional()?)
    }

    /// 所有机器，按名字排序
    pub fn hosts(&self, now: i64) -> Result<Vec<HostInfo>> {
        let mut st = self.conn.prepare(&format!("SELECT {HOST_COLS} FROM hosts ORDER BY name"))?;
        let rows = st.query_map([], host_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
        rows.iter().map(|h| self.host_info(h, now)).collect()
    }

    pub fn host(&self, name: &str, now: i64) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        self.host_info(&h, now)
    }

    /// 机器信息，加上「有 N 张可领取的卡片没有匹配任何规则」（有规则、没设兜底时）
    fn host_info(&self, h: &HostRow, now: i64) -> Result<HostInfo> {
        let mut info = h.info(now);
        // 规则里这台机器有目录的项目里都没有的标签：界面上标出来
        let known = self.host_labels(h)?;
        for r in info.rules.iter_mut() {
            r.missing = r.labels.iter().filter(|l| !known.contains(&l.to_lowercase())).cloned().collect();
        }
        if let Some(rules) = h.claim_rules().filter(|r| r.fallback.is_none()) {
            for dir in &h.dirs {
                let Ok(board) = self.resolve(&dir.board) else { continue };
                let flow = flow_of(self.conn, board.id)?;
                for s in flow.0.iter().filter(|s| s.claim && flow.claim_target(&s.key).is_some()) {
                    let mut st = self.conn.prepare("SELECT id FROM cards WHERE board = ?1 AND col = ?2 AND deleted_at IS NULL")?;
                    let ids = st.query_map(params![board.id, s.key], |r| r.get::<_, i64>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
                    for id in ids {
                        if !rules.matches_any(&card_label_names(self.conn, board.id, id)?) {
                            info.unmatched += 1;
                        }
                    }
                }
            }
        }
        Ok(info)
    }

    /// 这台机器有目录的白板所在项目里的标签名（小写）
    fn host_labels(&self, h: &HostRow) -> Result<Vec<String>> {
        let mut out: Vec<String> = vec![];
        for dir in &h.dirs {
            let Some(project) = dir.board.split('/').next() else { continue };
            let mut st = self.conn.prepare("SELECT lower(name) FROM labels WHERE project = ?1")?;
            for n in st.query_map([project], |r| r.get::<_, String>(0))? {
                let n = n?;
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        Ok(out)
    }

    fn dispatches(&self, tail: &str, p: impl rusqlite::Params) -> Result<Vec<Dispatch>> {
        let mut st = self.conn.prepare(&format!("{DISPATCH_SELECT} {tail}"))?;
        let rows = st.query_map(p, dispatch_row)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn dispatch(&self, id: i64) -> Result<Dispatch> {
        self.dispatches("WHERE d.id = ?1", [id])?
            .pop()
            .ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("派发 #{id} 不存在")))
    }

    /// 排队和正在跑的派发（所有机器，按创建先后）
    pub fn active_dispatches(&self) -> Result<Vec<Dispatch>> {
        self.dispatches("WHERE d.state != 'done' ORDER BY d.id", [])
    }

    /// 一张卡片排队或正在跑的派发
    pub fn card_dispatch(&self, board: &BoardRef, card: i64) -> Result<Option<Dispatch>> {
        Ok(self.dispatches("WHERE d.board = ?1 AND d.card = ?2 AND d.state != 'done' ORDER BY d.id DESC LIMIT 1", params![board.id, card])?.pop())
    }

    /// 一台机器最近结束的派发（新的在前）
    pub fn host_recent(&self, host_id: i64, limit: i64) -> Result<Vec<Dispatch>> {
        self.dispatches("WHERE d.host_id = ?1 AND d.state = 'done' ORDER BY d.ended_at DESC, d.id DESC LIMIT ?2", params![host_id, limit])
    }

    /// 派发的日志（最后 500 行）
    pub fn dispatch_log(&self, id: i64) -> Result<Vec<String>> {
        let log: Option<String> = self.conn.query_row("SELECT log FROM dispatches WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        let log = log.ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("派发 #{id} 不存在")))?;
        Ok(if log.is_empty() { vec![] } else { log.split('\n').map(str::to_string).collect() })
    }

    /// 发给一台 board host 的任务。在跑的派发按机器名找（机器被移除、再连上来之后，原来在跑的派发照样收到停止请求）
    pub fn host_work(&self, name: &str, now: i64) -> Result<HostWork> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        Ok(HostWork {
            running: self.dispatches("WHERE d.host = ?1 AND d.state = 'running' ORDER BY d.id", [name])?,
            queued: self.dispatches("WHERE d.host_id = ?1 AND d.state = 'queued' ORDER BY d.id", [h.id])?,
            host: self.host_info(&h, now)?,
            now,
        })
    }
}

/* ------------------------------ 写 ------------------------------ */

/// 数据库里一次派发的几项（写操作用）
struct DRow {
    id: i64,
    board: i64,
    card: i64,
    host_id: i64,
    host: String,
    agent: String,
    agent_label: String,
    name: Option<String>,
    env_name: Option<String>,
    source: String,
    state: String,
    stop_by: Option<String>,
    started_at: Option<i64>,
    env: Option<String>,
    /// 会话标识：第一张的派发 ID
    session: i64,
}

impl<'a> W<'a> {
    fn host_row(&self, name: &str) -> Result<Option<HostRow>> {
        self.read().host_row(name)
    }

    /// board host 按名字找自己的机器记录。名字找不到时，再按这个进程的标识找：机器刚被改名
    /// （桌面端「设置 → 本机」保存机器名，先在服务端改名、再写配置文件），正在跑的 board host
    /// 读到新配置之前还在用旧名字上报、取任务，认回改名后的那条记录，不新建第二台机器。
    /// 标识每次启动随机生成，只有这个进程自己知道，别的机器借不到
    fn host_of_proc(&self, name: &str, instance: &str) -> Result<Option<HostRow>> {
        if let Some(h) = self.host_row(name)? {
            return Ok(Some(h));
        }
        if instance.trim().is_empty() {
            return Ok(None);
        }
        Ok(self.tx.query_row(&format!("SELECT {HOST_COLS} FROM hosts WHERE instance = ?1"), [instance], host_row).optional()?)
    }

    fn drow(&self, id: i64) -> Result<DRow> {
        self.tx
            .query_row(
                "SELECT id, board, card, host_id, host, agent, agent_label, name, source, state, stop_by, started_at, env_name, env, COALESCE(session, id) FROM dispatches WHERE id = ?1",
                [id],
                |r| {
                    Ok(DRow {
                        id: r.get(0)?,
                        board: r.get(1)?,
                        card: r.get(2)?,
                        host_id: r.get(3)?,
                        host: r.get(4)?,
                        agent: r.get(5)?,
                        agent_label: r.get(6)?,
                        name: r.get(7)?,
                        source: r.get(8)?,
                        state: r.get(9)?,
                        stop_by: r.get(10)?,
                        started_at: r.get(11)?,
                        env_name: r.get(12)?,
                        env: r.get(13)?,
                        session: r.get(14)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("派发 #{id} 不存在")))
    }

    fn board_of(&self, id: i64) -> Result<BoardRef> {
        Ok(self.tx.query_row("SELECT id, project, key FROM boards WHERE id = ?1", [id], |r| {
            Ok(BoardRef { id: r.get(0)?, project: r.get(1)?, key: r.get(2)? })
        })?)
    }

    fn card_exists(&self, b: i64, id: i64) -> Result<bool> {
        Ok(self.tx.query_row("SELECT COUNT(*) FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![b, id], |r| r.get::<_, i64>(0))? > 0)
    }

    /// 在卡片动态里记一条派发的事；卡片已经删除时不记
    fn track_dispatch(&mut self, b: i64, card: i64, who: &Actor, action: &str, text: &str) -> Result<()> {
        if self.card_exists(b, card)? {
            self.track(b, card, who, action, text)?;
        }
        Ok(())
    }

    fn touch_hosts(&mut self) {
        self.hosts_changed = true;
        self.changed = true;
    }

    /// 卡片排队中的派发（别的 agent 领取时用）
    pub(crate) fn queued_dispatch(&self, b: i64, card: i64) -> Result<Option<(i64, String)>> {
        Ok(self
            .tx
            .query_row(
                "SELECT id, host FROM dispatches WHERE board = ?1 AND card = ?2 AND state = 'queued' ORDER BY id LIMIT 1",
                params![b, card],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// 按规则 1 找下一张可领取的卡片：在连到锁定状态的可领取状态里（按工作流顺序，状态内从上到下），
    /// 没有排队派发的卡片中，带优先标签的先领；都不带时取最前面那张。
    /// `skip` 是调用方额外要跳过的卡片（定时领取传这一轮已经派发过的卡片）。
    pub(crate) fn next_claimable(&self, b: i64, flow: &Flow, skip: &[i64]) -> Result<Option<(i64, String)>> {
        let list = self.claimable_order(b, flow, skip)?;
        let urgent = self.priority_cards(b)?;
        Ok(list.iter().find(|(id, _)| urgent.contains(id)).or(list.first()).cloned())
    }

    /// 可领取的卡片，按规则 1 的顺序（连到锁定状态的可领取状态按工作流顺序、状态内从上到下），
    /// 去掉有排队派发的和 `skip` 里的；第二项是所在状态
    fn claimable_order(&self, b: i64, flow: &Flow, skip: &[i64]) -> Result<Vec<(i64, String)>> {
        let mut taken: Vec<i64> = self.collect("SELECT card FROM dispatches WHERE board = ?1 AND state = 'queued'", [b], |r| r.get(0))?;
        taken.extend_from_slice(skip);
        let mut out = vec![];
        for s in flow.0.iter().filter(|s| s.claim && flow.claim_target(&s.key).is_some()) {
            out.extend(self.col_ids(b, &s.key)?.into_iter().filter(|id| !taken.contains(id)).map(|id| (id, s.key.clone())));
        }
        Ok(out)
    }

    /// 带优先标签的卡片
    fn priority_cards(&self, b: i64) -> Result<Vec<i64>> {
        self.collect(
            "SELECT DISTINCT cl.card FROM card_labels cl JOIN labels l ON l.id = cl.label WHERE cl.board = ?1 AND l.priority = 1",
            [b],
            |r| r.get(0),
        )
    }

    /// `board next`（只在 board host 启动的会话里用）：结算这个名字正在跑的派发，挑下一张，以同一个名字领取，
    /// 记一条新的派发（同一台机器、agent、启动环境，来源定时，同一个会话）。都在一个写事务里。
    ///
    /// 怎么挑：候选是会话所在白板上可领取、没有排队派发、这一轮（或这个会话里）没派发过的卡片。
    /// 1. 有带优先标签的，按规则 1 领到的就是它；2. 否则和会话第一张卡共同标签最多的（至少一个），并列按规则 1 的顺序；
    /// 3. 都没有共同标签时按规则 1 的顺序取第一张。
    pub fn dispatch_next(&mut self, name: &str) -> Result<CardDetail> {
        let cur: Option<i64> = self
            .tx
            .query_row("SELECT id FROM dispatches WHERE name = ?1 AND state = 'running' ORDER BY id DESC LIMIT 1", [name], |r| r.get(0))
            .optional()?;
        let Some(cur) = cur else {
            return Err(BoardError::usage(format!(
                "board next 只能在 board host 启动的会话里用：{name} 没有正在跑的派发"
            )));
        };
        let d = self.drow(cur)?;
        let board = self.board_of(d.board)?;
        let holder: Option<String> = self
            .tx
            .query_row("SELECT holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0))
            .optional()?
            .flatten();
        if holder.as_deref() == Some(name) {
            return Err(BoardError::new(
                ErrorCode::AlreadyHolding,
                format!("#{} 还由你持有：先交审（board heartbeat final，再沿连线移到交审状态）或交给人处理，再 board next", d.card),
            ));
        }
        let done = |why: String| Err(BoardError::new(ErrorCode::SessionDone, format!("{why}：结束这个会话")));
        if d.source != "cron" {
            return done("这个会话是手动派发启动的，只做指定的那一张".into());
        }
        if d.stop_by.is_some() {
            return done("有人停止了这个会话".into());
        }
        let Some(h) = self.host_row(&d.host)? else {
            return done(format!("机器「{}」已被移除", d.host));
        };
        if h.paused {
            return done(format!("机器「{}」暂停接单", h.name));
        }
        let n: i64 = self.tx.query_row("SELECT COUNT(*) FROM dispatches WHERE COALESCE(session, id) = ?1", [d.session], |r| r.get(0))?;
        if n >= h.per_session {
            return done(format!("这个会话已经做了 {n} 张（机器设置：一个会话最多做 {} 张）", h.per_session));
        }
        // 候选：这一轮、这个会话里这台机器派发过的卡片不再领
        let (first_card, first_at): (i64, i64) =
            self.tx.query_row("SELECT card, created_at FROM dispatches WHERE id = ?1", [d.session], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let since = h.round_at.map_or(first_at, |r| r.min(first_at));
        let skip: Vec<i64> = self.collect(
            "SELECT DISTINCT card FROM dispatches WHERE board = ?1 AND host_id = ?2 AND created_at >= ?3",
            params![d.board, h.id, since],
            |r| r.get(0),
        )?;
        let flow = flow_of(self.tx, d.board)?;
        let list = self.claimable_order(d.board, &flow, &skip)?;
        // 只留按规则交给这个会话的 agent 的卡片；规则里的标签只表示谁来做，不算「共同标签」。
        // 一条规则也没有了（例如被删光）时不接着领：这个会话做完当前这张就结束
        let Some(rules) = h.claim_rules() else {
            return done(format!("机器「{}」已经没有领取规则", h.name));
        };
        let mut keep = vec![];
        for c in list {
            if rules.route(&card_label_names(self.tx, d.board, c.0)?) == Some(d.agent.as_str()) {
                keep.push(c);
            }
        }
        let list = keep;
        let rule_labels = rules.labels();
        if list.is_empty() {
            return done(format!("白板 {} 上没有这个会话能领的卡片", board.path()));
        }
        let urgent = self.priority_cards(d.board)?;
        let card = match list.iter().find(|(id, _)| urgent.contains(id)) {
            Some((id, _)) => *id,
            None => {
                let mine: Vec<String> =
                    card_label_names(self.tx, d.board, first_card)?.into_iter().filter(|l| !rule_labels.contains(l)).collect();
                let mut best = (0usize, list[0].0);
                for (id, _) in &list {
                    let theirs = card_label_names(self.tx, d.board, *id)?;
                    let common = theirs.iter().filter(|l| mine.contains(l)).count();
                    if common > best.0 {
                        best = (common, *id);
                    }
                }
                best.1
            }
        };
        // 结算上一张：卡片已经不由这个名字持有，在交审状态记「已交审」，其他状态记「做不下去」
        let col: Option<String> =
            self.tx.query_row("SELECT col FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0)).optional()?;
        let result = if col.as_deref().is_some_and(|c| flow.is_review(c)) { "review" } else { "stuck" };
        self.close_row(&d, result, None)?;
        let actor = Actor::agent(name.to_string());
        let took = d.started_at.map(|t| format!("，用时 {}", crate::engine::dur_words(self.now - t))).unwrap_or_default();
        let text = format!("派发结束：{}（{}{took}），接着做下一张", result_words(result), Self::dispatch_words(&d));
        self.track_dispatch(d.board, d.card, &actor, "dispatch", &text)?;
        // 以同一个名字领取下一张
        self.dispatch_ok = true;
        let claimed = self.exec(&board, &actor, Cmd::Claim { id: Some(card) });
        self.dispatch_ok = false;
        claimed?;
        let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, card], |r| r.get(0))?;
        let by = Actor::system();
        self.tx.execute(
            "INSERT INTO dispatches (board, card, title, host_id, host, agent, agent_label, name, env, env_name, source, by_kind, by_name, state, created_at, started_at, session)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'cron', ?11, ?12, 'running', ?13, ?13, ?14)",
            params![
                d.board, card, title, h.id, h.name, d.agent, d.agent_label, name, d.env, d.env_name,
                by.kind.as_str(), by.name, self.now, d.session
            ],
        )?;
        let who = match &d.env_name {
            Some(e) => format!("{}（{e}）", d.agent_label),
            None => format!("{} ", d.agent_label),
        };
        let text = format!("派发开始：{} 上的 {who}以 {name} 接着做（定时，这个会话的第 {} 张）", h.name, n + 1);
        self.track(d.board, card, &actor, "dispatch", &text)?;
        self.touch_hosts();
        self.read().card(&board, card)
    }

    /// 写一条派发的结束（不动名字：board next 接着做下一张时用）
    fn close_row(&mut self, d: &DRow, result: &str, exit_code: Option<i64>) -> Result<()> {
        self.tx.execute(
            "UPDATE dispatches SET state = 'done', result = ?2, ended_at = ?3, exit_code = ?4 WHERE id = ?1",
            params![d.id, result, self.now, exit_code],
        )?;
        self.touch_hosts();
        Ok(())
    }

    /// 写一条派发的结束，并让它的名字离线（名字只给这个会话用）
    fn close_dispatch(&mut self, d: &DRow, result: &str, exit_code: Option<i64>) -> Result<()> {
        self.close_row(d, result, exit_code)?;
        if let Some(name) = &d.name {
            let online: bool = self.tx.query_row(
                "SELECT COUNT(*) > 0 FROM agents WHERE name = ?1 AND off_at IS NULL",
                [name],
                |r| r.get(0),
            )?;
            if online {
                self.tx.execute("UPDATE agents SET off_at = ?2 WHERE name = ?1", params![name, self.now])?;
            }
        }
        self.touch_hosts();
        Ok(())
    }

    /// 动态里的「mbp · Claude Code · claude-mbp/opus-5.5，手动」；有启动环境时「mbp · pi · Git Bash · pi-mbp/pi，手动」
    fn dispatch_words(d: &DRow) -> String {
        let mut s = format!("{} · {}", d.host, d.agent_label);
        if let Some(e) = &d.env_name {
            s += &format!(" · {e}");
        }
        if let Some(n) = &d.name {
            s += &format!(" · {n}");
        }
        s + "，" + source_words(&d.source)
    }

    /* ---------------- 机器 ---------------- */

    /// board host 的上报（每 10 秒；启动时第一次）：建立或更新机器，替在跑的派发发心跳，返回它的任务。
    ///
    /// - 机器名正被另一个在线的 board host 使用（标识不同，也不是 resume 接回）时 `HOST_NAME_TAKEN`
    /// - 被移除过的机器再连上来，按新机器建立设置
    /// - 在跑的派发：本机有进程的，替它的名字发 agent 级心跳（维持名字在线和卡片的持有）；本机没有进程的
    ///   （board host 重启后接回），按「agent 退出，卡片仍由这个名字持有」处理，结果「异常退出」
    /// - `final`：board host 退出，机器马上离线
    pub fn host_beat(&mut self, rep: &HostReport, token: Option<i64>) -> Result<HostWork> {
        check_host_name(&rep.name).map_err(BoardError::usage)?;
        if rep.instance.trim().is_empty() {
            return Err(BoardError::usage("缺少 instance"));
        }
        for a in &rep.agents {
            check_agent_id(&a.id).map_err(BoardError::usage)?;
        }
        for e in &rep.envs {
            if e.id.trim().is_empty() || e.id.len() > 64 {
                return Err(BoardError::usage(format!("启动环境标识不对：{}", e.id)));
            }
        }
        let mut seen: Vec<&str> = vec![];
        for d in &rep.dirs {
            let board = d.board.trim();
            if !valid_board_ref(board) {
                return Err(BoardError::usage(format!("白板要写成 <项目>/<白板>：{}", d.board)));
            }
            if d.path.trim().is_empty() {
                return Err(BoardError::usage(format!("白板 {board} 还没有目录")));
            }
            if seen.contains(&board) {
                return Err(BoardError::usage(format!("白板 {board} 写了两次")));
            }
            seen.push(board);
        }
        let now = self.now;
        let agents = serde_json::to_string(&rep.agents).unwrap_or_else(|_| "[]".into());
        let envs = serde_json::to_string(&rep.envs).unwrap_or_else(|_| "[]".into());
        // 改名后还用旧名字上报：认回改名后的记录（见 host_of_proc），下面都用记录里的名字
        let found = self.host_of_proc(&rep.name, &rep.instance)?;
        let name = found.as_ref().map_or_else(|| rep.name.clone(), |h| h.name.clone());
        match found {
            None => {
                if rep.done {
                    return Err(no_such_host(&rep.name));
                }
                let mut dirs = clean_dirs(&rep.dirs, &[]);
                for d in &mut dirs {
                    d.clone = None;
                }
                let dirs = serde_json::to_string(&dirs).unwrap_or_else(|_| "[]".into());
                self.tx.execute(
                    "INSERT INTO hosts (name, token, instance, os, version, agents, dirs, dirs_rev, dirs_at, dirs_by, envs, first_at, beat_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, '本机', ?9, ?8, ?8)",
                    params![rep.name, token, rep.instance, rep.os, rep.version, agents, dirs, now, envs],
                )?;
                self.touch_hosts();
            }
            Some(h) => {
                let online = h.online(now);
                if online && h.instance != rep.instance && rep.resume.as_deref() != Some(h.instance.as_str()) {
                    return Err(BoardError::new(
                        ErrorCode::HostNameTaken,
                        format!("机器名「{}」正被在线的 board host 使用：换一个 --name，或先停掉那台机器上的 board host", rep.name),
                    )
                    .with_detail(json!({ "host": rep.name })));
                }
                if rep.done {
                    if h.instance == rep.instance && h.off_at.is_none() {
                        self.tx.execute("UPDATE hosts SET off_at = ?2, round_at = NULL WHERE id = ?1", params![h.id, now])?;
                        self.touch_hosts();
                    }
                    return self.read().host_work(&name, now);
                }
                let mut current = h.dirs.clone();
                let changed_settings = !same_dir_settings(&current, &rep.dirs);
                let mut dirs_rev = h.dirs_rev;
                let mut dirs_at = h.dirs_at;
                let mut dirs_by = h.dirs_by.clone();
                if rep.dirs_rev == h.dirs_rev && changed_settings {
                    // 没有远程修改在先，但配置里的目录变了：本机改了配置，服务端收下并升版本
                    current = clean_dirs(&rep.dirs, &current);
                    dirs_rev = h.dirs_rev + 1;
                    dirs_at = Some(now);
                    dirs_by = Some("本机".into());
                } else {
                    // 服务端有更新的设置时以服务端为准；只把同一路径的检查结果合进来
                    let mut next = current.clone();
                    merge_dir_checks(&mut next, &rep.dirs);
                    current = next;
                }
                let dirs = serde_json::to_string(&current).unwrap_or_else(|_| "[]".into());
                let differs = !online
                    || h.instance != rep.instance
                    || h.os != rep.os
                    || h.version != rep.version
                    || h.agents_raw != agents
                    || h.envs_raw != envs
                    || h.dirs_raw != dirs;
                self.tx.execute(
                    "UPDATE hosts SET token = ?2, instance = ?3, os = ?4, version = ?5, agents = ?6, dirs = ?7, beat_at = ?8, off_at = NULL, envs = ?9,
                                      dirs_rev = ?10, dirs_at = ?11, dirs_by = ?12 WHERE id = ?1",
                    params![h.id, token, rep.instance, rep.os, rep.version, agents, dirs, now, envs, dirs_rev, dirs_at, dirs_by],
                )?;
                // 换了一个 board host 进程（重启接回）：上一个进程的那一轮领取不再继续
                if h.instance != rep.instance && h.round_at.is_some() {
                    self.tx.execute("UPDATE hosts SET round_at = NULL WHERE id = ?1", [h.id])?;
                }
                if differs {
                    self.touch_hosts();
                }
            }
        }
        // 在跑的派发。board host 报的是它知道的派发 ID：board next 刚换到下一张、board host 还没取到新任务时，
        // 它报的还是上一张。同一个会话（同一个进程）里的任何一张在报，就算有进程
        let running: Vec<i64> =
            self.collect("SELECT id FROM dispatches WHERE host = ?1 AND state = 'running' ORDER BY id", [&name], |r| r.get(0))?;
        let mut alive: Vec<i64> = vec![];
        for id in &rep.running {
            if let Some(s) = self
                .tx
                .query_row("SELECT COALESCE(session, id) FROM dispatches WHERE id = ?1", [id], |r| r.get::<_, i64>(0))
                .optional()?
            {
                alive.push(s);
            }
        }
        for id in running {
            let d = self.drow(id)?;
            if rep.running.contains(&id) || alive.contains(&d.session) {
                if let Some(name) = &d.name {
                    match self.agent_beat(name, false) {
                        Ok(_) => {}
                        // 名字已经离线：扫描会按机器掉线结束这次派发
                        Err(e) if e.code == ErrorCode::NameOffline => {}
                        Err(e) => return Err(e),
                    }
                }
            } else {
                self.finish(&d, None, &[], Finish::Missing)?;
            }
        }
        self.read().host_work(&name, now)
    }

    /// 替换这台机器的「白板与目录」，服务端成为新设置的主版本。`--dir` 固定的白板要原样保留，
    /// 其他白板可以照常增删改
    fn host_set_dirs(&mut self, name: &str, raw: &[HostDir], by: &str) -> Result<()> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        for old in h.dirs.iter().filter(|d| d.fixed) {
            let kept = raw
                .iter()
                .find(|d| d.board.trim() == old.board)
                .is_some_and(|d| d.path.trim() == old.path);
            if !kept {
                return Err(BoardError::usage(format!(
                    "{} 用 --dir 固定，不能删除或修改；要在那台机器上改启动参数",
                    old.board
                )));
            }
        }
        let mut seen: Vec<String> = vec![];
        for d in raw {
            let board = d.board.trim().to_string();
            let path = d.path.trim();
            if !valid_board_ref(&board) {
                return Err(BoardError::usage(format!("白板要写成 <项目>/<白板>：{}", d.board)));
            }
            if path.is_empty() {
                return Err(BoardError::usage(format!("白板 {board} 还没有填目录")));
            }
            if seen.contains(&board) {
                return Err(BoardError::usage(format!("白板 {board} 写了两次")));
            }
            seen.push(board.clone());
            self.resolve(&board)?;
        }
        if same_dir_settings(&h.dirs, raw) {
            return Ok(());
        }
        let mut dirs = clean_dirs(raw, &h.dirs);
        for d in &mut dirs {
            d.fixed = h.dirs.iter().any(|old| old.fixed && old.board == d.board && old.path == d.path);
        }
        self.tx.execute(
            "UPDATE hosts SET dirs = ?2, dirs_rev = ?3, dirs_at = ?4, dirs_by = ?5 WHERE id = ?1",
            params![
                h.id,
                serde_json::to_string(&dirs).unwrap_or_else(|_| "[]".into()),
                h.dirs_rev + 1,
                self.now,
                by.trim().to_string()
            ],
        )?;
        self.touch_hosts();
        Ok(())
    }

    /// 界面上请求在一台机器的某个目录里克隆仓库；真正执行的是那台机器的 board host
    pub fn host_clone_request(&mut self, name: &str, board: &str, url: &str, by: &str) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let url = url.trim();
        if url.is_empty() {
            return Err(BoardError::usage("先填仓库地址"));
        }
        let mut dirs = h.dirs.clone();
        let d = dirs
            .iter_mut()
            .find(|d| d.board == board)
            .ok_or_else(|| BoardError::usage(format!("机器「{name}」没有白板 {board} 的目录")))?;
        if d.fixed {
            return Err(BoardError::usage(format!("{board} 用 --dir 固定，不能从界面上克隆")));
        }
        if d.status.as_deref() != Some("missing") {
            return Err(BoardError::usage(format!("{board} 的目录现在不需要克隆")));
        }
        if d.clone.as_ref().is_some_and(|c| c.state == "queued" || c.state == "running") {
            return Err(BoardError::usage(format!("{board} 已经有一个克隆在进行")));
        }
        d.clone = Some(HostClone {
            state: "queued".into(),
            url: url.into(),
            message: "等待 board host 开始克隆".into(),
            updated_at: self.now,
            by: by.trim().to_string(),
        });
        self.tx.execute(
            "UPDATE hosts SET dirs = ?2 WHERE id = ?1",
            params![h.id, serde_json::to_string(&dirs).unwrap_or_else(|_| "[]".into())],
        )?;
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// 界面上修改机器设置（任务下发需求 §四）
    pub fn host_update(&mut self, name: &str, p: &HostPatch) -> Result<HostInfo> {
        self.host_update_as(name, p, "人")
    }

    /// 同 [`Self::host_update`]；by 是界面上是谁改的（写进「白板与目录」的修改记录）
    pub fn host_update_as(&mut self, name: &str, p: &HostPatch, by: &str) -> Result<HostInfo> {
        if p.dirs.is_some() {
            self.host_set_dirs(name, p.dirs.as_deref().unwrap_or_default(), by)?;
        }
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if let Some(max) = p.max {
            if !(1..=HOST_MAX_LIMIT).contains(&max) {
                return Err(BoardError::usage(format!("同时跑几个要在 1 到 {HOST_MAX_LIMIT} 之间")));
            }
            self.tx.execute("UPDATE hosts SET max = ?2 WHERE id = ?1", params![h.id, max])?;
        }
        if let Some(n) = p.per_session {
            if !(1..=PER_SESSION_LIMIT).contains(&n) {
                return Err(BoardError::usage(format!("一个会话最多做几张要在 1 到 {PER_SESSION_LIMIT} 之间")));
            }
            self.tx.execute("UPDATE hosts SET per_session = ?2 WHERE id = ?1", params![h.id, n])?;
        }
        if let Some(rules) = &p.rules {
            if rules.len() > RULES_LIMIT {
                return Err(BoardError::usage(format!("领取规则最多 {RULES_LIMIT} 条")));
            }
            let known = self.read().host_labels(&h)?;
            let mut out: Vec<ClaimRule> = vec![];
            for r in rules {
                let r = r.clone().upgraded();
                if !RULE_OPS.contains(&r.op.as_str()) {
                    return Err(BoardError::usage(format!("领取规则的条件「{}」不对：只能是 {}", r.op, RULE_OPS.join("、"))));
                }
                if op_needs_labels(&r.op) && r.labels.is_empty() {
                    return Err(BoardError::usage("领取规则至少要选一个标签"));
                }
                // 新加或改过的规则：标签要在这台机器有目录的项目里有，agent 要在这台机器上找得到。
                // 原样保留的规则（标签被删了、agent 之后找不到了）不拦，界面上标出来
                let same = |x: &ClaimRule| {
                    let low = |v: &[String]| {
                        let mut v: Vec<String> = v.iter().map(|l| l.to_lowercase()).collect();
                        v.sort();
                        v
                    };
                    x.op == r.op && x.agent == r.agent && low(&x.labels) == low(&r.labels)
                };
                if !h.rules.iter().any(same) {
                    if let Some(l) = r.labels.iter().find(|l| !known.contains(&l.to_lowercase())) {
                        return Err(BoardError::usage(format!("这台机器有目录的项目里没有标签「{l}」")));
                    }
                    if !h.has_agent(&r.agent) {
                        return Err(BoardError::usage(format!("机器「{name}」上没有找到 agent「{}」，可选：{}", r.agent, found_list(&h))));
                    }
                }
                out.push(r);
            }
            let raw = serde_json::to_string(&out.iter().map(ClaimRule::stored).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
            self.tx.execute("UPDATE hosts SET rules = ?2 WHERE id = ?1", params![h.id, raw])?;
        }
        if let Some(f) = &p.fallback {
            let f = f.trim();
            if f.is_empty() {
                self.tx.execute("UPDATE hosts SET fallback = NULL WHERE id = ?1", [h.id])?;
            } else {
                if h.fallback.as_deref() != Some(f) && !h.has_agent(f) {
                    return Err(BoardError::usage(format!("机器「{name}」上没有找到 agent「{f}」，可选：{}", found_list(&h))));
                }
                self.tx.execute("UPDATE hosts SET fallback = ?2 WHERE id = ?1", params![h.id, f])?;
            }
        }
        if let Some(paused) = p.paused {
            self.tx.execute("UPDATE hosts SET paused = ?2 WHERE id = ?1", params![h.id, paused as i64])?;
            // 暂停接单时这一轮就结束；恢复接单后不补（PRD §9.1）
            if paused {
                self.tx.execute("UPDATE hosts SET round_at = NULL WHERE id = ?1", [h.id])?;
            }
        }
        if let Some(agent) = &p.agent {
            if !h.resolved().iter().any(|a| &a.id == agent && a.found) {
                return Err(BoardError::new(ErrorCode::AgentNotOnHost, format!("机器「{name}」上没有找到 agent「{agent}」")));
            }
            self.tx.execute("UPDATE hosts SET agent = ?2 WHERE id = ?1", params![h.id, agent])?;
        }
        if let Some(on) = p.cron_on {
            self.tx.execute("UPDATE hosts SET cron_on = ?2 WHERE id = ?1", params![h.id, on as i64])?;
        }
        if let Some(times) = &p.cron {
            // 每条是一个 cron 表达式（旧的 HH:MM 换成等价的「M H * * *」）；不对的拒绝，写明哪一段不对
            let mut v: Vec<String> = vec![];
            for t in times {
                let t = crate::cron::normalize(t).map_err(BoardError::usage)?;
                if !v.contains(&t) {
                    v.push(t);
                }
            }
            self.tx.execute("UPDATE hosts SET cron = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&v).unwrap_or_default()])?;
        }
        if let Some(e) = &p.env {
            let check = h.env_check(&e.agent, &e.env)?;
            let mut run = h.run.clone();
            run.entry(e.agent.clone()).or_default().env = Some(check.env);
            self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&run).unwrap_or_default()])?;
        }
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// 开始一轮领取：定时到点（board host 发起）或界面上「现在领一轮」。机器离线、暂停接单、或者一条领取规则也没有时
    /// 不开始；一轮还没结束时什么也不做（不重复开始）
    pub fn host_round(&mut self, name: &str) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if !h.online(self.now) {
            return Err(BoardError::usage(format!("机器「{name}」离线，不能领取")));
        }
        if h.paused {
            return Err(BoardError::usage(format!("机器「{name}」暂停接单，不能领取")));
        }
        if !h.has_rules() {
            return Err(BoardError::usage(format!("机器「{name}」还没有领取规则：先在「领取规则」里添加一条，才会自动领取卡片")));
        }
        if h.round_at.is_none() {
            self.tx.execute("UPDATE hosts SET round_at = ?2 WHERE id = ?1", params![h.id, self.now])?;
            self.touch_hosts();
        }
        self.read().host(name, self.now)
    }

    /// 移除机器（任务下发需求 §九）：取消它所有排队中的派发，停止正在跑的派发，删除机器设置；派发记录保留
    pub fn host_remove(&mut self, name: &str, by: &Actor, who: &str) -> Result<()> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let queued: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE host_id = ?1 AND state = 'queued'", [h.id], |r| r.get(0))?;
        for id in queued {
            let d = self.drow(id)?;
            self.close_dispatch(&d, "canceled", None)?;
            let text = format!("移除了机器 {}，派发取消（{}）", h.name, Self::dispatch_words(&d));
            self.track_dispatch(d.board, d.card, by, "dispatch", &text)?;
        }
        let running: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE host = ?1 AND state = 'running'", [name], |r| r.get(0))?;
        for id in running {
            self.request_stop(id, by, who)?;
        }
        self.tx.execute("DELETE FROM hosts WHERE id = ?1", [h.id])?;
        self.touch_hosts();
        Ok(())
    }

    /// 机器改名（桌面端「设置 → 本机」保存机器名时调用）：机器记录原地改名，id 不变，
    /// 所有设置（同时跑几个、默认 agent、领取规则、定时领取、暂停 …）和历史派发记录都保留，
    /// 派发记录里的机器名也跟着改（在跑的派发按机器名找）。
    /// - 新名字的规则和 [`check_host_name`] 相同；正被另一台**在线**机器使用时 `HOST_NAME_TAKEN`；
    ///   被一台**离线**机器占用时 `USAGE`（先在「机器」里移除它再改）
    /// - 旧名字在服务端没有记录（这台机器还没连上来过）时 `NO_SUCH_HOST`：没有要改的
    pub fn host_rename(&mut self, from: &str, to: &str) -> Result<HostInfo> {
        let to = to.trim();
        check_host_name(to).map_err(BoardError::usage)?;
        let h = self.host_row(from)?.ok_or_else(|| no_such_host(from))?;
        if h.name != to {
            if let Some(other) = self.host_row(to)? {
                return Err(if other.online(self.now) {
                    BoardError::new(
                        ErrorCode::HostNameTaken,
                        format!("机器名「{to}」正被在线的 board host 使用：先停掉那台机器上的 board host，或换一个名字"),
                    )
                    .with_detail(json!({ "host": to }))
                } else {
                    BoardError::usage(format!("机器名「{to}」已经有一台离线的机器：在「机器」里移除它之后再改"))
                });
            }
            self.tx.execute("UPDATE hosts SET name = ?2 WHERE id = ?1", params![h.id, to])?;
            self.tx.execute("UPDATE dispatches SET host = ?2 WHERE host_id = ?1", params![h.id, to])?;
            self.touch_hosts();
        }
        self.read().host(to, self.now)
    }

    /* ---------------- 派发 ---------------- */

    /// 手动派发（任务下发需求 §五）：校验通过后进入排队。agent 为空时用这台机器的默认 agent
    pub fn dispatch_create(&mut self, board: &BoardRef, card: i64, host: &str, agent: Option<&str>, by: &Actor) -> Result<Dispatch> {
        let flow = flow_of(self.tx, board.id)?;
        let col: Option<String> =
            self.tx.query_row("SELECT col FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![board.id, card], |r| r.get(0)).optional()?;
        let col = col.ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("卡片 #{card} 不存在")))?;
        if !flow.is_claim(&col) {
            let names: Vec<String> = flow.0.iter().filter(|s| s.claim).map(|s| format!("「{}」", s.name)).collect();
            return Err(BoardError::new(
                ErrorCode::NotInTodo,
                format!("#{card} 在「{}」，不在可领取的状态{}，不能派发", flow.name(&col), names.join("")),
            ));
        }
        if let Some(d) = self.read().card_dispatch(board, card)? {
            let what = if d.state == "queued" { "排队" } else { "在跑" };
            return Err(BoardError::new(
                ErrorCode::AlreadyDispatched,
                format!("#{card} 已经派发给 {} · {}，正在{what}：先取消或停止它", d.host, d.agent_label),
            )
            .with_detail(json!({ "dispatch": d.id })));
        }
        let h = self.host_row(host)?.ok_or_else(|| no_such_host(host))?;
        let all = h.resolved();
        let a = match agent.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => all.iter().find(|a| a.id == id && a.found).cloned().ok_or_else(|| {
                let missing = all.iter().find(|a| a.id == id);
                BoardError::new(
                    ErrorCode::AgentNotOnHost,
                    match missing {
                        Some(a) => match &a.env {
                            Some(e) => format!("机器「{host}」上在{}里没找到 agent「{id}」的命令", h.env_name(e)),
                            None => format!("机器「{host}」上没找到 agent「{id}」的命令"),
                        },
                        None => format!("机器「{host}」上没有 agent「{id}」，可选：{}", found_list(&h)),
                    },
                )
            })?,
            None => h
                .default_agent()
                .ok_or_else(|| BoardError::new(ErrorCode::AgentNotOnHost, format!("机器「{host}」上一个 agent 也没找到")))?,
        };
        let path = board.path();
        if !h.dirs.iter().any(|d| d.board == path) {
            return Err(BoardError::new(
                ErrorCode::BoardNotOnHost,
                format!("机器「{host}」没有白板 {path} 的目录：那台机器的 board host 启动时用 --dir {path}=<路径> 加上"),
            ));
        }
        let (agent_id, label) = (a.id.clone(), a.name.clone());
        self.insert_dispatch(board.id, card, &h, &agent_id, &label, "manual", by)
    }

    fn insert_dispatch(&mut self, b: i64, card: i64, h: &HostRow, agent: &str, label: &str, source: &str, by: &Actor) -> Result<Dispatch> {
        let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![b, card], |r| r.get(0))?;
        self.tx.execute(
            "INSERT INTO dispatches (board, card, title, host_id, host, agent, agent_label, source, by_kind, by_name, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'queued', ?11)",
            params![b, card, title, h.id, h.name, agent, label, source, by.kind.as_str(), by.name, self.now],
        )?;
        let id = self.tx.last_insert_rowid();
        let wait = if !h.online(self.now) {
            "，排队等机器上线"
        } else if h.paused {
            "，排队（机器暂停接单）"
        } else {
            "，排队"
        };
        self.track(b, card, by, "dispatch", &format!("派发到 {} · {}（{}）{wait}", h.name, label, source_words(source)))?;
        self.touch_hosts();
        self.read().dispatch(id)
    }

    /// 取消排队中的派发，或请求停止正在跑的派发（board host 结束 agent 进程后上报结束）。who 是给人看的称呼
    pub fn dispatch_cancel(&mut self, id: i64, by: &Actor, who: &str) -> Result<Dispatch> {
        let d = self.drow(id)?;
        match d.state.as_str() {
            "queued" => {
                self.close_dispatch(&d, "canceled", None)?;
                self.track_dispatch(d.board, d.card, by, "dispatch", &format!("取消了派发（{}）", Self::dispatch_words(&d)))?;
            }
            "running" => self.request_stop(id, by, who)?,
            _ => return Err(BoardError::usage(format!("派发 #{id} 已经结束"))),
        }
        self.read().dispatch(id)
    }

    /// 一张卡片排队或正在跑的派发；没有时 USAGE
    pub fn card_dispatch_id(&self, board: &BoardRef, card: i64) -> Result<i64> {
        self.read()
            .card_dispatch(board, card)?
            .map(|d| d.id)
            .ok_or_else(|| BoardError::usage(format!("#{card} 没有排队或正在跑的派发")))
    }

    /// 请求停止正在跑的派发：记下是谁，board host 收到后结束 agent 进程树再上报结束
    pub(crate) fn request_stop(&mut self, id: i64, by: &Actor, who: &str) -> Result<()> {
        let d = self.drow(id)?;
        if d.state != "running" || d.stop_by.is_some() {
            return Ok(());
        }
        self.tx.execute("UPDATE dispatches SET stop_by = ?2 WHERE id = ?1", params![id, who])?;
        self.track_dispatch(d.board, d.card, by, "dispatch", &format!("停止派发：通知 {} 结束 agent 进程（{}）", d.host, Self::dispatch_words(&d)))?;
        self.touch_hosts();
        Ok(())
    }

    /// 人把卡片拖出可领取状态：排队中的派发自动取消（规则引擎调用）
    pub(crate) fn cancel_queued_on_move(&mut self, b: i64, card: i64, by: &Actor) -> Result<()> {
        if let Some((id, _)) = self.queued_dispatch(b, card)? {
            let d = self.drow(id)?;
            self.close_dispatch(&d, "canceled", None)?;
            self.track(b, card, by, "dispatch", &format!("卡片移出了可领取状态，派发自动取消（{}）", Self::dispatch_words(&d)))?;
        }
        Ok(())
    }

    /// 卡片删除前：排队的派发取消，正在跑的请求停止（规则引擎调用）
    pub(crate) fn on_card_delete(&mut self, b: i64, card: i64, by: &Actor) -> Result<()> {
        let ids: Vec<(i64, String)> =
            self.collect("SELECT id, state FROM dispatches WHERE board = ?1 AND card = ?2 AND state != 'done'", params![b, card], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        for (id, state) in ids {
            if state == "queued" {
                let d = self.drow(id)?;
                self.close_dispatch(&d, "canceled", None)?;
            } else {
                self.tx.execute("UPDATE dispatches SET stop_by = COALESCE(stop_by, ?2) WHERE id = ?1", params![id, by.display()])?;
                self.touch_hosts();
            }
        }
        Ok(())
    }

    /// 校验 board host：机器存在，并且是这个进程在上报
    fn host_of(&self, name: &str, instance: &str) -> Result<HostRow> {
        let h = self.host_of_proc(name, instance)?.ok_or_else(|| no_such_host(name))?;
        if h.instance != instance {
            return Err(BoardError::new(ErrorCode::HostNameTaken, format!("机器名「{name}」已经由另一个 board host 使用")));
        }
        Ok(h)
    }

    fn running_on(&self, h: &HostRow) -> Result<i64> {
        Ok(self.tx.query_row("SELECT COUNT(*) FROM dispatches WHERE host = ?1 AND state = 'running'", [&h.name], |r| r.get(0))?)
    }

    /// 开始一次排队中的派发（board host 有空位时按创建先后调用）：机器在线、没有暂停、有空位才开始
    pub fn dispatch_begin(&mut self, host: &str, instance: &str, id: i64, token: Option<i64>) -> Result<Dispatch> {
        let h = self.host_of(host, instance)?;
        let d = self.drow(id)?;
        if d.state != "queued" || d.host_id != h.id {
            return Err(BoardError::usage(format!("派发 #{id} 不在机器「{host}」的排队里")));
        }
        if h.paused {
            return Err(BoardError::usage(format!("机器「{host}」暂停接单")));
        }
        if self.running_on(&h)? >= h.max {
            return Err(BoardError::usage(format!("机器「{host}」没有空位（同时跑 {}）", h.max)));
        }
        self.start(&h, d, token)
    }

    /// 生成名字、check、以它领取卡片。领不到时这次派发结束，结果「没领到」
    fn start(&mut self, h: &HostRow, d: DRow, token: Option<i64>) -> Result<Dispatch> {
        // 名字：<agent 标识>-<机器名>/<模型名>（dsh-win/deepseek-v4.1-flash）。模型名来自 board host 上报的
        // agent 数据（配置文件 [agents.<标识>] model，内置模板有默认值），没有时用 agent 标识。
        // 不带序号的名字空闲就直接用；同一台机器同一 agent、模型有多个并发派发时模型名后面加 -2、-3 …
        // （dsh-win/deepseek-v4.1-flash-2），只有在线的名字算占用，
        // 上一个会话离线后名字可以复用
        let model = h.agents.iter().find(|a| a.id == d.agent).map(|a| a.model.as_str()).unwrap_or_default();
        let gap = crate::agents::NAME_GAP_MS;
        let mut seq = 0u32;
        let name = loop {
            let n = dispatch_agent_name(&d.agent, &h.name, model, seq);
            let taken: bool = self.tx.query_row(
                "SELECT COUNT(*) > 0 FROM agents WHERE name = ?1 AND off_at IS NULL AND beat_at + ?3 > ?2",
                params![n, self.now, gap],
                |r| r.get(0),
            )?;
            if !taken {
                break n;
            }
            // 不带序号的被占用：依次试 -2、-3 …
            seq = if seq == 0 { 2 } else { seq + 1 };
        };
        // 用哪个启动环境：按开始这一刻的机器设置
        let env = h.agents.iter().find(|a| a.id == d.agent).and_then(|a| h.pick(a)).map(|c| c.env.clone());
        let env_name = env.as_deref().map(|e| h.env_name(e));
        self.tx.execute(
            "UPDATE dispatches SET name = ?2, env = ?3, env_name = ?4 WHERE id = ?1",
            params![d.id, name, env, env_name],
        )?;
        let d = DRow { name: Some(name.clone()), env_name: env_name.clone(), ..d };
        self.agent_check(&name, token)?;
        let board = self.board_of(d.board)?;
        let actor = Actor::agent(name.clone());
        self.dispatch_ok = true;
        let claimed = self.exec(&board, &actor, Cmd::Claim { id: Some(d.card) });
        self.dispatch_ok = false;
        match claimed {
            Ok(_) => {
                self.tx.execute("UPDATE dispatches SET state = 'running', started_at = ?2, session = ?1 WHERE id = ?1", params![d.id, self.now])?;
                self.touch_hosts();
                // 「mbp 上的 Claude Code（Git Bash）以 claude-mbp/opus-5.5 领取」；旧版 board host 没有启动环境：「mbp 上的 Claude Code 以 …」
                let who = match &env_name {
                    Some(e) => format!("{}（{e}）", d.agent_label),
                    None => format!("{} ", d.agent_label),
                };
                let text = format!("派发开始：{} 上的 {who}以 {name} 领取（{}）", h.name, source_words(&d.source));
                self.track(d.board, d.card, &actor, "dispatch", &text)?;
            }
            Err(e) if e.code == ErrorCode::Internal => return Err(e),
            Err(e) => {
                self.close_dispatch(&d, "unclaimed", None)?;
                let text = format!("派发没领到：{}（{}）", e.message, Self::dispatch_words(&d));
                self.track_dispatch(d.board, d.card, &Actor::system(), "dispatch", &text)?;
            }
        }
        self.read().dispatch(d.id)
    }

    /// 定时领取（任务下发需求 §六）：正在一轮领取、没有暂停、有空位时，在这台机器有目录的白板上按 --dir 的顺序轮流，
    /// 按领取规则挑下一张卡片和它的 agent。一条领取规则也没有时不领卡（这一轮直接结束，返回 None）。
    /// 规则没命中的卡片、也没有可领取的卡片时这一轮结束，返回 None
    pub fn dispatch_pull(&mut self, host: &str, instance: &str, token: Option<i64>) -> Result<Option<Dispatch>> {
        let h = self.host_of(host, instance)?;
        if h.round_at.is_none() || h.paused || self.running_on(&h)? >= h.max {
            return Ok(None);
        }
        let Some(rules) = h.claim_rules() else {
            return self.end_round(&h);
        };
        let n = h.dirs.len();
        for k in 0..n {
            let i = (h.rr.max(0) as usize + k) % n;
            let Ok(board) = self.resolve(&h.dirs[i].board) else { continue };
            let flow = flow_of(self.tx, board.id)?;
            // 这一轮里这台机器已经派发过的卡片不再领：派发异常退出/被停止后卡片 50 秒会回到可领取状态最上面，
            // 不跳的话同一个空位会反复领到它、白跑 agent。下一轮（round_at 更新后）可以正常再领。
            let round_at = h.round_at.unwrap_or(0);
            let skip: Vec<i64> = self.collect(
                "SELECT DISTINCT card FROM dispatches WHERE board = ?1 AND host_id = ?2 AND created_at >= ?3",
                params![board.id, h.id, round_at],
                |r| r.get(0),
            )?;
            // 规则会给它派一个这台机器上找得到的 agent 的卡片，按规则 1 的先后（带优先标签的在前）取第一张
            let found = h.resolved();
            let mut hit: Option<(i64, HostAgent)> = None;
            for (id, _) in self.claimable_order(board.id, &flow, &skip)? {
                let Some(want) = rules.route(&card_label_names(self.tx, board.id, id)?) else { continue };
                if let Some(a) = found.iter().find(|x| x.id == want) {
                    hit = Some((id, a.clone()));
                    break;
                }
            }
            let Some((card, agent)) = hit else { continue };
            self.tx.execute("UPDATE hosts SET rr = ?2 WHERE id = ?1", params![h.id, ((i + 1) % n) as i64])?;
            let d = self.insert_dispatch(board.id, card, &h, &agent.id, &agent.name, "cron", &Actor::system())?;
            let d = self.start(&h, self.drow(d.id)?, token)?;
            if d.state == "running" {
                return Ok(Some(d));
            }
        }
        self.end_round(&h)
    }

    fn end_round(&mut self, h: &HostRow) -> Result<Option<Dispatch>> {
        if h.round_at.is_some() {
            self.tx.execute("UPDATE hosts SET round_at = NULL WHERE id = ?1", [h.id])?;
            self.touch_hosts();
        }
        Ok(None)
    }

    /// 派发结束（任务下发需求 §七「结束」）：board host 在 agent 进程退出后上报退出码和日志最后 20 行
    pub fn dispatch_end(&mut self, id: i64, exit_code: Option<i64>, tail: &[String], how: EndHow) -> Result<Dispatch> {
        let mut d = self.drow(id)?;
        // 进程退出时 board host 可能还不知道会话已经换到下一张（board next）：结算这个会话正在跑的那一张
        if d.state == "done" {
            let cur: Option<i64> = self
                .tx
                .query_row(
                    "SELECT id FROM dispatches WHERE COALESCE(session, id) = ?1 AND state = 'running' ORDER BY id DESC LIMIT 1",
                    [d.session],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(cur) = cur {
                d = self.drow(cur)?;
            }
        }
        let id = d.id;
        if d.state == "running" {
            let f = match how {
                EndHow::Exit => Finish::Exit,
                EndHow::HostExit => Finish::HostExit,
            };
            self.finish(&d, exit_code, tail, f)?;
        }
        self.read().dispatch(id)
    }

    fn finish(&mut self, d: &DRow, exit_code: Option<i64>, tail: &[String], how: Finish) -> Result<()> {
        let Some(name) = d.name.clone() else {
            return self.close_dispatch(d, "crashed", exit_code);
        };
        let actor = Actor::agent(name.clone());
        let stop_by = match how {
            Finish::HostExit => Some(format!("board host（{}）", d.host)),
            _ => d.stop_by.clone(),
        };
        let row: Option<(String, Option<String>)> = self
            .tx
            .query_row("SELECT col, holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let board = self.board_of(d.board)?;
        let flow = flow_of(self.tx, d.board)?;
        let result = match &row {
            // 卡片仍由这个名字持有：发 final，沿连线移到「错误」节点（没有时去需要人处理的状态），评论原因。
            // 这几步尽力而为：规则拒绝（例如刚好被超时释放）不影响记下派发的结果
            Some((col, Some(holder))) if *holder == name => {
                soft(self.exec(&board, &actor, Cmd::Beat { id: d.card, done: true }))?;
                let still: Option<String> = self
                    .tx
                    .query_row("SELECT holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0))
                    .optional()?
                    .flatten();
                let error = flow.error_after(col);
                let human = flow.human_after(col);
                // 错误节点优先于需要人处理的状态：卡片离开可领取状态，配了定时领取也不会被下一轮重领
                let target = error.as_ref().or(human.as_ref());
                let still_held = still.as_deref() == Some(name.as_str());
                if still_held {
                    if let Some(to) = target {
                        soft(self.exec(&board, &actor, Cmd::Move { id: d.card, column: to.key.clone(), top: false, index: None, order: None, before: None, after: None }))?;
                    }
                }
                let mut text = match (&stop_by, how) {
                    (_, Finish::HostExit) => format!("board host（{}）退出，停止了这次派发", d.host),
                    (Some(who), _) => format!("{who} 停止了这次派发"),
                    (None, Finish::Missing) => {
                        "agent 异常退出（board host 上没有这次派发的 agent 进程：board host 重启过，或开始派发时连接中断）".to_string()
                    }
                    (None, _) => {
                        let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into());
                        let mut t = format!("agent 异常退出（退出码 {code}）");
                        if !tail.is_empty() {
                            t += &format!("\n\n日志最后 {} 行：\n\n```\n{}\n```", tail.len(), tail.join("\n").replace("```", "ˋˋˋ"));
                        }
                        t
                    }
                };
                // 工作流里从当前状态连不出去能接住异常退出的状态：评论写明卡片会回到哪里，别让人以为它悄悄消失了
                if still_held && target.is_none() {
                    let claimed_from: Option<String> = self
                        .tx
                        .query_row("SELECT claimed_from FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0))
                        .optional()?
                        .flatten();
                    let back = flow.back_to(claimed_from.as_deref()).name.clone();
                    // 状态存在、只是从当前状态连不出去：单独说明，免得人以为白板没配
                    // 一个错误节点 = 有锁定状态连过去，所以白板上有错误节点、而这里没找到，就是从当前状态到不了
                    let unreachable = |what: &str, ok: Option<&Status>, exists: bool| {
                        if ok.is_none() && exists {
                            format!("这块白板有{what}状态，但从「{}」连不出去；", flow.name(col))
                        } else {
                            String::new()
                        }
                    };
                    let no_error = unreachable("错误", error, flow.0.iter().any(|s| flow.is_error(&s.key)));
                    let no_human = unreachable("需要人处理", human, flow.0.iter().any(|s| flow.is_human(&s.key)));
                    text += &format!("\n\n没有能接住异常退出的状态：{no_error}{no_human}卡片 50 秒后会回到「{}」最上面。", back);
                }
                soft(self.exec(&board, &actor, Cmd::Comment { id: d.card, text }))?;
                if stop_by.is_some() { "stopped" } else { "crashed" }
            }
            _ if stop_by.is_some() => "stopped",
            Some((col, _)) if flow.is_review(col) => "review",
            _ => "stuck",
        };
        self.close_dispatch(d, result, exit_code)?;
        let took = d.started_at.map(|t| format!("，用时 {}", crate::engine::dur_words(self.now - t))).unwrap_or_default();
        let code = exit_code.map(|c| format!("，退出码 {c}")).unwrap_or_default();
        let text = format!("派发结束：{}（{}{took}{code}）", result_words(result), Self::dispatch_words(d));
        self.track_dispatch(d.board, d.card, &actor, "dispatch", &text)?;
        Ok(())
    }

    /// 追加日志，只保留最后 500 行（不算变更，不推送；界面打开日志时自己去取）
    pub fn dispatch_log_append(&mut self, id: i64, lines: &[String]) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        let log: Option<String> = self.tx.query_row("SELECT log FROM dispatches WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        let log = log.ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("派发 #{id} 不存在")))?;
        let mut all: Vec<&str> = if log.is_empty() { vec![] } else { log.split('\n').collect() };
        all.extend(lines.iter().map(|l| l.trim_end_matches(['\r', '\n'])));
        let keep = all.len().saturating_sub(LOG_KEEP);
        self.tx.execute("UPDATE dispatches SET log = ?2 WHERE id = ?1", params![id, all[keep..].join("\n")])?;
        Ok(())
    }

    /// 扫描（服务端每秒一次）：上报中断超过 50 秒的机器标为离线（这一轮领取不再继续）；名字已经离线的在跑派发结束，
    /// 结果「机器掉线」。卡片按现有的心跳超时规则释放（在 sweep_board 里，先于这里执行）
    pub(crate) fn sweep_hosts(&mut self) -> Result<usize> {
        let n = self.tx.execute(
            "UPDATE hosts SET off_at = beat_at + ?2, round_at = NULL WHERE off_at IS NULL AND beat_at + ?2 <= ?1",
            params![self.now, HOST_GAP_MS],
        )?;
        if n > 0 {
            self.touch_hosts();
        }
        let lost: Vec<i64> = self.collect(
            "SELECT d.id FROM dispatches d LEFT JOIN agents a ON a.name = d.name
             WHERE d.state = 'running' AND (a.name IS NULL OR a.off_at IS NOT NULL OR a.beat_at + ?2 <= ?1)",
            params![self.now, HOST_GAP_MS],
            |r| r.get(0),
        )?;
        for id in &lost {
            let d = self.drow(*id)?;
            self.close_dispatch(&d, "lost", None)?;
            let text = format!("派发结束：机器掉线，超过 {} 秒没有收到 {} 的上报（{}）", HOST_GAP_MS / 1000, d.host, Self::dispatch_words(&d));
            self.track_dispatch(d.board, d.card, &Actor::system(), "dispatch", &text)?;
        }
        Ok(n + lost.len())
    }
}

/// 派发是怎么结束的
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    /// agent 进程退出
    Exit,
    /// board host 退出前结束了 agent 进程
    HostExit,
    /// board host 重启后，本机没有这次派发的进程
    Missing,
}

/// 规则拒绝不算失败（存储故障照样报错）
fn soft<T>(r: Result<T>) -> Result<()> {
    match r {
        Err(e) if e.code == ErrorCode::Internal => Err(e),
        _ => Ok(()),
    }
}

fn found_list(h: &HostRow) -> String {
    let v: Vec<String> = h.resolved().into_iter().filter(|a| a.found).map(|a| a.id).collect();
    if v.is_empty() { "（一个也没找到）".into() } else { v.join("、") }
}

impl HostRow {
    /// 改 agent 的启动环境前校验（启动环境与桌面端需求 §五）：这台 board host 上报了启动环境、有这个 agent、
    /// 本机有这个环境、agent 支持它、在里面找得到命令。不满足时 USAGE，写明原因
    fn env_check(&self, agent: &str, env: &str) -> Result<EnvCheck> {
        let name = &self.name;
        if self.envs.is_empty() {
            return Err(BoardError::usage(format!("机器「{name}」上的 board host 是旧版，不上报启动环境：升级后再设置")));
        }
        let a = self
            .agents
            .iter()
            .find(|a| a.id == agent)
            .ok_or_else(|| BoardError::usage(format!("机器「{name}」上没有 agent「{agent}」")))?;
        if !self.envs.iter().any(|e| e.id == env) {
            return Err(BoardError::usage(format!("机器「{name}」上没有启动环境「{env}」")));
        }
        let c = a
            .envs
            .iter()
            .find(|c| c.env == env)
            .ok_or_else(|| BoardError::usage(format!("{} 不支持在{}里启动", a.name, self.env_name(env))))?;
        if !c.found {
            return Err(BoardError::usage(format!("机器「{name}」上在{}里没找到 {} 的命令", self.env_name(env), a.name)));
        }
        Ok(c.clone())
    }
}
