import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ColorPick } from './bits';

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
