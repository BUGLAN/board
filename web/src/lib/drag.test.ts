/* 拖拽松手时幽灵的落点（settledRect）：状态块刚解冻、正从冻结时的位置补间到新位置（default/main#419） */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { settledRect } from './drag';

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
