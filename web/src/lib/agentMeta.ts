/* ==========================================================================
   内置 agent 的说明（default/main#399）：服务端 board-core 的 agentdef.rs 是唯一的一份，随 GET /api/providers 的 meta 下发
   - 名称、认哪些接口、强度、读用户级 skill 的目录、支不支持 MCP 同步、主色
   - 取到之前、连旧服务端（meta 没有 skills / mcp / color）时用这里的兜底：和服务端现在的值一样，界面上只有这一份
   - 图标、动作这些画法在 hosts.ts 的 MARKS 里；主色用这里的
   ========================================================================== */
import { create } from 'zustand';
import type { AgentMeta } from './types';

/** 兜底：和服务端 agentdef.rs 一致，顺序就是默认 agent 的顺序 */
const FALLBACK: Required<AgentMeta>[] = [
  { id: 'dsh', name: 'DeepSeek Harness', protocols: ['anthropic', 'openai-chat', 'openai-responses'], efforts: ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'], skills: ['~/.agents/skills'], mcp: false, color: '#4D6BFE' },
  { id: 'claude', name: 'Claude Code', protocols: ['anthropic'], efforts: ['low', 'medium', 'high', 'xhigh', 'max'], skills: ['~/.claude/skills'], mcp: true, color: '#D97757' },
  { id: 'codex', name: 'Codex', protocols: ['openai-responses'], efforts: ['minimal', 'low', 'medium', 'high', 'xhigh', 'max'], skills: ['$CODEX_HOME/skills'], mcp: true, color: '#10A37F' },
];

/** 服务端给的说明叠在兜底上：旧服务端没有的字段（skills、mcp、color）用兜底的 */
export function mergeMeta(server: AgentMeta[] | null | undefined): Required<AgentMeta>[] {
  if (!server?.length) return FALLBACK;
  return server.map(m => {
    const f = FALLBACK.find(x => x.id === m.id);
    return {
      id: m.id,
      name: m.name || f?.name || m.id,
      protocols: m.protocols ?? f?.protocols ?? [],
      efforts: m.efforts ?? f?.efforts ?? [],
      skills: m.skills ?? f?.skills ?? [],
      mcp: m.mcp ?? f?.mcp ?? false,
      color: m.color || f?.color || '',
    };
  });
}

export const useAgentMeta = create<{ list: Required<AgentMeta>[] }>(() => ({ list: FALLBACK }));

/** 取到了服务端的说明（GET /api/providers）：换上 */
export const setAgentMeta = (server: AgentMeta[] | null | undefined) => {
  if (server?.length) useAgentMeta.setState({ list: mergeMeta(server) });
};

const metas = () => useAgentMeta.getState().list;
const metaOf = (id: string) => metas().find(m => m.id === id);

/** 支持 MCP 同步的内置 agent（默认同步给它们），按默认 agent 的顺序 */
export const mcpAgents = (list = metas()) => list.filter(m => m.mcp).map(m => m.id);
/** 内置 agent 的名称；不认识的原样 */
export const builtinName = (id: string) => metaOf(id)?.name ?? id;
/** 读用户级 skill 的第一个目录（装 board-skills 的地方）；不认识的 agent 为空 */
export const skillHome = (id: string) => metaOf(id)?.skills[0];
/** 主色；不认识的为空 */
export const metaColor = (id: string) => metaOf(id)?.color || undefined;
