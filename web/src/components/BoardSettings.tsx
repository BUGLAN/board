/* ==========================================================================
   白板设置（工作流条末尾「白板设置」）：每块白板自己的设置，存在服务端、所有窗口和机器同步
   - 布局：保存布局（把现在状态块的位置和宽度存下来）、应用布局（换回存下的）、复原布局（默认布局，存下的保留）
   - 保存顺序：开着时可以在同一状态块里拖动卡片调整顺序；关掉后只能换状态
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../lib/api';
import type { BoardSnapshot, Tile } from '../lib/types';
import { cx } from '../lib/util';
import { flowKey, optimistic, useBusy, useData } from '../store/data';
import { toast, toastError } from '../store/ui';
import { withDefaults } from './BoardView';

/** 改白板设置：当帧生效（给白板快照叠一层补丁），被拒绝时改回来并提示。和工作流共用 flowKey：保存完之前不能再改布局 */
const run = (path: string, board: (b: BoardSnapshot) => BoardSnapshot, req: () => Promise<unknown>, done?: string) =>
  optimistic({ board, keys: [flowKey(path)] }, req)
    .then(() => done && toast({ type: 'success', title: done }))
    .catch(toastError);

const sameLayout = (a: Record<string, Tile>, b: Record<string, Tile>) =>
  Object.keys(a).length === Object.keys(b).length && Object.entries(a).every(([k, t]) => b[k] && b[k].x === t.x && b[k].w === t.w && b[k].y === t.y);

export function BoardSettings({ path, anchor, onClose }: { path: string; anchor: HTMLElement | null; onClose: () => void }) {
  const board = useData(s => (s.board?.path === path ? s.board : null));
  const busy = useBusy(flowKey(path));
  const close = useRef(onClose);
  close.current = onClose;
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const place = () => {
      const r = anchor?.getBoundingClientRect();
      if (r) setPos({ left: Math.max(8, Math.min(r.right - 300, innerWidth - 308)), top: r.bottom + 8 });
    };
    place();
    addEventListener('resize', place);
    addEventListener('scroll', place, true);
    return () => {
      removeEventListener('resize', place);
      removeEventListener('scroll', place, true);
    };
  }, [anchor]);
  useEffect(() => {
    const down = (e: PointerEvent) => {
      if (!(e.target as HTMLElement).closest('.bs-menu, .wf-settings')) close.current();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        close.current();
      }
    };
    addEventListener('pointerdown', down);
    addEventListener('keydown', key, true);
    return () => {
      removeEventListener('pointerdown', down);
      removeEventListener('keydown', key, true);
    };
  }, []);

  if (!pos || !board) return null;
  const keys = board.statuses.map(s => s.key);
  const current = Object.fromEntries(board.statuses.filter(s => s.layout).map(s => [s.key, s.layout!]));
  // 界面上现在的样子（没摆过的块按默认规则）
  const shown = withDefaults(keys, current);
  const saved = board.savedLayout ?? null;
  const hasCurrent = Object.keys(current).length > 0;
  const keepOrder = board.keepOrder !== false;

  const save = () => run(path, b => ({ ...b, savedLayout: shown }), () => api.saveLayout(path, shown), '已保存布局');
  const apply = () =>
    saved &&
    run(
      path,
      // 已删除的状态不在 saved 里；之后新增的状态没有位置，按默认规则补位
      b => ({ ...b, statuses: b.statuses.map(s => ({ ...s, layout: saved[s.key] ?? null })) }),
      () => api.applyLayout(path),
    );
  const reset = () => run(path, b => ({ ...b, statuses: b.statuses.map(s => (s.layout ? { ...s, layout: null } : s)) }), () => api.flowLayout(path, {}));
  const order = () => run(path, b => ({ ...b, keepOrder: !keepOrder }), () => api.keepOrder(path, !keepOrder));

  const savedNow = !!saved && sameLayout(withDefaults(keys, saved), shown);
  return createPortal(
    <div
      className={cx('wf-menu bs-menu', busy && 'busy')}
      role="dialog"
      aria-label="白板设置"
      aria-busy={busy || undefined}
      style={{ ['--c' as string]: 'var(--brand)', position: 'fixed', left: pos.left, top: pos.top }}
      onPointerDown={e => e.stopPropagation()}
    >
      <div className="bs-title">白板设置</div>
      <section className="bs-sec">
        <div className="bs-head">
          <b>状态块布局</b>
          <span>{saved ? (savedNow ? '现在就是保存的布局' : '保存过一份布局') : '还没有保存过布局'}</span>
        </div>
        <div className="bs-acts">
          <button type="button" className="btn sm" onClick={save} disabled={savedNow} title="把现在状态块的位置和宽度存成这块白板的布局">
            保存布局
          </button>
          <button type="button" className="btn sm" onClick={apply} disabled={!saved || savedNow} title="换回保存的布局；之后新增的状态按默认规则补位">
            应用布局
          </button>
          <button type="button" className="btn sm ghost" onClick={reset} disabled={!hasCurrent} title="回到默认布局；保存的布局保留，可以再应用">
            复原布局
          </button>
        </div>
      </section>
      <section className="bs-sec">
        <button type="button" className={cx('wf-sw', keepOrder && 'on')} role="switch" aria-checked={keepOrder} onClick={order}>
          <span className="wf-sw-track">
            <i />
          </span>
          <span className="wf-sw-text">
            <b>保存顺序</b>
            <span>{keepOrder ? '可以在同一状态里拖动卡片调整顺序，顺序保存在服务端' : '同一状态里不能拖动排序，只能拖到别的状态'}</span>
          </span>
        </button>
      </section>
    </div>,
    document.body,
  );
}
