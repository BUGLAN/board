/* ==========================================================================
   原型（用完即弃）：工种、规则、机器的假数据和「这张卡会交给谁」的推算。三个分工页变体、派发对话框、卡片上的「接下来」共用
   - 工种 = agent + 供应商·模型（或本机登录）+ 强度 + 附加说明；「我」不派给机器，等人自己开会话
   - 规则 = 条件 → 工种，第一条命中的生效；其余卡片（兜底）不领
   - 机器只说明能跑哪些工种：装了这个 agent、登录了 / 连得上这个供应商
   - 推算：按白板上的顺序一张张过，每张找到工种后在能跑的机器里挑空位最多的（一样多挑最久没接的），
     挑不到就排队，写明每台机器为什么不行。自动领取看接单时段，手动派发不看
   ========================================================================== */
import { create } from 'zustand';
import type { BoardSnapshot, Workspace } from '../../lib/types';

export type AgentId = 'claude' | 'codex' | 'dsh';
export const AGENTS: { id: AgentId; name: string }[] = [
  { id: 'claude', name: 'Claude Code' },
  { id: 'codex', name: 'Codex' },
  { id: 'dsh', name: 'DeepSeek Harness' },
];
export const agentName = (id: AgentId | null) => (id ? (AGENTS.find(a => a.id === id)?.name ?? id) : '我');

/** 近 30 天的成绩（假数据） */
export interface Stats {
  /** 交审几张 */
  done: number;
  /** 一次通过率 % */
  pass: number;
  rework: number;
  crash: number;
  /** 机器掉线结束的次数 */
  lost?: number;
  /** 平均用时（分钟） */
  avgMin: number;
  /** 费用；agent 不报时为空 */
  cost: string | null;
}

export interface Profile {
  id: string;
  name: string;
  /** null = 我：不派给机器，等人自己开会话 */
  agent: AgentId | null;
  /** null = 本机登录：用机器上 agent 自己登录的账号 */
  provider: string | null;
  model: string;
  effort: string | null;
  /** 附加说明：派发时接在提示词后面 */
  note: string;
  /** 只在这些机器上跑；null = 所有能跑的 */
  only: string[] | null;
  stats: Stats;
}

export interface Rule {
  /** in 带其中任一个 · all 全都带 · not_in 一个都不带 · none 没有标签 */
  op: 'in' | 'all' | 'not_in' | 'none';
  labels: string[];
  /** 工种 id */
  to: string;
}

export interface Run {
  profile: string;
  ref: string;
  title: string;
  min: number;
}

export interface PHost {
  name: string;
  os: string;
  online: boolean;
  /** 离线多久（分钟） */
  offMin?: number;
  max: number;
  /** 自动接单时段 [起, 止)，一天里的分钟；null = 全天 */
  win: [number, number] | null;
  /** 装了哪些 agent、能不能用（自检）；没写的 = 没装 */
  agents: Partial<Record<AgentId, { ok: boolean; why?: string }>>;
  /** 连不连得上供应商（自检时测过）；没写的当连得上 */
  reach: Record<string, { ok: boolean; why: string }>;
  running: Run[];
  /** 最近一次接派发是几分钟前 */
  lastMin: number;
}

export interface ProviderInfo {
  name: string;
  models: string[];
  efforts: string[];
}
/** 供应商（共用配置里已经有的，假数据）；本机登录时每个 agent 自己能选的模型 */
export const PROVIDERS: ProviderInfo[] = [
  { name: 'local-proxy', models: ['model-a', 'model-a-pro'], efforts: ['high', 'max'] },
  { name: 'relay-a', models: ['model-b', 'model-b-mini'], efforts: ['low', 'medium', 'high', 'max'] },
  { name: 'relay-b', models: ['model-c-flash', 'model-c', 'model-d'], efforts: ['high', 'max'] },
];
export const LOCAL_MODELS: Record<AgentId, { models: string[]; efforts: string[] }> = {
  claude: { models: ['sonnet[1m]', 'opus', 'haiku'], efforts: ['low', 'medium', 'high', 'xhigh', 'max'] },
  codex: { models: ['model-b', 'model-b-codex'], efforts: ['minimal', 'low', 'medium', 'high'] },
  dsh: { models: ['deepseek-v4-flash'], efforts: [] },
};

const nowMin = () => {
  const d = new Date();
  return d.getHours() * 60 + d.getMinutes();
};
const round30 = (m: number) => (Math.ceil(m / 30) * 30) % 1440;

function initial() {
  // win2 的接单时段从两个半小时后开始：现在一定在时段外，能看到「时段外，几点开始」
  const start = round30(nowMin() + 150);
  const profiles: Profile[] = [
    {
      id: 'main',
      name: '主力',
      agent: 'claude',
      provider: null,
      model: 'sonnet[1m]',
      effort: 'high',
      note: '界面改动做完要在浏览器里自己走一遍，截图附在交审评论里。',
      only: null,
      stats: { done: 34, pass: 82, rework: 4, crash: 1, avgMin: 18, cost: '$41.2' },
    },
    {
      id: 'fix',
      name: '修复',
      agent: 'codex',
      provider: 'relay-a',
      model: 'model-b',
      effort: 'max',
      note: '先写能复现的测试，再改。',
      only: null,
      stats: { done: 27, pass: 71, rework: 6, crash: 3, avgMin: 26, cost: '$18.7' },
    },
    {
      id: 'bulk',
      name: '批量',
      agent: 'codex',
      provider: 'local-proxy',
      model: 'model-a',
      effort: 'max',
      note: '',
      only: null,
      stats: { done: 40, pass: 58, rework: 11, crash: 7, lost: 58, avgMin: 34, cost: null },
    },
    {
      id: 'cheap',
      name: '便宜',
      agent: 'dsh',
      provider: 'relay-b',
      model: 'model-c-flash',
      effort: null,
      note: '只改文档和文案，不碰代码。',
      only: null,
      stats: { done: 3, pass: 33, rework: 2, crash: 2, avgMin: 12, cost: '$0.9' },
    },
    {
      id: 'me',
      name: '我',
      agent: null,
      provider: null,
      model: '',
      effort: null,
      note: '',
      only: null,
      stats: { done: 118, pass: 90, rework: 9, crash: 0, avgMin: 41, cost: null },
    },
  ];
  const rules: Rule[] = [
    { op: 'all', labels: ['bug', 'Windows'], to: 'fix' },
    { op: 'in', labels: ['bug'], to: 'bulk' },
    { op: 'in', labels: ['文档'], to: 'cheap' },
    { op: 'in', labels: ['重构'], to: 'main' },
    { op: 'in', labels: ['社区'], to: 'me' },
  ];
  const hosts: PHost[] = [
    {
      name: 'mac',
      os: 'macOS 15.8 Sequoia',
      online: true,
      max: 4,
      win: null,
      agents: { claude: { ok: true }, codex: { ok: true }, dsh: { ok: true } },
      reach: {},
      running: [
        { profile: 'bulk', ref: 'oss/release#2', title: '各平台安装包签名（macOS / Windows）', min: 12 },
        { profile: 'bulk', ref: 'docs/content#4', title: '示例仓库截图过期，需要重截', min: 31 },
        { profile: 'main', ref: 'site/launch#1', title: '首页首屏：一句话讲清「人和 agent 共用一块白板」', min: 8 },
        { profile: 'fix', ref: 'oss/release#4', title: '跑一遍完整的验收标准（PRD §13）', min: 22 },
      ],
      lastMin: 8,
    },
    {
      name: 'win',
      os: 'Windows 11 Pro',
      online: true,
      max: 4,
      win: null,
      agents: { claude: { ok: true }, codex: { ok: true }, dsh: { ok: true } },
      reach: { 'local-proxy': { ok: false, why: '本机没开代理' } },
      running: [
        { profile: 'cheap', ref: 'docs/i18n#2', title: '翻译错误码页面', min: 5 },
        { profile: 'fix', ref: 'site/launch#2', title: '定价页：开源版 / 团队版对比表', min: 15 },
      ],
      lastMin: 5,
    },
    {
      name: 'win2',
      os: 'Windows 11 Pro',
      online: true,
      max: 4,
      win: [start, (start + 600) % 1440],
      agents: { claude: { ok: false, why: 'Claude Code 没登录（自检没过）' }, codex: { ok: true }, dsh: { ok: true } },
      reach: {},
      running: [],
      lastMin: 300,
    },
    {
      name: 'mbp',
      os: 'macOS 15.6 Sequoia',
      online: false,
      offMin: 190,
      max: 2,
      win: null,
      agents: { claude: { ok: true }, codex: { ok: true } },
      reach: { 'local-proxy': { ok: false, why: '本机没开代理' } },
      running: [],
      lastMin: 600,
    },
  ];
  return { profiles, rules, fallback: null as string | null, hosts };
}

type State = ReturnType<typeof initial>;
export const useModel = create<State>(() => initial());
export const resetModel = () => useModel.setState(initial());

/* ------------------------------ 改（只在内存里） ------------------------------ */

export const putProfile = (p: Profile) =>
  useModel.setState(s => ({ profiles: s.profiles.some(x => x.id === p.id) ? s.profiles.map(x => (x.id === p.id ? p : x)) : [...s.profiles, p] }));
export const dropProfile = (id: string) =>
  useModel.setState(s => ({ profiles: s.profiles.filter(x => x.id !== id), rules: s.rules.filter(r => r.to !== id), fallback: s.fallback === id ? null : s.fallback }));
export const setRules = (rules: Rule[]) => useModel.setState({ rules });
export const setFallback = (fallback: string | null) => useModel.setState({ fallback });
export const moveRule = (i: number, d: number) =>
  useModel.setState(s => {
    const next = [...s.rules];
    const [r] = next.splice(i, 1);
    next.splice(Math.max(0, Math.min(next.length, i + d)), 0, r);
    return { rules: next };
  });
export const toggleOnly = (id: string, host: string, all: string[]) =>
  useModel.setState(s => ({
    profiles: s.profiles.map(p => {
      if (p.id !== id) return p;
      const on = p.only ?? all;
      const next = on.includes(host) ? on.filter(x => x !== host) : [...on, host];
      return { ...p, only: next.length === all.length && all.every(x => next.includes(x)) ? null : next };
    }),
  }));

/* ------------------------------ 文字 ------------------------------ */

export const hhmm = (m: number) => `${String(Math.floor(m / 60) % 24).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`;
export const dur = (min: number) => (min < 60 ? `${min} 分钟` : min % 60 ? `${Math.floor(min / 60)} 小时 ${min % 60} 分` : `${min / 60} 小时`);
/** 「Codex · relay-a · model-b · max」；本机登录写「本机登录」 */
export const recipe = (p: Profile) => (p.agent ? [agentName(p.agent), p.provider ?? '本机登录', p.model, p.effort].filter(Boolean).join(' · ') : '不派给机器：等你自己开会话');
/** 不带 agent 名的短写法：「model-b · max」 */
export const short = (p: Profile) => (p.agent ? [p.model, p.effort].filter(Boolean).join(' · ') : '你自己开会话');
export const ruleText = (r: Rule) => {
  const l = r.labels.map(x => `「${x}」`).join('');
  switch (r.op) {
    case 'in':
      return r.labels.length > 1 ? `带 ${l} 之一` : `带 ${l}`;
    case 'all':
      return `同时带 ${l}`;
    case 'not_in':
      return `不带 ${l}`;
    case 'none':
      return '没有标签';
  }
};
export const OPS: { value: Rule['op']; label: string }[] = [
  { value: 'in', label: '带' },
  { value: 'all', label: '同时带' },
  { value: 'not_in', label: '不带' },
  { value: 'none', label: '没有标签' },
];

/* ------------------------------ 推算 ------------------------------ */

const has = (names: string[], l: string) => names.some(n => n.toLowerCase() === l.toLowerCase());

/** 一条规则对这些标签命不命中，以及为什么 */
export function ruleHit(r: Rule, names: string[]): { hit: boolean; why: string } {
  switch (r.op) {
    case 'in': {
      const got = r.labels.filter(l => has(names, l));
      return got.length ? { hit: true, why: `带「${got[0]}」` } : { hit: false, why: r.labels.length > 1 ? `${r.labels.map(x => `「${x}」`).join('')}都没带` : `没带「${r.labels[0]}」` };
    }
    case 'all': {
      const miss = r.labels.filter(l => !has(names, l));
      return miss.length ? { hit: false, why: `没带${miss.map(x => `「${x}」`).join('')}` } : { hit: true, why: '都带了' };
    }
    case 'not_in': {
      const got = r.labels.filter(l => has(names, l));
      return got.length ? { hit: false, why: `带了「${got[0]}」` } : { hit: true, why: '都没带' };
    }
    case 'none':
      return names.length ? { hit: false, why: '有标签' } : { hit: true, why: '没有标签' };
  }
}

/** 第一条命中的规则（rule = -1 是兜底）；都没命中、没设兜底时为空 */
export function routeOf(names: string[], rules: Rule[], fallback: string | null): { rule: number; to: string } | null {
  const i = rules.findIndex(r => ruleHit(r, names).hit);
  if (i >= 0) return { rule: i, to: rules[i].to };
  return fallback ? { rule: -1, to: fallback } : null;
}

/** 这台机器能不能跑这个工种（装没装、登没登录、连不连得上供应商、选没选这台） */
export function canRun(p: Profile, h: PHost): { ok: boolean; why: string } {
  if (!p.agent) return { ok: false, why: '「我」不派给机器' };
  if (p.only && !p.only.includes(h.name)) return { ok: false, why: '没选这台' };
  const a = h.agents[p.agent];
  if (!a) return { ok: false, why: `没装 ${agentName(p.agent)}` };
  if (!a.ok) return { ok: false, why: a.why ?? `${agentName(p.agent)} 用不了` };
  if (p.provider) {
    const r = h.reach[p.provider];
    if (r && !r.ok) return { ok: false, why: `连不上 ${p.provider}：${r.why}` };
  }
  return { ok: true, why: p.provider ? `装了 ${agentName(p.agent)}，连得上 ${p.provider}` : `${agentName(p.agent)} 已登录` };
}

export type Tone = 'ok' | 'busy' | 'off' | 'bad';
/** 机器现在能不能开新会话：auto = 自动领取（看接单时段）；used = 这次推算里已经分给它的 */
export function slot(h: PHost, used: number, auto: boolean): { free: number; ok: boolean; why: string; tone: Tone } {
  if (!h.online) return { free: 0, ok: false, why: `离线 ${dur(h.offMin ?? 0)}`, tone: 'off' };
  if (auto && h.win && !inWin(h.win, nowMin())) return { free: 0, ok: false, why: `时段外，${hhmm(h.win[0])} 开始接单`, tone: 'off' };
  const run = h.running.length + used;
  if (run >= h.max) return { free: 0, ok: false, why: `满 ${run}/${h.max}`, tone: 'busy' };
  return { free: h.max - run, ok: true, why: `空 ${h.max - run} 位`, tone: 'ok' };
}
const inWin = ([a, b]: [number, number], m: number) => (a <= b ? m >= a && m < b : m >= a || m < b);

export interface HostWhy {
  name: string;
  /** 能跑这个工种 */
  can: boolean;
  /** 现在就能开始 */
  ok: boolean;
  why: string;
  tone: Tone;
  free: number;
  picked?: boolean;
}

/** 一个工种在每台机器上现在怎么样；auto = 按自动领取算（看接单时段） */
export function hostsFor(p: Profile, hosts: PHost[], auto: boolean, used: Record<string, number> = {}): HostWhy[] {
  return hosts.map(h => {
    const c = canRun(p, h);
    if (!c.ok) return { name: h.name, can: false, ok: false, why: c.why, tone: 'bad' as Tone, free: 0 };
    const s = slot(h, used[h.name] ?? 0, auto);
    return { name: h.name, can: true, ok: s.ok, why: s.why, tone: s.tone, free: s.free };
  });
}

/** 在能马上开始的机器里挑一台：空位多的优先，一样多挑最久没接的 */
export function pickHost(list: HostWhy[], hosts: PHost[]): HostWhy | null {
  const last = (n: string) => hosts.find(h => h.name === n)?.lastMin ?? 0;
  return list.filter(x => x.ok).sort((a, b) => b.free - a.free || last(b.name) - last(a.name) || a.name.localeCompare(b.name))[0] ?? null;
}

/** 排队时最快什么时候开始 */
export function soonest(list: HostWhy[]): string {
  const can = list.filter(x => x.can);
  if (!can.length) return '没有机器能跑这个工种';
  const busy = can.filter(x => x.tone === 'busy').map(x => x.name);
  const later = can.filter(x => x.why.startsWith('时段外')).map(x => `${x.name} ${x.why.replace('时段外，', '')}`);
  const parts = [busy.length ? `${busy.join('、')} 空出来就开始` : '', ...later].filter(Boolean);
  return parts.length ? parts.join('；') : `能跑的 ${can.map(x => x.name).join('、')} 都离线`;
}

export interface Fc {
  card: number;
  title: string;
  names: string[];
  route: { rule: number; to: string } | null;
  /** start 马上开始 · wait 排队 · me 等你开会话 · none 没命中、不自动领 · off 这块白板不参与自动领取 */
  kind: 'start' | 'wait' | 'me' | 'none';
  host?: string;
  /** 排队：这个工种里排第几 */
  pos?: number;
  hosts: HostWhy[];
  soon?: string;
}

export interface CardIn {
  id: number;
  title: string;
  names: string[];
}

/** 推算一组可领取卡片（按白板上的顺序）各自会怎么走 */
export function forecast(cards: CardIn[], st: Pick<State, 'profiles' | 'rules' | 'fallback' | 'hosts'>): Map<number, Fc> {
  const out = new Map<number, Fc>();
  const used: Record<string, number> = {};
  const queue: Record<string, number> = {};
  for (const c of cards) {
    const route = routeOf(c.names, st.rules, st.fallback);
    const p = route && st.profiles.find(x => x.id === route.to);
    if (!route || !p) {
      out.set(c.id, { card: c.id, title: c.title, names: c.names, route: null, kind: 'none', hosts: [] });
      continue;
    }
    if (!p.agent) {
      out.set(c.id, { card: c.id, title: c.title, names: c.names, route, kind: 'me', hosts: [] });
      continue;
    }
    const list = hostsFor(p, st.hosts, true, used);
    const got = pickHost(list, st.hosts);
    if (got) {
      used[got.name] = (used[got.name] ?? 0) + 1;
      out.set(c.id, { card: c.id, title: c.title, names: c.names, route, kind: 'start', host: got.name, hosts: list.map(x => (x.name === got.name ? { ...x, picked: true } : x)) });
    } else {
      queue[p.id] = (queue[p.id] ?? 0) + 1;
      out.set(c.id, { card: c.id, title: c.title, names: c.names, route, kind: 'wait', pos: queue[p.id], hosts: list, soon: soonest(list) });
    }
  }
  return out;
}

/** 一块白板上可领取的卡片（按状态顺序、状态里的顺序）和它们的标签名 */
export function claimableCards(ws: Workspace | null, board: BoardSnapshot | null): CardIn[] {
  if (!board) return [];
  const proj = board.path.split('/')[0];
  const labels = ws?.projects.find(p => p.key === proj)?.labels ?? [];
  return board.statuses
    .filter(s => s.claim)
    .flatMap(s => board.cols[s.key] ?? [])
    .map(id => board.cards[id])
    .filter(c => c && !c.holder)
    .map(c => ({ id: c.id, title: c.title, names: c.labels.map(id => labels.find(l => l.id === id)?.name).filter((x): x is string => !!x) }));
}

/** 一个工种现在：在跑几个（哪台）、排队几张 */
export function loadOf(p: Profile, hosts: PHost[], fc: Map<number, Fc>) {
  const running = hosts.flatMap(h => h.running.filter(r => r.profile === p.id).map(r => ({ ...r, host: h.name })));
  const all = [...fc.values()].filter(f => f.route?.to === p.id);
  return { running, waiting: all.filter(f => f.kind === 'wait' || f.kind === 'me'), starting: all.filter(f => f.kind === 'start') };
}
