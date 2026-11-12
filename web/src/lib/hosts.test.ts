import { describe, expect, it } from 'vitest';
import { boardHosts, checkFor, hostLoad, inSession, isLocalHost, queueReason, dirReadiness, readyOk, readyWhy, sessionQueue, sessionRunning, skillGaps } from './hosts';
import type { Dispatch, HostAgent, HostInfo, Workspace } from './types';

const agent = (id: string, x: Partial<HostAgent> = {}): HostAgent => ({ id, name: id, version: '1', found: true, ...x });
const host = (agents: HostAgent[]): HostInfo => ({
  id: 1,
  name: 'mac',
  os: 'macOS',
  version: '0.1.15',
  agents,
  envs: [{ id: 'pwsh', name: 'PowerShell 7' }, { id: 'wsl:Ubuntu', name: 'WSL（Ubuntu）' }] as HostInfo['envs'],
  dirs: [],
  max: 1,
  paused: false,
  agent: null,
  cronOn: false,
  cron: [],
  round: false,
  online: true,
  firstAt: 0,
  beatAt: 0,
  offAt: null,
});

describe('skillGaps（看不到 board-skills 的 agent 分组）', () => {
  it('能一键装的列出目录（去重），WSL 里的和不认识的只能手动装', () => {
    const h = host([
      agent('claude', { env: 'pwsh', no_skill: '~/.claude/skills/board-skills' }),
      agent('dsh', { env: 'pwsh', no_skill: '~/.agents/skills/board-skills' }),
      agent('pi', { env: 'pwsh', no_skill: '~/.agents/skills/board-skills' }),
      agent('codex', { env: 'wsl:Ubuntu', no_skill: 'WSL（Ubuntu）里的 $CODEX_HOME/skills/board-skills' }),
      agent('mine', { no_skill: '~/.mine/skills/board-skills' }),
      agent('ok'),
      agent('gone', { found: false, no_skill: '~/.x' }),
    ]);
    const g = skillGaps(h);
    expect(g.missing.map(a => a.id)).toEqual(['claude', 'dsh', 'pi', 'codex', 'mine']);
    expect(g.auto.map(a => a.id)).toEqual(['claude', 'dsh', 'pi']);
    expect(g.manual.map(a => a.id)).toEqual(['codex', 'mine']);
    expect(g.dirs).toEqual(['~/.claude/skills', '~/.agents/skills']);
    expect(g.manualPlaces).toEqual(['WSL（Ubuntu）里的 $CODEX_HOME/skills/board-skills', '~/.mine/skills/board-skills']);
    expect(g.places.slice(0, 2)).toEqual(['~/.claude/skills/board-skills', '~/.agents/skills/board-skills']);
  });

  it('都看得到时没有缺口', () => {
    expect(skillGaps(host([agent('claude')])).missing).toEqual([]);
  });
});

describe('isLocalHost（看的是不是本机桌面端跑的 board host）', () => {
  const h = host([]);
  it('配置的机器名为空时按默认名比，没在跑不算', () => {
    expect(isLocalHost(h, { running: true, name: '', defaultName: 'mac' })).toBe(true);
    expect(isLocalHost(h, { running: true, name: ' mac ', defaultName: 'other' })).toBe(true);
    expect(isLocalHost(h, { running: true, name: 'win', defaultName: 'mac' })).toBe(false);
    expect(isLocalHost(h, { running: false, name: '', defaultName: 'mac' })).toBe(false);
    expect(isLocalHost(h, null)).toBe(false);
  });
});

const disp = (id: number, x: Partial<Dispatch> = {}): Dispatch =>
  ({
    id,
    board: 'default/main',
    card: id,
    title: `卡${id}`,
    hostId: 1,
    host: 'mac',
    agent: 'claude',
    agentLabel: 'Claude Code',
    name: null,
    source: 'cron',
    by: { kind: 'system', name: '系统' },
    state: 'queued',
    result: null,
    createdAt: 0,
    startedAt: null,
    endedAt: null,
    exitCode: null,
    stopBy: null,
    beatAt: null,
    ...x,
  }) as Dispatch;

describe('会话里排着的派发', () => {
  // 会话 #10：第 1 张在跑，#11、#12 排在会话里；#20 是普通排队（等空位）
  const run = disp(10, { state: 'running', name: 'claude-mac/opus-5.5', session: 10, seq: 1, total: 3 });
  const q1 = disp(11, { session: 10, seq: 2, total: 3 });
  const q2 = disp(12, { session: 10, seq: 3, total: 3 });
  const plain = disp(20, { source: 'manual' });
  const ws = { hosts: [host([])], dispatches: [run, q1, q2, plain] } as unknown as Workspace;

  it('认得出来：排队中、session 不是自己；旧版服务端没有 session 时都不是', () => {
    expect([run, q1, q2, plain].map(inSession)).toEqual([false, true, true, false]);
    expect(inSession(disp(30, { session: 30 }))).toBe(false);
  });

  it('排队原因写「会话第 N/M 张」，普通排队照旧', () => {
    expect(queueReason(host([]), q1)).toBe('会话第 2/3 张');
    expect(queueReason(host([]), plain)).toBe('等空位');
    expect(queueReason(undefined, q2)).toBe('会话第 3/3 张');
  });

  it('机器的排队不含会话里排着的；会话排着的按顺序列出，能找到会话正在做的那张', () => {
    const load = hostLoad(ws, host([]));
    expect(load.running.map(d => d.id)).toEqual([10]);
    expect(load.queued.map(d => d.id)).toEqual([20]);
    expect(sessionQueue(ws, 10).map(d => d.id)).toEqual([11, 12]);
    expect(sessionQueue(ws, undefined)).toEqual([]);
    expect(sessionRunning(ws, q2)?.card).toBe(10);
  });
});

describe('环境检查（readiness）', () => {
  const rc = { cmd: 'cargo --version', win: 'where cargo' };
  const dir = (ready?: { cmd: string; ok: boolean; tail?: string[]; code?: number }) => ({ board: 'p/a', path: '~/a', ready: ready && { tail: [], at: 5, ...ready } });

  it('按系统选命令：Windows 写了单独的就用它，否则和所有系统一样；都空时不检查', () => {
    expect(checkFor(rc, 'macOS 15.4')).toBe('cargo --version');
    expect(checkFor(rc, 'Windows 11 Pro')).toBe('where cargo');
    expect(checkFor({ cmd: 'cargo --version', win: ' ' }, 'Windows 11')).toBe('cargo --version');
    expect(checkFor({ cmd: '', win: '' }, 'macOS')).toBeNull();
    expect(checkFor(undefined, 'macOS')).toBeNull();
  });

  it('通过 / 没通过（输出最后一行）/ 还没检查（命令改了、旧版 board host）/ 不检查', () => {
    const h = host([]);
    expect(dirReadiness(undefined, h, dir())).toEqual({ key: 'none' });
    expect(dirReadiness(rc, h, dir())).toEqual({ key: 'pending' });
    expect(dirReadiness(rc, h, dir({ cmd: 'node --version', ok: true }))).toEqual({ key: 'pending' });
    expect(dirReadiness(rc, h, dir({ cmd: 'cargo --version', ok: true }))).toEqual({ key: 'ok', at: 5 });
    const bad = dirReadiness(rc, h, dir({ cmd: 'cargo --version', ok: false, tail: ['zsh: command not found: cargo', ''] }));
    expect(bad).toEqual({ key: 'failed', last: 'zsh: command not found: cargo', at: 5 });
    expect(dirReadiness(rc, h, dir({ cmd: 'cargo --version', ok: false, code: 2 }))).toMatchObject({ last: '退出码 2' });
    expect(readyOk({ key: 'none' }) && readyOk({ key: 'ok', at: 1 })).toBe(true);
    expect(readyOk(bad) || readyOk({ key: 'pending' })).toBe(false);
    expect(readyWhy(bad)).toBe('环境检查没通过：zsh: command not found: cargo');
  });

  it('boardHosts：只列有这块白板目录的机器，带检查结果', () => {
    const ws = {
      projects: [{ key: 'p', boards: [{ path: 'p/a', readyCheck: rc }] }],
      hosts: [
        { ...host([]), name: 'mac', dirs: [dir({ cmd: 'cargo --version', ok: true })] },
        { ...host([]), id: 2, name: 'win', os: 'Windows 11', dirs: [dir({ cmd: 'cargo --version', ok: true })] },
        { ...host([]), id: 3, name: 'none', dirs: [] },
      ],
    } as unknown as Workspace;
    expect(boardHosts(ws, 'p/a').map(x => [x.h.name, x.ready.key])).toEqual([
      ['mac', 'ok'],
      ['win', 'pending'],
    ]);
    expect(boardHosts(ws, 'p/b')).toEqual([]);
  });
});
