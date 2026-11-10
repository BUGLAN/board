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
//!   `<agent 标识>/<机器名>-<序号>`、check 这个名字、以它领取卡片。领不到时这次派发结束，结果「没领到」。
//!   之后 host 每次上报时替这些名字发 agent 级心跳，直到派发结束。
//! - **结束**（[`W::dispatch_end`]）：agent 退出后，卡片仍由这个名字持有就发 final、沿连线移到需要人处理的状态并评论
//!   （异常退出 / 已停止）；否则看卡片在哪：交审状态是「已交审」，其他是「做不下去」。
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
  seq      INTEGER NOT NULL DEFAULT 0,-- agent 名字的序号
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

/// agent 标识：小写字母、数字、`.`、`-`，以字母开头，最长 16 个字符（名字 `<标识>/<机器名>-<序号>` 不超过 48 个字符）
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostDir {
    /// `<项目>/<白板>`
    pub board: String,
    pub path: String,
}

/// 定时领取的一条领取规则：卡片的标签满足条件就由这个 agent 做。标签按名称匹配，不区分大小写
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimRule {
    /// `in` 带其中任一个 | `not_in` 一个都不带 | `equal` 标签正好是这几个（顺序无关）
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
pub const RULE_OPS: [&str; 3] = ["in", "not_in", "equal"];

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
    /// 同时跑几个（1–8）
    pub max: i64,
    /// 定时领取启动的会话一个最多做几张（1–8，默认 4）；旧版服务端没有这一项
    #[serde(default = "default_per_session")]
    pub per_session: i64,
    /// 定时领取的领取规则（有顺序）；没有规则时领所有可领取的卡片、用默认 agent
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
    "id, name, instance, os, version, agents, dirs, max, paused, agent, cron_on, cron, round_at, rr, first_at, beat_at, off_at, envs, run, per_session, rules, fallback";

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
    /// 解析后的领取规则；没有规则、也没设兜底时为 None（领所有卡片、用默认 agent）
    fn claim_rules(&self) -> Option<Rules> {
        if self.rules.is_empty() && self.fallback.is_none() {
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
                    let mut st = self.conn.prepare("SELECT id FROM cards WHERE board = ?1 AND col = ?2")?;
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
        Ok(self.tx.query_row("SELECT COUNT(*) FROM cards WHERE board = ?1 AND id = ?2", params![b, id], |r| r.get::<_, i64>(0))? > 0)
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
            .query_row("SELECT holder FROM cards WHERE board = ?1 AND id = ?2", params![d.board, d.card], |r| r.get(0))
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
        let mut list = self.claimable_order(d.board, &flow, &skip)?;
        // 有领取规则：只留按规则交给这个会话的 agent 的卡片；规则里的标签只表示谁来做，不算「共同标签」
        let rules = h.claim_rules();
        if let Some(r) = &rules {
            let mut keep = vec![];
            for c in list {
                if r.route(&card_label_names(self.tx, d.board, c.0)?) == Some(d.agent.as_str()) {
                    keep.push(c);
                }
            }
            list = keep;
        }
        let rule_labels = rules.as_ref().map(Rules::labels).unwrap_or_default();
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
            self.tx.query_row("SELECT col FROM cards WHERE board = ?1 AND id = ?2", params![d.board, d.card], |r| r.get(0)).optional()?;
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
        let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2", params![d.board, card], |r| r.get(0))?;
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

    /// 动态里的「mbp · Claude Code · claude/mbp-1，手动」；有启动环境时「mbp · pi · Git Bash · pi/mbp-1，手动」
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
        for d in &rep.dirs {
            if !d.board.contains('/') {
                return Err(BoardError::usage(format!("白板要写成 <项目>/<白板>：{}", d.board)));
            }
        }
        let now = self.now;
        let agents = serde_json::to_string(&rep.agents).unwrap_or_else(|_| "[]".into());
        let dirs = serde_json::to_string(&rep.dirs).unwrap_or_else(|_| "[]".into());
        let envs = serde_json::to_string(&rep.envs).unwrap_or_else(|_| "[]".into());
        match self.host_row(&rep.name)? {
            None => {
                if rep.done {
                    return Err(no_such_host(&rep.name));
                }
                self.tx.execute(
                    "INSERT INTO hosts (name, token, instance, os, version, agents, dirs, envs, first_at, beat_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?9, ?8, ?8)",
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
                    return self.read().host_work(&rep.name, now);
                }
                let differs = !online
                    || h.instance != rep.instance
                    || h.os != rep.os
                    || h.version != rep.version
                    || h.agents_raw != agents
                    || h.envs_raw != envs
                    || h.dirs_raw != dirs;
                self.tx.execute(
                    "UPDATE hosts SET token = ?2, instance = ?3, os = ?4, version = ?5, agents = ?6, dirs = ?7, beat_at = ?8, off_at = NULL, envs = ?9 WHERE id = ?1",
                    params![h.id, token, rep.instance, rep.os, rep.version, agents, dirs, now, envs],
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
            self.collect("SELECT id FROM dispatches WHERE host = ?1 AND state = 'running' ORDER BY id", [&rep.name], |r| r.get(0))?;
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
        self.read().host_work(&rep.name, now)
    }

    /// 界面上修改机器设置（任务下发需求 §四）
    pub fn host_update(&mut self, name: &str, p: &HostPatch) -> Result<HostInfo> {
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
                    return Err(BoardError::usage(format!("领取规则的条件「{}」不对：只能是 in、not_in、equal", r.op)));
                }
                if r.labels.is_empty() {
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

    /// 开始一轮领取：定时到点（board host 发起）或界面上「现在领一轮」。机器离线或暂停接单时不开始；
    /// 一轮还没结束时什么也不做（不重复开始）
    pub fn host_round(&mut self, name: &str) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if !h.online(self.now) {
            return Err(BoardError::usage(format!("机器「{name}」离线，不能领取")));
        }
        if h.paused {
            return Err(BoardError::usage(format!("机器「{name}」暂停接单，不能领取")));
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
            self.tx.query_row("SELECT col FROM cards WHERE board = ?1 AND id = ?2", params![board.id, card], |r| r.get(0)).optional()?;
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
        let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2", params![b, card], |r| r.get(0))?;
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
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
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
        let mut seq: i64 = self.tx.query_row("SELECT seq FROM hosts WHERE id = ?1", [h.id], |r| r.get(0))?;
        let gap = crate::agents::NAME_GAP_MS;
        let name = loop {
            seq += 1;
            let n = format!("{}/{}-{}", d.agent, h.name, seq);
            let taken: bool = self.tx.query_row(
                "SELECT COUNT(*) > 0 FROM agents WHERE name = ?1 AND off_at IS NULL AND beat_at + ?3 > ?2",
                params![n, self.now, gap],
                |r| r.get(0),
            )?;
            if !taken {
                break n;
            }
        };
        self.tx.execute("UPDATE hosts SET seq = ?2 WHERE id = ?1", params![h.id, seq])?;
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
                // 「mbp 上的 pi（Git Bash）以 pi/mbp-1 领取」；旧版 board host 没有启动环境：「mbp 上的 pi 以 …」
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
    /// 按 `board claim`（不带 ID）的规则领下一张，用默认 agent 开始。所有白板都没有可领取的卡片时这一轮结束，返回 None
    pub fn dispatch_pull(&mut self, host: &str, instance: &str, token: Option<i64>) -> Result<Option<Dispatch>> {
        let h = self.host_of(host, instance)?;
        if h.round_at.is_none() || h.paused || self.running_on(&h)? >= h.max {
            return Ok(None);
        }
        let Some(a) = h.default_agent() else {
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
            // 有领取规则：按规则 1 的先后（带优先标签的在前），取第一张有 agent 接的卡片；没有规则：规则 1、默认 agent
            let (card, agent) = match h.claim_rules() {
                None => match self.next_claimable(board.id, &flow, &skip)? {
                    Some((card, _)) => (card, a.clone()),
                    None => continue,
                },
                Some(rules) => {
                    let list = self.claimable_order(board.id, &flow, &skip)?;
                    let urgent = self.priority_cards(board.id)?;
                    let ordered = list.iter().filter(|(id, _)| urgent.contains(id)).chain(list.iter().filter(|(id, _)| !urgent.contains(id)));
                    let mut hit = None;
                    for (id, _) in ordered {
                        if let Some(agent) = rules.route(&card_label_names(self.tx, board.id, *id)?) {
                            hit = h.resolved().into_iter().find(|x| x.id == agent).map(|x| (*id, x));
                            break;
                        }
                    }
                    match hit {
                        Some(x) => x,
                        None => continue,
                    }
                }
            };
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
            .query_row("SELECT col, holder FROM cards WHERE board = ?1 AND id = ?2", params![d.board, d.card], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let board = self.board_of(d.board)?;
        let flow = flow_of(self.tx, d.board)?;
        let result = match &row {
            // 卡片仍由这个名字持有：发 final，沿连线移到需要人处理的状态，评论原因。
            // 这几步尽力而为：规则拒绝（例如刚好被超时释放）不影响记下派发的结果
            Some((col, Some(holder))) if *holder == name => {
                soft(self.exec(&board, &actor, Cmd::Beat { id: d.card, done: true }))?;
                let still: Option<String> = self
                    .tx
                    .query_row("SELECT holder FROM cards WHERE board = ?1 AND id = ?2", params![d.board, d.card], |r| r.get(0))
                    .optional()?
                    .flatten();
                let human = flow.human_after(col).map(|s| s.key.clone());
                let still_held = still.as_deref() == Some(name.as_str());
                if still_held {
                    if let Some(to) = &human {
                        soft(self.exec(&board, &actor, Cmd::Move { id: d.card, column: to.clone(), top: false, index: None, order: None, before: None, after: None }))?;
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
                // 工作流里没有需要人处理的状态：评论写明卡片会回到哪里，别让人以为它悄悄消失了
                if still_held && human.is_none() {
                    let claimed_from: Option<String> = self
                        .tx
                        .query_row("SELECT claimed_from FROM cards WHERE board = ?1 AND id = ?2", params![d.board, d.card], |r| r.get(0))
                        .optional()?
                        .flatten();
                    let back = flow.back_to(claimed_from.as_deref()).name.clone();
                    text += &format!(
                        "\n\n没有需要人处理的状态：这块白板从「{}」连出去的状态里没有需要人处理的状态，卡片 50 秒后会回到「{}」最上面。",
                        flow.name(col),
                        back
                    );
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
