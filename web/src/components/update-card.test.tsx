import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CheckState } from '../lib/update';
import { useUpdate } from '../lib/update-runner';
import { settleConfirm, useUI } from '../store/ui';
import { FLASH_MS, UpdateCard, useFlash } from './update-card';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  useUI.setState({ confirm: null });
  useUpdate.setState({ check: { kind: 'idle' }, install: { kind: 'idle' } });
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

describe('UpdateCard 的自动安装等待和手动确认', () => {
  beforeEach(() => {
    useUpdate.setState({
      check: {
        kind: 'available',
        version: '0.1.55',
        result: { current: '0.1.54', version: '0.1.55', notes: '', date: null },
      },
      install: { kind: 'waiting', version: '0.1.55', running: 2 },
    });
  });

  it('有派发在跑时显示等待状态和数量', () => {
    act(() => root.render(<UpdateCard />));
    expect(host.textContent).toContain('等当前派发结束后安装');
    expect(host.textContent).toContain('这台电脑还有 2 个派发在跑');
    expect(host.textContent).toContain('不会停掉 board host');
  });

  it('手动点「现在安装」仍走确认，并写明会结束正在跑的派发', async () => {
    act(() => root.render(<UpdateCard />));
    const button = [...host.querySelectorAll('button')].find(b => b.textContent?.includes('现在安装'));
    expect(button).toBeTruthy();
    await act(async () => {
      button!.click();
    });
    const req = useUI.getState().confirm;
    expect(req?.title).toBe('下载并安装 0.1.55？');
    expect(req?.text).toContain('结束这台机器上正在跑的派发');
    expect(req?.tone).toBe('warn');
    await act(async () => settleConfirm(false));
  });
});
