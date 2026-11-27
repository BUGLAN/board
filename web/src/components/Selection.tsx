/** 勾选、快捷键和底部批量条的公共实现 */
import { useCallback, useEffect, useRef, type ReactNode } from 'react';
import { rangeIds, unionIds } from '../lib/selection';
import { cx } from '../lib/util';
import { useData } from '../store/data';
import { overlayOpen } from '../store/ui';
import { Tip } from './bits';
import { Icon } from './Icon';

const typing = (target: EventTarget | null) =>
  !!(target as HTMLElement | null)?.closest?.('input, textarea, select, [contenteditable]');

type SelectUpdate = (update: (current: number[]) => number[]) => void;

interface RangeSelectOptions {
  /** 这个编号能不能被选上（例如还在新建的临时卡） */
  canSelect?: (id: number) => boolean;
  /** 换了页面、白板时清掉连选起点 */
  resetKey?: unknown;
}

/** 点一张切换勾选；按住 Shift 从上一张连选。勾选本身由调用方保存 */
export function useRangeSelect(order: readonly number[], setSelected: SelectUpdate, options: RangeSelectOptions = {}) {
  const anchor = useRef<number | null>(null);
  const latest = useRef({ order, setSelected, options });
  latest.current = { order, setSelected, options };

  useEffect(() => {
    anchor.current = null;
  }, [options.resetKey]);

  return useCallback((id: number, shift: boolean) => {
    const { order, setSelected, options } = latest.current;
    const canSelect = options.canSelect ?? (() => true);
    if (!canSelect(id)) return;
    const range = shift ? rangeIds(order, anchor.current, id)?.filter(canSelect) : null;
    setSelected(current => {
      const on = !current.includes(id);
      if (range)
        return on ? unionIds(current, range) : current.filter(x => !range.includes(x));
      return on ? [...current, id] : current.filter(x => x !== id);
    });
    anchor.current = id;
  }, []);
}

interface SelectKeysOptions {
  /** 复选框里的这些编号能全选 */
  ids: readonly number[];
  onSelectAll: (ids: number[]) => void;
  /** 有东西可以取消时返回 true，避免输入框、弹层里的 esc 被抢走 */
  onEscape: () => boolean;
  /** 当前页面另有弹层（例如合并预览）时不接快捷键 */
  blocked?: () => boolean;
  enabled?: boolean;
}

/** Esc 取消、⌘A / Ctrl+A 全选：输入框、弹层、卡片详情里不接 */
export function useSelectKeys({ ids, onSelectAll, onEscape, blocked, enabled = true }: SelectKeysOptions) {
  const latest = useRef({ ids, onSelectAll, onEscape, blocked });
  latest.current = { ids, onSelectAll, onEscape, blocked };

  useEffect(() => {
    if (!enabled) return;
    const key = (e: KeyboardEvent) => {
      if (typing(e.target) || overlayOpen() || useData.getState().detailId != null || latest.current.blocked?.()) return;
      if (e.key === 'Escape') {
        if (latest.current.onEscape()) e.preventDefault();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a' && latest.current.ids.length) {
        e.preventDefault();
        latest.current.onSelectAll([...latest.current.ids]);
      }
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [enabled]);
}

interface BulkBarProps {
  /** 批量条是否露出来 */
  open: boolean;
  count: number;
  /** 已选几张的单位：条 / 张 */
  unit?: '条' | '张';
  /** 一张没选时显示的提示 */
  hint?: ReactNode;
  className?: string;
  onClear: () => void;
  clearLabel?: string;
  children: ReactNode;
}

/** 从底部浮出的批量条外壳：张数、分隔线、各页自己的按钮、取消 */
export function BulkBar({ open, count, unit = '条', hint, className, onClear, clearLabel = '取消选择', children }: BulkBarProps) {
  return (
    <div
      className={cx('ib-bulk', className, open && 'on')}
      role="toolbar"
      aria-label="批量操作"
      aria-hidden={!open}
      inert={!open || undefined}
    >
      <span className="ib-bulk-n">
        {count > 0 ? (
          <>
            <b className="mono" key={count}>
              {count}
            </b>
            {unit}已选
          </>
        ) : (
          hint
        )}
      </span>
      <i className="ib-bulk-sep" />
      {children}
      <Tip text={`${clearLabel} · esc`}>
        <button type="button" className="icon-btn sm" aria-label={clearLabel} data-tip-pos="top" onClick={onClear}>
          <Icon name="close" />
        </button>
      </Tip>
    </div>
  );
}
