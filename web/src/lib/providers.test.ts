import { describe, expect, it } from 'vitest';
import { assignProvider, effectiveProvider, endpointLines, maskKey, providerCell, providerError, providersFor, toProviderSave } from './providers';
import type { AgentProvider, HostInfo, Provider, ProviderView } from './types';

const prov = (x: Partial<Provider> = {}): Provider => ({
  name: 'deepseek',
  endpoints: [{ interface: 'anthropic', url: 'https://api.deepseek.com/anthropic' }],
  apiKey: null,
  tail: '6789',
  models: ['deepseek-chat', 'deepseek-reasoner'],
  ...x,
});
const assign = (x: Partial<AgentProvider> = {}): AgentProvider => ({ agent: 'claude', provider: 'deepseek', model: 'deepseek-chat', effort: '', ...x });
const host = (name: string, x: Partial<HostInfo>): HostInfo =>
  ({
    id: 1,
    name,
    online: true,
    agents: [
      { id: 'claude', name: 'Claude Code', found: true, version: '1' },
      { id: 'codex', name: 'Codex', found: true, version: '1' },
    ],
    ...x,
  }) as HostInfo;

describe('模型供应商共用配置', () => {
  it('写入情况：旧版、没装、本机自己管、不改、离线、已写入、失败、不支持、正在写入', () => {
    const view: ProviderView = { providers: [prov()], agents: [assign()] };
    const kind = (x: Partial<HostInfo>, agent = 'claude') => providerCell(host('h', x), agent, view).kind;
    expect(kind({ providers: null })).toBe('old');
    expect(kind({ providers: [], agents: [{ id: 'claude', name: 'Claude Code', found: false, version: '' }] })).toBe('missing');
    expect(kind({ providers: [], agents: [{ id: 'claude', name: 'Claude Code', found: true, version: '1', run_provider: '@local' }] })).toBe('local');
    // 共用配置里是「不管」、机器也没单独指定：没有要写的
    expect(kind({ providers: [] }, 'codex')).toBe('none');
    expect(kind({ providers: [], online: false })).toBe('offline');
    expect(kind({ providers: [{ agent: 'claude', state: 'synced', provider: 'deepseek', model: 'deepseek-chat' }] })).toBe('synced');
    // 写进去的是别的供应商（还没按新设置改写）：正在写入
    expect(kind({ providers: [{ agent: 'claude', state: 'synced', provider: 'old', model: 'x' }] })).toBe('pending');
    expect(kind({ providers: [] })).toBe('pending');
    // 失败不带供应商名（例如「共用配置里没有供应商 X」）也要显示出来，不能当成「没有 agent 用它」
    const bad = providerCell(host('h', { providers: [{ agent: 'claude', state: 'failed', why: '共用配置里没有供应商「deepseek」' }] }), 'claude', view);
    expect(bad).toMatchObject({ kind: 'failed', why: '共用配置里没有供应商「deepseek」', provider: 'deepseek' });
    expect(kind({ providers: [{ agent: 'claude', state: 'unsupported', why: '接口对不上' }] })).toBe('unsupported');
    // 这台机器单独指定了供应商：按它算，共用配置是「不管」也一样
    expect(kind({ providers: [{ agent: 'codex', state: 'synced', provider: 'deepseek', model: 'deepseek-chat' }], agents: [{ id: 'codex', name: 'Codex', found: true, version: '1', run_provider: 'deepseek' }] }, 'codex')).toBe('synced');
  });

  it('地址相同的接口并成一行；换供应商时模型取它的第一个、强度保留', () => {
    const p = prov({ endpoints: [
      { interface: 'openai-chat', url: 'https://x/v1' },
      { interface: 'openai-responses', url: 'https://x/v1' },
      { interface: 'anthropic', url: 'https://x/anthropic' },
    ] });
    expect(endpointLines(p)).toEqual([
      { url: 'https://x/v1', names: ['OpenAI Chat', 'OpenAI Responses'] },
      { url: 'https://x/anthropic', names: ['Anthropic'] },
    ]);
    const view: ProviderView = { providers: [prov(), prov({ name: 'other', models: ['m1'] })], agents: [] };
    expect(assignProvider(assign({ effort: 'high' }), 'other', view)).toEqual({ agent: 'claude', provider: 'other', model: 'm1', effort: 'high' });
    expect(assignProvider(assign({ effort: 'high' }), '', view)).toEqual({ agent: 'claude', provider: '', model: '', effort: '' });
  });

  it('机器详情写进配置文件的：本机自己管 > 这台机器单独指定 > 共用配置', () => {
    const view: ProviderView = {
      providers: [prov(), prov({ name: 'other', endpoints: [{ interface: 'openai-responses', url: 'https://x' }], models: ['o1', 'o2'] })],
      agents: [assign({ model: 'deepseek-reasoner', effort: 'max' })],
    };
    const agent = { id: 'claude', name: 'Claude Code', found: true, version: '1' };
    expect(effectiveProvider(view, agent)).toMatchObject({ mode: 'follow', name: 'deepseek', model: 'deepseek-reasoner', effort: 'max', gone: false });
    expect(effectiveProvider(view, { ...agent, run_provider: '@local' })).toMatchObject({ mode: 'local', provider: null, model: '' });
    // 单独指定别的供应商：用它的第一个模型，强度不写
    expect(effectiveProvider(view, { ...agent, run_provider: 'other' })).toMatchObject({ mode: 'own', name: 'other', model: 'o1', effort: '' });
    expect(effectiveProvider(view, { ...agent, run_provider: 'gone' })).toMatchObject({ mode: 'own', provider: null, gone: true });
    expect(effectiveProvider({ providers: [prov()], agents: [] }, agent)).toMatchObject({ mode: 'follow', provider: null, gone: false });
    // 供应商下拉只列接口对得上的
    expect(providersFor(view, 'claude').map(p => p.name)).toEqual(['deepseek']);
    expect(providersFor(view, 'codex').map(p => p.name)).toEqual(['other']);
    expect(providersFor(view, 'pi').map(p => p.name)).toEqual(['deepseek', 'other']);
  });

  it('表单：名称、接口、地址、密钥、模型；修改时密钥留空保留原值', () => {
    expect(maskKey(prov())).toBe('••••6789');
    expect(providerError(prov({ name: 'bad name' }), [], false)).toContain('名称');
    expect(providerError(prov(), ['deepseek'], false)).toContain('已经有');
    expect(providerError(prov(), ['deepseek'], true)).toBeNull();
    expect(providerError(prov({ endpoints: [] }), [], false)).toContain('接口');
    expect(providerError(prov({ endpoints: [{ interface: 'anthropic', url: 'ftp://x' }] }), [], false)).toContain('http');
    expect(providerError(prov({ apiKey: '', tail: null }), [], false)).toContain('API key');
    expect(providerError(prov({ apiKey: '', tail: '6789' }), ['deepseek'], true)).toBeNull();
    expect(providerError(prov({ models: [], apiKey: 'k' }), [], false)).toContain('模型');
    expect(providerError(prov({ models: ['bad model'], apiKey: 'k' }), [], false)).toContain('模型名');
    const out = toProviderSave(prov({ apiKey: ' sk-new ', endpoints: [{ interface: 'anthropic', url: ' https://a ' }, { interface: 'openai-chat', url: ' ' }] }));
    expect(out).toMatchObject({ name: 'deepseek', apiKey: 'sk-new', endpoints: [{ interface: 'anthropic', url: 'https://a' }] });
    expect(out).not.toHaveProperty('tail');
  });
});
