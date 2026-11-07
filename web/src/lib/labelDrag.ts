/* ==========================================================================
   拖动标签到卡片上：给卡片加上这个标签（侧栏「标签」里的胶囊）
   - 鼠标按下移动 5px 开始；触摸长按 220ms 开始（与卡片拖动一致）；只是点击时仍是按标签筛选
   - 经过的卡片高亮；松手在卡片上就加标签，已经有这个标签时只提示；Esc 取消
   - 不用浏览器原生拖放：桌面端（Tauri）在 Windows 上会拦下原生拖放，触屏上也不可用
   ========================================================================== */
import type { PointerEvent as ReactPointerEvent } from 'react';
import { exec, useData } from '../store/data';
import { flash, toast, toastError } from '../store/ui';
import type { Label } from './types';

interface St {
  label: Label;
  pointerId: number;
  type: string;
  x0: number;
  y0: number;
  x: number;
  y: number;
  started: boolean;
  timer?: number;
  raf?: number;
  ghost?: HTMLElement;
  target: HTMLElement | null;
}

let st: St | null = null;
/** 刚拖完：松手时浏览器还会补一次 click，胶囊据此不触发筛选 */
let endedAt = 0;
export const labelDragJustEnded = () => performance.now() - endedAt < 400;

/** 标签胶囊的 onPointerDown */
export function labelPointerDown(e: ReactPointerEvent, label: Label) {
  if (e.button !== 0 || st) return;
  st = { label, pointerId: e.pointerId, type: e.pointerType, x0: e.clientX, y0: e.clientY, x: e.clientX, y: e.clientY, started: false, target: null };
  if (e.pointerType !== 'mouse') {
    st.timer = window.setTimeout(() => {
      if (st && !st.started) start();
    }, 220);
  }
  window.addEventListener('pointermove', onMove, { passive: false });
  window.addEventListener('pointerup', onUp);
  window.addEventListener('pointercancel', cancel);
  window.addEventListener('keydown', onKey, true);
  window.addEventListener('touchmove', onTouchMove, { passive: false });
  window.addEventListener('contextmenu', onContext);
}

function onMove(e: PointerEvent) {
  if (!st || e.pointerId !== st.pointerId) return;
  st.x = e.clientX;
  st.y = e.clientY;
  if (!st.started) {
    const dist = Math.hypot(st.x - st.x0, st.y - st.y0);
    if (st.type === 'mouse' && dist > 5) start();
    else if (st.type !== 'mouse' && dist > 10) return cleanup(); // 手指在滑动侧栏
    if (!st?.started) return;
  }
  e.preventDefault();
  if (!st.raf) st.raf = requestAnimationFrame(frame);
}

function start() {
  if (!st) return;
  clearTimeout(st.timer);
  st.started = true;
  const g = document.createElement('div');
  g.className = 'label-ghost';
  g.style.setProperty('--lc', st.label.color);
  const dot = document.createElement('i');
  g.append(dot, document.createTextNode(st.label.name));
  document.body.appendChild(g);
  st.ghost = g;
  document.body.classList.add('is-dragging', 'label-dragging');
  frame();
}

/** 指针下的卡片：白板状态块和侧栏「未整理」里的卡片 */
function cardAt(x: number, y: number) {
  return document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-drop] .card[data-id]') ?? null;
}

function frame() {
  if (!st?.ghost) return;
  st.raf = undefined;
  st.ghost.style.transform = `translate(${st.x + 12}px, ${st.y + 14}px)`;
  const card = cardAt(st.x, st.y);
  if (card !== st.target) {
    st.target?.classList.remove('label-target');
    st.target = card;
    if (card) {
      card.style.setProperty('--lc', st.label.color);
      card.classList.add('label-target');
    }
  }
  const id = card ? Number(card.dataset.id) : null;
  const has = id != null && !!useData.getState().board?.cards[id]?.labels.includes(st.label.id);
  st.ghost.classList.toggle('has', has);
  st.ghost.classList.toggle('over', !!card && !has);
}

function onUp(e: PointerEvent) {
  if (!st || e.pointerId !== st.pointerId) return;
  if (!st.started) return cleanup();
  const label = st.label;
  const card = cardAt(e.clientX, e.clientY);
  endedAt = performance.now();
  cleanup();
  if (card) add(Number(card.dataset.id), label);
}

async function add(id: number, label: Label) {
  const c = useData.getState().board?.cards[id];
  if (c?.labels.includes(label.id)) {
    toast({ type: 'info', title: `#${id} 已经有「${label.name}」`, duration: 1800 });
    return;
  }
  try {
    await exec({ op: 'label', id, add: [label.id] });
    flash(id, label.color, `加上了「${label.name}」`);
  } catch (e) {
    toastError(e);
  }
}

function cancel() {
  if (st?.started) endedAt = performance.now();
  cleanup();
}

function onKey(e: KeyboardEvent) {
  if (e.key !== 'Escape' || !st?.started) return;
  e.preventDefault();
  e.stopPropagation();
  cancel();
}

function onTouchMove(e: TouchEvent) {
  if (st?.started) e.preventDefault();
}

function onContext(e: Event) {
  if (st) e.preventDefault();
}

function cleanup() {
  if (!st) return;
  clearTimeout(st.timer);
  if (st.raf) cancelAnimationFrame(st.raf);
  st.ghost?.remove();
  st.target?.classList.remove('label-target');
  document.body.classList.remove('is-dragging', 'label-dragging');
  st = null;
  window.removeEventListener('pointermove', onMove);
  window.removeEventListener('pointerup', onUp);
  window.removeEventListener('pointercancel', cancel);
  window.removeEventListener('keydown', onKey, true);
  window.removeEventListener('touchmove', onTouchMove);
  window.removeEventListener('contextmenu', onContext);
}
