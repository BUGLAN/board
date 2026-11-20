//! 读：机器在库里的一行（`HostRow`）、领取规则的匹配、`R` 上的查询。

use super::*;

/* ------------------------------ 读 ------------------------------ */

pub(crate) struct HostRow {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) instance: String,
    pub(crate) os: String,
    pub(crate) version: String,
    pub(crate) agents: Vec<HostAgent>,
    pub(crate) envs: Vec<HostEnv>,
    pub(crate) run: std::collections::BTreeMap<String, AgentRun>,
    /// 工作空间（存的是 JSON；升级前只有白板与目录时为空，用 dirs 导出）
    pub(crate) workspace_raw: Option<String>,
    /// 旧的白板与目录（升级前的数据，或者旧版 board host 上报的）
    pub(crate) dirs: Vec<HostDir>,
    pub(crate) dirs_rev: i64,
    pub(crate) dirs_at: Option<i64>,
    pub(crate) dirs_by: Option<String>,
    pub(crate) max: i64,
    pub(crate) per_session: i64,
    pub(crate) max_run: i64,
    pub(crate) idle_run: i64,
    pub(crate) rules: Vec<ClaimRule>,
    pub(crate) fallback: Option<String>,
    /// 每个 agent 明确不参与的机器 id（`sched.picks`）
    pub(crate) picks: crate::sched::Picks,
    pub(crate) paused: bool,
    pub(crate) agent: Option<String>,
    /// 参与自动接单（`sched.rs`）
    pub(crate) auto: bool,
    /// 接单时段：一天里的几段 `[起, 止)`，分钟，按服务端时区
    pub(crate) windows: Vec<crate::sched::Window>,
    /// 自动升级已经替它试过升级到哪个服务端版本
    pub(crate) up_tried: Option<String>,
    /// 升级到 `up_tried` 自动试了几次
    pub(crate) up_tries: i64,
    /// 自动升级的下一次：`None` 这一次还在做（或还没试过），毫秒时间戳是临时失败后到这个时间再试，`0` 不再自动试
    pub(crate) up_retry: Option<i64>,
    pub(crate) round_at: Option<i64>,
    pub(crate) rr: i64,
    pub(crate) first_at: i64,
    pub(crate) beat_at: i64,
    pub(crate) off_at: Option<i64>,
    pub(crate) ask: Option<HostAsk>,
    pub(crate) ask_seq: i64,
    pub(crate) drain_at: Option<i64>,
    pub(crate) up_pause: bool,
    pub(crate) agent_auto: bool,
    pub(crate) rechecked_at: Option<i64>,
    pub(crate) token: Option<i64>,
    pub(crate) selftest: SelfTest,
    pub(crate) sync_skills: bool,
    pub(crate) sync_mcp: bool,
    pub(crate) sync_git: bool,
    pub(crate) config_sync: ConfigSync,
    pub(crate) skill_import: SkillImport,
    pub(crate) res: Option<crate::hostres::HostRes>,
    pub(crate) agents_raw: String,
    pub(crate) envs_raw: String,
    pub(crate) dirs_raw: String,
    /// 项目设置里填了「项目目录」的项目（项目标识 → 目录名，#488）：派生目录用，见 [`HostRow::workspace`]
    pub(crate) proj_dirs: BTreeMap<String, String>,
}

pub(super) const HOST_COLS: &str =
    "id, name, instance, os, version, agents, workspace, max, paused, agent, cron_on, cron, round_at, rr, first_at, beat_at, off_at, envs, run, per_session, COALESCE((SELECT rules FROM sched WHERE id = 1), '[]'), (SELECT fallback FROM sched WHERE id = 1), dirs_rev, dirs_at, dirs_by, ask, ask_seq, drain_at, up_pause, rechecked_at, token, COALESCE((SELECT max_run FROM sched WHERE id = 1), 180), COALESCE((SELECT idle_run FROM sched WHERE id = 1), 20), selftest, sync_skills, sync_mcp, config_sync, skill_import, dirs, auto, windows, up_tried, up_tries, up_retry, res, sync_git, agent_auto, COALESCE((SELECT picks FROM sched WHERE id = 1), '{}') AS sched_picks, COALESCE((SELECT json_group_object(key, dir) FROM projects WHERE dir != ''), '{}') AS proj_dirs";

fn host_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HostRow> {
    let agents_raw: String = r.get(5)?;
    let workspace_raw: Option<String> = r.get(6)?;
    let dirs_raw: String = r.get(38)?;
    let envs_raw: String = r.get(17)?;
    let run: String = r.get(18)?;
    Ok(HostRow {
        id: r.get(0)?,
        name: r.get(1)?,
        instance: r.get(2)?,
        os: r.get(3)?,
        version: r.get(4)?,
        agents: serde_json::from_str(&agents_raw).unwrap_or_default(),
        proj_dirs: serde_json::from_str(&r.get::<_, String>("proj_dirs")?).unwrap_or_default(),
        envs: serde_json::from_str(&envs_raw).unwrap_or_default(),
        run: serde_json::from_str(&run).unwrap_or_default(),
        workspace_raw,
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
        picks: crate::sched::parse_picks(&r.get::<_, String>("sched_picks")?),
        paused: r.get::<_, i64>(8)? != 0,
        agent: r.get(9)?,
        auto: r.get::<_, i64>(39)? != 0,
        windows: serde_json::from_str(&r.get::<_, String>(40)?).unwrap_or_else(|_| crate::sched::all_day()),
        up_tried: r.get(41)?,
        up_tries: r.get(42)?,
        up_retry: r.get(43)?,
        res: r.get::<_, Option<String>>(44)?.and_then(|j| serde_json::from_str(&j).ok()),
        round_at: r.get(12)?,
        rr: r.get(13)?,
        first_at: r.get(14)?,
        beat_at: r.get(15)?,
        off_at: r.get(16)?,
        ask: r.get::<_, Option<String>>(25)?.and_then(|j| serde_json::from_str(&j).ok()),
        ask_seq: r.get(26)?,
        drain_at: r.get(27)?,
        up_pause: r.get::<_, i64>(28)? != 0,
        agent_auto: r.get::<_, i64>(46)? != 0,
        rechecked_at: r.get(29)?,
        token: r.get(30)?,
        selftest: r.get::<_, Option<String>>(33)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default(),
        sync_skills: r.get::<_, i64>(34)? != 0,
        sync_mcp: r.get::<_, i64>(35)? != 0,
        sync_git: r.get::<_, i64>(45)? != 0,
        config_sync: r.get::<_, Option<String>>(36)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default(),
        skill_import: r.get::<_, Option<String>>(37)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default(),
        agents_raw,
        envs_raw,
        dirs_raw,
    })
}

impl HostRow {
    pub(crate) fn online(&self, now: i64) -> bool {
        self.off_at.is_none() && self.beat_at + HOST_GAP_MS > now
    }
    /// 一个 agent 实际用的启动环境（启动环境与桌面端需求 §2.4）：设置过、本机还有（在它支持的环境里）就用它，
    /// 找没找到都按它算；否则按支持的顺序取第一个找到命令的。都找不到时取第一个装了 agent、只缺 Node.js（npx）这类
    /// 命令的（还是用不了，界面上写缺什么，#458），再没有为空。
    /// 旧版 board host 不上报启动环境：返回 None，找没找到按它上报的
    pub(super) fn pick<'h>(&self, a: &'h HostAgent) -> Option<&'h EnvCheck> {
        if self.envs.is_empty() {
            return None;
        }
        let set = self.run.get(&a.id).and_then(|r| r.env.as_deref());
        set.and_then(|e| a.envs.iter().find(|c| c.env == e))
            .or_else(|| a.envs.iter().find(|c| c.found))
            .or_else(|| a.envs.iter().find(|c| c.missing.is_some()))
    }
    /// 本机 agent，`found`、`version`、`env` 换成实际用的启动环境里的结果
    pub(crate) fn resolved(&self) -> Vec<HostAgent> {
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
                    out.missing = c.and_then(|c| c.missing.clone());
                    if let Some(m) = c.and_then(|c| c.manage.clone()) {
                        out.manage = Some(m);
                    }
                }
                let run = self.run.get(&a.id);
                out.run_env = run.and_then(|r| r.env.clone());
                let mode = self.mode_of(&a.id);
                // 旧版 board host 只认 run_provider：不改 = @local，指定 = 供应商名称
                out.run_provider = match mode {
                    MODE_OWN => run.and_then(|r| r.provider.clone()),
                    _ => Some(crate::providers::PROVIDER_LOCAL.into()),
                };
                if mode == MODE_OWN {
                    out.run_model = run.and_then(|r| r.model.clone());
                    out.run_effort = run.and_then(|r| r.effort.clone());
                    out.run_protocol = run.and_then(|r| r.protocol.clone());
                }
                out.run_mode = Some(mode.to_string());
                out
            })
            .collect()
    }
    /// 一个 agent 的模型怎么配：`own` / `off`（没设过的是 `off`）
    pub(crate) fn mode_of(&self, agent: &str) -> &'static str {
        match self.run.get(agent).and_then(|r| r.mode.as_deref()) {
            Some(MODE_OWN) => MODE_OWN,
            _ => MODE_OFF,
        }
    }
    /// 生成 agent 名字时第二段（模型名）用什么，见 [`model_of`]
    pub(super) fn model_for(&self, agent: &str) -> (String, bool) {
        model_of(&self.agents, agent)
    }
    /// 开始派发时记进派发记录的模型、强度：board host 上报的、这个 agent 配置文件里现在的
    pub(super) fn run_opts(&self, agent: &str) -> (Option<String>, Option<String>) {
        let a = self.agents.iter().find(|a| a.id == agent);
        let model = a.map(|a| a.model.trim().to_string()).filter(|m| !m.is_empty());
        let effort = a.map(|a| a.effort.trim().to_string()).filter(|e| !e.is_empty());
        (model, effort)
    }

    /// 启动环境的名称
    pub(crate) fn env_name(&self, id: &str) -> String {
        self.envs.iter().find(|e| e.id == id).map(|e| e.name.clone()).unwrap_or_else(|| id.to_string())
    }
    /// 这台机器上找得到这个 agent
    pub(crate) fn has_agent(&self, id: &str) -> bool {
        self.resolved().iter().any(|a| a.id == id && a.found)
    }
    /// 解析后的领取规则；一条规则也没有时为 None：定时领取不领卡（兜底只处理规则没命中的卡片，不算规则）
    pub(super) fn claim_rules(&self) -> Option<Rules> {
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
    pub(super) fn has_rules(&self) -> bool {
        !self.rules.is_empty()
    }
    /// 这台机器上按「由哪些机器做」挑卡片的规则：勾了这台机器的 agent 才接它的卡。
    /// 给回来的 `Routes` 要拿着不放（它借用规则里的名字）
    pub(super) fn routes<'r>(&'r self, rules: &'r Rules) -> Routes<'r> {
        Routes { inner: rules, picks: &self.picks, host: self.id }
    }
    /// 默认 agent：设置过且找到了就是它，否则第一个找到的
    pub(crate) fn default_agent(&self) -> Option<HostAgent> {
        let all = self.resolved();
        let found = |id: &str| all.iter().find(|a| a.id == id && a.found).cloned();
        self.agent.as_deref().and_then(found).or_else(|| all.iter().find(|a| a.found).cloned())
    }
    /// 这台机器的工作空间（[`workspace_of`]），带上项目设置里的项目目录：派生目录、发给界面和 board host 都用它（#488）
    pub(crate) fn workspace(&self) -> Option<HostWorkspace> {
        workspace_of(self.workspace_raw.as_deref(), &self.dirs).map(|mut w| {
            w.projects = self.proj_dirs.clone();
            w
        })
    }
    pub(super) fn info(&self, now: i64) -> HostInfo {
        let workspace = self.workspace();
        HostInfo {
            id: self.id,
            name: self.name.clone(),
            os: self.os.clone(),
            version: self.version.clone(),
            agents: self.resolved(),
            envs: self.envs.clone(),
            dirs: derived_dirs(&self.dirs, workspace.as_ref(), &self.os),
            workspace,
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
            agent_auto: self.agent_auto,
            agent: self.default_agent().map(|a| a.id.clone()),
            agent_set: self.agent.clone().filter(|a| !self.has_agent(a)),
            cron_on: false,
            cron: vec![],
            round: self.auto_active(now),
            auto: self.auto,
            windows: self.windows.clone(),
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
            sync_skills: self.sync_skills,
            sync_mcp: self.sync_mcp,
            sync_git: self.sync_git,
            sync: self.config_sync.clone(),
            skill_import: self.skill_import.clone(),
            res: self.res.clone(),
        }
    }
    /// 现在在自动接单：开了自动接单、没暂停接单、有领取规则，并且现在在接单时段里（按服务端的时区）
    pub(crate) fn auto_active(&self, now: i64) -> bool {
        self.auto && !self.paused && self.has_rules() && crate::sched::in_windows(&self.windows, crate::sched::server_minute(now))
    }
    /// 正在等重启或升级：board host 不再开始新的派发
    pub(super) fn restarting(&self) -> bool {
        self.ask.as_ref().is_some_and(|a| a.kind == "restart" || a.kind == "upgrade")
    }
}

/// 一台机器上看得见的 agent + 明确不参与它的机器（拿到的 agent 名按它自己的生命周期借出去，和 `Rules` 一样长）
pub(super) struct Routes<'a> {
    pub(super) inner: &'a Rules,
    pub(super) picks: &'a crate::sched::Picks,
    pub(super) host: i64,
}

impl<'a> Routes<'a> {
    /// 这台机器参不参与这个 agent 的自动领取：不在排除列表里就参与（默认参与）
    pub(super) fn takes(&self, agent: &str) -> bool {
        self.picks.get(agent).is_none_or(|off| !off.contains(&self.host))
    }

    /// 这道命中规则交给这个 agent；这台机器被排除了时不交
    pub(super) fn agent(&self, agent: &'a str) -> Option<&'a str> {
        self.takes(agent).then_some(agent)
    }

    /// 一张卡片交给哪个 agent：有规则命中时按 [`Rules::agent_for`]（第一条命中的规则生效才交）；
    /// 一条都没命中时交给兜底的 agent（要找得到）。交出去的 agent 都看这台机器是否被排除在它的自动领取之外
    pub(super) fn route(&self, labels: &[String]) -> Option<&'a str> {
        if self.inner.matches_any(labels) {
            return self.inner.agent_for(labels).and_then(|a| self.agent(a));
        }
        self.inner
            .fallback
            .as_ref()
            .filter(|(_, ok)| *ok)
            .and_then(|(a, _)| self.takes(a).then_some(a.as_str()))
    }
}

/// 一条解析后的领取规则
pub(super) struct Cond {
    pub(super) op: String,
    /// 标签名小写
    pub(super) labels: Vec<String>,
    pub(super) agent: String,
    /// agent 在这台机器上找得到
    pub(super) ok: bool,
}

impl Cond {
    /// 卡片的标签（小写）满足条件吗
    pub(super) fn matches(&self, card: &[String]) -> bool {
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
pub(super) struct Rules {
    pub(super) list: Vec<Cond>,
    /// 兜底的 agent，找得到
    pub(super) fallback: Option<(String, bool)>,
}

impl Rules {
    /// 一张卡片（标签名小写）交给哪个 agent：按顺序第一条命中、生效的规则；先命中的是不生效的规则（agent 找不到）时不领它；
    /// 一条都没命中时用兜底；都没有时不领它
    pub(super) fn agent_for(&self, labels: &[String]) -> Option<&str> {
        let hit = self.list.iter().find(|c| c.matches(labels))?;
        if hit.ok {
            return Some(&hit.agent);
        }
        None
    }
    /// 有一条规则命中（不管生效没有）
    pub(super) fn matches_any(&self, labels: &[String]) -> bool {
        self.list.iter().any(|c| c.matches(labels))
    }
    /// 规则里用到的标签（小写）
    pub(super) fn labels(&self) -> Vec<String> {
        let mut out: Vec<String> = self.list.iter().flat_map(|c| c.labels.clone()).collect();
        out.sort();
        out.dedup();
        out
    }
}

/// 卡片的标签名（小写）
pub(super) fn card_label_names(conn: &rusqlite::Connection, b: i64, card: i64) -> Result<Vec<String>> {
    let mut st = conn.prepare("SELECT lower(l.name) FROM card_labels cl JOIN labels l ON l.id = cl.label WHERE cl.board = ?1 AND cl.card = ?2")?;
    let out = st.query_map(params![b, card], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(out)
}

/// 生成 agent 名字时第二段（模型名）用什么，以及有没有：board host 上报的这个 agent 现在用的模型
/// （内置 agent 读它自己的配置文件，写进去的或人配的；配置文件里新加的 agent 是 `[agents.<标识>] model`）。
/// 没有时为空，名字只写 `<机器名>/<agent 标识>`
pub(super) fn model_of(agents: &[HostAgent], agent: &str) -> (String, bool) {
    let reported = agents.iter().find(|a| a.id == agent).map(|a| a.model.trim().to_string()).unwrap_or_default();
    if !reported.is_empty() && check_model(&reported).is_ok() {
        return (reported, true);
    }
    (String::new(), false)
}

/// 生成 agent 名字时的思考强度：board host 上报的这个 agent 配置文件里现在的强度，没有时为空
pub(super) fn effort_of(agents: &[HostAgent], agent: &str) -> String {
    agents.iter().find(|a| a.id == agent).map(|a| a.effort.trim().to_string()).filter(|e| !e.is_empty()).unwrap_or_default()
}

/// `board check --agent <标识>` 用：按写事务里的机器记录（[`W::agent_check_generated`] 已经按机器名或令牌找到的那台）
/// 算这个 agent 该用的模型名、强度，以及第二段是不是真的模型名
pub(crate) fn name_opts_for_transaction(
    _conn: &rusqlite::Connection,
    h: &HostInfo,
    agent: &str,
) -> Result<(String, String, bool)> {
    let (model, modeled) = model_of(&h.agents, agent);
    Ok((model, effort_of(&h.agents, agent), modeled))
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

/// 令牌表有没有 `host` 列（机器令牌绑定的机器名，#507；服务端启动时加上）
pub(super) fn token_host_column(conn: &rusqlite::Connection) -> Result<bool> {
    Ok(conn.query_row("SELECT COUNT(*) > 0 FROM pragma_table_info('auth_tokens') WHERE name = 'host'", [], |r| r.get(0))?)
}

pub(crate) fn no_such_host(name: &str) -> BoardError {
    BoardError::new(ErrorCode::NoSuchHost, format!("机器「{name}」不存在：在那台机器上运行 board host，界面「机器」里能看到所有机器"))
        .with_detail(json!({ "host": name }))
}

/// 令牌调用 board host 的接口，机器不存在或不归这个令牌（见 [`R::host_owned_by`]）
pub(super) fn not_owned_host(name: &str) -> BoardError {
    BoardError::new(
        ErrorCode::NoSuchHost,
        format!("机器「{name}」不存在，或不归这个访问令牌：只有这台机器的 board host（用它上报时的令牌）能调用，人在界面上操作"),
    )
    .with_detail(json!({ "host": name }))
}

impl<'a> R<'a> {
    pub(crate) fn host_row(&self, name: &str) -> Result<Option<HostRow>> {
        Ok(self.conn.query_row(&format!("SELECT {HOST_COLS} FROM hosts WHERE name = ?1"), [name], host_row).optional()?)
    }

    /// 所有机器的记录，按名字排序
    pub(crate) fn host_rows(&self) -> Result<Vec<HostRow>> {
        let mut st = self.conn.prepare(&format!("SELECT {HOST_COLS} FROM hosts ORDER BY name"))?;
        let rows = st.query_map([], host_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 所有机器，按名字排序
    pub fn hosts(&self, now: i64) -> Result<Vec<HostInfo>> {
        self.host_rows()?.iter().map(|h| self.host_info(h, now)).collect()
    }

    pub fn host(&self, name: &str, now: i64) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        self.host_info(&h, now)
    }

    /// board host 按名字找自己的机器记录。名字找不到时，再按这个进程的标识找：机器刚被改名
    /// （桌面端「设置 → 本机」保存机器名，先在服务端改名、再写配置文件），正在跑的 board host
    /// 读到新配置之前还在用旧名字上报、取任务，认回改名后的那条记录，不新建第二台机器。
    /// 标识每次启动随机生成，只有这个进程自己知道，别的机器借不到
    pub(crate) fn host_of_proc(&self, name: &str, instance: &str) -> Result<Option<HostRow>> {
        if let Some(h) = self.host_row(name)? {
            return Ok(Some(h));
        }
        if instance.trim().is_empty() {
            return Ok(None);
        }
        Ok(self.conn.query_row(&format!("SELECT {HOST_COLS} FROM hosts WHERE instance = ?1"), [instance], host_row).optional()?)
    }

    /// 令牌调用 board host 的接口（取任务、开始派发、回报结果、领一轮、改名……；上报的规则见 [`W::host_beat`]）时，
    /// 机器得归这个令牌：它最后一次上报用的就是这个令牌（上报时绑定）。`token` 为空（登录的界面、本机模式）不查。
    /// 机器不存在、不归这个令牌都是 NO_SUCH_HOST：别的令牌既动不了它，也不会因此多知道什么
    pub fn host_owned_by(&self, name: &str, token: Option<i64>) -> Result<()> {
        self.host_owned_by_proc(name, "", token)
    }

    /// 同 [`R::host_owned_by`]，`instance` 是 board host 进程的标识：机器刚改名、它还在用旧名字时按标识认回
    /// 改名后的记录（见 [`R::host_of_proc`]）
    pub fn host_owned_by_proc(&self, name: &str, instance: &str, token: Option<i64>) -> Result<()> {
        let Some(t) = token else { return Ok(()) };
        match self.host_of_proc(name, instance)? {
            Some(h) if h.token == Some(t) => Ok(()),
            _ => Err(not_owned_host(name)),
        }
    }

    /// 令牌调用派发的接口（结束、存 wip、读写日志、传事件）时，派发得在这个令牌的机器上。机器按派发记录里的
    /// 机器名找（机器被移除、再连上来之后，原来在跑的派发照样能结束，同 [`R::host_work`]），找不到时按机器 ID。
    /// `token` 为空（登录的界面、本机模式）不查。派发不存在、不在这个令牌的机器上都是 NOT_FOUND
    pub fn dispatch_owned_by(&self, id: i64, token: Option<i64>) -> Result<()> {
        let Some(t) = token else { return Ok(()) };
        let row: Option<(String, i64)> =
            self.conn.query_row("SELECT host, host_id FROM dispatches WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let owner = match row {
            None => None,
            Some((name, host_id)) => match self.host_row(&name)? {
                Some(h) => h.token,
                None => self.conn.query_row("SELECT token FROM hosts WHERE id = ?1", [host_id], |r| r.get::<_, Option<i64>>(0)).optional()?.flatten(),
            },
        };
        if owner == Some(t) {
            return Ok(());
        }
        Err(BoardError::new(ErrorCode::NotFound, format!("派发 #{id} 不存在，或不在这个访问令牌的机器上")))
    }

    /// 访问令牌还在（没被吊销）时返回它的名称。没有令牌表（只用 board-core 的库，不经过服务端）时当作不在
    pub(super) fn live_token_name(&self, id: i64) -> Result<Option<String>> {
        let has: bool = self.conn.query_row("SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'auth_tokens'", [], |r| r.get(0))?;
        if !has {
            return Ok(None);
        }
        Ok(self.conn.query_row("SELECT name FROM auth_tokens WHERE id = ?1", [id], |r| r.get(0)).optional()?)
    }

    /// 这个令牌能不能登记（新建或换绑）机器 `name`（#507）：只有界面「添加机器」新建的令牌绑定了机器名
    /// （`auth_tokens.host`），名字完全一致才行；普通访问令牌、绑定了别的名字的令牌都不行（`HOST_NAME_TAKEN`，
    /// 旧版 board host 遇到它会退出、不会反复重报）。没有令牌表（只用 board-core 的库，不经过服务端）时不限
    pub(crate) fn token_may_register(&self, id: i64, name: &str) -> Result<()> {
        let has: bool = self.conn.query_row("SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'auth_tokens'", [], |r| r.get(0))?;
        if !has {
            return Ok(());
        }
        let sql = if token_host_column(self.conn)? { "SELECT name, host FROM auth_tokens WHERE id = ?1" } else { "SELECT name, NULL FROM auth_tokens WHERE id = ?1" };
        let bound: Option<(String, Option<String>)> = self.conn.query_row(sql, [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let (tn, host) = bound.unwrap_or_default();
        if host.as_deref() == Some(name) {
            return Ok(());
        }
        let why = match host {
            Some(h) => format!("访问令牌「{tn}」是给机器「{h}」的，不能登记机器「{name}」"),
            None => format!("访问令牌「{tn}」是普通访问令牌，不能登记新机器「{name}」"),
        };
        Err(BoardError::new(
            ErrorCode::HostNameTaken,
            format!("{why}：在界面「机器 → 添加机器」填机器名「{name}」生成命令（会新建绑定这个机器名的令牌），用那条命令在这台机器上重新登录、运行 board host"),
        )
        .with_detail(json!({ "host": name })))
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
    pub(super) fn host_info(&self, h: &HostRow, now: i64) -> Result<HostInfo> {
        let mut info = h.info(now);
        for a in info.agents.iter_mut() {
            a.tripped = self.agent_tripped(h, &a.id)?;
        }
        info.ops = self.host_ops(h.id)?;
        info.token = self.host_token(h, now)?;
        // 服务端上的每块白板都列出来（default/main#330）：机器只上报它已经知道的白板，运行中新建的白板、项目
        // 不在它报过的里面——这里按工作空间派生好目录一起发下去，board host 拿到就建目录、克隆，不用重启。
        // 没有工作空间路径的机器（旧版 board host 逐块配的目录）不派生
        if let Some(ws) = info.workspace.clone().filter(|w| !w.path.trim().is_empty()) {
            let mut st = self.conn.prepare("SELECT b.project || '/' || b.key FROM boards b JOIN projects p ON p.key = b.project ORDER BY p.sort, p.created_at, p.key, b.sort, b.created_at, b.key")?;
            let all = st.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            for board in all {
                if info.dirs.iter().any(|d| d.board == board) {
                    continue;
                }
                if let Some(d) = derive_dir(&ws, &board, &h.os) {
                    info.dirs.push(d);
                }
            }
        }
        // 规则里这台机器有目录的项目里都没有的标签：界面上标出来
        let known = self.host_labels(h)?;
        for r in info.rules.iter_mut() {
            r.missing = r.labels.iter().filter(|l| !known.contains(&l.to_lowercase())).cloned().collect();
        }
        if let Some(rules) = h.claim_rules().filter(|r| r.fallback.is_none()) {
            for dir in info.dirs.iter() {
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
    pub(super) fn host_ops(&self, host_id: i64) -> Result<Vec<HostOp>> {
        let mut st = self.conn.prepare("SELECT at, by_name, text FROM host_ops WHERE host_id = ?1 ORDER BY id DESC LIMIT ?2")?;
        let rows = st.query_map(params![host_id, OPS_KEEP], |r| Ok(HostOp { at: r.get(0)?, by: r.get(1)?, text: r.get(2)? }))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 机器用的令牌（令牌表在服务端那一层建；没有这张表、令牌已经吊销时为空）
    pub(super) fn host_token(&self, h: &HostRow, now: i64) -> Result<Option<HostToken>> {
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

    /// 这台机器有目录的白板所在项目里的标签名（小写）。工作空间能放任意白板，规则里用哪块白板的标签就在哪块生效
    pub(super) fn host_labels(&self, _h: &HostRow) -> Result<Vec<String>> {
        // 领取规则是全局的：哪个项目里有这个标签都算有
        self.all_labels()
    }

    pub(super) fn dispatches(&self, tail: &str, p: impl rusqlite::Params) -> Result<Vec<Dispatch>> {
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
            git: Some(self.git_version()?),
            config_key: self.config_key(&h)?,
            ls: vec![],
        })
    }

    /// 这台机器要写成的样子（[`ConfigSync`]）：board-skills 的版本（随服务端），skills、MCP 用服务器的时库里的
    /// 名称和版本，每个找到的内置 agent 的启动环境和模型怎么配（指定时带上供应商的版本和接口），Git 凭据用服务器的时它的版本。
    /// 内容的哈希前 12 位；一样就不用再写
    pub(crate) fn config_key(&self, h: &HostRow) -> Result<String> {
        use sha2::{Digest, Sha256};
        let providers = self.providers()?;
        let version = |name: &str| providers.iter().find(|p| p.name == name).map(|p| p.version.clone()).unwrap_or_default();
        let mut agents = vec![];
        for a in h.resolved().into_iter().filter(|a| a.found && crate::agentdef::is_builtin(&a.id)) {
            let mode = h.mode_of(&a.id);
            let (provider, model, effort, protocol) = match mode {
                MODE_OWN => (
                    a.run_provider.clone().unwrap_or_default(),
                    a.run_model.clone().unwrap_or_default(),
                    a.run_effort.clone().unwrap_or_default(),
                    a.run_protocol.clone().unwrap_or_default(),
                ),
                _ => Default::default(),
            };
            agents.push(json!({ "id": a.id, "env": a.env, "mode": mode, "provider": provider, "v": version(&provider), "model": model, "effort": effort, "protocol": protocol }));
        }
        let mut body = json!({ "board": env!("CARGO_PKG_VERSION"), "agents": agents });
        for k in &SYNC_KINDS {
            body[k.key] = (k.want)(self, (k.on)(h))?;
        }
        Ok(Sha256::digest(body.to_string().as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect())
    }
}

impl HostRow {
    /// 改 agent 的启动环境前校验（启动环境与桌面端需求 §五）：这台 board host 上报了启动环境、有这个 agent、
    /// 本机有这个环境、agent 支持它、在里面找得到命令。不满足时 USAGE，写明原因
    pub(crate) fn env_check(&self, agent: &str, env: &str) -> Result<EnvCheck> {
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

impl<'a> W<'a> {
    pub(crate) fn host_row(&self, name: &str) -> Result<Option<HostRow>> {
        self.read().host_row(name)
    }

    /// 见 [`R::host_of_proc`]
    pub(super) fn host_of_proc(&self, name: &str, instance: &str) -> Result<Option<HostRow>> {
        self.read().host_of_proc(name, instance)
    }

    /// 校验 board host：机器存在，并且是这个进程在上报
    pub(super) fn host_of(&self, name: &str, instance: &str) -> Result<HostRow> {
        let h = self.host_of_proc(name, instance)?.ok_or_else(|| no_such_host(name))?;
        if h.instance != instance {
            return Err(BoardError::new(ErrorCode::HostNameTaken, format!("机器名「{name}」已经由另一个 board host 使用")));
        }
        Ok(h)
    }

    /// 这台机器现在开着几个会话（= 同时在跑的几个 agent 进程）。`max` 限制的是会话数，不是派发条数：
    /// 定时领取的会话里排着的派发（`session != id`）不占额外的会话位，它们由同一个进程用 `board next` 接着做；
    /// 手动派发的一张卡各自是一个会话（`session` 为空或等于自己）
    pub(super) fn sessions_on(&self, h: &HostRow) -> Result<i64> {
        Ok(self.tx.query_row(
            "SELECT COUNT(DISTINCT COALESCE(session, id)) FROM dispatches WHERE host = ?1 AND state = 'running'",
            [&h.name],
            |r| r.get(0),
        )?)
    }
}
