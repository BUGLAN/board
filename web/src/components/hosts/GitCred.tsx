/* ==========================================================================
   「共用配置」的 Git 凭据一节（PRD §10.12「共用配置」）：服务端存一份 https 远端的令牌，
   机器选了「用服务器的」（默认），board host 把它配进 Board 管的仓库，备代码、agent 推集成分支直接用
   - 没设时一句话 +「设置」；设了一张卡：主机、令牌末尾 4 位、谁什么时候改的、修改、删除；用户名、远端
   - 卡片底下「机器」：一台一行——系统图标、机器名（点了打开那台机器）、状态（已同步 · 配进了几个仓库 / 同步失败：原因 / 不同步）、
     「同步 | 不同步」开关。开关就是机器详情「设置」里那一项（机器设置，当帧生效、旁边转圈）；新连上的机器默认同步，
     有人在用的电脑（例如自己的主力机）在这里关掉
   - 设置、修改在居中弹窗里（和供应商、MCP 同一种表单）；修改时令牌留空保留原来的
   - 增、改、删要等服务端（按钮转圈、禁用直到返回），删掉时确认并列出要去掉凭据的机器
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { blankGit, gitCredError, gitDraftOf, normGitHost, type GitDraft } from '../../lib/gitcred';
import { gitOld } from '../../lib/hostDetail';
import { since } from '../../lib/hosts';
import { gitHosts, gitRow } from '../../lib/syncPills';
import type { GitCred, HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, openHost, toast, toastError } from '../../store/ui';
import { Segmented, Sync, Tip, useNow } from '../bits';
import { Icon } from '../Icon';
import { OsGlyph } from './bits';
import { FormField, FormFoot, FormModal, FormSec } from './FormModal';
import { useSaver } from './Saver';
import { HowTip, OldHostsNote } from './SyncBits';
import { useRemoteList } from './useRemoteList';

export function GitCredSection({ hosts }: { hosts: HostInfo[] }) {
  const ws = useData(s => s.ws);
  const now = useNow(10000);
  const { list: view, err, refresh: load } = useRemoteList(() => api.git(), ws?.hosts);
  const cred = view?.cred ?? null;
  // 正在编辑的：null 没有
  const [edit, setEdit] = useState<{ d: GitDraft; editing: boolean } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const save = async () => {
    if (!edit) return;
    setBusy('save');
    try {
      const d = edit.d;
      const out = await optimistic(null, () => api.putGit({ host: normGitHost(d.host), username: d.username.trim(), token: d.token.trim() }));
      setEdit(null);
      await load();
      toast({ type: 'success', title: `已保存 ${out.host} 的 Git 凭据`, text: '用服务器的机器马上配进 Board 管的仓库', duration: 2200 });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const remove = async () => {
    if (!cred) return;
    const users = hosts.filter(h => h.syncGit).map(h => h.name);
    const ok = await confirm({
      title: `删掉 ${cred.host} 的 Git 凭据？`,
      text: users.length
        ? `${users.join('、')} 用着它：board host 会从仓库里去掉它、删掉本机的令牌文件，之后用各自本机的 git 凭据推送。`
        : '没有机器用着它。',
      ok: '删掉',
      icon: 'trash',
      tone: 'warn',
    });
    if (!ok) return;
    setBusy('rm');
    try {
      await optimistic(null, () => api.removeGit());
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const old = hosts.filter(h => !h.sync?.able || gitOld(h));

  return (
    <section className="sc sc-git">
      <div className="sc-head">
        <div>
          <h2>Git 凭据</h2>
          <p>
            设一份 https 远端的令牌，board host 把它配进各台机器上 Board 管的仓库：拉代码、agent 推集成分支直接用，不用每台机器手动配。新连上的机器默认同步，有人在用的电脑在卡片上逐台关掉。{' '}
            <HowTip tip="只改仓库自己的 git 配置（credential.https://<主机>.helper），不碰全局配置；令牌放在 ~/.board/host/board-git-credentials，只有本人能读，不进日志和上报。一台机器关掉同步、或这里删掉后，配进去的会去掉。SSH 地址（git@…）的仓库不管，照旧用本机的 key。" />
          </p>
        </div>
        {!cred && view && (
          <div className="sc-acts">
            <button className="btn sm" type="button" disabled={!!busy || !!edit} onClick={() => setEdit({ d: blankGit(), editing: false })}>
              <Icon name="plus" />
              设置
            </button>
          </div>
        )}
      </div>
      {err && <p className="hp-note lh-warn">取不到 Git 凭据：{err}</p>}
      {!view && !err && <p className="hp-note">正在读取…</p>}
      {view && !cred && <p className="hp-note">还没有：点「设置」，填主机、用户名和令牌</p>}
      {cred && <GitCard cred={cred} hosts={hosts} now={now} busy={busy} editing={!!edit} onEdit={() => setEdit({ d: gitDraftOf(cred), editing: true })} onRemove={() => void remove()} />}
      {edit && <GitForm value={edit.d} editing={edit.editing} tail={cred?.tail ?? null} busy={busy === 'save'} onChange={d => setEdit({ ...edit, d })} onSave={() => void save()} onCancel={() => setEdit(null)} />}
      {view && <OldHostsNote hosts={old} what=" Git 凭据" />}
    </section>
  );
}

function GitCard({ cred, hosts, now, busy, editing, onEdit, onRemove }: { cred: GitCred; hosts: HostInfo[]; now: number; busy: string | null; editing: boolean; onEdit: () => void; onRemove: () => void }) {
  return (
    <article className="pv-card gc-card">
      <header className="pv-head">
        <Icon name="key" />
        <b className="pv-name">{cred.host}</b>
        <span className="sc-fixed">{cred.tail ? `令牌 ••••${cred.tail}` : '没填令牌'}</span>
        <span className="pv-upd">{cred.updatedAt ? `${cred.by ? `${cred.by} ` : ''}${since(cred.updatedAt, now)}更新` : ''}</span>
        <Tip text="修改主机、用户名、令牌">
          <button className="icon-btn sm" type="button" aria-label="修改 Git 凭据" disabled={!!busy || editing} onClick={onEdit}>
            <Icon name="edit" />
          </button>
        </Tip>
        <Tip text="删掉">
          <button className={cx('icon-btn sm', busy === 'rm' && 'is-busy')} type="button" aria-label="删掉 Git 凭据" data-tip-align="end" disabled={!!busy} onClick={onRemove}>
            <Icon name="trash" />
          </button>
        </Tip>
      </header>
      <div className="pv-kv">
        <div className="pv-line">
          <span className="pv-k">用户名</span>
          <code>{cred.username}</code>
        </div>
        <div className="pv-line">
          <span className="pv-k">用于</span>
          <code>https://{cred.host}/…</code>
        </div>
      </div>
      <GitHosts hosts={hosts} now={now} />
    </article>
  );
}

type OnOff = 'on' | 'off';

/** 卡片底下的机器：一台一行，「同步 | 不同步」就是机器详情「设置」里那个开关 */
function GitHosts({ hosts, now }: { hosts: HostInfo[]; now: number }) {
  const list = gitHosts(hosts);
  const on = list.filter(h => h.syncGit).length;
  return (
    <div className="gc-hosts">
      <div className="gc-hosts-head">
        <span className="pv-k">机器</span>
        {list.length > 0 && (
          <span className="gc-count">
            {on}/{list.length} 台同步
          </span>
        )}
      </div>
      {list.length ? (
        <div className="gc-rows">
          {list.map(h => (
            <GitHostRow key={h.id} h={h} now={now} />
          ))}
        </div>
      ) : (
        <span className="pv-none">还没有会同步 Git 凭据的机器</span>
      )}
      <p className="gc-note">
        <Icon name="info" />
        <span>新连上的机器默认同步。有人在用的电脑（例如你自己的主力机）在这里关掉：Board 不碰它仓库的 git 配置，之前配进去的会去掉。</span>
      </p>
    </div>
  );
}

function GitHostRow({ h, now }: { h: HostInfo; now: number }) {
  const { save, sync } = useSaver(h.name);
  const row = gitRow(h, now);
  const v: OnOff = h.syncGit ? 'on' : 'off';
  return (
    <div className={cx('gc-row', !h.syncGit && 'is-off', sync.syncGit === 'failed' && 'hp-rollback')}>
      <OsGlyph os={h.os} small />
      <Tip text="打开这台机器">
        <button type="button" className="gc-name" onClick={() => openHost(h.name)}>
          {h.name}
        </button>
      </Tip>
      <span className={cx('gc-state', `k-${row.kind}`)}>
        <i className="gc-mark" aria-hidden="true" />
        <span>{row.text}</span>
      </span>
      <span className="hp-ctl">
        <Sync s={sync.syncGit} />
        <Segmented<OnOff>
          value={v}
          options={[
            ['on', '同步'],
            ['off', '不同步'],
          ]}
          onChange={x => x !== v && save({ syncGit: x === 'on' })}
        />
      </span>
    </div>
  );
}

/** 设置 / 修改：居中弹窗，和供应商、MCP 同一种表单。点保存先在界面上校验 */
function GitForm({
  value: d,
  editing,
  tail,
  busy,
  onChange,
  onSave,
  onCancel,
}: {
  value: GitDraft;
  editing: boolean;
  tail: string | null;
  busy: boolean;
  onChange: (d: GitDraft) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  const [tried, setTried] = useState(false);
  const why = gitCredError(d, editing);
  return (
    <FormModal
      title={editing ? '修改 Git 凭据' : '设置 Git 凭据'}
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
          note="保存后用服务器的机器马上配进仓库"
        />
      }
    >
      <FormSec title="远端" hint="只管 https 地址；SSH 地址（git@…）的仓库照旧用本机的 key">
        <div className="fm-fields">
          <FormField label="主机" hint="https 远端的主机名，例如 github.com；可以带 :端口">
            <input className="input mono" value={d.host} placeholder="github.com" spellCheck={false} autoComplete="off" onChange={e => onChange({ ...d, host: e.target.value })} />
          </FormField>
          <FormField label="用户名" hint="令牌所属的账号">
            <input className="input mono" value={d.username} placeholder="octocat" spellCheck={false} autoComplete="off" onChange={e => onChange({ ...d, username: e.target.value })} />
          </FormField>
        </div>
      </FormSec>
      <FormSec title="令牌" hint="只存在服务端，界面上只显示末尾 4 位">
        <div className="fm-fields">
          <FormField label="令牌" hint="要有推送权限（GitHub 的 fine-grained 令牌给 Contents 读写）">
            <input
              className="input mono"
              type="password"
              value={d.token}
              placeholder={editing ? `留空保留原来的${tail ? `（••••${tail}）` : ''}` : 'ghp_… / github_pat_…'}
              spellCheck={false}
              autoComplete="new-password"
              onChange={e => onChange({ ...d, token: e.target.value })}
            />
          </FormField>
        </div>
      </FormSec>
    </FormModal>
  );
}
