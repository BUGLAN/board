//! 「跑第一张卡」清单（default/main#397）：一块白板的卡要被一台机器跑完还差什么，只在这里判断。
//!
//! 前提分散在机器、agent、项目设置、Git 凭据、领取规则几处，以前没有一处告诉人还差什么。服务端按白板算一份清单，
//! 随工作区数据（`checklist`）下发，界面只照着画（和 `fit.rs` 的 `blocks` 一样），每项带一个去处：
//!
//! 1. `host`：有在线、能接这块白板的机器（配了工作空间、目录对，见 [`R::board_unfit`]）
//! 2. `agent`：其中至少一台上有找得到命令、看得到 board-skills、最近一次自检通过的 agent（没自检过算「还没确认」）
//! 3. `repo`：这个项目设了仓库地址（没设也能派发，只是机器上是空目录、agent 推不了代码）
//! 4. `push`：其中至少一台对这个项目的目录自检「能推送」（同上，没自检过算「还没确认」）
//! 5. `auto`：有领取规则，至少一台能接的机器是「自动领」，白板参与自动领取。只影响自动领取，不影响手动派发
//!
//! 自检不自动做（会消耗 agent 的额度）：没自检过的项给出「自检」这个去处，人点了才做。
//! 这块白板有卡被机器交审过（`done`）之后，界面不再提示。

use crate::fit::Unfit;
use crate::hosts::HostRow;
use crate::{BoardRef, Result, R};
use serde::{Deserialize, Serialize};

/// 一块白板的清单
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checklist {
    /// `<项目>/<白板>`
    pub board: String,
    /// 这块白板已经有卡被机器交审过：界面不再提示
    pub done: bool,
    pub items: Vec<CheckItem>,
}

/// 清单的一项
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckItem {
    /// `host` | `agent` | `repo` | `push` | `auto`
    pub key: String,
    /// `ok` 通过 | `fail` 没通过 | `unknown` 还没确认（没自检过）| `wait` 要等前面的项
    pub state: String,
    /// 一句话：通过时写凭什么，没通过时写差什么
    pub why: String,
    /// 去哪处理（界面按它给按钮）：`add-host`、`hosts`、`host-settings:<机器>`、`host-agents:<机器>`、`selftest:<机器>`、
    /// `project-repo`、`git`、`rules`、`accept`、`board-auto`；通过时、要等前面的项时为空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub go: Option<String>,
    /// 只影响自动领取，不影响手动派发
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
}

fn item(key: &str, state: &str, why: impl Into<String>, go: Option<String>) -> CheckItem {
    CheckItem { key: key.into(), state: state.into(), why: why.into(), go, auto: false }
}

impl R<'_> {
    /// 所有白板的清单，按项目、白板的顺序
    pub fn checklist(&self, now: i64) -> Result<Vec<Checklist>> {
        let hosts = self.host_rows()?;
        let sched = self.sched(now)?;
        let rules = !sched.rules.is_empty();
        let mut st = self.conn.prepare(
            "SELECT b.id, b.project, b.key, b.auto, p.repo,
                    EXISTS (SELECT 1 FROM dispatches d WHERE d.board = b.id AND d.result = 'review')
             FROM boards b JOIN projects p ON p.key = b.project
             ORDER BY p.sort, p.created_at, p.key, b.sort, b.created_at, b.key",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    BoardRef { id: r.get(0)?, project: r.get(1)?, key: r.get(2)? },
                    r.get::<_, i64>(3)? != 0,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)? != 0,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = vec![];
        for (b, board_auto, repo, done) in rows {
            let repo = repo.map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
            out.push(self.board_checklist(&hosts, &b, board_auto, repo.as_deref(), rules, done, now)?);
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn board_checklist(&self, hosts: &[HostRow], b: &BoardRef, board_auto: bool, repo: Option<&str>, rules: bool, done: bool, now: i64) -> Result<Checklist> {
        let path = b.path();
        let mut items = vec![];

        // 1. 有在线、能接这块白板的机器
        let online: Vec<&HostRow> = hosts.iter().filter(|h| h.online(now)).collect();
        let mut fits: Vec<&HostRow> = vec![];
        let mut first_unfit: Option<(&HostRow, Unfit)> = None;
        for &h in &online {
            match self.board_unfit(h, b)? {
                None => fits.push(h),
                Some(u) => {
                    if first_unfit.is_none() {
                        first_unfit = Some((h, u));
                    }
                }
            }
        }
        items.push(if hosts.is_empty() {
            item("host", "fail", "还没有机器：在要干活的机器上运行 board host", Some("add-host".into()))
        } else if online.is_empty() {
            let names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
            item("host", "fail", format!("机器都不在线：{}", names.join("、")), Some("hosts".into()))
        } else if fits.is_empty() {
            let (h, u) = first_unfit.expect("在线的机器都接不了");
            let more = if online.len() > 1 { format!("（在线的 {} 台都接不了）", online.len()) } else { String::new() };
            item("host", "fail", format!("{}：{}{more}", h.name, u.short()), Some(format!("host-settings:{}", h.name)))
        } else {
            let names: Vec<&str> = fits.iter().map(|h| h.name.as_str()).collect();
            item("host", "ok", format!("{} 在线，能接这块白板", names.join("、")), None)
        });

        // 2. 能接的机器上有能干活的 agent：找得到命令、看得到 board-skills、自检通过
        items.push(if fits.is_empty() {
            item("agent", "wait", "有能接这块白板的机器之后再看", None)
        } else {
            let mut good = None;
            let mut untested = None;
            let mut testing = None;
            let mut bad: Option<(String, String)> = None;
            for h in &fits {
                if !h.selftest.asks.is_empty() && testing.is_none() {
                    testing = Some(h.name.clone());
                }
                let all = h.resolved();
                let found: Vec<_> = all.iter().filter(|a| a.found).cloned().collect();
                if found.is_empty() && bad.is_none() {
                    // 装了 agent、只是缺 ACP 入口要的命令（Node.js）：说缺什么，别让人去重装 agent（#458）
                    let why = match all.iter().find_map(|a| a.missing.as_ref().map(|m| (a, m))) {
                        Some((a, m)) => format!("{} 上的 {} 缺 {m}：装上后点「重新检查」", h.name, a.name),
                        None => format!("{} 上没找到 agent：装上并登录后点「重新检查」", h.name),
                    };
                    bad = Some((why, h.name.clone()));
                }
                for a in found {
                    if let Some(where_) = &a.no_skill {
                        if bad.is_none() {
                            bad = Some((format!("{} 上的 {} 看不到 board-skills（{where_}）", h.name, a.name), h.name.clone()));
                        }
                        continue;
                    }
                    match h.selftest.items.iter().find(|i| i.kind == "agent" && i.key == a.id) {
                        Some(t) if t.ok => {
                            good.get_or_insert_with(|| format!("{} 上的 {} 自检通过", h.name, a.name));
                        }
                        Some(t) => {
                            if bad.is_none() {
                                bad = Some((format!("{} 上的 {} 自检没通过：{}", h.name, a.name, t.why.as_deref().unwrap_or("没回话")), h.name.clone()));
                            }
                        }
                        None => {
                            untested.get_or_insert_with(|| (format!("{} 上的 {} 还没自检：确认登录过、有模型", h.name, a.name), h.name.clone()));
                        }
                    }
                }
            }
            match (good, testing, untested, bad) {
                (Some(w), ..) => item("agent", "ok", w, None),
                (None, Some(h), ..) => item("agent", "unknown", format!("{h} 正在自检…"), None),
                (None, None, Some((w, h)), _) => item("agent", "unknown", w, Some(format!("selftest:{h}"))),
                (None, None, None, Some((w, h))) => item("agent", "fail", w, Some(format!("host-agents:{h}"))),
                (None, None, None, None) => item("agent", "fail", "能接的机器上都没有能干活的 agent", Some(format!("host-agents:{}", fits[0].name))),
            }
        });

        // 3. 项目设了仓库地址
        items.push(match repo {
            Some(r) => item("repo", "ok", r, None),
            None => item("repo", "fail", format!("项目 {} 还没设仓库地址：机器上是空目录，agent 做完推不了代码", b.project), Some("project-repo".into())),
        });

        // 4. 能接的机器对这个项目的目录自检「能推送」（同一个项目的白板在一台机器上是同一个目录）
        items.push(if repo.is_none() {
            item("push", "wait", "设了仓库地址之后再看", None)
        } else if fits.is_empty() {
            item("push", "wait", "有能接这块白板的机器之后再看", None)
        } else {
            let of_project = |k: &str| k.split_once('/').is_some_and(|(p, _)| p == b.project);
            let mut good = None;
            let mut untested = None;
            let mut bad = None;
            for h in &fits {
                let tests: Vec<_> = h.selftest.items.iter().filter(|i| i.kind == "git" && of_project(&i.key)).collect();
                if tests.iter().any(|t| t.ok) {
                    good.get_or_insert_with(|| format!("{} 能推送", h.name));
                } else if let Some(t) = tests.first() {
                    bad.get_or_insert_with(|| format!("{} 推不了：{}", h.name, t.why.as_deref().unwrap_or("没通过")));
                } else if h.selftest.asks.iter().any(|a| a.agent.is_none()) {
                    untested.get_or_insert_with(|| (format!("{} 正在自检…", h.name), None));
                } else {
                    untested.get_or_insert_with(|| (format!("{} 还没自检能不能推送", h.name), Some(format!("selftest:{}", h.name))));
                }
            }
            match (good, untested, bad) {
                (Some(w), ..) => item("push", "ok", w, None),
                (None, Some((w, go)), _) => item("push", "unknown", w, go),
                (None, None, Some(w)) => item("push", "fail", w, Some("git".into())),
                (None, None, None) => item("push", "unknown", "还没自检能不能推送", None),
            }
        });

        // 5. 自动领取：规则、自动领的机器、白板参与
        let mut miss: Vec<(&str, &str)> = vec![];
        if !rules {
            miss.push(("还没有领取规则", "rules"));
        }
        if !fits.iter().any(|h| h.auto) {
            miss.push(("能接的机器都不是「自动领」", "accept"));
        }
        if !board_auto {
            miss.push(("这块白板不参与自动领取", "board-auto"));
        }
        let mut auto = if miss.is_empty() {
            let names: Vec<&str> = fits.iter().filter(|h| h.auto).map(|h| h.name.as_str()).collect();
            item("auto", "ok", format!("有领取规则，{} 自动领", names.join("、")), None)
        } else {
            let words: Vec<&str> = miss.iter().map(|(w, _)| *w).collect();
            item("auto", "fail", format!("{}：只影响自动领取，手动派发照常", words.join("；")), Some(miss[0].1.into()))
        };
        auto.auto = true;
        items.push(auto);

        Ok(Checklist { board: path, done, items })
    }
}
