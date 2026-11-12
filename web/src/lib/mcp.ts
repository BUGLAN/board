/* ==========================================================================
   MCP 服务器共用配置（PRD §10.12「共用配置」）：界面上用到的纯函数
   - 每个服务器在各台机器、各个 agent 上的同步状态（mcpRows）
   - 编辑表单：空白的一项、把表单整理成要保存的（参数一行一个、密钥留空保留原值）、校验（和服务端一致）
   ========================================================================== */
import type { HostInfo, McpServer, McpState, McpVar } from './types';

/** 支持同步 MCP 的内置 agent（和服务端一致） */
export const MCP_AGENTS = ['claude', 'codex', 'pi'] as const;
export const MCP_AGENT_NAMES: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', pi: 'pi' };

export type McpCell = { agent: string; state: McpState['state'] | 'pending'; why?: string | null };
export type McpRow = { host: HostInfo; old: boolean; cells: McpCell[] };

export const MCP_WORDS: Record<McpCell['state'], string> = {
  synced: '已同步',
  local: '有同名，没有覆盖',
  unsupported: '不支持',
  failed: '失败',
  pending: '正在同步',
};

/** 一个服务器在每台机器上的状态：旧版 board host 不同步（old）；该同步、还没报的 agent 是「正在同步」 */
export function mcpRows(hosts: HostInfo[], s: McpServer): McpRow[] {
  return hosts.map(h => {
    if (h.mcp == null) return { host: h, old: true, cells: [] };
    const cells: McpCell[] = h.mcp.filter(x => x.server === s.name).map(x => ({ agent: x.agent, state: x.state, why: x.why }));
    if (h.online) {
      for (const a of h.agents.filter(a => a.found && forAgent(s, a.id))) {
        if (!cells.some(c => c.agent === a.id)) cells.push({ agent: a.id, state: 'pending' });
      }
    }
    return { host: h, old: false, cells };
  });
}

export const forAgent = (s: Pick<McpServer, 'agents'>, agent: string) => (s.agents.length ? s.agents.includes(agent) : (MCP_AGENTS as readonly string[]).includes(agent));

export function mcpSummary(rows: McpRow[]) {
  let synced = 0;
  let bad = 0;
  let old = 0;
  for (const r of rows) {
    if (r.old) old++;
    else if (r.cells.some(c => c.state === 'failed' || c.state === 'local')) bad++;
    else if (r.cells.length && r.cells.every(c => c.state === 'synced' || c.state === 'unsupported')) synced++;
  }
  return { synced, bad, old };
}

export const blankMcp = (): McpServer => ({ name: '', kind: 'stdio', command: '', args: [], env: [], url: '', headers: [], agents: [] });

/** 一行一个参数（空行去掉） */
export const argsOf = (text: string) => text.split('\n').map(s => s.trim()).filter(Boolean);

/** 密钥的显示：保存过的只给末尾 4 位 */
export const maskOf = (v: McpVar) => (v.tail != null ? `••••${v.tail}` : '••••');

/** 校验（和服务端一致）：不对时返回原因 */
export function mcpError(s: McpServer, existing: string[], editing: boolean): string | null {
  if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(s.name)) return '名称只能用字母、数字、_、-，以字母或数字开头';
  if (!editing && existing.includes(s.name)) return `已经有「${s.name}」了：点它修改`;
  if (s.kind === 'stdio' && !s.command.trim()) return '本地命令要填命令';
  if (s.kind === 'http' && !/^https?:\/\//.test(s.url.trim())) return '远程 HTTP 要填以 http:// 或 https:// 开头的地址';
  const vars = s.kind === 'stdio' ? s.env : s.headers;
  const what = s.kind === 'stdio' ? '环境变量' : '请求头';
  const seen = new Set<string>();
  for (const v of vars) {
    const k = v.key.trim();
    if (!k || /[=\n"]/.test(k)) return `${what}的名字不对：「${k}」`;
    if (seen.has(k)) return `${what}「${k}」写了两次`;
    seen.add(k);
    // 密钥留空：保存过的保留原值；新加的必须填
    if (!v.value && !(v.secret && v.tail != null)) return `${what}「${k}」还没有值`;
  }
  return null;
}

/** 要保存的：密钥没改（留空）时 value 为空，服务端保留原值；不带 tail */
export function toSave(s: McpServer): McpServer {
  const clean = (vs: McpVar[]) => vs.map(v => ({ key: v.key.trim(), value: v.value ? v.value : null, secret: !!v.secret }));
  return { ...s, name: s.name.trim(), command: s.command.trim(), url: s.url.trim(), env: clean(s.env), headers: clean(s.headers) };
}
