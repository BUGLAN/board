/* ==========================================================================
   瀑布流：状态块里的卡片上下挨紧
   网格行高 1px，每张卡片按自身高度 + 间距跨行，位置由这里算好、直接写进 grid-row / grid-column。
   摆法和 CSS 网格的自动摆放（grid-auto-flow: row）一样：按 DOM 顺序逐个放到游标之后最先空出来的位置，
   所以起始行单调不减，阅读顺序仍是从左到右、从上到下（与 board ls 的优先级一致）。
   块（data-block 开头、data-block-end 结尾，例如同一会话的卡，default/main#319）：
   整块当成一个矩形摆，只占它的卡片需要的几栏（至少 MIN_BLOCK 宽，不超过总栏数），
   块里的卡片在这几栏里照样瀑布流，块的下沿取最低的那栏，块旁边可以并排别的卡片或块。
   间距读列表的 column-gap（CSS 里由 --gap 统一给出）。
   ========================================================================== */
import { useLayoutEffect, type RefObject } from 'react';

/** 块至少多宽（px）：磁贴一栏很窄，两三张成组时组头也要放得下机器名 */
export const MIN_BLOCK = 180;

export type Entry =
  | { kind: 'item'; rows: number }
  /** 块的开头：min 是块至少占几栏 */
  | { kind: 'head'; rows: number; min: number }
  | { kind: 'foot'; rows: number };

export interface Place {
  row: number;
  col: number;
  cols: number;
  rows: number;
}

/** 在 bottom（每栏已占到第几行）上按网格游标放一个 k 栏宽、h 行高的矩形：
    游标只往后走，游标这一行只能放在游标所在栏及其右边，往下的行哪一栏都行；取最上面、同一行取最左 */
function fit(bottom: number[], k: number, h: number, cur: { row: number; col: number }): { row: number; col: number } {
  let best = { row: Infinity, col: 0 };
  for (let c = 0; c + k <= bottom.length; c++) {
    let top = c >= cur.col ? cur.row : cur.row + 1;
    for (let j = c; j < c + k; j++) top = Math.max(top, bottom[j]);
    if (top < best.row) best = { row: top, col: c };
  }
  for (let j = best.col; j < best.col + k; j++) bottom[j] = best.row + h;
  cur.row = best.row;
  cur.col = best.col + k;
  return best;
}

/** 按 DOM 顺序摆放，T 栏；返回每一项的位置（行、栏都从 0 数）。块外的卡片是 item，块是 head、若干 item、foot */
export function pack(entries: Entry[], T: number): Place[] {
  const cols = Math.max(1, T);
  const out: Place[] = new Array(entries.length);
  const bottom = new Array<number>(cols).fill(0);
  const cur = { row: 0, col: 0 };
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i];
    if (e.kind !== 'head') {
      // 落单的结尾（没有开头）按普通的一格摆
      const p = fit(bottom, 1, e.rows, cur);
      out[i] = { ...p, cols: 1, rows: e.rows };
      continue;
    }
    // 块：先在 k 栏里摆好块内的卡片，得到块的高度，再整块找位置
    let end = i + 1;
    while (end < entries.length && entries[end].kind === 'item') end++;
    const foot = end < entries.length && entries[end].kind === 'foot' ? end : -1;
    const n = end - i - 1;
    const k = Math.min(cols, Math.max(1, n, e.min));
    const inner = new Array<number>(k).fill(e.rows);
    const ic = { row: e.rows, col: 0 };
    const rel: { row: number; col: number }[] = [];
    for (let j = i + 1; j < end; j++) rel.push(fit(inner, 1, entries[j].rows, ic));
    const footRows = foot >= 0 ? entries[foot].rows : 0;
    const h = Math.max(...inner) + footRows;
    const at = fit(bottom, k, h, cur);
    out[i] = { row: at.row, col: at.col, cols: k, rows: e.rows };
    rel.forEach((r, j) => {
      out[i + 1 + j] = { row: at.row + r.row, col: at.col + r.col, cols: 1, rows: entries[i + 1 + j].rows };
    });
    if (foot >= 0) out[foot] = { row: at.row + h - footRows, col: at.col, cols: k, rows: footRows };
    i = foot >= 0 ? foot : end - 1;
  }
  return out;
}

function apply(list: HTMLElement, after?: (list: HTMLElement) => void) {
  // 所在页面被隐藏（切到别的 Tab，display: none）时量不到高度：保持原样，切回来不用重排、不跳
  if (!list.offsetParent) return;
  const cs = getComputedStyle(list);
  const gap = parseFloat(cs.columnGap) || 0;
  const tracks = cs.gridTemplateColumns.split(/\s+/).map(parseFloat).filter(n => n > 0);
  const T = Math.max(1, tracks.length);
  const w = tracks[0] ?? list.clientWidth;
  const min = Math.ceil((MIN_BLOCK + gap) / (w + gap));
  const items = [...list.children] as HTMLElement[];
  // 先全部读，再全部写，只触发一次重排
  const entries: Entry[] = items.map(el => {
    const h = el.offsetHeight;
    const rows = h ? Math.max(1, Math.round(h + gap)) : 1;
    if (el.hasAttribute('data-block-end')) return { kind: 'foot', rows };
    if (el.hasAttribute('data-block')) return { kind: 'head', rows, min };
    return { kind: 'item', rows };
  });
  const places = pack(entries, T);
  items.forEach((el, i) => {
    const p = places[i];
    const col = `${p.col + 1} / span ${p.cols}`;
    const row = `${p.row + 1} / span ${p.rows}`;
    if (el.style.gridColumn !== col) el.style.gridColumn = col;
    if (el.style.gridRow !== row) el.style.gridRow = row;
  });
  after?.(list);
}

/** 每次渲染后同步排一次（在父组件的 FLIP play 之前），卡片尺寸变化时再排；after：排完之后（例如量会话框的高度） */
export function useMasonry(ref: RefObject<HTMLElement | null>, after?: (list: HTMLElement) => void) {
  useLayoutEffect(() => {
    if (ref.current) apply(ref.current, after);
  });
  useLayoutEffect(() => {
    const list = ref.current;
    if (!list || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => apply(list, after));
    const watch = () => {
      ro.disconnect();
      ro.observe(list);
      for (const el of list.children) ro.observe(el);
    };
    watch();
    const mo = new MutationObserver(watch);
    mo.observe(list, { childList: true });
    return () => {
      ro.disconnect();
      mo.disconnect();
    };
  }, [ref]); // eslint-disable-line react-hooks/exhaustive-deps
}
