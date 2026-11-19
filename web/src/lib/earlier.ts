/* ==========================================================================
   终点状态的「更早的 N 张」（PRD §10.3 状态块，default/main#485）
   - 服务端快照默认每个终点状态只带最近移入的 FINAL_RECENT 张，省略的张数在 omitted 里
   - 点「更早的 N 张 / +N」展开：取一次全量（store/data 的 loadFull），这个状态显示全部；同一位置变成「收起」
   - 收起只在界面上截断：已经取回的全量留着、不重新请求，只显示最近移入的 FINAL_RECENT 张（和服务端截断同一个规则：
     按最后一次移入 / 新建的时间，新的在前），其余的张数还写在「更早的 N 张」上
   - 筛选（文字、标签）时不截断：更早的卡片里匹配的也要看得到
   ========================================================================== */
import type { BoardSnapshot, Card } from './types';

/** 和服务端 FINAL_RECENT 一致：终点状态收起时显示的张数 */
export const FINAL_RECENT = 50;

/** 展开状态的键：这块白板的这个状态 */
export const earlierKey = (path: string, col: string) => `${path}\n${col}`;

/** 一块状态末尾的那一格：n 是更早没显示的张数（收起时），open 时是「收起」 */
export interface Earlier {
  n: number;
  open: boolean;
}

/** 收起时留下的那几张：最近移入的 FINAL_RECENT 张（movedAt 新的在前，一样时编号大的在前） */
export function recentIds(cards: Record<number, Card>, ids: number[]): Set<number> {
  const at = (id: number) => cards[id]?.movedAt ?? cards[id]?.createdAt ?? 0;
  return new Set([...ids].sort((a, b) => at(b) - at(a) || b - a).slice(0, FINAL_RECENT));
}

/**
 * 按展开状态截断看到的各列（纯函数，不改输入）：
 * - 快照省略了卡片（omitted）的状态：服务端已经截断，原样，末尾「更早的 N 张」（展开中、全量还没回来时 open）
 * - 全量快照里的终点状态、没展开、没在筛选、超过 FINAL_RECENT 张：只留最近移入的那几张，末尾「更早的 N 张」
 * - 展开着、超过 FINAL_RECENT 张、没在筛选：全部显示，末尾「收起」
 * 返回截断后的各列和每列末尾那一格（没有的不在里面）
 */
export function collapseCols(
  board: Pick<BoardSnapshot, 'cards' | 'cols' | 'omitted' | 'statuses' | 'path'>,
  shown: Record<string, number[]>,
  open: Record<string, boolean>,
  filtering: boolean,
): { cols: Record<string, number[]>; more: Record<string, Earlier> } {
  const cols: Record<string, number[]> = {};
  const more: Record<string, Earlier> = {};
  const finals = new Set(board.statuses.filter(s => s.final).map(s => s.key));
  for (const [k, ids] of Object.entries(shown)) {
    cols[k] = ids;
    const isOpen = !!open[earlierKey(board.path, k)];
    const omitted = board.omitted?.[k] ?? 0;
    if (omitted > 0) {
      more[k] = { n: omitted, open: isOpen };
      continue;
    }
    if (!finals.has(k) || filtering) continue;
    const all = board.cols[k] ?? [];
    if (all.length <= FINAL_RECENT) continue;
    if (isOpen) {
      more[k] = { n: 0, open: true };
      continue;
    }
    const keep = recentIds(board.cards, all);
    cols[k] = ids.filter(id => keep.has(id));
    more[k] = { n: all.length - keep.size, open: false };
  }
  return { cols, more };
}
