import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readiness } from '../../lib/ready';
import type { HostAgent, HostInfo, Workspace } from '../../lib/types';
import { useUI } from '../../store/ui';
import { Compare, ReadySection } from './Ready';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  useUI.setState({ host: null });
});

const agent = (id: string, x: Partial<HostAgent> = {}): HostAgent => ({ id, name: id, version: '1', found: true, env: 'login', ...x });
const machine = (x: Partial<HostInfo> = {}): HostInfo => ({
  id: 1,
  name: 'mac',
  os: 'macOS',
  version: '0.1.19',
  agents: [agent('claude')],
  envs: [],
  dirs: [{ board: 'default/main', path: '~/repo', status: 'ok' }],
  max: 2,
  paused: false,
  agent: 'claude',
  rules: [],
  fallback: null,
  cronOn: false,
  cron: [],
  round: false,
  online: true,
  firstAt: 0,
  beatAt: Date.now(),
  offAt: null,
  ...x,
});

describe('机器详情的就绪清单', () => {
  it('有没通过的项时默认展开，点按钮打开对应的设置', () => {
    const go = vi.fn();
    const h = machine({ dirs: [], agents: [agent('claude', { no_skill: '~/.claude/skills/board-skills' })] });
    act(() => root.render(<ReadySection checks={readiness(h, Date.now())} go={go} i={1} />));
    expect(host.querySelector('.hp-ready-n')!.textContent).toBe('2/4 项通过');
    const bad = [...host.querySelectorAll('.hp-ready-list li.bad')];
    expect(bad.map(li => li.querySelector('b')!.textContent)).toEqual(['每块白板的目录和仓库正常', 'agent 看得到 board-skills']);
    act(() => (bad[0].querySelector('button') as HTMLButtonElement).click());
    act(() => (bad[1].querySelector('button') as HTMLButtonElement).click());
    expect(go.mock.calls).toEqual([['dirs'], ['agents']]);
  });

  it('全部通过时收起，点标题展开', () => {
    act(() => root.render(<ReadySection checks={readiness(machine(), Date.now())} go={() => {}} i={1} />));
    expect(host.querySelector('.hp-ready-n')!.textContent).toBe('4/4 项通过');
    expect(host.querySelector('.hp-ready-list')).toBeNull();
    act(() => (host.querySelector('.hp-fold-btn') as HTMLButtonElement).click());
    expect(host.querySelectorAll('.hp-ready-list li.ok').length).toBe(4);
  });
});

describe('对比视图', () => {
  it('每台机器一行，claude 版本不同的格子标出来，点机器名打开详情', () => {
    const a = machine({ id: 1, name: 'a', agents: [agent('claude', { version: '2.1.3' })] });
    const b = machine({ id: 2, name: 'b', agents: [agent('claude', { version: '2.1.5' })] });
    const ws = { hosts: [a, b], dispatches: [] } as unknown as Workspace;
    act(() => root.render(<Compare hosts={[a, b]} ws={ws} now={Date.now()} server={null} />));
    const rows = [...host.querySelectorAll('tbody tr')];
    expect(rows.map(r => r.querySelector('th')!.textContent)).toEqual(['a', 'b']);
    const diff = [...host.querySelectorAll('td.diff')].map(td => td.textContent);
    expect(diff).toEqual(['2.1.3', '2.1.5']);
    act(() => (rows[1].querySelector('th button') as HTMLButtonElement).click());
    expect(useUI.getState().host).toBe('b');
  });
});
