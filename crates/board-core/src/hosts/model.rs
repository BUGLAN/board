//! 机器、本机 agent、启动环境、工作空间和目录、领取规则这些数据的类型，以及机器信息（`HostInfo`）。

use super::*;

/* ------------------------------ 数据 ------------------------------ */

/// 本机 agent：board host 启动时按配置文件和内置模板逐个检查（任务下发需求 §三）。
///
/// board host 上报的 `found`、`version` 是它在默认启动环境里的结果；服务端发出去的（[`HostInfo::agents`]）
/// 换成按机器设置实际用的启动环境（`env`）里的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostAgent {
    /// 标识：claude、dsh、codex …
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
    /// 实际用的启动环境里找到了命令、却缺 ACP 入口另外要的命令（例如「Node.js（npx）」，见 [`EnvCheck::missing`]）：
    /// 这时 `found` 是 false，派发、自动领取、自检都不用它（#458）。board host 上报时是第一个这样的启动环境里的
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
    /// 这个 agent 现在用的模型名：派发名字的第二段（`<机器名>/<模型名>-<强度>`）和派发记录里的模型。
    /// board host 上报：内置 agent 读它自己配置文件里现在的模型（同步写进去的、或人自己配的），
    /// 配置文件里新加的 agent 用 `[agents.<标识>] model`；都没有时为空，生成名字时只写 `<机器名>/<agent 标识>`
    #[serde(default)]
    pub model: String,
    /// 这个 agent 配置文件里现在的思考强度（board host 读出来上报，写进派发记录）；没有时为空
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
    /// 旧版 board host 上报的「派发时能选的模型和强度」：不再用，收到也不看
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opts: Option<AgentOpts>,
    /// 服务端发出去时才有：模型怎么配——`off` 不改 | `own` 指定
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_mode: Option<String>,
    /// 服务端发出去时才有：指定的模型；没选时写供应商的第一个
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_model: Option<String>,
    /// 服务端发出去时才有：指定的思考强度；没选时不写
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_effort: Option<String>,
    /// 服务端发出去时才有：指定用供应商的哪种接口（`anthropic`、`openai-chat`、`openai-responses`）；
    /// 没选时用供应商里第一个这个 agent 认的（`Provider::endpoint_for`）。旧版 board host 不看，总是用第一个
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_protocol: Option<String>,
    /// 服务端发出去时才有：机器设置里选的启动环境（可能是从别的机器复制来、这台机器上没有的：那时 `env` 是实际用的）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_env: Option<String>,
    /// 服务端发出去时才有：指定的供应商。给旧版 board host 的写法：不改时是 `@local`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_provider: Option<String>,
    /// 服务端发出去时才有：这个 agent 在这台机器上连续异常退出、已经暂停接单（PRD §9.1「能不能接」），人点「恢复」才继续
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tripped: bool,
    /// 安装、版本检查和更新状态（board host 上报；内置 agent 才有）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manage: Option<AgentManage>,
    /// 配置文件里新加的 agent（`[agents.<标识>]`，不是内置的）：board host 上报。旧版 board host 不报，按内置处理——
    /// 已经移除的内置 agent（[`crate::agentdef::REMOVED_IDS`]）这时被丢掉，自定义的同名 agent 不受影响（#484）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub custom: bool,
}

/// 一个内置 agent 在这台机器上的安装与更新状态（PRD §9.1「agent」）。
///
/// board host 每天查一次 npm 上各内置 agent 的最新版本（不在 10 秒心跳里查），再按本机命令的实际路径判断
/// 安装方式。配置文件里新加的 agent、被配置覆盖了命令的 agent 和在 WSL 里跑的 agent 不代装、不代升。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentManage {
    /// npm-global | brew | script | app | config | wsl | unknown | missing
    pub method: String,
    /// 界面上显示的安装方式：npm 全局、Homebrew、官方安装脚本、桌面 app、配置文件……
    pub method_label: String,
    /// 没找到时给的安装命令；找到时为空
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub install_command: String,
    /// npm 上查到的最新版本
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub latest_version: String,
    /// 最近一次版本检查的时间
    #[serde(default)]
    pub checked_at: i64,
    /// 有新版本
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub update_available: bool,
    /// 能由 board host 安装 / 更新
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub installable: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub updatable: bool,
    /// 不代装、不代升的原因；能管时为空
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// 空 = 空闲 | running 正在安装/更新 | failed 上一次失败
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    /// 上一次失败的原因
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// 旧版 board host 上报的「一个 agent 派发时能选什么」：现在模型、强度只写进配置文件，不再按次传参，收下不用
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
    /// 这个启动环境里的安装方式、最新版本和更新状态
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manage: Option<AgentManage>,
    /// 找到了 agent 命令、却缺 ACP 入口另外要的命令时，缺的是什么（例如「Node.js（npx）」，内置描述的 `needs`）。
    /// 这时 `found` 是 false：旧版服务端、界面不认这一项也照样当不可用；`version` 仍是 agent 的版本（#458）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
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
    /// 模型怎么配：`own` 指定 | 没有或 `off` = 不改（新机器的默认）。以前的 `server`（用服务器的）迁移成了 `own`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// 指定的模型；没有时用供应商的第一个
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 指定的思考强度；没有时不写
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// 指定的供应商名称
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// 指定用供应商的哪种接口；没有时用供应商里第一个这个 agent 认的
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// 人最近一次点「恢复」的时间：连续异常退出从这之后重新数（`fit.rs` 的 `agent_streak`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_at: Option<i64>,
}

/// 一台机器的工作空间（PRD §9.1「工作空间」）：每台设备只配一个目录，每块白板在它里面的目录固定是
/// `<工作空间>/<项目标识>/<白板标识>`，目录不存在时由 board host 建好（白板设了仓库地址时 `git clone`）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostWorkspace {
    /// 配置里写的路径（`~` 不展开）
    pub path: String,
    /// 目录不存在、是在这台机器上给这块白板新建的（还在等仓库地址）
    #[serde(default)]
    pub created: bool,
    /// `--dir <项目>=<路径>` 单独覆盖的项目目录（旧写法 `--dir <项目>/<白板>=<路径>` 只覆盖那一块白板，兼容保留）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<HostDirOverride>,
    /// 项目设置里填了「项目目录」的项目：项目标识 → 目录名，目录是 `<工作空间>/<目录名>`；没填的用项目标识（PRD §4，#488）。
    /// 不存进机器的数据：服务端读机器时按项目表填上，发给界面和 board host；旧版 board host 不认，照旧按项目标识
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub projects: BTreeMap<String, String>,
}

/// `--dir` 单独覆盖的一个项目（或旧写法的一块白板）的目录
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDirOverride {
    /// `<项目>`：项目下所有白板都用这个目录；旧写法 `<项目>/<白板>`：只管这一块白板（[`crate::valid_dir_key`]）
    pub board: String,
    /// 配置里写的路径（`~` 不展开）
    pub path: String,
}

/// 一块白板在一台机器上的目录：从工作空间（或 `--dir` 覆盖）派生出来，加上 board host 上报的检查结果。
/// 不存进数据库，每次读的时候按「这台机器的工作空间 + 这块白板」算出来
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDir {
    /// `<项目>/<白板>`
    pub board: String,
    /// 实际用的路径：`<工作空间>/<项目>`（同一个项目的几块白板是同一个目录；`--dir` 覆盖的是覆盖的路径）
    pub path: String,
    /// 命令行用 `--dir` 单独覆盖：界面上只读，要改启动参数
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
    /// board host 上报过这块白板的目录状态、还没被这台机器上报过的替换（server 发出时才有）
    #[serde(default, skip_serializing_if = "is_false")]
    pub reported: bool,
    /// board host 最近一次在这个目录里 `git fetch origin` 没成的原因（拉成了、没拉过为空）：这台机器先不接这个项目的卡
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_error: Option<String>,
}

pub(super) fn is_false(b: &bool) -> bool {
    !*b
}

/// 一块白板在一台机器上的目录（PRD §9.1「工作空间」）：它所在项目的目录 `<工作空间>/<项目标识>`——一个项目一个仓库，
/// 项目下的几块白板共用这一个目录。路径分隔符按那台机器的系统选（Windows 用 `\`，其余用 `/`），工作空间末尾的分隔符去掉；
/// 工作空间为空、`board` 不是 `<项目>/<白板>` 时返回 None
pub fn board_dir(workspace: &str, board: &str, os: &str) -> Option<String> {
    let (project, name) = board.split_once('/')?;
    if name.is_empty() {
        return None;
    }
    project_dir(workspace, project, os)
}

/// 工作空间里的一个目录 `<工作空间>/<目录名>`：分隔符按那台机器的系统选，工作空间末尾的分隔符去掉；工作空间、目录名为空时返回 None
pub fn project_dir(workspace: &str, dir: &str, os: &str) -> Option<String> {
    let base = workspace.trim().trim_end_matches(['/', '\\']);
    if base.is_empty() || dir.is_empty() {
        return None;
    }
    let sep = if os.to_lowercase().contains("windows") { "\\" } else { "/" };
    Some(format!("{base}{sep}{dir}"))
}

/// 派生一块白板的目录：`--dir` 覆盖了的用覆盖的路径（`fixed`；先看只管这一块白板的旧写法，再看它所在的项目），
/// 否则由工作空间派生成 `<工作空间>/<项目目录>`——项目设置里填了「项目目录」用它，没填用项目标识（#488）。
/// 工作空间为空、`board` 不是 `<项目>/<白板>` 时返回 None
pub fn derive_dir(ws: &HostWorkspace, board: &str, os: &str) -> Option<HostDir> {
    let project = board.split_once('/').map(|(p, _)| p);
    if let Some(o) = ws.overrides.iter().find(|o| o.board == board).or_else(|| ws.overrides.iter().find(|o| Some(o.board.as_str()) == project)) {
        return Some(HostDir { board: board.to_string(), path: o.path.clone(), fixed: true, ..Default::default() });
    }
    let (project, name) = board.split_once('/')?;
    if project.is_empty() || name.is_empty() {
        return None;
    }
    let dir = ws.projects.get(project).map(|d| d.trim()).filter(|d| !d.is_empty()).unwrap_or(project);
    let path = project_dir(&ws.path, dir, os)?;
    Some(HostDir { board: board.to_string(), path, ..Default::default() })
}

/// 登录 shell 自己打的启动、退出提示，不是命令的输出：派发的 agent 在 `$SHELL -l -i -c` 里跑，
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

/// 工作空间的设置路径：去掉首尾空白（`~` 不展开，和配置里写的一样）。没有路径、都是空白时为空
pub(super) fn clean_workspace(w: &HostWorkspace) -> HostWorkspace {
    // 项目目录（projects）不存：读的时候按项目表填
    let mut out = HostWorkspace { path: w.path.trim().to_string(), created: w.created, ..Default::default() };
    for o in &w.overrides {
        let (board, path) = (o.board.trim().to_string(), o.path.trim().to_string());
        if board.is_empty() || path.is_empty() || out.overrides.iter().any(|x| x.board == board) {
            continue;
        }
        out.overrides.push(HostDirOverride { board, path });
    }
    out
}

/// 只比较工作空间设置本身（路径、是否新建的、`--dir` 覆盖了哪几块白板），不看检查结果和克隆状态
pub(super) fn same_workspace(a: Option<&HostWorkspace>, b: Option<&HostWorkspace>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.path == b.path
                && a.created == b.created
                && a.overrides.len() == b.overrides.len()
                && a.overrides.iter().all(|x| b.overrides.iter().any(|y| y.board == x.board && y.path == x.path))
        }
        _ => false,
    }
}

/// 旧数据（只有白板与目录列表、没有工作空间）导出成工作空间：同一台机器上有几块白板就取共同的上两级
/// （`<工作空间>/<项目>/<白板>`）；不是同一个父目录的、或者项目、白板标识和路径上最后两段对不上的，
/// 留成 `--dir` 覆盖的目录（照样能接卡，界面上写明）；只有一块白板时取它的上一级当工作空间。
/// 没有目录时为空
pub(super) fn exported_workspace(dirs: &[HostDir]) -> Option<HostWorkspace> {
    let dirs: Vec<&HostDir> = dirs.iter().filter(|d| !d.board.trim().is_empty() && !d.path.trim().is_empty()).collect();
    if dirs.is_empty() {
        return None;
    }
    let split = |p: &str| -> Vec<String> {
        p.split(['/', '\\']).map(str::to_string).collect()
    };
    let parts: Vec<Vec<String>> = dirs.iter().map(|d| split(d.path.trim())).collect();
    let same_parent = parts.iter().all(|p| p.len() == parts[0].len() && p[..p.len() - 1] == parts[0][..parts[0].len() - 1]);
    let pairs: Vec<(String, String)> = dirs
        .iter()
        .map(|d| d.board.trim().split_once('/').map(|(p, b)| (p.to_string(), b.to_string())).unwrap_or_default())
        .collect();
    // 路径最后两段和 `<项目>/<白板>` 对得上：取上两级当工作空间
    let matches = |p: &[String], i: usize| p.len() >= 2 && p[p.len() - 2] == pairs[i].0 && p[p.len() - 1] == pairs[i].1;
    let mut overrides: Vec<HostDirOverride> = vec![];
    let mut base: Option<String> = None;
    if !dirs.iter().all(|d| path_like(d)) {
        // 旧版 `board host` 上报的 `board` 一列是白板标识、没有工作空间：没有路径可以导出
        return None;
    }
    let sep = if dirs[0].path.contains('\\') { "\\" } else { "/" };
    if same_parent && (0..dirs.len()).all(|i| matches(&parts[i], i)) {
        let common = parts[0].len() - 2;
        base = Some(parts[0][..common].join(sep));
    } else if dirs.len() == 1 && parts[0].len() >= 3 {
        /* 只有一块白板、路径最后两段和 `<项目>/<白板>` 对不上：仍取它的上一级当工作空间，这块白板走 `--dir` 覆盖 */
        base = Some(parts[0][..parts[0].len() - 2].join(sep));
    }
    let Some(base_path) = base.clone() else { return None };
    for d in dirs.iter() {
        let covered = board_dir(&base_path, d.board.trim(), if d.path.contains('\\') { "windows" } else { "" })
            .is_some_and(|p| p == d.path.trim());
        if !covered {
            overrides.push(HostDirOverride { board: d.board.trim().to_string(), path: d.path.trim().to_string() });
        }
    }
    base.map(|path| HostWorkspace { path, created: false, overrides, ..Default::default() })
}

/// 这台机器的工作空间：新数据读 `workspace` 列；还没有时（升级前的机器）用旧的「白板与目录」导出，
/// 导出不了时退回第一块非 `--dir` 白板的上一级；也没有时为空
pub(crate) fn workspace_of(stored: Option<&str>, dirs: &[HostDir]) -> Option<HostWorkspace> {
    // 存着的（包括清掉了路径、只剩 `--dir` 覆盖的）：以它为准，不从旧的目录里再导出
    if let Some(w) = stored.and_then(|j| serde_json::from_str::<HostWorkspace>(j).ok()) {
        return Some(w).filter(|w| !w.path.trim().is_empty() || !dirs.is_empty());
    }
    // 这台机器没配工作空间、存着的白板又不是 `<项目>/<白板>`（旧版 board host 报的白板标识）：也没有可以导出的
    if !dirs.is_empty() && dirs.iter().any(|d| !crate::valid_board(d.board.trim()) && !dirs.iter().any(|x| path_like(x))) {
        return None;
    }
    exported_workspace(dirs).or_else(|| {
        dirs.iter()
            .find(|d| !d.fixed && !d.path.trim().is_empty())
            .map(|d| HostWorkspace { path: d.path.trim().to_string(), ..Default::default() })
    })
}

/// 旧数据里像路径的：`/a/b`、`C:\a\b`、`~/a`、`../a`，或者带了好几段（`repo/board`）。上一版 `board host` 的
/// `--dir`、`[host] dirs` 里 `board` 一列写的是白板标识，这里只认明显是路径的
pub(super) fn path_like(d: &HostDir) -> bool {
    let p = d.path.trim();
    p.starts_with('/') || p.starts_with('~') || p.starts_with('.') || p.contains('\\') || p.contains(':') || p.split('/').count() >= 2
}

/// 这台机器上这块白板当前的目录：机器有工作空间时由它派生（服务端不再因为「没有这块白板的目录」拒绝派发）
pub fn host_board_dir(ws: Option<&HostWorkspace>, board: &str, os: &str) -> Option<HostDir> {
    ws.and_then(|w| derive_dir(w, board, os))
}

/// 这台机器上当每块白板的目录：工作空间里摆着 `[host] dirs`（升级前逐块配的）里的白板、`--dir` 覆盖的白板，
/// 加上 board host 上报过的目录（盘上确实有、但主人删了 `[host] dirs` 的那些也留着，免得检查结果没了）。
/// 每块白板的路径由工作空间派生，检查结果、克隆状态从上报的目录里按白板取
pub(super) fn derived_dirs(stored: &[HostDir], ws: Option<&HostWorkspace>, os: &str) -> Vec<HostDir> {
    let Some(ws) = ws.filter(|w| !w.path.trim().is_empty() || !w.overrides.is_empty()) else { return vec![] };
    let mut out: Vec<HostDir> = vec![];
    // 存着的目录（升级前逐块配的，或者 board host 上报过的）；旧版 board host 上报的 `board` 一列是白板标识、
    // 派生不出 `<项目>/<白板>` 的那几块单独列在后面
    for old in stored.iter() {
        let board = old.board.trim();
        // 旧版 board host 只报白板标识、这台机器又没有工作空间：按老办法用存着的目录
        let legacy = ws.path.trim().is_empty() && !crate::valid_board(board);
        if board.is_empty() || (!legacy && out.iter().any(|d| d.board == board)) {
            continue;
        }
        if legacy {
            out.push(old.clone());
            continue;
        }
        match derive_dir(ws, board, os) {
            Some(mut d) => {
                let same = old.path == d.path;
                d.status = old.status.clone().filter(|_| same);
                d.fetch_error = old.fetch_error.clone().filter(|_| same);
                d.origin = old.origin.clone().filter(|_| same);
                d.branch = old.branch.clone().filter(|_| same);
                d.checked_at = old.checked_at.filter(|_| same);
                d.clone = old.clone.clone();
                d.reported = old.reported;
                out.push(d);
            }
            None => out.push(old.clone()),
        }
    }
    // `--dir` 覆盖的白板（还没在存着的里面）；覆盖整个项目的不是一块白板，由上面每块白板派生时用上
    for o in &ws.overrides {
        if crate::valid_board(&o.board) && !out.iter().any(|d| d.board == o.board) {
            if let Some(d) = derive_dir(ws, &o.board, os) {
                out.push(d);
            }
        }
    }
    out
}

/// 服务端设置不变时，把 board host 刚检查到的目录状态合进来；旧版 board host 没有状态，保留原值
pub(super) fn merge_dir_checks(settings: &mut [HostDir], checks: &[HostDir]) {
    for s in settings {
        let Some(r) = checks.iter().find(|r| r.board == s.board && r.path == s.path) else { continue };
        s.reported = true;
        if r.checked_at.is_some() || r.status.is_some() {
            s.status = r.status.clone();
            s.fetch_error = r.fetch_error.as_ref().map(|e| e.chars().take(300).collect());
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

pub(super) fn op_in() -> String {
    "in".into()
}

/// 领取规则的条件
pub const RULE_OPS: [&str; 6] = ["in", "all", "not_in", "equal", "only", "none"];

/// 不带标签列表的条件：「没有标签」只看卡片有没有标签
pub(super) fn op_needs_labels(op: &str) -> bool {
    op != "none"
}

impl ClaimRule {
    /// 旧数据的单个标签换成 `in [标签]`；标签去掉空白、按名称（不区分大小写）去重
    pub(crate) fn upgraded(mut self) -> ClaimRule {
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
    pub(crate) fn stored(&self) -> serde_json::Value {
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
    /// 工作空间：每台设备一个目录，每块白板的目录由它派生。还没配时为空
    #[serde(default)]
    pub workspace: Option<HostWorkspace>,
    /// 这台机器上每块白板的目录（从工作空间派生，加上 board host 上报的检查结果）。
    /// 还没配工作空间、也没有 `--dir` 覆盖时为空
    pub dirs: Vec<HostDir>,
    /// 服务端工作空间设置的版本
    #[serde(default)]
    pub dirs_rev: i64,
    /// 最近一次改工作空间的时间、是谁改的
    #[serde(default)]
    pub dirs_changed_at: Option<i64>,
    #[serde(default)]
    pub dirs_changed_by: Option<String>,
    /// 最多几个会话：同一时间最多开几个 agent 进程（1–8）
    pub max: i64,
    /// 定时领取启动的会话一个最多做几张（1–8，默认 4）；旧版服务端没有这一项
    #[serde(default = "default_per_session")]
    pub per_session: i64,
    /// 单次最长运行（分钟，0 = 不限，默认 180）：所有机器一个值（`sched` 表），随机器发给 board host；旧版服务端没有这一项
    #[serde(default = "default_max_run")]
    pub max_run: i64,
    /// 多久没有输出算卡住（分钟，0 = 不限，默认 20）：同上
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
    /// 内置 agent 自动更新（默认关）：有新版本且没有在跑的派发时按实际安装方式更新
    #[serde(default)]
    pub agent_auto: bool,
    /// 默认 agent：设置过且找到了就是它，否则是本机 agent 里第一个找到的；都没找到时为空
    pub agent: Option<String>,
    /// 设置的默认 agent 在这台机器上找不到时是它（例如从别的机器复制来的）：不生效，用的是 `agent`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_set: Option<String>,
    /// 旧的定时领取开关和 cron 表达式：现在一直是关、空的（换成了 `auto`、`windows`），留着给旧版 board host、旧界面读
    pub cron_on: bool,
    pub cron: Vec<String>,
    /// 现在在自动接单：开了自动接单、没暂停、有领取规则、现在在接单时段里。board host 看到它就在有空位时来领卡
    /// （字段名沿用以前的「正在进行一轮领取」，旧版 board host 照样认）
    pub round: bool,
    /// 参与自动接单（PRD §9.1「自动领取」）
    #[serde(default)]
    pub auto: bool,
    /// 接单时段：一天里的几段 `[起, 止)`，单位分钟，每天一样，按服务端的时区；一段都没有 = 不接单
    #[serde(default = "crate::sched::all_day")]
    pub windows: Vec<crate::sched::Window>,
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
    /// skills 用服务器的（库里的都写进这台机器各个 agent 的 skill 目录）；否则不改（只装 board-skills）
    #[serde(default)]
    pub sync_skills: bool,
    /// MCP 用服务器的；否则不改
    #[serde(default)]
    pub sync_mcp: bool,
    /// Git 凭据用服务器的（写进这台机器的用户级 git 配置，按主机全局生效）；否则不改。旧版服务端没有这一项
    #[serde(default)]
    pub sync_git: bool,
    /// 同步配置：等 board host 做的请求、最近一次的结果
    #[serde(default)]
    pub sync: ConfigSync,
    /// 从这台机器导入 skills：本机有哪些 skill、等 board host 做的请求、最近一次的结果；旧版服务端没有这一项
    #[serde(default)]
    pub skill_import: SkillImport,
    /// 资源（CPU、内存、GPU、磁盘）：最近一次采样和最近几次的使用率；旧版 board host 不报，没有
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub res: Option<crate::hostres::HostRes>,
}

pub(super) fn default_per_session() -> i64 {
    4
}

pub(super) fn default_max_run() -> i64 {
    MAX_RUN_DEFAULT
}

pub(super) fn default_idle_run() -> i64 {
    IDLE_RUN_DEFAULT
}
