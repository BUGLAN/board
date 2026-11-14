/* 共用配置的添加 / 修改表单（模型供应商、MCP 服务器）放在居中弹窗里，同一种样子（PRD §10.12「共用配置」）：
   标题、关闭；正文分几段（小标题 + 一句说明），字段的标签在上、说明在下；可以有一排「磁贴」二选一 / 多选一；
   底部一行：左边写哪些 agent 会用到它（图标亮的是），点过保存且没填对时换成原因；右边取消 / 保存。
   esc、点遮罩关闭；打开时焦点给第一个能填的框 */
import { useEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { cx } from '../../lib/util';
import { useUI } from '../../store/ui';
import { Tip } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon } from './bits';

export function FormModal({ title, onCancel, children, foot }: { title: string; onCancel: () => void; children: ReactNode; foot: ReactNode }) {
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
    box.current?.querySelector<HTMLInputElement>('.fm-body input:not(:disabled), .fm-body textarea')?.focus({ preventScroll: true });
    return () => window.removeEventListener('keydown', key, true);
  }, []);
  return createPortal(
    <>
      <div className="scrim on fm-scrim" onClick={onCancel} />
      <div className="fm-wrap">
        <div ref={box} className="fm" role="dialog" aria-modal="true" aria-label={title}>
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

/** 一段：小标题（后面一句说明），右边可以放一个操作 */
export function FormSec({ title, hint, act, children }: { title: string; hint?: string; act?: ReactNode; children: ReactNode }) {
  return (
    <section className="fm-sec">
      <h4>
        <span>
          {title}
          {hint && <small>{hint}</small>}
        </span>
        {act}
      </h4>
      {children}
    </section>
  );
}

/** 一个字段：标签在上，说明在下 */
export function FormField({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="fm-field">
      <span>{label}</span>
      {children}
      {hint && <em>{hint}</em>}
    </label>
  );
}

/** 一块磁贴：左边一个色块或图标，右边名称和一句说明 */
export function FormTile({ on, mark, color, title, desc, onClick }: { on: boolean; mark: ReactNode; color?: string; title: string; desc: string; onClick: () => void }) {
  return (
    <button type="button" className={cx('fm-tile', on && 'on')} aria-pressed={on} onClick={onClick}>
      <span className={cx('fm-mark', !color && 'plain')} style={color ? { ['--vc' as string]: color } : undefined}>
        {mark}
      </span>
      <span>
        <b>{title}</b>
        <small>{desc}</small>
      </span>
    </button>
  );
}

/** 项目自己的开关（不用系统的勾选框） */
export function FormSwitch({ on, onClick, label }: { on: boolean; onClick: () => void; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} className={cx('toggle fm-switch', on && 'on')} onClick={onClick}>
      <i />
    </button>
  );
}

/** 底部一行：左边是哪些 agent 会用到它（或点过保存后的校验原因），右边取消 / 保存 */
export function FormFoot({
  why,
  tried,
  busy,
  onCancel,
  onSave,
  agents,
  agentsLabel,
  note,
  saveLabel = '保存',
  cancelLabel = '取消',
  canSave = true,
}: {
  why: string | null;
  tried: boolean;
  busy: boolean;
  onCancel: () => void;
  onSave: () => void;
  agents?: { id: string; on: boolean; tip: string }[];
  agentsLabel?: string;
  /** 没有 agent 图标时左边写的一句话 */
  note?: string;
  saveLabel?: string;
  cancelLabel?: string;
  canSave?: boolean;
}) {
  return (
    <>
      {tried && why ? (
        <span className="fm-why">
          <Icon name="alert" />
          {why}
        </span>
      ) : (
        <span className="fm-who">
          {note}
          {agents && (
            <>
              <span>{agentsLabel}</span>
              {agents.map(a => (
                <Tip key={a.id} text={a.tip}>
                  <span className={cx('fm-who-a', a.on && 'on')}>
                    <AgentIcon id={a.id} />
                  </span>
                </Tip>
              ))}
            </>
          )}
        </span>
      )}
      <button className="btn sm ghost" type="button" disabled={busy} onClick={onCancel}>
        {cancelLabel}
      </button>
      <button className={cx('btn sm brand', busy && 'is-busy')} type="button" disabled={busy || !canSave} onClick={onSave}>
        {saveLabel}
      </button>
    </>
  );
}
