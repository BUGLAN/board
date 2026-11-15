/* ==========================================================================
   「共用配置」的模型供应商一节（PRD §10.12「共用配置」）：一个供应商一张卡
   - 卡片上面：名称、API key 末尾 4 位、更新时间、修改、删除；接口和地址（地址相同的接口并成一行）、模型（带标了的强度）
   - 卡片底下「在用」：只读，按 agent 汇总——一个 agent 一枚（图标、名称、几台、这几台里最该看的状态），
     点开列出哪几台：模型、用哪种接口、强度、写进去没有（失败写原因），点一行打开那台机器的 agent 页去改。
     每台机器上每个 agent 用哪个供应商在机器详情的 agent 页里选，这里不改（agent 会越来越多，不能一处一枚、一个 agent 一列）
   - 下面一句话写 board 版本太旧、不同步供应商的机器
   - 供应商的增、改、删要等服务端（按钮转圈、禁用直到返回），删掉时确认并列出用着它的机器和 agent
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import {
  agentGroups,
  blankProvider,
  endpointLines,
  interfaceName,
  maskKey,
  metasOf,
  PROVIDER_CELL_WORDS,
  providerError,
  toProviderSave,
  usesOf,
  worstCell,
  type ProviderUse,
} from '../../lib/providers';
import type { AgentMeta, HostInfo, Provider, ProviderView } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, openHost, toast, toastError } from '../../store/ui';
import { Tip, useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon } from './bits';
import { HowTip, OldHostsNote } from './SyncBits';
import { ProviderForm } from './ProviderForm';
import { useRemoteList } from './useRemoteList';

export function ProvidersSection({ hosts }: { hosts: HostInfo[] }) {
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
      toast({ type: 'success', title: `已保存 ${out.name}`, text: '用着它的机器马上改写 agent 的配置文件', duration: 2200 });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const remove = async (p: Provider) => {
    const users = view ? usesOf(hosts, view, p.name).map(u => `${u.h.name} 的 ${metasOf(view).find(m => m.id === u.a.id)?.name ?? u.a.name}`) : [];
    const ok = await confirm({
      title: `删掉模型供应商 ${p.name}？`,
      text: users.length
        ? `${users.join('、')} 指定了它：下次同步时报失败，要在机器详情里换一个。已经写进 agent 配置文件的不恢复。`
        : '已经写进各台机器 agent 配置文件的不恢复。',
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
  const old = hosts.filter(h => !h.sync?.able);

  return (
    <section className="sc sc-providers">
      <div className="sc-head">
        <div>
          <h2>模型供应商</h2>
          <p>
            供应商在这里加、改。每台机器上每个 agent 用哪个供应商、哪个模型，在机器详情的「agent」页里选；board host 写进 agent 自己的用户级配置文件，agent 直接连供应商，不经过
            Board。{' '}
            <HowTip tip="只改 Board 管的那几项；机器上改成「不改」后之前写进去的不恢复，Board 只是不再碰它。强度不在模型支持的档位里就不写。API key 只存在服务端，界面上只显示末尾 4 位，完整值只发给要用它的机器上的 board host。" />
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
              view={view}
              uses={usesOf(hosts, view, p.name)}
              now={now}
              busy={busy}
              editing={!!edit}
              onEdit={() => setEdit({ p: { ...p, apiKey: '', endpoints: p.endpoints.map(e => ({ ...e })), efforts: { ...(p.efforts ?? {}) } }, editing: true })}
              onRemove={() => void remove(p)}
            />
          ))}
        </div>
      )}
      {view && <OldHostsNote hosts={old} what="供应商" verb="写入" />}
    </section>
  );
}

/** 标了的强度：三档以上写成「low–max」 */
const effortRange = (es: string[]) => (es.length > 2 ? `${es[0]}–${es[es.length - 1]}` : es.join(' · '));

function ProviderCard({
  p,
  view,
  uses,
  now,
  busy,
  editing,
  onEdit,
  onRemove,
}: {
  p: Provider;
  view: ProviderView;
  uses: ProviderUse[];
  now: number;
  busy: string | null;
  editing: boolean;
  onEdit: () => void;
  onRemove: () => void;
}) {
  // 点开的 agent：列出哪几台
  const [open, setOpen] = useState<string | null>(null);
  const groups = agentGroups(uses, view);
  const cur = groups.find(g => g.meta.id === open);
  return (
    <article className="pv-card">
      <header className="pv-head">
        <b className="pv-name">{p.name}</b>
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
          {groups.length ? (
            <span className="pv-ags">
              {groups.map(g => (
                <AgentChip key={g.meta.id} meta={g.meta} uses={g.uses} on={open === g.meta.id} onClick={() => setOpen(open === g.meta.id ? null : g.meta.id)} />
              ))}
            </span>
          ) : (
            <span className="pv-none">没有机器在用：在机器详情的 agent 页里选它</span>
          )}
          {cur && <UseList uses={cur.uses} />}
        </div>
      </div>
    </article>
  );
}

/** 一个 agent 的汇总：图标、名称、几台、状态点（这几台里最该看的：失败 > 正在写入 > 离线 > 已写入）；悬停看每台的状态 */
function AgentChip({ meta, uses, on, onClick }: { meta: AgentMeta; uses: ProviderUse[]; on: boolean; onClick: () => void }) {
  return (
    <Tip text={uses.map(u => `${u.h.name} ${PROVIDER_CELL_WORDS[u.cell.kind]}`).join(' · ')}>
      <button type="button" className={cx('pv-ag', `k-${worstCell(uses)}`, on && 'on')} aria-expanded={on} onClick={onClick}>
        <AgentIcon id={meta.id} />
        <b>{meta.name}</b>
        <span className="pv-ag-n">{uses.length} 台</span>
        <i className="pv-cell-mark" aria-hidden="true" />
      </button>
    </Tip>
  );
}

/** 点开一个 agent：哪几台、模型、接口、强度、写进去没有；点一行打开那台机器的 agent 页 */
function UseList({ uses }: { uses: ProviderUse[] }) {
  return (
    <div className="pv-ml">
      {uses.map(u => {
        const bad = u.cell.kind === 'failed' || u.cell.kind === 'unsupported';
        return (
          <button key={u.h.id} type="button" className={cx('pv-mr', `k-${u.cell.kind}`)} onClick={() => openHost(u.h.name, 'agents')}>
            <i className="pv-cell-mark" aria-hidden="true" />
            <b>{u.h.name}</b>
            <code>{u.pick.model}</code>
            <span className="pv-if">{u.pick.ep ? interfaceName(u.pick.ep.interface) : '接口对不上'}</span>
            <span className="pv-mr-e">{u.pick.warn ? '不写强度' : u.pick.effort ? `强度 ${u.pick.effort}` : '强度默认'}</span>
            <span className="pv-mr-s">
              {PROVIDER_CELL_WORDS[u.cell.kind]}
              {bad && u.cell.why ? `：${u.cell.why}` : ''}
            </span>
            <span className="pv-mr-go">
              去改
              <Icon name="chevRight" />
            </span>
          </button>
        );
      })}
    </div>
  );
}
