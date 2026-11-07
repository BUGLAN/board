/* ==========================================================================
   平台：网页 / 桌面端（Tauri）；主题、卡片密度、动效偏好
   桌面端：顶栏即标题栏，按住空白处拖动窗口，双击最大化
   ========================================================================== */
import { getCurrentWindow } from '@tauri-apps/api/window';
import { pref } from './util';

const isTauri = typeof window !== 'undefined' && ('__TAURI_INTERNALS__' in window || window.__BOARD_SHELL__ === 'desktop');
export const shell: 'web' | 'desktop' = isTauri ? 'desktop' : 'web';

export type ThemePref = 'auto' | 'light' | 'dark';
export type Density = 'comfy' | 'compact';
export type Motion = 'full' | 'reduced';

export function applyPrefs() {
  const root = document.documentElement;
  root.dataset.shell = shell;
  root.dataset.os = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent) ? 'mac' : 'other';
  setTheme(pref.get<ThemePref>('theme', 'auto'));
  root.dataset.density = pref.get<Density>('density', 'comfy');
  root.dataset.motion = pref.get<Motion>('motion', 'full');
}

export function setTheme(p: ThemePref) {
  const root = document.documentElement;
  root.dataset.themePref = p;
  if (p === 'auto') delete root.dataset.theme;
  else root.dataset.theme = p;
  pref.set('theme', p);
}
export const getTheme = () => pref.get<ThemePref>('theme', 'auto');
export function cycleTheme(): ThemePref {
  const order: ThemePref[] = ['auto', 'light', 'dark'];
  const next = order[(order.indexOf(getTheme()) + 1) % order.length];
  setTheme(next);
  return next;
}
export function setDensity(d: Density) {
  document.documentElement.dataset.density = d;
  pref.set('density', d);
}
export function setMotion(m: Motion) {
  document.documentElement.dataset.motion = m;
  pref.set('motion', m);
}

// 静态导入：按下鼠标时要立刻发起拖动（macOS 需要在按住期间调用）
const win = () => (isTauri ? getCurrentWindow() : null);
export function startWindowDrag() {
  win()?.startDragging().catch(() => {});
}
export function toggleMaximize() {
  win()?.toggleMaximize().catch(() => {});
}
