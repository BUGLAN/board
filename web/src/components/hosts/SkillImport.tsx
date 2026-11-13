/* ==========================================================================
   从机器导入 skills（PRD §10.12「共用配置」）：居中弹窗，和其他共用配置的表单同一种样子
   - 从哪台机器：一台一块磁贴（机器名、有几个本机的 skill）；离线、board 太旧的不能选，写明原因
   - 这台机器上的 skill：它的各 agent 目录里不是同步装的，一个一行（勾选、名称、版本、和库里比：库里没有 / 内容不一样会换成这份 /
     和库里一样 / 不能导入的原因，在哪些 agent 的目录里、文件数和大小）；只有前两种能勾
   - 导入：服务端记下请求，那台机器的 board host 把选中的目录传进库；弹窗留着写「正在等 <机器> 上传」，做完列出结果
   请求要等服务端：按钮转圈、禁用直到返回；结果跟着机器的上报更新
   ========================================================================== */
import { useEffect, useMemo, useState } from 'react';
import { api } from '../../lib/api';
import { since } from '../../lib/hosts';
import { importRows, type ImportKind } from '../../lib/skills';
import type { HostInfo, SkillImportPick, SkillInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic } from '../../store/data';
import { toastError } from '../../store/ui';
import { useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon, OsGlyph } from './bits';
import { FormFoot, FormModal, FormSec, FormTile } from './FormModal';

const kb = (n: number) => (n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${Math.ceil(n / 1024)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);
const KIND_WORDS: Record<ImportKind, string> = { new: '新的', differs: '会换掉库里的', same: '和库里一样', bad: '不能导入' };

/** 一台机器能不能选、为什么 */
function hostNote(h: HostInfo): { ok: boolean; note: string } {
  if (!h.skillImport?.able) return { ok: false, note: `board ${h.version || '?'} 太旧，要升级` };
  if (!h.online) return { ok: false, note: '离线' };
  const n = h.skillImport.local.length;
  return { ok: true, note: n ? `${n} 个本机的 skill` : '没有本机的 skill' };
}

export function SkillImportModal({ hosts, lib, onClose }: { hosts: HostInfo[]; lib: SkillInfo[]; onClose: () => void }) {
  const now = useNow(5000);
  const first = hosts.find(h => hostNote(h).ok && h.skillImport!.local.length) ?? hosts.find(h => hostNote(h).ok) ?? null;
  const [name, setName] = useState<string | null>(first?.name ?? null);
  const [picked, setPicked] = useState<SkillImportPick[]>([]);
  const [busy, setBusy] = useState(false);
  /** 发出去的请求：等这台机器做完（结果里的序号对上） */
  const [sent, setSent] = useState<{ host: string; id: number } | null>(null);
  const h = hosts.find(x => x.name === (sent?.host ?? name)) ?? null;
  const rows = useMemo(() => (h?.skillImport ? importRows(h.skillImport.local, lib) : []), [h, lib]);
  useEffect(() => setPicked([]), [name]);
  const last = h?.skillImport?.last;
  const done = !!sent && last?.id === sent.id;
  const waiting = !!sent && !done;
  const has = (n: string) => picked.some(p => p.name === n);
  const flip = (n: string, dir: string) => setPicked(list => (list.some(p => p.name === n) ? list.filter(p => p.name !== n) : [...list, { name: n, dir }]));
  const go = async () => {
    if (!h || !picked.length) return;
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
  const can = rows.filter(r => r.kind === 'new' || r.kind === 'differs');
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
            note={picked.length ? `选了 ${picked.length} 个：${h?.name} 的 board host 会把它们传进库，再同步到每台机器` : '勾选要放进共用库的 skill'}
            saveLabel={picked.length ? `导入 ${picked.length} 个` : '导入'}
            canSave={picked.length > 0}
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
                <span key={x.id} className={cx('si-host', !n.ok && 'off')} data-tip={n.ok ? undefined : n.note}>
                  <FormTile on={x.name === h?.name} mark={<OsGlyph os={x.os} small />} title={x.name} desc={n.note} onClick={() => n.ok && !sent && setName(x.name)} />
                </span>
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
          hint={sent ? undefined : h.skillImport?.able ? `${rows.length} 个` : undefined}
          act={
            !sent && can.length > 0 ? (
              <button type="button" className="act hp-link" onClick={() => setPicked(picked.length === can.length ? [] : can.map(r => ({ name: r.skill.name, dir: r.skill.dirs[0] })))}>
                {picked.length === can.length ? '全不选' : '全选能导入的'}
              </button>
            ) : undefined
          }
        >
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
          ) : !rows.length ? (
            <p className="fm-empty">这台机器的各 agent 目录里没有自己放的 skill：都是从库里同步装的，或者还没装 skill</p>
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
                    disabled={!ok}
                    className={cx('si-row', on && 'on', `k-${r.kind}`)}
                    onClick={() => flip(r.skill.name, r.skill.dirs[0])}
                  >
                    <span className={cx('fm-check', on && 'on')}>{on && <Icon name="check" />}</span>
                    <span className="si-main">
                      <span className="si-name">
                        <b>{r.skill.name}</b>
                        {r.skill.version && <code>{r.skill.version.slice(0, 7)}</code>}
                        <em className={cx('si-kind', `k-${r.kind}`)} data-tip={r.note}>
                          {KIND_WORDS[r.kind]}
                        </em>
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
        </FormSec>
      )}
    </FormModal>
  );
}
