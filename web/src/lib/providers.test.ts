import { describe, expect, it } from 'vitest';
import { agentGroups, endpointLines, maskKey, protocolChoices, providerCell, providerError, providersFor, runPick, runWords, toProviderSave, usesOf, worstCell, writableEfforts } from './providers';
import type { AgentMeta, HostAgent, HostInfo, Provider, ProviderView, SyncResult } from './types';

const META: AgentMeta[] = [
  { id: 'claude', name: 'Claude Code', protocols: ['anthropic'], efforts: ['low', 'medium', 'high', 'xhigh', 'max'] },
  { id: 'codex', name: 'Codex', protocols: ['openai-responses'], efforts: ['minimal', 'low', 'medium', 'high'] },
  { id: 'pi', name: 'pi', protocols: ['anthropic', 'openai-chat', 'openai-responses'], efforts: ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] },
];
const prov = (x: Partial<Provider> = {}): Provider => ({
  name: 'deepseek',
  endpoints: [{ interface: 'anthropic', url: 'https://api.deepseek.com/anthropic' }],
  apiKey: null,
  tail: '6789',
  models: ['deepseek-chat', 'deepseek-reasoner'],
  efforts: { 'deepseek-reasoner': ['low', 'high', 'max'] },
  ...x,
});
const claude = (x: Partial<HostAgent> = {}): HostAgent => ({ id: 'claude', name: 'Claude Code', found: true, version: '1', run_mode: 'own', run_provider: 'deepseek', run_model: 'deepseek-chat', ...x });
const last = (agents: SyncResult['agents']): SyncResult => ({ key: 'k', at: 1, agents, skills: { state: 'off', count: 1, dirs: [], replaced: [], links: [] }, mcp: { state: 'off', count: 0, agents: [], replaced: [] } });
const host = (x: Partial<HostInfo> = {}): HostInfo =>
  ({ id: 1, name: 'h', online: true, version: '0.2.0', agents: [claude(), { id: 'codex', name: 'Codex', found: true, version: '1', run_mode: 'own', run_provider: '' }], sync: { able: true }, ...x }) as HostInfo;

describe('模型供应商共用配置', () => {
  it('能写的强度：模型标了的、agent 也能写的；没标的不能选', () => {
    const p = prov();
    expect(writableEfforts(META[0], p, 'deepseek-reasoner')).toEqual(['low', 'high', 'max']);
    // codex 写不了 max
    expect(writableEfforts(META[1], p, 'deepseek-reasoner')).toEqual(['low', 'high']);
    expect(writableEfforts(META[0], p, 'deepseek-chat')).toEqual([]);
    expect(writableEfforts(undefined, p, 'deepseek-reasoner')).toEqual([]);
  });

  it('写入情况：旧版、没装、不改、没配、离线、正在写入、已写入、失败、不支持', () => {
    const view: ProviderView = { providers: [prov()], meta: META };
    const kind = (x: Partial<HostInfo>, agent = 'claude') => providerCell(host(x), agent, view).kind;
    expect(kind({ sync: { able: false } })).toBe('old');
    expect(kind({ agents: [claude({ found: false })] })).toBe('missing');
    expect(kind({ agents: [claude({ run_mode: 'off' })] })).toBe('local');
    // 指定了却没有供应商：没有要写的
    expect(kind({}, 'codex')).toBe('none');
    expect(kind({ online: false })).toBe('offline');
    expect(kind({ sync: { able: true, ask: { id: 1, key: 'k', by: 'me', at: 1 } } })).toBe('pending');
    expect(kind({ sync: { able: true, last: last([{ agent: 'claude', state: 'synced', provider: 'deepseek', model: 'deepseek-chat' }]) } })).toBe('synced');
    expect(kind({ sync: { able: true, last: last([]) } })).toBe('pending');
    const bad = providerCell(host({ sync: { able: true, last: last([{ agent: 'claude', state: 'failed', why: '文件解析不了' }]) } }), 'claude', view);
    expect(bad).toMatchObject({ kind: 'failed', why: '文件解析不了', provider: 'deepseek' });
    expect(kind({ sync: { able: true, last: last([{ agent: 'claude', state: 'unsupported', why: 'WSL' }]) } })).toBe('unsupported');
    // 按它指定的供应商算
    const own = providerCell(host({ agents: [claude({ run_mode: 'own', run_provider: 'other' })] }), 'claude', view);
    expect(own.provider).toBe('other');
  });

  it('机器上要写成什么：不改 / 指定；强度模型不支持时写配置时不写', () => {
    const view: ProviderView = {
      providers: [prov(), prov({ name: 'other', endpoints: [{ interface: 'openai-responses', url: 'https://x' }], models: ['o1', 'o2'], efforts: {} })],
      meta: META,
    };
    expect(runPick(view, claude({ run_mode: 'off' }))).toMatchObject({ mode: 'off', provider: '' });
    expect(runPick(view, claude({ run_mode: undefined }))).toMatchObject({ mode: 'off' });
    const pick = runPick(view, claude({ run_model: 'deepseek-reasoner', run_effort: 'max' }));
    expect(pick).toMatchObject({ mode: 'own', provider: 'deepseek', model: 'deepseek-reasoner', effort: 'max', ep: { interface: 'anthropic' } });
    expect(pick.warn).toBeUndefined();
    // 模型留空用第一个
    expect(runPick(view, claude({ run_provider: 'other', run_model: null }))).toMatchObject({ mode: 'own', provider: 'other', model: 'o1', effort: '' });
    expect(runPick(view, claude({ run_provider: 'gone' }))).toMatchObject({ gone: true });
    // claude 只认 Anthropic：other 只有 Responses，接口对不上
    expect(runPick(view, claude({ run_provider: 'other' })).ep).toBeUndefined();
    // 选的强度模型不支持
    const warn = runPick(view, claude({ run_mode: 'own', run_provider: 'deepseek', run_model: 'deepseek-chat', run_effort: 'high' }));
    expect(warn.warn).toBe('deepseek-chat 不支持强度 high：写配置时不写强度');
    expect(runWords(warn)).toBe('deepseek · deepseek-chat · 不写强度');
    expect(runWords({ provider: 'p', model: 'm', effort: '' })).toBe('p · m · 强度默认');
    // 供应商下拉只列接口对得上的（按服务端给的说明）
    expect(providersFor(view, 'claude').map(p => p.name)).toEqual(['deepseek']);
    expect(providersFor(view, 'codex').map(p => p.name)).toEqual(['other']);
    expect(providersFor(view, 'pi').map(p => p.name)).toEqual(['deepseek', 'other']);
  });

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

  it('接口：agent 认几种、供应商也给了几种时能选；没选或对不上时用供应商里第一个对得上的', () => {
    const multi = prov({
      name: 'relay',
      endpoints: [
        { interface: 'openai-chat', url: 'https://r/v1' },
        { interface: 'anthropic', url: 'https://r' },
        { interface: 'openai-responses', url: 'https://r/v1' },
      ],
    });
    const view: ProviderView = { providers: [multi], meta: META };
    expect(protocolChoices(view, 'pi', 'relay').map(e => e.interface)).toEqual(['openai-chat', 'anthropic', 'openai-responses']);
    expect(protocolChoices(view, 'claude', 'relay').map(e => e.interface)).toEqual(['anthropic']);
    const pi = (x: Partial<HostAgent> = {}): HostAgent => ({ id: 'pi', name: 'pi', found: true, version: '1', run_mode: 'own', run_provider: 'relay', ...x });
    expect(runPick(view, pi()).ep).toEqual({ interface: 'openai-chat', url: 'https://r/v1' });
    expect(runPick(view, pi({ run_protocol: 'anthropic' })).ep).toEqual({ interface: 'anthropic', url: 'https://r' });
    // 指定的那种这个 agent 不认：回到第一个对得上的
    expect(runPick(view, claude({ run_provider: 'relay', run_protocol: 'openai-chat' })).ep?.interface).toBe('anthropic');
  });

  it('卡片的「在用」：指定了它的、按 agent 汇总，状态取最该看的', () => {
    const relay = prov({ name: 'relay', endpoints: [{ interface: 'anthropic', url: 'https://r' }, { interface: 'openai-responses', url: 'https://r/v1' }] });
    const view: ProviderView = { providers: [prov(), relay], meta: META };
    const codex = (x: Partial<HostAgent> = {}): HostAgent => ({ id: 'codex', name: 'Codex', found: true, version: '1', run_mode: 'own', run_provider: 'relay', ...x });
    const failed = { able: true, last: last([{ agent: 'claude', state: 'failed', why: 'x' }, { agent: 'codex', state: 'synced' }]) };
    const ok = { able: true, last: last([{ agent: 'claude', state: 'synced' }, { agent: 'codex', state: 'synced' }]) };
    const hosts = [
      host({ id: 1, name: 'a', agents: [claude({ run_provider: 'relay' }), codex()], sync: failed }),
      host({ id: 2, name: 'b', agents: [claude({ run_provider: 'relay' }), codex({ run_mode: 'off' })], sync: ok }),
      host({ id: 3, name: 'c', agents: [claude({ run_provider: 'deepseek' }), codex({ found: false })], sync: ok }),
    ];
    const uses = usesOf(hosts, view, 'relay');
    expect(uses.map(u => `${u.h.name}/${u.a.id}`)).toEqual(['a/claude', 'a/codex', 'b/claude']);
    const groups = agentGroups(uses, view);
    expect(groups.map(g => [g.meta.id, g.uses.length])).toEqual([
      ['claude', 2],
      ['codex', 1],
    ]);
    expect(worstCell(groups[0].uses)).toBe('failed');
    expect(worstCell(groups[1].uses)).toBe('synced');
    expect(usesOf(hosts, view, 'deepseek').map(u => u.h.name)).toEqual(['c']);
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
