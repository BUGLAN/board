/* ==========================================================================
   MCP 服务器共用配置（PRD §10.16「共用配置」）：界面上用到的纯函数
   - 编辑表单：空白的一项、把表单整理成要保存的（参数一行一个、密钥留空保留原值）、校验（和服务端一致）
   ========================================================================== */
import { mcpAgents } from './agentMeta';
import type { McpServer, McpVar } from './types';

/** 这个 MCP 服务器同步给这个 agent：选了的；没选（空）= 所有支持 MCP 的内置 agent（agentMeta.ts） */
export const forAgent = (s: Pick<McpServer, 'agents'>, agent: string) => (s.agents.length ? s.agents.includes(agent) : mcpAgents().includes(agent));

export const blankMcp = (): McpServer => ({ name: '', kind: 'stdio', command: '', args: [], env: [], url: '', headers: [], agents: [] });

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
