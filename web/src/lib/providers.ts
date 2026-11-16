/* ==========================================================================
   模型供应商共用配置（PRD §10.16「共用配置」）：界面上用到的纯函数和常量
   - 三种接口；内置 agent 认哪些接口、能写哪些强度从服务端取（ProviderView.meta，board-core 留一份）
   - 常用供应商的预填：真实接口地址和常用模型（API key 要自己填）
   - 一台机器上一个 agent 的写入情况（providerCell）、地址相同的接口并成一行（endpointLines）
   - 一台机器上一个 agent 的配置文件要写成什么（runPick）：不改 / 指定（供应商、模型、强度、接口）
   - 供应商卡片底下的「在用」（usesOf、agentGroups）：按 agent 汇总哪几台机器用着它
   - 编辑表单：空白的一项、校验、整理成要保存的
   ========================================================================== */
import type { AgentMeta, HostAgent, HostInfo, Provider, ProviderEndpoint, ProviderView, RunMode } from './types';

/** 三种接口 */
export const PROVIDER_INTERFACES: { id: ProviderEndpoint['interface']; name: string; short: string; placeholder: string }[] = [
  { id: 'anthropic', name: 'Anthropic Messages', short: 'Anthropic', placeholder: 'https://api.anthropic.com' },
  { id: 'openai-chat', name: 'OpenAI Chat Completions', short: 'OpenAI Chat', placeholder: 'https://api.openai.com/v1' },
  { id: 'openai-responses', name: 'OpenAI Responses', short: 'OpenAI Responses', placeholder: 'https://api.openai.com/v1' },
];

/** 所有强度档位，从低到高（供应商里给模型标的只能是这几个，和服务端 ALL_EFFORTS 一致） */
export const ALL_EFFORTS = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'];

/** 内置 agent 的说明（认哪些接口、配置文件能写哪几档强度）：服务端 board-core 留一份，`GET /api/providers` 的 meta */
export const metasOf = (view: ProviderView | null): AgentMeta[] => view?.meta ?? [];
export const metaOf = (view: ProviderView | null, agent: string) => metasOf(view).find(m => m.id === agent);

/** 这个 agent 用这家供应商的这个模型时能写哪几档强度：模型标了的、agent 也能写的（从低到高）；没标时为空，不能选 */
export function writableEfforts(meta: AgentMeta | undefined, p: Provider | null | undefined, model: string): string[] {
  const marked = p?.efforts?.[model] ?? [];
  return (meta?.efforts ?? []).filter(e => marked.includes(e));
}

export const interfaceName = (id: string) => PROVIDER_INTERFACES.find(i => i.id === id)?.short ?? id;

/** 常用供应商：接口地址和常用模型（2026 年核实过的写法；API key 要自己填）。
 *  Anthropic 兼容接口的地址都不带 /v1：Claude Code 自己接 /v1/messages（保存时服务端也会去掉） */
export const PROVIDER_PRESETS: { name: string; label: string; endpoints: ProviderEndpoint[]; models: string[] }[] = [
  {
    name: 'deepseek',
    label: 'DeepSeek',
    endpoints: [
      { interface: 'openai-chat', url: 'https://api.deepseek.com/v1' },
      { interface: 'anthropic', url: 'https://api.deepseek.com/anthropic' },
    ],
    models: ['deepseek-chat', 'deepseek-reasoner'],
  },
  {
    name: 'kimi',
    label: 'Kimi',
    endpoints: [
      { interface: 'openai-chat', url: 'https://api.moonshot.cn/v1' },
      { interface: 'anthropic', url: 'https://api.moonshot.cn/anthropic' },
    ],
    models: ['kimi-k2-0905-preview', 'kimi-k2-turbo-preview'],
  },
  {
    name: 'zhipu',
    label: '智谱 GLM',
    endpoints: [
      { interface: 'openai-chat', url: 'https://open.bigmodel.cn/api/paas/v4' },
      { interface: 'anthropic', url: 'https://open.bigmodel.cn/api/anthropic' },
    ],
    models: ['glm-4.6', 'glm-4.5-air'],
  },
  {
    name: 'openai',
    label: 'OpenAI',
    endpoints: [
      { interface: 'openai-chat', url: 'https://api.openai.com/v1' },
      { interface: 'openai-responses', url: 'https://api.openai.com/v1' },
    ],
    models: ['gpt-5.2', 'gpt-5-mini', 'o4-mini'],
  },
  {
    name: 'anthropic',
    label: 'Anthropic',
    endpoints: [{ interface: 'anthropic', url: 'https://api.anthropic.com' }],
    models: ['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5'],
  },
];

export const blankProvider = (): Provider => ({ name: '', endpoints: [], apiKey: '', models: [], efforts: {} });

/** API key 的显示：只给末尾 4 位 */
export const maskKey = (p: Pick<Provider, 'tail'>) => (p.tail ? `••••${p.tail}` : '••••');

/** 校验（和服务端一致）：不对时返回原因 */
export function providerError(p: Provider, existing: string[], editing: boolean): string | null {
  if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(p.name)) return '名称只能用字母、数字、_、-，以字母或数字开头';
  if (!editing && existing.includes(p.name)) return `已经有「${p.name}」了：点它修改`;
  const eps = p.endpoints.filter(e => e.url.trim());
  if (!eps.length) return '至少填一个接口的地址';
  for (const e of eps) {
    if (!/^https?:\/\//.test(e.url.trim())) return `${interfaceName(e.interface)} 的地址要以 http:// 或 https:// 开头`;
  }
  // 密钥留空：修改时保留原值；新加的必须填
  if (!(p.apiKey ?? '').trim() && !(editing && p.tail != null)) return '要填 API key';
  if (!p.models.length) return '至少要填一个模型（一行一个）';
  for (const m of p.models) {
    if (m.length > 64 || !/^[A-Za-z0-9][A-Za-z0-9._:/@+-]*$/.test(m)) return `模型名「${m}」不对：字母或数字开头，只能用字母、数字和 . _ : / @ + -，最长 64 个字符`;
  }
  return null;
}

/** 要保存的：地址去掉没填的接口、密钥留空时不带（服务端保留原值）、不带 tail */
export function toProviderSave(p: Provider): Provider {
  return {
    name: p.name.trim(),
    endpoints: p.endpoints.filter(e => e.url.trim()).map(e => ({ interface: e.interface, url: e.url.trim() })),
    apiKey: (p.apiKey ?? '').trim() || null,
    models: p.models,
    efforts: Object.fromEntries(Object.entries(p.efforts ?? {}).filter(([m, l]) => p.models.includes(m) && l.length)),
    version: p.version,
  };
}

/* ------------------------------ 各机器的写入状态 ------------------------------ */

/** 一台机器上一个 agent 的写入情况（界面上一个小胶囊）：
 *  old 旧版 board host 不按请求同步 · missing 没找到这个 agent · local 这台机器选了「不改」· none 没有要写的（指定了却没有供应商）·
 *  offline 该写、机器离线 · synced 写进去的就是该写的 · failed / unsupported 上报的失败、不支持 · pending 正在写入 */
export type ProviderCellKind = 'synced' | 'pending' | 'local' | 'failed' | 'unsupported' | 'offline' | 'old' | 'missing' | 'none';
export type ProviderCell = { kind: ProviderCellKind; why?: string | null; provider?: string };

export const PROVIDER_CELL_WORDS: Record<ProviderCellKind, string> = {
  synced: '已写入',
  pending: '正在写入',
  local: '不改',
  failed: '失败',
  unsupported: '不支持',
  offline: '离线',
  old: '版本太旧',
  missing: '没装',
  none: '不写',
};

export function providerCell(h: HostInfo, agent: string, view: ProviderView | null): ProviderCell {
  if (!h.sync?.able) return { kind: 'old', why: `board ${h.version || '?'} 不按请求同步配置：升级这台机器的 board` };
  const a = h.agents.find(x => x.id === agent);
  if (!a?.found) return { kind: 'missing' };
  const pick = runPick(view, a);
  if (pick.mode === 'off') return { kind: 'local', why: '这台机器选了「不改」：Board 不碰它的配置文件' };
  if (!pick.provider) return { kind: 'none' };
  if (!h.online) return { kind: 'offline', why: '上线后写入', provider: pick.provider };
  if (h.sync.ask) return { kind: 'pending', why: 'board host 收到后写配置文件、上报', provider: pick.provider };
  const st = h.sync.last?.agents.find(s => s.agent === agent);
  if (st?.state === 'synced') return { kind: 'synced', why: [st.provider, st.model, st.effort].filter(Boolean).join(' · '), provider: pick.provider };
  if (st?.state === 'failed' || st?.state === 'unsupported') return { kind: st.state, why: st.why, provider: pick.provider };
  return { kind: 'pending', why: '等 board host 写入', provider: pick.provider };
}

/** 接口和地址：地址相同的接口并成一行（OpenAI Chat · OpenAI Responses → 同一个地址） */
export function endpointLines(p: Pick<Provider, 'endpoints'>) {
  const m = new Map<string, string[]>();
  for (const e of p.endpoints) m.set(e.url, [...(m.get(e.url) ?? []), interfaceName(e.interface)]);
  return [...m.entries()].map(([url, names]) => ({ url, names }));
}

/* ------------------------------ 机器详情 ------------------------------ */

/** 这个 agent 用这家供应商时能选的接口：agent 认的、供应商给了的（按供应商里的顺序，和 board host 一样） */
export function protocolChoices(view: ProviderView | null, agent: string, provider: string): ProviderEndpoint[] {
  const want = metaOf(view, agent)?.protocols ?? [];
  return (view?.providers.find(x => x.name === provider)?.endpoints ?? []).filter(e => want.includes(e.interface));
}

/** 一台机器上一个 agent 的配置文件要写成什么：
 *  - mode：off 不改 / own 指定
 *  - provider、model、effort：要写进配置文件的；gone 是选的供应商已经删了
 *  - ep：实际用哪个接口和地址（指定了、供应商给着、agent 也认就是它，否则第一个对得上的）；对不上任何一种时没有
 *  - warn：选的强度这个模型不支持（写配置时不写强度） */
export interface RunPick {
  mode: RunMode;
  provider: string;
  model: string;
  effort: string;
  ep?: ProviderEndpoint;
  gone?: boolean;
  warn?: string;
}

export function runPick(view: ProviderView | null, a: HostAgent): RunPick {
  if (a.run_mode !== 'own') return { mode: 'off', provider: '', model: '', effort: '' };
  const out: RunPick = { mode: 'own', provider: a.run_provider ?? '', model: a.run_model ?? '', effort: a.run_effort ?? '' };
  const p = view?.providers.find(x => x.name === out.provider);
  if (view && out.provider && !p) out.gone = true;
  if (p && !out.model) out.model = p.models[0] ?? '';
  const eps = protocolChoices(view, a.id, out.provider);
  out.ep = eps.find(e => e.interface === a.run_protocol) ?? eps[0];
  if (p && out.effort && !writableEfforts(metaOf(view, a.id), p, out.model).includes(out.effort)) out.warn = `${out.model} 不支持强度 ${out.effort}：写配置时不写强度`;
  return out;
}

/* ------------------------------ 供应商卡片的「在用」 ------------------------------ */

/** 一台机器上一个 agent 用着某个供应商 */
export interface ProviderUse {
  h: HostInfo;
  a: HostAgent;
  pick: RunPick;
  cell: ProviderCell;
}

/** 这个供应商在哪几台机器的哪几个 agent 上用着（指定了它的；没找到的 agent 不算） */
export function usesOf(hosts: HostInfo[], view: ProviderView, provider: string): ProviderUse[] {
  const out: ProviderUse[] = [];
  for (const h of hosts)
    for (const a of h.agents) {
      if (!a.found) continue;
      const pick = runPick(view, a);
      if (pick.mode === 'own' && pick.provider === provider) out.push({ h, a, pick, cell: providerCell(h, a.id, view) });
    }
  return out;
}

/** 一组里最该看的状态：失败 > 正在写入 > 离线 > 已写入 */
const RANK: ProviderCellKind[] = ['failed', 'unsupported', 'pending', 'offline', 'old', 'synced'];
export const worstCell = (us: ProviderUse[]): ProviderCellKind => RANK.find(k => us.some(u => u.cell.kind === k)) ?? 'synced';

/** 按 agent 汇总（按内置 agent 的顺序）：卡片上一个 agent 一枚，点开列出哪几台 */
export function agentGroups(us: ProviderUse[], view: ProviderView): { meta: AgentMeta; uses: ProviderUse[] }[] {
  return metasOf(view)
    .map(meta => ({ meta, uses: us.filter(u => u.a.id === meta.id) }))
    .filter(g => g.uses.length);
}

/** 一句话：「newapi-buglan · cn:deepseek-v4.1-flash · 强度 high」 */
export function runWords(p: Pick<RunPick, 'provider' | 'model' | 'effort' | 'warn'>): string {
  return [p.provider, p.model, p.warn ? '不写强度' : p.effort ? `强度 ${p.effort}` : '强度默认'].filter(Boolean).join(' · ');
}

/** 这个 agent 能选的供应商：接口对得上 */
export const providersFor = (view: ProviderView | null, agent: string) => {
  const want = metaOf(view, agent)?.protocols ?? [];
  return (view?.providers ?? []).filter(p => p.endpoints.some(e => want.includes(e.interface)));
};
