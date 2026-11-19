/* ==========================================================================
   白板上勾选卡片、批量打标签（PRD §10.3「批量打标签」）
   - 进入：侧栏「标签」标题旁的「批量」，或按住 ⌘ / Ctrl / Shift 点卡片
   - 选择里：点卡片（或聚焦后 Enter / 空格）勾选、再点取消；Shift 在同一个状态块里连选；
     块标题上的勾选框选上 / 取消这一块（有筛选时只算筛出的）
   - 底部批量条（components/BoardSelect）：全选（筛出的）、打标签、取消；esc、换白板时退出
   - 写入走 store/data 的 addLabel / removeLabel：乐观更新，卡片上的标签当帧出现、画成 pending；一张失败只撤那一张
   ========================================================================== */
import { create } from 'zustand';
import { justDropped } from './drag';
import { addLabel, removeLabel, useData } from '../store/data';
import { overlayOpen, toastError } from '../store/ui';

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
export const selectIds = (ids: number[]) => set(s => ({ on: true, ids: union(s.ids, ids) }));

const union = (a: number[], b: number[]) => [...new Set([...a, ...b])];

/** 点一张：选上或取消，并记成连选的起点 */
export function toggleCard(id: number) {
  set(s => ({ on: true, anchor: id, ids: s.ids.includes(id) ? s.ids.filter(x => x !== id) : [...s.ids, id] }));
}

/** 连选：order 是同一块里看到的顺序；起点不在这一块时返回 null（只切换点的这一张） */
export function rangeIds(order: number[], anchor: number | null, id: number): number[] | null {
  const a = anchor == null ? -1 : order.indexOf(anchor);
  const b = order.indexOf(id);
  if (a < 0 || b < 0) return null;
  return order.slice(Math.min(a, b), Math.max(a, b) + 1);
}

export function selectRange(order: number[], id: number) {
  const r = rangeIds(order, useBoardSel.getState().anchor, id);
  if (!r) return toggleCard(id);
  set(s => ({ on: true, anchor: id, ids: union(s.ids, r) }));
}

/** 块标题勾选框的样子：一张没选 / 选了一部分 / 全选上 */
export function blockState(ids: number[], picked: number[]): 'none' | 'some' | 'all' {
  const n = ids.filter(id => picked.includes(id)).length;
  return n === 0 ? 'none' : n === ids.length ? 'all' : 'some';
}

/** 点块标题的勾选框：全选上时全部取消，否则全部选上 */
export function toggleBlock(ids: number[]) {
  if (!ids.length) return;
  set(s => (blockState(ids, s.ids) === 'all' ? { ids: s.ids.filter(id => !ids.includes(id)) } : { on: true, ids: union(s.ids, ids) }));
}

/** 批量加 / 去一个标签：返回真的改了几张（本来就有 / 本来就没有的不算）；失败的提示第一个错误 */
export async function bulkLabel(ids: number[], label: number, add: boolean) {
  const results = await Promise.allSettled(ids.map(id => (add ? addLabel(id, label) : removeLabel(id, label))));
  const bad = results.find(r => r.status === 'rejected') as PromiseRejectedResult | undefined;
  if (bad) toastError(bad.reason);
  return results.filter(r => r.status === 'fulfilled' && r.value).length;
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
    if (e.key === 'Escape') {
      e.stopPropagation();
      clearSelect();
      return;
    }
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
