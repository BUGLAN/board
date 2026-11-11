/* 大盘里的两条时间轴
   - agent 时间轴：每个 agent 一条泳道，每次领取一段（card_runs）；第一条泳道是空转
   - 卡片时间线：这段时间完成的卡片，每张一行，按状态分段（moves） */
import { useMemo, useState } from 'react';
import { statusColor } from '../../lib/schema';
import type { RunResult, StatsLife, StatsRun } from '../../lib/types';
import { agentColor, cx } from '../../lib/util';
import { fmtDay, fmtHour, useTip, useWidth } from './charts';
import { fmtDur } from './format';

const H = 3600_000;
const STEPS = [H, 2 * H, 3 * H, 6 * H, 12 * H, 24 * H, 48 * H, 7 * 24 * H, 14 * 24 * H, 28 * 24 * H];

function ticks(from: number, to: number, width: number) {
  const max = Math.max(2, Math.floor(width / 84));
  const step = STEPS.find(s => (to - from) / s <= max) ?? STEPS[STEPS.length - 1];
  const out: number[] = [];
  const d = new Date(from);
  d.setMinutes(0, 0, 0);
  if (step >= 24 * H) {
    d.setHours(0);
    for (; d.getTime() <= to; d.setDate(d.getDate() + step / (24 * H))) if (d.getTime() >= from) out.push(d.getTime());
    return out;
  }
  d.setHours(Math.ceil(d.getHours() / (step / H)) * (step / H));
  for (let t = d.getTime(); t <= to; t += step) if (t >= from) out.push(t);
  return out;
}
const tickLabel = (t: number) => (new Date(t).getHours() === 0 ? fmtDay(t) : fmtHour(t));
const clock = (t: number) => `${fmtDay(t)} ${String(new Date(t).getHours()).padStart(2, '0')}:${String(new Date(t).getMinutes()).padStart(2, '0')}`;

function Axis({ from, to, width, top }: { from: number; to: number; width: number; top?: boolean }) {
  const pct = (t: number) => ((t - from) / (to - from)) * 100;
  return (
    <div className={cx('tl-axis', top && 'top')}>
      {ticks(from, to, width).map(t => (
        <span key={t} style={{ left: `${pct(t)}%` }} className={cx(new Date(t).getHours() === 0 && 'day')}>
          {tickLabel(t)}
        </span>
      ))}
    </div>
  );
}

function Grid({ from, to, width, now }: { from: number; to: number; width: number; now: number }) {
  const pct = (t: number) => ((t - from) / (to - from)) * 100;
  return (
    <div className="tl-grid" aria-hidden="true">
      {ticks(from, to, width).map(t => (
        <i key={t} style={{ left: `${pct(t)}%` }} className={cx(new Date(t).getHours() === 0 && 'day')} />
      ))}
      {now >= from && now <= to && <b className="tl-now" style={{ left: `${pct(now)}%` }} />}
    </div>
  );
}

/* ---------- agent 时间轴 ---------- */

export const RESULT: Record<RunResult, { label: string; legend: string }> = {
  review: { label: '交审', legend: '交审' },
  rework: { label: '被打回', legend: '被打回（交审后又回到可领取状态）' },
  lost: { label: '异常退出', legend: '异常退出（心跳断了）' },
  gaveup: { label: '做不下去', legend: '做不下去（移到需要人处理的状态）' },
  released: { label: '领取被释放', legend: '领取被释放（回到可领取状态）' },
  running: { label: '进行中', legend: '进行中' },
};
const ORDER: RunResult[] = ['review', 'rework', 'lost', 'gaveup', 'released', 'running'];

export function AgentTimeline({
  runs,
  from,
  to,
  now,
  idle,
  showBoard,
}: {
  runs: StatsRun[];
  from: number;
  to: number;
  now: number;
  idle: { board: string; from: number; to: number }[];
  showBoard: boolean;
}) {
  const [ref, W] = useWidth<HTMLDivElement>();
  const tip = useTip();
  const lanes = useMemo(() => {
    const by = new Map<string, StatsRun[]>();
    for (const r of runs) (by.get(r.agent) ?? by.set(r.agent, []).get(r.agent)!).push(r);
    return [...by.entries()]
      .sort((a, b) => b[1].length - a[1].length || a[0].localeCompare(b[0]))
      .map(([agent, rs]) => {
        // 同一个 agent 同时开了几个会话：叠成几行
        const ends: number[] = [];
        const placed = rs.map(r => {
          const end = r.end ?? now;
          let row = ends.findIndex(e => e <= r.start);
          if (row < 0) row = ends.push(0) - 1;
          ends[row] = end;
          return { r, row, end };
        });
        const busy = placed.reduce((n, p) => n + Math.max(0, Math.min(p.end, to) - Math.max(p.r.start, from)), 0);
        return { agent, placed, rows: Math.max(1, ends.length), n: rs.length, busy };
      });
  }, [runs, from, to, now]);
  const used = useMemo(() => ORDER.filter(k => runs.some(r => r.result === k)), [runs]);
  const pct = (t: number) => ((Math.max(from, Math.min(to, t)) - from) / (to - from)) * 100;
  const trackW = Math.max(200, W - 168);
  const idleMs = idle.reduce((n, x) => n + x.to - x.from, 0);
  return (
    <div className="tl" ref={ref}>
      <div className="tl-row tl-head">
        <span />
        <div className="tl-track">
          <Axis from={from} to={to} width={trackW} top />
        </div>
      </div>
      <div className="tl-row">
        <span className="tl-lane idle">
          <i />
          <b>空转</b>
          <small>{fmtDur(idleMs)} · 可领取状态有卡却没人在跑</small>
        </span>
        <div className="tl-track lane idle">
          <Grid from={from} to={to} width={trackW} now={now} />
          {idle.map(x => (
            <span
              key={x.board + x.from}
              className="tl-idle"
              style={{ left: `${pct(x.from)}%`, width: `max(3px, ${pct(x.to) - pct(x.from)}%)` }}
              onPointerMove={e =>
                tip.show(
                  e,
                  <>
                    <b>空转 {fmtDur(x.to - x.from)}</b>
                    <span>
                      {clock(x.from)} – {clock(x.to)}
                    </span>
                    <span>{showBoard ? `${x.board} · ` : ''}可领取状态里有卡，但没有 agent 在跑</span>
                  </>,
                )
              }
              onPointerLeave={tip.hide}
            />
          ))}
        </div>
      </div>
      {lanes.map(l => (
        <div className="tl-row" key={l.agent}>
          <span className="tl-lane">
            <i style={{ background: agentColor(l.agent) }} />
            <b title={l.agent}>{l.agent}</b>
            <small>
              {l.n} 次 · 忙 {fmtDur(l.busy)}
            </small>
          </span>
          <div className="tl-track lane" style={{ height: l.rows * 12 + 14 }}>
            <Grid from={from} to={to} width={trackW} now={now} />
            {l.placed.map(({ r, row, end }) => (
              <span
                key={`${r.board}#${r.card}@${r.start}`}
                className={cx('tl-run', r.result)}
                style={{ left: `${pct(r.start)}%`, width: `max(3px, ${pct(end) - pct(r.start)}%)`, top: 7 + row * 12 }}
                onPointerMove={e =>
                  tip.show(
                    e,
                    <>
                      <b>
                        {showBoard && `${r.board} `}#{r.card} {r.title}
                      </b>
                      <span>
                        {r.name} · {clock(r.start)} 起 · {fmtDur(end - r.start)}
                      </span>
                      <span>结果：{RESULT[r.result].label}</span>
                    </>,
                  )
                }
                onPointerLeave={tip.hide}
              />
            ))}
          </div>
        </div>
      ))}
      {!lanes.length && <p className="tl-empty">这段时间没有领取</p>}
      <div className="tl-legend">
        {used.map(k => (
          <span key={k}>
            <i className={cx('tl-run', k)} />
            {RESULT[k].legend}
          </span>
        ))}
        <span>
          <i className="tl-idle" />
          空转
        </span>
        <span>
          <i className="tl-nowkey" />
          现在
        </span>
      </div>
      {tip.node}
    </div>
  );
}

/* ---------- 卡片时间线 ---------- */

const segColor = (c: string | null) => (c ? statusColor(c) : 'var(--text-4)');

export function CardTimeline({ lives, showBoard, doneColor }: { lives: StatsLife[]; showBoard: boolean; doneColor: string }) {
  const [ref, W] = useWidth<HTMLDivElement>();
  const tip = useTip();
  const [all, setAll] = useState(false);
  const rows = all ? lives : lives.slice(0, 10);
  const legend = useMemo(() => {
    const seen = new Map<string, string | null>();
    for (const l of rows) for (const s of l.segs) if (!seen.has(s.name)) seen.set(s.name, s.color);
    return [...seen.entries()];
  }, [rows]);
  if (!rows.length) return <p className="tl-empty">这段时间没有完成的卡片</p>;
  const from = Math.min(...rows.map(l => l.created));
  const to = Math.max(...rows.map(l => l.done));
  const pad = Math.max(60_000, (to - from) * 0.02);
  const a = from - pad,
    b = to + pad;
  const pct = (t: number) => ((t - a) / (b - a)) * 100;
  const trackW = Math.max(200, W - 380);
  return (
    <div className="tl tl-cards" ref={ref}>
      <div className="tl-row tl-head">
        <span />
        <div className="tl-track">
          <Axis from={a} to={b} width={trackW} top />
        </div>
        <span className="tl-sum-h">从新建到完成</span>
      </div>
      {rows.map(l => (
        <div className="tl-row" key={`${l.board}#${l.card}`}>
          <span className="tl-card" title={`${l.board}#${l.card} ${l.title}`}>
            {showBoard && <small className="sp-tag">{l.board}</small>}
            <em>#{l.card}</em>
            {l.title}
          </span>
          <div className="tl-track lane">
            <Grid from={a} to={b} width={trackW} now={Number.NaN} />
            {l.segs.map((s, i) => (
              <span
                key={i}
                className="tl-seg"
                style={{ left: `${pct(s.from)}%`, width: `max(2px, ${pct(s.to) - pct(s.from)}%)`, ['--c' as string]: segColor(s.color) }}
                onPointerMove={e =>
                  tip.show(
                    e,
                    <>
                      <b>
                        #{l.card} 在「{s.name}」
                      </b>
                      <span>
                        {clock(s.from)} – {clock(s.to)} · {fmtDur(s.to - s.from)}
                      </span>
                    </>,
                  )
                }
                onPointerLeave={tip.hide}
              />
            ))}
            <span className="tl-done" style={{ left: `${pct(l.done)}%`, ['--c' as string]: doneColor }} title={`${clock(l.done)} 完成`} />
          </div>
          <span className="tl-sum">
            <b>{fmtDur(l.done - l.created)}</b>
            {l.reworks > 0 && <small>打回 {l.reworks} 次</small>}
          </span>
        </div>
      ))}
      <div className="tl-legend">
        {legend.map(([name, color]) => (
          <span key={name}>
            <i className="tl-seg" style={{ ['--c' as string]: segColor(color) }} />
            {name}
          </span>
        ))}
        <span>
          <i className="tl-done" style={{ ['--c' as string]: doneColor }} />
          完成
        </span>
        <span className="tl-sp" />
        {lives.length > 10 && (
          <button className="tl-more" type="button" onClick={() => setAll(!all)}>
            {all ? '只看最近 10 张' : `显示全部 ${lives.length} 张`}
          </button>
        )}
      </div>
      {tip.node}
    </div>
  );
}
