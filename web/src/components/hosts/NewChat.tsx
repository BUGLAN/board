/* ==========================================================================
   机器详情「对话」页签右栏的「新对话」（PRD §9.1「机器对话」）
   - 最上面选在哪个目录里跑（#624）：主目录，或者这台机器配了的项目目录（目录不存在、还没检查的禁用写原因，不是 git 仓库的能选、标黄；
     没配工作空间时只有主目录；board 版本太旧时项目目录禁用）。请求只带项目标识，路径由服务端算；开始后不能换
   - 上面选用什么跑：有角色时列角色（这台机器跑不了的禁用、写原因），没有角色时列这台机器上找得到的 agent
   - 下面写第一条消息：原样发给 agent，前后不加说明
   - 底栏只写会在哪个路径里跑（主目录是 ~，太长从开头省略）
   - 「开始」要等服务端返回才有对话：按钮转圈、禁用到返回；成功后右栏换成这个对话。失败时提示原因，写的消息留着
   ========================================================================== */
import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { fitOf, profilesOf, short } from '../../lib/profiles';
import type { HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { toastError } from '../../store/ui';
import { Select } from '../bits';
import { versionLt } from './Manage';
import { Icon } from '../Icon';
import { colorOf } from '../profiles/bits';
import { AgentIcon } from './bits';

type Pick = { profile: string } | { agent: string };

/** 能选运行目录的 board 版本：旧版 board host 不认对话的 cwd，会照旧在主目录里跑 */
export const CHAT_DIR_SINCE = '0.1.53';

/** 运行目录的选项：主目录 + 这台机器配了的项目目录（dirs 按项目去重）。目录不存在、还没检查的禁用；不是 git 仓库的能选（#624） */
export function dirChoices(h: HostInfo, names: Map<string, string>) {
  const seen = new Map<string, { project: string; name: string; path: string; status: string; branch?: string | null }>();
  for (const d of h.dirs ?? []) {
    const project = d.board.split('/')[0];
    if (!project || seen.has(project)) continue;
    seen.set(project, { project, name: names.get(project) ?? project, path: d.path, status: d.status ?? (d.reported ? 'ok' : 'unknown'), branch: d.branch });
  }
  return [...seen.values()];
}

const DIR_WHY: Record<string, string> = { missing: '目录不存在', unknown: '还没检查', not_git: '不是 git 仓库' };
const OLD_HOST = 'board 版本太旧，先升级';

export function NewChatForm({ h, onStarted }: { h: HostInfo; onStarted: (id: number) => void }) {
  const ws = useData(s => s.ws);
  const profiles = profilesOf(ws);
  const agents = h.agents.filter(a => a.found);
  // 能选的：有角色时是角色（这台机器能跑的），没有角色时是 agent
  const choices = useMemo(
    () =>
      profiles.length
        ? profiles.map(p => ({ key: `p:${p.id}`, pick: { profile: p.id } as Pick, ...fitOf(h, p.id), p }))
        : agents.map(a => ({ key: `a:${a.id}`, pick: { agent: a.id } as Pick, ok: true, why: '', a })),
    [profiles, agents, h],
  );
  const first = choices.find(c => c.ok);
  const [sel, setSel] = useState<string>(first?.key ?? '');
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const names = useMemo(() => new Map((ws?.projects ?? []).map(p => [p.key, p.name])), [ws]);
  const dirs = useMemo(() => dirChoices(h, names), [h, names]);
  const [dir, setDir] = useState('');
  const curDir = dirs.find(d => d.project === dir);
  const noWs = !h.workspace?.path && !dirs.length;
  const old = !!h.version && versionLt(h.version, CHAT_DIR_SINCE);
  const box = useRef<HTMLTextAreaElement>(null);
  const cur = choices.find(c => c.key === sel && c.ok);
  const canStart = !!cur && !!text.trim() && h.online;

  useEffect(() => {
    box.current?.focus({ preventScroll: true });
  }, []);

  const start = async () => {
    if (!cur || !canStart || busy) return;
    setBusy(true);
    try {
      // 要服务端建好才有对话：等返回（按钮转圈）；返回后刷新工作区，左栏多一条，右栏换成它
      const c = await optimistic(null, () => api.startChat(h.name, cur.pick, text.trim(), curDir?.project));
      onStarted(c.id);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="nc">
      <div className="nc-body">
        <section className="fm-sec nc-dir">
          <h4>
            <span>
              在哪个目录里跑
              <small>开始后不能换，要换就新建对话</small>
            </span>
          </h4>
          <Select
            aria-label="运行目录"
            value={dir}
            disabled={busy}
            onChange={setDir}
            options={[
              { value: '', label: '主目录（~）' },
              ...dirs.map(d => ({
                value: d.project,
                label: `${d.name} · ${d.path}${old ? ` · ${OLD_HOST}` : DIR_WHY[d.status] ? ` · ${DIR_WHY[d.status]}` : ''}`,
                disabled: old || d.status === 'missing' || d.status === 'unknown',
              })),
            ]}
          />
          <p className={cx('nc-dir-note', curDir?.status === 'not_git' && 'warn')}>
            {noWs
              ? '这台机器没配工作空间：只能在主目录里跑'
              : old && dirs.length
                ? `${h.name} 的 ${OLD_HOST}，才能选项目目录`
              : curDir
                ? curDir.status === 'not_git'
                  ? '不是 git 仓库'
                  : curDir.branch
                    ? `git 仓库 · 分支 ${curDir.branch}`
                    : 'git 仓库'
                : `${h.name} 上当前用户的主目录`}
          </p>
        </section>
        <section className="fm-sec">
          <h4>
            <span>
              {profiles.length ? '用哪个角色' : '用哪个 agent'}
              <small>{profiles.length ? '模型、强度、供应商按角色带上' : '还没有角色：用这台机器上 agent 自己的设置'}</small>
            </span>
          </h4>
          {choices.length ? (
            <div className="nc-list" role="radiogroup">
              {choices.map(c => (
                <button
                  key={c.key}
                  type="button"
                  role="radio"
                  aria-checked={sel === c.key}
                  className={cx('fm-tile nc-tile', sel === c.key && c.ok && 'on')}
                  disabled={!c.ok || busy}
                  onClick={() => setSel(c.key)}
                >
                  <span className="fm-mark plain" style={{ ['--vc' as string]: 'p' in c ? colorOf(c.p) : undefined }}>
                    <AgentIcon id={'p' in c ? c.p.agent : c.a.id} />
                  </span>
                  <span>
                    <b>{'p' in c ? c.p.name : c.a.name}</b>
                    <small>{c.ok ? ('p' in c ? short(c.p) : c.a.version || '本机登录') : c.why}</small>
                  </span>
                </button>
              ))}
            </div>
          ) : (
            <p className="fm-empty">这台机器上没有找到 agent。</p>
          )}
        </section>
        <section className="fm-sec nc-msg">
          <h4>
            <span>
              第一条消息
              <small>原样发给 agent</small>
            </span>
          </h4>
          <textarea
            ref={box}
            className="input nc-text"
            value={text}
            placeholder="例如：看看主目录下哪些文件夹最大"
            disabled={busy || !h.online}
            onChange={e => setText(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                void start();
              }
            }}
          />
        </section>
      </div>
      <div className="nc-foot">
        <span className="nc-where">
          <Icon name="folder" />
          {!h.online ? (
            <span>机器离线</span>
          ) : !first ? (
            <span>这台机器跑不了任何角色</span>
          ) : (
            <span className="path" title={curDir?.path ?? '主目录'}>
              <bdi>{curDir?.path ?? '~'}</bdi>
            </span>
          )}
        </span>
        <span className="nc-key">⌘ Enter</span>
        <button className={cx('btn sm brand', busy && 'is-busy')} type="button" disabled={busy || !canStart} onClick={() => void start()}>
          开始
        </button>
      </div>
    </div>
  );
}
