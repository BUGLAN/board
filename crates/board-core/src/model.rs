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
    /// 卡片形态；没有设置过时按 CardView::default_for 取
    #[serde(default)]
    pub view: CardView,
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
    /// 从锁定状态 from 沿连线能去的、需要人处理的状态（按工作流顺序第一个）。可领取状态和终点不算：
    /// 放回可领取状态人看不到出了问题，终点只归人管
    pub fn human_after(&self, from: &str) -> Option<&Status> {
        let s = self.get(from)?;
        s.to.iter().filter_map(|k| self.get(k)).find(|t| !t.claim && !t.fin && !t.lock && words_in(t, &HUMAN_WORDS))
    }
    fn role_of(&self, key: &str, words: &[&str]) -> bool {
        let Some(s) = self.get(key) else { return false };
        !s.claim && !s.lock && !s.fin && words_in(s, words) && self.0.iter().any(|l| l.lock && l.to.iter().any(|t| t == key))
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

/// 谁在操作：界面上是人，CLI 是 agent，超时释放是系统
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
    /// 在文本（动态、CLI 输出）里怎么称呼
    pub fn display(&self) -> &str {
        match self.kind {
            ActorKind::Human => "人",
            ActorKind::System => "系统",
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardInfo {
    pub key: String,
    pub path: String,
    pub name: String,
    pub desc: String,
    pub pos: Pos,
    pub created_at: i64,
    pub updated_at: i64,
    pub counts: Counts,
    /// 工作流（不含布局）
    pub statuses: Vec<Status>,
    /// 正在这块白板上工作的 agent（按工作流和状态内顺序）
    pub holders: Vec<String>,
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
    pub boards: Vec<BoardInfo>,
}

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
    /// 领取它时所在的可领取状态；释放后回到这里（PRD §8 规则 7）
    #[serde(default)]
    pub claimed_from: Option<String>,
    /// 标签 ID（属于白板所在的项目）
    pub labels: Vec<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub created_by: Actor,
    /// 评论数
    pub comments: i64,
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
    /// 最近的事件（新的在后）
    pub log: Vec<Event>,
}
