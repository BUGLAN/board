/* ==========================================================================
   白板设置「保留顺序」（PRD §10.3）：开着时每个状态块按自己的排序依据（Status.order，存在服务端）显示
   - 只改显示顺序：服务端存的优先级（cols）、board claim 的领取顺序、「下一张」都不变
   - 拖动只换状态、不改顺序：松手后卡片从落点补间到按排序依据的位置（lib/drag.ts）
   - 关着时按优先级显示，拖到哪就在哪
   ========================================================================== */
import { timing } from './schema';
import type { BoardSnapshot, Card, Label, Order, OrderBy, Status } from './types';

export interface OrderOption {
  by: OrderBy;
  name: string;
  /** 块标题上的简称（窄的块里用） */
  short: string;
  /** 方向的说法：[正序, 倒序]；按标签不分方向 */
  dirs?: [string, string];
  /** 选这个依据时默认的方向 */
  desc: boolean;
}

export const ORDERS: OrderOption[] = [
  { by: 'moved', name: '移入时间', short: '移入', dirs: ['新移入的在最后', '新移入的在最前'], desc: false },
  { by: 'id', name: '编号', short: '编号', dirs: ['小号在前', '大号在前'], desc: false },
  { by: 'updated', name: '最近更新', short: '更新', dirs: ['最近更新的在后', '最近更新的在前'], desc: true },
  { by: 'labels', name: '标签', short: '标签', desc: false },
  { by: 'work', name: '累计耗时', short: '耗时', dirs: ['耗时短的在前', '耗时长的在前'], desc: true },
];

const DEFAULT: Order = { by: 'moved', desc: false };

/** 这块的排序依据；旧服务端没有时按移入时间，新移入的在最后 */
export const orderOf = (s: Pick<Status, 'order'> | undefined): Order => {
  const o = s?.order;
  return o && ORDERS.some(x => x.by === o.by) ? { by: o.by, desc: o.by !== 'labels' && !!o.desc } : DEFAULT;
};

export const orderOption = (o: Order): OrderOption => ORDERS.find(x => x.by === o.by) ?? ORDERS[0];
/** 「编号」「移入时间」 */
export const orderName = (o: Order) => orderOption(o).name;
/** 方向的说法：「新移入的在最后」「大号在前」；按标签时是「按项目里的标签顺序」 */
export const orderDir = (o: Order) => {
  const x = orderOption(o);
  return x.dirs ? x.dirs[o.desc ? 1 : 0] : '按项目里的标签顺序';
};

/** 按排序依据重排一块的卡片。相同的按优先级顺序（稳定排序）；按标签时按项目标签顺序分组，没有标签的排最后 */
export function sortIds(ids: number[], cards: Record<string, Card>, o: Order, labels: Label[], now: number): number[] {
  const rank = new Map(labels.map((l, i) => [l.id, i]));
  const key = (c: Card): number => {
    switch (o.by) {
      case 'moved':
        return c.movedAt || c.createdAt;
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
  const sign = o.by !== 'labels' && o.desc ? -1 : 1;
  return ids
    .map((id, i) => ({ id, i, k: cards[id] ? key(cards[id]) : Infinity }))
    .sort((a, b) => (a.k === b.k ? a.i - b.i : a.k === Infinity ? 1 : b.k === Infinity ? -1 : sign * (a.k - b.k)) || a.i - b.i)
    .map(x => x.id);
}

/** 白板上各块显示的顺序：保留顺序开着时每块按自己的排序依据，关着时按优先级（「未整理」不在工作流里，不排） */
export function shownCols(b: BoardSnapshot, labels: Label[], now: number): BoardSnapshot['cols'] {
  if (!b.keepSorted) return b.cols;
  const out = { ...b.cols };
  for (const s of b.statuses) {
    const ids = b.cols[s.key];
    if (ids && ids.length > 1) out[s.key] = sortIds(ids, b.cards, orderOf(s), labels, now);
  }
  return out;
}
