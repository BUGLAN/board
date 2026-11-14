/* ==========================================================================
   拖动标签（侧栏「标签」里的胶囊）：拖到卡片上是加标签，拖在标签区里是调整顺序
   - 鼠标按下移动 5px 开始；触摸长按 220ms 开始（与卡片拖动一致）；只是点击时仍是按标签筛选
   - ghost 在指针下面：按下时指针在胶囊上的相对位置，就是指针在 ghost 上的位置（像把胶囊拿起来）
   - 经过的卡片高亮（描边 + 一圈脉冲 + 轻微放大），ghost 放大吸附；松手在卡片上就加标签，
     ghost 同时飞进卡片的标签区（≤240ms，不等请求，下一次拖动会打断它）；已经有这个标签时只提示；Esc 取消
   - 在标签区里拖动：两个胶囊之间出现插入竖线，松手当帧换位（其余胶囊补间让位），
     顺序写进项目（store/data.ts reorderLabels），卡片上的标签跟着变；保存失败时回到原来的顺序并提示
   - 减弱动态效果时不放大、不飞行，松手直接落位
   - 不用浏览器原生拖放：桌面端（Tauri）在 Windows 上会拦下原生拖放，触屏上也不可用
   ========================================================================== */
import type { PointerEvent as ReactPointerEvent } from 'react';
import { flushSync } from 'react-dom';
import { addLabel, reorderLabels, useData } from '../store/data';
import { flash, toast, toastError } from '../store/ui';
import type { Label } from './types';
import { reduceMotion } from './util';

/** 标签区里的插入位置：插到第 index 个胶囊前面（index = 数量时排到最后）；el / side 是画竖线的胶囊和哪一侧 */
interface Slot {
  index: number;
  el: HTMLElement;
  side: 'before' | 'after';
}

interface St {
  label: Label;
  project: string;
  /** 标签区（.lb-field）和被拖的胶囊（.lb-item） */
  field: HTMLElement | null;
  item: HTMLElement | null;
  pointerId: number;
  type: string;
  x0: number;
  y0: number;
  x: number;
  y: number;
  /** 指针在胶囊上的相对位置（0–1）；ghost 建好后换算成 ghost 上的像素偏移 ox / oy */
  rx: number;
  ry: number;
  ox: number;
  oy: number;
  started: boolean;
  timer?: number;
  raf?: number;
  ghost?: HTMLElement;
  target: HTMLElement | null;
  slot: Slot | null;
}

let st: St | null = null;
/** 松手后正在飞向卡片的 ghost：新的拖动开始时直接收掉 */
let flying: Animation | null = null;
/** 刚拖完：松手时浏览器还会补一次 click，胶囊据此不触发筛选 */
let endedAt = 0;
export const labelDragJustEnded = () => performance.now() - endedAt < 400;

/** 标签胶囊的 onPointerDown：拖到卡片上加标签，在侧栏标签区里拖动调整顺序 */
export function labelPointerDown(e: ReactPointerEvent, label: Label, project: string) {
  if (e.button !== 0 || st) return;
  const el = e.currentTarget as HTMLElement;
  const r = el.getBoundingClientRect();
  const rel = (v: number, a: number, len: number) => (len > 0 ? Math.min(1, Math.max(0, (v - a) / len)) : 0.5);
  st = {
    label,
    project,
    field: el.closest<HTMLElement>('.lb-field'),
    item: el.closest<HTMLElement>('.lb-item'),
    pointerId: e.pointerId,
    type: e.pointerType,
    x0: e.clientX,
    y0: e.clientY,
    x: e.clientX,
    y: e.clientY,
    rx: rel(e.clientX, r.left, r.width),
    ry: rel(e.clientY, r.top, r.height),
    ox: 0,
    oy: 0,
    started: false,
    target: null,
    slot: null,
  };
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
  flying?.finish();
  clearTimeout(st.timer);
  st.started = true;
  const g = document.createElement('div');
  g.className = 'label-ghost';
  g.style.setProperty('--lc', st.label.color);
  const dot = document.createElement('i');
  g.append(dot, document.createTextNode(st.label.name));
  document.body.appendChild(g);
  // 指针落在 ghost 上与按下时在胶囊上相同的位置；放大、出现动画也以这一点为中心，ghost 不会从指针下面滑开
  st.ox = Math.round(st.rx * g.offsetWidth);
  st.oy = Math.round(st.ry * g.offsetHeight);
  g.style.transformOrigin = `${st.ox}px ${st.oy}px`;
  st.ghost = g;
  st.item?.classList.add('lb-lifted');
  document.body.classList.add('is-dragging', 'label-dragging');
  frame();
}

/** 指针下的卡片：白板状态块和侧栏「未整理」里的卡片 */
function cardAt(x: number, y: number) {
  const card = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-drop] .card[data-id]') ?? null;
  // 还在新建的临时卡片（负数 ID）不能加标签
  return card && Number(card.dataset.id) > 0 ? card : null;
}

const items = (field: HTMLElement) => [...field.querySelectorAll<HTMLElement>(':scope > .lb-item')];

/** 指针在标签区里时的插入位置：先找指针所在的那一行（胶囊会换行），再按胶囊中线决定插在谁前面。
    插回原处（自己前后）算没有移动，返回 null */
function slotAt(x: number, y: number): Slot | null {
  const f = st?.field;
  if (!f || !st?.item) return null;
  const fr = f.getBoundingClientRect();
  if (x < fr.left - 8 || x > fr.right + 8 || y < fr.top - 8 || y > fr.bottom + 8) return null;
  const list = items(f).map((el, i) => ({ el, i, r: el.getBoundingClientRect() }));
  if (!list.length) return null;
  let best = Infinity;
  let top = 0;
  for (const { r } of list) {
    const d = y < r.top ? r.top - y : y > r.bottom ? y - r.bottom : 0;
    if (d < best) [best, top] = [d, r.top];
  }
  const row = list.filter(o => Math.abs(o.r.top - top) < 2);
  const hit = row.find(o => x < o.r.left + o.r.width / 2);
  const last = row[row.length - 1];
  const slot: Slot = hit ? { index: hit.i, el: hit.el, side: 'before' } : { index: last.i + 1, el: last.el, side: 'after' };
  const from = list.findIndex(o => o.el === st!.item);
  return slot.index === from || slot.index === from + 1 ? null : slot;
}

function showSlot(slot: Slot | null) {
  if (!st) return;
  const prev = st.slot;
  if (prev?.el === slot?.el && prev?.side === slot?.side) return;
  prev?.el.classList.remove('ins-before', 'ins-after');
  st.slot = slot;
  slot?.el.classList.add(slot.side === 'before' ? 'ins-before' : 'ins-after');
}

function frame() {
  if (!st?.ghost) return;
  st.raf = undefined;
  // 用独立的 translate 属性跟随：它在 scale 之前合成，放大只围绕指针那一点，不会把位移一起放大
  // （写成 transform: translate() 时 scale 会作用在位移上，ghost 被推到指针右下方，越靠右下偏得越远）
  st.ghost.style.translate = `${st.x - st.ox}px ${st.y - st.oy}px`;
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
  showSlot(card ? null : slotAt(st.x, st.y));
}

function onUp(e: PointerEvent) {
  if (!st || e.pointerId !== st.pointerId) return;
  if (!st.started) return cleanup();
  const label = st.label;
  const card = cardAt(e.clientX, e.clientY);
  endedAt = performance.now();
  const slot = card ? null : slotAt(e.clientX, e.clientY);
  if (slot) return drop(slot);
  const id = card ? Number(card.dataset.id) : null;
  const has = id != null && !!useData.getState().board?.cards[id]?.labels.includes(label.id);
  // 落位动画：ghost 交给 fly()，cleanup 不再删它
  if (card && !has && st.ghost && !reduceMotion()) {
    fly(st.ghost, card, st.x - st.ox, st.y - st.oy);
    st.ghost = undefined;
  }
  cleanup();
  if (id != null) add(id, label);
}

/** 在标签区里松手：当帧换位，其余胶囊从旧位置补间到新位置，被拖的胶囊从 ghost 的位置落进空位 */
function drop(slot: Slot) {
  if (!st?.field || !st.item) return cleanup();
  const { field, item, project, label, ghost } = st;
  const ids = items(field).map(el => Number(el.dataset.label));
  const from = ids.indexOf(label.id);
  ids.splice(from, 1);
  ids.splice(slot.index > from ? slot.index - 1 : slot.index, 0, label.id);
  const animate = !reduceMotion();
  const before = new Map(items(field).map(el => [el, el.getBoundingClientRect()]));
  const g = ghost?.getBoundingClientRect();
  cleanup();
  let saving!: Promise<void>;
  // flushSync：顺序写进 store 后立即渲染，下面量到的就是新位置
  flushSync(() => {
    saving = reorderLabels(project, ids);
  });
  saving.catch(e => {
    toast({ type: 'error', title: '标签顺序没有保存，已恢复原来的顺序', code: (e as { code?: string }).code, text: (e as Error).message });
  });
  if (!animate) return;
  for (const el of items(field)) {
    const a = el === item && g ? g : before.get(el);
    const b = el.getBoundingClientRect();
    if (!a) continue;
    const dx = a.left - b.left;
    const dy = a.top - b.top;
    if (!dx && !dy) continue;
    el.animate(
      el === item
        ? [{ translate: `${dx}px ${dy}px`, scale: '1.06' }, { translate: '0 0', scale: '1' }]
        : [{ translate: `${dx}px ${dy}px` }, { translate: '0 0' }],
      { duration: el === item ? 260 : 220, easing: 'cubic-bezier(.22, 1.18, .36, 1)' },
    );
  }
}

/** ghost 从松手的位置飞进卡片标签区的末尾（没有标签时落在卡片左下），≤240ms 后消失 */
function fly(ghost: HTMLElement, card: HTMLElement, x: number, y: number) {
  flying?.finish();
  const r = card.getBoundingClientRect();
  const chips = card.querySelector('.card-labels');
  const last = chips?.lastElementChild?.getBoundingClientRect();
  const gh = ghost.offsetHeight || 26;
  const to = last ? { x: last.right + 4, y: last.top + (last.height - gh) / 2 } : { x: r.left + 13, y: r.bottom - 12 - gh };
  ghost.classList.add('landing');
  // 落位按 ghost 左上角对齐目标位置：缩放也以左上角为中心
  ghost.style.transformOrigin = '0 50%';
  const a = ghost.animate(
    [
      { translate: `${x}px ${y}px`, scale: '1.06', opacity: 1 },
      { translate: `${to.x}px ${to.y}px`, scale: '.82', opacity: 1, offset: 0.75 },
      { translate: `${to.x}px ${to.y}px`, scale: '.82', opacity: 0 },
    ],
    { duration: 220, easing: 'cubic-bezier(.16, 1, .3, 1)', fill: 'forwards' },
  );
  flying = a;
  const done = () => {
    ghost.remove();
    if (flying === a) flying = null;
  };
  a.onfinish = done;
  a.oncancel = done;
  card.classList.remove('label-landed');
  void card.offsetWidth;
  card.classList.add('label-landed');
  setTimeout(() => card.classList.remove('label-landed'), 420);
}

/** 加标签：卡片上当帧出现 pending 态的标签（store/data.ts addLabel），确认后转正；失败时撤回并提示 */
async function add(id: number, label: Label) {
  const c = useData.getState().board?.cards[id];
  if (c?.labels.includes(label.id)) {
    toast({ type: 'info', title: `#${id} 已经有「${label.name}」`, duration: 1800 });
    return;
  }
  try {
    if (await addLabel(id, label.id)) flash(id, label.color, `加上了「${label.name}」`);
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
  st.slot?.el.classList.remove('ins-before', 'ins-after');
  st.item?.classList.remove('lb-lifted');
  document.body.classList.remove('is-dragging', 'label-dragging');
  st = null;
  window.removeEventListener('pointermove', onMove);
  window.removeEventListener('pointerup', onUp);
  window.removeEventListener('pointercancel', cancel);
  window.removeEventListener('keydown', onKey, true);
  window.removeEventListener('touchmove', onTouchMove);
  window.removeEventListener('contextmenu', onContext);
}
