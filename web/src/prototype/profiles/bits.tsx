/* 原型（用完即弃）：三个分工页变体、派发对话框、卡片「接下来」共用的小零件——工种标记、工种表单、推算的 hook、成绩一行 */
import { useEffect, useMemo, useState } from 'react';
import { create } from 'zustand';
import { api } from '../../lib/api';
import type { BoardSnapshot } from '../../lib/types';
import { cx } from '../../lib/util';
import { useData } from '../../store/data';
import { Select } from '../../components/bits';
import { AgentIcon } from '../../components/hosts/bits';
import { Icon } from '../../components/Icon';
import {
  AGENTS,
  LOCAL_MODELS,
  PROVIDERS,
  claimableCards,
  dropProfile,
  forecast,
  putProfile,
  useModel,
  type AgentId,
  type Fc,
  type HostWhy,
  type Profile,
  type Stats,
} from './model';

/** 工种的标记：agent 的图标；「我」是一个圆点头像 */
export function PMark({ p, size }: { p: Profile; size?: 'sm' | 'lg' }) {
  return <span className={cx('pf-mark', size, !p.agent && 'me')}>{p.agent ? <AgentIcon id={p.agent} /> : '我'}</span>;
}

/** 工种名胶囊：图标 + 名字 */
export function PChip({ p, sub, on, onClick }: { p: Profile; sub?: string; on?: boolean; onClick?: () => void }) {
  const Tag = onClick ? 'button' : 'span';
  return (
    <Tag type={onClick ? 'button' : undefined} className={cx('pf-chip', on && 'on', !p.agent && 'me')} onClick={onClick}>
      <PMark p={p} size="sm" />
      <b>{p.name}</b>
      {sub && <small>{sub}</small>}
    </Tag>
  );
}

/** 一台机器对一个工种：✓ 空 2 位 / 满 4/4 / 时段外 / ✗ 原因 */
export function HostWhyChip({ x, compact }: { x: HostWhy; compact?: boolean }) {
  return (
    <span className={cx('pf-hw', `t-${x.tone}`, x.picked && 'picked')} title={`${x.name}：${x.why}`}>
      <i />
      <b>{x.name}</b>
      {!compact && <small>{x.why}</small>}
    </span>
  );
}

/** 近 30 天的成绩（假数据） */
export function StatsLine({ s, me }: { s: Stats; me?: boolean }) {
  return (
    <span className="pf-stats">
      <span>
        {me ? '你开的会话' : '交审'} <b>{s.done}</b> 张
      </span>
      <span className={cx(s.pass < 60 && 'bad')}>
        一次通过 <b>{s.pass}%</b>
      </span>
      <span>
        返工 <b>{s.rework}</b>
      </span>
      {!me && (
        <span className={cx(s.crash >= 5 && 'bad')}>
          异常 <b>{s.crash}</b>
          {s.lost ? ` · 掉线 ${s.lost}` : ''}
        </span>
      )}
      <span>
        平均 <b>{s.avgMin}</b> 分钟
      </span>
      {s.cost && (
        <span>
          费用 <b>{s.cost}</b>
        </span>
      )}
    </span>
  );
}

/* ------------------------------ 推算 ------------------------------ */

/** oss/main 的快照：当前白板就是它时用 store 里的，否则取一次（经过 mock，带多塞的卡） */
const useOss = create<{ b: BoardSnapshot | null }>(() => ({ b: null }));
export function useOssBoard() {
  const path = useData(s => s.path);
  const cur = useData(s => s.board);
  const cached = useOss(s => s.b);
  useEffect(() => {
    if (path === 'oss/main' || cached) return;
    api.board('oss/main').then(b => useOss.setState({ b }));
  }, [path, cached]);
  return path === 'oss/main' && cur?.path === 'oss/main' ? cur : cached;
}

/** 一块白板（默认 oss/main）上可领取的卡片各自会怎么走 */
export function useFc(board?: BoardSnapshot | null): Map<number, Fc> {
  const oss = useOssBoard();
  const b = board === undefined ? oss : board;
  const ws = useData(s => s.ws);
  const m = useModel();
  return useMemo(() => forecast(claimableCards(ws, b), m), [ws, b, m]);
}

/* ------------------------------ 工种表单 ------------------------------ */

const blank = (): Profile => ({
  id: `p${Date.now()}`,
  name: '',
  agent: 'codex',
  provider: 'relay-a',
  model: 'model-b',
  effort: 'high',
  note: '',
  only: null,
  stats: { done: 0, pass: 0, rework: 0, crash: 0, avgMin: 0, cost: null },
});

/** 模型下拉的值：「provider|model」，本机登录是「|model」 */
const modelKey = (p: Pick<Profile, 'provider' | 'model'>) => `${p.provider ?? ''}|${p.model}`;

const useForm = create<{ edit: Profile | null; isNew: boolean }>(() => ({ edit: null, isNew: false }));
export const openProfile = (p: Profile | null) => useForm.setState(p ? { edit: { ...p }, isNew: false } : { edit: blank(), isNew: true });

/** 新建 / 改一个工种：名字、agent、模型（本机登录或供应商 · 模型）、强度、附加说明、只在哪些机器上跑 */
export function ProfileForm() {
  const { edit, isNew } = useForm();
  const hosts = useModel(s => s.hosts);
  const rules = useModel(s => s.rules);
  const [d, setD] = useState<Profile | null>(null);
  useEffect(() => setD(edit), [edit]);
  // Esc 关掉（下拉开着时先关下拉：它自己拦掉了 Esc）
  useEffect(() => {
    if (!edit) return;
    const key = (e: KeyboardEvent) => e.key === 'Escape' && !e.defaultPrevented && useForm.setState({ edit: null });
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [edit]);
  if (!edit || !d) return null;
  const close = () => useForm.setState({ edit: null });
  const me = !d.agent;
  const agent = d.agent ?? 'codex';
  const options = [
    ...LOCAL_MODELS[agent].models.map(m => ({ value: `|${m}`, label: `本机登录 · ${m}` })),
    ...PROVIDERS.filter(p => agent !== 'claude' || p.name !== 'relay-a').flatMap(p => p.models.map(m => ({ value: `${p.name}|${m}`, label: `${p.name} · ${m}` }))),
  ];
  const efforts = d.provider ? (PROVIDERS.find(p => p.name === d.provider)?.efforts ?? []) : LOCAL_MODELS[agent].efforts;
  const used = rules.filter(r => r.to === d.id).length;
  const all = hosts.map(h => h.name);
  const on = d.only ?? all;
  return (
    <div className="dialog-wrap on" onClick={e => e.target === e.currentTarget && close()}>
      <div className="dialog pf-form" role="dialog" aria-modal="true" aria-label={isNew ? '新建工种' : `改工种 ${d.name}`}>
        <h3>{isNew ? '新建工种' : `工种「${edit.name}」`}</h3>
        <p className="pf-form-sub">工种定下用哪个 agent、哪个模型、多大强度。派发时按次带上，不改机器上 agent 自己的配置。</p>
        <label className="pf-f">
          <span>名字</span>
          <input className="input" value={d.name} placeholder="例如：修复、批量、界面" onChange={e => setD({ ...d, name: e.target.value })} />
        </label>
        <div className="pf-f">
          <span>谁来做</span>
          <div className="pf-agents">
            {AGENTS.map(a => (
              <button
                key={a.id}
                type="button"
                className={cx('pf-agent', d.agent === a.id && 'on')}
                onClick={() => {
                  const lm = LOCAL_MODELS[a.id];
                  const keep = d.provider && (a.id !== 'claude' || d.provider !== 'relay-a');
                  setD({ ...d, agent: a.id as AgentId, provider: keep ? d.provider : null, model: keep ? d.model : lm.models[0], effort: keep ? d.effort : (lm.efforts[2] ?? null) });
                }}
              >
                <AgentIcon id={a.id} />
                {a.name}
              </button>
            ))}
            <button type="button" className={cx('pf-agent', me && 'on')} onClick={() => setD({ ...d, agent: null, provider: null, model: '', effort: null })}>
              <span className="pf-mark sm me">我</span>
              我自己开会话
            </button>
          </div>
        </div>
        {!me && (
          <>
            <div className="pf-f pf-f2">
              <label>
                <span>模型</span>
                <Select
                  value={modelKey(d)}
                  options={options}
                  aria-label="模型"
                  onChange={v => {
                    const [prov, model] = v.split('|');
                    const eff = prov ? (PROVIDERS.find(p => p.name === prov)?.efforts ?? []) : LOCAL_MODELS[agent].efforts;
                    setD({ ...d, provider: prov || null, model, effort: d.effort && eff.includes(d.effort) ? d.effort : (eff[eff.length - 1] ?? null) });
                  }}
                />
              </label>
              <label>
                <span>强度</span>
                <Select
                  value={d.effort ?? ''}
                  options={[{ value: '', label: '不设' }, ...efforts.map(e => ({ value: e, label: e }))]}
                  disabled={!efforts.length}
                  aria-label="强度"
                  onChange={v => setD({ ...d, effort: v || null })}
                />
              </label>
            </div>
            <p className="pf-hint">
              {d.provider
                ? `派发时 board host 用 ${d.provider} 的地址和 key 启动这一个 ${AGENTS.find(a => a.id === agent)?.name} 进程，只对这次派发生效。`
                : `用机器上 ${AGENTS.find(a => a.id === agent)?.name} 自己登录的账号，只在会话里切到 ${d.model}。没登录的机器跑不了。`}
            </p>
            <label className="pf-f">
              <span>附加说明</span>
              <textarea className="input" rows={2} value={d.note} placeholder="可以不写。写了接在提示词后面，例如「先写能复现的测试，再改」" onChange={e => setD({ ...d, note: e.target.value })} />
            </label>
            <div className="pf-f">
              <span>在哪些机器上跑</span>
              <div className="pf-only">
                {hosts.map(h => (
                  <button
                    key={h.name}
                    type="button"
                    className={cx('pf-only-h', on.includes(h.name) && 'on')}
                    onClick={() => {
                      const next = on.includes(h.name) ? on.filter(x => x !== h.name) : [...on, h.name];
                      setD({ ...d, only: next.length === all.length ? null : next });
                    }}
                  >
                    {on.includes(h.name) && <Icon name="check" />}
                    {h.name}
                  </button>
                ))}
                <small>{d.only ? `只在这 ${d.only.length} 台` : '默认：所有能跑的机器'}</small>
              </div>
            </div>
          </>
        )}
        {me && <p className="pf-hint">命中这个工种的卡不派给机器，在卡片上写「等你开会话」，带「复制提示词」。就是现在 claude 标签的用法。</p>}
        <div className="dialog-actions pf-form-acts">
          {!isNew && (
            <button
              type="button"
              className="btn sm danger"
              onClick={() => {
                dropProfile(d.id);
                close();
              }}
            >
              <Icon name="trash" />
              删除{used ? `（${used} 条规则跟着删）` : ''}
            </button>
          )}
          <span className="hp-sp" />
          <button className="btn" type="button" onClick={close}>
            取消
          </button>
          <button
            className="btn brand"
            type="button"
            disabled={!d.name.trim()}
            onClick={() => {
              putProfile({ ...d, name: d.name.trim() });
              close();
            }}
          >
            {isNew ? '新建' : '保存'}
          </button>
        </div>
      </div>
    </div>
  );
}
