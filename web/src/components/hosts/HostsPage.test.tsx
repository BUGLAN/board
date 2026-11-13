import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HostAgent, HostInfo, Workspace } from '../../lib/types';
import { settleConfirm, useUI } from '../../store/ui';
import { FleetActions, Rack } from './HostsPage';

// 机器页要发的请求：默认都成功，用例里换成自己的
const mocked = vi.hoisted(() => ({
  pauseAllHosts: async (_paused: boolean): Promise<unknown[]> => [],
  stopAllHosts: async (): Promise<unknown[]> => [],
}));
vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return { ...real, api: { ...real.api, pauseAllHosts: (p: boolean) => mocked.pauseAllHosts(p), stopAllHosts: () => mocked.stopAllHosts() } };
});

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
  // 默认配了工作空间（有工作空间就视为有这块白板的目录）
  workspace: { path: 'C:\\repo' },
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

describe('磁贴上的「还差 N 项」', () => {
  it('没有全部就绪时显示，在状态那一行，不占底部说明的位置', () => {
    // 没配工作空间：差 1 项
    show(machine([agent('dsh')], { workspace: null }));
    const badge = host.querySelector('.hp-rack-state .hp-ready-badge');
    expect(badge?.textContent).toBe('还差 1 项');
    expect(badge?.getAttribute('title')).toContain('每块白板的目录和仓库正常');
    expect(host.querySelector('.hp-rack-notes')).toBeNull();
  });

  it('全部就绪时不显示', () => {
    show(machine([agent('dsh')], { workspace: { path: '~' }, dirs: [{ board: 'default/main', path: '~/default/main', status: 'ok' }] }));
    expect(host.querySelector('.hp-ready-badge')).toBeNull();
  });
});

describe('机位卡片：机器 → 会话 → 会话里的卡', () => {
  const ready = { workspace: { path: '~' }, dirs: [{ board: 'default/main', path: '~/default/main', status: 'ok' as const }] };
  const base = { board: 'default/main', hostId: 1, host: 'win2', agent: 'dsh', agentLabel: 'dsh', source: 'cron', by: { kind: 'system', name: '系统' }, result: null, createdAt: 0, endedAt: null, exitCode: null, stopBy: null, beatAt: null };

  it('一栏一个会话：做完几张、正在做的、后面排着的；排队列表里不列会话里的', () => {
    const h = machine([agent('dsh')], { ...ready, perSession: 4 });
    const dispatches = [
      { ...base, id: 5, card: 5, title: '第二张', name: 'dsh-win2/x', state: 'running', startedAt: Date.now(), session: 4, seq: 2, total: 4 },
      { ...base, id: 6, card: 6, title: '第三张', name: null, state: 'queued', startedAt: null, session: 4, seq: 3, total: 4 },
      { ...base, id: 7, card: 7, title: '第四张', name: null, state: 'queued', startedAt: null, session: 4, seq: 4, total: 4 },
    ];
    show(h, { hosts: [h], dispatches } as unknown as Workspace);
    const col = host.querySelector('.hp-sess.on')!;
    expect(col.querySelector('.hp-sess-head b')?.textContent).toBe('会话 1');
    expect(col.querySelector('.hp-sess-n')?.textContent).toBe('2/4');
    expect(col.querySelector('.hp-sess-done')?.textContent).toBe('做完 1 张');
    expect(col.querySelector('.hp-sess-card.cur')?.textContent).toContain('#5');
    expect([...col.querySelectorAll('.hp-sess-card.next')].map(e => e.querySelector('.hp-cid')?.textContent)).toEqual(['#6', '#7']);
    expect(host.querySelectorAll('.hp-lane.queued').length).toBe(0);
    // 上限 4 个会话：另外 3 栏是空位，只有第一栏写怎么开会话
    const empty = host.querySelectorAll('.hp-sess.empty');
    expect(empty.length).toBe(3);
    expect(empty[0].textContent).toContain('一次排好最多 4 张');
    expect(empty[1].querySelector('p')).toBeNull();
    expect(host.querySelector('.hp-sess-sum')?.textContent).toBe('会话 1/4定时领取一个会话最多做 4 张');
  });

  it('暂停接单：空的会话位画成暂停；离线：不画会话，只写离线', () => {
    show(machine([agent('dsh')], { ...ready, max: 2, paused: true }));
    expect([...host.querySelectorAll('.hp-sess.empty.paused')].length).toBe(2);
    expect(host.querySelector('.hp-sess-sum')?.textContent).toContain('暂停接单，不开新会话');
    show(machine([agent('dsh')], { ...ready, online: false }));
    expect(host.querySelector('.hp-sess')).toBeNull();
    expect(host.querySelector('.hp-lane.dead')?.textContent).toContain('离线');
  });

  it('不属于会话的排队照旧排在下面', () => {
    const h = machine([agent('dsh')], { ...ready, max: 1 });
    const dispatches = [
      { ...base, id: 5, card: 5, title: '在跑', name: 'dsh-win2/x', state: 'running', startedAt: Date.now(), session: 5, seq: 1, total: 1 },
      { ...base, id: 8, card: 8, title: '等空位', name: null, state: 'queued', startedAt: null, source: 'manual' },
    ];
    show(h, { hosts: [h], dispatches } as unknown as Workspace);
    expect(host.querySelector('.hp-sess-n')).toBeNull();
    expect(host.querySelectorAll('.hp-sess.empty').length).toBe(0);
    expect(host.querySelector('.hp-lane.queued')?.textContent).toContain('#8');
  });
});

describe('全部暂停 / 全部停止', () => {
  it('在线机器都暂停时按钮是「全部恢复」，否则是「全部暂停」；离线机器不算', () => {
    act(() => root.render(<FleetActions hosts={[machine([agent('dsh')], { paused: true }), machine([agent('dsh')], { paused: true, name: 'off', online: false })]} />));
    expect(host.textContent).toContain('全部恢复');
    act(() => root.render(<FleetActions hosts={[machine([agent('dsh')], { paused: true }), machine([agent('dsh')], { paused: false, name: 'win' })]} />));
    expect(host.textContent).toContain('全部暂停');
  });

  it('没有在线机器时「全部暂停」不能点；全部停止随时可以点（会二次确认）', () => {
    act(() => root.render(<FleetActions hosts={[]} />));
    const [pause, stop] = [...host.querySelectorAll('button')] as HTMLButtonElement[];
    expect(pause.textContent).toContain('全部暂停');
    expect(pause.disabled).toBe(true);
    expect(stop.textContent).toContain('全部停止');
    expect(stop.disabled).toBe(false);
  });

  it('点「全部暂停」调接口；按钮在请求返回之前禁用', async () => {
    let done!: (v: HostInfo[]) => void;
    const call = vi.fn(() => new Promise<HostInfo[]>(res => (done = res)));
    mocked.pauseAllHosts = call;
    act(() => root.render(<FleetActions hosts={[machine([agent('dsh')])]} />));
    const pause = [...host.querySelectorAll('button')].find(b => b.textContent?.includes('全部暂停')) as HTMLButtonElement;
    act(() => pause.click());
    await act(async () => {});
    expect(call).toHaveBeenCalledWith(true);
    const now = [...host.querySelectorAll('button')].find(b => b.textContent?.includes('全部暂停')) as HTMLButtonElement;
    expect(now.disabled).toBe(true);
    await act(async () => done([]));
    expect(useUI.getState().toasts.at(-1)?.title).toBe('所有机器暂停接单');
  });

  it('点「全部停止」先确认：取消时不调接口；确认后调接口并提示', async () => {
    const stop = vi.fn(async () => []);
    mocked.stopAllHosts = stop;
    act(() => root.render(<FleetActions hosts={[machine([agent('dsh')])]} />));
    const btn = () => [...host.querySelectorAll('button')].find(b => b.textContent?.includes('全部停止')) as HTMLButtonElement;
    act(() => btn().click());
    // 确认框打开，说明里写了会结束几个派发、卡片按异常退出处理
    const req = useUI.getState().confirm!;
    expect(req.title).toContain('停止所有机器');
    expect(req.tone).toBe('danger');
    act(() => settleConfirm(false));
    await act(async () => {});
    expect(stop).not.toHaveBeenCalled();
    // 再点一次，确认
    act(() => btn().click());
    act(() => settleConfirm(true));
    await act(async () => {});
    expect(stop).toHaveBeenCalledTimes(1);
    expect(useUI.getState().toasts.at(-1)?.title).toBe('已停止所有派发，所有机器暂停接单');
  });
});
