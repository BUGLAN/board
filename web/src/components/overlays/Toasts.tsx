/* 提示条（toast） */
import { useState } from 'react';
import { cx } from '../../lib/util';
import { dismissToast, useUI, type Toast } from '../../store/ui';
import { Icon, type IconName } from '../Icon';

export function Toasts() {
  const toasts = useUI(s => s.toasts);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map(t => (
        <ToastItem key={t.id} t={t} />
      ))}
    </div>
  );
}

function ToastItem({ t }: { t: Toast }) {
  const [out, setOut] = useState(false);
  const close = () => setOut(true);
  const icon: IconName = t.type === 'error' ? 'alert' : t.type === 'success' ? 'checkCircle' : 'info';
  return (
    <div className={cx('toast', t.type, out && 'out')} role={t.type === 'error' ? 'alert' : 'status'} onAnimationEnd={e => out && e.target === e.currentTarget && dismissToast(t.id)}>
      <div className="toast-ic">
        <Icon name={icon} />
      </div>
      <div className="toast-body">
        <div className="toast-title">
          {t.title}
          {t.code && <span className="code-chip">{t.code}</span>}
        </div>
        {t.text && <div className="toast-text">{t.text}</div>}
        {t.action && (
          <div className="toast-action">
            <button
              className="btn sm"
              type="button"
              onClick={() => {
                t.action!.fn();
                close();
              }}
            >
              {t.action.label}
            </button>
          </div>
        )}
      </div>
      <button className="toast-close" type="button" aria-label="关闭" onClick={close}>
        <Icon name="close" />
      </button>
      <div className="toast-bar" style={{ animationDuration: `${t.duration}ms` }} onAnimationEnd={close} />
    </div>
  );
}
