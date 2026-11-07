/* ==========================================================================
   数据：工作区、当前白板（每个窗口可以看不同的白板）、打开的卡片详情
   - 服务端推送 rev 变化 → 重新取快照；新事件交给界面做高亮与提示
   - 写操作走 API，成功后立即刷新（不等推送）
   ========================================================================== */
import { create } from 'zustand';
import { api, connect, CLIENT_ID, type LiveStatus } from '../lib/api';
import * as flip from '../lib/flip';
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
  set({
    ws,
    board,
    rev: Math.max(ws.rev, board.rev),
    offset: ws.now - Date.now(),
    detail: st.detailId === detailId ? detail : st.detail,
  });
}

function switchTo(path: string) {
  save(path);
  flip.reset();
  set(s => ({ path, board: null, detailId: null, detail: null, generation: s.generation + 1 }));
}

/** 打开另一块白板 */
export function setPath(path: string) {
  if (path === get().path && get().board) return;
  switchTo(path);
  return refresh();
}

/* ------------------------------ 卡片详情 ------------------------------ */

export async function openDetail(id: number) {
  set({ detailId: id, detail: get().detail?.id === id ? get().detail : null });
  try {
    const d = await api.card(get().path, id);
    if (get().detailId === id) set({ detail: d });
  } catch {
    if (get().detailId === id) set({ detailId: null, detail: null });
  }
}
export function closeDetail() {
  set({ detailId: null, detail: null });
}

/* ------------------------------ 写 ------------------------------ */

/** 对当前白板执行卡片命令；失败抛 ApiError */
export async function exec(cmd: Cmd): Promise<CardDetail> {
  const r = await api.cmd(get().path, cmd);
  if (get().detailId === r.id && cmd.op !== 'delete') set({ detail: r });
  await refresh();
  return r;
}

/** 拖拽松手：先在本地落位，再等服务端确认 */
export function moveLocally(id: number, to: ColumnKey, index: number) {
  const b = get().board;
  if (!b) return;
  const cols = Object.fromEntries(Object.entries(b.cols).map(([k, v]) => [k, v.filter(x => x !== id)])) as BoardSnapshot['cols'];
  cols[to] = cols[to] ?? [];
  cols[to].splice(Math.min(index, cols[to].length), 0, id);
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
