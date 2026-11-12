import { describe, expect, it } from 'vitest';
import { hostLoad, inSession, isLocalHost, queueReason, sessionQueue, sessionRunning, skillGaps } from './hosts';
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
