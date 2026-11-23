import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as flip from './flip';

/* jsdom 不排版：位置用假的 getBoundingClientRect，动画用假的 animate。窗口是 1024 × 768 */
const rect = (left: number, top: number, w = 100, h = 40) =>
  ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} }) as DOMRect;

let log: string[];
let list: HTMLElement;

function card(id: string, at: [number, number], parent: HTMLElement = list) {
  const el = document.createElement('article');
  el.dataset.flip = id;
  parent.appendChild(el);
  let cur = at;
  el.getBoundingClientRect = () => {
    log.push(`read:${id}`);
    return rect(cur[0], cur[1]);
  };
  const animate = vi.fn((..._a: unknown[]) => {
    log.push(`animate:${id}`);
    return {} as Animation;
  });
  el.animate = animate as unknown as typeof el.animate;
  return { el, animate, move: (to: [number, number]) => (cur = to) };
}

beforeEach(() => {
  log = [];
  list = document.createElement('div');
  list.dataset.list = 'todo';
  document.body.appendChild(list);
  flip.reset();
});
afterEach(() => {
  document.body.innerHTML = '';
});

describe('FLIP play（default/main#629）', () => {
  it('先读完所有元素现在的位置，再统一启动动画', () => {
    const cards = ['a', 'b', 'c'].map((id, i) => card(id, [0, i * 50]));
    flip.capture();
    cards.forEach((c, i) => c.move([0, i * 50 + 50]));
    log = [];
    flip.play();
    expect(log.filter(x => x.startsWith('animate'))).toEqual(['animate:a', 'animate:b', 'animate:c']);
    // 读写不交替：最后一次读在第一次启动动画之前
    expect(log.map(x => x.split(':')[0]).join(' ')).toBe('read read read animate animate animate');
    expect(cards[0].animate.mock.calls[0][0]).toEqual([{ transform: 'translate(0px, -50px)' }, { transform: 'none' }]);
  });

  it('补间前后都在窗口外面的卡片不启动动画；有一头在窗口里的照常补间', () => {
    const below = card('below', [0, 2000]);
    const enters = card('enters', [0, 2000]);
    const leaves = card('leaves', [0, 300]);
    const right = card('right', [3000, 300]);
    flip.capture();
    below.move([0, 2100]);
    enters.move([0, 300]);
    leaves.move([0, 2000]);
    right.move([3000, 400]);
    flip.play();
    expect(below.animate).not.toHaveBeenCalled();
    expect(right.animate).not.toHaveBeenCalled();
    expect(enters.animate).toHaveBeenCalledTimes(1);
    expect(leaves.animate).toHaveBeenCalledTimes(1);
  });

  it('位置没变的不补间；换了状态的带倾斜和层级；占位换位置时淡入', () => {
    const still = card('still', [0, 0]);
    const cross = card('cross', [0, 100]);
    const ph = card('ph', [0, 200]);
    ph.el.classList.add('is-placeholder');
    flip.capture();
    const other = document.createElement('div');
    other.dataset.list = 'done';
    document.body.appendChild(other);
    other.appendChild(cross.el);
    cross.move([300, 100]);
    ph.move([0, 300]);
    flip.play();
    expect(still.animate).not.toHaveBeenCalled();
    expect(cross.animate.mock.calls[0][1]).toMatchObject({ duration: 640 });
    expect((cross.animate.mock.calls[0][0] as Keyframe[])[0].zIndex).toBe(5);
    expect((ph.animate.mock.calls[0][0] as Keyframe[])[0]).toMatchObject({ opacity: 0 });
  });

  it('被删掉的卡片在原位淡出；删掉时在窗口外面的不管', () => {
    const seen = card('seen', [0, 100]);
    const far = card('far', [0, 5000]);
    flip.capture();
    seen.el.remove();
    far.el.remove();
    flip.play();
    expect(seen.el.parentElement).toBe(document.body);
    expect(seen.el.style.position).toBe('fixed');
    expect(seen.animate).toHaveBeenCalledTimes(1);
    expect(far.el.isConnected).toBe(false);
    expect(far.animate).not.toHaveBeenCalled();
  });

  it('没有 capture、或者记下太久（1 秒）之后不补间', () => {
    const a = card('a', [0, 0]);
    flip.play();
    expect(a.animate).not.toHaveBeenCalled();
    const now = vi.spyOn(performance, 'now');
    now.mockReturnValue(1000);
    flip.capture();
    a.move([0, 80]);
    now.mockReturnValue(2500);
    flip.play();
    expect(a.animate).not.toHaveBeenCalled();
    now.mockRestore();
  });
});
