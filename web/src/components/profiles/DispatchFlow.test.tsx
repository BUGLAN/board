import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HostInfo } from '../../lib/types';
import { snapshot, workspace } from '../../../test/fixtures';
import { useData } from '../../store/data';
import { DispatchFlow } from './DispatchFlow';

const { cloneHostDir } = vi.hoisted(() => ({ cloneHostDir: vi.fn(async (..._a: unknown[]) => ({}) as never) }));

vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return {
    ...real,
    api: {
      ...real.api,
      providers: async () => ({ providers: [], meta: [] }),
      cloneHostDir: (...args: unknown[]) => cloneHostDir(...args),
    },
  };
});

vi.mock('../../store/data', async orig => {
  const real = await orig<typeof import('../../store/data')>();
  return {
    ...real,
    optimistic: async (_patch: unknown, req: () => Promise<unknown>) => req(),
    hold: (_patch: unknown, done: Promise<unknown>) => done,
    refresh: async () => undefined,
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

const baseHost = (over: Partial<HostInfo>): HostInfo => ({
  id: 1,
  name: 'win-preview',
  os: 'Windows',
  version: '0.1.50',
  agents: [{ id: 'codex', name: 'Codex', version: '1', found: true }],
  dirs: [],
  max: 2,
  paused: false,
  agent: 'codex',
  cronOn: false,
  cron: [],
  round: false,
  online: true,
  firstAt: 0,
  beatAt: 0,
  offAt: null,
  fits: [{ profile: 'p1', ok: true }],
  ...over,
});

const machine = baseHost({});

// 这台机器上 p 项目的目录克隆失败了：派发对话框里不能选，要能就地点「重新克隆」
const mac = baseHost({
  id: 2,
  name: 'mac',
  os: 'macOS',
  dirs: [{ board: 'p/a', path: '/Users/lanshe/p', status: 'missing', clone: { state: 'failed', url: 'git@x:p/p.git', message: 'Permission denied', updatedAt: 0 } }],
});

const BLOCK_WHY = '/Users/lanshe/p 还没有克隆下来：克隆失败，Permission denied';

beforeEach(() => {
  cloneHostDir.mockClear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({
    ws: workspace({
      hosts: [machine, mac],
      blocks: [{ host: 'mac', board: 'p/a', code: 'BOARD_NOT_ON_HOST', why: BLOCK_WHY }],
      sched: {
        minute: 0,
        rules: [{ op: 'in', labels: ['bug', 'ui'], profile: 'p1' }],
        profiles: [{ id: 'p1', name: 'Codex', agent: 'codex', provider: '', model: 'gpt-5.4', effort: 'high', note: '', only: [] }],
      },
    }),
    path: 'p/a',
    board: snapshot('p/a', { todo: [2] }, [{ id: 2, title: '测试派发', labels: [1, 2] }]),
  } as never);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  useData.setState({ ws: null, board: null });
});

describe('派发对话框里的规则节点', () => {
  it('只写命中的规则，不铺标签胶囊；标签仍在悬停说明里', async () => {
    await act(async () => root.render(<DispatchFlow card={2} on />));
    const node = host.querySelector<HTMLElement>('.pfz-card')!;
    expect(node.textContent).toBe('规则 1');
    expect(node.querySelector('.lb')).toBeNull();
    expect(node.title).toBe('规则 1：带 bug、ui');
  });
});

describe('克隆失败的机器', () => {
  it('写明原因，并给一个「重新克隆」按钮（这一行仍然不能选）', async () => {
    await act(async () => root.render(<DispatchFlow card={2} on />));
    const row = [...host.querySelectorAll<HTMLElement>('.pfz-host')].find(x => x.textContent?.includes('mac'))!;
    expect(row.textContent).toContain(BLOCK_WHY);
    const btn = row.querySelector<HTMLButtonElement>('.pfz-reclone')!;
    expect(btn.textContent).toBe('重新克隆');
    expect(btn.disabled).toBe(false);
  });

  it('点「重新克隆」会请那台机器重新克隆这块白板的目录', async () => {
    await act(async () => root.render(<DispatchFlow card={2} on />));
    const btn = host.querySelector<HTMLButtonElement>('.pfz-reclone')!;
    await act(async () => {
      btn.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(cloneHostDir).toHaveBeenCalledWith('mac', 'p/a', 'git@x:p/p.git');
  });
});
