/* ==========================================================================
   回收站（PRD §8 规则 10、§10.3）：白板 Tab 侧栏底部的「回收站 · N」打开
   - 居中弹窗，列出这块白板删掉的卡片：编号、标题、删除时间、谁删的、原来在哪个状态
   - 每张可以「恢复」或「永久删除」，勾选几条可以一起操作
   - 超过 30 天的卡片由服务端自动永久删除（附件一起删）
   ========================================================================== */
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { statusColor } from '../../lib/schema';
import { purgeCards, restoreMany, restoredToast, trashOf } from '../../lib/trash';
import type { Status, TrashCard } from '../../lib/types';
import { actorName, ago, cx } from '../../lib/util';
import { useNow } from '../bits';
import { useData } from '../../store/data';
import { confirm, openTrash, useUI } from '../../store/ui';
import { Icon } from '../Icon';
import { Skeleton } from '../Skeleton';
import { usePresence } from './presence';

/** 删除后多少天自动永久删除（和服务端一致） */
const KEEP_DAYS = 30;
/** 还剩几天以内时用警告色 */
const SOON_DAYS = 7;

export function TrashDialog() {
  const path = useUI(s => s.trash);
  const { mounted, on } = usePresence(!!path, 220);
  const last = useRef(path);
  if (path) last.current = path;
  if (!mounted || !last.current) return null;
  // 弹窗是 aside（和卡片详情一样），底下的遮罩必须是 div：scrim 的样式写在 div 上
  return createPortal(
    <div>
      <div className={cx('scrim', on && 'on')} onClick={() => openTrash(null)} />
      <TrashBody path={last.current} on={on} />
    </div>,
    document.body,
  );
}

function TrashBody({ path, on }: { path: string; on: boolean }) {
  const board = useData(s => (s.board?.path === path ? s.board : null));
  // 和卡片上的相对时间一样，按服务端时间算，每分钟刷新
  const now = useNow();
  const statuses = board?.statuses;
  const { list, error, reload } = useTrash(path, on);
  const [checked, setChecked] = useState<Set<number>>(() => new Set());
  const [busy, setBusy] = useState(false);
  const close = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!on) return;
    setChecked(new Set());
    const t = setTimeout(() => close.current?.focus({ preventScroll: true }), 80);
    return () => clearTimeout(t);
  }, [on, path]);

  useEffect(() => {
    if (!on) return;
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      if (useUI.getState().confirm) return; // 确认框先关自己
      e.preventDefault();
      e.stopPropagation();
      openTrash(null);
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [on]);

  const all = list ?? [];
  const ids = all.map(t => t.id);
  const chosen = ids.filter(id => checked.has(id));
  const allOn = ids.length > 0 && chosen.length === ids.length;
  const drop = (targets: number[]) =>
    setChecked(prev => {
      const next = new Set(prev);
      targets.forEach(id => next.delete(id));
      return next;
    });

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
    <aside className={cx('drawer trash-modal', on && 'on')} role="dialog" aria-modal="true" aria-hidden={!on} aria-label="回收站">
      <header className="d-head">
        <span className="trash-title">
          <Icon name="trash" />
          <b>回收站</b>
          <span className="trash-sub">
            {error ? '没取到回收站' : list ? `${all.length} 张 · 删除后 ${KEEP_DAYS} 天内可以恢复` : '正在加载…'}
          </span>
        </span>
        <span className="spacer" />
        <button ref={close} aria-label="关闭" className="icon-btn" type="button" data-tip="关闭  esc" data-tip-align="end" onClick={() => openTrash(null)}>
          <Icon name="close" />
        </button>
      </header>

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

      <div className="trash-scroll">
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
        ) : all.length === 0 ? (
          <div className="trash-empty">
            <Icon name="trash" />
            <b>回收站是空的</b>
            <span>
              删除的卡片会先放在这里，{KEEP_DAYS} 天内可以恢复；超过 {KEEP_DAYS} 天、或者在这里点了「永久删除」的，附件一起删掉，不能再恢复。
            </span>
          </div>
        ) : (
          <div className="trash-list" role="list">
            {all.map(t => {
              const days = Math.floor((now - t.deletedAt) / 86_400_000);
              const left = Math.max(0, KEEP_DAYS - days);
              return (
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
                  <span className={cx('trash-left', left <= SOON_DAYS && 'soon')} data-tip={left > 0 ? `${left} 天后自动永久删除` : '马上会永久删除'}>
                    {left > 0 ? `${left} 天` : '即将'}
                  </span>
                  <span className="trash-acts">
                    <button className="btn sm" type="button" disabled={busy} onClick={() => void restore([t.id])} data-tip="恢复到原来状态">
                      <Icon name="replay" />
                      恢复
                    </button>
                    <button className="icon-btn sm danger" type="button" aria-label={`永久删除 #${t.id}`} data-tip="永久删除" disabled={busy} onClick={() => void purge([t.id])}>
                      <Icon name="trash" />
                    </button>
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </aside>
  );
}

/** 回收站里的卡片：打开时取一次，随 rev（任何写入）重新取；重新取的时候保留上一份，不闪空白 */
function useTrash(path: string, on: boolean) {
  const rev = useData(s => (s.board?.path === path ? s.board.rev : s.rev));
  const [list, setList] = useState<TrashCard[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!on) return;
    let alive = true;
    setError(null);
    trashOf(path).then(
      l => alive && setList(l),
      e => alive && setError((e as Error).message || '网络错误'),
    );
    return () => {
      alive = false;
    };
  }, [path, on, rev, tick]);
  return { list, error, reload: () => setTick(t => t + 1) };
}

const nameOf = (key: string, statuses: Status[] | undefined) => statuses?.find(s => s.key === key)?.name ?? key;
const colorOf = (key: string, statuses: Status[] | undefined) => statuses?.find(s => s.key === key)?.color ?? 'todo';

/** 描述的第一行文字（跳过小标题、图片） */
function firstLine(body: string) {
  const lines = body.split('\n').map(l => l.trim()).filter(Boolean);
  const text = (l: string) => l.replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1').replace(/^[-*#>\s]+|\[[ xX]\]\s*/g, '').trim();
  return lines.filter(l => !/^#{1,6}\s/.test(l)).map(text).find(Boolean) ?? '';
}
