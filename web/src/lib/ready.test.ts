import { describe, expect, it } from 'vitest';
import { cmpVersion, compareHosts, oddOnes, readiness } from './ready';
import type { HostAgent, HostInfo } from './types';

const agent = (id: string, x: Partial<HostAgent> = {}): HostAgent => ({ id, name: id, version: '1.0.0', found: true, env: 'login', model: 'm', ...x });

const machine = (x: Partial<HostInfo> = {}): HostInfo => ({
  id: 1,
  name: 'mac',
  os: 'macOS 15',
  version: '0.1.19',
  agents: [agent('claude')],
  envs: [{ id: 'login', name: '登录 shell', path: '/bin/zsh' }],
  workspace: { path: '~/repo' },
  dirs: [{ board: 'default/main', path: '~/repo/default/main', status: 'ok', origin: 'git@x:y.git', branch: 'main' }],
  max: 2,
  paused: false,
  agent: 'claude',
  rules: [],
  fallback: null,
  cronOn: false,
  cron: [],
  round: false,
  online: true,
  firstAt: 0,
  beatAt: 0,
  offAt: null,
  ...x,
});

const now = 10 * 60 * 1000;
const failed = (h: HostInfo, server?: string) =>
  readiness(h, now, server)
    .filter(c => !c.ok)
    .map(c => c.key);

describe('就绪清单', () => {
  it('配好的机器全部通过', () => {
    const cs = readiness(machine(), now, '0.1.19');
    expect(cs.map(c => c.key)).toEqual(['online', 'version', 'dirs', 'skills', 'agent']);
    expect(cs.every(c => c.ok)).toBe(true);
  });

  it('刚加进来、没配目录、agent 看不到 board-skills：列出缺的项，每项都有按钮打开处理的地方；补齐后全部通过', () => {
    const fresh = machine({ workspace: null, dirs: [], agents: [agent('claude', { no_skill: '~/.claude/skills/board-skills' })] });
    const cs = readiness(fresh, now, '0.1.19');
    const miss = cs.filter(c => !c.ok);
    expect(miss.map(c => c.key)).toEqual(['dirs', 'skills']);
    expect(miss.find(c => c.key === 'dirs')).toMatchObject({ why: '还没有配工作空间，派不了卡', go: 'dirs', act: '设置工作空间' });
    expect(miss.find(c => c.key === 'skills')).toMatchObject({ go: 'agents' });
    expect(miss.find(c => c.key === 'skills')!.why).toContain('claude 看不到 board-skills');

    expect(failed(machine(), '0.1.19')).toEqual([]);
  });

  it('目录不存在、不是 git 仓库：写明是哪块白板', () => {
    const h = machine({
      dirs: [
        { board: 'default/main', path: '~/a', status: 'missing', clone: { state: 'running', url: 'u', message: '', updatedAt: 0 } },
        { board: 'mini/main', path: '~/b', status: 'not_git' },
        { board: 'x/main', path: '~/c', status: 'ok' },
      ],
    });
    const c = readiness(h, now).find(x => x.key === 'dirs')!;
    expect(c.ok).toBe(false);
    expect(c.why).toBe('default/main 的目录不存在（正在克隆）；mini/main 的目录不是 git 仓库');
    expect(c.act).toBe('去处理');
  });

  it('旧版 board host 不报目录状态时不算没通过', () => {
    expect(failed(machine({ dirs: [{ board: 'default/main', path: '~/a' }] }))).toEqual([]);
  });

  it('离线：写多久前在线，没有按钮，写在那台机器上怎么做', () => {
    const c = readiness(machine({ online: false, offAt: now - 5 * 60 * 1000 }), now).find(x => x.key === 'online')!;
    expect(c).toMatchObject({ ok: false, why: '离线，最近一次在线是5 分钟前' });
    expect(c.go).toBeUndefined();
    expect(c.hint).toContain('board host');
  });

  it('board 版本落后于服务端时没通过；还没取到服务端版本时不列这一项', () => {
    const c = readiness(machine({ version: '0.1.17' }), now, '0.1.19').find(x => x.key === 'version')!;
    expect(c).toMatchObject({ ok: false, why: 'board 0.1.17，服务端是 0.1.19' });
    expect(failed(machine({ version: '0.1.20' }), '0.1.19')).toEqual([]);
    expect(readiness(machine(), now).some(x => x.key === 'version')).toBe(false);
    expect(readiness(machine(), now, null).some(x => x.key === 'version')).toBe(false);
  });

  it('没找到能用的 agent：默认 agent 这一项没通过', () => {
    expect(failed(machine({ agent: null, agents: [agent('claude', { found: false })] }))).toEqual(['agent']);
    expect(readiness(machine({ agent: null, agents: [] }), now).find(x => x.key === 'agent')!.why).toBe('board host 没有上报 agent');
  });

  it('版本号比较', () => {
    expect(cmpVersion('0.1.9', '0.1.10')).toBeLessThan(0);
    expect(cmpVersion('v0.2.0', '0.1.10')).toBeGreaterThan(0);
    expect(cmpVersion('0.1.19', '0.1.19')).toBe(0);
    expect(cmpVersion('dev', '0.1.19')).toBeNull();
  });
});

describe('多机对比', () => {
  it('不一致：有多数时标出少数，几种值一样多时全部标出，没有数据的不参与', () => {
    expect(oddOnes(['a', 'a', 'b'])).toEqual([false, false, true]);
    expect(oddOnes(['a', 'b'])).toEqual([true, true]);
    expect(oddOnes(['a', null, 'a'])).toEqual([false, false, false]);
    expect(oddOnes([null, null])).toEqual([false, false]);
  });

  const cell = (hosts: HostInfo[], key: string) => compareHosts(hosts, now, '0.1.19').flatMap(g => g.cols).find(c => c.key === key)!.cells;

  it('两台机器的 claude 版本不同：这一格标出不一致', () => {
    const a = machine({ id: 1, name: 'a', agents: [agent('claude', { version: '2.1.3 (Claude Code)' })] });
    const b = machine({ id: 2, name: 'b', agents: [agent('claude', { version: '2.1.5 (Claude Code)' })] });
    expect(cell([a, b], 'claude:version').map(c => [c.text, !!c.diff])).toEqual([
      ['2.1.3 (Claude Code)', true],
      ['2.1.5 (Claude Code)', true],
    ]);
    // 模型一致：不标
    expect(cell([a, b], 'claude:model').some(c => c.diff)).toBe(false);
  });

  it('一台机器少了一个 agent：那一格写「没找到」并标出', () => {
    const a = machine({ id: 1, name: 'a', agents: [agent('claude'), agent('codex')] });
    const b = machine({ id: 2, name: 'b', agents: [agent('claude'), agent('codex')] });
    const c = machine({ id: 3, name: 'c', agents: [agent('claude'), agent('codex', { found: false })] });
    expect(cell([a, b, c], 'codex:version').map(x => [x.text, !!x.diff])).toEqual([
      ['1.0.0', false],
      ['1.0.0', false],
      ['没找到', true],
    ]);
  });

  it('启动环境只和同一种系统的机器比', () => {
    const mac = machine({ id: 1, name: 'mac' });
    const win = machine({
      id: 2,
      name: 'win',
      os: 'Windows 11',
      envs: [{ id: 'git-bash', name: 'Git Bash', path: 'C:/git/bash.exe' }],
      agents: [agent('claude', { env: 'git-bash' })],
    });
    expect(cell([mac, win], 'claude:env').map(x => [x.text, !!x.diff])).toEqual([
      ['登录 shell', false],
      ['Git Bash', false],
    ]);
  });

  it('就绪列：每台机器的通过数和每一项', () => {
    const ok = machine({ id: 1, name: 'ok' });
    const fresh = machine({ id: 2, name: 'fresh', workspace: null, dirs: [] });
    expect(cell([ok, fresh], 'ready:n').map(c => [c.text, c.ok])).toEqual([
      ['5/5', true],
      ['4/5', false],
    ]);
    expect(cell([ok, fresh], 'ready:dirs').map(c => c.ok)).toEqual([true, false]);
    expect(cell([ok, fresh], 'ready:dirs')[1].tip).toBe('还没有配工作空间，派不了卡');
  });
});
