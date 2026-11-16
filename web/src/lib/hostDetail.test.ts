import { describe, expect, it } from 'vitest';
import { acceptOf, acceptPatch, acceptSummary } from './hosts';
import { agentHealth, configLine, gitLine, laterProjects, mcpLine, skillsLine, todayCounts, workspaceExceptions } from './hostDetail';
import type { Dispatch, HostAgent, HostInfo, ProviderView, SyncResult } from './types';

const view: ProviderView = {
  providers: [{ name: 'p1', endpoints: [{ interface: 'anthropic', url: 'https://x' }], models: ['m1'], efforts: { m1: ['high'] } }],
  meta: [{ id: 'claude', name: 'Claude Code', protocols: ['anthropic'], efforts: ['low', 'high', 'max'] }],
};
const result = (x: Partial<SyncResult> = {}): SyncResult => ({
  key: 'k',
  at: 1000,
  agents: [{ agent: 'claude', state: 'synced', provider: 'p1', model: 'm1', effort: 'high' }],
  skills: { state: 'synced', count: 6, dirs: [], replaced: [], links: [] },
  mcp: { state: 'synced', count: 2, agents: ['Claude Code', 'Codex'], replaced: [] },
  git: { state: 'synced', host: 'github.com', repos: ['/w/board'], skipped: [] },
  ...x,
});
const claude = (x: Partial<HostAgent> = {}): HostAgent => ({ id: 'claude', name: 'Claude Code', found: true, version: '1', run_mode: 'own', run_provider: 'p1', run_model: 'm1', run_effort: 'high', ...x });
const host = (x: Partial<HostInfo> = {}): HostInfo =>
  ({ id: 1, name: 'win2', os: 'Windows 11', online: true, version: '0.2.0', max: 3, perSession: 4, paused: false, auto: true, rules: [{ op: 'in', labels: ['x'], agent: 'claude' }], windows: [[0, 1440]], agents: [claude()], dirs: [], syncSkills: true, syncMcp: true, sync: { able: true, last: result() }, ...x }) as HostInfo;
const NOW = 61_000;

describe('机器详情', () => {
  it('接单三态：自动领 / 只做派给它的 / 暂停；暂停只改暂停', () => {
    expect(acceptOf({ paused: false, auto: true })).toBe('auto');
    expect(acceptOf({ paused: false, auto: false })).toBe('manual');
    expect(acceptOf({ paused: true, auto: true })).toBe('paused');
    expect(acceptPatch('paused')).toEqual({ paused: true });
    expect(acceptPatch('manual')).toEqual({ paused: false, auto: false });
    expect(acceptSummary(host(), 0)).toBe('全天（服务器时间）自动领 · 最多 3 个会话 · 一个会话 4 张');
    expect(acceptSummary(host({ paused: true }), 2)).toBe('暂停：不启动新的派发，还有 2 个在跑，跑完就是「已排空」');
    expect(acceptSummary(host({ paused: true }), 0)).toBe('暂停：已排空，可以关机或维护');
    expect(acceptSummary(host({ auto: false }), 0)).toBe('只做派给它的卡 · 最多 3 个会话 · 一个会话 4 张');
  });

  it('配置写进去没有：已写入、正在写入、不改、没有要写的、离线、写入失败', () => {
    const line = (h: HostInfo, a = h.agents[0]) => configLine(h, a, view, NOW).text;
    expect(line(host())).toBe('已写入 p1 · m1 · 强度 high · 1 分钟前');
    expect(line(host({ sync: { able: true, ask: { id: 2, key: 'k2', by: 'me', at: 1 }, last: result() } }))).toBe('正在写入 p1 · m1 · 强度 high…');
    expect(line(host({ agents: [claude({ run_mode: 'off' })] }))).toBe('不改：Board 不碰它的配置文件');
    // 旧版 board host 报的「没配」
    expect(line(host({ sync: { able: true, last: result({ agents: [{ agent: 'claude', state: 'none' }] }) } }))).toBe('没有要写的供应商：不写');
    expect(line(host({ online: false, sync: { able: true, ask: { id: 2, key: 'k2', by: 'me', at: 1 } } }))).toBe('机器离线：上线后写入 p1 · m1 · 强度 high');
    const bad = configLine(host({ sync: { able: true, last: result({ agents: [{ agent: 'claude', state: 'failed', why: '文件解析不了' }] }) } }), claude(), view, NOW);
    expect(bad).toEqual({ tone: 'bad', text: '写入失败：文件解析不了', retry: true });
    // 模型不支持选的强度：写的时候没写
    const dropped = result({ agents: [{ agent: 'claude', state: 'synced', provider: 'p1', model: 'm1', dropped: 'max' }] });
    expect(line(host({ sync: { able: true, last: dropped } }))).toBe('已写入 p1 · m1 · 不写强度 · 1 分钟前');
  });

  it('agent 正不正常：没找到、缺 board-skills、配置没写进去、自检没过', () => {
    expect(agentHealth(host(), claude({ found: false })).ok).toBe(false);
    expect(agentHealth(host(), claude({ no_skill: '~/.claude/skills' })).chip).toBe('缺 board-skills');
    const failed = host({ sync: { able: true, last: result({ agents: [{ agent: 'claude', state: 'failed', why: 'x' }] }) } });
    expect(agentHealth(failed, claude()).chip).toBe('配置没写进去');
    // 不改的 agent 不算
    expect(agentHealth(failed, claude({ run_mode: 'off' })).ok).toBe(true);
    const tested = host({ selftest: { items: [{ kind: 'agent', key: 'claude', ok: false, why: '不支持强度 max', at: 5 }] } });
    expect(agentHealth(tested, claude())).toMatchObject({ ok: false, chip: '自检没过', act: 'selftest', why: '自检没通过：不支持强度 max' });
    expect(agentHealth(host(), claude())).toMatchObject({ ok: true });
  });

  it('skills、MCP 一行', () => {
    expect(skillsLine(host(), 6, NOW).text).toBe('库里的 6 个 skill 已写进这台机器各个 agent 的 skill 目录 · 1 分钟前');
    expect(skillsLine(host({ syncSkills: false }), 6, NOW).text).toBe('只装 board-skills，其余不碰');
    expect(skillsLine(host({ sync: { able: true, ask: { id: 2, key: 'k', by: '', at: 0 }, last: result() } }), 7, NOW).text).toBe('正在写入库里的 7 个 skill…');
    expect(mcpLine(host(), ['github', 'db'], NOW).text).toBe('库里的 2 个 MCP（github、db）已写进 Claude Code、Codex · 1 分钟前');
    expect(mcpLine(host({ syncMcp: false }), null, NOW).text).toBe('Board 不碰这台机器上的 MCP 配置');
  });

  it('工作空间只列例外；还没建的项目并成一行', () => {
    const h = host({
      workspace: { path: 'D:\\repo', overrides: [{ board: 'oss', path: 'E:\\oss' }] },
      dirs: [
        { board: 'default/main', path: 'D:\\repo\\default', status: 'ok' },
        { board: 'mini/main', path: 'D:\\repo\\mini', status: 'missing' },
        { board: 'web/main', path: 'D:\\repo\\web', status: 'not_git' },
        { board: 'opennote/main', path: 'D:\\repo\\opennote', status: 'missing', clone: { state: 'failed', url: 'git@x:o.git', message: 'Permission denied', updatedAt: 1 } },
      ],
    });
    expect(workspaceExceptions(h).map(x => `${x.tone}:${x.label}:${x.text}`)).toEqual([
      'info:oss:用 --dir 单独指定：E:\\oss',
      'bad:web:D:\\repo\\web 不是 git 仓库',
      'bad:opennote:克隆失败：Permission denied',
    ]);
    expect(workspaceExceptions(h)[2].reclone).toEqual({ board: 'opennote/main', url: 'git@x:o.git' });
    expect(laterProjects(h)).toEqual(['mini']);
  });

  it('今天交审几张、异常退出几次', () => {
    const d = (result: Dispatch['result'], endedAt: number) => ({ result, endedAt }) as Dispatch;
    const now = new Date(2026, 10, 15, 12).getTime();
    const y = new Date(2026, 10, 14, 23).getTime();
    expect(todayCounts([d('review', now - 1000), d('review', y), d('crashed', now - 5000)], now)).toEqual({ review: 1, crashed: 1 });
  });
});

describe('Git 凭据一行', () => {
  const cred = { host: 'github.com', username: 'buglan', tail: '1234' };
  const git = (g: Partial<SyncResult['git']>) => result({ git: { state: 'synced', host: 'github.com', repos: [], skipped: [], ...g } });
  it('不改：用本机自己的', () => {
    expect(gitLine(host({ syncGit: false }), cred, 2000)).toMatchObject({ tone: 'muted', text: '用这台机器自己的 git 凭据，Board 不碰' });
  });
  it('已同步：配进了几个仓库；没有用这个主机的仓库也写明', () => {
    expect(gitLine(host({ syncGit: true }), cred, 61000)).toMatchObject({ tone: 'ok', text: expect.stringMatching(/^已同步：github.com 的令牌配进了 1 个仓库 · /) });
    expect(gitLine(host({ syncGit: true, sync: { able: true, last: git({}) } }), cred, 2000).text).toMatch(/^已同步：令牌已就绪，还没有远端是 github.com 的仓库/);
  });
  it('同步失败写原因；正在同步、离线、服务端没设、旧版各有一句', () => {
    expect(gitLine(host({ syncGit: true, sync: { able: true, last: git({ state: 'failed', why: '主机不匹配：/w/x（远端是 gitlab.com）' }) } }), cred, 2000)).toEqual({ tone: 'bad', text: '同步失败：主机不匹配：/w/x（远端是 gitlab.com）' });
    expect(gitLine(host({ syncGit: true, sync: { able: true, ask: { id: 1, key: 'k2', by: '人', at: 1 }, last: result() } }), cred, 2000).tone).toBe('busy');
    expect(gitLine(host({ syncGit: true, online: false, sync: { able: true, ask: { id: 1, key: 'k2', by: '人', at: 1 }, last: result() } }), cred, 2000).text).toBe('机器离线：上线后同步');
    expect(gitLine(host({ syncGit: true }), null, 2000).text).toBe('服务端还没设 Git 凭据：在「Agent → Git 凭据」里设');
    expect(gitLine(host({ syncGit: true, sync: { able: true, last: git({ state: '' }) } }), cred, 2000).text).toBe('board 0.2.0 是旧版：升级后才用服务端的 Git 凭据');
  });
});
