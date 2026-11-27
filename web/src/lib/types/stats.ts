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

/** 大盘「按角色」、分工页「近 30 天」的一行：派发记录上是这个角色的领取 */
export interface StatsProfile {
  profile: string;
  /** 角色现在的名字；删掉了时是派发时的名字（gone） */
  name: string;
  gone: boolean;
  claims: number;
  /** 交审过的张数（同一张算一次，含之后被打回的） */
  review: number;
  firstPass: number | null;
  /** 交审后被打回的次数 */
  rework: number;
  /** 异常退出（心跳中断、超时） */
  lost: number;
  gaveup: number;
  spent: number;
  /** 平均每次领取的用时（已结束的） */
  avg: number | null;
}

/** 进步曲线（大盘总览下面那张）：横轴是创建以来的时间，竖轴是各状态的卡片数 */
export interface StatsProgress {
  /** 从什么时候画起：白板、项目的创建时间，全部项目是最早的项目 */
  since: number;
  /** 采样时刻（毫秒）：第一个是 since，最后一个是现在 */
  t: number[];
  /** 状态：工作流顺序（未整理在最前），和 present.statuses 一致 */
  statuses: { key: string; name: string; color: string; final: boolean }[];
  /** n[i][j]：第 j 个时刻在第 i 个状态里的张数 */
  n: number[][];
  /** 范围内项目在 since 之后发的版本，按发版时间排；cards 是范围内带这个版本标签的卡片数 */
  versions: { project: string; name: string; at: number; cards: number }[];
}

export interface Stats {
  scope: StatsScope;
  path: string | null;
  from: number;
  to: number;
  now: number;
  tz: string;
  bucket: 'hour' | 'day';
  activity: number;
  cur: StatsPeriod;
  prev: StatsPeriod;
  series: StatsBucket[];
  idle: { board: string; from: number; to: number }[];
  agents: { agent: string; claims: number; spent: number; firstPass: number | null; lost: number; gaveup: number }[];
  /** 按角色：按角色派发启动的领取（旧服务端没有） */
  profiles?: StatsProfile[];
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
  /** 进步曲线：创建以来各状态的卡片数和发版，不受时间范围影响（旧服务端没有） */
  progress?: StatsProgress;
  boards: StatsBrief[];
  projects: StatsBrief[];
  all: StatsBrief;
}
