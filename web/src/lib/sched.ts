/* 调度（PRD §9.1「自动领取」）界面上要算的东西：接单时段的文字和整理、机器现在接不接单。
   纯函数，不碰界面和接口；「现在」是服务端时区一天里的第几分钟（工作区数据里带着），不用浏览器的时区 */
import type { BoardInfo, ClaimRule, HostAgent, HostInfo, Workspace } from './types';

/** 一段接单时段：一天里的 [起, 止)，分钟 */
export type Win = [number, number];
export const DAY = 1440;
/** 时间带上拖动时的一格：半小时 */
export const SNAP = 30;
export const ALL_DAY: Win[] = [[0, DAY]];

export const hhmm = (m: number) => `${String(Math.floor(m / 60)).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`;

/** 几段时段合成一句：整天写「全天」，一段都没有写「不接单」，跨零点的两段（22:00–24:00、00:00–08:00）写成 22:00–08:00 */
export function winWords(list: Win[]): string {
  const v = [...list].sort((a, b) => a[0] - b[0]);
  if (v.length === 0) return '不接单';
  if (v.length === 1 && v[0][0] === 0 && v[0][1] === DAY) return '全天';
  const first = v[0];
  const last = v[v.length - 1];
  const wrap = v.length > 1 && first[0] === 0 && last[1] === DAY;
  const parts = (wrap ? v.slice(1, -1) : v).map(([a, b]) => `${hhmm(a)}–${hhmm(b)}`);
  if (wrap) parts.push(`${hhmm(last[0])}–${hhmm(first[1])}`);
  return parts.join('、');
}

/** 去掉空的、排好，挨着和重叠的并成一段（和服务端 tidy_windows 一样） */
export function tidyWins(list: Win[]): Win[] {
  const v = list.filter(([a, b]) => b > a).sort((a, b) => a[0] - b[0]);
  const out: Win[] = [];
  for (const [a, b] of v) {
    const prev = out[out.length - 1];
    if (prev && a <= prev[1]) prev[1] = Math.max(prev[1], b);
    else out.push([a, b]);
  }
  return out;
}

export const inWins = (list: Win[], minute: number) => list.some(([a, b]) => a <= minute && minute < b);

/** 下一次开始接单是几点；一段都没有时为空 */
export function nextStart(list: Win[], minute: number): string | null {
  const starts = list.map(w => w[0]).sort((a, b) => a - b);
  if (!starts.length) return null;
  return hhmm(starts.find(s => s > minute) ?? starts[0]);
}

/** 服务端现在是一天里的第几分钟：取数据那一刻的值加上之后过去的时间（本机和服务端的钟差得离谱时就用取数据时的值） */
export function serverMinute(ws: Pick<Workspace, 'now' | 'sched'> | null, now: number): number {
  const base = ws?.sched?.minute ?? 0;
  const passed = ws ? Math.round((now - ws.now) / 60_000) : 0;
  return Math.abs(passed) > 12 * 60 ? base : (((base + passed) % DAY) + DAY) % DAY;
}

export interface Take {
  ok: boolean;
  /** 能接时写空位，不能接时写原因 */
  why: string;
  /** ok 现在能接 · wait 等一等就能接（时段外、满了、暂停） · off 接不了（离线、没开、没有这个 agent） */
  tone: 'ok' | 'wait' | 'off';
}

/** 机器现在自动接不接单（不分 agent）：running 是它在跑的会话数 */
export function hostTake(h: HostInfo, running: number, minute: number): Take {
  const wins = h.windows ?? ALL_DAY;
  if (!h.online) return { ok: false, why: '离线', tone: 'off' };
  // 接单三态：暂停 > 只做派给它的 > 自动领
  if (h.paused) return { ok: false, why: '暂停接单', tone: 'wait' };
  if (!h.auto) return { ok: false, why: '只做派给它的', tone: 'off' };
  if (wins.length === 0) return { ok: false, why: '没有接单时段', tone: 'off' };
  if (!inWins(wins, minute)) return { ok: false, why: `时段外，${nextStart(wins, minute)} 开始`, tone: 'wait' };
  if (running >= h.max) return { ok: false, why: `${running} 个会话在跑，已满`, tone: 'wait' };
  return { ok: true, why: `空位 ${h.max - running}（${running}/${h.max} 在跑）`, tone: 'ok' };
}

/** 机器现在能不能自动接这个 agent 的卡 */
export function agentTake(h: HostInfo, agent: string, running: number, minute: number): Take {
  const a: HostAgent | undefined = h.agents.find(x => x.id === agent);
  if (!a) return { ok: false, why: '没有这个 agent', tone: 'off' };
  if (!a.found) return { ok: false, why: '没找到命令', tone: 'off' };
  if (a.tripped) return { ok: false, why: '连续异常退出，已暂停', tone: 'off' };
  const t = hostTake(h, running, minute);
  return t.ok ? { ...t, why: `空位 ${h.max - running}` } : t;
}

/** 这块白板上现在能领的卡片数（可领取状态里的卡片；排队、在跑的不在这些状态里） */
export const claimable = (b: BoardInfo) => b.statuses.filter(s => s.claim).reduce((n, s) => n + (b.counts[s.key] ?? 0), 0);

/** 一个 agent 由这些机器做（领取规则里按 agent 一组）：没设置过时所有装了它的机器都参与 */
export interface PickState {
  /** 选中了的机器 id（没设置过时是能选的全部） */
  on: number[];
  /** 一台都没选：这个 agent 不自动领卡 */
  none: boolean;
  /** 没设置过（旧数据、之后才装上的机器）：所有装了它的机器都参与 */
  unset: boolean;
}

/** 这个 agent 现在选了哪些机器。`has` 是装着它的机器：没设置过、旧版服务端没有这一项时都算选中 */
export function pickState(ws: Pick<Workspace, 'sched'> | null | undefined, agent: string, has: number[]): PickState {
  const p = ws?.sched?.picks?.find(x => x.agent === agent);
  if (!p) return { on: [...has], none: false, unset: true };
  if (p.none) return { on: [], none: true, unset: false };
  if (p.unset && !p.hosts?.length) return { on: [...has], none: false, unset: true };
  return { on: p.hosts ?? [...has], none: false, unset: !!p.unset };
}

/** 点一下胶囊：选中就取消，没选中就勾上 */
export const togglePick = (st: PickState, id: number): number[] => (st.on.includes(id) ? st.on.filter(h => h !== id) : [...st.on, id]);

/** 规则里出现的 agent，按第一次出现的先后；兜底的 agent 排在最后（没出现过才加） */
export function ruleAgents(rules: ClaimRule[], fallback?: string | null): string[] {
  const out: string[] = [];
  for (const r of rules) if (!out.includes(r.agent)) out.push(r.agent);
  if (fallback && !out.includes(fallback)) out.push(fallback);
  return out;
}
