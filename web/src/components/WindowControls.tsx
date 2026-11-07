/* ==========================================================================
   窗口按钮：最小化 / 最大化（还原）/ 关闭
   只在没有系统标题栏的外壳里出现（Windows 桌面端），样式对齐 Windows 11 的标题栏按钮
   ========================================================================== */
import { useEffect, useState } from 'react';
import { closeWindow, customTitlebar, minimizeWindow, shell, startWindowDrag, toggleMaximize, watchMaximized } from '../lib/platform';

export function WindowControls() {
  const [max, setMax] = useState(false);
  useEffect(() => (customTitlebar ? watchMaximized(setMax) : undefined), []);
  if (!customTitlebar) return null;
  return (
    <div className="win-controls">
      <button type="button" className="win-btn" aria-label="最小化" title="最小化" onClick={minimizeWindow}>
        <svg viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 5.5h10" />
        </svg>
      </button>
      <button type="button" className="win-btn" aria-label={max ? '还原' : '最大化'} title={max ? '还原' : '最大化'} onClick={toggleMaximize}>
        {max ? (
          <svg viewBox="0 0 10 10" aria-hidden="true">
            <rect x=".5" y="2.5" width="7" height="7" rx="1" />
            <path d="M2.5 2.5v-.5a1.5 1.5 0 0 1 1.5-1.5h4a1.5 1.5 0 0 1 1.5 1.5v4A1.5 1.5 0 0 1 8 7.5h-.5" />
          </svg>
        ) : (
          <svg viewBox="0 0 10 10" aria-hidden="true">
            <rect x=".5" y=".5" width="9" height="9" rx="1" />
          </svg>
        )}
      </button>
      <button type="button" className="win-btn close" aria-label="关闭" title="关闭" onClick={closeWindow}>
        <svg viewBox="0 0 10 10" aria-hidden="true">
          <path d="m.5.5 9 9m0-9-9 9" />
        </svg>
      </button>
    </div>
  );
}

/** 没有顶栏的页面（登录、连接中）：桌面端顶部一条拖动区域，Windows 上右上角是窗口按钮 */
export function TitleStrip() {
  if (shell !== 'desktop') return null;
  const onDown = (e: React.MouseEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('button')) return;
    if (e.detail === 2) toggleMaximize();
    else startWindowDrag();
  };
  return (
    <div className="title-strip" onMouseDown={onDown}>
      <WindowControls />
    </div>
  );
}
