import { describe, expect, it } from 'vitest';
import { argsOf, blankMcp, forAgent, mcpError, mcpRows, mcpSummary, toSave } from './mcp';
import type { HostInfo, McpServer } from './types';

const srv = (x: Partial<McpServer> = {}): McpServer => ({ ...blankMcp(), name: 'db', command: 'npx', args: ['-y', 'db-mcp'], ...x });
const host = (name: string, x: Partial<HostInfo>): HostInfo =>
  ({
    id: 1,
    name,
    online: true,
    agents: [
      { id: 'claude', name: 'Claude Code', found: true, version: '1' },
      { id: 'codex', name: 'Codex', found: true, version: '1' },
      { id: 'dsh', name: 'dsh', found: true, version: '1' },
    ],
    ...x,
  }) as HostInfo;

describe('MCP 共用配置', () => {
  it('状态：旧版不同步；该同步还没报的是正在同步；有同名没覆盖算有问题', () => {
    const rows = mcpRows(
      [
        host('old', { mcp: null }),
        host('new', { mcp: [] }),
        host('ok', { mcp: [{ server: 'db', agent: 'claude', state: 'synced' }, { server: 'db', agent: 'codex', state: 'synced' }, { server: 'db', agent: 'dsh', state: 'unsupported' }] }),
        host('mine', { mcp: [{ server: 'db', agent: 'claude', state: 'local', why: '已有同名' }, { server: 'db', agent: 'codex', state: 'synced' }] }),
      ],
      srv(),
    );
    expect(rows.map(r => (r.old ? 'old' : r.cells.map(c => `${c.agent}:${c.state}`).join()))).toEqual([
      'old',
      'claude:pending,codex:pending',
      'claude:synced,codex:synced,dsh:unsupported',
      'claude:local,codex:synced',
    ]);
    expect(mcpSummary(rows)).toEqual({ synced: 1, bad: 1, old: 1 });
    expect(forAgent(srv(), 'pi') && !forAgent(srv(), 'dsh')).toBe(true);
    expect(forAgent(srv({ agents: ['codex'] }), 'claude')).toBe(false);
  });
  it('校验和保存：密钥留空保留原值、新密钥必须填、参数一行一个', () => {
    expect(argsOf('-y\n  db-mcp \n\n')).toEqual(['-y', 'db-mcp']);
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
