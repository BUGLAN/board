/* ==========================================================================
   数据：工作区、当前白板（每个窗口可以看不同的白板）、打开的卡片详情
   - 服务端推送 rev 变化 → 重新取快照；新事件交给界面做高亮与提示
   - 写操作乐观更新（见「乐观更新」一节）：当帧生效，按发起顺序一个接一个发给服务端，
     成功后刷新再撤掉本地补丁，失败只撤掉自己那一层
   ========================================================================== */
import { create } from 'zustand';
import { api, connect, CLIENT_ID, type LiveStatus } from '../lib/api';
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
  /** 还没确认的写操作涉及的卡片（「白板路径#卡片」）和评论（「白板路径@动态 ID」）→ 进行中的个数，界面据此画成「保存中」 */
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
    if (l.path !== path) continue;
    if (l.board && board?.path === path) board = l.board(board);
    if (l.detail && detail) detail = l.detail(detail);
  }
  set({ ws, board, detail, saving, ...extra });
}

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

/** 写请求按发起顺序一个接一个发：连续操作时服务端收到的顺序与界面一致（补丁当帧就叠上，不用等前面的） */
let chain: Promise<unknown> = Promise.resolve();
function queue<T>(req: () => Promise<T>): Promise<T> {
  const run = chain.then(req);
  chain = run.catch(() => undefined);
  return run;
}

/** 乐观执行一个写请求：补丁当帧生效；成功后 refresh，再撤掉补丁；失败撤掉补丁并抛出错误（由调用方提示）。
    patch 为 null 时只是排队执行 + 刷新（拿不到结果就没法预先画出来的操作，例如新建项目），调用方自己显示进行中 */
export async function optimistic<T>(patch: Patch | null, req: () => Promise<T>): Promise<T> {
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

let tempSeq = 0;
/** 新建卡片：先用一个负数 ID 的临时卡片画在目标状态最下面（服务端的规则），服务端返回后由 settle 记下真正的 ID：
    刷新拿到真卡片的那一刻临时卡片不再画，两张不会同时出现 */
export function draftCard(p: { title: string; body: string; labels: number[]; column: ColumnKey }) {
  const id = -++tempSeq;
  let real: number | null = null;
  const now = serverNow();
  const card: Card = {
    id, title: p.title.trim(), body: p.body.trim(), column: p.column, holder: null, claimedAt: null, beatAt: null, runStartedAt: null,
    workMs: 0, runs: 0, runsLost: 0, claimedFrom: null, labels: p.labels, createdAt: now, updatedAt: now,
    createdBy: { kind: 'human', name: '' }, comments: 0,
  };
  const patch: Patch = {
    board: b => {
      if (real != null && b.cards[real]) return b;
      return { ...b, cards: { ...b.cards, [id]: card }, cols: { ...b.cols, [p.column]: [...(b.cols[p.column] ?? []), id] } };
    },
    cards: [id],
  };
  return { id, patch, settle: (realId: number) => (real = realId) };
}

/** 临时卡片（还在新建）：没有详情、不能拖、不能打开 */
export const isDraft = (id: number) => id < 0;

/** 改一个项目（名称、颜色、标签……） */
export const patchProject = (key: string, fn: (p: ProjectInfo) => ProjectInfo): Patch => ({
  ws: ws => ({ ...ws, projects: ws.projects.map(p => (p.key === key ? fn(p) : p)) }),
});

/** 改项目里的一个标签（名称、颜色） */
export const editLabelPatch = (project: string, id: number, change: { name?: string; color?: string }) =>
  patchProject(project, p => ({ ...p, labels: p.labels.map(l => (l.id === id ? { ...l, ...change } : l)) }));

/** 删除项目里的一个标签：从项目和当前白板的卡片上一起拿掉 */
export const deleteLabelPatch = (project: string, id: number): Patch => ({
  ...patchProject(project, p => ({ ...p, labels: p.labels.filter(l => l.id !== id) })),
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

/** 在线的 agent 名字；工作区还没取到（或旧服务端没有在线列表）时为 null */
const onlineOf = (ws: Workspace | null): Set<string> | null => (ws?.agents ? new Set(ws.agents.map(a => a.name)) : null);
let onlineCache: [Workspace | null, Set<string> | null] = [null, null];
export const useOnline = () =>
  useData(s => {
    if (onlineCache[0] !== s.ws) onlineCache = [s.ws, onlineOf(s.ws)];
    return onlineCache[1];
  });

/* ------------------------------ 加载 ------------------------------ */

let inflight: Promise<void> | null = null;
let again = false;

export function refresh(): Promise<void> {
  if (inflight) {
    again = true;
    return inflight;
  }
  inflight = (async () => {
    do {
      again = false;
      try {
        await load();
        set({ error: null });
      } catch (e) {
        set({ error: (e as Error).message });
      }
    } while (again);
  })().finally(() => {
    inflight = null;
  });
  return inflight;
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
  const [board, detail] = await Promise.all([
    api.board(path),
    detailId != null ? api.card(path, detailId).catch(() => null) : Promise.resolve(null),
  ]);
  const st = get();
  if (st.path !== path) {
    again = true;
    return;
  }
  const prev = st.board;
  if (prev && prev.path === board.path) flip.capture();
  else flip.reset();
  snapshots.set(board.path, board);
  if (detail) details.set(`${path}#${detail.id}`, detail);
  base.ws = ws;
  base.board = board;
  if (st.detailId === detailId && detail) base.detail = detail;
  render({ rev: Math.max(ws.rev, board.rev), offset: ws.now - Date.now() });
}

/** 只刷新机器和派发（机器页打开时每 10 秒一次：机器心跳不推送，靠它更新「最近心跳」） */
export async function refreshHosts() {
  const r = await api.hosts();
  if (!base.ws) return;
  base.ws = { ...base.ws, hosts: r.hosts, dispatches: r.dispatches };
  render();
}

/* ------------------------------ 标签：乐观更新 ------------------------------ */

const pkey = (path: string, id: number) => `${path}#${id}`;

export const pendingLabels = (path: string, id: number) => get().pending[pkey(path, id)];

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
    const d = await api.card(path, id);
    details.set(k, d);
    if (get().detailId === id && get().path === path) {
      base.detail = d;
      render();
    }
  } catch (e) {
    if (get().detailId !== id || get().path !== path) return;
    // 卡片已经不在了（例如在另一个窗口删除）：关掉；否则留着摘要，告诉人详情没取回来
    if (!get().board?.cards[id]) closeDetail();
    else {
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
    await refresh();
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

export function boot() {
  refresh();
  return connect(
    m => {
      const fresh = m.rev !== get().rev || m.rev < 0;
      const events = (m.events || []).filter(e => e.client !== CLIENT_ID || e.via !== 'ui');
      const done = fresh ? refresh() : Promise.resolve();
      if (events.length) done.then(() => listeners.forEach(fn => fn(events)));
    },
    status => {
      set({ status });
      // 断开时也取一次：会话过期会得到 401，界面切到登录页
      if (status === 'live' || status === 'offline') refresh();
    },
  );
}
