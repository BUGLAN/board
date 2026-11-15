/* ==========================================================================
   agent logo 的签名动作（default/main#357）
   每个认得出的 agent 一个自己的动作，体现 logo 本身：claude 的像素小人蹦一下、dsh 的鲸鱼摆尾游、
   codex 的云飘起来晃一晃、pi 的方块压扁再弹高。用在三处（AgentAvatar）：
   - hover：做一次（ACT）
   - 出现、换人：同一个动作的进场版（ENTER）——小人从上面掉下来、鲸鱼从左边游进来、云飘下来、方块从扁的一条弹起来
   - 在线干活（顶栏心跳正常、画布上正在工作的白板）：每 LOOP_MS 做一次 ACT，几个错开（loopFrames）
   都用 Web Animations 播在 logo 图上（.logo-art），不占 CSS 的 animation，和状态的滤镜动画（掉线变灰等）互不干扰。
   「设置 → 外观 → 动效 = 减弱」或系统要求减少动态效果时一个都不播，在播的循环也停下（subscribeMotion）。
   ========================================================================== */
import { reduceMotion } from './util';

export type AgentMotion = 'hop' | 'swim' | 'float' | 'squash';

/** hover 时做一次的动作：都从原样开始、回到原样结束 */
export const ACT: Record<AgentMotion, Keyframe[]> = {
  // 蹲、起跳拉长、落地压扁、回弹
  hop: [
    { transform: 'none' },
    { transform: 'translateY(6%) scale(1.14, .8)', offset: 0.15 },
    { transform: 'translateY(-55%) scale(.9, 1.12)', offset: 0.4 },
    { transform: 'translateY(0) scale(1.16, .82)', offset: 0.62 },
    { transform: 'scale(.96, 1.05)', offset: 0.82 },
    { transform: 'none' },
  ],
  // 摆尾往前游一下
  swim: [
    { transform: 'none' },
    { transform: 'translateX(-12%) rotate(-18deg)', offset: 0.22 },
    { transform: 'translateX(14%) rotate(12deg)', offset: 0.5 },
    { transform: 'translateX(-3%) rotate(-4deg)', offset: 0.76 },
    { transform: 'none' },
  ],
  // 飘起来、轻轻晃
  float: [
    { transform: 'none' },
    { transform: 'translateY(-40%) rotate(-12deg)', offset: 0.32 },
    { transform: 'translateY(-22%) rotate(9deg)', offset: 0.62 },
    { transform: 'none' },
  ],
  // 压扁再弹高
  squash: [
    { transform: 'none' },
    { transform: 'scale(1.24, .66)', offset: 0.2 },
    { transform: 'translateY(-34%) scale(.86, 1.2)', offset: 0.45 },
    { transform: 'scale(1.08, .92)', offset: 0.7 },
    { transform: 'none' },
  ],
};

/** 出现、换人时的进场版：从看不见开始，落到原样 */
export const ENTER: Record<AgentMotion, Keyframe[]> = {
  // 从上面掉下来，落地压扁再站直
  hop: [
    { opacity: 0, transform: 'translateY(-70%) scale(.9, 1.1)' },
    { opacity: 1, transform: 'translateY(0) scale(1.16, .8)', offset: 0.45 },
    { transform: 'translateY(-12%) scale(.96, 1.04)', offset: 0.7 },
    { transform: 'none' },
  ],
  // 从左边游进来
  swim: [
    { opacity: 0, transform: 'translateX(-50%) rotate(-14deg)' },
    { opacity: 1, transform: 'translateX(8%) rotate(8deg)', offset: 0.55 },
    { transform: 'translateX(-2%) rotate(-3deg)', offset: 0.8 },
    { transform: 'none' },
  ],
  // 从上面飘下来
  float: [
    { opacity: 0, transform: 'translateY(-45%) rotate(-10deg)' },
    { opacity: 1, transform: 'translateY(6%) rotate(5deg)', offset: 0.6 },
    { transform: 'none' },
  ],
  // 从扁的一条弹成方块
  squash: [
    { opacity: 0, transform: 'scale(1.3, 0)' },
    { opacity: 1, transform: 'scale(.9, 1.18)', offset: 0.5 },
    { transform: 'scale(1.05, .95)', offset: 0.75 },
    { transform: 'none' },
  ],
};

export const ACT_MS = 800;
export const ENTER_MS = 680;
/** 在线干活时多久做一次 */
export const LOOP_MS = 5000;
/** 几个 logo 并排循环时，相邻两个错开多久（顶栏、画布） */
export const LOOP_STAGGER_MS = 1250;
const EASE = 'cubic-bezier(.3, .7, .4, 1)';

/** 把「做一次」的关键帧压进一个循环的开头：每 period 毫秒里只在开头 ms 毫秒做一次，其余时间停在原样 */
export function loopFrames(frames: Keyframe[], ms = ACT_MS, period = LOOP_MS): Keyframe[] {
  const k = Math.min(1, ms / period);
  const last = frames.length - 1;
  const out = frames.map((f, i) => ({ ...f, offset: (typeof f.offset === 'number' ? f.offset : last ? i / last : 0) * k, easing: 'ease-in-out' }));
  return k < 1 ? [...out, { transform: 'none', offset: 1 }] : out;
}

/** 能不能播：元素在、浏览器支持、没开减弱动效 */
const canPlay = (el: Element | null | undefined): el is Element => !!el && typeof el.animate === 'function' && !reduceMotion();

/** 播一次签名动作（hover 或进场）；上一次还没播完就不叠着播。返回动画，播不了返回 null */
export function playAgentMotion(el: Element | null | undefined, motion: AgentMotion | undefined, kind: 'act' | 'enter'): Animation | null {
  if (!motion || !canPlay(el)) return null;
  if (el.getAnimations?.().some(a => a.id === 'agent-act' && a.playState === 'running')) return null;
  const enter = kind === 'enter';
  const a = el.animate(enter ? ENTER[motion] : ACT[motion], { duration: enter ? ENTER_MS : ACT_MS, easing: EASE, delay: enter ? 60 : 0 });
  a.id = 'agent-act';
  return a;
}

/** 在线干活时循环：每 LOOP_MS 做一次，delay 用来和旁边的错开。返回动画（停的时候 cancel），播不了返回 null */
export function loopAgentMotion(el: Element | null | undefined, motion: AgentMotion | undefined, delay = 0): Animation | null {
  if (!motion || !canPlay(el)) return null;
  const a = el.animate(loopFrames(ACT[motion]), { duration: LOOP_MS, iterations: Infinity, delay });
  a.id = 'agent-loop';
  return a;
}

/* ---- 减弱动效的开关变了：通知在循环的头像停下或重新开始（useSyncExternalStore 用） ---- */
const listeners = new Set<() => void>();
let watching = false;

function watch() {
  if (watching || typeof window === 'undefined') return;
  watching = true;
  const fire = () => listeners.forEach(f => f());
  // 设置里切换「动效」写的是 <html data-motion>
  if (typeof MutationObserver === 'function')
    new MutationObserver(fire).observe(document.documentElement, { attributes: true, attributeFilter: ['data-motion'] });
  window.matchMedia?.('(prefers-reduced-motion: reduce)').addEventListener?.('change', fire);
}

export function subscribeMotion(f: () => void) {
  watch();
  listeners.add(f);
  return () => {
    listeners.delete(f);
  };
}
