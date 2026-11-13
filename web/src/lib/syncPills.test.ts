import { describe, expect, it } from 'vitest';
import { blankMcp } from './mcp';
import { hostSync, mcpSync, skillSync } from './syncPills';
import type { HostInfo, McpServer, SkillInfo } from './types';

const host = (name: string, x: Partial<HostInfo> = {}): HostInfo =>
  ({
    id: name.length,
    name,
    online: true,
    version: '0.1.23',
    agents: [
      { id: 'claude', name: 'Claude Code', found: true, version: '1' },
      { id: 'codex', name: 'Codex', found: true, version: '1' },
      { id: 'dsh', name: 'DeepSeek Harness', found: true, version: '1' },
    ],
    ...x,
  }) as HostInfo;

const skill: SkillInfo = { name: 'design', version: 'v2v2v2v2v2v2', size: 1, files: 1, updatedAt: 0, by: '界面', source: '界面' };
const sk = (agent: string, state: 'synced' | 'local' | 'failed', version = 'v2v2v2v2v2v2') => ({ skill: 'design', agent, env: '', dir: '', state, version });

describe('一台机器合成一个同步状态', () => {
  it('失败 > 版本落后、有同名没覆盖 > 正在同步 > 已同步；离线、没有 agent 单独写', () => {
    const cell = (state: string, agent = 'claude') => ({ agent, state, word: state });
    expect(hostSync(host('a'), [cell('synced'), cell('failed', 'codex')]).word).toBe('Codex 失败');
    expect(hostSync(host('a'), [cell('pending'), cell('local', 'codex')]).kind).toBe('warn');
    expect(hostSync(host('a'), [cell('synced'), cell('pending', 'codex')]).kind).toBe('pending');
    expect(hostSync(host('a'), [cell('synced'), cell('synced', 'codex')]).kind).toBe('synced');
    expect(hostSync(host('a'), []).kind).toBe('none');
    const off = hostSync(host('a', { online: false }), [cell('synced')]);
    expect(off.kind).toBe('offline');
    expect(off.tip).toContain('上次上报');
  });

  it('skill：旧版不在里面；版本落后算 warn；还没报的每个 agent 都是正在同步', () => {
    const list = skillSync(
      [
        host('old', { skills: null }),
        host('ok', { skills: [sk('claude', 'synced'), sk('codex', 'synced'), sk('dsh', 'synced')] }),
        host('behind', { skills: [sk('claude', 'synced', 'v1v1v1v1v1v1'), sk('codex', 'synced')] }),
        host('new', { skills: [] }),
      ],
      skill,
    );
    expect(list.map(x => `${x.host.name}:${x.kind}`)).toEqual(['ok:synced', 'behind:warn', 'new:pending']);
    expect(list[1].word).toBe('Claude Code 版本落后');
    expect(list[2].cells.map(c => c.agent)).toEqual(['claude', 'codex', 'dsh']);
  });

  it('MCP：「不支持」的 agent 不算；只同步给 Claude Code 时别的 agent 不出现', () => {
    const s: McpServer = { ...blankMcp(), name: 'db', command: 'npx', agents: ['claude'] };
    const [x] = mcpSync([host('h', { mcp: [{ server: 'db', agent: 'claude', state: 'synced' }, { server: 'db', agent: 'dsh', state: 'unsupported' }] })], s);
    expect(x.kind).toBe('synced');
    expect(x.cells.map(c => c.agent)).toEqual(['claude']);
  });
});
