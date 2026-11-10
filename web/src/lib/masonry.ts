/* ==========================================================================
   瀑布流：状态块里的卡片上下挨紧
   网格行高 1px，每张卡片按自身高度 + 间距跨行（grid-row-end: span N）。
   网格按 DOM 顺序逐个放到最先空出来的位置，所以起始行单调不减，
   阅读顺序仍是从左到右、从上到下（与 board ls 的优先级一致）。
   间距读列表的 column-gap（CSS 里由 --gap 统一给出）。
   ========================================================================== */
import { useLayoutEffect, type RefObject } from 'react';

function apply(list: HTMLElement) {
  // 所在页面被隐藏（切到别的 Tab，display: none）时量不到高度：保持原样，切回来不用重排、不跳
  if (!list.offsetParent) return;
  const gap = parseFloat(getComputedStyle(list).columnGap) || 0;
  const items = [...list.children] as HTMLElement[];
  // 先全部读，再全部写，只触发一次重排
  const spans = items.map(el => {
    const h = el.offsetHeight;
    return h ? `span ${Math.max(1, Math.round(h + gap))}` : '';
  });
  items.forEach((el, i) => {
    if (el.style.gridRowEnd !== spans[i]) el.style.gridRowEnd = spans[i];
  });
}

/** 每次渲染后同步排一次（在父组件的 FLIP play 之前），卡片尺寸变化时再排 */
export function useMasonry(ref: RefObject<HTMLElement | null>) {
  useLayoutEffect(() => {
    if (ref.current) apply(ref.current);
  });
  useLayoutEffect(() => {
    const list = ref.current;
    if (!list || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => apply(list));
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
  }, [ref]);
}
