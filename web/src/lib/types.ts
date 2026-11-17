/* 与 board-core 的 JSON 对应 */

/** 状态标识：工作流里的状态（todo、doing、s1…），或「未整理」inbox */
export type ColumnKey = string;

/** 状态块在 12 栏网格上的位置 */
export interface Tile {
  x: number;
  w: number;
  y: number;
}

/** 状态块里卡片的形态：卡片 / 单行 / 磁贴（只显示编号，横向排列、自动换行） */
export type StatusView = 'card' | 'line' | 'tile';

/** 「保留顺序」的排序依据：移入时间 / 编号 / 最近更新 / 标签（项目里的标签顺序，不分方向）/ 累计耗时 */
export type OrderBy = 'moved' | 'id' | 'updated' | 'labels' | 'work';
export interface Order {
  by: OrderBy;
  /** 倒序；按标签时不用 */
  desc: boolean;
}

/** 工作流里的一个状态（PRD §7） */
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
  /** 跑在哪个 agent 里；旧服务端、旧数据没有时为空，界面按旧格式从名字解析 */
  agent?: string;
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
  /** 仅人可打：只有人能在界面上给已有卡片加、去，agent 加、去时被拒绝；agent 建卡时可以带上。旧服务端没有这个字段，按 agent 可打 */
  humanOnly?: boolean;
  /** 优先：不带 ID 领取（和自动领取）时，带这个标签的卡片先被领取 */
  priority?: boolean;
}

export interface BoardInfo {
  key: string;
  path: string;
  name: string;
  desc: string;
  /** 白板说明（Markdown，可以为空）：大盘白板概览里不显示，只在「编辑白板」里改 */
  readme?: string;
  /** 所在项目的环境检查（设在项目上，这里带一份）：机器先在这块白板的目录里跑通它，才接这块白板的卡；都为空时不检查。旧服务端没有这一项 */
  readyCheck?: ReadyCheck;
  /** 所在项目的代码仓库地址（git 远端，例如 git@github.com:o/r.git）；空表示没设。仓库设在项目上，这里带一份。旧服务端没有这一项 */
  repo?: string;
  /** 所在项目的集成分支；空表示按仓库的 AGENTS.md / CLAUDE.md，没写就用远端默认分支。旧服务端没有这一项 */
  branch?: string;
  pos: Pos;
  createdAt: number;
  updatedAt: number;
  counts: Counts;
  statuses: Status[];
  holders: string[];
  /** 定时建卡（PRD §4）。旧服务端没有这一项 */
  cardCrons?: CardCron[];
  /** 参与自动领取（默认参与）：关掉后机器不自动领这块白板的卡，手动派发照常；旧服务端没有这一项 */
  auto?: boolean;
}

/** 一条定时建卡（PRD §4「定时建卡」）：到点按模板在白板上建一张卡；上一张还没完成时跳过 */
export interface CardCron {
  id: number;
  /** 白板内唯一，最长 24 个字；也是建卡标题（建卡时后面加日期） */
  name: string;
  /** 和 `name` 相同：旧版客户端的「卡片标题」，服务端只为兼容旧版返回，新建、修改都不用传 */
  title: string;
  body: string;
  labels: number[];
  /** home 首个可领取状态 | inbox 未整理 */
  place: 'home' | 'inbox';
  cron: string;
  /** IANA 时区名，保存时取浏览器的 */
  tz: string;
  on: boolean;
  /** 建出卡后派给哪台机器（机器名）；不派发、随机派发时为空。修改时发空字符串表示不派发 */
  host?: string | null;
  /** 用哪个 agent；空表示那台机器（随机派发时是挑中那台）的默认 agent */
  agent?: string | null;
  /** 选哪台机器：host 派给 `host`，random 每次在能接这块白板、在线、没暂停接单的机器里随机挑一台；不派发时也留着（host） */
  pick?: 'host' | 'random';
  /** 下次到点（毫秒）；关着、一年内不会到点时为空 */
  nextAt?: number | null;
  /** 最近一次到点（或「现在建一张」）的时间 */
  ranAt?: number | null;
  /** 那一次建的卡 */
  ranCard?: number | null;
  /** 那一次跳过：上一张（这个编号）还没完成 */
  ranSkip?: number | null;
  /** 那一次建的卡派发到了哪：「<机器> · <agent>」 */
  ranTo?: string | null;
  /** 那一次建的卡没派发出去的原因（卡留在首个可领取状态） */
  ranErr?: string | null;
  lastCard?: number | null;
  createdAt: number;
  updatedAt: number;
}

/** 新建、修改定时建卡时发的内容（修改时没给的项不变） */
export type CardCronInput = Partial<Pick<CardCron, 'name' | 'body' | 'labels' | 'place' | 'cron' | 'tz' | 'on' | 'host' | 'agent' | 'pick'>>;

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
  /** 隐藏的标签：名称匹配任一条正则的不出现在侧栏、提示词菜单、标签选择器（lib/sidebarLabels，default/main#324）；旧服务端没有，按默认 `^v\d` */
  hiddenLabels?: string[];
  /** 代码仓库地址（git 远端）；空表示没设。一个项目一个仓库，项目下所有白板共用（PRD §4）。旧服务端没有这一项（那时设在白板上） */
  repo?: string;
  /** 集成分支；空表示按仓库的 AGENTS.md / CLAUDE.md，没写就用远端默认分支。旧服务端没有这一项 */
  branch?: string;
  /** 环境检查，项目下所有白板共用（PRD §9.1）。旧服务端没有这一项（那时设在白板上） */
  readyCheck?: ReadyCheck;
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
  /** 接不了的「机器 × 白板」和原因（服务端 fit.rs 算好）：界面照着禁用、显示；旧服务端没有这个字段 */
  blocks?: HostBlock[];
  /** 调度的全局设置：领取规则、兜底、服务端现在的时间；旧服务端没有这个字段 */
  sched?: Sched;
  /** 每块白板「跑第一张卡」还差什么（服务端 firstrun.rs 算好）：界面照着画清单；旧服务端没有这个字段，没有时不显示 */
  checklist?: Checklist[];
}

/** 一块白板「跑第一张卡」的清单 */
export interface Checklist {
  /** `<项目>/<白板>` */
  board: string;
  /** 已经有卡被机器交审过：不再提示 */
  done: boolean;
  items: CheckItem[];
}

/** 清单的一项：host 有能接的在线机器 / agent 能干活 / repo 项目设了仓库 / push 能推送 / auto 自动领取 */
export interface CheckItem {
  key: string;
  /** ok 通过 / fail 没通过 / unknown 还没确认（没自检过）/ wait 要等前面的项 */
  state: 'ok' | 'fail' | 'unknown' | 'wait';
  why: string;
  /** 去哪处理：add-host、hosts、host-settings:<机器>、host-agents:<机器>、selftest:<机器>、project-repo、project-check、git、rules、accept、board-auto */
  go?: string | null;
  /** 只影响自动领取，不影响手动派发 */
  auto?: boolean;
}

/** 一台机器接不了一块白板的卡 */
export interface HostBlock {
  host: string;
  /** `<项目>/<白板>` */
  board: string;
  /** 手动派发会得到的错误码：BOARD_NOT_ON_HOST、BOARD_NOT_READY */
  code: string;
  /** 原因，一句话 */
  why: string;
}

/* ---------------- 任务下发（board host） ---------------- */

/** 本机 agent：board host 启动时按配置文件和内置模板找到的（没找到的也上报） */
export interface HostAgent {
  /** 标识：claude、dsh、codex … */
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
  /** 这个 agent 现在用的模型（board host 读它的配置文件上报）：派发名字（<机器名>/<模型名>-<强度>）的第二段 */
  model?: string;
  /** 这个 agent 配置文件里现在的思考强度 */
  effort?: string;
  /** 模型怎么配：off 不改 | own 指定 */
  run_mode?: RunMode | null;
  /** 指定的供应商、模型、强度（run_mode 是 own 时） */
  run_provider?: string | null;
  run_model?: string | null;
  run_effort?: string | null;
  /** 指定用供应商的哪种接口（agent 认几种、供应商也给了几种时才要选）；空 = 供应商里第一个对得上的 */
  run_protocol?: string | null;
  /** 机器设置里选的启动环境 */
  run_env?: string | null;
  /** 这个 agent 在这台机器上连续异常退出、已经暂停接单：点「恢复」才继续 */
  tripped?: boolean;
  /** 安装方式、最新版本、更新状态（内置 agent 才有；旧版 board host 没有） */
  manage?: AgentManage | null;
}

/** 一个内置 agent 在这台机器上的安装和更新状态 */
export interface AgentManage {
  /** npm-global | brew | script | app | config | wsl | unknown | missing */
  method: string;
  /** 界面上显示的安装方式 */
  method_label: string;
  /** 没找到时给的安装命令 */
  install_command?: string;
  /** npm 上查到的最新版本 */
  latest_version?: string;
  /** 最近一次版本检查的时间 */
  checked_at?: number;
  /** 有新版本 */
  update_available?: boolean;
  /** 能由 board host 安装 / 升级 */
  installable?: boolean;
  updatable?: boolean;
  /** 不能代管的原因 */
  reason?: string;
  /** 空 = 空闲；running = 正在安装/更新；failed = 上一次失败 */
  state?: 'running' | 'failed' | '';
  /** 上一次失败的原因 */
  error?: string;
}

/** 一个 agent 的模型怎么配：不改（Board 不碰它的配置文件）/ 指定（这台机器上用哪个供应商、模型、强度、接口） */
export type RunMode = 'off' | 'own';

/** 一个 agent 在一个启动环境里的检查结果 */
export interface EnvCheck {
  env: string;
  found: boolean;
  version: string;
  /** 找到了命令、却看不到 board-skills：该装到哪里 */
  no_skill?: string | null;
  /** 这个启动环境里的安装方式、最新版本和更新状态 */
  manage?: AgentManage | null;
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

/** 一台机器的工作空间（PRD §9.1）：每个项目的目录是 `<工作空间>/<项目>`，项目下的白板共用 */
export interface HostWorkspace {
  /** 配置里写的路径（~ 不展开） */
  path: string;
  /** 目录不存在、是在这台机器上给这块白板新建的 */
  created?: boolean;
  /** `--dir <项目>=<路径>` 单独指定的项目目录（旧写法 `<项目>/<白板>=<路径>` 只管那一块白板，兼容保留） */
  overrides?: { board: string; path: string }[];
}

export interface HostDir {
  /** 项目/白板 */
  board: string;
  /** 实际用的路径：所在项目的 `<工作空间>/<项目>`（`--dir` 覆盖的是覆盖的路径） */
  path: string;
  /** 命令行用 --dir 单独覆盖：界面只读 */
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
  /** board host 上报过这块白板的目录状态 */
  reported?: boolean;
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
  /** 工作空间：每台设备一个目录，每个项目的目录由它派生（`<工作空间>/<项目>`）；还没配时没有 */
  workspace?: HostWorkspace | null;
  dirs: HostDir[];
  /** 服务端工作空间设置的版本 */
  dirsRev?: number;
  /** 最近一次改目录设置的时间、是谁改的 */
  dirsChangedAt?: number | null;
  dirsChangedBy?: string | null;
  /** 最多几个会话：同一时间最多开几个 agent 进程（1–8） */
  max: number;
  /** 自动领取启动的会话一个最多做几张（1–8，默认 4）；旧版服务端没有 */
  perSession?: number;
  /** 单次最长运行（分钟，0 = 不限）：所有机器一个值，在 Agent tab「跑多久停」里改（Sched.maxRun） */
  maxRun?: number;
  /** 多久没有输出算卡住（分钟，0 = 不限）：同上 */
  idleRun?: number;
  /** 领取规则（全局的一份，每台机器读到的都一样；active 按这台机器算）：没有一条规则时不自动领卡 */
  rules?: ClaimRule[];
  /** 兜底：不带任何规则标签的卡片由这个 agent 做；没设时不领这些卡片 */
  fallback?: string | null;
  fallbackActive?: boolean;
  /** 有规则、没设兜底时：可领取、没有匹配任何规则的卡片数 */
  unmatched?: number;
  paused: boolean;
  /** 内置 agent 自动更新（默认关）：有新版本且没有在跑的派发时按实际安装方式更新；旧服务端没有这一项 */
  agentAuto?: boolean;
  /** 默认 agent（设置过且找到了就是它，否则第一个找到的） */
  agent: string | null;
  /** 设置的默认 agent 在这台机器上找不到时是它（例如从别的机器复制来的）：不生效 */
  agentSet?: string | null;
  /** 旧的定时领取：现在一直是关、空的（换成了 auto、windows） */
  cronOn: boolean;
  cron: string[];
  /** 现在在自动接单：开了自动接单、没暂停、有领取规则、现在在接单时段里 */
  round: boolean;
  /** 参与自动接单；旧服务端没有这一项 */
  auto?: boolean;
  /** 接单时段：一天里的几段 [起, 止)，分钟，每天一样，按服务器时间；旧服务端没有这一项 */
  windows?: [number, number][];
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
  /** skills 用服务器的（库里的写进各个 agent 的 skill 目录）；否则不改（只装 board-skills） */
  syncSkills?: boolean;
  /** MCP 用服务器的；否则不改 */
  syncMcp?: boolean;
  /** Git 凭据用服务器的（写进用户级 git 配置，按主机全局生效）；否则不改。旧版服务端没有 */
  syncGit?: boolean;
  /** 同步配置：等 board host 做的请求、最近一次的结果 */
  sync?: ConfigSync;
  /** 从这台机器导入 skills：本机的 skill、等 board host 做的请求、最近一次的结果；旧版服务端没有 */
  skillImport?: SkillImport;
  /** 资源（CPU、内存、GPU、磁盘）：最近一次采样和最近几次的使用率；旧版 board host 不报，没有 */
  res?: HostRes;
}

/** 机器资源（PRD §10.12「资源」）：board host 后台每 10 秒采一次，随上报带给服务端 */
export interface HostRes {
  /** 采样时间（毫秒） */
  at: number;
  /** 取不到时没有 */
  cpu?: { pct: number; cores: number; load?: [number, number, number]; model?: string };
  /** 字节 */
  mem?: { total: number; used: number };
  /** 取不到（系统不支持）时没有；找过、没有 GPU 时是空数组 */
  gpus?: ResGpu[];
  /** 工作空间所在的盘排第一；取不到时没有 */
  disks?: ResDisk[];
  /** 最近几次的使用率（0–100，旧的在前，最多 30 个）；gpu 是第一块 */
  hist: { cpu: number[]; mem: number[]; gpu: number[] };
}

export interface ResGpu {
  name: string;
  vendor?: string;
  /** 使用率；读不到时没有 */
  pct?: number;
  /** 显存（字节）；Apple 芯片和内存共用，没有 */
  vramUsed?: number;
  vramTotal?: number;
}

export interface ResDisk {
  mount: string;
  /** 卷名（macOS 的「Macintosh HD」） */
  label?: string;
  total: number;
  avail: number;
  /** 工作空间在这块盘上 */
  ws?: boolean;
}

/** MCP 服务器的一个环境变量或请求头。列表接口里密钥的 value 是空的，tail 是末尾 4 位；保存时密钥留空表示保留原来的值 */
export interface McpVar {
  key: string;
  value?: string | null;
  secret?: boolean;
  tail?: string | null;
}

/** 同步配置（机器详情与配置同步重构）：服务器上一改就给受影响的机器下发请求，board host 写完上报 */
export interface ConfigSync {
  /** board host 会按请求同步、上报结果；旧版不会 */
  able?: boolean;
  /** 等 board host 做（或正在做）的同步 */
  ask?: { id: number; key: string; by: string; at: number } | null;
  /** 最近一次的结果 */
  last?: SyncResult | null;
}

export interface SyncResult {
  id?: number | null;
  key: string;
  at: number;
  /** 每个找到的 agent 一行 */
  agents: ProviderState[];
  /** skills：synced 库里的都写好了 / off 不改（只装了 board-skills）/ failed 有没写进去的 */
  skills: SkillsSync;
  /** MCP：synced 写好了 / off 不改 / failed 有没写进去的 */
  mcp: McpSync;
  /** Git 凭据：synced 配好了 / off 不改 / none 服务端没设 / failed 没配上；旧版 board host 不报（空） */
  git: GitSync;
}

/** 同步到机器的一类配置这一次的结果（skills、MCP、Git 凭据形状一样，见 syncKinds.ts）：
    共同的部分是状态、数量、原因（时间在外层），各自多出来的明细和它们平铺在同一层 */
export type SyncPart<D> = {
  state: 'synced' | 'off' | 'none' | 'failed' | '';
  /** 写进去几个：skill、MCP 服务器、配好的主机（Git 凭据；旧版 board host 不报） */
  count?: number;
  /** 没写进去的原因 */
  why?: string | null;
} & D;

export type SkillsSync = SyncPart<{
  count: number;
  /** 写进了哪几个 skill 目录（例如 ~/.claude/skills） */
  dirs: string[];
  /** 这一次换掉了本机同名的 */
  replaced: string[];
  /** 本机同名的是软链接，没有覆盖 */
  links: string[];
}>;

export type McpSync = SyncPart<{
  count: number;
  /** 写进了哪几个 agent（名称） */
  agents: string[];
  /** 这一次换掉了本机同名的条目 */
  replaced: string[];
}>;

export type GitSync = SyncPart<{
  /** 凭据的主机 */
  host?: string;
  /** 配好的仓库（目录）；按主机全局配置之后为空，旧版 board host 才报 */
  repos: string[];
  /** 没配的仓库：「目录：原因」（远端是 SSH）；按主机全局配置之后为空 */
  skipped: string[];
}>;

/** 照另一台机器配（机器详情「维护」、添加机器的「上线后照 <机器> 配」）：预览时 done 为 false */
export interface HostCopy {
  from: string;
  to: string;
  /** 要变的项（没有变化的不列）；空 = 已经一样 */
  changes: { key: string; label: string; from: string; to: string }[];
  /** 没照着改、要说一声的：系统不同启动环境没复制、这台机器上还没找到的 agent */
  notes: string[];
  done: boolean;
}

/** 服务端的 Git 凭据（共用配置）：令牌只有末尾 4 位 */
export interface GitCred {
  host: string;
  username: string;
  /** 提交作者的用户名（用户级 git 配置的 user.name） */
  commitName?: string;
  /** 提交作者的邮箱（用户级 git 配置的 user.email） */
  commitEmail?: string;
  /** 保存时填；列表里没有 */
  token?: string | null;
  tail?: string | null;
  version?: string;
  updatedAt?: number;
  by?: string;
  source?: string;
}

/** 一个文件夹（选工作空间时 board host 列出来的） */
export interface DirEntry {
  name: string;
  path: string;
  /** 它本身是 git 仓库 */
  git: boolean;
  /** 它下面一层有几个 git 仓库 */
  repos: number;
}

export interface DirList {
  /** 列的是哪个文件夹（~ 已经展开）；最上面一层为空 */
  path: string;
  /** 从最上面一层（空）到它自己的每一级 */
  chain: string[];
  entries: DirEntry[];
  error?: string | null;
}

/** 共用配置里的一个 MCP 服务器（PRD §10.16「共用配置」） */
export interface McpServer {
  name: string;
  kind: 'stdio' | 'http';
  command: string;
  args: string[];
  env: McpVar[];
  url: string;
  headers: McpVar[];
  /** 同步给哪些 agent；空 = 所有支持的（claude、codex） */
  agents: string[];
  version?: string;
  updatedAt?: number;
  by?: string;
  source?: string;
}

/* ---------------- 模型供应商（PRD §10.16「共用配置」） ---------------- */

/** 一个供应商的一个接口和地址（三种接口各一个地址） */
export interface ProviderEndpoint {
  interface: 'anthropic' | 'openai-chat' | 'openai-responses';
  url: string;
}

/** 共用配置里的一个模型供应商：API key 只存在服务端，列表里只有末尾 4 位 */
export interface Provider {
  name: string;
  endpoints: ProviderEndpoint[];
  /** 列表接口里为空，保存时留空 = 保留原来的值 */
  apiKey?: string | null;
  /** 密钥末尾 4 位 */
  tail?: string | null;
  models: string[];
  /** 每个模型支持的思考强度（从低到高）；没标的模型不能选强度 */
  efforts?: Record<string, string[]>;
  version?: string;
  updatedAt?: number;
  by?: string;
  source?: string;
}

/** 一台机器上一个 agent 这次同步配置的结果 */
export interface ProviderState {
  agent: string;
  /** synced 已写入 | off 不改 | unsupported 不支持 | failed 写入失败 | none 旧版 board host 报的「服务器上没给它配」 */
  state: 'synced' | 'off' | 'none' | 'unsupported' | 'failed';
  /** 写进去的供应商、模型、强度（synced 时） */
  provider?: string;
  model?: string;
  effort?: string;
  /** 选了、但模型不支持所以没写的强度 */
  dropped?: string;
  why?: string | null;
}

/** 内置 agent 的说明（服务端 board-core 留一份）：认哪些接口、配置文件能写哪几档强度 */
export interface AgentMeta {
  id: string;
  name: string;
  protocols: string[];
  efforts: string[];
  /** 读用户级 skill 的目录（第一个是装 board-skills 的地方）；旧版服务端没有 */
  skills?: string[];
  /** 支持 MCP 同步；旧版服务端没有 */
  mcp?: boolean;
  /** 界面上的主色；旧版服务端没有 */
  color?: string;
}

/** 列表接口：供应商（打码）、内置 agent 的说明 */
export interface ProviderView {
  providers: Provider[];
  meta?: AgentMeta[];
}

/** 一台机器上各 agent 的 skill 目录里、不是同步装的 skill（「从机器导入」用）；同名同内容在几个目录里的合成一项 */
export interface LocalSkill {
  name: string;
  /** 内容的版本号（和库里的算法一样）；读不了时为空 */
  version: string;
  files: number;
  size: number;
  agents: string[];
  /** 所在的 skill 目录（显示用），导入时用它找 */
  dirs: string[];
  /** 不能导入的原因 */
  why?: string | null;
}

export interface SkillImportPick {
  name: string;
  dir: string;
}

/** 从一台机器导入 skills（PRD §10.16「共用配置」） */
export interface SkillImport {
  /** 这台机器的 board host 会上报、会导入；旧版不会 */
  able: boolean;
  local: LocalSkill[];
  /** 等 board host 做（或正在做）的导入 */
  ask?: { id: number; picks: SkillImportPick[]; by: string; at: number } | null;
  /** 最近一次导入的结果 */
  last?: { id: number; at: number; by: string; ok: string[]; failed: { name: string; why: string }[] } | null;
}

/** skills 共用库里的一个 skill（PRD §10.16「共用配置」） */
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

/** 自检（PRD §10.12「自检」）：agent 能不能干活、每个项目的目录能不能推送、board 版本是否落后 */
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
  kind: 'recheck' | 'restart' | 'upgrade' | 'drain' | 'cancel' | 'agent-install' | 'agent-update';
  by: string;
  at: number;
  /** 升级：目标版本 */
  to?: string | null;
  /** 升级：不等在跑的派发结束 */
  now?: boolean;
  /** agent-install / agent-update：哪个 agent */
  agent?: string | null;
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

export type DispatchResult = 'review' | 'stuck' | 'crashed' | 'stopped' | 'timeout' | 'canceled' | 'lost' | 'unclaimed' | 'split' | 'fine' | 'nosplit';

/** 一条领取规则：卡片的标签满足条件就由这个 agent 做 */
export interface ClaimRule {
  /** in 带其中任一个 · all 全都带 · not_in 一个都不带 · equal 标签正好是这几个 · only 只在这几个里 · none 没有标签 */
  op: 'in' | 'all' | 'not_in' | 'equal' | 'only' | 'none';
  labels: string[];
  agent: string;
  /** agent 找得到（服务端算）：机器上的规则看这台机器，全局的规则看有没有任一台机器；找不到时这条规则不生效 */
  active?: boolean;
  /** 所有项目里都没有的标签（服务端算） */
  missing?: string[];
}

/** 一个 agent 的「由这些机器做」（服务端算）：当前选中的机器，以及默认 / 空选状态 */
export interface SchedPick {
  agent: string;
  /** 当前选中的机器 id（默认全选时也是现在装了它的全部机器） */
  hosts: number[];
  /** 一台都没选：这个 agent 不自动领卡 */
  none?: boolean;
  /** 没设置过（或已经清掉排除）：所有装了它的机器都参与，之后新装的也默认参与 */
  unset?: boolean;
}

/** 调度的全局设置（随工作区数据下发） */
export interface Sched {
  /** 领取规则，有顺序：第一条命中的生效 */
  rules: ClaimRule[];
  /** 兜底的 agent；没有时规则没命中的卡片不领 */
  fallback?: string | null;
  fallbackActive?: boolean;
  /** 服务端的时区（没设时为空） */
  tz?: string;
  /** 取数据那一刻是服务端时区一天里的第几分钟 */
  minute: number;
  /** 自动升级：服务端的版本比机器新时，机器等手上的派发做完自己升级 */
  autoUpgrade?: boolean;
  /** 单次最长运行（分钟，0 = 不限），所有机器一样 */
  maxRun?: number;
  /** 没输出多久算卡住（分钟，0 = 不限），所有机器一样 */
  idleRun?: number;
  /** 每个 agent 由哪些机器做（没有这一项的 agent 所有装了它的机器都参与） */
  picks?: SchedPick[];
}

/** 一次派发：在某台机器上用某种 agent 做某张卡 */
export interface Dispatch {
  id: number;
  board: string;
  card: number;
  title: string;
  /** 派给任意机器（服务端挑）：还没挑到机器时 host 是空的、hostId 是 0，没指定 agent 时 agent 也是空的 */
  any?: boolean;
  hostId: number;
  host: string;
  agent: string;
  agentLabel: string;
  /** 开始时生成的 agent 名字 <机器名>/<模型名>-<强度>（并发派发时末尾带 -<序号>） */
  name: string | null;
  /** 开始时用的启动环境标识与名称（Git Bash、WSL（Ubuntu）…）；旧版 board host、还没开始时为空 */
  env?: string | null;
  envName?: string | null;
  /** manual 手动 | cron 自动领取 | schedule 定时建卡 */
  source: 'manual' | 'cron' | 'schedule';
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
  /** '' = 做这张卡（领取）；'split' = 拆解「未整理」里的这条（不领取、不移动）。旧服务端没有 */
  kind?: string;
}

/** 在线的 agent（PRD §3）：名字最近一次心跳在 50 秒以内 */
export interface AgentInfo {
  name: string;
  /** 这个会话跑在哪个 agent 里（claude、dsh、codex …）：名字里不再带标识，画图标用它 */
  agent?: string | null;
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
    /** agent 的输出格式：claude、codex、dsh；'' = 只有原始输出；没有 = board host 没报（旧版，或还没开始） */
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
  /** 持有者跑在哪个 agent 里（服务端从名字表带出来）：新名字不再带 agent 标识，画图标用它；旧服务端、认不出时为空 */
  holderAgent?: string | null;
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
  /** 「稍后再做」（default/main#303）：人把「未整理」里的这一条收起来的时间；没收起为 null。旧服务端没有这个字段 */
  laterAt?: number | null;
  /** 拆自：同一块白板上另一张卡的编号（board new --from）；原卡删除后照样留着编号。旧服务端没有 */
  splitFrom?: number | null;
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
  /** 拆出：「拆自」是这张的卡片（不含回收站里的），按编号。旧服务端没有 */
  splitInto?: number[];
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
  /** 所在项目的代码仓库地址（空表示没设）：评论里的提交号、gh#123 按它变成链接。旧服务端没有这个字段 */
  repo?: string;
  /** 所在项目的集成分支（空表示按仓库的 AGENTS.md / CLAUDE.md）。旧服务端没有这个字段 */
  branch?: string;
}

/** 回收站里的一张卡片（PRD §8 规则 10）：删除前在哪个状态、排第几位、谁删的、什么时候删的 */
export interface TrashCard extends Card {
  deletedAt: number;
  deletedBy: Actor;
  /** 删除前所在的状态（工作流状态或 inbox） */
  col: string;
  pos: number;
  /** 删除前领取它的那个可领取状态（被删时还被持有的才有）；卡片本身的 claimedFrom 在删除时已经清空。旧服务端没有 */
  heldFrom?: string | null;
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
  | { op: 'seen'; id?: number }
  /** 「稍后再做」（default/main#303）：收起（on）/ 放回「未整理」里的一条，仅人 */
  | { op: 'later'; id: number; on: boolean };

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
