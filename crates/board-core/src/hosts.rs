//! 任务下发（board host）：机器与派发（任务下发需求 §三–§十）。
//!
//! - **机器**：每台干活的机器上常驻一个 `board host`。它每 10 秒上报一次（[`W::host_beat`]）：机器名、系统、
//!   board 版本、本机 agent、白板与目录，以及本机正在跑的派发；连续 5 次没收到算离线。机器名正被另一个在线的
//!   board host 使用时 `HOST_NAME_TAKEN`；同一台机器上 board host 异常退出后重启，带上上一次的标识（resume）可以接回名字。
//! - **机器设置**（最多几个会话、暂停接单、默认 agent、定时领取、每个 agent 的启动环境）存在服务端，界面上修改，
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
//!   并评论（异常退出 / 已停止 / 超时）；否则看卡片在哪：交审状态是「已交审」，其他是「做不下去」。
//! - **兜底**：每台机器可以设「单次最长运行」和「没输出多久算卡住」（分钟，0 = 不限）。board host 到时间后按「人停止」的流程
//!   结束整个进程树，上报 `how: "timeout"` 和原因，服务端记结果「超时」、按异常退出的规则处理卡片。
//! - **占用**：派发排队时卡片沿连线进入领取时会进的锁定状态（「进行中」），没有持有者、显示排队（[`W::reserve`]）；
//!   开始时在那里原地领取，没开始就取消（或没领到）时回到原来的可领取状态最上面（[`W::unreserve`]）。
//!   别的 agent `board claim <ID>` 报 `DISPATCHED`，也不能移动它；人把它拖出那个状态时这次派发自动取消。
//! - **掉线**：机器掉线超过 50 秒，名字跟着离线，卡片按现有的心跳超时规则释放，在跑的派发结束（机器掉线），见 [`W::sweep_hosts`]。
//!
//! - **拆解**（`kind = split`，PRD §9.1）：派发「未整理」里的条目，让 agent 把它拆成几张卡、记回「未整理」（`board new --inbox --from`）。
//!   不领取、不移动这条，也不占用它（人照样能编辑、整理）；开始时只生成名字、check，host 替这个名字发心跳；
//!   结束时按这次派发期间建了几张「拆自」是它的卡记「已拆解」或「没拆出卡」，退出码非 0 记「异常退出」并评论，人停止、超时记「已停止」并评论。
//!   和普通派发一样占机器的一个会话位。
//!
//! 派发、取消、开始、结束都在卡片动态里追加一条，写明机器、agent、来源和结果。

use crate::engine::At;
use crate::error::{BoardError, ErrorCode, Result};
use crate::model::*;
use crate::store::{flow_of, R, W};
use crate::Cmd;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

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
  ask      TEXT,                      -- 等 board host 处理的请求（JSON：重新检查 / 重启 / 升级，见 HostAsk）；没有时为空
  ask_seq  INTEGER NOT NULL DEFAULT 0,-- 请求的序号（board host 处理完带着它回报）
  drain_at INTEGER,                   -- 排空开始的时间（暂停接单，等在跑的派发结束）；不在排空时为空
  up_pause INTEGER NOT NULL DEFAULT 0,-- 1 = 为升级暂停了接单：升级完成或失败后恢复
  rechecked_at INTEGER,               -- 最近一次「重新检查」做完的时间
  selftest TEXT,                      -- 自检（JSON，见 SelfTest）：等 board host 做的请求和最近一次的结果；没做过时为空
  skills   TEXT,                      -- skills 同步状态（JSON，见 SkillState）；旧版 board host 不上报，为空
  mcp      TEXT,                      -- MCP 服务器同步状态（JSON，见 McpState）；旧版 board host 不上报，为空
  providers TEXT,                     -- 模型供应商写进各 agent 配置文件的状态（JSON，见 ProviderState）；旧版 board host 不上报，为空
  skill_import TEXT                   -- 从这台机器导入 skills（JSON，见 SkillImport）：本机的 skill、等 board host 做的请求、最近一次的结果
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
        ("hosts", "ask", "TEXT"),
        ("hosts", "ask_seq", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "drain_at", "INTEGER"),
        ("hosts", "up_pause", "INTEGER NOT NULL DEFAULT 0"),
        ("hosts", "rechecked_at", "INTEGER"),
        ("dispatches", "model", "TEXT"),
        ("dispatches", "effort", "TEXT"),
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
    ] {
        if !has(table, col)? {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"))?;
        }
    }
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
/// 「复制设置到…」能选的项：标识和说法
pub const COPY_ITEMS: [(&str, &str); 5] = [
    ("limits", "最多几个会话和一个会话最多做几张"),
    ("cron", "定时领取"),
    ("rules", "领取规则和兜底"),
    ("agent", "默认 agent"),
    ("run", "每个 agent 的启动环境、模型、强度"),
];
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

/// 没有机器（没跑 board host、白板不在这台机器）时的 agent 名字：`<agent 标识>/<模型名>`，例如 `pi/pi`、`dsh/deepseek-v4.1-flash`。
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
    /// 机器设置里能选的模型和思考强度（board host 按模板上报，PRD §9.1）；旧版 board host 没有这一项：界面上置灰、要升级
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opts: Option<AgentOpts>,
    /// 服务端发出去时才有：机器设置里选的模型；没选是 agent 自己的默认（不传参数）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_model: Option<String>,
    /// 服务端发出去时才有：机器设置里选的思考强度；没选是 agent 自己的默认
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_effort: Option<String>,
    /// 服务端发出去时才有：机器设置里选的启动环境（可能是从别的机器复制来、这台机器上没有的：那时 `env` 是实际用的）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_env: Option<String>,
    /// 服务端发出去时才有：机器设置里选的模型供应商。没选 = 跟随共用配置；`@local` = 本机自己管
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_provider: Option<String>,
    /// 共用配置里有这个标识的 agent 模板，但这台机器的配置文件里有同标识的 `[agents.*]`：用的是本机的那份（board host 上报）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local_override: bool,
}

/// 一个 agent 能在机器设置里选什么（模板决定怎么传参）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentOpts {
    /// 能选模型（模板有传模型的参数）
    #[serde(default)]
    pub model: bool,
    /// 下拉里列的常用模型（也可以手填）
    #[serde(default)]
    pub models: Vec<String>,
    /// 支持的思考强度档位，按从低到高；空表示不能选强度
    #[serde(default)]
    pub efforts: Vec<String>,
    /// 这个 agent 认哪些模型供应商接口（`anthropic`、`openai-chat`、`openai-responses`）：
    /// 共用配置里「模型供应商」的下拉据此过滤；旧版 board host 没有这一项
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocols: Vec<String>,
}

/// 模型名：字母或数字开头，字母、数字和 `. _ : / @ + -`，最长 64 个字符（作为一个参数传给 agent，也是派发名字的第二段）
pub fn check_model(m: &str) -> std::result::Result<(), String> {
    let ok = !m.is_empty()
        && m.len() <= 64
        && m.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && m.chars().all(|c| c.is_ascii_alphanumeric() || "._:/@+-".contains(c));
    if ok { Ok(()) } else { Err(format!("模型名「{m}」不对：字母或数字开头，只能用字母、数字和 . _ : / @ + -，最长 64 个字符")) }
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
    /// 模型；没有时不传（agent 自己的默认）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 思考强度；没有时不传
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// 模型供应商：没有 = 跟随共用配置；`@local` = 本机自己管（不改配置文件）；其余是供应商名称
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
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
    /// 服务端发出去时才有：这块白板的环境检查命令（按这台机器的系统选好）；白板没写检查命令时为空。不存进数据库
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    /// board host 最近一次跑环境检查的结果；旧版 board host 不上报
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<DirReady>,
}

/// 一台机器在一块白板的目录里跑环境检查的结果（PRD §9.1「环境检查」）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirReady {
    /// 跑的是哪条命令：白板的检查命令改了之后，旧结果不算
    pub cmd: String,
    /// 退出码为 0
    pub ok: bool,
    /// 退出码；超时被结束、没能运行时为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
    /// 输出（标准输出和标准错误）的最后 20 行
    #[serde(default)]
    pub tail: Vec<String>,
    /// 跑完的时间
    pub at: i64,
}

/// 环境检查保留的输出行数
pub const READY_TAIL: usize = 20;
/// 环境检查最长跑多久，超过就结束它、算没通过
pub const READY_LIMIT_MS: i64 = 120_000;

/// 登录 shell 自己打的启动、退出提示，不是命令的输出：白板的环境检查、派发的 agent 都在 `$SHELL -l -i -c` 里跑，
/// 没有终端（CI、桌面端、`setsid`）时 bash 会打作业控制提示，退出时再打一行 `logout`，发行版的
/// `/etc/bash.bashrc` 还会打几句 sudo 提示。上报和判断结果之前去掉这些行，否则失败原因会显示成 `logout`。
/// 只认这几种固定形状：`zsh:1: command not found: cargo` 这类真正的错误要留着（它就是派发被拒的原因）
pub fn shell_noise(line: &str) -> bool {
    let l = line.trim();
    l == "logout"
        || l.starts_with("bash: cannot set terminal process group")
        || l.starts_with("bash: no job control")
        // stty 在没有终端时对 stdin 报的错（`-i` 的交互式 shell 会调它）
        || (l.starts_with("stty: ") && l.contains("Inappropriate ioctl"))
        // Ubuntu/Debian 的 /etc/bash.bashrc：没有 ~/.sudo_as_admin_successful 时打的 sudo 提示
        || (l.starts_with("To run a command as administrator") && l.ends_with("use \"sudo <command>\"."))
        || l == "See \"man sudo_root\" for details."
}

/// 一台机器上一块白板能不能接卡（环境检查）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// 白板没写检查命令：照常接卡
    NoCheck,
    Passed,
    /// 没通过：输出的最后一行（没有输出时写退出码）
    Failed(String),
    /// 还没检查，或者命令改了之后还没跑完；旧版 board host 一直是这样
    Pending,
}

impl Readiness {
    /// 接这块白板的卡
    pub fn ok(&self) -> bool {
        matches!(self, Readiness::NoCheck | Readiness::Passed)
    }
}

/// 按检查命令和 board host 上报的结果判断（cmd 为空表示白板不检查）
pub fn readiness(cmd: Option<&str>, ready: Option<&DirReady>) -> Readiness {
    let Some(cmd) = cmd else { return Readiness::NoCheck };
    match ready {
        Some(r) if r.cmd == cmd => {
            if r.ok {
                Readiness::Passed
            } else {
                let last = r.tail.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty() && !shell_noise(l));
                Readiness::Failed(match (last, r.code) {
                    (Some(l), _) => l.to_string(),
                    (None, Some(c)) => format!("退出码 {c}"),
                    (None, None) => "没有输出".into(),
                })
            }
        }
        _ => Readiness::Pending,
    }
}

/// 上报的检查结果：去掉登录 shell 自己的提示，最多留最后 20 行，每行最长 500 个字符
fn clean_ready(r: &DirReady) -> DirReady {
    let mut tail: Vec<&String> = r.tail.iter().filter(|l| !shell_noise(l)).collect();
    let skip = tail.len().saturating_sub(READY_TAIL);
    tail.drain(..skip);
    DirReady {
        cmd: r.cmd.clone(),
        ok: r.ok,
        code: r.code,
        tail: tail.iter().map(|l| l.chars().take(500).collect()).collect(),
        at: r.at,
    }
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
                check: None,
                ready: d.ready.as_ref().map(clean_ready),
            };
            if let Some(prev) = old.iter().find(|x| x.board == board && x.path == path) {
                out.status = prev.status.clone();
                out.origin = prev.origin.clone();
                out.branch = prev.branch.clone();
                out.checked_at = prev.checked_at;
                out.clone = prev.clone.clone();
                out.ready = prev.ready.clone();
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
        // 环境检查的结果：旧版 board host 不上报，保留原值
        if let Some(ready) = &r.ready {
            s.ready = Some(clean_ready(ready));
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
    /// 最多几个会话：同一时间最多开几个 agent 进程（1–8）
    pub max: i64,
    /// 定时领取启动的会话一个最多做几张（1–8，默认 4）；旧版服务端没有这一项
    #[serde(default = "default_per_session")]
    pub per_session: i64,
    /// 单次最长运行（分钟，0 = 不限，默认 180）；旧版服务端没有这一项
    #[serde(default = "default_max_run")]
    pub max_run: i64,
    /// 多久没有输出算卡住（分钟，0 = 不限，默认 20）；旧版服务端没有这一项
    #[serde(default = "default_idle_run")]
    pub idle_run: i64,
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
    /// 设置的默认 agent 在这台机器上找不到时是它（例如从别的机器复制来的）：不生效，用的是 `agent`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_set: Option<String>,
    /// 定时领取开关和 cron 表达式（5 段「分 时 日 月 周」，机器本地时区）
    pub cron_on: bool,
    pub cron: Vec<String>,
    /// 正在进行一轮领取（定时到点或「现在领一轮」）
    pub round: bool,
    pub online: bool,
    pub first_at: i64,
    pub beat_at: i64,
    pub off_at: Option<i64>,
    /// 等 board host 处理的请求（重新检查、重启、升级）；旧版服务端没有这一项
    #[serde(default)]
    pub ask: Option<HostAsk>,
    /// 正在排空：暂停接单，等在跑的派发结束（都结束了就是「已排空」）
    #[serde(default)]
    pub drain: bool,
    /// 最近一次「重新检查」做完的时间
    #[serde(default)]
    pub rechecked_at: Option<i64>,
    /// 最近的操作记录（新的在前）
    #[serde(default)]
    pub ops: Vec<HostOp>,
    /// 这台机器最近一次上报用的令牌；令牌已经吊销、还没上报过时为空
    #[serde(default)]
    pub token: Option<HostToken>,
    /// 自检：等 board host 做的请求、最近一次的结果；旧版服务端没有这一项
    #[serde(default)]
    pub selftest: SelfTest,
    /// skills 共用库同步到这台机器各个 agent 目录的状态；旧版 board host 不同步，为空
    #[serde(default)]
    pub skills: Option<Vec<crate::skills::SkillState>>,
    /// MCP 服务器同步到这台机器各个 agent 的状态；旧版 board host 不同步，为空
    #[serde(default)]
    pub mcp: Option<Vec<crate::mcp::McpState>>,
    /// 模型供应商写进这台机器各个 agent 配置文件的状态；旧版 board host 不同步，为空
    #[serde(default)]
    pub providers: Option<Vec<crate::providers::ProviderState>>,
    /// 从这台机器导入 skills：本机有哪些 skill、等 board host 做的请求、最近一次的结果；旧版服务端没有这一项
    #[serde(default)]
    pub skill_import: SkillImport,
}

/// 从一台机器导入 skills（PRD §10.12「共用配置」）：board host 上报本机各 agent skill 目录里、不是同步装的 skill；
/// 人在界面上勾了要导入的，board host 把这几个目录传进共用库（和 `board skills push` 一样），再回报结果
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillImport {
    /// 这台机器的 board host 会上报、会导入；旧版不会，界面上写要升级
    #[serde(default)]
    pub able: bool,
    #[serde(default)]
    pub local: Vec<crate::skills::LocalSkill>,
    /// 等 board host 做（或正在做）的导入；做完清掉
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<SkillImportAsk>,
    /// 最近一次导入的结果
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<SkillImportResult>,
}

/// 要导入的一个：名称和它所在的 skill 目录（几个目录里有同名不同内容时靠它分）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillImportPick {
    pub name: String,
    pub dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillImportAsk {
    /// 序号：board host 做完带着它回报
    pub id: i64,
    pub picks: Vec<SkillImportPick>,
    pub by: String,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillImportFail {
    pub name: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillImportResult {
    pub id: i64,
    pub at: i64,
    pub by: String,
    /// 传进库里的
    #[serde(default)]
    pub ok: Vec<String>,
    #[serde(default)]
    pub failed: Vec<SkillImportFail>,
}

/// 导入请求多久没做完算作废：之后可以重新请求
pub const SKILL_IMPORT_STALE_MS: i64 = 5 * 60_000;

/// 自检（PRD §10.12「自检」）：人在机器详情点「自检」（整台机器，或「本机 agent」的某一行），board host 在后台检查
/// agent 能不能干活、每块白板的目录能不能推送，服务端补上 board 版本是否落后。结果一直保留到下一次自检
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfTest {
    /// 这台机器的 board host 会做自检（上报里带 `selftest: true`）；旧版不会，界面上置灰、要升级
    #[serde(default)]
    pub able: bool,
    /// 等 board host 做（或正在做）的自检；做完清掉
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<SelfTestAsk>,
    /// 最近的结果：agent（按本机 agent 的顺序）、git（按白板）、版本
    #[serde(default)]
    pub items: Vec<SelfTestItem>,
}

/// 一次自检请求
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfTestAsk {
    /// 序号：board host 做完带着它回报
    pub id: i64,
    /// 只检查这个 agent（「本机 agent」那一行的按钮）；空是整台机器
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub by: String,
    pub at: i64,
}

/// 自检的一项
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfTestItem {
    /// agent | git | version
    pub kind: String,
    /// agent 标识、白板路径；版本这一项是 board
    pub key: String,
    pub ok: bool,
    /// 没通过的原因
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// 补充：agent 回答的前几行、git 输出的最后几行
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 检查的时间（服务端收到结果的时间）
    #[serde(default)]
    pub at: i64,
}

/// 自检请求多久没做完算作废（agent 一项最长 3 分钟，git 每块白板最长 2 分钟）：之后可以重新请求
pub const SELFTEST_STALE_MS: i64 = 15 * 60 * 1000;

fn default_per_session() -> i64 {
    4
}

/// 界面上对一台机器发的、等 board host 处理的请求（远程管理，PRD §9.1「远程管理」）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostAsk {
    /// 序号：board host 处理完带着它回报（`HostReport.asked`）
    pub id: i64,
    /// recheck 重新检查 | restart 重启 board host | upgrade 升级
    pub kind: String,
    /// 谁请求的
    pub by: String,
    pub at: i64,
    /// 升级：目标版本（服务端的版本）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// 升级：不等在跑的派发结束（它们已经按「人停止」结束）
    #[serde(default)]
    pub now: bool,
}

/// 机器的一条操作记录（谁、什么时间、做了什么）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostOp {
    pub at: i64,
    pub by: String,
    pub text: String,
}

/// 机器最近一次上报用的访问令牌
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostToken {
    pub id: i64,
    pub name: String,
    /// 令牌开头几位
    pub hint: String,
    pub last_used_at: Option<i64>,
    /// 同一个令牌还被这些在线的机器使用（这时移除机器不能顺带吊销它）
    #[serde(default)]
    pub shared: Vec<String>,
}

/// 远程管理的请求种类
pub const ASK_KINDS: [&str; 5] = ["recheck", "restart", "upgrade", "drain", "cancel"];
/// 机器详情「最近」里带几条操作记录
pub const OPS_KEEP: i64 = 30;

/// 版本号 a 比 b 旧（x.y.z 逐段按数字比；认不出的段当 0）
pub fn version_lt(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.trim().trim_start_matches('v').split(['.', '-', '+']).take(3).map(|x| x.parse().unwrap_or(0)).collect() };
    let (mut x, mut y) = (parts(a), parts(b));
    x.resize(3, 0);
    y.resize(3, 0);
    x < y
}

fn default_max_run() -> i64 {
    MAX_RUN_DEFAULT
}

fn default_idle_run() -> i64 {
    IDLE_RUN_DEFAULT
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
    /// 处理完（或开始处理）的请求序号（`HostAsk.id`）：服务端收到后清掉这个请求
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<i64>,
    /// 升级（或重启）没成：原因，记进操作记录；为升级暂停的接单随之恢复
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_error: Option<String>,
    /// 会做自检（PRD §10.12「自检」）；旧版 board host 不带
    #[serde(default)]
    pub selftest: bool,
    /// skills 同步状态（PRD §10.12「共用配置」）；旧版 board host 不带，带了才更新
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<crate::skills::SkillState>>,
    /// MCP 服务器同步状态；旧版 board host 不带，带了才更新
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<Vec<crate::mcp::McpState>>,
    /// 模型供应商写进各 agent 配置文件的状态；旧版 board host 不带，带了才更新
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<Vec<crate::providers::ProviderState>>,
    /// 本机各 agent skill 目录里、不是同步装的 skill（「从机器导入」用）；旧版 board host 不带，带了就算会导入
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_skills: Option<Vec<crate::skills::LocalSkill>>,
}

/// 发给一台 board host 的任务：设置、在跑的派发（含停止请求）、排队的派发（按创建先后）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostWork {
    pub host: HostInfo,
    pub running: Vec<Dispatch>,
    pub queued: Vec<Dispatch>,
    pub now: i64,
    /// skills 共用库（不带内容）：board host 按它同步本机；服务端在前面补上随它版本的 board-skills。旧版服务端没有这一项
    #[serde(default)]
    pub skills: Option<Vec<crate::skills::SkillInfo>>,
    /// MCP 服务器共用配置的索引（名称、版本）：变了 board host 用令牌接口取完整内容。旧版服务端没有这一项
    #[serde(default)]
    pub mcp: Option<Vec<crate::mcp::McpIndex>>,
    /// 模型供应商共用配置：索引（取完整内容用）和每个 agent 用哪个。旧版服务端没有这一项
    #[serde(default)]
    pub providers: Option<crate::providers::ProviderWork>,
    /// 共用配置里的 agent 模板（全部内容）：board host 合进本机 agent；旧版服务端没有
    #[serde(default)]
    pub templates: Option<Vec<crate::templates::AgentTemplate>>,
}

/// 界面上修改机器设置
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPatch {
    #[serde(default)]
    pub max: Option<i64>,
    #[serde(default)]
    pub per_session: Option<i64>,
    /// 单次最长运行（分钟，0 = 不限）
    #[serde(default)]
    pub max_run: Option<i64>,
    /// 多久没有输出算卡住（分钟，0 = 不限）
    #[serde(default)]
    pub idle_run: Option<i64>,
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
    /// 一个 agent 的模型、思考强度（各自 None 表示不改，空字符串表示回到 agent 自己的默认）
    #[serde(default)]
    pub run: Option<RunPatch>,
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

/// 改一个 agent 的模型、思考强度
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunPatch {
    pub agent: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// 模型供应商：空字符串 = 跟随共用配置，`@local` = 本机自己管，其余是供应商名称
    #[serde(default)]
    pub provider: Option<String>,
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
    /// manual 手动 | cron 定时领取 | schedule 定时建卡（[`SOURCE_SCHEDULE`]）
    pub source: String,
    /// 发起人
    pub by: Actor,
    /// queued 排队 | running 在跑 | done 结束
    pub state: String,
    /// review 已交审 | stuck 做不下去 | crashed 异常退出 | stopped 已停止 | timeout 超时 | canceled 已取消 | lost 机器掉线 | unclaimed 没领到
    pub result: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
    /// 有人请求停止（还在跑时）：谁
    pub stop_by: Option<String>,
    /// 持有者最近一次心跳（在跑时）
    pub beat_at: Option<i64>,
    /// 同一个 agent 进程做的几张卡共用的会话标识：第一张的派发 ID（旧数据、还没开始时就是自己的 ID）。
    /// 排队中、`session` 不是自己的派发是开会话时排进去的，由这个会话的 `board next` 开始
    #[serde(default)]
    pub session: i64,
    /// 这个会话的第几张（从 1 开始），和这个会话一共排了几张（开会话时排好的，取消了的也算）；不属于会话时是 1、1
    #[serde(default = "one")]
    pub seq: i64,
    #[serde(default = "one")]
    pub total: i64,
    /// 开始时按机器设置传给 agent 的模型、思考强度；没设、旧版时为空
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// 空 = 做这张卡（领取它）；`split` = 拆解「未整理」里的这条（不领取、不移动，见 [`KIND_SPLIT`]）。旧服务端没有这个字段
    #[serde(default)]
    pub kind: String,
}

/// 派发的类型：拆解「未整理」里的条目（PRD §9.1）
pub const KIND_SPLIT: &str = "split";

/// 派发的来源：定时建卡建出卡后按这一条的设置派给指定的机器（PRD §4「定时建卡」）。
/// 和手动派发一样排队、占用卡片，只做这一张；动态里写「定时建卡」
pub const SOURCE_SCHEDULE: &str = "schedule";

fn one() -> i64 {
    1
}

impl Dispatch {
    /// 开会话时排进去、还在等这个会话的 `board next` 开始的派发。
    /// 服务端只按 `state` 筛排队中的派发，界面拿的是 `session` 和 `id`，所以只在测试里用它
    #[cfg(test)]
    pub fn in_session_queue(&self) -> bool {
        self.state == "queued" && self.session != self.id
    }
}

/// 一个会话（界面「会话」弹窗）：会话标识是第一张的派发 ID
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    pub session: i64,
    /// 这个会话的派发，按先后
    pub parts: Vec<SessionPart>,
    /// 事件的版本（meta.runs）：下次带上 `since` 只取之后新增、改过的事件
    #[serde(default)]
    pub runs: i64,
}

/// 会话里的一张卡：派发记录、它的输出（最后 500 行）和结构化事件（runlog.rs）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPart {
    #[serde(flatten)]
    pub dispatch: Dispatch,
    pub log: Vec<String>,
    /// agent 的输出格式：`claude`、`codex`、`pi`、`dsh`；空字符串 = 这个 agent 只有原始输出；没有 = board host 没报（旧版，或还没开始）
    #[serde(default)]
    pub fmt: Option<String>,
    /// 事件（按先后）；增量取（`since`）时只有那之后新增、改过的
    #[serde(default)]
    pub events: Vec<crate::runlog::RunEvent>,
    /// 留着的事件里最早的序号：增量取时界面丢掉比它早的（被裁掉了）。没有事件时为 0
    #[serde(default)]
    pub events_from: i64,
    /// 删掉了几条（超过 2000 条删最早的；结束 30 天后只留结果和最后 50 条）
    #[serde(default)]
    pub events_dropped: i64,
}

/// 结果的中文
pub fn result_words(r: &str) -> &'static str {
    match r {
        "review" => "已交审",
        "stuck" => "做不下去",
        "crashed" => "异常退出",
        "stopped" => "已停止",
        "timeout" => "超时",
        "canceled" => "已取消",
        "lost" => "机器掉线",
        "unclaimed" => "没领到",
        "split" => "已拆解",
        "nosplit" => "没拆出卡",
        _ => "已结束",
    }
}

/// 动态里的「，模型 sonnet，强度 high」；都没设时为空
fn opts_words(model: &Option<String>, effort: &Option<String>) -> String {
    let mut s = String::new();
    if let Some(m) = model {
        s += &format!("，模型 {m}");
    }
    if let Some(e) = effort {
        s += &format!("，强度 {e}");
    }
    s
}

fn source_words(s: &str) -> &'static str {
    match s {
        "cron" => "定时",
        SOURCE_SCHEDULE => "定时建卡",
        _ => "手动",
    }
}

/// 派发怎么结束（host 上报）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndHow {
    /// agent 进程退出（任何退出码；人停止时也是这样上报）
    Exit,
    /// board host 退出前结束了 agent 进程
    HostExit,
    /// board host 按机器的兜底设置结束了 agent 进程（运行超时 / 长时间没有输出），原因在 `why` 里
    Timeout,
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
    max_run: i64,
    idle_run: i64,
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
    ask: Option<HostAsk>,
    ask_seq: i64,
    drain_at: Option<i64>,
    up_pause: bool,
    rechecked_at: Option<i64>,
    token: Option<i64>,
    selftest: SelfTest,
    skills_raw: Option<String>,
    mcp_raw: Option<String>,
    providers_raw: Option<String>,
    skill_import: SkillImport,
    agents_raw: String,
    envs_raw: String,
    dirs_raw: String,
}

const HOST_COLS: &str =
    "id, name, instance, os, version, agents, dirs, max, paused, agent, cron_on, cron, round_at, rr, first_at, beat_at, off_at, envs, run, per_session, rules, fallback, dirs_rev, dirs_at, dirs_by, ask, ask_seq, drain_at, up_pause, rechecked_at, token, max_run, idle_run, selftest, skills, mcp, providers, skill_import";

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
        max_run: r.get(31)?,
        idle_run: r.get(32)?,
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
        ask: r.get::<_, Option<String>>(25)?.and_then(|j| serde_json::from_str(&j).ok()),
        ask_seq: r.get(26)?,
        drain_at: r.get(27)?,
        up_pause: r.get::<_, i64>(28)? != 0,
        rechecked_at: r.get(29)?,
        token: r.get(30)?,
        selftest: r.get::<_, Option<String>>(33)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default(),
        skills_raw: r.get(34)?,
        mcp_raw: r.get(35)?,
        providers_raw: r.get(36)?,
        skill_import: r.get::<_, Option<String>>(37)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default(),
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
                let run = self.run.get(&a.id);
                out.run_model = run.and_then(|r| r.model.clone());
                out.run_effort = run.and_then(|r| r.effort.clone());
                out.run_env = run.and_then(|r| r.env.clone());
                out.run_provider = run.and_then(|r| r.provider.clone());
                out
            })
            .collect()
    }
    /// 生成 agent 名字时第二段（模型名）用什么，见 [`model_of`]
    fn model_for(&self, agent: &str) -> (String, bool) {
        model_of(&self.agents, &self.run, agent)
    }

    /// 开始派发时传给这个 agent 的模型、思考强度：机器设置里选了、board host 上报了这个 agent 能选（新版）才传
    fn run_opts(&self, agent: &str) -> (Option<String>, Option<String>) {
        let Some(opts) = self.agents.iter().find(|a| a.id == agent).and_then(|a| a.opts.as_ref()) else { return (None, None) };
        let run = self.run.get(agent);
        let model = run.and_then(|r| r.model.clone()).filter(|_| opts.model);
        let effort = run.and_then(|r| r.effort.clone()).filter(|e| opts.efforts.contains(e));
        (model, effort)
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
            max_run: self.max_run,
            idle_run: self.idle_run,
            rules: self.rules.iter().map(|x| ClaimRule { active: self.has_agent(&x.agent), ..x.clone() }).collect(),
            fallback: self.fallback.clone(),
            fallback_active: self.fallback.as_deref().is_some_and(|a| self.has_agent(a)),
            unmatched: 0,
            paused: self.paused,
            agent: self.default_agent().map(|a| a.id.clone()),
            agent_set: self.agent.clone().filter(|a| !self.has_agent(a)),
            cron_on: self.cron_on,
            cron: self.cron.clone(),
            round: self.round_at.is_some(),
            online: self.online(now),
            first_at: self.first_at,
            beat_at: self.beat_at,
            off_at: self.off_at,
            ask: self.ask.clone(),
            drain: self.drain_at.is_some(),
            rechecked_at: self.rechecked_at,
            ops: vec![],
            token: None,
            selftest: self.selftest.clone(),
            skills: self.skills_raw.as_deref().and_then(|j| serde_json::from_str(j).ok()),
            mcp: self.mcp_raw.as_deref().and_then(|j| serde_json::from_str(j).ok()),
            providers: self.providers_raw.as_deref().and_then(|j| serde_json::from_str(j).ok()),
            skill_import: self.skill_import.clone(),
        }
    }
    /// 正在等重启或升级：board host 不再开始新的派发
    fn restarting(&self) -> bool {
        self.ask.as_ref().is_some_and(|a| a.kind == "restart" || a.kind == "upgrade")
    }
}

const DISPATCH_SELECT: &str = "SELECT d.id, b.project || '/' || b.key, d.card, COALESCE(c.title, d.title), d.host_id, d.host, d.agent,
       d.agent_label, d.name, d.source, d.by_kind, d.by_name, d.state, d.result, d.created_at, d.started_at, d.ended_at,
       d.exit_code, d.stop_by, CASE WHEN c.holder = d.name THEN c.beat_at END, d.env, d.env_name, COALESCE(d.session, d.id),
       CASE WHEN d.session IS NULL THEN 1 ELSE (SELECT COUNT(*) FROM dispatches s WHERE s.session = d.session AND s.id <= d.id) END,
       CASE WHEN d.session IS NULL THEN 1 ELSE (SELECT COUNT(*) FROM dispatches s WHERE s.session = d.session) END,
       d.model, d.effort, d.kind
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
        seq: r.get(23)?,
        total: r.get(24)?,
        model: r.get(25)?,
        effort: r.get(26)?,
        kind: r.get(27)?,
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

/// 生成 agent 名字时第二段（模型名）用什么，以及它是不是机器设置里选的：机器设置里选了这个 agent 的模型时用它
/// （`set` 为真），否则用 board host 上报的（配置文件 `[agents.<标识>] model` → 内置模板默认值 → 空，空表示用 agent 标识）
fn model_of(agents: &[HostAgent], run: &BTreeMap<String, AgentRun>, agent: &str) -> (String, bool) {
    let reported = agents.iter().find(|a| a.id == agent).map(|a| a.model.clone()).unwrap_or_default();
    // 机器设置里选了模型：board host 上报了这个 agent 能选（新版）才算
    let set = agents.iter().find(|a| a.id == agent).and_then(|a| a.opts.as_ref()).is_some_and(|o| o.model);
    let selected = run.get(agent).and_then(|r| r.model.clone()).filter(|_| set).filter(|m| check_model(m).is_ok());
    if let Some(m) = selected {
        return (m, true);
    }
    // 没在机器设置里选：用 board host 上报的（配置文件 → 内置模板默认值）。它就是名字里的模型名，算得上是「按设置来的」
    let reported = reported.trim().to_string();
    if !reported.is_empty() {
        return (reported, true);
    }
    (String::new(), false)
}

/// `board check --agent <标识>` 用：按写事务里的机器记录（[`W::agent_check_generated`] 已经按机器名或令牌找到的那台）
/// 算这个 agent 该用的模型名
pub(crate) fn model_for_transaction(conn: &rusqlite::Connection, h: &HostInfo, agent: &str) -> Result<(String, bool)> {
    let run: BTreeMap<String, AgentRun> = conn
        .query_row("SELECT run FROM hosts WHERE id = ?1", [h.id], |r| r.get::<_, String>(0))
        .optional()?
        .map(|raw| serde_json::from_str(&raw).unwrap_or_default())
        .unwrap_or_default();
    Ok(model_of(&h.agents, &run, agent))
}

/// 按名字找一台在线的机器（带上按机器设置算好的 agent 数据）：`board check --agent <标识>` 用它。
/// 机器不存在、已移除、离线时为空（这条命令不报 `NO_SUCH_HOST`，按「查不到机器」退回 `<agent 标识>/<模型>`）
pub(crate) fn named_host(conn: &rusqlite::Connection, name: &str, now: i64) -> Result<Option<HostInfo>> {
    let h: Option<HostRow> = conn.query_row(&format!("SELECT {HOST_COLS} FROM hosts WHERE name = ?1"), [name], host_row).optional()?;
    Ok(h.filter(|h| h.online(now)).map(|h| h.info(now)))
}


/// 令牌是某台在线机器的 board host 最后一次上报用的那个：`board check --agent <标识>` 没带机器名时按它认机器。
/// 没有时为空（同时有多台机器用同一个令牌时取名字排在前面的，不该发生）
pub(crate) fn host_by_token(conn: &rusqlite::Connection, token: i64, now: i64) -> Result<Option<HostInfo>> {
    let h: Option<HostRow> =
        conn.query_row(&format!("SELECT {HOST_COLS} FROM hosts WHERE token = ?1 ORDER BY name LIMIT 1"), [token], host_row).optional()?;
    Ok(h.filter(|h| h.online(now)).map(|h| h.info(now)))
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

    /// 这台机器用的是不是这个令牌：board host 每 10 秒上报一次，最后一次上报用的令牌就是它现在的令牌。
    /// 机器不存在、已移除、或者没上报过令牌时为 false；只看在线的机器（离线的不再取配置）
    pub fn host_uses_token(&self, name: &str, token: i64, now: i64) -> Result<bool> {
        let Some(h) = self.host_row(name)? else { return Ok(false) };
        Ok(h.token == Some(token) && h.online(now))
    }

    /// `GET /api/hosts/mine`：这个令牌是哪台在线机器用的（`board check --agent <标识>` 在旧服务端上自己拼名字时用它）。
    /// 本机模式（服务端以 `--local` 运行，本机来的请求没有令牌）返回本机这一台：按本机主机名转成的机器名找，
    /// 找不到时（机器名改过、还没重新上报）取唯一的一台。登录的界面不带令牌，不看令牌找时为空
    pub fn my_host(&self, token: Option<i64>, now: i64) -> Result<Option<HostInfo>> {
        match token {
            Some(t) => host_by_token(self.conn, t, now),
            None if self.local => {
                let mut hosts = self.hosts(now)?;
                if hosts.len() > 1 {
                    // 本机到底跑的是哪一台：按主机名转成的机器名找
                    if let Ok(name) = crate::hosts::machine_hostname() {
                        let want = crate::hosts::host_name_from(&name);
                        hosts.retain(|h| h.name == want);
                    }
                }
                hosts.retain(|h| h.online);
                Ok(if hosts.len() == 1 { hosts.pop() } else { None })
            }
            None => Ok(None),
        }
    }

    /// 机器信息，加上「有 N 张可领取的卡片没有匹配任何规则」（有规则、没设兜底时）
    fn host_info(&self, h: &HostRow, now: i64) -> Result<HostInfo> {
        let mut info = h.info(now);
        info.ops = self.host_ops(h.id)?;
        info.token = self.host_token(h, now)?;
        // 每块白板的环境检查命令（按这台机器的系统选好）：board host 照着跑，界面照着判断结果算不算数
        for d in info.dirs.iter_mut() {
            d.check = match self.resolve(&d.board) {
                Ok(b) => self.ready_check(b.id)?.for_os(&h.os),
                Err(_) => None,
            };
        }
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

    /// 机器最近的操作记录（新的在前）
    fn host_ops(&self, host_id: i64) -> Result<Vec<HostOp>> {
        let mut st = self.conn.prepare("SELECT at, by_name, text FROM host_ops WHERE host_id = ?1 ORDER BY id DESC LIMIT ?2")?;
        let rows = st.query_map(params![host_id, OPS_KEEP], |r| Ok(HostOp { at: r.get(0)?, by: r.get(1)?, text: r.get(2)? }))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 机器用的令牌（令牌表在服务端那一层建；没有这张表、令牌已经吊销时为空）
    fn host_token(&self, h: &HostRow, now: i64) -> Result<Option<HostToken>> {
        let Some(id) = h.token else { return Ok(None) };
        let has: bool = self.conn.query_row("SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'auth_tokens'", [], |r| r.get(0))?;
        if !has {
            return Ok(None);
        }
        let row: Option<(String, String, Option<i64>)> = self
            .conn
            .query_row("SELECT name, hint, last_used_at FROM auth_tokens WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((name, hint, last_used_at)) = row else { return Ok(None) };
        let mut st = self.conn.prepare("SELECT name FROM hosts WHERE token = ?1 AND id != ?2 AND off_at IS NULL AND beat_at + ?4 > ?3 ORDER BY name")?;
        let shared = st.query_map(params![id, h.id, now, HOST_GAP_MS], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(Some(HostToken { id, name, hint, last_used_at, shared }))
    }

    /// 白板的环境检查命令
    pub(crate) fn ready_check(&self, board: i64) -> Result<ReadyCheck> {
        Ok(self.conn.query_row("SELECT ready_check, ready_check_win FROM boards WHERE id = ?1", [board], |r| {
            Ok(ReadyCheck { cmd: r.get(0)?, win: r.get(1)? })
        })?)
    }

    /// 这台机器上这块白板能不能接卡（环境检查）
    fn dir_readiness(&self, h: &HostRow, board: &BoardRef) -> Result<Readiness> {
        let path = board.path();
        let cmd = self.ready_check(board.id)?.for_os(&h.os);
        let ready = h.dirs.iter().find(|d| d.board == path).and_then(|d| d.ready.as_ref());
        Ok(readiness(cmd.as_deref(), ready))
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

    /// 一张卡片的全部派发（排队、在跑、结束的都算），新的在前（卡片详情「派发记录」）
    pub fn card_dispatches(&self, board: &BoardRef, card: i64) -> Result<Vec<Dispatch>> {
        self.dispatches("WHERE d.board = ?1 AND d.card = ?2 ORDER BY d.id DESC", params![board.id, card])
    }

    /// 一个会话（同一个 agent 进程用 board next 接着做的几张卡）：给出其中任一次派发，返回这个会话的全部派发（按先后，
    /// 开会话时排着、还没开始的也在）、各自的日志（每个派发最后 500 行）和全部事件
    pub fn session(&self, id: i64) -> Result<SessionView> {
        self.session_since(id, 0)
    }

    /// 同 [`R::session`]，事件只取 `rev` 大于 `since` 的（新增的、合进了结果的）；since = 0 取全部
    pub fn session_since(&self, id: i64, since: i64) -> Result<SessionView> {
        let s: i64 = self
            .conn
            .query_row("SELECT COALESCE(session, id) FROM dispatches WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("派发 #{id} 不存在")))?;
        let list = self.dispatches("WHERE COALESCE(d.session, d.id) = ?1 ORDER BY d.id", [s])?;
        let parts = list
            .into_iter()
            .map(|d| {
                let log = self.dispatch_log(d.id)?;
                let (fmt, events_dropped): (Option<String>, i64) =
                    self.conn.query_row("SELECT fmt, ev_dropped FROM dispatches WHERE id = ?1", [d.id], |r| Ok((r.get(0)?, r.get(1)?)))?;
                let events = self.dispatch_events(d.id, since)?;
                let events_from: i64 =
                    self.conn.query_row("SELECT COALESCE(MIN(id), 0) FROM dispatch_events WHERE dispatch = ?1", [d.id], |r| r.get(0))?;
                Ok(SessionPart { dispatch: d, log, fmt, events, events_from, events_dropped })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(SessionView { session: s, parts, runs: self.runs_rev()? })
    }

    /// 事件的版本：每次有事件写入（或合进结果）加一。实时推送里带上，界面据此增量取
    pub fn runs_rev(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COALESCE((SELECT value FROM meta WHERE key = 'runs'), 0)", [], |r| r.get(0))?)
    }

    /// 一次派发的事件（按先后），只取 rev 大于 since 的
    pub fn dispatch_events(&self, id: i64, since: i64) -> Result<Vec<crate::runlog::RunEvent>> {
        let mut st = self.conn.prepare("SELECT id, data FROM dispatch_events WHERE dispatch = ?1 AND rev > ?2 ORDER BY id")?;
        let rows = st.query_map(params![id, since], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = vec![];
        for row in rows {
            let (eid, data) = row?;
            let mut e: crate::runlog::RunEvent = serde_json::from_str(&data).unwrap_or_else(|_| crate::runlog::RunEvent::text("text", data));
            e.id = eid;
            e.call = None;
            out.push(e);
        }
        Ok(out)
    }

    /// since 之后有新事件的派发（实时推送里带上，打开着的会话据此马上取）
    pub fn dispatches_with_events_since(&self, since: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT DISTINCT dispatch FROM dispatch_events WHERE rev > ?1 ORDER BY dispatch LIMIT 100")?;
        let rows = st.query_map([since], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<Vec<i64>, _>>()?)
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
            // 会话里排着的派发只由那个会话的 board next 开始，不给 board host（它按创建先后开始 queued 里的）
            queued: self.dispatches("WHERE d.host_id = ?1 AND d.state = 'queued' AND d.session IS NULL ORDER BY d.id", [h.id])?,
            host: self.host_info(&h, now)?,
            now,
            skills: Some(self.skills()?),
            mcp: Some(self.mcp_index()?),
            providers: Some(self.provider_work()?),
            templates: Some(self.agent_templates()?),
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
    model: Option<String>,
    effort: Option<String>,
    /// 空 = 做这张卡；`split` = 拆解
    kind: String,
}

impl DRow {
    fn split(&self) -> bool {
        self.kind == KIND_SPLIT
    }
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
                "SELECT id, board, card, host_id, host, agent, agent_label, name, source, state, stop_by, started_at, env_name, env, COALESCE(session, id), model, effort, kind FROM dispatches WHERE id = ?1",
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
                        model: r.get(15)?,
                        effort: r.get(16)?,
                        kind: r.get(17)?,
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

    /// 派发排队：卡片沿连线进入领取时会进的锁定状态（排在那一块最下面），没有持有者，`claimed_from` 记下原来的可领取状态。
    /// 排队的卡片别人领不到，留在可领取状态里看起来像「谁都能领」（PRD §9.1「占用」）。
    /// 卡片不在可领取状态、被持有、或者那个状态没有连到锁定状态时不动。返回进入的状态名称（写进动态）
    fn reserve(&mut self, b: i64, card: i64, by: &Actor) -> Result<Option<String>> {
        let row: Option<(String, Option<String>)> = self
            .tx
            .query_row("SELECT col, holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![b, card], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((col, None)) = row else { return Ok(None) };
        let flow = flow_of(self.tx, b)?;
        if !flow.is_claim(&col) {
            return Ok(None);
        }
        let Some(dest) = flow.claim_target(&col).cloned() else { return Ok(None) };
        self.place(b, card, Some(&col), &dest.key, At::Bottom)?;
        self.tx.execute("UPDATE cards SET claimed_from = ?1 WHERE board = ?2 AND id = ?3", params![col, b, card])?;
        self.log_move(b, card, "dispatch", &col, &dest.key, by, &flow)?;
        Ok(Some(dest.name))
    }

    /// 排队的派发没开始就结束了（取消、没领到、会话结束、移除机器）：卡片还在排队时进的锁定状态里、没有人持有时，
    /// 回到原来的可领取状态最上面（和释放一样，PRD §8 规则 7）；已经在可领取状态里的（例如锁定状态被删掉时被移了回去）
    /// 只清掉记录。返回写进动态的后半句（没有移动时为空）
    fn unreserve(&mut self, b: i64, card: i64, by: &Actor) -> Result<String> {
        let row: Option<(String, Option<String>, Option<String>)> = self
            .tx
            .query_row(
                "SELECT col, holder, claimed_from FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL",
                params![b, card],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((col, None, from)) = row else { return Ok(String::new()) };
        let flow = flow_of(self.tx, b)?;
        // 没有持有者时 claimed_from 只在派发排队时记（释放会清掉它）；锁定状态被删掉时它也被清掉，这时按「在锁定状态里」认
        if from.is_none() && !flow.is_lock(&col) {
            return Ok(String::new());
        }
        self.tx.execute("UPDATE cards SET claimed_from = NULL WHERE board = ?1 AND id = ?2", params![b, card])?;
        if col == INBOX || flow.is_claim(&col) {
            return Ok(String::new());
        }
        let dest = flow.back_to(from.as_deref()).clone();
        self.place(b, card, Some(&col), &dest.key, At::Top)?;
        self.log_move(b, card, "dispatch", &col, &dest.key, by, &flow)?;
        Ok(format!("，卡片回到「{}」最上面", dest.name))
    }

    /// 锁定状态里被持有的卡片排在派发排队的卡片前面：领取进来（含排队后原地开始）时放在第一张排队的卡片前面，没有就放到最下面
    pub(crate) fn lock_slot(&self, b: i64, col: &str, card: i64) -> Result<At> {
        let queued: Vec<i64> = self.collect("SELECT card FROM dispatches WHERE board = ?1 AND state = 'queued'", [b], |r| r.get(0))?;
        if queued.is_empty() {
            return Ok(At::Bottom);
        }
        let ids: Vec<i64> = self.col_ids(b, col)?.into_iter().filter(|x| *x != card).collect();
        Ok(match ids.iter().position(|x| queued.contains(x)) {
            Some(i) => At::Index(i),
            None => At::Bottom,
        })
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

    /// `board next`（只在 board host 启动的会话里用）：结算这个名字正在跑的派发，按开会话时排好的顺序开始这个会话排着的
    /// 下一张（以同一个名字领取，同一台机器、agent、启动环境）。都在一个写事务里。领取失败的那张结果「没领到」，接着取再下一张。
    ///
    /// 返回 `Ok(Err(SESSION_DONE))` 时这个会话不再接着做：还排着的派发已经取消、卡片不动。闭包返回错误会让写事务回滚，
    /// 所以调用方先提交、再把这个错误交给 agent（见 [`crate::Store::next`]）。
    pub fn dispatch_next(&mut self, name: &str) -> Result<std::result::Result<CardDetail, BoardError>> {
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
        let h = self.host_row(&d.host)?;
        let why = if d.source != "cron" {
            Some(format!("这个会话是{}派发启动的，只做指定的那一张", source_words(&d.source)))
        } else if d.stop_by.is_some() {
            Some("有人停止了这个会话".into())
        } else {
            match &h {
                None => Some(format!("机器「{}」已被移除", d.host)),
                Some(h) if h.paused => Some(format!("机器「{}」暂停接单", h.name)),
                // 规则被删光：做完当前这张就结束，排着的取消
                Some(h) if h.claim_rules().is_none() => Some(format!("机器「{}」已经没有领取规则", h.name)),
                Some(_) => None,
            }
        };
        if let Some(why) = why {
            return self.session_done(&d, &why);
        }
        let h = h.expect("上面已经处理了机器不存在");
        let queue: Vec<i64> =
            self.collect("SELECT id FROM dispatches WHERE session = ?1 AND state = 'queued' ORDER BY id", [d.session], |r| r.get(0))?;
        if queue.is_empty() {
            return self.session_done(&d, "这个会话排着的卡片都开始过或取消了");
        }
        // 结算上一张：卡片已经不由这个名字持有，在交审状态记「已交审」，其他状态记「做不下去」
        let flow = flow_of(self.tx, d.board)?;
        let col: Option<String> =
            self.tx.query_row("SELECT col FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| r.get(0)).optional()?;
        let result = if col.as_deref().is_some_and(|c| flow.is_review(c)) { "review" } else { "stuck" };
        self.close_row(&d, result, None)?;
        let actor = Actor::agent(name.to_string());
        let took = d.started_at.map(|t| format!("，用时 {}", crate::engine::dur_words(self.now - t))).unwrap_or_default();
        let text = format!("派发结束：{}（{}{took}），接着做下一张", result_words(result), Self::dispatch_words(&d));
        self.track_dispatch(d.board, d.card, &actor, "dispatch", &text)?;
        // 按排好的顺序以同一个名字领取；领不到的记「没领到」，接着取再下一张
        for id in queue {
            let q = self.drow(id)?;
            let q = DRow { name: Some(name.to_string()), env: d.env.clone(), env_name: d.env_name.clone(), model: d.model.clone(), effort: d.effort.clone(), ..q };
            self.dispatch_ok = true;
            let claimed = self.exec(&board, &actor, Cmd::Claim { id: Some(q.card) });
            self.dispatch_ok = false;
            match claimed {
                Ok(_) => {
                    self.tx.execute(
                        "UPDATE dispatches SET state = 'running', name = ?2, env = ?3, env_name = ?4, started_at = ?5, model = ?6, effort = ?7 WHERE id = ?1",
                        params![q.id, name, q.env, q.env_name, self.now, q.model, q.effort],
                    )?;
                    let seq: i64 = self.tx.query_row("SELECT COUNT(*) FROM dispatches WHERE session = ?1 AND id <= ?2", params![d.session, q.id], |r| r.get(0))?;
                    let who = match &q.env_name {
                        Some(e) => format!("{}（{e}）", q.agent_label),
                        None => format!("{} ", q.agent_label),
                    };
                    let text = format!("派发开始：{} 上的 {who}以 {name} 接着做（定时，这个会话的第 {seq} 张{}）", h.name, opts_words(&q.model, &q.effort));
                    self.track(d.board, q.card, &actor, "dispatch", &text)?;
                    self.touch_hosts();
                    return Ok(Ok(self.read().card(&board, q.card)?));
                }
                Err(e) if e.code == ErrorCode::Internal => return Err(e),
                Err(e) => {
                    self.close_row(&q, "unclaimed", None)?;
                    let back = self.unreserve(q.board, q.card, &Actor::system())?;
                    let text = format!("派发没领到：{}（{}）{back}", e.message, Self::dispatch_words(&q));
                    self.track_dispatch(q.board, q.card, &Actor::system(), "dispatch", &text)?;
                }
            }
        }
        Ok(Err(session_done_error("这个会话排着的卡片都没领到")))
    }

    /// 会话不再接着做：取消这个会话还排着的派发（卡片不动），返回 SESSION_DONE（由调用方提交后交给 agent）
    fn session_done(&mut self, d: &DRow, why: &str) -> Result<std::result::Result<CardDetail, BoardError>> {
        self.cancel_session_rest(d.session, why)?;
        Ok(Err(session_done_error(why)))
    }

    /// 会话不再接着做（board next 返回 SESSION_DONE、进程退出、被人停止、机器掉线）：这个会话还排着的派发都取消，
    /// 结果「已取消」，卡片回到原来的可领取状态最上面；这一轮还没结束时由新开的会话重新排
    fn cancel_session_rest(&mut self, session: i64, why: &str) -> Result<()> {
        // 后排的先回去：卡片一张张回到可领取状态最上面，最后回去的在最上面，回去后仍是排好的顺序
        let ids: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE session = ?1 AND state = 'queued' ORDER BY id DESC", [session], |r| r.get(0))?;
        for id in ids {
            let q = self.drow(id)?;
            self.close_dispatch(&q, "canceled", None)?;
            let back = self.unreserve(q.board, q.card, &Actor::system())?;
            let text = format!("会话结束（{why}），排着的派发取消（{}）{back}", Self::dispatch_words(&q));
            self.track_dispatch(q.board, q.card, &Actor::system(), "dispatch", &text)?;
        }
        Ok(())
    }

    /// 开会话时排好后面几张（第一张已经开始）：从同一块白板、按领取规则交给同一个 agent 的可领取卡片里取，连第一张最多
    /// 「一个会话最多做几张」张。先后：带优先标签的在前（按规则 1 的顺序），再是其余卡片里和第一张共同标签多的
    /// （领取规则里用到的标签不算，它只表示谁来做），一样多时按规则 1 的顺序。每张记一条排队的派发，属于这个会话
    fn plan_session(&mut self, h: &HostRow, rules: &Rules, first: &Dispatch, board: &BoardRef, flow: &Flow, skip: &[i64]) -> Result<()> {
        let want = (h.per_session.max(1) - 1) as usize;
        if want == 0 {
            return Ok(());
        }
        let mut skip = skip.to_vec();
        skip.push(first.card);
        let rule_labels = rules.labels();
        let mine: Vec<String> = card_label_names(self.tx, board.id, first.card)?.into_iter().filter(|l| !rule_labels.contains(l)).collect();
        let urgent = self.priority_cards(board.id)?;
        // （不带优先标签，共同标签数取反，规则 1 的顺序，卡片）
        let mut list: Vec<(bool, std::cmp::Reverse<usize>, usize, i64)> = vec![];
        for (i, (id, _)) in self.claimable_order(board.id, flow, &skip)?.into_iter().enumerate() {
            let labels = card_label_names(self.tx, board.id, id)?;
            if rules.route(&labels) != Some(first.agent.as_str()) {
                continue;
            }
            let hot = urgent.contains(&id);
            let common = if hot { 0 } else { labels.iter().filter(|l| mine.contains(l)).count() };
            list.push((!hot, std::cmp::Reverse(common), i, id));
        }
        list.sort();
        list.truncate(want);
        if list.is_empty() {
            return Ok(());
        }
        let total = list.len() + 1;
        let name = first.name.clone().unwrap_or_default();
        let by = Actor::system();
        for (k, (.., card)) in list.into_iter().enumerate() {
            let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2", params![board.id, card], |r| r.get(0))?;
            self.tx.execute(
                "INSERT INTO dispatches (board, card, title, host_id, host, agent, agent_label, source, by_kind, by_name, state, created_at, session)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'cron', ?8, ?9, 'queued', ?10, ?11)",
                params![board.id, card, title, h.id, h.name, first.agent, first.agent_label, by.kind.as_str(), by.name, self.now, first.id],
            )?;
            let at = self.reserve(board.id, card, &by)?.map(|n| format!("在「{n}」")).unwrap_or_default();
            let text = format!("派发到 {} · {}（定时），会话第 {}/{total} 张，{at}排队等 {name} 接着做", h.name, first.agent_label, k + 2);
            self.track(board.id, card, &by, "dispatch", &text)?;
        }
        self.touch_hosts();
        Ok(())
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

    /// 超时：卡片仍由这个名字持有。和 final 一样结束这一次（记耗时、之后 50 秒内要移走），
    /// 但这一次记成 `how = timeout`：大盘「按 agent」里算进异常退出
    fn timeout_beat(&mut self, b: i64, card: i64, actor: &Actor) -> Result<()> {
        // 先像普通心跳一样打一次：没有进行中的一次时由它开始（生产里 board host 早就打过心跳了）
        self.beat_card(b, card, actor)?;
        if let Some((n, ms)) = self.close_run(b, card, self.now, "timeout")? {
            self.tx.execute("UPDATE cards SET beat_at = ?1, final_at = ?1 WHERE board = ?2 AND id = ?3", params![self.now, b, card])?;
            self.track(b, card, actor, "beat", &format!("结束了心跳：超时，第 {n} 次耗时 {}", crate::engine::dur_words(ms)))?;
        }
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
            if !crate::valid_board(board) {
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
                if rep.selftest {
                    let st = SelfTest { able: true, ..Default::default() };
                    self.tx.execute("UPDATE hosts SET selftest = ?2 WHERE name = ?1", params![rep.name, serde_json::to_string(&st).unwrap_or_default()])?;
                }
                if let Some(local) = &rep.local_skills {
                    let si = SkillImport { able: true, local: local.clone(), ..Default::default() };
                    self.tx.execute("UPDATE hosts SET skill_import = ?2 WHERE name = ?1", params![rep.name, serde_json::to_string(&si).unwrap_or_default()])?;
                }
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
                self.host_acks(&h, rep)?;
                // skills 同步状态：带了、变了才写
                if let Some(st) = &rep.skills {
                    let raw = serde_json::to_string(st).unwrap_or_else(|_| "[]".into());
                    if h.skills_raw.as_deref() != Some(raw.as_str()) {
                        self.tx.execute("UPDATE hosts SET skills = ?2 WHERE id = ?1", params![h.id, raw])?;
                        self.touch_hosts();
                    }
                }
                // MCP 同步状态：带了、变了才写
                if let Some(st) = &rep.mcp {
                    let raw = serde_json::to_string(st).unwrap_or_else(|_| "[]".into());
                    if h.mcp_raw.as_deref() != Some(raw.as_str()) {
                        self.tx.execute("UPDATE hosts SET mcp = ?2 WHERE id = ?1", params![h.id, raw])?;
                        self.touch_hosts();
                    }
                }
                // 模型供应商写进各 agent 配置文件的状态：带了、变了才写
                if let Some(st) = &rep.providers {
                    let raw = serde_json::to_string(st).unwrap_or_else(|_| "[]".into());
                    if h.providers_raw.as_deref() != Some(raw.as_str()) {
                        self.tx.execute("UPDATE hosts SET providers = ?2 WHERE id = ?1", params![h.id, raw])?;
                        self.touch_hosts();
                    }
                }
                // 本机的 skill（「从机器导入」）：带了、变了才写；带了就算会导入
                if let Some(local) = &rep.local_skills {
                    if !h.skill_import.able || &h.skill_import.local != local {
                        let si = SkillImport { able: true, local: local.clone(), ..h.skill_import.clone() };
                        self.tx.execute("UPDATE hosts SET skill_import = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&si).unwrap_or_default()])?;
                        self.touch_hosts();
                    }
                }
                // 换成了会（或不会）自检的 board host
                if h.selftest.able != rep.selftest {
                    let st = SelfTest { able: rep.selftest, ..h.selftest.clone() };
                    self.tx.execute("UPDATE hosts SET selftest = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&st).unwrap_or_default()])?;
                    self.touch_hosts();
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
                    match self.agent_beat_by(name, false, crate::agents::BeatBy::Host) {
                        Ok(_) => {}
                        // 名字已经离线：扫描会按机器掉线结束这次派发
                        Err(e) if e.code == ErrorCode::NameOffline => {}
                        Err(e) => return Err(e),
                    }
                }
            } else {
                self.finish(&d, None, &Tail { lines: &[], events: &[] }, Finish::Missing)?;
            }
        }
        self.read().host_work(&name, now)
    }

    /// board host 对远程管理请求的回报（在上报里）：
    /// - `asked`：开始处理（重启、升级）或做完（重新检查）了这个请求，清掉它，记一条操作
    /// - `ask_error`：升级（或重启）没成，记下原因；为升级暂停的接单恢复
    /// - 为升级暂停了接单、请求已经交给它、换了一个 board host 进程上来：升级做完了（版本变了）或没生效，恢复接单
    fn host_acks(&mut self, h: &HostRow, rep: &HostReport) -> Result<()> {
        let sys = "board host";
        if let (Some(id), Some(ask)) = (rep.asked, &h.ask) {
            if ask.id == id {
                self.tx.execute("UPDATE hosts SET ask = NULL WHERE id = ?1", [h.id])?;
                match ask.kind.as_str() {
                    "recheck" => {
                        self.tx.execute("UPDATE hosts SET rechecked_at = ?2 WHERE id = ?1", params![h.id, self.now])?;
                        self.host_op(h.id, sys, "重新检查完了：启动环境、本机 agent、board-skills 已重新上报")?;
                    }
                    "restart" => self.host_op(h.id, sys, "正在重启 board host")?,
                    _ => self.host_op(h.id, sys, &format!("开始升级到 {}", ask.to.as_deref().unwrap_or("服务端的版本")))?,
                }
                self.touch_hosts();
            }
        }
        let restore = |w: &mut Self| -> Result<()> {
            if h.up_pause {
                w.tx.execute("UPDATE hosts SET paused = 0, up_pause = 0 WHERE id = ?1", [h.id])?;
            }
            Ok(())
        };
        if let Some(err) = rep.ask_error.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
            let text: String = err.chars().take(500).collect();
            self.host_op(h.id, sys, &format!("{text}{}", if h.up_pause { "；已恢复接单" } else { "" }))?;
            restore(self)?;
            self.touch_hosts();
        } else if h.up_pause && h.ask.is_none() && h.instance != rep.instance {
            if rep.version != h.version {
                self.host_op(h.id, sys, &format!("升级到 {}，重新上线；已恢复接单", rep.version))?;
            } else {
                self.host_op(h.id, sys, &format!("升级没有生效：重新上线后还是 {}；已恢复接单", rep.version))?;
            }
            restore(self)?;
            self.touch_hosts();
        }
        Ok(())
    }

    /// 记一条机器的操作（最近的 OPS_KEEP 条以外的删掉）
    fn host_op(&mut self, host_id: i64, by: &str, text: &str) -> Result<()> {
        self.tx.execute("INSERT INTO host_ops (host_id, at, by_name, text) VALUES (?1, ?2, ?3, ?4)", params![host_id, self.now, by, text])?;
        self.tx.execute(
            "DELETE FROM host_ops WHERE host_id = ?1 AND id NOT IN (SELECT id FROM host_ops WHERE host_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![host_id, OPS_KEEP],
        )?;
        Ok(())
    }

    /// 远程管理（PRD §9.1「远程管理」，只能在登录后的界面上做）：
    /// - `recheck` 重新检查：board host 重新查找启动环境、本机 agent、board-skills，立即上报；在跑的派发不受影响
    /// - `restart` 重启 board host：在跑的派发按「人停止」结束，board host 重新启动（重新读配置、重新检查）
    /// - `upgrade` 升级到服务端的版本：先暂停接单，等在跑的派发结束（`now` 时按「人停止」结束它们）再升级；
    ///   升级完或失败后恢复接单。只在这台机器的版本比服务端旧时可以
    /// - `drain` 排空：暂停接单，等在跑的派发结束（界面上显示「已排空」）；恢复接单时结束排空
    /// - `cancel` 取消还没处理的请求（为升级暂停的接单恢复）
    ///
    /// 重新检查、重启、升级要机器在线；同一时间只有一个请求，新的换掉旧的
    pub fn host_ask(&mut self, name: &str, kind: &str, now_flag: bool, by: &str) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if !ASK_KINDS.contains(&kind) {
            return Err(BoardError::usage(format!("不认识的操作「{kind}」：只能是 {}", ASK_KINDS.join("、"))));
        }
        let by = if by.trim().is_empty() { "人" } else { by.trim() };
        let running: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE host = ?1 AND state = 'running' ORDER BY id", [&h.name], |r| r.get(0))?;
        let what = match kind {
            "recheck" => "重新检查",
            "restart" => "重启 board host",
            "upgrade" => "升级",
            _ => "",
        };
        if !what.is_empty() && !h.online(self.now) {
            return Err(BoardError::usage(format!("机器「{name}」离线：等它上线再{what}")));
        }
        let server = env!("CARGO_PKG_VERSION");
        let mut ask = HostAsk { id: h.ask_seq + 1, kind: kind.into(), by: by.into(), at: self.now, to: None, now: false };
        let text = match kind {
            "recheck" => "请求重新检查".to_string(),
            "restart" => {
                for id in &running {
                    self.request_stop(*id, &Actor::human(), by)?;
                }
                format!("请求重启 board host{}", if running.is_empty() { String::new() } else { format!("：先结束 {} 个在跑的派发", running.len()) })
            }
            "upgrade" => {
                if !version_lt(&h.version, server) {
                    return Err(BoardError::usage(format!("机器「{name}」已经是 {}，不比服务端（{server}）旧，不用升级", h.version)));
                }
                ask.to = Some(server.into());
                ask.now = now_flag;
                if !h.paused {
                    self.tx.execute("UPDATE hosts SET paused = 1, up_pause = 1, round_at = NULL WHERE id = ?1", [h.id])?;
                }
                if now_flag {
                    for id in &running {
                        self.request_stop(*id, &Actor::human(), by)?;
                    }
                    format!("请求立即升级到 {server}{}", if running.is_empty() { String::new() } else { format!("：结束 {} 个在跑的派发", running.len()) })
                } else if running.is_empty() {
                    format!("请求升级到 {server}：暂停接单")
                } else {
                    format!("请求升级到 {server}：暂停接单，等 {} 个派发结束后升级", running.len())
                }
            }
            "drain" => {
                self.tx.execute("UPDATE hosts SET paused = 1, drain_at = COALESCE(drain_at, ?2), round_at = NULL WHERE id = ?1", params![h.id, self.now])?;
                self.host_op(h.id, by, &format!("排空：暂停接单{}", if running.is_empty() { "，没有在跑的派发".to_string() } else { format!("，等 {} 个在跑的派发结束", running.len()) }))?;
                self.touch_hosts();
                return self.read().host(name, self.now);
            }
            _ => {
                let Some(old) = &h.ask else { return self.read().host(name, self.now) };
                self.tx.execute("UPDATE hosts SET ask = NULL WHERE id = ?1", [h.id])?;
                if h.up_pause {
                    self.tx.execute("UPDATE hosts SET paused = 0, up_pause = 0 WHERE id = ?1", [h.id])?;
                }
                let w = match old.kind.as_str() {
                    "recheck" => "重新检查",
                    "restart" => "重启",
                    _ => "升级",
                };
                self.host_op(h.id, by, &format!("取消了{w}请求{}", if h.up_pause { "，恢复接单" } else { "" }))?;
                self.touch_hosts();
                return self.read().host(name, self.now);
            }
        };
        self.tx.execute(
            "UPDATE hosts SET ask = ?2, ask_seq = ?3 WHERE id = ?1",
            params![h.id, serde_json::to_string(&ask).unwrap_or_default(), ask.id],
        )?;
        self.host_op(h.id, by, &text)?;
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// 自检（PRD §10.12「自检」，只能在登录后的界面上做）：记下请求，board host 取到任务时在后台做。
    /// `agent` 给了就只检查这个 agent（「本机 agent」那一行的按钮），否则整台机器（每个找得到的 agent、每块白板的目录）。
    /// 要机器在线；同一台机器同时只做一次（上一次超过 [`SELFTEST_STALE_MS`] 还没做完的算作废）。不占会话位，不影响在跑的派发
    pub fn host_selftest(&mut self, name: &str, agent: Option<&str>, by: &str) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if !h.online(self.now) {
            return Err(BoardError::usage(format!("机器「{name}」离线：等它上线再自检")));
        }
        if !h.selftest.able {
            return Err(BoardError::usage(format!(
                "机器「{name}」的 board（{}）还不会自检：先升级到 {}",
                if h.version.is_empty() { "旧版" } else { &h.version },
                env!("CARGO_PKG_VERSION")
            )));
        }
        if let Some(a) = &h.selftest.ask {
            if a.at + SELFTEST_STALE_MS > self.now {
                return Err(BoardError::usage(format!("机器「{name}」正在自检（{} 请求的）：等它做完再来", a.by)));
            }
        }
        let agent = agent.map(str::trim).filter(|a| !a.is_empty());
        let label = match agent {
            Some(id) => {
                let a = h.resolved().into_iter().find(|a| a.id == id).ok_or_else(|| BoardError::usage(format!("机器「{name}」上没有 agent「{id}」")))?;
                if !a.found {
                    return Err(BoardError::usage(format!("机器「{name}」上找不到 {} 的命令，没法自检", a.name)));
                }
                format!("请求自检 {}", a.name)
            }
            None => "请求自检".to_string(),
        };
        let by = if by.trim().is_empty() { "人" } else { by.trim() };
        let mut st = h.selftest.clone();
        st.ask = Some(SelfTestAsk { id: h.ask_seq + 1, agent: agent.map(str::to_string), by: by.into(), at: self.now });
        self.tx.execute(
            "UPDATE hosts SET selftest = ?2, ask_seq = ?3 WHERE id = ?1",
            params![h.id, serde_json::to_string(&st).unwrap_or_default(), h.ask_seq + 1],
        )?;
        self.host_op(h.id, by, &label)?;
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// 从这台机器导入 skills（PRD §10.12「共用配置」，只能在登录后的界面上做）：记下要导入的（名称 + 所在目录），
    /// board host 取到任务时把这几个目录传进共用库。要机器在线、board host 会导入；同一台机器同时只做一次
    /// （上一次超过 [`SKILL_IMPORT_STALE_MS`] 还没做完的算作废）；只能选它上报的、没写原因的，board-skills 不能导入
    pub fn host_skill_import(&mut self, name: &str, picks: Vec<SkillImportPick>, by: &str) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if !h.online(self.now) {
            return Err(BoardError::usage(format!("机器「{name}」离线：等它上线再导入")));
        }
        if !h.skill_import.able {
            return Err(BoardError::usage(format!(
                "机器「{name}」的 board（{}）还不会导入 skills：先升级到 {}",
                if h.version.is_empty() { "旧版" } else { &h.version },
                env!("CARGO_PKG_VERSION")
            )));
        }
        if let Some(a) = &h.skill_import.ask {
            if a.at + SKILL_IMPORT_STALE_MS > self.now {
                return Err(BoardError::usage(format!("机器「{name}」正在导入（{} 请求的）：等它做完再来", a.by)));
            }
        }
        let mut seen: Vec<SkillImportPick> = vec![];
        for p in picks {
            let p = SkillImportPick { name: p.name.trim().to_string(), dir: p.dir.trim().to_string() };
            if p.name == crate::skills::BUILTIN {
                return Err(BoardError::usage("board-skills 随服务端版本，不能导入"));
            }
            let Some(l) = h.skill_import.local.iter().find(|l| l.name == p.name && l.dirs.contains(&p.dir)) else {
                return Err(BoardError::usage(format!("机器「{name}」的 {} 里没有 skill「{}」：刷新后再选", p.dir, p.name)));
            };
            if let Some(why) = &l.why {
                return Err(BoardError::usage(format!("{} 不能导入：{why}", p.name)));
            }
            if seen.iter().any(|x| x.name == p.name) {
                return Err(BoardError::usage(format!("skill「{}」选了两份：同名的只能导入一份", p.name)));
            }
            seen.push(p);
        }
        if seen.is_empty() {
            return Err(BoardError::usage("至少选一个要导入的 skill"));
        }
        let by = if by.trim().is_empty() { "人" } else { by.trim() };
        let label = format!("请求导入 skills：{}", seen.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join("、"));
        let mut si = h.skill_import.clone();
        si.ask = Some(SkillImportAsk { id: h.ask_seq + 1, picks: seen, by: by.into(), at: self.now });
        self.tx.execute(
            "UPDATE hosts SET skill_import = ?2, ask_seq = ?3 WHERE id = ?1",
            params![h.id, serde_json::to_string(&si).unwrap_or_default(), h.ask_seq + 1],
        )?;
        self.host_op(h.id, by, &label)?;
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// board host 做完了导入：记下结果（传进去的、没传成的和原因），清掉请求，记一条操作。
    /// 序号对不上（被新的请求换掉、或是别的 board host 进程）时不记
    pub fn host_skill_imported(&mut self, name: &str, instance: &str, id: i64, ok: Vec<String>, failed: Vec<SkillImportFail>) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let Some(ask) = h.skill_import.ask.clone().filter(|a| a.id == id && h.instance == instance) else {
            return self.read().host(name, self.now);
        };
        let failed: Vec<SkillImportFail> = failed.into_iter().map(|f| SkillImportFail { name: f.name, why: f.why.chars().take(300).collect() }).collect();
        let mut si = h.skill_import.clone();
        si.ask = None;
        si.last = Some(SkillImportResult { id, at: self.now, by: ask.by.clone(), ok: ok.clone(), failed: failed.clone() });
        self.tx.execute("UPDATE hosts SET skill_import = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&si).unwrap_or_default()])?;
        let mut text = if ok.is_empty() { "导入 skills 没成".to_string() } else { format!("导入了 skills：{}", ok.join("、")) };
        if !failed.is_empty() {
            text += &format!("；{}", failed.iter().map(|f| format!("{} 没传成（{}）", f.name, f.why)).collect::<Vec<_>>().join("；"));
        }
        self.host_op(h.id, &ask.by, &text)?;
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// board host 做完了自检：换掉这次检查到的项（整台机器时全部换掉，只查一个 agent 时只换它那一项），
    /// 补上 board 版本是否落后于服务端，清掉请求，记一条操作。序号对不上（被新的请求换掉、或是别的 board host 进程）时不记
    pub fn host_selftest_done(&mut self, name: &str, instance: &str, id: i64, items: Vec<SelfTestItem>) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let Some(ask) = h.selftest.ask.clone().filter(|a| a.id == id && h.instance == instance) else {
            return self.read().host(name, self.now);
        };
        let server = env!("CARGO_PKG_VERSION");
        let mut fresh: Vec<SelfTestItem> = items
            .into_iter()
            .filter(|i| matches!(i.kind.as_str(), "agent" | "git"))
            .filter(|i| ask.agent.is_none() || (i.kind == "agent" && Some(&i.key) == ask.agent.as_ref()))
            .map(|mut i| {
                i.at = self.now;
                i.why = i.why.map(|w| w.chars().take(500).collect());
                i.detail = i.detail.map(|d| d.chars().take(2000).collect());
                i
            })
            .collect();
        let behind = version_lt(&h.version, server);
        fresh.push(SelfTestItem {
            kind: "version".into(),
            key: "board".into(),
            ok: !behind,
            why: behind.then(|| format!("board {} 落后于服务端 {server}", if h.version.is_empty() { "（没有上报版本）" } else { &h.version })),
            detail: None,
            at: self.now,
        });
        let mut st = h.selftest.clone();
        st.ask = None;
        if ask.agent.is_none() {
            st.items = fresh.clone();
        } else {
            for f in &fresh {
                match st.items.iter_mut().find(|i| i.kind == f.kind && i.key == f.key) {
                    Some(old) => *old = f.clone(),
                    None => st.items.push(f.clone()),
                }
            }
            // 按本机 agent 的顺序、再 git、再版本
            let order = |i: &SelfTestItem| -> (usize, usize) {
                match i.kind.as_str() {
                    "agent" => (0, h.agents.iter().position(|a| a.id == i.key).unwrap_or(usize::MAX)),
                    "git" => (1, h.dirs.iter().position(|d| d.board == i.key).unwrap_or(usize::MAX)),
                    _ => (2, 0),
                }
            };
            st.items.sort_by_key(order);
        }
        self.tx.execute("UPDATE hosts SET selftest = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&st).unwrap_or_default()])?;
        let bad: Vec<String> = fresh
            .iter()
            .filter(|i| !i.ok)
            .map(|i| match i.kind.as_str() {
                "agent" => h.agents.iter().find(|a| a.id == i.key).map(|a| a.name.clone()).unwrap_or_else(|| i.key.clone()),
                "git" => format!("{} 的 git", i.key),
                _ => "board 版本".into(),
            })
            .collect();
        let text = if bad.is_empty() {
            format!("自检通过（{} 项）", fresh.len())
        } else {
            format!("自检：{} 没通过（共 {} 项）", bad.join("、"), fresh.len())
        };
        self.host_op(h.id, "board host", &text)?;
        self.touch_hosts();
        self.read().host(name, self.now)
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
            if !crate::valid_board(&board) {
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
                return Err(BoardError::usage(format!("最多几个会话要在 1 到 {HOST_MAX_LIMIT} 之间")));
            }
            self.tx.execute("UPDATE hosts SET max = ?2 WHERE id = ?1", params![h.id, max])?;
        }
        if let Some(n) = p.per_session {
            if !(1..=PER_SESSION_LIMIT).contains(&n) {
                return Err(BoardError::usage(format!("一个会话最多做几张要在 1 到 {PER_SESSION_LIMIT} 之间")));
            }
            self.tx.execute("UPDATE hosts SET per_session = ?2 WHERE id = ?1", params![h.id, n])?;
        }
        if let Some(n) = p.max_run {
            if !(0..=MAX_RUN_LIMIT).contains(&n) {
                return Err(BoardError::usage(format!("单次最长运行要在 1 到 {MAX_RUN_LIMIT} 分钟之间，或者不限（0）")));
            }
            self.tx.execute("UPDATE hosts SET max_run = ?2 WHERE id = ?1", params![h.id, n])?;
        }
        if let Some(n) = p.idle_run {
            if !(0..=IDLE_RUN_LIMIT).contains(&n) {
                return Err(BoardError::usage(format!("没输出多久算卡住要在 1 到 {IDLE_RUN_LIMIT} 分钟之间，或者不限（0）")));
            }
            self.tx.execute("UPDATE hosts SET idle_run = ?2 WHERE id = ?1", params![h.id, n])?;
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
            // 恢复接单：排空结束；为升级暂停的也算人恢复了，升级完不再替人改
            if !paused {
                if h.drain_at.is_some() {
                    self.host_op(h.id, by, "恢复接单，排空结束")?;
                }
                self.tx.execute("UPDATE hosts SET drain_at = NULL, up_pause = 0 WHERE id = ?1", [h.id])?;
            }
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
        if let Some(rp) = &p.run {
            let a = h
                .agents
                .iter()
                .find(|a| a.id == rp.agent)
                .ok_or_else(|| BoardError::usage(format!("机器「{name}」上没有 agent「{}」", rp.agent)))?;
            let Some(opts) = &a.opts else {
                return Err(BoardError::usage(format!(
                    "机器「{name}」的 board 是旧版（{}），不能设置模型和思考强度：升级到 {} 后再设",
                    h.version,
                    env!("CARGO_PKG_VERSION")
                )));
            };
            let mut run = h.run.clone();
            let cur = run.entry(rp.agent.clone()).or_default();
            if let Some(m) = &rp.model {
                let m = m.trim();
                if m.is_empty() {
                    cur.model = None;
                } else {
                    if !opts.model {
                        return Err(BoardError::usage(format!("{} 不能在机器设置里选模型", a.name)));
                    }
                    check_model(m).map_err(BoardError::usage)?;
                    cur.model = Some(m.to_string());
                }
            }
            if let Some(e) = &rp.effort {
                let e = e.trim();
                if e.is_empty() {
                    cur.effort = None;
                } else if !opts.efforts.iter().any(|x| x == e) {
                    return Err(BoardError::usage(if opts.efforts.is_empty() {
                        format!("{} 不能在机器设置里选思考强度", a.name)
                    } else {
                        format!("{} 不支持思考强度「{e}」：只能是 {}", a.name, opts.efforts.join("、"))
                    }));
                } else {
                    cur.effort = Some(e.to_string());
                }
            }
            if let Some(pv) = &rp.provider {
                let pv = pv.trim();
                if pv.is_empty() {
                    cur.provider = None;
                } else if pv == crate::providers::PROVIDER_LOCAL {
                    cur.provider = Some(crate::providers::PROVIDER_LOCAL.into());
                } else {
                    let known = self.read().providers()?.into_iter().any(|p| p.name == pv);
                    if !known {
                        return Err(BoardError::usage(format!("没有模型供应商「{pv}」")));
                    }
                    cur.provider = Some(pv.to_string());
                }
            }
            self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&run).unwrap_or_default()])?;
        }
        if let Some(e) = &p.env {
            let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
            let check = h.env_check(&e.agent, &e.env)?;
            let mut run = h.run.clone();
            run.entry(e.agent.clone()).or_default().env = Some(check.env);
            self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![h.id, serde_json::to_string(&run).unwrap_or_default()])?;
        }
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// 复制机器设置（PRD §10.12「复制设置到…」）：把 `from` 的这几项换到每台目标机器上，没选的项不变。
    /// 照原样复制、不按目标机器校验：目标机器上找不到的 agent、没有的启动环境、项目里没有的标签照样写进去，界面按现有规则标「不生效」。
    /// 每台目标机器记一条操作记录
    pub fn host_copy(&mut self, from: &str, to: &[String], items: &[String], by: &str) -> Result<Vec<HostInfo>> {
        let src = self.host_row(from)?.ok_or_else(|| no_such_host(from))?;
        if items.is_empty() {
            return Err(BoardError::usage("至少选一项要复制的设置"));
        }
        if let Some(bad) = items.iter().find(|i| !COPY_ITEMS.iter().any(|(k, _)| k == i)) {
            return Err(BoardError::usage(format!("不认识要复制的设置「{bad}」：只能是 {}", COPY_ITEMS.map(|(k, _)| k).join("、"))));
        }
        let mut targets: Vec<HostRow> = vec![];
        for name in to {
            if name == from {
                return Err(BoardError::usage("不能复制到它自己"));
            }
            let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
            if !targets.iter().any(|t| t.id == h.id) {
                targets.push(h);
            }
        }
        if targets.is_empty() {
            return Err(BoardError::usage("至少选一台目标机器"));
        }
        let rules = serde_json::to_string(&src.rules.iter().map(ClaimRule::stored).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
        let cron = serde_json::to_string(&src.cron).unwrap_or_else(|_| "[]".into());
        let words: Vec<&str> = COPY_ITEMS.iter().filter(|(k, _)| items.iter().any(|i| i == k)).map(|(_, w)| *w).collect();
        let mut out = vec![];
        for t in &targets {
            for item in items {
                match item.as_str() {
                    "limits" => {
                        self.tx.execute("UPDATE hosts SET max = ?2, per_session = ?3 WHERE id = ?1", params![t.id, src.max, src.per_session])?;
                    }
                    "cron" => {
                        self.tx.execute("UPDATE hosts SET cron_on = ?2, cron = ?3 WHERE id = ?1", params![t.id, src.cron_on as i64, cron])?;
                    }
                    "rules" => {
                        self.tx.execute("UPDATE hosts SET rules = ?2, fallback = ?3 WHERE id = ?1", params![t.id, rules, src.fallback])?;
                    }
                    "agent" => {
                        self.tx.execute("UPDATE hosts SET agent = ?2 WHERE id = ?1", params![t.id, src.agent])?;
                    }
                    "run" => {
                        // 每个 agent 的启动环境、模型、强度：源机器设置过的 agent 换成它的；源机器没设置的保持目标机器原来的
                        let mut run = t.run.clone();
                        for (k, v) in &src.run {
                            run.insert(k.clone(), v.clone());
                        }
                        self.tx.execute("UPDATE hosts SET run = ?2 WHERE id = ?1", params![t.id, serde_json::to_string(&run).unwrap_or_default()])?;
                    }
                    _ => {}
                }
            }
            self.host_op(t.id, by, &format!("复制了 {from} 的设置：{}", words.join("、")))?;
        }
        self.touch_hosts();
        for t in &targets {
            out.push(self.read().host(&t.name, self.now)?);
        }
        Ok(out)
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

    /// 移除机器（任务下发需求 §九），不吊销令牌：见 [`Self::host_remove_revoke`]。
    /// 服务端删除机器都走 `host_remove_revoke`（带不带吊销由请求决定），只有测试用不吊销的这一个
    #[cfg(test)]
    pub fn host_remove(&mut self, name: &str, by: &Actor, who: &str) -> Result<()> {
        self.host_remove_revoke(name, by, who, false).map(|_| ())
    }

    /// 移除机器（任务下发需求 §九）：取消它所有排队中的派发，停止正在跑的派发，删除机器设置；派发记录保留。
    /// revoke 时同时吊销它最近一次上报用的令牌（只在别的在线机器都不用这个令牌时），
    /// 返回吊销的令牌 ID
    pub fn host_remove_revoke(&mut self, name: &str, by: &Actor, who: &str, revoke: bool) -> Result<Option<i64>> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let mut revoked = None;
        if revoke {
            let t = self.read().host_token(&h, self.now)?;
            let Some(t) = t else {
                return Err(BoardError::usage(format!("机器「{name}」没有能吊销的令牌（还没上报过，或令牌已经吊销）")));
            };
            if !t.shared.is_empty() {
                return Err(BoardError::usage(format!("令牌「{}」还在被在线的机器 {} 使用，不能一起吊销", t.name, t.shared.join("、"))));
            }
            self.tx.execute("DELETE FROM auth_tokens WHERE id = ?1", [t.id])?;
            revoked = Some(t.id);
        }
        // 后派的先回去：卡片回到可领取状态最上面后仍按派发的先后排
        let queued: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE host_id = ?1 AND state = 'queued' ORDER BY id DESC", [h.id], |r| r.get(0))?;
        for id in queued {
            let d = self.drow(id)?;
            self.close_dispatch(&d, "canceled", None)?;
            let back = self.unreserve(d.board, d.card, by)?;
            let text = format!("移除了机器 {}，派发取消（{}）{back}", h.name, Self::dispatch_words(&d));
            self.track_dispatch(d.board, d.card, by, "dispatch", &text)?;
        }
        let running: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE host = ?1 AND state = 'running'", [name], |r| r.get(0))?;
        for id in running {
            self.request_stop(id, by, who)?;
        }
        self.tx.execute("DELETE FROM hosts WHERE id = ?1", [h.id])?;
        self.tx.execute("DELETE FROM host_ops WHERE host_id = ?1", [h.id])?;
        self.touch_hosts();
        Ok(revoked)
    }

    /// 机器页顶部的「全部暂停 / 全部恢复」：所有机器一起设成暂停接单（或恢复），排队的派发照样排队。
    /// 返回所有机器
    pub fn hosts_pause_all(&mut self, paused: bool) -> Result<Vec<HostInfo>> {
        self.tx.execute("UPDATE hosts SET paused = ?1", [paused as i64])?;
        if paused {
            // 暂停接单时这一轮就结束（与单台机器一样，PRD §9.1）
            self.tx.execute("UPDATE hosts SET round_at = NULL WHERE round_at IS NOT NULL", [])?;
        }
        self.touch_hosts();
        self.read().hosts(self.now)
    }

    /// 机器页顶部的「全部停止」：请求停止所有正在跑的派发，同时全部暂停（界面二次确认后调用）。
    /// 各台机器的 board host 收到停止请求后结束 agent 进程树并上报；排队的派发继续排队
    pub fn hosts_stop_all(&mut self, by: &Actor, who: &str) -> Result<Vec<HostInfo>> {
        let running: Vec<i64> = self.collect("SELECT id FROM dispatches WHERE state = 'running' ORDER BY id", [], |r| r.get(0))?;
        for id in running {
            self.request_stop(id, by, who)?;
        }
        self.hosts_pause_all(true)
    }

    /// 机器改名（桌面端「设置 → 本机」保存机器名时调用）：机器记录原地改名，id 不变，
    /// 所有设置（最多几个会话、默认 agent、领取规则、定时领取、暂停 …）和历史派发记录都保留，
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
            // 定时建卡按机器名派发：跟着改名
            self.tx.execute("UPDATE card_crons SET host = ?2 WHERE host = ?1", params![h.name, to])?;
            self.touch_hosts();
        }
        self.read().host(to, self.now)
    }

    /* ---------------- 派发 ---------------- */

    /// 手动派发（任务下发需求 §五）：校验通过后进入排队。agent 为空时用这台机器的默认 agent
    pub fn dispatch_create(&mut self, board: &BoardRef, card: i64, host: &str, agent: Option<&str>, by: &Actor) -> Result<Dispatch> {
        self.dispatch_create_from(board, card, host, agent, by, "manual")
    }

    /// 定时建卡保存前看它要派的机器：机器存在、有这块白板的目录，指定的 agent 在这台机器上（找没找到命令到点时再看）。
    /// 不合适时 `USAGE`，写明哪里不对
    pub(crate) fn dispatch_target_check(&self, board: &BoardRef, host: &str, agent: Option<&str>) -> Result<()> {
        let h = self.host_row(host)?.ok_or_else(|| BoardError::usage(format!("没有叫「{host}」的机器：界面「机器」里能看到所有机器")))?;
        let path = board.path();
        if !h.dirs.iter().any(|d| d.board == path) {
            return Err(BoardError::usage(format!(
                "机器「{host}」没有白板 {path} 的目录：先在机器详情的「白板与目录」里加上，或者那台机器的 board host 启动时用 --dir {path}=<路径>"
            )));
        }
        if let Some(a) = agent {
            if !h.agents.iter().any(|x| x.id == a) {
                let all: Vec<String> = h.agents.iter().map(|x| x.id.clone()).collect();
                let all = if all.is_empty() { "（这台机器还没上报 agent）".to_string() } else { all.join("、") };
                return Err(BoardError::usage(format!("机器「{host}」上没有 agent「{a}」，可选：{all}")));
            }
        }
        Ok(())
    }

    /// 派发一张卡（手动派发、定时建卡共用）：source 是 manual 或 [`SOURCE_SCHEDULE`]
    pub(crate) fn dispatch_create_from(&mut self, board: &BoardRef, card: i64, host: &str, agent: Option<&str>, by: &Actor, source: &str) -> Result<Dispatch> {
        let flow = flow_of(self.tx, board.id)?;
        let col: Option<String> =
            self.tx.query_row("SELECT col FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![board.id, card], |r| r.get(0)).optional()?;
        let col = col.ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("卡片 #{card} 不存在")))?;
        // 先看派发：排队的卡片已经在锁定状态里，报「已经派发」比「不在可领取状态」清楚
        if let Some(d) = self.read().card_dispatch(board, card)? {
            let what = if d.state == "queued" { "排队" } else { "在跑" };
            return Err(BoardError::new(
                ErrorCode::AlreadyDispatched,
                format!("#{card} 已经派发给 {} · {}，正在{what}：先取消或停止它", d.host, d.agent_label),
            )
            .with_detail(json!({ "dispatch": d.id })));
        }
        // 「未整理」里的条目：派给 agent 拆解（不领取、不移动）
        let split = col == INBOX;
        if !split && !flow.is_claim(&col) {
            let names: Vec<String> = flow.0.iter().filter(|s| s.claim).map(|s| format!("「{}」", s.name)).collect();
            return Err(BoardError::new(
                ErrorCode::NotInTodo,
                format!("#{card} 在「{}」，不在可领取的状态{}，不能派发（「{INBOX_NAME}」里的条目可以派给 agent 拆解）", flow.name(&col), names.join("")),
            ));
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
        match self.read().dir_readiness(&h, board)? {
            Readiness::Failed(last) => {
                return Err(BoardError::new(ErrorCode::BoardNotReady, format!("机器「{host}」上 {path} 的环境检查没通过：{last}"))
                    .with_detail(json!({ "host": host, "board": path })));
            }
            Readiness::Pending => {
                return Err(BoardError::new(
                    ErrorCode::BoardNotReady,
                    format!("机器「{host}」还没跑完 {path} 的环境检查：等它跑完再派（旧版 board host 不会检查，要先升级 board）"),
                )
                .with_detail(json!({ "host": host, "board": path })));
            }
            Readiness::NoCheck | Readiness::Passed => {}
        }
        let (agent_id, label) = (a.id.clone(), a.name.clone());
        let kind = if split { KIND_SPLIT } else { "" };
        self.insert_dispatch(board.id, card, &h, &agent_id, &label, source, by, kind)
    }

    /// 记一条排队的派发。手动派发、定时建卡派发（source 是 manual、schedule）的卡片进入领取时会进的锁定状态排队
    /// （[`W::reserve`]）；定时领取（cron）的第一张马上开始，不经过排队；拆解（kind 是 split）不动卡片
    #[allow(clippy::too_many_arguments)]
    fn insert_dispatch(&mut self, b: i64, card: i64, h: &HostRow, agent: &str, label: &str, source: &str, by: &Actor, kind: &str) -> Result<Dispatch> {
        let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![b, card], |r| r.get(0))?;
        self.tx.execute(
            "INSERT INTO dispatches (board, card, title, host_id, host, agent, agent_label, source, by_kind, by_name, state, created_at, kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'queued', ?11, ?12)",
            params![b, card, title, h.id, h.name, agent, label, source, by.kind.as_str(), by.name, self.now, kind],
        )?;
        let id = self.tx.last_insert_rowid();
        let split = kind == KIND_SPLIT;
        let reserve = source != "cron" && !split;
        let into = if reserve { self.reserve(b, card, by)? } else { None };
        let at = into.map(|n| format!("在「{n}」")).unwrap_or_default();
        let wait = if !h.online(self.now) {
            format!("，{at}排队等机器上线")
        } else if h.paused {
            format!("，{at}排队（机器暂停接单）")
        } else {
            format!("，{at}排队")
        };
        let what = if split { format!("派给 {} · {} 拆解", h.name, label) } else { format!("派发到 {} · {}", h.name, label) };
        self.track(b, card, by, "dispatch", &format!("{what}（{}）{wait}", source_words(source)))?;
        self.touch_hosts();
        self.read().dispatch(id)
    }

    /// 取消排队中的派发，或请求停止正在跑的派发（board host 结束 agent 进程后上报结束）。who 是给人看的称呼
    pub fn dispatch_cancel(&mut self, id: i64, by: &Actor, who: &str) -> Result<Dispatch> {
        let d = self.drow(id)?;
        match d.state.as_str() {
            "queued" => {
                // 记下是谁取消的：会话结束时自动取消的排队这一轮还能重排，人取消的不再排（见 dispatch_pull 的 skip）
                self.tx.execute("UPDATE dispatches SET stop_by = ?2 WHERE id = ?1", params![id, who])?;
                self.close_dispatch(&d, "canceled", None)?;
                let back = self.unreserve(d.board, d.card, by)?;
                self.track_dispatch(d.board, d.card, by, "dispatch", &format!("取消了派发（{}）{back}", Self::dispatch_words(&d)))?;
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

    /// 人把排队的卡片移走（拖出它排队的锁定状态，或拖出可领取状态）：排队中的派发自动取消（规则引擎调用，卡片已经放到新位置）
    pub(crate) fn cancel_queued_on_move(&mut self, b: i64, card: i64, by: &Actor) -> Result<()> {
        if let Some((id, _)) = self.queued_dispatch(b, card)? {
            let d = self.drow(id)?;
            self.close_dispatch(&d, "canceled", None)?;
            // 排队时记下的原来的可领取状态：卡片已经被人移走，不再用它
            self.tx.execute("UPDATE cards SET claimed_from = NULL WHERE board = ?1 AND id = ?2 AND holder IS NULL", params![b, card])?;
            self.track(b, card, by, "dispatch", &format!("卡片被移走，排队的派发自动取消（{}）", Self::dispatch_words(&d)))?;
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

    /// 这台机器现在开着几个会话（= 同时在跑的几个 agent 进程）。`max` 限制的是会话数，不是派发条数：
    /// 定时领取的会话里排着的派发（`session != id`）不占额外的会话位，它们由同一个进程用 `board next` 接着做；
    /// 手动派发的一张卡各自是一个会话（`session` 为空或等于自己）
    fn sessions_on(&self, h: &HostRow) -> Result<i64> {
        Ok(self.tx.query_row(
            "SELECT COUNT(DISTINCT COALESCE(session, id)) FROM dispatches WHERE host = ?1 AND state = 'running'",
            [&h.name],
            |r| r.get(0),
        )?)
    }

    /// 开始一次排队中的派发（board host 有空位时按创建先后调用）：机器在线、没有暂停、有空位才开始
    /// （「空位」是会话位：同时在跑的几个 agent 进程还没到「最多几个会话」）
    pub fn dispatch_begin(&mut self, host: &str, instance: &str, id: i64, token: Option<i64>) -> Result<Dispatch> {
        let h = self.host_of(host, instance)?;
        let d = self.drow(id)?;
        if d.state != "queued" || d.host_id != h.id {
            return Err(BoardError::usage(format!("派发 #{id} 不在机器「{host}」的排队里")));
        }
        if d.session != d.id {
            return Err(BoardError::usage(format!("派发 #{id} 排在会话里，由那个会话的 board next 开始")));
        }
        if h.paused {
            return Err(BoardError::usage(format!("机器「{host}」暂停接单")));
        }
        if h.restarting() {
            return Err(BoardError::usage(format!("机器「{host}」正在等重启或升级，不开始新的派发")));
        }
        if self.sessions_on(&h)? >= h.max {
            return Err(BoardError::usage(format!("机器「{host}」没有空位（最多 {} 个会话同时跑）", h.max)));
        }
        self.start(&h, d, token)
    }

    /// `<agent 标识>-<机器名>/<模型名>` 这个名字没被在线 agent 用就返回它，否则按 [`dispatch_agent_name`] 的规则
    /// 依次试模型名后面的 `-2`、`-3` …（只有在线的名字算占用）。派发开始时和 `board check --agent <标识>` 都用它
    pub(crate) fn free_agent_name(&self, agent: &str, host: &str, model: &str) -> Result<String> {
        let gap = crate::agents::NAME_GAP_MS;
        let mut seq = 0u32;
        loop {
            let n = dispatch_agent_name(agent, host, model, seq);
            let taken: bool = self.tx.query_row(
                "SELECT COUNT(*) > 0 FROM agents WHERE name = ?1 AND off_at IS NULL AND beat_at + ?3 > ?2",
                params![n, self.now, gap],
                |r| r.get(0),
            )?;
            if !taken {
                return Ok(n);
            }
            // 不带序号的被占用：依次试 -2、-3 …
            seq = if seq == 0 { 2 } else { seq + 1 };
        }
    }

    /// 生成名字、check、以它领取卡片。领不到时这次派发结束，结果「没领到」
    fn start(&mut self, h: &HostRow, d: DRow, token: Option<i64>) -> Result<Dispatch> {
        // 名字：<agent 标识>-<机器名>/<模型名>（dsh-win/deepseek-v4.1-flash），见 free_agent_name。
        // 模型名来自机器设置里选的模型、board host 上报的 agent 数据（配置文件 [agents.<标识>] model，内置模板有默认值），
        // 都没有时用 agent 标识
        let (run_model, run_effort) = h.run_opts(&d.agent);
        let name = self.free_agent_name(&d.agent, &h.name, h.model_for(&d.agent).0.as_str())?;
        // 用哪个启动环境：按开始这一刻的机器设置
        let env = h.agents.iter().find(|a| a.id == d.agent).and_then(|a| h.pick(a)).map(|c| c.env.clone());
        let env_name = env.as_deref().map(|e| h.env_name(e));
        self.tx.execute(
            "UPDATE dispatches SET name = ?2, env = ?3, env_name = ?4, model = ?5, effort = ?6 WHERE id = ?1",
            params![d.id, name, env, env_name, run_model, run_effort],
        )?;
        let d = DRow { name: Some(name.clone()), env_name: env_name.clone(), model: run_model.clone(), effort: run_effort.clone(), ..d };
        // 这个名字属于这台 board host：派发会话里的 agent 自己 heartbeat stop 也不会让它离线
        self.agent_check_owned(&name, token, crate::agents::NameOwner::Host)?;
        let board = self.board_of(d.board)?;
        let actor = Actor::agent(name.clone());
        // 拆解：不领取、不移动这一条，名字上线就开始（host 替它发心跳）
        if d.split() {
            self.tx.execute("UPDATE dispatches SET state = 'running', started_at = ?2, session = ?1 WHERE id = ?1", params![d.id, self.now])?;
            self.touch_hosts();
            let who = match &env_name {
                Some(e) => format!("{}（{e}）", d.agent_label),
                None => format!("{} ", d.agent_label),
            };
            let text = format!("拆解开始：{} 上的 {who}以 {name} 拆解这一条（{}{}）", h.name, source_words(&d.source), opts_words(&d.model, &d.effort));
            self.track_dispatch(d.board, d.card, &actor, "dispatch", &text)?;
            return self.read().dispatch(d.id);
        }
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
                let text = format!("派发开始：{} 上的 {who}以 {name} 领取（{}{}）", h.name, source_words(&d.source), opts_words(&d.model, &d.effort));
                self.track(d.board, d.card, &actor, "dispatch", &text)?;
            }
            Err(e) if e.code == ErrorCode::Internal => return Err(e),
            Err(e) => {
                self.close_dispatch(&d, "unclaimed", None)?;
                let back = self.unreserve(d.board, d.card, &Actor::system())?;
                let text = format!("派发没领到：{}（{}）{back}", e.message, Self::dispatch_words(&d));
                self.track_dispatch(d.board, d.card, &Actor::system(), "dispatch", &text)?;
            }
        }
        self.read().dispatch(d.id)
    }

    /// 定时领取（任务下发需求 §六）：正在一轮领取、没有暂停、有空位（会话位没满）时，在这台机器有目录的白板上按 --dir 的顺序轮流，
    /// 按领取规则挑下一张卡片和它的 agent。一条领取规则也没有时不领卡（这一轮直接结束，返回 None）。
    /// 规则没命中的卡片、也没有可领取的卡片时这一轮结束，返回 None
    pub fn dispatch_pull(&mut self, host: &str, instance: &str, token: Option<i64>) -> Result<Option<Dispatch>> {
        let h = self.host_of(host, instance)?;
        if h.round_at.is_none() || h.paused || h.restarting() || self.sessions_on(&h)? >= h.max {
            return Ok(None);
        }
        let Some(rules) = h.claim_rules() else {
            return self.end_round(&h);
        };
        let n = h.dirs.len();
        for k in 0..n {
            let i = (h.rr.max(0) as usize + k) % n;
            let Ok(board) = self.resolve(&h.dirs[i].board) else { continue };
            // 环境检查没通过（或还没检查）的白板这台机器不接
            if !self.read().dir_readiness(&h, &board)?.ok() {
                continue;
            }
            let flow = flow_of(self.tx, board.id)?;
            // 这一轮里这台机器已经派发过的卡片不再领：派发异常退出/被停止后卡片 50 秒会回到可领取状态最上面，
            // 不跳的话同一个空位会反复领到它、白跑 agent。下一轮（round_at 更新后）可以正常再领。
            // 会话结束时还没轮到、被自动取消的排队不算（stop_by 为空；人取消的记了是谁）：新开的会话可以把它们重新排进去
            let round_at = h.round_at.unwrap_or(0);
            let skip: Vec<i64> = self.collect(
                "SELECT DISTINCT card FROM dispatches WHERE board = ?1 AND host_id = ?2 AND created_at >= ?3
                   AND NOT (state = 'done' AND result = 'canceled' AND started_at IS NULL AND stop_by IS NULL AND session IS NOT NULL AND session != id)",
                params![board.id, h.id, round_at],
                |r| r.get(0),
            )?;
            // 规则会给它派一个这台机器上找得到的 agent 的卡片，按规则 1 的先后（带优先标签的在前）取第一张
            let found = h.resolved();
            let mut hit: Option<(i64, HostAgent)> = None;
            let urgent = self.priority_cards(board.id)?;
            let mut order = self.claimable_order(board.id, &flow, &skip)?;
            order.sort_by_key(|(id, _)| !urgent.contains(id));
            for (id, _) in order {
                let Some(want) = rules.route(&card_label_names(self.tx, board.id, id)?) else { continue };
                if let Some(a) = found.iter().find(|x| x.id == want) {
                    hit = Some((id, a.clone()));
                    break;
                }
            }
            let Some((card, agent)) = hit else { continue };
            self.tx.execute("UPDATE hosts SET rr = ?2 WHERE id = ?1", params![h.id, ((i + 1) % n) as i64])?;
            let d = self.insert_dispatch(board.id, card, &h, &agent.id, &agent.name, "cron", &Actor::system(), "")?;
            let d = self.start(&h, self.drow(d.id)?, token)?;
            if d.state == "running" {
                // 开会话时把这个会话要做的卡片一次排好：先把一个会话排满，再开下一个
                self.plan_session(&h, &rules, &d, &board, &flow, &skip)?;
                return self.read().dispatch(d.id).map(Some);
            }
        }
        // 还有会话排着卡片时这一轮不算结束：会话中途退出时它们被取消，要能由新开的会话重新排
        let pending: i64 = self.tx.query_row(
            "SELECT COUNT(*) FROM dispatches WHERE host_id = ?1 AND state = 'queued' AND session IS NOT NULL",
            [h.id],
            |r| r.get(0),
        )?;
        if pending > 0 {
            return Ok(None);
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

    /// 派发结束（任务下发需求 §七「结束」）：board host 在 agent 进程退出后上报退出码和日志最后 20 行。
    /// `why` 只在 `how = timeout` 时有：写给人看的原因（「运行超过 3 小时…」）
    pub fn dispatch_end(&mut self, id: i64, exit_code: Option<i64>, tail: &[String], how: EndHow, why: Option<&str>) -> Result<Dispatch> {
        self.dispatch_end_with(id, exit_code, tail, &[], how, why)
    }

    /// 同 [`W::dispatch_end`]，另外带上最后几个事件的文字（runlog.rs `RunEvent::line`）：异常退出、超时的评论里写它们，
    /// 原始输出的最后 20 行折叠在后面。旧版 board host 不报事件，评论照旧写「日志最后 20 行」
    pub fn dispatch_end_with(&mut self, id: i64, exit_code: Option<i64>, tail: &[String], events: &[String], how: EndHow, why: Option<&str>) -> Result<Dispatch> {
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
                EndHow::Timeout => Finish::Timeout(why.map(str::trim).filter(|w| !w.is_empty())),
            };
            self.finish(&d, exit_code, &Tail { lines: tail, events }, f)?;
        }
        self.read().dispatch(id)
    }

    fn finish(&mut self, d: &DRow, exit_code: Option<i64>, tail: &Tail, how: Finish) -> Result<()> {
        // 超时按异常退出的规则处理，但原因写在评论里，结果记「超时」
        let because = match how {
            Finish::Timeout(why) => Some(why.map(str::to_string).unwrap_or_else(|| "board host 按机器的兜底设置在超时后停止了它".into())),
            _ => None,
        };
        let Some(name) = d.name.clone() else {
            return self.close_dispatch(d, if because.is_some() { "timeout" } else { "crashed" }, exit_code);
        };
        let actor = Actor::agent(name.clone());
        let stop_by = match how {
            Finish::HostExit => Some(format!("board host（{}）", d.host)),
            _ => d.stop_by.clone(),
        };
        if d.split() {
            return self.finish_split(d, &actor, exit_code, tail, how, because, stop_by);
        }
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
                // 超时：和 final 一样结束这次计时，但记成「超时」，大盘里算异常退出（§10.13 按 agent）
                match how {
                    Finish::Timeout(_) => soft(self.timeout_beat(d.board, d.card, &actor))?,
                    _ => soft(self.exec(&board, &actor, Cmd::Beat { id: d.card, done: true }))?,
                }
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
                    (_, Finish::Timeout(_)) => {
                        let mut t = format!("{}，board host 停止了它", because.clone().unwrap_or_default());
                        t += &tail.words();
                        t
                    }
                    (_, Finish::HostExit) => format!("board host（{}）退出，停止了这次派发", d.host),
                    (Some(who), _) => format!("{who} 停止了这次派发"),
                    (None, Finish::Missing) => {
                        "agent 异常退出（board host 上没有这次派发的 agent 进程：board host 重启过，或开始派发时连接中断）".to_string()
                    }
                    (None, _) => {
                        let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into());
                        let mut t = format!("agent 异常退出（退出码 {code}）");
                        t += &tail.words();
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
                if because.is_some() {
                    "timeout"
                } else if stop_by.is_some() {
                    "stopped"
                } else {
                    "crashed"
                }
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
        // 进程不在了：这个会话还排着的取消
        let why = match (&stop_by, how) {
            (_, Finish::HostExit) => format!("board host（{}）退出", d.host),
            (Some(who), _) => format!("{who} 停止了这个会话"),
            _ => "agent 进程退出".to_string(),
        };
        self.cancel_session_rest(d.session, &why)
    }

    /// 拆解结束（PRD §9.1）：卡片在任何情况下都不动。退出码 0 时按这次派发期间建了几张「拆自」是这条的卡记「已拆解」或「没拆出卡」；
    /// 退出码非 0（或 board host 上没有进程）记「异常退出」，人停止、board host 退出、超时记「已停止」，都在这一条上评论原因
    #[allow(clippy::too_many_arguments)]
    fn finish_split(&mut self, d: &DRow, actor: &Actor, exit_code: Option<i64>, tail: &Tail, how: Finish, because: Option<String>, stop_by: Option<String>) -> Result<()> {
        let since = d.started_at.unwrap_or(self.now);
        let made: i64 = self.tx.query_row(
            "SELECT COUNT(*) FROM cards WHERE board = ?1 AND split_from = ?2 AND created_at >= ?3",
            params![d.board, d.card, since],
            |r| r.get(0),
        )?;
        let (result, why) = match (how, &stop_by) {
            (Finish::Timeout(_), _) => ("stopped", Some(format!("{}，board host 停止了这次拆解{}", because.unwrap_or_default(), tail.words()))),
            (Finish::HostExit, _) => ("stopped", Some(format!("board host（{}）退出，停止了这次拆解", d.host))),
            (_, Some(who)) => ("stopped", Some(format!("{who} 停止了这次拆解"))),
            (Finish::Missing, None) => (
                "crashed",
                Some("拆解的 agent 异常退出（board host 上没有这次派发的 agent 进程：board host 重启过，或开始派发时连接中断）".to_string()),
            ),
            (Finish::Exit, None) if exit_code == Some(0) => (if made > 0 { "split" } else { "nosplit" }, None),
            (Finish::Exit, None) => {
                let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into());
                ("crashed", Some(format!("拆解的 agent 异常退出（退出码 {code}）{}", tail.words())))
            }
        };
        if let Some(text) = why {
            let board = self.board_of(d.board)?;
            soft(self.exec(&board, actor, Cmd::Comment { id: d.card, text }))?;
        }
        self.close_dispatch(d, result, exit_code)?;
        let took = d.started_at.map(|t| format!("，用时 {}", crate::engine::dur_words(self.now - t))).unwrap_or_default();
        let code = exit_code.map(|c| format!("，退出码 {c}")).unwrap_or_default();
        let made = if made > 0 { format!("，拆出 {made} 条") } else { String::new() };
        let text = format!("拆解结束：{}（{}{took}{code}{made}）", result_words(result), Self::dispatch_words(d));
        self.track_dispatch(d.board, d.card, actor, "dispatch", &text)?;
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

    /// 追加结构化事件（runlog.rs）：`fmt` 是 agent 的输出格式（空 = 只有原始输出），每次都带上，覆盖原来的。
    /// 工具结果合进同一个调用先到的那一条；单条超过 4 KB 截断；超过 2000 条删最早的，记下删了几条。
    /// 不算变更（不推整个工作区），只把 meta.runs 加一：实时推送带上它和有新事件的派发，打开着的会话据此马上取
    pub fn dispatch_events_append(&mut self, id: i64, fmt: Option<&str>, events: &[crate::runlog::RunEvent]) -> Result<usize> {
        use crate::runlog::{RunEvent, EVENTS_KEEP, KINDS};
        let exists: Option<i64> = self.tx.query_row("SELECT id FROM dispatches WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        if exists.is_none() {
            return Err(BoardError::new(ErrorCode::NotFound, format!("派发 #{id} 不存在")));
        }
        if let Some(f) = fmt {
            let f: String = f.trim().chars().take(32).collect();
            self.tx.execute("UPDATE dispatches SET fmt = ?2 WHERE id = ?1 AND fmt IS NOT ?2", params![id, f])?;
        }
        if events.is_empty() {
            return Ok(0);
        }
        self.tx.execute("INSERT INTO meta (key, value) VALUES ('runs', 1) ON CONFLICT(key) DO UPDATE SET value = value + 1", [])?;
        let rev: i64 = self.tx.query_row("SELECT value FROM meta WHERE key = 'runs'", [], |r| r.get(0))?;
        let mut n = 0;
        for e in events {
            let mut e = e.clone();
            if !KINDS.contains(&e.kind.as_str()) {
                e.kind = "text".into();
            }
            e.id = 0;
            if e.at <= 0 {
                e.at = self.now;
            }
            let call = e.call.take().filter(|c| !c.is_empty()).map(|c| c.chars().take(200).collect::<String>());
            if e.kind == "tool" {
                if let Some(c) = call.as_deref() {
                    let old: Option<(i64, String)> = self
                        .tx
                        .query_row(
                            "SELECT id, data FROM dispatch_events WHERE dispatch = ?1 AND call = ?2 ORDER BY id DESC LIMIT 1",
                            params![id, c],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    if let Some((eid, data)) = old {
                        let mut prev: RunEvent = serde_json::from_str(&data).unwrap_or_else(|_| RunEvent::new("tool"));
                        prev.merge(&e);
                        prev.fit();
                        let data = serde_json::to_string(&prev).unwrap_or_default();
                        self.tx.execute("UPDATE dispatch_events SET data = ?2, rev = ?3 WHERE id = ?1", params![eid, data, rev])?;
                        n += 1;
                        continue;
                    }
                }
            }
            e.fit();
            let data = serde_json::to_string(&e).unwrap_or_default();
            self.tx.execute(
                "INSERT INTO dispatch_events (dispatch, kind, call, rev, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, e.kind, call, rev, data],
            )?;
            n += 1;
        }
        let total: i64 = self.tx.query_row("SELECT COUNT(*) FROM dispatch_events WHERE dispatch = ?1", [id], |r| r.get(0))?;
        if total > EVENTS_KEEP {
            let drop = total - EVENTS_KEEP;
            self.tx.execute(
                "DELETE FROM dispatch_events WHERE id IN (SELECT id FROM dispatch_events WHERE dispatch = ?1 ORDER BY id LIMIT ?2)",
                params![id, drop],
            )?;
            self.tx.execute("UPDATE dispatches SET ev_dropped = ev_dropped + ?2 WHERE id = ?1", params![id, drop])?;
        }
        Ok(n)
    }

    /// 结束超过 30 天的派发只留 `result` 事件和最后 50 条（服务端每 10 分钟一次，每次最多处理 200 个派发）。返回删了几条
    pub fn prune_run_events(&mut self) -> Result<usize> {
        use crate::runlog::{EVENTS_OLD_KEEP, EVENTS_OLD_MS};
        let ids: Vec<i64> = self.collect(
            "SELECT id FROM dispatches WHERE state = 'done' AND ev_old = 0 AND ended_at IS NOT NULL AND ended_at <= ?1 ORDER BY id LIMIT 200",
            [self.now - EVENTS_OLD_MS],
            |r| r.get(0),
        )?;
        let mut total = 0;
        for id in ids {
            let n = self.tx.execute(
                "DELETE FROM dispatch_events WHERE dispatch = ?1 AND kind != 'result' \
                 AND id NOT IN (SELECT id FROM dispatch_events WHERE dispatch = ?1 ORDER BY id DESC LIMIT ?2)",
                params![id, EVENTS_OLD_KEEP],
            )?;
            self.tx.execute("UPDATE dispatches SET ev_old = 1, ev_dropped = ev_dropped + ?2 WHERE id = ?1", params![id, n as i64])?;
            total += n;
        }
        Ok(total)
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
            self.cancel_session_rest(d.session, "机器掉线")?;
        }
        Ok(n + lost.len())
    }
}

/// 异常退出、超时时 board host 报的最后的输出：原始输出最后 20 行，和最后 10 个事件的文字（旧版 board host 没有）
struct Tail<'a> {
    lines: &'a [String],
    events: &'a [String],
}

impl Tail<'_> {
    /// 评论里接在原因后面的部分：有事件时写「最后 N 个事件」，原始输出折叠在后面；没有事件时照旧写「日志最后 N 行」
    fn words(&self) -> String {
        let fence = |ls: &[String]| format!("```\n{}\n```", ls.join("\n").replace("```", "ˋˋˋ"));
        let events: Vec<String> = self.events.iter().filter(|l| !l.trim().is_empty()).cloned().collect();
        if events.is_empty() {
            if self.lines.is_empty() {
                return String::new();
            }
            return format!("\n\n日志最后 {} 行：\n\n{}", self.lines.len(), fence(self.lines));
        }
        let mut t = format!("\n\n最后 {} 个事件：\n\n{}", events.len(), fence(&events));
        if !self.lines.is_empty() {
            t += &format!("\n\n<details><summary>原始输出最后 {} 行</summary>\n\n{}\n\n</details>", self.lines.len(), fence(self.lines));
        }
        t
    }
}

/// 派发是怎么结束的
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish<'a> {
    /// agent 进程退出
    Exit,
    /// board host 退出前结束了 agent 进程
    HostExit,
    /// board host 按机器的兜底设置结束了 agent 进程（运行超时 / 长时间没有输出）；带给人看的原因
    Timeout(Option<&'a str>),
    /// board host 重启后，本机没有这次派发的进程
    Missing,
}

/// `board next` 的 SESSION_DONE：这个会话不再接着做
fn session_done_error(why: &str) -> BoardError {
    BoardError::new(ErrorCode::SessionDone, format!("{why}：结束这个会话"))
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
