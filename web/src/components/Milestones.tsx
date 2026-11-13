/* ==========================================================================
   里程碑 Tab（PRD §10.15）：按版本看当前项目所有白板上的卡片，一个版本一页
   - 版本 = v 开头的标签；发版时 board versions sync 按 git tag 和提交里的「卡片：#ID」反推，卡片带上版本标签，这一版的提交记到服务端
   - 主区：标题行（版本、发版时间、距上一版多久、提交数、完成数）、进度条、各状态张数（做完的版本按标签数）、进度图、
     还没做完的、做了哪些（带这一版里的提交号）、没连到卡片的提交
   - 侧栏（Workbench 的 MilestonesSide）：未发版、版本从新到旧（最近 6 个，更早的收起）、没有版本；J / K 上下翻
   - 只读：点一行打开卡片详情（别的白板上的卡片先切到那块白板，不离开这一页）
   数据：GET /api/milestones?project=<项目>，切项目立即取，有推送（工作区 rev 变了）等 1.2 秒再取
   ========================================================================== */
import { useEffect, useMemo, useRef, useState } from 'react';
import { create } from 'zustand';
import { api } from '../lib/api';
import {
  cardRef,
  cardsOf,
  commitsByCard,
  defaultPage,
  gap,
  looseCommits,
  matchCard,
  pagesOf,
  roleOf,
  sortCards,
  startOf,
  summarize,
  versionOf,
  type MilestoneCard,
  type Milestones,
  type MsKey,
  type ReleaseCommit,
  type Summary,
} from '../lib/milestones';
import { statusColor } from '../lib/schema';
import type { Label, ProjectInfo, Status } from '../lib/types';
import { cx, dateTime, reduceMotion, since } from '../lib/util';
import { jumpDetail, openDetail, setPath, useCurrent, useData } from '../store/data';
import { overlayOpen } from '../store/ui';
import { Avatar, LabelChip, Title, useNow } from './bits';
import { Icon } from './Icon';
import { Skeleton } from './Skeleton';

/* ------------------------------ 数据 ------------------------------ */

interface MsState {
  project: string | null;
  data: Milestones | null;
  loading: boolean;
  error: string | null;
  /** 正在看的一页；null = 默认页（未发版有卡片就是它，否则最新的版本） */
  sel: MsKey | null;
  /** 侧栏里更早的版本展开了没有 */
  older: boolean;
  /** 筛选：标题、#编号 */
  q: string;
}
export const useMs = create<MsState>(() => ({ project: null, data: null, loading: false, error: null, sel: null, older: false, q: '' }));
const setMs = (p: Partial<MsState>) => useMs.setState(p);

let seq = 0;
async function load(project: string) {
  const my = ++seq;
  setMs({ loading: true });
  try {
    const data = await api.milestones(project);
    if (my === seq && useMs.getState().project === project) setMs({ data, loading: false, error: null });
  } catch (e) {
    if (my === seq) setMs({ loading: false, error: (e as Error).message });
  }
}

/** 切项目立即取（清掉上一个项目的数据和选中的页）；有推送时等 1.2 秒再取，连着的几次推送合成一次 */
function useLoader(project: string | undefined) {
  const rev = useData(s => s.ws?.rev);
  useEffect(() => {
    if (!project) return;
    if (useMs.getState().project !== project) {
      setMs({ project, data: null, error: null, sel: null, older: false });
      void load(project);
      return;
    }
    const t = setTimeout(() => void load(project), 1200);
    return () => clearTimeout(t);
  }, [project, rev]);
}

/** 当前页：选中的不在了（标签删掉、改名不再是版本）就回到默认页 */
function usePage(): { data: Milestones | null; page: MsKey | null; pages: MsKey[] } {
  const data = useMs(s => s.data);
  const sel = useMs(s => s.sel);
  return useMemo(() => {
    if (!data) return { data, page: null, pages: [] };
    const pages = pagesOf(data);
    const page = sel != null && pages.includes(sel) ? sel : defaultPage(data);
    return { data, page, pages };
  }, [data, sel]);
}

/** 卡片所在的状态（按白板找） */
function useStatusOf(project: ProjectInfo | undefined) {
  return useMemo(() => {
    const flows = new Map<string, Status[]>((project?.boards ?? []).map(b => [b.path, b.statuses]));
    return (c: MilestoneCard) => flows.get(c.board)?.find(s => s.key === c.column);
  }, [project]);
}

/** 打开卡片详情：别的白板上的先切过去（不离开里程碑 Tab） */
async function openMsCard(path: string, id: number) {
  if (path !== useData.getState().path) await setPath(path);
  const open = useData.getState().detailId;
  if (open != null && open !== id) await jumpDetail(id);
  else await openDetail(id);
}

/** J / K、上下按钮：在页之间走（到头就停） */
function step(pages: MsKey[], page: MsKey | null, d: number) {
  const i = page == null ? -1 : pages.indexOf(page);
  const next = pages[Math.max(0, Math.min(pages.length - 1, i + d))];
  if (next == null) return;
  const versions = pages.filter((p): p is number => typeof p === 'number');
  // 翻到收起的老版本时把它们展开
  setMs({ sel: next, ...(typeof next === 'number' && versions.indexOf(next) >= RECENT ? { older: true } : {}) });
}

/** 侧栏默认露出几个版本，更早的收起 */
const RECENT = 6;

/* ------------------------------ 主区 ------------------------------ */

export function MilestonesPage() {
  const cur = useCurrent();
  const project = cur?.project;
  useLoader(project?.key);
  const { data, page, pages } = usePage();
  const error = useMs(s => s.error);
  const q = useMs(s => s.q);
  const now = useNow(60000);
  const statusOf = useStatusOf(project);
  const scroll = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scroll.current?.scrollTo({ top: 0 });
  }, [page]);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.closest?.('input, textarea, select, [contenteditable]') || e.metaKey || e.ctrlKey || e.altKey || overlayOpen() || useData.getState().detailId != null) return;
      const k = e.key.toLowerCase();
      if (k !== 'j' && k !== 'k') return;
      e.preventDefault();
      step(pages, page, k === 'j' ? 1 : -1);
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [pages, page]);

  if (!data)
    return (
      <div className="page page-ms">
        <div className="msp-scroll">
          <div className="msp-wrap">{error ? <div className="msp-empty">{error}</div> : <PageSkeleton />}</div>
        </div>
      </div>
    );

  if (page == null)
    return (
      <div className="page page-ms">
        <div className="msp-scroll">
          <div className="msp-wrap">
            <Empty />
          </div>
        </div>
      </div>
    );

  return (
    <div className="page page-ms">
      <div className="msp-scroll" ref={scroll}>
        <div className="msp-wrap" key={String(page)}>
          <VersionView data={data} page={page} pages={pages} project={project!} statusOf={statusOf} q={q} now={now} />
        </div>
      </div>
    </div>
  );
}

function VersionView({
  data,
  page,
  pages,
  project,
  statusOf,
  q,
  now,
}: {
  data: Milestones;
  page: MsKey;
  pages: MsKey[];
  project: ProjectInfo;
  statusOf: (c: MilestoneCard) => Status | undefined;
  q: string;
  now: number;
}) {
  const v = versionOf(data, page);
  const all = useMemo(() => cardsOf(data, page), [data, page]);
  const cards = useMemo(() => all.filter(matchCard(q)), [all, q]);
  const s = useMemo(() => summarize(all, statusOf), [all, statusOf]);
  const sorted = useMemo(() => sortCards(cards, statusOf), [cards, statusOf]);
  const open = sorted.filter(c => roleOf(statusOf(c)) !== 'done');
  const done = sorted.filter(c => roleOf(statusOf(c)) === 'done');
  const commits = useMemo(() => commitsByCard(v), [v]);
  const loose = looseCommits(v);
  const released = v?.releasedAt != null;
  const start = startOf(data, page);
  const i = pages.indexOf(page);
  const label = v ? project.labels.find(l => l.id === v.label) ?? { id: v.label, name: v.name, color: v.color } : null;
  const boardName = (c: MilestoneCard) => (project.boards.length > 1 ? project.boards.find(b => b.path === c.board)?.name ?? c.board : null);

  return (
    <>
      <div className="msp-crumb">
        <span>{project.name}</span>
        <Icon name="chevRight" />
        <span>里程碑</span>
        <span className="msp-nav">
          <button type="button" className="icon-btn" aria-label="新一点的版本" data-tip="新一点的版本（K）" disabled={i <= 0} onClick={() => step(pages, page, -1)}>
            <Icon name="chevUp" />
          </button>
          <button type="button" className="icon-btn" aria-label="更早的版本" data-tip="更早的版本（J）" disabled={i >= pages.length - 1} onClick={() => step(pages, page, 1)}>
            <Icon name="chevDown" />
          </button>
        </span>
      </div>

      <header className="msp-h">
        <span className="msp-name">
          {label ? <LabelChip label={label} /> : <span className={cx('msp-pseudo', page === 'next' && 'next')}>{page === 'next' ? '未发版' : '没有版本'}</span>}
        </span>
        <ReleaseTag page={page} released={released} left={s.total - s.done} />
        <span className="msp-num">
          <b className="mono">{s.done}</b>
          <span className="mono">/{s.total}</span>
          {s.total > 0 && <em className="mono">{s.pct}%</em>}
        </span>
      </header>
      <p className="msp-meta">{metaText(data, page, s, start)}</p>

      {s.total > 0 && <Bar s={s} />}
      <div className="msp-counts">{released && s.total > 0 && s.done === s.total ? <LabelMix cards={all} labels={project.labels} /> : <Counts s={s} />}</div>

      {s.total > 0 && page !== 'none' && <Burn cards={all} statusOf={statusOf} now={now} start={start} end={v?.releasedAt ?? null} live={page === 'next' || !released} />}

      {open.length > 0 && (
        <section className="msp-sec">
          <h2>
            {released ? '发版了但还没验收' : '还没做完'} <span className="mono">{open.length}</span>
          </h2>
          <div className="msp-rows">
            {open.map(c => (
              <Row key={`${c.board}#${c.id}`} c={c} s={statusOf(c)} labels={project.labels} commits={commits.get(cardRef(c))} boardName={boardName(c)} now={now} />
            ))}
          </div>
        </section>
      )}
      {done.length > 0 && (
        <section className="msp-sec">
          <h2>
            {page === 'none' ? '做完了、没有版本' : '做了哪些'} <span className="mono">{done.length}</span>
          </h2>
          <div className="msp-rows">
            {done.map(c => (
              <Row key={`${c.board}#${c.id}`} c={c} s={statusOf(c)} labels={project.labels} commits={commits.get(cardRef(c))} boardName={boardName(c)} now={now} />
            ))}
          </div>
        </section>
      )}
      {cards.length === 0 && <div className="msp-empty">{q ? '没有匹配的卡片' : emptyText(page, released)}</div>}
      {loose.length > 0 && <Loose commits={loose} now={now} />}
    </>
  );
}

function emptyText(page: MsKey, released: boolean) {
  if (page === 'next') return '上次发版之后还没有开始做的卡片';
  if (released) return '这个版本的提交都没有写「卡片：#ID」，连不到卡片';
  return '还没有卡片带着这个版本标签';
}

/** 已发版 / 还没打 tag；发了版还有卡片没到终点时标出来 */
function ReleaseTag({ page, released, left }: { page: MsKey; released: boolean; left: number }) {
  if (page === 'none') return null;
  if (!released)
    return (
      <span className="msp-state" data-s="next">
        还没打 tag
      </span>
    );
  return (
    <span className="msp-state" data-s={left ? 'warn' : 'done'}>
      {left ? `已发版 · ${left} 张没验收` : '已发版'}
    </span>
  );
}

function metaText(data: Milestones, page: MsKey, s: Summary, start: number | null) {
  if (page === 'none') return '最近一次发版之前就做完、却没有版本标签的卡片：提交里没写「卡片：#ID」，或者不是改代码的卡片';
  if (page === 'next') {
    const last = data.versions.find(v => v.releasedAt != null);
    const doing = s.byStatus.filter(x => x.role === 'doing').reduce((a, x) => a + x.n, 0);
    return [last ? `${last.name} 之后` : '还没有发过版', `${s.total} 张已经开始做`, doing ? `${doing} 张正在做` : ''].filter(Boolean).join(' · ');
  }
  const v = versionOf(data, page);
  if (!v) return '';
  if (v.releasedAt == null) return '手动建的版本标签，还没有发版记录（打 tag 后运行 board versions sync）';
  return [`${dateTime(v.releasedAt)} 发版`, start != null ? `距上一版 ${gap(v.releasedAt - start)}` : '第一个版本', `${v.commits.length} 个提交`].join(' · ');
}

/** 分段进度条：按状态归类，做完的在左 */
function Bar({ s }: { s: Summary }) {
  return (
    <div className="msp-bar" role="img" aria-label={`完成 ${s.done} / ${s.total}`}>
      {s.byStatus.map(x => (
        <i key={x.name} data-role={x.role} style={{ flexGrow: x.n, background: statusColor(x.color) }} title={`${x.name} ${x.n}`} />
      ))}
    </div>
  );
}

function Counts({ s }: { s: Summary }) {
  return (
    <span className="msp-cnts">
      {s.byStatus.map(x => (
        <span key={x.name} className="msp-cnt">
          <i style={{ background: statusColor(x.color) }} data-role={x.role} />
          {x.name}
          <b className="mono">{x.n}</b>
        </span>
      ))}
    </span>
  );
}

/** 做完的版本：按其他标签数一数，比「已完成 N」更能看出这一版做了什么 */
function LabelMix({ cards, labels }: { cards: MilestoneCard[]; labels: Label[] }) {
  const n = new Map<number, number>();
  let bare = 0;
  for (const c of cards) {
    const others = c.labels.filter(id => labels.some(l => l.id === id && !/^v\d/i.test(l.name)));
    if (!others.length) bare++;
    for (const id of others) n.set(id, (n.get(id) ?? 0) + 1);
  }
  const rows = labels.filter(l => n.has(l.id)).sort((a, b) => n.get(b.id)! - n.get(a.id)!);
  return (
    <span className="msp-cnts">
      {rows.map(l => (
        <span key={l.id} className="msp-mix">
          <LabelChip label={l} />
          <b className="mono">{n.get(l.id)}</b>
        </span>
      ))}
      {bare > 0 && (
        <span className="msp-mix bare">
          没有其他标签<b className="mono">{bare}</b>
        </span>
      )}
    </span>
  );
}

/** 一行卡片：状态点、编号、标题、白板（项目有多块白板时）、这一版里的提交号、其他标签、做完多久了 / 谁在做 / 状态 */
function Row({
  c,
  s,
  labels,
  commits,
  boardName,
  now,
}: {
  c: MilestoneCard;
  s: Status | undefined;
  labels: Label[];
  commits: ReleaseCommit[] | undefined;
  /** 项目有多块白板时写出卡片在哪块白板上 */
  boardName: string | null;
  now: number;
}) {
  const role = roleOf(s);
  const others = labels.filter(l => c.labels.includes(l.id) && !/^v\d/i.test(l.name));
  return (
    <button type="button" className={cx('msp-row', role === 'done' && 'is-done')} onClick={() => void openMsCard(c.board, c.id)}>
      <i className="msp-dot" data-role={role} style={{ ['--sc' as string]: statusColor(s?.color ?? 'todo') }} title={s?.name} />
      <span className="msp-id mono">#{c.id}</span>
      <span className="msp-title">
        <Title text={c.title} />
      </span>
      <span className="msp-tags">
        {boardName && <span className="msp-board">{boardName}</span>}
        {commits && commits.length > 0 && (
          <span className="msp-sha mono" title={commits.map(x => `${x.h}  ${x.s}`).join('\n')}>
            {commits[0].h}
            {commits.length > 1 && <em>+{commits.length - 1}</em>}
          </span>
        )}
        {others.map(l => (
          <LabelChip key={l.id} label={l} />
        ))}
      </span>
      <span className="msp-when">
        {role === 'done' && c.movedAt ? (
          <span title={dateTime(c.movedAt)}>{since(c.movedAt, now)}</span>
        ) : c.holder ? (
          <span className="msp-holder">
            <Avatar actor={{ kind: 'agent', name: c.holder }} className="msp-av" />
            <span className="mono">{c.holder}</span>
          </span>
        ) : (
          s?.name ?? c.column
        )}
      </span>
    </button>
  );
}

/** 这一版里没写「卡片：#ID」的提交：连不到卡片，收起来放在最后 */
function Loose({ commits, now }: { commits: ReleaseCommit[]; now: number }) {
  const [open, setOpen] = useState(false);
  return (
    <section className="msp-sec msp-loose">
      <button type="button" className="msp-loose-h" aria-expanded={open} onClick={() => setOpen(!open)}>
        <Icon name={open ? 'chevDown' : 'chevRight'} />
        没连到卡片的提交 <span className="mono">{commits.length}</span>
      </button>
      {open && (
        <div className="msp-rows">
          {commits.map(c => (
            <div key={c.h} className="msp-row msp-commit">
              <i className="msp-dot" data-role="commit" />
              <span className="msp-id mono">{c.h}</span>
              <span className="msp-title">{c.s}</span>
              <span />
              <span className="msp-when" title={dateTime(c.at)}>
                {since(c.at, now)}
              </span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

/* ------------------------------ 进度图 ------------------------------ */

/** 加进来的（建卡）和做完的（到终点）两条阶梯线；横轴从上一个发了版的版本画到发版（还没发版的画到现在），起点之前的直接算进起点 */
function Burn({
  cards,
  statusOf,
  now,
  start,
  end,
  live,
}: {
  cards: MilestoneCard[];
  statusOf: (c: MilestoneCard) => Status | undefined;
  now: number;
  start: number | null;
  end: number | null;
  live: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [w, setW] = useState(640);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setW(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const H = 132;
  const pad = { l: 28, r: 12, t: 10, b: 22 };
  const created = cards.map(c => c.createdAt);
  const doneAt = cards.filter(c => roleOf(statusOf(c)) === 'done').map(c => c.movedAt ?? c.createdAt);
  const first = Math.min(...created);
  const t1 = live ? now : end ?? Math.max(now, ...doneAt);
  const t0 = start != null ? Math.max(start, Math.min(first, t1 - 3600e3)) : first;
  const span = Math.max(t1 - t0, 3600e3);
  const x = (t: number) => pad.l + ((Math.min(Math.max(t, t0), t1) - t0) / span) * (w - pad.l - pad.r);
  const y = (n: number) => pad.t + (1 - n / cards.length) * (H - pad.t - pad.b);
  const steps = (ts: number[]) => {
    const sorted = [...ts].sort((a, b) => a - b);
    let n = sorted.filter(t => t <= t0).length;
    let d = `M${x(t0)},${y(n)}`;
    for (const t of sorted.filter(t => t > t0)) d += `H${x(t)}V${y(++n)}`;
    return d + `H${x(t1)}`;
  };
  const scope = steps(created);
  const done = steps(doneAt);
  const day = 86400e3;
  const ticks: number[] = [];
  const d0 = new Date(t0);
  d0.setHours(0, 0, 0, 0);
  for (let d = d0.getTime() + day; d < t1; d += day) if (x(d) - x(t0) > 70 && x(t1) - x(d) > 70) ticks.push(d);
  const md = (t: number) => {
    const d = new Date(t);
    return `${d.getMonth() + 1}-${String(d.getDate()).padStart(2, '0')}`;
  };
  const hm = (t: number) => {
    const d = new Date(t);
    return `${md(t)} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };
  return (
    <section className="msp-burn" ref={ref}>
      <div className="msp-burn-h">
        <span>进度</span>
        <span className="msp-lg">
          <i className="scope" />
          加进来的
        </span>
        <span className="msp-lg">
          <i className="done" />
          做完的
        </span>
      </div>
      <svg width={w} height={H} viewBox={`0 0 ${w} ${H}`} role="img" aria-label={`加进来 ${cards.length} 张，做完 ${doneAt.length} 张`}>
        <line x1={pad.l} x2={w - pad.r} y1={y(0)} y2={y(0)} className="axis" />
        {ticks.map(t => (
          <g key={t}>
            <line x1={x(t)} x2={x(t)} y1={pad.t} y2={y(0)} className="grid" />
            <text x={x(t)} y={H - 6} textAnchor="middle">
              {md(t)}
            </text>
          </g>
        ))}
        <text x={pad.l} y={H - 6} textAnchor="start" className="edge">
          {hm(t0)}
        </text>
        <text x={w - pad.r} y={H - 6} textAnchor="end" className="edge">
          {live ? '现在' : hm(t1)}
        </text>
        <text x={pad.l - 6} y={y(cards.length) + 4} textAnchor="end">
          {cards.length}
        </text>
        <text x={pad.l - 6} y={y(0) + 4} textAnchor="end">
          0
        </text>
        <path d={`${done}V${y(0)}H${x(t0)}Z`} className="done-area" />
        <path d={scope} className="scope" />
        <path d={done} className={cx('done', !reduceMotion() && 'draw')} pathLength={1} />
        {!live && end != null ? (
          <g className="rel">
            <line x1={x(end)} x2={x(end)} y1={pad.t} y2={y(0)} />
            <text x={x(end) - 4} y={pad.t + 8} textAnchor="end">
              发版
            </text>
          </g>
        ) : (
          <line x1={x(t1)} x2={x(t1)} y1={pad.t} y2={y(0)} className="now" />
        )}
      </svg>
    </section>
  );
}

/* ------------------------------ 空状态、加载中 ------------------------------ */

function Empty() {
  return (
    <div className="msp-intro">
      <h1>里程碑</h1>
      <p>这个项目还没有版本。版本是 v 开头的标签（例如 v0.1.0）：</p>
      <ul>
        <li>
          发版打 tag 后，在代码仓库里运行 <code>board versions sync</code>：按 git 历史反推，上一个 tag 之后的提交里写了「卡片：项目/白板#ID」的卡片，带上这个 tag 的标签。
        </li>
        <li>也可以手动建一个 v 开头的标签，给计划放进这一版的卡片打上。</li>
      </ul>
    </div>
  );
}

function PageSkeleton() {
  return (
    <div aria-busy="true" aria-label="正在加载里程碑">
      <Skeleton w={160} h={12} />
      <div className="msp-sk-h">
        <Skeleton w={120} h={34} r={6} />
        <Skeleton w={70} h={22} r={999} />
      </div>
      <Skeleton w="46%" h={12} />
      <Skeleton w="100%" h={8} r={999} style={{ marginTop: 16 }} />
      <Skeleton w="100%" h={150} r={13} style={{ marginTop: 18 }} />
      <Skeleton w="100%" h={240} r={13} style={{ marginTop: 26 }} />
    </div>
  );
}

/* ------------------------------ 侧栏 ------------------------------ */

/** 侧栏标题旁的数：版本数 */
export function useMsCount() {
  return useMs(s => s.data?.versions.length ?? null);
}

/** 侧栏里的页列表（Workbench 的 MilestonesSide 放在筛选框下面） */
export function MilestonesSideParts() {
  const cur = useCurrent();
  const project = cur?.project;
  const { data, page } = usePage();
  const older = useMs(s => s.older);
  const statusOf = useStatusOf(project);
  if (!data)
    return (
      <div className="msp-side" aria-hidden="true">
        {Array.from({ length: 6 }, (_, i) => (
          <div key={i} className="msp-item">
            <Skeleton w={[56, 64, 52, 60, 58, 54][i]} h={20} r={6} />
            <Skeleton w={70} h={10} />
          </div>
        ))}
      </div>
    );
  const pages = pagesOf(data);
  const versions = data.versions;
  const recent = versions.slice(0, RECENT);
  const rest = versions.slice(RECENT);
  // 选中的老版本一直露出来
  const restShown = older ? rest : rest.filter(v => v.label === page);
  const left = (key: MsKey) => cardsOf(data, key).filter(c => roleOf(statusOf(c)) !== 'done').length;
  const row = (v: (typeof versions)[number]) => {
    const n = cardsOf(data, v.label).length;
    const l = left(v.label);
    const label = project?.labels.find(x => x.id === v.label) ?? { id: v.label, name: v.name, color: v.color };
    return (
      <button key={v.label} type="button" className={cx('msp-item', page === v.label && 'on')} aria-current={page === v.label ? 'page' : undefined} onClick={() => setMs({ sel: v.label })}>
        <LabelChip label={label} />
        {v.releasedAt != null && l > 0 && <i className="msp-warn" title={`${l} 张没验收`} />}
        <span className="msp-date mono">{v.releasedAt != null ? hmShort(v.releasedAt) : '还没打 tag'}</span>
        <span className="msp-n mono">{n} 张</span>
      </button>
    );
  };
  return (
    <div className="msp-side">
      {pages.includes('next') && (
        <button type="button" className={cx('msp-item', page === 'next' && 'on')} aria-current={page === 'next' ? 'page' : undefined} onClick={() => setMs({ sel: 'next' })}>
          <span className="msp-pseudo next sm">未发版</span>
          <span className="msp-date">{versions.find(v => v.releasedAt != null)?.name ?? ''} 之后</span>
          <span className="msp-n mono">{cardsOf(data, 'next').length} 张</span>
        </button>
      )}
      {versions.length > 0 && <div className="msp-side-h">版本</div>}
      {recent.map(row)}
      {restShown.map(row)}
      {rest.length > 0 && (
        <button type="button" className="msp-item msp-older" onClick={() => setMs({ older: !older })}>
          <Icon name={older ? 'chevUp' : 'chevDown'} />
          {older ? '收起更早的版本' : `更早的 ${rest.length} 个版本`}
        </button>
      )}
      {pages.includes('none') && (
        <button type="button" className={cx('msp-item', page === 'none' && 'on')} aria-current={page === 'none' ? 'page' : undefined} onClick={() => setMs({ sel: 'none' })}>
          <i className="msp-hollow" />
          <span className="msp-plain">没有版本</span>
          <span className="msp-n mono">{cardsOf(data, 'none').length} 张</span>
        </button>
      )}
    </div>
  );
}

/** 侧栏里的发版时间：11-12 10:52 */
function hmShort(t: number) {
  const d = new Date(t);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getMonth() + 1}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 侧栏筛选框写进这里 */
export const setMsFilter = (q: string) => setMs({ q });
export const useMsFilter = () => useMs(s => s.q);
