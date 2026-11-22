/* 「未整理 → 待办」的离场动效（default/main#361，原型里选的 A「飞进白板」）。
   整理当帧就从「未整理」里移走（乐观更新），这里只管看起来的样子：
   1. 槽：React 已经把这一行（整组都走了时是整组）摘掉了，把摘下来的那块包进一个槽、放回原来的位置。
      槽是列表里的一个块，收起时列表的边框、阴影、下面的行都跟着连续往上走，不跳、不留空白，也不用 FLIP 补位
   2. 按下：点的那个按钮在槽里按下去、填满品牌色；这一行染上一层品牌色，随即淡掉
   3. 飞走：行缩成一张小卡片（编号 + 标题，状态点从未整理的紫变成待办的颜色），沿弧线飞进「白板」入口
      （桌面在左侧活动栏，手机在底部标签栏）；入口弹一下、冒出 +n
   4. 提示：等小卡片落进入口再出来（TRIAGE_TOAST_DELAY），不和动画叠在一起
   和别的离场分得开：稍后再做是往右滑走，删除是原地淡出，合并是往上收。「减弱动效」时都不做，列表直接更新 */
import { reduceMotion, replay } from '../lib/util';

/** 提示晚这么久出来：小卡片落进入口的时候 */
export const TRIAGE_TOAST_DELAY = 520;
export const triageToastDelay = () => (reduceMotion() ? 0 : TRIAGE_TOAST_DELAY);

/** 收起槽的缓动：先快后慢、不过冲（高度不能小于 0，过冲会在最后一下顿住） */
const COLLAPSE = 'cubic-bezier(.32, .72, 0, 1)';
/** 一次整理很多条时，最多这么多条有自己的动作（错开），其余的跟着一起收 */
const MAX_STAGGER = 6;
const STAGGER = 35;
/** 小卡片从出现到落进入口 */
const FLY = 620;

/** 整理走的一行：el 是原来那一行（已经放回槽里），how 是从哪里点的（行上的图标、展开后的按钮），那个按钮按下去 */
export type LeaveRow = { el: HTMLElement; id: number; how?: 'row' | 'edit' };
/** 一个槽：unit 是从列表里摘下来的那块（一行，或整组），放回 host 里 before 前面 */
export type Slot = { slot: HTMLElement; unit: HTMLElement; host: HTMLElement; before: Node | null; rows: LeaveRow[] };

/** 行往上直到 root 的各层容器（含 root）：行被摘下来以后，靠它找回原来挂在哪 */
export function ancestors(el: HTMLElement, root: HTMLElement) {
  const up: HTMLElement[] = [];
  for (let a = el.parentElement; a; a = a === root ? null : a.parentElement) up.push(a);
  return up;
}

/** 把离开的行原地放回列表：prev 是上一次记下的行（按列表里的顺序，up 是当时的各层容器），
    claim 认领这次整理走的（返回 false 不要；放不回去的——原来的容器也没了——不会问它，留给调用方另外处理）。
    每块摘下来的（一行，或者整组都走了时的整组）包进一个槽，放在同一个容器里后面第一条还在的行前面；
    后面没有了就放在容器里最后一块后面（整块列表都空了：放在空状态前面）。
    槽里的行去掉 data-ib，不再算列表里的行。还没开始播动画 */
export function placeSlots(
  root: HTMLElement,
  prev: [number, { el: HTMLElement; up: HTMLElement[] }][],
  claim: (id: number) => LeaveRow['how'] | false,
): Slot[] {
  const slots: Slot[] = [];
  prev.forEach(([id, p], i) => {
    if (p.el.isConnected) return;
    // 摘下来的那块：从行往上找到最外面那层（React 只把它和原来的容器断开，里面的层级都还在）
    let unit = p.el;
    while (unit.parentElement) unit = unit.parentElement;
    const chain = [p.el, ...p.up];
    const host = chain[chain.indexOf(unit) + 1];
    if (!host?.isConnected) return;
    const how = claim(id);
    if (how === false) return;
    const had = slots.find(s => s.unit === unit);
    if (had) return void had.rows.push({ el: p.el, id, how });
    let before: Node | null = null;
    for (const [, q] of prev.slice(i + 1)) {
      if (!q.el.isConnected || !host.contains(q.el)) continue;
      let c: HTMLElement = q.el;
      while (c.parentElement && c.parentElement !== host) c = c.parentElement;
      before = c;
      break;
    }
    if (!before) {
      const tail = [...host.children].reverse().find(ch => ch.matches('.ib-row, .ib-grp, .ib-leave'));
      before = tail ? tail.nextSibling : host === root ? root.querySelector(':scope > .ib-empty, :scope > .ib-filtered') : null;
    }
    slots.push({ slot: document.createElement('div'), unit, host, before, rows: [{ el: p.el, id, how }] });
  });
  for (const s of slots) {
    s.slot.className = 'ib-leave';
    s.slot.setAttribute('aria-hidden', 'true');
    s.host.insertBefore(s.slot, s.before?.parentNode === s.host ? s.before : null);
    s.slot.appendChild(s.unit);
    [s.unit, ...s.unit.querySelectorAll<HTMLElement>('[data-ib]')].forEach(el => el.removeAttribute('data-ib'));
  }
  return slots;
}

/** 给槽里的行播离场（按下 → 缩成小卡片飞进「白板」入口），再把槽收起；全部收完调 settled */
export function playLeave(slots: Slot[], settled: () => void) {
  const rows = slots.flatMap(s => s.rows);
  let left = slots.length;
  const done = () => --left === 0 && settled();
  let flights = 0;
  rows.forEach((r, i) => {
    const k = Math.min(i, MAX_STAGGER);
    press(r);
    r.el.classList.add('leaving');
    r.el.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 180, delay: 40 + k * STAGGER, easing: 'ease-out', fill: 'forwards' });
    if (flights < MAX_STAGGER && fly(lineRect(r.el), r.id, rowTitle(r.el), k * STAGGER, idLeft(r.el))) flights++;
  });
  slots.forEach((s, i) => collapse(s.slot, 140 + Math.min(i, MAX_STAGGER) * STAGGER, 360, done));
  setTimeout(() => bumpBoard(rows.length), flights ? FLY - 80 + Math.min(flights - 1, MAX_STAGGER) * STAGGER : 300);
}

/** 逐条看时整理：卡片由逐条视图换下一张，这里补「去了白板」的那一下（卡片头飞进入口）。要在移走之前调，量得到位置 */
export function leaveOne(card: HTMLElement | null, id: number, title: string) {
  if (reduceMotion()) return;
  const head = card?.querySelector<HTMLElement>('.ib-one-head') ?? card;
  const flew = !!head && fly(head.getBoundingClientRect(), id, title, 0);
  setTimeout(() => bumpBoard(1), flew ? FLY - 80 : 260);
}

/* ------------------------------ 零件 ------------------------------ */

function collapse(slot: HTMLElement, delay: number, duration: number, done: () => void) {
  const h = slot.offsetHeight;
  // 高的槽（展开着的行、整组）收得慢一点：每帧挪的距离和一行时差不多，不会一下子塌下去
  duration += Math.min(Math.max(h - 60, 0) * 0.7, 300);
  slot.style.height = `${h}px`;
  let over = false;
  const end = () => {
    if (over) return;
    over = true;
    slot.remove();
    done();
  };
  slot.animate([{ height: `${h}px` }, { height: '0px' }], { duration, delay, easing: COLLAPSE, fill: 'forwards' }).onfinish = end;
  // 动画迟迟没播完（页面在后台之类）：照样移走
  setTimeout(end, delay + duration + 500);
}

/** 按下的反馈：点的那个按钮留在槽里，按下去、填满品牌色（批量条上点的不在行里，不用） */
function press(r: LeaveRow) {
  const btn =
    r.how === 'row' ? r.el.querySelector<HTMLElement>('.ib-acts button') : r.how === 'edit' ? r.el.querySelector<HTMLElement>('.ib-editacts .ib-triage') : null;
  if (!btn) return;
  r.el.classList.add('acts-on');
  btn.classList.add('ib-pressed');
}

const lineRect = (row: HTMLElement) => (row.querySelector<HTMLElement>('.ib-line') ?? row).getBoundingClientRect();
const rowTitle = (row: HTMLElement) => row.querySelector('.ib-t b')?.textContent?.trim() ?? '';
/** 行上编号的左边（手机上编号那一栏不显示：没有） */
const idLeft = (row: HTMLElement) => {
  const r = row.querySelector('.ib-line .ib-id')?.getBoundingClientRect();
  return r?.width ? r.left : undefined;
};

/** 「白板」入口：桌面在左侧活动栏，手机在底部标签栏 */
const boardTab = () => document.querySelector<HTMLElement>('.activitybar .ab-item[data-tab="board"]');

/** 从 from（行的那一条）缩成小卡片，沿弧线飞进「白板」入口；这一行在屏幕外、入口看不见时不飞 */
function fly(from: DOMRect, id: number, title: string, delay: number, idAt?: number) {
  if (from.bottom < 0 || from.top > window.innerHeight || !from.width) return false;
  const tab = boardTab();
  const to = (tab?.querySelector('svg') ?? tab)?.getBoundingClientRect();
  if (!to?.width) return false;
  const outer = document.createElement('div');
  outer.className = 'ib-fly';
  outer.setAttribute('aria-hidden', 'true');
  const chip = document.createElement('div');
  chip.className = 'ib-fly-chip';
  const no = document.createElement('b');
  no.textContent = `#${id}`;
  const t = document.createElement('span');
  t.textContent = title;
  chip.append(document.createElement('i'), no, t);
  outer.appendChild(chip);
  document.body.appendChild(outer);
  // 小卡片压在行的编号、标题上出现（行的其余部分同时淡掉），编号和行上的编号对齐，看起来是这一行缩成了它
  const w = chip.offsetWidth;
  const h = chip.offsetHeight;
  const x = idAt != null ? idAt - no.offsetLeft : from.left + Math.min(26, from.width / 6);
  const y = from.top + from.height / 2 - h / 2;
  outer.style.transform = `translate(${x}px, ${y}px)`;
  const dx = to.left + to.width / 2 - (x + w / 2);
  const dy = to.top + to.height / 2 - (y + h / 2);
  // 弧线：横向先慢后快；纵向一路升到比入口高一点、最后落进去。入口在下方（手机）时先往上抛一下再落
  const up = dy < 0;
  const peak = up ? dy - 18 : -28;
  const at = up ? 0.74 : 0.36;
  const scale = (p: number) => 1 - 0.6 * ((p - 0.18) / 0.72);
  outer.animate(
    [
      { transform: `translate(${x}px, ${y}px)` },
      { transform: `translate(${x}px, ${y}px)`, offset: 0.18, easing: 'cubic-bezier(.45, 0, .7, 1)' },
      { transform: `translate(${x + dx}px, ${y}px)`, offset: 0.9 },
      { transform: `translate(${x + dx}px, ${y}px)` },
    ],
    { duration: FLY, delay, fill: 'both' },
  );
  chip.animate(
    [
      { transform: 'translateY(0) scale(.9)', opacity: 0 },
      { transform: 'translateY(-3px) scale(1.05)', opacity: 1, offset: 0.1 },
      { transform: 'translateY(-3px) scale(1)', opacity: 1, offset: 0.18, easing: 'cubic-bezier(.3, .55, .5, 1)' },
      { transform: `translateY(${peak}px) scale(${scale(at)})`, opacity: 1, offset: at, easing: 'cubic-bezier(.5, 0, .9, .5)' },
      { transform: `translateY(${dy}px) scale(.4)`, opacity: 1, offset: 0.9 },
      { transform: `translateY(${dy}px) scale(.28)`, opacity: 0 },
    ],
    { duration: FLY, delay, fill: 'both' },
  ).onfinish = () => outer.remove();
  setTimeout(() => outer.remove(), delay + FLY + 400);
  return true;
}

/** 「白板」入口弹一下，旁边冒出 +n；接连整理时数字累加，不叠好几个 */
let plus: { el: HTMLElement; n: number } | null = null;
function bumpBoard(n: number) {
  if (reduceMotion()) return;
  const tab = boardTab();
  const icon = tab?.querySelector<SVGElement>('svg');
  const r = icon?.getBoundingClientRect();
  if (!tab || !icon || !r?.width) return;
  icon.animate([{ transform: 'scale(1)' }, { transform: 'scale(1.28)', offset: 0.32 }, { transform: 'scale(.94)', offset: 0.68 }, { transform: 'scale(1)' }], {
    duration: 520,
    easing: 'ease-out',
  });
  replay(tab, 'ab-got');
  if (plus?.el.isConnected) {
    plus.n += n;
    plus.el.textContent = `+${plus.n}`;
    replay(plus.el, 'go');
    return;
  }
  const el = document.createElement('span');
  el.className = 'ab-plus mono';
  el.setAttribute('aria-hidden', 'true');
  el.textContent = `+${n}`;
  el.style.left = `${r.right - 4}px`;
  el.style.top = `${r.top - 8}px`;
  document.body.appendChild(el);
  replay(el, 'go');
  const me = { el, n };
  plus = me;
  el.addEventListener('animationend', () => {
    el.remove();
    if (plus === me) plus = null;
  });
}
