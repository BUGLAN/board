/* ==========================================================================
   「共用配置」的模型供应商一节（PRD §10.12「共用配置」）
   - 供应商：名称、接口（三种，各填地址）、API key（只存在服务端，界面上只显示末尾 4 位）、模型（一行一个）
   - 添加时能预填常用供应商的真实接口地址和常用模型；改、删要等服务端（按钮转圈、禁用直到返回）
   - 删掉时确认，并列出用着它的 agent
   - 每个内置 agent 用哪个供应商、哪个模型、什么强度：改了当帧生效，保存状态画在那一行旁边；被拒绝时那一行回到原处
   - 每个供应商展开看每台机器、每个 agent 写没写进去；旧版 board host 那一行写要升级
   ========================================================================== */
import { useEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import { agentsUsing, blankAgentProvider, blankProvider, interfaceName, maskKey, modelsOf, PROVIDER_AGENT_NAMES, PROVIDER_AGENTS, PROVIDER_INTERFACES, PROVIDER_PRESETS, PROVIDER_WORDS, providerError, providerRows, providerSummary, toProviderSave } from '../../lib/providers';
import type { AgentProvider, HostInfo, Provider } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, openHost, toast, toastError } from '../../store/ui';
import { Select, Sync, useNow, useSync, type SyncState } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon, Dot } from './bits';
import { useRemoteList } from './useRemoteList';

/** 把一项按 agent 换掉（没有就加上） */
const upsert = (list: AgentProvider[], a: AgentProvider) => (list.some(x => x.agent === a.agent) ? list.map(x => (x.agent === a.agent ? a : x)) : [...list, a]);

export function ProvidersSection({ hosts }: { hosts: HostInfo[] }) {
  const ws = useData(s => s.ws);
  const now = useNow(10000);
  const { list, err, refresh: load } = useRemoteList(() => api.providers(), ws?.hosts);
  const [open, setOpen] = useState<string | null>(null);
  // 正在编辑的：null 没有；name 为空串是新加
  const [edit, setEdit] = useState<{ p: Provider; editing: boolean } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  // 每个 agent 的分配：本地先改（当帧生效），存的时候记保存状态；被拒绝时只回滚这一行
  const [agents, setAgents] = useState<AgentProvider[]>([]);
  const saving = useRef(new Set<string>());
  const { sync: agentSync, track } = useSync();
  useEffect(() => {
    if (list && saving.current.size === 0) setAgents(list.agents);
  }, [list]);
  const save = async () => {
    if (!edit || !list) return;
    const why = providerError(edit.p, list.providers.map(x => x.name), edit.editing);
    if (why) {
      toast({ type: 'error', title: '没有保存', text: why });
      return;
    }
    setBusy('save');
    try {
      const out = await optimistic(null, () => api.putProvider(toProviderSave(edit.p)));
      setEdit(null);
      setOpen(out.name);
      await load();
      toast({ type: 'success', title: `已保存 ${out.name}`, text: '在线的机器马上改写各 agent 的配置文件', duration: 2200 });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const remove = async (p: Provider, users: AgentProvider[]) => {
    const names = users.map(a => PROVIDER_AGENT_NAMES[a.agent] ?? a.agent);
    const ok = await confirm({
      title: `删掉模型供应商 ${p.name}？`,
      text: names.length
        ? `用着它的 ${names.join('、')} 会回到「不管」；各台机器按备份恢复这些 agent 配置文件里原来的值。`
        : '各台机器按备份恢复这个供应商写进 agent 配置文件的内容。',
      ok: '删掉',
      icon: 'trash',
      tone: 'warn',
    });
    if (!ok) return;
    setBusy(`rm:${p.name}`);
    try {
      await optimistic(null, () => api.removeProvider(p.name));
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const saveAgent = (next: AgentProvider) => {
    const id = next.agent;
    const before = agents.find(x => x.agent === id) ?? blankAgentProvider(id);
    saving.current.add(id);
    setAgents(x => upsert(x, next));
    const done = api
      .putProviderAgent(next)
      .then(out => setAgents(x => upsert(x, out)))
      .catch(e => {
        setAgents(x => upsert(x, before));
        throw e;
      })
      .finally(() => {
        saving.current.delete(id);
        void load();
      });
    track(`agent:${id}`, done);
  };

  return (
    <section className="sc sc-providers">
      <div className="sc-head">
        <div>
          <h2>模型供应商</h2>
          <p>
            每台机器的 board host 把这里的供应商写进 Claude Code、Codex、pi、dsh 的用户级配置文件（地址、API key、模型），agent 直接连供应商，不经过 Board。
            API key 只存在服务端，界面上只显示末尾 4 位，完整值只发给这台机器用的 board host。第一次改写前会备份原来的值，改成「不管」或删掉供应商时恢复。
          </p>
        </div>
        <div className="sc-acts">
          <button className="btn sm" type="button" disabled={!!busy || !!edit} onClick={() => setEdit({ p: blankProvider(), editing: false })}>
            <Icon name="plus" />
            添加
          </button>
        </div>
      </div>
      {err && <p className="hp-note lh-warn">取不到模型供应商配置：{err}</p>}
      {edit && <ProviderForm value={edit.p} editing={edit.editing} busy={busy === 'save'} onChange={p => setEdit({ ...edit, p })} onSave={() => void save()} onCancel={() => setEdit(null)} />}
      {!list && !err && <p className="hp-note">正在读取…</p>}
      {list && !list.providers.length && !edit && <p className="hp-note">还没有：点「添加」，或从常用供应商预填一个</p>}
      {list && list.providers.length > 0 && (
        <div className="sc-list">
          {list.providers.map(p => {
            const users = agentsUsing(agents, p.name);
            const rows = providerRows(hosts, p, agents);
            const sum = providerSummary(rows);
            const on = open === p.name;
            return (
              <div key={p.name} className={cx('sc-item', on && 'open')}>
                <div className="sc-row">
                  <button type="button" className="sc-toggle" aria-expanded={on} onClick={() => setOpen(on ? null : p.name)}>
                    <Icon name={on ? 'chevDown' : 'chevRight'} />
                    <b>{p.name}</b>
                    <span className="sc-fixed">{p.tail ? maskKey(p) : '没填 key'}</span>
                  </button>
                  <span className="sc-meta" title={p.endpoints.map(e => `${interfaceName(e.interface)}：${e.url}`).join('\n')}>
                    {p.endpoints.map(e => interfaceName(e.interface)).join(' · ')}
                    {' · '}
                    {p.models.length} 个模型
                    {' · '}
                    {users.length ? users.map(a => PROVIDER_AGENT_NAMES[a.agent] ?? a.agent).join('、') : '还没有 agent 用'}
                    {p.updatedAt ? ` · ${since(p.updatedAt, now)}更新` : ''}
                  </span>
                  <span className={cx('sc-sum', sum.bad ? 'bad' : sum.used && sum.synced === sum.used ? 'ok' : '')}>
                    {sum.used === 0
                      ? rows.every(r => r.old) && rows.length > 0
                        ? '机器的 board 版本太旧'
                        : '还没有机器写入'
                      : `${sum.synced}/${sum.used} 台已写入${sum.bad ? ` · ${sum.bad} 台有问题` : ''}${sum.old ? ` · ${sum.old} 台不同步` : ''}`}
                  </span>
                  <button className="icon-btn sm" type="button" aria-label={`修改 ${p.name}`} data-tip="修改" disabled={!!busy || !!edit} onClick={() => setEdit({ p: { ...p, apiKey: '', endpoints: p.endpoints.map(e => ({ ...e })) }, editing: true })}>
                    <Icon name="edit" />
                  </button>
                  <button className={cx('icon-btn sm', busy === `rm:${p.name}` && 'is-busy')} type="button" aria-label={`删掉 ${p.name}`} data-tip="删掉" data-tip-align="end" disabled={!!busy} onClick={() => void remove(p, users)}>
                    <Icon name="trash" />
                  </button>
                </div>
                {on && (
                  <div className="sc-hosts">
                    <div className="sc-vars">
                      {p.endpoints.map(e => (
                        <span key={e.interface}>
                          <b>{interfaceName(e.interface)}</b> <code>{e.url}</code>
                        </span>
                      ))}
                      <span>
                        <b>模型</b> {p.models.join('、')}
                      </span>
                    </div>
                    {rows.map(r => (
                      <div key={r.host.id} className="sc-host">
                        <button type="button" className="sc-host-name" onClick={() => openHost(r.host.name)}>
                          <Dot h={r.host} running={0} />
                          {r.host.name}
                        </button>
                        <div className="sc-cells">
                          {r.old ? (
                            <span className="sc-cell old">board 版本太旧（{r.host.version || '?'}），不同步：升级这台机器的 board</span>
                          ) : !r.cells.length ? (
                            <span className="sc-cell old">{r.host.online ? '这台机器上没有要用它的 agent' : '离线'}</span>
                          ) : (
                            r.cells.map(c => (
                              <span key={c.agent} className={cx('sc-cell', c.state === 'unsupported' ? 'pending' : c.state)}>
                                <AgentIcon id={c.agent} />
                                <b>{r.host.agents.find(a => a.id === c.agent)?.name ?? PROVIDER_AGENT_NAMES[c.agent] ?? c.agent}</b>
                                <em>{PROVIDER_WORDS[c.state]}</em>
                                {c.state === 'synced' && c.provider && <small>{c.provider} · {c.model}</small>}
                                {c.why && <small>{c.why}</small>}
                              </span>
                            ))
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
      {list && (
        <div className="sc-agents-map">
          <div className="sc-sub-head">
            <b>每个 agent 用哪个</b>
            <span className="hp-note">模型、强度写进配置文件，作为 agent 的默认；机器详情里还能按机器改成「本机自己管」或单独指定供应商。机器设置里的模型、强度只管 Board 派发的那一次。</span>
          </div>
          {PROVIDER_AGENTS.map(m => {
            const a = agents.find(x => x.agent === m.id) ?? blankAgentProvider(m.id);
            const p = list.providers.find(x => x.name === a.provider);
            const fits = list.providers.filter(x => x.endpoints.some(e => m.protocols.includes(e.interface)));
            return (
              <div key={m.id} className="sc-provider-agent">
                <span className="sc-provider-agent-name">
                  <AgentIcon id={m.id} />
                  <b>{m.name}</b>
                </span>
                <label className="sc-provider-f">
                  <span>供应商</span>
                  <Select
                    value={a.provider}
                    options={[{ value: '', label: '不管（不改配置文件）' }, ...fits.map(x => ({ value: x.name, label: x.name }))]}
                    onChange={v => saveAgent({ agent: m.id, provider: v, model: v ? (fits.find(x => x.name === v)?.models[0] ?? '') : '', effort: v ? a.effort : '' })}
                    aria-label={`${m.name} 用哪个供应商`}
                    data-tip="「不管」= 不碰这个 agent 的配置文件"
                  />
                </label>
                <label className="sc-provider-f">
                  <span>模型</span>
                  <Select
                    value={a.model}
                    disabled={!p}
                    options={p ? p.models.map(x => ({ value: x, label: x })) : [{ value: '', label: '先选供应商' }]}
                    onChange={v => saveAgent({ ...a, model: v })}
                    aria-label={`${m.name} 用哪个模型`}
                  />
                </label>
                <label className="sc-provider-f">
                  <span>强度</span>
                  <Select
                    value={a.effort}
                    disabled={!p}
                    options={[{ value: '', label: '默认' }, ...m.efforts.map(x => ({ value: x, label: x }))]}
                    onChange={v => saveAgent({ ...a, effort: v })}
                    aria-label={`${m.name} 的思考强度`}
                  />
                </label>
                <Sync s={agentSync[`agent:${m.id}`] as SyncState | undefined} />
              </div>
            );
          })}
          {!list.providers.length && <p className="hp-note">先添加一个供应商，这里才能选。</p>}
        </div>
      )}
    </section>
  );
}

function ProviderForm({ value: p, editing, busy, onChange, onSave, onCancel }: { value: Provider; editing: boolean; busy: boolean; onChange: (p: Provider) => void; onSave: () => void; onCancel: () => void }) {
  const [models, setModels] = useState(p.models.join('\n'));
  const setEndpoint = (id: string, url: string) => {
    const others = p.endpoints.filter(e => e.interface !== id);
    onChange({ ...p, endpoints: url.trim() ? [...others, { interface: id as Provider['endpoints'][number]['interface'], url }] : others });
  };
  const fill = (name: string) => {
    const preset = PROVIDER_PRESETS.find(x => x.name === name);
    if (!preset) return;
    setModels(preset.models.join('\n'));
    onChange({ ...p, name: preset.name, endpoints: preset.endpoints.map(e => ({ ...e })), models: preset.models });
  };
  return (
    <div className="sc-form">
      <div className="hp-add-row">
        <span className="hp-add-k">名称</span>
        <input className="input mono" value={p.name} disabled={editing} placeholder="deepseek" spellCheck={false} autoComplete="off" onChange={e => onChange({ ...p, name: e.target.value.trim() })} />
      </div>
      {!editing && (
        <div className="hp-add-row">
          <span className="hp-add-k">预填</span>
          <div className="sc-presets">
            {PROVIDER_PRESETS.map(x => (
              <button key={x.name} className="btn sm ghost" type="button" onClick={() => fill(x.name)}>
                {x.label}
              </button>
            ))}
            <span className="hp-note">只填接口地址和常用模型，API key 要自己填</span>
          </div>
        </div>
      )}
      {PROVIDER_INTERFACES.map(i => (
        <div className="hp-add-row" key={i.id}>
          <span className="hp-add-k">{i.name}</span>
          <input
            className="input mono"
            value={p.endpoints.find(e => e.interface === i.id)?.url ?? ''}
            placeholder={`${i.placeholder}（不用这个接口就留空）`}
            spellCheck={false}
            autoComplete="off"
            onChange={e => setEndpoint(i.id, e.target.value)}
          />
        </div>
      ))}
      <div className="hp-add-row">
        <span className="hp-add-k">API key</span>
        <input
          className="input mono"
          type="password"
          value={p.apiKey ?? ''}
          placeholder={editing && p.tail != null ? `${maskKey(p)}（不改就留空）` : 'sk-…'}
          spellCheck={false}
          autoComplete="off"
          onChange={e => onChange({ ...p, apiKey: e.target.value })}
        />
      </div>
      <div className="hp-add-row top">
        <span className="hp-add-k">模型</span>
        <textarea
          className="input mono sc-args"
          value={models}
          rows={4}
          placeholder={'一行一个，例如\ndeepseek-chat\ndeepseek-reasoner'}
          spellCheck={false}
          onChange={e => {
            setModels(e.target.value);
            onChange({ ...p, models: modelsOf(e.target.value) });
          }}
        />
      </div>
      <div className="lh-actions sc-form-acts">
        <button className="btn sm ghost" type="button" disabled={busy} onClick={onCancel}>
          取消
        </button>
        <button className={cx('btn sm brand', busy && 'is-busy')} type="button" disabled={busy} onClick={onSave}>
          保存
        </button>
      </div>
    </div>
  );
}
