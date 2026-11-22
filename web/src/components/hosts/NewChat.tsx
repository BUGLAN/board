/* ==========================================================================
   机器详情「对话」页签右栏的「新对话」（PRD §9.1「机器对话」）
   - 上面选用什么跑：有角色时列角色（这台机器跑不了的禁用、写原因），没有角色时列这台机器上找得到的 agent
   - 下面写第一条消息：原样发给 agent，前后不加说明
   - 「开始」要等服务端返回才有对话：按钮转圈、禁用到返回；成功后右栏换成这个对话。失败时提示原因，写的消息留着
   ========================================================================== */
import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { fitOf, profilesOf, short } from '../../lib/profiles';
import type { HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { toastError } from '../../store/ui';
import { Icon } from '../Icon';
import { colorOf } from '../profiles/bits';
import { AgentIcon } from './bits';

type Pick = { profile: string } | { agent: string };

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
      const c = await optimistic(null, () => api.startChat(h.name, cur.pick, text.trim()));
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
          {!h.online ? '机器离线' : !first ? '这台机器跑不了任何角色' : `在 ${h.name} 的主目录里跑 · 权限请求自动允许 · 不占会话位`}
        </span>
        <span className="nc-key">⌘ Enter</span>
        <button className={cx('btn sm brand', busy && 'is-busy')} type="button" disabled={busy || !canStart} onClick={() => void start()}>
          开始
        </button>
      </div>
    </div>
  );
}
