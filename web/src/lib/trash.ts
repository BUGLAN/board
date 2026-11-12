/* ==========================================================================
   回收站（PRD §8 规则 10、§10.3）：删除的卡片移进回收站，30 天内可以恢复
   - 删除、恢复、永久删除都走 store/data 的 exec：补丁当帧生效，失败时撤回并提示
   - 删除后弹出的提示里可以「撤销」（= 从回收站恢复）
   - 回收站里的内容按需取（GET /api/trash），取回来按白板路径和 rev 缓存一份：
     rev 变了（任何写入都会）就重新取；「撤销」要知道卡片原来在哪一列、排第几位
   ========================================================================== */
import { api } from './api';
import type { Card, Status, TrashCard } from './types';
import { exec, removeCard, restoreCard, serverNow, useData } from '../store/data';
import { toast, toastError } from '../store/ui';

/* ------------------------------ 取回收站 ------------------------------ */

const cache = new Map<string, { rev: number; list: Promise<TrashCard[]> }>();

/** 这块白板回收站里的卡片（按删除时间，最近删的在最后）。rev 变了就重新取 */
export function trashOf(path: string): Promise<TrashCard[]> {
  const s = useData.getState();
  const rev = s.board?.path === path ? s.board.rev : s.rev;
  const hit = cache.get(path);
  if (hit && hit.rev === rev) return hit.list;
  const list = api.trash(path).catch(e => {
    if (cache.get(path)?.list === list) cache.delete(path);
    throw e;
  });
  cache.set(path, { rev, list });
  return list;
}

/** 回收站里的这张卡片（删除时要知道它原来在哪一列、排第几位）。取不到时返回 null */
export async function trashed(path: string, id: number): Promise<TrashCard | null> {
  try {
    return (await trashOf(path)).find(t => t.id === id) ?? null;
  } catch {
    return null;
  }
}

/* ------------------------------ 删除 ------------------------------ */

/** 删掉几张卡片（都移进回收站）。全成功时提示「已删除 … 撤销」；有被拒绝的提示原因。返回是不是全都删掉了 */
export async function deleteCards(ids: number[], { silent = false, titles = {} }: { silent?: boolean; titles?: Record<number, string> } = {}): Promise<boolean> {
  const path = useData.getState().path;
  const ok = ids.filter(id => id > 0);
  if (!ok.length) return false;
  const board = useData.getState().board;
  // 删除之前先取回收站的编号：撤销时要知道每张卡片原来在哪一列、排第几位（取不到时按放回最下面算）
  await Promise.all(ok.map(id => trashed(path, id)));
  const results = await Promise.allSettled(
    ok.map(id =>
      exec({ op: 'delete', id }, removeCard(id)).then((r): { id: number; title: string } => ({
        id,
        title: titles[id] ?? board?.cards[id]?.title ?? r.title,
      })),
    ),
  );
  const done = results.flatMap(r => (r.status === 'fulfilled' ? [r.value] : []));
  const bad = results.find(r => r.status === 'rejected') as PromiseRejectedResult | undefined;
  if (bad) toastError(bad.reason);
  if (!done.length) return false;
  if (!silent) {
    toast({
      type: 'success',
      title: done.length === 1 ? `已删除 #${done[0].id}` : `已删除 ${done.length} 张卡片`,
      text: done.length === 1 ? `「${done[0].title}」在回收站里，30 天内可以恢复` : `${done.map(d => `#${d.id}`).join('、')} 在回收站里，30 天内可以恢复`,
      action: { label: '撤销', fn: () => void undoDelete(path, done.map(d => d.id)) },
      duration: 6000,
    });
  }
  return !bad;
}

/* ------------------------------ 恢复 ------------------------------ */

/**
 * 恢复后的列，和服务端的规则一致：
 * - 「未整理」不在工作流里，从这儿删的条目恢复后还是回「未整理」
 * - 原来在锁定状态（被删时还在做）：回到当初领取它的可领取状态（{@link TrashCard.heldFrom}），那个状态没了才回首个可领取状态
 * - 原来那一列还在就用它；已经删了的回首个可领取状态
 */
export function restoreColumn(t: Pick<TrashCard, 'col' | 'heldFrom'> | null | undefined, statuses: Status[] | undefined): string {
  const home = statuses?.find(s => s.claim)?.key ?? 'todo';
  const saved = t?.col;
  if (!saved) return home;
  if (saved === 'inbox') return saved;
  const s = statuses?.find(x => x.key === saved);
  if (!s) return home;
  if (!s.lock) return saved;
  const from = t?.heldFrom;
  return from && statuses?.some(x => x.key === from && x.claim) ? from : home;
}

/** 恢复后排第几位：还是原来那一列就放回原来的位置，否则放最后 */
export function restorePos(t: TrashCard | null, column: string, list: number[] | undefined): number {
  const n = list?.length ?? 0;
  if (!t || t.column !== column) return n;
  return Math.min(Math.max(0, t.pos), n);
}

/** 恢复几张卡片：先取回收站里的原位，当帧放回白板上，再让服务端恢复。返回成功的张数 */
export async function restoreMany(path: string, ids: number[]): Promise<number> {
  const results = await Promise.allSettled(ids.map(id => restoreOne(path, id)));
  const bad = results.find(r => r.status === 'rejected') as PromiseRejectedResult | undefined;
  if (bad) toastError(bad.reason);
  return results.filter(r => r.status === 'fulfilled').length;
}

async function restoreOne(path: string, id: number) {
  const t = await trashed(path, id);
  const w = useData.getState();
  const live = w.path === path && w.board?.path === path ? w.board : null;
  const column = restoreColumn(t, live?.statuses);
  const pos = restorePos(t, column, live?.cols[column]);
  // 补丁里放上去的那一份：回收站里有就按它的（标题、标签、附件都还在），没有就等刷新
  const card = (t as unknown as Card | null) ?? ({ id, title: '', body: '', column } as Card);
  await exec({ op: 'restore', id }, restoreCard({ id, column, pos, movedAt: serverNow(), card }));
}

/** 撤销删除：全成功时一句「已恢复」 */
export async function undoDelete(path: string, ids: number[]): Promise<void> {
  const n = await restoreMany(path, ids);
  if (n && n === ids.length) toast({ type: 'success', title: ids.length === 1 ? `已恢复 #${ids[0]}` : `已恢复 ${n} 张卡片`, duration: 2200 });
}

/** 回收站里恢复之后：一句话归位（原来在锁定状态的，回到了当初领取它的可领取状态） */
export function restoredToast(ids: number[], path: string): void {
  const s = useData.getState();
  const b = s.board?.path === path ? s.board : null;
  const where = ids.map(id => b?.cards[id]).filter(Boolean) as Card[];
  const columns = [...new Set(where.map(c => c.column))];
  const name = (key: string) => (key === 'inbox' ? '未整理' : b?.statuses.find(x => x.key === key)?.name ?? key);
  const at = where.length && columns.length === 1 ? `，回到「${name(columns[0])}」` : '';
  toast({ type: 'success', title: ids.length === 1 ? `已恢复 #${ids[0]}` : `已恢复 ${ids.length} 张卡片`, text: `和卡片一样可以再删除${at}`, duration: 2600 });
}

/* ------------------------------ 永久删除 ------------------------------ */

/** 从回收站永久删除：附件、评论一起删，不能恢复。全成功时提示。返回是不是全都删掉了 */
export async function purgeCards(ids: number[]): Promise<boolean> {
  const results = await Promise.allSettled(ids.map(id => exec({ op: 'purge', id })));
  const bad = results.find(r => r.status === 'rejected') as PromiseRejectedResult | undefined;
  const n = results.filter(r => r.status === 'fulfilled').length;
  if (bad) toastError(bad.reason);
  if (n) toast({ type: 'success', title: n === 1 ? `已永久删除 #${ids[0]}` : `已永久删除 ${n} 张卡片`, duration: 2400 });
  return !bad;
}
