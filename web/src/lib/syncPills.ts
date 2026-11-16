/* ==========================================================================
   共用配置（skills、MCP）在每台机器上写得怎么样：一台机器一个胶囊（PRD §10.12「共用配置」）
   - 机器上 skills、MCP 各有一个开关：用服务器的 / 不改。不改的机器写「不改」（board-skills 不管开关都装）
   - 用服务器的：按最近一次同步配置的结果（每台机器 skills、MCP 各一行）合成：
     failed 写入失败 > warn 本机同名的是软链接、没覆盖 > pending 正在写入 > synced 已写入；
     离线的机器 offline（悬停写上次的结果），这台机器上没有会用它的 agent 时 none
   - 旧版 board host（不按请求同步、不上报这一行）不在这里，另写一句
   - Git 凭据（gitSync）：一台机器一个，写「已同步 / 同步失败」
   ========================================================================== */
import { since } from './hosts';
import { forAgent } from './mcp';
import { gitOld } from './hostDetail';
import type { HostInfo, McpServer, SkillInfo } from './types';

export type HostSyncKind = 'synced' | 'pending' | 'warn' | 'failed' | 'offline' | 'none' | 'off';
export type HostSync = { host: HostInfo; kind: HostSyncKind; word: string; tip: string };

const AGENT_NAMES: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', pi: 'pi', dsh: 'DeepSeek Harness' };
export const syncAgentName = (h: HostInfo | null, id: string) => h?.agents.find(a => a.id === id)?.name ?? AGENT_NAMES[id] ?? id;

/** 会按请求同步、上报结果的机器（旧版 board host 不算） */
export const syncAble = (h: HostInfo) => !!h.sync?.able;

function pill(h: HostInfo, on: boolean, state: string | undefined, why: string | null | undefined, ok: string, now: number, extra?: { warn?: string }): HostSync {
  const last = h.sync?.last;
  const when = last ? `最近一次同步 ${since(last.at, now)}` : '还没同步过';
  if (!on) return { host: h, kind: 'off', word: '不改', tip: '这台机器选了「不改」：Board 不碰它' };
  if (!h.online) return { host: h, kind: 'offline', word: '离线', tip: `上线后写入。${when}` };
  if (h.sync?.ask) return { host: h, kind: 'pending', word: '正在写入', tip: 'board host 收到同步请求，写完上报' };
  if (state === 'failed') return { host: h, kind: 'failed', word: '失败', tip: why ?? '没写进去' };
  if (extra?.warn) return { host: h, kind: 'warn', word: '软链接没覆盖', tip: extra.warn };
  if (state === 'synced') return { host: h, kind: 'synced', word: '已写入', tip: `${ok}。${when}` };
  return { host: h, kind: 'pending', word: '正在写入', tip: '等 board host 写入' };
}

/** 一个 skill 在各台机器上（不含旧版 board host）。board-skills 不管开关都装 */
export const skillSync = (hosts: HostInfo[], s: SkillInfo, now = Date.now()): HostSync[] =>
  hosts.filter(syncAble).map(h => {
    const r = h.sync?.last?.skills;
    const on = !!s.builtin || !!h.syncSkills;
    const link = r?.links.includes(s.name) ? `本机的 ${s.name} 是软链接，没有覆盖（开发中的 skill）` : undefined;
    const dirs = r?.dirs.length ? `写进了 ${r.dirs.join('、')}` : '已写入';
    // 开关是「不改」时 board-skills 照样装：结果里的 state 是 off，算已写入
    const state = s.builtin && r?.state === 'off' ? 'synced' : r?.state;
    return pill(h, on, state, r?.why, dirs, now, { warn: link });
  });

/** 一个 MCP 服务器在各台机器上（不含旧版 board host）。这台机器上没有会用它的 agent 时写「没有要写的 agent」 */
export const mcpSync = (hosts: HostInfo[], s: McpServer, now = Date.now()): HostSync[] =>
  hosts.filter(syncAble).map(h => {
    if (h.syncMcp && !h.agents.some(a => a.found && forAgent(s, a.id))) return { host: h, kind: 'none', word: '没有要写的 agent', tip: '这台机器上没找到会用它的 agent' };
    const r = h.sync?.last?.mcp;
    return pill(h, !!h.syncMcp, r?.state, r?.why, r?.agents.length ? `写进了 ${r.agents.join('、')}` : '已写入', now);
  });

/** Git 凭据在各台机器上（不含旧版 board host）：已同步（配进了几个仓库）/ 同步失败（原因）/ 正在同步 / 不改 */
export const gitSync = (hosts: HostInfo[], now = Date.now()): HostSync[] =>
  hosts
    .filter(h => syncAble(h) && !gitOld(h))
    .map(h => {
      const r = h.sync?.last?.git;
      const ok = r?.repos.length ? `配进了 ${r.repos.length} 个仓库` : '令牌已就绪，还没有用这个主机的仓库';
      const p = pill(h, !!h.syncGit, r?.state === 'none' ? '' : r?.state, r?.why, ok, now);
      const word: Partial<Record<HostSyncKind, string>> = { synced: '已同步', failed: '同步失败', pending: '正在同步' };
      return { ...p, word: word[p.kind] ?? p.word, tip: p.kind === 'off' ? '这台机器选了「不改」：用它自己的 git 凭据' : p.tip };
    });
