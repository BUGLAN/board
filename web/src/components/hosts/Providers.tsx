/* ==========================================================================
   「共用配置」的模型供应商一节（PRD §10.12「共用配置」）：一个供应商一张卡
   - 卡片上面：名称、API key 末尾 4 位、更新时间、修改、删除；接口和地址（地址相同的接口并成一行）、模型
   - 卡片下面：用这个供应商的 agent，每个一行：模型、强度两个下拉，各台机器写没写进去（小胶囊，点了打开那台机器），× 不再用它；
     「＋ 给 agent 用…」把它分给别的 agent（接口对得上的才列；已经用着别的供应商的写「从 X 换过来」）
   - 卡片下面一行：没用任何供应商的 agent（不改配置文件）；再下面一句话写 board 版本太旧、不同步供应商的机器
   - 供应商的增、改、删要等服务端（按钮转圈、禁用直到返回），删掉时确认并列出用着它的 agent；
     agent 的分配改了当帧生效，保存状态画在那一行旁边，被拒绝时那一行回到原处
   ========================================================================== */
import { useEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import {
  assignProvider,
  blankProvider,
  endpointLines,
  maskKey,
  PROVIDER_AGENT_NAMES,
  PROVIDER_AGENTS,
  PROVIDER_CELL_WORDS,
  providerCell,
  providerError,
  providersFor,
  sharedOf,
  toProviderSave,
  type ProviderCell,
} from '../../lib/providers';
import type { AgentProvider, HostInfo, Provider, ProviderView } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, openHost, toast, toastError } from '../../store/ui';
import { Select, Sync, useNow, useSync, type SyncState } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon } from './bits';
import { HowTip, OldHostsNote } from './SyncBits';
import { ProviderForm } from './ProviderForm';
import { useRemoteList } from './useRemoteList';

/** 把一项按 agent 换掉（没有就加上） */
const upsert = (list: AgentProvider[], a: AgentProvider) => (list.some(x => x.agent === a.agent) ? list.map(x => (x.agent === a.agent ? a : x)) : [...list, a]);

export function ProvidersSection({ hosts }: { hosts: HostInfo[] }) {
  const ws = useData(s => s.ws);
  const now = useNow(10000);
  const { list, err, refresh: load } = useRemoteList(() => api.providers(), ws?.hosts);
  // 正在编辑的：null 没有；editing = false 是新加
  const [edit, setEdit] = useState<{ p: Provider; editing: boolean } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  // 每个 agent 的分配：本地先改（当帧生效），存的时候记保存状态；被拒绝时只回滚这一行
  const [agents, setAgents] = useState<AgentProvider[]>([]);
  const saving = useRef(new Set<string>());
  const { sync: agentSync, track } = useSync();
  useEffect(() => {
    if (list && saving.current.size === 0) setAgents(list.agents);
  }, [list]);
  const view: ProviderView | null = list ? { providers: list.providers, agents } : null;
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
      await load();
      toast({ type: 'success', title: `已保存 ${out.name}`, text: '在线的机器马上改写各 agent 的配置文件', duration: 2200 });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const remove = async (p: Provider) => {
    const names = agents.filter(a => a.provider === p.name).map(a => PROVIDER_AGENT_NAMES[a.agent] ?? a.agent);
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
    const before = agents.find(x => x.agent === id) ?? sharedOf(null, id);
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
  const idle = view ? PROVIDER_AGENTS.filter(m => !sharedOf(view, m.id).provider) : [];
  const old = hosts.filter(h => h.providers == null);

  return (
    <section className="sc sc-providers">
      <div className="sc-head">
        <div>
          <h2>模型供应商</h2>
          <p>
            给 agent 选一个供应商，每台机器的 board host 就把地址、API key、模型写进那个 agent 的用户级配置文件，agent 直接连供应商，不经过 Board。{' '}
            <HowTip tip="第一次改写前备份 agent 配置文件里原来的值，改成「不管」或删掉供应商时恢复。API key 只存在服务端，界面上只显示末尾 4 位，完整值只发给要用它的机器上的 board host。" />
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
      {edit && (
        <ProviderForm value={edit.p} editing={edit.editing} busy={busy === 'save'} existing={list?.providers.map(x => x.name) ?? []} onChange={p => setEdit({ ...edit, p })} onSave={() => void save()} onCancel={() => setEdit(null)} />
      )}
      {!view && !err && <p className="hp-note">正在读取…</p>}
      {view && !view.providers.length && !edit && <p className="hp-note">还没有：点「添加」，或从常用供应商预填一个</p>}
      {view && view.providers.length > 0 && (
        <div className="pv-cards">
          {view.providers.map(p => {
            const users = PROVIDER_AGENTS.filter(m => sharedOf(view, m.id).provider === p.name);
            const addable = PROVIDER_AGENTS.filter(m => sharedOf(view, m.id).provider !== p.name && providersFor(view, m.id).some(x => x.name === p.name));
            return (
              <article key={p.name} className="pv-card">
                <header className="pv-head">
                  <b className="pv-name">{p.name}</b>
                  <span className="sc-fixed">{p.tail ? maskKey(p) : '没填 key'}</span>
                  <span className="pv-upd">{p.updatedAt ? `${since(p.updatedAt, now)}更新` : ''}</span>
                  <button
                    className="icon-btn sm"
                    type="button"
                    aria-label={`修改 ${p.name}`}
                    data-tip="修改地址、key、模型"
                    disabled={!!busy || !!edit}
                    onClick={() => setEdit({ p: { ...p, apiKey: '', endpoints: p.endpoints.map(e => ({ ...e })) }, editing: true })}
                  >
                    <Icon name="edit" />
                  </button>
                  <button className={cx('icon-btn sm', busy === `rm:${p.name}` && 'is-busy')} type="button" aria-label={`删掉 ${p.name}`} data-tip="删掉" data-tip-align="end" disabled={!!busy} onClick={() => void remove(p)}>
                    <Icon name="trash" />
                  </button>
                </header>
                <div className="pv-kv">
                  {endpointLines(p).map(l => (
                    <div className="pv-line" key={l.url}>
                      <span className="pv-k">{l.names.join(' · ')}</span>
                      <code>{l.url}</code>
                    </div>
                  ))}
                  <div className="pv-line">
                    <span className="pv-k">模型</span>
                    <span className="pv-models">
                      {p.models.map(m => (
                        <code key={m}>{m}</code>
                      ))}
                    </span>
                  </div>
                </div>
                <div className="pv-users">
                  {users.map(m => {
                    const a = sharedOf(view, m.id);
                    const cells = hosts.map(h => ({ h, c: providerCell(h, m.id, view) })).filter(x => x.c.kind !== 'none' && x.c.kind !== 'missing' && x.c.kind !== 'old');
                    return (
                      <div className="pv-user" key={m.id}>
                        <span className="pv-agent">
                          <AgentIcon id={m.id} />
                          <b>{m.name}</b>
                        </span>
                        <span className="pv-ctl">
                          <Select className="pv-sel" value={a.model} options={p.models.map(x => ({ value: x, label: x }))} onChange={v => saveAgent({ ...a, model: v })} aria-label={`${m.name} 用哪个模型`} />
                          <Select
                            className="pv-sel sm"
                            value={a.effort}
                            options={[{ value: '', label: '强度默认' }, ...m.efforts.map(x => ({ value: x, label: `强度 ${x}` }))]}
                            onChange={v => saveAgent({ ...a, effort: v })}
                            aria-label={`${m.name} 的思考强度`}
                          />
                          <Sync s={agentSync[`agent:${m.id}`] as SyncState | undefined} inline />
                        </span>
                        <span className="pv-cells">
                          {cells.length ? cells.map(({ h, c }) => <CellPill key={h.id} h={h} c={c} />) : <em>没有机器要写</em>}
                        </span>
                        <button
                          className="icon-btn sm pv-x"
                          type="button"
                          aria-label={`${m.name} 不用 ${p.name}`}
                          data-tip={`不再给 ${m.name} 用：配置文件恢复原来的值`}
                          data-tip-align="end"
                          onClick={() => saveAgent(assignProvider(a, '', view))}
                        >
                          <Icon name="close" />
                        </button>
                      </div>
                    );
                  })}
                  {!users.length && <p className="pv-none">还没有 agent 用它</p>}
                  {addable.length > 0 && (
                    <Select
                      className="pv-add"
                      value=""
                      placeholder="＋ 给 agent 用…"
                      options={addable.map(m => {
                        const cur = sharedOf(view, m.id).provider;
                        return { value: m.id, label: cur ? `${m.name}（从 ${cur} 换过来）` : m.name };
                      })}
                      onChange={id => saveAgent(assignProvider(sharedOf(view, id), p.name, view))}
                      aria-label={`把 ${p.name} 给哪个 agent 用`}
                    />
                  )}
                </div>
              </article>
            );
          })}
        </div>
      )}
      {view && view.providers.length > 0 && idle.length > 0 && (
        <p className="pv-idle">
          <span className="pv-idle-k">不改配置文件</span>
          {idle.map(m => (
            <span key={m.id} className="pv-idle-a">
              <AgentIcon id={m.id} />
              {m.name}
            </span>
          ))}
          <span className="pv-idle-hint">在供应商卡片里「给 agent 用」</span>
        </p>
      )}
      {view && <OldHostsNote hosts={old} what="供应商" verb="写入" />}
    </section>
  );
}

/** 一台机器上一个 agent 的写入情况：小胶囊（状态标记、机器名、状态），悬停看原因，点了打开那台机器 */
function CellPill({ h, c }: { h: HostInfo; c: ProviderCell }) {
  return (
    <button type="button" className={cx('pv-cell', `k-${c.kind}`)} data-tip={c.why ?? undefined} onClick={() => openHost(h.name)}>
      <i className="pv-cell-mark" aria-hidden="true" />
      <b>{h.name}</b>
      <span>{PROVIDER_CELL_WORDS[c.kind]}</span>
    </button>
  );
}
