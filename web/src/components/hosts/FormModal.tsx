/* 共用配置的添加 / 修改表单放在居中弹窗里（不把列表往下推）：标题、关闭、正文、底部一行（校验提示 + 取消 / 保存）。
   esc、点遮罩关闭；打开时焦点给第一个能填的框 */
import { useEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { cx } from '../../lib/util';
import { useUI } from '../../store/ui';
import { Icon } from '../Icon';

export function FormModal({ title, onCancel, children, foot, wide }: { title: string; onCancel: () => void; children: ReactNode; foot: ReactNode; wide?: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const cancel = useRef(onCancel);
  cancel.current = onCancel;
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      // 选择框的列表、确认框开着时先关它们
      const ui = useUI.getState();
      if (ui.ctx || ui.confirm) return;
      e.stopPropagation();
      cancel.current();
    };
    window.addEventListener('keydown', key, true);
    box.current?.querySelector<HTMLInputElement>('input:not(:disabled), textarea')?.focus({ preventScroll: true });
    return () => window.removeEventListener('keydown', key, true);
  }, []);
  return createPortal(
    <>
      <div className="scrim on fm-scrim" onClick={onCancel} />
      <div className="fm-wrap">
        <div ref={box} className={cx('fm', wide && 'wide')} role="dialog" aria-modal="true" aria-label={title}>
          <div className="fm-head">
            <b>{title}</b>
            <button className="icon-btn sm" type="button" aria-label="关闭" onClick={onCancel}>
              <Icon name="close" />
            </button>
          </div>
          <div className="fm-body">{children}</div>
          <div className="fm-foot">{foot}</div>
        </div>
      </div>
    </>,
    document.body,
  );
}

/** 底部一行：左边是校验提示（点过保存才显示），右边取消 / 保存 */
export function FormFoot({ why, tried, busy, onCancel, onSave, left }: { why: string | null; tried: boolean; busy: boolean; onCancel: () => void; onSave: () => void; left?: ReactNode }) {
  return (
    <>
      {tried && why ? (
        <span className="fm-why">
          <Icon name="alert" />
          {why}
        </span>
      ) : (
        <span className="fm-left">{left}</span>
      )}
      <button className="btn sm ghost" type="button" disabled={busy} onClick={onCancel}>
        取消
      </button>
      <button className={cx('btn sm brand', busy && 'is-busy')} type="button" disabled={busy} onClick={onSave}>
        保存
      </button>
    </>
  );
}
