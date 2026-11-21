/* ==========================================================================
   从机器导入 skills（PRD §10.16「共用配置」）：居中弹窗，和其他共用配置的表单同一种样子
   - 从哪台机器：一台一块磁贴（机器名、有几个本机的 skill）；离线、board 太旧的不能选，写明原因
   - 这台机器上的 skill：它的各 agent 目录里不是同步装的，一个一行（勾选、名称、版本、和库里比：库里没有 / 内容不一样会换成这份 /
     和库里一样 / 不能导入的原因，在哪些 agent 的目录里、文件数和大小）；只有前两种能勾
   - 导入：服务端记下请求，那台机器的 board host 把选中的目录传进库；弹窗留着写「正在等 <机器> 上传」，做完列出结果
   请求要等服务端：按钮转圈、禁用直到返回；结果跟着机器的上报更新
   ========================================================================== */
import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import { importRows, type ImportKind } from '../../lib/skills';
import type { HostInfo, SkillImportPick, SkillInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic } from '../../store/data';
import { toastError } from '../../store/ui';
import { Tip, useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon, OsGlyph } from './bits';
import { FormFoot, FormModal, FormSec, FormTile } from './FormModal';

const kb = (n: number) => (n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${Math.ceil(n / 1024)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);
const KIND_WORDS: Record<ImportKind, string> = { new: '新的', differs: '会换掉库里的', same: '和库里一样', bad: '不能导入' };

/** 一台机器能不能选、为什么 */
function hostNote(h: HostInfo): { ok: boolean; note: string } {
  if (!h.skillImport?.able) return { ok: false, note: `board ${h.version || '?'} 太旧，要升级` };
  const n = h.skillImport.local.length;
  if (!h.online) return { ok: true, note: `离线 · 上次上报的 ${n} 个` };
  if (!h.skillImport.rescan) return { ok: true, note: `${n} 个（缓存，board 太旧）` };
  return { ok: true, note: n ? `${n} 个本机的 skill` : '没有本机的 skill' };
}

export function SkillImportModal({ hosts, lib, onClose }: { hosts: HostInfo[]; lib: SkillInfo[]; onClose: () => void }) {
  const now = useNow(5000);
  const first = hosts.find(h => hostNote(h).ok && h.skillImport!.local.length) ?? hosts.find(h => hostNote(h).ok) ?? null;
  const [name, setName] = useState<string | null>(first?.name ?? null);
  const [picked, setPicked] = useState<SkillImportPick[]>([]);
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  /** 这次打开弹窗已经自动重扫过的机器：换回同一台时不马上再发，服务端另有 10 秒防抖 */
  const scanned = useRef<{ name: string; at: number } | null>(null);
  /** 发出去的请求：等这台机器做完（结果里的序号对上） */
  const [sent, setSent] = useState<{ host: string; id: number } | null>(null);
  const h = hosts.find(x => x.name === (sent?.host ?? name)) ?? null;
  const allRows = useMemo(() => (h?.skillImport ? importRows(h.skillImport.local, lib) : []), [h, lib]);
  const q = query.trim().toLowerCase();
  const rows = q ? allRows.filter(r => r.skill.name.toLowerCase().includes(q)) : allRows;
  useEffect(() => {
    setPicked([]);
    setQuery('');
  }, [name]);
  const last = h?.skillImport?.last;
  const done = !!sent && last?.id === sent.id;
  const waiting = !!sent && !done;
  const has = (n: string) => picked.some(p => p.name === n);
  const flip = (n: string, dir: string) => setPicked(list => (list.some(p => p.name === n) ? list.filter(p => p.name !== n) : [...list, { name: n, dir }]));
  const go = async () => {
    if (!h || !h.online || !picked.length) return;
    setBusy(true);
    try {
      const out = await optimistic(null, () => api.importSkills(h.name, picked));
      if (out.skillImport?.ask) setSent({ host: h.name, id: out.skillImport.ask.id });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  const rescan = async (host: HostInfo, quiet = false) => {
    if (!host.online || !host.skillImport?.rescan || scanning) return;
    setScanning(true);
    try {
      await optimistic(null, () => api.rescanHostSkills(host.name));
    } catch (e) {
      if (!quiet) toastError(e);
    } finally {
      setScanning(false);
    }
  };
  useEffect(() => {
    if (!h?.online || !h.skillImport?.rescan) return;
    const at = Date.now();
    if (scanned.current?.name === h.name && at - scanned.current.at < 10_000) return;
    scanned.current = { name: h.name, at };
    void rescan(h, true);
    // 只跟着选中的机器和它的重扫能力走；机器每 10 秒的普通上报不重复触发
  }, [h?.name, h?.online, h?.skillImport?.rescan]);
  const can = rows.filter(r => r.kind === 'new' || r.kind === 'differs');
  const canNames = new Set(can.map(r => r.skill.name));
  const allCan = can.length > 0 && can.every(r => has(r.skill.name));
  const toggleAll = () =>
    setPicked(list => {
      if (allCan) return list.filter(p => !canNames.has(p.name));
      const have = new Set(list.map(p => p.name));
      return [...list, ...can.filter(r => !have.has(r.skill.name)).map(r => ({ name: r.skill.name, dir: r.skill.dirs[0] }))];
    });
  return (
    <FormModal
      title="从机器导入 skills"
      onCancel={onClose}
      foot={
        sent ? (
          <FormFoot why={null} tried={false} busy={false} note={done ? '做完了：库里的 skills 已经换上' : `正在等 ${sent.host} 把它们传进库…`} cancelLabel="关闭" saveLabel="完成" canSave={done} onCancel={onClose} onSave={onClose} />
        ) : (
          <FormFoot
            why={null}
            tried={false}
            busy={busy}
            note={
              !h?.online
                ? '机器离线：只能看上次上报的清单，等它上线后再导入'
                : picked.length
                  ? `选了 ${picked.length} 个：${h.name} 的 board host 会把它们传进库，再同步到每台机器`
                  : '勾选要放进共用库的 skill'
            }
            saveLabel={picked.length ? `导入 ${picked.length} 个` : '导入'}
            canSave={!!h?.online && picked.length > 0}
            onCancel={onClose}
            onSave={() => void go()}
          />
        )
      }
    >
      <FormSec title="从哪台机器" hint="列出它各 agent 的 skill 目录里、不是从库里同步装的">
        {hosts.length ? (
          <div className="fm-tiles">
            {hosts.map(x => {
              const n = hostNote(x);
              return (
                <Tip key={x.id} text={n.ok ? '' : n.note}>
                  <span className={cx('si-host', !n.ok && 'off')}>
                    <FormTile on={x.name === h?.name} mark={<OsGlyph os={x.os} small />} title={x.name} desc={n.note} onClick={() => n.ok && !sent && setName(x.name)} />
                  </span>
                </Tip>
              );
            })}
          </div>
        ) : (
          <p className="fm-empty">还没有机器</p>
        )}
      </FormSec>
      {h && (
        <FormSec
          title={sent ? `${h.name} ${done ? '导入完成' : '正在导入'}` : `${h.name} 上的 skill`}
          hint={sent ? undefined : h.skillImport?.able ? (scanning || h.ask?.kind === 'rescan-skills' ? '正在重扫…' : q ? `${rows.length} / ${allRows.length} 个` : `${allRows.length} 个`) : undefined}
          act={
            !sent && h ? (
              <span className="si-acts">
                {can.length > 0 && (
                  <button type="button" className="act hp-link" onClick={toggleAll}>
                    {allCan ? '全不选' : '全选能导入的'}
                  </button>
                )}
                {h.skillImport?.rescan ? (
                  <Tip text={h.online ? '让 board host 重新看一遍各 agent 的 skill 目录' : '机器离线：上线后会自动重扫'}>
                    <button
                      type="button"
                      className={cx('act hp-link', (scanning || h.ask?.kind === 'rescan-skills') && 'is-busy')}
                      disabled={!h.online || scanning || h.ask?.kind === 'rescan-skills'}
                      onClick={() => void rescan(h)}
                    >
                      <Icon name="renew" />
                      {scanning || h.ask?.kind === 'rescan-skills' ? '正在重扫…' : '刷新'}
                    </button>
                  </Tip>
                ) : (
                  <span className="si-old">board {h.version || '?'} 太旧，要升级</span>
                )}
              </span>
            ) : undefined
          }
        >
          {!h.online && <p className="hp-note lh-warn">机器离线：下面是上次上报的 skill，重新打开弹窗、等它上线后会自动重扫。</p>}
          {h.online && h.skillImport?.able && !h.skillImport.rescan && (
            <p className="hp-note lh-warn">board {h.version || '?'} 太旧，要升级：下面是上次上报的 skill，升级后重新打开会自动重扫。</p>
          )}
          {sent ? (
            <div className="si-result">
              {(done ? [...(last?.ok ?? []).map(n => ({ name: n, ok: true, why: '' })), ...(last?.failed ?? []).map(f => ({ name: f.name, ok: false, why: f.why }))] : (h.skillImport?.ask?.picks ?? picked).map(p => ({ name: p.name, ok: false, why: '' }))).map(r => (
                <div key={r.name} className={cx('si-res', done && (r.ok ? 'ok' : 'bad'))}>
                  {done ? <Icon name={r.ok ? 'check' : 'alert'} /> : <i className="si-spin" />}
                  <b>{r.name}</b>
                  <span>{done ? (r.ok ? '已放进库，在线的机器马上同步' : r.why) : '等 board host 上传'}</span>
                </div>
              ))}
              {waiting && h.skillImport?.ask && <p className="fm-empty">{since(h.skillImport.ask.at, now)}请求的：board host 每 10 秒取一次任务，取到就开始</p>}
            </div>
          ) : !allRows.length ? (
            <p className="fm-empty">这台机器的各 agent 目录里没有自己放的 skill：都是从库里同步装的，或者还没装 skill</p>
          ) : (
            <>
              <label className="si-search">
                <Icon name="search" />
                <input value={query} placeholder="搜索 skill…" spellCheck={false} autoComplete="off" aria-label="搜索 skill" onChange={e => setQuery(e.target.value)} />
              </label>
              {!rows.length ? (
                <p className="fm-empty si-empty">没有匹配的 skill</p>
              ) : (
                <div className="si-list">
                  {rows.map(r => {
                    const ok = r.kind === 'new' || r.kind === 'differs';
                    const on = has(r.skill.name) && picked.find(p => p.name === r.skill.name)?.dir === r.skill.dirs[0];
                    return (
                      <button
                        key={`${r.skill.name}:${r.skill.version}:${r.skill.dirs[0]}`}
                        type="button"
                        role="checkbox"
                        aria-checked={on}
                        disabled={!ok || !h.online}
                        className={cx('si-row', on && 'on', `k-${r.kind}`)}
                        onClick={() => flip(r.skill.name, r.skill.dirs[0])}
                      >
                        <span className={cx('fm-check', on && 'on')}>{on && <Icon name="check" />}</span>
                        <span className="si-main">
                          <span className="si-name">
                            <b>{r.skill.name}</b>
                            {r.skill.version && <code>{r.skill.version.slice(0, 7)}</code>}
                            <Tip text={r.note}>
                              <em className={cx('si-kind', `k-${r.kind}`)}>{KIND_WORDS[r.kind]}</em>
                            </Tip>
                          </span>
                          <span className="si-meta">
                            {r.kind === 'bad' || r.kind === 'differs' ? `${r.note} · ` : ''}
                            {r.skill.dirs.join('、')}
                            {r.skill.files ? ` · ${r.skill.files} 个文件 · ${kb(r.skill.size)}` : ''}
                          </span>
                        </span>
                        <span className="si-agents">
                          {r.skill.agents.map(a => (
                            <AgentIcon key={a} id={a} />
                          ))}
                        </span>
                      </button>
                    );
                  })}
                </div>
              )}
            </>
          )}
        </FormSec>
      )}
    </FormModal>
  );
}
