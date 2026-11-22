/* ==========================================================================
   模型供应商共用配置（PRD §10.16「共用配置」）：界面上用到的纯函数和常量
   - 三种接口；内置 agent 认哪些接口、能写哪些强度从服务端取（ProviderView.meta，board-core 留一份）
   - 常用供应商的预填：真实接口地址和常用模型（API key 要自己填）
   - 地址相同的接口并成一行（endpointLines）。机器上不再给 agent 指定供应商（#576），没有各机器的写入情况
   - 编辑表单：空白的一项、校验、整理成要保存的
   ========================================================================== */
import type { AgentMeta, Provider, ProviderEndpoint, ProviderView } from './types';

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

/** 接口和地址：地址相同的接口并成一行（OpenAI Chat · OpenAI Responses → 同一个地址） */
export function endpointLines(p: Pick<Provider, 'endpoints'>) {
  const m = new Map<string, string[]>();
  for (const e of p.endpoints) m.set(e.url, [...(m.get(e.url) ?? []), interfaceName(e.interface)]);
  return [...m.entries()].map(([url, names]) => ({ url, names }));
}

/** 这个 agent 能选的供应商：接口对得上 */
export const providersFor = (view: ProviderView | null, agent: string) => {
  const want = metaOf(view, agent)?.protocols ?? [];
  return (view?.providers ?? []).filter(p => p.endpoints.some(e => want.includes(e.interface)));
};
