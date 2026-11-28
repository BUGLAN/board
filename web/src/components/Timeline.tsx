/* ==========================================================================
   项目 Tab 的主内容区：提交时间轴（PRD §10.4）
   - 一个 git 提交一栏，从新到旧往下翻（无限滚动：快到底时多画一段）；提交说明里写了「卡片：…」的，栏里挂着卡片，
     点开卡片详情（不离开这一页）；没连到卡片的提交只显示提交说明，不能点
   - 按版本分段：每个版本一种颜色（时间轴线、节点、段头），段头滚动时吸顶；右栏是版本索引，点了跳过去
   - 最上面是「未发版」：还没发版的提交（board land 推完、board versions sync 时写进服务端），和它们上面还没有提交连着的卡片
   - 范围：项目跟着当前白板走（侧栏点项目、顶栏切换器都一样）；侧栏点白板时只看连到这块白板卡片的提交
   数据：GET /api/milestones?project=<项目>（和里程碑页同一份），切项目立即取，有推送（工作区 rev 变了）等 1.2 秒再取
   ========================================================================== */
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { openCardAt } from '../lib/actions';
import { api } from '../lib/api';
import { cardRef, roleOf, statusLookup, type MilestoneCard, type Milestones, type ReleaseCommit } from '../lib/milestones';
import { statusColor } from '../lib/schema';
import { dayLabel, gapText, rowsThrough, segmentsOf, summary, takeRows, withDays, type Seg } from '../lib/timeline';
import type { Label, Status } from '../lib/types';
import { cx, dateTime, hhmm, reduceMotion } from '../lib/util';
import { useData } from '../store/data';
import { useTlScope } from '../store/timeline';
import { AgentAvatar, LabelChip, Title } from './bits';
import { Icon } from './Icon';
import { Skeleton } from './Skeleton';

/* ------------------------------ 数据 ------------------------------ */

/** 每个项目上一次取到的数据：切回来先画它，再取新的 */
const cache = new Map<string, Milestones>();

function useTimelineData(project: string | null) {
  const rev = useData(s => s.ws?.rev);
  const [data, setData] = useState<Milestones | null>(() => (project ? cache.get(project) ?? null : null));
  const [error, setError] = useState<string | null>(null);
  const shown = useRef(project);
  /** 这个组件上一次取的是哪个项目：同一个项目再取（有推送）才等 */
  const fetched = useRef<string | null>(null);
  useEffect(() => {
    if (!project) return;
    let live = true;
    if (shown.current !== project) {
      shown.current = project;
      setData(cache.get(project) ?? null);
      setError(null);
    }
    // 进来、切项目立即取（先画上次取到的）；有推送时等一会儿，连着的几次推送合成一次。取的时候又有推送就丢掉这次的结果
    const delay = fetched.current === project ? 1200 : 0;
    fetched.current = project;
    const t = setTimeout(
      () =>
        api
          .milestones(project)
          .then(d => {
            cache.set(project, d);
            if (live) {
              setData(d);
              setError(null);
            }
          })
          .catch(e => live && setError((e as Error).message)),
      delay,
    );
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [project, rev]);
  return { data: shown.current === project ? data : null, error };
}

/* ------------------------------ 页面 ------------------------------ */

/** 一次多画多少行（往下翻到底再加） */
const CHUNK = 40;

export function TimelinePage() {
  const ws = useData(s => s.ws);
  const scope = useTlScope();
  const project = ws?.projects.find(p => p.key === scope.project);
  const { data, error } = useTimelineData(scope.project);
  const statusOf = useMemo(() => statusLookup(project?.boards ?? []), [project]);
  const segs = useMemo(() => (data ? segmentsOf(data, scope.board, statusOf) : []), [data, scope.board, statusOf]);
  const cards = useMemo(() => new Map((data?.cards ?? []).map(c => [cardRef(c), c])), [data]);
  const [limit, setLimit] = useState(CHUNK);
  const [active, setActive] = useState<string | null>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const sentinel = useRef<HTMLDivElement>(null);
  const total = segs.reduce((a, s) => a + s.commits.length + s.cards.length, 0);
  const more = limit < total;

  // 换范围：回到顶上，从头画
  useLayoutEffect(() => {
    if (scroll.current) scroll.current.scrollTop = 0;
    setLimit(CHUNK);
  }, [scope.project, scope.board]);

  // 无限滚动：哨兵进到视口下方 600px 内就多画一段
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !more) return;
    const io = new IntersectionObserver(es => es[0]?.isIntersecting && setLimit(l => l + CHUNK), { root: scroll.current, rootMargin: '0px 0px 600px 0px' });
    io.observe(el);
    return () => io.disconnect();
  }, [segs, more]);

  // 正在看哪个版本：顶边已经滚过去的最后一段（段头吸顶的那一段）
  useEffect(() => {
    const el = scroll.current;
    if (!el) return;
    let raf = 0;
    const on = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        const top = el.getBoundingClientRect().top + 8;
        let cur: string | null = null;
        for (const s of el.querySelectorAll<HTMLElement>('.gt-seg')) {
          if (s.getBoundingClientRect().top <= top) cur = s.dataset.seg ?? null;
          else break;
        }
        setActive(cur ?? segs[0]?.key ?? null);
      });
    };
    on();
    el.addEventListener('scroll', on, { passive: true });
    return () => {
      el.removeEventListener('scroll', on);
      cancelAnimationFrame(raf);
    };
  }, [segs, limit]);

  /** 跳到一个版本：还没画到的先画出来 */
  const jump = (key: string) => {
    const need = rowsThrough(segs, key);
    if (need > limit) setLimit(need + CHUNK);
    requestAnimationFrame(() => {
      const sc = scroll.current;
      const el = sc?.querySelector<HTMLElement>(`.gt-seg[data-seg="${key}"]`);
      if (!el || !sc) return;
      const top = el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop;
      sc.scrollTo({ top, behavior: reduceMotion() ? 'auto' : 'smooth' });
    });
  };

  /** 看整个项目、项目有多块白板时，卡片上写它在哪块白板 */
  const boardName = (path: string) => (project && project.boards.length > 1 && !scope.board ? project.boards.find(b => b.path === path)?.name ?? null : null);
  const boardInfo = scope.board ? project?.boards.find(b => b.path === scope.board) : null;
  const sum = summary(segs);

  return (
    <div className="page page-tl">
      <div className="gt-scroll" ref={scroll}>
        <div className="gt-wrap">
          {/* 页头和里程碑页同一套：项目 › 白板，右边是总数 */}
          <div className="msp-crumb gt-crumb">
            {project ? (
              <>
                <span>{project.name}</span>
                <Icon name="chevRight" />
                <span>{boardInfo?.name ?? '全部白板'}</span>
              </>
            ) : (
              <Skeleton w={160} h={12} />
            )}
            {data && sum && <span className="gt-sum">{sum}</span>}
          </div>
          <div className="gt-main">
            {!data ? (
              error ? <div className="gt-empty">{error}</div> : <TimelineSkeleton />
            ) : segs.length === 0 ? (
              <div className="gt-empty">
                <b>{scope.board ? '这块白板还没有发版记录' : '还没有发版记录'}</b>
                <span>
                  在仓库里打 tag 后运行 <code>board versions sync</code>
                </span>
              </div>
            ) : (
              takeRows(segs, limit).map(({ seg, take }) =>
                take > 0 ? <Segment key={seg.key} seg={seg} take={take} cards={cards} statusOf={statusOf} labels={project?.labels ?? []} boardName={boardName} /> : null,
              )
            )}
            {data && more && <div className="gt-more" ref={sentinel} />}
            {data && segs.length > 0 && !more && <div className="gt-end" aria-hidden="true" />}
          </div>
          {segs.length > 1 && <VersionIndex segs={segs} active={active} onJump={jump} />}
        </div>
      </div>
    </div>
  );
}

/* ------------------------------ 一段（一个版本） ------------------------------ */

function Segment({
  seg,
  take,
  cards,
  statusOf,
  labels,
  boardName,
}: {
  seg: Seg;
  take: number;
  cards: Map<string, MilestoneCard>;
  statusOf: (c: MilestoneCard) => Status | undefined;
  labels: Label[];
  boardName: (path: string) => string | null;
}) {
  const linked = new Set(seg.commits.flatMap(c => c.cards));
  // 这一版连到的卡片里还没到终点的：发了版还没验收
  const left = [...linked].filter(r => {
    const c = cards.get(r);
    return c && roleOf(statusOf(c)) !== 'done';
  }).length;
  // 未发版：还没有提交的卡片画在提交上面（比哪个提交都新），先画它们
  const solo = seg.cards.slice(0, take);
  const commits = seg.commits.slice(0, Math.max(0, take - seg.cards.length));
  return (
    <section className={cx('gt-seg', seg.kind === 'next' && 'is-next')} data-seg={seg.key} style={{ ['--vc' as string]: seg.color }}>
      <header className="gt-vh">
        <i className="gt-vdot" />
        <div className="gt-vbody">
          <span className="gt-vname">{seg.name}</span>
          {seg.kind === 'version' ? (
            <>
              <span className="gt-vdate" title={dateTime(seg.releasedAt!)}>
                {dayLabel(seg.releasedAt!)} {hhmm(seg.releasedAt!)}
              </span>
              <span className="gt-vmeta">
                {seg.commits.length} 个提交 · {linked.size} 张卡片
                {seg.prevAt != null && ` · 距上一版 ${gapText(seg.releasedAt! - seg.prevAt)}`}
              </span>
              {left > 0 && <span className="gt-vwarn">{left} 张没验收</span>}
            </>
          ) : (
            <span className="gt-vmeta">
              {[seg.commits.length ? `${seg.commits.length} 个提交` : '', `${linked.size + seg.cards.length} 张卡片`, seg.prevAt != null ? `距上一版 ${gapText(Date.now() - seg.prevAt)}` : '']
                .filter(Boolean)
                .join(' · ')}
            </span>
          )}
        </div>
      </header>
      <ol className="gt-list">
        {solo.length > 0 && seg.commits.length > 0 && (
          <li className="gt-day" aria-hidden="true">
            还没有提交
          </li>
        )}
        {solo.map(c => (
          <li key={cardRef(c)} className="gt-item has-card solo">
            <i className="gt-dot" />
            <div className="gt-body">
              <CardTile c={c} s={statusOf(c)} labels={labels} boardName={boardName(c.board)} />
            </div>
          </li>
        ))}
        {withDays(commits).map(({ c, day }) => (
          <Fragment key={c.h}>
            {day && (
              <li className="gt-day" aria-hidden="true">
                {day}
              </li>
            )}
            <CommitItem c={c} cards={cards} statusOf={statusOf} labels={labels} boardName={boardName} />
          </Fragment>
        ))}
      </ol>
    </section>
  );
}

/* ------------------------------ 一栏（一个提交） ------------------------------ */

function CommitItem({
  c,
  cards,
  statusOf,
  labels,
  boardName,
}: {
  c: ReleaseCommit;
  cards: Map<string, MilestoneCard>;
  statusOf: (c: MilestoneCard) => Status | undefined;
  labels: Label[];
  boardName: (path: string) => string | null;
}) {
  const linked = c.cards.map(r => ({ r, card: cards.get(r) }));
  return (
    <li className={cx('gt-item', linked.length ? 'has-card' : 'bare')}>
      <i className="gt-dot" />
      <div className="gt-body">
        <div className="gt-commit">
          <span className="gt-sha mono">{c.h}</span>
          <span className="gt-msg" title={c.s}>
            {c.s}
          </span>
          <time className="gt-t mono" dateTime={new Date(c.at).toISOString()} title={dateTime(c.at)}>
            {hhmm(c.at)}
          </time>
        </div>
        {linked.map(({ r, card }) =>
          card ? (
            <CardTile key={r} c={card} s={statusOf(card)} labels={labels} boardName={boardName(card.board)} />
          ) : (
            <span key={r} className="gt-gone mono" title="卡片不在这个项目里，或者已经删除">
              {r}
            </span>
          ),
        )}
      </div>
    </li>
  );
}

/** 挂在提交下面的卡片：编号、标题、白板（看整个项目、有多块白板时）、标签（不含版本标签）、持有者、状态 */
function CardTile({ c, s, labels, boardName }: { c: MilestoneCard; s: Status | undefined; labels: Label[]; boardName: string | null }) {
  const role = roleOf(s);
  const others = labels.filter(l => c.labels.includes(l.id) && !/^v\d/i.test(l.name));
  return (
    <button type="button" className={cx('gt-card', role === 'done' && 'is-done')} onClick={() => void openCardAt(c.board, c.id)}>
      <span className="gt-id mono">#{c.id}</span>
      <span className="gt-title">
        <Title text={c.title} />
      </span>
      <span className="gt-tags">
        {boardName && <span className="gt-board">{boardName}</span>}
        {others.map(l => (
          <LabelChip key={l.id} label={l} />
        ))}
        {c.holder && <AgentAvatar name={c.holder} agentId={c.holderAgent} className="gt-av" />}
        <span className="gt-st" data-role={role} style={{ ['--sc' as string]: statusColor(s?.color ?? 'todo') }}>
          {s?.name ?? c.column}
        </span>
      </span>
    </button>
  );
}

/* ------------------------------ 版本索引 ------------------------------ */

function VersionIndex({ segs, active, onJump }: { segs: Seg[]; active: string | null; onJump: (key: string) => void }) {
  return (
    <nav className="gt-index" aria-label="版本">
      {segs.map(s => (
        <button
          key={s.key}
          type="button"
          className={cx('gt-ix', s.key === active && 'on', s.kind === 'next' && 'is-next')}
          aria-current={s.key === active ? 'location' : undefined}
          style={{ ['--vc' as string]: s.color }}
          onClick={() => onJump(s.key)}
        >
          <i />
          <span>{s.name}</span>
        </button>
      ))}
    </nav>
  );
}

function TimelineSkeleton() {
  return (
    <div className="gt-sk">
      <Skeleton w={120} h={22} />
      {Array.from({ length: 7 }, (_, i) => (
        <div key={i} className="gt-sk-row">
          <Skeleton w={i % 3 === 2 ? '40%' : '75%'} h={i % 3 === 2 ? 14 : 44} />
        </div>
      ))}
    </div>
  );
}
