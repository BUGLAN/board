//! 同步配置：按服务端的共用配置写本机 agent 的 skills、MCP、Git 凭据，以及从本机导入 skills。模型不写（用哪个模型按角色、
//! 派发时带给这一个进程，#576），只读回各 agent 配置文件里现在的模型和强度上报。

use super::*;

/// 一个要同步的 agent（找到了的）：启动环境、Claude Code 的命令、读用户级 skill 的目录
pub(super) struct SyncAgent {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) env: Env,
    pub(super) skill_dir: Option<String>,
}

/// 一次同步要的东西（交给后台线程）
pub(super) struct SyncJob {
    pub(super) rm: Remote,
    pub(super) name: String,
    pub(super) exe: PathBuf,
    /// 回应的请求序号；启动时自己写的为空
    pub(super) id: Option<i64>,
    /// 任务里的 config_key：写的是哪个样子
    pub(super) key: String,
    pub(super) skills_on: bool,
    pub(super) mcp_on: bool,
    /// skills 库（含 board-skills）
    pub(super) lib: Vec<board_core::skills::SkillInfo>,
    /// 服务器上有 MCP 服务器（没有时不用取）
    pub(super) mcp_any: bool,
    pub(super) agents: Vec<SyncAgent>,
    /// `~/.board/host`：MCP 加过哪些、备份放在这里
    pub(super) host_dir: PathBuf,
    /// Git 凭据用服务器的
    pub(super) git_on: bool,
    /// 服务端 Git 凭据的版本（没设时空的）；旧版服务端没有这一项（None）：Git 凭据整项不碰
    pub(super) git: Option<String>,
    /// Board 管的仓库：工作空间里每个项目的目录
    pub(super) repos: Vec<PathBuf>,
}

/// `GET /api/hosts/{名字}/git` 的返回
#[derive(serde::Deserialize, Default)]
pub(super) struct GitView {
    #[serde(default)]
    pub(super) cred: Option<board_core::gitcred::GitCred>,
}

/// 一次同步的结果
pub(super) struct SyncOut {
    pub(super) result: SyncResult,
    pub(super) changes: Vec<String>,
    pub(super) local: Vec<board_core::skills::LocalSkill>,
    pub(super) targets: Vec<skills::Target>,
    pub(super) now: BTreeMap<String, (String, String)>,
    /// 这次取到的供应商 key、MCP 密钥和 Git 令牌（只留在内存里，上传日志前脱敏）
    pub(super) secrets: Vec<String>,
}

/// 同步配置（在后台线程里）：skills（选了「不改」时只装 board-skills）、MCP（选了「不改」时不碰）、
/// Git 凭据（选了「不改」时去掉之前写进去的），最后读回每个内置 agent 配置文件里现在的模型和强度（不写模型，#576）
pub(super) fn run_sync(job: SyncJob) -> SyncOut {
    let mut changes = vec![];
    let mut secrets = vec![];
    // skills
    let list: Vec<(String, Env, String)> = job.agents.iter().filter_map(|a| a.skill_dir.clone().map(|d| (a.id.clone(), a.env.clone(), d))).collect();
    let (targets, bad) = skills::host_targets(list, &job.lib);
    let backup = job.host_dir.join("skills-backup");
    let rm = job.rm.clone();
    let out = skills::sync(&job.lib, &targets, Some(&job.exe), skills::Opts { only_builtin: !job.skills_on, backup: &backup }, |name| rm.get(&format!("/api/skills/{name}"), &[]));
    changes.extend(out.changes.iter().map(|c| format!("skills：{c}")));
    let mut fails: Vec<String> = bad.iter().chain(out.states.iter()).filter(|s| s.state == "failed" || s.state == "behind").map(|s| format!("{}/{}：{}", s.dir, s.skill, s.why.as_deref().unwrap_or("没写进去"))).collect();
    fails.dedup();
    let written = if job.skills_on { job.lib.len() } else { job.lib.iter().filter(|s| s.name == board_core::skills::BUILTIN).count() };
    let skills = SkillsSync {
        state: if !fails.is_empty() { "failed" } else if job.skills_on { "synced" } else { "off" }.into(),
        count: written as i64,
        why: (!fails.is_empty()).then(|| fails.join("；")),
        detail: board_core::SkillsDetail { dirs: targets.iter().map(|t| t.shown.clone()).collect(), replaced: out.replaced.clone(), links: out.links.clone() },
    };
    let local = skills::local_skills(&targets);

    // MCP
    let mcp = if !job.mcp_on {
        McpSync { state: "off".into(), ..Default::default() }
    } else {
        let lib: std::result::Result<Vec<board_core::mcp::McpServer>, String> =
            if job.mcp_any { job.rm.get(&format!("/api/hosts/{}/mcp", job.name), &[]).map_err(|e| format!("取不到 MCP 配置：{}", e.message)) } else { Ok(vec![]) };
        match lib {
            Err(why) => McpSync { state: "failed".into(), why: Some(why), ..Default::default() },
            Ok(lib) => {
                secrets.extend(mcpsync::secret_values(&lib));
                let mut targets = vec![];
                let mut unsupported = vec![];
                let mut bad = vec![];
                for a in &job.agents {
                    if !board_core::mcp::agents().contains(&a.id.as_str()) {
                        unsupported.push(a.id.clone());
                        continue;
                    }
                    match mcpsync::file_of(&a.id, &a.env) {
                        Some((file, shown)) => targets.push(mcpsync::Target { agent: a.id.clone(), file, shown }),
                        None => bad.push(format!("{}：取不到{}里的配置文件位置", a.name, a.env.name)),
                    }
                }
                let out = mcpsync::sync(&lib, &targets, &unsupported, &job.host_dir.join("mcp-sync.json"), &job.host_dir.join("mcp-backup.json"));
                changes.extend(out.changes.iter().map(|c| format!("MCP：{c}")));
                bad.extend(out.states.iter().filter(|s| s.state == "failed").map(|s| format!("{} 的 {}：{}", s.agent, s.server, s.why.as_deref().unwrap_or("失败"))));
                let names = |id: &str| job.agents.iter().find(|a| a.id == id).map(|a| a.name.clone()).unwrap_or_else(|| id.to_string());
                let mut into: Vec<String> = vec![];
                for s in out.states.iter().filter(|s| s.state == "synced") {
                    let n = names(&s.agent);
                    if !into.contains(&n) {
                        into.push(n);
                    }
                }
                McpSync {
                    state: if bad.is_empty() { "synced" } else { "failed" }.into(),
                    count: lib.len() as i64,
                    why: (!bad.is_empty()).then(|| bad.join("；")),
                    detail: board_core::McpDetail { agents: into, replaced: out.replaced },
                }
            }
        }
    };

    // 模型不写（#576）：读回每个内置 agent 配置文件里现在的模型和强度（人自己配的，或以前写进去的）
    let mut files: BTreeMap<String, providersync::Files> = BTreeMap::new();
    for a in job.agents.iter().filter(|a| board_core::agentdef::is_builtin(&a.id)) {
        if let Some(f) = providersync::files_of(&a.id, &a.env) {
            files.insert(a.id.clone(), f);
        }
    }
    let now: BTreeMap<String, (String, String)> = files.iter().filter_map(|(id, f)| providersync::current(f).map(|x| (id.clone(), x))).collect();

    // Git 凭据：用服务器的、服务端设了才取原文；取不到时不动之前配好的（多半是一时连不上）
    let git = match job.git.as_deref() {
        None => board_core::GitSync::default(),
        Some(v) => {
            let cred = if job.git_on && !v.is_empty() {
                job.rm.get::<GitView>(&format!("/api/hosts/{}/git", job.name), &[]).map(|g| g.cred).map_err(|e| format!("取不到 Git 凭据：{}", e.message))
            } else {
                Ok(None)
            };
            match cred {
                Err(why) => board_core::GitSync { state: "failed".into(), why: Some(why), ..Default::default() },
                Ok(c) => {
                    if let Some(token) = c.as_ref().and_then(|c| c.token.as_deref()).filter(|t| !t.trim().is_empty()) {
                        secrets.push(token.to_string());
                    }
                    let out = gitsync::sync(job.git_on, c.as_ref(), &job.repos, &job.host_dir);
                    changes.extend(out.changes);
                    out.sync
                }
            }
        }
    };
    secrets.sort();
    secrets.dedup();
    SyncOut { result: SyncResult { id: job.id, key: job.key, at: 0, skills, mcp, git }, changes, local, targets, now, secrets }
}

impl Host<'_> {
    /// 同步配置（机器详情与配置同步重构 §三）：启动后写一次，取到的任务里有新的同步请求再写一次（在后台）；
    /// 做完回报结果（回报失败下一轮再报）。返回要不要马上上报（agent 配置里的模型、本机的 skill 变了）
    pub(super) fn poll_sync(&mut self) -> bool {
        let mut changed = false;
        if self.sync_job.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            let (_, h) = self.sync_job.take().expect("上面看过");
            match h.join() {
                Ok(out) => {
                    for c in &out.changes {
                        self.say(format!("{} 同步配置：{c}", self.p.green("✓")));
                    }
                    let r = &out.result;
                    let mut bad: Vec<String> = vec![];
                    if r.skills.state == "failed" {
                        bad.push(format!("skills：{}", r.skills.why.as_deref().unwrap_or("没写进去")));
                    }
                    if r.mcp.state == "failed" {
                        bad.push(format!("MCP：{}", r.mcp.why.as_deref().unwrap_or("没写进去")));
                    }
                    if !bad.is_empty() {
                        self.say(format!("{} 同步配置有没写进去的：{}", self.p.yellow("!"), bad.join("；")));
                    }
                    changed = out.local != self.local_skills || out.now != self.agent_now;
                    self.local_skills = out.local;
                    self.skill_targets = out.targets;
                    self.agent_now = out.now;
                    self.remember_secrets(out.secrets);
                    // 装上、换掉了 skill：重新看一遍各 agent 看不看得到 board-skills
                    if !out.changes.is_empty() && self.recheck_skills() {
                        changed = true;
                    }
                    self.sync_out = Some(out.result);
                }
                Err(_) => self.say(format!("{} 同步配置的线程出错了：下次收到同步请求再写", self.p.yellow("!"))),
            }
        }
        if let Some(res) = self.sync_out.take() {
            let body = json!({ "instance": self.instance, "result": res });
            match self.rm().post::<HostInfo>(&format!("/api/hosts/{}/synced", self.name), body) {
                Ok(_) => {}
                Err(e) if e.code == ErrorCode::Unreachable => self.sync_out = Some(res),
                // 旧版服务端没有这个接口：结果不用报
                Err(e) if e.code == ErrorCode::NotFound => {}
                Err(e) => self.say(format!("{} 同步结果没报上去：{}", self.p.yellow("!"), e.message)),
            }
        }
        if self.sync_job.is_some() {
            return changed;
        }
        let Some(w) = self.work.as_ref() else { return changed };
        let ask = w.host.sync.ask.as_ref().map(|a| a.id);
        if self.sync_started && ask.is_none_or(|id| self.sync_seen == Some(id)) {
            return changed;
        }
        self.sync_started = true;
        if ask.is_some() {
            self.sync_seen = ask;
        }
        let used: Vec<HostAgent> = w.host.agents.clone();
        let mut agents = vec![];
        for spec in &self.agents {
            let Some(u) = used.iter().find(|a| a.id == spec.id && a.found) else { continue };
            let env_id = u.env.clone().or_else(|| spec.default_check().map(|c| c.env.clone())).unwrap_or_else(|| "direct".into());
            let Some(env) = self.envs.iter().find(|e| e.id == env_id) else { continue };
            agents.push(SyncAgent {
                id: spec.id.clone(),
                name: spec.name.clone(),
                env: env.clone(),
                skill_dir: spec.skills.first().cloned(),
            });
        }
        let host_dir = self.host_dir();
        let job = SyncJob {
            rm: self.rm().clone(),
            name: self.name.clone(),
            exe: self.exe.clone(),
            id: ask,
            key: w.config_key.clone(),
            skills_on: w.host.sync_skills,
            mcp_on: w.host.sync_mcp,
            lib: w.skills.clone().unwrap_or_default(),
            mcp_any: w.mcp.as_ref().is_some_and(|m| !m.is_empty()),
            agents,
            host_dir,
            git_on: w.host.sync_git,
            git: w.git.clone(),
            // 同一个项目的几块白板是同一个目录：一个目录只配一次
            repos: {
                let mut r: Vec<PathBuf> = ws_pairs(&self.ws, &self.boards, &self.os).into_iter().map(|(_, p)| p).collect();
                r.sort();
                r.dedup();
                r
            },
        };
        self.sync_job = Some((ask, std::thread::spawn(move || run_sync(job))));
        changed
    }

    /// 从这台机器导入 skills（PRD §10.16「共用配置」）：取到的任务里有新的请求就在后台把选中的目录传进库；
    /// 做完回报结果（回报失败下一轮再报），再同步一次（库里多了它们，本机这份内容一样就算已同步）
    pub(super) fn poll_skill_import(&mut self) {
        if self.import_job.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            let (id, h) = self.import_job.take().expect("上面看过");
            let (ok, failed) = h.join().unwrap_or_else(|_| (vec![], vec![SkillImportFail { name: "?".into(), why: "导入的线程出错".into() }]));
            self.import_out = Some((id, ok, failed));
        }
        if let Some((id, ok, failed)) = self.import_out.take() {
            let body = json!({ "instance": self.instance, "id": id, "ok": ok, "failed": failed });
            match self.rm().post::<HostInfo>(&format!("/api/hosts/{}/skills/imported", self.name), body) {
                Ok(_) => {
                    if !ok.is_empty() {
                        self.say(format!("{} 导入了 skills：{}", self.p.green("✓"), ok.join("、")));
                    }
                    for f in &failed {
                        self.say(format!("{} {} 没导入：{}", self.p.yellow("!"), f.name, f.why));
                    }
                    self.import_seen = Some(id);
                }
                Err(e) if e.code == ErrorCode::Unreachable => self.import_out = Some((id, ok, failed)),
                Err(e) => {
                    self.say(format!("{} 导入结果没报上去：{}", self.p.yellow("!"), e.message));
                    self.import_seen = Some(id);
                }
            }
        }
        if self.import_job.is_some() || self.import_out.is_some() {
            return;
        }
        let Some(ask) = self.work.as_ref().and_then(|w| w.host.skill_import.ask.clone()) else { return };
        if self.import_seen == Some(ask.id) {
            return;
        }
        self.say(format!("导入 skills 到共用库：{}", ask.picks.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join("、")));
        let rm = self.rm().clone();
        let targets = self.skill_targets.clone();
        let source = self.name.clone();
        let h = std::thread::spawn(move || {
            let mut ok = vec![];
            let mut failed = vec![];
            for p in &ask.picks {
                let res = skills::import_one(&targets, &p.name, &p.dir, |name, files| rm.post("/api/skills", json!({ "name": name, "files": files, "source": source })));
                match res {
                    Ok(n) => ok.push(n),
                    Err(why) => failed.push(SkillImportFail { name: p.name.clone(), why }),
                }
            }
            (ok, failed)
        });
        self.import_job = Some((ask.id, h));
    }
}
