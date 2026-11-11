/* ==========================================================================
   大盘（PRD §10.13）：活动栏「大盘」Tab
   - 侧栏是范围导航：全部项目 → 项目 → 白板；点项目或白板时顶栏的当前白板跟着切换
   - 页面是报告式：总览 → 现在 → 这段时间（总量、积压净流量、交付周期、趋势、agent 时间轴、
     热力图、卡片时间线、按 agent、状态停留）
   - 数据一次取完（GET /api/stats，按浏览器的时区分桶）；切换范围和时间时保留旧数据直到新数据回来；
     有推送（工作区 rev 变了）就重新取
   ========================================================================== */
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { create } from 'zustand';
import { editBoard, openBoard } from '../../lib/actions';
import { api } from '../../lib/api';
import { statusColor } from '../../lib/schema';
import type { BoardInfo, ProjectInfo, Stats, StatsBrief, StatsScope } from '../../lib/types';
import { agentColor, cx } from '../../lib/util';
import { useCurrent, useData } from '../../store/data';
import { Markdown, ProjectAvatar } from '../bits';
import { StatsIcon24 } from '../Icon';
import { CalHeat, HeatLegend, HourHeat, PairBars, Spark, TrendBars, fmtDay, useWidth } from './charts';
import { HINT, LEAD_BINS, METRICS, binOf, durParts, fmtDur, metricFmt, metricValue, num, pct, type Metric } from './format';
import { AgentTimeline, CardTimeline } from './timeline';
import './stats.css';

/* ------------------------------ 状态 ------------------------------ */

const H = 3600_000;
const DAY = 24 * H;

type Kind = '24h' | '3d' | '7d' | 'custom';
const KINDS: { key: Kind; label: string }[] = [
  { key: '24h', label: '24 小时' },
  { key: '3d', label: '3 天' },
  { key: '7d', label: '7 天' },
  { key: 'custom', label: '自定义' },
];

const pad2 = (n: number) => String(n).padStart(2, '0');
const ymd = (t: number) => {
  const d = new Date(t);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
};
const parseYmd = (s: string) => {
  const [y, m, d] = s.split('-').map(Number);
  return new Date(y, m - 1, d).getTime();
};

/** 浏览器的 IANA 时区：传给接口分桶，图上的时间也按它显示 */
const TZ = (() => {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
})();

interface SP {
  scope: StatsScope;
  kind: Kind;
  cFrom: string;
  cTo: string;
  metric: Metric;
  set: (p: Partial<SP>) => void;
}
const useSP = create<SP>(set => ({
  scope: 'board',
  kind: '7d',
  cFrom: ymd(Date.now() - 6 * DAY),
  cTo: ymd(Date.now()),
  metric: 'done',
  set: p => set(p),
}));

/** 当前整点（向上取整），每分钟对一次：预设范围的结束时间跟着走 */
function useHour() {
  const [h, setH] = useState(() => Math.ceil(Date.now() / H) * H);
  useEffect(() => {
    const t = setInterval(() => setH(Math.ceil(Date.now() / H) * H), 60_000);
    return () => clearInterval(t);
  }, []);
  return h;
}

function useRange() {
  const { kind, cFrom, cTo } = useSP();
  const hour = useHour();
  return useMemo(() => {
    if (kind === 'custom') {
      // 整天：含结束那天
      const from = parseYmd(cFrom);
      const d = new Date(parseYmd(cTo));
      d.setDate(d.getDate() + 1);
      return { from, to: Math.max(from + H, d.getTime()) };
    }
    const span = { '24h': 24, '3d': 72, '7d': 168 }[kind] * H;
    return { from: hour - span, to: hour };
  }, [kind, cFrom, cTo, hour]);
}

/** 当前要看的范围：白板 / 项目 / 全部项目 */
function useQuery() {
  const scope = useSP(s => s.scope);
  const cur = useCurrent();
  const { from, to } = useRange();
  if (scope !== 'all' && !cur) return null;
  const path = scope === 'board' ? cur!.board.path : scope === 'project' ? cur!.project.key : undefined;
  return { scope, path, from, to, tz: TZ, key: `${scope}|${path ?? ''}|${from}|${to}` };
}

/* 取数：只认最后一次请求的结果；新数据回来之前一直显示旧的 */
interface SD {
  data: Stats | null;
  key: string | null;
  loading: boolean;
  error: string | null;
}
const useSD = create<SD>(() => ({ data: null, key: null, loading: false, error: null }));
let seq = 0;
async function load(q: NonNullable<ReturnType<typeof useQuery>>) {
  const my = ++seq;
  useSD.setState({ loading: true });
  try {
    const data = await api.stats(q);
    if (my === seq) useSD.setState({ data, key: q.key, loading: false, error: null });
  } catch (e) {
    if (my === seq) useSD.setState({ loading: false, error: (e as Error).message });
  }
}

/** 切换范围、时间立即取；有推送时等 1.2 秒再取（连着的几次推送合成一次） */
function useLoader() {
  const q = useQuery();
  const rev = useData(s => s.ws?.rev);
  const last = useRef<string | null>(null);
  useEffect(() => {
    if (!q) return;
    if (q.key !== last.current) {
      last.current = q.key;
      void load(q);
      return;
    }
    const t = setTimeout(() => void load(q), 1200);
    return () => clearTimeout(t);
  }, [q?.key, rev]); // eslint-disable-line react-hooks/exhaustive-deps
}

const span = (from: number, to: number) => {
  const end = to - 1;
  const midnight = (t: number) => new Date(t).getHours() === 0 && new Date(t).getMinutes() === 0;
  if (midnight(from) && midnight(to)) return ymd(from) === ymd(end) ? `${fmtDay(from)} 全天` : `${fmtDay(from)} – ${fmtDay(end)}`;
  const hm = (t: number) => `${pad2(new Date(t).getHours())}:${pad2(new Date(t).getMinutes())}`;
  if (ymd(from) === ymd(end)) return `${fmtDay(from)} ${hm(from)}–${hm(to)}`;
  return `${fmtDay(from)} ${hm(from)} – ${fmtDay(end)} ${hm(to)}`;
};

const fmtDate = (t: number) => {
  const d = new Date(t);
  return `${d.getFullYear()} 年 ${d.getMonth() + 1} 月 ${d.getDate()} 日`;
};

/* ------------------------------ 小部件 ------------------------------ */

function Card({ title, extra, children, className, id }: { title: string; extra?: ReactNode; children: ReactNode; className?: string; id?: string }) {
  return (
    <section className={cx('sp-card', className)} id={id}>
      <header>
        <h2>{title}</h2>
        {extra}
      </header>
      {children}
    </section>
  );
}

const Note = ({ children }: { children: ReactNode }) => <span className="sp-card-note">{children}</span>;

function RangeBar({ loading }: { loading: boolean }) {
  const { kind, cFrom, cTo, set } = useSP();
  const { from, to } = useRange();
  const today = ymd(Date.now());
  return (
    <div className="sp-range">
      <div className="segmented sp-seg" role="tablist" aria-label="时间范围">
        {KINDS.map(k => (
          <button key={k.key} type="button" role="tab" aria-selected={kind === k.key} className={cx(kind === k.key && 'on')} onClick={() => set({ kind: k.key })}>
            {k.label}
          </button>
        ))}
      </div>
      {kind === 'custom' && (
        <span className="sp-dates">
          <input type="date" aria-label="开始日期" value={cFrom} max={cTo} onChange={e => e.target.value && set({ cFrom: e.target.value })} />
          <span>至</span>
          <input type="date" aria-label="结束日期" value={cTo} min={cFrom} max={today} onChange={e => e.target.value && set({ cTo: e.target.value })} />
        </span>
      )}
      <span className="sp-range-note">
        {loading && <i className="sp-loading">正在更新</i>}
        {span(from, to)}
        <em>对比 {span(from - (to - from), from)}</em>
      </span>
    </div>
  );
}

function Head({ loading }: { loading: boolean }) {
  const cur = useCurrent();
  const ws = useData(s => s.ws);
  const scope = useSP(s => s.scope);
  const projects = ws?.projects ?? [];
  const nBoards = projects.reduce((n, p) => n + p.boards.length, 0);
  const title = scope === 'all' ? '全部项目' : scope === 'project' ? cur?.project.name : cur?.board.name;
  const where =
    scope === 'all'
      ? `${projects.length} 个项目 · ${nBoards} 块白板`
      : scope === 'project'
        ? `${cur?.project.key} · ${cur?.project.boards.length ?? 0} 块白板`
        : cur?.board.path;
  return (
    <header className="sp-head">
      <div>
        <span className="sp-head-k">大盘</span>
        <h1>{title}</h1>
        <p>
          {where} · 时区 {TZ}
        </p>
      </div>
      <RangeBar loading={loading} />
    </header>
  );
}

function Empty() {
  return (
    <div className="sp-empty">
      <StatsIcon24 />
      <b>这段时间没有动态</b>
      <span>换一个时间范围看看，或者回到最近 7 天。</span>
      <button className="btn sm" type="button" onClick={() => useSP.getState().set({ kind: '7d' })}>
        看最近 7 天
      </button>
    </div>
  );
}

/* ------------------------------ 总览 ------------------------------ */

function Overview({ s }: { s: Stats }) {
  const scope = useSP(st => st.scope);
  if (scope === 'all') return <AllOverview s={s} />;
  if (scope === 'project') return <ProjectOverview s={s} />;
  return <BoardOverview s={s} />;
}

function BoardOverview({ s }: { s: Stats }) {
  const cur = useCurrent();
  const [open, setOpen] = useState(true);
  if (!cur) return null;
  const { project: p, board: b } = cur;
  const life = s.scope === 'board' && s.path === b.path ? s.life : null;
  const flow = s.scope === 'board' && s.path === b.path ? s.present.statuses : [];
  const total = flow.reduce((n, x) => n + x.n, 0);
  const labels = [...p.labels].sort((a, c) => c.uses - a.uses).slice(0, 8);
  const siblings = p.boards.filter(x => x.path !== b.path);
  const days = Math.max(1, Math.round((s.now - b.createdAt) / DAY));
  const readme = (b.readme ?? '').trim();
  const edit = () => editBoard(b.path, { focus: 'readme' });
  return (
    <section className="sp-card sp-ov" id="sp-overview">
      <header className="sp-ov-head">
        <ProjectAvatar name={p.name} color={p.color} className="sp-ov-av" />
        <div className="sp-ov-title">
          <span className="sp-ov-crumb">
            {p.name}
            {p.desc && <em> · {p.desc}</em>}
          </span>
          <h2>{b.name}</h2>
          <p>{b.desc || <i>这块白板还没有一句话描述</i>}</p>
        </div>
        <div className="sp-ov-meta">
          <span>
            <b className="mono">{b.path}</b>
          </span>
          <span>
            创建于 {fmtDate(b.createdAt)} · 已经 {days} 天
          </span>
        </div>
      </header>

      <div className="sp-ov-grid">
        <div className="sp-ov-readme">
          <div className="sp-ov-sub">
            <span>白板说明</span>
            <span className="sp-sp" />
            {readme && (
              <button className="tl-more" type="button" aria-expanded={open} onClick={() => setOpen(!open)}>
                {open ? '收起' : '展开'}
              </button>
            )}
            <button className="tl-more" type="button" onClick={edit}>
              编辑
            </button>
          </div>
          {!readme ? (
            <div className="sp-ov-blank">
              <p>写这块白板是做什么的、有哪些约定，打开大盘的人第一眼就能看到。</p>
              <button className="btn sm" type="button" onClick={edit}>
                编辑
              </button>
            </div>
          ) : open ? (
            <Markdown src={readme} className="sp-ov-md" />
          ) : (
            <p className="sp-ov-fold">已收起</p>
          )}
        </div>

        <div className="sp-ov-facts">
          <div className="sp-ov-sub">
            <span>创建以来</span>
            <span className="sp-ov-since">{days} 天</span>
          </div>
          {life && (
            <div className="sp-ov-life">
              <div>
                <b>{num(life.cards)}</b>
                <span>张卡片</span>
              </div>
              <div>
                <b>{num(life.finals)}</b>
                <span>已完成</span>
              </div>
              <div>
                <b>{num(life.reworks)}</b>
                <span>次返工</span>
              </div>
              <div>
                <b>{num(life.agents)}</b>
                <span>个 agent 参与</span>
              </div>
              <div>
                <b>{num(Math.round(life.spent / H))}</b>
                <span>小时 agent 工时</span>
              </div>
              <div>
                <b>{life.perDay.toFixed(1)}</b>
                <span>张 / 天 平均完成</span>
              </div>
            </div>
          )}

          <div className="sp-ov-sub">
            <span>现在卡片在哪</span>
            <span className="sp-ov-since">共 {total} 张</span>
          </div>
          <div className="sp-ov-dist" role="img" aria-label="各状态卡片数">
            {flow
              .filter(x => x.n > 0)
              .map(x => (
                <i key={x.key} style={{ flexGrow: x.n, background: statusColor(x.color) }} title={`${x.name} ${x.n} 张`} />
              ))}
          </div>
          <div className="sp-ov-flow">
            {flow.map((x, i) => (
              <span key={x.key} className={cx(!x.n && 'zero')}>
                {i > 0 && <em>→</em>}
                <i style={{ background: statusColor(x.color) }} />
                {x.name}
                <b>{x.n}</b>
              </span>
            ))}
          </div>

          {labels.length > 0 && (
            <>
              <div className="sp-ov-sub">
                <span>标签</span>
                <span className="sp-ov-since">项目共用</span>
              </div>
              <div className="sp-ov-labels">
                {labels.map(l => (
                  <span key={l.id} className="sp-ov-label" style={{ ['--lc' as string]: l.color }}>
                    {l.name}
                    <b>{l.uses}</b>
                  </span>
                ))}
              </div>
            </>
          )}

          {siblings.length > 0 && (
            <>
              <div className="sp-ov-sub">
                <span>同项目的其他白板</span>
              </div>
              <div className="sp-ov-sibs">
                {siblings.map(x => (
                  <button key={x.path} type="button" onClick={() => openBoard(x.path, { stay: true })}>
                    <b>{x.name}</b>
                    <span>{x.desc || x.path}</span>
                    <em>{Object.values(x.counts ?? {}).reduce((n, v) => n + v, 0)} 张</em>
                  </button>
                ))}
              </div>
            </>
          )}
        </div>
      </div>
    </section>
  );
}

/* ---------- 项目 / 全部项目的总览：对比表 ---------- */

interface Row {
  key: string;
  avatar: ReactNode;
  title: string;
  sub: string;
  brief: StatsBrief | undefined;
  onClick: () => void;
}

function CompareTable({ s, rows, unit }: { s: Stats; rows: Row[]; unit: string }) {
  return (
    <div className="sp-table-wrap">
      <table className="sp-table sp-cmp">
        <thead>
          <tr>
            <th>{unit}</th>
            <th className="num" title="现在在可领取状态的张数">
              待办
            </th>
            <th className="num" title="现在在交审状态的张数">
              待复盘
            </th>
            <th>这段时间完成</th>
            <th className="num">交付周期 · 中位</th>
            <th className="num">一次通过率</th>
            <th className="num">返工</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(r => {
            const b = r.brief;
            return (
              <tr key={r.key} onClick={r.onClick} className="sp-cmp-row" tabIndex={0} onKeyDown={e => e.key === 'Enter' && r.onClick()}>
                <td>
                  <span className="sp-cmp-name">
                    {r.avatar}
                    <span>
                      <b>{r.title}</b>
                      <small>{r.sub}</small>
                    </span>
                  </span>
                </td>
                <td className="num">{b?.todo ?? '—'}</td>
                <td className={cx('num', (b?.review ?? 0) > 0 && 'strong')}>{b?.review ?? '—'}</td>
                <td>
                  <span className="sp-cmp-done">
                    <b>{b?.done ?? '—'}</b>
                    {b && <Spark data={b.series.map((v, i) => ({ t: s.series[i]?.t ?? i, v }))} fmt={v => String(v)} label="完成" bucket={s.bucket} height={22} />}
                  </span>
                </td>
                <td className="num">{b?.leadP50 != null ? fmtDur(b.leadP50) : '—'}</td>
                <td className={cx('num', b?.firstPass != null && b.firstPass < 0.75 && 'warn')}>{pct(b?.firstPass)}</td>
                <td className="num">{b?.rework ?? '—'}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function ProjectOverview({ s }: { s: Stats }) {
  const cur = useCurrent();
  const set = useSP(st => st.set);
  if (!cur) return null;
  const p = cur.project;
  const rows: Row[] = p.boards.map(b => ({
    key: b.path,
    avatar: <i className="sp-cmp-dot" style={{ background: p.color }} />,
    title: b.name,
    sub: b.desc || b.path,
    brief: s.boards.find(x => x.key === b.path),
    onClick: () => {
      openBoard(b.path, { stay: true });
      set({ scope: 'board' });
    },
  }));
  return (
    <section className="sp-card sp-ov" id="sp-overview">
      <header className="sp-ov-head">
        <ProjectAvatar name={p.name} color={p.color} className="sp-ov-av" />
        <div className="sp-ov-title">
          <span className="sp-ov-crumb">项目</span>
          <h2>{p.name}</h2>
          <p>{p.desc || <i>这个项目还没有一句话描述</i>}</p>
        </div>
        <div className="sp-ov-meta">
          <span>
            <b className="mono">{p.key}</b>
          </span>
          <span>
            创建于 {fmtDate(p.createdAt)} · {p.boards.length} 块白板 · {p.labels.length} 个标签
          </span>
        </div>
      </header>
      <div className="sp-ov-sub">
        <span>各白板</span>
        <span className="sp-ov-since">点一行看那块白板的大盘</span>
      </div>
      <CompareTable s={s} rows={rows} unit="白板" />
    </section>
  );
}

function AllOverview({ s }: { s: Stats }) {
  const ws = useData(st => st.ws);
  const set = useSP(st => st.set);
  const cur = useCurrent();
  const projects = ws?.projects ?? [];
  const rows: Row[] = projects.map(p => ({
    key: p.key,
    avatar: <ProjectAvatar name={p.name} color={p.color} className="sp-cmp-av" />,
    title: p.name,
    sub: `${p.boards.length} 块白板${p.desc ? ` · ${p.desc}` : ''}`,
    brief: s.projects.find(x => x.key === p.key),
    onClick: () => {
      if (cur?.project.key !== p.key && p.boards[0]) openBoard(p.boards[0].path, { stay: true });
      set({ scope: 'project' });
    },
  }));
  return (
    <section className="sp-card sp-ov" id="sp-overview">
      <div className="sp-ov-sub">
        <span>项目对比</span>
        <span className="sp-ov-since">点一行看那个项目</span>
      </div>
      <CompareTable s={s} rows={rows} unit="项目" />
    </section>
  );
}

/* ------------------------------ 现在 ------------------------------ */

/** 待复盘最久的一张等了这么久就提醒 */
const REVIEW_ALERT = 8 * H;

function NowCard({ s }: { s: Stats }) {
  const { review, todo, doing, stuck } = s.present;
  const late = (review.oldest ?? 0) >= REVIEW_ALERT;
  const many = s.scope !== 'board';
  return (
    <Card title="现在" id="sp-now" className="sp-now" extra={<Note>不随时间范围变 · 有推送就刷新</Note>}>
      <div className="sp-now-grid">
        <div className="sp-now-stats">
          <div className={cx('sp-now-stat', late && 'alert')}>
            <span>待复盘</span>
            <b>
              {review.n}
              <small>张</small>
            </b>
            <em>{review.oldest != null ? `${late ? '⚠ ' : ''}最久的等了 ${fmtDur(review.oldest)}` : '没有等复盘的卡片'}</em>
            <i>已经进了集成分支，还没人复盘</i>
          </div>
          <div className="sp-now-stat">
            <span>进行中</span>
            <b>
              {doing.n}
              <small>个</small>
            </b>
            <em>{doing.n ? doing.holders.join('、') : '没有 agent 在跑'}</em>
          </div>
          <div className="sp-now-stat">
            <span>待办</span>
            <b>
              {todo.n}
              <small>张</small>
            </b>
            <em>{todo.oldest != null ? `最久的等了 ${fmtDur(todo.oldest)}` : '可领取状态里没有卡片'}</em>
          </div>
        </div>
        <div className="sp-stuck">
          <div className="sp-stuck-h">正在卡着的卡 · 按在当前状态停了多久排</div>
          {!stuck.length && <p className="tl-empty">没有卡着的卡片</p>}
          {stuck.map(c => (
            <div className="sp-stuck-row" key={`${c.board}#${c.card}`}>
              <span className="sp-status" style={{ ['--c' as string]: statusColor(c.color) }}>
                <i />
                {c.name}
              </span>
              <span className="sp-stuck-t">
                {many && <span className="sp-tag">{c.board}</span>}
                <em>#{c.card}</em>
                {c.title}
                {(c.holder || c.note) && <small>{c.holder ?? c.note}</small>}
              </span>
              <b>{fmtDur(s.now - c.since)}</b>
            </div>
          ))}
        </div>
      </div>
    </Card>
  );
}

/* ------------------------------ 这段时间：总量 ------------------------------ */

function Delta({ cur, prev, kind, lowerBetter }: { cur: number | null; prev: number | null; kind: 'count' | 'pct' | 'dur'; lowerBetter?: boolean }) {
  const fmt = (v: number) => (kind === 'pct' ? pct(v) : kind === 'dur' ? fmtDur(v) : num(v));
  if (prev == null) return <span className="sp-delta flat">上一周期没有数据</span>;
  if (cur == null)
    return (
      <span className="sp-delta flat">
        <em>上一周期 {fmt(prev)}</em>
      </span>
    );
  const same = kind === 'pct' ? Math.round(cur * 100) === Math.round(prev * 100) : kind === 'dur' ? Math.round((cur - prev) / 60_000) === 0 : cur === prev;
  if (same) return <span className="sp-delta flat">与上一周期持平</span>;
  const d = cur - prev;
  const up = d > 0;
  const good = lowerBetter ? !up : up;
  let txt: string;
  if (kind === 'pct') txt = `${Math.abs(Math.round(cur * 100) - Math.round(prev * 100))} 个百分点`;
  else if (prev === 0) txt = `${up ? '+' : '−'}${kind === 'dur' ? fmtDur(Math.abs(d)) : num(Math.abs(d))}`;
  else txt = `${Math.round((Math.abs(d) / prev) * 100)}%`;
  return (
    <span className={cx('sp-delta', good ? 'good' : 'bad')}>
      <i>{up ? '▲' : '▼'}</i>
      {txt}
      <em>上一周期 {fmt(prev)}</em>
    </span>
  );
}

function Tile({ label, value, unit, sub, hint, children }: { label: string; value: string; unit?: string; sub?: string; hint: string; children: ReactNode }) {
  return (
    <div className="sp-kpi" title={hint}>
      <span className="sp-kpi-l">{label}</span>
      <b>
        {value}
        {unit && <small>{unit}</small>}
      </b>
      {sub && <span className="sp-kpi-sub">{sub}</span>}
      {children}
    </div>
  );
}

function Totals({ s }: { s: Stats }) {
  const { cur, prev } = s;
  const lead = cur.leadP50 != null ? durParts(cur.leadP50) : { v: '—', u: '' };
  const idle = durParts(cur.idle);
  return (
    <div id="sp-totals">
      <div className="sp-kpis six">
        <Tile label="完成" value={num(cur.done)} hint={HINT.done}>
          <Delta cur={cur.done} prev={prev.done} kind="count" />
        </Tile>
        <Tile label="交付周期 · 中位" value={lead.v} unit={lead.u} sub={cur.leadP85 != null ? `P85 ${fmtDur(cur.leadP85)}` : undefined} hint={HINT.lead}>
          <Delta cur={cur.leadP50} prev={prev.leadP50} kind="dur" lowerBetter />
        </Tile>
        <Tile label="一次通过率" value={pct(cur.firstPass)} hint={HINT.firstPass}>
          <Delta cur={cur.firstPass} prev={prev.firstPass} kind="pct" />
        </Tile>
        <Tile label="返工" value={num(cur.rework)} hint={HINT.rework}>
          <Delta cur={cur.rework} prev={prev.rework} kind="count" lowerBetter />
        </Tile>
        <Tile label="领取" value={num(cur.claims)} hint={HINT.claims}>
          <Delta cur={cur.claims} prev={prev.claims} kind="count" />
        </Tile>
        <Tile label="空转" value={idle.v} unit={idle.u} hint={HINT.idle}>
          <Delta cur={cur.idle} prev={prev.idle} kind="dur" lowerBetter />
        </Tile>
      </div>
      <p className="sp-minor">
        其他：新建 <b>{num(cur.created)}</b>（上一周期 {num(prev.created)}）· 累计耗时 <b>{fmtDur(cur.spent)}</b> · 评论 <b>{num(cur.comments)}</b> · 心跳{' '}
        <b>{num(cur.beats)}</b>
      </p>
    </div>
  );
}

function NetFlow({ s }: { s: Stats }) {
  const created = s.cur.created;
  const done = s.cur.done;
  const net = done - created;
  return (
    <Card title="积压净流量" id="sp-net" extra={<Note>新建 vs 完成 · 按{s.bucket === 'hour' ? '小时' : '天'}</Note>}>
      <p className="sp-net-sum">
        新建 <b>{created}</b> · 完成 <b>{done}</b> ·{' '}
        <span className={cx(net > 0 ? 'good' : net < 0 && 'bad')}>{net === 0 ? '积压不变' : net > 0 ? `积压减少 ${net} 张` : `积压增加 ${-net} 张`}</span>
      </p>
      <PairBars data={s.series.map(b => ({ t: b.t, created: b.created, done: b.done }))} bucket={s.bucket} height={170} />
      <div className="tl-legend">
        <span>
          <i className="sp-key b2" />
          新建
        </span>
        <span>
          <i className="sp-key b1" />
          完成
        </span>
      </div>
    </Card>
  );
}

function LeadCard({ s }: { s: Stats }) {
  const { cur } = s;
  const hist = cur.leadHist;
  const max = Math.max(1, ...hist);
  const total = hist.reduce((a, b) => a + b, 0);
  const p50 = cur.leadP50 != null ? binOf(cur.leadP50) : -1;
  const p85 = cur.leadP85 != null ? binOf(cur.leadP85) : -1;
  const wait = cur.waitP50 ?? 0;
  const work = cur.workP50 ?? 0;
  const q = wait + work || 1;
  return (
    <Card title="交付周期" id="sp-lead" extra={<Note>这段时间完成的 {total} 张卡 · 从新建到完成</Note>}>
      <div className="sp-lead-top">
        <div>
          <span>中位</span>
          <b>{cur.leadP50 != null ? fmtDur(cur.leadP50) : '—'}</b>
        </div>
        <div>
          <span>P85</span>
          <b>{cur.leadP85 != null ? fmtDur(cur.leadP85) : '—'}</b>
        </div>
        <div className="sp-split">
          <span>中位拆开看</span>
          <div className="sp-split-bar">
            <i className="q" style={{ width: `${(wait / q) * 100}%` }} />
            <i className="w" style={{ width: `${(work / q) * 100}%` }} />
          </div>
          <small>{cur.waitP50 != null ? `排队等人领 ${fmtDur(wait)} · 领取到完成 ${fmtDur(work)}` : '还没有领取过又完成的卡片'}</small>
        </div>
      </div>
      <div className="sp-hist">
        {hist.map((v, i) => (
          <div className="sp-hist-col" key={i} title={`${LEAD_BINS[i].label}：${v} 张`}>
            <em>{v || ''}</em>
            <i style={{ height: `${(v / max) * 100}%` }} className={cx(v === 0 && 'zero')} />
            <span>{LEAD_BINS[i].label}</span>
            {(i === p50 || i === p85) && <small>{i === p50 && i === p85 ? '中位 · P85' : i === p50 ? '中位' : 'P85'}</small>}
          </div>
        ))}
      </div>
    </Card>
  );
}

function Trend({ s }: { s: Stats }) {
  const { metric, set } = useSP();
  const m = METRICS.find(x => x.key === metric)!;
  return (
    <Card
      title="趋势"
      id="sp-trend"
      extra={
        <div className="sp-metric-tabs">
          {METRICS.map(x => (
            <button key={x.key} type="button" title={x.hint} className={cx(metric === x.key && 'on')} onClick={() => set({ metric: x.key })}>
              {x.label}
            </button>
          ))}
          <span className="sp-bucket">按{s.bucket === 'hour' ? '小时' : '天'}</span>
        </div>
      }
    >
      <TrendBars data={s.series.map(b => ({ t: b.t, v: metricValue(b, metric) }))} bucket={s.bucket} label={m.label} fmt={metricFmt(metric)} />
    </Card>
  );
}

function CalCard({ s }: { s: Stats }) {
  const [ref, W] = useWidth<HTMLDivElement>();
  const weeks = Math.max(8, Math.min(26, Math.floor((W - 44) / 17)));
  return (
    <Card title="日历热力图" id="sp-cal" className="sp-cal-card" extra={<Note>最近 {weeks} 周 · 每天的动态条数 · 框出的是当前时间范围</Note>}>
      <div ref={ref} />
      <CalHeat days={s.dayHeat} from={s.from} to={s.to} maxWeeks={weeks} />
      <div className="sp-card-foot">
        <HeatLegend />
      </div>
    </Card>
  );
}

function HourCard({ s }: { s: Stats }) {
  return (
    <Card title="时段热力图" id="sp-hour" extra={<Note>当前时间范围内 · 星期几 × 几点</Note>}>
      <HourHeat m={s.hourHeat} />
      <div className="sp-card-foot">
        <HeatLegend />
      </div>
    </Card>
  );
}

/** 一次通过率低于它、异常退出率或放弃率达到它时标红 */
const PASS_WARN = 0.75;
const RATE_WARN = 0.08;

function Agents({ s }: { s: Stats }) {
  const max = Math.max(1, ...s.agents.map(a => a.claims));
  if (!s.agents.length) return <p className="tl-empty">这段时间没有领取</p>;
  return (
    <>
      <div className="sp-table-wrap">
        <table className="sp-table">
          <thead>
            <tr>
              <th>agent</th>
              <th className="num">领取</th>
              <th className="num">累计耗时</th>
              <th className="num" title="它最后一次领取后完成的卡片里，一次都没有返工的比例">
                一次通过率
              </th>
              <th className="num" title="心跳中断的次数 ÷ 领取次数">
                异常退出率
              </th>
              <th className="num" title="领取后由它移到需要人处理的状态的次数 ÷ 领取次数">
                放弃率
              </th>
            </tr>
          </thead>
          <tbody>
            {s.agents.map(a => {
              const lost = a.lost / a.claims;
              const gave = a.gaveup / a.claims;
              return (
                <tr key={a.agent}>
                  <td>
                    <span className="sp-agent">
                      <i style={{ background: agentColor(a.agent) }} />
                      {a.agent}
                    </span>
                  </td>
                  <td className="num">
                    <span className="sp-inbar">
                      <i style={{ width: `${(a.claims / max) * 100}%`, background: agentColor(a.agent) }} />
                    </span>
                    {a.claims}
                  </td>
                  <td className="num">{fmtDur(a.spent)}</td>
                  <td className={cx('num', a.firstPass != null && a.firstPass < PASS_WARN && 'warn')}>{pct(a.firstPass)}</td>
                  <td className={cx('num', lost >= RATE_WARN && 'warn')}>{pct(lost)}</td>
                  <td className={cx('num', gave >= RATE_WARN && 'warn')}>{pct(gave)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <p className="sp-foot">红色：一次通过率低于 75%，或异常退出率、放弃率达到 8%。派发启动的按「agent 标识/机器名」合成一行。</p>
    </>
  );
}

function Dwell({ s }: { s: Stats }) {
  const max = Math.max(1, ...s.dwell.map(d => Math.max(d.mean, d.median)));
  if (!s.dwell.length) return <p className="tl-empty">这段时间没有卡片离开过哪个状态</p>;
  return (
    <div className="sp-dwell">
      {s.dwell.map(d => (
        <div className="sp-dwell-row" key={d.key}>
          <span className="sp-dwell-name">
            <i style={{ background: statusColor(d.color) }} />
            {d.name}
          </span>
          <span className="sp-dwell-bar" title={`中位数 ${fmtDur(d.median)}，平均 ${fmtDur(d.mean)}`}>
            <i style={{ width: `${(d.median / max) * 100}%` }} />
            <em style={{ left: `${(d.mean / max) * 100}%` }} />
          </span>
          <span className="sp-dwell-v">
            <b>{fmtDur(d.median)}</b>
            <small>
              平均 {fmtDur(d.mean)} · {d.n} 次
            </small>
          </span>
        </div>
      ))}
      <p className="sp-foot">粗条是中位数，竖线是平均数。按离开这个状态的时间落在区间内统计，终点状态不算。</p>
    </div>
  );
}

/* ------------------------------ 页面 ------------------------------ */

function Report({ s }: { s: Stats }) {
  const many = s.scope !== 'board';
  const board = useData(st => st.ws?.projects.flatMap(p => p.boards).find(b => b.path === s.path));
  const fin = board?.statuses.find(x => x.final);
  const doneColor = fin ? statusColor(fin.color) : 'var(--c-done)';
  return (
    <>
      <Overview s={s} />
      <NowCard s={s} />
      <h3 className="sp-sec-h">这段时间</h3>
      {s.activity === 0 ? (
        <Empty />
      ) : (
        <>
          <Totals s={s} />
          <div className="sp-two">
            <NetFlow s={s} />
            <LeadCard s={s} />
          </div>
          <Trend s={s} />
          <Card title="agent 时间轴" id="sp-agent-tl" extra={<Note>每次领取一段 · 同一个 agent 同时跑几个就叠几行 · 悬停看是哪张卡</Note>}>
            <AgentTimeline runs={s.runs} from={s.from} to={s.to} now={s.now} idle={s.idle} showBoard={many} />
          </Card>
          <div className="sp-two heat">
            <CalCard s={s} />
            <HourCard s={s} />
          </div>
          <Card title="卡片时间线" id="sp-cards-tl" extra={<Note>这段时间完成的 {s.lives.length} 张卡，最近完成的在上面 · 每段是在一个状态里待了多久</Note>}>
            <CardTimeline lives={s.lives} showBoard={many} doneColor={doneColor} />
          </Card>
          <div className="sp-two">
            <Card title="按 agent" id="sp-agents">
              <Agents s={s} />
            </Card>
            <Card title="状态停留" id="sp-dwell">
              <Dwell s={s} />
            </Card>
          </div>
        </>
      )}
    </>
  );
}

export function StatsPage() {
  const { data, loading, error } = useSD();
  useLoader();
  // 打开大盘时默认看顶栏当前的白板
  useEffect(() => useSP.getState().set({ scope: 'board' }), []);
  const q = useQuery();
  return (
    <div className="page page-stats" data-page="stats">
      <div className="sp-page">
        <Head loading={loading && !!data} />
        {data ? (
          <Report s={data} />
        ) : error ? (
          <div className="sp-empty">
            <StatsIcon24 />
            <b>大盘没取到</b>
            <span>{error}</span>
            <button className="btn sm" type="button" onClick={() => q && void load(q)}>
              重试
            </button>
          </div>
        ) : (
          <div className="sp-empty sp-wait">
            <StatsIcon24 />
            <b>正在统计…</b>
          </div>
        )}
      </div>
    </div>
  );
}

/* ------------------------------ 侧栏：范围导航 ------------------------------ */

export function StatsSide() {
  const ws = useData(st => st.ws);
  const cur = useCurrent();
  const { scope, set, kind } = useSP();
  const data = useSD(st => st.data);
  const projects = ws?.projects ?? [];
  const range = KINDS.find(k => k.key === kind)?.label ?? '';
  const briefOf = (list: StatsBrief[] | undefined, key: string) => list?.find(x => x.key === key);
  const Nums = ({ b }: { b: StatsBrief | undefined }) =>
    b ? (
      <span className="sp-nav-n">
        {b.review > 0 && <em title="现在待复盘">{b.review}</em>}
        <b title="这段时间完成">{b.done}</b>
      </span>
    ) : null;
  const pickProject = (p: ProjectInfo) => {
    if (cur?.project.key !== p.key && p.boards[0]) openBoard(p.boards[0].path, { stay: true });
    set({ scope: 'project' });
  };
  const pickBoard = (b: BoardInfo) => {
    openBoard(b.path, { stay: true });
    set({ scope: 'board' });
  };
  return (
    <section className="side-view" data-view="stats">
      <header className="side-head">
        <span className="side-title">大盘</span>
        <span className="side-sub">{kind === 'custom' ? '自定义范围' : `最近 ${range}`}</span>
      </header>
      <div className="side-scroll sp-nav">
        <div className="sp-nav-legend">
          <span>
            <em>n</em>现在待复盘
          </span>
          <span>
            <b>n</b>这段时间完成
          </span>
        </div>
        <button type="button" className={cx('sp-nav-row top', scope === 'all' && 'active')} onClick={() => set({ scope: 'all' })}>
          <span className="sp-nav-ic">
            <StatsIcon24 />
          </span>
          <span className="sp-nav-t">全部项目</span>
          <Nums b={data?.all} />
        </button>
        {projects.map(p => {
          const inP = cur?.project.key === p.key;
          return (
            <div key={p.key} className="sp-nav-group">
              <button type="button" className={cx('sp-nav-row', scope === 'project' && inP && 'active')} onClick={() => pickProject(p)}>
                <ProjectAvatar name={p.name} color={p.color} className="sp-nav-av" />
                <span className="sp-nav-t">{p.name}</span>
                <Nums b={briefOf(data?.projects, p.key)} />
              </button>
              {p.boards.map(b => {
                const busy = b.holders.length > 0;
                return (
                  <button key={b.path} type="button" className={cx('sp-nav-row sub', scope === 'board' && cur?.board.path === b.path && 'active')} onClick={() => pickBoard(b)}>
                    <span className={cx('sp-nav-live', busy && 'on')} title={busy ? '有 agent 正在干活' : undefined} />
                    <span className="sp-nav-t">{b.name}</span>
                    <Nums b={briefOf(data?.boards, b.path)} />
                  </button>
                );
              })}
            </div>
          );
        })}
      </div>
    </section>
  );
}
