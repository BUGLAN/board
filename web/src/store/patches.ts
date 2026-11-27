/* 乐观更新的纯函数补丁：每次刷新都会重新叠到新数据上，所以要能重复执行（结果不变），
   目标不在了原样返回。保存完之前不能再改同一个对象（busy），见 data.ts 的写队列。 */
import { FINAL_RECENT, recentIds } from '../lib/earlier';
import { removePrompt, updatePrompt, type Prompt } from '../lib/prompts';
import { boardFlow, INBOX } from '../lib/schema';
import type {
  BoardSnapshot,
  Card,
  CardCronInput,
  CardDetail,
  ColumnKey,
  Dispatch,
  Profile,
  ProjectInfo,
  Workspace,
} from '../lib/types';
import { get, serverNow } from './state';

export interface Patch {
  ws?: (ws: Workspace) => Workspace;
  /** 只作用于发起时所在的白板 */
  board?: (b: BoardSnapshot) => BoardSnapshot;
  detail?: (d: CardDetail) => CardDetail;
  /** 快捷提示词的整列（和白板无关，同 ws） */
  prompts?: (list: Prompt[]) => Prompt[];
  /** 画成「保存中」的卡片 */
  cards?: number[];
  /** 画成「保存中」的评论（动态 ID） */
  acts?: number[];
  /** 其他保存完之前不能再改的对象，全局唯一的字符串（hostKey、flowKey、labelKey、projectKey） */
  keys?: string[];
}

/** 对象的 busy 键：保存完之前不能再改 */
export const cardKey = (path: string, id: number) => `${path}#${id}`;
export const hostKey = (name: string) => `host:${name}`;
export const flowKey = (path: string) => `flow:${path}`;
export const labelKey = (project: string, id: number) => `label:${project}:${id}`;
export const projectKey = (project: string) => `project:${project}`;

/** 保存完之前不能再改的角色 */
export const profileKey = (id: string) => `profile:${id}`;

/** 一条定时建卡在保存完之前不能再改（编辑、删除、现在建一张）：改模板时补丁带上它；开关是设置项（saveSetting），不锁 */
export const cardCronKey = (path: string, id: number) => `cardCron:${path}:${id}`;

/** 隐藏的标签规则保存完之前不能再改（管理标签里的规则列表按它禁用） */
export const hiddenLabelsKey = (project: string) => `hiddenLabels:${project}`;

/** 快捷提示词整列的 busy 键 */
export const PROMPTS_BUSY = 'prompts';

const withCard = (b: BoardSnapshot, id: number, fn: (c: Card) => Card): BoardSnapshot => {
  const c = b.cards[id];
  return c ? { ...b, cards: { ...b.cards, [id]: fn(c) } } : b;
};

/** 改一张卡片：白板上的摘要和打开着的详情一起改。mark=false 时不画「保存中」（例如标签有自己的 pending 样式） */
export function editCard(id: number, change: Partial<Card> | ((c: Card) => Partial<Card>), mark = true): Patch {
  const ch = (c: Card) => (typeof change === 'function' ? change(c) : change);
  return {
    board: b => withCard(b, id, c => ({ ...c, ...ch(c) })),
    detail: d => (d.id === id ? { ...d, ...ch(d) } : d),
    cards: mark ? [id] : undefined,
  };
}

/** 一张卡片排队中的派发去掉（卡片被人移走、取消派发时服务端随之取消）。没有这一条时原样返回 */
export function unqueuePatch(board: string, card: number): Patch {
  const hit = (d: Dispatch) => d.board === board && d.card === card && d.state === 'queued';
  return { ws: ws => (ws.dispatches?.some(hit) ? { ...ws, dispatches: ws.dispatches.filter(d => !hit(d)) } : ws) };
}

/** 几层补丁合成一层：按顺序叠，「保存中」的卡片、评论、对象合在一起 */
export function joinPatch(...list: Patch[]): Patch {
  const chain = <T,>(fs: (((x: T) => T) | undefined)[]) => {
    const on = fs.filter((f): f is (x: T) => T => !!f);
    return on.length ? (x: T) => on.reduce((v, f) => f(v), x) : undefined;
  };
  const all = <T,>(xs: (T[] | undefined)[]) => (xs.some(Boolean) ? xs.flatMap(x => x ?? []) : undefined);
  return {
    ws: chain(list.map(p => p.ws)),
    board: chain(list.map(p => p.board)),
    detail: chain(list.map(p => p.detail)),
    prompts: chain(list.map(p => p.prompts)),
    cards: all(list.map(p => p.cards)),
    acts: all(list.map(p => p.acts)),
    keys: all(list.map(p => p.keys)),
  };
}

/** 派发排队的卡片（在它排队的锁定状态里，没有持有者）离开这里时，服务端取消这次派发：和服务端同一条规则——
    换了状态、并且不是在两个可领取状态之间移动。在建补丁时按当前白板判断 */
function leavesQueue(id: number, to: ColumnKey) {
  const { board, path, ws } = get();
  const c = board?.cards[id];
  if (!c || c.holder || c.column === to) return false;
  const f = boardFlow(board);
  if (f.get(c.column)?.claim && f.get(to)?.claim) return false;
  return !!ws?.dispatches?.some(d => d.board === path && d.card === id && d.state === 'queued');
}

/** 把卡片放到 to 的第 index 位（按去掉它之后的列表算；省略时放最后）。order：排序状态下拖动时整块的新顺序（与服务端规则一致：没列出的补在后面）。
    extra：一起改的其他字段（例如释放时清空持有者）。
    换了状态时移入时间、更新时间记成新的（「保留顺序」按移入时间、最近更新排时，确认之前就排到新移入的位置），见 freshAt。
    拖进只带了最近几张的终点状态时，最旧的那张当帧落进「更早的」，见 trimFinal。
    派发排队的卡片被移走时，排队的派发当帧一起去掉（服务端随之取消）。
    batch：同一批一起移动的卡片和发起的时刻（批量移动用 batchMovePatch） */
export function movePatch(id: number, to: ColumnKey, index?: number, order?: number[], extra: Partial<Card> = {}, batch?: { ids: number[]; at: number }): Patch {
  const at = batch?.at ?? serverNow();
  const skip = new Set(batch?.ids);
  const unqueue = leavesQueue(id, to);
  if (unqueue) extra = { claimedFrom: null, ...extra };
  const moved = <T extends Card>(c: T, t = { moved: at, updated: at }): T =>
    c.column === to ? { ...c, ...extra, column: to } : { ...c, ...extra, column: to, movedAt: t.moved, updatedAt: Math.max(c.updatedAt, t.updated) };
  return {
    ws: unqueue ? unqueuePatch(get().path, id).ws : undefined,
    board: b => {
      const c = b.cards[id];
      if (!c) return b;
      const cols: BoardSnapshot['cols'] = {};
      for (const [k, v] of Object.entries(b.cols)) cols[k] = v.filter(x => x !== id);
      const list = cols[to] ?? [];
      list.splice(Math.min(index ?? list.length, list.length), 0, id);
      cols[to] = order ? [...new Set([...order.filter(x => list.includes(x)), ...list])] : list;
      const next = { ...b, cols, cards: { ...b.cards, [id]: moved(c, freshAt(b, at, skip)) } };
      return c.column === to ? next : trimFinal(next, to);
    },
    detail: d => (d.id === id ? moved(d) : d),
    cards: [id],
  };
}

/** 批量移动（batchCards 的 patchOf）：每张卡片一份 movePatch，同一批的移入时间一样（见 freshAt） */
export function batchMovePatch(ids: number[], to: ColumnKey): (id: number) => Patch {
  const batch = { ids, at: serverNow() };
  return id => movePatch(id, to, undefined, undefined, {}, batch);
}

/** 换状态时补丁里的移入时间、更新时间：发起的时刻，但至少比快照里所有卡片都新（同一快照重复执行结果不变）。
    服务端在处理请求的那一刻写这两个时间（engine 的 place、track），比快照里已有的都晚；补丁的发起时刻在慢网下可能更早：
    serverNow 按收到响应时校的时钟，少算了一段路上的时间，写请求还要在队列里等前面的确认。
    连着拖两张进按移入时间排的块，第二张的发起时刻比第一张确认后的移入时间还早：第一张确认时两张换位，第二张确认时又换回来，
    看上去像又有卡片拖了进来。取「比快照里最新的还新」后，没确认的卡片排在哪，确认后就还在哪。
    同一批一起移动的（skip）不算：服务端在一个事务里写，时间一样（一样时按优先级顺序排），补丁里也要一样 */
function freshAt(b: BoardSnapshot, at: number, skip: Set<number>): { moved: number; updated: number } {
  let moved = 0;
  let updated = 0;
  for (const c of Object.values(b.cards)) {
    if (skip.has(c.id)) continue;
    moved = Math.max(moved, c.movedAt || c.createdAt);
    updated = Math.max(updated, c.updatedAt);
  }
  return { moved: Math.max(at, moved + 1), updated: Math.max(at, updated + 1) };
}

/** 终点状态的快照只带最近移入的 FINAL_RECENT 张（omitted 记着省略的张数）：拖进一张后服务端的快照里最旧的那张落进「更早的」。
    补丁当帧照做（按同一个规则 recentIds 选留下的），不然慢网下要等确认（松手几秒后）它才在块的末尾淡出。
    快照没有省略（取过全量）时由界面按同一个规则截断（lib/earlier 的 collapseCols），这里不动 */
function trimFinal(b: BoardSnapshot, col: ColumnKey): BoardSnapshot {
  const omitted = b.omitted?.[col] ?? 0;
  const ids = b.cols[col] ?? [];
  if (!omitted || ids.length <= FINAL_RECENT) return b;
  const keep = recentIds(b.cards, ids);
  const out = ids.filter(x => !keep.has(x));
  const cards = { ...b.cards };
  for (const x of out) delete cards[x];
  return { ...b, cols: { ...b.cols, [col]: ids.filter(x => keep.has(x)) }, cards, omitted: { ...b.omitted, [col]: omitted + out.length } };
}

/** 取消排队的派发：卡片还在它排队的锁定状态里（当前白板上、没有持有者）时，当帧回到原来的可领取状态最上面（和服务端一致）；
    排队的派发一起去掉 */
export function unqueueCard(board: string, card: number): Patch {
  const { board: b, path } = get();
  const c = path === board ? b?.cards[card] : undefined;
  const f = boardFlow(b);
  const back = c && !c.holder && f.isLock(c.column) ? (c.claimedFrom && f.get(c.claimedFrom)?.claim ? c.claimedFrom : f.home?.key) : undefined;
  return back ? joinPatch(unqueuePatch(board, card), movePatch(card, back, 0, undefined, { claimedFrom: null })) : unqueuePatch(board, card);
}

/** 从白板上拿掉一张卡片（删除：移进回收站）。回收站里的张数跟着加一；卡片的列换个新数组（FLIP 用得上） */
export function removeCard(id: number): Patch {
  return {
    board: b => {
      const c = b.cards[id];
      if (!c) return b;
      const cards = { ...b.cards };
      delete cards[id];
      const cols: BoardSnapshot['cols'] = { ...b.cols, [c.column]: (b.cols[c.column] ?? []).filter(x => x !== id) };
      return { ...b, cols, cards, trash: (b.trash ?? 0) + 1 };
    },
  };
}

/** 从回收站恢复一张卡片：当帧回到它的位置（那一列的第 pos 位，由 lib/trash.ts 决定），回收站里少一张。
    恢复时卡片还不在快照里，column、pos、card 在把卡片放上去的那一刻写定，之后每次重新叠加用的都是这一份。
    已经在白板上的原样返回（重复执行结果不变）。detail 是打开着的这张卡片的详情：内容留着，回到白板上 */
export function restoreCard(plan: { id: number; column: ColumnKey; pos: number; movedAt: number; card: Card }): Patch {
  const { id, column, pos, movedAt: at, card } = plan;
  return {
    board: b => {
      if (b.cards[id]) return b;
      const cols: BoardSnapshot['cols'] = {};
      for (const [k, v] of Object.entries(b.cols)) cols[k] = v.filter(x => x !== id);
      const list = [...(cols[column] ?? [])];
      list.splice(Math.min(Math.max(0, pos), list.length), 0, id);
      cols[column] = list;
      return { ...b, cols, cards: { ...b.cards, [id]: { ...card, column, movedAt: at } }, trash: Math.max(0, (b.trash ?? 0) - 1) };
    },
    detail: d => (d.id === id ? { ...d, column } : d),
  };
}

/** 合并「未整理」里的条目：留下 keep（标题、描述换成合并后的，标签取并集），from 里的从白板上拿掉。
    keep 已经不在：原样返回（来源也不动）；已经拿掉的来源跳过，重复执行结果不变 */
export function mergePatch(keep: number, from: number[], change: { title: string; body: string }): Patch {
  const others = from.filter(id => id !== keep);
  return {
    board: b => {
      const k = b.cards[keep];
      if (!k) return b;
      const gone = others.filter(id => b.cards[id]);
      const labels = [...new Set([...k.labels, ...gone.flatMap(id => b.cards[id].labels)])];
      const cards: BoardSnapshot['cards'] = { ...b.cards, [keep]: { ...k, ...change, labels } };
      gone.forEach(id => delete cards[id]);
      const cols: BoardSnapshot['cols'] = {};
      for (const [c, v] of Object.entries(b.cols)) cols[c] = gone.length ? v.filter(x => !gone.includes(x)) : v;
      return { ...b, cols, cards };
    },
    detail: d => (d.id === keep ? { ...d, ...change } : d),
    cards: [keep, ...others],
  };
}

/** 删除一条评论（人删自己发的）：动态里的这条当帧消失，卡片上的评论数减一；这条评论已经不在了原样返回 */
export function removeComment(card: number, act: number): Patch {
  return {
    detail: d =>
      d.id !== card || !d.activity.some(x => x.id === act)
        ? d
        : { ...d, activity: d.activity.filter(x => x.id !== act), comments: Math.max(0, d.comments - 1) },
  };
}

let tempSeq = 0;
/** 已经认作某张临时卡片的真卡片：一张真卡片只顶替一张临时卡片 */
const claimed = new Set<number>();
/** 真卡片 ID → 它顶替的临时卡片 ID */
const replaced = new Map<number, number>();
/** 卡片在界面上的身份（React key、FLIP、入场动画）：顶替了临时卡片的真卡片沿用临时卡片的，
    换成真卡片时是同一个元素原地更新（编号从「…」变成真实 ID），不卸载重建、不重播入场动画 */
export const renderKey = (id: number) => replaced.get(id) ?? id;
/** 新建卡片：先用一个负数 ID 的临时卡片画在目标状态最下面（服务端的规则），刷新拿到真卡片的那一刻临时卡片不再画，两张不会同时出现。
    真卡片怎么认：服务端返回后由 settle 记下 ID；返回之前，推送触发的刷新可能已经带来了真卡片（弱网下常见），
    这时按「新建时白板上还没有的、人建的、标题和状态都一样」认出来 */
export function draftCard(p: { title: string; body: string; labels: number[]; column: ColumnKey }) {
  const id = -++tempSeq;
  let real: number | null = null;
  const now = serverNow();
  const board = get().board;
  const known = Math.max(0, ...Object.keys(board?.cards ?? {}).map(Number));
  const card: Card = {
    id, title: p.title.trim(), body: p.body.trim(), column: p.column, holder: null, claimedAt: null, beatAt: null, runStartedAt: null,
    workMs: 0, runs: 0, runsLost: 0, claimedFrom: null, labels: p.labels, createdAt: now, updatedAt: now,
    createdBy: { kind: 'human', name: '' }, comments: 0,
  };
  const patch: Patch = {
    board: b => {
      if (real == null) {
        const hit = Object.values(b.cards).find(
          x => x.id > known && !claimed.has(x.id) && x.createdBy.kind === 'human' && x.title === card.title && x.column === p.column,
        );
        if (hit) {
          real = hit.id;
          claimed.add(hit.id);
          replaced.set(hit.id, id);
        }
      }
      if (real != null && b.cards[real]) return b;
      return { ...b, cards: { ...b.cards, [id]: card }, cols: { ...b.cols, [p.column]: [...(b.cols[p.column] ?? []), id] } };
    },
    cards: [id],
  };
  const settle = (realId: number) => {
    if (real != null && real !== realId) {
      claimed.delete(real);
      if (replaced.get(real) === id) replaced.delete(real);
    }
    real = realId;
    claimed.add(realId);
    replaced.set(realId, id);
  };
  return { id, patch, settle };
}

/** 临时卡片（还在新建）：没有详情、不能拖、不能打开 */
export const isDraft = (id: number) => id < 0;

/** 改一个项目（名称、颜色、标签……） */
export const patchProject = (key: string, fn: (p: ProjectInfo) => ProjectInfo): Patch => ({
  ws: ws => ({ ...ws, projects: ws.projects.map(p => (p.key === key ? fn(p) : p)) }),
});

/** 白板参不参与自动领取（boards.auto）：白板设置的开关、Agent → 领取规则里的白板胶囊共用；白板不在了原样返回 */
export const boardAutoPatch = (path: string, on: boolean): Patch => ({
  ws: ws => {
    if (!ws.projects.some(p => p.boards.some(b => b.path === path))) return ws;
    return { ...ws, projects: ws.projects.map(p => ({ ...p, boards: p.boards.map(b => (b.path === path ? { ...b, auto: on } : b)) })) };
  },
});

/** 改一个角色（分工页、角色编辑框）：按标识换掉；角色不在了原样返回 */
export const editProfilePatch = (p: Profile): Patch => ({
  keys: [profileKey(p.id)],
  ws: ws => {
    const s = ws.sched;
    if (!s?.profiles?.some(x => x.id === p.id)) return ws;
    return { ...ws, sched: { ...s, profiles: s.profiles.map(x => (x.id === p.id ? { ...x, ...p } : x)) } };
  },
});

/** 删掉一个角色：交给它的规则一起拿掉（和服务端一样）；角色不在了原样返回 */
export const deleteProfilePatch = (id: string): Patch => ({
  keys: [profileKey(id)],
  ws: ws => {
    const s = ws.sched;
    if (!s?.profiles?.some(x => x.id === id)) return ws;
    return { ...ws, sched: { ...s, profiles: s.profiles.filter(x => x.id !== id), rules: s.rules.filter(r => r.profile !== id) } };
  },
});

/** 改项目的代码仓库地址和集成分支（PRD §4，一个项目一个仓库）：项目、它下面每块白板、当前白板（在这个项目里时）的快照一起改；
 *  项目不在了原样返回 */
export const repoPatch = (pk: string, change: { repo?: string; branch?: string }): Patch => ({
  ws: ws => {
    if (!ws.projects.some(p => p.key === pk)) return ws;
    return {
      ...ws,
      projects: ws.projects.map(p =>
        p.key === pk
          ? {
              ...p,
              repo: p.repo ?? '',
              branch: p.branch ?? '',
              ...change,
              boards: p.boards.map(b => ({ ...b, repo: b.repo ?? '', branch: b.branch ?? '', ...change })),
            }
          : p,
      ),
    };
  },
  board: b => (b.path.startsWith(`${pk}/`) ? { ...b, repo: b.repo ?? '', branch: b.branch ?? '', ...change } : b),
});

/** 改白板的一条定时建卡（PRD §4）：开关、模板、派给机器（指定一台 / 随机）、时间。下次时间等服务端算，改了时间就先清掉；
 *  机器、agent 发空字符串表示不派发 / 默认 agent，画成空；选了随机就没有指定的机器；
 *  名称就是建卡标题，服务端把它同时写回 `title`（旧版客户端读它），改名时 `title` 一起改；
 *  白板或这条不在了原样返回 */
export const cardCronPatch = (path: string, id: number, input: CardCronInput): Patch => ({
  ws: ws => {
    const has = ws.projects.some(p => p.boards.some(b => b.path === path && b.cardCrons?.some(c => c.id === id)));
    if (!has) return ws;
    const random = input.pick === 'random';
    const change = {
      ...input,
      ...(random || input.host !== undefined ? { host: random ? null : input.host || null } : {}),
      ...(input.profile !== undefined ? { profile: input.profile || null, agent: null } : {}),
      // 不派发：角色不留
      ...(!random && input.host === '' ? { profile: null } : {}),
      ...(input.name !== undefined ? { title: input.name.trim() } : {}),
    };
    const stale = change.cron !== undefined || change.tz !== undefined || change.on === false;
    return {
      ...ws,
      projects: ws.projects.map(p => ({
        ...p,
        boards: p.boards.map(b =>
          b.path === path ? { ...b, cardCrons: b.cardCrons?.map(c => (c.id === id ? { ...c, ...change, ...(stale ? { nextAt: null } : {}) } : c)) } : b,
        ),
      })),
    };
  },
});

/** 改项目里的一个标签（名称、颜色、类型、是否优先） */
export const editLabelPatch = (project: string, id: number, change: { name?: string; color?: string; humanOnly?: boolean; priority?: boolean }): Patch => ({
  ...patchProject(project, p => ({ ...p, labels: p.labels.map(l => (l.id === id ? { ...l, ...change } : l)) })),
  keys: [labelKey(project, id)],
});

/** 改项目的隐藏标签规则（整组替换）：侧栏、提示词菜单、标签选择器当帧按新规则显示；项目不在了原样返回 */
export const hiddenLabelsPatch = (project: string, rules: string[]): Patch => ({
  ...patchProject(project, p => ({ ...p, hiddenLabels: rules })),
  keys: [hiddenLabelsKey(project)],
});

/** 删除项目里的一个标签：从项目和当前白板的卡片上一起拿掉 */
export const deleteLabelPatch = (project: string, id: number): Patch => ({
  ...patchProject(project, p => ({ ...p, labels: p.labels.filter(l => l.id !== id) })),
  keys: [labelKey(project, id)],
  board: b => {
    if (!b.path.startsWith(`${project}/`)) return b;
    let cards = b.cards;
    for (const [k, c] of Object.entries(b.cards))
      if (c.labels.includes(id)) {
        if (cards === b.cards) cards = { ...cards };
        cards[k] = { ...c, labels: c.labels.filter(l => l !== id) };
      }
    return cards === b.cards ? b : { ...b, cards };
  },
});

/** 「未整理」里的条目标成看过（PRD §10.5）：点当帧消失，记下看过的时刻。
    目标不在了、已经不在「未整理」里、已经看过：原样返回；时刻在建补丁时取定，重复执行结果不变 */
export function seenPatch(ids: number[]): Patch {
  const at = serverNow();
  const mark = <T extends Card>(c: T): T => (c.column === INBOX && !c.seenAt ? { ...c, seenAt: at } : c);
  return {
    board: b => {
      const hit = ids.filter(id => b.cards[id]);
      if (!hit.length) return b;
      const cards = { ...b.cards };
      hit.forEach(id => (cards[id] = mark(cards[id])));
      return { ...b, cards };
    },
    detail: d => (ids.includes(d.id) ? mark(d) : d),
  };
}

/** 「稍后再做」（default/main#303）：把「未整理」里的条目收起（on）或放回。当帧从「未整理」的列表里消失 / 回来，
    位置、标签都不动。目标不在了、不在「未整理」里、已经是这样：原样返回；时刻在建补丁时取定，重复执行结果不变 */
export function laterPatch(ids: number[], on: boolean): Patch {
  const at = serverNow();
  const mark = <T extends Card>(c: T): T => (c.column !== INBOX || !!c.laterAt === on ? c : { ...c, laterAt: on ? at : null });
  return {
    board: b => {
      const hit = ids.filter(id => b.cards[id] && mark(b.cards[id]) !== b.cards[id]);
      if (!hit.length) return b;
      const cards = { ...b.cards };
      hit.forEach(id => (cards[id] = mark(cards[id])));
      return { ...b, cards };
    },
    detail: d => (ids.includes(d.id) ? mark(d) : d),
  };
}

/** 新建一条（p 由 lib/prompts 的 makePrompt 生成，id 在建补丁时定好）：放在最前面；已经有这个 id 时原样返回 */
export function addPromptPatch(p: Prompt): Patch {
  return { prompts: list => (list.some(x => x.id === p.id) ? list : [p, ...list]), keys: [PROMPTS_BUSY] };
}

/** 改一条的标题、内容；不在了原样返回 */
export function editPromptPatch(id: string, title: string, text: string): Patch {
  return { prompts: list => (list.some(x => x.id === id) ? updatePrompt(list, id, title, text) : list), keys: [PROMPTS_BUSY] };
}

/** 删一条；不在了原样返回 */
export function removePromptPatch(id: string): Patch {
  return { prompts: list => (list.some(x => x.id === id) ? removePrompt(list, id) : list), keys: [PROMPTS_BUSY] };
}

/** 整列换成 list（把本机旧存的搬上去时用）；内容一样时原样返回 */
export function replacePromptsPatch(next: Prompt[]): Patch {
  return { prompts: list => (JSON.stringify(list) === JSON.stringify(next) ? list : next), keys: [PROMPTS_BUSY] };
}

/** 加标签的补丁：这几张卡片画上这个标签（已经有的原样返回）。去标签不改数据，只画 pending（同 removeLabel） */
export function labelAddPatch(ids: number[], labelId: number): Patch {
  const on = new Set(ids);
  const plus = <T extends { labels: number[] }>(c: T): T => (c.labels.includes(labelId) ? c : { ...c, labels: [...c.labels, labelId] });
  return {
    board: b => ids.reduce((x, id) => withCard(x, id, plus), b),
    detail: d => (on.has(d.id) ? plus(d) : d),
  };
}

/** 机器对话里发一条消息：这个对话当帧变成「回答中」、排到最前（可以重复执行，对话不在了原样返回） */
export const chatPromptPatch = (id: number, at: number): Patch => ({
  ws: ws =>
    ws.chats?.some(c => c.id === id) ? { ...ws, chats: ws.chats.map(c => (c.id === id ? { ...c, turn: 'busy' as const, lastAt: Math.max(c.lastAt, at) } : c)) } : ws,
  keys: [`chat:${id}`],
});
