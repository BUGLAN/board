import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { workspace } from '../../../test/fixtures';
import type { HostInfo } from '../../lib/types';
import { useData } from '../../store/data';
import { settleConfirm, useUI } from '../../store/ui';
import { HostSettings } from './HostSettings';

const mocked = vi.hoisted(() => ({
  removeHost: vi.fn(async (_name: string, _revoke: boolean) => ({ ok: true, revoked: 42 })),
}));

vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return {
    ...real,
    api: {
      ...real.api,
      removeHost: (name: string, revoke: boolean) => mocked.removeHost(name, revoke),
      skills: async () => [],
      mcp: async () => [],
      git: async () => ({ cred: null }),
    },
  };
});

// 写队列和刷新不在这里测：直接执行请求
vi.mock('../../store/data', async orig => {
  const real = await orig<typeof import('../../store/data')>();
  return { ...real, optimistic: (_patch: unknown, req: () => Promise<unknown>) => req() };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const machine: HostInfo = {
  id: 1,
  name: 'win2',
  os: 'Windows 11 Pro',
  version: '0.1.54',
  agents: [],
  dirs: [],
  max: 4,
  paused: false,
  agent: 'codex',
  round: false,
  online: true,
  beatAt: Date.now(),
  offAt: null,
  token: { id: 42, name: 'win2 的 board host', hint: 'bd_1234', lastUsedAt: null, shared: [] },
};

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  mocked.removeHost.mockClear();
  useUI.setState({ toasts: [], confirm: null });
  useData.setState({ ws: workspace({ hosts: [machine] }), path: 'p/a' });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  useData.setState({ ws: null, board: null });
});

describe('机器设置', () => {
  it('移除机器后按服务端返回的令牌 ID 提示已吊销', async () => {
    await act(async () =>
      root.render(<HostSettings h={machine} save={() => {}} sync={{}} now={Date.now()} server={null} behind={false} running={0} close={() => {}} />),
    );
    const remove = [...host.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent?.trim() === '移除')!;
    await act(async () => remove.click());
    const req = useUI.getState().confirm!;
    expect(req.title).toContain('移除 win2');
    req.check!.box.checked = true;
    await act(async () => settleConfirm(true));
    expect(mocked.removeHost).toHaveBeenCalledWith('win2', true);
    expect(useUI.getState().toasts.at(-1)).toMatchObject({
      type: 'success',
      title: '已移除 win2',
      text: '已吊销令牌 #42',
    });
  });
});
