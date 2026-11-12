/* ==========================================================================
   机器详情「自检」（PRD §10.12「自检」）：在界面上确认这台机器真的能干活，不用拿真卡片试错
   - 整台机器一个「自检」按钮；「本机 agent」每一行一个小按钮，只查那一个
   - board host 在后台做：问每个 agent 一句（带设置好的启动环境、模型、强度，最长 3 分钟）、
     每块白板的目录 git fetch 和 push --dry-run；服务端补上 board 版本是否落后
   - 会消耗 agent 的额度：只在点按钮时做，不定时做；同一台机器同时只做一次
   - 结果逐项写通过或没通过、原因、检查时间，一直保留到下一次自检
   请求要等服务端记下才算数：按钮进行中（转圈、禁用）直到返回（optimistic(null, …)）；进展按服务端推回来的 selftest 画
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { selftestBusy, selftestRows, selftestState, since } from '../../lib/hosts';
import type { HostAgent, HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic } from '../../store/data';
import { toast, toastError } from '../../store/ui';
import { Icon } from '../Icon';
import { useNow } from '../bits';

/** 发自检请求；返回进行中的状态和发请求的函数（整机、某个 agent 共用一个，免得同时点两个） */
function useSelfTest(h: HostInfo) {
  const [busy, setBusy] = useState<string | null>(null);
  const run = async (agent?: HostAgent) => {
    if (busy) return;
    setBusy(agent?.id ?? '*');
    try {
      await optimistic(null, () => api.selfTest(h.name, agent?.id));
      toast({ type: 'info', title: agent ? `${h.name} 正在自检 ${agent.name}` : `${h.name} 正在自检`, text: '会消耗 agent 的额度，最长 3 分钟', duration: 2400 });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  return { busy, run };
}

const when = (at: number) => new Date(at).toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });

export function SelfTestSection({ h, server, i }: { h: HostInfo; server: string | null; i: number }) {
  const now = useNow(5000);
  const { busy, run } = useSelfTest(h);
  // 旧版服务端没有自检：不显示这一节
  if (h.selftest === undefined) return null;
  const state = selftestState(h, now);
  const pending = selftestBusy(h, now);
  const rows = selftestRows(h);
  const bad = rows.filter(r => !r.ok).length;
  const last = rows.reduce((m, r) => Math.max(m, r.at), 0);
  const old = !h.selftest.able;
  const tip = old
    ? `这台机器的 board（${h.version || '旧版'}）还不会自检：先升级到 ${server ?? '服务端的版本'}`
    : !h.online
      ? '机器离线'
      : pending
        ? '正在自检，等它做完'
        : '问每个 agent 一句、检查每块白板能不能推送、board 版本是否落后；会消耗 agent 的额度';
  return (
    <section className="d-section hp-selftest" style={{ ['--i' as string]: i }}>
      <div className="d-label">
        自检
        {rows.length > 0 && (
          <span className={cx('hp-ready-n', bad ? 'bad' : 'ok')}>{bad ? `${bad} 项没通过` : `${rows.length} 项都通过`}</span>
        )}
        {last > 0 && <span className="act">{when(last)} 检查</span>}
      </div>
      <div className="hp-manage-acts">
        <span data-tip={tip}>
          <button className={cx('btn sm', busy === '*' && 'is-busy')} type="button" disabled={!!busy || pending || !h.online || old} aria-busy={busy === '*' || undefined} onClick={() => void run()}>
            <Icon name="check" />
            自检
          </button>
        </span>
        {old ? (
          <span className="hp-note hp-st-none">{tip}</span>
        ) : (
          !rows.length && !state && <span className="hp-note hp-st-none">还没有自检过：加完一台机器先点一下，确认它真的能干活</span>
        )}
      </div>
      {state && (
        <div className={cx('hp-ask', state.tone)} role="status">
          {state.tone === 'busy' && <i className="hp-ask-spin" />}
          <span>
            {state.text}
            {h.selftest.ask && (
              <em>
                {' '}
                · {h.selftest.ask.by} {since(h.selftest.ask.at, now)}请求
              </em>
            )}
          </span>
        </div>
      )}
      {rows.length > 0 && (
        <ul className="hp-st-list">
          {rows.map(r => (
            <li key={`${r.kind}:${r.key}`} className={cx(r.ok ? 'ok' : 'bad')}>
              <Icon name={r.ok ? 'check' : 'alert'} />
              <div>
                <b>{r.label}</b>
                {r.why && <span className="hp-st-why">{r.why}</span>}
                {r.detail && <pre className="hp-st-detail">{r.detail}</pre>}
              </div>
              <em className="hp-st-at" title={new Date(r.at).toLocaleString('zh-CN')}>
                {since(r.at, now)}
              </em>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** 「本机 agent」一行里的小按钮：只自检这一个 agent */
export function SelfTestAgentButton({ h, a }: { h: HostInfo; a: HostAgent }) {
  const now = useNow(5000);
  const { busy, run } = useSelfTest(h);
  if (h.selftest === undefined || !h.selftest.able || !a.found) return null;
  const pending = selftestBusy(h, now);
  const mine = pending && h.selftest.ask?.agent === a.id;
  const res = h.selftest.items.find(x => x.kind === 'agent' && x.key === a.id);
  const tip = !h.online
    ? '机器离线'
    : pending
      ? '正在自检，等它做完'
      : res
        ? `${res.ok ? '上次自检通过' : `上次自检没通过：${res.why ?? ''}`}（${since(res.at, now)}）；点一下用设置好的启动环境、模型、强度问它一句`
        : '用设置好的启动环境、模型、强度问它一句，确认它登录了、认得 board-skills';
  return (
    <span data-tip={tip} data-tip-align="end">
      <button
        className={cx('hp-st-btn', (busy === a.id || mine) && 'is-busy', res && (res.ok ? 'ok' : 'bad'))}
        type="button"
        aria-label={`自检 ${a.name}`}
        disabled={!!busy || pending || !h.online}
        onClick={() => void run(a)}
      >
        <Icon name={res && !res.ok ? 'alert' : 'check'} />
        自检
      </button>
    </span>
  );
}
