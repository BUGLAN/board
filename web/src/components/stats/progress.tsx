/* 进步曲线：创建以来每个时刻各状态有几张卡（累积流图）。
   横轴是时间，竖轴是卡片张数，一张卡算 1；各状态按工作流顺序从上往下叠，终点状态垫在最下面，
   它的上沿就是「做完了多少」的那条线。数据是 GET /api/stats 的 progress（服务端从流转记录还原） */
import { useState, type KeyboardEvent } from 'react';
import { statusColor } from '../../lib/schema';
import type { StatsProgress } from '../../lib/types';
import { cx } from '../../lib/util';
import { fmtDay, fmtHour, fmtShortDay, fmtWeekday, useWidth } from './charts';

/** 图上的一次发版：全部项目时 project 是项目名（悬停里写出来），其他范围为空 */
export interface ProgressVersion {
  name: string;
  at: number;
  cards: number;
  project?: string;
}

/** x.y.0 这种升了大版本、小版本的：在图上画一条竖线、标出版本号 */
export const bigVersion = (name: string) => {
  const m = /^v?(\d+)\.(\d+)(?:\.(\d+))?/i.exec(name);
  return !!m && (m[3] == null || Number(m[3]) === 0);
};

const pad2 = (n: number) => String(n).padStart(2, '0');
const fmtHm = (t: number) => `${pad2(new Date(t).getHours())}:${pad2(new Date(t).getMinutes())}`;

const H = 3600_000;
const DAY = 24 * H;

/** 竖轴刻度：大约 4 格，步长取 1 / 2 / 2.5 / 5 × 10ⁿ 里的整数，至少 1 张 */
export function yTicks(v: number) {
  const raw = Math.max(1, v) / 4;
  const mag = 10 ** Math.floor(Math.log10(raw));
  // 卡片是整张的：步长要是整数（2.5 只在 25、250 这种时候用）
  const step = Math.max(1, ([1, 2, 2.5, 5, 10].find(m => m * mag >= raw && Number.isInteger(m * mag)) ?? 10) * mag);
  const top = Math.max(step, Math.ceil(v / step) * step);
  const out: number[] = [];
  for (let x = 0; x <= top + 1e-9; x += step) out.push(Math.round(x * 10) / 10);
  return out;
}

/** 横轴刻度：按天（跨度短时按小时）挑间隔，字不挤 */
function xTicks(t0: number, t1: number, px: number) {
  const span = t1 - t0;
  const out: { t: number; text: string; strong?: boolean }[] = [];
  if (span <= 2 * DAY) {
    const per = (px / span) * H;
    const k = [1, 2, 3, 6, 12].find(s => s * per >= 44) ?? 24;
    const d = new Date(t0);
    d.setMinutes(0, 0, 0);
    for (let x = d.getTime() + H; x < t1; x += H) {
      const hr = new Date(x).getHours();
      if (hr % k) continue;
      out.push(hr === 0 ? { t: x, text: fmtShortDay(x), strong: true } : { t: x, text: fmtHour(x) });
    }
    return out;
  }
  const per = (px / span) * DAY;
  const k = [1, 2, 3, 7, 14, 30].find(s => s * per >= 46) ?? 61;
  const d = new Date(t0);
  d.setHours(24, 0, 0, 0);
  let i = 0;
  for (let x = d.getTime(); x < t1; i++) {
    const day = new Date(x);
    const month = day.getDate() === 1;
    // 间隔一周以上时月初一定标，离月初太近的那一格让掉
    const near = k >= 7 && !month && (day.getDate() <= Math.ceil(k * 0.6) || day.getDate() > new Date(day.getFullYear(), day.getMonth() + 1, 0).getDate() - Math.ceil(k * 0.6));
    if ((k >= 7 && month) || (i % k === 0 && !near)) out.push({ t: x, text: fmtShortDay(x), strong: month });
    day.setDate(day.getDate() + 1);
    x = day.getTime();
  }
  return out;
}

export function ProgressChart({ p, versions = [], height = 276 }: { p: StatsProgress; versions?: ProgressVersion[]; height?: number }) {
  const [ref, W] = useWidth<HTMLDivElement>();
  // 悬停：图里是某个采样时刻（hi），版本那一行是某次发版（hv）
  const [hi, setHi] = useState<number | null>(null);
  const [hv, setHv] = useState<number | null>(null);
  const L = 44, R = 14, T = 12, B = 42;
  const { t, statuses, n } = p;
  const m = t.length;
  const t0 = p.since, t1 = t[m - 1];
  const x = (v: number) => L + ((v - t0) / Math.max(1, t1 - t0)) * (W - L - R);
  const used = statuses.map((_, i) => n[i].some(v => v > 0));
  // 叠放顺序（从下往上）：终点状态，再按工作流倒着往上
  const order = [...statuses.keys()].filter(i => used[i] && statuses[i].final).concat([...statuses.keys()].filter(i => used[i] && !statuses[i].final).reverse());
  const finals = order.filter(i => statuses[i].final);
  const total = t.map((_, j) => order.reduce((a, i) => a + n[i][j], 0));
  const doneAt = (j: number) => finals.reduce((a, i) => a + n[i][j], 0);
  const ticks = yTicks(Math.max(...total, 1));
  const top = ticks[ticks.length - 1];
  const y0 = height - B;
  const y = (v: number) => T + (y0 - T) * (1 - v / top);

  // 每一层的下沿、上沿
  const lower = new Array<number>(m).fill(0);
  const bands = order.map(i => {
    const lo = lower.slice();
    for (let j = 0; j < m; j++) lower[j] += n[i][j];
    const up = lower.slice();
    const edge = t.map((v, j) => `${j ? 'L' : 'M'}${x(v).toFixed(1)},${y(up[j]).toFixed(1)}`).join('');
    const back = t
      .map((v, j) => [v, j] as const)
      .reverse()
      .map(([v, j]) => `L${x(v).toFixed(1)},${y(lo[j]).toFixed(1)}`)
      .join('');
    return { i, up, edge, area: `${edge}${back}Z` };
  });
  const lines = bands.filter((b, k) => statuses[b.i].final || k === bands.length - 1);
  // 新建之前那一段（白板刚建、还没有卡片）从 0 画起
  const lead = m && t[0] > t0 ? `M${x(t0)},${y(0)}L${x(t[0])},${y(0)}` : '';

  // 版本：只画创建以来发的，按时间排
  const vs = versions.filter(v => v.at >= t0 && v.at <= t1).sort((a, b) => a.at - b.at);
  /** 某个时刻之前（含）最后一个采样点 */
  const sampleAt = (at: number) => {
    let j = 0;
    while (j + 1 < m && t[j + 1] <= at) j++;
    return j;
  };
  /** 某个时刻已经发了的最新版本 */
  const versionAt = (at: number) => {
    let v: ProgressVersion | null = null;
    for (const x of vs) if (x.at <= at) v = x;
    return v;
  };
  const RUG = { top: y0 + 4, bottom: y0 + 18 };

  const move = (e: React.PointerEvent<SVGSVGElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const px = e.clientX - r.left;
    const py = e.clientY - r.top;
    if (vs.length && py > y0 + 1) {
      // 版本那一行：挑离指针最近的一次发版（10px 以内）
      let best = -1;
      for (let k = 0; k < vs.length; k++) if (best < 0 || Math.abs(x(vs[k].at) - px) < Math.abs(x(vs[best].at) - px)) best = k;
      setHi(null);
      setHv(best >= 0 && Math.abs(x(vs[best].at) - px) <= 10 ? best : null);
      return;
    }
    let best = 0;
    for (let j = 1; j < m; j++) if (Math.abs(x(t[j]) - px) < Math.abs(x(t[best]) - px)) best = j;
    setHv(null);
    setHi(best);
  };
  const key = (e: KeyboardEvent) => {
    const step = e.shiftKey ? 24 : 1;
    setHv(null);
    if (e.key === 'ArrowLeft') setHi(h => Math.max(0, (h ?? m - 1) - step));
    else if (e.key === 'ArrowRight') setHi(h => Math.min(m - 1, (h ?? m - 1) + step));
    else if (e.key === 'Escape') setHi(null);
    else return;
    e.preventDefault();
  };
  const clear = () => {
    setHi(null);
    setHv(null);
  };

  const xt = xTicks(t0, t1, W - L - R);
  // 第一个点是创建时间、不在整点上：按中间的间隔判断是不是按天采样的
  const hourly = (m > 2 ? t[2] - t[1] : t[m - 1] - t[0]) < DAY;
  const cur = hv != null ? vs[hv] : null;
  const cj = cur ? sampleAt(cur.at) : hi;
  const hx = cur ? x(cur.at) : hi != null ? x(t[hi]) : 0;
  const tipLeft = hx > W * 0.62;
  const ver = hi != null ? versionAt(t[hi]) : null;
  return (
    <div className="pg-chart" ref={ref}>
      <svg width={W} height={height} role="img" aria-label="进步曲线：创建以来各状态的卡片数和每次发版" tabIndex={0} onKeyDown={key} onBlur={clear} onPointerMove={move} onPointerLeave={clear}>
        <defs>
          {bands.map(b => (
            <linearGradient key={b.i} id={`pg-g-${b.i}`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" style={{ stopColor: statusColor(statuses[b.i].color), stopOpacity: statuses[b.i].final ? 0.42 : 0.85 }} />
              <stop offset="1" style={{ stopColor: statusColor(statuses[b.i].color), stopOpacity: statuses[b.i].final ? 0.06 : 0.85 }} />
            </linearGradient>
          ))}
        </defs>
        {ticks.map(v => (
          <g key={v}>
            <line x1={L} x2={W - R} y1={y(v)} y2={y(v)} className={cx('sp-grid', v === 0 && 'base')} />
            <text x={L - 8} y={y(v) + 4} className="sp-axis" textAnchor="end">
              {v}
            </text>
          </g>
        ))}
        {bands.map(b => (
          <path key={b.i} d={b.area} fill={`url(#pg-g-${b.i})`} className={cx('pg-band', (hi != null || cur) && 'dim')} />
        ))}
        {/* 大版本（x.y.0）：一条贯穿的竖线，顶上写版本号 */}
        {vs.filter(v => bigVersion(v.name)).map(v => {
          const vx = x(v.at);
          const right = vx + 60 > W - R;
          return (
            <g key={`${v.project ?? ''}${v.name}`} className="pg-major">
              <line x1={vx} x2={vx} y1={T} y2={y0} />
              <text x={right ? vx - 5 : vx + 5} y={T + 10} textAnchor={right ? 'end' : 'start'} className="sp-axis strong">
                {v.name}
              </text>
            </g>
          );
        })}
        {/* 只描两条线：终点状态的上沿（做完了多少）和最上面的总数；中间各层薄，描边会缠在一起 */}
        {lines.map(b => (
          <path key={b.i} d={b.edge} className={cx('pg-edge', statuses[b.i].final && 'final')} style={{ stroke: statusColor(statuses[b.i].color) }} />
        ))}
        {lead && <path d={lead} className="pg-edge" style={{ stroke: 'var(--line-3)' }} />}
        {/* 版本那一行：每次发版一道刻度，大版本长一点 */}
        {vs.map((v, k) => {
          const big = bigVersion(v.name);
          return (
            <line
              key={`${v.project ?? ''}${v.name}`}
              x1={x(v.at)}
              x2={x(v.at)}
              y1={big ? RUG.top : RUG.top + 3}
              y2={big ? RUG.bottom : RUG.bottom - 3}
              className={cx('pg-ver', big && 'big', hv === k && 'on')}
            />
          );
        })}
        {xt.map(k => (
          <text key={k.t} x={x(k.t)} y={height - 6} className={cx('sp-axis', k.strong && 'strong')} textAnchor="middle">
            {k.text}
          </text>
        ))}
        {cj != null && (
          <g className="pg-hover">
            <line x1={hx} x2={hx} y1={T} y2={cur ? RUG.bottom : y0} className="pg-cross" />
            {(cur ? lines.filter(b => statuses[b.i].final) : lines)
              .filter(b => n[b.i][cj] > 0)
              .map(b => (
                <circle key={b.i} cx={hx} cy={y(b.up[cj])} r={4} className="pg-dot" style={{ fill: statusColor(statuses[b.i].color) }} />
              ))}
          </g>
        )}
      </svg>
      {cur && cj != null && (
        <div className={cx('pg-tip', tipLeft && 'left')} style={{ left: tipLeft ? hx - 12 : hx + 12, top: T }}>
          <b>{cur.name}</b>
          {cur.project && <span>{cur.project}</span>}
          <span>
            {fmtDay(cur.at)} {fmtWeekday(cur.at)} {fmtHm(cur.at)}
          </span>
          <span className="pg-tip-sum">
            这一版<em>{cur.cards} 张卡</em>
          </span>
          <span>
            发版时已完成<em>{doneAt(cj)}</em>
          </span>
        </div>
      )}
      {hi != null && (
        <div className={cx('pg-tip', tipLeft && 'left')} style={{ left: tipLeft ? hx - 12 : hx + 12, top: T }}>
          <b className="pg-tip-h">
            {fmtDay(t[hi])} {fmtWeekday(t[hi])}
            {hi === m - 1 ? ' · 现在' : hourly ? ` ${fmtHour(t[hi])}` : ''}
            {ver && <small>{ver.name}</small>}
          </b>
          {[...order].reverse().map(i => (
            <span key={i} className={cx(!n[i][hi] && 'zero')}>
              <i style={{ background: statusColor(statuses[i].color) }} />
              {statuses[i].name}
              <em>{n[i][hi]}</em>
            </span>
          ))}
          <span className="pg-tip-sum">
            共<em>{total[hi]}</em>
          </span>
        </div>
      )}
    </div>
  );
}

export function ProgressLegend({ p, versions = 0 }: { p: StatsProgress; versions?: number }) {
  return (
    <div className="tl-legend pg-legend">
      {p.statuses.map((s, i) =>
        p.n[i].some(v => v > 0) ? (
          <span key={s.key}>
            <i className="pg-key" style={{ background: statusColor(s.color) }} />
            {s.name}
          </span>
        ) : null,
      )}
      {versions > 0 && (
        <span>
          <i className="pg-vkey" />
          发版 {versions} 次
        </span>
      )}
    </div>
  );
}
