/* ==========================================================================
   「共用配置」的 MCP 服务器一节（PRD §10.12「共用配置」）
   - 一个服务器一行：名称、类型、命令和参数（或地址）、同步给谁（agent 图标）、各台机器的胶囊（一台一个，合成这台机器上各 agent 的状态）、修改、删除；
     点一行展开环境变量 / 请求头（密钥只写末尾 4 位）和「机器 × agent」的矩阵；版本太旧、不同步的机器在列表下面一句话说完
   - 添加 / 修改在居中弹窗里：名称、类型（本地命令 stdio：命令、参数、环境变量；远程 HTTP：地址、请求头）、同步给（所有支持的 agent / 只给哪几个）。
     值可以标成「密钥」：保存后只显示末尾 4 位，修改时留空保留原值
   - 桌面端「从本机导入」：读出这台电脑上各 agent 已经配好的，勾选后加进来（名字像密钥的值标成密钥）
   保存、删除、导入要等服务端：按钮转圈、禁用直到返回（optimistic(null, …)）；失败时表单留着，人填的不丢
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import { blankMcp, maskOf, MCP_AGENT_NAMES, MCP_AGENTS, mcpError, toSave } from '../../lib/mcp';
import { mcpSync } from '../../lib/syncPills';
import { localMcp, shell } from '../../lib/platform';
import type { HostInfo, McpServer, McpVar } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, toast, toastError } from '../../store/ui';
import { Segmented, useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon } from './bits';
import { ChipInput } from './ChipInput';
import { FormFoot, FormModal } from './FormModal';
import { HostSyncPills, HowTip, OldHostsNote, SyncMatrix } from './SyncBits';
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

  const old = hosts.filter(h => h.mcp == null);
  const startEdit = (s: McpServer) => setEdit({ s: { ...s, env: s.env.map(v => ({ ...v, value: v.secret ? '' : v.value })), headers: s.headers.map(v => ({ ...v, value: v.secret ? '' : v.value })) }, editing: true });

  return (
    <section className="sc sc-mcp">
      <div className="sc-head">
        <div>
          <h2>MCP 服务器</h2>
          <p>
            每台机器的 board host 把这里的 MCP 服务器写进 Claude Code、Codex、pi 的用户级配置（dsh 不支持）。{' '}
            <HowTip tip="只动同步加进去的，agent 里已有同名的不覆盖。值可以标成密钥：只存在服务端，界面上只显示末尾 4 位，完整值只发给 board host。" />
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
      {edit && (
        <McpForm value={edit.s} editing={edit.editing} busy={busy === 'save'} existing={list?.map(x => x.name) ?? []} onChange={s => setEdit({ ...edit, s })} onSave={() => void save()} onCancel={() => setEdit(null)} />
      )}
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
      {list && !list.length && <p className="hp-note">还没有：点「添加」，或在桌面端「从本机导入」</p>}
      {list && list.length > 0 && (
        <div className="sy-list sy-mcp">
          {list.map(s => {
            const sync = mcpSync(hosts, s);
            const on = open === s.name;
            const vars = [...s.env, ...s.headers];
            const cmd = s.kind === 'http' ? s.url : [s.command, ...s.args].join(' ');
            return (
              <div key={s.name} className={cx('sy-item', on && 'open')}>
                <div className="sy-row">
                  <button type="button" className="sy-toggle" aria-expanded={on} onClick={() => setOpen(on ? null : s.name)}>
                    <Icon name="chevRight" />
                    <b>{s.name}</b>
                    <span className="sc-fixed">{s.kind === 'http' ? 'HTTP' : 'stdio'}</span>
                  </button>
                  <code className="sy-cmd" title={cmd}>
                    {cmd}
                  </code>
                  <span className="sy-who" data-tip={s.agents.length ? `只同步给 ${s.agents.map(a => MCP_AGENT_NAMES[a] ?? a).join('、')}` : '同步给所有支持的 agent（Claude Code、Codex、pi）'}>
                    {(s.agents.length ? s.agents : [...MCP_AGENTS]).map(a => (
                      <AgentIcon key={a} id={a} />
                    ))}
                  </span>
                  <HostSyncPills list={sync} />
                  <span className="sy-acts">
                    <button className="icon-btn sm" type="button" aria-label={`修改 ${s.name}`} data-tip="修改" disabled={!!busy || !!edit} onClick={() => startEdit(s)}>
                      <Icon name="edit" />
                    </button>
                    <button className={cx('icon-btn sm', busy === `rm:${s.name}` && 'is-busy')} type="button" aria-label={`删掉 ${s.name}`} data-tip="删掉" data-tip-align="end" disabled={!!busy} onClick={() => void remove(s)}>
                      <Icon name="trash" />
                    </button>
                  </span>
                </div>
                {on && (
                  <div className="sy-detail">
                    {s.updatedAt ? (
                      <p className="sy-upd">
                        {since(s.updatedAt, now)}更新{s.by ? ` · ${s.by}` : ''}
                      </p>
                    ) : null}
                    {vars.length > 0 && (
                      <div className="sy-vars">
                        <span>{s.kind === 'http' ? '请求头' : '环境变量'}</span>
                        {vars.map(v => (
                          <code key={v.key}>
                            {v.key}={v.secret ? <em>{maskOf(v)}</em> : v.value}
                            {v.secret && <Icon name="lock" />}
                          </code>
                        ))}
                      </div>
                    )}
                    <SyncMatrix list={sync} />
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
      <OldHostsNote hosts={old} what=" MCP" />
    </section>
  );
}

const KINDS: ['stdio' | 'http', string][] = [
  ['stdio', '本地命令（stdio）'],
  ['http', '远程 HTTP'],
];

/** 添加 / 修改：居中弹窗。名称和类型一行；stdio 是命令 + 参数（一个参数一个胶囊，下面预览整条命令），HTTP 是地址；
 *  环境变量 / 请求头一行一个（名字 = 值，「密钥」开关）；同步给「所有支持的 agent / 只给」，只给时点选 agent。点保存时先在界面上校验 */
function McpForm({ value: s, editing, busy, existing, onChange, onSave, onCancel }: { value: McpServer; editing: boolean; busy: boolean; existing: string[]; onChange: (s: McpServer) => void; onSave: () => void; onCancel: () => void }) {
  const [tried, setTried] = useState(false);
  const [only, setOnly] = useState(s.agents.length > 0);
  const field = s.kind === 'stdio' ? 'env' : 'headers';
  const vars = s[field];
  const setVars = (vs: McpVar[]) => onChange({ ...s, [field]: vs });
  const why = mcpError(s, existing, editing);
  return (
    <FormModal
      title={editing ? `修改 ${s.name}` : '添加 MCP 服务器'}
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
        />
      }
    >
      <div className="fm-row two">
        <label className="fm-f">
          <span className="fm-k">名称</span>
          <input className="input mono" value={s.name} disabled={editing} placeholder="github" spellCheck={false} autoComplete="off" onChange={e => onChange({ ...s, name: e.target.value.trim() })} />
        </label>
        <span className="fm-f">
          <span className="fm-k">类型</span>
          <Segmented value={s.kind} options={KINDS} onChange={k => onChange({ ...s, kind: k })} />
        </span>
      </div>
      {s.kind === 'stdio' ? (
        <>
          <div className="fm-row">
            <span className="fm-k">命令</span>
            <div className="fm-v">
              <input className="input mono fm-short" value={s.command} placeholder="npx" spellCheck={false} aria-label="命令" onChange={e => onChange({ ...s, command: e.target.value })} />
            </div>
          </div>
          <div className="fm-row">
            <span className="fm-k">参数</span>
            <div className="fm-v">
              <ChipInput value={s.args} allowSpace onChange={args => onChange({ ...s, args })} placeholder="一个参数一个，回车加，例如 -y、@modelcontextprotocol/server-github" label="参数" />
              {(s.command || s.args.length > 0) && (
                <code className="fm-preview" title="board host 会这样启动它">
                  {[s.command, ...s.args.map(a => (/\s/.test(a) ? `"${a}"` : a))].filter(Boolean).join(' ')}
                </code>
              )}
            </div>
          </div>
        </>
      ) : (
        <div className="fm-row">
          <span className="fm-k">地址</span>
          <div className="fm-v">
            <input className="input mono" value={s.url} placeholder="https://example.com/mcp" spellCheck={false} aria-label="地址" onChange={e => onChange({ ...s, url: e.target.value })} />
          </div>
        </div>
      )}
      <div className="fm-row">
        <span className="fm-k">{s.kind === 'stdio' ? '环境变量' : '请求头'}</span>
        <div className="fm-v fm-vars">
          {vars.map((v, i) => (
            <div className="fm-var" key={i}>
              <input className="input mono" value={v.key} placeholder={s.kind === 'stdio' ? 'GITHUB_TOKEN' : 'Authorization'} spellCheck={false} aria-label="名字" onChange={e => setVars(vars.map((x, j) => (j === i ? { ...x, key: e.target.value } : x)))} />
              <span className="fm-eq">=</span>
              <input
                className="input mono"
                type={v.secret ? 'password' : 'text'}
                value={v.value ?? ''}
                placeholder={v.secret && v.tail != null ? `${maskOf(v)}，不改就留空` : '值'}
                spellCheck={false}
                autoComplete="off"
                aria-label="值"
                onChange={e => setVars(vars.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))}
              />
              <button
                type="button"
                className={cx('fm-secret', v.secret && 'on')}
                aria-pressed={!!v.secret}
                data-tip="密钥：只存在服务端，界面上只显示末尾 4 位，完整值只发给 board host"
                onClick={() => setVars(vars.map((x, j) => (j === i ? { ...x, secret: !x.secret } : x)))}
              >
                <Icon name="lock" />
                密钥
              </button>
              <button className="icon-btn sm" type="button" aria-label="去掉这一行" onClick={() => setVars(vars.filter((_, j) => j !== i))}>
                <Icon name="close" />
              </button>
            </div>
          ))}
          <button className="act hp-link fm-add" type="button" onClick={() => setVars([...vars, { key: '', value: '', secret: false }])}>
            <Icon name="plus" />
            加一个
          </button>
        </div>
      </div>
      <div className="fm-row">
        <span className="fm-k">同步给</span>
        <div className="fm-v fm-who">
          <Segmented
            value={only ? 'some' : 'all'}
            options={[
              ['all', '所有支持的 agent'],
              ['some', '只给'],
            ]}
            onChange={v => {
              setOnly(v === 'some');
              if (v === 'all') onChange({ ...s, agents: [] });
              else if (!s.agents.length) onChange({ ...s, agents: ['claude'] });
            }}
          />
          {only ? (
            <span className="fm-agents">
              {MCP_AGENTS.map(a => {
                const yes = s.agents.includes(a);
                return (
                  <button key={a} type="button" className={cx('fm-agent', yes && 'on')} aria-pressed={yes} onClick={() => onChange({ ...s, agents: yes ? s.agents.filter(x => x !== a) : [...s.agents, a] })}>
                    <AgentIcon id={a} />
                    {MCP_AGENT_NAMES[a]}
                  </button>
                );
              })}
            </span>
          ) : (
            <em className="fm-hint">Claude Code、Codex、pi（dsh 不支持）</em>
          )}
        </div>
      </div>
    </FormModal>
  );
}
