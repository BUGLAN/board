//! 「这台机器能不能接这块白板的卡」只在这里判断（PRD §9.1「能不能接」）。
//!
//! 手动派发（`hosts.rs` 的 `dispatch_create_from`）、定时领取（`dispatch_pull`）、定时建卡挑机器（`card_crons.rs`）
//! 和界面（工作区数据里的 `blocks`）以前各写各的，条件互相对不上；现在都问这里：
//!
//! - [`R::board_unfit`]：机器 × 白板。还没有配工作空间、项目设了仓库但目录不对（不存在、不是 git 仓库、`origin` 不是
//!   项目的仓库）、环境检查没通过或还没跑完时接不了，返回原因 [`Unfit`]。
//! - [`R::agent_tripped`]：机器 × agent。这个 agent 在这台机器上连续 [`TRIP_AFTER`] 次异常退出后暂停接单，
//!   人在机器详情里点「恢复」才继续。
//! - [`R::blocks`]：所有接不了的「机器 × 白板」，随工作区数据发给界面，界面只照着显示，不自己再算一遍。
//!
//! 「现在会不会马上开始」（离线、暂停接单、没有空位）不在这里：那只决定排不排队，不决定能不能派。

use crate::hosts::{derive_dir, readiness, workspace_of, HostRow, Readiness};
use crate::{BoardError, BoardRef, ErrorCode, Result, R};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// 一个 agent 在一台机器上连续几次异常退出就暂停接单
pub const TRIP_AFTER: usize = 3;

/// 不能把这块白板的卡派给这台机器的原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unfit {
    /// 机器还没有配工作空间
    NoWorkspace,
    /// 项目设了仓库地址，机器上它的目录还不存在；`clone` 是克隆的进展或失败原因
    DirMissing { path: String, clone: Option<String> },
    /// 项目设了仓库地址，机器上它的目录不是 git 仓库
    DirNotGit { path: String },
    /// 目录是 git 仓库，但 `origin` 不是项目的仓库
    WrongRepo { path: String, origin: Option<String>, repo: String },
    /// 环境检查没通过：输出的最后一行
    CheckFailed(String),
    /// 环境检查还没跑完（旧版 board host 不会检查）
    CheckPending,
}

impl Unfit {
    pub fn code(&self) -> ErrorCode {
        match self {
            Unfit::CheckFailed(_) | Unfit::CheckPending => ErrorCode::BoardNotReady,
            _ => ErrorCode::BoardNotOnHost,
        }
    }

    /// 界面上写在机器旁边的一句
    pub fn short(&self) -> String {
        match self {
            Unfit::NoWorkspace => "还没有配工作空间".into(),
            Unfit::DirMissing { path, clone: Some(c) } => format!("{path} 还没有克隆下来：{c}"),
            Unfit::DirMissing { path, clone: None } => format!("{path} 还不存在，仓库没有克隆下来"),
            Unfit::DirNotGit { path } => format!("{path} 不是 git 仓库"),
            Unfit::WrongRepo { path, origin: Some(o), repo } => format!("{path} 的 origin 是 {o}，不是项目的仓库 {repo}"),
            Unfit::WrongRepo { path, origin: None, repo } => format!("{path} 没有 origin，项目的仓库是 {repo}"),
            Unfit::CheckFailed(last) => format!("环境检查没通过：{last}"),
            Unfit::CheckPending => "环境检查还没跑完".into(),
        }
    }

    /// 手动派发被拒时的报错
    pub fn error(&self, host: &str, board: &BoardRef) -> BoardError {
        let path = board.path();
        let text = match self {
            Unfit::NoWorkspace => format!("机器「{host}」还没有配工作空间：先在机器详情里填上，{} 项目的目录是 <工作空间>/{}", board.project, board.project),
            Unfit::DirMissing { .. } => format!("机器「{host}」上 {}：等它克隆完，或在机器详情里重新克隆", self.short()),
            Unfit::DirNotGit { .. } => format!("机器「{host}」上 {}：把它换成项目仓库的克隆", self.short()),
            Unfit::WrongRepo { .. } => format!("机器「{host}」上 {}：改项目设置里的仓库地址，或把那台机器的目录换成项目的仓库", self.short()),
            Unfit::CheckFailed(last) => format!("机器「{host}」上 {path} 的环境检查没通过：{last}"),
            Unfit::CheckPending => format!("机器「{host}」还没跑完 {path} 的环境检查：等它跑完再派（旧版 board host 不会检查，要先升级 board）"),
        };
        BoardError::new(self.code(), text).with_detail(json!({ "host": host, "board": path }))
    }
}

/// 一台机器接不了一块白板的卡（发给界面）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub host: String,
    /// `<项目>/<白板>`
    pub board: String,
    /// 手动派发会得到的错误码：`BOARD_NOT_ON_HOST`、`BOARD_NOT_READY`
    pub code: String,
    /// 原因，一句话
    pub why: String,
}

/// 仓库地址归一成 `主机/路径`，用来比较两个地址是不是同一个仓库：`git@github.com:o/r.git`、
/// `ssh://git@github.com/o/r`、`https://github.com/o/r.git` 都是 `github.com/o/r`。不区分大小写；
/// 认不出形状的（本机路径）原样去掉末尾的 `/` 和 `.git`
pub fn repo_key(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    let u = u.strip_suffix(".git").unwrap_or(u).trim_end_matches('/');
    let out = if let Some((_, rest)) = u.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = host.split_once(':').map_or(host, |(h, _)| h);
        format!("{host}/{path}")
    } else if let Some((head, path)) = u.split_once(':').filter(|(h, _)| h.len() > 1 && !h.contains(['/', '\\'])) {
        // scp 写法 `[用户@]主机:路径`；`C:\repo` 这种盘符不算
        let host = head.rsplit_once('@').map_or(head, |(_, h)| h);
        format!("{host}/{}", path.trim_start_matches('/'))
    } else {
        u.to_string()
    };
    out.to_lowercase()
}

/// 分配「任意机器」的派发时的一台候选机器（已经确认能接、现在能开始）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    /// 还能再开几个会话（会话上限 − 在跑的 − 已经分给它、还没开始的）
    pub free: i64,
    /// 最近一次给它派发的时间；没派过是 0
    pub last_at: i64,
}

/// 在候选机器里挑一台（PRD §9.1「任意机器」）：空位多的优先；一样多时挑最久没接过派发的，几台机器轮流；
/// 再一样按名字。没有空位的不挑；都没有空位时为 None
pub fn pick_host(list: &[Candidate]) -> Option<&Candidate> {
    list.iter().filter(|c| c.free > 0).max_by(|a, b| a.free.cmp(&b.free).then(b.last_at.cmp(&a.last_at)).then(b.name.cmp(&a.name)))
}

impl R<'_> {
    /// 这台机器接不了这块白板的卡时返回原因；能接时为 None。先看工作空间，再看目录，最后看环境检查
    pub(crate) fn board_unfit(&self, h: &HostRow, board: &BoardRef) -> Result<Option<Unfit>> {
        let path = board.path();
        let ws = workspace_of(h.workspace_raw.as_deref(), &h.dirs);
        let own = h.dirs.iter().find(|d| d.board == path);
        if ws.is_none() && own.is_none() {
            return Ok(Some(Unfit::NoWorkspace));
        }
        // 目录：只在项目设了仓库地址时看（没设仓库的项目用空目录，board host 开始派发时自己建）
        let repo: Option<String> = self.conn.query_row("SELECT repo FROM projects WHERE key = ?1", [&board.project], |r| r.get(0)).optional()?.flatten();
        if let Some(repo) = repo.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
            let want = ws.as_ref().and_then(|w| derive_dir(w, &path, &h.os)).map(|d| d.path);
            // 机器按白板上报目录状态；同一个项目的白板共用一个目录，这块白板还没报过时用同一个目录的那条。
            // 路径和现在的工作空间对不上的是换工作空间之前的旧结果，不算数
            let seen = own
                .filter(|d| want.as_deref().is_none_or(|w| w == d.path))
                .or_else(|| want.as_deref().and_then(|w| h.dirs.iter().find(|d| d.path == w)));
            if let Some(d) = seen {
                match d.status.as_deref() {
                    Some("missing") => {
                        let clone = d.clone.as_ref().map(|c| match c.state.as_str() {
                            "failed" => format!("克隆失败，{}", c.message),
                            _ => "正在克隆".to_string(),
                        });
                        return Ok(Some(Unfit::DirMissing { path: d.path.clone(), clone }));
                    }
                    Some("not_git") => return Ok(Some(Unfit::DirNotGit { path: d.path.clone() })),
                    Some("ok") if d.origin.as_deref().is_none_or(|o| repo_key(o) != repo_key(repo)) => {
                        return Ok(Some(Unfit::WrongRepo { path: d.path.clone(), origin: d.origin.clone(), repo: repo.to_string() }));
                    }
                    // 旧版 board host 不上报目录状态：不拦
                    _ => {}
                }
            }
        }
        let cmd = self.ready_check(board.id)?.for_os(&h.os);
        Ok(match readiness(cmd.as_deref(), own.and_then(|d| d.ready.as_ref())) {
            Readiness::Failed(last) => Some(Unfit::CheckFailed(last)),
            Readiness::Pending => Some(Unfit::CheckPending),
            Readiness::NoCheck | Readiness::Passed => None,
        })
    }

    /// 这个 agent 在这台机器上最近连续异常退出了几次（最多数到 [`TRIP_AFTER`] + 1）。只看真的跑过 agent 的派发：
    /// 没开始的（已取消、没领到）、不是 agent 的问题的（人停止、机器掉线）不算，也不打断连续；交审、做不下去、超时、
    /// 拆解完都说明 agent 能跑，打断连续。人点「恢复」之后重新数
    pub(crate) fn agent_streak(&self, h: &HostRow, agent: &str) -> Result<usize> {
        let since = h.run.get(agent).and_then(|r| r.resumed_at).unwrap_or(0);
        let mut st = self.conn.prepare(
            "SELECT result FROM dispatches
             WHERE host_id = ?1 AND agent = ?2 AND state = 'done' AND ended_at > ?3
               AND result NOT IN ('canceled', 'unclaimed', 'stopped', 'lost')
             ORDER BY ended_at DESC, id DESC LIMIT ?4",
        )?;
        let rows = st.query_map(params![h.id, agent, since, (TRIP_AFTER + 1) as i64], |r| r.get::<_, Option<String>>(0))?;
        let mut n = 0;
        for r in rows {
            if r?.as_deref() != Some("crashed") {
                break;
            }
            n += 1;
        }
        Ok(n)
    }

    /// 这个 agent 在这台机器上暂停接单了（连续异常退出）
    pub(crate) fn agent_tripped(&self, h: &HostRow, agent: &str) -> Result<bool> {
        Ok(self.agent_streak(h, agent)? >= TRIP_AFTER)
    }

    /// 所有接不了的「机器 × 白板」，按机器名、白板排序
    pub fn blocks(&self) -> Result<Vec<Block>> {
        let hosts = self.host_rows()?;
        if hosts.is_empty() {
            return Ok(vec![]);
        }
        let mut st = self.conn.prepare("SELECT id, project, key FROM boards ORDER BY project, key")?;
        let boards = st.query_map([], |r| Ok(BoardRef { id: r.get(0)?, project: r.get(1)?, key: r.get(2)? }))?.collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = vec![];
        for h in &hosts {
            for b in &boards {
                if let Some(u) = self.board_unfit(h, b)? {
                    out.push(Block { host: h.name.clone(), board: b.path(), code: u.code().as_str().to_string(), why: u.short() });
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{pick_host, repo_key, Candidate};

    fn c(name: &str, free: i64, last_at: i64) -> Candidate {
        Candidate { name: name.into(), free, last_at }
    }

    /// 挑机器的规则，一行一种情况：候选（名字、空位、最近一次派发的时间）→ 挑中谁
    #[test]
    fn pick_host_prefers_free_slots_then_rotates() {
        let cases: Vec<(&str, Vec<Candidate>, Option<&str>)> = vec![
            ("没有候选", vec![], None),
            ("都没有空位", vec![c("a", 0, 1), c("b", 0, 2)], None),
            ("只有一台有空位", vec![c("a", 0, 1), c("b", 1, 9)], Some("b")),
            ("空位多的优先，不管谁更久没接", vec![c("a", 1, 1), c("b", 3, 9)], Some("b")),
            ("空位一样多：最久没接过的", vec![c("a", 2, 9), c("b", 2, 3)], Some("b")),
            ("空位一样多：没接过的（0）最先", vec![c("a", 2, 5), c("b", 2, 0)], Some("b")),
            ("都一样：按名字", vec![c("b", 1, 4), c("a", 1, 4)], Some("a")),
            ("负数空位（上限调小了）不挑", vec![c("a", -1, 0), c("b", 1, 9)], Some("b")),
        ];
        for (what, list, want) in cases {
            assert_eq!(pick_host(&list).map(|x| x.name.as_str()), want, "{what}");
        }
    }

    #[test]
    fn repo_addresses_compare_across_spellings() {
        let want = "github.com/buglan/board";
        for u in [
            "git@github.com:BUGLAN/board.git",
            "ssh://git@github.com/buglan/board",
            "ssh://git@github.com:22/buglan/board.git",
            "https://github.com/BUGLAN/board.git",
            "https://user:pw@github.com/buglan/board/",
            "  https://github.com/buglan/board  ",
        ] {
            assert_eq!(repo_key(u), want, "{u}");
        }
        assert_ne!(repo_key("git@github.com:buglan/board.git"), repo_key("git@github.com:buglan/board-docs.git"));
        assert_ne!(repo_key("git@github.com:o/r.git"), repo_key("git@gitlab.com:o/r.git"));
        // 本机路径、盘符不当成 scp 写法
        assert_eq!(repo_key("/srv/git/board.git"), "/srv/git/board");
        assert_eq!(repo_key("C:\\repos\\board"), "c:\\repos\\board");
    }
}
