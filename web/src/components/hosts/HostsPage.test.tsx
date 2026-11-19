import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../../lib/api';
import type { HostAgent, HostInfo, Workspace } from '../../lib/types';
import { useData } from '../../store/data';
import { openHost, settleConfirm, useUI } from '../../store/ui';
import { AgentPage } from '../agent/AgentPage';
import { useAgentSec, type SecId } from '../agent/AgentNav';
import { HostDetail, type DetailTab } from './HostDetail';
import { HostModal } from './HostModal';
import { FleetActions, HostsPage, Rack } from './HostsPage';

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
  it('不显示 agent 胶囊，自动接单的时段摘要在', () => {
    show(machine([agent('dsh', { env: 'pwsh' }), agent('claude', { env: 'git-bash' })], { auto: true, windows: [[1320, 1440], [0, 480]] }));
    const rack = host.querySelector('.hp-rack');
    expect(rack).not.toBeNull();
    expect(rack!.querySelector('.hp-agent')).toBeNull();
    expect(rack!.textContent).not.toContain('DeepSeek Harness');
    expect(rack!.querySelector('.hp-rack-notes')!.textContent).toContain('自动接单 · 22:00–08:00');
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
    expect(badge?.getAttribute('title')).toContain('每个项目的目录和仓库正常');
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
    expect(host.querySelector('.hp-sess-sum')?.textContent).toBe('会话 1/4自动领取的会话一个最多做 4 张');
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

/* 界面和服务端版本不一致（服务端升级后开着的旧页面、桌面端自带的界面连别的版本的服务端）：接口少了界面以为一定有的数组。
   线上那次是 v0.1.30 的 GET /api/providers 去掉了 agents，v0.1.29 的页面在「共用配置」里 view.agents.find 整页崩（default/main#359）。
   这里用假的服务端返回缺字段的响应，走真实的 api（进门时补齐，lib/normalize.ts），把机器页、Agent tab 的各节和机器详情都画一遍 */
describe('接口少了字段时机器页不整页崩', () => {
  const META = [{ id: 'claude', name: 'Claude Code', protocols: ['anthropic'], efforts: ['low', 'high'] }];
  // 一台机器：agents、dirs、cron 都没有，同步结果、自检、导入里的数组也没有
  const RAW = {
    id: 2,
    name: 'old',
    os: 'Windows 11',
    version: '0.1.20',
    online: true,
    firstAt: 0,
    beatAt: Date.now(),
    offAt: null,
    max: 2,
    paused: false,
    agent: 'claude',
    round: false,
    cronOn: false,
    auto: true,
    rules: [{ op: 'in', agent: 'claude' }],
    workspace: { path: '~/ws' },
    sync: { able: true, last: { key: 'k', at: 1 } },
    selftest: { able: true, asks: [] },
    skillImport: { able: true },
    syncSkills: true,
    syncMcp: true,
  };
  const server = (providers: unknown) =>
    vi.fn(async (url: string) => {
      const path = String(url).split('?')[0];
      const body: unknown =
        path === '/api/workspace'
          ? { rev: 1, now: Date.now(), projects: [], hosts: [RAW], dispatches: null, sched: { minute: 0 } }
          : path === '/api/hosts'
            ? { hosts: [RAW] }
            : path === '/api/providers'
              ? providers
              : path === '/api/mcp'
                ? [{ name: 'm', kind: 'stdio', command: 'x' }]
                : path === '/api/info'
                  ? { version: '0.1.30' }
                  : null;
      return new Response(body == null ? '' : JSON.stringify(body), { status: 200 });
    });

  afterEach(() => {
    vi.unstubAllGlobals();
    useData.setState({ ws: null });
    useUI.setState({ hostsView: 'racks' });
    useAgentSec.setState({ sec: 'providers' });
  });

  // v0.1.30 的供应商列表（没有 agents）、更旧的没有 providers、空响应
  for (const [what, providers] of [
    ['v0.1.30 的供应商列表', { providers: [], meta: META }],
    ['供应商列表缺了 providers', { meta: META }],
    ['供应商列表是空的', null],
  ] as const)
    it(`${what}：机器页三个视图、Agent tab 的共用配置和自动领取、机器详情的三个页签都画得出来`, async () => {
      vi.stubGlobal('fetch', server(providers));
      const ws = await api.workspace();
      act(() => useData.setState({ ws }));
      const h = ws.hosts![0];
      for (const view of ['racks', 'compare', 'sched'] as const) {
        act(() => useUI.setState({ hostsView: view }));
        await act(async () => root.render(<HostsPage />));
        await act(async () => {});
        expect(host.querySelector('.page-hosts')).not.toBeNull();
        if (view === 'racks') expect(host.querySelector('.hp-rack')?.textContent).toContain('old');
        if (view === 'sched') expect(host.querySelector('.sd-line')?.textContent).toContain('old');
      }
      for (const sec of ['providers', 'mcp', 'skills', 'git', 'rules', 'crons', 'limits'] as SecId[]) {
        act(() => useAgentSec.setState({ sec }));
        await act(async () => root.render(<AgentPage />));
        await act(async () => {});
        expect(host.querySelector('.page-agent')).not.toBeNull();
        // 模型供应商一节读完了（不是「正在读取…」，也不是读取失败）
        if (sec === 'providers') {
          expect(host.querySelector('.sc-providers')?.textContent).toContain('还没有');
          expect(host.querySelector('.sc-providers .lh-warn')).toBeNull();
        }
      }
      for (const tab of ['overview', 'agents', 'settings'] as DetailTab[]) {
        await act(async () => root.render(<HostDetail h={h} i={0} n={1} step={() => {}} close={() => {}} on tab={tab} setTab={() => {}} />));
        await act(async () => {});
        expect(host.querySelector('.drawer-body')).not.toBeNull();
      }
    });

  /* 保存状态（.hp-sync，不是 inline 的）绝对定位在 .hp-ctl 左边：父元素不是 .hp-ctl 时定位落到页面或弹窗上，
     画到可视区外面，改设置时看不到保存中、已保存（default/main#391：工作空间、领取规则、自动升级） */
  it('机器页三个视图、Agent tab 各节和机器详情三个页签里，控件旁的保存状态都在 .hp-ctl 里', async () => {
    vi.stubGlobal('fetch', server({ providers: [], meta: META }));
    const ws = await api.workspace();
    act(() => useData.setState({ ws }));
    const loose = () => [...host.querySelectorAll('.hp-sync:not(.in)')].filter(s => !s.parentElement?.classList.contains('hp-ctl')).map(s => s.parentElement?.className);
    for (const view of ['racks', 'compare', 'sched'] as const) {
      act(() => useUI.setState({ hostsView: view }));
      await act(async () => root.render(<HostsPage />));
      await act(async () => {});
      expect(loose()).toEqual([]);
      // 接单里自动升级一个
      if (view === 'sched') expect(host.querySelectorAll('.sd-up .hp-ctl > .hp-sync')).toHaveLength(1);
    }
    for (const sec of ['providers', 'mcp', 'skills', 'git', 'rules', 'limits'] as SecId[]) {
      act(() => useAgentSec.setState({ sec }));
      await act(async () => root.render(<AgentPage />));
      await act(async () => {});
      expect(loose()).toEqual([]);
      // 领取规则：「加一条规则」旁边一个
      if (sec === 'rules') expect(host.querySelectorAll('.sc-acts .hp-ctl > .hp-sync')).toHaveLength(1);
    }
    for (const tab of ['overview', 'agents', 'settings'] as DetailTab[]) {
      await act(async () => root.render(<HostDetail h={ws.hosts![0]} i={0} n={1} step={() => {}} close={() => {}} on tab={tab} setTab={() => {}} />));
      await act(async () => {});
      expect(loose()).toEqual([]);
      if (tab === 'settings') expect(host.querySelector('.hd-ws-field .hp-ctl > .hp-sync')).not.toBeNull();
    }
  });
});

/* 手机适配（default/main#384）：机器 tab 的三个视图、Agent tab 和机器详情三个页签从 360px 起不出横向滚动条。
   浏览器宽度 jsdom 量不出来，这里直接盯样式表：手机上必须有一条 ≤760px 的规则管住这些块 */
// 样式表按本测试文件的位置取（vitest 里 import.meta.url 是运行时路径，不是源文件）
const css = (name: string) => readFileSync(new URL(`../../styles/${name}`, pathToFileURL(__filename)), 'utf8');
/** 样式表里最后一条「整屏手机」那段（断点和仓库其它页面一致：760px） */
const phoneBlock = (text: string, at: number) => text.slice(text.indexOf('@media (max-width: 760px)', at));
describe('手机适配的样式还在（机器 tab、机器详情）', () => {
  it('机器页、Agent tab 只纵向滚动，页头、视图切换、磁贴、对比、共用配置、领取规则和接单都有手机规则', () => {
    const hosts = css('hosts.css');
    expect(hosts).toMatch(/\.page-hosts \{[^}]*overflow-x: hidden/);
    expect(css('settings.css')).toMatch(/\.page-agent \{[^}]*overflow-x: hidden/);
    const at = hosts.indexOf('机器 Tab 的三个视图、Agent tab、机器详情弹窗从 360px 起逐页适配');
    expect(at).toBeGreaterThan(0);
    const mobile = phoneBlock(hosts, at);
    for (const sel of ['.hp-b-acts .segmented', '.hp-sess-grid', '.hp-cmp', '.sy-row', '.pv-line', '.sd-row', '.sd-cond .input.sel', '.hp-ctl > .hp-sync']) expect(mobile).toContain(sel);
    // 弹窗里的表格自己横向滚动：滚动框的宽度不算内容宽度（否则页面被撑宽）
    expect(hosts).toMatch(/\.hp-cmp-box \{[^}]*min-width: 0/);
  });

  it('机器详情：页签不挤、概况一列、下拉和按钮不超出屏幕', () => {
    const detail = css('hostdetail.css');
    const at = detail.indexOf('机器详情三个页签从 360px 起能完整看');
    expect(at).toBeGreaterThan(0);
    const mobile = phoneBlock(detail, at);
    for (const sel of ['.hd-tabs', '.hd-over', '.hd-v .hp-ctl', '.hd-ws-field', '.hd-sheet']) expect(mobile).toContain(sel);
  });

  it('以前撑宽机器详情的长路径、长选项都在自己的框里换行', () => {
    const detail = css('hostdetail.css');
    // 机器详情只纵向滚动：以前设置页的长路径把它撑宽、能横滑，切页签后三页都被裁掉左边
    expect(detail).toMatch(/\.hd-body \{[^}]*overflow-x: hidden/);
    // 「每个项目一个目录：<工作空间>\<项目>」这串路径没有空格，要能在中间断行
    expect(detail).toMatch(/\.hd-ws-main > small \{[^}]*overflow-wrap: anywhere/);
    // 选择框的选项列表（供应商 · 模型名很长）不比屏幕宽
    expect(css('canvas.css')).toMatch(/\.ctx\.ctx-select \{[^}]*max-width: calc\(100vw - 16px\)/);
  });
});

describe('机器详情开着时背景页面不滚动', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    useData.setState({ ws: null });
  });

  it('打开时锁住 html、body 的滚动，关掉后放回原来的值', async () => {
    // 弹窗会轮询机器、取最近的派发和供应商：请求都挂着不回，只看滚动锁
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(() => {})));
    document.body.style.overflow = 'auto';
    act(() => useData.setState({ ws: ws([machine([agent('dsh')])]) }));
    await act(async () => root.render(<HostModal />));
    expect(document.documentElement.style.overflow).toBe('');

    await act(async () => openHost('win2'));
    expect(document.documentElement.style.overflow).toBe('hidden');
    expect(document.body.style.overflow).toBe('hidden');

    await act(async () => openHost(null));
    expect(document.documentElement.style.overflow).toBe('');
    expect(document.body.style.overflow).toBe('auto');
    document.body.style.overflow = '';
  });
});
