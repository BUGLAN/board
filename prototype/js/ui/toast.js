/* ==========================================================================
   Board · Toast
   toast({ type: 'error'|'success'|'info', title, text, code, duration, action:{label,fn} })
   ========================================================================== */
(function (B) {
  'use strict';
  const { $, el, esc, icons } = B.ui;
  const MAX = 4;

  function toast(opt) {
    const wrap = $('#toasts');
    const type = opt.type || 'info';
    const duration = opt.duration ?? (type === 'error' ? 5200 : 3200);
    const ic = type === 'error' ? icons.alert : type === 'success' ? icons.checkCircle : icons.info;
    const node = el(`
      <div class="toast ${type}" role="${type === 'error' ? 'alert' : 'status'}">
        <div class="toast-ic">${ic}</div>
        <div class="toast-body">
          <div class="toast-title">${esc(opt.title || '')}${opt.code ? `<span class="code-chip">${esc(opt.code)}</span>` : ''}</div>
          ${opt.text ? `<div class="toast-text">${opt.html ? opt.text : esc(opt.text)}</div>` : ''}
          ${opt.action ? `<div class="toast-action"><button class="btn sm" type="button">${esc(opt.action.label)}</button></div>` : ''}
        </div>
        <button class="toast-close" type="button" aria-label="关闭">${icons.close}</button>
        <div class="toast-bar" style="animation-duration:${duration}ms"></div>
      </div>`);
    const close = () => {
      if (node.classList.contains('out')) return;
      node.classList.add('out');
      node.addEventListener('animationend', () => node.remove(), { once: true });
    };
    node.querySelector('.toast-close').onclick = close;
    if (opt.action) node.querySelector('.toast-action button').onclick = () => { opt.action.fn(); close(); };
    node.querySelector('.toast-bar').addEventListener('animationend', close);
    wrap.appendChild(node);
    const list = [...wrap.children].filter(n => !n.classList.contains('out'));
    if (list.length > MAX) list.slice(0, list.length - MAX).forEach(n => n.querySelector('.toast-close').click());
    return close;
  }

  /** 把 BoardError 转成友好的提示 */
  const TITLES = {
    FORBIDDEN_COLUMN: '不能放到这一列',
    NOT_HOLDER: '这张卡片正被 agent 持有',
    EMPTY_TITLE: '标题不能为空',
    NOT_FOUND: '卡片不存在',
    USAGE: '操作无效',
  };
  function toastError(err) {
    return toast({ type: 'error', title: TITLES[err.code] || '操作被拒绝', code: err.code, text: err.message });
  }

  B.ui.toast = toast;
  B.ui.toastError = toastError;
})(window.Board = window.Board || {});
