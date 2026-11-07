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
  /** 领取起点：board claim 从这里领；新建、超时释放、强制释放都回到这里 */
  claim: boolean;
  /** 锁定：只能由 agent 领取进入；被持有期间只有持有者能动 */
  lock: boolean;
  /** 终点：只有人能移入移出 */
  final: boolean;
  layout: Tile | null;
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
  expiresAt: number | null;
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

export interface CardDetail extends Card {
  activity: Activity[];
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

export type Cmd =
  | { op: 'new'; title: string; body?: string; column?: 'inbox'; top?: boolean; labels?: number[] }
  | { op: 'edit'; id: number; title?: string; body?: string }
  | { op: 'move'; id: number; column: ColumnKey; index?: number; top?: boolean }
  | { op: 'comment'; id: number; text: string }
  | { op: 'label'; id: number; add?: number[]; remove?: number[] }
  | { op: 'release'; id: number }
  | { op: 'delete'; id: number };
