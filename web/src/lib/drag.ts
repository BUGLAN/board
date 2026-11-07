/* ==========================================================================
   卡片交互（白板状态块与侧栏「未整理」共用）
   - 拖拽：鼠标按下移动 5px 开始；触摸长按 220ms 开始
   - 拖动中按规则预判：不允许的状态显示禁止原因；松手后弹回原位并提示（PRD §10.3）
   - 点击 / 回车打开详情；方向键在卡片间移动
   ========================================================================== */
import { flushSync } from 'react-dom';
import { openCard } from './actions';
import * as flip from './flip';
import { boardFlow, INBOX, type FlowQ } from './schema';
import type { Card, ColumnKey } from './types';
import { reduceMotion } from './util';
import { exec, moveLocally, refresh, useData } from '../store/data';
import { rejectColumn, setDrag, toastError, useUI, type DragUI } from '../store/ui';

interface Veto {
  code: string;
  message: string;
  title: string;
  holder?: string | null;
}

/** 人在界面上移动卡片的规则（与服务端一致；服务端仍会再检查一次） */
export function vetoFor(c: Card, to: ColumnKey, f: FlowQ): Veto | null {
  if (c.holder) {
    return {
      code: 'NOT_HOLDER',
      message: `#${c.id} 由 ${c.holder} 持有，只有持有者能移动。需要的话，打开卡片「强制释放」。`,
      title: `${c.holder} 正持有这张卡片`,
    };
  }
  if (to !== c.column && f.isLock(to)) {
    return {
      code: 'FORBIDDEN_COLUMN',
      message: `「${f.name(to)}」已锁定：卡片只能由 agent 通过 board claim 领取进入`,
      title: '只能由 agent 领取进入',
    };
  }
  return null;
}

/** 白板上状态块里的卡片列表，按工作流顺序 */
const flowLists = () => {
  const f = boardFlow(useData.getState().board);
  return f.list.map(s => listEl(s.key)).filter((l): l is HTMLElement => !!l);
};

interface St {
  id: number;
  card: HTMLElement;
  pointerId: number;
  type: string;
  x0: number;
  y0: number;
  x: number;
  y: number;
  fromCol: ColumnKey;
  fromIndex: number;
  started: boolean;
  rot: number;
  vx: number;
  lastX: number;
  lastT: number;
  timer?: number;
  raf?: number;
  ghost?: HTMLElement;
  offX: number;
  offY: number;
  scrolling: boolean;
  target: { col: ColumnKey; index: number } | null;
  valid: boolean;
  veto: Veto | null;
}

let st: St | null = null;
let justDroppedAt = 0;
export const justDropped = () => performance.now() - justDroppedAt < 150;

const cardEl = (id: number) => document.querySelector<HTMLElement>(`[data-flip="c${id}"]`);
const listEl = (col: string) => document.querySelector<HTMLElement>(`[data-list="${col}"]`);
const zones = () =>
  [...document.querySelectorAll<HTMLElement>('[data-drop]')].filter(z => z.offsetParent !== null && z.getBoundingClientRect().width > 40);

export function installCardInteractions() {
  const cardOf = (e: Event) => (e.target as HTMLElement).closest?.<HTMLElement>('[data-drop] .card');

  const onClick = (e: MouseEvent) => {
    const card = cardOf(e);
    if (!card || justDropped() || (e.target as HTMLElement).closest('button, a')) return;
    openCard(Number(card.dataset.id));
  };
  const onKey = (e: KeyboardEvent) => {
    const card = cardOf(e);
    if (!card || e.target !== card) return;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      openCard(Number(card.dataset.id));
    }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const sib = (e.key === 'ArrowDown' ? card.nextElementSibling : card.previousElementSibling) as HTMLElement | null;
      sib?.focus();
    }
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      e.preventDefault();
      const d = e.key === 'ArrowRight' ? 1 : -1;
      const lists = flowLists();
      const at = lists.indexOf(card.parentElement as HTMLElement);
      for (let k = at + d; at >= 0 && k >= 0 && k < lists.length; k += d) {
        const first = lists[k].firstElementChild as HTMLElement | null;
        if (first) {
          first.focus();
          break;
        }
      }
    }
  };
  const onGlow = (e: PointerEvent) => {
    const card = cardOf(e);
    if (!card || e.pointerType !== 'mouse') return;
    const r = card.getBoundingClientRect();
    card.style.setProperty('--mx', `${e.clientX - r.left}px`);
    card.style.setProperty('--my', `${e.clientY - r.top}px`);
  };
  const onTouchMove = (e: TouchEvent) => {
    if (st?.started) e.preventDefault();
  };
  const onContext = (e: Event) => {
    if (st) e.preventDefault();
  };

  document.addEventListener('click', onClick);
  document.addEventListener('keydown', onKey);
  document.addEventListener('pointermove', onGlow);
  document.addEventListener('pointerdown', onDown);
  document.addEventListener('touchmove', onTouchMove, { passive: false });
  document.addEventListener('contextmenu', onContext);
  return () => {
    document.removeEventListener('click', onClick);
    document.removeEventListener('keydown', onKey);
    document.removeEventListener('pointermove', onGlow);
    document.removeEventListener('pointerdown', onDown);
    document.removeEventListener('touchmove', onTouchMove);
    document.removeEventListener('contextmenu', onContext);
  };
}

function onDown(e: PointerEvent) {
  const t = e.target as HTMLElement;
  const card = t.closest?.<HTMLElement>('[data-drop] .card');
  if (!card || e.button !== 0 || st) return;
  if (t.closest('button, a, input, textarea')) return;
  const id = Number(card.dataset.id);
  const list = card.parentElement!;
  st = {
    id, card, pointerId: e.pointerId, type: e.pointerType,
    x0: e.clientX, y0: e.clientY, x: e.clientX, y: e.clientY,
    fromCol: list.dataset.list as ColumnKey,
    fromIndex: [...list.children].indexOf(card),
    started: false, rot: 0, vx: 0, lastX: e.clientX, lastT: performance.now(),
    offX: 0, offY: 0, scrolling: false, target: null, valid: true, veto: null,
  };
  if (e.pointerType !== 'mouse') {
    st.timer = window.setTimeout(() => {
      if (st && !st.started) start();
    }, 220);
  }
  window.addEventListener('pointermove', onMove, { passive: false });
  window.addEventListener('pointerup', onUp);
  window.addEventListener('pointercancel', onCancel);
  window.addEventListener('keydown', onEsc);
}

function onMove(e: PointerEvent) {
  if (!st || e.pointerId !== st.pointerId) return;
  st.x = e.clientX;
  st.y = e.clientY;
  const dist = Math.hypot(st.x - st.x0, st.y - st.y0);
  if (!st.started) {
    if (st.type === 'mouse' && dist > 5) start();
    else if (st.type !== 'mouse' && dist > 10) return cleanup(); // 手指在滑动页面
    if (!st?.started) return;
  }
  e.preventDefault();
  const now = performance.now();
  const dt = Math.max(1, now - st.lastT);
  st.vx = st.vx * 0.7 + ((st.x - st.lastX) / dt) * 0.3;
  st.lastX = st.x;
  st.lastT = now;
  if (!st.raf) st.raf = requestAnimationFrame(frame);
}

function start() {
  if (!st) return;
  const s = st;
  clearTimeout(s.timer);
  const card = useData.getState().board?.cards[s.id];
  if (!card) return cleanup();
  s.started = true;
  const r = s.card.getBoundingClientRect();
  s.offX = s.x0 - r.left;
  s.offY = s.y0 - r.top;
  const ghost = s.card.cloneNode(true) as HTMLElement;
  ghost.classList.remove('flash', 'entering', 'is-open', 'pulse');
  ghost.removeAttribute('data-flip');
  ghost.querySelector('.card-badge')?.remove();
  ghost.classList.add('drag-ghost');
  ghost.style.width = `${r.width}px`;
  ghost.style.animation = 'none';
  ghost.style.transform = `translate(${r.left}px, ${r.top}px)`;
  document.body.appendChild(ghost);
  s.ghost = ghost;
  document.body.classList.add('is-dragging');
  s.target = { col: s.fromCol, index: s.fromIndex };
  setDrag({ id: s.id, fromCol: s.fromCol, col: s.fromCol, index: s.fromIndex, over: s.fromCol, valid: true, veto: null });
  if (s.type !== 'mouse') navigator.vibrate?.(10);
  ghost.animate([{ scale: '1' }, { scale: '1.045' }], { duration: 220, easing: 'cubic-bezier(.34,1.56,.64,1)', fill: 'forwards' });
  frame();
}

function frame() {
  if (!st?.started || !st.ghost) return;
  st.raf = 0;
  const tx = st.x - st.offX;
  const ty = st.y - st.offY;
  const targetRot = Math.max(-7, Math.min(7, st.vx * 9));
  st.rot += (targetRot - st.rot) * 0.25;
  st.vx *= 0.86; // 指针停下后，倾斜慢慢回正
  st.ghost.style.transform = `translate(${tx}px, ${ty}px) rotate(${st.rot.toFixed(2)}deg)`;
  locate();
  autoScroll();
  if (st && (Math.abs(st.rot) > 0.05 || st.scrolling)) st.raf = requestAnimationFrame(frame);
}

/** 找到指针下的列与插入位置，并按规则预判 */
function locate() {
  const s = st!;
  const cols = zones();
  let col: HTMLElement | undefined;
  for (const c of cols) {
    const r = c.getBoundingClientRect();
    if (s.x >= r.left - 4 && s.x <= r.right + 4 && s.y >= r.top - 40 && s.y <= r.bottom + 40) {
      col = c;
      break;
    }
  }
  const key = (col?.dataset.drop ?? s.target?.col ?? s.fromCol) as ColumnKey;
  const list = listEl(key);
  const items = list ? [...list.children].filter(n => n !== s.card && n.getAttribute('data-flip') !== `c${s.id}`) : [];
  const index = insertIndex(items, s.x, s.y);
  const same = key === s.fromCol && index === s.fromIndex;
  const state = useData.getState();
  const card = state.board?.cards[s.id];
  let veto: Veto | null = null;
  if (!same && card) {
    veto = vetoFor({ ...card, column: s.fromCol }, key, boardFlow(state.board));
    if (veto && key === INBOX && veto.code !== 'NOT_HOLDER') veto.title = '不能放回「未整理」';
  }
  const valid = !veto;
  const prev = useUI.getState().drag;
  s.target = { col: key, index };
  s.valid = valid;
  s.veto = veto;
  const dest = valid ? s.target : { col: s.fromCol, index: s.fromIndex };
  const next: DragUI = {
    id: s.id, fromCol: s.fromCol, col: dest.col, index: dest.index, over: key, valid,
    veto: veto ? { title: veto.title, message: veto.message, code: veto.code } : null,
  };
  if (prev && prev.col === next.col && prev.index === next.index && prev.over === next.over && prev.valid === next.valid) return;
  // 占位移动时，让同列卡片补间让位
  if (!prev || prev.col !== next.col || prev.index !== next.index) flip.capture();
  setDrag(next);
  if (s.ghost) {
    s.ghost.classList.toggle('bad', !valid);
    s.ghost.querySelector('.ghost-hint')?.remove();
    if (veto) {
      const hint = document.createElement('span');
      hint.className = 'ghost-hint';
      hint.innerHTML =
        '<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="4.5" y="8.5" width="11" height="8" rx="2.2"/><path d="M7 8.5V6.6a3 3 0 0 1 6 0v1.9"/></svg>';
      const t = document.createElement('span');
      t.textContent = veto.title;
      hint.appendChild(t);
      s.ghost.appendChild(hint);
    }
  }
}

/** 插入位置。卡片是瀑布流（lib/masonry.ts）：先找指针所在的那一竖列（没有正对的就取最近的一列），
    插到这一列里第一张「中线在指针下方」的卡片前面；指针在这一列所有卡片下方时，插到这一列最后一张之后 */
function insertIndex(items: Element[], x: number, y: number): number {
  if (!items.length) return 0;
  const rects = items.map(el => el.getBoundingClientRect());
  let best = Infinity;
  let track = 0;
  rects.forEach(r => {
    const d = x < r.left ? r.left - x : x > r.right ? x - r.right : 0;
    if (d < best - 0.5) {
      best = d;
      track = r.left;
    }
  });
  let last = -1;
  for (let i = 0; i < rects.length; i++) {
    const r = rects[i];
    if (Math.abs(r.left - track) > 1) continue;
    if (y < r.top + r.height / 2) return i;
    last = i;
  }
  return last + 1;
}

/** 靠近边缘时自动滚动（列内纵向 / 白板横向） */
function autoScroll() {
  const s = st!;
  s.scrolling = false;
  const edge = 56;
  const speed = 14;
  const zone = document.querySelector<HTMLElement>(`[data-drop="${s.target?.col || s.fromCol}"]`);
  // 白板上整块区域纵向滚动；侧栏「未整理」在自己的列表里滚动
  const body = zone?.closest<HTMLElement>('.flow-scroll') ?? zone?.querySelector<HTMLElement>('.side-scroll');
  if (body) {
    const r = body.getBoundingClientRect();
    if (s.y < r.top + edge && body.scrollTop > 0) {
      body.scrollTop -= speed * (1 - (s.y - r.top) / edge);
      s.scrolling = true;
    } else if (s.y > r.bottom - edge && body.scrollTop < body.scrollHeight - body.clientHeight) {
      body.scrollTop += speed * (1 - (r.bottom - s.y) / edge);
      s.scrolling = true;
    }
  }
  const board = document.querySelector<HTMLElement>('.board');
  if (board && board.scrollWidth > board.clientWidth) {
    const br = board.getBoundingClientRect();
    if (s.x < br.left + edge) {
      board.scrollLeft -= speed;
      s.scrolling = true;
    } else if (s.x > br.right - edge) {
      board.scrollLeft += speed;
      s.scrolling = true;
    }
  }
}

function onUp(e: PointerEvent) {
  if (!st || e.pointerId !== st.pointerId) return;
  if (!st.started) return cleanup();
  finish(false);
}
function onCancel() {
  if (st?.started) finish(true);
  else cleanup();
}
function onEsc(e: KeyboardEvent) {
  if (e.key === 'Escape' && st?.started) {
    e.preventDefault();
    e.stopPropagation();
    finish(true);
  }
}

function finish(cancelled: boolean) {
  const s = st!;
  locate();
  const { target, valid, veto } = s;
  const same = !target || (target.col === s.fromCol && target.index === s.fromIndex);
  const commit = !cancelled && valid && !same && target;
  let failed = false;

  if (!commit) {
    flushSync(() => setDrag({ ...useUI.getState().drag!, col: s.fromCol, index: s.fromIndex, valid: true, veto: null, over: null }));
    if (!cancelled && veto && target) {
      toastError({ code: veto.code, message: veto.message });
      rejectColumn(target.col);
    }
  } else {
    exec({ op: 'move', id: s.id, column: target.col, index: target.index }).catch(err => {
      failed = true;
      toastError(err);
      refresh();
    });
  }

  // 幽灵飞回占位处
  const ghost = s.ghost!;
  const dest = cardEl(s.id)?.getBoundingClientRect();
  const anim = ghost.animate(
    [
      { transform: ghost.style.transform, scale: '1.045' },
      { transform: dest ? `translate(${dest.left}px, ${dest.top}px) rotate(0deg)` : ghost.style.transform, scale: '1' },
    ],
    {
      duration: reduceMotion() ? 1 : commit ? 280 : 460,
      easing: commit ? 'cubic-bezier(.2,1.2,.4,1)' : 'cubic-bezier(.34,1.3,.64,1)',
      fill: 'forwards',
    },
  );
  const id = s.id;
  anim.onfinish = () => {
    if (commit && !failed) moveLocally(id, target.col, target.index);
    setDrag(null);
    ghost.remove();
    if (commit && !failed) {
      requestAnimationFrame(() =>
        cardEl(id)?.animate([{ transform: 'scale(1.03)' }, { transform: 'none' }], { duration: 300, easing: 'cubic-bezier(.34,1.56,.64,1)' }),
      );
    }
  };
  justDroppedAt = performance.now();
  cleanup(true);
}

function cleanup(keepDrag = false) {
  if (!st) return;
  clearTimeout(st.timer);
  if (st.raf) cancelAnimationFrame(st.raf);
  document.body.classList.remove('is-dragging');
  window.removeEventListener('pointermove', onMove);
  window.removeEventListener('pointerup', onUp);
  window.removeEventListener('pointercancel', onCancel);
  window.removeEventListener('keydown', onEsc);
  if (!keepDrag && st.started) setDrag(null);
  st = null;
}

export const COLUMN_TITLE = (col: ColumnKey) => `不能放进「${boardFlow(useData.getState().board).name(col)}」`;
