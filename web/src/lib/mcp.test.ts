import { describe, expect, it } from 'vitest';
import { blankMcp, forAgent, mcpError, toSave } from './mcp';
import type { McpServer } from './types';

const srv = (x: Partial<McpServer> = {}): McpServer => ({ ...blankMcp(), name: 'db', command: 'npx', args: ['-y', 'db-mcp'], ...x });
describe('MCP 共用配置', () => {
  it('同步给谁：空 = 所有支持的（claude、codex），dsh 不支持', () => {
    expect(forAgent(srv(), 'codex') && !forAgent(srv(), 'dsh')).toBe(true);
    expect(forAgent(srv({ agents: ['codex'] }), 'claude')).toBe(false);
  });
  it('校验和保存：密钥留空保留原值、新密钥必须填', () => {
    expect(mcpError(srv({ name: 'bad name' }), [], false)).toContain('名称');
    expect(mcpError(srv(), ['db'], false)).toContain('已经有');
    expect(mcpError(srv(), ['db'], true)).toBeNull();
    expect(mcpError(srv({ command: ' ' }), [], false)).toContain('命令');
    expect(mcpError(srv({ kind: 'http', url: 'ftp://x' }), [], false)).toContain('地址');
    expect(mcpError(srv({ env: [{ key: 'T', value: '', secret: true }] }), [], false)).toContain('还没有值');
    expect(mcpError(srv({ env: [{ key: 'T', value: '', secret: true, tail: 'abcd' }] }), [], true)).toBeNull();
    expect(mcpError(srv({ env: [{ key: 'A', value: '1' }, { key: 'A', value: '2' }] }), [], false)).toContain('两次');
    const s = toSave(srv({ env: [{ key: ' T ', value: '', secret: true, tail: 'abcd' }, { key: 'U', value: 'x' }] }));
    expect(s.env).toEqual([
      { key: 'T', value: null, secret: true },
      { key: 'U', value: 'x', secret: false },
    ]);
  });
});
