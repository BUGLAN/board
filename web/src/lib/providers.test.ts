import { describe, expect, it } from 'vitest';
import { endpointLines, maskKey, providerError, toProviderSave } from './providers';
import type { Provider } from './types';

const prov = (x: Partial<Provider> = {}): Provider => ({
  name: 'deepseek',
  endpoints: [{ interface: 'anthropic', url: 'https://api.deepseek.com/anthropic' }],
  apiKey: null,
  tail: '6789',
  models: ['deepseek-chat', 'deepseek-reasoner'],
  efforts: { 'deepseek-reasoner': ['low', 'high', 'max'] },
  ...x,
});
describe('模型供应商共用配置', () => {
  it('地址相同的接口并成一行', () => {
    const p = prov({
      endpoints: [
        { interface: 'openai-chat', url: 'https://x/v1' },
        { interface: 'openai-responses', url: 'https://x/v1' },
        { interface: 'anthropic', url: 'https://x/anthropic' },
      ],
    });
    expect(endpointLines(p)).toEqual([
      { url: 'https://x/v1', names: ['OpenAI Chat', 'OpenAI Responses'] },
      { url: 'https://x/anthropic', names: ['Anthropic'] },
    ]);
  });

  it('表单：名称、接口、地址、密钥、模型；修改时密钥留空保留原值；强度只存列表里有的模型', () => {
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
    const out = toProviderSave(
      prov({ apiKey: ' sk-new ', endpoints: [{ interface: 'anthropic', url: ' https://a ' }, { interface: 'openai-chat', url: ' ' }], efforts: { 'deepseek-chat': [], 'deepseek-reasoner': ['high'], gone: ['low'] } }),
    );
    expect(out).toMatchObject({ name: 'deepseek', apiKey: 'sk-new', endpoints: [{ interface: 'anthropic', url: 'https://a' }], efforts: { 'deepseek-reasoner': ['high'] } });
    expect(out).not.toHaveProperty('tail');
  });
});
