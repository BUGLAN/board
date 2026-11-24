import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { workspace } from '../../../test/fixtures';
import type { Profile } from '../../lib/types';
import { useData } from '../../store/data';
import { useUI } from '../../store/ui';
import { openProfile, ProfileForm } from './bits';

const mocked = vi.hoisted(() => ({
  deleteProfile: vi.fn(async (_id: string) => ({ ok: true, rules: 2 })),
}));

vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return {
    ...real,
    api: {
      ...real.api,
      providers: async () => ({ providers: [], meta: [] }),
      deleteProfile: (id: string) => mocked.deleteProfile(id),
    },
  };
});

// 写队列和刷新不在这里测：直接执行请求
vi.mock('../../store/data', async orig => {
  const real = await orig<typeof import('../../store/data')>();
  return { ...real, optimistic: (_patch: unknown, req: () => Promise<unknown>) => req() };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const profile: Profile = { id: 'p1', name: '修复', agent: 'codex', provider: '', model: '', effort: '', note: '', only: [] };

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  mocked.deleteProfile.mockClear();
  useUI.setState({ toasts: [] });
  useData.setState({
    ws: workspace({
      sched: {
        minute: 0,
        rules: [{ op: 'any', labels: [], profile: 'p1' }],
        profiles: [profile],
      },
    } as never),
    path: 'p/a',
  });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  useData.setState({ ws: null, board: null });
});

describe('角色表单', () => {
  it('删除角色后提示同时删掉了几条规则', async () => {
    await act(async () => {
      openProfile(profile);
      root.render(<ProfileForm />);
    });
    const remove = [...document.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent?.includes('删除'))!;
    await act(async () => remove.click());
    expect(mocked.deleteProfile).toHaveBeenCalledWith('p1');
    expect(useUI.getState().toasts.at(-1)).toMatchObject({
      type: 'success',
      title: '已删除角色「修复」',
      text: '同时删除 2 条规则',
    });
  });
});
