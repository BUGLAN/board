/* 机器详情「agent」页：本机 agent 缺 board-skills 时的一键安装（#563）。
   只在桌面端看这台电脑、agent 能自动装时给「一键安装」（board host 按共用配置同步时「立即同步」），点了调桌面端的 installSkill；
   网页端、别的机器、WSL 里的 agent 不给，照旧是让 board host 同步的「装上」 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { normHost } from '../../lib/normalize';
import type { HostAgent, HostInfo } from '../../lib/types';
import { useUI } from '../../store/ui';
import { HostAgents } from './HostAgents';

const env = vi.hoisted(() => ({
  shell: 'desktop' as 'web' | 'desktop',
  local: { running: true, name: 'mbp', defaultName: 'mbp' } as { running: boolean; name: string; defaultName: string } | null,
  install: [] as string[][],
  fail: null as string | null,
}));
vi.mock('../../lib/platform', async orig => {
  const real = await orig<typeof import('../../lib/platform')>();
  return {
    ...real,
    get shell() {
      return env.shell;
    },
    localHost: async () => env.local,
    installSkill: async (agents: string[]) => {
      env.install.push(agents);
      if (env.fail) throw env.fail;
      return agents.map(a => `/home/me/.${a}/skills/board-skills`);
    },
  };
});
const synced: string[] = [];
vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return { ...real, api: { ...real.api, syncHost: async (name: string) => void synced.push(name) } };
});
// 写操作的队列和刷新不在这里测：直接执行请求
vi.mock('../../store/data', async orig => {
  const real = await orig<typeof import('../../store/data')>();
  return { ...real, optimistic: (_patch: unknown, f: () => Promise<unknown>) => f() };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let el: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  el = document.createElement('div');
  document.body.appendChild(el);
  root = createRoot(el);
  Object.assign(env, { shell: 'desktop', local: { running: true, name: 'mbp', defaultName: 'mbp' }, install: [], fail: null });
  synced.length = 0;
  useUI.setState({ toasts: [] });
});
afterEach(() => {
  act(() => root.unmount());
  el.remove();
});

const agent = (x: Partial<HostAgent> = {}): HostAgent =>
  ({ id: 'claude', name: 'Claude Code', version: '2.4.1', found: true, env: 'login', envs: [{ env: 'login', found: true, version: '2.4.1' }], no_skill: '~/.claude/skills/board-skills', ...x }) as HostAgent;
const host = (agents: HostAgent[], x: Partial<HostInfo> = {}): HostInfo =>
  normHost({
    id: 1,
    name: 'mbp',
    os: 'macOS 15.4',
    version: '0.1.46',
    agents,
    envs: [
      { id: 'login', name: '登录 shell', path: '/bin/zsh' },
      { id: 'wsl:Ubuntu', name: 'WSL Ubuntu', path: 'wsl' },
    ],
    online: true,
    ...x,
  } as unknown as HostInfo);

const render = async (h: HostInfo) => {
  await act(async () => root.render(<HostAgents h={h} view={null} save={() => {}} sync={{}} now={Date.now()} />));
  // 等 localHost() 回来、本机判断画上去
  await act(async () => {});
};
const buttons = () => [...el.querySelectorAll('button')].map(b => b.textContent?.trim() ?? '');
const button = (text: string) => [...el.querySelectorAll('button')].find(b => b.textContent?.trim() === text)!;

describe('缺 board-skills 的一键安装', () => {
  it('桌面端看这台电脑：缺 skill 的 agent 有「一键安装」，点了调 installSkill，成功有提示，不走 board host 同步', async () => {
    await render(host([agent()]));
    expect(buttons()).toContain('一键安装');
    expect(buttons()).not.toContain('装上');
    await act(async () => button('一键安装').click());
    expect(env.install).toEqual([['claude']]);
    expect(synced).toEqual([]);
    expect(useUI.getState().toasts.map(t => [t.type, t.title])).toEqual([['success', 'board-skills 已装好']]);
  });

  it('board host 按共用配置同步时按钮是「立即同步」', async () => {
    await render(host([agent()], { sync: { able: true } } as Partial<HostInfo>));
    expect(buttons()).toContain('立即同步');
    await act(async () => button('立即同步').click());
    expect(env.install).toEqual([['claude']]);
  });

  it('装失败：提示原因，按钮还在，可以再点', async () => {
    env.fail = '写不进 ~/.claude/skills：Permission denied';
    await render(host([agent()]));
    await act(async () => button('一键安装').click());
    const t = useUI.getState().toasts.at(-1)!;
    expect([t.type, t.text]).toEqual(['error', '写不进 ~/.claude/skills：Permission denied']);
    expect(button('一键安装').disabled).toBe(false);
  });

  it('旧版桌面端不支持：提示更新桌面端，之后不再给按钮', async () => {
    env.fail = 'unsupported';
    await render(host([agent()]));
    await act(async () => button('一键安装').click());
    expect(useUI.getState().toasts.at(-1)?.title).toBe('这个版本的桌面端不能一键安装');
    expect(buttons()).not.toContain('一键安装');
    expect(buttons()).toContain('装上');
  });

  it('网页端、别的机器、board host 没在跑、WSL 里的 agent：没有「一键安装」，照旧「装上」', async () => {
    env.shell = 'web';
    await render(host([agent()]));
    expect(buttons()).not.toContain('一键安装');
    expect(buttons()).toContain('装上');

    env.shell = 'desktop';
    await render(host([agent()], { name: 'win' }));
    expect(buttons()).not.toContain('一键安装');

    env.local = { running: false, name: 'mbp', defaultName: 'mbp' };
    await render(host([agent()], { name: 'mbp', id: 2 } as Partial<HostInfo>));
    expect(buttons()).not.toContain('一键安装');

    env.local = { running: true, name: '', defaultName: 'mbp' };
    await render(host([agent({ env: 'wsl:Ubuntu', envs: [{ env: 'wsl:Ubuntu', found: true, version: '2.4.1' }] })], { id: 3 } as Partial<HostInfo>));
    expect(buttons()).not.toContain('一键安装');
    expect(env.install).toEqual([]);
  });

  it('不缺 skill 的 agent 不给按钮', async () => {
    await render(host([agent({ no_skill: undefined })]));
    expect(buttons()).not.toContain('一键安装');
    expect(buttons()).not.toContain('装上');
  });
});
