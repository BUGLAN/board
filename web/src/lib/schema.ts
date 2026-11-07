import type { BoardSnapshot, Card, ErrorCode, Status } from './types';

export const INBOX = 'inbox';
export const INBOX_NAME = '未整理';

/** 状态色板：前 4 个是主题色（随浅色 / 深色变化），其余固定 */
export const STATUS_COLORS = ['todo', 'doing', 'review', 'done', '#A855F7', '#EC4899', '#14B8A6', '#F59E0B', '#EF4444', '#84CC16'];
/** 状态颜色 → CSS 颜色 */
export const statusColor = (c: string) => (c.startsWith('#') ? c : `var(--c-${c})`);

/** 工作流的查询（白板快照或工作区里的 statuses 都可以） */
export function flowOf(statuses: Status[] | undefined | null) {
  const list = statuses ?? [];
  const get = (k: string) => list.find(s => s.key === k);
  const claim = list.find(s => s.claim) ?? list[0];
  return {
    list,
    get,
    claim,
    name: (k: string) => (k === INBOX ? INBOX_NAME : get(k)?.name ?? k),
    color: (k: string) => (k === INBOX ? 'var(--c-inbox)' : statusColor(get(k)?.color ?? 'todo')),
    isLock: (k: string) => !!get(k)?.lock,
    isFinal: (k: string) => !!get(k)?.final,
  };
}
export type FlowQ = ReturnType<typeof flowOf>;
export const boardFlow = (b: BoardSnapshot | null) => flowOf(b?.statuses);

/** 心跳间隔；从领取或最近一次心跳算起，连续 BEAT_MISSES 次没收到就释放（与 board-core 一致） */
export const BEAT_MS = 10 * 1000;
export const BEAT_MISSES = 5;

/** 卡片的耗时（PRD §8 规则 6）。now 用 useNow 的服务端对齐时间 */
export function timing(c: Card, now: number) {
  const cur = c.runStartedAt != null ? Math.max(0, now - c.runStartedAt) : null;
  const ref = c.beatAt ?? c.claimedAt;
  // 已经几次没收到心跳（只在进行中的这一次里有意义）
  const missed = c.holder && cur != null && ref != null ? Math.min(BEAT_MISSES, Math.floor(Math.max(0, now - ref) / BEAT_MS)) : 0;
  return {
    /** 这一次的耗时；没有进行中的一次时为 null */
    cur,
    total: c.workMs + (cur ?? 0),
    runs: c.runs,
    lost: c.runsLost,
    /** 心跳状态：live 正常 · late 晚了（已 1 次以上没收到）· wait 领取了还没开始心跳 · ended 发了 final 还没交审 · null 没被持有 */
    beat: !c.holder ? null : cur == null ? (c.runs > 0 && c.beatAt != null ? 'ended' : 'wait') : missed >= 1 ? 'late' : 'live',
    missed,
    ageSec: ref != null ? Math.max(0, Math.round((now - ref) / 1000)) : null,
  } as const;
}

export const PROJECT_COLORS = ['#FF6A33', '#4DB5FF', '#3DDC97', '#B98CFF', '#FFB020', '#FF5C8A', '#2DD4BF', '#94A3B8'];
export const LABEL_COLORS = ['#F43F5E', '#F97316', '#EAB308', '#22C55E', '#14B8A6', '#3B82F6', '#8B5CF6', '#EC4899', '#64748B'];

/** 错误码与默认原因（设置页 · Agent 接入） */
export const ERRORS: [ErrorCode, string][] = [
  ['NO_IDENTITY', 'CLI 没有提供 agent 名字：使用 --as <名字> 或设置环境变量 BOARD_AS'],
  ['NO_BOARD', 'CLI 没有指定白板：使用 --board <项目>/<白板> 或设置环境变量 BOARD'],
  ['BOARD_NOT_FOUND', '指定的项目或白板不存在；或项目下有多块白板却只写了项目标识'],
  ['NOT_FOUND', '卡片不存在'],
  ['EMPTY_TITLE', '新建或编辑时标题为空'],
  ['EMPTY_TODO', '不带 ID 领取时领取起点为空'],
  ['NOT_IN_TODO', '带 ID 领取的卡片不在领取起点，且不是自己持有的卡片'],
  ['ALREADY_HOLDING', '该 agent 在这块白板上已经持有另一张卡片'],
  ['NOT_HOLDER', '编辑或移动被持有卡片的不是持有者'],
  ['FORBIDDEN_COLUMN', '把卡片直接移进锁定状态；agent 把卡片移入、移出终点状态；工作流里没有锁定状态时领取'],
  ['USAGE', '命令格式或参数不正确：未知的命令、未知的状态、项目里不存在的标签、缺少必需的参数、评论内容为空'],
  ['NOT_LOGGED_IN', '没有登录：界面没有登录会话；CLI 设置了 BOARD_URL 却没有 BOARD_TOKEN；用访问令牌做只有人能在界面上做的操作'],
  ['INVALID_TOKEN', '访问令牌不存在或已吊销'],
  ['UNREACHABLE', 'CLI 连不上 BOARD_URL 指定的服务端'],
];

/** 标识：小写字母、数字和 -，以字母或数字开头，最长 32 个字符 */
export const KEY_RE = /^[a-z0-9][a-z0-9-]{0,31}$/;
export const slug = (name: string) =>
  String(name || '')
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 32);
