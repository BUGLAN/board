import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Status } from '../lib/types';
import { Magnet } from './BoardView';

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

describe('Magnet 拖动时冻结尺寸（default/main#419）', () => {
  it('块高度在拖动中反复变化也不重排，松手后按最终尺寸补测一次', () => {
    const column = tile('done');

    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen={false} render={() => <i />} />));
    const magnet = host.querySelector<HTMLElement>('.magnet')!;
    const done = host.querySelector<HTMLElement>('[data-tile="done"]')!;
    const height = vi.spyOn(done, 'offsetHeight', 'get').mockReturnValue(100);
    const watcher = FakeResizeObserver.all.find(ro => ro.observed.has(done))!;

    act(() => watcher.fire([done]));
    expect(magnet.style.height).toBe('104px');

    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen render={() => <i />} />));
    height.mockReturnValue(500);
    act(() => watcher.fire([done]));
    height.mockReturnValue(300);
    act(() => watcher.fire([done]));
    expect(magnet.style.height).toBe('104px');

    act(() => root.render(<Magnet path="test/main" statuses={[column]} frozen={false} render={() => <i />} />));
    expect(magnet.style.height).toBe('304px');
  });
});
