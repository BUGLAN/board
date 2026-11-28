import { afterEach, describe, expect, it } from 'vitest';
import { observeAppInert } from './modal';

let stop: (() => void) | null = null;

const tick = () => new Promise<void>(resolve => setTimeout(resolve, 0));

afterEach(() => {
  stop?.();
  stop = null;
  document.body.innerHTML = '';
});

describe('observeAppInert', () => {
  it('只让打开的模态弹窗把 .app 移出 Tab 序列，关闭后恢复', async () => {
    document.body.innerHTML = '<div class="app"><button>背景</button></div>';
    const app = document.querySelector<HTMLElement>('.app')!;
    stop = observeAppInert(app);
    expect(app.hasAttribute('inert')).toBe(false);

    const modal = document.createElement('div');
    modal.setAttribute('role', 'dialog');
    modal.setAttribute('aria-modal', 'true');
    document.body.append(modal);
    await tick();
    expect(app.hasAttribute('inert')).toBe(true);

    modal.setAttribute('aria-hidden', 'true');
    await tick();
    expect(app.hasAttribute('inert')).toBe(false);

    modal.removeAttribute('aria-hidden');
    await tick();
    expect(app.hasAttribute('inert')).toBe(true);

    modal.remove();
    await tick();
    expect(app.hasAttribute('inert')).toBe(false);
  });

  it('状态已经知道打开时立即 inert；.app 里面的模态不会把它自己 inert', () => {
    document.body.innerHTML = '<div class="app"><div role="dialog" aria-modal="true"></div></div>';
    const app = document.querySelector<HTMLElement>('.app')!;
    stop = observeAppInert(app, true);
    expect(app.hasAttribute('inert')).toBe(true);
    stop();
    stop = null;
    expect(app.hasAttribute('inert')).toBe(false);
  });
});
