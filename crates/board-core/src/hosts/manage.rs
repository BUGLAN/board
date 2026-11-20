//! 远程管理：界面发给机器的请求（重新检查、重启、升级、排空、安装和更新 agent）、自检、操作记录、自动升级。

use super::*;

/// 自检（PRD §10.12「自检」）：人在机器详情点「自检」（整台机器，或「本机 agent」的某一行），board host 在后台检查
/// agent 能不能干活、每块白板的目录能不能推送，服务端补上 board 版本是否落后。结果一直保留到下一次自检
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SelfTestWire", into = "SelfTestWire")]
pub struct SelfTest {
    /// 这台机器的 board host 会做自检（上报里带 `selftest: true`）；旧版不会，界面上置灰、要升级
    pub able: bool,
    /// 等 board host 做（或正在做）的自检，按请求的先后；做完一个清掉一个。
    /// 可以同时有几个「只查一个 agent」的请求（每个 agent 一个），整台机器的请求和它们互斥
    pub asks: Vec<SelfTestAsk>,
    /// 最近的结果：agent（按本机 agent 的顺序）、git（按白板）、版本
    pub items: Vec<SelfTestItem>,
}

/// [`SelfTest`] 存进数据库、发给界面和 board host 的样子：除了 `asks`，还带最早的一个请求 `ask`，
/// 给只认一个请求的旧版 board host（0.1.40 及之前）：它一次做一个，做完回报后下一个顶上来。
/// 读的时候没有 `asks`（旧数据、旧服务端）就把 `ask` 当成唯一的请求
#[derive(Serialize, Deserialize)]
pub(super) struct SelfTestWire {
    #[serde(default)]
    pub(super) able: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) asks: Vec<SelfTestAsk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ask: Option<SelfTestAsk>,
    #[serde(default)]
    pub(super) items: Vec<SelfTestItem>,
}

impl From<SelfTestWire> for SelfTest {
    fn from(w: SelfTestWire) -> Self {
        let asks = if w.asks.is_empty() { w.ask.into_iter().collect() } else { w.asks };
        SelfTest { able: w.able, asks, items: w.items }
    }
}

impl From<SelfTest> for SelfTestWire {
    fn from(s: SelfTest) -> Self {
        SelfTestWire { able: s.able, ask: s.asks.first().cloned(), asks: s.asks, items: s.items }
    }
}

impl SelfTest {
    /// 还没过期（[`SELFTEST_STALE_MS`] 内请求的）的自检请求
    pub fn live(&self, now: i64) -> impl Iterator<Item = &SelfTestAsk> {
        self.asks.iter().filter(move |a| a.at + SELFTEST_STALE_MS > now)
    }
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
    /// agent-install / agent-update：哪个内置 agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
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
pub const ASK_KINDS: [&str; 7] = ["recheck", "restart", "upgrade", "drain", "cancel", "agent-install", "agent-update"];
/// 机器详情「最近」里带几条操作记录
pub const OPS_KEEP: i64 = 30;
/// 自动升级临时失败（发版包还没出来、下载不了、网络不通）后隔多久再试：10 分钟
pub const AUTO_UPGRADE_RETRY_MS: i64 = 10 * 60 * 1000;
/// 同一个服务端版本最多自动试几次（含第一次）
pub const AUTO_UPGRADE_TRIES: i64 = 5;
/// board host 报的升级失败原因里带这句：机器上的 board 太旧、不会自己升级（没有 `board update`），自动升级不再重试
pub const NO_SELF_UPDATE: &str = "没有 board update";

/// 版本号 a 比 b 旧（SemVer 的常见写法：`v` 前缀、`x.y.z`、`-rc.1` 预发布、`+build` 后缀）。
/// agent 的 `--version` 前面可能带产品名（`codex-cli 0.169.0`），先找第一段数字。
pub fn version_lt(a: &str, b: &str) -> bool {
    fn parse(v: &str) -> Option<(Vec<u64>, Option<Vec<String>>)> {
        let s = v.trim();
        let start = s.find(|c: char| c.is_ascii_digit())?;
        let s = &s[start..];
        // 版本号后面可能跟着产品名（`2.1.300 (Claude Code)`）：只取到第一段不属于版本号的字符
        let end = s.find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))).unwrap_or(s.len());
        let s = s[..end].split('+').next().unwrap_or_default();
        let (core, pre) = s.split_once('-').map_or((s, None), |(c, p)| (c, Some(p)));
        let nums: Vec<u64> = core.split('.').take(3).map(|x| x.parse().ok()).collect::<Option<_>>()?;
        if nums.is_empty() {
            return None;
        }
        let mut nums = nums;
        nums.resize(3, 0);
        let pre = pre
            .filter(|p| !p.is_empty())
            .map(|p| p.split('.').map(|x| x.to_ascii_lowercase()).collect::<Vec<_>>());
        Some((nums, pre))
    }
    let (Some((mut x, xp)), Some((mut y, yp))) = (parse(a), parse(b)) else { return false };
    x.resize(3, 0);
    y.resize(3, 0);
    if x != y {
        return x < y;
    }
    // 有预发布标记的比同一个正式版本旧；两段预发布按数字、字符串逐段比
    match (xp, yp) {
        (Some(p), Some(q)) => {
            for (u, v) in p.iter().zip(q.iter()) {
                match (u.parse::<u64>(), v.parse::<u64>()) {
                    (Ok(a), Ok(b)) if a != b => return a < b,
                    (Ok(_), Ok(_)) => {}
                    (Ok(_), Err(_)) => return true,
                    (Err(_), Ok(_)) => return false,
                    (Err(_), Err(_)) if u != v => return u < v,
                    _ => {}
                }
            }
            p.len() < q.len()
        }
        (Some(_), None) => true,
        _ => false,
    }
}

impl<'a> W<'a> {
    /// board host 对远程管理请求的回报（在上报里）：
    /// - `asked`：开始处理（重启、升级）或做完（重新检查）了这个请求，清掉它，记一条操作
    /// - `ask_error`：升级（或重启）没成，记下原因；为升级暂停的接单恢复
    /// - 为升级暂停了接单、请求已经交给它、换了一个 board host 进程上来：升级做完了（版本变了）或没生效，恢复接单
    pub(super) fn host_acks(&mut self, h: &HostRow, rep: &HostReport) -> Result<()> {
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
                    "agent-install" => {
                        let name = ask.agent.as_deref().and_then(|id| h.agents.iter().find(|a| a.id == id)).map(|a| a.name.as_str()).unwrap_or("agent");
                        self.host_op(h.id, sys, &format!("开始安装 {name}"))?;
                    }
                    "agent-update" => {
                        let name = ask.agent.as_deref().and_then(|id| h.agents.iter().find(|a| a.id == id)).map(|a| a.name.as_str()).unwrap_or("agent");
                        self.host_op(h.id, sys, &format!("开始更新 {name}{}", ask.to.as_deref().filter(|s| !s.is_empty()).map(|v| format!(" 到 {v}")).unwrap_or_default()))?;
                    }
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
            let retry = self.auto_upgrade_failed(h, &text)?;
            self.host_op(h.id, sys, &format!("{text}{}{retry}", if h.up_pause { "；已恢复接单" } else { "" }))?;
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

    /// 自动升级（PRD §9.1「自动升级」）：开关开着、这台机器的 board 比服务端旧、没有别的请求在等，就替它发起一次
    /// 和人点「升级到 <版本>」一样的升级（暂停接单，等在跑的派发结束再升）。
    /// 同一个服务端版本：第一次直接试；没成时按 [`Self::auto_upgrade_failed`] 排的时间再试，最多 [`AUTO_UPGRADE_TRIES`] 次。
    /// 服务端先部署、发版包晚十几分钟才出来时，机器第一次下载不到，过 10 分钟再试就能升上去
    pub(super) fn auto_upgrade_host(&mut self, name: &str) -> Result<()> {
        if !self.read().auto_upgrade()? {
            return Ok(());
        }
        let Some(h) = self.host_row(name)? else { return Ok(()) };
        let server = env!("CARGO_PKG_VERSION");
        if h.ask.is_some() || h.version.trim().is_empty() || !version_lt(&h.version, server) {
            return Ok(());
        }
        let tries = if h.up_tried.as_deref() == Some(server) {
            // 这个版本试过了：上一次临时失败、到了重试时间、次数没用完才再试；还在做的、不再试的（`0`）不动
            match h.up_retry {
                Some(at) if at > 0 && at <= self.now && h.up_tries < AUTO_UPGRADE_TRIES => h.up_tries + 1,
                _ => return Ok(()),
            }
        } else {
            1
        };
        self.tx.execute("UPDATE hosts SET up_tried = ?2, up_tries = ?3, up_retry = NULL WHERE id = ?1", params![h.id, server, tries])?;
        let by = if tries == 1 { "自动升级".to_string() } else { format!("自动升级（第 {tries} 次）") };
        self.host_ask(name, "upgrade", false, &by)?;
        Ok(())
    }

    /// agent 自动更新（PRD §9.1「agent」）：开关开着、没有在跑的派发、内置 agent 报了新版本且能按实际安装方式升级，
    /// 就替它发一次和界面上「更新」相同的请求。失败后 board host 把状态记成 failed，不会每隔 10 秒反复重试；
    /// 人点「重试」仍然可以再发。
    pub(super) fn auto_update_agents(&mut self, name: &str, rep: &HostReport) -> Result<()> {
        let Some(h) = self.host_row(name)? else { return Ok(()) };
        if !h.agent_auto || h.ask.is_some() || !rep.running.is_empty() {
            return Ok(());
        }
        // 用按机器设置解析后的启动环境，不按 board host 的默认环境：实际用哪个环境，就更新那里的 agent
        let agents = h.resolved();
        let pick = agents.iter().find(|a| {
            a.found
                && a.manage.as_ref().is_some_and(|m| {
                    m.update_available
                        && m.updatable
                        && m.state != "running"
                        && m.state != "failed"
                        && !m.latest_version.trim().is_empty()
                        && version_lt(&a.version, &m.latest_version)
                })
        });
        let Some(a) = pick else { return Ok(()) };
        let to = a.manage.as_ref().map(|m| m.latest_version.as_str()).unwrap_or_default();
        self.host_agent_ask(name, "agent-update", &a.id, Some(to), "自动更新")?;
        Ok(())
    }

    /// 升级没成（board host 报了 `ask_error`）：这一次是自动升级发起的（试过服务端这个版本、还没排下一次）时，
    /// 临时失败（发版包还没出来、下载 404、网络不通）排上 10 分钟后再试；旧版本不会自己升级（[`NO_SELF_UPDATE`]）、
    /// 次数用完了的不再自动试。返回接在操作记录后面的半句（不是自动升级的这一次时为空）
    pub(super) fn auto_upgrade_failed(&mut self, h: &HostRow, err: &str) -> Result<String> {
        let server = env!("CARGO_PKG_VERSION");
        if h.up_tried.as_deref() != Some(server) || h.up_tries < 1 || h.up_retry.is_some() || !self.read().auto_upgrade()? {
            return Ok(String::new());
        }
        let (at, note) = if err.contains(NO_SELF_UPDATE) {
            (0, "；这台机器的 board 不会自己升级，自动升级不再重试".to_string())
        } else if h.up_tries >= AUTO_UPGRADE_TRIES {
            (0, format!("；自动升级到 {server} 已经试了 {AUTO_UPGRADE_TRIES} 次，不再重试，请手动升级"))
        } else {
            (
                self.now + AUTO_UPGRADE_RETRY_MS,
                format!("；{} 分钟后自动再试（第 {}/{AUTO_UPGRADE_TRIES} 次）", AUTO_UPGRADE_RETRY_MS / 60_000, h.up_tries + 1),
            )
        };
        self.tx.execute("UPDATE hosts SET up_retry = ?2 WHERE id = ?1", params![h.id, at])?;
        Ok(note)
    }

    /// 记一条机器的操作（最近的 OPS_KEEP 条以外的删掉）
    pub(crate) fn host_op(&mut self, host_id: i64, by: &str, text: &str) -> Result<()> {
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
        let mut ask = HostAsk { id: h.ask_seq + 1, kind: kind.into(), by: by.into(), at: self.now, to: None, now: false, agent: None };
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

    /// 安装或更新这台机器上的一个内置 agent（PRD §9.1「agent」）。不暂停接单，也不影响正在跑的派发；
    /// board host 按命令实际所在的位置识别安装方式，装完自己重新检查并上报版本。
    pub fn host_agent_ask(&mut self, name: &str, kind: &str, agent: &str, to: Option<&str>, by: &str) -> Result<HostInfo> {
        if !matches!(kind, "agent-install" | "agent-update") {
            return Err(BoardError::usage(format!("agent 操作只能是 agent-install 或 agent-update，不能是「{kind}」")));
        }
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if !h.online(self.now) {
            return Err(BoardError::usage(format!("机器「{name}」离线：等它上线再{}", if kind == "agent-install" { "安装" } else { "更新" })));
        }
        if let Some(a) = &h.ask {
            let what = if a.kind == "agent-install" { "安装" } else { "更新" };
            return Err(BoardError::usage(format!("机器「{name}」正在处理另一个请求（{what}）：等它完成再试")));
        }
        let a = h.resolved().into_iter().find(|a| a.id == agent).ok_or_else(|| BoardError::usage(format!("机器「{name}」上没有 agent「{agent}」")))?;
        let m = a.manage.clone().unwrap_or_default();
        if m.state == "running" {
            return Err(BoardError::usage(format!("{} 正在{}：等它完成再试", a.name, if kind == "agent-install" { "安装" } else { "更新" })));
        }
        if kind == "agent-install" {
            if a.found {
                return Err(BoardError::usage(format!("{} 已经装好了", a.name)));
            }
            if !m.installable {
                return Err(BoardError::usage(if m.reason.trim().is_empty() { "这个 agent 不能由 board host 安装".into() } else { m.reason.clone() }));
            }
        } else {
            if !a.found {
                return Err(BoardError::usage(format!("{} 还没装：先安装", a.name)));
            }
            if !m.updatable {
                return Err(BoardError::usage(if m.reason.trim().is_empty() { "这个 agent 不能由 board host 更新".into() } else { m.reason.clone() }));
            }
        }
        let to = to.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).or_else(|| Some(m.latest_version.clone()).filter(|s| !s.is_empty()));
        if to.is_none() {
            return Err(BoardError::usage(format!("还不知道 {} 的最新版本：等下一次版本检查后重试", a.name)));
        }
        let by = if by.trim().is_empty() { "人" } else { by.trim() };
        let ask = HostAsk {
            id: h.ask_seq + 1,
            kind: kind.into(),
            by: by.into(),
            at: self.now,
            to: to.clone(),
            now: false,
            agent: Some(agent.into()),
        };
        let target = to.as_deref().unwrap_or("");
        let text = if by.starts_with("自动更新") {
            format!("自动更新 {} 到 {target}", a.name)
        } else if kind == "agent-install" {
            format!("请求安装 {}{}", a.name, if target.is_empty() { String::new() } else { format!(" {target}") })
        } else {
            format!("请求更新 {} 到 {target}", a.name)
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
    /// 要机器在线。同一台机器可以同时查几个不同的 agent（同一个 agent 同时只做一次）；整台机器的自检和它们互斥：
    /// 有没做完的请求时不能整台自检，整台自检没做完时也不能单查。超过 [`SELFTEST_STALE_MS`] 还没做完的算作废。
    /// 不占会话位，不影响在跑的派发
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
        let agent = agent.map(str::trim).filter(|a| !a.is_empty());
        let agent_name = |id: &str| h.agents.iter().find(|a| a.id == id).map(|a| a.name.clone()).unwrap_or_else(|| id.to_string());
        let live: Vec<&SelfTestAsk> = h.selftest.live(self.now).collect();
        if let Some(a) = live.iter().find(|a| a.agent.is_none()) {
            return Err(BoardError::usage(format!("机器「{name}」正在整台自检（{} 请求的）：等它做完再来", a.by)));
        }
        match agent {
            None if !live.is_empty() => {
                let who: Vec<String> = live.iter().filter_map(|a| a.agent.as_deref()).map(agent_name).collect();
                return Err(BoardError::usage(format!("机器「{name}」正在自检 {}：等它们做完再整台自检", who.join("、"))));
            }
            Some(id) => {
                if let Some(a) = live.iter().find(|a| a.agent.as_deref() == Some(id)) {
                    return Err(BoardError::usage(format!("机器「{name}」正在自检 {}（{} 请求的）：等它做完再来", agent_name(id), a.by)));
                }
            }
            None => {}
        }
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
        // 过期没做完的请求丢掉（board host 那边作废了），新请求接在后面
        st.asks.retain(|a| a.at + SELFTEST_STALE_MS > self.now);
        st.asks.push(SelfTestAsk { id: h.ask_seq + 1, agent: agent.map(str::to_string), by: by.into(), at: self.now });
        self.tx.execute(
            "UPDATE hosts SET selftest = ?2, ask_seq = ?3 WHERE id = ?1",
            params![h.id, serde_json::to_string(&st).unwrap_or_default(), h.ask_seq + 1],
        )?;
        self.host_op(h.id, by, &label)?;
        self.touch_hosts();
        self.read().host(name, self.now)
    }

    /// board host 做完了自检：换掉这次检查到的项（整台机器时全部换掉，只查一个 agent 时只换它那一项，别的 agent 的结果不动），
    /// 补上 board 版本是否落后于服务端，清掉这个请求（同时在做的别的请求留着），记一条操作。序号对不上（被新的请求换掉、或是别的 board host 进程）时不记
    pub fn host_selftest_done(&mut self, name: &str, instance: &str, id: i64, items: Vec<SelfTestItem>) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        let Some(ask) = h.selftest.asks.iter().find(|a| a.id == id && h.instance == instance).cloned() else {
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
        st.asks.retain(|a| a.id != id);
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
            let dirs = derived_dirs(&h.dirs, h.workspace().as_ref(), &h.os);
            let order = |i: &SelfTestItem| -> (usize, usize) {
                match i.kind.as_str() {
                    "agent" => (0, h.agents.iter().position(|a| a.id == i.key).unwrap_or(usize::MAX)),
                    "git" => (1, dirs.iter().position(|d| d.board == i.key).unwrap_or(usize::MAX)),
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
}
