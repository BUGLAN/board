/* ==========================================================================
   状态块的一次性排序：只改这块的显示顺序，不写服务端、不改优先级
   - 状态存在 store/ui（按「白板路径 + 状态」各自独立，不记住，刷新后回到优先级顺序）
   - 排序状态下在这块里拖动：按拖动后看到的顺序整块保存为优先级，排序状态结束（lib/drag.ts）
   ========================================================================== */
import { timing } from './schema';
import type { BoardSnapshot, Card, Label } from './types';

export type SortBy = 'id' | 'updated' | 'labels' | 'work';
export interface SortSpec {
  by: SortBy;
  /** 降序；按标签排序时不用 */
  desc: boolean;
}

export const SORTS: { by: SortBy; name: string; dir: boolean; desc: boolean }[] = [
  { by: 'id', name: '编号', dir: true, desc: false },
  { by: 'updated', name: '最近更新', dir: true, desc: true },
  { by: 'labels', name: '标签', dir: false, desc: false },
  { by: 'work', name: '累计耗时', dir: true, desc: true },
];

export const sortName = (s: SortSpec) => SORTS.find(x => x.by === s.by)?.name ?? '';

export const sortKey = (path: string, col: string) => `${path}|${col}`;

/** 按排序依据重排一块的卡片。相同的按优先级顺序（稳定排序）；按标签时按项目标签顺序分组，没有标签的排最后 */
export function sortIds(ids: number[], cards: Record<string, Card>, s: SortSpec, labels: Label[], now: number): number[] {
  const rank = new Map(labels.map((l, i) => [l.id, i]));
  const key = (c: Card): number => {
    switch (s.by) {
      case 'id':
        return c.id;
      case 'updated':
        return c.updatedAt;
      case 'work':
        return timing(c, now).total;
      case 'labels': {
        const rs = c.labels.map(l => rank.get(l)).filter((r): r is number => r != null);
        return rs.length ? Math.min(...rs) : Infinity;
      }
    }
  };
  const sign = s.by !== 'labels' && s.desc ? -1 : 1;
  return ids
    .map((id, i) => ({ id, i, k: cards[id] ? key(cards[id]) : Infinity }))
    .sort((a, b) => (a.k === b.k ? a.i - b.i : a.k === Infinity ? 1 : b.k === Infinity ? -1 : sign * (a.k - b.k)) || a.i - b.i)
    .map(x => x.id);
}

/** 白板上各块显示的顺序：排序中的块按排序依据，其余按优先级 */
export function shownCols(b: BoardSnapshot, sorts: Record<string, SortSpec>, labels: Label[], now: number): BoardSnapshot['cols'] {
  let out = b.cols;
  for (const [col, ids] of Object.entries(b.cols)) {
    const s = sorts[sortKey(b.path, col)];
    if (!s) continue;
    if (out === b.cols) out = { ...b.cols };
    out[col] = sortIds(ids, b.cards, s, labels, now);
  }
  return out;
}
