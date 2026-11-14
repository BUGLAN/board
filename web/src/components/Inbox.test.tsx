import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Check } from './Inbox';

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

const box = () => host.querySelector('.ib-check') as HTMLElement;

describe('未整理的勾选框（default/main#315）', () => {
  it('是一个带名称、带勾选状态的勾选框，不再是 `.ib-check` 的空盒子里塞一个没有名字的 input', () => {
    act(() => root.render(<Check on={false} label="勾选 #9 调研：一个仓库是否需要多块白板" onToggle={() => {}} />));
    const el = box();
    expect(el.getAttribute('role')).toBe('checkbox');
    expect(el.tagName).toBe('BUTTON');
    expect(el.getAttribute('aria-checked')).toBe('false');
    expect(el.getAttribute('aria-label')).toBe('勾选 #9 调研：一个仓库是否需要多块白板');
    expect(el.querySelector('input')).toBeNull();
  });

  it('勾上、部分选中各自报出来', () => {
    act(() => root.render(<Check on label="取消勾选 #9" onToggle={() => {}} />));
    expect(box().getAttribute('aria-checked')).toBe('true');
    act(() => root.render(<Check on={false} some label="取消勾选「这一周」" onToggle={() => {}} />));
    expect(box().getAttribute('aria-checked')).toBe('mixed');
    act(() => root.render(<Check on some label="取消勾选「这一周」" onToggle={() => {}} />));
    expect(box().getAttribute('aria-checked')).toBe('mixed');
  });

  it('点一下切换，按住 Shift 时把 shiftKey 带出去', () => {
    const onToggle = vi.fn();
    act(() => root.render(<Check on={false} label="勾选 #9" onToggle={onToggle} />));
    act(() => box().click());
    expect(onToggle).toHaveBeenLastCalledWith(false);
    act(() => box().dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true })));
    expect(onToggle).toHaveBeenLastCalledWith(true);
  });

  it('空格、回车都能切换（空格不滚页面）', () => {
    const onToggle = vi.fn();
    act(() => root.render(<Check on={false} label="勾选 #9" onToggle={onToggle} />));
    const space = new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true });
    act(() => {
      box().dispatchEvent(space);
    });
    expect(space.defaultPrevented).toBe(true);
    expect(onToggle).toHaveBeenCalledTimes(1);
    act(() => {
      box().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
      box().click();
    });
    expect(onToggle).toHaveBeenCalledTimes(2);
  });

  it('点勾选框不会顺便展开这一行', () => {
    const onToggle = vi.fn();
    const onRow = vi.fn();
    act(() =>
      root.render(
        <div onClick={onRow}>
          <Check on={false} label="勾选 #9" onToggle={onToggle} />
        </div>,
      ),
    );
    act(() => box().click());
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(onRow).not.toHaveBeenCalled();
  });
});
