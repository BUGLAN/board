/* ==========================================================================
   卡片落位动画：本窗口拖动松手（lib/drag.ts finish）和别的窗口收到这次移动（lib/flip.ts 里换了状态块的卡片）共用，
   两边看到的是同一段动画（default/main#653）：
   - 幽灵（卡片的克隆，带拖动时的阴影）飞到目标位置，缩放回 1、转角回正；
   - 目标的形态（卡片 / 单行 / 磁贴）或尺寸不同：一路变成目标的宽高、圆角、阴影，不回弹；形态不同时幽灵里的内容先淡出，
     目标在飞到 1/4 处显出来、幽灵后半程淡出，交叉过渡；
   - 落位后真卡片轻弹一下（形态变了、又没有交叉过渡时改成内容淡入）。
   别的窗口没有拖动这一段：幽灵在卡片原来的位置先抬起来（和拖动开始时一样放大、朝去的方向倾斜），再按路程远近多飞一会儿
   ========================================================================== */
import { reduceMotion } from './util';

/** 卡片的形态：磁贴、单行、完整卡片 */
export const shapeOf = (el: Element) => (el.classList.contains('v-tile') ? 'tile' : el.classList.contains('v-line') ? 'line' : 'card');

/** 拖动开始时抬起的时长和放大倍数（别的窗口起飞前的抬起也用它） */
export const LIFT_MS = 220;
const LIFT_SCALE = '1.045';
const LIFT_EASE = 'cubic-bezier(.34,1.56,.64,1)';

/** 元素在所在状态块补间结束后的位置。松手当帧状态块刚解冻、开始从冻结时的位置补间到新位置（.tile 的 transform 过渡），
    getBoundingClientRect 量到的还是补间的起点：加上块的位移（行内写的目标 translate − 现在的 transform）才是幽灵该落的地方 */
export function settledRect(el: HTMLElement): DOMRect {
  const r = el.getBoundingClientRect();
  const tile = el.closest<HTMLElement>('[data-tile]');
  const to = tile && /translate\(\s*(-?[\d.]+)px,\s*(-?[\d.]+)px\s*\)/.exec(tile.style.transform);
  if (!tile || !to) return r;
  const m = /matrix\(([^)]*)\)/.exec(getComputedStyle(tile).transform);
  const [x, y] = m ? m[1].split(',').slice(4).map(parseFloat) : [0, 0];
  return new DOMRect(r.left + (parseFloat(to[1]) - (x || 0)), r.top + (parseFloat(to[2]) - (y || 0)), r.width, r.height);
}

/** 幽灵：卡片的克隆挂到 body 上（.drag-ghost：fixed、带拖动时的阴影），放在 r 的位置 */
export function makeGhost(card: HTMLElement, r: DOMRect): HTMLElement {
  const ghost = card.cloneNode(true) as HTMLElement;
  ghost.classList.remove('flash', 'entering', 'is-open', 'pulse');
  ghost.removeAttribute('data-flip');
  ghost.querySelector('.card-badge')?.remove();
  ghost.classList.add('drag-ghost');
  ghost.style.width = `${r.width}px`;
  ghost.style.animation = 'none';
  ghost.style.transform = `translate(${r.left}px, ${r.top}px)`;
  document.body.appendChild(ghost);
  return ghost;
}

/** 抬起：放大一点（拖动开始、别的窗口起飞前）；rot 是同时倾斜到的角度 */
export function lift(ghost: HTMLElement, rot = 0, at?: { x: number; y: number }) {
  if (!at) return ghost.animate([{ scale: '1' }, { scale: LIFT_SCALE }], { duration: LIFT_MS, easing: LIFT_EASE, fill: 'forwards' });
  return ghost.animate(
    [
      { transform: `translate(${at.x}px, ${at.y}px) rotate(0deg)`, scale: '1' },
      { transform: `translate(${at.x}px, ${at.y}px) rotate(${rot}deg)`, scale: LIFT_SCALE },
    ],
    { duration: LIFT_MS, easing: LIFT_EASE, fill: 'forwards' },
  );
}

export interface FlyReq {
  /** 挂在 body 上、已经抬起来的幽灵 */
  ghost: HTMLElement;
  /** 目标位置上的元素：本窗口是占位，别的窗口是真卡片；没有时落在起点 */
  to: HTMLElement | null;
  /** 目标位置（窗口坐标）；不给时量 to（settledRect） */
  dest?: DOMRect;
  /** 起点：幽灵左上角和倾斜角度 */
  from: { x: number; y: number; rot: number };
  /** 落进新位置；false 是取消、被拒绝，弹回原处 */
  commit: boolean;
  /** 按路程加时：别的窗口从原位置飞到新位置，路程比松手处到落点远 */
  far?: boolean;
  /** 形态变了、要落进目标时，目标在 ms 毫秒后显出来（本窗口给占位打 data-landing，别的窗口真卡片淡入） */
  reveal?: (ms: number) => void;
}

export interface Flight {
  anim: Animation;
  morph: boolean;
  reshape: boolean;
  /** 目标在飞行中途就显出来了（交叉过渡） */
  landing: boolean;
  /** 从现在到落位的毫秒数 */
  total: number;
}

/** 幽灵飞到目标位置。结束后由调用方移除幽灵，再对真卡片调用 landed */
export function fly(f: FlyReq): Flight {
  const { ghost, to, commit, from } = f;
  const dest = f.dest ?? (to ? settledRect(to) : undefined);
  const motion = !reduceMotion();
  const w0 = ghost.offsetWidth;
  const h0 = ghost.offsetHeight;
  const reshape = !!to && shapeOf(to) !== shapeOf(ghost);
  const morph = motion && !!to && !!dest && (reshape || Math.abs(dest.width - w0) > 1 || Math.abs(dest.height - h0) > 1);
  const travel = f.far && motion && dest ? Math.min(320, Math.hypot(dest.left - from.x, dest.top - from.y) * 0.6) : 0;
  const duration = !motion ? 1 : (morph ? 380 : commit ? 280 : 460) + travel;
  const start = `translate(${from.x}px, ${from.y}px) rotate(${from.rot}deg)`;
  const keyframes: Keyframe[] = [
    { transform: start, scale: LIFT_SCALE },
    { transform: dest ? `translate(${dest.left}px, ${dest.top}px) rotate(0deg)` : start, scale: '1' },
  ];
  if (morph && to && dest) {
    // 落位后的样子：目标块里别的卡片（没有就用目标本身）的圆角和阴影
    const like = [...(to.parentElement?.children ?? [])].find(n => n !== to && n.classList.contains('card') && !n.classList.contains('is-placeholder')) ?? to;
    const cs = getComputedStyle(like);
    Object.assign(keyframes[0], { width: `${w0}px`, height: `${h0}px`, borderRadius: getComputedStyle(ghost).borderRadius });
    Object.assign(keyframes[1], { width: `${dest.width}px`, height: `${dest.height}px`, borderRadius: cs.borderRadius });
    ghost.style.overflow = 'hidden';
    // 幽灵的阴影写在样式表里且带 !important，动画盖不过：写成行内 !important，靠 transition 过渡
    ghost.style.setProperty('transition', `box-shadow ${duration}ms cubic-bezier(.2,.9,.3,1)`);
    requestAnimationFrame(() => ghost.style.setProperty('box-shadow', cs.boxShadow, 'important'));
    if (reshape)
      for (const n of ghost.children) (n as HTMLElement).animate([{ opacity: 1 }, { opacity: 0 }], { duration: duration * 0.3, easing: 'ease-out', fill: 'forwards' });
  }
  // 形态变了、要落进目标（例如卡片进「已完成」变成磁贴）：幽灵里的内容淡完，目标就显出来，幽灵在后半程淡出，
  // 真卡片从它下面透出来。以前幽灵内容淡完后是一块空框，要等落位、再等真卡片淡入，中间像卡住了一下
  const landing = commit && morph && reshape;
  if (landing) {
    f.reveal?.(duration * 0.25);
    ghost.animate([{ opacity: 1 }, { opacity: 0 }], { duration: duration * 0.5, delay: duration * 0.25, easing: 'ease-out', fill: 'forwards' });
  }
  const anim = ghost.animate(keyframes, {
    duration,
    // 尺寸在变时不回弹（宽高回弹看起来像抖），只在原样飞回时带一点弹性
    easing: morph ? 'cubic-bezier(.2,.9,.3,1)' : commit ? 'cubic-bezier(.2,1.2,.4,1)' : 'cubic-bezier(.34,1.3,.64,1)',
    fill: 'forwards',
  });
  return { anim, morph, reshape, landing, total: duration };
}

/** 落位后真卡片的收尾（幽灵已经移除）：形态变了、没有交叉过渡时内容接着幽灵淡出的内容淡入；
    尺寸没变、也没有在移向最终位置（别的动画）时轻弹一下 */
export function landed(el: HTMLElement, f: Pick<Flight, 'morph' | 'reshape' | 'landing'>) {
  if (f.reshape && !f.landing) for (const n of el.children) (n as HTMLElement).animate([{ opacity: 0 }, { opacity: 1 }], { duration: 200, easing: 'ease-out' });
  else if (!f.morph && !el.getAnimations().length) el.animate([{ transform: 'scale(1.03)' }, { transform: 'none' }], { duration: 300, easing: 'cubic-bezier(.34,1.56,.64,1)' });
}

/** 真卡片最多藏多久：幽灵没能落下（页面在后台、动画被打断）时也不会一直看不见 */
const HIDE_MAX = 2000;

/** 别的窗口（或撤回补丁）里换了状态块的卡片：在原来的位置抬起一个幽灵，飞到新位置落下，和本窗口拖动松手同一段动画。
    old 是原来的卡片元素（已经从原状态块里移走，画的还是原来的样子），from 是它原来的位置，el 是新位置上的真卡片，
    toward 是 el 现在的位置（只用来定倾斜的方向）。真卡片在幽灵落下前藏着。
    落点在抬起结束时才量：数据刚变时状态块还会再排一两次（磁贴换行、块宽重量），这时量的位置落下时可能已经不对了；
    旧的平移补间是相对位移、会自己跟上，幽灵飞向的是绝对位置，得等排好再量。onFlight 拿到起飞后的这一段（测试用） */
export function flyIn(old: HTMLElement, from: DOMRect, el: HTMLElement, toward: DOMRect, onFlight?: (f: Flight) => void) {
  const ghost = makeGhost(old, from);
  // 朝去的方向倾斜，像拖动时被带着走
  const rot = Math.max(-1, Math.min(1, (toward.left - from.left) / 120)) * 4;
  const up = lift(ghost, rot, { x: from.left, y: from.top });
  let hide: Animation | null = el.animate([{ opacity: 0 }, { opacity: 0 }], { duration: HIDE_MAX });
  const show = (keyframes: Keyframe[] | null, duration = 0) => {
    hide?.cancel();
    hide = null;
    if (keyframes) el.animate(keyframes, { duration, easing: 'ease-out' });
  };
  up.onfinish = () => {
    if (!ghost.isConnected) return;
    if (!el.isConnected) {
      ghost.remove();
      return;
    }
    const f = fly({
      ghost,
      to: el,
      dest: settledRect(el),
      from: { x: from.left, y: from.top, rot },
      commit: true,
      far: true,
      // 形态变了：真卡片在幽灵飞到 1/4 处淡入（和本窗口占位显出真卡片同一时刻）
      reveal: ms => show([{ opacity: 0 }, { opacity: 0, offset: ms / (ms + 160) }, { opacity: 1 }], ms + 160),
    });
    f.anim.onfinish = () => {
      show(null);
      ghost.remove();
      landed(el, f);
    };
    onFlight?.(f);
  };
}
