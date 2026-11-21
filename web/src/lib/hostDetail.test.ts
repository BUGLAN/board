import { describe, expect, it } from 'vitest';
import { acceptOf, acceptPatch, acceptSummary } from './hosts';
import { agentHealth, laterProjects, syncLine, syncNotes, todayCounts, workspaceExceptions } from './hostDetail';
import { SYNC_KINDS, syncKind, type SyncLib } from './syncKinds';
import type { Dispatch, HostAgent, HostInfo, SyncResult } from './types';

const result = (x: Partial<SyncResult> = {}): SyncResult => ({
  key: 'k',
  at: 1000,
  skills: { state: 'synced', count: 6, dirs: [], replaced: [], links: [] },
  mcp: { state: 'synced', count: 2, agents: ['Claude Code', 'Codex'], replaced: [] },
  git: { state: 'synced', host: 'github.com', repos: [], skipped: [] },
  ...x,
});
const claude = (x: Partial<HostAgent> = {}): HostAgent => ({ id: 'claude', name: 'Claude Code', found: true, version: '1', ...x });
const host = (x: Partial<HostInfo> = {}): HostInfo =>
  ({ id: 1, name: 'win2', os: 'Windows 11', online: true, version: '0.2.0', max: 3, perSession: 4, paused: false, auto: true, rules: [{ op: 'in', labels: ['x'], agent: 'claude' }], windows: [[0, 1440]], agents: [claude()], dirs: [], syncSkills: true, syncMcp: true, sync: { able: true, last: result() }, ...x }) as HostInfo;
const NOW = 61_000;
const SKILLS = syncKind('skills');
const MCP = syncKind('mcp');
const GIT = syncKind('git');
/** 服务端库里有 n 个 */
const lib = (n: number): SyncLib => ({ names: Array.from({ length: n }, (_, i) => `s${i}`) });

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

  it('agent 正不正常：没找到、缺 board-skills、自检没过', () => {
    expect(agentHealth(host(), claude({ found: false })).ok).toBe(false);
    expect(agentHealth(host(), claude({ found: false, missing: 'Node.js（npx）' })).chip).toBe('缺 Node.js（npx）');
    expect(agentHealth(host(), claude({ no_skill: '~/.claude/skills' })).chip).toBe('缺 board-skills');
    const tested = host({ selftest: { asks: [], items: [{ kind: 'agent', key: 'claude', ok: false, why: '不支持强度 max', at: 5 }] } });
    expect(agentHealth(tested, claude())).toMatchObject({ ok: false, chip: '自检没过', act: 'selftest', why: '自检没通过：不支持强度 max' });
    expect(agentHealth(host(), claude())).toMatchObject({ ok: true });
  });

  it('skills、MCP 一行', () => {
    expect(syncLine(SKILLS, host(), lib(6), NOW).text).toBe('库里的 6 个 skill 已写进这台机器各个 agent 的 skill 目录 · 1 分钟前');
    expect(syncLine(SKILLS, host({ syncSkills: false }), lib(6), NOW).text).toBe('只装 board-skills，其余不碰');
    expect(syncLine(SKILLS, host({ sync: { able: true, ask: { id: 2, key: 'k', by: '', at: 0 }, last: result() } }), lib(7), NOW).text).toBe('正在写入库里的 7 个 skill…');
    expect(syncLine(MCP, host(), { names: ['github', 'db'] }, NOW).text).toBe('库里的 2 个 MCP（github、db）已写进 Claude Code、Codex · 1 分钟前');
    expect(syncLine(MCP, host({ syncMcp: false }), null, NOW).text).toBe('Board 不碰这台机器上的 MCP 配置');
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
  // 服务端设了 / 没设
  const cred: SyncLib = { names: [], none: false };
  const gitLine = (h: HostInfo, c: SyncLib | null, now: number) => syncLine(GIT, h, c, now);
  const git = (g: Partial<SyncResult['git']>) => result({ git: { state: 'synced', host: 'github.com', repos: [], skipped: [], ...g } });
  it('不改：用本机自己的', () => {
    expect(gitLine(host({ syncGit: false }), cred, 2000)).toMatchObject({ tone: 'muted', text: '用这台机器自己的 git 凭据，Board 不碰' });
  });
  it('已同步：按主机全局生效', () => {
    expect(gitLine(host({ syncGit: true }), cred, 61000)).toMatchObject({ tone: 'ok', text: expect.stringMatching(/^已同步：github.com 的令牌已就绪，按主机全局生效 · /) });
    expect(gitLine(host({ syncGit: true, sync: { able: true, last: git({}) } }), cred, 2000).text).toMatch(/^已同步：github.com 的令牌已就绪，按主机全局生效/);
  });
  it('同步失败写原因；正在同步、离线、服务端没设、旧版各有一句', () => {
    expect(gitLine(host({ syncGit: true, sync: { able: true, last: git({ state: 'failed', why: '主机不匹配：/w/x（远端是 gitlab.com）' }) } }), cred, 2000)).toEqual({ tone: 'bad', text: '同步失败：主机不匹配：/w/x（远端是 gitlab.com）' });
    expect(gitLine(host({ syncGit: true, sync: { able: true, ask: { id: 1, key: 'k2', by: '人', at: 1 }, last: result() } }), cred, 2000).tone).toBe('busy');
    expect(gitLine(host({ syncGit: true, online: false, sync: { able: true, ask: { id: 1, key: 'k2', by: '人', at: 1 }, last: result() } }), cred, 2000).text).toBe('机器离线：上线后同步');
    expect(gitLine(host({ syncGit: true }), { names: [], none: true }, 2000).text).toBe('服务端还没设 Git 凭据：在「Agent → Git 凭据」里设');
    expect(gitLine(host({ syncGit: true, sync: { able: true, last: git({ state: 'none' }) } }), null, 2000).text).toBe('服务端还没设 Git 凭据：在「Agent → Git 凭据」里设');
    expect(gitLine(host({ syncGit: true, sync: { able: true, last: git({ state: '' }) } }), cred, 2000).text).toBe('board 0.2.0 是旧版：升级后才用服务端的 Git 凭据');
  });
});

describe('skills、MCP、Git 凭据：同一个函数按种类取文案', () => {
  const ask = { id: 1, key: 'k2', by: '人', at: 1 };
  /** 这一类的结果换成 x */
  const part = (key: 'skills' | 'mcp' | 'git', x: object) => result({ [key]: { ...result()[key], ...x } });
  // 每类在六种状态下的那一句（和拆开之前的 skillsLine、mcpLine、gitLine 逐字一样）
  const cases: [string, (k: (typeof SYNC_KINDS)[number]) => HostInfo, Record<string, [string, string]>][] = [
    ['不改', k => host({ [k.flag]: false }), {
      skills: ['muted', '只装 board-skills，其余不碰'],
      mcp: ['muted', 'Board 不碰这台机器上的 MCP 配置'],
      git: ['muted', '用这台机器自己的 git 凭据，Board 不碰'],
    }],
    ['正在写入', k => host({ [k.flag]: true, sync: { able: true, ask, last: result() } }), {
      skills: ['busy', '正在写入库里的 4 个 skill…'],
      mcp: ['busy', '正在写入 4 个 MCP…'],
      git: ['busy', '正在同步：写进用户级 git 配置…'],
    }],
    ['机器离线', k => host({ [k.flag]: true, online: false, sync: { able: true, ask, last: result() } }), {
      skills: ['muted', '机器离线：上线后写入库里的 4 个 skill'],
      mcp: ['muted', '机器离线：上线后写入 4 个 MCP'],
      git: ['muted', '机器离线：上线后同步'],
    }],
    ['等 board host', k => host({ [k.flag]: true, sync: { able: true } }), {
      skills: ['busy', '等 board host 写入…'],
      mcp: ['busy', '等 board host 写入…'],
      git: ['busy', '等 board host 同步…'],
    }],
    ['写入失败', k => host({ [k.flag]: true, sync: { able: true, last: part(k.key, { state: 'failed', why: '没有权限' }) } }), {
      skills: ['bad', '写入失败：没有权限'],
      mcp: ['bad', '写入失败：没有权限'],
      git: ['bad', '同步失败：没有权限'],
    }],
    ['已写入', k => host({ [k.flag]: true }), {
      skills: ['ok', '库里的 6 个 skill 已写进这台机器各个 agent 的 skill 目录 · 1 分钟前'],
      mcp: ['ok', '库里的 2 个 MCP（s0、s1、s2、s3）已写进 Claude Code、Codex · 1 分钟前'],
      git: ['ok', '已同步：github.com 的令牌已就绪，按主机全局生效 · 1 分钟前'],
    }],
  ];
  for (const [state, make, want] of cases)
    it(`${state}：三类各一句`, () => {
      for (const k of SYNC_KINDS) expect(syncLine(k, make(k), lib(4), NOW), `${k.key} ${state}`).toEqual({ tone: want[k.key][0], text: want[k.key][1] });
    });

  it('失败没报原因、取到服务端之前按上一次写的数', () => {
    for (const k of SYNC_KINDS) expect(syncLine(k, host({ [k.flag]: true, sync: { able: true, last: part(k.key, { state: 'failed', why: null }) } }), null, NOW).text).toMatch(/失败：原因没报上来$/);
    const busy = host({ syncSkills: true, sync: { able: true, ask, last: result() } });
    expect(syncLine(SKILLS, busy, null, NOW).text).toBe('正在写入库里的 6 个 skill…');
  });

  it('下面的补充：换掉了本机同名的、软链接、没配的仓库；不改、正在写入时不写', () => {
    const last = result({
      skills: { state: 'synced', count: 2, dirs: [], replaced: ['design'], links: ['dev'] },
      mcp: { state: 'synced', count: 1, agents: [], replaced: ['github'] },
      git: { state: 'synced', host: 'github.com', repos: [], skipped: ['/w/x：远端不是 https 地址，用本机自己的凭据'] },
    });
    const h = host({ syncGit: true, sync: { able: true, last } });
    expect(syncNotes(SKILLS, h)).toEqual([{ text: '替换了本机同名的 design：旧的挪进了 ', code: '~/.board/host/skills-backup/' }, { text: '本机的 dev 是软链接，没有覆盖' }]);
    expect(syncNotes(MCP, h)).toEqual([{ text: '换掉了本机同名的 github：原来的值存进了 ', code: '~/.board/host/', tail: ' 下的备份文件' }]);
    expect(syncNotes(GIT, h)).toEqual([{ text: '旧版按仓库同步时没配的仓库：/w/x：远端不是 https 地址，用本机自己的凭据' }]);
    expect(syncNotes(GIT, host({ syncGit: false, sync: { able: true, last } }))).toEqual([]);
    expect(syncNotes(SKILLS, host({ sync: { able: true, ask, last } }))).toEqual([]);
    expect(syncNotes(MCP, host())).toEqual([]);
  });
});
