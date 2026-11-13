/* ==========================================================================
   模型供应商共用配置（PRD §10.12「共用配置」）：界面上用到的纯函数和常量
   - 三种接口、四个内置 agent 认哪些接口、能选哪些思考强度（和服务端 board-core/providers.rs 一致）
   - 常用供应商的预填：真实接口地址和常用模型（API key 要自己填）
   - 每个供应商在各台机器上的写入状态（providerRows）、汇总
   - 一台机器上一个 agent 实际用的供应商和模型（agentProviderView）：机器详情里的下拉据此过滤
   - 编辑表单：空白的一项、校验、整理成要保存的
   ========================================================================== */
import type { AgentProvider, HostAgent, HostInfo, Provider, ProviderEndpoint, ProviderState, ProviderView } from './types';

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

/** 一行一个模型（空行去掉） */
export const modelsOf = (text: string) =>
  text
    .split('\n')
    .map(s => s.trim())
    .filter(Boolean);

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

export type ProviderCell = { agent: string; state: ProviderState['state'] | 'pending'; provider?: string; model?: string; why?: string | null };
export type ProviderRow = { host: HostInfo; old: boolean; cells: ProviderCell[] };

export const PROVIDER_WORDS: Record<ProviderCell['state'], string> = {
  synced: '已写入',
  local: '本机自己管',
  unsupported: '不支持',
  failed: '失败',
  pending: '正在写入',
};

/** 一个供应商在每台机器上的写入状态：旧版 board host 不同步（old）；
 *  共用配置里给它分了 agent、机器上又找到了的，还没报的算「正在写入」（机器设成「本机自己管」的不算）；
 *  这台机器把它设成「本机自己管」「不支持」的也列出来，写明原因 */
export function providerRows(hosts: HostInfo[], p: Provider, agents: AgentProvider[]): ProviderRow[] {
  const assigned = agents.filter(a => a.provider === p.name);
  return hosts.map(h => {
    if (h.providers == null) return { host: h, old: true, cells: [] };
    const cells: ProviderCell[] = h.providers
      .filter(s => s.provider === p.name)
      .map(s => ({ agent: s.agent, state: s.state, provider: s.provider, model: s.model, why: s.why }));
    if (h.online) {
      for (const a of assigned) {
        const st = h.providers.find(x => x.agent === a.agent);
        const ha = h.agents.find(x => x.id === a.agent);
        if (!ha?.found || cells.some(c => c.agent === a.agent)) continue;
        if (st && (st.state === 'local' || st.state === 'unsupported')) cells.push({ agent: a.agent, state: st.state, why: st.why });
        else if (!st && ha.run_provider !== PROVIDER_LOCAL) cells.push({ agent: a.agent, state: 'pending' });
      }
    }
    return { host: h, old: false, cells };
  });
}

export function providerSummary(rows: ProviderRow[]) {
  let synced = 0;
  let used = 0;
  let bad = 0;
  let old = 0;
  for (const r of rows) {
    if (r.old) {
      old++;
      continue;
    }
    // 本机自己管、不支持：这台机器不会为这个供应商写文件，不算进分母
    const active = r.cells.filter(c => c.state !== 'local' && c.state !== 'unsupported');
    if (!active.length) continue;
    used++;
    if (active.some(c => c.state === 'failed')) bad++;
    else if (active.every(c => c.state === 'synced')) synced++;
  }
  return { synced, used, bad, old };
}

/** 用着这个供应商的内置 agent */
export const agentsUsing = (agents: AgentProvider[], name: string) => agents.filter(a => a.provider === name);

/* ------------------------------ 机器详情 ------------------------------ */

/** 这台机器上一个 agent 实际用的供应商、模型：
 *  - local：机器设置成「本机自己管」，不改配置文件
 *  - provider：共用配置里给它分的，或这台机器单独指定的（机器设置优先）
 *  - model：共用配置里选的那个（跟着供应商）；机器设置的模型只是派发时的覆盖，不算在内 */
export function agentProviderView(view: ProviderView | null, a: HostAgent): { local: boolean; shared: AgentProvider | null; provider: Provider | null } {
  const shared = view?.agents.find(x => x.agent === a.id) ?? null;
  if (a.run_provider === PROVIDER_LOCAL) return { local: true, shared, provider: null };
  const name = a.run_provider || shared?.provider || '';
  return { local: false, shared, provider: view?.providers.find(p => p.name === name) ?? null };
}

/** 这个 agent 能选的供应商：接口对得上 */
export const providersFor = (view: ProviderView | null, agent: string) => {
  const meta = PROVIDER_AGENTS.find(a => a.id === agent);
  const want = meta?.protocols ?? [];
  return (view?.providers ?? []).filter(p => p.endpoints.some(e => want.includes(e.interface)));
};
