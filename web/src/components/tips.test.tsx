import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { dismissToast, toast, useUI } from '../store/ui';
import { FloatTips, InfoTip, Tip, TipHost } from './bits';
import { Toasts } from './overlays/Toasts';

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
  document.getElementById('board-tips')?.remove();
  useUI.setState({ toasts: [] });
});

/** 提示宿主（App 里挂一份）：提示浮层本身始终对读屏隐藏 */
const withHost = (ui: React.ReactNode) => (
  <>
    {ui}
    <TipHost />
  </>
);

const note = (id: string | null) => (id ? document.getElementById(id) : null);

describe('悬停提示的可读名称（default/main#314）', () => {
  it('data-tip 照旧画提示，同时用 aria-describedby 指到一个隐藏、对读屏隐藏的浮层', () => {
    act(() => root.render(withHost(<Tip text="删除"><button type="button" aria-label="删除 #9">×</button></Tip>)));
    const btn = host.querySelector('button')!;
    expect(btn.getAttribute('data-tip')).toBe('删除');
    const id = btn.getAttribute('aria-describedby');
    expect(id).toBeTruthy();
    const span = note(id)!;
    expect(span.textContent).toBe('删除');
    expect(span.getAttribute('aria-hidden')).toBe('true');
    expect(span.closest('#board-tips')).toBeTruthy();
    // 有名字的按钮不另造一个 aria-label
    expect(btn.getAttribute('aria-label')).toBe('删除 #9');
  });

  it('只有图标、自己没有名称的元素：提示就是它的名称', () => {
    act(() => root.render(withHost(<Tip text="永久删除"><button type="button"><svg /></button></Tip>)));
    expect(host.querySelector('button')!.getAttribute('aria-label')).toBe('永久删除');
  });

  it('同一段提示共用一个浮层；元素不在了浮层跟着摘掉', () => {
    act(() =>
      root.render(
        withHost(
          <>
            <Tip text="复制"><button type="button" aria-label="复制链接">a</button></Tip>
            <Tip text="复制"><button type="button" aria-label="复制提示词">b</button></Tip>
          </>,
        ),
      ),
    );
    const [a, b] = [...host.querySelectorAll('button')];
    expect(a.getAttribute('aria-describedby')).toBe(b.getAttribute('aria-describedby'));
    expect(document.querySelectorAll('#board-tips span')).toHaveLength(1);
    act(() => root.render(withHost(<Tip text="复制"><button type="button" aria-label="复制链接">a</button></Tip>)));
    return new Promise(r => setTimeout(r, 10)).then(() => {
      expect(document.querySelectorAll('#board-tips span')).toHaveLength(1); // b 没了，a 还在
      act(() => root.render(withHost(<div />)));
      return new Promise(r => setTimeout(r, 10)).then(() => {
        expect(document.querySelectorAll('#board-tips span')).toHaveLength(0);
      });
    });
  });

  it('提示文字变了，浮层和 aria-describedby 跟着变（剩下的旧浮层被摘掉）', () => {
    act(() => root.render(withHost(<Tip text="勾选 #9"><button type="button" aria-label="9">9</button></Tip>)));
    const first = host.querySelector('button')!.getAttribute('aria-describedby');
    act(() => root.render(withHost(<Tip text="取消勾选 #9"><button type="button" aria-label="9">9</button></Tip>)));
    const second = host.querySelector('button')!.getAttribute('aria-describedby');
    expect(second).not.toBe(first);
    expect(note(second)!.textContent).toBe('取消勾选 #9');
  });

  it('没有宿主时（单独渲染一个组件）照样画提示、不报错', () => {
    act(() => root.render(<Tip text="删除"><button type="button">×</button></Tip>));
    expect(host.querySelector('button')!.getAttribute('data-tip')).toBe('删除');
    expect(host.querySelector('button')!.getAttribute('aria-describedby')).toBeNull();
  });

  it('提示文字为空（按钮能点了、这话已经用不上）时不补名称和浮层', () => {
    act(() => root.render(withHost(<Tip text=""><button type="button" aria-label="合并成一条">合并</button></Tip>)));
    const btn = host.querySelector('button')!;
    expect(btn.getAttribute('aria-describedby')).toBeNull();
    expect(document.querySelectorAll('#board-tips span')).toHaveLength(0);
  });

  it('说明小图标（InfoTip）的名称是那段说明，不被提示改掉', () => {
    act(() => root.render(withHost(<InfoTip text="到点在这块白板能接的机器里随机挑一台" />)));
    const el = host.querySelector('.info-tip') as HTMLElement;
    expect(el.getAttribute('aria-label')).toBe('到点在这块白板能接的机器里随机挑一台');
    expect(el.getAttribute('data-tip')).toBe('到点在这块白板能接的机器里随机挑一台');
    expect(el.querySelector('svg')).toBeTruthy();
  });
});

describe('提示条（toast）的可读名称', () => {
  it('成功用 role=status，失败用 role=alert，容器是 aria-live=polite', () => {
    act(() => root.render(<Toasts />));
    act(() => {
      toast({ type: 'success', title: '已创建 #20' });
      toast({ type: 'error', title: '删除失败', text: '卡片不存在' });
    });
    const wrap = host.querySelector('.toasts')!;
    expect(wrap.getAttribute('aria-live')).toBe('polite');
    const items = [...host.querySelectorAll('.toast')];
    expect(items.map(i => i.getAttribute('role'))).toEqual(['status', 'alert']);
    expect(items[0].textContent).toContain('已创建 #20');
    expect(items[1].textContent).toContain('删除失败');
    expect(items[1].textContent).toContain('卡片不存在');
  });

  it('关掉、自动消失之后从列表里拿掉（读屏不会读到已经不存在的提示）', () => {
    act(() => root.render(<Toasts />));
    let done: () => void = () => {};
    act(() => {
      done = toast({ type: 'info', title: '已标为已读' });
    });
    expect(host.querySelectorAll('.toast')).toHaveLength(1);
    act(() => done());
    expect(host.querySelectorAll('.toast')).toHaveLength(0);
    expect(useUI.getState().toasts).toHaveLength(0);
    expect(dismissToast).toBeTypeOf('function');
  });

  it('有「撤销」按钮的提示：按钮在提示条里，读屏能在提示里找到它', () => {
    act(() => root.render(<Toasts />));
    act(() => {
      toast({ type: 'success', title: '已整理到「待办」', action: { label: '撤销', fn: () => {} } });
    });
    const item = host.querySelector('.toast')!;
    const btn = item.querySelector('.toast-action button')!;
    expect(btn.textContent).toBe('撤销');
    expect(item.contains(btn)).toBe(true);
  });
});

describe('全站提示浮层（default/main#360）', () => {
  const originalMatchMedia = window.matchMedia;
  let width: ReturnType<typeof vi.spyOn>;
  let height: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.useFakeTimers();
    window.matchMedia = ((query: string) =>
      ({
        matches: query === '(hover: hover)',
        media: query,
        onchange: null,
        addEventListener() {},
        removeEventListener() {},
        addListener() {},
        removeListener() {},
        dispatchEvent: () => false,
      }) as MediaQueryList) as typeof window.matchMedia;
    width = vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockImplementation(function (this: HTMLElement) {
      return this.classList.contains('float-tip') ? 96 : 0;
    });
    height = vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockImplementation(function (this: HTMLElement) {
      return this.classList.contains('float-tip') ? 24 : 0;
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    width.mockRestore();
    height.mockRestore();
    window.matchMedia = originalMatchMedia;
    document.body.classList.remove('is-dragging', 'tile-moving');
    document.querySelectorAll('body > button[data-tip]').forEach(el => el.remove());
  });

  const trigger = (tip = '删除这张卡') => {
    const el = document.createElement('button');
    el.type = 'button';
    el.dataset.tip = tip;
    el.getBoundingClientRect = () =>
      ({ x: 100, y: 100, top: 100, right: 140, bottom: 120, left: 100, width: 40, height: 20, toJSON: () => ({}) }) as DOMRect;
    document.body.append(el);
    return el;
  };

  const over = (el: HTMLElement) => {
    act(() => {
      el.dispatchEvent(new MouseEvent('pointerover', { bubbles: true }));
      vi.advanceTimersByTime(350);
    });
  };

  it('任意 data-tip 都挂到 body 的同一个顶层浮层里，不再受组件自身的叠层和 overflow 影响', () => {
    act(() => root.render(<FloatTips />));
    const btn = trigger();
    over(btn);
    const tip = document.querySelector('.float-tip') as HTMLElement;
    expect(tip.parentElement).toBe(document.body);
    expect(tip.textContent).toBe('删除这张卡');
    expect(tip.getAttribute('aria-hidden')).toBe('true');
    expect(tip.getAttribute('popover')).toBe('manual');
    expect(tip.dataset.placed).toBe('true');
    expect(tip.dataset.side).toBe('bottom');
    expect(tip.style.left).toBe('72px');
    expect(tip.style.top).toBe('128px');
  });

  it('靠近视口底部时翻到触发器上方', () => {
    act(() => root.render(<FloatTips />));
    const btn = trigger();
    btn.getBoundingClientRect = () =>
      ({ x: 100, y: 730, top: 730, right: 140, bottom: 750, left: 100, width: 40, height: 20, toJSON: () => ({}) }) as DOMRect;
    over(btn);
    const tip = document.querySelector('.float-tip') as HTMLElement;
    expect(tip.dataset.side).toBe('top');
    expect(tip.style.top).toBe('698px');
  });

  it('滚动、按下和 Esc 都收起，不在抽屉或弹窗打开后残留', () => {
    act(() => root.render(<FloatTips />));
    const btn = trigger();
    over(btn);
    expect(document.querySelector('.float-tip')).toBeTruthy();
    act(() => document.dispatchEvent(new Event('scroll', { bubbles: true })));
    expect(document.querySelector('.float-tip')).toBeNull();

    over(btn);
    act(() => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(document.querySelector('.float-tip')).toBeNull();
  });

  it('触发器被拿掉（切页、弹窗换内容）后浮层也跟着收起', async () => {
    act(() => root.render(<FloatTips />));
    const btn = trigger();
    over(btn);
    expect(document.querySelector('.float-tip')).toBeTruthy();
    await act(async () => {
      btn.remove();
      await Promise.resolve();
    });
    expect(document.querySelector('.float-tip')).toBeNull();
  });

  it('拖动、移动磁贴时不弹提示；无 hover 的触屏保持原交互，也不启用浮层', () => {
    act(() => root.render(<FloatTips />));
    const btn = trigger();
    document.body.classList.add('is-dragging');
    over(btn);
    expect(document.querySelector('.float-tip')).toBeNull();
    document.body.classList.remove('is-dragging');

    act(() => root.render(<div />));
    window.matchMedia = ((query: string) =>
      ({
        matches: query !== '(hover: hover)',
        media: query,
        onchange: null,
        addEventListener() {},
        removeEventListener() {},
        addListener() {},
        removeListener() {},
        dispatchEvent: () => false,
      }) as MediaQueryList) as typeof window.matchMedia;
    act(() => root.render(<FloatTips />));
    over(btn);
    expect(document.querySelector('.float-tip')).toBeNull();
  });
});
