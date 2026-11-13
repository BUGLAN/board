/* ==========================================================================
   共用配置（skills、MCP）在每台机器上同步得怎么样：一台机器合成一个状态（PRD §10.12「共用配置」）
   - 输入是 skillRows / mcpRows 里一台机器的每个 agent 的状态，合成：
     failed 有 agent 失败 > warn 有 agent 版本落后、有同名没覆盖 > pending 还有 agent 正在同步 > synced 全部已同步；
     离线的机器 offline（悬停写上次上报的），这台机器上没有要同步的 agent 时 none
   - 旧版 board host（不同步）不在这里，另写一句
   ========================================================================== */
import { mcpRows } from './mcp';
import { skillRows } from './skills';
import type { HostInfo, McpServer, SkillInfo } from './types';

export type HostSyncKind = 'synced' | 'pending' | 'warn' | 'failed' | 'offline' | 'none';
export type HostSyncCell = { agent: string; state: string; word: string; why?: string | null };
export type HostSync = { host: HostInfo; kind: HostSyncKind; word: string; tip: string; cells: HostSyncCell[] };

const AGENT_NAMES: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', pi: 'pi', dsh: 'DeepSeek Harness' };
export const syncAgentName = (h: HostInfo | null, id: string) => h?.agents.find(a => a.id === id)?.name ?? AGENT_NAMES[id] ?? id;

const SKILL_WORDS: Record<string, string> = { synced: '已同步', behind: '版本落后', local: '有同名没覆盖', failed: '失败', pending: '正在同步' };
const MCP_WORDS: Record<string, string> = { synced: '已同步', local: '有同名没覆盖', failed: '失败', pending: '正在同步' };

/** 一台机器的各个 agent 合成一个状态 */
export function hostSync(h: HostInfo, cells: HostSyncCell[]): HostSync {
  const has = (...s: string[]) => cells.some(c => s.includes(c.state));
  const tip = cells.map(c => `${syncAgentName(h, c.agent)}：${c.word}${c.why ? `（${c.why}）` : ''}`).join('\n');
  if (!h.online) return { host: h, kind: 'offline', word: '离线', tip: tip ? `上次上报：\n${tip}` : '离线', cells };
  if (!cells.length) return { host: h, kind: 'none', word: '没有要同步的 agent', tip: '这台机器上没找到会用它的 agent', cells };
  if (has('failed')) {
    const who = cells.filter(c => c.state === 'failed').map(c => syncAgentName(h, c.agent));
    return { host: h, kind: 'failed', word: `${who.join('、')} 失败`, tip, cells };
  }
  const warn = cells.find(c => c.state === 'behind' || c.state === 'local');
  if (warn) return { host: h, kind: 'warn', word: `${syncAgentName(h, warn.agent)} ${warn.word}`, tip, cells };
  if (has('pending')) return { host: h, kind: 'pending', word: '正在同步', tip, cells };
  return { host: h, kind: 'synced', word: '已同步', tip, cells };
}

/** 一个 skill 在各台机器上（不含旧版 board host）。还没报这个 skill 的机器，每个找到的 agent 都算正在同步 */
export const skillSync = (hosts: HostInfo[], s: SkillInfo): HostSync[] =>
  skillRows(hosts, s)
    .filter(r => !r.old)
    .map(r =>
      hostSync(
        r.host,
        r.cells.some(c => !c.agent)
          ? r.host.agents.filter(a => a.found).map(a => ({ agent: a.id, state: 'pending', word: SKILL_WORDS.pending }))
          : r.cells.map(c => ({ agent: c.agent, state: c.state, word: SKILL_WORDS[c.state] ?? c.state, why: c.why })),
      ),
    );

/** 一个 MCP 服务器在各台机器上（不含旧版 board host）。「不支持」的 agent（dsh、配置文件里的）不算 */
export const mcpSync = (hosts: HostInfo[], s: McpServer): HostSync[] =>
  mcpRows(hosts, s)
    .filter(r => !r.old)
    .map(r =>
      hostSync(
        r.host,
        r.cells.filter(c => c.state !== 'unsupported').map(c => ({ agent: c.agent, state: c.state, word: MCP_WORDS[c.state] ?? c.state, why: c.why })),
      ),
    );
