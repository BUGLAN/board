/* ==========================================================================
   模型供应商共用配置（PRD §10.12「共用配置」）：界面上用到的纯函数和常量
   - 三种接口、四个内置 agent 认哪些接口、能选哪些思考强度（和服务端 board-core/providers.rs 一致）
   - 常用供应商的预填：真实接口地址和常用模型（API key 要自己填）
   - 一台机器上一个 agent 的写入情况（providerCell）、地址相同的接口并成一行（endpointLines）
   - 一台机器上一个 agent 的配置文件实际写什么（effectiveProvider）：机器详情据此写摘要、过滤下拉
   - 编辑表单：空白的一项、校验、整理成要保存的
   ========================================================================== */
import type { AgentProvider, HostAgent, HostInfo, Provider, ProviderEndpoint, ProviderView } from './types';

/** 三种接口 */
export const PROVIDER_INTERFACES: { id: ProviderEndpoint['interface']; name: string; short: string; placeholder: string }[] = [
  { id: 'anthropic', name: 'Anthropic Messages', short: 'Anthropic', placeholder: 'https://api.anthropic.com' },
  { id: 'openai-chat', name: 'OpenAI Chat Completions', short: 'OpenAI Chat', placeholder: 'https://api.openai.com/v1' },
  { id: 'openai-responses', name: 'OpenAI Responses', short: 'OpenAI Responses', placeholder: 'https://api.openai.com/v1' },
];

/** 机器设置里「本机自己管」的写法（和服务端一致） */
export const PROVIDER_LOCAL = '@local';

/** 内置 agent：名称、认哪些接口、能选的思考强度（从低到高） */
export const PROVIDER_AGENTS: { id: string; name: string; protocols: string[]; efforts: string[] }[] = [
  { id: 'claude', name: 'Claude Code', protocols: ['anthropic'], efforts: ['low', 'medium', 'high', 'xhigh', 'max'] },
  // Codex 的 model_providers 走 Responses 接口（wire_api = "responses"）
  { id: 'codex', name: 'Codex', protocols: ['openai-responses'], efforts: ['minimal', 'low', 'medium', 'high'] },
  { id: 'pi', name: 'pi', protocols: ['anthropic', 'openai-chat', 'openai-responses'], efforts: ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] },
  { id: 'dsh', name: 'DeepSeek Harness', protocols: ['anthropic', 'openai-chat', 'openai-responses'], efforts: ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] },
];

export const PROVIDER_AGENT_NAMES: Record<string, string> = Object.fromEntries(PROVIDER_AGENTS.map(a => [a.id, a.name]));

export const interfaceName = (id: string) => PROVIDER_INTERFACES.find(i => i.id === id)?.short ?? id;

/** 常用供应商：接口地址和常用模型（2026 年核实过的写法；API key 要自己填）。
 *  Anthropic 兼容接口的地址都不带 /v1：Claude Code 自己接 /v1/messages */
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

export const blankProvider = (): Provider => ({ name: '', endpoints: [], apiKey: '', models: [] });

export const blankAgentProvider = (agent: string): AgentProvider => ({ agent, provider: '', model: '', effort: '' });

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
    version: p.version,
  };
}

/* ------------------------------ 各机器的写入状态 ------------------------------ */

/** 一台机器上一个 agent 的写入情况（界面上一个小胶囊）：
 *  old 旧版 board host 不同步 · missing 没找到这个 agent · local 机器设置为本机自己管 · none 没有要写的（共用配置「不管」、机器也没单独指定）·
 *  offline 该写、机器离线 · synced 写进去的就是该写的 · failed / unsupported 上报的失败、不支持（不管带没带供应商名都算）· pending 该写、还没报 */
export type ProviderCellKind = 'synced' | 'pending' | 'local' | 'failed' | 'unsupported' | 'offline' | 'old' | 'missing' | 'none';
export type ProviderCell = { kind: ProviderCellKind; why?: string | null; provider?: string };

export const PROVIDER_CELL_WORDS: Record<ProviderCellKind, string> = {
  synced: '已写入',
  pending: '正在写入',
  local: '本机自己管',
  failed: '失败',
  unsupported: '不支持',
  offline: '离线',
  old: '版本太旧',
  missing: '没装',
  none: '不改',
};

export function providerCell(h: HostInfo, agent: string, view: ProviderView | null): ProviderCell {
  if (h.providers == null) return { kind: 'old', why: `board ${h.version || '?'} 不同步供应商：升级这台机器的 board` };
  const a = h.agents.find(x => x.id === agent);
  if (!a?.found) return { kind: 'missing' };
  if (a.run_provider === PROVIDER_LOCAL) return { kind: 'local', why: '机器详情里设置为本机自己管：不改它的配置文件' };
  const want = a.run_provider || sharedOf(view, agent).provider;
  if (!want) return { kind: 'none' };
  if (!h.online) return { kind: 'offline', why: '上线后写入', provider: want };
  const st = h.providers.find(s => s.agent === agent);
  if (st?.state === 'synced' && st.provider === want) return { kind: 'synced', why: `${st.provider} · ${st.model}`, provider: want };
  if (st?.state === 'failed' || st?.state === 'unsupported') return { kind: st.state, why: st.why, provider: want };
  return { kind: 'pending', why: 'board host 收到后改写配置文件、上报', provider: want };
}

/** 接口和地址：地址相同的接口并成一行（OpenAI Chat · OpenAI Responses → 同一个地址） */
export function endpointLines(p: Pick<Provider, 'endpoints'>) {
  const m = new Map<string, string[]>();
  for (const e of p.endpoints) m.set(e.url, [...(m.get(e.url) ?? []), interfaceName(e.interface)]);
  return [...m.entries()].map(([url, names]) => ({ url, names }));
}

/** 共用配置里一个内置 agent 的分配（没有时是「不管」） */
export const sharedOf = (view: ProviderView | null, agent: string): AgentProvider => view?.agents.find(a => a.agent === agent) ?? blankAgentProvider(agent);

/** 给一个 agent 换供应商：模型取新供应商的第一个，强度保留；换成「不管」时都清空 */
export const assignProvider = (a: AgentProvider, provider: string, view: ProviderView): AgentProvider => ({
  agent: a.agent,
  provider,
  model: provider ? (view.providers.find(p => p.name === provider)?.models[0] ?? '') : '',
  effort: provider ? a.effort : '',
});

/* ------------------------------ 机器详情 ------------------------------ */

/** 这台机器上一个 agent 的配置文件实际写什么：
 *  - mode：follow 跟随共用配置 / local 本机自己管（不改配置文件）/ own 这台机器单独指定（机器设置优先）
 *  - provider、model、effort：写进配置文件的；模型、强度跟着共用配置里给这个供应商选的，单独指定别的供应商时用它的第一个模型、强度不写
 *  - gone：设置的供应商在共用配置里没有了
 *  机器设置里的模型、强度只是派发那一次的覆盖，不算在内 */
export function effectiveProvider(view: ProviderView | null, a: HostAgent) {
  const shared = sharedOf(view, a.id);
  const mode: 'follow' | 'local' | 'own' = a.run_provider === PROVIDER_LOCAL ? 'local' : a.run_provider ? 'own' : 'follow';
  const name = mode === 'local' ? '' : mode === 'own' ? (a.run_provider ?? '') : shared.provider;
  const provider = view?.providers.find(p => p.name === name) ?? null;
  const same = shared.provider === name;
  const model = provider ? (same && provider.models.includes(shared.model) ? shared.model : (provider.models[0] ?? '')) : '';
  const effort = provider && same ? shared.effort : '';
  return { mode, shared, name, provider, model, effort, gone: !!name && !provider };
}

/** 这个 agent 能选的供应商：接口对得上 */
export const providersFor = (view: ProviderView | null, agent: string) => {
  const meta = PROVIDER_AGENTS.find(a => a.id === agent);
  const want = meta?.protocols ?? [];
  return (view?.providers ?? []).filter(p => p.endpoints.some(e => want.includes(e.interface)));
};
