import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LocalHost } from '../../lib/platform';
import type { HostInfo } from '../../lib/types';
import { settleConfirm, useUI } from '../../store/ui';
import { LocalHostAdd, LocalHostPanel } from './LocalHost';

// 桌面端外壳的命令和建令牌：用例里看调了什么、按什么顺序
const calls: string[] = [];
const shellState = vi.hoisted(() => ({ st: null as LocalHost | null }));
vi.mock('../../lib/platform', async orig => {
  const real = await orig<typeof import('../../lib/platform')>();
  return {
    ...real,
    canHost: true,
    isWindows: false,
    localHost: async () => shellState.st,
    setBoardEnv: async (token: string) => {
      calls.push(`setBoardEnv:${token}`);
      return '~/.board/config.toml';
    },
    enableLocalHost: async (on: boolean, name: string, workspace: string) => {
      calls.push(`enable:${on}:${name}:${workspace}`);
    },
    saveLocalHost: async (name: string, workspace: string) => {
      calls.push(`save:${name}:${workspace}`);
    },
    setAutostart: async (on: boolean) => on,
    pickFolder: async () => null,
  };
});
vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return {
    ...real,
    api: {
      ...real.api,
      createToken: async (name: string) => {
        calls.push(`createToken:${name}`);
        return { token: 'raw-token', info: { id: 9, name, hint: 'raw', createdAt: 0, lastUsedAt: null } };
      },
    },
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  calls.length = 0;
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  useUI.setState({ confirm: null });
});

const local = (x: Partial<LocalHost> = {}): LocalHost => ({
  enabled: false,
  running: false,
  exit: null,
  name: '',
  defaultName: 'mbp',
  workspace: '',
  log: [],
  loggedIn: false,
  cli: true,
  autostart: false,
  ...x,
});

const flush = () => act(async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
});

const type = (el: HTMLInputElement, v: string) =>
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });

const button = (text: string) => [...host.querySelectorAll('button')].find(b => b.textContent?.includes(text)) as HTMLButtonElement;

async function start(st: LocalHost) {
  shellState.st = st;
  act(() => root.render(<LocalHostAdd close={() => {}} />));
  await flush();
  type(host.querySelector('input[aria-label="工作空间"]') as HTMLInputElement, '~/repo');
  act(() => button('开始接任务').click());
  await flush();
  // 确认框和以前一样：机器名、工作空间、免审批运行的风险
  const c = useUI.getState().confirm;
  expect(c?.title).toBe('让这台电脑开始接任务？');
  expect(c?.text).toContain('机器名：mbp（本机主机名）');
  expect(c?.text).toContain('工作空间：~/repo');
  expect(c?.text).toContain('免审批模式');
  act(() => settleConfirm(true));
  await flush();
}

describe('添加机器 → 这台电脑', () => {
  it('配置文件里没有令牌：以机器名新建一个、写进配置文件，再开始接任务', async () => {
    await start(local());
    expect(calls).toEqual(['createToken:mbp', 'setBoardEnv:raw-token', 'enable:true::~/repo']);
  });

  it('已经有能用的令牌：不再新建', async () => {
    await start(local({ loggedIn: true }));
    expect(calls).toEqual(['enable:true::~/repo']);
  });

  it('取消确认：什么都不做', async () => {
    shellState.st = local();
    act(() => root.render(<LocalHostAdd close={() => {}} />));
    await flush();
    type(host.querySelector('input[aria-label="工作空间"]') as HTMLInputElement, '~/repo');
    act(() => button('开始接任务').click());
    await flush();
    act(() => settleConfirm(false));
    await flush();
    expect(calls).toEqual([]);
  });

  it('没填工作空间不能开始；已经在接任务时写状态，能停下', async () => {
    shellState.st = local();
    act(() => root.render(<LocalHostAdd close={() => {}} />));
    await flush();
    expect(button('开始接任务').disabled).toBe(true);
    shellState.st = local({ enabled: true, running: true, workspace: '~/repo', log: ['board host 启动'] });
    act(() => root.render(<LocalHostAdd key="2" close={() => {}} />));
    await flush();
    expect(host.textContent).toContain('这台电脑已经在接任务');
    expect(host.textContent).toContain('board host 的输出');
    act(() => button('停止接任务').click());
    await flush();
    expect(calls).toEqual(['enable:false::~/repo']);
  });
});

describe('本机的机器详情', () => {
  const h = (name: string) => ({ name, online: true }) as HostInfo;

  it('只在这台机器就是本机在跑的 board host 时显示：状态、改名、开机自启、停止接任务', async () => {
    shellState.st = local({ enabled: true, running: true, workspace: '~/repo', autostart: true });
    act(() => root.render(<LocalHostPanel h={h('win')} />));
    await flush();
    expect(host.textContent).toBe('');
    act(() => root.render(<LocalHostPanel key="mbp" h={h('mbp')} />));
    await flush();
    expect(host.textContent).toContain('这台电脑');
    expect(host.textContent).toContain('在线：mbp');
    expect(host.querySelector('[aria-label="开机时自动启动"]')?.getAttribute('aria-checked')).toBe('true');
    // 改名：服务端原地改名（外壳的 host_save）
    type(host.querySelector('input[aria-label="机器名"]') as HTMLInputElement, 'mbp2');
    act(() => button('改名').click());
    await flush();
    expect(calls).toEqual(['save:mbp2:~/repo']);
  });

  it('没在接任务时不显示', async () => {
    shellState.st = local({ enabled: false });
    act(() => root.render(<LocalHostPanel h={h('mbp')} />));
    await flush();
    expect(host.textContent).toBe('');
  });
});
