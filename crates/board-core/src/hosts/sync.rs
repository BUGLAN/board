//! 同步配置（skills、MCP、模型供应商、Git 凭据）和从机器导入 skills：类型、请求和回报。

use super::*;

/// 从一台机器导入 skills（PRD §10.16「共用配置」）：board host 上报本机各 agent skill 目录里、不是同步装的 skill；
/// 人在界面上勾了要导入的，board host 把这几个目录传进共用库（和 `board skills push` 一样），再回报结果
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillImport {
    /// 这台机器的 board host 会上报、会导入；旧版不会，界面上写要升级
    #[serde(default)]
    pub able: bool,
    /// 会按请求重扫本机的 skill（`rescan-skills`）；旧版 board host 不会，界面只能看缓存、要升级
    #[serde(default)]
    pub rescan: bool,
    /// 最近一次请求重扫的时间：短时间重复打开、刷新时不再发新请求
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "rescannedAt")]
    pub rescanned_at: Option<i64>,
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

/// 同一台机器两次重扫请求至少隔多久：10 秒内重复打开弹窗、点刷新不再扫
pub const SKILL_RESCAN_GAP_MS: i64 = 10_000;

/// 同步配置（机器详情与配置同步重构 §三）：把服务器上的供应商、skills、MCP 按机器上的选择写进这台机器的 agent。
/// 服务端算出这台机器要写成的样子（[`R::config_key`]），和上次写好的（`last.key`）不一样就下发请求（`ask`）；
/// board host 启动时写一次、收到请求时写一次，写完回报（[`W::host_synced`]），结果记进操作记录
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSync {
    /// 这台机器的 board host 会按请求同步、上报结果；旧版不会（它按老办法自己定时同步），不给它发请求
    #[serde(default)]
    pub able: bool,
    /// 等 board host 做（或正在做）的同步；做完清掉
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<SyncAsk>,
    /// 最近一次的结果
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<SyncResult>,
}

/// 一次同步配置请求
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncAsk {
    /// 序号：board host 做完带着它回报
    pub id: i64,
    /// 要写成的样子（[`R::config_key`]）
    pub key: String,
    pub by: String,
    pub at: i64,
}

/// 一次同步配置的结果（board host 上报）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncResult {
    /// 回应的请求序号；启动时自己写的那一次为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// 写的是哪个样子（board host 取到的任务里的 `config_key`）
    #[serde(default)]
    pub key: String,
    /// 服务端收到的时间
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub skills: SkillsSync,
    #[serde(default)]
    pub mcp: McpSync,
    /// Git 凭据；旧版 board host 不报（`state` 为空）
    #[serde(default)]
    pub git: GitSync,
}

/// 同步到机器的一类配置这一次的结果（skills、MCP、Git 凭据形状一样，见 [`SyncKind`]）：共同的部分是状态、数量、原因
/// （时间在外层 [`SyncResult::at`]），各自多出来的项是明细 `D`。JSON 里明细和共同的部分平铺在同一层，字段名和拆开之前一样：
/// 旧版 board host 的上报照样能读，新版的上报旧服务端也照样能读（多出来的字段不认就丢掉）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPart<D> {
    /// synced 写好了 | off 不改 | none 服务端没设 | failed 有没写进去的；旧版 board host 不报的那一类为空
    #[serde(default)]
    pub state: String,
    /// 写进去几个：skill（含 board-skills）、MCP 服务器、配好的主机（Git 凭据；旧版 board host 不报，按 `repos` 数）
    #[serde(default)]
    pub count: i64,
    /// 没写进去的原因
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(flatten)]
    pub detail: D,
}

/// skills 这一次同步的结果。synced 库里的都写好了 | off 不改（只装了 board-skills）| failed 有没写进去的
pub type SkillsSync = SyncPart<SkillsDetail>;
/// MCP 这一次同步的结果。synced 写好了 | off 不改 | failed 有没写进去的
pub type McpSync = SyncPart<McpDetail>;
/// Git 凭据这一次同步的结果。synced 配好了 | off 不改（之前写进去的已经去掉）| none 服务端没设 | failed 没配上（令牌为空、git 配置写不进去）
pub type GitSync = SyncPart<GitDetail>;

/// skills 同步的明细
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsDetail {
    /// 写进了哪几个 skill 目录（显示用，例如 ~/.claude/skills）
    #[serde(default)]
    pub dirs: Vec<String>,
    /// 这一次换掉了本机同名的（不是 Board 装的，旧的挪进了 ~/.board/host/skills-backup/）
    #[serde(default)]
    pub replaced: Vec<String>,
    /// 本机同名的是软链接，没有覆盖
    #[serde(default)]
    pub links: Vec<String>,
}

/// MCP 同步的明细
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpDetail {
    /// 写进了哪几个 agent（名称）
    #[serde(default)]
    pub agents: Vec<String>,
    /// 这一次换掉了本机同名的条目（原来的值存进了 ~/.board/host/ 下的备份文件）
    #[serde(default)]
    pub replaced: Vec<String>,
}

/// Git 凭据同步的明细
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDetail {
    /// 凭据的主机
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host: String,
    /// 配好的仓库（目录）；按主机全局配置之后为空，旧版 board host 才报
    #[serde(default)]
    pub repos: Vec<String>,
    /// 没配的仓库：「目录：原因」（远端是 SSH、没有 origin）；按主机全局配置之后为空
    #[serde(default)]
    pub skipped: Vec<String>,
}

/// 同步到机器的一类配置（skills、MCP、Git 凭据）。形状都一样：服务端一份 → 每台机器一个开关（用服务器的 / 不改，`hosts` 表的一列）
/// → 一次同步一行结果（[`SyncPart`]）→ 界面上每台机器一个胶囊。操作记录里那句话（[`sync_words`]）、要写成的样子（[`R::config_key`]）、
/// 改开关（[`W::host_update_as`]）都按 [`SYNC_KINDS`] 遍历，不逐个写。
/// 加一类：这里登记一项、[`SyncResult`] 加一行、`hosts` 表加一列开关、界面的 `web/src/lib/syncKinds.ts` 登记同一个标识；
/// 它自己的存储、下发接口、board host 里怎么写、共用配置里的区块另做
pub(crate) struct SyncKind {
    /// 标识：[`SyncResult`] 里那一行的字段名，也是 `config_key` 里的键
    pub key: &'static str,
    /// 名称（操作记录里失败那一句的开头）
    pub name: &'static str,
    /// `hosts` 表里这台机器的开关那一列（1 用服务器的，0 不改）
    pub col: &'static str,
    /// 这台机器的开关
    pub on: fn(&HostRow) -> bool,
    /// 界面上改开关（[`HostPatch`] 里的那一项）
    pub patch: fn(&HostPatch) -> Option<bool>,
    /// 改开关时另记的一条操作记录（改成用服务器的 / 改成不改）；空的不另记，同步完的结果照样记
    pub op: Option<(&'static str, &'static str)>,
    /// 一次同步的结果里它那一行：状态、原因
    pub part: fn(&SyncResult) -> (&str, Option<&str>),
    /// 操作记录里写好了怎么说。写进 agent 目录的库（`lib`）只写数量，末尾的「已写入」并在一起写
    pub done: fn(&SyncResult) -> String,
    /// 没写好、上报里没有原因时怎么说
    pub fail: &'static str,
    /// 别的状态（不改、还没报）时怎么说；空的不写
    pub idle: Option<&'static str>,
    /// 写进 agent 目录的库（skills、MCP）：操作记录里并成一段（「6 个 skill、2 个 MCP 已写入，替换了本机同名的 …」）
    pub lib: bool,
    /// 这一次换掉了本机同名的（`lib` 那一段末尾写）
    pub replaced: fn(&SyncResult) -> &[String],
    /// 要写成的样子里这一类（[`R::config_key`]）：开关开着时库里的名称和版本（Git 凭据是它的版本）
    pub want: fn(&R, bool) -> Result<serde_json::Value>,
}

/// 同步到机器的几类配置，按界面上的顺序
pub(crate) const SYNC_KINDS: [SyncKind; 3] = [
    SyncKind {
        key: "skills",
        name: "skills",
        col: "sync_skills",
        on: |h| h.sync_skills,
        patch: |p| p.sync_skills,
        op: None,
        part: |r| (&r.skills.state, r.skills.why.as_deref()),
        done: |r| format!("{} 个 skill", r.skills.count),
        fail: "没写进去",
        idle: Some("只装 board-skills"),
        lib: true,
        replaced: |r| &r.skills.detail.replaced,
        // 不改时库里的一个都不写（board-skills 随服务端版本，已经在 `board` 那一项里）
        want: |r, on| Ok(json!(if on { r.skills()?.into_iter().map(|s| (s.name, s.version)).collect() } else { vec![] })),
    },
    SyncKind {
        key: "mcp",
        name: "MCP",
        col: "sync_mcp",
        on: |h| h.sync_mcp,
        patch: |p| p.sync_mcp,
        op: None,
        part: |r| (&r.mcp.state, r.mcp.why.as_deref()),
        done: |r| format!("{} 个 MCP", r.mcp.count),
        fail: "没写进去",
        idle: None,
        lib: true,
        replaced: |r| &r.mcp.detail.replaced,
        want: |r, on| Ok(json!(if on { r.mcp_index()?.into_iter().map(|s| (s.name, s.version)).collect() } else { vec![] })),
    },
    SyncKind {
        key: "git",
        name: "Git 凭据",
        col: "sync_git",
        on: |h| h.sync_git,
        patch: |p| p.sync_git,
        op: Some(("Git 凭据改成用服务器的", "Git 凭据改成不改：去掉之前写进用户级 git 配置的")),
        part: |r| (&r.git.state, r.git.why.as_deref()),
        done: |r| {
            if !r.git.detail.repos.is_empty() {
                format!("Git 凭据（{}）已配进 {} 个仓库", r.git.detail.host, r.git.detail.repos.len())
            } else if r.git.detail.host.is_empty() {
                "Git 凭据已按主机全局生效".into()
            } else {
                format!("Git 凭据（{}）已按主机全局生效", r.git.detail.host)
            }
        },
        fail: "没配上",
        idle: None,
        lib: false,
        replaced: |_| &[],
        // 用服务器的时带上它的版本（没设时空的：之前配进去的要去掉）；不改时为空
        want: |r, on| Ok(if on { json!(r.git_version()?) } else { serde_json::Value::Null }),
    },
];

/// 一次同步配置写进操作记录的一句话，例如「同步配置：6 个 skill、2 个 MCP 已写入，替换了本机同名的 frontend-design」。
/// agent 的模型不再写（#576），旧版 board host 报上来的那几行不看
pub(crate) fn sync_words(r: &SyncResult) -> String {
    let mut parts: Vec<String> = vec![];
    // 写进 agent 目录的库（skills、MCP）并成一段，别的（Git 凭据）各自一段
    let (mut lib, mut own, mut done, mut replaced) = (vec![], vec![], false, vec![]);
    for k in &SYNC_KINDS {
        let (state, why) = (k.part)(r);
        // 名称是英文结尾的和「失败」之间空一格：「MCP 失败」「Git 凭据失败」
        let gap = if k.name.ends_with(|c: char| c.is_ascii_alphanumeric()) { " " } else { "" };
        let words = match state {
            "synced" => Some((k.done)(r)),
            "failed" => Some(format!("{}{gap}失败（{}）", k.name, why.unwrap_or(k.fail))),
            _ => k.idle.map(String::from),
        };
        if k.lib {
            lib.extend(words);
            done |= state == "synced";
            replaced.extend((k.replaced)(r).iter().map(String::as_str));
        } else {
            own.extend(words);
        }
    }
    let mut tail = lib.join("、");
    if done {
        tail.push_str(" 已写入");
    }
    if !replaced.is_empty() {
        tail.push_str(&format!("，替换了本机同名的 {}", replaced.join("、")));
    }
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts.extend(own);
    format!("同步配置：{}", parts.join("；"))
}

impl<'a> W<'a> {
    /// 服务端的 Git 凭据改了、删了：用服务器的机器各记一条操作记录（不含令牌原文）
    pub(crate) fn git_ops(&mut self, by: &str, text: &str) -> Result<()> {
        for h in self.read().host_rows()?.into_iter().filter(|h| h.sync_git) {
            self.host_op(h.id, by, text)?;
        }
        Ok(())
    }

    pub(super) fn save_config_sync(&mut self, host_id: i64, cs: &ConfigSync) -> Result<()> {
        self.tx.execute("UPDATE hosts SET config_sync = ?2 WHERE id = ?1", params![host_id, serde_json::to_string(cs).unwrap_or_default()])?;
        self.touch_hosts();
        Ok(())
    }

    /// 算一遍每台机器要写成的样子（[`R::config_key`]）：和上次写好的不一样、也没有等着的同一个请求，就下发「同步配置」请求；
    /// 改回了上次写好的样子，就收回等着的请求。旧版 board host（不会按请求同步）不发。
    /// 离线的机器照样记下请求：上线后（board host 启动时也会写一次）按它写。
    /// 改了配置的写事务提交前调用（`Store::write`），服务端启动时也调用一次（board-skills 的版本随服务端）
    pub fn request_config_sync(&mut self) -> Result<()> {
        let by = if !self.config_by.trim().is_empty() {
            self.config_by.trim().to_string()
        } else {
            match self.origin.via {
                Via::Ui => "人".to_string(),
                Via::Cli => "命令行".to_string(),
                Via::System => "服务端".to_string(),
            }
        };
        for h in self.read().host_rows()? {
            if !h.config_sync.able {
                continue;
            }
            let key = self.read().config_key(&h)?;
            let done = h.config_sync.last.as_ref().is_some_and(|l| l.key == key);
            let waiting = h.config_sync.ask.as_ref().is_some_and(|a| a.key == key);
            if done && h.config_sync.ask.is_some() {
                let cs = ConfigSync { ask: None, ..h.config_sync.clone() };
                self.save_config_sync(h.id, &cs)?;
            } else if !done && !waiting {
                self.ask_config_sync(&h, key, &by)?;
            }
        }
        Ok(())
    }

    pub(super) fn ask_config_sync(&mut self, h: &HostRow, key: String, by: &str) -> Result<()> {
        let id = h.ask_seq + 1;
        self.tx.execute("UPDATE hosts SET ask_seq = ?2 WHERE id = ?1", params![h.id, id])?;
        let cs = ConfigSync { ask: Some(SyncAsk { id, key, by: by.to_string(), at: self.now }), ..h.config_sync.clone() };
        self.save_config_sync(h.id, &cs)
    }

    /// 「重试」：写入失败后重新下发同步请求（要写成的样子没变也发）
    pub fn host_sync_retry(&mut self, name: &str, by: &str) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if !h.config_sync.able {
            return Err(BoardError::usage(format!("机器「{name}」的 board 是旧版（{}），不会按请求同步配置：升级后再试", h.version)));
        }
        let key = self.read().config_key(&h)?;
        self.ask_config_sync(&h, key, by)?;
        self.read().host(name, self.now)
    }

    /// board host 回报一次同步配置的结果（启动时写的那一次 `id` 为空）：存下来，回应的是等着的请求就清掉它，记一条操作记录。
    /// 要写成的样子在它写的时候又变了（`key` 对不上）：重新下发
    pub fn host_synced(&mut self, name: &str, instance: &str, mut res: SyncResult) -> Result<HostInfo> {
        let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
        if h.instance != instance {
            return Err(BoardError::usage("这台机器已经换了一个 board host 进程：结果不收"));
        }
        res.at = self.now;
        let text = sync_words(&res);
        let mut cs = ConfigSync { able: true, last: Some(res.clone()), ..h.config_sync.clone() };
        if cs.ask.as_ref().is_some_and(|a| Some(a.id) == res.id || a.key == res.key) {
            cs.ask = None;
        }
        self.save_config_sync(h.id, &cs)?;
        let by = h.config_sync.ask.as_ref().filter(|a| Some(a.id) == res.id).map(|a| a.by.clone()).unwrap_or_else(|| "board host".into());
        self.host_op(h.id, &by, &text)?;
        // 写的时候又改了：再发一次
        let now_key = self.read().config_key(&self.host_row(name)?.ok_or_else(|| no_such_host(name))?)?;
        if now_key != res.key && cs.ask.is_none() {
            let h = self.host_row(name)?.ok_or_else(|| no_such_host(name))?;
            self.ask_config_sync(&h, now_key, "人")?;
        }
        self.read().host(name, self.now)
    }

    /// 从这台机器导入 skills（PRD §10.16「共用配置」，只能在登录后的界面上做）：记下要导入的（名称 + 所在目录），
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
}
