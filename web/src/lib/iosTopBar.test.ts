/* iOS 主屏安装版顶部实色层（default/main#627）：只在 iOS standalone 下动，动一下实色层、内容停在最上面时悄悄滚 1px 再滚回 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { installIosTopBar, isIosStandalone, nudgeTopBar, topScroller } from './iosTopBar';

/** 把 rAF 回调攒起来，一帧一帧地跑 */
function frames() {
  let queue: (() => void)[] = [];
  const raf = (cb: () => void) => queue.push(cb);
  const step = () => {
    const now = queue;
    queue = [];
    for (const cb of now) cb();
  };
  return { raf, step };
}

function fakeWin(standalone: boolean, ios: boolean) {
  return {
    matchMedia: (q: string) => ({ matches: q === '(display-mode: standalone)' && standalone }),
    CSS: { supports: (p: string, v: string) => p === '-webkit-touch-callout' && v === 'none' && ios },
  } as unknown as Window;
}

const made: HTMLElement[] = [];
function add<T extends HTMLElement>(el: T) {
  document.body.appendChild(el);
  made.push(el);
  return el;
}

function scrollArea(scrollable: boolean) {
  const el = add(document.createElement('div'));
  el.style.overflowY = 'auto';
  Object.defineProperty(el, 'scrollHeight', { configurable: true, value: scrollable ? 1000 : 100 });
  Object.defineProperty(el, 'clientHeight', { configurable: true, value: 100 });
  const inner = document.createElement('div');
  el.appendChild(inner);
  document.elementFromPoint = () => inner;
  return el;
}

afterEach(() => {
  for (const el of made.splice(0)) el.remove();
  // jsdom 没有 elementFromPoint：测完删掉
  delete (document as { elementFromPoint?: unknown }).elementFromPoint;
});

describe('isIosStandalone', () => {
  it('standalone 而且是 iOS 才算', () => {
    expect(isIosStandalone(fakeWin(true, true))).toBe(true);
    expect(isIosStandalone(fakeWin(false, true))).toBe(false); // Safari 里打开
    expect(isIosStandalone(fakeWin(true, false))).toBe(false); // Android、桌面装的 PWA
    expect(isIosStandalone({} as Window)).toBe(false);
  });
});

describe('nudgeTopBar', () => {
  it('实色层高 1px 停两帧再还原', () => {
    const bar = add(document.createElement('div'));
    bar.className = 'ios-top';
    const f = frames();
    nudgeTopBar(document, f.raf);
    expect(bar.classList.contains('is-nudge')).toBe(true);
    f.step();
    expect(bar.classList.contains('is-nudge')).toBe(true);
    f.step();
    expect(bar.classList.contains('is-nudge')).toBe(false);
  });

  it('内容能滚、停在最上面：滚 1px，下一帧滚回', () => {
    const el = scrollArea(true);
    const f = frames();
    nudgeTopBar(document, f.raf);
    expect(el.scrollTop).toBe(1);
    f.step();
    expect(el.scrollTop).toBe(0);
  });

  it('人已经往下滚过：不改位置', () => {
    const el = scrollArea(true);
    el.scrollTop = 300;
    const f = frames();
    nudgeTopBar(document, f.raf);
    f.step();
    expect(el.scrollTop).toBe(300);
  });

  it('滚回之前人自己滚了：不拉回去', () => {
    const el = scrollArea(true);
    const f = frames();
    nudgeTopBar(document, f.raf);
    el.scrollTop = 80;
    f.step();
    expect(el.scrollTop).toBe(80);
  });

  it('内容不能滚、没有 elementFromPoint 时只动实色层', () => {
    scrollArea(false);
    expect(topScroller()).toBeNull();
    delete (document as { elementFromPoint?: unknown }).elementFromPoint;
    expect(topScroller()).toBeNull();
    expect(() => nudgeTopBar(document, frames().raf)).not.toThrow();
  });
});

describe('installIosTopBar', () => {
  it('不是 iOS 主屏安装版时什么都不做', () => {
    const w = { ...fakeWin(true, false), requestAnimationFrame: vi.fn(), setTimeout: vi.fn(), addEventListener: vi.fn() } as unknown as Window;
    installIosTopBar(w);
    expect(w.requestAnimationFrame).not.toHaveBeenCalled();
    expect(w.setTimeout).not.toHaveBeenCalled();
    expect(w.addEventListener).not.toHaveBeenCalled();
  });

  it('iOS 主屏安装版：启动后、隔一会儿、切回前台时各动一次', () => {
    const w = {
      ...fakeWin(true, true),
      document,
      requestAnimationFrame: vi.fn(),
      setTimeout: vi.fn(),
      addEventListener: vi.fn(),
    } as unknown as Window;
    const docListen = vi.spyOn(document, 'addEventListener');
    installIosTopBar(w);
    expect(w.requestAnimationFrame).toHaveBeenCalledTimes(1);
    expect((w.setTimeout as ReturnType<typeof vi.fn>).mock.calls.map(c => c[1])).toEqual([800, 2500]);
    expect((w.addEventListener as ReturnType<typeof vi.fn>).mock.calls.map(c => c[0])).toEqual(['pageshow']);
    expect(docListen.mock.calls.map(c => c[0])).toContain('visibilitychange');
    docListen.mockRestore();
  });
});
