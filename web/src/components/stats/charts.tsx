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
export const fmtShortDay = (t: number) => {
  const d = new Date(t);
  return `${d.getMonth() + 1}/${d.getDate()}`;
};
const WD = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];
export const fmtWeekday = (t: number) => WD[new Date(t).getDay()];

/* ---------- 横轴标签：按字宽挑要显示的桶，首尾一定显示且贴边 ---------- */

/** 小屏（≤420px）上横轴字是 9px，宽屏是 10.5px */
const axisFont = (W: number) => (W <= 420 ? 9 : 10.5);

/** 一段文字的宽度（px）：等宽字体按字号 0.6 算，中文按 1em（「11月6日」的「月」「日」是全角） */
const textPx = (text: string, fs: number) => text.replace(/[^\x00-\xff]/g, 'xx').length * 0.6 * fs;

/** 两个标签之间至少留的空（px） */
const LABEL_GAP = 8;

/** 没有时间时按 1 小时一格摆（只拿来挑个数） */
const HOUR_MS = 3600_000;

/** 横轴上这个桶写什么：按天写短格式（11/6）；按小时只有整天那一格写「11月6日」，其余写整点 */
const axisText = (t: number, hour: boolean) => (hour ? fmtHour(t) : fmtShortDay(t));

/** 一个横轴标签：位置（相对 svg 左边）、对齐方式、文字，以及它属于第几个桶 */
export type AxisLabel = { x: number; anchor: 'start' | 'middle' | 'end'; text: string; i: number };

/**
 * 挑横轴要标的桶并摆好位置：第一个贴着左边（L）、最后一个贴着右边（W - 2），中间的按字宽挑，放不下就不标。
 *
 * - `hour`：按小时。整天那一格写「11月6日」、字号大，一定标，它左右各留一个整点的位置。
 * - 其余（按天）：每格都一样宽，按字宽等间隔挑。
 *
 * 按宽度算而不是按第几个桶：柱子多、容器窄时字挤在一起，正是这张卡要修的问题。
 */
export function axisPlacement(ts: number[], W: number, L: number, hour: boolean, edge?: (i: number) => boolean): AxisLabel[] {
  const n = ts.length;
  if (n < 1) return [];
  if (!Number.isFinite(W)) W = 600;
  const fs = axisFont(W);
  const right = Math.max(L + 4, W - 2);
  const out: AxisLabel[] = [];
  const isEdge = (i: number) => hour && i > 0 && i < n - 1 && !!edge?.(i);
  const width = (i: number) => textPx(isEdge(i) ? fmtDay(ts[i]) : axisText(ts[i], hour), fs);
  const center = (i: number) => L + ((i + 0.5) * (right - L)) / n;
  /** 第 i 格居中摆的位置 */
  const slot = (i: number) => center(i) - width(i) / 2;

  out.push({ x: L, anchor: 'start', text: axisText(ts[0], hour), i: 0 });
  if (n === 1) return out;
  let last = L + width(0) + LABEL_GAP; // 下一个标签能开始的最左位置
  const rightStart = right - width(n - 1) - LABEL_GAP; // 上一个中间标签必须在这个位置之前结束
  const takenLeft = new Array<boolean>(n + 1).fill(false); // 这一格左右要不要留空

  // 先放日界：它是这一天唯一的日期，字也最大
  if (hour) {
    // 最后一个标签是贴右边的，可能比「整点」宽（范围末尾正好落在 0 点时它就是「11月14日」），按长日期估它的左边缘
    const tail = right - LABEL_GAP - textPx(fmtDay(ts[n - 1]), fs);
    for (let i = 1; i < n - 1; i++) {
      if (!isEdge(i)) continue;
      const wi = width(i);
      const x = slot(i);
      // 放不下（顶上上一个，或者挤到最后贴着右边的那一格）就不标：首尾那两格已经写着日期了
      if (x < last || x + wi + LABEL_GAP > Math.min(rightStart, tail)) continue;
      out.push({ x, anchor: 'middle', text: fmtDay(ts[i]), i });
      last = x + wi + LABEL_GAP;
      takenLeft[i - 1] = true;
      takenLeft[i + 1] = true;
    }
  }

  // 再按字宽补其余格子：居中摆，放不下（和上一个挨着、或顶到最后一个）就跳过
  for (let i = 1; i < n - 1; i++) {
    if (takenLeft[i]) continue;
    const wi = width(i);
    const x = slot(i);
    if (x < last || x + wi + LABEL_GAP > rightStart) continue;
    out.push({ x, anchor: 'middle', text: axisText(ts[i], hour), i });
    last = x + wi + LABEL_GAP;
  }
  out.push({ x: right, anchor: 'end', text: axisText(ts[n - 1], hour), i: n - 1 });
  return out;
}

/** 挑要标的桶：`show[i]` 标不标；`label[i]` 标的时候画在哪（x 相对于 svg 左边）、写什么、怎么对齐 */
export function axisLayout(n: number, W: number, L: number, hour: boolean, ts: number[], edge?: (i: number) => boolean) {
  const show = new Array<boolean>(n).fill(false);
  const label: (AxisLabel | null)[] = new Array(n).fill(null);
  if (n < 1) return { show, label };
  const seq = ts.length === n ? ts : Array.from({ length: n }, (_, i) => i * HOUR_MS);
  for (const p of axisPlacement(seq, W, L, hour, edge)) {
    show[p.i] = true;
    label[p.i] = p;
  }
  return { show, label };
}


/**
 * 横轴只标了首尾两天、中间的日子撤进悬停提示时，图例旁边补的一句；标得下时不写。
 */
export const axisNote = (all: number[], W: number, L: number) => {
  if (all.length < 3) return '';
  // 中间一个日子都没标出来（只有首尾两个）才需要这句
  if (axisPlacement(all, W, L, false).some(p => p.i > 0 && p.i < all.length - 1)) return '';
  const first = fmtShortDay(all[0]);
  const last = fmtShortDay(all[all.length - 1]);
  return all.length > 24 ? `横轴：日期在悬停提示里，跨度 ${first} – ${last}` : `横轴：只标了 ${first} 和 ${last} 两天，其余悬停看`;
};

/* ---------- 趋势柱状图：一个指标、一根轴 ---------- */

export function TrendBars({
  data,
  bucket,
  label,
  fmt,
  height = 200,
  onAxisNote,
}: {
  data: { t: number; v: number }[];
  bucket: 'hour' | 'day';
  label: string;
  fmt: (v: number) => string;
  height?: number;
  /** 横轴日期撤到悬停提示里时，把一句说明交出去（写在图例旁边） */
  onAxisNote?: (note: string) => void;
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
  // 按天时用短格式（11/6）放得多一些；按小时时整天记得长（11月6日），放不下就抽稀
  const short = bucket === 'day';
  const isEdge = (i: number) => bucket === 'hour' && new Date(data[i].t).getHours() === 0;
  const ax = axisLayout(n, W, L, !short, data.map(d => d.t), isEdge);
  const note = onAxisNote ? axisNote(data.map(d => d.t), W, L) : '';
  useLayoutEffect(() => {
    onAxisNote?.(note);
  });
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
          const lab = ax.label[i];
          const dayEdge = lab != null && bucket === 'hour' && new Date(d.t).getHours() === 0;
          return (
            <g key={d.t}>
              {d.v > 0 && (
                <path
                  className={cx('sp-bar', hi === i && 'hi', hi != null && hi !== i && 'dim')}
                  d={`M${x},${y(0)} V${y(0) - h + r} Q${x},${y(0) - h} ${x + r},${y(0) - h} H${x + bw - r} Q${x + bw},${y(0) - h} ${x + bw},${y(0) - h + r} V${y(0)} Z`}
                />
              )}
              {lab && (
                <text x={lab.x} y={height - 6} className={cx('sp-axis', dayEdge && 'strong')} textAnchor={lab.anchor}>
                  {lab.text}
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
  onAxisNote,
}: {
  data: { t: number; created: number; done: number }[];
  bucket: 'hour' | 'day';
  height?: number;
  /** 横轴日期撤到悬停提示里时，把一句说明交出去（写在图例旁边） */
  onAxisNote?: (note: string) => void;
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
  const short = bucket === 'day';
  const isEdge = (i: number) => bucket === 'hour' && new Date(data[i].t).getHours() === 0;
  const ax = axisLayout(n, W, L, !short, data.map(d => d.t), isEdge);
  const note = onAxisNote ? axisNote(data.map(d => d.t), W, L) : '';
  useLayoutEffect(() => {
    onAxisNote?.(note);
  });
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
          const dayEdge = ax.label[i] != null && bucket === 'hour' && new Date(d.t).getHours() === 0;
          return (
            <g key={d.t}>
              {bar(x0, d.created, 'sp-bar2', i)}
              {bar(x0 + bw + 2, d.done, 'sp-bar', i)}
              {ax.label[i] && (
                <text x={ax.label[i]!.x} y={height - 6} className={cx('sp-axis', dayEdge && 'strong')} textAnchor={ax.label[i]!.anchor}>
                  {ax.label[i]!.text}
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
