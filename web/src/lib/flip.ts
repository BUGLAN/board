/* ==========================================================================
   FLIP：卡片位置变化（排序、换状态、别处的修改）用弹簧补间；被删除的卡片在原位淡出
   用法：数据变更前 capture()，渲染完成后（useLayoutEffect）play()
   需要动画的元素带 data-flip="唯一标识"，父元素带 data-list="状态"
   ========================================================================== */
import { reduceMotion } from './util';

interface Snap {
  rect: DOMRect;
  el: HTMLElement;
  col: string | undefined;
}

const SPRING = 'cubic-bezier(.22, 1.18, .36, 1)';
let first: Map<string, Snap> | null = null;
/** 记录的时间：记下后一直没有重画（例如撤掉的补丁没改变画面），过一会儿就作废，不拿旧位置去补间之后无关的变化 */
let firstAt = 0;
const STALE = 1000;

export function capture() {
  if (reduceMotion()) {
    first = null;
    return;
  }
  const m = new Map<string, Snap>();
  document.querySelectorAll<HTMLElement>('[data-flip]').forEach(el => {
    m.set(el.dataset.flip!, { rect: el.getBoundingClientRect(), el, col: el.parentElement?.dataset.list });
  });
  first = m;
  firstAt = performance.now();
}

/** 放弃已记录的位置（例如切换白板时整体重建） */
export function reset() {
  first = null;
}

/** 补间前后都在窗口外面（上下左右各放宽 MARGIN）的卡片不补间：看不到，直接到位 */
const MARGIN = 64;
const offscreen = (r: DOMRect) =>
  r.bottom < -MARGIN || r.top > window.innerHeight + MARGIN || r.right < -MARGIN || r.left > window.innerWidth + MARGIN;

/** 分两遍：先读完所有元素现在的位置，再统一启动动画。
    以前是逐个「读位置 → 启动动画」：每启动一个动画样式就脏一次，下一个元素的读取又强制重算一遍，
    终点状态画出几百张磁贴、占位一进一出全体让位时，一次渲染里要强制重算几百遍（default/main#629）。
    正在补间的元素每帧都要重算样式（实测每张磁贴每帧约 0.15ms，470 张同时补间时一帧 40–100ms），
    所以窗口外面看不到的不启动动画 */
export function play() {
  const f = first;
  first = null;
  if (!f || performance.now() - firstAt > STALE) return;
  const now = new Map<string, HTMLElement>();
  document.querySelectorAll<HTMLElement>('[data-flip]').forEach(el => now.set(el.dataset.flip!, el));

  // 第一遍：只读
  const moves: { el: HTMLElement; keyframes: Keyframe[]; options: KeyframeAnimationOptions }[] = [];
  const exits: Snap[] = [];
  for (const [id, snap] of f) {
    const el = now.get(id);
    if (!el) {
      if (!snap.el.isConnected && snap.rect.width && !offscreen(snap.rect)) exits.push(snap);
      continue;
    }
    const r = el.getBoundingClientRect();
    if (offscreen(r) && offscreen(snap.rect)) continue;
    if (el.classList.contains('is-placeholder')) {
      if (Math.abs(snap.rect.top - r.top) > 1 || Math.abs(snap.rect.left - r.left) > 1)
        moves.push({ el, keyframes: [{ opacity: 0, transform: 'scaleY(.6)' }, { opacity: 1, transform: 'none' }], options: { duration: 260, easing: 'cubic-bezier(.16,1,.3,1)' } });
      continue;
    }
    if (!r.width || !snap.rect.width) continue;
    const dx = snap.rect.left - r.left;
    const dy = snap.rect.top - r.top;
    if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
    const cross = snap.col !== el.parentElement?.dataset.list;
    moves.push({
      el,
      keyframes: cross
        ? [
            { transform: `translate(${dx}px, ${dy}px) scale(1.03) rotate(${dx > 0 ? 1.5 : -1.5}deg)`, zIndex: 5 },
            { transform: 'none', zIndex: 5 },
          ]
        : [{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }],
      options: { duration: cross ? 640 : 420, easing: SPRING },
    });
  }

  // 第二遍：只写
  for (const m of moves) m.el.animate(m.keyframes, m.options);
  for (const s of exits) exitGhost(s.el, s.rect);
}

function exitGhost(node: HTMLElement, r: DOMRect) {
  const g = node;
  Object.assign(g.style, {
    position: 'fixed', left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`,
    margin: '0', zIndex: '60', pointerEvents: 'none',
  });
  document.body.appendChild(g);
  g.animate(
    [
      { opacity: 1, transform: 'none', filter: 'blur(0)' },
      { opacity: 0, transform: 'scale(.86) translateY(8px)', filter: 'blur(6px)' },
    ],
    { duration: 420, easing: 'cubic-bezier(.4,0,.2,1)', fill: 'forwards' },
  ).onfinish = () => g.remove();
}
