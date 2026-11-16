import { describe, expect, it } from 'vitest';
import { blankMcp } from './mcp';
import { syncKind } from './syncKinds';
import { mcpSync, skillSync, syncHosts, syncRow } from './syncPills';
import type { HostInfo, McpServer, SkillInfo, SyncResult } from './types';

const result = (x: Partial<SyncResult> = {}): SyncResult => ({
  key: 'k',
  at: 1000,
  agents: [],
  skills: { state: 'synced', count: 3, dirs: ['~/.claude/skills'], replaced: [], links: [] },
  mcp: { state: 'synced', count: 1, agents: ['Claude Code'], replaced: [] },
  git: { state: 'synced', host: 'github.com', repos: [], skipped: [] },
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

describe('Git 凭据卡片上的机器', () => {
  const GIT = syncKind('git');
  const gitRow = (h: HostInfo, now?: number) => syncRow(GIT, h, now);
  const failed = result({ git: { state: 'failed', host: 'github.com', repos: [], skipped: [], why: '主机不匹配' } });
  it('一台一行：已同步写按主机全局生效，同步失败写原因，关掉的写用自己的凭据', () => {
    expect(gitRow(host('mbp', { syncGit: true }), 61000)).toMatchObject({ kind: 'synced', text: expect.stringMatching(/^已同步 · github\.com 的令牌已就绪，按主机全局生效 · /) });
    expect(gitRow(host('win', { syncGit: true, sync: { able: true, last: failed } }))).toEqual({ kind: 'failed', text: '同步失败：主机不匹配' });
    expect(gitRow(host('air', { syncGit: false }))).toEqual({ kind: 'off', text: '不同步：用这台机器自己的 git 凭据' });
  });
  it('离线、正在同步、还没同步过各有一句', () => {
    expect(gitRow(host('mbp', { syncGit: true, online: false })).text).toBe('离线：上线后同步');
    expect(gitRow(host('mbp', { syncGit: true, sync: { able: true, ask: { id: 1, key: 'k', by: '人', at: 1 }, last: result() } }))).toEqual({ kind: 'pending', text: '正在同步…' });
    expect(gitRow(host('mbp', { syncGit: true, sync: { able: true } })).text).toBe('等 board host 同步…');
  });
  it('旧版 board host（不按请求同步、不报 Git 凭据这一行）不列', () => {
    const list = syncHosts(GIT, [
      host('mbp', { syncGit: true }),
      host('old', { syncGit: true, sync: { able: true, last: result({ git: { state: '', repos: [], skipped: [] } }) } }),
      host('older', { syncGit: true, sync: { able: false } }),
    ]);
    expect(list.map(h => h.name)).toEqual(['mbp']);
  });
});
