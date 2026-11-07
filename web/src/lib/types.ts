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
}

export interface BoardInfo {
  key: string;
  path: string;
  name: string;
  desc: string;
  pos: Pos;
  createdAt: number;
  updatedAt: number;
  counts: Counts;
  statuses: Status[];
  holders: string[];
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
  /** 领取它时所在的可领取状态；释放后回到这里 */
  claimedFrom: string | null;
  labels: number[];
  createdAt: number;
  updatedAt: number;
  createdBy: Actor;
  comments: number;
}

export interface Activity {
  id: number;
  at: number;
  actor: Actor;
  kind: 'comment' | 'system';
  action: string;
  text: string;
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
  log: BoardEvent[];
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
  | 'INTERNAL'
  | 'NETWORK';

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
  | { op: 'move'; id: number; column: ColumnKey; index?: number; top?: boolean }
  | { op: 'comment'; id: number; text: string }
  | { op: 'label'; id: number; add?: number[]; remove?: number[] }
  | { op: 'release'; id: number }
  | { op: 'delete'; id: number };
