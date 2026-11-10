import type { BoardSnapshot, Card, ErrorCode, Status, StatusView } from './types';

export const INBOX = 'inbox';
export const INBOX_NAME = '未整理';

/** 状态色板：前 4 个是主题色（随浅色 / 深色变化），其余固定 */
export const STATUS_COLORS = ['todo', 'doing', 'review', 'done', '#A855F7', '#EC4899', '#14B8A6', '#F59E0B', '#EF4444', '#84CC16'];
/** 状态颜色 → CSS 颜色 */
export const statusColor = (c: string) => (c.startsWith('#') ? c : `var(--c-${c})`);

/** 状态块里卡片的形态；没设置过时终点用磁贴，其他用卡片 */
export const viewOf = (s: Status): StatusView => s.view ?? (s.final ? 'tile' : 'card');
export const VIEWS: [StatusView, string, string][] = [
  ['card', '卡片', '编号、标题、标签、持有者和计时'],
  ['line', '单行', '编号和标题占一行'],
  ['tile', '磁贴', '只显示编号，横向排列；悬停看标题'],
];

/** 工作流的查询（白板快照或工作区里的 statuses 都可以） */
export function flowOf(statuses: Status[] | undefined | null) {
  const list = statuses ?? [];
  const get = (k: string) => list.find(s => s.key === k);
  /** 首个可领取状态：新建卡片、整理、删除状态时卡片放进这里 */
  const home = list.find(s => s.claim) ?? list[0];
  const hasEdge = (a: string, b: string) => !!get(a)?.to?.includes(b);
  /** 从可领取状态领取后进入的锁定状态：它连向的锁定状态里排在最前面的 */
  const claimTarget = (from: string) => list.find(s => s.lock && hasEdge(from, s.key));
  return {
    list,
    get,
    home,
    hasEdge,
    claimTarget,
    /** 可领取状态（按工作流顺序） */
    claimables: list.filter(s => s.claim),
    /** 可领取状态里的卡片总数 */
    ready: (counts: Record<string, number> | undefined) => list.filter(s => s.claim).reduce((n, s) => n + (counts?.[s.key] ?? 0), 0),
    /** 不带 ID 执行 board claim 时会被领走的那张（PRD §8 规则 1）：有排队派发的卡片跳过（skip）；
     *  带优先标签的（urgent）先领，都不带时取最前面那张（与 board-core 的 next_claimable 一致） */
    nextCard: (cols: Record<string, number[]> | null | undefined, skip?: (id: number) => boolean, urgent?: (id: number) => boolean) => {
      let first: number | undefined;
      for (const s of list) {
        if (!s.claim || !claimTarget(s.key)) continue;
        for (const id of cols?.[s.key] ?? []) {
          if (skip?.(id)) continue;
          if (urgent?.(id)) return id;
          first ??= id;
        }
      }
      return first;
    },
    name: (k: string) => (k === INBOX ? INBOX_NAME : get(k)?.name ?? k),
    color: (k: string) => (k === INBOX ? 'var(--c-inbox)' : statusColor(get(k)?.color ?? 'todo')),
    isLock: (k: string) => !!get(k)?.lock,
    isFinal: (k: string) => !!get(k)?.final,
  };
}
export type FlowQ = ReturnType<typeof flowOf>;
/** 释放后卡片回到哪里：领取它时所在的可领取状态，否则首个可领取状态（PRD §8 规则 7） */
export const backTo = (f: FlowQ, from: string | null | undefined) => (from && f.get(from)?.claim ? f.name(from) : f.home?.name ?? '待办');
export const boardFlow = (b: BoardSnapshot | null) => flowOf(b?.statuses);

/** 心跳间隔；连续 BEAT_MISSES 次没收到：名字离线，卡片（从领取或最近一次心跳算起）释放（与 board-core 一致） */
export const BEAT_MS = 10 * 1000;
export const BEAT_MISSES = 5;

/** 卡片的耗时（PRD §8 规则 6）。now 用 useNow 的服务端对齐时间；online 是在线的 agent 名字（useOnline），
    还没取到时为 null。只更新心跳时间的心跳不推送给界面，界面上的 beatAt 不是最新的：心跳正不正常看持有者在不在线 */
export function timing(c: Card, now: number, online?: ReadonlySet<string> | null) {
  const cur = c.runStartedAt != null ? Math.max(0, now - c.runStartedAt) : null;
  const alive = !c.holder || !online || online.has(c.holder);
  return {
    /** 这一次的耗时；没有进行中的一次时为 null */
    cur,
    total: c.workMs + (cur ?? 0),
    runs: c.runs,
    lost: c.runsLost,
    beats: c.beats ?? 0,
    /** 心跳状态：live 正常 · late 持有者已离线（心跳中断，马上释放）· wait 领取了还没开始心跳 · ended 发了 final 还没移走 · null 没被持有 */
    beat: !c.holder ? null : cur == null ? (c.runs > 0 && c.beatAt != null ? 'ended' : 'wait') : alive ? 'live' : 'late',
  } as const;
}

export const PROJECT_COLORS = ['#FF6A33', '#4DB5FF', '#3DDC97', '#B98CFF', '#FFB020', '#FF5C8A', '#2DD4BF', '#94A3B8'];
export const LABEL_COLORS = ['#F43F5E', '#F97316', '#EAB308', '#22C55E', '#14B8A6', '#3B82F6', '#8B5CF6', '#EC4899', '#64748B'];

/** 错误码与默认原因（设置页 · Agent 接入） */
export const ERRORS: [ErrorCode, string][] = [
  ['NO_IDENTITY', 'CLI 没有提供 agent 名字：先 board check --as <名字>，之后每条命令都带 --as <名字>'],
  ['NO_BOARD', 'CLI 没有指定白板：使用 --board <项目>/<白板> 或设置环境变量 BOARD'],
  ['BOARD_NOT_FOUND', '指定的项目或白板不存在；或项目下有多块白板却只写了项目标识'],
  ['NOT_FOUND', '卡片不存在'],
  ['EMPTY_TITLE', '新建或编辑时标题为空'],
  ['EMPTY_TODO', '不带 ID 领取时，连到锁定状态的可领取状态里都没有卡片'],
  ['NOT_IN_TODO', '带 ID 领取的卡片不在可领取状态，且不是自己持有的卡片'],
  ['ALREADY_HOLDING', '该 agent 在这块白板上已经持有另一张卡片'],
  ['NOT_HOLDER', '编辑或移动被持有卡片的不是持有者'],
  ['FORBIDDEN_COLUMN', '把卡片直接移进锁定状态；agent 把卡片移入、移出终点状态或「未整理」；agent 不沿连线移动卡片；从没有连到锁定状态的可领取状态领取'],
  ['USAGE', '命令格式或参数不正确：未知的命令、未知的状态、项目里不存在的标签、缺少必需的参数、评论内容为空'],
  ['NOT_LOGGED_IN', '没有登录：界面没有登录会话；CLI 还没有 board login（环境变量和 ~/.board/config.toml 里都没有服务端地址或令牌）；用访问令牌做只有人能在界面上做的操作'],
  ['INVALID_TOKEN', '访问令牌不存在或已吊销'],
  ['UNREACHABLE', 'CLI 连不上服务端'],
  ['NAME_TAKEN', 'board check 的名字正被在线 agent 使用：加一个和本会话相关的后缀再 check'],
  ['NAME_OFFLINE', '名字不在线：没有 check，或心跳中断超过 50 秒，先 board check --as <名字>'],
  ['NOT_AUTHOR', '修改不是自己发的评论：只有人能在界面上修改自己发的评论'],
  ['HOST_NAME_TAKEN', 'board host 的机器名正被另一个在线的 board host 使用'],
  ['ALREADY_DISPATCHED', '卡片已经有排队或正在跑的派发'],
  ['NO_SUCH_HOST', '机器不存在（含已移除的机器）'],
  ['AGENT_NOT_ON_HOST', '机器上没有这种 agent，或标为没找到'],
  ['BOARD_NOT_ON_HOST', '机器没有这块白板的目录'],
  ['DISPATCHED', '卡片有排队中的派发：别的 agent 不能领取它'],
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
