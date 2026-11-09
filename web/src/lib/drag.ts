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
import { sortIds, sortKey } from './sort';
import type { Card, ColumnKey } from './types';
import { reduceMotion } from './util';
import { boardMeta, cardBusy, exec, hold, isDraft, movePatch, serverNow, useData } from '../store/data';
import { rejectColumn, setDrag, setSort, toast, toastError, useUI, type DragUI } from '../store/ui';

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

/** 卡片详情「移动到」：相当于把卡片拖到目标状态的末尾。规则与拖拽一致（vetoFor），不允许时给出同样的提示、不发请求；
    当帧在本地落位，服务端拒绝时回到原处并提示 */
export async function moveTo(id: number, to: ColumnKey) {
  const board = useData.getState().board;
  const card = board?.cards[id];
  if (!board || !card || card.column === to) return;
  const f = boardFlow(board);
  const veto = vetoFor(card, to, f);
  if (veto) {
    toastError({ code: veto.code, message: veto.message });
    return;
  }
  // 末尾：目标块优先级顺序的最后（排序中的块也一样，与从别的块拖到最后一张之后相同）
  try {
    await exec({ op: 'move', id, column: to }, movePatch(id, to));
    toast({ type: 'success', title: `#${id} 已移到「${f.name(to)}」`, duration: 1800 });
  } catch (e) {
    toastError(e);
  }
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
  /** 被拖卡片的高度（占位的高度） */
  h: number;
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
  if (isDraft(id) || cardBusy(id)) return; // 还在新建、保存中的卡片不能拖（保存完再动）
  const list = card.parentElement!;
  st = {
    id, card, pointerId: e.pointerId, type: e.pointerType,
    x0: e.clientX, y0: e.clientY, x: e.clientX, y: e.clientY,
    fromCol: list.dataset.list as ColumnKey,
    fromIndex: [...list.children].indexOf(card),
    started: false, rot: 0, vx: 0, lastX: e.clientX, lastT: performance.now(),
    offX: 0, offY: 0, h: card.offsetHeight, scrolling: false, target: null, valid: true, veto: null,
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
  const col = zoneAt(zones(), s.x, s.y, s.target?.col);
  const key = (col?.dataset.drop ?? s.target?.col ?? s.fromCol) as ColumnKey;
  const list = listEl(key);
  const items = list ? ([...list.children] as HTMLElement[]).filter(n => n !== s.card && n.getAttribute('data-flip') !== `c${s.id}`) : [];
  const index = insertIndex(list, items, s.x, s.y, cardEl(s.id)?.offsetHeight || s.h);
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

/** 指针落在哪一块。块的命中范围上下放宽 40px、左右放宽 4px（指针在两块之间的空隙里也算数），但放宽的范围会盖进挨得近的隔壁块：
    以前按顺序取第一个命中的，指针明明已经在下面那块（例如「已完成」）的顶部，却判成上面那块（例如锁定的「进行中」）；
    指针在分界附近抖一下，就在「锁定」和「可放入」之间来回切，锁的提示一闪一闪。现在：
    - 指针在块里面：就是这一块；
    - 在空隙里：取离指针最近的一块；
    - 迟滞：当前这块（cur）还在范围里、别的块只近不到 HYSTERESIS 像素时，保持当前这块，指针在分界上抖动不会来回切 */
const HYSTERESIS = 6;
function zoneAt(cols: HTMLElement[], x: number, y: number, cur: ColumnKey | undefined): HTMLElement | undefined {
  let best: HTMLElement | undefined;
  let bestD = Infinity;
  let curEl: HTMLElement | undefined;
  let curD = Infinity;
  for (const c of cols) {
    const r = c.getBoundingClientRect();
    const dx = x < r.left ? r.left - x : x > r.right ? x - r.right : 0;
    const dy = y < r.top ? r.top - y : y > r.bottom ? y - r.bottom : 0;
    if (dx > 4 || dy > 40) continue;
    const d = Math.hypot(dx, dy);
    if (d < bestD) {
      best = c;
      bestD = d;
    }
    if (c.dataset.drop === cur) {
      curEl = c;
      curD = d;
    }
  }
  return curEl && curD - bestD < HYSTERESIS ? curEl : best;
}

/** 插入位置。卡片是瀑布流（lib/masonry.ts），占位一挪，别的卡片换竖列、换高度。
    如果按「当前」画面里卡片的位置算，占位挪过去后画面变了、算出的位置又变，占位就会来回跳。
    所以不看当前画面：对每个候选位置，按网格的摆放规则推算「占位放在这里时」它会落在哪，
    取离指针最近的那个（指针在里面时距离为 0，再按离中心的距离比）。结果只由指针决定，指针不动占位就不动 */
function insertIndex(list: HTMLElement | null, items: HTMLElement[], x: number, y: number, phHeight: number): number {
  if (!list || !items.length) return 0;
  const cs = getComputedStyle(list);
  const gap = parseFloat(cs.columnGap) || 0;
  const tracks = cs.gridTemplateColumns.split(/\s+/).map(parseFloat).filter(n => n > 0);
  const w = tracks[0] ?? list.clientWidth;
  const T = Math.max(1, tracks.length);
  const lr = list.getBoundingClientRect();
  const left0 = lr.left + list.clientLeft + (parseFloat(cs.paddingLeft) || 0);
  const top0 = lr.top + list.clientTop + (parseFloat(cs.paddingTop) || 0);
  const span = (h: number) => (h ? Math.max(1, Math.round(h + gap)) : 1); // 与 masonry.apply 一致
  const spans = items.map(el => span(el.offsetHeight));
  const ph = span(phHeight);

  let best = 0;
  let bestD = Infinity;
  let bestC = Infinity;
  for (let i = 0; i <= items.length; i++) {
    const slot = placeAt([...spans.slice(0, i), ph, ...spans.slice(i)], T, i);
    const l = left0 + slot.col * (w + gap);
    const t = top0 + slot.row;
    const r = l + w;
    const b = t + Math.max(1, phHeight);
    const d = Math.hypot(x < l ? l - x : x > r ? x - r : 0, y < t ? t - y : y > b ? y - b : 0);
    const c = Math.hypot(x - (l + r) / 2, y - (t + b) / 2);
    if (d < bestD - 0.5 || (Math.abs(d - bestD) <= 0.5 && c < bestC)) {
      best = i;
      bestD = d;
      bestC = c;
    }
  }
  return best;
}

/** 按 CSS 网格自动摆放（grid-auto-flow: row，行高 1px，每张卡片占一竖列、span 行）推算第 k 张的位置。
    游标只往后走，所以某竖列在第 r 行能放下，当且仅当这一列已占用的底部 ≤ r */
export function placeAt(spans: number[], T: number, k: number): { row: number; col: number } {
  const bottom = new Array<number>(T).fill(0);
  let row = 0;
  let col = 0;
  for (let i = 0; i < spans.length; i++) {
    let c = -1;
    for (let j = col; j < T; j++) {
      if (bottom[j] <= row) {
        c = j;
        break;
      }
    }
    if (c < 0) {
      row = Math.max(row + 1, Math.min(...bottom));
      c = bottom.findIndex(v => v <= row);
    }
    bottom[c] = row + spans[i];
    if (i === k) return { row, col: c };
    col = c + 1;
  }
  return { row, col };
}

/** 靠近上下边缘时自动纵向滚动。白板（.board）是磁吸布局、不横向滚动（overflow-x: hidden），这里也不横向滚：
    以前指针靠近右边缘时会给白板加 scrollLeft，只要有一点横向溢出（例如右侧磁贴的悬停卡片），整块内容就左移，
    而 overflow-x: hidden 人滚不回来（BoardView 的 keepLeft 兜底把它拉回 0） */
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
  let done: Promise<unknown> | null = null;
  let plan: Drop = { index: target?.index ?? 0 };

  if (!commit) {
    flushSync(() => setDrag({ ...useUI.getState().drag!, col: s.fromCol, index: s.fromIndex, valid: true, veto: null, over: null }));
    if (!cancelled && veto && target) {
      toastError({ code: veto.code, message: veto.message });
      rejectColumn(target.col);
    }
  } else {
    plan = planDrop(s.id, s.fromCol, target);
    // 松手当帧就画成「保存中」（不能再拖、再改），落位的补丁等飞回动画结束再叠（hold）
    done = exec({ op: 'move', id: s.id, column: target.col, index: plan.index, order: plan.order }, { cards: [s.id] });
    done.catch(err => {
      failed = true;
      toastError(err);
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
    if (commit && !failed && done) {
      if (plan.clear) setSort(plan.clear, null);
      // 落位：叠一层补丁直到服务端确认并刷新（期间推送触发的刷新不会把它冲回原处）；被拒绝时补丁撤掉，卡片回到原处
      hold(movePatch(id, target.col, plan.index, plan.order), done);
    }
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

interface Drop {
  /** 优先级顺序里的位置（不计被拖的卡片本身） */
  index: number;
  /** 整块的新顺序，一起保存为优先级 */
  order?: number[];
  /** 落位后结束这块的排序状态 */
  clear?: string;
}

/** 松手位置换算成要保存的优先级。占位的 index 是「看到的顺序」里的位置：
    - 目标块没在排序：看到的就是优先级顺序，原样保存
    - 在排序中的块里拖动：按拖动后看到的顺序整块保存为优先级，这块的排序状态结束
    - 从别的块拖进排序中的块：放到看到的下一张卡片前面（优先级顺序里），这块继续排序，别的块不动 */
function planDrop(id: number, from: ColumnKey, target: { col: ColumnKey; index: number }): Drop {
  const { board, path, ws } = useData.getState();
  const key = sortKey(path, target.col);
  const spec = useUI.getState().sorts[key];
  if (!board || !spec) return { index: target.index };
  const labels = boardMeta(ws, path)?.project.labels ?? [];
  const prio = (board.cols[target.col] ?? []).filter(x => x !== id);
  const shown = sortIds(prio, board.cards, spec, labels, serverNow());
  if (target.col === from) {
    const order = [...shown];
    order.splice(Math.min(target.index, order.length), 0, id);
    return { index: target.index, order, clear: key };
  }
  const next = shown[target.index];
  return { index: next == null ? prio.length : prio.indexOf(next) };
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
