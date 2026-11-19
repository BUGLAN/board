/* ==========================================================================
   回收站（PRD §8 规则 10、§10.5）：「未整理」页标题的第三个标签（未整理 | 稍后再做 | 回收站，default/main#486）
   - 列出这块白板删掉的卡片（最近删的在前）：编号、标题、描述第一行、原来在哪个状态、谁删的、什么时候删的
   - 每张可以「恢复」或「永久删除」，勾选几条可以一起操作；侧栏的筛选也管这一页
   - 删掉的卡片一直留着，服务端不自动清理：只有人在这里点「永久删除」才真的删掉（附件一起删）
   ========================================================================== */
import { useEffect, useMemo, useState } from 'react';
import { statusColor } from '../lib/schema';
import { purgeCards, restoreMany, restoredToast, trashOf } from '../lib/trash';
import type { Status, TrashCard } from '../lib/types';
import { actorName, ago, cx } from '../lib/util';
import { Tip, useNow } from './bits';
import { useData } from '../store/data';
import { confirm, overlayOpen, useUI } from '../store/ui';
import { Icon } from './Icon';
import { Skeleton } from './Skeleton';

export function TrashPage({ path }: { path: string }) {
  const board = useData(s => (s.board?.path === path ? s.board : null));
  // 和卡片上的相对时间一样，按服务端时间算，每分钟刷新
  const now = useNow();
  const statuses = board?.statuses;
  const { list, error, reload } = useTrash(path);
  const filter = useUI(s => s.filter);
  const q = filter.trim().toLowerCase();
  const [checked, setChecked] = useState<Set<number>>(() => new Set());
  const [busy, setBusy] = useState(false);

  // 换白板：勾选清空
  useEffect(() => setChecked(new Set()), [path]);

  // 最近删的在前（服务端按删除时间给，最近删的在最后）
  const all = useMemo(() => [...(list ?? [])].reverse().filter(t => trashHit(t, q)), [list, q]);
  const ids = useMemo(() => all.map(t => t.id), [all]);
  const chosen = ids.filter(id => checked.has(id));
  const allOn = ids.length > 0 && chosen.length === ids.length;
  const drop = (targets: number[]) =>
    setChecked(prev => {
      const next = new Set(prev);
      targets.forEach(id => next.delete(id));
      return next;
    });

  // esc 取消勾选，⌘A / Ctrl+A 全选（和「未整理」「稍后再做」一样）
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest?.('input, textarea, select, [contenteditable]') || overlayOpen() || useData.getState().detailId != null) return;
      if (e.key === 'Escape' && checked.size) {
        e.preventDefault();
        setChecked(new Set());
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a' && ids.length) {
        e.preventDefault();
        setChecked(new Set(ids));
      }
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [checked, ids]);

  const restore = async (targets: number[]) => {
    if (!targets.length || busy) return;
    setBusy(true);
    try {
      const n = await restoreMany(path, targets);
      if (n) restoredToast(targets.slice(0, n), path);
      drop(targets);
    } finally {
      setBusy(false);
    }
  };

  const purge = async (targets: number[]) => {
    if (!targets.length || busy) return;
    const one = targets.length === 1 ? all.find(t => t.id === targets[0]) : null;
    const yes = await confirm(
      targets.length === 1
        ? {
            title: `永久删除 #${targets[0]}？`,
            text: `「${one?.title ?? ''}」及其全部动态、评论、附件会被永久删除，无法恢复。想留着的话用「恢复」。`,
            ok: '永久删除',
          }
        : {
            title: `永久删除 ${targets.length} 张？`,
            text: `${targets.map(id => `#${id}`).join('、')} 及其全部动态、评论、附件会被永久删除，无法恢复。`,
            ok: `永久删除 ${targets.length} 张`,
          },
    );
    if (!yes) return;
    setBusy(true);
    try {
      await purgeCards(targets);
      drop(targets);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="trash-page">
      <p className="ib-later-sub">删除的卡片放在这里，一直留着、不会自动清理：可以恢复到原来的状态；在这里点「永久删除」才会连同附件一起删掉。</p>

      {all.length > 0 && (
        <div className="trash-bar">
          <label className="trash-check">
            <input
              type="checkbox"
              checked={allOn}
              ref={el => {
                if (el) el.indeterminate = chosen.length > 0 && !allOn;
              }}
              onChange={() => setChecked(allOn ? new Set() : new Set(ids))}
            />
            <span>{allOn ? '全不选' : '全选'}</span>
          </label>
          <span className="spacer" />
          {chosen.length > 0 ? (
            <>
              <span className="trash-sel">已选 {chosen.length} 张</span>
              <button className="btn sm" type="button" disabled={busy} onClick={() => void restore(chosen)}>
                <Icon name="replay" />
                恢复
              </button>
              <button className="btn sm danger" type="button" disabled={busy} onClick={() => void purge(chosen)}>
                <Icon name="trash" />
                永久删除
              </button>
            </>
          ) : (
            <span className="trash-hint">勾选可以一起恢复、永久删除</span>
          )}
        </div>
      )}

      {error ? (
        <div className="trash-empty">
          <b>回收站没取回来</b>
          <span>{error}</span>
          <button className="btn sm" type="button" onClick={reload}>
            重试
          </button>
        </div>
      ) : !list ? (
        <div className="trash-list" aria-busy="true">
          {[0, 1, 2].map(i => (
            <div className="trash-row sk" key={i}>
              <Skeleton w={16} h={16} r={4} />
              <Skeleton w={`${54 - i * 9}%`} h={11} />
            </div>
          ))}
        </div>
      ) : list.length === 0 ? (
        <div className="trash-empty">
          <Icon name="trash" />
          <b>回收站是空的</b>
          <span>删除的卡片会放在这里，随时可以恢复；在这里点了「永久删除」的，附件一起删掉，不能再恢复。</span>
        </div>
      ) : all.length === 0 ? (
        <div className="trash-empty">
          <b>没有符合的卡片</b>
          <span>换个词试试，或者清除筛选。</span>
        </div>
      ) : (
        <div className="trash-list" role="list">
          {all.map(t => (
            <div className={cx('trash-row', checked.has(t.id) && 'on')} key={t.id} role="listitem">
              <input type="checkbox" aria-label={`勾选 #${t.id}`} checked={checked.has(t.id)} onChange={() => setChecked(prev => new Set(prev.has(t.id) ? [...prev].filter(x => x !== t.id) : [...prev, t.id]))} />
              <span className="trash-id mono">#{t.id}</span>
              <span className="trash-main">
                <span className="trash-name">{t.title}</span>
                {firstLine(t.body) && <span className="trash-desc">{firstLine(t.body)}</span>}
              </span>
              <span className="trash-meta">
                <span className="trash-col">
                  <i style={{ background: statusColor(t.column === 'inbox' ? 'inbox' : colorOf(t.column, statuses)) }} />
                  {t.column === 'inbox' ? '未整理' : nameOf(t.column, statuses)}
                </span>
                <span>
                  {actorName(t.deletedBy)} 删除 · {ago(t.deletedAt, now)}
                </span>
              </span>
              <span className="trash-acts">
                <Tip text="恢复到原来状态">
                  <button className="btn sm" type="button" disabled={busy} onClick={() => void restore([t.id])}>
                    <Icon name="replay" />
                    恢复
                  </button>
                </Tip>
                <Tip text="永久删除">
                  <button className="icon-btn sm danger" type="button" aria-label={`永久删除 #${t.id}`} disabled={busy} onClick={() => void purge([t.id])}>
                    <Icon name="trash" />
                  </button>
                </Tip>
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/** 回收站里的卡片：进这一页时取一次，随 rev（任何写入）重新取；重新取的时候保留上一份，不闪空白 */
function useTrash(path: string) {
  const rev = useData(s => (s.board?.path === path ? s.board.rev : s.rev));
  const [list, setList] = useState<TrashCard[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let alive = true;
    setError(null);
    trashOf(path).then(
      l => alive && setList(l),
      e => alive && setError((e as Error).message || '网络错误'),
    );
    return () => {
      alive = false;
    };
  }, [path, rev, tick]);
  return { list, error, reload: () => setTick(t => t + 1) };
}

/** 侧栏的筛选：#编号、标题、描述 */
export function trashHit(t: Pick<TrashCard, 'id' | 'title' | 'body'>, q: string) {
  if (!q) return true;
  if (q.startsWith('#')) return String(t.id).startsWith(q.slice(1));
  return String(t.id) === q || t.title.toLowerCase().includes(q) || t.body.toLowerCase().includes(q);
}

const nameOf = (key: string, statuses: Status[] | undefined) => statuses?.find(s => s.key === key)?.name ?? key;
const colorOf = (key: string, statuses: Status[] | undefined) => statuses?.find(s => s.key === key)?.color ?? 'todo';

/** 描述的第一行文字（跳过小标题、图片） */
function firstLine(body: string) {
  const lines = body.split('\n').map(l => l.trim()).filter(Boolean);
  const text = (l: string) => l.replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1').replace(/^[-*#>\s]+|\[[ xX]\]\s*/g, '').trim();
  return lines.filter(l => !/^#{1,6}\s/.test(l)).map(text).find(Boolean) ?? '';
}
