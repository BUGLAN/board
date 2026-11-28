/* ==========================================================================
   白板上勾选卡片、批量打标签、批量移动、批量派送（PRD §10.3「批量打标签」）
   - 进入：侧栏「标签」标题旁的「批量」，或按住 ⌘ / Ctrl / Shift 点卡片
   - 选择里：点卡片（或聚焦后 Enter / 空格）勾选、再点取消；Shift 在同一个状态块里连选；
     块标题上的勾选框选上 / 取消这一块（有筛选时只算筛出的）
   - 底部批量条（components/BoardSelect）：全选（筛出的）、打标签、移动到、派送、取消；esc、换白板时退出
   - 打标签走 store/data 的 labelCards（一次 POST /api/labels/batch，「未整理」「稍后再做」的批量条也用 bulkLabel）；
     移动走 batchCards（一次 POST /api/cards/batch）：乐观更新，当帧生效；一张失败只撤那一张
   - 移动到：同一块白板的状态（和「未整理」），规则同拖动（lib/drag vetoFor）：锁定状态不能移入，被持有的跳过
   - 派送：按单卡同一套规则留下能派的（lib/hosts dispatchRoute），已有排队 / 在跑、锁定、被持有的跳过；
     其余一次 POST /api/dispatches/batch 派整批（服务端按会话分组），单张失败不影响别的
   ========================================================================== */
import { create } from 'zustand';
import { ApiError } from './api';
import { justDropped, vetoFor } from './drag';
import { dispatchRoute } from './hosts';
import { boardFlow, INBOX, INBOX_NAME } from './schema';
import { rangeIds, unionIds } from './selection';
import type { BoardSnapshot, Card, ColumnKey, Dispatch, ErrorCode, Status } from './types';
import { batchCards, batchMovePatch, labelCards, useData, type LabelBatchResult } from '../store/data';
import { overlayOpen, toast, toastError } from '../store/ui';

export { rangeIds } from './selection';

interface SelState {
  /** 在选择里（从「批量」进来、一张还没选也算） */
  on: boolean;
  ids: number[];
  /** Shift 连选的起点：上一次点的卡片 */
  anchor: number | null;
}

export const useBoardSel = create<SelState>(() => ({ on: false, ids: [], anchor: null }));
const set = useBoardSel.setState;

export const startSelect = () => set({ on: true });
export const clearSelect = () => set(s => (s.on || s.ids.length ? { on: false, ids: [], anchor: null } : s));
export const selectIds = (ids: number[]) => set(s => ({ on: true, ids: unionIds(s.ids, ids) }));

/** 点一张：选上或取消，并记成连选的起点 */
export function toggleCard(id: number) {
  set(s => ({ on: true, anchor: id, ids: s.ids.includes(id) ? s.ids.filter(x => x !== id) : [...s.ids, id] }));
}

export function selectRange(order: number[], id: number) {
  const r = rangeIds(order, useBoardSel.getState().anchor, id);
  if (!r) return toggleCard(id);
  set(s => ({ on: true, anchor: id, ids: unionIds(s.ids, r) }));
}

/** 块标题勾选框的样子：一张没选 / 选了一部分 / 全选上 */
export function blockState(ids: number[], picked: number[]): 'none' | 'some' | 'all' {
  const n = ids.filter(id => picked.includes(id)).length;
  return n === 0 ? 'none' : n === ids.length ? 'all' : 'some';
}

/** 点块标题的勾选框：全选上时全部取消，否则全部选上 */
export function toggleBlock(ids: number[]) {
  if (!ids.length) return;
  set(s => (blockState(ids, s.ids) === 'all' ? { ids: s.ids.filter(id => !ids.includes(id)) } : { on: true, ids: unionIds(s.ids, ids) }));
}

/** 批量加 / 去一个标签（白板、「未整理」、「稍后再做」的批量条共用）：一次请求（store/data labelCards）。
    失败只提示一次（第一个错误）；整个请求失败时所有卡片都算失败。勾选不动，由调用方决定 */
export async function bulkLabel(ids: number[], label: number, add: boolean): Promise<LabelBatchResult> {
  try {
    const r = await labelCards(ids, label, add);
    if (r.failed[0]) toastError(r.failed[0].error);
    return r;
  } catch (e) {
    toastError(e);
    return { changed: [], skipped: [], failed: ids.map(id => ({ id, error: e })) };
  }
}

export interface DispatchPlan {
  /** 能派发的卡片，按选中的顺序 */
  ids: number[];
  /** 不派发的卡片和原因 */
  skipped: { id: number; why: string }[];
}

/** 批量派送前按单卡同一套规则挑出能派的：锁定的、被持有的、已有排队或在跑的、未整理里的都跳过。
    active 是这块白板上已有排队 / 在跑的派发卡片，避免界面上先发一遍再等服务端报 ALREADY_DISPATCHED */
export function dispatchPlan(statuses: Status[], cards: Record<number, Card>, ids: number[], active: ReadonlySet<number>): DispatchPlan {
  const seen = new Set<number>();
  const plan: DispatchPlan = { ids: [], skipped: [] };
  for (const id of ids) {
    if (seen.has(id)) continue;
    seen.add(id);
    const card = cards[id];
    if (!card) {
      plan.skipped.push({ id, why: '卡片不存在' });
      continue;
    }
    if (active.has(id)) {
      plan.skipped.push({ id, why: '已有排队或在跑的派发' });
      continue;
    }
    if (card.column === INBOX) {
      plan.skipped.push({ id, why: '未整理不在批量派送范围' });
      continue;
    }
    const status = statuses.find(s => s.key === card.column);
    if (!status) {
      plan.skipped.push({ id, why: '卡片所在状态不存在' });
      continue;
    }
    if (card.holder) {
      plan.skipped.push({ id, why: `被 ${card.holder} 持有` });
      continue;
    }
    if (status.lock) {
      plan.skipped.push({ id, why: `在锁定状态「${status.name}」` });
      continue;
    }
    if (!dispatchRoute(statuses, card.column, card.holder)) {
      plan.skipped.push({ id, why: `在「${status.name}」不能派发` });
      continue;
    }
    plan.ids.push(id);
  }
  return plan;
}

/** 批量派发接口（api.dispatchBatch）的返回换成收尾提示要的样子：失败的带上服务端的错误码和原因 */
export function bulkDispatchResult(r: { ok: Dispatch[]; failed: { id: number; code: ErrorCode; message: string }[] }): {
  ok: Dispatch[];
  failed: { id: number; error: unknown }[];
} {
  return { ok: r.ok, failed: r.failed.map(f => ({ id: f.id, error: new ApiError(f.code, f.message) })) };
}

/** 批量派送收尾的提示：成功、跳过、失败各几张，正文写去向和每张跳过、失败的原因。
    按机器 + agent 派（hosts/Dispatch）和按角色派（profiles/DispatchFlow）共用 */
export function bulkDispatchToast(
  result: { ok: Dispatch[]; failed: { id: number; error: unknown }[] },
  skipped: { id: number; why: string }[],
  where: string,
): { type: 'success' | 'error'; title: string; text?: string } {
  const lines = [
    where,
    skipped.length ? `跳过：${skipped.map(x => `#${x.id} ${x.why}`).join('；')}` : '',
    result.failed.length ? `失败：${result.failed.map(x => `#${x.id} ${(x.error as Error)?.message || '派发失败'}`).join('；')}` : '',
  ].filter(Boolean);
  return {
    type: result.failed.length ? 'error' : 'success',
    title: `成功 ${result.ok.length} 张 · 跳过 ${skipped.length} 张 · 失败 ${result.failed.length} 张`,
    text: lines.join('\n') || undefined,
  };
}

/** 「移动到」菜单的一行：能移过去的卡片（按白板上的顺序）、跳过的，和整行不能选的原因 */
export interface MoveTarget {
  key: ColumnKey;
  name: string;
  ids: number[];
  /** 已经在这个状态里的 */
  here: number;
  /** 被 agent 持有、只有持有者能移动的 */
  held: number;
  /** 整个状态不能移入（锁定状态只能由 agent 领取进入）：写在行上 */
  locked: string | null;
}

export function moveTargets(board: BoardSnapshot, ids: number[]): MoveTarget[] {
  const f = boardFlow(board);
  const keys = [...f.list.map(s => s.key), INBOX];
  const pos = new Map(keys.flatMap(k => board.cols[k] ?? []).map((id, i) => [id, i]));
  const cards = ids
    .map(id => board.cards[id])
    .filter((c): c is Card => !!c)
    .sort((a, b) => (pos.get(a.id) ?? 1e9) - (pos.get(b.id) ?? 1e9));
  return keys.map(key => {
    const rest = cards.filter(c => c.column !== key);
    // 锁定只看目标状态，和是哪张卡片无关：拿一张没人持有的卡片问 vetoFor
    const lock = key !== INBOX ? vetoFor({ ...(rest[0] ?? cards[0]), holder: null }, key, f) : null;
    return {
      key,
      name: key === INBOX ? INBOX_NAME : f.name(key),
      ids: lock ? [] : rest.filter(c => !c.holder).map(c => c.id),
      here: cards.length - rest.length,
      held: rest.filter(c => c.holder).length,
      locked: lock?.title ?? null,
    };
  });
}

/** 批量移动：按白板上的顺序一张张排到目标状态末尾（和「移动到」一张一样），失败的提示第一个错误 */
export async function bulkMove(t: MoveTarget) {
  if (!t.ids.length) return 0;
  const path = useData.getState().path;
  let result;
  try {
    result = await batchCards(path, t.ids, { op: 'move', column: t.key }, batchMovePatch(t.ids, t.key));
  } catch (e) {
    toastError(e);
    return 0;
  }
  if (result.failed[0]) toastError(result.failed[0].error);
  const ok = result.ok.length;
  const skip = [t.here && `${t.here} 张本来就在这里`, t.held && `${t.held} 张被 agent 持有`].filter(Boolean).join('，');
  if (ok) toast({ type: 'success', title: `${ok} 张移到了「${t.name}」`, text: skip ? `跳过：${skip}` : undefined });
  return ok;
}

const cardOf = (t: EventTarget | null) => (t as HTMLElement | null)?.closest?.<HTMLElement>('[data-drop] .card') ?? null;
/** 同一块里看到的顺序（跳过会话组头这类装饰） */
const listOrder = (card: HTMLElement) =>
  [...(card.parentElement?.querySelectorAll<HTMLElement>(':scope > .card') ?? [])].map(n => Number(n.dataset.id)).filter(n => n > 0);

function pick(card: HTMLElement, shift: boolean) {
  const id = Number(card.dataset.id);
  if (!(id > 0)) return false; // 还在新建的临时卡片
  if (shift) selectRange(listOrder(card), id);
  else toggleCard(id);
  return true;
}

/** 挂在 window 的捕获阶段：比 lib/drag 的「点卡片打开详情」「Enter 打开」先拿到 */
export function installBoardSelect() {
  const onClick = (e: MouseEvent) => {
    const card = cardOf(e.target);
    if (!card || justDropped() || (e.target as HTMLElement).closest('a, button')) return;
    if (!useBoardSel.getState().on && !(e.metaKey || e.ctrlKey || e.shiftKey)) return;
    if (!pick(card, e.shiftKey)) return;
    e.preventDefault();
    e.stopPropagation();
  };
  const onKey = (e: KeyboardEvent) => {
    if (!useBoardSel.getState().on || overlayOpen() || useData.getState().detailId != null) return;
    const card = cardOf(e.target);
    if (card && e.target === card && (e.key === 'Enter' || e.key === ' ') && pick(card, e.shiftKey)) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  window.addEventListener('click', onClick, true);
  window.addEventListener('keydown', onKey, true);
  return () => {
    window.removeEventListener('click', onClick, true);
    window.removeEventListener('keydown', onKey, true);
  };
}
