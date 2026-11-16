import { describe, expect, it } from 'vitest';
import { blankMcp } from './mcp';
import { gitSync, mcpSync, skillSync } from './syncPills';
import type { HostInfo, McpServer, SkillInfo, SyncResult } from './types';

const result = (x: Partial<SyncResult> = {}): SyncResult => ({
  key: 'k',
  at: 1000,
  agents: [],
  skills: { state: 'synced', count: 3, dirs: ['~/.claude/skills'], replaced: [], links: [] },
  mcp: { state: 'synced', count: 1, agents: ['Claude Code'], replaced: [] },
  git: { state: 'synced', host: 'github.com', repos: ['/w/board'], skipped: [] },
  ...x,
});
const host = (name: string, x: Partial<HostInfo> = {}): HostInfo =>
  ({
    id: name.length,
    name,
    online: true,
    version: '0.2.0',
    syncSkills: true,
    syncMcp: true,
    sync: { able: true, last: result() },
    agents: [
      { id: 'claude', name: 'Claude Code', found: true, version: '1' },
      { id: 'dsh', name: 'DeepSeek Harness', found: true, version: '1' },
    ],
    ...x,
  }) as HostInfo;

const skill: SkillInfo = { name: 'design', version: 'v2v2v2v2v2v2', size: 1, files: 1, updatedAt: 0, by: '界面', source: '界面' };
const builtin: SkillInfo = { ...skill, name: 'board-skills', builtin: true };

describe('一台机器一个胶囊', () => {
  it('skill：旧版不在里面；不改、离线、正在写入、失败、软链接没覆盖、已写入', () => {
    const list = skillSync(
      [
        host('old', { sync: { able: false } }),
        host('off', { syncSkills: false }),
        host('away', { online: false }),
        host('busy', { sync: { able: true, ask: { id: 1, key: 'k', by: 'me', at: 1 }, last: result() } }),
        host('bad', { sync: { able: true, last: result({ skills: { state: 'failed', count: 0, dirs: [], replaced: [], links: [], why: '没有权限' } }) } }),
        host('link', { sync: { able: true, last: result({ skills: { state: 'synced', count: 3, dirs: [], replaced: [], links: ['design'] } }) } }),
        host('ok'),
      ],
      skill,
      2000,
    );
    expect(list.map(x => `${x.host.name}:${x.kind}`)).toEqual(['off:off', 'away:offline', 'busy:pending', 'bad:failed', 'link:warn', 'ok:synced']);
    expect(list[3].tip).toBe('没有权限');
  });

  it('board-skills 不管开关都装：不改的机器上也算已写入', () => {
    const [x] = skillSync([host('off', { syncSkills: false, sync: { able: true, last: result({ skills: { state: 'off', count: 1, dirs: [], replaced: [], links: [] } }) } })], builtin, 2000);
    expect(x.kind).toBe('synced');
  });

  it('MCP：只同步给别的 agent、这台机器上没有会用它的 agent 时写「没有要写的 agent」', () => {
    const s: McpServer = { ...blankMcp(), name: 'db', command: 'npx', agents: ['codex'] };
    const [x] = mcpSync([host('h')], s, 2000);
    expect(x.kind).toBe('none');
    const [y] = mcpSync([host('h')], { ...s, agents: [] }, 2000);
    expect(y.kind).toBe('synced');
    expect(y.tip).toContain('Claude Code');
    const [z] = mcpSync([host('h', { syncMcp: false })], { ...s, agents: [] }, 2000);
    expect(z.kind).toBe('off');
  });
});

describe('Git 凭据的机器胶囊', () => {
  it('已同步 / 同步失败 / 不改；旧版 board host 不画', () => {
    const list = gitSync([
      host('mbp', { syncGit: true }),
      host('win', { syncGit: true, sync: { able: true, last: result({ git: { state: 'failed', host: 'github.com', repos: [], skipped: [], why: '主机不匹配' } }) } }),
      host('air', { syncGit: false }),
      host('old', { syncGit: true, sync: { able: true, last: result({ git: { state: '', repos: [], skipped: [] } }) } }),
    ]);
    expect(list.map(s => [s.host.name, s.kind, s.word])).toEqual([
      ['mbp', 'synced', '已同步'],
      ['win', 'failed', '同步失败'],
      ['air', 'off', '不改'],
    ]);
    expect(list[1].tip).toBe('主机不匹配');
    expect(list[2].tip).toContain('自己的 git 凭据');
  });
});
