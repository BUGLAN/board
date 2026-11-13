/* ==========================================================================
   数据：工作区、当前白板（每个窗口可以看不同的白板）、打开的卡片详情
   - 服务端推送 rev 变化 → 重新取快照；新事件交给界面做高亮与提示
   - 写操作乐观更新（见「乐观更新」一节）：当帧生效，按发起顺序一个接一个发给服务端，
     成功后刷新再撤掉本地补丁，失败只撤掉自己那一层；保存完之前同一个对象不能再改（见 busy）
   - 当前白板、打开的卡片都在地址栏里（lib/route.ts）：打开、关闭详情和切换白板时写进去
   ========================================================================== */
import { create } from 'zustand';
import { api, ApiError, connect, CLIENT_ID, type LiveStatus } from '../lib/api';
import { applySilently, bindRoute, parseRoute, syncUrl } from '../lib/route';
import * as flip from '../lib/flip';
import { clearLabelSel, setTab, toast, useUI, type Tab } from './ui';
import { boardFlow, INBOX } from '../lib/schema';
import { isNewEntry, type BoardEvent, type BoardInfo, type BoardSnapshot, type Card, type CardDetail, type Cmd, type ColumnKey, type Dispatch, type ProjectInfo, type Workspace } from '../lib/types';

interface DataState {
  ws: Workspace | null;
  path: string;
  board: BoardSnapshot | null;
  detailId: number | null;
  detail: CardDetail | null;
  status: LiveStatus;
  rev: number;
  /** 服务端时钟 - 本地时钟 */
  offset: number;
  /** 切换白板时 +1：卡片整体重建，不做位置补间 */
  generation: number;
  error: string | null;
  /** 详情没取回来的原因（取回或换一张卡片时清空）；有值时详情里不再显示骨架 */
  detailError: string | null;
  /** 正在写入的标签（乐观更新）：「白板路径#卡片」→ 标签 ID。正在加上的已经并进 board 里卡片的 labels，正在去掉的还留在里面；界面都画成 pending 态 */
  pending: Record<string, number[]>;
  /** 正在发送的评论（乐观更新）：「白板路径#卡片」→ 按发送顺序。动态底部画成「发送中」的气泡，服务端确认后换成正式评论 */
  sending: Record<string, Sending[]>;
  /** 还没确认的写操作涉及的卡片（「白板路径#卡片」）、评论（「白板路径@动态 ID」）和其他对象（Patch.keys，例如「host:mbp」）
      → 进行中的个数。界面据此画成「保存中」，并禁用对它们的操作（见 busy） */
  saving: Record<string, number>;
}

export interface Sending {
  key: number;
  text: string;
  at: number;
}

const CURRENT = 'board.current';
const saved = () => {
  try {
    return sessionStorage.getItem(CURRENT) || localStorage.getItem(CURRENT) || '';
  } catch {
    return '';
  }
};
const save = (p: string) => {
  try {
    sessionStorage.setItem(CURRENT, p);
    localStorage.setItem(CURRENT, p);
  } catch {
    /* 忽略 */
  }
};

/** 启动时的地址：/<项目>/<白板>[/<ID>]，没有时用本地记住的白板 */
const initial = typeof location !== 'undefined' ? parseRoute(location.pathname, location.search) : { path: null, card: null, tab: null };
/** 地址栏里的卡片：第一次加载时和列表一起取回来再打开（见 load）；之后由 applyRoute 直接打开 */
let pendingCard: number | null = initial.card;

export const useData = create<DataState>(() => ({
  ws: null,
  path: initial.path ?? saved(),
  board: null,
  detailId: null,
  detail: null,
  status: 'connecting',
  rev: 0,
  offset: 0,
  generation: 0,
  error: null,
  pending: {},
  sending: {},
  saving: {},
  detailError: null,
}));

const set = useData.setState;
const get = useData.getState;
// 地址栏跟着 store 里的白板、卡片和 Tab 走（lib/route 的 syncUrl 读它）
bindRoute(() => ({ path: get().path, card: get().detailId, tab: useUI.getState().tab }));

/* ------------------------------ 乐观更新 ------------------------------ */
/*
  界面上的 ws / board / detail = 服务端数据（base）依次叠上还没确认的写操作的补丁（layers）：
  - refresh 只替换 base，补丁重新叠上去：推送触发的刷新不会把还没确认的修改冲回旧状态
  - 成功：等 refresh 拿到包含这次修改的数据，再撤掉补丁，界面不闪
  - 失败：只撤掉自己这一层，之后发起的操作照样叠着（不会被回滚覆盖）
  补丁是纯函数，每次 render 都重新执行，要容忍目标已经不在（卡片被删、换了白板），并且可以重复执行。
  保存完之前同一个对象（补丁的 cards / keys）不能再改：界面上禁用对它的操作，漏网的由 guard 拒绝（BUSY，不发请求）。
  这样同一个对象上不会有两笔还没确认的修改，不会出现「前一笔失败、后一笔是在它的基础上算的」这类结果和预期不符。
*/
export interface Patch {
  ws?: (ws: Workspace) => Workspace;
  /** 只作用于发起时所在的白板 */
  board?: (b: BoardSnapshot) => BoardSnapshot;
  detail?: (d: CardDetail) => CardDetail;
  /** 画成「保存中」的卡片 */
  cards?: number[];
  /** 画成「保存中」的评论（动态 ID） */
  acts?: number[];
  /** 其他保存完之前不能再改的对象，全局唯一的字符串（hostKey、flowKey、labelKey、projectKey） */
  keys?: string[];
}
interface Layer extends Patch {
  path: string;
}

const base: { ws: Workspace | null; board: BoardSnapshot | null; detail: CardDetail | null } = { ws: null, board: null, detail: null };
let layers: Layer[] = [];

/** 按 base + layers 重新算出界面数据；extra 一起写进 store（同一次更新） */
function render(extra: Partial<DataState> = {}) {
  const path = extra.path ?? get().path;
  let { ws, board, detail } = base;
  const saving: Record<string, number> = {};
  const mark = (k: string) => (saving[k] = (saving[k] ?? 0) + 1);
  for (const l of layers) {
    if (l.ws && ws) ws = l.ws(ws);
    l.cards?.forEach(id => mark(`${l.path}#${id}`));
    l.acts?.forEach(id => mark(`${l.path}@${id}`));
    l.keys?.forEach(mark);
    if (l.path !== path) continue;
    if (l.board && board?.path === path) board = l.board(board);
    if (l.detail && detail) detail = l.detail(detail);
  }
  set({ ws, board, detail, saving, ...extra });
}

/** 对象的 busy 键：保存完之前不能再改 */
export const cardKey = (path: string, id: number) => `${path}#${id}`;
export const hostKey = (name: string) => `host:${name}`;
export const flowKey = (path: string) => `flow:${path}`;
export const labelKey = (project: string, id: number) => `label:${project}:${id}`;
export const projectKey = (project: string) => `project:${project}`;

/** 这个对象有还没确认的修改（画成「保存中」）：界面上禁用对它的操作 */
export const busy = (key: string) => !!get().saving[key];
export const useBusy = (key: string) => useData(s => !!s.saving[key]);
/** 当前白板上的这张卡片在保存中 */
export const cardBusy = (id: number) => busy(cardKey(get().path, id));
export const useCardBusy = (id: number) => useData(s => !!s.saving[cardKey(s.path, id)]);

/** 补丁涉及的对象还有没确认的修改时拒绝（不叠补丁、不发请求），由调用方提示 */
function guard(p: Patch) {
  const path = get().path;
  const keys = [...(p.cards ?? []).map(id => cardKey(path, id)), ...(p.keys ?? [])];
  if (keys.some(busy)) throw new ApiError('BUSY', '上一次修改还在保存，保存完再改');
}

/** 每发起一个写请求 +1：发出时和返回时不一样的读请求（机器列表、卡片详情）可能是写之前的旧数据，不用它覆盖 */
let writes = 0;

function push(p: Patch): Layer {
  const l: Layer = { ...p, path: get().path };
  layers = [...layers, l];
  render();
  return l;
}
function drop(l: Layer) {
  if (!layers.includes(l)) return;
  layers = layers.filter(x => x !== l);
  render();
}

/** 写请求按发起顺序一个接一个发：连续操作时服务端收到的顺序与界面一致（补丁当帧就叠上，不用等前面的）。
    请求有超时（lib/api）：网络断了时一个挂住的请求不会让后面的一直排队 */
let chain: Promise<unknown> = Promise.resolve();
function queue<T>(req: () => Promise<T>): Promise<T> {
  writes++;
  const run = chain.then(req);
  chain = run.catch(() => undefined);
  // 超时、断线：那次写可能已经生效了，补丁撤掉后按服务端的数据再画一次
  run.catch(e => {
    if (e instanceof ApiError && e.code === 'NETWORK') void refresh();
  });
  return run;
}

/** 乐观执行一个写请求：补丁当帧生效；成功后等「响应之后才开始的第一轮加载」完成（见 afterWrite），再撤掉补丁；
    失败撤掉补丁并抛出错误（由调用方提示）。不等排在后面的写操作和它们的刷新（#174）。
    patch 为 null 时只是排队执行 + 刷新（拿不到结果就没法预先画出来的操作，例如新建项目），调用方自己显示进行中 */
export async function optimistic<T>(patch: Patch | null, req: () => Promise<T>): Promise<T> {
  if (patch) guard(patch);
  const l = patch ? push(patch) : null;
  try {
    const r = await queue(req);
    await afterWrite();
    return r;
  } finally {
    if (l) drop(l);
  }
}

/** 叠上补丁，直到 done 结束再撤掉（done 成功时应已刷新过）。用于先发请求、稍后才确定本地效果的操作（拖拽松手的飞回动画结束后才落位） */
export function hold<T>(patch: Patch, done: Promise<T>): Promise<T> {
  const l = push(patch);
  // 撤补丁时卡片可能换位置（服务端的顺序和补丁不同、被拒绝回到原处）：先记下位置，由 FLIP 补间，不瞬移
  const off = () => {
    if (layers.includes(l)) flip.capture();
    drop(l);
  };
  done.then(off, off);
  return done;
}

/* 设置项：不上锁的乐观保存。
   设置项的补丁写的是字段的最终值（最多几个会话 = 3），不依赖前一笔的结果：前一笔还没确认也可以接着改，
   不走 guard、不禁用别的控件（服务端慢的时候，整块设置不会跟着一起变灰）。
   - 补丁当帧叠上，并带着 key 记进 saving（关窗口时有离开确认）
   - 同一个 key 在 delay 毫秒内连着改（步进器连点）：合并成一个请求，发最后一次的值
   - 请求不进全局的写队列：同一个 key 的一个接一个发（服务端最后收到的是最后一次的值），不同 key 的同时发，
     互相不用等（各改各的字段，谁先到都一样）
   - 请求一返回 Promise 就结束（界面上的「保存中」到此为止）；补丁留到随后的刷新完成再撤，值不会闪回旧的
   - 失败：这一批的补丁一起撤掉（值回到这一批之前），Promise 抛出错误，由调用方提示
   同一批的调用拿到的是同一个 Promise：调用方按它去重，不要每次调用都提示一遍 */
interface SettingBatch {
  layers: Layer[];
  req: () => Promise<unknown>;
  timer: number;
  promise: Promise<unknown>;
  resolve: (r: unknown) => void;
  reject: (e: unknown) => void;
}
const settings = new Map<string, SettingBatch>();
/** 每个 key 自己的请求链 */
const settingChains = new Map<string, Promise<unknown>>();
function queueSetting(key: string, req: () => Promise<unknown>): Promise<unknown> {
  writes++;
  const run = (settingChains.get(key) ?? Promise.resolve()).then(req);
  const tail = run.catch(() => undefined);
  settingChains.set(key, tail);
  void tail.then(() => {
    if (settingChains.get(key) === tail) settingChains.delete(key);
  });
  // 超时、断线：那次写可能已经生效了，补丁撤掉后按服务端的数据再画一次
  run.catch(e => {
    if (e instanceof ApiError && e.code === 'NETWORK') void refresh();
  });
  return run;
}
export function saveSetting<T>(key: string, patch: Patch, req: () => Promise<T>, delay = 0): Promise<T> {
  const l = push({ ...patch, keys: [...(patch.keys ?? []), key] });
  let b = settings.get(key);
  if (b) {
    clearTimeout(b.timer);
    b.layers.push(l);
    b.req = req;
  } else {
    let resolve!: (r: unknown) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<unknown>((ok, no) => {
      resolve = ok;
      reject = no;
    });
    b = { layers: [l], req, timer: 0, promise, resolve, reject };
    settings.set(key, b);
  }
  const batch = b;
  const send = () => {
    settings.delete(key);
    const off = () => batch.layers.forEach(drop);
    queueSetting(key, () => batch.req()).then(
      r => {
        batch.resolve(r);
        void refresh().then(off, off);
      },
      e => {
        off();
        batch.reject(e);
      },
    );
  };
  if (delay > 0) batch.timer = window.setTimeout(send, delay);
  else send();
  return batch.promise as Promise<T>;
}

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
    换了状态时移入时间记成发起的时刻（「保留顺序」按移入时间排时，确认之前就排到新移入的位置）；时刻在建补丁时取定，重复执行结果不变。
    派发排队的卡片被移走时，排队的派发当帧一起去掉（服务端随之取消） */
export function movePatch(id: number, to: ColumnKey, index?: number, order?: number[], extra: Partial<Card> = {}): Patch {
  const at = serverNow();
  const unqueue = leavesQueue(id, to);
  if (unqueue) extra = { claimedFrom: null, ...extra };
  const moved = <T extends Card>(c: T): T => ({ ...c, ...extra, movedAt: c.column === to ? c.movedAt : at, column: to });
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
      return { ...b, cols, cards: { ...b.cards, [id]: moved(c) } };
    },
    detail: d => (d.id === id ? moved(d) : d),
    cards: [id],
  };
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

/** 改白板的环境检查命令（所有系统 / Windows）：白板不在了原样返回 */
export const readyCheckPatch = (path: string, change: { cmd?: string; win?: string }): Patch => ({
  ws: ws => {
    if (!ws.projects.some(p => p.boards.some(b => b.path === path))) return ws;
    return {
      ...ws,
      projects: ws.projects.map(p =>
        p.boards.some(b => b.path === path)
          ? { ...p, boards: p.boards.map(b => (b.path === path ? { ...b, readyCheck: { cmd: '', win: '', ...b.readyCheck, ...change } } : b)) }
          : p,
      ),
    };
  },
});

/** 改白板的代码仓库地址和集成分支（PRD §4）：工作区、当前白板的快照一起改；白板不在了原样返回 */
export const repoPatch = (path: string, change: { repo?: string; branch?: string }): Patch => ({
  ws: ws => {
    if (!ws.projects.some(p => p.boards.some(b => b.path === path))) return ws;
    return {
      ...ws,
      projects: ws.projects.map(p => ({
        ...p,
        boards: p.boards.map(b => (b.path === path ? { ...b, repo: b.repo ?? '', branch: b.branch ?? '', ...change } : b)),
      })),
    };
  },
  board: b => (b.path === path ? { ...b, repo: b.repo ?? '', branch: b.branch ?? '', ...change } : b),
});

/** 改项目里的一个标签（名称、颜色、类型、是否优先） */
export const editLabelPatch = (project: string, id: number, change: { name?: string; color?: string; humanOnly?: boolean; priority?: boolean }): Patch => ({
  ...patchProject(project, p => ({ ...p, labels: p.labels.map(l => (l.id === id ? { ...l, ...change } : l)) })),
  keys: [labelKey(project, id)],
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

/* ------------------------------ 「未整理」的新条目 ------------------------------ */

/** 已经标过「看过」的条目（补丁生效到服务端确认之间）：点当帧消失，别的刷新叠不上补丁时也不会闪回来 */
const seenIds = new Set<string>();

/** 这条算不算「新」：agent 记下、人还没看过；本窗口刚标过的当帧就不算了 */
export const entryIsNew = (path: string, c: Card) => isNewEntry(c) && !seenIds.has(cardKey(path, c.id));

/** 「未整理」里有多少条新的。
    sent：本窗口刚标过「全部已读」、服务端快照还没回来时的时刻（serverNow），那时已经记下的条目都不再算新，
    标完之后记下的照常算 */
export function newEntryCount(path: string, b: BoardSnapshot | null, sent: number | null): number {
  if (!b) return 0;
  return (b.cols[INBOX] ?? []).map(id => b.cards[id]).filter(c => c && entryIsNew(path, c) && (sent == null || c.createdAt > sent)).length;
}

/** 人看过一条（展开了这一行）：服务端记下看过的时刻，同步到所有窗口。当帧把点去掉；被拒绝时（例如已经在「未整理」之外）只是回到原样，不提示 */
export async function markSeen(id: number): Promise<void> {
  const path = get().path;
  const c = get().board?.cards[id];
  if (!c || !entryIsNew(path, c)) return;
  const k = cardKey(path, id);
  seenIds.add(k);
  render();
  try {
    await optimistic(seenPatch([id]), () => api.cmd(path, { op: 'seen', id }));
  } catch {
    /* 下次刷新按服务端的为准 */
  } finally {
    seenIds.delete(k);
    render();
  }
}

/** 「全部标为已读」：当帧去掉所有的点，接着记下的条目照常算新；已经看过的不再请求。返回标了几条 */
export async function markAllSeen(): Promise<number> {
  const path = get().path;
  const b = get().board;
  if (!b) return 0;
  const ids = (b.cols[INBOX] ?? []).map(id => b.cards[id]).filter(c => c && entryIsNew(path, c)).map(c => c.id);
  ids.forEach(id => seenIds.add(cardKey(path, id)));
  render();
  try {
    if (ids.length) await optimistic(seenPatch(ids), () => api.cmd(path, { op: 'seen' }));
    return ids.length;
  } finally {
    ids.forEach(id => seenIds.delete(cardKey(path, id)));
    render();
  }
}

/* ------------------------------ 选择器 ------------------------------ */

export const allBoards = (ws: Workspace | null): (BoardInfo & { project: ProjectInfo })[] =>
  ws ? ws.projects.flatMap(p => p.boards.map(b => ({ ...b, project: p }))) : [];

export function boardMeta(ws: Workspace | null, path: string) {
  if (!ws) return null;
  const [pk] = path.split('/');
  const p = ws.projects.find(x => x.key === pk);
  const b = p?.boards.find(x => x.path === path);
  return p && b ? { project: p, board: b } : null;
}

export const useCurrent = () => {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  return boardMeta(ws, path);
};

export const serverNow = () => Date.now() + get().offset;

/** 在线的 agent 名字；工作区还没取到（或旧服务端没有在线列表）时为 null。
    名字没变就沿用同一个 Set（agent 每次心跳都会让列表换成新的对象，卡片不用因此重渲染） */
const online: { src: unknown; key: string | null; set: Set<string> | null } = { src: {}, key: null, set: null };
function onlineOf(ws: Workspace | null): Set<string> | null {
  const agents = ws?.agents;
  if (agents === online.src) return online.set;
  online.src = agents;
  const key = agents ? agents.map(a => a.name).join('\n') : null;
  if (key !== online.key) {
    online.key = key;
    online.set = agents ? new Set(agents.map(a => a.name)) : null;
  }
  return online.set;
}
export const useOnline = () => useData(s => onlineOf(s.ws));

/* ------------------------------ 加载 ------------------------------ */

/** 结构共享：next 里和 prev 相同的部分沿用 prev 的对象。每次刷新拿到的都是全新的对象，
    不这样做的话一条和某张卡无关的推送也会让整块白板的卡片重渲染（React.memo、选择器都按引用比较） */
export function share<T>(prev: unknown, next: T): T {
  if (prev === next || typeof prev !== 'object' || typeof next !== 'object' || !prev || !next) return next;
  if (Array.isArray(prev) !== Array.isArray(next)) return next;
  if (Array.isArray(next)) {
    const p = prev as unknown[];
    let same = p.length === next.length;
    const out = next.map((v, i) => {
      const x = share(p[i], v);
      if (x !== p[i]) same = false;
      return x;
    });
    return (same ? prev : out) as T;
  }
  const p = prev as Record<string, unknown>;
  const n = next as Record<string, unknown>;
  const keys = Object.keys(n);
  let same = keys.length === Object.keys(p).length;
  const out: Record<string, unknown> = {};
  for (const k of keys) {
    const x = share(p[k], n[k]);
    out[k] = x;
    if (x !== p[k] || !(k in p)) same = false;
  }
  return (same ? prev : out) as T;
}

let inflight: Promise<void> | null = null;
/** 正在进行的这一轮之后再来一轮全量：发起时不知道正在进行的那一轮带不带得上（写操作返回后、切换白板、断线重连） */
let again = false;
/** 推送说服务端到了这个 rev，当前白板要重取到它。推送一般在写操作的响应之后到，那时写操作发起的刷新已经能拿到它：
    这一轮加载到的 rev 不比它旧就不再来一轮（以前每次写都完整加载两遍） */
let wantBoard = 0;
/** 只有别的白板变了：工作区（顶栏、切换器的卡片数）要到这个 rev，当前白板不用重取 */
let wantWs = 0;
/** 当前白板、工作区分别加载到的 rev（rev 是全局的） */
let boardRev = 0;
let wsRev = 0;

export function refresh(): Promise<void> {
  again = true;
  return pump();
}

/* 写操作撤补丁的时机（#174）：补丁要留到服务端数据里已经有这次修改（撤掉时界面不闪），但不能等共用的 pump() 结束——
   后面每个写操作返回时都会再要一轮，前面的就一直被拖住。所以每个写操作等的是「它的响应返回之后才开始的第一轮加载」
   （那一轮读到的一定包含这次修改）；推送已经说明这次修改到了哪个 rev 时，任何一轮加载到那个 rev 也算。 */
/** 已经开始的加载轮数（pump 每开始一轮加一） */
let roundsStarted = 0;
/** 等加载的写操作：need 是它要等的那一轮（含）；ok 为真时提前算数 */
let roundWaiters: { need: number; ok?: () => boolean; done: () => void }[] = [];

/** 一轮加载完成后（成功、失败都算：失败时撤补丁、按现有数据画，和以前一样）放行等到的写操作 */
function releaseWaiters(round: number) {
  const keep: typeof roundWaiters = [];
  for (const w of roundWaiters) {
    if (round >= w.need || w.ok?.()) w.done();
    else keep.push(w);
  }
  roundWaiters = keep;
}

/** 写操作返回后：要一轮新的全量加载，等「这之后才开始的第一轮」完成（不等更后面的轮次） */
function afterWrite(): Promise<void> {
  const need = roundsStarted + 1;
  again = true;
  const p = new Promise<void>(done => roundWaiters.push({ need, done }));
  void pump();
  return p;
}

/** 推送说这次修改在 rev 里：已经加载到 rev 就不用等；否则等一轮加载到 rev（正在进行的那一轮也算），
    或者这之后才开始的一轮完成 */
function afterRev(rev: number): Promise<void> {
  if (boardRev >= rev) return Promise.resolve();
  wantBoard = Math.max(wantBoard, rev);
  const need = roundsStarted + 1;
  const p = new Promise<void>(done => roundWaiters.push({ need, ok: () => boardRev >= rev, done }));
  void pump();
  return p;
}

/** 推送里见过的卡片变化：「白板路径#卡片」→ 最新一条事件的时间和那次推送的 rev（只留最近的一些） */
const pushed = new Map<string, { at: number; rev: number }>();
function notePushed(rev: number, events: BoardEvent[]) {
  for (const e of events) {
    const k = `${e.board}#${e.cardId}`;
    const old = pushed.get(k);
    pushed.delete(k);
    pushed.set(k, { at: Math.max(e.at, old?.at ?? 0), rev: Math.max(rev, old?.rev ?? 0) });
  }
  while (pushed.size > 200) pushed.delete(pushed.keys().next().value!);
}

/** 写操作返回后的刷新。推送常常比响应先到（服务端提交后马上推送）：那条推送已经让当前白板重取到包含这次修改的 rev，
    只要等它；认法是服务端给这次修改记的动态和推送里的事件时间相同。认不出（没有动态、推送还没到）时全量刷新一轮 */
function refreshAfter(path: string, r: CardDetail | null): Promise<void> {
  const at = r?.activity?.length ? Math.max(...r.activity.map(a => a.at)) : null;
  const seen = r && pushed.get(`${path}#${r.id}`);
  if (at == null || !seen || seen.at < at || get().path !== path) return afterWrite();
  return afterRev(seen.rev);
}

/** 按需要一轮接一轮地加载，同时只有一轮在进行 */
function pump(): Promise<void> {
  if (inflight) return inflight;
  if (!again && boardRev >= wantBoard && wsRev >= wantWs) return Promise.resolve();
  inflight = (async () => {
    while (again || boardRev < wantBoard || wsRev < wantWs) {
      const full = again || boardRev < wantBoard;
      again = false;
      const round = ++roundsStarted;
      // 这一轮开始前就要的 rev：这一轮读到的白板一定不比它旧（服务端的 rev 是读事务里的全局版本）。
      // 读回来的白板 rev 万一还是更小（数据不一致），也算到了，不原地一轮接一轮
      const target = wantBoard;
      try {
        await (full ? load() : loadWs());
        if (full && get().board && boardRev < target) boardRev = target;
        set({ error: null });
      } catch (e) {
        set({ error: (e as Error).message });
        // 失败了不按 rev 反复重试：等下一次推送、重连或操作
        wantBoard = Math.min(wantBoard, boardRev);
        wantWs = Math.min(wantWs, wsRev);
      }
      releaseWaiters(round);
    }
  })().finally(() => {
    inflight = null;
    // 不会再有下一轮了：还在等的（例如加载到一半换了白板、rev 没追上）都放行，补丁不会一直挂着
    releaseWaiters(Number.MAX_SAFE_INTEGER);
  });
  return inflight;
}

/* ------------------------------ 终点状态的更早卡片 ------------------------------ */

/** 这个窗口里取过全量的白板：之后的刷新都带上 full（展开过的不再收起） */
const fullPaths = new Set<string>();

/** 快照里这个状态被省略的（更早的）张数 */
export const omittedIn = (b: BoardSnapshot | null | undefined, col: string) => b?.omitted?.[col] ?? 0;
/** 这个状态的卡片总数（含快照里省略的） */
export const colCount = (b: BoardSnapshot, col: string) => (b.cols[col]?.length ?? 0) + omittedIn(b, col);

/** 取当前白板的全量快照（终点状态更早的卡片）：点「更早的 M 张」、筛选、搜索、排序时。已经是全量时什么也不做 */
export function loadFull(): Promise<void> {
  const { path, board } = get();
  if (fullPaths.has(path) || (board?.path === path && !Object.values(board.omitted ?? {}).some(Boolean))) return Promise.resolve();
  fullPaths.add(path);
  return refresh();
}

async function load() {
  // 先让出一个微任务：同步调用方（applyRoute：先 setPath 再记 pendingCard）把这一轮要用的状态设完
  await Promise.resolve();
  let path = get().path;
  // 当前白板在上一次取到的工作区里：工作区、白板、卡片三个一起取（一轮一个往返，#174）。
  // 不在（第一次加载、白板被删了）：先取工作区、换白板，再取白板（两个往返，和以前一样）
  const known = !!base.ws && allBoards(base.ws).some(b => b.path === path);
  let ws: Workspace | null = null;
  if (!known) {
    ws = await api.workspace();
    const paths = allBoards(ws).map(b => b.path);
    if (!paths.includes(path)) {
      const s = saved();
      path = paths.includes(s) ? s : paths[0];
      pendingCard = null; // 地址里的白板不在了：地址里的卡片也不打开
      switchTo(path);
    }
  }
  // 地址栏里的卡片（刷新页面、从别处打开链接）：和这块白板一起取回来，取回来的那一帧就打开详情。
  // 不写地址栏：这时可能正按地址（后退、前进）改状态，写回去会把历史弄乱
  let detailId = get().detailId;
  if (pendingCard != null) {
    detailId = pendingCard;
    pendingCard = null;
    base.detail = null;
    render({ detailId });
  }
  const [wsNow, got, rawDetail] = await Promise.all([
    ws ?? api.workspace(),
    // 并行取时白板可能刚被删掉：先接住错误，看工作区再决定
    api.board(path, fullPaths.has(path)).catch((e: unknown) => ({ failed: e })),
    // 卡片不在了（NOT_FOUND）记为 gone：详情要关掉。不在快照里的卡片（终点状态更早的）不能靠快照判断
    detailId != null ? api.card(path, detailId).catch(e => (e instanceof ApiError && e.code === 'NOT_FOUND' ? ('gone' as const) : null)) : Promise.resolve(null),
  ]);
  ws = wsNow;
  if (!allBoards(ws).some(b => b.path === path)) {
    // 当前白板在这期间不在了：记下新的工作区，下一轮按它换白板
    base.ws = share(base.ws, ws);
    again = true;
    return;
  }
  if ('failed' in got) throw got.failed;
  const fetched = got;
  const st = get();
  if (st.path !== path) {
    again = true;
    return;
  }
  const prev = st.board;
  if (prev && prev.path === fetched.path) flip.capture();
  else flip.reset();
  const board = share(base.board?.path === fetched.path ? base.board : snapshots.get(fetched.path), fetched);
  const gone = rawDetail === 'gone' && detailId != null && st.detailId === detailId && !fetched.cards[detailId];
  const detail = rawDetail && rawDetail !== 'gone' && share(base.detail?.id === rawDetail.id ? base.detail : details.get(`${path}#${rawDetail.id}`), rawDetail);
  snapshots.set(board.path, board);
  if (detail) details.set(`${path}#${detail.id}`, detail);
  base.ws = share(base.ws, ws);
  base.board = board;
  if (st.detailId === detailId && detail) base.detail = detail;
  if (gone) base.detail = null;
  // 三个并行取时工作区可能比白板新：当前白板按白板自己的 rev 记（落后的那部分等推送再取，不会漏）
  boardRev = known ? board.rev : Math.max(ws.rev, board.rev);
  wsRev = ws.rev;
  render({ rev: boardRev, offset: ws.now - Date.now(), ...(gone ? { detailId: null } : {}) });
  // 地址栏和现在打开的保持一致：记着的白板不在了、换了白板之后地址跟着改（后退、前进时由 applySilently 挡住）
  syncUrl();
}

/** 只取工作区：推送里的变化都属于别的白板时用（顶栏、切换器要用它的卡片数），不重取当前白板 */
async function loadWs() {
  const ws = await api.workspace();
  // 当前白板不在了（例如在别处删掉）：全量刷新，换到别的白板
  if (!allBoards(ws).some(b => b.path === get().path)) {
    again = true;
    return;
  }
  base.ws = share(base.ws, ws);
  wsRev = ws.rev;
  render({ offset: ws.now - Date.now() });
}

/** 只刷新机器和派发（机器页打开时每 10 秒一次：机器心跳不推送，靠它更新「最近心跳」） */
export async function refreshHosts() {
  const w = writes;
  const r = await api.hosts();
  // 请求发出后又有写操作：这份可能是写之前的旧数据，用它会把刚保存的修改冲回去（例如「设为默认」又变回原来的）
  if (!base.ws || w !== writes) return;
  base.ws = { ...base.ws, hosts: r.hosts, dispatches: r.dispatches };
  render();
}

/* ------------------------------ 标签：乐观更新 ------------------------------ */

const pkey = (path: string, id: number) => `${path}#${id}`;

/** 给卡片加标签：松手当帧就画上（pending 态），服务端确认后转正；失败时撤回并抛出错误。
    卡片已经有这个标签时（包括正在加）返回 false，不发请求 */
export async function addLabel(id: number, labelId: number): Promise<boolean> {
  const { board, path } = get();
  const c = board?.cards[id];
  if (!board || !c || c.labels.includes(labelId)) return false;
  const k = pkey(path, id);
  setPending(k, labelId, true);
  try {
    await exec({ op: 'label', id, add: [labelId] }, editCard(id, x => (x.labels.includes(labelId) ? {} : { labels: [...x.labels, labelId] }), false));
    return true;
  } finally {
    // 成功时刷新后的快照里已经有这个标签：去掉 pending 只是换掉样式，位置不动；失败时补丁已经撤掉，标签跟着消失
    setPending(k, labelId, false);
  }
}

/** 从卡片上去掉标签：当帧把这个标签画成和「正在加上」一样的 pending 态（× 还在、不能点），服务端确认、刷新后消失；
    失败时恢复原样并抛出错误。不把整张卡片画成「保存中」，其他标签照常显示、照常能操作。
    这个标签正在加上或去掉时（pending）不发请求，返回 false */
export async function removeLabel(id: number, labelId: number): Promise<boolean> {
  const { board, path, pending } = get();
  const c = board?.cards[id];
  const k = pkey(path, id);
  if (!board || !c || !c.labels.includes(labelId) || pending[k]?.includes(labelId)) return false;
  setPending(k, labelId, true);
  try {
    await exec({ op: 'label', id, remove: [labelId] });
    return true;
  } finally {
    setPending(k, labelId, false);
  }
}

function setPending(k: string, labelId: number, on: boolean) {
  set(s => {
    const ls = (s.pending[k] ?? []).filter(l => l !== labelId);
    if (on) ls.push(labelId);
    const pending = { ...s.pending };
    if (ls.length) pending[k] = ls;
    else delete pending[k];
    return { pending };
  });
}

/* ------------------------------ 标签顺序：乐观更新 ------------------------------ */

/** 按给定顺序排项目的标签；没出现在 ids 里的（例如刚新建的）按原顺序排在后面，与服务端规则一致 */
function sortLabels<T extends { id: number }>(labels: T[], ids: number[]): T[] {
  const rank = new Map(ids.map((id, i) => [id, i]));
  return labels
    .map((l, i) => ({ l, k: rank.get(l.id) ?? ids.length + i }))
    .sort((a, b) => a.k - b.k)
    .map(x => x.l);
}

/** 调整项目里标签的顺序：当帧生效（侧栏和卡片上的标签一起变），失败时撤掉这一次并抛出错误（之后的拖动不受影响） */
export async function reorderLabels(project: string, ids: number[]) {
  await optimistic(patchProject(project, p => ({ ...p, labels: sortLabels(p.labels, ids) })), () => api.reorderLabels(project, ids));
}

/** 打开过的白板的最近一份快照：切回来时先显示它，不闪骨架，refresh 拿到新的再替换 */
const snapshots = new Map<string, BoardSnapshot>();

function switchTo(path: string, opts: { open?: number | null } = {}) {
  save(path);
  flip.reset();
  clearLabelSel(); // 标签属于项目，换白板时清掉选中
  base.board = snapshots.get(path) ?? null;
  base.detail = null;
  // open：切过去的同时打开那张卡片（每一张卡片有自己的网址，不是「白板 + 详情」两条）
  render({ path, detailId: opts.open ?? null, generation: get().generation + 1 });
  // 切换白板写进地址栏（新的一块白板是一条新的历史记录：后退能回到原来那块）；地址里开着卡片时连卡片一起写
  syncUrl(opts.open != null ? 'replace' : get().detailId == null ? 'push' : 'replace');
}

/** 打开另一块白板 */
export function setPath(path: string) {
  if (path === get().path && get().board) return;
  switchTo(path);
  return refresh();
}

/** 打开另一块白板上的卡片：只算一条地址（`/<项目>/<白板>/<ID>`），后退直接回到打开之前那张
    （先切白板再打开卡片会写两条：退一次只关掉详情，停在只有白板那一页） */
export async function switchToCard(path: string, id: number) {
  if (path === get().path) return openDetail(id);
  switchTo(path, { open: id });
  // 详情和白板一起取：详情有缓存时先显示它，不闪骨架
  const shown = openDetail(id);
  await refresh();
  await shown;
}

/* ------------------------------ 卡片详情 ------------------------------ */

/** 打开过的卡片详情：再次打开先显示它（不闪骨架），取回新的再替换 */
const details = new Map<string, CardDetail>();

/**
 * 卡片详情的历史记录（PRD §10.8「前进 / 返回」）：这个窗口里按地址打开过的卡片，新的在前。
 * 换掉当前这条地址时（详情里切上一张 / 下一张、点卡片编号跳过去）记下原来那张，
 * 于是详情里能在卡片之间「返回」「前进」，和浏览器的后退、前进不是一回事。
 * 另一块白板的卡片记成「白板路径#ID」，返回时先切回那块白板。
 */
interface CardPlace {
  path: string;
  id: number;
}
let cardBack: CardPlace[] = [];
let cardFwd: CardPlace[] = [];

/** 一张卡片在这个窗口里的地址：另一块白板的白板路径 + ID */
const placeKey = (p: CardPlace) => `${p.path}#${p.id}`;

function placeOf(id: number): CardPlace {
  return { path: get().path, id };
}

/** 记一次「换掉当前的卡片」：原来那张能返回，前进的记录作废 */
function remember(from: CardDetail | null | undefined) {
  if (!from) return;
  cardBack.push(placeOf(from.id));
  cardFwd = [];
}

export async function openDetail(id: number) {
  if (isDraft(id)) return;
  const path = get().path;
  const k = `${path}#${id}`;
  const was = get().detailId;
  base.detail = base.detail?.id === id ? base.detail : details.get(k) ?? null;
  render({ detailId: id, detailError: null });
  // 打开卡片加一条历史记录（后退能关掉）；详情里切上一张 / 下一张换掉当前这条。按地址栏改状态时不写
  if (was !== id) syncUrl(was == null ? 'push' : 'replace');
  save(path);
  try {
    const w = writes;
    const d = await api.card(path, id);
    // 请求发出后又有写操作：可能是旧数据（例如刚改的标题），不用它，重新取一次（refresh 会带回打开着的详情）
    if (w !== writes) return void refresh();
    details.set(k, d);
    if (get().detailId === id && get().path === path) {
      base.detail = d;
      render();
    }
  } catch (e) {
    if (get().detailId !== id || get().path !== path) return;
    // 卡片已经不在了（例如在另一个窗口删除）、或者快照里没有它（终点状态更早的卡片）没有摘要可显示：关掉；否则留着摘要，告诉人详情没取回来
    const gone = e instanceof ApiError && e.code === 'NOT_FOUND';
    if (gone || !get().board?.cards[id]) {
      closeDetail();
      if (!gone) toast({ type: 'error', title: `#${id} 的详情没有加载出来`, text: (e as Error).message || '网络错误' });
    } else {
      const msg = (e as Error).message || '网络错误';
      set({ detailError: msg });
      toast({ type: 'error', title: `#${id} 的详情没有加载出来`, text: msg });
    }
  }
}
export function closeDetail() {
  const had = get().detailId != null;
  base.detail = null;
  render({ detailId: null });
  // 关掉详情也加一条历史记录：后退能重新打开（卡片已经被删、不是从这个窗口打开的不算）。按地址改的时候不写
  if (had) syncUrl('push');
  // 关了就不再是「在哪张卡片之间返回」：再打开一张重新算
  cardBack = [];
  cardFwd = [];
}

/** 详情里跳到另一张卡片：换掉当前这条地址，原来那张记进「返回」 */
export async function jumpDetail(id: number) {
  if (id === get().detailId) return;
  remember(base.detail);
  await openDetail(id);
}

/** 打开另一块白板上的卡片（描述、评论里的 `项目/白板#ID`）：切到那块白板，换掉当前这条地址 */
export async function jumpToCard(path: string, id: number) {
  if (path === get().path) return jumpDetail(id);
  remember(base.detail);
  await switchToCard(path, id);
}

const cardBackwards = () => cardBack.some(p => p.path !== get().path || p.id !== get().detailId);
const cardForwards = () => cardFwd.some(p => p.path !== get().path || p.id !== get().detailId);

/** 切到那张卡片、换掉当前这条地址；返回 / 前进的记录由调用方先放好 */
async function openPlace(p: CardPlace) {
  if (p.path !== get().path) await switchToCard(p.path, p.id);
  else await openDetail(p.id);
}

/** 「返回」：回到跳转之前在看的卡片（在另一块白板上时先切过去）。没有可返回的就不做 */
export function cardBackStep(): Promise<void> | void {
  const cur = get().detailId;
  if (cur == null || !cardBackwards()) return;
  const to = cardBack.pop()!;
  cardBack = cardBack.filter(p => placeKey(p) !== placeKey(to)); // 同一张卡片只留最近一次
  cardFwd = [{ path: get().path, id: cur }, ...cardFwd.filter(p => p.path !== get().path || p.id !== cur)];
  return openPlace(to);
}

/** 「前进」：回到刚才返回之前看的那张（在另一块白板上时先切过去）。没有可前进的就不做 */
export function cardFwdStep(): Promise<void> | void {
  const cur = get().detailId;
  if (cur == null || !cardForwards()) return;
  const to = cardFwd.shift()!;
  cardFwd = cardFwd.filter(p => placeKey(p) !== placeKey(to));
  cardBack = [...cardBack.filter(p => p.path !== get().path || p.id !== cur), { path: get().path, id: cur }];
  return openPlace(to);
}

/** 详情里的「返回 / 前进」能不能点（组件用，不是 zustand 的 store） */
export const canCardBack = () => cardBackwards();
export const canCardFwd = () => cardForwards();

/** 按地址栏改界面状态（后退、前进、打开链接时 App 调用）：期间不往地址栏写。地址里没有白板时不动 */
export function applyRoute(r: { path: string | null; card: number | null; tab: Tab }) {
  // Tab 也在里面改：它一般会顺手写地址栏，这时列表还没取回来，会把地址里的卡片弄丢
  applySilently(() => {
    setTab(r.tab);
    const switching = !!r.path && (r.path !== get().path || !get().board);
    if (switching) void setPath(r.path!);
    if (r.card != null) {
      // 刚切过去、列表还没到：等它到了和详情一起打开（见 load）
      if (switching) pendingCard = r.card;
      else if (get().detailId !== r.card) void openDetail(r.card);
    } else {
      pendingCard = null;
      if (get().detailId != null) closeDetail();
    }
  });
}

/* ------------------------------ 写 ------------------------------ */

/* ------------------------------ 评论：乐观更新 ------------------------------ */

const NO_SENDING: Sending[] = [];
export const useSending = (id: number) => useData(s => s.sending[pkey(s.path, id)] ?? NO_SENDING);

/** 由本窗口发出、发送中气泡换成的正式评论：它们不再播放「新动态」的入场动画，免得转正时闪一下 */
const settled = new Set<number>();
export const isSettled = (activityId: number) => settled.has(activityId);

let sendSeq = 0;

/** 发评论：当帧在动态底部画上「发送中」的气泡；服务端确认后，在同一次更新里换成正式评论；失败时移除气泡并抛出错误。
    和其他写操作排在同一个队列里，连续快速发送时服务端收到的顺序与界面一致 */
export function sendComment(id: number, text: string): Promise<void> {
  const path = get().path;
  const k = pkey(path, id);
  const item: Sending = { key: ++sendSeq, text, at: serverNow() };
  const drop = (p: Record<string, Sending[]>) => {
    const ls = (p[k] ?? []).filter(x => x.key !== item.key);
    const next = { ...p };
    if (ls.length) next[k] = ls;
    else delete next[k];
    return next;
  };
  set(s => ({ sending: { ...s.sending, [k]: [...(s.sending[k] ?? []), item] } }));
  return queue(async () => {
    try {
      const r = await api.cmd(path, { op: 'comment', id, text });
      details.set(k, r);
      const mine = [...r.activity].reverse().find(a => a.kind === 'comment' && a.actor.kind === 'human' && a.text.trim() === text.trim());
      if (mine) settled.add(mine.id);
      const s = get();
      if (s.detailId === id && s.path === path) {
        base.detail = r;
        render({ sending: drop(s.sending), detailError: null });
      } else set({ sending: drop(s.sending) });
    } catch (e) {
      set(s => ({ sending: drop(s.sending) }));
      throw e;
    }
    void refresh();
  });
}

/** 对当前白板执行卡片命令；patch 是它在本地的效果，当帧生效（见 optimistic）。失败抛 ApiError，由调用方提示 */
/** settle：服务端一返回就调用（在刷新之前），例如新建时让临时卡片认下真卡片的 ID，刷新带来真卡片的那一帧就顶替上去 */
export async function exec(cmd: Cmd, patch?: Patch, settle?: (r: CardDetail) => void): Promise<CardDetail> {
  const path = get().path;
  if ('id' in cmd) guard({ ...patch, cards: [...(patch?.cards ?? []), cmd.id ?? 0].filter(id => id > 0) });
  else if (patch) guard(patch);
  const l = patch ? push(patch) : null;
  try {
    const r = await queue(() => api.cmd(path, cmd));
    settle?.(r);
    // 删除、永久删除返回的是删除前的卡片：写进详情缓存会把已经不在的卡片又画出来
    if (cmd.op !== 'delete' && cmd.op !== 'purge') {
      details.set(pkey(path, r.id), r);
      if (get().detailId === r.id && get().path === path) {
        base.detail = r;
        render({ detailError: null });
      }
    }
    await refreshAfter(path, cmd.op === 'delete' || cmd.op === 'purge' ? null : r);
    return r;
  } finally {
    if (l) drop(l);
  }
}

/** 卡片的删除、恢复、永久删除在 lib/trash.ts：它们要知道回收站里的内容（卡片的原位置），还要让删除后的提示能撤销 */

/* ------------------------------ 实时 ------------------------------ */

type Listener = (events: BoardEvent[]) => void;
const listeners = new Set<Listener>();
export function onEvents(fn: Listener) {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

/** 有新的结构化事件的派发（推送里的 ran）：打开着的会话据此马上增量取（#184） */
type RunListener = (dispatches: number[]) => void;
const runListeners = new Set<RunListener>();
export function onRuns(fn: RunListener) {
  runListeners.add(fn);
  return () => {
    runListeners.delete(fn);
  };
}

/** 还有没确认的修改（保存中、发送中）时关窗口：浏览器弹离开确认，免得丢 */
function beforeUnload(e: BeforeUnloadEvent) {
  const s = get();
  if (Object.keys(s.saving).length || Object.keys(s.sending).length) {
    e.preventDefault();
    e.returnValue = '';
  }
}

export function boot() {
  refresh();
  window.addEventListener('beforeunload', beforeUnload);
  const stop = connect(
    m => {
      const all = m.events || [];
      const events = all.filter(e => e.client !== CLIENT_ID || e.via !== 'ui');
      let done: Promise<void>;
      // 落后太多（rev < 0）：全量刷新
      if (m.rev < 0) done = refresh();
      else {
        // 变化都属于别的白板：只取工作区。没有事件的变化（超时释放、机器、标签……）不知道影响哪里，按当前白板算
        notePushed(m.rev, all);
        const here = !all.length || all.some(e => e.board === get().path);
        if (here) wantBoard = Math.max(wantBoard, m.rev);
        wantWs = Math.max(wantWs, m.rev);
        done = pump();
      }
      if (events.length) done.then(() => listeners.forEach(fn => fn(events)));
      if (m.ran?.length) runListeners.forEach(fn => fn(m.ran!));
    },
    status => {
      set({ status });
      // 断开时也取一次：会话过期会得到 401，界面切到登录页
      if (status === 'live' || status === 'offline') refresh();
    },
  );
  return () => {
    stop();
    window.removeEventListener('beforeunload', beforeUnload);
  };
}
