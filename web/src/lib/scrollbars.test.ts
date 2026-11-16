import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { dismissScrollbars, installScrollbars, scrollSilently } from './scrollbars';

const made: HTMLElement[] = [];

function area() {
  const el = document.createElement('div');
  el.style.overflowY = 'auto';
  Object.defineProperty(el, 'scrollHeight', { configurable: true, value: 1000 });
  Object.defineProperty(el, 'clientHeight', { configurable: true, value: 100 });
  document.body.appendChild(el);
  made.push(el);
  return el;
}

function scroll(el: HTMLElement, top: number) {
  el.scrollTop = top;
  el.dispatchEvent(new Event('scroll'));
}

describe('浮层滚动条', () => {
  beforeAll(() => {
    vi.stubGlobal('requestAnimationFrame', () => 0);
    installScrollbars();
  });

  afterEach(() => {
    dismissScrollbars();
    for (const el of made.splice(0)) el.remove();
  });

  it('程序化滚动不画滚动条', () => {
    const el = area();
    scrollSilently(el, () => scroll(el, 200));
    expect(document.querySelector('.sbar')).toBeNull();
  });

  it('用户一动滚轮就结束静默', () => {
    const el = area();
    scrollSilently(el, () => scroll(el, 200));
    document.body.dispatchEvent(new Event('wheel', { bubbles: true }));

    scroll(el, 300);
    expect(document.querySelector('.sbar')).not.toBeNull();
  });

  it('弹窗打开时，下面的滚动不画滚动条', () => {
    const below = area();
    const modal = document.createElement('div');
    modal.className = 'drawer on';
    document.body.appendChild(modal);
    made.push(modal);

    scroll(below, 200);
    expect(document.querySelector('.sbar')).toBeNull();

    const inside = area();
    modal.appendChild(inside);
    scroll(inside, 100);
    expect(document.querySelector('.sbar')).not.toBeNull();
  });

  it('打开浮层时撤掉下面正显示着的滚动条', () => {
    const el = area();
    scroll(el, 200);
    expect(document.querySelector('.sbar')).not.toBeNull();

    dismissScrollbars();
    expect(document.querySelector('.sbar')).toBeNull();
  });
});
