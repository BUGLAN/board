/* ==========================================================================
   「共用配置」的模型供应商一节（PRD §10.16「共用配置」）：一个供应商一张卡
   - 卡片上面：名称、API key 末尾 4 位、更新时间、修改、删除；接口和地址（地址相同的接口并成一行）、模型（带标了的强度）
   - 卡片底下「在用」：用它的角色（Agent → 分工），一个角色一枚（agent 图标、名字、模型），点一下去分工页
   - 供应商的增、改、删要等服务端（按钮转圈、禁用直到返回）；还有角色在用的删不了（先改角色）
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import {
  blankProvider,
  endpointLines,
  maskKey,
  metasOf,
  providerError,
  toProviderSave,
} from '../../lib/providers';
import type { Provider } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, setTab, toast, toastError } from '../../store/ui';
import { goSec } from '../agent/AgentNav';
import { Tip, useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon } from './bits';
import { HowTip } from './SyncBits';
import { ProviderForm } from './ProviderForm';
import { useRemoteList } from './useRemoteList';

export function ProvidersSection() {
  const ws = useData(s => s.ws);
  const now = useNow(10000);
  const { list: view, err, refresh: load } = useRemoteList(() => api.providers(), ws?.hosts);
  // 正在编辑的：null 没有；editing = false 是新加
  const [edit, setEdit] = useState<{ p: Provider; editing: boolean } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const save = async () => {
    if (!edit || !view) return;
    const why = providerError(edit.p, view.providers.map(x => x.name), edit.editing);
    if (why) {
      toast({ type: 'error', title: '没有保存', text: why });
      return;
    }
    setBusy('save');
    try {
      const out = await optimistic(null, () => api.putProvider(toProviderSave(edit.p)));
      setEdit(null);
      await load();
      toast({ type: 'success', title: `已保存 ${out.name}`, text: '用它的角色下次派发时生效', duration: 2200 });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const remove = async (p: Provider) => {
    const roles = (useData.getState().ws?.sched?.profiles ?? []).filter(r => r.provider === p.name).map(r => `「${r.name}」`);
    if (roles.length) {
      toast({ type: 'error', title: `角色${roles.join('')}还在用 ${p.name}`, text: '先在 Agent → 分工里把它们改成别的供应商或本机配置' });
      return;
    }
    const ok = await confirm({
      title: `删掉模型供应商 ${p.name}？`,
      text: '旧版 Board 以前写进各台机器 agent 配置文件的那组设置不会跟着删除。',
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
  return (
    <section className="sc sc-providers">
      <div className="sc-head">
        <div>
          <h2>模型供应商</h2>
          <p>
            供应商在这里加、改。哪个角色用哪个供应商、哪个模型，在 Agent → 分工里选；派发时 board host 只给这一次 agent 进程换上，agent 直接连供应商，不经过 Board。{' '}
            <HowTip tip="只对这次派发生效，不改 agent 自己的配置文件。以前旧版 Board 写进 agent 配置文件的那组设置不会自动清理；本机配置、没填模型的角色仍可能用到它，要换就在 agent 自己的配置里改。角色只能选模型标了的强度。API key 只存在服务端，界面上只显示末尾 4 位，完整值只发给要用它的机器上的 board host。" />
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
        <ProviderForm
          value={edit.p}
          editing={edit.editing}
          busy={busy === 'save'}
          existing={view?.providers.map(x => x.name) ?? []}
          meta={metasOf(view)}
          onChange={p => setEdit({ ...edit, p })}
          onSave={() => void save()}
          onCancel={() => setEdit(null)}
        />
      )}
      {!view && !err && <p className="hp-note">正在读取…</p>}
      {view && !view.providers.length && !edit && <p className="hp-note">还没有：点「添加」，或从常用供应商预填一个</p>}
      {view && view.providers.length > 0 && (
        <div className="pv-cards">
          {view.providers.map(p => (
            <ProviderCard
              key={p.name}
              p={p}
              now={now}
              busy={busy}
              editing={!!edit}
              onEdit={() => setEdit({ p: { ...p, apiKey: '', endpoints: p.endpoints.map(e => ({ ...e })), efforts: { ...(p.efforts ?? {}) } }, editing: true })}
              onRemove={() => void remove(p)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

/** 标了的强度：三档以上写成「low–max」 */
const effortRange = (es: string[]) => (es.length > 2 ? `${es[0]}–${es[es.length - 1]}` : es.join(' · '));

function ProviderCard({
  p,
  now,
  busy,
  editing,
  onEdit,
  onRemove,
}: {
  p: Provider;
  now: number;
  busy: string | null;
  editing: boolean;
  onEdit: () => void;
  onRemove: () => void;
}) {
  // 用它的角色（Agent → 分工）
  const roles = (useData(s => s.ws?.sched?.profiles) ?? []).filter(r => r.provider === p.name);
  return (
    <article className="pv-card">
      <header className="pv-head">
        <b className="pv-name" title={p.name}>{p.name}</b>
        <span className="sc-fixed">{p.tail ? maskKey(p) : '没填 key'}</span>
        <span className="pv-upd">{p.updatedAt ? `${since(p.updatedAt, now)}更新` : ''}</span>
        <Tip text="修改地址、key、模型">
          <button className="icon-btn sm" type="button" aria-label={`修改 ${p.name}`} disabled={!!busy || editing} onClick={onEdit}>
            <Icon name="edit" />
          </button>
        </Tip>
        <Tip text="删掉">
          <button className={cx('icon-btn sm', busy === `rm:${p.name}` && 'is-busy')} type="button" aria-label={`删掉 ${p.name}`} data-tip-align="end" disabled={!!busy} onClick={onRemove}>
            <Icon name="trash" />
          </button>
        </Tip>
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
            {p.models.map(m => {
              const marked = p.efforts?.[m] ?? [];
              return (
                <code key={m}>
                  {m}
                  {marked.length > 0 && <small>{effortRange(marked)}</small>}
                </code>
              );
            })}
          </span>
        </div>
      </div>
      <div className="pv-use">
        <span className="pv-k">在用</span>
        <div className="pv-use-body">
          {roles.length ? (
            <span className="pv-ags">
              {roles.map(r => (
                <Tip key={r.id} text={`角色「${r.name}」：${[r.model, r.effort].filter(Boolean).join(' · ')}`}>
                  <button type="button" className="pv-ag" onClick={() => (setTab('agent'), goSec('rules'))}>
                    <AgentIcon id={r.agent} />
                    <b>{r.name}</b>
                    <span className="pv-ag-n">{r.model}</span>
                  </button>
                </Tip>
              ))}
            </span>
          ) : (
            <span className="pv-none">没有角色在用：在 Agent → 分工里新建角色时选它</span>
          )}
        </div>
      </div>
    </article>
  );
}

