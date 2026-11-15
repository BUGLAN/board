export type TipSide = 'top' | 'bottom' | 'left' | 'right';

export interface TipAnchor {
  top: number;
  right: number;
  bottom: number;
  left: number;
  width: number;
  height: number;
}

export interface TipSize {
  width: number;
  height: number;
}

/** 提示离视口边缘至少留这么多，和触发器之间也留出空隙 */
const EDGE = 8;
const GAP = 8;
const SIDE_GAP = 10;

const clamp = (n: number, min: number, max: number) => Math.max(min, Math.min(max, n));

/** 提示浮层的摆放：优先放在指定的一侧，放不下就翻到对面，最后再夹在视口里。
    只算位置，不碰 DOM；组件和单元测试共用。 */
export function placeTip(
  anchor: TipAnchor,
  tip: TipSize,
  viewport: TipSize,
  opts: { pos?: string; align?: string } = {},
): { left: number; top: number; side: TipSide } {
  const maxX = Math.max(EDGE, viewport.width - tip.width - EDGE);
  const maxY = Math.max(EDGE, viewport.height - tip.height - EDGE);
  const centerX = anchor.left + anchor.width / 2 - tip.width / 2;
  const centerY = anchor.top + anchor.height / 2 - tip.height / 2;

  if (opts.pos === 'right') {
    const right = viewport.width - anchor.right - SIDE_GAP;
    const left = anchor.left - SIDE_GAP - tip.width;
    const side: TipSide =
      right >= tip.width ? 'right' : left >= EDGE ? 'left' : right >= left ? 'right' : 'left';
    return {
      left: Math.round(clamp(side === 'right' ? anchor.right + SIDE_GAP : anchor.left - SIDE_GAP - tip.width, EDGE, maxX)),
      top: Math.round(clamp(centerY, EDGE, maxY)),
      side,
    };
  }

  const below = viewport.height - anchor.bottom - GAP;
  const above = anchor.top - GAP;
  let side: TipSide = opts.pos === 'top' ? 'top' : 'bottom';
  const opposite: TipSide = side === 'top' ? 'bottom' : 'top';
  const space = (s: TipSide) => (s === 'bottom' ? below : above);
  if (space(side) < tip.height && space(opposite) >= tip.height) side = opposite;
  else if (space(side) < tip.height && space(opposite) < tip.height) side = space(side) >= space(opposite) ? side : opposite;

  const wantedX =
    opts.align === 'end' ? anchor.right - tip.width : opts.align === 'start' ? anchor.left : centerX;
  return {
    left: Math.round(clamp(wantedX, EDGE, maxX)),
    top: Math.round(clamp(side === 'bottom' ? anchor.bottom + GAP : anchor.top - GAP - tip.height, EDGE, maxY)),
    side,
  };
}
