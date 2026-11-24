import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HostInfo } from '../../lib/types';
import { snapshot, workspace } from '../../../test/fixtures';
import { useData } from '../../store/data';
import { useUI } from '../../store/ui';
import { DispatchFlow } from './DispatchFlow';

const { cloneHostDir, dispatchApi } = vi.hoisted(() => ({
  cloneHostDir: vi.fn(async (..._a: unknown[]) => ({}) as never),
  dispatchApi: vi.fn(),
}));

vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return {
    ...real,
    api: {
      ...real.api,
      providers: async () => ({ providers: [], meta: [] }),
      cloneHostDir: (...args: unknown[]) => cloneHostDir(...args),
      dispatch: (...args: unknown[]) => dispatchApi(...args),
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
  round: false,
  online: true,
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
    await act(async () => root.render(<DispatchFlow cards={[2]} on />));
    const node = host.querySelector<HTMLElement>('.pfz-card')!;
    expect(node.textContent).toBe('规则 1');
    expect(node.querySelector('.lb')).toBeNull();
    expect(node.title).toBe('规则 1：带 bug、ui');
  });
});

describe('克隆失败的机器', () => {
  it('写明原因，并给一个「重新克隆」按钮（这一行仍然不能选）', async () => {
    await act(async () => root.render(<DispatchFlow cards={[2]} on />));
    const row = [...host.querySelectorAll<HTMLElement>('.pfz-host')].find(x => x.textContent?.includes('mac'))!;
    expect(row.textContent).toContain(BLOCK_WHY);
    const btn = row.querySelector<HTMLButtonElement>('.pfz-reclone')!;
    expect(btn.textContent).toBe('重新克隆');
    expect(btn.disabled).toBe(false);
  });

  it('点「重新克隆」会请那台机器重新克隆这块白板的目录', async () => {
    await act(async () => root.render(<DispatchFlow cards={[2]} on />));
    const btn = host.querySelector<HTMLButtonElement>('.pfz-reclone')!;
    await act(async () => {
      btn.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(cloneHostDir).toHaveBeenCalledWith('mac', 'p/a', 'git@x:p/p.git');
  });
});

describe('批量派送（多张卡片）', () => {
  // 规则 1：带 bug 的给 Codex；规则 2：带 ui 的给 Claude。#6 在锁定状态，跳过
  beforeEach(() => {
    dispatchApi.mockReset();
    useUI.setState({ dispatchOf: { cards: [2, 3, 4, 5, 6] }, toasts: [] });
    useData.setState({
      ws: workspace({
        hosts: [baseHost({ fits: [{ profile: 'p1', ok: true }, { profile: 'p2', ok: true }] })],
        sched: {
          minute: 0,
          rules: [
            { op: 'in', labels: ['bug'], profile: 'p1' },
            { op: 'in', labels: ['ui'], profile: 'p2' },
          ],
          profiles: [
            { id: 'p1', name: 'Codex', agent: 'codex', provider: '', model: 'gpt-5.4', effort: 'high', note: '', only: [] },
            { id: 'p2', name: 'Claude', agent: 'claude', provider: '', model: 'opus', effort: 'high', note: '', only: [] },
          ],
        },
      }),
      path: 'p/a',
      board: snapshot('p/a', { todo: [2, 3, 4, 5], doing: [6] }, [
        { id: 2, title: '修 bug', labels: [1] },
        { id: 3, title: '改界面', labels: [2] },
        { id: 4, title: '再改界面', labels: [2] },
        { id: 5, title: '没有标签', labels: [] },
      ]),
    } as never);
  });

  afterEach(() => {
    useUI.setState({ dispatchOf: null, toasts: [] });
  });

  it('用按角色派的面板：标题写张数、写明跳过的；卡片栏按规则分组，预选按规则交给它最多的角色', async () => {
    await act(async () => root.render(<DispatchFlow cards={[2, 3, 4, 5, 6]} on />));
    expect(host.querySelector('h3')?.childNodes[0].textContent).toBe('派发 5 张卡片');
    expect(host.querySelector('.dp-plan')?.textContent).toBe('可派发 4 张；跳过 1 张：#6 在锁定状态「doing」');
    const groups = [...host.querySelectorAll<HTMLElement>('.pfz-card')].map(n => [n.querySelector('b')?.textContent, n.querySelector('small')?.textContent]);
    expect(groups).toEqual([
      ['规则 1', '1 张'],
      ['规则 2', '2 张'],
      ['没命中规则', '1 张'],
    ]);
    expect(host.querySelector<HTMLElement>('.pfz-card')!.title).toBe('规则 1：带 bug\n#2 修 bug');
    const on = host.querySelector<HTMLElement>('.pfz-prof.on')!;
    expect(on.querySelector('b')?.textContent).toBe('Claude');
    expect([...host.querySelectorAll('.pfz-prof .pfx-tag')].map(t => t.textContent)).toEqual(['按规则 1 张', '按规则 2 张']);
    expect(host.querySelector('.pfz-foot .btn.brand')?.textContent).toBe('派送 4 张');
    // 选机器、选 agent 的旧对话框不出现
    expect(host.querySelector('.dp-hosts')).toBeNull();
  });

  it('能派的逐张带上选中的角色派发；单张失败不影响后面的，汇总成功、跳过、失败', async () => {
    dispatchApi.mockImplementation(async (board: string, card: number) => {
      if (card === 4) throw new Error('机器没有这块白板的目录');
      return { id: 100 + card, board, card, hostId: 1, host: 'win-preview', state: 'queued', profile: 'p1' };
    });
    await act(async () => root.render(<DispatchFlow cards={[2, 3, 4, 5, 6]} on />));
    // 换成 Codex、指定 win-preview
    await act(async () => [...host.querySelectorAll<HTMLButtonElement>('.pfz-prof')].find(b => b.textContent?.includes('Codex'))!.click());
    await act(async () => [...host.querySelectorAll<HTMLButtonElement>('.pfz-host:not(.pfz-any)')].find(b => b.textContent?.includes('win-preview'))!.click());
    expect(host.querySelector('.pfd-say')?.textContent).toContain('4 张派给「Codex」· win-preview');
    await act(async () => {
      host.querySelector<HTMLButtonElement>('.pfz-foot .btn.brand')!.click();
      await new Promise(resolve => setTimeout(resolve, 0));
    });
    expect(dispatchApi.mock.calls).toEqual([
      ['p/a', 2, 'win-preview', undefined, 'p1'],
      ['p/a', 3, 'win-preview', undefined, 'p1'],
      ['p/a', 4, 'win-preview', undefined, 'p1'],
      ['p/a', 5, 'win-preview', undefined, 'p1'],
    ]);
    expect(useUI.getState().dispatchOf).toBeNull();
    const t = useUI.getState().toasts.at(-1)!;
    expect(t).toMatchObject({ type: 'error', title: '成功 3 张 · 跳过 1 张 · 失败 1 张' });
    expect(t.text).toBe('派给「Codex」· win-preview\n跳过：#6 在锁定状态「doing」\n失败：#4 机器没有这块白板的目录');
  });

  it('选中的都不能派：写明没有可派发的卡片，按钮禁用', async () => {
    useUI.setState({ dispatchOf: { cards: [6, 7] } });
    useData.setState({ board: snapshot('p/a', { doing: [6, 7] }) } as never);
    await act(async () => root.render(<DispatchFlow cards={[6, 7]} on />));
    expect(host.querySelector('.dp-plan')?.textContent).toContain('没有可派发的卡片');
    expect(host.querySelector('.pfz-card')?.textContent).toBe('没有可派发的卡片');
    expect(host.querySelector<HTMLButtonElement>('.pfz-foot .btn.brand')!.disabled).toBe(true);
  });
});
