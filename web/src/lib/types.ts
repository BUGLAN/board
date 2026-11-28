/* 与 board-core 的 JSON 对应 */
import type { AgentInfo, Chat, Dispatch, HostInfo, Sched } from './types/hosts';

export * from './types/hosts';
export * from './types/stats';

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
  /** 交审状态（状态设置里的开关）：board submit 把做完的卡移到这里。旧服务端没有这个字段 */
  review?: boolean;
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

/** 各状态的卡片数；`inbox` 是不含「稍后再做」的未整理条目，`later` 是收起的（两者相加才是未整理总数） */
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
  /** 所在项目的代码仓库地址（git 远端，例如 git@github.com:o/r.git）；空表示没设。仓库设在项目上，这里带一份。旧服务端没有这一项 */
  repo?: string;
  /** 所在项目的集成分支；空表示按仓库的 AGENTS.md / CLAUDE.md，没写就用远端默认分支。旧服务端没有这一项 */
  branch?: string;
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
  /** 派发用哪个角色（标识）；空表示按规则会交给的角色（规则没命中、也没有角色时用机器的默认 agent） */
  profile?: string | null;
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
  createdAt: number;
  updatedAt: number;
}

/** 新建、修改定时建卡时发的内容（修改时没给的项不变） */
export type CardCronInput = Partial<Pick<CardCron, 'name' | 'body' | 'labels' | 'place' | 'cron' | 'tz' | 'on' | 'host' | 'profile' | 'pick'>>;

export interface ProjectInfo {
  key: string;
  name: string;
  desc: string;
  color: string;
  createdAt: number;
  labels: Label[];
  /** 隐藏的标签：名称匹配任一条正则的不出现在侧栏、提示词菜单、标签选择器（lib/sidebarLabels，default/main#324）；旧服务端没有，按默认 `^v\d` */
  hiddenLabels?: string[];
  /** 代码仓库地址（git 远端）；空表示没设。一个项目一个仓库，项目下所有白板共用（PRD §4）。旧服务端没有这一项（那时设在白板上） */
  repo?: string;
  /** 集成分支；空表示按仓库的 AGENTS.md / CLAUDE.md，没写就用远端默认分支。旧服务端没有这一项 */
  branch?: string;
  /** 项目目录：每台机器上是 <工作空间>/<项目目录>；空表示用项目标识（lib/projectDir，#488）。旧服务端没有这一项 */
  dir?: string;
  /** 不在左侧项目面板显示；左侧有搜索词或它是当前项目时仍然显示。左上角切换器只保留当前项目（default/main#610、#615）。旧服务端没有这一项，按显示 */
  hidden?: boolean;
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
  /** 机器对话（最近有动静的在前）；旧服务端没有这个字段 */
  chats?: Chat[];
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
  /** 手动派发会得到的错误码：BOARD_NOT_ON_HOST */
  code: string;
  /** 原因，一句话 */
  why: string;
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
  /** 已经结束的各次耗时之和（毫秒），不含进行中的这一次。服务端每一次先截断到整秒再相加，等于动态里逐次记的耗时之和 */
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
  /** 复制自：同一块白板上另一张卡的编号（「复制为新卡片」，PRD §10.8）。原卡删除后照样留着编号；旧服务端没有 */
  copiedFrom?: number | null;
  /** 拆解过（default/main#735）：第一次有卡拆自它的时间；没拆过为 null。「未整理」里拆解过的原条目一直隐藏，
      删掉拆出的卡也不变，只有「撤销拆解」清掉。旧服务端没有这个字段（undefined） */
  splitAt?: number | null;
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
  /** 复制为：「复制自」是这张的卡片（不含回收站里的），按编号。旧服务端没有 */
  copiedInto?: number[];
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
  labels: string[];
  updatedAt: number;
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
  /** 拆出的卡被上面的省略挡在快照外的原条目：原条目 ID → 省略掉的拆出卡张数（不含回收站里的）。
      没有省略、取全量时、旧服务端没有（default/main#534） */
  splitOmitted?: Record<string, number>;
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
  /** 机器令牌绑定的机器名（「添加机器」新建的）：只有它能登记这台新机器；普通访问令牌为空（#507） */
  host?: string | null;
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
  /** 合并「未整理」里的条目（仅人）：from 并进 id，只留下 id；标签取并集，来源的附件、评论归到 id */
  | { op: 'merge'; id: number; from: number[]; title?: string; body?: string }
  /** 人看过「未整理」里的条目（仅人）：不给 id 时是这块白板「未整理」里的全部（「全部标为已读」） */
  | { op: 'seen'; id?: number }
  /** 「稍后再做」（default/main#303）：收起（on）/ 放回「未整理」里的一条，仅人 */
  | { op: 'later'; id: number; on: boolean }
  /** 撤销拆解（仅人，default/main#735）：id 是「未整理」里拆解过的原条目；拆出的卡（都还在未整理里）进回收站，原条目回到列表里 */
  | { op: 'unsplit'; id: number }
  /** 复制为新卡片（仅人，PRD §10.8）：按给的标题、描述、标签新建一张，不带原卡的动态、领取、耗时；
      files 把新描述里引用的原卡附件复制一份；move_to（移到一个状态）/ trash（移进回收站）在同一个写事务里处理原卡 */
  | { op: 'copy'; id: number; title: string; body?: string; column?: 'inbox'; labels?: number[]; files?: boolean; move_to?: ColumnKey; trash?: boolean };
