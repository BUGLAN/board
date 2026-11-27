/** 勾选共用的纯函数：并集、Shift 连选的范围 */

export const unionIds = (a: readonly number[], b: readonly number[]) => [...new Set([...a, ...b])];

/** 从起点到这张，按列表里看到的顺序，前后都行；起点或这张不在这个列表里时返回 null */
export function rangeIds(order: readonly number[], anchor: number | null, id: number): number[] | null {
  const a = anchor == null ? -1 : order.indexOf(anchor);
  const b = order.indexOf(id);
  if (a < 0 || b < 0) return null;
  return order.slice(Math.min(a, b), Math.max(a, b) + 1);
}
