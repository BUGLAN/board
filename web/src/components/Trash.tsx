/* ==========================================================================
   回收站（PRD §8 规则 10、§10.5）：「未整理」页标题的第三个标签（未整理 | 稍后再做 | 回收站，default/main#486）
   - 列出这块白板删掉的卡片（最近删的在前）：编号、标题、描述第一行、原来在哪个状态、谁删的、什么时候删的
   - 每张可以「恢复」或「永久删除」，勾选几条可以一起操作；侧栏的筛选也管这一页
   - 删掉的卡片一直留着，服务端不自动清理：只有人在这里点「永久删除」才真的删掉（附件一起删）
   ========================================================================== */
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { statusColor } from '../lib/schema';
import { timeGroups } from '../lib/split';
import { purgeCards, restoreMany, restoredToast, trashOf } from '../lib/trash';
import type { Status, TrashCard } from '../lib/types';
import { actorName, ago, cx } from '../lib/util';
import { ActorAvatar, Tip, Title, useNow } from './bits';
import { useData } from '../store/data';
import { confirm, overlayOpen, setFilter, useUI } from '../store/ui';
import { Check, RowsSkeleton } from './ibRows';
import { Icon } from './Icon';

/** 回收站页：和「稍后再做」同一套样子（default/main#486 复盘）——按删除时间分组（今天、这一周、更早），
    组头能整组勾选；行是「未整理」的行（勾选、编号、谁删的、标题和描述第一行、原来的状态、删除时间），
    鼠标移上去露出「恢复」「永久删除」；勾选后底部弹出批量条（挂在 bulkHost 上：「未整理」页的根，贴着视口底部） */
export function TrashPage({ path, bulkHost }: { path: string; bulkHost?: HTMLElement | null }) {
  const board = useData(s => (s.board?.path === path ? s.board : null));
  // 和卡片上的相对时间一样，按服务端时间算，每分钟刷新
  const now = useNow(60000);
  const statuses = board?.statuses;
  const { list, error, reload } = useTrash(path);
  const filter = useUI(s => s.filter);
  const q = filter.trim().toLowerCase();
  const [checked, setChecked] = useState<number[]>([]);
  const [busy, setBusy] = useState(false);
  const lastCheck = useRef<number | null>(null);

  // 换白板：勾选清空
  useEffect(() => setChecked([]), [path]);
  // 恢复、永久删除了，或者别的窗口动了：不在回收站里的从勾选里去掉
  useEffect(() => setChecked(cs => (list && cs.some(id => !list.some(t => t.id === id)) ? cs.filter(id => list.some(t => t.id === id)) : cs)), [list]);

  // 最近删的在前（服务端按删除时间给，最近删的在最后）
  const all = useMemo(() => [...(list ?? [])].reverse().filter(t => trashHit(t, q)), [list, q]);
  const groups = useMemo(() => timeGroups(all, t => t.deletedAt, now, '删除'), [all, now]);
  const flat = useMemo(() => groups.flatMap(g => g.rows), [groups]);

  const check = (id: number, shift: boolean) => {
    setChecked(cur => {
      const on = !cur.includes(id);
      if (shift && lastCheck.current != null && flat.some(t => t.id === lastCheck.current)) {
        const a = flat.findIndex(t => t.id === lastCheck.current);
        const b = flat.findIndex(t => t.id === id);
        const range = flat.slice(Math.min(a, b), Math.max(a, b) + 1).map(t => t.id);
        return on ? [...new Set([...cur, ...range])] : cur.filter(c => !range.includes(c));
      }
      return on ? [...cur, id] : cur.filter(c => c !== id);
    });
    lastCheck.current = id;
  };

  // esc 取消勾选，⌘A / Ctrl+A 全选（和「未整理」「稍后再做」一样）
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest?.('input, textarea, select, [contenteditable]') || overlayOpen() || useData.getState().detailId != null) return;
      if (e.key === 'Escape' && checked.length) {
        e.preventDefault();
        setChecked([]);
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a' && flat.length) {
        e.preventDefault();
        setChecked(flat.map(t => t.id));
      }
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [flat, checked.length]);

  const drop = (targets: number[]) => setChecked(cur => cur.filter(id => !targets.includes(id)));

  const restore = async (targets: number[]) => {
    if (!targets.length || busy) return;
    setBusy(true);
    try {
      const result = await restoreMany(path, targets);
      if (result.ok.length) restoredToast(result.ok, path);
      drop(result.ok);
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
      const result = await purgeCards(targets);
      drop(result.ok);
    } finally {
      setBusy(false);
    }
  };

  if (error)
    return (
      <div className="ib-empty">
        <b>回收站没取回来</b>
        <span>{error}</span>
        <button className="btn sm" type="button" onClick={reload}>
          重试
        </button>
      </div>
    );
  if (!list) return <RowsSkeleton n={Math.min(Math.max(board?.trash ?? 3, 1), 5)} />;
  return (
    <div className="trash-page">
      {groups.map(g => {
        const ids = g.rows.map(t => t.id);
        const allOn = ids.every(id => checked.includes(id));
        const some = !allOn && ids.some(id => checked.includes(id));
        return (
          <section key={g.key} className="ib-grp" data-group={g.key}>
            <div className="ib-gh">
              <Check
                on={allOn}
                some={some}
                label={allOn ? `取消勾选「${g.label}」` : `勾选「${g.label}」这一组`}
                onToggle={() => setChecked(cur => (allOn ? cur.filter(c => !ids.includes(c)) : [...new Set([...cur, ...ids])]))}
              />
              <span>{g.label}</span>
              <span className="ib-gh-n mono">{g.rows.length}</span>
            </div>
            <div className="ib-rows" role="list">
              {g.rows.map(t => (
                <TrashRow key={t.id} t={t} now={now} statuses={statuses} busy={busy} checked={checked.includes(t.id)} onCheck={check} onRestore={() => void restore([t.id])} onPurge={() => void purge([t.id])} />
              ))}
            </div>
          </section>
        );
      })}
      {list.length === 0 && (
        <div className="ib-empty">
          <div className="ib-later-art trash-art" aria-hidden="true">
            <Icon name="trash" />
          </div>
          <b>回收站是空的</b>
          <span>删除的卡片会放在这里，随时可以恢复；在这里点了「永久删除」的，附件一起删掉，不能再恢复。</span>
        </div>
      )}
      {list.length > 0 && all.length === 0 && (
        <div className="ib-empty">
          <b>没有符合的卡片</b>
          <span>换个词试试，或者清除筛选。</span>
          <button className="btn sm" type="button" onClick={() => setFilter('')}>
            清除筛选
          </button>
        </div>
      )}
      {(() => {
        const bar = <TrashBulk n={checked.length} busy={busy} onRestore={() => void restore(checked)} onPurge={() => void purge(checked)} onClear={() => setChecked([])} />;
        return bulkHost ? createPortal(bar, bulkHost) : bar;
      })()}
    </div>
  );
}

/** 回收站的一行：点一下（或 x）勾选，Shift 连选；「恢复」「永久删除」鼠标移上去才露出来，手机上勾选后用底部的批量条 */
function TrashRow({
  t, now, statuses, busy, checked, onCheck, onRestore, onPurge,
}: { t: TrashCard; now: number; statuses: Status[] | undefined; busy: boolean; checked: boolean; onCheck: (id: number, shift: boolean) => void; onRestore: () => void; onPurge: () => void }) {
  const desc = firstLine(t.body);
  const inbox = t.column === 'inbox';
  const who = actorName(t.deletedBy);
  return (
    <div className={cx('ib-row', checked && 'checked')} role="listitem">
      <div
        className="ib-line ib-trash-line"
        role="button"
        tabIndex={0}
        aria-pressed={checked}
        aria-label={`#${t.id} ${t.title}`}
        onClick={e => onCheck(t.id, e.shiftKey)}
        onKeyDown={e => {
          if (e.target !== e.currentTarget) return;
          if (e.key === 'Enter' || e.key === ' ' || e.key === 'x') {
            e.preventDefault();
            onCheck(t.id, e.shiftKey);
          }
        }}
      >
        <Check on={checked} label={`勾选 #${t.id} ${t.title}`} onToggle={shift => onCheck(t.id, shift)} />
        <span className="ib-id">#{t.id}</span>
        <Tip text={`${who} 删除`}>
          <span className="ib-trash-who">
            <ActorAvatar actor={t.deletedBy} className="ib-av" />
          </span>
        </Tip>
        <span className="ib-t">
          <b>
            <Title text={t.title} />
          </b>
          {desc && <span>{desc}</span>}
        </span>
        <span className="ib-trash-from" title="删除前在哪个状态，恢复后回到这里">
          <i style={{ background: statusColor(inbox ? 'inbox' : colorOf(t.column, statuses)) }} />
          {inbox ? '未整理' : nameOf(t.column, statuses)}
        </span>
        <span className="ib-acts">
          <Tip text="恢复到原来的状态">
            <button
              type="button"
              className="icon-btn sm"
              aria-label={`恢复 #${t.id}`}
              disabled={busy}
              onClick={e => {
                e.stopPropagation();
                onRestore();
              }}
            >
              <Icon name="replay" />
            </button>
          </Tip>
          <Tip text="永久删除（连同附件，不能恢复）">
            <button
              type="button"
              className="icon-btn sm ib-del"
              aria-label={`永久删除 #${t.id}`}
              disabled={busy}
              onClick={e => {
                e.stopPropagation();
                onPurge();
              }}
            >
              <Icon name="trash" />
            </button>
          </Tip>
        </span>
        <span className="ib-meta">
          <span className="ib-age" title={`${who} 删除`}>
            {ago(t.deletedAt, now)}
          </span>
        </span>
      </div>
    </div>
  );
}

/** 勾选后的批量条（和「未整理」「稍后再做」同一个）：永久删除、恢复 */
function TrashBulk({ n, busy, onRestore, onPurge, onClear }: { n: number; busy: boolean; onRestore: () => void; onPurge: () => void; onClear: () => void }) {
  return (
    <div className={cx('ib-bulk', n > 0 && 'on')} role="toolbar" aria-label="批量操作" aria-hidden={!n} inert={!n || undefined}>
      <span className="ib-bulk-n">
        <b className="mono" key={n}>
          {n}
        </b>
        张已选
      </span>
      <i className="ib-bulk-sep" />
      <button type="button" className="btn sm ghost ib-del" disabled={busy} onClick={onPurge}>
        <Icon name="trash" />
        <span className="ib-bl">永久删除</span>
      </button>
      <button type="button" className="btn sm ghost" disabled={busy} onClick={onRestore}>
        <Icon name="replay" />
        恢复
      </button>
      <Tip text="取消选择 · esc">
        <button type="button" className="icon-btn sm" aria-label="取消选择" data-tip-pos="top" onClick={onClear}>
          <Icon name="close" />
        </button>
      </Tip>
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
