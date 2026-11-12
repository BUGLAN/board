import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { HostAgent, HostInfo, Workspace } from '../../lib/types';
import { useUI } from '../../store/ui';
import { Rack } from './HostsPage';

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

const agent = (id: string, x: Partial<HostAgent> = {}): HostAgent => ({ id, name: id, version: '1', found: true, ...x });

const machine = (agents: HostAgent[], x: Partial<HostInfo> = {}): HostInfo => ({
  id: 1,
  name: 'win2',
  os: 'Windows 11 Pro',
  version: '0.1.19',
  agents,
  envs: [{ id: 'pwsh', name: 'Windows PowerShell' }, { id: 'git-bash', name: 'Git Bash' }] as HostInfo['envs'],
  dirs: [],
  max: 4,
  paused: false,
  agent: 'dsh',
  rules: [],
  fallback: null,
  cronOn: false,
  cron: [],
  round: false,
  unmatched: 0,
  online: true,
  firstAt: 0,
  beatAt: Date.now(),
  offAt: null,
  ...x,
});

const ws = (hosts: HostInfo[]): Workspace => ({ hosts, dispatches: [] }) as unknown as Workspace;
const show = (h: HostInfo, w: Workspace | null = ws([h])) => act(() => root.render(<Rack h={h} ws={w} now={Date.now()} index={0} />));

describe('机器磁贴底部', () => {
  it('不显示 agent 胶囊，定时领取的摘要还在', () => {
    show(machine([agent('dsh', { env: 'pwsh' }), agent('claude', { env: 'git-bash' })], { cronOn: true, cron: ['0 23 * * *'] }));
    const rack = host.querySelector('.hp-rack');
    expect(rack).not.toBeNull();
    expect(rack!.querySelector('.hp-agent')).toBeNull();
    expect(rack!.textContent).not.toContain('DeepSeek Harness');
    expect(rack!.querySelector('.hp-rack-notes')!.textContent).toContain('每天 23:00');
    expect(rack!.textContent).not.toContain('board-skills');
  });

  it('「没有匹配规则」的提示照旧', () => {
    show(machine([agent('dsh')], { unmatched: 2 }));
    expect(host.querySelector('.hp-rack')!.textContent).toContain('2 张卡片没有匹配规则');
  });

  it('有 agent 看不到 board-skills 时底部一行警告，点一下打开机器详情；没有时不显示', () => {
    show(machine([agent('dsh'), agent('claude', { env: 'git-bash', no_skill: '~/.claude/skills/board-skills' })]));
    const warn = [...host.querySelectorAll('.hp-rack-notes .warn')].find(e => e.textContent?.includes('board-skills'));
    expect(warn?.textContent).toBe('1 个 agent 看不到 board-skills');
    act(() => (host.querySelector('.hp-rack-id') as HTMLElement).click());
    expect(useUI.getState().host).toBe('win2');

    show(machine([agent('dsh')]));
    expect(host.textContent).not.toContain('board-skills');
    // 没有要写的摘要时不占位置
    expect(host.querySelector('.hp-rack-notes')).toBeNull();
  });

  it('没找到命令的 agent、看得到 board-skills 的不算', () => {
    show(machine([agent('dsh', { found: false, no_skill: '~/.agents/skills/board-skills' }), agent('claude')]));
    expect(host.textContent).not.toContain('board-skills');
  });

});
