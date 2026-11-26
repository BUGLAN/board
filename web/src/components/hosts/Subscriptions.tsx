/* ==========================================================================
   「Agent → 模型供应商」页的「官方订阅」一节：一个账号（agent + 邮箱，#669）一张卡
   - 5 小时、本周两个窗口：用了百分之几、什么时候重置；条上的竖线是这个窗口的时间过去了多少，用量跑在前面很多就是用得快
   - 额度从哪来：跑卡时 agent 推的（board host 随上报带上，差不多实时），或者点「刷新」（让一台登着这个账号的机器
     发本地命令 /usage、/status，不调模型、不花额度）
   - 额度用完：同一账号的机器一起暂停，点「恢复」一起恢复
   - 下面两行：登着这个账号的机器、用它的角色（同一个 agent 的「官方订阅」「本机配置」）
   数据见 lib/subscriptions.ts
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { hostsOf, whenWords } from '../../lib/hosts';
import { accountsOf, elapsedOf, toneOf, WIN_LABEL, type QuotaWin, type SubAccount } from '../../lib/subscriptions';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { openHost, setTab, toastError } from '../../store/ui';
import { goSec } from '../agent/AgentNav';
import { Tip, useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon } from './bits';
import { useSaver } from './Saver';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/** 「1 小时 50 分后」「3 天 4 小时后」 */
function inWords(t: number, now: number) {
  const d = Math.max(0, t - now);
  if (d < HOUR) return `${Math.max(1, Math.round(d / MIN))} 分钟后`;
  if (d < DAY) {
    const h = Math.floor(d / HOUR);
    const m = Math.round((d % HOUR) / MIN);
    return m ? `${h} 小时 ${m} 分后` : `${h} 小时后`;
  }
  const days = Math.floor(d / DAY);
  const h = Math.round((d % DAY) / HOUR);
  return h ? `${days} 天 ${h} 小时后` : `${days} 天后`;
}

const ago = (t: number, now: number) => {
  const d = now - t;
  if (d < MIN) return '刚刚';
  if (d < HOUR) return `${Math.round(d / MIN)} 分钟前`;
  if (d < DAY) return `${Math.round(d / HOUR)} 小时前`;
  return `${Math.round(d / DAY)} 天前`;
};

function tipOf(w: QuotaWin, now: number) {
  if (w.used == null) return '还没看到：用这个账号跑一张卡，或点刷新';
  if (w.reset) return '看到之后已经过了重置时间：点刷新看现在的';
  const e = elapsedOf(w, now);
  const t = toneOf(w, now);
  const pace = t === 'out' ? '已经用完' : t === 'fast' ? '用得比时间快，可能撑不到重置' : t === 'high' ? '快用完了' : '按这个速度够用';
  return `用了 ${w.used}%${e != null ? `，窗口过去了 ${Math.round(e * 100)}%` : ''}：${pace}`;
}

function Meter({ w, now }: { w: QuotaWin; now: number }) {
  const e = elapsedOf(w, now);
  const tone = toneOf(w, now);
  const foot = w.reset ? '已重置' : w.resetsAt ? `${whenWords(w.resetsAt, now)} 重置 · ${inWords(w.resetsAt, now)}` : ' ';
  return (
    <div className="sq-meter">
      <div className="sq-m-top">
        <span className="sq-m-k">{WIN_LABEL[w.key]}</span>
        <b className={cx('sq-pct', `q-${tone}`)}>{w.used == null ? '—' : `${w.used}%`}</b>
      </div>
      <Tip text={tipOf(w, now)}>
        <span className={cx('sq-bar', `q-${tone}`)}>
          <i style={{ width: `${Math.min(100, w.used ?? 0)}%` }} />
          {e != null && <em style={{ left: `${e * 100}%` }} />}
        </span>
      </Tip>
      <span className="sq-m-reset">{foot}</span>
    </div>
  );
}

/** 用这个账号的角色：同一个 agent 的「官方订阅」「本机配置」 */
function Roles({ agent }: { agent: string }) {
  const roles = (useData(s => s.ws?.sched?.profiles) ?? []).filter(r => r.agent === agent && (r.provider === '' || r.provider === '@subscription'));
  if (!roles.length) return <span className="pv-none">没有角色用它</span>;
  return (
    <span className="pv-ags">
      {roles.map(r => {
        const src = r.provider === '@subscription' ? '官方订阅' : '本机配置';
        return (
          <Tip key={r.id} text={`角色「${r.name}」：${src}`}>
            <button type="button" className="pv-ag" onClick={() => (setTab('agent'), goSec('rules'))}>
              <AgentIcon id={r.agent} />
              <b>{r.name}</b>
              <span className="pv-ag-n">{src}</span>
            </button>
          </Tip>
        );
      })}
    </span>
  );
}

/** 刷新：让一台在线的机器查一次；查的时候转圈（服务端返回之后到查到之前都算） */
function Refresh({ a }: { a: SubAccount }) {
  const [sending, setSending] = useState(false);
  const busy = sending || a.asking;
  const tip = a.probeWhy ?? (a.fail ? `上次没查到：${a.fail}` : `让 ${a.probeHost} 查一次（不花额度）`);
  const run = async () => {
    if (!a.probeHost || busy) return;
    setSending(true);
    try {
      await optimistic(null, () => api.quotaAsk(a.probeHost!, a.agent));
    } catch (e) {
      toastError(e);
    } finally {
      setSending(false);
    }
  };
  return (
    <Tip text={tip}>
      <button
        type="button"
        className={cx('icon-btn sm', busy && 'is-busy', a.fail && !busy && 'sq-failed')}
        aria-label="刷新额度"
        data-tip-align="end"
        disabled={!a.probeHost || busy}
        onClick={() => void run()}
      >
        <Icon name={a.fail && !busy ? 'alert' : 'renew'} />
      </button>
    </Tip>
  );
}

/** 额度用完、暂停着：恢复发给暂停着的一台机器，服务端把同一账号的一起恢复 */
function Paused({ a, now }: { a: SubAccount; now: number }) {
  const { sync, save } = useSaver(a.pausedHost ?? '');
  if (!a.pausedUntil || !a.pausedHost) return null;
  return (
    <div className="sq-paused">
      <Icon name="pause" />
      <span>
        额度用完，<b>{whenWords(a.pausedUntil, now)}</b> 恢复
      </span>
      <button className={cx('btn sm', sync[`resume:${a.agent}`] === 'saving' && 'is-busy')} type="button" onClick={() => save({ resume: a.agent })}>
        恢复
      </button>
    </div>
  );
}

function Card({ a, now }: { a: SubAccount; now: number }) {
  return (
    <article className={cx('pv-card sq-card', !!a.pausedUntil && 'is-paused')}>
      <header className="pv-head sq-head">
        <span className="sq-who">
          <AgentIcon id={a.agent} />
          <b>{a.label}</b>
          <span className="sq-mail">{a.email}</span>
        </span>
        <span className="sq-seen">
          {a.asking ? '正在查…' : a.seenAt ? `${ago(a.seenAt, now)}看到` : '还没看到额度'}
          <Refresh a={a} />
        </span>
      </header>
      <Paused a={a} now={now} />
      <div className="sq-meters">
        {a.wins.map(w => (
          <Meter key={w.key} w={w} now={now} />
        ))}
      </div>
      <div className="pv-use sq-use">
        <span className="pv-k">机器</span>
        <div className="pv-use-body">
          <span className="sq-hosts">
            {a.hosts.map(h => (
              <button
                key={h.name}
                type="button"
                className={cx('sq-host', !h.online && 'off')}
                onClick={() => {
                  setTab('hosts');
                  openHost(h.name);
                }}
              >
                <i />
                {h.name}
              </button>
            ))}
          </span>
        </div>
      </div>
      <div className="pv-use sq-use">
        <span className="pv-k">在用</span>
        <div className="pv-use-body">
          <Roles agent={a.agent} />
        </div>
      </div>
    </article>
  );
}

export function SubscriptionsSection() {
  const ws = useData(s => s.ws);
  const now = useNow(30000);
  const list = accountsOf(hostsOf(ws), now);
  return (
    <section className="sc sq">
      <div className="sc-head">
        <div>
          <h2>官方订阅</h2>
        </div>
      </div>
      {list.length ? (
        <div className="sq-grid">
          {list.map(a => (
            <Card key={a.id} a={a} now={now} />
          ))}
        </div>
      ) : (
        <p className="hp-note">还没有机器报订阅账号：Claude Code、Codex 用订阅登录的机器连上后会出现在这里</p>
      )}
    </section>
  );
}
