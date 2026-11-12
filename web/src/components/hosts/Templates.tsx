/* ==========================================================================
   「共用配置」的 agent 模板一节（PRD §10.12「共用配置」，卡片 #188）
   - 列表：标识、名称、命令模板、更新时间、各机器的汇总（几台找到了）；展开看每台机器：找到了（版本）/ 没找到命令 /
     本机配置覆盖 / 还没上报 / 离线
   - 添加 / 修改：标识、名称、命令模板（一行一个参数，要有 {prompt}）、默认模型、传模型的参数、传强度的参数、能选的强度、
     Windows 上支持的启动环境。同标识的覆盖内置模板的命令（没写的项用内置的）
   - 保存后服务端下发给所有 board host：它们重新检查本机 agent 并上报，不用重启。某台机器的配置文件里有同标识的
     [agents.*] 时以本机为准，标「本机配置覆盖」
   保存、删除要等服务端：按钮转圈、禁用直到返回（optimistic(null, …)）；失败时表单留着，人填的不丢
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import { blankForm, CELL_WORDS, cmdLine, fromForm, templateError, templateRows, toForm, WIN_ENVS, type TemplateForm } from '../../lib/templates';
import type { AgentTemplate, HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, openHost, toast, toastError } from '../../store/ui';
import { useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon, Dot } from './bits';
import { useRemoteList } from './useRemoteList';

/** 内置模板的标识：同标识的模板是覆盖它的命令 */
const BUILTIN: Record<string, string> = { dsh: 'DeepSeek Harness', claude: 'Claude Code', pi: 'pi', codex: 'Codex' };

export function TemplatesSection({ hosts }: { hosts: HostInfo[] }) {
  const ws = useData(s => s.ws);
  const now = useNow(10000);
  const { list, err, refresh: load } = useRemoteList(() => api.agentTemplates(), ws?.hosts);
  const [open, setOpen] = useState<string | null>(null);
  const [edit, setEdit] = useState<{ f: TemplateForm; editing: boolean } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const save = async () => {
    if (!edit || !list) return;
    const t = fromForm(edit.f);
    const why = templateError(t, list.map(x => x.id), edit.editing);
    if (why) {
      toast({ type: 'error', title: '没有保存', text: why });
      return;
    }
    setBusy('save');
    try {
      const out = await optimistic(null, () => api.putAgentTemplate(t));
      setEdit(null);
      setOpen(out.id);
      await load();
      toast({ type: 'success', title: `已保存 ${out.id}`, text: '在线的机器马上重新检查本机 agent 并上报', duration: 2200 });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const remove = async (t: AgentTemplate) => {
    const ok = await confirm({
      title: `删掉 agent 模板 ${t.id}？`,
      text: BUILTIN[t.id]
        ? `各台机器上的 ${BUILTIN[t.id]} 换回内置模板的命令；本机配置文件里写的不受影响。`
        : '各台机器重新检查后不再有这个 agent（本机配置文件里写了的除外）；用它的领取规则会标成不生效。',
      ok: '删掉',
      icon: 'trash',
      tone: 'warn',
    });
    if (!ok) return;
    setBusy(`rm:${t.id}`);
    try {
      await optimistic(null, () => api.removeAgentTemplate(t.id));
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="sc sc-tpl">
      <div className="sc-head">
        <div>
          <h2>agent 模板</h2>
          <p>
            在这里接入新的 agent，或者改内置 agent 的命令：保存后每台机器的 board host 重新检查本机 agent 并上报，不用重启。
            某台机器的 <code>~/.board/config.toml</code> 里有同标识的 <code>[agents.&lt;标识&gt;]</code> 时用那台机器自己的。
          </p>
        </div>
        <div className="sc-acts">
          <button className="btn sm" type="button" disabled={!!busy || !!edit} onClick={() => setEdit({ f: blankForm(), editing: false })}>
            <Icon name="plus" />
            添加
          </button>
        </div>
      </div>
      {err && <p className="hp-note lh-warn">取不到 agent 模板：{err}</p>}
      {edit && <TemplateFormView value={edit.f} editing={edit.editing} busy={busy === 'save'} onChange={f => setEdit({ ...edit, f })} onSave={() => void save()} onCancel={() => setEdit(null)} />}
      {!list && !err && <p className="hp-note">正在读取…</p>}
      {list && !list.length && !edit && <p className="hp-note">还没有：内置的 DeepSeek Harness、Claude Code、pi、Codex 不用加。点「添加」接入别的 agent</p>}
      {list && list.length > 0 && (
        <div className="sc-list">
          {list.map(t => {
            const rows = templateRows(hosts, t);
            const found = rows.filter(r => r.state === 'found').length;
            const local = rows.filter(r => r.state === 'local').length;
            const on = open === t.id;
            return (
              <div key={t.id} className={cx('sc-item', on && 'open')}>
                <div className="sc-row">
                  <button type="button" className="sc-toggle" aria-expanded={on} onClick={() => setOpen(on ? null : t.id)}>
                    <Icon name={on ? 'chevDown' : 'chevRight'} />
                    <AgentIcon id={t.id} />
                    <b>{t.name || BUILTIN[t.id] || t.id}</b>
                    <code>{t.id}</code>
                    {BUILTIN[t.id] && <span className="sc-fixed">覆盖内置</span>}
                  </button>
                  <span className="sc-meta" title={cmdLine(t.cmd)}>
                    <code>{cmdLine(t.cmd)}</code>
                    {t.updatedAt ? ` · ${since(t.updatedAt, now)}更新` : ''}
                  </span>
                  <span className={cx('sc-sum', rows.length && found === rows.length ? 'ok' : '')}>
                    {rows.length === 0 ? '还没有机器' : `${found}/${rows.length} 台找到了${local ? ` · ${local} 台本机配置覆盖` : ''}`}
                  </span>
                  <button className="icon-btn sm" type="button" aria-label={`修改 ${t.id}`} data-tip="修改" disabled={!!busy || !!edit} onClick={() => setEdit({ f: toForm(t), editing: true })}>
                    <Icon name="edit" />
                  </button>
                  <button className={cx('icon-btn sm', busy === `rm:${t.id}` && 'is-busy')} type="button" aria-label={`删掉 ${t.id}`} data-tip="删掉" data-tip-align="end" disabled={!!busy} onClick={() => void remove(t)}>
                    <Icon name="trash" />
                  </button>
                </div>
                {on && (
                  <div className="sc-hosts">
                    <div className="sc-vars">
                      {t.model && (
                        <span>
                          <b>默认模型</b> <code>{t.model}</code>
                        </span>
                      )}
                      {t.modelArgs.length > 0 && (
                        <span>
                          <b>选模型</b> <code>{cmdLine(t.modelArgs)}</code>
                        </span>
                      )}
                      {t.effortArgs.length > 0 && (
                        <span>
                          <b>选强度</b> <code>{cmdLine(t.effortArgs)}</code>（{(t.efforts.length ? t.efforts : ['low', 'medium', 'high']).join('、')}）
                        </span>
                      )}
                      {t.win.length > 0 && (
                        <span>
                          <b>Windows</b> {t.win.map(w => WIN_ENVS.find(([k]) => k === w)?.[1] ?? w).join(' → ')}
                        </span>
                      )}
                    </div>
                    {rows.map(r => (
                      <div key={r.host.id} className="sc-host">
                        <button type="button" className="sc-host-name" onClick={() => openHost(r.host.name)}>
                          <Dot h={r.host} running={0} />
                          {r.host.name}
                        </button>
                        <div className="sc-cells">
                          <span className={cx('sc-cell', r.state === 'found' ? 'synced' : r.state === 'missing' ? 'failed' : r.state === 'local' ? 'local' : 'pending')}>
                            <em>{CELL_WORDS[r.state]}</em>
                            {r.version && <small>{r.version}</small>}
                            {r.state === 'local' && <small>这台机器的配置文件里有 [agents.{t.id}]，用的是它</small>}
                          </span>
                        </div>
                      </div>
                    ))}
                    {!rows.length && <p className="hp-note">还没有机器：加了机器之后它会自动用上</p>}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}

function TemplateFormView({ value: f, editing, busy, onChange, onSave, onCancel }: { value: TemplateForm; editing: boolean; busy: boolean; onChange: (f: TemplateForm) => void; onSave: () => void; onCancel: () => void }) {
  const builtin = BUILTIN[f.id.trim()];
  return (
    <div className="sc-form">
      <div className="hp-add-row">
        <span className="hp-add-k">标识</span>
        <input className="input mono" value={f.id} disabled={editing} placeholder="kimi" maxLength={16} spellCheck={false} autoComplete="off" onChange={e => onChange({ ...f, id: e.target.value.trim().toLowerCase() })} />
      </div>
      {builtin && <p className="hp-note sc-form-note">和内置的 {builtin} 同标识：保存后各台机器用这里的命令；没写的项（名称、传参方式、启动环境）用内置的。</p>}
      <div className="hp-add-row">
        <span className="hp-add-k">名称</span>
        <input className="input" value={f.name} placeholder={builtin || 'Kimi CLI'} maxLength={32} onChange={e => onChange({ ...f, name: e.target.value })} />
      </div>
      <div className="hp-add-row top">
        <span className="hp-add-k">命令模板</span>
        <textarea
          className="input mono sc-args"
          value={f.cmd}
          rows={4}
          placeholder={'一行一个参数，第一行是命令，{prompt} 换成提示词，例如\nkimi\n--print\n--yolo\n{prompt}'}
          spellCheck={false}
          onChange={e => onChange({ ...f, cmd: e.target.value })}
        />
      </div>
      <div className="hp-add-row">
        <span className="hp-add-k">默认模型</span>
        <input className="input mono" value={f.model} placeholder="派发名字的第二段，可以不填" spellCheck={false} onChange={e => onChange({ ...f, model: e.target.value })} />
      </div>
      <div className="hp-add-row top">
        <span className="hp-add-k">传模型</span>
        <textarea className="input mono sc-args" value={f.modelArgs} rows={2} placeholder={'选了模型时插到 {prompt} 前面，例如\n--model\n{model}'} spellCheck={false} onChange={e => onChange({ ...f, modelArgs: e.target.value })} />
      </div>
      <div className="hp-add-row top">
        <span className="hp-add-k">传强度</span>
        <textarea className="input mono sc-args" value={f.effortArgs} rows={2} placeholder={'选了思考强度时插的参数，例如\n--effort\n{effort}'} spellCheck={false} onChange={e => onChange({ ...f, effortArgs: e.target.value })} />
      </div>
      <div className="hp-add-row">
        <span className="hp-add-k">能选的强度</span>
        <input className="input mono" value={f.efforts} placeholder="从低到高，空格隔开；不填是 low medium high" spellCheck={false} onChange={e => onChange({ ...f, efforts: e.target.value })} />
      </div>
      <div className="hp-add-row">
        <span className="hp-add-k">Windows</span>
        <div className="sc-agents">
          {WIN_ENVS.map(([k, label]) => (
            <label key={k}>
              <input type="checkbox" checked={f.win.includes(k)} onChange={e => onChange({ ...f, win: e.currentTarget.checked ? [...f.win, k] : f.win.filter(x => x !== k) })} />
              {label}
            </label>
          ))}
          <span className="hp-note">{f.win.length ? '按这个顺序选默认的' : builtin ? '都不勾 = 和内置的一样' : '都不勾 = 所有启动环境'}</span>
        </div>
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
