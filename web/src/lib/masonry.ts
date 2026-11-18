/* ==========================================================================
   瀑布流：状态块里的卡片上下挨紧
   网格行高 1px，每张卡片按自身高度 + 间距跨行，位置由这里算好、直接写进 grid-row / grid-column。
   摆法和 CSS 网格的自动摆放（grid-auto-flow: row）一样：按 DOM 顺序逐个放到游标之后最先空出来的位置，
   所以起始行单调不减，阅读顺序仍是从左到右、从上到下（与 board ls 的优先级一致）。
   块（data-block 开头、data-block-end 结尾，例如同一会话的卡，default/main#319）：
   整块当成一个矩形摆，只占它的卡片需要的几栏（至少 MIN_BLOCK 宽，不超过总栏数），
   块里的卡片在这几栏里照样瀑布流，块的下沿取最低的那栏，块旁边可以并排别的卡片或块；
   块里的卡一行就排得下时拉成一样高（取最高的那张），并排的卡上下沿都对齐（default/main#462）。
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
  /** 块里排成一行的卡：被拉到和这一行最高的那张一样高（rows 已经是拉高后的） */
  even?: boolean;
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
    // 一行排得下、高度又不一样：每张都按最高的那张占行，上下沿对齐（本来一样高的不动，单行、磁贴一般就是）
    const rs = entries.slice(i + 1, end).map(x => x.rows);
    const even = n > 1 && n <= k && Math.min(...rs) !== Math.max(...rs) ? Math.max(...rs) : 0;
    for (let j = i + 1; j < end; j++) rel.push(fit(inner, 1, even || entries[j].rows, ic));
    const footRows = foot >= 0 ? entries[foot].rows : 0;
    const h = Math.max(...inner) + footRows;
    const at = fit(bottom, k, h, cur);
    out[i] = { row: at.row, col: at.col, cols: k, rows: e.rows };
    rel.forEach((r, j) => {
      out[i + 1 + j] = even
        ? { row: at.row + r.row, col: at.col + r.col, cols: 1, rows: even, even: true }
        : { row: at.row + r.row, col: at.col + r.col, cols: 1, rows: entries[i + 1 + j].rows };
    });
    if (foot >= 0) out[foot] = { row: at.row + h - footRows, col: at.col, cols: k, rows: footRows };
    i = foot >= 0 ? foot : end - 1;
  }
  return out;
}

/** 计算样式里的 grid-template-columns（每栏一个 px 值）→ 显式栏的宽度。
    卡片的 grid-column 是按上一次的栏数写的：列表变窄（拖窄状态块、窗口变窄、宽度过渡）后，
    排在后面几栏的卡片超出 auto-fill 排出的栏，网格为它们生成隐式栏，计算样式把隐式栏也列出来。
    照着它数栏，栏数就停在旧值，卡片一直排在状态块外面（default/main#387）。
    CSS 里 .col-list 的 grid-auto-columns 是 0：隐式栏宽 0，这里只留宽度大于 0 的栏 */
export function explicitTracks(template: string): number[] {
  return template.split(/\s+/).map(parseFloat).filter(n => n > 0);
}

/** 列表现在的栏数 T、每栏宽 w、栏间距 gap（摆放和拖动插入位置都按它算） */
export function gridTracks(list: HTMLElement, cs: CSSStyleDeclaration = getComputedStyle(list)) {
  const gap = parseFloat(cs.columnGap) || 0;
  const tracks = explicitTracks(cs.gridTemplateColumns);
  return { T: Math.max(1, tracks.length), w: tracks[0] ?? list.clientWidth, gap };
}

/** 一项占几行：高度 + 间距。块的开头再加上它的 margin-bottom：CSS 给组头负的 margin-bottom，
    组头和块里第一行卡片之间就比卡片间距近，第一行的卡片一起上移、上沿对齐。
    以前用 `.sess-head + .card` 的负 margin-top，只挪了组里第一张，同一行的其他卡低一截（default/main#462） */
export function measure(el: HTMLElement, gap: number, min: number): Entry {
  const h = el.offsetHeight;
  const rows = (extra = 0) => (h ? Math.max(1, Math.round(h + gap + extra)) : 1);
  if (el.hasAttribute('data-block-end')) return { kind: 'foot', rows: rows() };
  if (el.hasAttribute('data-block')) return { kind: 'head', rows: rows(parseFloat(getComputedStyle(el).marginBottom) || 0), min };
  return { kind: 'item', rows: rows() };
}

function apply(list: HTMLElement, after?: (list: HTMLElement) => void) {
  // 所在页面被隐藏（切到别的 Tab，display: none）时量不到高度：保持原样，切回来不用重排、不跳
  if (!list.offsetParent) return;
  const { T, w, gap } = gridTracks(list);
  const min = Math.ceil((MIN_BLOCK + gap) / (w + gap));
  const items = [...list.children] as HTMLElement[];
  // 上一次拉齐时写的 min-height 先去掉，量卡片本来的高度（不然拉高过的卡再也缩不回去）
  for (const el of items) {
    if (!el.hasAttribute('data-even')) continue;
    el.style.minHeight = '';
    el.removeAttribute('data-even');
  }
  // 先全部读，再全部写，只触发一次重排（有拉齐过的卡时多一次：先去掉 min-height 再量）
  const entries: Entry[] = items.map(el => measure(el, gap, min));
  const places = pack(entries, T);
  items.forEach((el, i) => {
    const p = places[i];
    const col = `${p.col + 1} / span ${p.cols}`;
    const row = `${p.row + 1} / span ${p.rows}`;
    if (el.style.gridColumn !== col) el.style.gridColumn = col;
    if (el.style.gridRow !== row) el.style.gridRow = row;
    if (p.even) {
      // offsetHeight 四舍五入成整数，最高那张实际可能多出不到 0.5px：补上，整行一样高（多出的落在下面的间距里）
      el.style.minHeight = `${p.rows - gap + 0.5}px`;
      el.setAttribute('data-even', '');
    }
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
