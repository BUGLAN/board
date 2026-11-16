/* ==========================================================================
   同步到机器的配置在每台机器上写得怎么样（PRD §10.16「共用配置」）：按 syncKinds.ts 登记的种类取文案
   - 每类走到哪一步按同一个顺序判断（syncStep）：不改 > 离线 > 收到请求正在写 > 失败 > 写好了 > 还没报上来
   - 一台机器一个胶囊（skills、MCP 区块里的每一项）：不改的机器写「不改」（board-skills 不管开关都装）；
     failed 写入失败 > warn 本机同名的是软链接、没覆盖 > pending 正在写入 > synced 已写入；
     离线的机器 offline（悬停写上次的结果），这台机器上没有会用它的 agent 时 none
   - 一台机器一行（syncRow，Git 凭据卡片）：写「已同步 · 配进了几个仓库 / 同步失败：原因 / 不同步」，旁边是开关
   - 旧版 board host（不按请求同步、不上报这一类）不在这里，另写一句
   ========================================================================== */
import { builtinName } from './agentMeta';
import { since } from './hosts';
import { forAgent } from './mcp';
import { syncKind, syncOld, type SyncKind } from './syncKinds';
import type { HostInfo, McpServer, SkillInfo } from './types';

export type HostSyncKind = 'synced' | 'pending' | 'warn' | 'failed' | 'offline' | 'none' | 'off';
export type HostSync = { host: HostInfo; kind: HostSyncKind; word: string; tip: string };

export const syncAgentName = (h: HostInfo | null, id: string) => h?.agents.find(a => a.id === id)?.name ?? builtinName(id);

/** 会按请求同步、上报结果的机器（旧版 board host 不算） */
export const syncAble = (h: HostInfo) => !!h.sync?.able;

/** 一类同步在一台机器上走到哪一步：不改 / 离线 / 收到请求正在写 / 失败 / 写好了 / 还没报上来 */
export type SyncStep = 'off' | 'offline' | 'asked' | 'failed' | 'synced' | 'wait';

export function syncStep(h: HostInfo, on: boolean, state: string | undefined): SyncStep {
  if (!on) return 'off';
  if (!h.online) return 'offline';
  if (h.sync?.ask) return 'asked';
  if (state === 'failed') return 'failed';
  return state === 'synced' ? 'synced' : 'wait';
}

/** 一类同步在一台机器上的胶囊。开关、状态默认按这一类的；单个 skill、MCP 服务器有自己的例外时传进来（board-skills 不管开关都装、软链接没覆盖） */
function pill(k: SyncKind, h: HostInfo, now: number, o: { on?: boolean; state?: string; warn?: string } = {}): HostSync {
  const last = h.sync?.last;
  const r = last?.[k.key];
  const when = last ? `最近一次同步 ${since(last.at, now)}` : '还没同步过';
  const step = syncStep(h, o.on ?? !!h[k.flag], o.state ?? r?.state);
  if (step === 'off') return { host: h, kind: 'off', word: '不改', tip: '这台机器选了「不改」：Board 不碰它' };
  if (step === 'offline') return { host: h, kind: 'offline', word: '离线', tip: `上线后写入。${when}` };
  if (step === 'asked') return { host: h, kind: 'pending', word: '正在写入', tip: 'board host 收到同步请求，写完上报' };
  if (step === 'failed') return { host: h, kind: 'failed', word: '失败', tip: r?.why ?? '没写进去' };
  if (o.warn) return { host: h, kind: 'warn', word: '软链接没覆盖', tip: o.warn };
  if (step === 'synced' && last) return { host: h, kind: 'synced', word: '已写入', tip: `${k.where(last)}。${when}` };
  return { host: h, kind: 'pending', word: '正在写入', tip: '等 board host 写入' };
}

const SKILLS = syncKind('skills');
const MCP = syncKind('mcp');

/** 一个 skill 在各台机器上（不含旧版 board host）。board-skills 不管开关都装 */
export const skillSync = (hosts: HostInfo[], s: SkillInfo, now = Date.now()): HostSync[] =>
  hosts.filter(syncAble).map(h => {
    const r = h.sync?.last?.skills;
    const link = r?.links.includes(s.name) ? `本机的 ${s.name} 是软链接，没有覆盖（开发中的 skill）` : undefined;
    // 开关是「不改」时 board-skills 照样装：结果里的 state 是 off，算已写入
    const state = s.builtin && r?.state === 'off' ? 'synced' : r?.state;
    return pill(SKILLS, h, now, { on: !!s.builtin || !!h.syncSkills, state, warn: link });
  });

/** 一个 MCP 服务器在各台机器上（不含旧版 board host）。这台机器上没有会用它的 agent 时写「没有要写的 agent」 */
export const mcpSync = (hosts: HostInfo[], s: McpServer, now = Date.now()): HostSync[] =>
  hosts.filter(syncAble).map(h => {
    if (h.syncMcp && !h.agents.some(a => a.found && forAgent(s, a.id))) return { host: h, kind: 'none', word: '没有要写的 agent', tip: '这台机器上没找到会用它的 agent' };
    return pill(MCP, h, now);
  });

/** 一台机器一行的区块（Git 凭据卡片）上列出的机器：会按请求同步、也报这一类的（旧版 board host 另写一句） */
export const syncHosts = (k: SyncKind, hosts: HostInfo[]): HostInfo[] => hosts.filter(h => syncAble(h) && !syncOld(k, h));

/** 一台机器一行：状态点和一句话。不同步 / 离线 / 正在同步 / 同步失败（原因）/ 等 board host / 已同步（配进了几个仓库）· 多久前 */
export function syncRow(k: SyncKind, h: HostInfo, now = Date.now()): { kind: HostSyncKind; text: string } {
  const w = k.row;
  const last = h.sync?.last;
  const r = last?.[k.key];
  if (!w) return { kind: 'none', text: '' };
  const step = syncStep(h, !!h[k.flag], r?.state);
  if (step === 'off') return { kind: 'off', text: w.off };
  if (step === 'offline') return { kind: 'offline', text: w.offline(last ?? undefined) };
  if (step === 'asked') return { kind: 'pending', text: w.asked };
  if (step === 'failed') return { kind: 'failed', text: `${k.line.failed}：${r?.why ?? '原因没报上来'}` };
  if (step !== 'synced' || !last) return { kind: 'pending', text: k.line.wait };
  return { kind: 'synced', text: `${w.ok(last)} · ${since(last.at, now)}` };
}
