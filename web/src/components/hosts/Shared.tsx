/* ==========================================================================
   机器 Tab「共用配置」（PRD §10.12「共用配置」）：第一节是 skills 共用库
   - 列表：名称、版本、更新时间、来源；board-skills 固定在第一项（随服务端版本，不能删、不能改）
   - 展开一个 skill：每台机器、每个 agent 的状态——已同步 / 版本落后 / 本机有同名没覆盖 / 失败及原因；旧版 board host 写「board 版本太旧，不同步」
   - 「上传目录」选本机的 skill 目录（桌面端、网页端都能选），「上传 zip」传一个 zip；同名的换成新版本，在线的机器马上同步
   上传、删除要等服务端记下：按钮转圈、禁用直到返回（optimistic(null, …)）；同步的进展按机器上报画
   ========================================================================== */
import { useRef, useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import { packFiles, readZip, skillRows, skillSummary, STATE_WORDS, type RawFile } from '../../lib/skills';
import type { HostInfo, SkillInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, openHost, toast, toastError } from '../../store/ui';
import { useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon, Dot } from './bits';
import { useRemoteList } from './useRemoteList';

const kb = (n: number) => (n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${Math.ceil(n / 1024)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);

export function SharedConfig({ hosts }: { hosts: HostInfo[] }) {
  const ws = useData(s => s.ws);
  const now = useNow(10000);
  const { list: lib, err, refresh: load } = useRemoteList(() => api.skills(), ws?.hosts);
  const [open, setOpen] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const dirInput = useRef<HTMLInputElement>(null);
  const zipInput = useRef<HTMLInputElement>(null);
  const push = async (raw: RawFile[], fallback: string) => {
    const p = packFiles(raw, fallback);
    if ('error' in p) {
      toast({ type: 'error', title: '没有上传', text: p.error });
      return;
    }
    setBusy('upload');
    try {
      const info = await optimistic(null, () => api.pushSkill(p.name, p.files));
      toast({ type: 'success', title: `已上传 ${info.name}`, text: `版本 ${info.version.slice(0, 7)}：在线的机器马上同步`, duration: 2400 });
      setOpen(info.name);
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const fromDir = async (files: FileList | null) => {
    if (!files?.length) return;
    const raw = await Promise.all(
      [...files].map(async f => ({ path: (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name, data: new Uint8Array(await f.arrayBuffer()) })),
    );
    await push(raw, '');
  };
  const fromZip = async (file: File | undefined) => {
    if (!file) return;
    try {
      await push(await readZip(await file.arrayBuffer()), file.name);
    } catch (e) {
      toast({ type: 'error', title: '没有上传', text: String((e as Error)?.message ?? e) });
    }
  };
  const remove = async (s: SkillInfo) => {
    const ok = await confirm({
      title: `从库里删掉 ${s.name}？`,
      text: '各台机器上由同步装的那份会被删掉；本机自己放的同名 skill 不受影响。',
      ok: '删掉',
      icon: 'trash',
      tone: 'warn',
    });
    if (!ok) return;
    setBusy(`rm:${s.name}`);
    try {
      await optimistic(null, () => api.removeSkill(s.name));
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="sc">
      <div className="sc-head">
        <div>
          <h2>skills</h2>
          <p>
            放在这里的 skill，每台机器的 board host 会同步到它找到的每个 agent 读 skill 的目录（~/.agents/skills、~/.claude/skills、$CODEX_HOME/skills，WSL 里的也装）。
            本机已经有的同名 skill 不覆盖。命令行：<code>board skills push &lt;目录&gt;</code>
          </p>
        </div>
        <div className="sc-acts">
          <button className={cx('btn sm', busy === 'upload' && 'is-busy')} type="button" disabled={!!busy} onClick={() => dirInput.current?.click()} data-tip="选一个 skill 目录（里面直接有 SKILL.md），合计不超过 5 MB" data-tip-align="end">
            <Icon name="folder" />
            上传目录
          </button>
          <button className="btn sm" type="button" disabled={!!busy} onClick={() => zipInput.current?.click()} data-tip="一个 skill 打成的 zip（解开后是 skill 目录，或直接是 SKILL.md 这些文件）" data-tip-align="end">
            <Icon name="file" />
            上传 zip
          </button>
          <input
            ref={dirInput}
            type="file"
            hidden
            multiple
            {...({ webkitdirectory: '', directory: '' } as Record<string, string>)}
            onChange={e => {
              void fromDir(e.currentTarget.files);
              e.currentTarget.value = '';
            }}
          />
          <input
            ref={zipInput}
            type="file"
            hidden
            accept=".zip,application/zip"
            onChange={e => {
              void fromZip(e.currentTarget.files?.[0]);
              e.currentTarget.value = '';
            }}
          />
        </div>
      </div>
      {err && <p className="hp-note lh-warn">取不到 skills 库：{err}</p>}
      {!lib && !err && <p className="hp-note">正在读取…</p>}
      {lib && (
        <div className="sc-list">
          {lib.map(s => {
            const rows = skillRows(hosts, s);
            const sum = skillSummary(rows);
            const on = open === s.name;
            return (
              <div key={s.name} className={cx('sc-item', on && 'open')}>
                <div className="sc-row">
                  <button type="button" className="sc-toggle" aria-expanded={on} onClick={() => setOpen(on ? null : s.name)}>
                    <Icon name={on ? 'chevDown' : 'chevRight'} />
                    <b>{s.name}</b>
                    <code title={s.version}>{s.version.slice(0, 7)}</code>
                    {s.builtin && <span className="sc-fixed">固定 · 随服务端版本</span>}
                  </button>
                  <span className="sc-meta">
                    {s.files} 个文件 · {kb(s.size)}
                    {s.builtin ? ` · ${s.source}` : ` · ${s.source}${s.by && s.by !== s.source ? `（${s.by}）` : ''} · ${since(s.updatedAt, now)}更新`}
                  </span>
                  <span className={cx('sc-sum', sum.bad ? 'bad' : sum.synced === rows.length - sum.old && rows.length ? 'ok' : '')}>
                    {rows.length === 0 ? '还没有机器' : `${sum.synced}/${rows.length} 台已同步${sum.bad ? ` · ${sum.bad} 台有问题` : ''}${sum.old ? ` · ${sum.old} 台不同步` : ''}`}
                  </span>
                  {!s.builtin && (
                    <button
                      className={cx('icon-btn sm', busy === `rm:${s.name}` && 'is-busy')}
                      type="button"
                      aria-label={`删掉 ${s.name}`}
                      data-tip="从库里删掉"
                      data-tip-align="end"
                      disabled={!!busy}
                      onClick={() => void remove(s)}
                    >
                      <Icon name="trash" />
                    </button>
                  )}
                </div>
                {on && (
                  <div className="sc-hosts">
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
                            <span className="sc-cell old">{r.host.online ? '没有找到能装 skill 的 agent' : '离线'}</span>
                          ) : (
                            r.cells.map((c, i) => (
                              <span key={`${c.agent}:${i}`} className={cx('sc-cell', c.state)} title={c.dir}>
                                {c.agent && <AgentIcon id={c.agent} />}
                                <b>{c.agent ? (r.host.agents.find(a => a.id === c.agent)?.name ?? c.agent) : '各 agent'}</b>
                                <em>{STATE_WORDS[c.state]}</em>
                                {c.why && <small>{c.why}</small>}
                              </span>
                            ))
                          )}
                        </div>
                      </div>
                    ))}
                    {!rows.length && <p className="hp-note">还没有机器：加了机器之后它会自动同步</p>}
                  </div>
                )}
              </div>
            );
          })}
          {lib.length === 1 && <p className="hp-note sc-empty">除了 board-skills 还没有别的：上传一个 skill 目录，每台机器都会装上</p>}
        </div>
      )}
    </section>
  );
}
