import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Status } from '../lib/types';
import { clearSelect, startSelect, toggleCard, useBoardSel } from '../lib/boardSelect';
import { useUI } from '../store/ui';
import { Magnet, useBoardSelectionScope } from './BoardView';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class FakeResizeObserver {
  static all: FakeResizeObserver[] = [];
  readonly observed = new Set<Element>();

  constructor(readonly cb: ResizeObserverCallback) {
    FakeResizeObserver.all.push(this);
  }

  observe(el: Element) {
    this.observed.add(el);
  }

  unobserve(el: Element) {
    this.observed.delete(el);
  }

  disconnect() {
    this.observed.clear();
  }

  takeRecords(): ResizeObserverEntry[] {
    return [];
  }

  fire(targets: Element[]) {
    this.cb(targets.map(target => ({ target }) as ResizeObserverEntry), this as unknown as ResizeObserver);
  }
}

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

const original = globalThis.ResizeObserver;
const tile = (key: string): Status => ({
  key,
  name: key,
  color: 'todo',
  claim: false,
  lock: false,
  final: false,
  layout: { x: 0, w: 3, y: 0 },
  to: [],
});

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  FakeResizeObserver.all = [];
  globalThis.ResizeObserver = FakeResizeObserver as unknown as typeof ResizeObserver;
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  globalThis.ResizeObserver = original;
});

function SelectionScope({ path }: { path: string }) {
  useBoardSelectionScope(path);
  return null;
}

describe('白板批量选择的生命周期（default/main#516）', () => {
  beforeEach(() => {
    useUI.setState({ tab: 'board' });
    clearSelect();
  });

  it('首次渲染和白板内重绘不清空；换白板时清空', () => {
    act(() => {
      startSelect();
      toggleCard(4);
    });
    act(() => root.render(<SelectionScope path="p/a" />));
    expect(useBoardSel.getState()).toMatchObject({ on: true, ids: [4] });

    act(() => root.render(<SelectionScope path="p/a" />));
    expect(useBoardSel.getState()).toMatchObject({ on: true, ids: [4] });

    act(() => root.render(<SelectionScope path="p/b" />));
    expect(useBoardSel.getState()).toEqual({ on: false, ids: [], anchor: null });
  });

  it('离开白板 Tab 时清空', () => {
    act(() => root.render(<SelectionScope path="p/a" />));
    act(() => {
      startSelect();
      toggleCard(4);
    });
    expect(useBoardSel.getState()).toMatchObject({ on: true, ids: [4] });

    act(() => useUI.setState({ tab: 'inbox' }));
    expect(useBoardSel.getState()).toEqual({ on: false, ids: [], anchor: null });
  });
});

describe('Magnet 拖动卡片时的尺寸（default/main#419、#502）', () => {
  it('块高度在拖动中跟着占位变：目标块撑开，下面的块让开', () => {
    const top: Status = { ...tile('review'), layout: { x: 0, w: 3, y: 0 } };
    const below: Status = { ...tile('check'), layout: { x: 0, w: 3, y: 1 } };

    act(() => root.render(<Magnet path="test/main" statuses={[top, below]} frozen={false} render={() => <i />} />));
    const magnet = host.querySelector<HTMLElement>('.magnet')!;
    const a = host.querySelector<HTMLElement>('[data-tile="review"]')!;
    const b = host.querySelector<HTMLElement>('[data-tile="check"]')!;
    const ha = vi.spyOn(a, 'offsetHeight', 'get').mockReturnValue(100);
    vi.spyOn(b, 'offsetHeight', 'get').mockReturnValue(50);
    const watcher = FakeResizeObserver.all.find(ro => ro.observed.has(a))!;
    act(() => watcher.fire([a, b]));
    expect(magnet.style.height).toBe('166px');
    const y0 = b.style.transform;

    // 拖动中占位进了上面那块：它撑开，下面那块跟着下移
    act(() => root.render(<Magnet path="test/main" statuses={[top, below]} frozen render={() => <i />} />));
    ha.mockReturnValue(300);
    act(() => watcher.fire([a]));
    expect(magnet.style.height).toBe('366px');
    expect(b.style.transform).not.toBe(y0);
  });

  it('白板内容的高度在拖动中只增不减，松手后恢复（default/main#628）', () => {
    const column = tile('review');
    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen={false} render={() => <i />} />));
    const magnet = host.querySelector<HTMLElement>('.magnet')!;
    const el = host.querySelector<HTMLElement>('[data-tile="review"]')!;
    const h = vi.spyOn(el, 'offsetHeight', 'get').mockReturnValue(600);
    const watcher = FakeResizeObserver.all.find(ro => ro.observed.has(el))!;
    act(() => watcher.fire([el]));
    expect(magnet.style.height).toBe('604px');

    // 拖动中卡片离开这块，它变矮：白板的高度留在原来的值（变短会让滚在底部的白板 scrollTop 被动回收）
    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen render={() => <i />} />));
    h.mockReturnValue(480);
    act(() => watcher.fire([el]));
    expect(magnet.style.height).toBe('604px');
    // 撑开过之后再变矮，留在出现过的最大值
    h.mockReturnValue(700);
    act(() => watcher.fire([el]));
    expect(magnet.style.height).toBe('704px');
    h.mockReturnValue(480);
    act(() => watcher.fire([el]));
    expect(magnet.style.height).toBe('704px');

    // 松手当帧解冻：按实际内容算，底部不留空白；下一次拖动从这时的高度算起
    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen={false} render={() => <i />} />));
    expect(magnet.style.height).toBe('484px');
    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen render={() => <i />} />));
    expect(magnet.style.height).toBe('484px');
  });

  it('手机上块上下排列：拖动中用 min-height 撑住出现过的最大高度，松手后去掉（default/main#628）', () => {
    const mm = window.matchMedia;
    window.matchMedia = ((q: string) => ({ matches: true, media: q, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia;
    try {
      const column = tile('review');
      act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen={false} render={() => <i />} />));
      const magnet = host.querySelector<HTMLElement>('.magnet')!;
      expect(magnet.classList.contains('stacked')).toBe(true);
      const h = vi.spyOn(magnet, 'offsetHeight', 'get').mockReturnValue(900);
      expect(magnet.style.minHeight).toBe('');

      act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen render={() => <i />} />));
      expect(magnet.style.minHeight).toBe('900px');
      // 内容变矮时 min-height 撑着，量到的还是 900；撑开后跟着变大
      h.mockReturnValue(1000);
      act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen render={() => <i />} />));
      expect(magnet.style.minHeight).toBe('1000px');

      act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen={false} render={() => <i />} />));
      expect(magnet.style.minHeight).toBe('');
    } finally {
      window.matchMedia = mm;
    }
  });

  it('白板宽度在拖动中不变（滚动条出现 / 消失不重排），松手后补测', () => {
    const column = tile('done');
    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen={false} render={() => <i />} />));
    const magnet = host.querySelector<HTMLElement>('.magnet')!;
    const width = vi.spyOn(magnet, 'clientWidth', 'get').mockReturnValue(1000);
    const outer = FakeResizeObserver.all.find(ro => ro.observed.has(magnet))!;
    act(() => outer.fire([magnet]));
    const done = host.querySelector<HTMLElement>('[data-tile="done"]')!;
    const w0 = done.style.width;

    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen render={() => <i />} />));
    width.mockReturnValue(600);
    act(() => outer.fire([magnet]));
    expect(done.style.width).toBe(w0);

    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen={false} render={() => <i />} />));
    expect(done.style.width).not.toBe(w0);
  });

  it('自己重排时交给状态块的拖动、调宽入口不变：状态块（memo）不用跟着重画卡片', () => {
    const column = tile('done');
    const seen: unknown[] = [];
    const render = (_: Status, h: unknown) => {
      seen.push(h);
      return <i />;
    };

    act(() => root.render(<Magnet path="test/main" statuses={[column]} render={render} />));
    const done = host.querySelector<HTMLElement>('[data-tile="done"]')!;
    const height = vi.spyOn(done, 'offsetHeight', 'get').mockReturnValue(100);
    const watcher = FakeResizeObserver.all.find(ro => ro.observed.has(done))!;
    act(() => watcher.fire([done]));
    height.mockReturnValue(260);
    act(() => watcher.fire([done]));

    expect(host.querySelector<HTMLElement>('.magnet')!.style.height).toBe('264px');
    expect(seen.length).toBeGreaterThan(2);
    expect(new Set(seen).size).toBe(1);
  });
});
