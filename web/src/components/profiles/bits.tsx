/* ==========================================================================
   角色（PRD §9.1「分工」）的小零件：分工页、派发对话框、卡片「接下来」、机器详情共用
   - PMark：角色的标记，直接画 agent 的 logo（不加底框）
   - RuleView：一条规则 = 条件词 + 标签胶囊（和卡片上的 LabelChip 同一个）
   - Slots：机器的会话位，在跑的用角色的 agent 颜色填上
   - ProfileForm：新建 / 改一个角色（弹出框，高度固定：用不上的字段留着、变灰，没有随选项变的说明）
   - useProviderList：共用配置里的供应商（打码的列表），角色表单和派发说明用；顺带把 agent 的说明换成服务端的
   ========================================================================== */
import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { create } from 'zustand';
import { api } from '../../lib/api';
import { builtinName, metaColor, setAgentMeta, useAgentMeta } from '../../lib/agentMeta';
import { localChoices, OP_WORDS, SUBSCRIPTION, subscribable } from '../../lib/profiles';
import type { ClaimRule, HostInfo, Label, Profile, Provider, Workspace } from '../../lib/types';
import { cx } from '../../lib/util';
import { deleteProfilePatch, editProfilePatch, optimistic, useBusy, profileKey, useData } from '../../store/data';
import { toast, toastError } from '../../store/ui';
import { LabelChip, Select, Tip } from '../bits';
import { AgentIcon } from '../hosts/bits';
import { Icon } from '../Icon';

/** agent 的名称（服务端的说明，取到之前用兜底） */
export const agentName = (id: string) => builtinName(id);

/** 角色的颜色：它的 agent 的主色 */
export const colorOf = (p: Pick<Profile, 'agent'> | undefined) => (p ? (metaColor(p.agent) ?? 'var(--text-3)') : 'var(--text-3)');

/** 角色的标记：agent 的 logo */
export function PMark({ p }: { p: Pick<Profile, 'agent'> }) {
  return (
    <span className="pf-mark">
      <AgentIcon id={p.agent} />
    </span>
  );
}

/** 工作区里所有项目的标签（同名按第一个）：规则里的标签名换成真的标签，画成和卡片上一样的胶囊 */
export function useAllLabels(): Label[] {
  const projects = useData(s => s.ws?.projects);
  return useMemo(() => {
    const seen = new Set<string>();
    return (projects ?? []).flatMap(p => p.labels).filter(l => !seen.has(l.name.toLowerCase()) && !!seen.add(l.name.toLowerCase()));
  }, [projects]);
}

const chip = (all: Label[], n: string) => all.find(l => l.name.toLowerCase() === n.toLowerCase()) ?? { id: -1, name: n, color: 'var(--text-4)' };

/** 一条规则：条件词 + 标签胶囊 */
export function RuleView({ r }: { r: Pick<ClaimRule, 'op' | 'labels'> }) {
  const all = useAllLabels();
  return (
    <span className="pf-rv">
      <span className="pf-rv-op">{OP_WORDS[r.op]}</span>
      {r.op !== 'none' && r.labels.map(n => <LabelChip key={n} label={chip(all, n)} />)}
      {r.op === 'in' && r.labels.length > 1 && <span className="pf-rv-op">之一</span>}
    </span>
  );
}

/** 卡片的标签胶囊（按名字） */
export function NameChips({ names }: { names: string[] }) {
  const all = useAllLabels();
  return (
    <>
      {names.map(n => (
        <LabelChip key={n} label={chip(all, n)} />
      ))}
    </>
  );
}

/** 机器的会话位：在跑的用那次派发的 agent 颜色填上，空位空着；离线画斜纹 */
export function Slots({ h, ws }: { h: HostInfo; ws: Workspace | null }) {
  const running = (ws?.dispatches ?? []).filter(d => d.state === 'running' && d.host === h.name);
  return (
    <span className={cx('pf-slots', !h.online && 'off')}>
      {Array.from({ length: h.max }, (_, i) => {
        const d = running[i];
        return <i key={i} className={cx(d && 'on')} style={d ? { ['--c' as string]: metaColor(d.agent) ?? 'var(--text-3)' } : undefined} title={d ? `${d.profileName || d.agentLabel} · #${d.card} ${d.title}` : '空位'} />;
      })}
    </span>
  );
}

/* ------------------------------ 供应商 ------------------------------ */

/** 共用配置里的供应商（打码的列表）：打开分工页、派发对话框时取一次；顺带换上服务端给的 agent 说明 */
export const useProviderList = create<{ list: Provider[] | null }>(() => ({ list: null }));
export function useProviders(): Provider[] | null {
  const list = useProviderList(s => s.list);
  useEffect(() => {
    let alive = true;
    api
      .providers()
      .then(v => {
        if (!alive) return;
        setAgentMeta(v.meta);
        useProviderList.setState({ list: v.providers });
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  return list;
}

/** 这个 agent 能用的供应商：供应商有它认的接口 */
export function providersFor(list: Provider[], protocols: string[]): Provider[] {
  return list.filter(p => p.endpoints.some(e => protocols.includes(e.interface)));
}

/* ------------------------------ 角色表单 ------------------------------ */

/** 打开着的角色表单：`edit` 是在改的那一份（新建时没有 id） */
export const useProfileForm = create<{ edit: (Omit<Profile, 'id'> & { id?: string }) | null }>(() => ({ edit: null }));

/** 打开角色表单：改一个，或者新建（默认第一个 agent、本机配置） */
export function openProfile(p: Profile | null) {
  const first = useAgentMeta.getState().list[0]?.id ?? 'claude';
  useProfileForm.setState({ edit: p ? { ...p } : { name: '', agent: first, provider: '', model: '', effort: '', note: '', only: [] } });
}

/** 新建 / 改一个角色：名字、谁来做、模型（本机配置、官方订阅或供应商 · 模型）、强度、附加说明、在哪些机器上跑。
    新建等服务端返回才画得出来（按钮转圈并禁用）；改是乐观更新，失败时带着改过的内容重新打开 */
export function ProfileForm() {
  const edit = useProfileForm(s => s.edit);
  const metas = useAgentMeta(s => s.list);
  const ws = useData(s => s.ws);
  const providers = useProviders() ?? [];
  const [d, setD] = useState(edit);
  const [busy, setBusy] = useState(false);
  useEffect(() => setD(edit), [edit]);
  const close = () => useProfileForm.setState({ edit: null });
  useEffect(() => {
    if (!edit) return;
    const key = (e: KeyboardEvent) => e.key === 'Escape' && !e.defaultPrevented && close();
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [edit]);
  if (!edit || !d) return null;
  const isNew = !d.id;
  const meta = metas.find(m => m.id === d.agent);
  const usable = providersFor(providers, meta?.protocols ?? []);
  const prov = providers.find(p => p.name === d.provider);
  // 强度：这个 agent 能设的；用供应商时再和这个模型标了的取交集（没标的模型不能选）
  const marked = prov ? (prov.efforts?.[d.model] ?? []) : null;
  const efforts = (meta?.efforts ?? []).filter(e => !marked || marked.includes(e));
  const hosts = ws?.hosts ?? [];
  // 本机配置、官方订阅（#666）：选中的机器报过模型表时，模型是下拉、强度跟着模型变；报不过的机器照旧
  const local = !d.provider || d.provider === SUBSCRIPTION;
  const lc = local ? localChoices(hosts, d.agent, d.only, d.model, d.effort) : null;
  const table = lc?.known ? lc : null;
  const effortList = table ? table.efforts : efforts;
  // 「只在这些机器上跑」：空 = 所有机器（以后新接的也算）；勾了哪几台就存哪几台，勾满现在的全部也照存（#664）
  const allHosts = d.only.length === 0;
  const on = d.only.filter(id => hosts.some(h => h.id === id));
  const rules = (ws?.sched?.rules ?? []).filter(r => !isNew && r.profile === d.id).length;
  // 官方订阅（#667）：只有 Claude Code、Codex 能选
  const sub = subscribable(d.agent);
  const sourceOptions = [
    { value: '', label: '本机配置' },
    ...(sub ? [{ value: SUBSCRIPTION, label: '官方订阅' }] : []),
    ...usable.map(p => ({ value: p.name, label: p.name })),
  ];
  if (d.provider && !(d.provider === SUBSCRIPTION ? sub : usable.some(p => p.name === d.provider)))
    sourceOptions.push({ value: d.provider, label: `${d.provider === SUBSCRIPTION ? '官方订阅' : d.provider}（不能用）` });
  const save = async () => {
    const body = { ...d, name: d.name.trim(), model: d.model.trim() };
    if (isNew) {
      setBusy(true);
      try {
        await optimistic(null, () => api.putProfile(body));
        close();
      } catch (e) {
        toastError(e);
      } finally {
        setBusy(false);
      }
      return;
    }
    close();
    try {
      await optimistic(editProfilePatch(body as Profile), () => api.putProfile(body));
    } catch (e) {
      toastError(e);
      useProfileForm.setState({ edit: body });
    }
  };
  const remove = async () => {
    if (!d.id) return;
    close();
    try {
      const out = await optimistic(deleteProfilePatch(d.id), () => api.deleteProfile(d.id!));
      toast({ type: 'success', title: `已删除角色「${d.name}」`, text: out.rules ? `同时删除 ${out.rules} 条规则` : undefined });
    } catch (e) {
      toastError(e);
    }
  };
  return createPortal(
    <div className="dialog-wrap on" onClick={e => e.target === e.currentTarget && close()}>
      <div className="dialog pf-form" role="dialog" aria-modal="true" aria-label={isNew ? '新建角色' : `角色「${edit.name}」`}>
        <h3>{isNew ? '新建角色' : `角色「${edit.name}」`}</h3>
        <label className="pf-f">
          <span>名字</span>
          <input className="input" value={d.name} maxLength={20} placeholder="例如：修复、批量、界面" onChange={e => setD({ ...d, name: e.target.value })} />
        </label>
        <div className="pf-f">
          <span>谁来做</span>
          <div className="pf-agents" role="radiogroup" aria-label="谁来做">
            {metas.map(a => (
              <button
                key={a.id}
                type="button"
                role="radio"
                aria-checked={d.agent === a.id}
                className={cx('pf-agent', d.agent === a.id && 'on')}
                onClick={() => {
                  // 换 agent：供应商对不上（官方订阅：这个 agent 不能选）就回到本机配置，强度这个 agent 设不了就清掉
                  const ok = d.provider === SUBSCRIPTION ? subscribable(a.id) : providersFor(providers, a.protocols).some(p => p.name === d.provider);
                  setD({ ...d, agent: a.id, provider: ok ? d.provider : '', model: ok ? d.model : '', effort: a.efforts.includes(d.effort) ? d.effort : '' });
                }}
              >
                <AgentIcon id={a.id} />
                {a.name}
              </button>
            ))}
          </div>
        </div>
        <div className="pf-f pf-f3">
          <label>
            <span>模型来源</span>
            <Select
              value={d.provider}
              options={sourceOptions}
              aria-label="模型来源"
              onChange={v => {
                const p = providers.find(x => x.name === v);
                setD({ ...d, provider: v, model: p ? (p.models[0] ?? '') : '', effort: '' });
              }}
            />
          </label>
          <label>
            <span>模型</span>
            {prov ? (
              <Select value={d.model} options={prov.models.map(m => ({ value: m, label: m }))} aria-label="模型" onChange={v => setD({ ...d, model: v, effort: '' })} />
            ) : table ? (
              <Select
                value={d.model}
                options={[
                  { value: '', label: 'agent 默认' },
                  ...table.models.map(m => ({ value: m.id, label: m.name })),
                  ...(d.model && !table.models.some(m => m.id === d.model) ? [{ value: d.model, label: `${d.model}（选中的机器上没有）` }] : []),
                ]}
                aria-label="模型"
                onChange={v => {
                  // 换了模型，强度这个模型设不了就清掉
                  const next = localChoices(hosts, d.agent, d.only, v, '');
                  setD({ ...d, model: v, effort: next.efforts.includes(d.effort) ? d.effort : '' });
                }}
              />
            ) : (
              <input className="input" value={d.model} placeholder={d.provider && d.provider !== SUBSCRIPTION ? '' : '不填 = agent 自己的默认'} aria-label="模型" onChange={e => setD({ ...d, model: e.target.value })} />
            )}
          </label>
          <label>
            <span>强度</span>
            <Select
              value={d.effort}
              options={[{ value: '', label: '不设' }, ...effortList.map(e => ({ value: e, label: e }))]}
              disabled={!effortList.length}
              aria-label="强度"
              onChange={v => setD({ ...d, effort: v })}
            />
          </label>
        </div>
        <label className="pf-f">
          <span>附加说明</span>
          <textarea className="input" rows={2} value={d.note} placeholder="可以不写。写了接在提示词后面，例如「先写能复现的测试，再改」" onChange={e => setD({ ...d, note: e.target.value })} />
        </label>
        <div className="pf-f">
          <span>在哪些机器上跑</span>
          <div className="pf-only">
            <button type="button" className={cx('pf-only-h', allHosts && 'on')} aria-pressed={allHosts} onClick={() => setD({ ...d, only: [] })}>
              {allHosts && <Icon name="check" />}
              所有机器
            </button>
            {hosts.map(h => {
              const picked = !allHosts && on.includes(h.id);
              // 这台机器的账号里没有选的模型、这个模型不支持选的强度：变灰，原因放在悬停提示里（#666）
              const miss = lc?.why.get(h.id);
              const chip = (
                <button
                  key={h.id}
                  type="button"
                  className={cx('pf-only-h', picked && 'on', miss && 'miss')}
                  aria-pressed={picked}
                  onClick={() => {
                    const next = allHosts ? [h.id] : picked ? on.filter(x => x !== h.id) : [...on, h.id];
                    // 一台都不勾不行（角色就没地方跑了），要不限就点「所有机器」
                    if (!next.length) return;
                    setD({ ...d, only: next });
                  }}
                >
                  {picked && <Icon name="check" />}
                  {h.name}
                </button>
              );
              return miss ? (
                <Tip key={h.id} text={miss}>
                  {chip}
                </Tip>
              ) : (
                chip
              );
            })}
            <small>{hosts.length === 0 ? '还没有机器' : allHosts ? '以后新接的也算' : `只在这 ${on.length} 台`}</small>
          </div>
        </div>
        <div className="dialog-actions pf-form-acts">
          {!isNew && (
            <button type="button" className="btn sm danger" onClick={remove}>
              <Icon name="trash" />
              删除{rules ? `（${rules} 条规则跟着删）` : ''}
            </button>
          )}
          <span className="hp-sp" />
          <button className="btn" type="button" onClick={close}>
            取消
          </button>
          <button className={cx('btn brand', busy && 'is-busy')} type="button" disabled={busy || !d.name.trim() || (!!d.provider && d.provider !== SUBSCRIPTION && !d.model)} onClick={save}>
            {isNew ? '新建' : '保存'}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

/** 这个角色保存完之前不能再改 */
export const useProfileBusy = (id: string) => useBusy(profileKey(id));
