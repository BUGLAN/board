/* 拖拽松手时幽灵的落点（settledRect）：状态块刚解冻、正从冻结时的位置补间到新位置（default/main#419） */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { settledRect, snapZones, zoneAt, type Zone } from './drag';

const rect = (left: number, top: number, width = 40, height = 24) => new DOMRect(left, top, width, height);

function tileWith(transform: string, current: string) {
  const tile = document.createElement('div');
  tile.dataset.tile = 'done';
  tile.style.transform = transform;
  const card = document.createElement('div');
  tile.appendChild(card);
  document.body.appendChild(tile);
  const real = window.getComputedStyle;
  vi.spyOn(window, 'getComputedStyle').mockImplementation((el, pseudo) => {
    if (el === tile) return { transform: current } as CSSStyleDeclaration;
    return real(el, pseudo);
  });
  return { tile, card };
}

afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = '';
});

describe('settledRect（松手时幽灵飞向哪里）', () => {
  it('块正在补间：按块的目标位置换算，而不是补间起点', () => {
    // 块要从 y=120 移到 y=200（行内写的是目标），现在还在起点
    const { card } = tileWith('translate(10px, 200px)', 'matrix(1, 0, 0, 1, 10, 120)');
    vi.spyOn(card, 'getBoundingClientRect').mockReturnValue(rect(300, 450));
    const r = settledRect(card);
    expect([r.left, r.top, r.width, r.height]).toEqual([300, 530, 40, 24]);
  });

  it('块往上补位、同时横移也算上', () => {
    const { card } = tileWith('translate(240px, 80px)', 'matrix(1, 0, 0, 1, 260, 153)');
    vi.spyOn(card, 'getBoundingClientRect').mockReturnValue(rect(500, 600));
    const r = settledRect(card);
    expect([r.left, r.top]).toEqual([480, 527]);
  });

  it('块没有在动：就是现在的位置', () => {
    const { card } = tileWith('translate(10px, 200px)', 'matrix(1, 0, 0, 1, 10, 200)');
    vi.spyOn(card, 'getBoundingClientRect').mockReturnValue(rect(300, 450));
    expect(settledRect(card).top).toBe(450);
  });

  it('手机上状态块不定位（没有行内 transform）、不在状态块里：原样返回', () => {
    const { card } = tileWith('', 'none');
    vi.spyOn(card, 'getBoundingClientRect').mockReturnValue(rect(12, 34));
    expect(settledRect(card).top).toBe(34);

    const loose = document.createElement('div');
    document.body.appendChild(loose);
    vi.spyOn(loose, 'getBoundingClientRect').mockReturnValue(rect(56, 78));
    expect(settledRect(loose).top).toBe(78);
  });
});

describe('落在哪一块：按开始拖动时的快照判定（default/main#502）', () => {
  const zone = (key: string, top: number, bottom: number, scroller: HTMLElement | null = null): Zone => ({
    key, left: 0, top, right: 300, bottom, scroller, scroll0: scroller?.scrollTop ?? 0,
  });

  it('块在拖动中撑开、挪动也不影响判定：只看快照', () => {
    const a = document.createElement('div');
    a.dataset.drop = 'review';
    a.getBoundingClientRect = () => rect(0, 0, 300, 100);
    const b = document.createElement('div');
    b.dataset.drop = 'check';
    b.getBoundingClientRect = () => rect(0, 112, 300, 100);
    const snap = snapZones([a, b]);
    // 之后上面那块撑开到 400 高、下面那块被推到 412：指针在 150 仍按快照判成下面那块
    a.getBoundingClientRect = () => rect(0, 0, 300, 400);
    b.getBoundingClientRect = () => rect(0, 412, 300, 100);
    expect(zoneAt(snap, 100, 150, 'review')).toBe('check');
    expect(zoneAt(snap, 100, 50, 'check')).toBe('review');
  });

  it('自动滚动后快照跟着平移', () => {
    const scroller = document.createElement('div');
    scroller.scrollTop = 0;
    const snap = [zone('review', 0, 100, scroller), zone('check', 112, 212, scroller)];
    expect(zoneAt(snap, 100, 150, undefined)).toBe('check');
    scroller.scrollTop = 100; // 往下滚了 100：下面那块现在在 12–112
    expect(zoneAt(snap, 100, 50, undefined)).toBe('check');
  });

  it('并排两块之间的竖缝里取左右两块，不判给上面整行宽的块（default/main#628）', () => {
    // 上面一整行（锁定的「进行中」），下面并排两块，中间 12px 的缝
    const snap: Zone[] = [
      { key: 'doing', left: 0, top: 0, right: 700, bottom: 100, scroller: null, scroll0: 0 },
      { key: 'review', left: 0, top: 112, right: 400, bottom: 600, scroller: null, scroll0: 0 },
      { key: 'done', left: 412, top: 112, right: 700, bottom: 600, scroller: null, scroll0: 0 },
    ];
    // 在块顶往下 5px、20px 的高度从左到右逐像素穿过竖缝：落点只从 review 换到 done 一次
    for (const y of [117, 132]) {
      let cur = 'review';
      const seen: string[] = [];
      for (let x = 396; x <= 416; x++) {
        cur = zoneAt(snap, x, y, cur) ?? cur;
        if (seen[seen.length - 1] !== cur) seen.push(cur);
      }
      expect(seen).toEqual(['review', 'done']);
      // 往回拖同样只换一次
      const back: string[] = [];
      for (let x = 416; x >= 396; x--) {
        cur = zoneAt(snap, x, y, cur) ?? cur;
        if (back[back.length - 1] !== cur) back.push(cur);
      }
      expect(back).toEqual(['done', 'review']);
    }
    // 指针真的到了上面那块（或更靠近它）仍然判给它
    expect(zoneAt(snap, 406, 90, 'review')).toBe('doing');
    expect(zoneAt(snap, 406, 103, 'doing')).toBe('doing');
  });

  it('指针在分界上抖动保持当前这块（迟滞）', () => {
    const snap = [zone('review', 0, 100), zone('check', 104, 200)];
    expect(zoneAt(snap, 100, 103, 'review')).toBe('review');
    expect(zoneAt(snap, 100, 101, 'check')).toBe('check');
  });
});
