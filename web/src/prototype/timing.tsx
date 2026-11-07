/* ==========================================================================
   原型（PROTOTYPE，用完即弃，不进 main）：心跳计时怎么显示
   问题：去掉 30 分钟倒计时后，进行中 / 待审核卡片、卡片详情、动态、顶栏、侧栏
         怎么显示「本次耗时」和「累计耗时（多次领取）」。
   三个变体，在现有白板页上用 ?variant=A|B|C 切换（底部悬浮条，← → 也可以）：
     A 计时胶囊：保留卡片底部一行，右侧是跳动的计时胶囊，下方小字累计
     B 分段时间条：每次领取是一段，卡片底部一条分段条；详情是逐次的时间轴
     C 极简徽章：去掉底部一行，持有者和耗时并进卡片头部；详情、动态只用文字
   数据：本次 = 现在 − 领取时间（真实）；之前几次是按卡片 ID 编出来的假数据。
   ========================================================================== */
import { useEffect, useSyncExternalStore } from 'react';
import type { Activity, Card } from '../lib/types';
import { agentColor, cx, hhmm } from '../lib/util';
import { Avatar, agent } from '../components/bits';
import './timing.css';

/* ---------------- 变体开关（只在开发环境） ---------------- */

export type Variant = 'A' | 'B' | 'C';
const VARIANTS: { key: Variant; name: string }[] = [
  { key: 'A', name: '计时胶囊' },
  { key: 'B', name: '分段时间条' },
  { key: 'C', name: '极简徽章' },
];
const read = (): Variant | null => {
  if (!import.meta.env.DEV) return null;
  const v = new URLSearchParams(location.search).get('variant');
  return v === 'A' || v === 'B' || v === 'C' ? v : null;
};
let current = read();
const subs = new Set<() => void>();
function setVariant(v: Variant) {
  const q = new URLSearchParams(location.search);
  q.set('variant', v);
  history.replaceState(null, '', `${location.pathname}?${q}`);
  current = v;
  subs.forEach(f => f());
}
export const useVariant = () =>
  useSyncExternalStore(
    f => (subs.add(f), () => subs.delete(f)),
    () => current,
  );

/* ---------------- 假数据：每次领取（一段心跳） ---------------- */

export interface Session {
  n: number;
  agent: string;
  start: number;
  end: number | null; // null = 进行中
  how: 'final' | 'lost' | null;
}
const MIN = 60e3;
const AGENTS = ['claude', 'deepseek', 'codex'];

export function sessionsOf(c: Card, now: number): Session[] {
  const s = c.id;
  if (c.column === 'inbox') return [];
  // 之前几次：进行中的卡片 0–2 次；其他状态里的卡片：待办里只有一部分做过（被打回），其余状态都做过 1–3 次
  const isTodo = c.column === 'todo';
  let past = c.holder ? s % 3 : isTodo ? (s % 4 === 0 ? 1 : 0) : 1 + (s % 3);
  if (c.column === 'done') past = 1 + (s % 2);
  const out: Session[] = [];
  let end = c.holder ? (c.claimedAt ?? now - 5 * MIN) - 40 * MIN : c.updatedAt;
  for (let k = past; k >= 1; k--) {
    const dur = (6 + ((s * 7 + k * 11) % 31)) * MIN + ((s * 13 + k * 17) % 60) * 1000;
    const lost = (s + k) % 3 === 0 && k < past; // 最后一次一般是正常结束（交审）
    out.unshift({ n: k, agent: c.holder ?? AGENTS[(s + k) % 3], start: end - dur, end, how: lost ? 'lost' : 'final' });
    end = end - dur - (25 + ((s * 3 + k) % 40)) * MIN;
  }
  if (c.holder) out.push({ n: past + 1, agent: c.holder, start: c.claimedAt ?? now - 5 * MIN, end: null, how: null });
  return out;
}
const durOf = (x: Session, now: number) => (x.end ?? now) - x.start;
export const totalOf = (xs: Session[], now: number) => xs.reduce((t, x) => t + durOf(x, now), 0);

/** 计时器格式：12:34 / 1:02:03 */
export function clock(ms: number) {
  const t = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), sec = t % 60;
  const p = (n: number) => String(n).padStart(2, '0');
  return h ? `${h}:${p(m)}:${p(sec)}` : `${p(m)}:${p(sec)}`;
}
/** 文字格式：23 分 10 秒 / 1 小时 5 分 */
export function words(ms: number) {
  const t = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), sec = t % 60;
  if (h) return `${h} 小时 ${m} 分`;
  if (m) return `${m} 分 ${sec} 秒`;
  return `${sec} 秒`;
}
/** 简写：41m / 1h05m */
const short = (ms: number) => {
  const m = Math.round(ms / MIN);
  return m >= 60 ? `${Math.floor(m / 60)}h${String(m % 60).padStart(2, '0')}m` : `${m}m`;
};

/* ---------------- 卡片（白板上） ---------------- */

/** 卡片底部（A、B）。C 不用底部，返回 null */
export function CardFoot({ v, card: c, now }: { v: Variant; card: Card; now: number }) {
  const xs = sessionsOf(c, now);
  const cur = xs.find(x => x.end == null);
  const total = totalOf(xs, now);
  if (!xs.length || v === 'C') return null;
  if (v === 'A') {
    if (cur)
      return (
        <div className="card-claim pt-a">
          <Avatar actor={agent(cur.agent)} />
          <span className="claim-name">{cur.agent}</span>
          <span className="pt-pill live" title="本次耗时（从第一次心跳算起）">
            <i className="pt-pulse" />
            {clock(now - cur.start)}
          </span>
          {xs.length > 1 && (
            <span className="pt-sub">
              累计 {clock(total)} · 第 {cur.n} 次
            </span>
          )}
        </div>
      );
    return (
      <div className="card-claim pt-a done">
        <span className="pt-pill" title={xs.map(x => `第 ${x.n} 次 ${x.agent} ${words(durOf(x, now))}`).join('\n')}>
          ⏱ 累计 {words(total)}
        </span>
        <span className="pt-sub">{xs.length} 次</span>
      </div>
    );
  }
  // B：分段条，宽度按耗时比例
  return (
    <div className="pt-b">
      <div className="pt-seg">
        {xs.map(x => (
          <i
            key={x.n}
            className={cx(x.end == null && 'live', x.how === 'lost' && 'lost')}
            style={{ flexGrow: Math.max(durOf(x, now), 60e3), ['--ac' as string]: agentColor(x.agent) }}
            title={`第 ${x.n} 次 · ${x.agent} · ${words(durOf(x, now))}${x.how === 'lost' ? ' · 异常退出' : ''}`}
          />
        ))}
      </div>
      <div className="pt-b-text">
        {cur ? (
          <>
            <Avatar actor={agent(cur.agent)} />
            <span className="claim-name">{cur.agent}</span>
            <span className="pt-b-now">本次 {clock(now - cur.start)}</span>
          </>
        ) : (
          <span className="pt-b-now muted">{xs.length} 次</span>
        )}
        <span className="pt-b-total">{xs.length > 1 || !cur ? `累计 ${clock(total)}` : ''}</span>
      </div>
    </div>
  );
}

/** C：卡片头部右侧的徽章（替代底部一行） */
export function HeadBadge({ card: c, now }: { card: Card; now: number }) {
  const xs = sessionsOf(c, now);
  if (!xs.length) return null;
  const cur = xs.find(x => x.end == null);
  const total = totalOf(xs, now);
  if (cur)
    return (
      <span className="pt-c-badge live" style={{ ['--ac' as string]: agentColor(cur.agent) }} title={`${cur.agent} 正在做 · 本次 ${words(now - cur.start)} · 累计 ${words(total)}（${xs.length} 次）`}>
        <Avatar actor={agent(cur.agent)} />
        {clock(now - cur.start)}
      </span>
    );
  return (
    <span className="pt-c-badge" title={xs.map(x => `第 ${x.n} 次 ${x.agent} ${words(durOf(x, now))}`).join('\n')}>
      ⏱ {short(total)}
    </span>
  );
}

/* ---------------- 卡片详情：耗时区 ---------------- */

export function DetailTiming({ v, card: c, now, release }: { v: Variant; card: Card; now: number; release: React.ReactNode }) {
  const xs = sessionsOf(c, now);
  if (!xs.length) return null;
  const cur = xs.find(x => x.end == null);
  const total = totalOf(xs, now);
  const lost = xs.filter(x => x.how === 'lost').length;

  if (v === 'A')
    return (
      <div className={cx('claim-card pt-da', !cur && 'idle')} style={{ ['--ac' as string]: agentColor(cur?.agent ?? xs[xs.length - 1].agent) }}>
        {cur ? <Avatar actor={agent(cur.agent)} className="lg" /> : <span className="pt-da-ic">⏱</span>}
        <div className="claim-who">
          <b>{cur ? cur.agent : '耗时'}</b>
          <span>
            {cur ? (
              <>
                第一次心跳 {hhmm(cur.start)} · <span className="pt-pulse" /> 心跳正常
              </>
            ) : (
              `最后一次结束于 ${hhmm(xs[xs.length - 1].end!)}`
            )}
          </span>
        </div>
        <div className="pt-da-big">
          <span>{cur ? '本次' : '累计'}</span>
          {clock(cur ? now - cur.start : total)}
        </div>
        <div className="pt-da-stats">
          <div>
            <span>累计</span>
            <b>{clock(total)}</b>
          </div>
          <div>
            <span>领取次数</span>
            <b>{xs.length}</b>
          </div>
          <div>
            <span>异常退出</span>
            <b className={cx(lost > 0 && 'bad')}>{lost}</b>
          </div>
        </div>
        {cur && release}
      </div>
    );

  if (v === 'B') {
    const t0 = xs[0].start, t1 = cur ? now : xs[xs.length - 1].end!;
    const span = Math.max(t1 - t0, 1);
    return (
      <div className="pt-db">
        <div className="pt-db-head">
          <b>耗时</b>
          <span>
            累计 <b className="mono">{clock(total)}</b> · {xs.length} 次
          </span>
        </div>
        {xs.map(x => (
          <div key={x.n} className={cx('pt-db-row', x.end == null && 'live', x.how === 'lost' && 'lost')} style={{ ['--ac' as string]: agentColor(x.agent) }}>
            <span className="n">#{x.n}</span>
            <span className="who mono">{x.agent}</span>
            <span className="track">
              <i style={{ left: `${((x.start - t0) / span) * 100}%`, width: `${Math.max(((durOf(x, now)) / span) * 100, 1.5)}%` }} />
            </span>
            <span className="when mono">
              {hhmm(x.start)}–{x.end ? hhmm(x.end) : '现在'}
            </span>
            <span className="dur mono">{clock(durOf(x, now))}</span>
            <span className="how">{x.end == null ? '进行中' : x.how === 'lost' ? '异常退出' : '正常结束'}</span>
          </div>
        ))}
        {cur && <div className="pt-db-foot">{release}</div>}
      </div>
    );
  }

  // C：一行文字
  return (
    <div className="pt-dc">
      {cur ? (
        <>
          <Avatar actor={agent(cur.agent)} />
          <span>
            <b className="mono" style={{ color: agentColor(cur.agent) }}>
              {cur.agent}
            </b>{' '}
            正在做 · 本次 <b className="mono">{clock(now - cur.start)}</b>
            {xs.length > 1 && (
              <>
                {' '}
                · 累计 <b className="mono">{clock(total)}</b>（{xs.length} 次）
              </>
            )}
          </span>
        </>
      ) : (
        <span>
          ⏱ 累计耗时 <b className="mono">{words(total)}</b>，共 {xs.length} 次{lost ? `，其中 ${lost} 次异常退出` : ''}
        </span>
      )}
      {cur && <span className="pt-dc-act">{release}</span>}
    </div>
  );
}

/* ---------------- 动态：每次的开始 / 结束 ---------------- */

export type ProtoActivity = Activity & { session?: Session; edge?: 'start' | 'end' };

export function withSessions(c: Card, activity: Activity[], now: number): ProtoActivity[] {
  const xs = sessionsOf(c, now);
  const fake: ProtoActivity[] = [];
  let id = -1;
  for (const x of xs) {
    fake.push({ id: id--, at: x.start, actor: agent(x.agent), kind: 'system', action: 'beat', text: '', session: x, edge: 'start' });
    if (x.end)
      fake.push({
        id: id--, at: x.end, actor: x.how === 'lost' ? { kind: 'system', name: '' } : agent(x.agent), kind: 'system',
        action: x.how === 'lost' ? 'lost' : 'beat', text: '', session: x, edge: 'end',
      });
  }
  // 真实动态里的心跳记录去掉，用假数据代替，避免重复
  return [...activity.filter(a => a.action !== 'beat' && a.action !== 'lost'), ...fake].sort((a, b) => a.at - b.at);
}

/** 返回 null 表示这一条用原来的样子渲染 */
export function SessionLine({ v, a, now, name, time }: { v: Variant; a: ProtoActivity; now: number; name: React.ReactNode; time: React.ReactNode }) {
  const x = a.session;
  if (!x) return null;
  const d = durOf(x, now);
  if (a.edge === 'start') {
    if (v === 'B') return <></>; // B：开始和结束合成一块，只在结束（或进行中）时显示
    return (
      <div className="tl-item">
        <div className="tl-icon k-renew">▶</div>
        <div className="tl-sys">
          {name} 开始第 {x.n} 次（第一次心跳）{time}
        </div>
      </div>
    );
  }
  if (v === 'A')
    return (
      <div className="tl-item">
        <div className={cx('tl-icon', x.how === 'lost' ? 'k-lost' : 'k-renew')}>{x.how === 'lost' ? '!' : '■'}</div>
        <div className="tl-sys">
          {x.how === 'lost' ? (
            <>
              <b>系统</b> 连续 5 次没收到 {x.agent} 的心跳，判定异常退出
            </>
          ) : (
            <>
              {name} 发送 final，正常退出
            </>
          )}{' '}
          {time}
          <span className={cx('pt-act-chip', x.how === 'lost' && 'bad')}>
            第 {x.n} 次 · {x.how === 'lost' ? '约 ' : ''}
            {clock(d)}
          </span>
        </div>
      </div>
    );
  if (v === 'B')
    return (
      <div className={cx('pt-act-block', x.how === 'lost' && 'lost')} style={{ ['--ac' as string]: agentColor(x.agent) }}>
        <div className="top">
          <b>第 {x.n} 次</b> <span className="mono">{x.agent}</span>
          <span className="mono when">
            {hhmm(x.start)} → {hhmm(x.end!)}
          </span>
          <span className="mono dur">{clock(d)}</span>
        </div>
        <div className="bar">
          <i style={{ width: `${Math.min(100, (d / (45 * MIN)) * 100)}%` }} />
        </div>
        <div className="how">{x.how === 'lost' ? '心跳中断，判定异常退出，卡片回到待办' : '正常退出（final）'}</div>
      </div>
    );
  // C
  return (
    <div className="tl-item">
      <div className={cx('tl-icon', x.how === 'lost' ? 'k-lost' : 'k-renew')}>{x.how === 'lost' ? '!' : '■'}</div>
      <div className="tl-sys">
        {x.how === 'lost' ? <b>系统</b> : name} {x.how === 'lost' ? `判定 ${x.agent} 异常退出` : '结束了心跳，正常退出'}（耗时{x.how === 'lost' ? '约' : ''} {words(d)}）{time}
      </div>
    </div>
  );
}

/** B：进行中的那一次在动态末尾显示一块「进行中」 */
export function LiveBlock({ v, card: c, now }: { v: Variant; card: Card; now: number }) {
  if (v !== 'B') return null;
  const cur = sessionsOf(c, now).find(x => x.end == null);
  if (!cur) return null;
  return (
    <div className="pt-act-block live" style={{ ['--ac' as string]: agentColor(cur.agent) }}>
      <div className="top">
        <b>第 {cur.n} 次</b> <span className="mono">{cur.agent}</span>
        <span className="mono when">{hhmm(cur.start)} → 现在</span>
        <span className="mono dur">{clock(now - cur.start)}</span>
      </div>
      <div className="bar">
        <i style={{ width: `${Math.min(100, ((now - cur.start) / (45 * MIN)) * 100)}%` }} />
      </div>
      <div className="how">
        <i className="pt-pulse" /> 进行中 · 心跳正常
      </div>
    </div>
  );
}

/* ---------------- 顶栏头像 / 侧栏「正在工作」 ---------------- */

export function PresenceItem({ v, card: c, now }: { v: Variant; card: Card; now: number }) {
  const xs = sessionsOf(c, now);
  const cur = xs.find(x => x.end == null);
  if (!cur) return null;
  const tip = `${cur.agent} 正在做 #${c.id} · 本次 ${clock(now - cur.start)} · 累计 ${clock(totalOf(xs, now))}`;
  return (
    <span className={cx('pt-pres', `v${v}`)} style={{ ['--c' as string]: agentColor(cur.agent) }} data-tip={tip} data-tip-align="end">
      <Avatar actor={agent(cur.agent)} />
      {v === 'A' && <i className="pt-pres-ring" />}
      {v === 'B' && <span className="pt-pres-t">{clock(now - cur.start)}</span>}
    </span>
  );
}

export function WorkingTime({ v, card: c, now }: { v: Variant; card: Card; now: number }) {
  const xs = sessionsOf(c, now);
  const cur = xs.find(x => x.end == null);
  if (!cur) return null;
  const total = totalOf(xs, now);
  if (v === 'A')
    return (
      <span className="tm pt-wt">
        <i className="pt-pulse" />
        {clock(now - cur.start)}
      </span>
    );
  if (v === 'B')
    return (
      <span className="tm pt-wt b">
        <span className="pt-seg mini">
          {xs.map(x => (
            <i key={x.n} className={cx(x.end == null && 'live')} style={{ flexGrow: Math.max(durOf(x, now), 60e3), ['--ac' as string]: agentColor(x.agent) }} />
          ))}
        </span>
        {clock(total)}
      </span>
    );
  return (
    <span className="tm pt-wt">
      {clock(now - cur.start)}
      {xs.length > 1 && <em> / {clock(total)}</em>}
    </span>
  );
}

/* ---------------- 底部悬浮切换条 ---------------- */

export function PrototypeSwitcher() {
  const v = useVariant();
  const i = Math.max(0, VARIANTS.findIndex(x => x.key === v));
  const go = (d: number) => setVariant(VARIANTS[(i + d + VARIANTS.length) % VARIANTS.length].key);
  useEffect(() => {
    if (!v) return;
    const key = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.closest('input, textarea, [contenteditable]')) return;
      if (e.key === 'ArrowLeft') go(-1);
      if (e.key === 'ArrowRight') go(1);
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });
  if (!v) return null;
  return (
    <div className="pt-switcher" role="toolbar" aria-label="原型变体">
      <button type="button" onClick={() => go(-1)} aria-label="上一个变体">
        ‹
      </button>
      <span>
        原型 · <b>{v}</b> {VARIANTS[i].name}
      </span>
      <button type="button" onClick={() => go(1)} aria-label="下一个变体">
        ›
      </button>
    </div>
  );
}
