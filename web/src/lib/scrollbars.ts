// 全局浮层滚动条：原生滚动条全部隐藏（base.css），这里给所有滚动区域画浮层的细条。
// - 滚动时：在滚动的那一侧出现 2px 发丝线，停止滚动 HIDE_DELAY 毫秒后淡出
// - 滚动条正显示着（滚动中，或正在悬停 / 拖动）时，鼠标移到滚动区域边缘 EDGE 像素内：发丝线展开成 8px 滑块，
//   可以拖，点轨道空白处翻一页。滚动条隐藏时鼠标靠近边缘不出现、不响应
// 不用给组件包任何东西：在 document 上监听 scroll（捕获）和 pointermove，按需给滚动元素建一条。
// 不想要滚动条的区域加 data-scrollbar="none"。

type Axis = 'y' | 'x';

interface Rail {
  track: HTMLDivElement;
  thumb: HTMLDivElement;
  scrolling: boolean;
  near: boolean;
  drag: boolean;
  timer: number;
  len: number;
  room: number;
}

interface Bar {
  el: Element;
  root: HTMLDivElement;
  clips: Element[];
  y: Rail;
  x: Rail;
  offAt: number;
}

const HIDE_DELAY = 900;
const FADE_MS = 400;
const EDGE = 16;
const TRACK = 12;
const PAD = 3;
const MIN_THUMB = 28;

const bars = new Map<Element, Bar>();
const lastPos = new WeakMap<Element, { top: number; left: number }>();
let layer: HTMLDivElement | null = null;
let raf = 0;
let installed = false;

function scroller(): Element {
  return document.scrollingElement ?? document.documentElement;
}

function canScroll(el: Element, axis: Axis): boolean {
  const over = axis === 'y' ? el.scrollHeight - el.clientHeight : el.scrollWidth - el.clientWidth;
  if (over <= 1) return false;
  if (el === scroller()) return true;
  const o = getComputedStyle(el)[axis === 'y' ? 'overflowY' : 'overflowX'];
  return o === 'auto' || o === 'scroll' || o === 'overlay';
}

function optedOut(el: Element): boolean {
  return !!el.closest('[data-scrollbar="none"]') || !!layer?.contains(el);
}

/** 这一侧的滚动条正显示着：滚动中、悬停展开着或正在拖 */
function shown(el: Element, axis: Axis): boolean {
  const r = bars.get(el)?.[axis];
  return !!r && (r.scrolling || r.near || r.drag);
}

function makeRail(axis: Axis, root: HTMLDivElement, bar: () => Bar): Rail {
  const track = document.createElement('div');
  track.className = `sbar-track ${axis}`;
  const thumb = document.createElement('div');
  thumb.className = 'sbar-thumb';
  track.appendChild(thumb);
  root.appendChild(track);
  const rail: Rail = { track, thumb, scrolling: false, near: false, drag: false, timer: 0, len: 0, room: 0 };

  let start = 0;
  let startPos = 0;
  thumb.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    try {
      thumb.setPointerCapture(e.pointerId);
    } catch {
      // 指针已经不在了（比如合成事件），不捕获也能拖
    }
    const { el } = bar();
    rail.drag = true;
    start = axis === 'y' ? e.clientY : e.clientX;
    startPos = axis === 'y' ? el.scrollTop : el.scrollLeft;
    wake();
  });
  thumb.addEventListener('pointermove', (e) => {
    if (!rail.drag || rail.room <= 0) return;
    const { el } = bar();
    const max = axis === 'y' ? el.scrollHeight - el.clientHeight : el.scrollWidth - el.clientWidth;
    const v = startPos + ((axis === 'y' ? e.clientY : e.clientX) - start) * (max / rail.room);
    if (axis === 'y') el.scrollTop = v;
    else el.scrollLeft = v;
  });
  const end = () => {
    if (!rail.drag) return;
    rail.drag = false;
    poke(bar(), axis);
  };
  thumb.addEventListener('pointerup', end);
  thumb.addEventListener('pointercancel', end);

  // 点轨道空白处：朝点击的方向翻一页
  track.addEventListener('pointerdown', (e) => {
    if (e.target !== track || e.button !== 0) return;
    e.preventDefault();
    const { el } = bar();
    const r = thumb.getBoundingClientRect();
    const before = axis === 'y' ? e.clientY < r.top : e.clientX < r.left;
    const page = (axis === 'y' ? el.clientHeight : el.clientWidth) * 0.9 * (before ? -1 : 1);
    const behavior: ScrollBehavior = matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth';
    el.scrollBy(axis === 'y' ? { top: page, behavior } : { left: page, behavior });
  });
  return rail;
}

function clipAncestors(el: Element): Element[] {
  const out: Element[] = [];
  for (let p = el.parentElement; p && p !== document.body && p !== document.documentElement; p = p.parentElement) {
    const s = getComputedStyle(p);
    if (s.overflowX !== 'visible' || s.overflowY !== 'visible') out.push(p);
  }
  return out;
}

function ensure(el: Element): Bar {
  let bar = bars.get(el);
  if (bar) return bar;
  if (!layer) {
    layer = document.createElement('div');
    layer.className = 'sbar-layer';
    layer.setAttribute('aria-hidden', 'true');
    document.body.appendChild(layer);
  }
  const root = document.createElement('div');
  root.className = 'sbar';
  layer.appendChild(root);
  const get = () => bar!;
  bar = {
    el,
    root,
    clips: el === scroller() ? [] : clipAncestors(el),
    y: makeRail('y', root, get),
    x: makeRail('x', root, get),
    offAt: 0,
  };
  bars.set(el, bar);
  return bar;
}

function poke(bar: Bar, axis: Axis) {
  const rail = bar[axis];
  rail.scrolling = true;
  clearTimeout(rail.timer);
  rail.timer = window.setTimeout(() => {
    rail.scrolling = false;
    wake();
  }, HIDE_DELAY);
  wake();
}

function wake() {
  if (!raf) raf = requestAnimationFrame(frame);
}

function frame(now: number) {
  raf = 0;
  for (const bar of bars.values()) {
    const { el, root, y, x } = bar;
    const on = (r: Rail) => r.scrolling || r.near || r.drag;
    if (!el.isConnected) {
      drop(bar);
      continue;
    }
    if (!on(y) && !on(x)) {
      if (!bar.offAt) bar.offAt = now;
      if (now - bar.offAt > FADE_MS) {
        drop(bar);
        continue;
      }
    } else bar.offAt = 0;

    // 元素内容区（去掉边框）在视口里的位置
    const isDoc = el === scroller();
    const r = isDoc ? new DOMRect(0, 0, innerWidth, innerHeight) : el.getBoundingClientRect();
    const left = r.left + (isDoc ? 0 : el.clientLeft);
    const top = r.top + (isDoc ? 0 : el.clientTop);
    const w = isDoc ? innerWidth : el.clientWidth;
    const h = isDoc ? innerHeight : el.clientHeight;

    // 被外层裁掉的部分不画
    let cl = Math.max(left, 0);
    let ct = Math.max(top, 0);
    let cr = Math.min(left + w, innerWidth);
    let cb = Math.min(top + h, innerHeight);
    for (const c of bar.clips) {
      const cr2 = c.getBoundingClientRect();
      cl = Math.max(cl, cr2.left);
      ct = Math.max(ct, cr2.top);
      cr = Math.min(cr, cr2.right);
      cb = Math.min(cb, cr2.bottom);
    }
    if (cr <= cl || cb <= ct) {
      root.style.display = 'none';
      continue;
    }
    root.style.display = '';
    root.style.transform = `translate(${cl}px, ${ct}px)`;
    root.style.width = `${cr - cl}px`;
    root.style.height = `${cb - ct}px`;

    const both = el.scrollHeight - el.clientHeight > 1 && el.scrollWidth - el.clientWidth > 1;
    place(y, 'y', on(y), left - cl, top - ct, w, h, el.scrollTop, el.scrollHeight, both);
    place(x, 'x', on(x), left - cl, top - ct, w, h, el.scrollLeft, el.scrollWidth, both);
  }
  if (bars.size) raf = requestAnimationFrame(frame);
}

function place(rail: Rail, axis: Axis, on: boolean, ox: number, oy: number, w: number, h: number, pos: number, full: number, both: boolean) {
  const { track, thumb } = rail;
  const size = axis === 'y' ? h : w;
  const tl = size - PAD * 2 - (both ? TRACK : 0);
  const max = full - size;
  if (max <= 1 || tl <= MIN_THUMB) {
    track.style.display = 'none';
    return;
  }
  track.style.display = '';
  if (axis === 'y') {
    track.style.transform = `translate(${ox + w - TRACK}px, ${oy + PAD}px)`;
    track.style.height = `${tl}px`;
  } else {
    track.style.transform = `translate(${ox + PAD}px, ${oy + h - TRACK}px)`;
    track.style.width = `${tl}px`;
  }
  rail.len = Math.max(MIN_THUMB, (tl * size) / full);
  rail.room = tl - rail.len;
  const off = Math.min(rail.room, Math.max(0, (pos / max) * rail.room));
  thumb.style[axis === 'y' ? 'height' : 'width'] = `${rail.len}px`;
  thumb.style.transform = axis === 'y' ? `translateY(${off}px)` : `translateX(${off}px)`;
  track.classList.toggle('is-on', on);
  track.classList.toggle('is-wide', rail.near || rail.drag);
}

function drop(bar: Bar) {
  clearTimeout(bar.y.timer);
  clearTimeout(bar.x.timer);
  bar.root.remove();
  bars.delete(bar.el);
}

function onScroll(e: Event) {
  const el = e.target === document ? scroller() : (e.target as Element);
  if (!(el instanceof Element) || optedOut(el)) return;
  const prev = lastPos.get(el);
  const top = el.scrollTop;
  const left = el.scrollLeft;
  lastPos.set(el, { top, left });
  const dy = !prev || prev.top !== top;
  const dx = !prev || prev.left !== left;
  const sy = dy && canScroll(el, 'y');
  const sx = dx && canScroll(el, 'x');
  if (!sy && !sx) return;
  const bar = ensure(el);
  if (sy && !bar.y.drag) poke(bar, 'y');
  if (sx && !bar.x.drag) poke(bar, 'x');
}

// 鼠标靠近哪个滚动区域的边缘：从指针下的元素往外找，每个方向取最里层的。
// 只认正显示着滚动条的那一侧：隐藏的滚动条不因为鼠标靠近而出现
let pending: PointerEvent | null = null;
/** 最里层靠近的区域滚动条没显示：就当没靠近任何区域（不再往外层找） */
const NONE = {} as Element;
function onPointerMove(e: PointerEvent) {
  if (e.pointerType !== 'mouse') return;
  const first = !pending;
  pending = e;
  if (first) requestAnimationFrame(hoverFrame);
}

function hoverFrame() {
  const e = pending;
  pending = null;
  if (!e) return;
  const target = e.target as Element | null;
  // 指针在滑块上：保持现状
  if (target && layer?.contains(target)) return;
  // 按着键（拖卡片、选文字）时不展开
  let ny: Element | null = null;
  let nx: Element | null = null;
  if (!e.buttons && target instanceof Element) {
    for (let el: Element | null = target; el && (!ny || !nx); el = el.parentElement) {
      if (el === document.body || el === document.documentElement) break;
      if (el.getAttribute('data-scrollbar') === 'none') break;
      const yOver = el.scrollHeight - el.clientHeight > 1;
      const xOver = el.scrollWidth - el.clientWidth > 1;
      if (!yOver && !xOver) continue;
      const r = el.getBoundingClientRect();
      const inner = r.left + el.clientLeft;
      const innerTop = r.top + el.clientTop;
      if (!ny && yOver) {
        const right = inner + el.clientWidth;
        if (right - e.clientX <= EDGE && e.clientX <= right && canScroll(el, 'y')) ny = shown(el, 'y') ? el : NONE;
      }
      if (!nx && xOver) {
        const bottom = innerTop + el.clientHeight;
        if (bottom - e.clientY <= EDGE && e.clientY <= bottom && canScroll(el, 'x')) nx = shown(el, 'x') ? el : NONE;
      }
    }
  }
  setNear(ny === NONE ? null : ny, 'y');
  setNear(nx === NONE ? null : nx, 'x');
}

function setNear(el: Element | null, axis: Axis) {
  let changed = false;
  for (const bar of bars.values()) {
    if (bar.el !== el && bar[axis].near) {
      bar[axis].near = false;
      changed = true;
    }
  }
  if (el) {
    const bar = ensure(el);
    if (!bar[axis].near) {
      bar[axis].near = true;
      changed = true;
    }
  }
  if (changed) wake();
}

function clearNear() {
  setNear(null, 'y');
  setNear(null, 'x');
}

export function installScrollbars() {
  if (installed) return;
  installed = true;
  document.addEventListener('scroll', onScroll, { capture: true, passive: true });
  document.addEventListener('pointermove', onPointerMove, { passive: true });
  document.documentElement.addEventListener('pointerleave', clearNear);
  window.addEventListener('blur', clearNear);
}
