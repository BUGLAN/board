/** 模态弹窗打开时，把背后的 .app 从 Tab 序列和可访问树里移出去。
    弹窗都挂在 .app 外面，读屏能看出 aria-modal，但键盘仍然会走到背景；这里统一用原生 inert，
    不再让每个弹窗各写一套焦点圈定。 */

/** 页面上有没有正打开的模态弹窗（.app 里面的不算，否则会把它自己也 inert 掉） */
export function hasOpenModal(app: HTMLElement): boolean {
  return [...document.querySelectorAll<HTMLElement>('[aria-modal="true"]')].some(
    el => !app.contains(el) && el.getAttribute('aria-hidden') !== 'true' && !el.closest('[aria-hidden="true"]'),
  );
}

/** 跟随弹窗的挂载、aria-modal 和 aria-hidden 变化切换 .app 的 inert；返回停止观察和清理的函数 */
export function observeAppInert(app: HTMLElement, knownOpen = false): () => void {
  const sync = () => app.toggleAttribute('inert', knownOpen || hasOpenModal(app));
  sync();
  const observer = new MutationObserver(sync);
  observer.observe(document.body, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ['aria-modal', 'aria-hidden'],
  });
  return () => {
    observer.disconnect();
    app.removeAttribute('inert');
  };
}
