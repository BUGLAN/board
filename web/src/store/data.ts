/* ==========================================================================
   数据：工作区、当前白板（每个窗口可以看不同的白板）、打开的卡片详情
   - 服务端推送 rev 变化 → 重新取快照；新事件交给界面做高亮与提示
   - 写操作走 API，成功后立即刷新（不等推送）
   ========================================================================== */
import { create } from 'zustand';
import { api, connect, CLIENT_ID, type LiveStatus } from '../lib/api';
import * as flip from '../lib/flip';
import { clearLabelSel, toast } from './ui';
import type { BoardEvent, BoardInfo, BoardSnapshot, CardDetail, Cmd, ColumnKey, ProjectInfo, Workspace } from '../lib/types';

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
  detailError: null,
}));

const set = useData.setState;
const get = useData.getState;

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
  const ws = withOrder(await api.workspace());
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
  set({
    ws,
    board: withPending(board, get().pending),
    rev: Math.max(ws.rev, board.rev),
    offset: ws.now - Date.now(),
    detail: st.detailId === detailId && detail ? detail : st.detail,
  });
}

/* ------------------------------ 标签：乐观更新 ------------------------------ */

const pkey = (path: string, id: number) => `${path}#${id}`;

/** 把正在写入的标签并进快照（refresh 拿到的新快照也要并，免得 pending 被冲掉；服务端已有的不重复） */
function withPending(board: BoardSnapshot, pending: Record<string, number[]>): BoardSnapshot {
  let cards = board.cards;
  for (const [k, ls] of Object.entries(pending)) {
    const [path, sid] = [k.slice(0, k.lastIndexOf('#')), k.slice(k.lastIndexOf('#') + 1)];
    const c = path === board.path ? cards[sid] : undefined;
    if (!c) continue;
    const add = ls.filter(l => !c.labels.includes(l));
    if (!add.length) continue;
    if (cards === board.cards) cards = { ...cards };
    cards[sid] = { ...c, labels: [...c.labels, ...add] };
  }
  return cards === board.cards ? board : { ...board, cards };
}

export const pendingLabels = (path: string, id: number) => get().pending[pkey(path, id)];

/** 给卡片加标签：松手当帧就画上（pending 态），服务端确认后转正；失败时撤回并抛出错误。
    卡片已经有这个标签时返回 false，不发请求 */
export async function addLabel(id: number, labelId: number): Promise<boolean> {
  const { board, path } = get();
  const c = board?.cards[id];
  if (!board || !c || c.labels.includes(labelId)) return false;
  const k = pkey(path, id);
  const drop = (p: Record<string, number[]>) => {
    const ls = (p[k] ?? []).filter(l => l !== labelId);
    const next = { ...p };
    if (ls.length) next[k] = ls;
    else delete next[k];
    return next;
  };
  set(s => {
    const pending = { ...s.pending, [k]: [...(s.pending[k] ?? []), labelId] };
    return { pending, board: s.board && withPending(s.board, pending) };
  });
  try {
    await exec({ op: 'label', id, add: [labelId] });
    // 刷新后的快照里已经有这个标签：去掉 pending 只是换掉样式，位置不动
    set(s => ({ pending: drop(s.pending) }));
    return true;
  } catch (e) {
    // 撤回：去掉 pending，并从当前快照里拿掉（服务端没有加上）
    set(s => {
      const pending = drop(s.pending);
      const b = s.board;
      const cur = b?.path === path ? b.cards[id] : undefined;
      if (!b || !cur) return { pending };
      return { pending, board: { ...b, cards: { ...b.cards, [id]: { ...cur, labels: cur.labels.filter(l => l !== labelId) } } } };
    });
    throw e;
  }
}

/* ------------------------------ 标签顺序：乐观更新 ------------------------------ */

/** 正在保存的标签顺序：项目 → 标签 ID。保存期间 refresh 拿到的工作区也按它排，免得被旧顺序冲回去 */
const orderPending = new Map<string, number[]>();

/** 按给定顺序排项目的标签；没出现在 ids 里的（例如刚新建的）按原顺序排在后面，与服务端规则一致 */
function sortLabels<T extends { id: number }>(labels: T[], ids: number[]): T[] {
  const rank = new Map(ids.map((id, i) => [id, i]));
  return labels
    .map((l, i) => ({ l, k: rank.get(l.id) ?? ids.length + i }))
    .sort((a, b) => a.k - b.k)
    .map(x => x.l);
}

function withOrder(ws: Workspace): Workspace {
  if (!orderPending.size) return ws;
  return {
    ...ws,
    projects: ws.projects.map(p => {
      const ids = orderPending.get(p.key);
      return ids ? { ...p, labels: sortLabels(p.labels, ids) } : p;
    }),
  };
}

const labelsOf = (project: string) => get().ws?.projects.find(p => p.key === project)?.labels ?? [];

/** 调整项目里标签的顺序：当帧生效（侧栏和卡片上的标签一起变），失败时回到原来的顺序并抛出错误 */
export async function reorderLabels(project: string, ids: number[]) {
  const before = labelsOf(project).map(l => l.id);
  const apply = (order: number[] | null) => {
    if (order) orderPending.set(project, order);
    else orderPending.delete(project);
    const ws = get().ws;
    if (!ws) return;
    set({
      ws: {
        ...ws,
        projects: ws.projects.map(p => (p.key === project ? { ...p, labels: sortLabels(p.labels, order ?? before) } : p)),
      },
    });
  };
  apply(ids);
  try {
    await api.reorderLabels(project, ids);
    await refresh();
    if (orderPending.get(project) === ids) orderPending.delete(project);
  } catch (e) {
    // 只在没有更新的拖动覆盖它时回滚
    if (orderPending.get(project) === ids) apply(null);
    void refresh();
    throw e;
  }
}

/** 打开过的白板的最近一份快照：切回来时先显示它，不闪骨架，refresh 拿到新的再替换 */
const snapshots = new Map<string, BoardSnapshot>();

function switchTo(path: string) {
  save(path);
  flip.reset();
  clearLabelSel(); // 标签属于项目，换白板时清掉选中
  set(s => ({ path, board: snapshots.get(path) ?? null, detailId: null, detail: null, generation: s.generation + 1 }));
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
  const path = get().path;
  const k = `${path}#${id}`;
  set({ detailId: id, detail: get().detail?.id === id ? get().detail : details.get(k) ?? null, detailError: null });
  try {
    const d = await api.card(path, id);
    details.set(k, d);
    if (get().detailId === id && get().path === path) set({ detail: d });
  } catch (e) {
    if (get().detailId !== id || get().path !== path) return;
    // 卡片已经不在了（例如在另一个窗口删除）：关掉；否则留着摘要，告诉人详情没取回来
    if (!get().board?.cards[id]) set({ detailId: null, detail: null });
    else {
      const msg = (e as Error).message || '网络错误';
      set({ detailError: msg });
      toast({ type: 'error', title: `#${id} 的详情没有加载出来`, text: msg });
    }
  }
}
export function closeDetail() {
  set({ detailId: null, detail: null });
}

/* ------------------------------ 写 ------------------------------ */

/** 对当前白板执行卡片命令；失败抛 ApiError */
export async function exec(cmd: Cmd): Promise<CardDetail> {
  const r = await api.cmd(get().path, cmd);
  if (cmd.op !== 'delete') details.set(`${get().path}#${r.id}`, r);
  if (get().detailId === r.id && cmd.op !== 'delete') set({ detail: r, detailError: null });
  await refresh();
  return r;
}

/** 拖拽松手：先在本地落位，再等服务端确认。order：排序状态下拖动时整块的新顺序（与服务端规则一致：没列出的补在后面） */
export function moveLocally(id: number, to: ColumnKey, index: number, order?: number[]) {
  const b = get().board;
  if (!b) return;
  const cols = Object.fromEntries(Object.entries(b.cols).map(([k, v]) => [k, v.filter(x => x !== id)])) as BoardSnapshot['cols'];
  cols[to] = cols[to] ?? [];
  cols[to].splice(Math.min(index, cols[to].length), 0, id);
  if (order) {
    const cur = cols[to];
    cols[to] = [...new Set([...order.filter(x => cur.includes(x)), ...cur])];
  }
  const card = { ...b.cards[id], column: to };
  set({ board: { ...b, cols, cards: { ...b.cards, [id]: card } } });
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
