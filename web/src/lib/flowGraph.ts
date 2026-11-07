/* ==========================================================================
   工作流条的有向图排版（PRD §10.3）
   - 第一行是主线：从首个可领取状态出发，沿连线走到终点状态的那条路
   - 其余状态排在第二行，居中对齐到连向它的主线状态下面；放不下就往右挪
   - 连线是直角折线：主线相邻的直接横连；两行之间的竖线能直上直下就对齐，
     否则在两行中间走横线，横线按区间分层错开；主线里跳着连的从上方绕
   纯函数：只依赖状态和每个状态胶囊的宽度，不碰 DOM
   ========================================================================== */
import type { Status } from './types';

/** 状态胶囊的高度 */
export const CH = 30;
/** 主线相邻状态之间的间距（放箭头） */
const GAP = 34;
/** 第二行状态之间的最小间距 */
const BGAP = 18;
/** 横线之间的间距 */
const LANE = 7;

export type Pt = [number, number];
export interface Box {
  x: number;
  y: number;
  w: number;
  row: 0 | 1;
}
export interface EdgeGeo {
  id: string;
  a: string;
  b: string;
  pts: Pt[];
}
export interface Graph {
  boxes: Record<string, Box>;
  edges: EdgeGeo[];
  /** 「+ 添加状态」的位置，以及通往它的虚线 */
  add: { x: number; y: number; from: Pt | null };
  width: number;
  height: number;
}

export const edgeId = (a: string, b: string) => `${a}>${b}`;

/** 主线：从首个可领取状态出发，每步走向排在最前、还没走过的状态（优先不可领取的），直到终点或走不动 */
export function mainPath(list: Status[]): string[] {
  if (!list.length) return [];
  const get = (k: string) => list.find(s => s.key === k);
  const first = list.find(s => s.claim) ?? list[0];
  const path = [first.key];
  let cur = first;
  while (!cur.final) {
    const outs = list.filter(s => cur.to?.includes(s.key) && !path.includes(s.key));
    const nx = outs.find(s => !s.claim) ?? outs[0];
    if (!nx) break;
    path.push(nx.key);
    cur = get(nx.key)!;
  }
  return path;
}

/** 给横线分层：区间重叠的放在不同层 */
function lanes<T extends { x1: number; x2: number; lane?: number }>(items: T[]): number {
  const used: T[][] = [];
  for (const it of [...items].sort((p, q) => p.x2 - p.x1 - (q.x2 - q.x1))) {
    let i = 0;
    while ((used[i] ??= []).some(r => !(it.x2 + 6 < r.x1 || it.x1 - 6 > r.x2))) i++;
    used[i].push(it);
    it.lane = i;
  }
  return used.length;
}

type Kind = 'next' | 'top' | 'mid' | 'bottom';
interface E {
  a: string;
  b: string;
  kind: Kind;
  xa: number;
  xb: number;
  x1: number;
  x2: number;
  lane?: number;
}

export function layoutFlow(list: Status[], widths: Record<string, number>, addW: number): Graph {
  const W = (k: string) => widths[k] ?? 96;
  const has = (a: string, b: string) => !!list.find(s => s.key === a)?.to?.includes(b);
  const main = mainPath(list);
  const branch = list.filter(s => !main.includes(s.key)).map(s => s.key);
  const R: Record<string, { x: number; w: number; row: 0 | 1 }> = {};

  // 横向位置
  let x = 0;
  for (const k of main) {
    R[k] = { x, w: W(k), row: 0 };
    x += W(k) + GAP;
  }
  const addX = x;
  const want = (k: string) => {
    const src = main.find(m => has(m, k)) ?? main.find(m => has(k, m));
    return src ? Math.max(0, R[src].x + R[src].w / 2 - W(k) / 2) : addX;
  };
  let cursor = 0;
  for (const k of [...branch].sort((p, q) => want(p) - want(q))) {
    const bx = Math.max(want(k), cursor);
    R[k] = { x: bx, w: W(k), row: 1 };
    cursor = bx + W(k) + BGAP;
  }
  const cx = (k: string) => R[k].x + R[k].w / 2;

  // 连线分类
  const idx = (k: string) => main.indexOf(k);
  const E: E[] = [];
  for (const s of list)
    for (const t of s.to ?? []) {
      if (!R[s.key] || !R[t]) continue;
      const ra = R[s.key],
        rb = R[t];
      const kind: Kind =
        ra.row === 0 && rb.row === 0 ? (idx(t) === idx(s.key) + 1 ? 'next' : 'top') : ra.row !== rb.row ? 'mid' : 'bottom';
      E.push({ a: s.key, b: t, kind, xa: 0, xb: 0, x1: 0, x2: 0 });
    }

  // 端口：每个状态每条边上的连接点
  type P = { e: E; end: 'a' | 'b'; other: number };
  const ports: Record<string, P[]> = {};
  const side = (e: E, k: string) => (e.kind === 'top' ? 'top' : e.kind === 'bottom' ? 'bot' : R[k].row === 0 ? 'bot' : 'top');
  for (const e of E) {
    if (e.kind === 'next') continue;
    for (const end of ['a', 'b'] as const) {
      const k = e[end],
        o = end === 'a' ? e.b : e.a;
      (ports[`${k}:${side(e, k)}`] ??= []).push({ e, end, other: cx(o) });
    }
  }
  const setX = (p: P, v: number) => (p.end === 'a' ? (p.e.xa = v) : (p.e.xb = v));
  const otherX = (p: P) => (p.end === 'a' ? p.e.xb : p.e.xa);
  // 先放主线一侧（以及第二行底边）：按另一端的位置等分
  for (const [key, ps] of Object.entries(ports)) {
    const k = key.slice(0, key.lastIndexOf(':'));
    if (R[k].row === 1 && key.endsWith(':top')) continue;
    ps.sort((p, q) => p.other - q.other);
    ps.forEach((p, i) => setX(p, R[k].x + (R[k].w * (i + 1)) / (ps.length + 1)));
  }
  // 再放第二行顶边：能直上直下就对齐另一端，否则从靠近另一端的一侧依次取点
  for (const [key, ps] of Object.entries(ports)) {
    const k = key.slice(0, key.lastIndexOf(':'));
    if (!(R[k].row === 1 && key.endsWith(':top'))) continue;
    const l = R[k].x + 10,
      r = R[k].x + R[k].w - 10,
      used: number[] = [],
      rest: P[] = [];
    for (const p of ps) {
      const ox = otherX(p);
      if (ox >= l && ox <= r && !used.some(u => Math.abs(u - ox) < 9)) {
        setX(p, ox);
        used.push(ox);
      } else rest.push(p);
    }
    let li = l,
      ri = r;
    for (const p of rest.sort((p, q) => p.other - q.other)) {
      let px: number;
      if (p.other < cx(k)) {
        while (used.some(u => Math.abs(u - li) < 9)) li += 10;
        px = li;
        li += 10;
      } else {
        while (used.some(u => Math.abs(u - ri) < 9)) ri -= 10;
        px = ri;
        ri -= 10;
      }
      setX(p, px);
      used.push(px);
    }
  }
  for (const e of E) {
    e.x1 = Math.min(e.xa, e.xb);
    e.x2 = Math.max(e.xa, e.xb);
  }

  // 横线分层，算出高度
  const mid = E.filter(e => e.kind === 'mid' && Math.abs(e.xa - e.xb) > 1);
  const nMid = lanes(mid);
  const nTop = lanes(E.filter(e => e.kind === 'top'));
  const nBot = lanes(E.filter(e => e.kind === 'bottom'));
  const topH = nTop ? 10 + nTop * LANE : 0;
  const gapH = Math.max(24, 16 + nMid * LANE);
  const y0 = topH;
  const y1 = y0 + CH + gapH;
  const height = (branch.length ? y1 + CH : y0 + CH) + (nBot ? 10 + nBot * LANE : 0);
  const Y = (k: string) => (R[k].row === 0 ? y0 : y1);

  const edges: EdgeGeo[] = E.map(e => {
    const A = R[e.a],
      B = R[e.b];
    let pts: Pt[];
    if (e.kind === 'next') {
      pts = [
        [A.x + A.w + 3, y0 + CH / 2],
        [B.x - 4, y0 + CH / 2],
      ];
    } else if (e.kind === 'mid') {
      const down = A.row === 0;
      const ya = down ? y0 + CH + 2 : y1 - 2,
        yb = down ? y1 - 4 : y0 + CH + 4;
      if (Math.abs(e.xa - e.xb) <= 1)
        pts = [
          [e.xa, ya],
          [e.xb, yb],
        ];
      else {
        const ly = y0 + CH + (gapH - (nMid - 1) * LANE) / 2 + (e.lane ?? 0) * LANE;
        pts = [
          [e.xa, ya],
          [e.xa, ly],
          [e.xb, ly],
          [e.xb, yb],
        ];
      }
    } else if (e.kind === 'top') {
      const ly = y0 - 10 - (e.lane ?? 0) * LANE;
      pts = [
        [e.xa, y0 - 2],
        [e.xa, ly],
        [e.xb, ly],
        [e.xb, y0 - 4],
      ];
    } else {
      const base = y1 + CH,
        ly = base + 10 + (e.lane ?? 0) * LANE;
      pts = [
        [e.xa, base + 2],
        [e.xa, ly],
        [e.xb, ly],
        [e.xb, base + 4],
      ];
    }
    return { id: edgeId(e.a, e.b), a: e.a, b: e.b, pts };
  });

  const boxes: Record<string, Box> = {};
  for (const k in R) boxes[k] = { x: R[k].x, y: Y(k), w: R[k].w, row: R[k].row };
  const last = main[main.length - 1];
  return {
    boxes,
    edges,
    add: { x: addX, y: y0, from: last ? [R[last].x + R[last].w + 3, y0 + CH / 2] : null },
    width: Math.max(addX + addW, cursor),
    height,
  };
}

/** 直角折线，拐角处圆一下 */
export function pathD(pts: Pt[], r = 5): string {
  let d = `M${pts[0][0]},${pts[0][1]}`;
  for (let i = 1; i < pts.length - 1; i++) {
    const [x0, y0] = pts[i - 1],
      [x, y] = pts[i],
      [x1, y1] = pts[i + 1];
    const l0 = Math.hypot(x - x0, y - y0),
      l1 = Math.hypot(x1 - x, y1 - y);
    const rr = Math.min(r, l0 / 2, l1 / 2);
    if (rr < 0.5) {
      d += ` L${x},${y}`;
      continue;
    }
    d += ` L${x - ((x - x0) / l0) * rr},${y - ((y - y0) / l0) * rr} Q${x},${y} ${x + ((x1 - x) / l1) * rr},${y + ((y1 - y) / l1) * rr}`;
  }
  const [lx, ly] = pts[pts.length - 1];
  return `${d} L${lx},${ly}`;
}

/** 折线最长一段的中点（放删除按钮） */
export function midOf(pts: Pt[]): Pt {
  let best = -1,
    bi = 0;
  for (let i = 0; i < pts.length - 1; i++) {
    const l = Math.hypot(pts[i + 1][0] - pts[i][0], pts[i + 1][1] - pts[i][1]);
    if (l > best) {
      best = l;
      bi = i;
    }
  }
  return [(pts[bi][0] + pts[bi + 1][0]) / 2, (pts[bi][1] + pts[bi + 1][1]) / 2];
}
