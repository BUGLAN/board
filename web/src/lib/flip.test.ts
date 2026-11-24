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
    return { cancel: () => {} } as Animation;
  });
  el.animate = animate as unknown as typeof el.animate;
  el.getAnimations = () => [];
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

  it('位置没变的不补间；占位换位置时淡入', () => {
    const still = card('still', [0, 0]);
    const ph = card('ph', [0, 200]);
    ph.el.classList.add('is-placeholder');
    flip.capture();
    ph.move([0, 300]);
    flip.play();
    expect(still.animate).not.toHaveBeenCalled();
    expect((ph.animate.mock.calls[0][0] as Keyframe[])[0]).toMatchObject({ opacity: 0 });
  });

  describe('换了状态块的卡片：和本窗口拖动松手同一段落位动画（default/main#653）', () => {
    // jsdom 没有 Element.animate：幽灵是克隆出来的新元素，记下每次动画是谁、什么关键帧
    let anims: { el: Element; keyframes: Keyframe[]; options: KeyframeAnimationOptions; anim: { onfinish: null | (() => void); cancel: () => void } }[];
    // jsdom 不排版，offsetWidth / offsetHeight 恒为 0：按行内宽度、高 40 算，和卡片一样大（不变形）
    const real = {
      w: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetWidth')!,
      h: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')!,
    };
    beforeEach(() => {
      anims = [];
      Object.defineProperty(HTMLElement.prototype, 'offsetWidth', { configurable: true, get(this: HTMLElement) { return parseFloat(this.style.width) || 0; } });
      Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, get: () => 40 });
      HTMLElement.prototype.animate = function (this: Element, keyframes: Keyframe[], options: KeyframeAnimationOptions) {
        const anim = { onfinish: null as null | (() => void), cancel: vi.fn() };
        anims.push({ el: this, keyframes, options, anim });
        return anim as unknown as Animation;
      } as unknown as typeof HTMLElement.prototype.animate;
    });
    afterEach(() => {
      delete (HTMLElement.prototype as { animate?: unknown }).animate;
      Object.defineProperty(HTMLElement.prototype, 'offsetWidth', real.w);
      Object.defineProperty(HTMLElement.prototype, 'offsetHeight', real.h);
    });

    const moveTo = (c: ReturnType<typeof card>, col: string, at: [number, number]) => {
      let other = document.querySelector<HTMLElement>(`[data-list="${col}"]`);
      if (!other) {
        other = document.createElement('div');
        other.dataset.list = col;
        document.body.appendChild(other);
      }
      // React 在新状态块里新建一个元素：旧元素拿走、新元素带同一个 data-flip
      const old = c.el;
      old.remove();
      const fresh = card(c.el.dataset.flip!, at, other);
      return { old, fresh };
    };

    it('原位置抬起一个幽灵飞到新位置；真卡片藏到幽灵落下；幽灵落下后移除、真卡片轻弹', () => {
      const a = card('a', [0, 100]);
      a.el.textContent = '原来的样子';
      flip.capture();
      const { fresh } = moveTo(a, 'done', [300, 120]);
      fresh.el.textContent = '新位置';
      flip.play();
      const ghost = document.querySelector<HTMLElement>('.drag-ghost')!;
      expect(ghost.textContent).toBe('原来的样子');
      expect(ghost.dataset.flip).toBeUndefined();
      // 平移补间不再用在它身上：真卡片只有一段「藏着」的透明度动画
      expect(fresh.animate).toHaveBeenCalledTimes(1);
      expect(fresh.animate.mock.calls[0][0]).toEqual([{ opacity: 0 }, { opacity: 0 }]);
      // 先在原处抬起（放大、朝右倾斜），抬起结束再飞到新位置落下
      const [up] = anims.filter(x => x.el === ghost);
      expect(up.keyframes[1]).toMatchObject({ transform: 'translate(0px, 100px) rotate(4deg)', scale: '1.045' });
      up.anim.onfinish!();
      const flight = anims.filter(x => x.el === ghost)[1];
      expect(flight.keyframes[0]).toMatchObject({ transform: 'translate(0px, 100px) rotate(4deg)', scale: '1.045' });
      expect(flight.keyframes[1]).toMatchObject({ transform: 'translate(300px, 120px) rotate(0deg)', scale: '1' });
      flight.anim.onfinish!();
      expect(ghost.isConnected).toBe(false);
      expect((fresh.animate.mock.calls[1][0] as Keyframe[])[0]).toEqual({ transform: 'scale(1.03)' });
    });

    it(`一次最多 ${flip.MAX_FLIGHTS} 张播落位动画，多出来的照旧平移补间`, () => {
      const cs = Array.from({ length: flip.MAX_FLIGHTS + 2 }, (_, i) => card(`c${i}`, [0, i * 50]));
      flip.capture();
      const moved = cs.map((c, i) => moveTo(c, 'done', [300, i * 50]).fresh);
      flip.play();
      expect(document.querySelectorAll('.drag-ghost')).toHaveLength(flip.MAX_FLIGHTS);
      const rest = moved.slice(flip.MAX_FLIGHTS);
      for (const c of rest) expect((c.animate.mock.calls[0][0] as Keyframe[])[0].zIndex).toBe(5);
    });
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
