import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CheckState } from '../lib/update';
import { FLASH_MS, useFlash } from './update-card';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  vi.useFakeTimers();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

function Probe({ kind }: { kind: CheckState['kind'] }) {
  return <span>{useFlash(kind) ?? 'none'}</span>;
}
const show = (kind: CheckState['kind']) => act(() => root.render(<Probe kind={kind} />));

describe('useFlash（检查结果在按钮上停 5 秒）', () => {
  it('检查完显示结果，5 秒后变回来', () => {
    show('idle');
    show('checking');
    show('up-to-date');
    expect(host.textContent).toBe('up-to-date');
    act(() => vi.advanceTimersByTime(FLASH_MS - 1));
    expect(host.textContent).toBe('up-to-date');
    act(() => vi.advanceTimersByTime(1));
    expect(host.textContent).toBe('none');
  });

  it('失败同样停 5 秒；再查一次马上清掉', () => {
    show('checking');
    show('failed');
    expect(host.textContent).toBe('failed');
    show('checking');
    expect(host.textContent).toBe('none');
  });

  it('打开页面时已有的结果、发现新版本都不闪', () => {
    show('up-to-date');
    expect(host.textContent).toBe('none');
    show('checking');
    show('available');
    expect(host.textContent).toBe('none');
  });
});
