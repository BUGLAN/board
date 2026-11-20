/* lib/normalize：接口数据进门时补齐缺的数组（default/main#359） */
import { describe, expect, it } from 'vitest';
import { acceptSummary, boardHosts, defaultAgent, hostDirFor, hostsOf, skillGaps } from './hosts';
import { normHost, normHosts, normMcp, normProviders, normSession, normWorkspace } from './normalize';
import { agentGroups, providerCell, runPick, usesOf } from './providers';
import { compareHosts, readiness } from './ready';
import type { HostInfo, ProviderView, Workspace } from './types';

/** 完整的一台机器（服务端照常返回的样子） */
const full = (): HostInfo =>
  ({
    id: 1,
    name: 'mbp',
    os: 'macOS 15.4',
    version: '0.1.30',
    agents: [{ id: 'claude', name: 'Claude Code', version: '2.4.1', found: true, envs: [{ env: 'login', found: true, version: '2.4.1' }], run_mode: 'own', run_provider: 'p1' }],
    envs: [{ id: 'login', name: '登录 shell', path: '/bin/zsh' }],
    workspace: { path: '~/ws', overrides: [{ board: 'oss', path: '~/oss' }] },
    dirs: [{ board: 'oss/main', path: '~/oss', status: 'ok' }],
    max: 2,
    rules: [{ op: 'in', labels: ['bug'], agent: 'claude', missing: ['bug'] }],
    paused: false,
    agent: 'claude',
    cronOn: false,
    cron: [],
    round: false,
    windows: [[0, 480]],
    online: true,
    firstAt: 0,
    beatAt: 1000,
    offAt: null,
    ops: [{ at: 1, by: '人', text: '改了' }],
    token: { id: 1, name: 't', hint: 'abcd', lastUsedAt: null, shared: ['win'] },
    selftest: { able: true, asks: [], items: [{ kind: 'agent', key: 'claude', ok: true, at: 1 }] },
    sync: {
      able: true,
      last: {
        key: 'k',
        at: 1,
        agents: [{ agent: 'claude', state: 'synced', provider: 'p1', model: 'm1' }],
        skills: { state: 'synced', count: 1, dirs: ['~/.claude/skills'], replaced: [], links: [] },
        mcp: { state: 'off', count: 0, agents: [], replaced: [] },
        git: { state: 'synced', host: 'github.com', repos: ['/w/board'], skipped: [] },
      },
    },
    skillImport: { able: true, local: [{ name: 's', version: 'v', files: 1, size: 1, agents: ['claude'], dirs: ['~/.claude/skills'] }], last: { id: 1, at: 1, by: '人', ok: ['s'], failed: [] } },
  }) as HostInfo;

/** 版本对不上的服务端发来的机器：界面以为一定有的数组缺了、是 null，同步结果里的对象也缺了 */
const skewed = (): HostInfo =>
  ({
    id: 2,
    name: 'old',
    os: 'Windows 11',
    version: '0.1.20',
    agents: [{ id: 'claude', name: 'Claude Code', version: '2.4.1', found: true, envs: null }],
    workspace: { path: '~/ws', overrides: null },
    dirs: [{ board: 'oss/main', path: '~/oss' }],
    max: 1,
    rules: [{ op: 'in', labels: null, agent: 'claude' }],
    paused: false,
    agent: 'claude',
    cronOn: false,
    cron: null,
    round: false,
    windows: null,
    online: true,
    firstAt: 0,
    beatAt: 1000,
    offAt: null,
    token: { id: 1, name: 't', hint: 'abcd', lastUsedAt: null, shared: null },
    selftest: { able: true },
    sync: { able: true, last: { key: 'k', at: 1 } },
    skillImport: { able: true, local: [{ name: 's', version: 'v', files: 1, size: 1 }], last: { id: 1, at: 1, by: '人' } },
  }) as unknown as HostInfo;

/** 连 agents、dirs 都没有的机器 */
const bare = () => ({ id: 3, name: 'bare', os: 'Linux', version: '', online: false, beatAt: 0, offAt: 0, max: 1, paused: false }) as unknown as HostInfo;

const VIEW: ProviderView = {
  providers: [{ name: 'p1', endpoints: [{ interface: 'anthropic', url: 'https://x' }], models: ['m1'] }],
  meta: [{ id: 'claude', name: 'Claude Code', protocols: ['anthropic'], efforts: ['low', 'high'] }],
};

describe('机器：必填的数组补齐', () => {
  it('完整的数据原样不变，补两遍和补一遍一样', () => {
    const h = full();
    expect(normHost(h)).toEqual(h);
    expect(normHost(normHost(h))).toEqual(normHost(h));
    const s = normHost(skewed());
    expect(normHost(s)).toEqual(s);
  });

  it('缺了、是 null 的必填数组补成空数组；同步结果里缺的 skills、mcp 补成空的', () => {
    const h = normHost(skewed());
    expect(h.cron).toEqual([]);
    expect(h.rules?.[0].labels).toEqual([]);
    expect(h.selftest?.items).toEqual([]);
    expect(h.selftest?.asks).toEqual([]);
    // 旧服务端只有一个 ask：当成唯一的请求
    const ask = { id: 2, agent: 'claude', by: '人', at: 1 };
    expect(normHost({ ...skewed(), selftest: { ask } } as unknown as HostInfo).selftest?.asks).toEqual([ask]);
    expect(h.sync?.last?.agents).toEqual([]);
    expect(h.sync?.last?.skills).toMatchObject({ state: '', count: 0, dirs: [], replaced: [], links: [] });
    expect(h.sync?.last?.mcp).toMatchObject({ state: '', count: 0, agents: [], replaced: [] });
    // 旧版 board host 不报 Git 凭据：补成空的一行
    expect(h.sync?.last?.git).toEqual({ state: '', repos: [], skipped: [] });
    expect(h.skillImport?.local[0]).toMatchObject({ agents: [], dirs: [] });
    expect(h.skillImport?.last).toMatchObject({ ok: [], failed: [] });
    const b = normHost(bare());
    expect(b.agents).toEqual([]);
    expect(b.dirs).toEqual([]);
    expect(b.cron).toEqual([]);
  });

  it('可选的数组是 null 时当作没有，不补成空数组（没有另有含义：windows 没有 = 全天接单）', () => {
    const h = normHost(skewed());
    expect(h.windows).toBeUndefined();
    expect(h.envs).toBeUndefined();
    expect(h.agents[0].envs).toBeUndefined();
    expect(h.workspace?.overrides).toBeUndefined();
    expect(h.token?.shared).toBeUndefined();
    expect(normHost(bare()).rules).toBeUndefined();
    expect(acceptSummary({ ...h, auto: true }, 0)).toContain('全天');
  });

  it('补过的机器交给机器页、机器详情、派发、共用配置用到的函数都不抛错', () => {
    for (const raw of [skewed(), bare()]) {
      const h = normHost(raw);
      const ws = { rev: 1, now: 0, projects: [], hosts: [h], dispatches: [] } as Workspace;
      expect(() => {
        readiness(h, 2000, '0.1.30');
        compareHosts([h, normHost(full())], 2000, '0.1.30');
        skillGaps(h);
        defaultAgent(h);
        hostDirFor(h, 'oss/main');
        boardHosts(ws, 'oss/main');
        for (const a of h.agents) runPick(VIEW, a);
        providerCell(h, 'claude', VIEW);
        agentGroups(usesOf([h], VIEW, 'p1'), VIEW);
      }).not.toThrow();
    }
  });
});

describe('工作区、机器列表', () => {
  it('资源（#358）：趋势缺了补成空数组，gpus、disks 是 null 当作没有；没有资源留着没有', () => {
    const raw = { ...full(), res: { at: 1, cpu: { pct: 5, cores: 4 }, gpus: null, disks: null, hist: { cpu: [5] } } } as unknown as HostInfo;
    const h = normHost(raw);
    expect(h.res?.hist).toEqual({ cpu: [5], mem: [], gpu: [] });
    expect(h.res?.gpus).toBeUndefined();
    expect(h.res?.disks).toBeUndefined();
    expect(normHost(h)).toEqual(h);
    expect(normHost({ ...full(), res: { at: 1 } } as unknown as HostInfo).res?.hist).toEqual({ cpu: [], mem: [], gpu: [] });
    expect(normHost(full()).res).toBeUndefined();
  });

  it('旧服务端没有 hosts、dispatches 时留着没有（hostsOf 给空列表）', () => {
    const ws = normWorkspace({ rev: 1, now: 0, projects: [] } as unknown as Workspace);
    expect(ws.hosts).toBeUndefined();
    expect(ws.dispatches).toBeUndefined();
    expect(hostsOf(ws)).toEqual([]);
  });

  it('工作区里的每台机器、调度的规则、在线 agent 的卡片都补齐', () => {
    const ws = normWorkspace({
      rev: 1,
      now: 0,
      projects: [],
      hosts: [bare()],
      dispatches: null,
      agents: [{ name: 'a/b', checkedAt: 0, beatAt: 0 }],
      sched: { minute: 0, rules: [{ op: 'none', agent: 'claude' }] },
    } as unknown as Workspace);
    expect(ws.hosts?.[0].agents).toEqual([]);
    expect(ws.dispatches).toBeUndefined();
    expect(ws.agents?.[0].cards).toEqual([]);
    expect(ws.sched?.rules[0].labels).toEqual([]);
  });

  it('GET /api/hosts 少了字段、是 null', () => {
    expect(normHosts(null)).toEqual({ hosts: [], dispatches: [] });
    expect(normHosts({ hosts: [bare()] }).hosts[0].dirs).toEqual([]);
  });
});

describe('共用配置、会话', () => {
  it('v0.1.30 的供应商列表（去掉了 agents）、更旧的（没有 meta）、空响应', () => {
    expect(normProviders({ providers: [], meta: [] })).toEqual({ providers: [], meta: [] });
    expect(normProviders({ meta: [{ id: 'claude', name: 'Claude Code' }] } as unknown as ProviderView)).toEqual({
      providers: [],
      meta: [{ id: 'claude', name: 'Claude Code', protocols: [], efforts: [] }],
    });
    expect(normProviders({ providers: [{ name: 'p' }] } as unknown as ProviderView).providers[0]).toEqual({ name: 'p', endpoints: [], models: [] });
    expect(normProviders(null)).toEqual({ providers: [], meta: undefined });
  });

  it('MCP 服务器缺了 args、env、headers、agents', () => {
    expect(normMcp([{ name: 'm', kind: 'stdio' }] as never)[0]).toMatchObject({ args: [], env: [], headers: [], agents: [] });
    expect(normMcp(null)).toEqual([]);
  });

  it('会话：每一段缺了输出、事件', () => {
    const v = normSession({ session: 1, parts: [{ id: 1 }] } as never);
    expect(v.parts[0]).toMatchObject({ log: [], events: [] });
    expect(normSession({ session: 1 } as never).parts).toEqual([]);
  });
});
