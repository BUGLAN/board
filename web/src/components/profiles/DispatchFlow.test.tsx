import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HostInfo } from '../../lib/types';
import { snapshot, workspace } from '../../../test/fixtures';
import { useData } from '../../store/data';
import { DispatchFlow } from './DispatchFlow';

vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return { ...real, api: { ...real.api, providers: async () => ({ providers: [], meta: [] }) } };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

const machine: HostInfo = {
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
};

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({
    ws: workspace({
      hosts: [machine],
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
