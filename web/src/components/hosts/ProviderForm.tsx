/* ==========================================================================
   添加 / 修改模型供应商（PRD §10.12「共用配置」）：居中弹窗，从上到下
   - 从常用的开始（只在添加时）：DeepSeek、Kimi、智谱 GLM、OpenAI、Anthropic 的磁贴，点了填好核实过的接口地址和常用模型；「自定义」清空
   - 连接：名称（修改时不能改）、API key（密码框，可以「显示」核对；修改时留空保留原值）
   - 接口：三种各一行，开关 + 地址；关着的写「不用」；写明哪个 agent 只认它
   - 模型：选好的是胶囊（× 去掉，也能手动输入、回车加）；「获取模型」让服务端用上面的地址和 key 问供应商，列出来勾选（筛选、全选、全不选）
   - 底部：哪些 agent 能用它（按打开的接口算），点保存先校验，原因写在底部
   ========================================================================== */
import { useMemo, useState } from 'react';
import { api } from '../../lib/api';
import { maskKey, PROVIDER_AGENTS, PROVIDER_INTERFACES, PROVIDER_PRESETS, providerError } from '../../lib/providers';
import type { Provider } from '../../lib/types';
import { cx } from '../../lib/util';
import { Icon } from '../Icon';
import { FormField, FormFoot, FormModal, FormSec, FormSwitch, FormTile } from './FormModal';

/** 常用供应商磁贴上的色块和一句说明（只是认得出来，不用对方的标志） */
const VENDOR: Record<string, { mark: string; color: string }> = {
  deepseek: { mark: 'DS', color: '#4D6BFE' },
  kimi: { mark: 'K', color: '#2B2B2E' },
  zhipu: { mark: 'Z', color: '#3859FF' },
  openai: { mark: 'AI', color: '#10A37F' },
  anthropic: { mark: 'A', color: '#D97757' },
};
const shortOf = (id: string) => PROVIDER_INTERFACES.find(i => i.id === id)?.short ?? id;

export function ProviderForm({ value: p, editing, busy, existing, onChange, onSave, onCancel }: { value: Provider; editing: boolean; busy: boolean; existing: string[]; onChange: (p: Provider) => void; onSave: () => void; onCancel: () => void }) {
  const [tried, setTried] = useState(false);
  // 打开着的接口：有地址的，或者人刚打开、还没填的
  const [on, setOn] = useState<string[]>(() => p.endpoints.map(e => e.interface));
  const [show, setShow] = useState(false);
  const urlOf = (id: string) => p.endpoints.find(e => e.interface === id)?.url ?? '';
  const setUrl = (id: string, url: string) => onChange({ ...p, endpoints: [...p.endpoints.filter(e => e.interface !== id), { interface: id as Provider['endpoints'][number]['interface'], url }] });
  const flip = (id: string) => {
    if (on.includes(id)) {
      setOn(on.filter(x => x !== id));
      onChange({ ...p, endpoints: p.endpoints.filter(e => e.interface !== id) });
    } else setOn([...on, id]);
  };
  const fill = (name: string) => {
    const preset = PROVIDER_PRESETS.find(x => x.name === name);
    if (!preset) {
      setOn([]);
      onChange({ ...p, name: '', endpoints: [], models: [] });
      return;
    }
    setOn(preset.endpoints.map(e => e.interface));
    onChange({ ...p, name: preset.name, endpoints: preset.endpoints.map(e => ({ ...e })), models: preset.models });
  };
  const live = p.endpoints.filter(e => e.url.trim()).map(e => e.interface as string);
  const why = providerError(p, existing, editing);
  const custom = !PROVIDER_PRESETS.some(x => x.name === p.name);
  return (
    <FormModal
      title={editing ? `修改 ${p.name}` : '添加模型供应商'}
      onCancel={onCancel}
      foot={
        <FormFoot
          why={why}
          tried={tried}
          busy={busy}
          onCancel={onCancel}
          onSave={() => {
            setTried(true);
            if (!why) onSave();
          }}
          agentsLabel="能用它的 agent"
          agents={PROVIDER_AGENTS.map(a => {
            const ok = a.protocols.some(x => live.includes(x));
            return { id: a.id, on: ok, tip: ok ? `${a.name} 能用` : `${a.name} 要 ${a.protocols.map(shortOf).join(' / ')} 接口` };
          })}
        />
      }
    >
      {!editing && (
        <FormSec title="从常用的开始">
          <div className="fm-tiles">
            {PROVIDER_PRESETS.map(x => (
              <FormTile key={x.name} on={p.name === x.name} mark={VENDOR[x.name]?.mark ?? x.label[0]} color={VENDOR[x.name]?.color} title={x.label} desc={x.endpoints.map(e => shortOf(e.interface)).join(' · ')} onClick={() => fill(x.name)} />
            ))}
            <FormTile on={custom} mark={<Icon name="plus" />} title="自定义" desc="中转站、自己部署的" onClick={() => fill('')} />
          </div>
        </FormSec>
      )}
      <FormSec title="连接">
        <div className="fm-fields">
          <FormField label="名称" hint={editing ? '名称不能改' : '字母、数字、_、-；agent 配置文件里用它'}>
            <input className="input mono" value={p.name} disabled={editing} placeholder="deepseek" spellCheck={false} autoComplete="off" onChange={e => onChange({ ...p, name: e.target.value.trim() })} />
          </FormField>
          <FormField label="API key" hint="只存在服务端，界面上只显示末尾 4 位">
            <span className="fm-key">
              <input
                className="input mono"
                type={show ? 'text' : 'password'}
                value={p.apiKey ?? ''}
                placeholder={editing && p.tail != null ? `${maskKey(p)}（不改就留空）` : 'sk-…'}
                spellCheck={false}
                autoComplete="off"
                onChange={e => onChange({ ...p, apiKey: e.target.value })}
              />
              <button type="button" className="fm-show" onClick={e => (e.preventDefault(), setShow(!show))}>
                {show ? '隐藏' : '显示'}
              </button>
            </span>
          </FormField>
        </div>
      </FormSec>
      <FormSec title="接口" hint="打开它提供的，填地址">
        <div className="fm-rows">
          {PROVIDER_INTERFACES.map(i => {
            const yes = on.includes(i.id);
            const only = PROVIDER_AGENTS.filter(a => a.protocols.length === 1 && a.protocols[0] === i.id);
            return (
              <div key={i.id} className={cx('fm-if', yes && 'on')}>
                <FormSwitch on={yes} onClick={() => flip(i.id)} label={i.name} />
                <span className="fm-if-name">
                  {i.name}
                  {only.length > 0 && <small>{only.map(a => a.name).join('、')} 只认它</small>}
                </span>
                {yes ? (
                  <input className="input mono" value={urlOf(i.id)} placeholder={i.placeholder} spellCheck={false} autoComplete="off" aria-label={`${i.name} 的地址`} onChange={e => setUrl(i.id, e.target.value)} />
                ) : (
                  <span className="fm-off">不用</span>
                )}
              </div>
            );
          })}
        </div>
      </FormSec>
      <Models p={p} editing={editing} onChange={onChange} />
    </FormModal>
  );
}

/** 模型：选好的胶囊 + 手动输入；「获取模型」问供应商，列出来勾选 */
function Models({ p, editing, onChange }: { p: Provider; editing: boolean; onChange: (p: Provider) => void }) {
  const [state, setState] = useState<{ busy: boolean; err: string | null; list: string[] | null; from: string }>({ busy: false, err: null, list: null, from: '' });
  const [q, setQ] = useState('');
  const [manual, setManual] = useState('');
  const pick = (m: string) => onChange({ ...p, models: p.models.includes(m) ? p.models.filter(x => x !== m) : [...p.models, m] });
  const add = () => {
    const v = manual.trim();
    if (v && !p.models.includes(v)) onChange({ ...p, models: [...p.models, v] });
    setManual('');
  };
  const run = async () => {
    setState(s => ({ ...s, busy: true, err: null }));
    try {
      const out = await api.providerModels({ name: editing ? p.name : undefined, endpoints: p.endpoints.filter(e => e.url.trim()), apiKey: p.apiKey ?? '' });
      setState({ busy: false, err: null, list: out.models, from: out.from });
      setQ('');
    } catch (e) {
      setState(s => ({ ...s, busy: false, err: String((e as Error)?.message ?? e) }));
    }
  };
  const shown = useMemo(() => (state.list ?? []).filter(m => m.toLowerCase().includes(q.trim().toLowerCase())), [state.list, q]);
  const picked = state.list ? p.models.filter(m => state.list!.includes(m)).length : 0;
  return (
    <FormSec
      title="模型"
      hint="写进 agent 配置文件时从这里选"
      act={
        <button
          type="button"
          className={cx('btn sm fm-fetch', state.busy && 'is-busy')}
          disabled={state.busy}
          onClick={() => void run()}
          data-tip="服务端用上面的地址和 API key 问供应商有哪些模型（修改时 key 留空就用存着的）"
          data-tip-align="end"
        >
          <Icon name="renew" />
          {state.busy ? '正在获取…' : state.list ? '重新获取' : '获取模型'}
        </button>
      }
    >
      <div className="fm-chosen">
        {p.models.map(m => (
          <span key={m} className="fm-chip">
            {m}
            <button type="button" aria-label={`去掉 ${m}`} onClick={() => pick(m)}>
              <Icon name="close" />
            </button>
          </span>
        ))}
        <input
          className="fm-manual"
          value={manual}
          placeholder={p.models.length ? '手动加一个…' : '还没选模型：点「获取模型」，或在这里输入、回车'}
          spellCheck={false}
          autoComplete="off"
          aria-label="手动加模型"
          onChange={e => setManual(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            } else if (e.key === 'Backspace' && !manual && p.models.length) onChange({ ...p, models: p.models.slice(0, -1) });
          }}
          onBlur={add}
        />
      </div>
      {state.err && (
        <p className="fm-err">
          <Icon name="alert" />
          {state.err}
        </p>
      )}
      {state.list && (
        <div className="fm-picker">
          <div className="fm-picker-head">
            <span className="fm-search">
              <Icon name="search" />
              <input value={q} placeholder="筛选" spellCheck={false} aria-label="筛选模型" onChange={e => setQ(e.target.value)} />
            </span>
            <span className="fm-count">
              共 {state.list.length} 个 · 选了 {picked} 个
            </span>
            <button type="button" className="act hp-link" onClick={() => onChange({ ...p, models: [...new Set([...p.models, ...shown])] })}>
              全选
            </button>
            <button type="button" className="act hp-link" onClick={() => onChange({ ...p, models: p.models.filter(m => !shown.includes(m)) })}>
              全不选
            </button>
          </div>
          <div className="fm-picker-list">
            {shown.map(m => {
              const yes = p.models.includes(m);
              return (
                <button key={m} type="button" role="checkbox" aria-checked={yes} className={cx('fm-pick', yes && 'on')} onClick={() => pick(m)}>
                  <span className={cx('fm-check', yes && 'on')}>{yes && <Icon name="check" />}</span>
                  <code>{m}</code>
                </button>
              );
            })}
            {!shown.length && <p className="fm-none">没有「{q}」</p>}
          </div>
          <p className="fm-from">从 {state.from} 取到的</p>
        </div>
      )}
    </FormSec>
  );
}
