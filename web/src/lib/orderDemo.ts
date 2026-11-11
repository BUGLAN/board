/* ==========================================================================
   白板设置右栏「预览」里的拖动演示（components/BoardSettings.tsx）：用示例卡片演示「保留顺序」开着 / 关着时拖动的结果
   - 拖到别的状态：按住一张卡片拖进这一块中间，松手先落在落点；开着时再滑到按排序依据该在的位置，关着时停在那里
   - 同一状态里拖动：把最后一张拖到最前面，开着时滑回原位（顺序不变），关着时停在那里
   演示画在 stage 里（innerHTML，不归 React 管：动画期间 React 重新渲染不会和移动过的节点打架），幽灵挂在 body 上
   ========================================================================== */
import { sortIds } from './sort';
import { statusColor, viewOf } from './schema';
import type { Card, Label, Order, Status, StatusView } from './types';

export type Scene = 'cross' | 'same';

export interface DemoInput {
  target: Status;
  /** 拖出来的状态（拖到别的状态时）；没有时只演示同一状态里拖动 */
  source: Status | null;
  order: Order;
  keep: boolean;
  labels: Label[];
}

const TITLES = ['整理发版说明', '修复登录后跳转', '补充接口文档', '升级依赖版本', '清理旧配置', '优化列表加载', '修正时区显示', '补测试用例', '调整导出格式'];
const MIN = 60_000;

const esc = (s: string) => s.replace(/[&<>"']/g, c => `&#${c.charCodeAt(0)};`);

/** 示例卡片：编号、移入时间、更新时间、耗时、标签故意错开，换哪种排序依据都看得出差别 */
function sample(input: DemoInput, now: number) {
  const tileView = viewOf(input.target) === 'tile';
  const tIds = tileView ? [14, 9, 21, 17, 11, 25] : [14, 9, 21];
  const sIds = [8, 23, 12];
  const lab = input.labels.slice(0, 3).map(l => l.id);
  const cards: Record<string, Card> = {};
  const mk = (id: number, i: number, col: string, moved: number, updated: number, work: number, labels: number[]): Card => ({
    id, title: TITLES[(id * 7) % TITLES.length], body: '', column: col, holder: null, claimedAt: null, beatAt: null, runStartedAt: null,
    workMs: work * MIN, runs: work ? 1 : 0, runsLost: 0, claimedFrom: null, labels, createdAt: now - (300 - id) * MIN,
    updatedAt: now - updated * MIN, movedAt: now - moved * MIN, createdBy: { kind: 'human', name: '' }, comments: i % 3,
  });
  const moved = [50, 20, 35, 10, 45, 5];
  const updated = [8, 30, 15, 40, 3, 25];
  const work = [12, 40, 5, 25, 60, 18];
  tIds.forEach((id, i) => (cards[id] = mk(id, i, input.target.key, moved[i], updated[i], work[i], lab.length ? [lab[i % lab.length]].filter((_, j) => (i + j) % 3 !== 2) : [])));
  sIds.forEach((id, i) => (cards[id] = mk(id, i, input.source?.key ?? '', 60 + i * 10, 2 + i * 20, 30 - i * 9, lab.length ? [lab[(i + 1) % lab.length]] : [])));
  return { cards, tIds, sIds };
}

function cardHTML(c: Card, view: StatusView) {
  const id = `data-id="${c.id}"`;
  if (view === 'tile') return `<div class="bsp-card v-tile" ${id}>#${c.id}</div>`;
  return `<div class="bsp-card v-${view}" ${id}><span class="cid">#${c.id}</span><span class="ct">${esc(c.title)}</span></div>`;
}

function blockHTML(s: Status, ids: number[], cards: Record<string, Card>, input: DemoInput) {
  const o = input.order;
  const chip = input.keep && s.key === input.target.key
    ? `<span class="col-order is-mini"><span class="ts">${{ moved: '移入', id: '编号', updated: '更新', labels: '标签', work: '耗时' }[o.by]}</span>${o.by === 'labels' ? '' : `<i>${o.desc ? '↓' : '↑'}</i>`}</span>`
    : '';
  return `<section class="col bsp-col" data-key="${esc(s.key)}" style="--c:${statusColor(s.color)}">
    <header class="col-head"><span class="col-dot"></span><span class="col-name">${esc(s.name)}</span><span class="bsp-sp"></span>${chip}</header>
    <div class="bsp-list v-${viewOf(s)}">${ids.map(id => cardHTML(cards[id], viewOf(s))).join('')}</div>
  </section>`;
}

const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
const easeInOut = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

function flip(lists: HTMLElement[], mutate: () => void, ms = 360, spring = false) {
  const before = new Map(lists.flatMap(l => [...l.children]).map(e => [e, e.getBoundingClientRect()]));
  mutate();
  for (const e of lists.flatMap(l => [...l.children])) {
    const b = before.get(e);
    if (!b) continue;
    const a = e.getBoundingClientRect();
    const dx = b.left - a.left;
    const dy = b.top - a.top;
    if (Math.abs(dx) > 0.5 || Math.abs(dy) > 0.5)
      e.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], { duration: ms, easing: spring ? 'cubic-bezier(.22,1.2,.36,1)' : 'cubic-bezier(.16,1,.3,1)' });
  }
}

/** 这一块能演示哪些：锁定状态里的卡片只能由 agent 领取进入、被持有期间人拖不动，两种都不演示 */
export function scenesOf(input: Pick<DemoInput, 'target' | 'source'>): Scene[] {
  if (input.target.lock) return [];
  return input.source ? ['cross', 'same'] : ['same'];
}

/** 演示的三步说明 */
export function stepsOf(scene: Scene, input: DemoInput, names: { order: string; dir: string }): string[] {
  const t = input.target.name;
  const first = scene === 'cross' ? `按住一张卡片，从「${input.source?.name ?? ''}」拖进「${t}」中间` : `在「${t}」里把最后一张拖到最前面`;
  if (!input.keep) return [first, '松手：卡片落在松手的位置', '就停在那里，顺序按拖动的结果保存'];
  return [
    first,
    '松手：卡片先落在松手的位置',
    scene === 'cross' ? `滑到按「${names.order}」该在的位置（${names.dir}）` : `滑回按「${names.order}」该在的位置，顺序不变`,
  ];
}

/** 放一遍演示；alive() 变成 false 时停下（换了状态、改了设置、关了弹窗）。onStep：进行到第几步（0、1、2） */
export async function playDemo(stage: HTMLElement, scene: Scene, input: DemoInput, alive: () => boolean, onStep: (i: number) => void, motion: boolean) {
  const now = Date.now();
  const { cards, tIds, sIds } = sample(input, now);
  const sort = (ids: number[], over: Record<string, Card> = {}) =>
    input.keep ? sortIds(ids, { ...cards, ...over }, input.order, input.labels, now) : ids;
  const shownT = sort(tIds);
  const shownS = sIds;
  const cross = scene === 'cross' && !!input.source;
  const tView = viewOf(input.target);
  const sView = input.source ? viewOf(input.source) : tView;
  // 拖哪张、放到哪：落点故意和最终位置不同，才看得出「滑到该在的位置」
  const id = cross ? shownS[1] : shownT[shownT.length - 1];
  const moved: Record<string, Card> = cross ? { [id]: { ...cards[id], movedAt: now + 1000, column: input.target.key } } : {};
  let drop = 0;
  if (cross) {
    const fin = sort([...shownT, id], moved).indexOf(id);
    drop = Math.floor(shownT.length / 2);
    if (input.keep && drop === fin) drop = fin === 0 ? shownT.length : 0;
  }
  stage.innerHTML = `<div class="bsp-board ${cross ? '' : 'one'}">${cross && input.source ? blockHTML(input.source, shownS, cards, input) : ''}${blockHTML(input.target, shownT, cards, input)}</div>`;
  const toCol = stage.querySelector<HTMLElement>(`.bsp-col[data-key="${CSS.escape(input.target.key)}"]`)!;
  const toList = toCol.querySelector<HTMLElement>('.bsp-list')!;
  const fromList = cross ? stage.querySelector<HTMLElement>('.bsp-col .bsp-list')! : toList;
  const card = fromList.querySelector<HTMLElement>(`[data-id="${id}"]`);
  if (!card) return;
  onStep(0);
  const placeholder = document.createElement('div');
  placeholder.innerHTML = cardHTML(cards[id], tView);
  const ph = placeholder.firstElementChild as HTMLElement;
  ph.classList.add('ph');
  const finalOrder = () => {
    const ids = [...toList.children].map(e => Number((e as HTMLElement).dataset.id));
    return sort(ids, moved);
  };
  if (!motion) {
    // 减弱动态效果：直接画出结果
    card.remove();
    const real = ph.cloneNode(true) as HTMLElement;
    real.classList.remove('ph');
    real.classList.add('hl');
    toList.insertBefore(real, toList.children[drop] ?? null);
    if (input.keep) for (const x of finalOrder()) toList.append(toList.querySelector(`[data-id="${x}"]`)!);
    onStep(2);
    return;
  }
  await sleep(500);
  if (!alive()) return;
  const r0 = card.getBoundingClientRect();
  const ghost = document.createElement('div');
  ghost.className = cx('bsp-ghost', sView === 'tile' && 'tile');
  ghost.innerHTML = `<div class="g-a">${cardHTML(cards[id], sView)}</div><div class="g-b">${cardHTML(cards[id], tView)}</div>${CURSOR}`;
  const put = (r: { left: number; top: number; width: number; height: number }) =>
    Object.assign(ghost.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });
  put(r0);
  document.body.append(ghost);
  try {
    await sleep(380);
    if (!alive()) return;
    // 按住：卡片浮起来，原位置空出
    ghost.style.transform = 'scale(1.04) rotate(-1.5deg)';
    card.style.visibility = 'hidden';
    await sleep(260);
    if (!alive()) return;
    // 拖出：原块收拢，目标块在落点让出占位
    toCol.classList.add('drop-ok');
    flip([fromList, toList], () => {
      card.remove();
      toList.insertBefore(ph, toList.children[drop] ?? null);
    });
    if (tView === 'tile') ghost.classList.add('tile');
    const ga = ghost.querySelector<HTMLElement>('.g-a')!;
    const gb = ghost.querySelector<HTMLElement>('.g-b')!;
    const morph = tView !== sView;
    await new Promise<void>(done => {
      const t0 = performance.now();
      const step = (t: number) => {
        if (!alive()) return done();
        const p = Math.min(1, (t - t0) / 900);
        const k = easeInOut(p);
        const r1 = ph.getBoundingClientRect();
        put({ left: r0.left + (r1.left - r0.left) * k, top: r0.top + (r1.top - r0.top) * k, width: r0.width + (r1.width - r0.width) * k, height: r0.height + (r1.height - r0.height) * k });
        if (morph) {
          ga.style.opacity = String(1 - Math.min(1, Math.max(0, (p - 0.2) / 0.4)));
          gb.style.opacity = String(Math.min(1, Math.max(0, (p - 0.45) / 0.4)));
        }
        if (p < 1) requestAnimationFrame(step);
        else done();
      };
      requestAnimationFrame(step);
    });
    if (!alive()) return;
    // 松手：落在松手的位置
    ghost.style.transform = 'none';
    await sleep(140);
    if (!alive()) return;
    onStep(1);
    ph.classList.remove('ph');
    ph.classList.add('hl');
    ghost.remove();
    toCol.classList.remove('drop-ok');
    await sleep(input.keep ? 750 : 500);
    if (!alive()) return;
    onStep(2);
    if (input.keep) flip([toList], () => finalOrder().forEach(x => toList.append(toList.querySelector(`[data-id="${x}"]`)!)), 680, true);
    await sleep(1600);
  } finally {
    ghost.remove();
  }
}

const cx = (...a: (string | false)[]) => a.filter(Boolean).join(' ');

const CURSOR =
  '<svg class="bsp-cursor" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 2.5 19.5 11l-6.4 1.7L9.6 19z" fill="#fff" stroke="#111" stroke-width="1.5" stroke-linejoin="round"/></svg>';
