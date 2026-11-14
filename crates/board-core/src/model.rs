//! 数据模型：工作流、身份、快照（界面与 CLI 共用）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 心跳间隔：agent 会话 check 之后，skill 里的脚本每 10 秒发送一次 `board beat`，同时维持名字在线和卡片的持有（PRD §3、§8 规则 7）
pub const BEAT_MS: i64 = 10 * 1000;
/// 连续这么多次没收到心跳：名字离线；卡片（从领取或最近一次心跳算起）释放（PRD §3、§8 规则 7）
pub const BEAT_MISSES: i64 = 5;

/// 项目颜色色板
pub const PROJECT_COLORS: [&str; 8] =
    ["#FF6A33", "#4DB5FF", "#3DDC97", "#B98CFF", "#FFB020", "#FF5C8A", "#2DD4BF", "#94A3B8"];
/// 标签颜色色板
pub const LABEL_COLORS: [&str; 9] =
    ["#F43F5E", "#F97316", "#EAB308", "#22C55E", "#14B8A6", "#3B82F6", "#8B5CF6", "#EC4899", "#64748B"];

/// 状态颜色色板：前 4 个是主题色（随浅色 / 深色主题变化），其余是固定颜色
pub const STATUS_COLORS: [&str; 10] =
    ["todo", "doing", "review", "done", "#A855F7", "#EC4899", "#14B8A6", "#F59E0B", "#EF4444", "#84CC16"];

/// 「未整理」：每块白板固定都有，不属于工作流
pub const INBOX: &str = "inbox";
pub const INBOX_NAME: &str = "未整理";

/// 状态块在白板上的位置：12 栏网格里的起始栏 x、宽度 w，以及决定上下先后的 y
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub x: i64,
    pub w: i64,
    pub y: f64,
}

/// 状态块里卡片的形态（PRD §7）：只影响界面显示
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardView {
    /// 卡片：编号、标题、标签、持有者和计时
    #[default]
    Card,
    /// 单行：编号和标题占一行
    Line,
    /// 磁贴：只显示编号，横向排列、自动换行
    Tile,
}

impl CardView {
    pub fn as_str(self) -> &'static str {
        match self {
            CardView::Card => "card",
            CardView::Line => "line",
            CardView::Tile => "tile",
        }
    }

    pub fn parse(s: &str) -> Option<CardView> {
        match s {
            "card" => Some(CardView::Card),
            "line" => Some(CardView::Line),
            "tile" => Some(CardView::Tile),
            _ => None,
        }
    }

    /// 没有设置过时：终点用磁贴，其他用卡片
    pub fn default_for(fin: bool) -> CardView {
        if fin { CardView::Tile } else { CardView::Card }
    }
}

/// 白板设置「保留顺序」开着时，状态块里卡片的排序依据（PRD §10.3）：只影响界面显示，不改优先级和领取顺序
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderBy {
    /// 移入这个状态的时间
    #[default]
    Moved,
    /// 编号
    Id,
    /// 最近更新
    Updated,
    /// 标签（按项目里的标签顺序，没有标签的排最后）；不分方向
    Labels,
    /// 累计耗时
    Work,
}

impl OrderBy {
    pub fn as_str(self) -> &'static str {
        match self {
            OrderBy::Moved => "moved",
            OrderBy::Id => "id",
            OrderBy::Updated => "updated",
            OrderBy::Labels => "labels",
            OrderBy::Work => "work",
        }
    }

    pub fn parse(s: &str) -> Option<OrderBy> {
        match s {
            "moved" => Some(OrderBy::Moved),
            "id" => Some(OrderBy::Id),
            "updated" => Some(OrderBy::Updated),
            "labels" => Some(OrderBy::Labels),
            "work" => Some(OrderBy::Work),
            _ => None,
        }
    }
}

/// 一个状态的排序依据：按什么排、是否倒序。默认按移入时间，新移入的在最后
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Order {
    pub by: OrderBy,
    /// 倒序（按标签时不用）
    #[serde(default)]
    pub desc: bool,
}

/// 工作流里的一个状态（PRD §7）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub key: String,
    pub name: String,
    pub color: String,
    /// 可领取：board claim 从这里领取（至少一个）。排在最前面的一个是「首个可领取状态」，新建卡片放在这里
    pub claim: bool,
    /// 锁定：只能由 agent 领取进入；被持有期间只有持有者能编辑和移动
    pub lock: bool,
    /// 终点：只有人能把卡片移入或移出
    #[serde(rename = "final")]
    pub fin: bool,
    /// 错误节点：agent 异常退出、做不下去时沿连线移到这里；只有人能移出。不和可领取、锁定、终点同时打开
    /// （不可领取，配了定时领取也不会被下一轮重新领走）
    #[serde(default, rename = "error")]
    pub err: bool,
    /// 卡片形态；没有设置过时按 CardView::default_for 取
    #[serde(default)]
    pub view: CardView,
    /// 白板设置「保留顺序」开着时这块按什么排；旧服务端没有这个字段（按移入时间）
    #[serde(default)]
    pub order: Order,
    /// 状态块的位置；没有时界面用默认布局
    pub layout: Option<Tile>,
    /// 连线：从这里出发可以去往的状态（按工作流顺序）。agent 只能沿连线移动卡片（PRD §8 规则 12）
    #[serde(default)]
    pub to: Vec<String>,
}

/// 新建白板时的默认工作流：(标识, 名称, 颜色, 可领取, 锁定, 终点)
pub const DEFAULT_FLOW: [(&str, &str, &str, bool, bool, bool); 6] = [
    ("todo", "待办", "todo", true, false, false),
    ("doing", "进行中", "doing", false, true, false),
    ("review", "待审核", "review", false, false, false),
    ("done", "已完成", "done", false, false, true),
    ("rework", "返工", "#A855F7", true, false, false),
    ("check", "需人工校验", "#F59E0B", false, false, false),
];

/// 默认工作流的连线
pub const DEFAULT_EDGES: [(&str, &str); 7] = [
    ("todo", "doing"),
    ("doing", "review"),
    ("doing", "check"),
    ("review", "done"),
    ("review", "rework"),
    ("rework", "doing"),
    ("check", "todo"),
];

/// 一块白板的工作流（按顺序）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Flow(pub Vec<Status>);

impl Flow {
    pub fn get(&self, key: &str) -> Option<&Status> {
        self.0.iter().find(|s| s.key == key)
    }
    /// 首个可领取状态：新建卡片、整理、删除状态时卡片都放进这里（数据保证至少一个可领取状态；万一没有，退回第一个状态）
    pub fn home(&self) -> &Status {
        self.0.iter().find(|s| s.claim).unwrap_or(&self.0[0])
    }
    /// 是否可领取
    pub fn is_claim(&self, key: &str) -> bool {
        self.get(key).is_some_and(|s| s.claim)
    }
    /// 有没有连线 a → b
    pub fn has_edge(&self, a: &str, b: &str) -> bool {
        self.get(a).is_some_and(|s| s.to.iter().any(|t| t == b))
    }
    /// a 能去的状态名称，用「、」连接（错误信息里用）
    pub fn outs(&self, a: &str) -> String {
        let v: Vec<String> = self.get(a).map(|s| s.to.iter().map(|t| format!("「{}」", self.name(t))).collect()).unwrap_or_default();
        if v.is_empty() { "（没有连线）".into() } else { v.join("") }
    }
    pub fn is_lock(&self, key: &str) -> bool {
        self.get(key).is_some_and(|s| s.lock)
    }
    pub fn is_final(&self, key: &str) -> bool {
        self.get(key).is_some_and(|s| s.fin)
    }
    /// 状态的中文名；「未整理」也在这里
    pub fn name<'a>(&'a self, key: &'a str) -> &'a str {
        if key == INBOX {
            return INBOX_NAME;
        }
        self.get(key).map(|s| s.name.as_str()).unwrap_or(key)
    }
    /// 从可领取状态 from 领取后进入的锁定状态：from 连向的锁定状态里按顺序排在最前面的（PRD §8 规则 1）
    pub fn claim_target(&self, from: &str) -> Option<&Status> {
        self.0.iter().find(|s| s.lock && self.has_edge(from, &s.key))
    }
    /// 释放后卡片回到哪里：领取它时所在的可领取状态；那个状态已删除或不再可领取时，回首个可领取状态（PRD §8 规则 7）
    pub fn back_to(&self, from: Option<&str>) -> &Status {
        from.and_then(|k| self.get(k)).filter(|s| s.claim).unwrap_or_else(|| self.home())
    }
    /// 用标识或名称（不区分大小写）找状态；`inbox` / 「未整理」返回 INBOX
    pub fn resolve(&self, raw: &str) -> Option<String> {
        let q = raw.trim();
        let lower = q.to_lowercase();
        if lower == INBOX || q == INBOX_NAME {
            return Some(INBOX.to_string());
        }
        self.0
            .iter()
            .find(|s| s.key == q)
            .or_else(|| self.0.iter().find(|s| s.name.to_lowercase() == lower))
            .map(|s| s.key.clone())
    }
    /// 可选值说明（错误信息里用）
    pub fn choices(&self) -> String {
        let mut v: Vec<String> = self.0.iter().map(|s| format!("{}（{}）", s.name, s.key)).collect();
        v.push(format!("{INBOX_NAME}（{INBOX}）"));
        v.join("、")
    }
    /// 交审状态：锁定状态连出去、名称里带「审核」之类关键词的状态（与 board-skills「认出角色」的规则相同）
    pub fn is_review(&self, key: &str) -> bool {
        self.role_of(key, &REVIEW_WORDS)
    }
    /// 需要人处理的状态：锁定状态连出去、名称里带「人工」之类关键词的状态
    pub fn is_human(&self, key: &str) -> bool {
        self.role_of(key, &HUMAN_WORDS)
    }
    /// 错误节点：状态设置里打开了「错误节点」的状态（看属性，不看名称）。和「需要人处理」分开认：
    /// 错误节点表示 agent 异常退出、做不下去，不是等人校验
    pub fn is_error(&self, key: &str) -> bool {
        self.get(key).is_some_and(|s| s.err)
    }
    /// 从锁定状态 from 沿连线能去的、需要人处理的状态（按工作流顺序第一个）。可领取状态和终点不算：
    /// 放回可领取状态人看不到出了问题，终点只归人管。「错误」节点另算（[`Self::error_after`]），不算在这里
    pub fn human_after(&self, from: &str) -> Option<&Status> {
        self.after(from, &HUMAN_WORDS)
    }
    /// 从锁定状态 from 沿连线能去的「错误」节点（按工作流顺序第一个）。agent 异常退出、做不下去时先去这里：
    /// 它不在可领取状态里，配了定时领取也不会被下一轮重新领走。
    /// 没有错误节点时由调用方退回 [`Self::human_after`]
    pub fn error_after(&self, from: &str) -> Option<&Status> {
        let s = self.get(from)?;
        s.to.iter().filter_map(|k| self.get(k)).find(|t| t.err)
    }
    /// 从锁定状态 from 连出去、名称里带 words 的状态里按工作流顺序排在最前面的一个（错误节点不算）
    fn after(&self, from: &str, words: &[&str]) -> Option<&Status> {
        let s = self.get(from)?;
        s.to.iter().filter_map(|k| self.get(k)).find(|t| !t.claim && !t.fin && !t.lock && !t.err && words_in(t, words))
    }
    fn role_of(&self, key: &str, words: &[&str]) -> bool {
        let Some(s) = self.get(key) else { return false };
        !s.claim && !s.lock && !s.fin && !s.err && words_in(s, words) && self.0.iter().any(|l| l.lock && l.to.iter().any(|t| t == key))
    }
}

/// 交审状态名称里的关键词（board-skills「认出角色」：等人审核的状态）
const REVIEW_WORDS: [&str; 4] = ["审核", "审查", "评审", "review"];
/// 需要人处理的状态名称里的关键词（board-skills「认出角色」：等人处理的状态）
const HUMAN_WORDS: [&str; 7] = ["人工", "人处理", "人确认", "需人", "check", "manual", "human"];

fn words_in(s: &Status, words: &[&str]) -> bool {
    let (name, key) = (s.name.to_lowercase(), s.key.to_lowercase());
    words.iter().any(|w| name.contains(w) || key.contains(w))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    Human,
    Agent,
    System,
}

impl ActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ActorKind::Human => "human",
            ActorKind::Agent => "agent",
            ActorKind::System => "system",
        }
    }
    pub fn parse(s: &str) -> ActorKind {
        match s {
            "agent" => ActorKind::Agent,
            "system" => ActorKind::System,
            _ => ActorKind::Human,
        }
    }
}

/// agent 名字最长多少个字符（PRD §3）
pub const AGENT_NAME_MAX: usize = 48;

/// agent 名字的格式说明（`USAGE` 的原因里用）
pub const AGENT_NAME_SHAPE: &str =
    "格式是 <运行环境>/<模型>，需要时加 -<后缀>；两段都只能用小写字母、数字、. 和 -，并以字母或数字开头，只有一个 /，总长不超过 48 个字符，例如 claude/opus-5.5、dsh/deepseek-v4.1-flash、claude/opus-5.5-login";

/// agent 名字的格式（PRD §3）：`<运行环境>/<模型>`，需要时加 `-<后缀>`，例如 `claude/opus-5.5`、
/// `claude/opus-5.5-login`。不合格式时返回原因（含格式说明）
pub fn check_agent_name(name: &str) -> Result<(), String> {
    let ok_part = |s: &str| {
        s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
    };
    if name.chars().count() > AGENT_NAME_MAX {
        return Err(format!("名字「{name}」超过 {AGENT_NAME_MAX} 个字符：{AGENT_NAME_SHAPE}"));
    }
    match name.split_once('/') {
        Some((env, model)) if ok_part(env) && ok_part(model) => Ok(()),
        _ => Err(format!("名字「{name}」格式不对：{AGENT_NAME_SHAPE}")),
    }
}

/// 谁在操作：界面上是人，CLI 是 agent，超时释放、心跳中断、定时派发是系统（文本里显示成 board）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub name: String,
}

impl Actor {
    pub fn human() -> Self {
        Actor { kind: ActorKind::Human, name: String::new() }
    }
    pub fn agent(name: impl Into<String>) -> Self {
        Actor { kind: ActorKind::Agent, name: name.into() }
    }
    pub fn system() -> Self {
        Actor { kind: ActorKind::System, name: String::new() }
    }
    pub fn is_agent(&self) -> bool {
        self.kind == ActorKind::Agent
    }
    pub fn is_human(&self) -> bool {
        self.kind == ActorKind::Human
    }
    /// 在文本（动态、CLI 输出）里怎么称呼。系统身份就是 board 本身，界面上用同一个词（web 的 SYSTEM_NAME）
    pub fn display(&self) -> &str {
        match self.kind {
            ActorKind::Human => "人",
            // 系统身份带名字的（「定时建卡」）显示名字，其余是 board 本身
            ActorKind::System if !self.name.is_empty() => &self.name,
            ActorKind::System => "board",
            ActorKind::Agent => &self.name,
        }
    }
}

/// 变更来自哪里：界面 / CLI / 系统（超时扫描）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Ui,
    Cli,
    System,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Ui => "ui",
            Via::Cli => "cli",
            Via::System => "system",
        }
    }
    pub fn parse(s: &str) -> Via {
        match s {
            "cli" => Via::Cli,
            "system" => Via::System,
            _ => Via::Ui,
        }
    }
}

/// 一次写操作的来源。client 是界面窗口的标识，用于让窗口忽略自己触发的高亮
#[derive(Debug, Clone)]
pub struct Origin {
    pub via: Via,
    pub client: Option<String>,
}

impl Origin {
    pub fn ui(client: Option<String>) -> Self {
        Origin { via: Via::Ui, client }
    }
    pub fn cli() -> Self {
        Origin { via: Via::Cli, client: None }
    }
    pub fn system() -> Self {
        Origin { via: Via::System, client: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pos {
    pub x: f64,
    pub y: f64,
}

/// 已定位的白板
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardRef {
    pub id: i64,
    pub project: String,
    pub key: String,
}

impl BoardRef {
    pub fn path(&self) -> String {
        format!("{}/{}", self.project, self.key)
    }
}

/* ------------------------------- 快照 ------------------------------- */

/// 各状态的卡片数（键是状态标识，含 `inbox`）
pub type Counts = BTreeMap<String, i64>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: i64,
    pub name: String,
    pub color: String,
    /// 项目内使用这个标签的卡片数
    pub uses: i64,
    /// 仅人可打：只有人能在界面上给卡片加、去；agent 加、去时被拒绝（USAGE）。旧数据默认 false（agent 可打）
    #[serde(rename = "humanOnly", default)]
    pub human_only: bool,
    /// 优先：不带 ID 领取（和定时领取）时，带这个标签的卡片先被领取。旧数据默认 false
    #[serde(default)]
    pub priority: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardInfo {
    pub key: String,
    pub path: String,
    pub name: String,
    pub desc: String,
    /// 白板说明（Markdown，可以为空）
    #[serde(default)]
    pub readme: String,
    /// 环境检查：机器要先在这块白板的目录里跑通它，才接这块白板的卡（PRD §9.1「环境检查」）。旧服务端没有这一项
    #[serde(default)]
    pub ready_check: ReadyCheck,
    /// 代码仓库地址（git 远端，例如 `git@github.com:o/r.git`）；空表示没设。旧服务端没有这一项。
    /// 提交号、`gh#123` 变链接和给新机器克隆都用它（PRD §4）
    #[serde(default)]
    pub repo: String,
    /// 集成分支；空表示按原来的规则（AGENTS.md / CLAUDE.md，没写就用远端默认分支）。旧服务端没有这一项
    #[serde(default)]
    pub branch: String,
    pub pos: Pos,
    pub created_at: i64,
    pub updated_at: i64,
    pub counts: Counts,
    /// 工作流（不含布局）
    pub statuses: Vec<Status>,
    /// 正在这块白板上工作的 agent（按工作流和状态内顺序）
    pub holders: Vec<String>,
    /// 定时建卡（PRD §4「白板设置」）。旧服务端没有这一项
    #[serde(default)]
    pub card_crons: Vec<crate::card_crons::CardCron>,
}

/// 白板的环境检查命令（PRD §9.1「环境检查」）：一行命令，在机器上这块白板的目录里执行，退出码 0 算通过。
/// 都为空时不检查；Windows 写了 `win` 就用它（cmd 里执行），没写用 `cmd`
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyCheck {
    /// 所有系统（macOS、Linux 在登录 shell 里执行，Windows 没单独写时在 cmd 里执行）
    #[serde(default)]
    pub cmd: String,
    /// 只给 Windows（cmd 里执行）；空表示和 `cmd` 一样
    #[serde(default)]
    pub win: String,
}

/// 环境检查命令最长多少个字符
pub const READY_CHECK_MAX: usize = 500;

/// 仓库地址最长多少个字符
pub const REPO_MAX: usize = 500;
/// 集成分支名最长多少个字符（git 的分支名实际上限远大于这个数）
pub const BRANCH_MAX: usize = 200;

impl ReadyCheck {
    /// 这台机器（按它上报的系统）要跑的命令；不检查时为 None
    pub fn for_os(&self, os: &str) -> Option<String> {
        let win = os.to_lowercase().contains("windows");
        let pick = if win && !self.win.trim().is_empty() { &self.win } else { &self.cmd };
        let t = pick.trim();
        (!t.is_empty()).then(|| t.to_string())
    }
    /// 两条命令都空：这块白板不检查。服务端和 CLI 都只用 [`Self::for_os`]（它已经对空命令返回 None），
    /// 界面上用前端自己的判断，所以只在测试里用
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.cmd.trim().is_empty() && self.win.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub key: String,
    pub name: String,
    pub desc: String,
    pub color: String,
    pub pos: Pos,
    pub created_at: i64,
    pub labels: Vec<Label>,
    /// 隐藏的标签：名称匹配任一条正则（不区分大小写）的标签不出现在侧栏、提示词菜单、标签选择器里（default/main#324）。
    /// 旧库、新建的项目默认 `^v\d`（版本标签）；旧服务端没有这个字段，界面按默认处理
    #[serde(default = "default_hidden_labels")]
    pub hidden_labels: Vec<String>,
    pub boards: Vec<BoardInfo>,
}

/// 项目默认隐藏的标签：版本标签（v0.1.24 这类，里程碑按它认版本）
pub fn default_hidden_labels() -> Vec<String> {
    vec![r"^v\d".to_string()]
}

/// 隐藏规则最多几条、每条最长几个字符
pub const HIDDEN_LABELS_MAX: usize = 20;
pub const HIDDEN_LABEL_LEN: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub rev: i64,
    pub now: i64,
    pub projects: Vec<ProjectInfo>,
    /// 在线的 agent（所有白板，按名字排序）
    #[serde(default)]
    pub agents: Vec<AgentInfo>,
    /// 机器（board host，按名字排序）
    #[serde(default)]
    pub hosts: Vec<crate::hosts::HostInfo>,
    /// 排队和正在跑的派发（按创建先后）
    #[serde(default)]
    pub dispatches: Vec<crate::hosts::Dispatch>,
}

/// 一个在线的 agent（PRD §3）：名字最近一次心跳在 50 秒以内
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: String,
    /// 首次 check 的时间
    pub checked_at: i64,
    /// 最近一次心跳（check 也算一次）
    pub beat_at: i64,
    /// 它持有的卡片（每块白板最多一张；可能没有）
    pub cards: Vec<HeldCard>,
    /// 会话标识：只在新版 CLI check 成功时返回一次，之后 agent 级心跳和 final 都要带上
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// 名字由服务端按机器设置生成时（`board check --agent <标识>`）才有：名字里的模型和这台机器、这个 agent
    /// 现在的设置一致。对不上（机器改名了、模型不是这个）时，之后的心跳会按设置算出的名字换过去
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modeled: Option<bool>,
}

/// agent 持有的一张卡片：白板路径加上卡片本身
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeldCard {
    pub board: String,
    #[serde(flatten)]
    pub card: Card,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: i64,
    pub title: String,
    pub body: String,
    /// 状态标识；「未整理」是 `inbox`
    pub column: String,
    pub holder: Option<String>,
    pub claimed_at: Option<i64>,
    /// 最近一次心跳（发了 final 之后是 final 的时间）；还没开始心跳时为空
    pub beat_at: Option<i64>,
    /// 这一次的开始时间（第一次心跳）；没有进行中的一次时为空
    pub run_started_at: Option<i64>,
    /// 已经结束的各次耗时之和（毫秒），不含进行中的这一次
    pub work_ms: i64,
    /// 做过几次（含进行中的这一次）
    pub runs: i64,
    /// 其中几次是异常退出
    pub runs_lost: i64,
    /// 收到的心跳总次数（各次之和，含 final）
    #[serde(default)]
    pub beats: i64,
    /// 最近一次领取结束的时间（final、心跳中断、释放都算）；没有结束过的领取时为空。评论、编辑、移动不影响它
    #[serde(default)]
    pub finished_at: Option<i64>,
    /// 领取它时所在的可领取状态；释放后回到这里（PRD §8 规则 7）
    #[serde(default)]
    pub claimed_from: Option<String>,
    /// 标签 ID（属于白板所在的项目）
    pub labels: Vec<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// 移入现在这个状态的时间（新建、移动、领取、释放都算；同一状态里调整顺序不算）。旧服务端没有这个字段
    #[serde(default)]
    pub moved_at: i64,
    pub created_by: Actor,
    /// 评论数
    pub comments: i64,
    /// 人看过这一条的时间（PRD §10.5）：「未整理」里 agent 记下、还没被人看过的条目算「新」。
    /// 人展开过、在详情里打开过、编辑过、整理过、合并过都算看过。旧服务端没有这个字段
    #[serde(default)]
    pub seen_at: Option<i64>,
    /// 拆自：同一块白板上另一张卡的编号（`board new --from`）。原卡删除、永久删除后照样留着编号。旧服务端没有这个字段
    #[serde(default)]
    pub split_from: Option<i64>,
    /// 「稍后再做」（default/main#303）：人把「未整理」里的这一条先收起来的时间；NULL = 没收起。
    /// 收起的条目不在「未整理」的列表里显示，状态、位置、标签、描述、评论都不动；离开「未整理」时清掉。旧服务端没有这个字段
    #[serde(default)]
    pub later_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub id: i64,
    pub at: i64,
    pub actor: Actor,
    /// comment | system
    pub kind: String,
    pub action: String,
    pub text: String,
    /// 评论被作者修改过时，最后一次修改的时间
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardDetail {
    #[serde(flatten)]
    pub card: Card,
    pub activity: Vec<Activity>,
    /// 附件（PRD §5）
    #[serde(default)]
    pub attachments: Vec<crate::files::Attachment>,
    /// 拆出：「拆自」是这张的卡片（不含回收站里的），按编号。旧服务端没有这个字段
    #[serde(default, rename = "splitInto")]
    pub split_into: Vec<i64>,
}

/// 回收站里的张数上限：超出时按删除时间先清掉最早的（PRD §8 规则 10）
pub const TRASH_MAX: i64 = 1000;

/// 回收站里的一张卡片（PRD §8 规则 10）：字段与卡片相同，另外记下删除的时间、谁删的、删除前在哪个状态、排在第几位
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashCard {
    #[serde(flatten)]
    pub card: Card,
    /// 删除的时间
    pub deleted_at: i64,
    /// 删除的人
    pub deleted_by: Actor,
    /// 删除前所在的状态（工作流状态或 `inbox`）
    pub col: String,
    /// 删除前在状态里的位置；恢复时排回原来的位置
    pub pos: i64,
    /// 删除前领取它的那个可领取状态（被删时还被持有的才有）：恢复后回到这里，而不是首个可领取状态。
    /// 卡片本身的 `claimedFrom` 在删除时已被清空，这里另起名字，免得和它的 JSON 字段撞上
    #[serde(default)]
    pub held_from: Option<String>,
}

/// 全局事件流里的一条（界面的「最近动态」与跨窗口高亮用）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: i64,
    pub board: String,
    pub card_id: i64,
    pub title: String,
    pub at: i64,
    pub actor: Actor,
    pub op: String,
    pub text: String,
    pub via: Via,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

/// 每个状态里的卡片 ID（按顺序；键是状态标识，含 `inbox`）
pub type Cols = BTreeMap<String, Vec<i64>>;

/// 一块白板的完整状态（不含每张卡片的动态，详情另取）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSnapshot {
    pub rev: i64,
    pub now: i64,
    pub path: String,
    /// 工作流（按顺序，含布局）
    pub statuses: Vec<Status>,
    pub cols: Cols,
    pub cards: BTreeMap<i64, Card>,
    /// 终点状态默认只带最近移入的 FINAL_RECENT 张（cols、cards 里都没有更早的）：状态 → 省略的张数。
    /// 取全量（full）或没有省略时为空；旧服务端没有这个字段
    #[serde(default)]
    pub omitted: BTreeMap<String, usize>,
    /// 最近的事件（新的在后）
    pub log: Vec<Event>,
    /// 白板设置里「保存布局」存下的布局（只含现在还在的状态）；没保存过为空。旧服务端没有这个字段
    #[serde(default)]
    pub saved_layout: Option<BTreeMap<String, Tile>>,
    /// 保留顺序：开着时界面上每个状态块按自己的排序依据（Status.order）显示，拖动只换状态、不改顺序；
    /// 关着时拖到哪就在哪，顺序按拖动的结果保存。旧服务端没有这个字段（当作关着）
    #[serde(default)]
    pub keep_sorted: bool,
    /// 回收站里的张数（PRD §8 规则 10、§10.3 侧栏「回收站 · N」）。旧服务端没有这个字段
    #[serde(default)]
    pub trash: i64,
    /// 这块白板的代码仓库地址（空表示没设）：评论里的提交号、`gh#123` 按它变成链接（PRD §4）。旧服务端没有这个字段
    #[serde(default)]
    pub repo: String,
    /// 这块白板的集成分支（空表示按仓库的 AGENTS.md / CLAUDE.md，没写就用远端默认分支）。旧服务端没有这个字段
    #[serde(default)]
    pub branch: String,
}

/// 白板快照里每个终点状态默认带的张数（最近移入的，也就是最下面的几张）；更早的按需取全量
pub const FINAL_RECENT: usize = 50;
