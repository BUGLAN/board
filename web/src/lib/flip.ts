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
}

/** 放弃已记录的位置（例如切换白板时整体重建） */
export function reset() {
  first = null;
}

export function play() {
  const f = first;
  first = null;
  if (!f) return;
  const now = new Map<string, HTMLElement>();
  document.querySelectorAll<HTMLElement>('[data-flip]').forEach(el => now.set(el.dataset.flip!, el));
  for (const [id, snap] of f) {
    const el = now.get(id);
    if (!el) {
      if (!snap.el.isConnected && snap.rect.width) exitGhost(snap.el, snap.rect);
      continue;
    }
    const r = el.getBoundingClientRect();
    if (el.classList.contains('is-placeholder')) {
      if (Math.abs(snap.rect.top - r.top) > 1 || Math.abs(snap.rect.left - r.left) > 1)
        el.animate([{ opacity: 0, transform: 'scaleY(.6)' }, { opacity: 1, transform: 'none' }], { duration: 260, easing: 'cubic-bezier(.16,1,.3,1)' });
      continue;
    }
    if (!r.width || !snap.rect.width) continue;
    const dx = snap.rect.left - r.left;
    const dy = snap.rect.top - r.top;
    if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
    const cross = snap.col !== el.parentElement?.dataset.list;
    el.animate(
      cross
        ? [
            { transform: `translate(${dx}px, ${dy}px) scale(1.03) rotate(${dx > 0 ? 1.5 : -1.5}deg)`, zIndex: 5 },
            { transform: 'none', zIndex: 5 },
          ]
        : [{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }],
      { duration: cross ? 640 : 420, easing: SPRING },
    );
  }
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
