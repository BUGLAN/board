/* 「跳到主内容」（#523）：在主界面最前面；回车 / 点击把焦点放进 #main，不改地址、不发请求；聚焦前视觉隐藏 */
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SkipLink } from './SkipLink';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});

const css = (name: string) => readFileSync(new URL(`../styles/${name}`, pathToFileURL(__filename)), 'utf8');

describe('跳到主内容', () => {
  it('是指向 #main 的原生链接，排在最前面，第一下 Tab 就到它', () => {
    act(() =>
      root.render(
        <>
          <SkipLink />
          <header>
            <button type="button">顶栏</button>
          </header>
          <main id="main" tabIndex={-1}>
            主内容
          </main>
        </>,
      ),
    );
    const a = host.querySelector<HTMLAnchorElement>('a.skip-link')!;
    expect(a.getAttribute('href')).toBe('#main');
    expect(a.textContent).toBe('跳到主内容');
    const focusable = [...host.querySelectorAll<HTMLElement>('a[href], button, [tabindex]:not([tabindex="-1"])')];
    expect(focusable[0]).toBe(a);
  });

  it('激活后焦点进 #main，地址不变、不发请求', () => {
    const fetch = vi.fn();
    vi.stubGlobal('fetch', fetch);
    act(() =>
      root.render(
        <>
          <SkipLink />
          <main id="main" tabIndex={-1}>
            主内容
          </main>
        </>,
      ),
    );
    const before = location.href;
    const a = host.querySelector<HTMLAnchorElement>('a.skip-link')!;
    a.focus();
    act(() => a.click());
    expect(document.activeElement?.id).toBe('main');
    expect(location.href).toBe(before);
    expect(fetch).not.toHaveBeenCalled();
  });

  it('聚焦前视觉隐藏，聚焦时显示在最上层；主区域被跳进来时不画整块焦点框', () => {
    const base = css('base.css');
    expect(base).toMatch(/\.skip-link \{[^}]*position: fixed[^}]*z-index: 10000/);
    expect(base).toMatch(/\.skip-link:not\(:focus\) \{[^}]*clip-path: inset\(50%\)/);
    expect(base).toMatch(/\.main:focus \{ outline: none; \}/);
  });
});
