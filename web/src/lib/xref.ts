/* ==========================================================================
   描述和评论里的卡片引用（`#12`、`项目/白板#12`）：查这张卡在不在、标题和状态（悬停提示用）
   - 当前白板从快照里取（一直是最新的）
   - 其他白板、快照里没有的（终点状态更早的）问一次服务端，结果缓存起来；不存在的记成 null
   - 查到了通知界面重画（markdown 的缓存按引用签名分开，见 lib/markdown.ts）
   ========================================================================== */
import { api, ApiError } from './api';
import type { RefInfo, Refs } from './markdown';
import { INBOX_NAME } from './schema';
import { useData } from '../store/data';

type Info = RefInfo | null; // null：卡片不存在（白板不存在也算）
const cache = new Map<string, Info>();
const asked = new Set<string>();
let version = 0;
const subs = new Set<() => void>();

function bump() {
  version++;
  for (const fn of subs) fn();
}

/** 卡片引用查到了新的结果时通知（Markdown 组件订阅它重画） */
export function subscribeRefs(fn: () => void) {
  subs.add(fn);
  return () => {
    subs.delete(fn);
  };
}
export const refsVersion = () => version;

/** 工作区里有没有这块白板 */
function hasBoard(path: string): boolean {
  const ws = useData.getState().ws;
  return !!ws?.projects.some(p => p.boards.some(b => b.path === path));
}

/** 状态标识 → 名称（未整理、各工作流状态） */
function statusName(path: string, column: string): string {
  if (column === 'inbox') return INBOX_NAME;
  const ws = useData.getState().ws;
  for (const p of ws?.projects ?? []) {
    const b = p.boards.find(x => x.path === path);
    if (b) return b.statuses.find(s => s.key === column)?.name ?? column;
  }
  return column;
}

/** 问一次服务端（同一张卡片只问一次；网络错误不算数，下次再问） */
function load(path: string, id: number) {
  const key = `${path}#${id}`;
  if (asked.has(key)) return;
  asked.add(key);
  api.card(path, id).then(
    d => {
      cache.set(key, { title: d.title, status: statusName(path, d.column) });
      bump();
    },
    e => {
      if (e instanceof ApiError && e.code === 'NOT_FOUND') {
        cache.set(key, null);
        bump();
      } else asked.delete(key);
    },
  );
}

/** 一块白板上的卡片：info、null（不存在）、undefined（还查不到） */
export function findRef(board: string, id: number): Info | undefined {
  const { path, board: snap } = useData.getState();
  if (board === path) {
    const c = snap?.cards[id];
    if (c) return { title: c.title, status: statusName(path, c.column) };
  }
  const key = `${board}#${id}`;
  if (cache.has(key)) return cache.get(key);
  if (!hasBoard(board)) {
    cache.set(key, null); // 白板删了或不存在
    return null;
  }
  load(board, id);
  return undefined;
}

/** 给 markdown 用的解析器：`#12` 落在当前白板 */
export function refsFor(path: string): Refs {
  return { path, find: findRef };
}
