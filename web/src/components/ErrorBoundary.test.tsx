import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { componentStack, crashReport, ErrorBoundary } from './ErrorBoundary';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const CRASH = "Cannot read properties of undefined (reading 'color')";

/** 线上那张截图：渲染时崩在 .color 上 */
function Boom(): never {
  throw new Error(CRASH);
}

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  // React 出错时会往控制台打一份，测试里不关心
  vi.spyOn(console, 'error').mockImplementation(() => {});
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

const shown = () => [...host.querySelectorAll('pre')].map(p => p.textContent);
const button = (text: string) => [...host.querySelectorAll('button')].find(b => b.textContent === text);

describe('componentStack（错误页上只留前几行）', () => {
  it('去掉空行，留前 8 行', () => {
    const info = Array.from({ length: 12 }, (_, i) => `    at Comp${i} (web/src/x.tsx:1:1)`).join('\n\n');
    const out = componentStack(info);
    expect(out.split('\n')).toHaveLength(8);
    expect(out.split('\n')[0]).toBe('at Comp0 (web/src/x.tsx:1:1)');
  });
  it('没有组件栈时为空，不画空的块', () => {
    expect(componentStack(null)).toBe('');
    expect(componentStack('')).toBe('');
  });
});

describe('crashReport（复制给维护者的内容）', () => {
  it('原因和组件栈放在一起', () => {
    expect(crashReport(new Error(CRASH), 'at SessionModal')).toBe(`原因：${CRASH}\n\n组件：\nat SessionModal`);
  });
  it('没有组件栈时只有原因', () => {
    expect(crashReport(new Error(CRASH), '')).toBe(`原因：${CRASH}`);
    expect(crashReport(null, 'at X')).toBe('');
  });
});

describe('ErrorBoundary（界面出错时不整页白屏）', () => {
  it('没出错时照常画子组件', () => {
    act(() => root.render(<ErrorBoundary><p>正常</p></ErrorBoundary>));
    expect(host.querySelector('.crash')).toBeNull();
    expect(host.textContent).toBe('正常');
  });

  it('出错时画原因，不画子组件', () => {
    act(() => root.render(<ErrorBoundary><Boom /></ErrorBoundary>));
    expect(host.querySelector('.crash')).not.toBeNull();
    expect(shown()[0]).toBe(CRASH);
    expect(button('重新加载')).toBeTruthy();
  });

  it('显示出错组件的栈，并和原因一起复制', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    act(() => root.render(<ErrorBoundary><Boom /></ErrorBoundary>));
    const stack = host.querySelector('.crash-stack')?.textContent ?? '';
    // 用 React 自己报告的组件栈：至少有出错的组件那一帧
    expect(stack).toContain('Boom');
    expect([...host.querySelectorAll('pre')]).toHaveLength(2);
    const copy = button('复制原因和组件')!;
    await act(async () => copy.click());
    expect(writeText).toHaveBeenCalledTimes(1);
    const text = writeText.mock.calls[0][0] as string;
    expect(text).toContain(`原因：${CRASH}`);
    expect(text).toContain(stack);
    expect(button('已复制')).toBeTruthy();
  });
});
