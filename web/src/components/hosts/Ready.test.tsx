import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { HostAgent, HostInfo, Workspace } from '../../lib/types';
import { useUI } from '../../store/ui';
import { Compare } from './Ready';

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
  workspace: { path: '~' },
  dirs: [{ board: 'default/main', path: '~/default/main', status: 'ok' }],
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
