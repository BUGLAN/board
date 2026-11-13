import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ErrorBoundary, crashText, STACK_LINES, stackLines } from './ErrorBoundary';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** 和线上一样：组件里对 undefined 取 .color */
function Boom({ what }: { what: string }): React.ReactNode {
  const row = (undefined as unknown as { color: string })!;
  return <span>{`${what}${row.color}`}</span>;
}

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
const written: string[] = [];

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  // 控制台里的那一行只是记录，这里不让它刷屏
  vi.spyOn(console, 'error').mockImplementation(() => {});
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (t: string) => void written.push(t) } });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
  written.length = 0;
});

/** 渲染出错的那一棵，拿到画出来的错误页 */
function crash() {
  act(() => root.render(<ErrorBoundary><Boom what="整页" /></ErrorBoundary>));
  return host.querySelector('.crash') as HTMLElement;
}

describe('错误页', () => {
  it('画出原因、出错的组件前几行，还有重新加载和复制', () => {
    const el = crash();
    expect(el.querySelector('pre')!.textContent).toContain("reading 'color'");
    const stack = el.querySelector('.crash-stack')!;
    // 最深的在前：Boom 就是出错的那个组件
    expect(stack.textContent!.split('\n')[0]).toContain('Boom');
    expect(stack.textContent!.split('\n').length).toBeLessThanOrEqual(STACK_LINES);
    expect(el.textContent).toContain('重新加载');
    expect(el.textContent).toContain('复制原因和组件');
  });

  it('一键复制原因和组件栈，和界面上看到的一致；复制完按钮短暂写「已复制」', async () => {
    const el = crash();
    const reason = el.querySelector('pre')!.textContent;
    const stack = [...el.querySelectorAll('.crash-stack')].map(n => n.textContent).join('');
    const btn = [...el.querySelectorAll('button')].find(b => b.textContent === '复制原因和组件') as HTMLButtonElement;
    await act(async () => {
      btn.click();
    });
    expect(written).toHaveLength(1);
    expect(written[0]).toContain(reason!);
    expect(written[0]).toContain('组件：');
    expect(written[0]).toContain('Boom');
    expect(written[0]).toContain(stack.split('\n')[0]);
    expect(btn.textContent).toBe('已复制');
  });

  it('没出错时照常画孩子', () => {
    act(() => root.render(<ErrorBoundary><span>好的</span></ErrorBoundary>));
    expect(host.textContent).toBe('好的');
    expect(host.querySelector('.crash')).toBeNull();
  });
});

describe('组件栈和复制的文字', () => {
  it('只取前几行，去掉空行和首尾空白', () => {
    const raw = '\n    at Boom (ErrorBoundary.test.tsx:9:24)\n\n    at ErrorBoundary\n    at ReactDOMRoot\n    at a\n    at b\n    at c\n';
    expect(stackLines(raw)).toEqual(['at Boom (ErrorBoundary.test.tsx:9:24)', 'at ErrorBoundary', 'at ReactDOMRoot', 'at a', 'at b', 'at c']);
    expect(stackLines('')).toEqual([]);
    expect(stackLines(null)).toEqual([]);
  });

  it('没有组件栈时写明不知道，不写空的一段', () => {
    const t = crashText(new Error('Cannot read properties of undefined (reading \'color\')'), '');
    expect(t).toContain("原因：Cannot read properties of undefined (reading 'color')");
    expect(t).toContain('组件：不知道');
    expect(t).not.toContain('组件：\n');
  });
});
