/* ==========================================================================
   接口数据进门时补齐（default/main#359）
   界面和服务端不一定是同一个版本：服务端升级后，浏览器里开着的旧页面接着调新接口；桌面端自带的界面连着别的版本的服务端。
   这时接口可能少了界面以为一定有的数组。线上那次：v0.1.30 的 GET /api/providers 去掉了 agents，开着的 v0.1.29 页面
   在机器页「共用配置」里 view.agents.find(...) 整页崩（Cannot read properties of undefined (reading 'find')）。

   api.ts 拿到机器、派发、调度、共用配置、会话的数据时在这里补一遍：
   - 类型里写成必填的数组：缺了、是 null 都补成空数组；必填的对象（同步结果里的 skills、mcp）缺了补成空的
   - 可选的数组：不是数组（null）时当作没有（undefined），调用方本来就按「没有」处理（`?.`、`??`）。
     不把它们补成空数组：有的「没有」另有含义，例如 windows 没有 = 全天接单、rules 没有 = 旧服务端
   于是机器页、机器详情、派发、共用配置这些调用方可以直接 .find / .filter / .map，不用每处兜底。
   不在 hostsOf() 里补：它在 useData 的选择器里调用，每次返回新对象会让选择器的结果永远「变了」
   ========================================================================== */
import type {
  AgentInfo,
  ClaimRule,
  Dispatch,
  HostAgent,
  HostInfo,
  HostRes,
  McpServer,
  Profile,
  Provider,
  ProviderView,
  SessionView,
  SkillImport,
  SkillInfo,
  SyncResult,
  Workspace,
} from './types';

/** 必填的数组：不是数组时为空 */
const list = <T>(v: T[] | null | undefined): T[] => (Array.isArray(v) ? v : []);
/** 可选的数组：不是数组时当作没有 */
const maybe = <T>(v: T[] | null | undefined): T[] | undefined => (Array.isArray(v) ? v : undefined);
/** 对象：不是对象时当作没有（null 留着 null，服务端用它表示「清空了」） */
const obj = <T extends object>(v: T | null | undefined): T | null | undefined => (v && typeof v === 'object' ? v : v === null ? null : undefined);

const normAgent = (a: HostAgent): HostAgent => ({ ...a, envs: maybe(a.envs) });

const normRule = (r: ClaimRule): ClaimRule => ({ ...r, profile: r.profile ?? '', labels: list(r.labels), missing: maybe(r.missing) });

const normSync = (r: SyncResult): SyncResult => {
  const s = obj(r.skills);
  const m = obj(r.mcp);
  const g = obj(r.git);
  return {
    ...r,
    skills: { state: '', count: 0, ...s, dirs: list(s?.dirs), replaced: list(s?.replaced), links: list(s?.links) },
    mcp: { state: '', count: 0, ...m, agents: list(m?.agents), replaced: list(m?.replaced) },
    git: { state: '', ...g, repos: list(g?.repos), skipped: list(g?.skipped) },
  };
};

const normImport = (s: SkillImport): SkillImport => {
  const last = obj(s.last);
  return {
    ...s,
    local: list(s.local).map(l => ({ ...l, agents: list(l.agents), dirs: list(l.dirs) })),
    last: last ? { ...last, ok: list(last.ok), failed: list(last.failed) } : last,
  };
};

/** 一台机器 */
/** 资源（default/main#358）：趋势缺了补成空数组；gpus、disks 不是数组时当作没有（界面写「不支持」「未上报」） */
function normRes(r: HostRes | null | undefined): HostRes | undefined {
  const x = obj(r);
  if (!x) return undefined;
  const hist = obj(x.hist);
  return { ...x, gpus: maybe(x.gpus), disks: maybe(x.disks), hist: { cpu: list(hist?.cpu), mem: list(hist?.mem), gpu: list(hist?.gpu) } };
}

export function normHost(h: HostInfo): HostInfo {
  const ws = obj(h.workspace);
  const token = obj(h.token);
  const selftest = obj(h.selftest);
  const sync = obj(h.sync);
  const last = obj(sync?.last);
  const imp = obj(h.skillImport);
  return {
    ...h,
    agents: list(h.agents).map(normAgent),
    envs: maybe(h.envs),
    workspace: ws ? { ...ws, overrides: maybe(ws.overrides) } : ws,
    dirs: list(h.dirs),
    rules: maybe(h.rules)?.map(normRule),
    fits: maybe(h.fits),
    cron: list(h.cron),
    windows: maybe(h.windows),
    ops: maybe(h.ops),
    token: token ? { ...token, shared: maybe(token.shared) } : token,
    // 旧服务端只有一个 ask：当成唯一的请求
    selftest: selftest ? { ...selftest, asks: selftest.asks ? list(selftest.asks) : selftest.ask ? [selftest.ask] : [], items: list(selftest.items) } : (selftest ?? undefined),
    sync: sync ? { ...sync, last: last ? normSync(last) : last } : (sync ?? undefined),
    skillImport: imp ? normImport(imp) : (imp ?? undefined),
    res: normRes(h.res),
  };
}

const normAgentInfo = (a: AgentInfo): AgentInfo => ({ ...a, cards: list(a.cards) });

/** 角色：字符串缺了补空，「只在这些机器上跑」不是数组时当作不限 */
const normProfile = (p: Profile): Profile => ({
  ...p,
  provider: p.provider ?? '',
  model: p.model ?? '',
  effort: p.effort ?? '',
  note: p.note ?? '',
  only: list(p.only),
});

/** 工作区：机器、派发、接不了的机器 × 白板、在线的 agent、调度。旧服务端没有的（hosts、dispatches…）留着没有 */
export function normWorkspace(ws: Workspace): Workspace {
  if (!ws || typeof ws !== 'object') return ws;
  const sched = obj(ws.sched);
  return {
    ...ws,
    agents: maybe(ws.agents)?.map(normAgentInfo),
    hosts: maybe(ws.hosts)?.map(normHost),
    dispatches: maybe(ws.dispatches),
    blocks: maybe(ws.blocks),
    checklist: maybe(ws.checklist)?.map(c => ({ ...c, items: list(c.items) })),
    sched: sched ? { ...sched, rules: list(sched.rules).map(normRule), profiles: maybe(sched.profiles)?.map(normProfile) } : (sched ?? undefined),
  };
}

/** 机器页每 10 秒取的机器和派发 */
export function normHosts(r: { hosts?: HostInfo[] | null; dispatches?: Dispatch[] | null } | null): { hosts: HostInfo[]; dispatches: Dispatch[] } {
  return { hosts: list(r?.hosts).map(normHost), dispatches: list(r?.dispatches) };
}

const normProvider = (p: Provider): Provider => ({ ...p, endpoints: list(p.endpoints), models: list(p.models) });

/** 共用配置的模型供应商列表 */
export function normProviders(v: ProviderView | null): ProviderView {
  return {
    ...v,
    providers: list(v?.providers).map(normProvider),
    meta: maybe(v?.meta)?.map(m => ({ ...m, protocols: list(m.protocols), efforts: list(m.efforts), skills: maybe(m.skills) ?? undefined })),
  };
}

/** 共用配置的 MCP 服务器列表 */
export const normMcp = (v: McpServer[] | null): McpServer[] =>
  list(v).map(s => ({ ...s, args: list(s.args), env: list(s.env), headers: list(s.headers), agents: list(s.agents) }));

/** 共用配置的 skills 库 */
export const normSkills = (v: SkillInfo[] | null): SkillInfo[] => list(v);

/** 派发列表（机器详情「最近」、卡片详情「派发记录」） */
export const normDispatches = (v: Dispatch[] | null): Dispatch[] => list(v);

/** 会话弹窗：每一段的输出、事件 */
export function normSession(v: SessionView): SessionView {
  if (!v || typeof v !== 'object') return v;
  return { ...v, parts: list(v.parts).map(p => ({ ...p, log: list(p.log), events: list(p.events) })) };
}
