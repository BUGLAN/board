//! 派发：类型、建立（指定机器、任意机器）、改派、掉线后接回、半成品交接、取消。

use super::*;

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
    /// 开始这次派发时这块白板在这台机器上的目录（工作空间里的 `<工作空间>/<项目>`）；还没开始、旧数据时为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 派给任意机器（PRD §9.1「任意机器」）：由服务端挑机器。还没挑到时 `host` 是空的、`hostId` 是 0；
    /// 没指定 agent 时 `agent` 也是空的，挑到机器后换成那台的默认 agent
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub any: bool,
    /// 这张卡所在项目的仓库地址（没设是空的）：board host 开始派发前据此拉最新代码（PRD §9.1「派发前更新代码」）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub repo: String,
    /// 项目的集成分支（没设是空的，board host 用远端的默认分支）：会话的 worktree 停在它上面
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
}

/// 「任意机器」的派发还没指定 agent 时写的名称
pub const ANY_AGENT_LABEL: &str = "机器的默认 agent";

/// 派发的类型：拆解「未整理」里的条目（PRD §9.1）
pub const KIND_SPLIT: &str = "split";

/// 派发前看卡片的结果（`dispatch_card_check`）
pub(super) struct CardCheck {
    /// 卡片在「未整理」：派给 agent 拆解
    pub(super) split: bool,
    /// 人从可领取状态以外派发：卡片现在所在的状态（排队时一步移进锁定状态，取消时回这里）
    pub(super) from: Option<String>,
}

/// 派发的来源：定时建卡建出卡后按这一条的设置派给指定的机器（PRD §4「定时建卡」）。
/// 和手动派发一样排队、占用卡片，只做这一张；动态里写「定时建卡」
pub const SOURCE_SCHEDULE: &str = "schedule";

pub(super) fn one() -> i64 {
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
    /// agent 的输出格式：`claude`、`codex`、`dsh`；空字符串 = 这个 agent 只有原始输出；没有 = board host 没报（旧版，或还没开始）
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
        "fine" => "不用拆",
        "nosplit" => "没拆出卡",
        _ => "已结束",
    }
}

/// 动态里的「，模型 sonnet，强度 high」；都没设时为空
pub(super) fn opts_words(model: &Option<String>, effort: &Option<String>) -> String {
    let mut s = String::new();
    if let Some(m) = model {
        s += &format!("，模型 {m}");
    }
    if let Some(e) = effort {
        s += &format!("，强度 {e}");
    }
    s
}

pub(super) fn source_words(s: &str) -> &'static str {
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

pub(super) const DISPATCH_SELECT: &str = "SELECT d.id, b.project || '/' || b.key, d.card, COALESCE(c.title, d.title), d.host_id, d.host, d.agent,
       d.agent_label, d.name, d.source, d.by_kind, d.by_name, d.state, d.result, d.created_at, d.started_at, d.ended_at,
       d.exit_code, d.stop_by, CASE WHEN c.holder = d.name THEN c.beat_at END, d.env, d.env_name, COALESCE(d.session, d.id),
       CASE WHEN d.session IS NULL THEN 1 ELSE (SELECT COUNT(*) FROM dispatches s WHERE s.session = d.session AND s.id <= d.id) END,
       CASE WHEN d.session IS NULL THEN 1 ELSE (SELECT COUNT(*) FROM dispatches s WHERE s.session = d.session) END,
       d.model, d.effort, d.kind, d.path, d.any_host, COALESCE(p.repo, ''), COALESCE(p.branch, '')
  FROM dispatches d JOIN boards b ON b.id = d.board LEFT JOIN projects p ON p.key = b.project LEFT JOIN cards c ON c.board = d.board AND c.id = d.card";

pub(super) fn dispatch_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Dispatch> {
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
        by: Actor { kind: ActorKind::parse(&r.get::<_, String>(10)?), name: r.get(11)?, agent: None },
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
        path: r.get(28)?,
        any: r.get::<_, i64>(29)? != 0,
        repo: r.get(30)?,
        branch: r.get(31)?,
    })
}

/* ------------------------------ 写 ------------------------------ */

/// 数据库里一次派发的几项（写操作用）
pub(super) struct DRow {
    pub(super) id: i64,
    pub(super) board: i64,
    pub(super) card: i64,
    pub(super) host_id: i64,
    pub(super) host: String,
    pub(super) agent: String,
    pub(super) agent_label: String,
    pub(super) name: Option<String>,
    pub(super) env_name: Option<String>,
    pub(super) source: String,
    pub(super) state: String,
    pub(super) stop_by: Option<String>,
    pub(super) started_at: Option<i64>,
    pub(super) env: Option<String>,
    /// 会话标识：第一张的派发 ID
    pub(super) session: i64,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    /// 空 = 做这张卡；`split` = 拆解
    pub(super) kind: String,
    /// 开始这次派发时这块白板在这台机器上的目录；还没开始、旧数据时为空
    pub(super) path: Option<String>,
    /// 派给任意机器：由服务端挑机器
    pub(super) any_host: bool,
    /// 派给任意机器时没指定 agent：用挑中那台的默认 agent
    pub(super) any_agent: bool,
}

impl DRow {
    pub(super) fn split(&self) -> bool {
        self.kind == KIND_SPLIT
    }
}

impl<'a> W<'a> {
    /// 手动派发（任务下发需求 §五）：校验通过后进入排队。agent 为空时用这台机器的默认 agent
    pub fn dispatch_create(&mut self, board: &BoardRef, card: i64, host: &str, agent: Option<&str>, by: &Actor) -> Result<Dispatch> {
        self.dispatch_create_from(board, card, host, agent, by, "manual")
    }

    /// 定时建卡保存前看它要派的机器：机器存在、有这块白板的目录，指定的 agent 在这台机器上（找没找到命令到点时再看）。
    /// 不合适时 `USAGE`，写明哪里不对
    pub(crate) fn dispatch_target_check(&self, board: &BoardRef, host: &str, agent: Option<&str>) -> Result<()> {
        let h = self.host_row(host)?.ok_or_else(|| BoardError::usage(format!("没有叫「{host}」的机器：界面「机器」里能看到所有机器")))?;
        // 有工作空间的机器就视为有这块白板的目录（目录状态只在机器详情里提示，不再拒绝派发）
        if h.workspace().is_none() && !h.dirs.iter().any(|d| d.board == board.path()) {
            return Err(BoardError::usage(format!(
                "机器「{host}」还没有配工作空间：先在机器详情里填上，{} 项目的目录是 <工作空间>/{}",
                board.project, board.project
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
        let check = self.dispatch_card_check(board, card, by)?;
        let h = self.host_row(host)?.ok_or_else(|| no_such_host(host))?;
        let a = self.dispatch_agent(&h, agent)?;
        // 这台机器接不了这块白板的卡：没有工作空间、目录不对（`fit.rs`）
        if let Some(u) = self.read().board_unfit(&h, board)? {
            return Err(u.error(host, board));
        }
        let kind = if check.split { KIND_SPLIT } else { "" };
        self.insert_dispatch(board.id, card, &h, &a.id, &a.name, source, by, kind, check.from.as_deref())
    }

    /// 派发前看卡片：存在、没有排队或在跑的派发、在可领取状态或「未整理」。人派发时也可以在别的状态（待审核、错误节点、
    /// 终点……），只要不在锁定状态、没人持有：排队时一步移进锁定状态，不在可领取状态里停留（停一下就可能被自动领取领走）
    pub(super) fn dispatch_card_check(&self, board: &BoardRef, card: i64, by: &Actor) -> Result<CardCheck> {
        let flow = flow_of(self.tx, board.id)?;
        let row: Option<(String, Option<String>)> = self
            .tx
            .query_row("SELECT col, holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![board.id, card], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let (col, holder) = row.ok_or_else(|| BoardError::new(ErrorCode::NotFound, format!("卡片 #{card} 不存在")))?;
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
        if split || flow.is_claim(&col) {
            return Ok(CardCheck { split, from: None });
        }
        let human = by.kind == ActorKind::Human;
        if human && !flow.is_lock(&col) && holder.is_none() && Self::dispatch_entry(&flow).is_some() {
            return Ok(CardCheck { split: false, from: Some(col) });
        }
        let names: Vec<String> = flow.0.iter().filter(|s| s.claim).map(|s| format!("「{}」", s.name)).collect();
        let why = if flow.is_lock(&col) || holder.is_some() {
            format!("#{card} 在「{}」，正在被领取，不能派发", flow.name(&col))
        } else {
            format!("#{card} 在「{}」，不在可领取的状态{}，不能派发（「{INBOX_NAME}」里的条目可以派给 agent 拆解）", flow.name(&col), names.join(""))
        };
        Err(BoardError::new(ErrorCode::NotInTodo, why))
    }

    /// 人从可领取状态以外派发时，卡片进哪个锁定状态排队：首个连到锁定状态的可领取状态领取时会进的那个。没有时为空（不能这样派发）
    pub(super) fn dispatch_entry(flow: &Flow) -> Option<(&Status, &Status)> {
        flow.0.iter().filter(|s| s.claim).find_map(|s| flow.claim_target(&s.key).map(|t| (s, t)))
    }

    /// 动态里写「从「X」」：人从可领取状态以外派发时
    pub(super) fn from_words(&self, b: i64, from: Option<&str>) -> Result<String> {
        let Some(k) = from else { return Ok(String::new()) };
        let flow = flow_of(self.tx, b)?;
        Ok(format!("从「{}」", flow.name(k)))
    }

    /// 这次派发在这台机器上用哪个 agent：指定了就要找得到命令，没指定用机器的默认 agent；
    /// 连续异常退出、暂停了接单的不接新的派发（`fit.rs`）
    pub(super) fn dispatch_agent(&self, h: &HostRow, agent: Option<&str>) -> Result<HostAgent> {
        let host = h.name.as_str();
        let all = h.resolved();
        let a = match agent.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => all.iter().find(|a| a.id == id && a.found).cloned().ok_or_else(|| {
                let missing = all.iter().find(|a| a.id == id);
                BoardError::new(
                    ErrorCode::AgentNotOnHost,
                    match missing {
                        // 装了 agent、缺 ACP 入口要的命令（Node.js）：说缺什么（#458）
                        Some(a) if a.missing.is_some() => {
                            let place = a.env.as_deref().map(|e| format!("在{}里", h.env_name(e))).unwrap_or_default();
                            format!("机器「{host}」上{place}缺 {}：agent「{id}」跑不起来，先装上", a.missing.as_deref().unwrap_or_default())
                        }
                        Some(a) => match &a.env {
                            Some(e) => format!("机器「{host}」上在{}里没找到 agent「{id}」的命令", h.env_name(e)),
                            None => format!("机器「{host}」上没找到 agent「{id}」的命令"),
                        },
                        None => format!("机器「{host}」上没有 agent「{id}」，可选：{}", found_list(h)),
                    },
                )
            })?,
            None => h
                .default_agent()
                .ok_or_else(|| BoardError::new(ErrorCode::AgentNotOnHost, format!("机器「{host}」上一个 agent 也没找到")))?,
        };
        // 连续异常退出、暂停了接单的 agent 不接新的派发（`fit.rs`）
        if self.read().agent_tripped(h, &a.id)? {
            return Err(BoardError::new(
                ErrorCode::AgentNotOnHost,
                format!(
                    "机器「{host}」上的 {} 连续 {} 次异常退出，已经暂停接单：修好后在机器详情里点「恢复」",
                    a.name,
                    crate::fit::TRIP_AFTER
                ),
            )
            .with_detail(json!({ "host": host, "agent": a.id })));
        }
        Ok(a)
    }

    /// 记一条排队的派发。手动派发、定时建卡派发（source 是 manual、schedule）的卡片进入领取时会进的锁定状态排队
    /// （[`W::reserve`]）；定时领取（cron）的第一张马上开始，不经过排队；拆解（kind 是 split）不动卡片
    #[allow(clippy::too_many_arguments)]
    pub(super) fn insert_dispatch(&mut self, b: i64, card: i64, h: &HostRow, agent: &str, label: &str, source: &str, by: &Actor, kind: &str, from: Option<&str>) -> Result<Dispatch> {
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
        let what = if split { format!("派给 {} · {} 拆解", h.name, label) } else { format!("{}派发到 {} · {}", self.from_words(b, from)?, h.name, label) };
        self.track(b, card, by, "dispatch", &format!("{what}（{}）{wait}", source_words(source)))?;
        self.touch_hosts();
        self.read().dispatch(id)
    }

    /// 派给任意机器：不指定机器，服务端在能接的机器里挑空位最多的一台（[`crate::fit::pick_host`]）；现在没有能马上
    /// 开始的就排着，等有机器上线、恢复接单、空出位置。agent 为空时用挑中那台的默认 agent。
    /// 一台能接这块白板的机器都没有（看的是能不能接，不看在不在线）时拒绝
    pub fn dispatch_create_any(&mut self, board: &BoardRef, card: i64, agent: Option<&str>, by: &Actor) -> Result<Dispatch> {
        let check = self.dispatch_card_check(board, card, by)?;
        let split = check.split;
        let agent = agent.map(str::trim).filter(|s| !s.is_empty());
        let label = self.any_target(board, agent)?;
        let title: String = self.tx.query_row("SELECT title FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![board.id, card], |r| r.get(0))?;
        self.tx.execute(
            "INSERT INTO dispatches (board, card, title, host_id, host, agent, agent_label, source, by_kind, by_name, state, created_at, kind, any_host, any_agent)
             VALUES (?1, ?2, ?3, 0, '', ?4, ?5, 'manual', ?6, ?7, 'queued', ?8, ?9, 1, ?10)",
            params![board.id, card, title, agent.unwrap_or(""), label, by.kind.as_str(), by.name, self.now, if split { KIND_SPLIT } else { "" }, agent.is_none() as i64],
        )?;
        let id = self.tx.last_insert_rowid();
        let into = if split { None } else { self.reserve(board.id, card, by)? };
        let at = into.map(|n| format!("，在「{n}」排队")).unwrap_or_else(|| "，排队".into());
        let what = if split { format!("派给任意机器 · {label} 拆解") } else { format!("{}派发到任意机器 · {label}", self.from_words(board.id, check.from.as_deref())?) };
        self.track(board.id, card, by, "dispatch", &format!("{what}（手动）{at}，等服务端挑一台能接的机器"))?;
        self.touch_hosts();
        self.assign_any()?;
        self.read().dispatch(id)
    }

    /// 「任意机器」能不能派：至少有一台机器接得了这块白板，并且上面有能用的这个 agent（没指定时是它的默认 agent）。
    /// 返回 agent 的名称（没指定时是 [`ANY_AGENT_LABEL`]）
    pub(super) fn any_target(&self, board: &BoardRef, agent: Option<&str>) -> Result<String> {
        let hosts = self.read().host_rows()?;
        if hosts.is_empty() {
            return Err(BoardError::new(ErrorCode::NoSuchHost, "还没有机器：在要干活的机器上运行 board host，它会出现在「机器」里"));
        }
        let path = board.path();
        let mut able: Vec<&HostRow> = vec![];
        let mut why: Vec<String> = vec![];
        for h in &hosts {
            match self.read().board_unfit(h, board)? {
                Some(u) => why.push(format!("{}（{}）", h.name, u.short())),
                None => able.push(h),
            }
        }
        if able.is_empty() {
            return Err(BoardError::new(ErrorCode::BoardNotOnHost, format!("没有能接 {path} 的机器：{}", why.join("；"))).with_detail(json!({ "board": path })));
        }
        let mut label: Option<String> = None;
        for h in &able {
            let a = match agent {
                Some(id) => h.resolved().into_iter().find(|a| a.id == id && a.found),
                None => h.default_agent(),
            };
            if let Some(a) = a {
                if !self.read().agent_tripped(h, &a.id)? {
                    label = Some(if agent.is_some() { a.name } else { ANY_AGENT_LABEL.to_string() });
                    break;
                }
            }
        }
        label.ok_or_else(|| {
            let names: Vec<&str> = able.iter().map(|h| h.name.as_str()).collect();
            BoardError::new(
                ErrorCode::AgentNotOnHost,
                match agent {
                    Some(id) => format!("能接 {path} 的机器（{}）上都没有能用的 agent「{id}」", names.join("、")),
                    None => format!("能接 {path} 的机器（{}）上都没有能用的 agent", names.join("、")),
                },
            )
        })
    }

    /// 挑机器时的一台候选：还能再开几个会话（上限 − 在跑的会话 − 已经排在它上面、还没开始的；会话里排着的不占会话位），
    /// 和最近一次给它派发的时间
    pub(crate) fn candidate(&self, h: &HostRow) -> Result<crate::fit::Candidate> {
        let queued: i64 = self.tx.query_row(
            "SELECT COUNT(*) FROM dispatches WHERE host_id = ?1 AND state = 'queued' AND (session IS NULL OR session = id)",
            [h.id],
            |r| r.get(0),
        )?;
        let last_at: i64 = self.tx.query_row("SELECT COALESCE(MAX(COALESCE(started_at, created_at)), 0) FROM dispatches WHERE host_id = ?1", [h.id], |r| r.get(0))?;
        Ok(crate::fit::Candidate { name: h.name.clone(), free: h.max - self.sessions_on(h)? - queued, last_at })
    }

    /// 一台机器现在能开始新的派发：在线、没暂停接单、没在等重启或升级
    pub(super) fn can_start(h: &HostRow, now: i64) -> bool {
        h.online(now) && !h.paused && !h.restarting()
    }

    /// 给「任意机器」的排队派发挑机器（创建时、机器上报时、有派发结束时、机器设置变了时都跑一遍）：
    /// 1. 已经分给一台机器、但那台机器现在开始不了它的（离线、暂停接单、等重启、被移除、接不了这块白板了）收回；
    /// 2. 没有机器的按创建先后，在「接得了这块白板、有能用的 agent、现在能开始、还有空位」的机器里用
    ///    [`crate::fit::pick_host`] 挑一台，记到派发上——之后和指定机器的派发一样，由那台 board host 开始。
    /// 没有合适的机器就接着排
    pub(crate) fn assign_any(&mut self) -> Result<()> {
        let pending: Vec<i64> = self.collect(
            "SELECT id FROM dispatches WHERE any_host = 1 AND state = 'queued' AND (session IS NULL OR session = id) ORDER BY id",
            [],
            |r| r.get(0),
        )?;
        if pending.is_empty() {
            return Ok(());
        }
        let hosts = self.read().host_rows()?;
        let now = self.now;
        for id in &pending {
            let d = self.drow(*id)?;
            if d.host_id == 0 {
                continue;
            }
            let board = self.board_of(d.board)?;
            let why = match hosts.iter().find(|h| h.id == d.host_id) {
                None => Some("已经被移除".to_string()),
                Some(h) if !h.online(now) => Some("离线了".to_string()),
                Some(h) if h.paused => Some("暂停接单了".to_string()),
                Some(h) if h.restarting() => Some("在等重启或升级".to_string()),
                Some(h) => self.read().board_unfit(h, &board)?.map(|u| format!("接不了这块白板了（{}）", u.short())),
            };
            if let Some(why) = why {
                self.unassign(&d, &why)?;
            }
        }
        let mut cands: Vec<(usize, crate::fit::Candidate)> = vec![];
        for (i, h) in hosts.iter().enumerate() {
            if !Self::can_start(h, now) {
                continue;
            }
            cands.push((i, self.candidate(h)?));
        }
        let mut seq = 0;
        for id in pending {
            let d = self.drow(id)?;
            if d.host_id != 0 {
                continue;
            }
            let board = self.board_of(d.board)?;
            // 这张卡能去的机器，和它在那台机器上用的 agent
            let mut list: Vec<crate::fit::Candidate> = vec![];
            let mut agents: Vec<(String, HostAgent)> = vec![];
            for (i, c) in &cands {
                let h = &hosts[*i];
                if c.free <= 0 || self.read().board_unfit(h, &board)?.is_some() {
                    continue;
                }
                let a = if d.any_agent { h.default_agent() } else { h.resolved().into_iter().find(|a| a.id == d.agent && a.found) };
                let Some(a) = a else { continue };
                if self.read().agent_tripped(h, &a.id)? {
                    continue;
                }
                list.push(c.clone());
                agents.push((h.name.clone(), a));
            }
            let Some(pick) = crate::fit::pick_host(&list).map(|c| c.name.clone()) else { continue };
            let Some((_, a)) = agents.into_iter().find(|(n, _)| *n == pick) else { continue };
            let Some((_, c)) = cands.iter_mut().find(|(_, c)| c.name == pick) else { continue };
            let h = hosts.iter().find(|h| h.name == pick).expect("候选来自 hosts");
            self.tx.execute(
                "UPDATE dispatches SET host_id = ?2, host = ?3, agent = ?4, agent_label = ?5 WHERE id = ?1",
                params![d.id, h.id, h.name, a.id, a.name],
            )?;
            let text = format!("任意机器：分给 {} · {}（能接的机器里空位最多，还有 {} 个）", h.name, a.name, c.free);
            self.track_dispatch(d.board, d.card, &Actor::system(), "dispatch", &text)?;
            // 这一遍里后面的卡片看到的是分完之后的空位；刚分到的排到轮流的最后
            c.free -= 1;
            seq += 1;
            c.last_at = now + seq;
            self.touch_hosts();
        }
        Ok(())
    }

    /// 收回一条已经分给某台机器、还没开始的「任意机器」派发：回到没有机器的状态，等下一遍重新挑
    pub(super) fn unassign(&mut self, d: &DRow, why: &str) -> Result<()> {
        self.tx.execute(
            "UPDATE dispatches SET host_id = 0, host = '',
                    agent = CASE WHEN any_agent = 1 THEN '' ELSE agent END,
                    agent_label = CASE WHEN any_agent = 1 THEN ?2 ELSE agent_label END
             WHERE id = ?1",
            params![d.id, ANY_AGENT_LABEL],
        )?;
        self.track_dispatch(d.board, d.card, &Actor::system(), "dispatch", &format!("任意机器：{} {why}，这次派发收回，重新挑机器", d.host))?;
        self.touch_hosts();
        Ok(())
    }

    /// 改派：把一条还在排队的派发换到另一台机器（`host` 是机器名），或者交给任意机器（`host` 为空）。
    /// 已经开始的不能改（先停止它）；会话里排着的由那个会话接着做，也不能改。
    /// 换到指定机器时：`agent` 没给就沿用原来的，那台机器上用不了时换成它的默认 agent；校验和手动派发一样
    pub fn dispatch_move(&mut self, id: i64, host: Option<&str>, agent: Option<&str>, by: &Actor) -> Result<Dispatch> {
        let d = self.drow(id)?;
        match d.state.as_str() {
            "queued" => {}
            "running" => return Err(BoardError::usage(format!("派发 #{id} 已经开始，不能改派：先停止它，再重新派发"))),
            _ => return Err(BoardError::usage(format!("派发 #{id} 已经结束"))),
        }
        if d.session != d.id {
            return Err(BoardError::usage(format!("派发 #{id} 排在会话里，由那个会话接着做，不能改派：先取消它，再重新派发")));
        }
        let board = self.board_of(d.board)?;
        let agent = agent.map(str::trim).filter(|s| !s.is_empty());
        let from = Self::dispatch_words(&d);
        match host.map(str::trim).filter(|s| !s.is_empty()) {
            None => {
                let label = self.any_target(&board, agent)?;
                self.tx.execute(
                    "UPDATE dispatches SET host_id = 0, host = '', agent = ?2, agent_label = ?3, any_host = 1, any_agent = ?4 WHERE id = ?1",
                    params![d.id, agent.unwrap_or(""), label, agent.is_none() as i64],
                )?;
                self.track_dispatch(d.board, d.card, by, "dispatch", &format!("改派给任意机器 · {label}（原来 {from}）"))?;
                self.touch_hosts();
                self.assign_any()?;
            }
            Some(name) => {
                let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
                // 没说用哪个 agent：沿用原来的；那台机器上用不了它时换成默认 agent
                let keep = if d.agent.is_empty() { None } else { Some(d.agent.as_str()) };
                let a = match agent {
                    Some(id) => self.dispatch_agent(&h, Some(id))?,
                    None => match keep.map(|id| self.dispatch_agent(&h, Some(id))) {
                        Some(Ok(a)) => a,
                        _ => self.dispatch_agent(&h, None)?,
                    },
                };
                if let Some(u) = self.read().board_unfit(&h, &board)? {
                    return Err(u.error(name, &board));
                }
                if h.id == d.host_id && a.id == d.agent && !d.any_host {
                    return self.read().dispatch(id);
                }
                self.tx.execute(
                    "UPDATE dispatches SET host_id = ?2, host = ?3, agent = ?4, agent_label = ?5, any_host = 0, any_agent = 0 WHERE id = ?1",
                    params![d.id, h.id, h.name, a.id, a.name],
                )?;
                let wait = if !h.online(self.now) {
                    "，等机器上线"
                } else if h.paused {
                    "（机器暂停接单）"
                } else {
                    ""
                };
                self.track_dispatch(d.board, d.card, by, "dispatch", &format!("改派到 {} · {}{wait}（原来 {from}）", h.name, a.name))?;
                self.touch_hosts();
            }
        }
        self.read().dispatch(id)
    }

    /// 机器掉线超过 50 秒，服务端已经把它在跑的派发按「机器掉线」结束、卡片放回了可领取状态；之后机器恢复连接、
    /// agent 进程其实还在干活。卡片还没被别人领走（还在可领取状态、没有持有者、没有别的排队或在跑的派发）时把这次派发接回：
    /// 名字重新上线、以它重新领取卡片、派发回到在跑，agent 接着做，不用结束它、换一台从头来。
    /// 接不回（卡片已经被领走、被人移走、被删）时什么也不做，board host 照旧结束那个 agent。返回有没有接回
    pub(super) fn dispatch_revive(&mut self, id: i64, host: &str, token: Option<i64>) -> Result<bool> {
        let row: Option<(String, Option<String>, String)> =
            self.tx.query_row("SELECT state, result, host FROM dispatches WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
        let Some((state, result, on)) = row else { return Ok(false) };
        if state != "done" || result.as_deref() != Some("lost") || on != host {
            return Ok(false);
        }
        let d = self.drow(id)?;
        let Some(name) = d.name.clone() else { return Ok(false) };
        let board = self.board_of(d.board)?;
        let actor = Actor::agent(name.clone());
        if !d.split() {
            let card: Option<(String, Option<String>)> = self
                .tx
                .query_row("SELECT col, holder FROM cards WHERE board = ?1 AND id = ?2 AND deleted_at IS NULL", params![d.board, d.card], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let Some((col, holder)) = card else { return Ok(false) };
            let flow = flow_of(self.tx, d.board)?;
            let busy: i64 = self.tx.query_row("SELECT COUNT(*) FROM dispatches WHERE board = ?1 AND card = ?2 AND state != 'done'", params![d.board, d.card], |r| r.get(0))?;
            if holder.is_some() || !flow.is_claim(&col) || busy > 0 {
                return Ok(false);
            }
        }
        // 名字被别的在线会话用着（很少见）：不接
        if self.agent_check_with_agent(Some(&d.agent), &name, token, crate::agents::NameOwner::Host).is_err() {
            return Ok(false);
        }
        if !d.split() {
            self.dispatch_ok = true;
            let claimed = self.exec(&board, &actor, Cmd::Claim { id: Some(d.card) });
            self.dispatch_ok = false;
            if claimed.is_err() {
                self.tx.execute("UPDATE agents SET off_at = ?2 WHERE name = ?1", params![name, self.now])?;
                return Ok(false);
            }
        }
        self.tx.execute("UPDATE dispatches SET state = 'running', result = NULL, ended_at = NULL, exit_code = NULL WHERE id = ?1", [id])?;
        let text = format!("机器 {host} 恢复连接：这张卡还没被别人领走，{name} 接着做（接回掉线前的派发，{}）", Self::dispatch_words(&d));
        self.track_dispatch(d.board, d.card, &actor, "dispatch", &text)?;
        self.touch_hosts();
        Ok(true)
    }

    /// 半成品交接：派发结束时 agent 的 worktree 里还有没推到远端的改动，board host 把它们保存成了 wip 分支。
    /// 在卡片上评论怎么结束的、留下了什么、agent 最后做了什么，最后一行是 `未完成的改动：<分支>`——接手的 agent
    /// 按 board-skills 的约定从这条分支接着做，不用从头来。派发结束之后也能调（机器掉线的那种就是结束之后才推的）
    pub fn dispatch_wip(&mut self, id: i64, branch: &str, how: &str, changes: &str, tail: &[String]) -> Result<()> {
        let branch = branch.trim();
        let ok = branch.starts_with("wip/") && branch.len() <= 120 && branch.chars().all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c)) && !branch.contains("..");
        if !ok {
            return Err(BoardError::usage(format!("wip 分支名不对：{branch}")));
        }
        let d = self.drow(id)?;
        if !self.card_exists(d.board, d.card)? {
            return Ok(());
        }
        let board = self.board_of(d.board)?;
        let line = |t: &str| t.trim().chars().take(300).collect::<String>();
        let mut text = format!(
            "board host（{}）：这次派发结束时，{} 的 worktree 里还有没推到远端的改动，已经保存到分支 `{branch}`。\n\n- 怎么结束的：{}\n- 留下了什么：{}",
            d.host,
            d.name.as_deref().unwrap_or(&d.agent_label),
            line(how),
            line(changes)
        );
        let tail: Vec<String> = tail.iter().map(|t| line(t)).filter(|t| !t.is_empty()).collect();
        if !tail.is_empty() {
            let skip = tail.len().saturating_sub(10);
            text += "\n- agent 最后做的：\n";
            text += &tail[skip..].iter().map(|t| format!("  - {t}")).collect::<Vec<_>>().join("\n");
        }
        text += &format!("\n\n接手时从这条分支接着做（先看它相对集成分支改了什么），不用从头来；做完推进集成分支后删掉这条分支。\n\n未完成的改动：{branch}");
        soft(self.exec(&board, &Actor::system(), Cmd::Comment { id: d.card, text }))?;
        Ok(())
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
}
