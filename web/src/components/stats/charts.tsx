/* 大盘用的几个图（纯 SVG / CSS，不引图表库）。时间按浏览器的时区显示：接口按同一个时区分桶 */
import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { cx } from '../../lib/util';

/* ---------- 悬停提示 ---------- */

export function useTip() {
  const [tip, setTip] = useState<{ x: number; y: number; body: ReactNode } | null>(null);
  const show = (e: React.PointerEvent | React.MouseEvent, body: ReactNode) => setTip({ x: e.clientX, y: e.clientY, body });
  const hide = () => setTip(null);
  const node = tip && (
    <div className="sp-tip" style={{ left: tip.x, top: tip.y }}>
      {tip.body}
    </div>
  );
  return { show, hide, node };
}

export function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [w, setW] = useState(600);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    // 隐藏时（切到别的 Tab）量出来是 0：保留上一次的宽度，免得画出负宽度
    const measure = () => el.clientWidth > 0 && setW(el.clientWidth);
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);
  return [ref, w] as const;
}

const nice = (v: number) => {
  if (v <= 4) return 4;
  const p = 10 ** Math.floor(Math.log10(v));
  const m = v / p;
  return (m <= 2 ? 2 : m <= 5 ? 5 : 10) * p;
};

const pad = (n: number) => String(n).padStart(2, '0');
export const fmtDay = (t: number) => {
  const d = new Date(t);
  return `${d.getMonth() + 1}月${d.getDate()}日`;
};
export const fmtHour = (t: number) => `${pad(new Date(t).getHours())}:00`;
const WD = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];
export const fmtWeekday = (t: number) => WD[new Date(t).getDay()];

/* ---------- 趋势柱状图：一个指标、一根轴 ---------- */

export function TrendBars({
  data,
  bucket,
  label,
  fmt,
  height = 200,
}: {
  data: { t: number; v: number }[];
  bucket: 'hour' | 'day';
  label: string;
  fmt: (v: number) => string;
  height?: number;
}) {
  const [ref, W] = useWidth<HTMLDivElement>();
  const [hi, setHi] = useState<number | null>(null);
  const tip = useTip();
  const L = 34, B = 22, T = 8;
  const max = nice(Math.max(1, ...data.map(d => d.v)));
  const n = data.length || 1;
  const step = Math.max(0, W - L) / n;
  const gap = Math.min(2, step * 0.25);
  const bw = Math.max(1, Math.min(step - gap, 44));
  const y = (v: number) => T + (height - T - B) * (1 - v / max);
  const ticks = [0, max / 2, max];
  const labelEvery = bucket === 'hour' ? (n > 48 ? 12 : 6) : n > 20 ? 7 : n > 10 ? 2 : 1;
  return (
    <div className="sp-trend" ref={ref}>
      <svg width={W} height={height} role="img" aria-label={`${label}趋势`}>
        {ticks.map(v => (
          <g key={v}>
            <line x1={L} x2={W} y1={y(v)} y2={y(v)} className={cx('sp-grid', v === 0 && 'base')} />
            <text x={L - 8} y={y(v) + 4} className="sp-axis" textAnchor="end">
              {fmt(v)}
            </text>
          </g>
        ))}
        {data.map((d, i) => {
          const x = L + i * step + (step - bw) / 2;
          const h = Math.max(0, y(0) - y(d.v));
          const r = Math.min(4, bw / 2, h);
          const lab = i % labelEvery === 0;
          const dayEdge = bucket === 'hour' && new Date(d.t).getHours() === 0;
          return (
            <g key={d.t}>
              {d.v > 0 && (
                <path
                  className={cx('sp-bar', hi === i && 'hi', hi != null && hi !== i && 'dim')}
                  d={`M${x},${y(0)} V${y(0) - h + r} Q${x},${y(0) - h} ${x + r},${y(0) - h} H${x + bw - r} Q${x + bw},${y(0) - h} ${x + bw},${y(0) - h + r} V${y(0)} Z`}
                />
              )}
              {(lab || dayEdge) && (
                <text x={x + bw / 2} y={height - 6} className={cx('sp-axis', dayEdge && 'strong')} textAnchor="middle">
                  {bucket === 'hour' ? (dayEdge ? fmtDay(d.t) : fmtHour(d.t)) : fmtDay(d.t)}
                </text>
              )}
              <rect
                x={L + i * step}
                y={T}
                width={step}
                height={height - T - B}
                fill="transparent"
                onPointerMove={e => {
                  setHi(i);
                  tip.show(
                    e,
                    <>
                      <b>{bucket === 'hour' ? `${fmtDay(d.t)} ${fmtHour(d.t)}–${fmtHour(d.t + 3600_000)}` : `${fmtDay(d.t)} ${fmtWeekday(d.t)}`}</b>
                      <span>
                        {label} {fmt(d.v)}
                      </span>
                    </>,
                  );
                }}
                onPointerLeave={() => {
                  setHi(null);
                  tip.hide();
                }}
              />
            </g>
          );
        })}
      </svg>
      {tip.node}
    </div>
  );
}

/* ---------- 迷你柱：指标卡里的小趋势 ---------- */

export function Spark({ data, fmt, label, bucket, height = 40 }: { data: { t: number; v: number }[]; fmt: (v: number) => string; label: string; bucket: 'hour' | 'day'; height?: number }) {
  const [ref, W] = useWidth<HTMLDivElement>();
  const tip = useTip();
  const [hi, setHi] = useState<number | null>(null);
  const H = height;
  const max = Math.max(1, ...data.map(d => d.v));
  const step = W / Math.max(1, data.length);
  const bw = Math.max(1, step - Math.min(2, step * 0.3));
  return (
    <div className="sp-spark" ref={ref} style={{ height }}>
      <svg width={W} height={H}>
        <line x1={0} x2={W} y1={H - 0.5} y2={H - 0.5} className="sp-grid base" />
        {data.map((d, i) => {
          const h = (H - 2) * (d.v / max);
          return (
            <g key={d.t}>
              {d.v > 0 && <rect x={i * step} y={H - 1 - h} width={bw} height={h} rx={Math.min(2, bw / 2)} className={cx('sp-bar', hi === i && 'hi', hi != null && hi !== i && 'dim')} />}
              <rect
                x={i * step}
                y={0}
                width={step}
                height={H}
                fill="transparent"
                onPointerMove={e => {
                  setHi(i);
                  tip.show(
                    e,
                    <>
                      <b>{bucket === 'hour' ? `${fmtDay(d.t)} ${fmtHour(d.t)}` : `${fmtDay(d.t)} ${fmtWeekday(d.t)}`}</b>
                      <span>
                        {label} {fmt(d.v)}
                      </span>
                    </>,
                  );
                }}
                onPointerLeave={() => {
                  setHi(null);
                  tip.hide();
                }}
              />
            </g>
          );
        })}
      </svg>
      {tip.node}
    </div>
  );
}

/* ---------- 热力图的色阶：一个色相，由浅到深（深色主题反过来） ---------- */

const level = (v: number, max: number) => (v <= 0 ? 0 : Math.min(5, 1 + Math.floor((v / max) * 4.999)));

export function HeatLegend() {
  return (
    <span className="sp-legend">
      少
      {[0, 1, 2, 3, 4, 5].map(l => (
        <i key={l} className={`sp-cell l${l}`} />
      ))}
      多
    </span>
  );
}

/* ---------- 时段热力图：星期几 × 24 小时 ---------- */

const ROWS = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'];

export function HourHeat({ m, big }: { m: number[][]; big?: boolean }) {
  const tip = useTip();
  const max = Math.max(1, ...m.flat());
  return (
    <div className={cx('sp-hh', big && 'big')}>
      <div className="sp-hh-grid">
        <span />
        {Array.from({ length: 24 }, (_, h) => (
          <span key={h} className="sp-hh-x">
            {h % 3 === 0 ? h : ''}
          </span>
        ))}
        {m.map((row, d) => (
          <Row key={d} label={ROWS[d]}>
            {row.map((v, h) => (
              <i
                key={h}
                className={`sp-cell l${level(v, max)}`}
                onPointerMove={e =>
                  tip.show(
                    e,
                    <>
                      <b>
                        {ROWS[d]} {pad(h)}:00–{pad(h + 1)}:00
                      </b>
                      <span>{v ? `${v} 条动态` : '没有动态'}</span>
                    </>,
                  )
                }
                onPointerLeave={tip.hide}
              />
            ))}
          </Row>
        ))}
      </div>
      {tip.node}
    </div>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <span className="sp-hh-y">{label}</span>
      {children}
    </>
  );
}

/* ---------- 日历热力图：一列一周，周一在最上面 ---------- */

export function CalHeat({
  days,
  onPick,
  big,
  maxWeeks = 26,
  cell,
}: {
  days: { t: number; v: number; future: boolean }[];
  onPick?: (t: number) => void;
  big?: boolean;
  /** 宽度放得下几周：放不下时从最早的一周开始少显示 */
  maxWeeks?: number;
  /** 格子边长（px）：放在宽处时放大到和别处一致，默认用 CSS 里的 14px */
  cell?: number;
}) {
  const tip = useTip();
  const all: (typeof days)[] = [];
  days.forEach((d, i) => (all[Math.floor(i / 7)] ??= []).push(d));
  const weeks = all.slice(Math.max(0, all.length - maxWeeks));
  const max = Math.max(1, ...weeks.flat().map(d => d.v));
  return (
    <div className={cx('sp-cal', big && 'big')} style={cell ? ({ ['--c' as string]: `${cell}px` } as CSSProperties) : undefined}>
      <div className="sp-cal-y">
        {ROWS.map((r, i) => (
          <span key={r}>{i % 2 === 0 ? r : ''}</span>
        ))}
      </div>
      <div className="sp-cal-weeks">
        {weeks.map((w, i) => {
          const first = new Date(w[0].t);
          const newMonth = i === 0 || first.getDate() <= 7;
          return (
            <div className="sp-cal-col" key={w[0].t}>
              <span className="sp-cal-m">{newMonth ? `${first.getMonth() + 1}月` : ''}</span>
              {w.map(d => {
                return (
                  <i
                    key={d.t}
                    className={cx('sp-cell', d.future ? 'future' : `l${level(d.v, max)}`, onPick && !d.future && 'pick')}
                    onClick={() => !d.future && onPick?.(d.t)}
                    onPointerMove={e =>
                      !d.future &&
                      tip.show(
                        e,
                        <>
                          <b>
                            {fmtDay(d.t)} {fmtWeekday(d.t)}
                          </b>
                          <span>{d.v ? `${d.v} 条动态` : '没有动态'}</span>
                          {onPick && <em>点一下只看这一天</em>}
                        </>,
                      )
                    }
                    onPointerLeave={tip.hide}
                  />
                );
              })}
            </div>
          );
        })}
      </div>
      {tip.node}
    </div>
  );
}

/* ---------- 成对柱：新建 vs 完成（同一单位，一根轴） ---------- */

export function PairBars({
  data,
  bucket,
  height = 180,
}: {
  data: { t: number; created: number; done: number }[];
  bucket: 'hour' | 'day';
  height?: number;
}) {
  const [ref, W] = useWidth<HTMLDivElement>();
  const [hi, setHi] = useState<number | null>(null);
  const tip = useTip();
  const L = 30, B = 22, T = 8;
  const max = nice(Math.max(1, ...data.flatMap(d => [d.created, d.done])));
  const n = data.length || 1;
  const step = Math.max(0, W - L) / n;
  const bw = Math.max(1, Math.min((step - 3) / 2, 20));
  const y = (v: number) => T + (height - T - B) * (1 - v / max);
  const labelEvery = bucket === 'hour' ? (n > 48 ? 12 : 6) : n > 20 ? 7 : n > 10 ? 2 : 1;
  const bar = (x: number, v: number, cls: string, i: number) => {
    const h = Math.max(0, y(0) - y(v));
    if (h <= 0) return null;
    const r = Math.min(3, bw / 2, h);
    return (
      <path
        className={cx(cls, hi === i && 'hi', hi != null && hi !== i && 'dim')}
        d={`M${x},${y(0)} V${y(0) - h + r} Q${x},${y(0) - h} ${x + r},${y(0) - h} H${x + bw - r} Q${x + bw},${y(0) - h} ${x + bw},${y(0) - h + r} V${y(0)} Z`}
      />
    );
  };
  return (
    <div className="sp-trend" ref={ref}>
      <svg width={W} height={height}>
        {[0, max / 2, max].map(v => (
          <g key={v}>
            <line x1={L} x2={W} y1={y(v)} y2={y(v)} className={cx('sp-grid', v === 0 && 'base')} />
            <text x={L - 8} y={y(v) + 4} className="sp-axis" textAnchor="end">
              {v}
            </text>
          </g>
        ))}
        {data.map((d, i) => {
          const x0 = L + i * step + (step - (bw * 2 + 2)) / 2;
          const dayEdge = bucket === 'hour' && new Date(d.t).getHours() === 0;
          return (
            <g key={d.t}>
              {bar(x0, d.created, 'sp-bar2', i)}
              {bar(x0 + bw + 2, d.done, 'sp-bar', i)}
              {(i % labelEvery === 0 || dayEdge) && (
                <text x={L + i * step + step / 2} y={height - 6} className={cx('sp-axis', dayEdge && 'strong')} textAnchor="middle">
                  {bucket === 'hour' ? (dayEdge ? fmtDay(d.t) : fmtHour(d.t)) : fmtDay(d.t)}
                </text>
              )}
              <rect
                x={L + i * step}
                y={T}
                width={step}
                height={height - T - B}
                fill="transparent"
                onPointerMove={e => {
                  setHi(i);
                  const net = d.done - d.created;
                  tip.show(
                    e,
                    <>
                      <b>{bucket === 'hour' ? `${fmtDay(d.t)} ${fmtHour(d.t)}` : `${fmtDay(d.t)} ${fmtWeekday(d.t)}`}</b>
                      <span>
                        新建 {d.created} · 完成 {d.done}
                      </span>
                      <span>{net === 0 ? '持平' : net > 0 ? `积压减少 ${net}` : `积压增加 ${-net}`}</span>
                    </>,
                  );
                }}
                onPointerLeave={() => {
                  setHi(null);
                  tip.hide();
                }}
              />
            </g>
          );
        })}
      </svg>
      {tip.node}
    </div>
  );
}
