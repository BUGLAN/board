/* iOS 主屏安装版顶部的系统级模糊（default/main#627）。
   index.html 里贴顶的实色层（.ios-top）让 iOS 不再画模糊，但 iOS 只在某些时候重新认它：
   首次打开时还是模糊，人滑一下内容才变清楚，之后一直清楚到退出重开。
   这里替人做那一下：启动后、从后台切回时，让实色层高 1px 停两帧再还原；
   顶部下面的内容能滚动、又停在最上面时，悄悄滚 1px 再滚回（不画浮层滚动条）。
   只在 iOS 的 standalone 下做，Safari 里打开、Android、桌面什么都不做 */
import { scrollSilently } from './scrollbars';

/** iOS 主屏安装版：standalone + 只有 iOS 的 WebKit 认的 -webkit-touch-callout（和 index.html 里 .ios-top 的条件一样） */
export function isIosStandalone(w: Window = window): boolean {
  const standalone = w.matchMedia?.('(display-mode: standalone)').matches ?? false;
  const css = (w as Window & { CSS?: { supports?: (prop: string, value: string) => boolean } }).CSS;
  const ios = css?.supports?.('-webkit-touch-callout', 'none') ?? false;
  return standalone && ios;
}

/** 顶部下面第一个能上下滚动的元素（白板、各个页签的主内容）：从屏幕中间那个元素往上找 */
export function topScroller(doc: Document = document): HTMLElement | null {
  const w = doc.defaultView;
  if (!w || typeof doc.elementFromPoint !== 'function') return null;
  let el = doc.elementFromPoint(w.innerWidth / 2, w.innerHeight / 2) as HTMLElement | null;
  for (; el && el !== doc.body && el !== doc.documentElement; el = el.parentElement) {
    const oy = w.getComputedStyle(el).overflowY;
    if ((oy === 'auto' || oy === 'scroll') && el.scrollHeight > el.clientHeight) return el;
  }
  return null;
}

/** 让 iOS 重新认一次顶部实色层 */
export function nudgeTopBar(doc: Document = document, raf: (cb: () => void) => unknown = requestAnimationFrame) {
  const bar = doc.querySelector<HTMLElement>('.ios-top');
  if (bar) {
    bar.classList.add('is-nudge');
    raf(() => raf(() => bar.classList.remove('is-nudge')));
  }
  const el = topScroller(doc);
  // 人已经往下滚过（scrollTop > 0）就不动：这时模糊已经好了，也不能改人的位置
  if (el && el.scrollTop === 0) {
    scrollSilently(el, () => (el.scrollTop = 1));
    raf(() => {
      if (el.scrollTop === 1) scrollSilently(el, () => (el.scrollTop = 0));
    });
  }
}

/** 启动时调用一次。白板的数据要等服务端返回（慢的时候几秒），内容能滚之前那几下只动实色层，所以隔一会儿再各做一次 */
export function installIosTopBar(w: Window = window) {
  if (!isIosStandalone(w)) return;
  const run = () => nudgeTopBar(w.document);
  w.requestAnimationFrame(() => w.requestAnimationFrame(run));
  for (const ms of [800, 2500]) w.setTimeout(run, ms);
  // 从后台切回（iOS 回到主屏应用不一定重新加载页面）
  w.addEventListener('pageshow', run);
  w.document.addEventListener('visibilitychange', () => {
    if (w.document.visibilityState === 'visible') w.setTimeout(run, 50);
  });
}
