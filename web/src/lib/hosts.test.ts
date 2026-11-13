import { describe, expect, it } from 'vitest';
import { boardHosts, canPickDir, checkFor, dirFresh, hostDirFor, hostLoad, hostNameError, inSession, installCommand, isLocalHost, pathParts, RESULT, resultOf, SELFTEST_STALE_MS, selftestBusy, selftestRows, selftestState, queueReason, dirReadiness, readyOk, readyWhy, sessionGroups, sessionQueue, sessionRunning, sessionSlots, skillGaps, sourceOf } from './hosts';
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

describe('目录选择文件夹（只在本机的桌面端）', () => {
  it('目录中间省略时保留开头的目录和最后一级，左端对得齐', () => {
    expect(pathParts('C:\\Users\\lan\\repo\\board')).toEqual({ head: 'C:\\Users\\lan\\repo\\', tail: 'board' });
    expect(pathParts('/home/me/repo/board')).toEqual({ head: '/home/me/repo/', tail: 'board' });
    // 没有分隔符：整串都在「最后一级」里
    expect(pathParts('Ubuntu-24.04')).toEqual({ head: '', tail: 'Ubuntu-24.04' });
  });

  it('只在本机桌面端能选工作空间的文件夹；别的机器、网页端都不显示按钮', () => {
    expect(canPickDir({ isLocal: true, desktop: true })).toBe(true);
    // 别的机器：选出来的目录是这台电脑的
    expect(canPickDir({ isLocal: false, desktop: true })).toBe(false);
    // 网页端没有系统的文件夹选择框
    expect(canPickDir({ isLocal: true, desktop: false })).toBe(false);
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

  it('boardHosts：配了工作空间的机器都算有这块白板的目录（目录由工作空间派生），带检查结果', () => {
    const ws = {
      projects: [{ key: 'p', boards: [{ path: 'p/a', readyCheck: rc }] }],
      hosts: [
        { ...host([]), name: 'mac', workspace: { path: '~/repo' }, dirs: [dir({ cmd: 'cargo --version', ok: true })] },
        // 工作空间里有这块白板、还没上报状态：算「还没检查」
        { ...host([]), id: 2, name: 'win', os: 'Windows 11', workspace: { path: 'C:\\repo' }, dirs: [] },
        { ...host([]), id: 3, name: 'none', workspace: null, dirs: [] },
      ],
    } as unknown as Workspace;
    expect(boardHosts(ws, 'p/a').map(x => [x.h.name, x.ready.key, x.d.path])).toEqual([
      ['mac', 'ok', '~/a'],
      ['win', 'pending', 'C:\\repo\\p\\a'],
    ]);
    // 配了工作空间的机器都有任意白板的目录（目录按 <工作空间>/<项目>/<白板> 派生）；白板没写检查命令时不检查
    expect(boardHosts(ws, 'p/b').map(x => [x.h.name, x.ready.key, x.d.path])).toEqual([
      ['mac', 'none', '~/repo/p/b'],
      ['win', 'none', 'C:\\repo\\p\\b'],
    ]);
  });

  it('dirFresh：刚重连上来、还没重新看过的目录不算数（board host 每 10 秒上报，最少每 30 秒看一次目录）', () => {
    const h = { ...host([]), beatAt: 1_000_000 };
    // 上报在心跳前后 30 秒内：算新的
    expect(dirFresh(h, { board: 'p/a', path: '~/a', status: 'ok', checkedAt: 990_000 })).toBe(true);
    expect(dirFresh(h, { board: 'p/a', path: '~/a', status: 'ok', checkedAt: 1_010_000 })).toBe(true);
    // 掉线之前的结果：不算（等它重新检查）
    expect(dirFresh(h, { board: 'p/a', path: '~/a', status: 'missing', checkedAt: 500_000 })).toBe(false);
    // 旧版 board host 没报检查时间：照旧算
    expect(dirFresh(h, { board: 'p/a', path: '~/a', status: 'missing' })).toBe(true);
  });

  it('hostDirFor：机器报过的用报的，没报过的按工作空间派生（分隔符跟着系统走）', () => {
    expect(hostDirFor({ ...host([]), workspace: { path: '/srv/repo/' } }, 'p/a')?.path).toBe('/srv/repo/p/a');
    expect(hostDirFor({ ...host([]), os: 'Windows 11', workspace: { path: 'C:\\repo' } }, 'p/a')?.path).toBe('C:\\repo\\p\\a');
    expect(hostDirFor({ ...host([]), workspace: null }, 'p/a')).toBeUndefined();
  });
});

describe('sessionSlots（会话弹窗取回来之前已经知道的几张）', () => {
  // 会话 #10 一共 3 张：第 1 张做完了（「最近」里带来的），第 2 张在跑、第 3 张排着（推送里的）
  const a = disp(10, { state: 'done', result: 'review', session: 10, seq: 1, total: 3 });
  const b = disp(11, { state: 'running', session: 10, seq: 2, total: 3 });
  const c = disp(12, { state: 'queued', session: 10, seq: 3, total: 3 });
  const other = disp(20, { state: 'running', session: 20, seq: 1, total: 1 });

  it('按 seq 放好位置，不是这个会话的不要；推送里的盖掉打开时带来的旧数据', () => {
    const stale = { ...b, state: 'queued' as const };
    const s = sessionSlots(10, [a, stale], [other, b, c]);
    expect(s.map(d => d?.id)).toEqual([10, 11, 12]);
    expect(s[1]?.state).toBe('running');
  });

  it('只知道一部分：一共几张取 total，不知道的位置是 null（画骨架）', () => {
    expect(sessionSlots(10, [b], []).map(d => d?.id ?? null)).toEqual([null, 11, null]);
    // total 比 seq 小（之后又排进来的）按 seq 算
    expect(sessionSlots(10, [disp(13, { session: 10, seq: 4, total: 3 })], []).length).toBe(4);
  });

  it('一张都不知道时为空；排得再多也最多先画 20 张', () => {
    expect(sessionSlots(10, undefined, [other])).toEqual([]);
    expect(sessionSlots(10, [disp(10, { session: 10, seq: 1, total: 99 })], []).length).toBe(20);
  });

  it('旧版服务端没有 session、seq：认自己的 ID，只列出知道的', () => {
    const old = disp(7, { state: 'done' });
    expect(sessionSlots(7, [old], [])).toEqual([old]);
  });
});

describe('sessionGroups（机器详情「最近」按会话合成一行）', () => {
  const d = (id: number, session: number, endedAt: number) => ({ id, session, endedAt, createdAt: 0, card: id }) as unknown as Dispatch;
  it('同一个会话的合成一组，按最近一张结束的时间排；单独的一张自己一组', () => {
    // 最近的在前：会话 1 做了 1、2、3，会话 4 只做了 4
    const g = sessionGroups([d(3, 1, 30), d(4, 4, 25), d(2, 1, 20), d(1, 1, 10)]);
    expect(g.map(x => [x.session, x.t, x.list.map(y => y.id)])).toEqual([
      [1, 30, [3, 2, 1]],
      [4, 25, [4]],
    ]);
    // 旧版服务端没有 session：按自己的 ID
    expect(sessionGroups([{ id: 7, endedAt: 1, createdAt: 0 } as unknown as Dispatch]).map(x => x.session)).toEqual([7]);
  });
});

describe('添加机器：一行安装命令', () => {
  const base = { origin: 'https://board.example.com', token: 'bd_abc', name: 'devbox' };
  it('macOS / Linux：curl | sh，带令牌、机器名、工作空间、--service', () => {
    expect(installCommand({ ...base, os: 'unix', workspace: '~/repo' })).toBe(
      'curl -fsSL https://board.example.com/install.sh | sh -s -- --token bd_abc --name devbox --workspace ~/repo --service',
    );
  });
  it('路径里有空格、引号时加引号；还没填工作空间时不带', () => {
    const c = installCommand({ ...base, os: 'unix', workspace: "/srv/my repo/it's" });
    expect(c).toContain(`--workspace '/srv/my repo/it'\\''s' --service`);
    expect(installCommand({ ...base, os: 'unix', workspace: '  ' })).not.toContain('--workspace');
  });
  it('Windows：一行 PowerShell', () => {
    expect(installCommand({ ...base, os: 'windows', workspace: "D:\\it's" })).toBe(
      "& ([scriptblock]::Create((irm 'https://board.example.com/install.ps1'))) -Token 'bd_abc' -Name 'devbox' -Workspace 'D:\\it''s' -Service",
    );
  });
  it('机器名的规则和服务端一致', () => {
    expect(hostNameError('devbox-1.lan')).toBeNull();
    expect(hostNameError('')).toBeTruthy();
    expect(hostNameError('DevBox')).toBeTruthy();
    expect(hostNameError('-x')).toBeTruthy();
    expect(hostNameError('a'.repeat(25))).toBeTruthy();
  });
});

describe('自检', () => {
  const now = 1_800_000_000_000;
  const h = (x: Partial<HostInfo> = {}): HostInfo => ({ ...host([agent('claude'), agent('pi', { name: 'pi' })]), online: true, ...x }) as HostInfo;
  it('没有请求时没有状态；正在做、等上线、过期分别写明', () => {
    expect(selftestState(h({ selftest: { items: [] } }), now)).toBeNull();
    const ask = { id: 3, by: 'octocat', at: now - 10_000 };
    expect(selftestState(h({ selftest: { ask, items: [] } }), now)).toMatchObject({ tone: 'busy' });
    expect(selftestBusy(h({ selftest: { ask, items: [] } }), now)).toBe(true);
    expect(selftestState(h({ selftest: { ask: { ...ask, agent: 'claude' }, items: [] } }), now)?.text).toContain('自检 claude');
    expect(selftestState(h({ online: false, selftest: { ask, items: [] } }), now)).toMatchObject({ tone: 'warn', text: '等机器上线后自检' });
    const old = { ...ask, at: now - SELFTEST_STALE_MS };
    expect(selftestState(h({ selftest: { ask: old, items: [] } }), now)?.text).toContain('没做完');
    expect(selftestBusy(h({ selftest: { ask: old, items: [] } }), now)).toBe(false);
  });
  it('结果按 agent、仓库、版本排，名称可读', () => {
    const rows = selftestRows(
      h({
        selftest: {
          items: [
            { kind: 'version', key: 'board', ok: false, why: 'board 0.1.1 落后于服务端 0.1.19', at: now },
            { kind: 'git', key: 'default/main', ok: true, at: now },
            { kind: 'agent', key: 'pi', ok: false, why: '没有登录：Not logged in', at: now },
          ],
        },
      }),
    );
    expect(rows.map(r => r.label)).toEqual(['pi', 'default/main 的仓库', 'board 版本']);
  });
});

/* 线上报过：整页变成「界面出错了」，原因是 Cannot read properties of undefined (reading 'color')。
   表里查不到结果（服务端先上线、界面还是旧版）时 resultOf 以前返回 undefined，调用方取 .color 就崩了 */
describe('派发结果的样子（查不到时兜底）', () => {
  const d = (x: Partial<Dispatch>) => ({ result: null, source: 'cron', ...x }) as Dispatch;
  it('认识的每个结果都有名称和颜色', () => {
    for (const k of Object.keys(RESULT)) {
      const r = resultOf(d({ result: k as Dispatch['result'] }));
      expect(r.color).toBeTruthy();
      expect(r.label).toBeTruthy();
    }
  });
  it('还没结束的（结果是 null）按「已结束」画', () => {
    expect(resultOf(d({ result: null }))).toEqual({ label: '已结束', color: 'var(--text-3)' });
  });
  it('查不到的结果不返回 undefined：取 .color、.label 都不崩', () => {
    const r = resultOf(d({ result: 'brand-new-result' as Dispatch['result'] }));
    expect(r).toEqual({ label: '已结束', color: 'var(--text-3)' });
    expect(r.color).toBeTruthy();
  });
  it('查不到的来源也不返回 undefined（只画文字）', () => {
    expect(sourceOf(d({ source: 'cron' }))).toBe('定时');
    expect(sourceOf(d({ source: 'brand-new-source' as Dispatch['source'] }))).toBe('派发');
  });
});
