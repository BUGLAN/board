/* 时间、agent 身份色、平台判断 */
import type { Actor } from './types';

export const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
export const MOD = isMac ? '⌘' : 'Ctrl';

/** 快捷键显示：macOS 用 ⌘1、⌘⇧N；Windows / Linux 用 Ctrl+1、Ctrl+Shift+N */
export function accel(key: string, shift = false): string {
  return isMac ? `⌘${shift ? '⇧' : ''}${key}` : `Ctrl+${shift ? 'Shift+' : ''}${key}`;
}
/** 返回 / 前进（和浏览器一致：macOS 上是 ⌘[ ⌘] ，Windows / Linux 上是 Alt+← Alt+→） */
export const BACK_KEY = isMac ? '⌘[' : 'Alt+←';
export const FWD_KEY = isMac ? '⌘]' : 'Alt+→';
export const isMobile = () => window.matchMedia('(max-width: 760px)').matches;
export const reduceMotion = () =>
  document.documentElement.dataset.motion === 'reduced' || window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/* ---------- agent 身份色 ---------- */
const AGENT_COLORS = ['#FF7A45', '#4DB5FF', '#3DDC97', '#FFB020', '#FF5C8A', '#9B87FF', '#2DD4BF', '#E879F9', '#84CC16'];
const GREEK: Record<string, string> = {
  alpha: 'α', beta: 'β', gamma: 'γ', delta: 'δ', epsilon: 'ε', zeta: 'ζ', eta: 'η', theta: 'θ', kappa: 'κ', lambda: 'λ', sigma: 'σ', omega: 'ω',
};
const FIXED: Record<string, number> = { alpha: 0, beta: 1, gamma: 2, delta: 3 };

export function agentColor(name: string) {
  const key = String(name || '').toLowerCase();
  if (key in FIXED) return AGENT_COLORS[FIXED[key]];
  let h = 0;
  for (const ch of key) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return AGENT_COLORS[h % AGENT_COLORS.length];
}
/** agent 身份色写成文字时用：浅色主题下加深到对比度 ≥ 4.5:1（--ink），深色主题原样 */
export const agentInk = (name: string) => `color-mix(in oklab, ${agentColor(name)}, var(--ink))`;
export const agentGlyph = (name: string) =>
  GREEK[String(name).toLowerCase()] || [...String(name || '?').trim()][0]?.toUpperCase() || '?';

/** 界面上怎么称呼：人 = 你，系统 = board（和服务端 Actor::display() 一致） */
/** 界面上怎么称呼：人是「你」；系统身份带名字的（「定时建卡」）写名字，其余是 board 本身 */
export const actorName = (a: Actor) => (a.kind === 'human' ? '你' : a.kind === 'system' ? a.name || 'board' : a.name);

/* ---------- 时间 ---------- */
const pad2 = (n: number) => String(n).padStart(2, '0');
export const hhmm = (t: number) => {
  const d = new Date(t);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
};
/** 计时：12:34 / 1:02:03 */
export const clock = (ms: number) => {
  const t = Math.floor(Math.max(0, ms) / 1000);
  const h = Math.floor(t / 3600);
  return h ? `${h}:${pad2(Math.floor((t % 3600) / 60))}:${pad2(t % 60)}` : `${pad2(Math.floor(t / 60))}:${pad2(t % 60)}`;
};
/** 文字：23 分 10 秒 / 1 小时 5 分 / 8 秒（与 board-core 的 dur_words 一致） */
export const durWords = (ms: number) => {
  const t = Math.floor(Math.max(0, ms) / 1000);
  const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), s = t % 60;
  return h ? `${h} 小时 ${m} 分` : m ? `${m} 分 ${s} 秒` : `${s} 秒`;
};
export function ago(t: number, now: number) {
  const d = Math.max(0, now - t);
  if (d < 45e3) return '刚刚';
  if (d < 3600e3) return `${Math.round(d / 60e3)} 分钟前`;
  if (d < 86400e3) return `${Math.round(d / 3600e3)} 小时前`;
  const x = new Date(t);
  return `${x.getMonth() + 1}月${x.getDate()}日 ${hhmm(t)}`;
}

/** 「完成于」用的相对时间：刚刚 / N 分钟前 / N 小时前 / N 天前 */
export function since(t: number, now: number) {
  const d = Math.max(0, now - t);
  if (d < 45e3) return '刚刚';
  if (d < 3600e3) return `${Math.round(d / 60e3)} 分钟前`;
  if (d < 86400e3) return `${Math.round(d / 3600e3)} 小时前`;
  return `${Math.round(d / 86400e3)} 天前`;
}
/** 具体的日期时间：2026-11-10 14:05 */
export function dateTime(t: number) {
  const d = new Date(t);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${hhmm(t)}`;
}

export const initial = (name: string) => [...String(name || '?').trim()][0]?.toUpperCase() || '?';

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.cssText = 'position:fixed;opacity:0;pointer-events:none';
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand('copy');
      ta.remove();
      return ok;
    } catch {
      return false;
    }
  }
}

export function autosize(ta: HTMLTextAreaElement | null) {
  if (!ta) return;
  ta.style.height = 'auto';
  ta.style.height = ta.scrollHeight + 'px';
}

/** 让 CSS 动画重新播放 */
export function replay(el: Element | null | undefined, cls: string) {
  if (!el) return;
  el.classList.remove(cls);
  void (el as HTMLElement).offsetWidth;
  el.classList.add(cls);
}

export const cx = (...xs: (string | false | null | undefined)[]) => xs.filter(Boolean).join(' ');

/* ---------- 本地偏好 ---------- */
export const pref = {
  get<T>(k: string, d: T): T {
    try {
      const v = localStorage.getItem('board.' + k);
      return v == null ? d : (JSON.parse(v) as T);
    } catch {
      return d;
    }
  },
  set(k: string, v: unknown) {
    try {
      if (v == null) localStorage.removeItem('board.' + k);
      else localStorage.setItem('board.' + k, JSON.stringify(v));
    } catch {
      /* 隐私模式等 */
    }
  },
};
