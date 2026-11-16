import { afterEach, describe, expect, it } from 'vitest';
import { builtinName, mcpAgents, mergeMeta, setAgentMeta, skillHome, useAgentMeta } from './agentMeta';
import { agentMark } from './hosts';
import { blankMcp, forAgent } from './mcp';
import { syncAgentName } from './syncPills';

const initial = useAgentMeta.getState().list;
afterEach(() => useAgentMeta.setState({ list: initial }));

describe('内置 agent 的说明', () => {
  it('取到服务端之前用兜底：MCP 名单、名称、skill 目录、主色和以前一样', () => {
    expect(mcpAgents()).toEqual(['claude', 'pi', 'codex']);
    expect(builtinName('dsh')).toBe('DeepSeek Harness');
    expect(builtinName('nope')).toBe('nope');
    expect([skillHome('dsh'), skillHome('pi'), skillHome('claude'), skillHome('codex'), skillHome('nope')]).toEqual([
      '~/.agents/skills',
      '~/.agents/skills',
      '~/.claude/skills',
      '$CODEX_HOME/skills',
      undefined,
    ]);
    expect(agentMark('claude').color).toBe('#D97757');
    expect(agentMark('codex')).toMatchObject({ short: 'CX', color: '#10A37F' });
    expect(syncAgentName(null, 'codex')).toBe('Codex');
    expect(forAgent(blankMcp(), 'pi') && !forAgent(blankMcp(), 'dsh')).toBe(true);
  });

  it('旧服务端的 meta 只有名称、接口、强度：其余用兜底', () => {
    const old = mergeMeta([{ id: 'claude', name: 'Claude Code', protocols: ['anthropic'], efforts: ['high'] }]);
    expect(old[0]).toMatchObject({ id: 'claude', efforts: ['high'], skills: ['~/.claude/skills'], mcp: true, color: '#D97757' });
  });

  it('新服务端给的换上：名单、名称、颜色跟着它', () => {
    setAgentMeta([
      { id: 'claude', name: 'Claude', protocols: ['anthropic'], efforts: [], skills: ['~/.claude/skills'], mcp: true, color: '#000000' },
      { id: 'neo', name: 'Neo', protocols: [], efforts: [], skills: ['~/.neo/skills'], mcp: true, color: '#111111' },
    ]);
    expect(mcpAgents()).toEqual(['claude', 'neo']);
    expect(builtinName('claude')).toBe('Claude');
    expect(skillHome('neo')).toBe('~/.neo/skills');
    expect(agentMark('claude').color).toBe('#000000');
    // 空的不换
    setAgentMeta([]);
    expect(builtinName('neo')).toBe('Neo');
  });
});
