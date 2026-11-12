/* 与 board-core 的 JSON 对应 */

/** 状态标识：工作流里的状态（todo、doing、s1…），或「未整理」inbox */
export type ColumnKey = string;

/** 状态块在 12 栏网格上的位置 */
export interface Tile {
  x: number;
  w: number;
  y: number;
}

/** 工作流里的一个状态（PRD §7） */
/** 状态块里卡片的形态：卡片 / 单行 / 磁贴（只显示编号，横向排列、自动换行） */
export type StatusView = 'card' | 'line' | 'tile';

/** 「保留顺序」的排序依据：移入时间 / 编号 / 最近更新 / 标签（项目里的标签顺序，不分方向）/ 累计耗时 */
export type OrderBy = 'moved' | 'id' | 'updated' | 'labels' | 'work';
export interface Order {
  by: OrderBy;
  /** 倒序；按标签时不用 */
  desc: boolean;
}

export interface Status {
  key: string;
  name: string;
  /** 主题色名（todo / doing / review / done）或 #RRGGBB */
  color: string;
  /** 可领取：board claim 从这里领（可以有多个，至少一个）。排在最前面的是首个可领取状态，新建卡片放在这里 */
  claim: boolean;
  /** 锁定：只能由 agent 领取进入；被持有期间只有持有者能动 */
  lock: boolean;
  /** 终点：只有人能移入移出 */
  final: boolean;
  /** 错误节点：agent 异常退出、做不下去时沿连线移到这里，只有人能移出；和可领取、锁定、终点互斥（旧服务端没有） */
  error?: boolean;
  /** 卡片形态；服务端没设置过时按规则给出（终点用磁贴，其他用卡片） */
  view?: StatusView;
  /** 白板设置「保留顺序」开着时这块按什么排（lib/sort.ts）；旧服务端没有（按移入时间，新移入的在最后） */
  order?: Order;
  layout: Tile | null;
  /** 连线：从这里出发可以去往的状态（按工作流顺序）。agent 只能沿连线移动卡片，人不受限制 */
  to: string[];
}

export type ActorKind = 'human' | 'agent' | 'system';
export interface Actor {
  kind: ActorKind;
  name: string;
}

export interface Pos {
  x: number;
  y: number;
}

export type Counts = Record<string, number>;

export interface Label {
  id: number;
  name: string;
  color: string;
  uses: number;
  /** 仅人可打：只有人能在界面上给卡片加、去；agent 加、去时被拒绝。旧服务端没有这个字段，按 agent 可打 */
  humanOnly?: boolean;
  /** 优先：不带 ID 领取（和定时领取）时，带这个标签的卡片先被领取 */
  priority?: boolean;
}

export interface BoardInfo {
  key: string;
  path: string;
  name: string;
  desc: string;
  /** 白板说明（Markdown，可以为空）：大盘白板概览里不显示，只在「编辑白板」里改 */
  readme?: string;
  /** 环境检查：机器先在这块白板的目录里跑通它，才接这块白板的卡；都为空时不检查。旧服务端没有这一项 */
  readyCheck?: ReadyCheck;
  /** 代码仓库地址（git 远端，例如 git@github.com:o/r.git）；空表示没设。旧服务端没有这一项 */
  repo?: string;
  /** 集成分支；空表示按仓库的 AGENTS.md / CLAUDE.md，没写就用远端默认分支。旧服务端没有这一项 */
  branch?: string;
  pos: Pos;
  createdAt: number;
  updatedAt: number;
  counts: Counts;
  statuses: Status[];
  holders: string[];
}

/** 白板的环境检查命令：一行，退出码 0 算通过。`win` 只给 Windows（cmd 里执行），空表示和 `cmd` 一样 */
export interface ReadyCheck {
  cmd: string;
  win: string;
}

export interface ProjectInfo {
  key: string;
  name: string;
  desc: string;
  color: string;
  pos: Pos;
  createdAt: number;
  labels: Label[];
  boards: BoardInfo[];
}

export interface Workspace {
  rev: number;
  now: number;
  projects: ProjectInfo[];
  /** 在线的 agent（所有白板，按名字排序）；旧服务端没有这个字段 */
  agents?: AgentInfo[];
  /** 机器（board host，按名字排序）；旧服务端没有这个字段 */
  hosts?: HostInfo[];
  /** 排队和正在跑的派发（按创建先后）；旧服务端没有这个字段 */
  dispatches?: Dispatch[];
}

/* ---------------- 任务下发（board host） ---------------- */

/** 本机 agent：board host 启动时按配置文件和内置模板找到的（没找到的也上报） */
export interface HostAgent {
  /** 标识：claude、dsh、pi … */
  id: string;
  /** 名称：Claude Code … */
  name: string;
  /** `<命令> --version` 的第一行（在实际用的启动环境里） */
  version: string;
  /** 在实际用的启动环境里找到了命令 */
  found: boolean;
  /** 它支持、本机有的启动环境，按默认选择的顺序，各自的检查结果；旧版 board host 没有 */
  envs?: EnvCheck[];
  /** 实际用的启动环境（机器设置里选的，没选时按顺序第一个找到的）；旧版 board host 没有 */
  env?: string | null;
  /** 在实际用的启动环境里找到了命令、却看不到 board-skills：该装到哪里。看得到、旧版 board host 没有 */
  no_skill?: string | null;
  /** 这个 agent 在这台机器上实际配的模型名：派发的 agent 名字（<agent 标识>-<机器名>/<模型名>）的第二段；旧版 board host 没有 */
  model?: string;
  /** 机器设置里能选的模型和思考强度（按模板）；旧版 board host 没有：置灰、要升级 */
  opts?: AgentOpts | null;
  /** 机器设置里选的模型、思考强度；没选是 agent 自己的默认（不传参数） */
  run_model?: string | null;
  run_effort?: string | null;
  /** 机器设置里选的启动环境（可能是从别的机器复制来、这台机器上没有的：那时 env 是实际用的） */
  run_env?: string | null;
  /** 共用配置里有这个标识的 agent 模板，但这台机器的配置文件里有同标识的 [agents.*]：用的是本机的 */
  local_override?: boolean;
}

/** 一个 agent 能在机器设置里选什么 */
export interface AgentOpts {
  /** 能选模型 */
  model: boolean;
  /** 「模型」下拉里列的常用模型（也可以手填） */
  models: string[];
  /** 支持的思考强度档位，从低到高；空表示不能选强度 */
  efforts: string[];
}

/** 一个 agent 在一个启动环境里的检查结果 */
export interface EnvCheck {
  env: string;
  found: boolean;
  version: string;
  /** 找到了命令、却看不到 board-skills：该装到哪里 */
  no_skill?: string | null;
}

/** 启动环境：board host 在哪种 shell 里启动 agent（直接启动、登录 shell、cmd、PowerShell、Git Bash、WSL …） */
export interface HostEnv {
  /** direct、login、cmd、powershell、pwsh、git-bash、wsl:<发行版> */
  id: string;
  /** 直接启动、登录 shell、Git Bash、WSL（Ubuntu）… */
  name: string;
  /** 程序路径；WSL 是发行版名；直接启动为空 */
  path: string;
}

export interface HostDir {
  /** 项目/白板 */
  board: string;
  /** 配置里写的路径（~ 不展开） */
  path: string;
  /** 命令行用 --dir 固定：界面只读 */
  fixed?: boolean;
  /** ok | missing | not_git；旧版 board host 没有 */
  status?: 'ok' | 'missing' | 'not_git' | null;
  /** 是 git 仓库时的 origin */
  origin?: string | null;
  /** 是 git 仓库时的当前分支 */
  branch?: string | null;
  /** board host 最近一次检查的时间 */
  checkedAt?: number | null;
  /** 目录不存在时的克隆状态 */
  clone?: HostClone | null;
  /** 服务端算好的：这块白板要这台机器跑的环境检查命令（按系统选好）；白板不检查时没有 */
  check?: string | null;
  /** board host 最近一次跑环境检查的结果；旧版 board host 没有 */
  ready?: DirReady | null;
}

/** 一台机器在一块白板的目录里跑环境检查的结果 */
export interface DirReady {
  /** 跑的是哪条命令：命令改了之后旧结果不算 */
  cmd: string;
  ok: boolean;
  code?: number | null;
  /** 输出的最后 20 行 */
  tail: string[];
  /** 跑完的时间 */
  at: number;
}

export interface HostClone {
  /** queued | running | done | failed */
  state: 'queued' | 'running' | 'done' | 'failed';
  url: string;
  message: string;
  updatedAt: number;
  by?: string;
}

/** 一台机器（运行 board host 的进程） */
export interface HostInfo {
  id: number;
  name: string;
  os: string;
  /** board 版本 */
  version: string;
  agents: HostAgent[];
  /** 启动环境（按发现的顺序）；旧版 board host 不上报，为空 */
  envs?: HostEnv[];
  dirs: HostDir[];
  /** 服务端目录设置的版本 */
  dirsRev?: number;
  /** 最近一次改目录设置的时间、是谁改的 */
  dirsChangedAt?: number | null;
  dirsChangedBy?: string | null;
  /** 同时跑几个（1–8） */
  max: number;
  /** 定时领取启动的会话一个最多做几张（1–8，默认 4）；旧版服务端没有 */
  perSession?: number;
  /** 单次最长运行（分钟，0 = 不限，默认 180）；旧版服务端没有 */
  maxRun?: number;
  /** 多久没有输出算卡住（分钟，0 = 不限，默认 20）；旧版服务端没有 */
  idleRun?: number;
  /** 定时领取的领取规则（有顺序）：带这个标签的卡片由这个 agent 做；没有一条规则时定时领取不领卡 */
  rules?: ClaimRule[];
  /** 兜底：不带任何规则标签的卡片由这个 agent 做；没设时不领这些卡片 */
  fallback?: string | null;
  fallbackActive?: boolean;
  /** 有规则、没设兜底时：可领取、没有匹配任何规则的卡片数 */
  unmatched?: number;
  paused: boolean;
  /** 默认 agent（设置过且找到了就是它，否则第一个找到的） */
  agent: string | null;
  /** 设置的默认 agent 在这台机器上找不到时是它（例如从别的机器复制来的）：不生效 */
  agentSet?: string | null;
  cronOn: boolean;
  /** 定时领取的时间点 HH:MM（机器本地时区） */
  cron: string[];
  /** 正在进行一轮领取 */
  round: boolean;
  online: boolean;
  firstAt: number;
  beatAt: number;
  offAt: number | null;
  /** 等 board host 处理的远程管理请求（重新检查、重启、升级）；旧版服务端没有 */
  ask?: HostAsk | null;
  /** 正在排空：暂停接单，等在跑的派发结束（都结束了就是「已排空」） */
  drain?: boolean;
  /** 最近一次「重新检查」做完的时间 */
  recheckedAt?: number | null;
  /** 最近的操作记录（新的在前）：谁、什么时间、做了什么 */
  ops?: HostOp[];
  /** 这台机器最近一次上报用的令牌；令牌已经吊销、还没上报过时为空 */
  token?: HostToken | null;
  /** 自检：等 board host 做的请求、最近一次的结果；旧版服务端没有 */
  selftest?: SelfTest;
  /** skills 共用库同步到这台机器各个 agent 目录的状态；旧版 board host 不同步，为 null / 没有 */
  skills?: SkillState[] | null;
  /** MCP 服务器同步到这台机器各个 agent 的状态；旧版 board host 不同步，为 null / 没有 */
  mcp?: McpState[] | null;
}

/** MCP 服务器的一个环境变量或请求头。列表接口里密钥的 value 是空的，tail 是末尾 4 位；保存时密钥留空表示保留原来的值 */
export interface McpVar {
  key: string;
  value?: string | null;
  secret?: boolean;
  tail?: string | null;
}

/** 共用配置里的一个 MCP 服务器（PRD §10.12「共用配置」） */
/** 共用配置里的 agent 模板（board-core templates.rs）：字段和配置文件的 [agents.<标识>] 一样 */
export interface AgentTemplate {
  id: string;
  name: string;
  cmd: string[];
  model: string;
  modelArgs: string[];
  effortArgs: string[];
  efforts: string[];
  /** Windows 上支持的启动环境，按默认选择的顺序；空 = 默认 */
  win: string[];
  version?: string;
  updatedAt?: number;
  by?: string;
}

export interface McpServer {
  name: string;
  kind: 'stdio' | 'http';
  command: string;
  args: string[];
  env: McpVar[];
  url: string;
  headers: McpVar[];
  /** 同步给哪些 agent；空 = 所有支持的（claude、codex、pi） */
  agents: string[];
  version?: string;
  updatedAt?: number;
  by?: string;
  source?: string;
}

export interface McpState {
  server: string;
  agent: string;
  state: 'synced' | 'local' | 'unsupported' | 'failed';
  why?: string | null;
}

/** skills 共用库里的一个 skill（PRD §10.12「共用配置」） */
export interface SkillInfo {
  name: string;
  /** 内容的哈希（12 位） */
  version: string;
  size: number;
  files: number;
  updatedAt: number;
  by: string;
  /** 从哪里上传：机器名、界面；board-skills 是「随服务端 x.y.z」 */
  source: string;
  /** board-skills：随服务端版本，不能删、不能改 */
  builtin?: boolean;
}

/** 一台机器上一个 agent 目录里一个 skill 的同步状态 */
export interface SkillState {
  skill: string;
  agent: string;
  env: string;
  dir: string;
  state: 'synced' | 'behind' | 'local' | 'failed';
  version?: string | null;
  why?: string | null;
}

/** 自检（PRD §10.12「自检」）：agent 能不能干活、每块白板的目录能不能推送、board 版本是否落后 */
export interface SelfTest {
  /** 这台机器的 board host 会做自检；旧版不会（按钮置灰、要升级） */
  able?: boolean;
  /** 等 board host 做（或正在做）的自检 */
  ask?: { id: number; agent?: string | null; by: string; at: number } | null;
  items: SelfTestItem[];
}

export interface SelfTestItem {
  kind: 'agent' | 'git' | 'version';
  /** agent 标识、白板路径；版本这一项是 board */
  key: string;
  ok: boolean;
  why?: string | null;
  /** agent 回答的前几行、git 输出的最后几行 */
  detail?: string | null;
  at: number;
}

/** 远程管理请求 */
export interface HostAsk {
  id: number;
  kind: 'recheck' | 'restart' | 'upgrade';
  by: string;
  at: number;
  /** 升级：目标版本 */
  to?: string | null;
  /** 升级：不等在跑的派发结束 */
  now?: boolean;
}

export interface HostOp {
  at: number;
  by: string;
  text: string;
}

export interface HostToken {
  id: number;
  name: string;
  /** 令牌开头几位 */
  hint: string;
  lastUsedAt: number | null;
  /** 同一个令牌还被这些在线的机器使用（这时不能随移除机器一起吊销） */
  shared?: string[];
}

export type DispatchResult = 'review' | 'stuck' | 'crashed' | 'stopped' | 'timeout' | 'canceled' | 'lost' | 'unclaimed';

/** 一次派发：在某台机器上用某种 agent 做某张卡 */
/** 一条领取规则：卡片的标签满足条件就由这个 agent 做 */
export interface ClaimRule {
  /** in 带其中任一个 · all 全都带 · not_in 一个都不带 · equal 标签正好是这几个 · only 只在这几个里 · none 没有标签 */
  op: 'in' | 'all' | 'not_in' | 'equal' | 'only' | 'none';
  labels: string[];
  agent: string;
  /** agent 在这台机器上找得到（服务端算）；找不到时这条规则不生效 */
  active?: boolean;
  /** 这台机器有目录的项目里都没有的标签（服务端算） */
  missing?: string[];
}

export interface Dispatch {
  id: number;
  board: string;
  card: number;
  title: string;
  hostId: number;
  host: string;
  agent: string;
  agentLabel: string;
  /** 开始时生成的 agent 名字 <agent 标识>-<机器名>/<模型名>（并发派发时模型名后面带 -<序号>） */
  name: string | null;
  /** 开始时用的启动环境标识与名称（Git Bash、WSL（Ubuntu）…）；旧版 board host、还没开始时为空 */
  env?: string | null;
  envName?: string | null;
  source: 'manual' | 'cron';
  by: Actor;
  state: 'queued' | 'running' | 'done';
  result: DispatchResult | null;
  createdAt: number;
  startedAt: number | null;
  endedAt: number | null;
  exitCode: number | null;
  /** 有人请求停止（还在跑时）：谁 */
  stopBy: string | null;
  /** 持有者最近一次心跳 */
  beatAt: number | null;
  /** 同一个 agent 进程做的几张卡共用的会话标识：第一张的派发 ID。排队中、session 不是自己的，是开会话时排进去的（只由那个会话的 board next 开始）；旧版服务端没有 */
  session?: number;
  /** 这个会话的第几张、一共排了几张；不属于会话时 1、1；旧版服务端没有 */
  seq?: number;
  total?: number;
  /** 开始时传给 agent 的模型、思考强度（机器设置里选的）；没选、旧版时没有 */
  model?: string | null;
  effort?: string | null;
}

/** 在线的 agent（PRD §3）：名字最近一次心跳在 50 秒以内 */
export interface AgentInfo {
  name: string;
  /** 首次 check 的时间 */
  checkedAt: number;
  /** 最近一次心跳（只在上线、下线、卡片状态变化时推送，界面上的值不一定是最新的） */
  beatAt: number;
  /** 它持有的卡片（每块白板最多一张；可能没有） */
  cards: HeldCard[];
}

/** agent 持有的一张卡片：白板路径加上卡片本身 */
export type HeldCard = Card & { board: string };

/** agent 运行过程的一条结构化事件（会话「过程」，board-core runlog.rs） */
export interface RunEvent {
  id: number;
  /** 毫秒 */
  at?: number;
  kind: 'message' | 'tool' | 'error' | 'result' | 'text';
  /** message：agent 说的话（Markdown）；error：出错信息；result：结束时的结果文字；text：解析不了的一行 */
  text?: string;
  /** tool：工具名、参数摘要、结果摘要、成功或失败（还没有结果时没有）、退出码、用时 */
  tool?: string;
  args?: string;
  out?: string;
  ok?: boolean;
  code?: number;
  ms?: number;
  /** result：轮数、输入 / 输出 tokens、费用（美元），agent 输出里有才有 */
  turns?: number;
  tokensIn?: number;
  tokensOut?: number;
  cost?: number;
  /** 子 agent 里的事件 */
  sub?: boolean;
  /** 超过 4 KB 截断过 */
  cut?: boolean;
}

/** 一个会话（同一个 agent 进程用 board next 接着做的几张卡）：会话标识是第一张的派发 ID */
export interface SessionView {
  session: number;
  /** 这个会话的派发，按先后；每张带它的输出（最后 500 行）和结构化事件 */
  parts: (Dispatch & {
    log: string[];
    /** agent 的输出格式：claude、codex、pi、dsh；'' = 只有原始输出；没有 = board host 没报（旧版，或还没开始） */
    fmt?: string | null;
    /** 事件（按先后）；带 since 取时只有之后新增、改过的 */
    events: RunEvent[];
    /** 留着的事件里最早的 id（0 = 一条都没有） */
    eventsFrom: number;
    /** 删掉了几条（超过 2000 条、结束 30 天后） */
    eventsDropped: number;
  })[];
  /** 事件的版本：下次带上 since 只取之后的 */
  runs?: number;
}

export interface Card {
  id: number;
  title: string;
  body: string;
  column: ColumnKey;
  holder: string | null;
  claimedAt: number | null;
  /** 最近一次心跳（发了 final 之后是 final 的时间）；还没开始心跳时为空 */
  beatAt: number | null;
  /** 这一次的开始时间（第一次心跳）；没有进行中的一次时为空 */
  runStartedAt: number | null;
  /** 已经结束的各次耗时之和（毫秒），不含进行中的这一次 */
  workMs: number;
  /** 做过几次（含进行中的这一次） */
  runs: number;
  /** 其中几次是异常退出 */
  runsLost: number;
  /** 收到的心跳总次数（各次之和，含 final）；旧服务端没有这个字段 */
  beats?: number;
  /** 最近一次领取结束的时间（异常退出也算）；没有结束过的领取、旧服务端时为空 */
  finishedAt?: number | null;
  /** 领取它时所在的可领取状态；释放后回到这里 */
  claimedFrom: string | null;
  labels: number[];
  createdAt: number;
  updatedAt: number;
  /** 移入现在这个状态的时间（新建、移动、领取、释放都算；同一状态里调整顺序不算）。旧服务端没有 */
  movedAt?: number;
  createdBy: Actor;
  comments: number;
  /** 人看过这一条的时间（PRD §10.5）：「未整理」里 agent 记下、还没被人看过的条目算「新」；旧服务端没有这个字段 */
  seenAt?: number | null;
}

/** 「未整理」里的条目是不是「新」：agent 记下、人还没看过 */
export const isNewEntry = (c: Card) => c.column === 'inbox' && c.createdBy.kind === 'agent' && !c.seenAt;

export interface Activity {
  id: number;
  at: number;
  actor: Actor;
  kind: 'comment' | 'system';
  action: string;
  text: string;
  /** 评论被作者修改过时，最后一次修改的时间 */
  edited_at?: number;
}

/** 卡片附件（PRD §5）：图片在描述、评论里直接显示，其他文件是下载链接 */
export interface Attachment {
  id: number;
  /** 新建弹窗里先上传、还没归到卡片时为 null */
  cardId: number | null;
  name: string;
  mime: string;
  size: number;
  image: boolean;
  url: string;
  /** 插入描述或评论用的 Markdown */
  markdown: string;
  createdAt: number;
  createdBy: Actor;
}

export interface CardDetail extends Card {
  activity: Activity[];
  attachments?: Attachment[];
}

/**
 * 一条跨白板搜索结果（`GET /api/search`）：命令面板里搜卡片时，当前白板之外的结果也在这里
 */
export interface SearchHit {
  /** 白板路径：项目/白板 */
  board: string;
  id: number;
  title: string;
  /** 状态标识；「未整理」是 inbox */
  column: string;
  /** 状态名称 */
  status: string;
  /** 匹配到哪里：title | body | comment | any | ref | label */
  matched: string;
  labels: string[];
  updatedAt: number;
  /** 相关度：越大越相关 */
  score: number;
}

export interface BoardEvent {
  id: number;
  board: string;
  cardId: number;
  title: string;
  at: number;
  actor: Actor;
  op: string;
  text: string;
  via: 'ui' | 'cli' | 'system';
  client?: string;
}

export interface BoardSnapshot {
  rev: number;
  now: number;
  path: string;
  statuses: Status[];
  cols: Record<string, number[]>;
  cards: Record<string, Card>;
  /** 终点状态默认只带最近移入的几张：状态 → 省略的（更早的）张数。取全量时、旧服务端没有 */
  omitted?: Record<string, number>;
  log: BoardEvent[];
  /** 白板设置里「保存布局」存下的布局（只含现在还在的状态）；没保存过为空。旧服务端没有 */
  savedLayout?: Record<string, Tile> | null;
  /** 白板设置「保留顺序」：开着时每个状态块按自己的排序依据（Status.order）显示，拖动只换状态、不改顺序；
      关着时拖到哪就在哪。旧服务端没有（当作关着） */
  keepSorted?: boolean;
  /** 回收站里的张数（侧栏「回收站 · N」）。旧服务端没有这个字段（当作 0） */
  trash?: number;
  /** 这块白板的代码仓库地址（空表示没设）：评论里的提交号、gh#123 按它变成链接。旧服务端没有这个字段 */
  repo?: string;
  /** 这块白板的集成分支（空表示按仓库的 AGENTS.md / CLAUDE.md）。旧服务端没有这个字段 */
  branch?: string;
}

/** 回收站里的一张卡片（PRD §8 规则 10）：删除前在哪个状态、排第几位、谁删的、什么时候删的 */
export interface TrashCard extends Card {
  deletedAt: number;
  deletedBy: Actor;
  /** 删除前所在的状态（工作流状态或 inbox） */
  col: string;
  pos: number;
}

export type ErrorCode =
  | 'NO_IDENTITY'
  | 'NO_BOARD'
  | 'BOARD_NOT_FOUND'
  | 'NOT_FOUND'
  | 'EMPTY_TITLE'
  | 'EMPTY_TODO'
  | 'NOT_IN_TODO'
  | 'ALREADY_HOLDING'
  | 'NOT_HOLDER'
  | 'FORBIDDEN_COLUMN'
  | 'USAGE'
  | 'NOT_LOGGED_IN'
  | 'INVALID_TOKEN'
  | 'UNREACHABLE'
  | 'NAME_TAKEN'
  | 'NAME_OFFLINE'
  | 'NOT_AUTHOR'
  | 'HOST_NAME_TAKEN'
  | 'ALREADY_DISPATCHED'
  | 'NO_SUCH_HOST'
  | 'AGENT_NOT_ON_HOST'
  | 'BOARD_NOT_ON_HOST'
  | 'BOARD_NOT_READY'
  | 'DISPATCHED'
  | 'SESSION_DONE'
  | 'INTERNAL'
  /** 卡片在回收站里，不能编辑、移动、评论：先恢复 */
  | 'IN_TRASH'
  | 'NETWORK'
  /** 界面自己的：上一次修改还没保存完，不能再改同一个对象（不发请求） */
  | 'BUSY';

/** 当前登录的人：线上是 GitHub 账号；本机模式不需要登录 */
export type Me = { mode: 'github'; login: string; name: string; avatar: string } | { mode: 'local' };

/** agent 访问令牌（原文只在新建时返回一次） */
export interface TokenInfo {
  id: number;
  name: string;
  hint: string;
  createdAt: number;
  lastUsedAt: number | null;
}

/** 服务端版本信息（GET /api/info）：部署后用来核对线上是不是新版 */
export interface ServerInfo {
  name: string;
  version: string;
  commit: string;
}

export type Cmd =
  | { op: 'new'; title: string; body?: string; column?: 'inbox'; top?: boolean; labels?: number[] }
  | { op: 'edit'; id: number; title?: string; body?: string }
  | { op: 'move'; id: number; column: ColumnKey; index?: number; top?: boolean; order?: number[]; before?: number; after?: number }
  /** 评论并移动（复盘台的打回、回复并放回）：服务端一个事务里先评论再移动 */
  | { op: 'reply'; id: number; text: string; column: ColumnKey }
  | { op: 'comment'; id: number; text: string }
  | { op: 'edit_comment'; id: number; activity: number; text: string }
  | { op: 'delete_comment'; id: number; activity: number }
  | { op: 'label'; id: number; add?: number[]; remove?: number[] }
  | { op: 'release'; id: number }
  /** 删除卡片：移进回收站，30 天内可以恢复（规则 10） */
  | { op: 'delete'; id: number }
  /** 从回收站恢复（仅人） */
  | { op: 'restore'; id: number }
  /** 从回收站永久删除（仅人） */
  | { op: 'purge'; id: number }
  /** 合并「未整理」里的条目（仅人）：from 并进 id，只留下 id；标签取并集，来源的附件、评论归到 id */
  | { op: 'merge'; id: number; from: number[]; title?: string; body?: string }
  /** 人看过「未整理」里的条目（仅人）：不给 id 时是这块白板「未整理」里的全部（「全部标为已读」） */
  | { op: 'seen'; id?: number };

/* ------------------------------ 大盘（GET /api/stats） ------------------------------ */

export type StatsScope = 'board' | 'project' | 'all';

/** 一段时间的总量；时长都是毫秒，比例 0..1，没有样本时为 null */
export interface StatsPeriod {
  created: number;
  claims: number;
  done: number;
  rework: number;
  comments: number;
  beats: number;
  spent: number;
  leadP50: number | null;
  leadP85: number | null;
  waitP50: number | null;
  workP50: number | null;
  leadHist: number[];
  firstPass: number | null;
  idle: number;
}

export interface StatsBucket {
  t: number;
  created: number;
  claims: number;
  done: number;
  rework: number;
  comments: number;
  beats: number;
  spent: number;
}

/** review 交审 | rework 被打回 | lost 异常退出 | gaveup 做不下去 | released 领取被释放 | running 进行中 */
export type RunResult = 'review' | 'rework' | 'lost' | 'gaveup' | 'released' | 'running';

export interface StatsRun {
  agent: string;
  name: string;
  board: string;
  card: number;
  title: string;
  start: number;
  end: number | null;
  result: RunResult;
}

export interface StatsSeg {
  key: string | null;
  name: string;
  color: string | null;
  from: number;
  to: number;
}

export interface StatsLife {
  board: string;
  card: number;
  title: string;
  created: number;
  done: number;
  reworks: number;
  segs: StatsSeg[];
}

export interface StatsBrief {
  key: string;
  review: number;
  todo: number;
  busy: boolean;
  done: number;
  series: number[];
  leadP50: number | null;
  firstPass: number | null;
  rework: number;
}

export interface StatsWaiting {
  n: number;
  oldest: number | null;
}

export interface Stats {
  scope: StatsScope;
  path: string | null;
  from: number;
  to: number;
  now: number;
  tz: string;
  bucket: 'hour' | 'day';
  boardsInScope: string[];
  activity: number;
  cur: StatsPeriod;
  prev: StatsPeriod;
  series: StatsBucket[];
  idle: { board: string; from: number; to: number }[];
  agents: { agent: string; claims: number; spent: number; firstPass: number | null; lost: number; gaveup: number }[];
  dwell: { key: string; name: string; color: string; median: number; mean: number; n: number }[];
  hourHeat: number[][];
  dayHeat: { t: number; v: number; future: boolean }[];
  runs: StatsRun[];
  lives: StatsLife[];
  present: {
    statuses: { key: string; name: string; color: string; n: number }[];
    review: StatsWaiting;
    todo: StatsWaiting;
    doing: { n: number; holders: string[] };
    stuck: { board: string; card: number; title: string; key: string; name: string; color: string; since: number; holder: string | null; note: string | null }[];
  };
  life: { since: number; days: number; cards: number; finals: number; reworks: number; agents: number; spent: number; perDay: number } | null;
  boards: StatsBrief[];
  projects: StatsBrief[];
  all: StatsBrief;
}
