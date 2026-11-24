/* 卡片落位动画（lib/landing）：本窗口拖动松手和别的窗口收到移动共用同一段（default/main#653） */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fly, flyIn, landed } from './landing';

type Rec = { el: Element; keyframes: Keyframe[]; options: KeyframeAnimationOptions; anim: { onfinish: null | (() => void); cancel: () => void } };
let anims: Rec[];
const real = {
  w: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetWidth')!,
  h: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')!,
};

beforeEach(() => {
  anims = [];
  // jsdom 没有 animate、getAnimations，也不排版：记下每次动画；尺寸按行内宽高（没写高度时 40）
  HTMLElement.prototype.animate = function (this: Element, keyframes: Keyframe[], options: KeyframeAnimationOptions) {
    const anim = { onfinish: null as null | (() => void), cancel: vi.fn() };
    anims.push({ el: this, keyframes, options, anim });
    return anim as unknown as Animation;
  } as unknown as typeof HTMLElement.prototype.animate;
  HTMLElement.prototype.getAnimations = () => [];
  Object.defineProperty(HTMLElement.prototype, 'offsetWidth', { configurable: true, get(this: HTMLElement) { return parseFloat(this.style.width) || 0; } });
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, get(this: HTMLElement) { return parseFloat(this.style.height) || 40; } });
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 0);
});

afterEach(() => {
  delete (HTMLElement.prototype as { animate?: unknown }).animate;
  delete (HTMLElement.prototype as { getAnimations?: unknown }).getAnimations;
  Object.defineProperty(HTMLElement.prototype, 'offsetWidth', real.w);
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', real.h);
  vi.restoreAllMocks();
  document.body.innerHTML = '';
  delete document.documentElement.dataset.motion;
});

const el = (cls: string, w = 100) => {
  const n = document.createElement('article');
  n.className = `card ${cls}`;
  n.style.width = `${w}px`;
  n.appendChild(document.createElement('span'));
  document.body.appendChild(n);
  return n;
};
const at = (left: number, top: number, w = 100, h = 40) => new DOMRect(left, top, w, h);
const of = (n: Element) => anims.filter(x => x.el === n);

describe('fly：幽灵飞到目标位置', () => {
  it('本窗口松手落进新位置：从松手处（带倾斜）飞过去，280ms、带一点回弹，不变形', () => {
    const ghost = el('drag-ghost');
    const ph = el('is-placeholder');
    const f = fly({ ghost, to: ph, dest: at(300, 120), from: { x: 10, y: 20, rot: 3 }, commit: true });
    expect(f).toMatchObject({ morph: false, reshape: false, landing: false, total: 280 });
    const [a] = of(ghost);
    expect(a.keyframes).toEqual([
      { transform: 'translate(10px, 20px) rotate(3deg)', scale: '1.045' },
      { transform: 'translate(300px, 120px) rotate(0deg)', scale: '1' },
    ]);
    expect(a.options).toMatchObject({ duration: 280, easing: 'cubic-bezier(.2,1.2,.4,1)', fill: 'forwards' });
  });

  it('取消、被拒绝：弹回原处 460ms', () => {
    const ghost = el('drag-ghost');
    expect(fly({ ghost, to: el(''), dest: at(0, 0), from: { x: 50, y: 50, rot: 0 }, commit: false }).total).toBe(460);
  });

  it('形态变了（卡片进磁贴）：变成目标的宽高，不回弹；幽灵内容淡出，1/4 处目标显出来、幽灵后半程淡出', () => {
    const ghost = el('drag-ghost', 200);
    const tile = el('v-tile', 80);
    const reveal = vi.fn();
    const f = fly({ ghost, to: tile, dest: at(300, 120, 80, 60), from: { x: 0, y: 0, rot: 0 }, commit: true, reveal });
    expect(f).toMatchObject({ morph: true, reshape: true, landing: true, total: 380 });
    expect(reveal).toHaveBeenCalledWith(95);
    const flight = of(ghost).find(x => x.options.fill === 'forwards' && x.keyframes[0].transform)!;
    expect(flight.keyframes[0]).toMatchObject({ width: '200px', height: '40px' });
    expect(flight.keyframes[1]).toMatchObject({ width: '80px', height: '60px' });
    expect(flight.options.easing).toBe('cubic-bezier(.2,.9,.3,1)');
    // 幽灵整体在后半程淡出；幽灵里的内容先淡出
    expect(of(ghost).some(x => x.keyframes[1].opacity === 0 && x.options.delay === 95)).toBe(true);
    expect(of(ghost.firstElementChild!)[0].keyframes).toEqual([{ opacity: 1 }, { opacity: 0 }]);
  });

  it('别的窗口：按路程加时（最多多 320ms）', () => {
    const near = fly({ ghost: el('drag-ghost'), to: el(''), dest: at(100, 0), from: { x: 0, y: 0, rot: 0 }, commit: true, far: true });
    expect(near.total).toBe(280 + 60);
    const far = fly({ ghost: el('drag-ghost'), to: el(''), dest: at(2000, 0), from: { x: 0, y: 0, rot: 0 }, commit: true, far: true });
    expect(far.total).toBe(280 + 320);
  });

  it('减弱动态效果：直接到位', () => {
    document.documentElement.dataset.motion = 'reduced';
    expect(fly({ ghost: el('drag-ghost'), to: el(''), dest: at(300, 0), from: { x: 0, y: 0, rot: 0 }, commit: true, far: true }).total).toBe(1);
  });
});

describe('landed：落位后真卡片的收尾', () => {
  it('没变形：轻弹一下；形态变了、没有交叉过渡：内容淡入；交叉过渡过了：什么都不做', () => {
    const a = el('');
    landed(a, { morph: false, reshape: false, landing: false });
    expect(of(a)[0].keyframes[0]).toEqual({ transform: 'scale(1.03)' });
    const b = el('');
    landed(b, { morph: true, reshape: true, landing: false });
    expect(of(b.firstElementChild!)[0].keyframes).toEqual([{ opacity: 0 }, { opacity: 1 }]);
    const c = el('');
    landed(c, { morph: true, reshape: true, landing: true });
    expect(of(c)).toHaveLength(0);
    expect(of(c.firstElementChild!)).toHaveLength(0);
  });
});

describe('flyIn：别的窗口里换了状态块的卡片', () => {
  const setup = () => {
    const old = el('');
    old.textContent = '旧';
    old.remove();
    const tile = el('v-tile', 80);
    // 抬起时状态块又排了一次：落点按抬起结束时量的位置（不是 play 那一刻的）
    let now = at(250, 150, 80, 60);
    tile.getBoundingClientRect = () => now;
    let flight: ReturnType<typeof fly> | null = null;
    flyIn(old, at(0, 0), tile, now, f => (flight = f));
    now = at(300, 120, 80, 60);
    const ghost = document.querySelector<HTMLElement>('.drag-ghost')!;
    return { tile, ghost, up: of(ghost)[0], flight: () => flight! };
  };

  it('先在原处抬起（朝去的方向倾斜），抬起结束才量落点起飞；真卡片一直藏到落下', () => {
    const { tile, ghost, up, flight } = setup();
    expect(up.keyframes[1]).toMatchObject({ transform: 'translate(0px, 0px) rotate(4deg)', scale: '1.045' });
    expect(of(tile)[0].keyframes).toEqual([{ opacity: 0 }, { opacity: 0 }]);
    up.anim.onfinish!();
    const fl = of(ghost).find(x => x !== up && x.keyframes[0].transform)!;
    expect(fl.keyframes[1]).toMatchObject({ transform: 'translate(300px, 120px) rotate(0deg)', width: '80px', height: '60px' });
    expect(flight().landing).toBe(true);
  });

  it('形态变了：真卡片在幽灵飞到 1/4 处淡入；落下后幽灵移除', () => {
    const { tile, up, flight } = setup();
    up.anim.onfinish!();
    const hide = of(tile)[0];
    expect(hide.anim.cancel).toHaveBeenCalled();
    const show = of(tile)[1];
    expect(show.keyframes.at(-1)).toEqual({ opacity: 1 });
    expect(show.options.duration).toBeCloseTo(flight().total * 0.25 + 160);
    (flight().anim.onfinish as unknown as () => void)();
    expect(document.querySelector('.drag-ghost')).toBeNull();
  });

  it('抬起结束时真卡片已经不在了（又被移走）：幽灵直接移除，不飞', () => {
    const { tile, up } = setup();
    tile.remove();
    up.anim.onfinish!();
    expect(document.querySelector('.drag-ghost')).toBeNull();
  });
});
