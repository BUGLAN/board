import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { snapshot, workspace } from '../../../test/fixtures';
import { agentMark } from '../../lib/hosts';
import type { Dispatch, HostInfo } from '../../lib/types';
import { useData } from '../../store/data';
import { useUI } from '../../store/ui';
import { CardDispatchBadge, DispatchDialog } from './Dispatch';

const { dispatchApi, batchApi } = vi.hoisted(() => ({ dispatchApi: vi.fn(), batchApi: vi.fn() }));

vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return {
    ...real,
    api: {
      ...real.api,
      dispatch: (...args: [string, number, string, string?]) => dispatchApi(...args),
      dispatchBatch: (...args: unknown[]) => batchApi(...args),
      providers: async () => ({ providers: [], meta: [] }),
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

const NOW = new Date(2026, 10, 16, 9).getTime();

const dispatch = (agent: string, agentLabel: string): Dispatch => ({
  id: 7,
  board: 'default/main',
  card: 404,
  title: '修一下派发排队行',
  hostId: 2,
  host: 'win2',
  agent,
  agentLabel,
  name: null,
  source: 'manual',
  by: { kind: 'human', name: '人' },
  state: 'queued',
  result: null,
  createdAt: NOW,
  startedAt: null,
  endedAt: null,
  stopBy: null,
  beatAt: null,
});

const machine: HostInfo = {
  id: 1,
  name: 'win-preview',
  os: 'Windows',
  version: '0.1.54',
  agents: [{ id: 'codex', name: 'Codex', version: '1', found: true }],
  dirs: [],
  max: 2,
  paused: false,
  agent: 'codex',
  round: false,
  online: true,
  beatAt: 0,
  offAt: null,
};

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

const render = (d: Dispatch) => {
  useData.setState({
    ws: { rev: 1, now: NOW, projects: [], dispatches: [d] },
    path: 'default/main',
    cards: {},
    saving: {},
    pending: {},
  } as never);
  act(() => root.render(<CardDispatchBadge card={d.card} />));
};

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  dispatchApi.mockReset();
  batchApi.mockReset();
  useUI.setState({ dispatchOf: null, toasts: [] });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
  useUI.setState({ dispatchOf: null });
  useData.setState({ ws: null, board: null });
});

describe('卡片上的派发排队角标', () => {
  it('认识的 agent 画它自己的 logo，不再是通用派发图标', () => {
    render(dispatch('codex', 'Codex'));
    const line = host.querySelector('.card-where') as HTMLElement;
    expect(line.textContent).toBe('→ win2');
    expect(line.title).toContain('排队 → win2 · Codex');
    expect(line.querySelector('img.hp-logo')?.getAttribute('src')).toBe(agentMark('codex').icon);
    // 通用派发图标是 <svg>；换了 logo 之后这一行没有它了
    expect(line.querySelector('svg')).toBeNull();
  });

  it('单色 logo（dsh 的鲸鱼）用遮罩画，深浅主题都跟 --agent-ink', () => {
    render(dispatch('dsh', 'DeepSeek Harness'));
    const line = host.querySelector('.card-where') as HTMLElement;
    const art = line.querySelector('.hp-logo.logo-mono') as HTMLElement;
    expect(art.style.maskImage).toContain(agentMark('dsh').icon!);
    expect(line.querySelector('img')).toBeNull();
  });

  it('认不出的 agent 退回两个字母，不出现空图标', () => {
    render(dispatch('alpha', 'Alpha'));
    const line = host.querySelector('.card-where') as HTMLElement;
    const mark = line.querySelector('b.hp-mini') as HTMLElement;
    expect(mark.textContent).toBe('AL');
    expect(line.querySelector('img, .logo-mono')).toBeNull();
    expect(line.title).toContain('排队 → win2 · Alpha');
  });
});

describe('批量派送对话框', () => {
  it('显示张数和跳过原因；一个请求派整批，失败的不影响别的，并汇总结果', async () => {
    const active = { ...dispatch('codex', 'Codex'), board: 'p/a', card: 1 };
    useData.setState({
      ws: workspace({ hosts: [machine], dispatches: [active] }),
      path: 'p/a',
      board: snapshot('p/a', { todo: [1], doing: [2], review: [3, 4] }),
    } as never);
    useUI.setState({ dispatchOf: { cards: [1, 2, 3, 4] } });
    batchApi.mockImplementation(async (_board: string, cards: number[]) => ({
      ok: cards.filter(c => c !== 3).map(card => ({ ...dispatch('codex', 'Codex'), id: 8, board: 'p/a', card, hostId: machine.id, host: machine.name })),
      failed: [{ id: 3, code: 'BOARD_NOT_ON_HOST', message: '机器没有这块白板的目录' }],
      rev: 2,
    }));

    await act(async () => root.render(<DispatchDialog />));
    expect(host.querySelector('h3')?.textContent).toBe('派发 4 张卡片');
    expect(host.querySelector('.dp-sub')?.textContent).toBe('共 4 张卡片，按会话分组排队');
    expect(host.querySelector('.dp-plan')?.textContent).toBe('可派发 2 张；跳过 2 张：#1 已有排队或在跑的派发；#2 在锁定状态「doing」');
    const button = host.querySelector<HTMLButtonElement>('.dialog-actions .btn.brand')!;
    expect(button.textContent).toBe('派送 2 张');

    await act(async () => {
      button.click();
      await new Promise(resolve => setTimeout(resolve, 0));
    });

    expect(batchApi.mock.calls).toEqual([['p/a', [3, 4], 'win-preview', 'codex']]);
    expect(dispatchApi).not.toHaveBeenCalled();
    expect(useUI.getState().dispatchOf).toBeNull();
    expect(useUI.getState().toasts.at(-1)).toMatchObject({
      type: 'error',
      title: '成功 1 张 · 跳过 2 张 · 失败 1 张',
    });
    expect(useUI.getState().toasts.at(-1)?.text).toContain('失败：#3 机器没有这块白板的目录');
    expect(useUI.getState().toasts.at(-1)?.text).toContain('跳过：#1 已有排队或在跑的派发；#2 在锁定状态「doing」');
  });
});

describe('有角色时的批量派送', () => {
  it('打开按角色派的面板（和单卡同一个），不是选机器、选 agent 的旧对话框', async () => {
    useData.setState({
      ws: workspace({
        hosts: [{ ...machine, fits: [{ profile: 'p1', ok: true }] }],
        sched: { minute: 0, rules: [], profiles: [{ id: 'p1', name: 'Codex', agent: 'codex', provider: '', model: 'gpt-5.4', effort: 'high', note: '', only: [] }] },
      }),
      path: 'p/a',
      board: snapshot('p/a', { todo: [1, 2] }),
    } as never);
    useUI.setState({ dispatchOf: { cards: [1, 2] } });
    await act(async () => root.render(<DispatchDialog />));
    expect(host.querySelector('.pfd.flow h3')?.childNodes[0].textContent).toBe('派发 2 张卡片');
    expect(host.querySelector('.pfz-prof.on b')?.textContent).toBe('Codex');
    expect(host.querySelector('.dp-hosts')).toBeNull();
  });
});
