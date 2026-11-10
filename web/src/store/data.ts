/* ==========================================================================
   数据：工作区、当前白板（每个窗口可以看不同的白板）、打开的卡片详情
   - 服务端推送 rev 变化 → 重新取快照；新事件交给界面做高亮与提示
   - 写操作乐观更新（见「乐观更新」一节）：当帧生效，按发起顺序一个接一个发给服务端，
     成功后刷新再撤掉本地补丁，失败只撤掉自己那一层；保存完之前同一个对象不能再改（见 busy）
   ========================================================================== */
import { create } from 'zustand';
import { api, ApiError, connect, CLIENT_ID, type LiveStatus } from '../lib/api';
import * as flip from '../lib/flip';
import { clearLabelSel, toast } from './ui';
import type { BoardEvent, BoardInfo, BoardSnapshot, Card, CardDetail, Cmd, ColumnKey, ProjectInfo, Workspace } from '../lib/types';

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
  /** 正在写入的标签（乐观更新）：「白板路径#卡片」→ 标签 ID。已经并进 board 里卡片的 labels，界面据此画成 pending 态 */
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

export const useData = create<DataState>(() => ({
  ws: null,
  path: saved(),
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

/** 乐观执行一个写请求：补丁当帧生效；成功后 refresh，再撤掉补丁；失败撤掉补丁并抛出错误（由调用方提示）。
    patch 为 null 时只是排队执行 + 刷新（拿不到结果就没法预先画出来的操作，例如新建项目），调用方自己显示进行中 */
export async function optimistic<T>(patch: Patch | null, req: () => Promise<T>): Promise<T> {
  if (patch) guard(patch);
  const l = patch ? push(patch) : null;
  try {
    const r = await queue(req);
    await refresh();
    return r;
  } finally {
    if (l) drop(l);
  }
}

/** 叠上补丁，直到 done 结束再撤掉（done 成功时应已刷新过）。用于先发请求、稍后才确定本地效果的操作（拖拽松手的飞回动画结束后才落位） */
export function hold<T>(patch: Patch, done: Promise<T>): Promise<T> {
  const l = push(patch);
  done.then(
    () => drop(l),
    () => drop(l),
  );
  return done;
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

/** 把卡片放到 to 的第 index 位（按去掉它之后的列表算；省略时放最后）。order：排序状态下拖动时整块的新顺序（与服务端规则一致：没列出的补在后面）。
    extra：一起改的其他字段（例如释放时清空持有者） */
export function movePatch(id: number, to: ColumnKey, index?: number, order?: number[], extra: Partial<Card> = {}): Patch {
  return {
    board: b => {
      const c = b.cards[id];
      if (!c) return b;
      const cols: BoardSnapshot['cols'] = {};
      for (const [k, v] of Object.entries(b.cols)) cols[k] = v.filter(x => x !== id);
      const list = cols[to] ?? [];
      list.splice(Math.min(index ?? list.length, list.length), 0, id);
      cols[to] = order ? [...new Set([...order.filter(x => list.includes(x)), ...list])] : list;
      return { ...b, cols, cards: { ...b.cards, [id]: { ...c, ...extra, column: to } } };
    },
    detail: d => (d.id === id ? { ...d, ...extra, column: to } : d),
    cards: [id],
  };
}

/** 从白板上拿掉一张卡片（删除） */
export function removeCard(id: number): Patch {
  return {
    board: b => {
      if (!b.cards[id]) return b;
      const cards = { ...b.cards };
      delete cards[id];
      const cols: BoardSnapshot['cols'] = {};
      for (const [k, v] of Object.entries(b.cols)) cols[k] = v.filter(x => x !== id);
      return { ...b, cols, cards };
    },
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
        }
      }
      if (real != null && b.cards[real]) return b;
      return { ...b, cards: { ...b.cards, [id]: card }, cols: { ...b.cols, [p.column]: [...(b.cols[p.column] ?? []), id] } };
    },
    cards: [id],
  };
  const settle = (realId: number) => {
    if (real != null && real !== realId) claimed.delete(real);
    real = realId;
    claimed.add(realId);
  };
  return { id, patch, settle };
}

/** 临时卡片（还在新建）：没有详情、不能拖、不能打开 */
export const isDraft = (id: number) => id < 0;

/** 改一个项目（名称、颜色、标签……） */
export const patchProject = (key: string, fn: (p: ProjectInfo) => ProjectInfo): Patch => ({
  ws: ws => ({ ...ws, projects: ws.projects.map(p => (p.key === key ? fn(p) : p)) }),
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
  if (at == null || !seen || seen.at < at || get().path !== path) return refresh();
  wantBoard = Math.max(wantBoard, seen.rev);
  return pump();
}

/** 按需要一轮接一轮地加载，同时只有一轮在进行 */
function pump(): Promise<void> {
  if (inflight) return inflight;
  if (!again && boardRev >= wantBoard && wsRev >= wantWs) return Promise.resolve();
  inflight = (async () => {
    while (again || boardRev < wantBoard || wsRev < wantWs) {
      const full = again || boardRev < wantBoard;
      again = false;
      try {
        await (full ? load() : loadWs());
        set({ error: null });
      } catch (e) {
        set({ error: (e as Error).message });
        // 失败了不按 rev 反复重试：等下一次推送、重连或操作
        wantBoard = Math.min(wantBoard, boardRev);
        wantWs = Math.min(wantWs, wsRev);
      }
    }
  })().finally(() => {
    inflight = null;
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
  const ws = await api.workspace();
  const paths = allBoards(ws).map(b => b.path);
  let path = get().path;
  if (!paths.includes(path)) {
    const s = saved();
    path = paths.includes(s) ? s : paths[0];
    switchTo(path);
  }
  const detailId = get().detailId;
  const [fetched, rawDetail] = await Promise.all([
    api.board(path, fullPaths.has(path)),
    // 卡片不在了（NOT_FOUND）记为 gone：详情要关掉。不在快照里的卡片（终点状态更早的）不能靠快照判断
    detailId != null ? api.card(path, detailId).catch(e => (e instanceof ApiError && e.code === 'NOT_FOUND' ? ('gone' as const) : null)) : Promise.resolve(null),
  ]);
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
  boardRev = Math.max(ws.rev, board.rev);
  wsRev = ws.rev;
  render({ rev: boardRev, offset: ws.now - Date.now(), ...(gone ? { detailId: null } : {}) });
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
  set(s => ({ pending: { ...s.pending, [k]: [...(s.pending[k] ?? []), labelId] } }));
  try {
    await exec({ op: 'label', id, add: [labelId] }, editCard(id, x => (x.labels.includes(labelId) ? {} : { labels: [...x.labels, labelId] }), false));
    return true;
  } finally {
    // 成功时刷新后的快照里已经有这个标签：去掉 pending 只是换掉样式，位置不动；失败时补丁已经撤掉，标签跟着消失
    set(s => {
      const ls = (s.pending[k] ?? []).filter(l => l !== labelId);
      const pending = { ...s.pending };
      if (ls.length) pending[k] = ls;
      else delete pending[k];
      return { pending };
    });
  }
}

/** 从卡片上去掉标签：当帧消失，失败时回来并抛出错误 */
export const removeLabel = (id: number, labelId: number) =>
  exec({ op: 'label', id, remove: [labelId] }, editCard(id, x => ({ labels: x.labels.filter(l => l !== labelId) })));

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

function switchTo(path: string) {
  save(path);
  flip.reset();
  clearLabelSel(); // 标签属于项目，换白板时清掉选中
  base.board = snapshots.get(path) ?? null;
  base.detail = null;
  render({ path, detailId: null, generation: get().generation + 1 });
}

/** 打开另一块白板 */
export function setPath(path: string) {
  if (path === get().path && get().board) return;
  switchTo(path);
  return refresh();
}

/* ------------------------------ 卡片详情 ------------------------------ */

/** 打开过的卡片详情：再次打开先显示它（不闪骨架），取回新的再替换 */
const details = new Map<string, CardDetail>();

export async function openDetail(id: number) {
  if (isDraft(id)) return;
  const path = get().path;
  const k = `${path}#${id}`;
  base.detail = base.detail?.id === id ? base.detail : details.get(k) ?? null;
  render({ detailId: id, detailError: null });
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
  base.detail = null;
  render({ detailId: null });
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
export async function exec(cmd: Cmd, patch?: Patch): Promise<CardDetail> {
  const path = get().path;
  if ('id' in cmd) guard({ ...patch, cards: [...(patch?.cards ?? []), cmd.id] });
  else if (patch) guard(patch);
  const l = patch ? push(patch) : null;
  try {
    const r = await queue(() => api.cmd(path, cmd));
    if (cmd.op !== 'delete') {
      details.set(pkey(path, r.id), r);
      if (get().detailId === r.id && get().path === path) {
        base.detail = r;
        render({ detailError: null });
      }
    }
    await refreshAfter(path, cmd.op === 'delete' ? null : r);
    return r;
  } finally {
    if (l) drop(l);
  }
}

/* ------------------------------ 实时 ------------------------------ */

type Listener = (events: BoardEvent[]) => void;
const listeners = new Set<Listener>();
export function onEvents(fn: Listener) {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
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
