/* 角色（PRD §9.1「分工」）界面上要算的东西：规则命中哪个角色、角色在每台机器上现在怎么样、一张卡会交给谁。
   纯函数，不碰界面和接口。能不能跑（装没装 agent、在不在角色的机器里）是服务端算好的（`HostInfo.fits`），
   这里只叠上「现在能不能开始」：在线、暂停、接单时段（只有自动领取看）、空位。
   推算和服务端自动领取同一套顺序：可领取状态里的卡片，带优先标签的在前；一张卡按第一条命中的规则交给角色，
   在能跑它、现在能开始的机器里挑空位最多的，挑不到就在这个角色里排队 */
import { dispatchesOf, hostLoad } from './hosts';
import { inWins, nextStart, serverMinute, ALL_DAY } from './sched';
import type { BoardSnapshot, ClaimRule, Dispatch, HostInfo, Profile, Workspace } from './types';

export const profilesOf = (ws: Pick<Workspace, 'sched'> | null | undefined): Profile[] => ws?.sched?.profiles ?? [];

/** 规则的条件对不对得上卡片的标签（都按小写比）：和服务端 rule_matches 一样 */
export function ruleMatches(op: ClaimRule['op'], labels: string[], card: string[]): boolean {
  const want = labels.map(l => l.trim().toLowerCase());
  const got = card.map(l => l.toLowerCase());
  switch (op) {
    case 'all':
      return want.length > 0 && want.every(l => got.includes(l));
    case 'not_in':
      return !want.some(l => got.includes(l));
    case 'equal': {
      const a = [...new Set(got)].sort();
      const b = [...new Set(want)].sort();
      return a.length === b.length && a.every((x, i) => x === b[i]);
    }
    case 'only':
      return want.length > 0 && got.length > 0 && got.every(l => want.includes(l));
    case 'none':
      return got.length === 0;
    default:
      return want.some(l => got.includes(l));
  }
}

/** 一条规则对这些标签命不命中，以及为什么（卡片「接下来」的「为什么」里用） */
export function ruleWhy(r: Pick<ClaimRule, 'op' | 'labels'>, names: string[]): { hit: boolean; why: string } {
  const has = (l: string) => names.some(n => n.toLowerCase() === l.toLowerCase());
  const q = (l: string[]) => l.map(x => `「${x}」`).join('');
  switch (r.op) {
    case 'all': {
      const miss = r.labels.filter(l => !has(l));
      return miss.length ? { hit: false, why: `没带${q(miss)}` } : { hit: true, why: '都带了' };
    }
    case 'not_in': {
      const got = r.labels.filter(has);
      return got.length ? { hit: false, why: `带了${q(got.slice(0, 1))}` } : { hit: true, why: '都没带' };
    }
    case 'none':
      return names.length ? { hit: false, why: '有标签' } : { hit: true, why: '没有标签' };
    case 'equal':
      return ruleMatches('equal', r.labels, names) ? { hit: true, why: '标签正好是这几个' } : { hit: false, why: '标签不是正好这几个' };
    case 'only':
      return ruleMatches('only', r.labels, names) ? { hit: true, why: '标签都在这几个里' } : { hit: false, why: '有别的标签' };
    default: {
      const got = r.labels.filter(has);
      return got.length ? { hit: true, why: `带${q(got.slice(0, 1))}` } : { hit: false, why: `${q(r.labels)}都没带` };
    }
  }
}

/** 第一条命中的规则；都没命中时为空（不自动领） */
export function routeOf(names: string[], rules: ClaimRule[]): { rule: number; profile: string } | null {
  const i = rules.findIndex(r => ruleMatches(r.op, r.labels, names));
  return i >= 0 ? { rule: i, profile: rules[i].profile } : null;
}

/** 这台机器能不能跑这个角色（服务端算好的；旧服务端没有时当作能跑） */
export function fitOf(h: HostInfo, profile: string): { ok: boolean; why: string } {
  const f = h.fits?.find(x => x.profile === profile);
  if (!f) return { ok: !h.fits, why: h.fits ? '这台机器还不知道这个角色' : '' };
  return { ok: f.ok, why: f.why ?? '' };
}

export type Tone = 'ok' | 'busy' | 'off' | 'bad';

/** 一个角色在一台机器上现在怎么样 */
export interface HostWhy {
  host: HostInfo;
  /** 能跑这个角色 */
  can: boolean;
  /** 现在就能开始 */
  ok: boolean;
  why: string;
  tone: Tone;
  free: number;
}

/** 机器现在能不能开新会话：auto = 按自动领取算（看「自动领」和接单时段）；used = 这次推算里已经分给它的 */
export function slotOf(ws: Workspace | null, h: HostInfo, minute: number, auto: boolean, used = 0): Omit<HostWhy, 'host' | 'can'> {
  if (!h.online) return { ok: false, why: '离线', tone: 'off', free: 0 };
  if (h.paused) return { ok: false, why: '暂停接单', tone: 'busy', free: 0 };
  if (auto) {
    const wins = h.windows ?? ALL_DAY;
    if (!h.auto) return { ok: false, why: '只做派给它的', tone: 'off', free: 0 };
    if (!wins.length) return { ok: false, why: '没有接单时段', tone: 'off', free: 0 };
    if (!inWins(wins, minute)) return { ok: false, why: `时段外，${nextStart(wins, minute)} 开始接单`, tone: 'busy', free: 0 };
  }
  const run = hostLoad(ws, h).running.length + used;
  if (run >= h.max) return { ok: false, why: `满 ${run}/${h.max}`, tone: 'busy', free: 0 };
  return { ok: true, why: `空 ${h.max - run} 位`, tone: 'ok', free: h.max - run };
}

/** 一个角色在每台机器上现在怎么样 */
export function hostsFor(ws: Workspace | null, profile: string, minute: number, auto: boolean, used: Record<string, number> = {}): HostWhy[] {
  return (ws?.hosts ?? []).map(h => {
    const f = fitOf(h, profile);
    if (!f.ok) return { host: h, can: false, ok: false, why: f.why, tone: 'bad', free: 0 };
    return { host: h, can: true, ...slotOf(ws, h, minute, auto, used[h.name] ?? 0) };
  });
}

/** 在能马上开始的机器里挑一台：空位多的优先，一样多挑最久没接的（看最近一次派发），再按名字 */
export function pickHost(ws: Workspace | null, list: HostWhy[]): HostWhy | null {
  const last = (h: HostInfo) => Math.max(0, ...dispatchesOf(ws).filter(d => d.hostId === h.id).map(d => d.startedAt ?? d.createdAt));
  return list.filter(x => x.ok).sort((a, b) => b.free - a.free || last(a.host) - last(b.host) || a.host.name.localeCompare(b.host.name))[0] ?? null;
}

/** 排队时最快什么时候开始 */
export function soonest(list: HostWhy[]): string {
  const can = list.filter(x => x.can);
  if (!can.length) return '没有机器能跑这个角色';
  const busy = can.filter(x => x.why.startsWith('满')).map(x => x.host.name);
  const later = can.filter(x => x.why.startsWith('时段外')).map(x => `${x.host.name} ${x.why.replace('时段外，', '')}`);
  const parts = [busy.length ? `${busy.join('、')} 空出来就开始` : '', ...later].filter(Boolean);
  return parts.length ? parts.join('；') : `能跑的 ${can.map(x => x.host.name).join('、')} 现在都接不了`;
}

/** 一张可领取的卡会怎么走 */
export interface Fc {
  card: number;
  title: string;
  names: string[];
  route: { rule: number; profile: string } | null;
  /** start 马上开始 · wait 排队 · none 没命中规则、不自动领 · off 这块白板不参与自动领取 */
  kind: 'start' | 'wait' | 'none' | 'off';
  host?: string;
  /** 排队：这个角色里排第几 */
  pos?: number;
  hosts: HostWhy[];
  soon?: string;
}

/** 卡片的标签名 */
export function labelNames(ws: Workspace | null, board: string, ids: number[]): string[] {
  const proj = board.split('/')[0];
  const labels = ws?.projects.find(p => p.key === proj)?.labels ?? [];
  return ids.map(id => labels.find(l => l.id === id)?.name).filter((x): x is string => !!x);
}

/** 一块白板上会被自动领取的卡片（可领取状态里、没人持有、没在派发），按领取的先后：带优先标签的在前，再按状态和状态里的顺序 */
export function claimableCards(ws: Workspace | null, board: BoardSnapshot | null): { id: number; title: string; names: string[] }[] {
  if (!board?.path) return [];
  const proj = board.path.split('/')[0];
  const labels = ws?.projects.find(p => p.key === proj)?.labels ?? [];
  const busy = new Set(dispatchesOf(ws).filter(d => d.board === board.path).map(d => d.card));
  const all = board.statuses
    .filter(s => s.claim)
    .flatMap(s => board.cols[s.key] ?? [])
    .map(id => board.cards[id])
    .filter(c => c && !c.holder && !busy.has(c.id));
  const hot = (ids: number[]) => ids.some(id => labels.find(l => l.id === id)?.priority);
  return [...all.filter(c => hot(c.labels)), ...all.filter(c => !hot(c.labels))].map(c => ({ id: c.id, title: c.title, names: labelNames(ws, board.path, c.labels) }));
}

/** 推算一块白板上可领取的卡各自会怎么走（自动领取：看接单时段）。白板不参与自动领取时每张都是 `off` */
export function forecast(ws: Workspace | null, board: BoardSnapshot | null, now: number): Map<number, Fc> {
  const out = new Map<number, Fc>();
  const profiles = profilesOf(ws);
  // 还没有角色（没配过分工）：不推算
  if (!ws || !board?.path || !profiles.length) return out;
  const rules = ws.sched?.rules ?? [];
  const minute = serverMinute(ws, now);
  const auto = ws.projects.flatMap(p => p.boards).find(b => b.path === board.path)?.auto !== false;
  const used: Record<string, number> = {};
  const queue: Record<string, number> = {};
  for (const c of claimableCards(ws, board)) {
    const route = routeOf(c.names, rules);
    const p = route && profiles.find(x => x.id === route.profile);
    if (!route || !p) {
      out.set(c.id, { card: c.id, title: c.title, names: c.names, route: null, kind: 'none', hosts: [] });
      continue;
    }
    const list = hostsFor(ws, p.id, minute, true, used);
    if (!auto) {
      out.set(c.id, { card: c.id, title: c.title, names: c.names, route, kind: 'off', hosts: list });
      continue;
    }
    const got = pickHost(ws, list);
    if (got) {
      used[got.host.name] = (used[got.host.name] ?? 0) + 1;
      out.set(c.id, { card: c.id, title: c.title, names: c.names, route, kind: 'start', host: got.host.name, hosts: list });
    } else {
      queue[p.id] = (queue[p.id] ?? 0) + 1;
      out.set(c.id, { card: c.id, title: c.title, names: c.names, route, kind: 'wait', pos: queue[p.id], hosts: list, soon: soonest(list) });
    }
  }
  return out;
}

/** 一个角色在跑的派发（按派发上记的角色） */
export const runningOf = (ws: Workspace | null, profile: string): Dispatch[] => dispatchesOf(ws).filter(d => d.state === 'running' && d.profile === profile);

/** 「Claude Code · relay · model-a · high」 */
export function recipe(p: Pick<Profile, 'agent' | 'provider' | 'model' | 'effort'>, agentName: (id: string) => string): string {
  return [agentName(p.agent), p.provider || '本机登录', p.model, p.effort].filter(Boolean).join(' · ');
}

/** 「model-a · high」：节点上一行写不下全部时用 */
export function short(p: Pick<Profile, 'model' | 'effort' | 'provider'>): string {
  return [p.model || (p.provider ? '' : '默认模型'), p.effort].filter(Boolean).join(' · ');
}

/** 规则里的条件词 */
export const OP_WORDS: Record<ClaimRule['op'], string> = { in: '带', all: '同时带', not_in: '不带', equal: '标签正好是', only: '只带这几个里的', none: '没有标签' };
