import { describe, expect, it } from 'vitest';
import { agentProviderView, maskKey, modelsOf, providerError, providerRows, providerSummary, providersFor, toProviderSave } from './providers';
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
  it('状态：旧版不同步；该写的还没报是正在写入；本机自己管、不支持也列出来并带原因', () => {
    const rows = providerRows(
      [
        host('old', { providers: null }),
        host('new', { providers: [] }),
        host('ok', { providers: [{ agent: 'claude', state: 'synced', provider: 'deepseek', model: 'deepseek-reasoner' }] }),
        host('mine', { providers: [{ agent: 'claude', state: 'local', why: '这台机器设置为本机自己管' }] }),
        host('no', { providers: [{ agent: 'claude', state: 'unsupported', why: '配置文件里新加的 agent 不支持按共用配置改写' }] }),
        host('bad', { providers: [{ agent: 'claude', state: 'failed', provider: 'deepseek', why: 'settings.json 解析不了' }] }),
      ],
      prov(),
      [assign()],
    );
    expect(rows.map(r => (r.old ? 'old' : r.cells.map(c => `${c.agent}:${c.state}${c.why ? `（${c.why}）` : ''}`).join()))).toEqual([
      'old',
      'claude:pending',
      'claude:synced',
      'claude:local（这台机器设置为本机自己管）',
      'claude:unsupported（配置文件里新加的 agent 不支持按共用配置改写）',
      'claude:failed（settings.json 解析不了）',
    ]);
    // 本机自己管、不支持的机器不算进「N/M 台已写入」的分母
    expect(providerSummary(rows)).toEqual({ synced: 1, used: 3, bad: 1, old: 1 });
  });

  it('机器详情用哪个：本机自己管 > 这台机器单独指定 > 共用配置', () => {
    const view: ProviderView = { providers: [prov(), prov({ name: 'other', endpoints: [{ interface: 'openai-responses', url: 'https://x' }] })], agents: [assign()] };
    const agent = { id: 'claude', name: 'Claude Code', found: true, version: '1' };
    expect(agentProviderView(view, agent).provider?.name).toBe('deepseek');
    expect(agentProviderView(view, { ...agent, run_provider: '@local' })).toMatchObject({ local: true, provider: null });
    expect(agentProviderView(view, { ...agent, run_provider: 'other' }).provider?.name).toBe('other');
    // 供应商下拉只列接口对得上的
    expect(providersFor(view, 'claude').map(p => p.name)).toEqual(['deepseek']);
    expect(providersFor(view, 'codex').map(p => p.name)).toEqual(['other']);
    expect(providersFor(view, 'pi').map(p => p.name)).toEqual(['deepseek', 'other']);
  });

  it('表单：名称、接口、地址、密钥、模型；修改时密钥留空保留原值', () => {
    expect(maskKey(prov())).toBe('••••6789');
    expect(modelsOf(' deepseek-chat\n\ndeepseek-reasoner \n')).toEqual(['deepseek-chat', 'deepseek-reasoner']);
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
