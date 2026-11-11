import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Avatar, BrandMark, ColorPick, SYSTEM_NAME } from './bits';

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
});

/** 模拟系统取色器：拖动时一串 input，确定时 change（浏览器就是这样发的）。
    值走原型上的 setter（用户改值不经过 React 装在元素上的 value 跟踪），React 才认得出值变了 */
const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
function pickInDialog(el: HTMLInputElement, ...colors: string[]) {
  for (const c of colors) {
    setValue.call(el, c);
    act(() => {
      el.dispatchEvent(new Event('input', { bubbles: true }));
    });
  }
  act(() => {
    el.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('系统身份的头像', () => {
  it('系统 = board：头像里是品牌图标（四根柱子），不再有时钟 emoji，悬停提示是 board', () => {
    act(() => root.render(<Avatar actor={{ kind: 'system', name: '' }} />));
    const el = host.querySelector('.avatar') as HTMLElement;
    expect(el.title).toBe('board');
    expect(SYSTEM_NAME).toBe('board');
    expect(el.textContent).not.toContain('⏱');
    expect(el.textContent).not.toContain('系统');
    expect(el.querySelectorAll('.brand-bars i')).toHaveLength(4);
  });

  it('品牌图标的大小图标用同一组柱子、高度比例不变', () => {
    act(() => root.render(<BrandMark className="logo" />));
    const bars = [...host.querySelectorAll('i')] as HTMLElement[];
    expect(bars.map(b => b.style.getPropertyValue('--h'))).toEqual(['100%', '70%', '46%', '82%']);
  });
});

describe('ColorPick', () => {
  it('取色器里拖动后确定，回调拿到选中的颜色（拖动时的 input 不会把值还原成旧颜色）', () => {
    const onPick = vi.fn();
    act(() => root.render(<ColorPick value="#F43F5E" custom={false} onPick={onPick} />));
    const el = host.querySelector('input[type=color]') as HTMLInputElement;
    pickInDialog(el, '#30c010', '#27c412');
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(onPick).toHaveBeenCalledWith('#27C412');
  });

  it('颜色从外面改了，取色器的初始颜色跟着变', () => {
    const onPick = vi.fn();
    act(() => root.render(<ColorPick value="#F43F5E" custom={false} onPick={onPick} />));
    const el = host.querySelector('input[type=color]') as HTMLInputElement;
    pickInDialog(el, '#27c412');
    act(() => root.render(<ColorPick value="#3B82F6" custom={false} onPick={onPick} />));
    expect(el.value).toBe('#3b82f6');
    act(() => root.render(<ColorPick value="#27C412" custom onPick={onPick} />));
    expect(el.value).toBe('#27c412');
    expect(host.querySelector('label')?.classList.contains('on')).toBe(true);
  });
});
