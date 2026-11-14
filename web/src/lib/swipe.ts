/* ==========================================================================
   行的横滑（未整理 ⇄ 稍后再做，default/main#303）：滑过一段距离松手就直接移走，不露按钮
   - 未整理里往右滑 → 收到「稍后再做」；稍后再做里往左滑 → 放回「未整理」（和页头两个标签的左右位置一致）
   - 只认一个方向：反方向拖不动；过了阈值继续拖会变「沉」，提示可以松手了
   - 松手时过了阈值，或者甩得够快（距离不太短），就算数
   纯函数，组件里的 useSwipe 用它们算位移和松手的结果
   ========================================================================== */

export type SwipeDir = 1 | -1;

/** 滑多远算数：行宽的 35%，至少 72px、最多 160px（桌面上的长行不用拖半个屏幕） */
export const swipeThreshold = (width: number) => Math.min(160, Math.max(72, width * 0.35));

/** 拖动中行的位移：mx 是手指（鼠标）横着走了多少；只往 dir 那边动，过了阈值后只跟一半多 */
export function swipeOffset(mx: number, dir: SwipeDir, width: number): number {
  const d = mx * dir;
  if (d <= 0) return 0;
  const t = swipeThreshold(width);
  return (d <= t ? d : t + (d - t) * 0.55) * dir;
}

/** 松手时算不算数：dx 是行的位移，v 是松手前的速度（px/ms，带方向） */
export function swipeCommits(dx: number, v: number, dir: SwipeDir, width: number): boolean {
  const d = dx * dir;
  return d >= swipeThreshold(width) || (d >= 40 && v * dir > 0.6);
}
