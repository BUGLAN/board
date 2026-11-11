import { describe, expect, it } from 'vitest';
import { isLocalHost, runFor, runInfo, skillGaps } from './hosts';
import type { Dispatch, HostAgent, HostInfo } from './types';

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

/* 派发的运行状态（卡片 #211）：日志为空时也能看出 agent 是在干活还是在空转 */
const T = 1_700_000_000_000;
const S = 1000;
const M = 60 * S;
const run = (x: Partial<Dispatch> = {}): Dispatch => ({
  id: 7,
  board: 'default/main',
  card: 178,
  title: 'x',
  hostId: 1,
  host: 'win',
  agent: 'pi',
  agentLabel: 'pi',
  name: 'pi-win/pi',
  source: 'manual',
  by: { kind: 'human', name: '人' } as Dispatch['by'],
  state: 'running',
  result: null,
  createdAt: T,
  startedAt: T,
  endedAt: null,
  exitCode: null,
  stopBy: null,
  beatAt: T,
  outputAt: null,
  cpu: 0,
  ...x,
});

describe('runInfo（派发的运行状态）', () => {
  it('只 sleep、不输出：启动 2 分钟内等待输出，之后可能空转，写明运行多久、没有任何输出、CPU', () => {
    expect(runInfo(run(), T + 90 * S)).toMatchObject({ key: 'waiting', label: '等待输出', basis: '已运行 1 分钟 · 没有任何输出 · CPU 0%', note: null });
    expect(runInfo(run(), T + 15 * M)).toMatchObject({ key: 'idle', label: '可能空转', basis: '已运行 15 分钟 · 没有任何输出 · CPU 0%' });
  });

  it('每几秒打印一行：工作中，写最近输出；停止打印 2 分钟后可能空转', () => {
    expect(runInfo(run({ outputAt: T + 5 * M - 12 * S }), T + 5 * M)).toMatchObject({ key: 'working', label: '工作中', basis: '最近输出 12 秒前 · CPU 0%' });
    expect(runInfo(run({ outputAt: T + 5 * M - 60 * S }), T + 5 * M)?.key).toBe('waiting');
    expect(runInfo(run({ outputAt: T + 5 * M - 2 * M }), T + 5 * M)).toMatchObject({ key: 'idle', basis: '最近输出 2 分钟前 · CPU 0%' });
  });

  it('只空转 CPU、不打印：工作中（CPU N%），不会因为日志为空被当成空转', () => {
    expect(runInfo(run({ cpu: 87 }), T + 30 * M)).toMatchObject({ key: 'working', basis: '已运行 30 分钟 · 没有任何输出 · CPU 87%' });
    expect(runInfo(run({ cpu: 5 }), T + 30 * M)?.key).toBe('working');
    // CPU 1%–5%：不算工作中，也不算空转
    expect(runInfo(run({ cpu: 2 }), T + 30 * M)?.key).toBe('waiting');
  });

  it('Claude Code 结束前不打印：一启动是等待输出，注明原因，不会马上标成可能空转', () => {
    const cc = (x: Partial<Dispatch> = {}) => run({ agent: 'claude', agentLabel: 'Claude Code', ...x });
    for (const t of [5 * S, 60 * S, 119 * S]) {
      expect(runInfo(cc(), T + t)).toMatchObject({ key: 'waiting', note: 'Claude Code 结束前不打印过程' });
    }
    // 只有一行启动提示（截图里的 unrecognized_model）也一样
    expect(runInfo(cc({ outputAt: T + S }), T + 90 * S)).toMatchObject({ key: 'waiting', note: 'Claude Code 结束前不打印过程' });
    expect(runInfo(cc({ cpu: 30 }), T + 10 * M)).toMatchObject({ key: 'working', note: null });
  });

  it('量不到 CPU（旧版 board host、WSL）：不下结论，只写依据；旧版服务端、排队、结束、正在停止不画', () => {
    expect(runInfo(run({ cpu: null }), T + 15 * M)).toMatchObject({ key: null, label: null, basis: '已运行 15 分钟 · 没有任何输出' });
    expect(runInfo(run({ cpu: null, outputAt: T + 15 * M - 3 * S }), T + 15 * M)?.basis).toBe('最近输出 3 秒前');
    expect(runInfo(run({ outputAt: undefined, cpu: undefined }), T + 15 * M)).toBeNull();
    expect(runInfo(run({ state: 'queued', startedAt: null }), T)).toBeNull();
    expect(runInfo(run({ state: 'done' }), T)).toBeNull();
    expect(runInfo(run({ stopBy: '人' }), T)).toBeNull();
  });

  it('判断口径写在悬停提示里：心跳不等于干活', () => {
    expect(runInfo(run(), T)?.tip).toContain('不代表在干活');
    expect(runInfo(run(), T)?.tip).toContain('CPU ≥ 5%');
    expect(runInfo(run({ cpu: null }), T)?.tip).toContain('WSL');
  });

  it('runFor：秒、分钟、小时', () => {
    expect([runFor(0), runFor(59 * S), runFor(15 * M + 32 * S), runFor(60 * M), runFor(125 * M)]).toEqual(['0 秒', '59 秒', '15 分钟', '1 小时', '2 小时 5 分钟']);
  });
});
