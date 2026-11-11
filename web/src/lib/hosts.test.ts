import { describe, expect, it } from 'vitest';
import { isLocalHost, skillGaps } from './hosts';
import type { HostAgent, HostInfo } from './types';

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
