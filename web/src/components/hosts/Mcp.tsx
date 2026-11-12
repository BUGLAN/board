/* ==========================================================================
   「共用配置」的 MCP 服务器一节（PRD §10.12「共用配置」）
   - 列表：名称、类型、命令或地址、同步给哪些 agent、更新时间、各机器的汇总；展开看每台机器每个 agent 的状态
   - 添加 / 修改：名称、类型（本地命令 stdio：命令、参数、环境变量；远程 HTTP：地址、请求头）、同步给哪些 agent（都不勾 = 所有支持的）。
     值可以标成「密钥」：保存后只显示末尾 4 位，修改时留空保留原值
   - 桌面端「从本机导入」：读出这台电脑上各 agent 已经配好的，勾选后加进来（名字像密钥的值标成密钥）
   保存、删除、导入要等服务端：按钮转圈、禁用直到返回（optimistic(null, …)）；失败时表单留着，人填的不丢
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import { argsOf, blankMcp, maskOf, MCP_AGENT_NAMES, MCP_AGENTS, MCP_WORDS, mcpError, mcpRows, mcpSummary, toSave } from '../../lib/mcp';
import { localMcp, shell } from '../../lib/platform';
import type { HostInfo, McpServer, McpVar } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, openHost, toast, toastError } from '../../store/ui';
import { Segmented, useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon, Dot } from './bits';
import { useRemoteList } from './useRemoteList';

export function McpSection({ hosts }: { hosts: HostInfo[] }) {
  const ws = useData(s => s.ws);
  const now = useNow(10000);
  const { list, err, refresh: load } = useRemoteList(() => api.mcp(), ws?.hosts);
  const [open, setOpen] = useState<string | null>(null);
  // 正在编辑的：null 没有；name 为空串是新加
  const [edit, setEdit] = useState<{ s: McpServer; editing: boolean } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [importing, setImporting] = useState<{ agent: string; server: McpServer; on: boolean }[] | null>(null);
  const save = async () => {
    if (!edit || !list) return;
    const why = mcpError(edit.s, list.map(x => x.name), edit.editing);
    if (why) {
      toast({ type: 'error', title: '没有保存', text: why });
      return;
    }
    setBusy('save');
    try {
      const out = await optimistic(null, () => api.putMcp(toSave(edit.s)));
      setEdit(null);
      setOpen(out.name);
      await load();
      toast({ type: 'success', title: `已保存 ${out.name}`, text: '在线的机器马上同步', duration: 2000 });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const remove = async (s: McpServer) => {
    const ok = await confirm({
      title: `删掉 MCP 服务器 ${s.name}？`,
      text: '各台机器上由同步加进 agent 配置的这一项会被删掉；agent 里用户自己加的不受影响。',
      ok: '删掉',
      icon: 'trash',
      tone: 'warn',
    });
    if (!ok) return;
    setBusy(`rm:${s.name}`);
    try {
      await optimistic(null, () => api.removeMcp(s.name));
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const startImport = async () => {
    setBusy('scan');
    try {
      const found = await localMcp();
      if (!found) {
        toast({ type: 'info', title: '这个版本的桌面端不能导入', text: '更新桌面端后再试' });
        return;
      }
      if (!found.length) {
        toast({ type: 'info', title: '这台电脑上没找到 MCP 服务器', text: '看的是 Claude Code、Codex、pi 的用户级配置' });
        return;
      }
      const have = new Set(list?.map(x => x.name));
      setImporting(found.map(f => ({ ...f, on: !have.has(f.server.name) })));
    } finally {
      setBusy(null);
    }
  };
  const doImport = async () => {
    if (!importing) return;
    const pick = importing.filter(x => x.on);
    // 同名的只导第一个
    const seen = new Set<string>();
    setBusy('import');
    let n = 0;
    try {
      for (const p of pick) {
        if (seen.has(p.server.name)) continue;
        seen.add(p.server.name);
        await optimistic(null, () => api.putMcp(toSave({ ...p.server, agents: [] })));
        n++;
      }
      setImporting(null);
      await load();
      toast({ type: 'success', title: `导入了 ${n} 个 MCP 服务器`, text: '密钥只显示末尾 4 位；在线的机器马上同步', duration: 2400 });
    } catch (e) {
      toastError(e);
      await load();
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="sc sc-mcp">
      <div className="sc-head">
        <div>
          <h2>MCP 服务器</h2>
          <p>
            每台机器的 board host 把这里的 MCP 服务器写进 Claude Code、Codex、pi 的用户级配置（dsh 不支持）。只动同步加进去的，agent 里已有同名的不覆盖。
            值可以标成密钥：只存在服务端，界面上只显示末尾 4 位。
          </p>
        </div>
        <div className="sc-acts">
          {shell === 'desktop' && (
            <button className={cx('btn sm', busy === 'scan' && 'is-busy')} type="button" disabled={!!busy} onClick={() => void startImport()} data-tip="读出这台电脑上各 agent 已经配好的 MCP 服务器，勾选后加进来" data-tip-align="end">
              <Icon name="download" />
              从本机导入
            </button>
          )}
          <button className="btn sm" type="button" disabled={!!busy || !!edit} onClick={() => setEdit({ s: blankMcp(), editing: false })}>
            <Icon name="plus" />
            添加
          </button>
        </div>
      </div>
      {err && <p className="hp-note lh-warn">取不到 MCP 配置：{err}</p>}
      {edit && <McpForm value={edit.s} editing={edit.editing} busy={busy === 'save'} onChange={s => setEdit({ ...edit, s })} onSave={() => void save()} onCancel={() => setEdit(null)} />}
      {importing && (
        <div className="sc-import">
          <b>这台电脑上找到 {importing.length} 个</b>
          {importing.map((f, i) => (
            <label key={`${f.agent}:${f.server.name}`} className="sc-import-row">
              <input type="checkbox" checked={f.on} onChange={e => setImporting(importing.map((x, j) => (j === i ? { ...x, on: e.currentTarget.checked } : x)))} />
              <AgentIcon id={f.agent} />
              <b>{f.server.name}</b>
              <code>{f.server.kind === 'http' ? f.server.url : [f.server.command, ...f.server.args].join(' ')}</code>
              {list?.some(x => x.name === f.server.name) && <em>共用配置里已有同名：会换成这个</em>}
              {[...f.server.env, ...f.server.headers].some(v => v.secret) && <em>密钥：{[...f.server.env, ...f.server.headers].filter(v => v.secret).map(v => v.key).join('、')}</em>}
            </label>
          ))}
          <div className="lh-actions">
            <button className="btn sm ghost" type="button" disabled={busy === 'import'} onClick={() => setImporting(null)}>
              取消
            </button>
            <button className={cx('btn sm brand', busy === 'import' && 'is-busy')} type="button" disabled={!!busy || !importing.some(x => x.on)} onClick={() => void doImport()}>
              导入所选
            </button>
          </div>
        </div>
      )}
      {!list && !err && <p className="hp-note">正在读取…</p>}
      {list && !list.length && !edit && <p className="hp-note">还没有：点「添加」，或在桌面端「从本机导入」</p>}
      {list && list.length > 0 && (
        <div className="sc-list">
          {list.map(s => {
            const rows = mcpRows(hosts, s);
            const sum = mcpSummary(rows);
            const on = open === s.name;
            return (
              <div key={s.name} className={cx('sc-item', on && 'open')}>
                <div className="sc-row">
                  <button type="button" className="sc-toggle" aria-expanded={on} onClick={() => setOpen(on ? null : s.name)}>
                    <Icon name={on ? 'chevDown' : 'chevRight'} />
                    <b>{s.name}</b>
                    <span className="sc-fixed">{s.kind === 'http' ? 'HTTP' : 'stdio'}</span>
                  </button>
                  <span className="sc-meta" title={s.kind === 'http' ? s.url : [s.command, ...s.args].join(' ')}>
                    <code>{s.kind === 'http' ? s.url : [s.command, ...s.args].join(' ')}</code>
                    {' · '}
                    {s.agents.length ? s.agents.map(a => MCP_AGENT_NAMES[a] ?? a).join('、') : '所有支持的 agent'}
                    {s.updatedAt ? ` · ${since(s.updatedAt, now)}更新` : ''}
                  </span>
                  <span className={cx('sc-sum', sum.bad ? 'bad' : rows.length && sum.synced === rows.length - sum.old ? 'ok' : '')}>
                    {rows.length === 0 ? '还没有机器' : `${sum.synced}/${rows.length} 台已同步${sum.bad ? ` · ${sum.bad} 台有问题` : ''}${sum.old ? ` · ${sum.old} 台不同步` : ''}`}
                  </span>
                  <button className="icon-btn sm" type="button" aria-label={`修改 ${s.name}`} data-tip="修改" disabled={!!busy || !!edit} onClick={() => setEdit({ s: { ...s, env: s.env.map(v => ({ ...v, value: v.secret ? '' : v.value })), headers: s.headers.map(v => ({ ...v, value: v.secret ? '' : v.value })) }, editing: true })}>
                    <Icon name="edit" />
                  </button>
                  <button className={cx('icon-btn sm', busy === `rm:${s.name}` && 'is-busy')} type="button" aria-label={`删掉 ${s.name}`} data-tip="删掉" data-tip-align="end" disabled={!!busy} onClick={() => void remove(s)}>
                    <Icon name="trash" />
                  </button>
                </div>
                {on && (
                  <div className="sc-hosts">
                    {[...s.env, ...s.headers].length > 0 && (
                      <div className="sc-vars">
                        {[...s.env, ...s.headers].map(v => (
                          <span key={v.key}>
                            <b>{v.key}</b> = {v.secret ? <em>{maskOf(v)}（密钥）</em> : <code>{v.value}</code>}
                          </span>
                        ))}
                      </div>
                    )}
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
                            <span className="sc-cell old">{r.host.online ? '这台机器上没有要同步的 agent' : '离线'}</span>
                          ) : (
                            r.cells.map(c => (
                              <span key={c.agent} className={cx('sc-cell', c.state === 'unsupported' ? 'pending' : c.state)}>
                                <AgentIcon id={c.agent} />
                                <b>{r.host.agents.find(a => a.id === c.agent)?.name ?? c.agent}</b>
                                <em>{MCP_WORDS[c.state]}</em>
                                {c.why && c.state !== 'unsupported' && <small>{c.why}</small>}
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
    </section>
  );
}

const KINDS: ['stdio' | 'http', string][] = [
  ['stdio', '本地命令（stdio）'],
  ['http', '远程 HTTP'],
];

function McpForm({ value: s, editing, busy, onChange, onSave, onCancel }: { value: McpServer; editing: boolean; busy: boolean; onChange: (s: McpServer) => void; onSave: () => void; onCancel: () => void }) {
  const [args, setArgs] = useState(s.args.join('\n'));
  const field = s.kind === 'stdio' ? 'env' : 'headers';
  const vars = s[field];
  const setVars = (vs: McpVar[]) => onChange({ ...s, [field]: vs });
  return (
    <div className="sc-form">
      <div className="hp-add-row">
        <span className="hp-add-k">名称</span>
        <input className="input mono" value={s.name} disabled={editing} placeholder="github" spellCheck={false} autoComplete="off" onChange={e => onChange({ ...s, name: e.target.value.trim() })} />
      </div>
      <div className="hp-add-row">
        <span className="hp-add-k">类型</span>
        <Segmented value={s.kind} options={KINDS} onChange={k => onChange({ ...s, kind: k })} />
      </div>
      {s.kind === 'stdio' ? (
        <>
          <div className="hp-add-row">
            <span className="hp-add-k">命令</span>
            <input className="input mono" value={s.command} placeholder="npx" spellCheck={false} onChange={e => onChange({ ...s, command: e.target.value })} />
          </div>
          <div className="hp-add-row top">
            <span className="hp-add-k">参数</span>
            <textarea
              className="input mono sc-args"
              value={args}
              rows={3}
              placeholder={'一行一个，例如\n-y\n@modelcontextprotocol/server-github'}
              spellCheck={false}
              onChange={e => {
                setArgs(e.target.value);
                onChange({ ...s, args: argsOf(e.target.value) });
              }}
            />
          </div>
        </>
      ) : (
        <div className="hp-add-row">
          <span className="hp-add-k">地址</span>
          <input className="input mono" value={s.url} placeholder="https://example.com/mcp" spellCheck={false} onChange={e => onChange({ ...s, url: e.target.value })} />
        </div>
      )}
      <div className="hp-add-row top">
        <span className="hp-add-k">{s.kind === 'stdio' ? '环境变量' : '请求头'}</span>
        <div className="sc-varlist">
          {vars.map((v, i) => (
            <div className="sc-var" key={i}>
              <input className="input mono" value={v.key} placeholder={s.kind === 'stdio' ? 'GITHUB_TOKEN' : 'Authorization'} spellCheck={false} aria-label="名字" onChange={e => setVars(vars.map((x, j) => (j === i ? { ...x, key: e.target.value } : x)))} />
              <input
                className="input mono"
                type={v.secret ? 'password' : 'text'}
                value={v.value ?? ''}
                placeholder={v.secret && v.tail != null ? `${maskOf(v)}（不改就留空）` : '值'}
                spellCheck={false}
                autoComplete="off"
                aria-label="值"
                onChange={e => setVars(vars.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))}
              />
              <label className="sc-secret" data-tip="只存在服务端，界面上只显示末尾 4 位，只发给 board host">
                <input type="checkbox" checked={!!v.secret} onChange={e => setVars(vars.map((x, j) => (j === i ? { ...x, secret: e.currentTarget.checked } : x)))} />
                密钥
              </label>
              <button className="icon-btn sm" type="button" aria-label="去掉这一行" onClick={() => setVars(vars.filter((_, j) => j !== i))}>
                <Icon name="close" />
              </button>
            </div>
          ))}
          <div className="lh-actions">
            <button className="btn sm" type="button" onClick={() => setVars([...vars, { key: '', value: '', secret: false }])}>
              <Icon name="plus" />
              加一个
            </button>
          </div>
        </div>
      </div>
      <div className="hp-add-row">
        <span className="hp-add-k">同步给</span>
        <div className="sc-agents">
          {MCP_AGENTS.map(a => (
            <label key={a}>
              <input
                type="checkbox"
                checked={s.agents.includes(a)}
                onChange={e => onChange({ ...s, agents: e.currentTarget.checked ? [...s.agents, a] : s.agents.filter(x => x !== a) })}
              />
              {MCP_AGENT_NAMES[a]}
            </label>
          ))}
          <span className="hp-note">{s.agents.length ? '' : '都不勾 = 所有支持的 agent'}</span>
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
