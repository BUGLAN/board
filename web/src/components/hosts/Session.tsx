/* ==========================================================================
   「会话」弹窗（PRD §10.8、§10.12）：同一个 agent 进程用 board next 接着做的几张卡算一个会话
   - 沿用卡片详情的居中弹窗（.scrim / .drawer / .d-*）；手机上从底部弹出，按住顶部下拉关闭
   - 头部：机器、agent、agent 名字、启动环境、来源、开始时间、用时、状态（在跑 / 已结束和结果），在跑时「停止」
   - 左栏：这个会话做过的卡片（按先后，#ID、标题、结果、用时）；点一行跳到它那一段输出，点 #ID 打开卡片
   - 主区：整个会话，按卡片分段。默认「过程」：agent 说的话（Markdown）、每个工具调用一行（点开看结果摘要；连续的读文件、
     搜索折叠成一行）、出错、每张卡末尾的结束（用时、tokens、费用）；可以切到「原始输出」（board host 转写后的日志）。
     事件第一次全取，之后带 since 增量取：推送里有这个会话的派发（ran）时马上取，在跑时另外每 5 秒取一次（原始输出每 5 秒
     上传）。滚在底部时跟着新内容，往上翻时位置不动
   - 头部：各张卡结束事件里的用量加起来（agent 输出里有才显示）
   - 第一次取回来之前：打开它的那一行带来的派发、推送里这个会话在跑和排着的（sessionSlots），先把头部、元信息、左栏的卡片行、
     各段标题按真实内容画出来，张数按会话一共几张；只有输出内容、不知道的那几张画骨架（Skeleton）。一张都不知道时整块按
     真实形状画骨架。容器 aria-busy；取到后内容淡入（sk-swap），先画出来的部分原地不动
   ========================================================================== */
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { dispatchesOf, hostsOf, resultOf, sessionSlots, sourceOf } from '../../lib/hosts';
import { groupEvents, groupKind, groupWords, mergeSession, resultText, runningTool, toolKind, totals, usageWords, type Row, type ToolKind } from '../../lib/runEvents';
import type { Dispatch, RunEvent, SessionView } from '../../lib/types';
import { cx, durWords, hhmm, isMobile, reduceMotion } from '../../lib/util';
import { onRuns, useData } from '../../store/data';
import { openHost, openSession, useUI, type SessionReq } from '../../store/ui';
import { Markdown, Segmented, Tip, useNow } from '../bits';
import { Icon, type IconName } from '../Icon';
import { usePresence } from '../overlays/presence';
import { Skeleton, SkeletonText } from '../Skeleton';
import { Mark } from './bits';
import { cancel, goCard } from './Dispatch';

type Part = SessionView['parts'][number];

/** 一张卡这一次的状态：结果、在跑、排着 */
export function partState(p: Dispatch): { label: string; color: string } {
  if (p.state === 'running') return { label: '在跑', color: 'var(--c-doing)' };
  if (p.state === 'queued') return { label: '排着', color: 'var(--text-3)' };
  return resultOf(p);
}

/** 这一次用了多久（在跑的算到现在；没开始为空） */
export function partTook(p: Dispatch, now: number): number | null {
  if (p.startedAt == null) return null;
  return (p.endedAt ?? now) - p.startedAt;
}

export function SessionModal() {
  const req = useUI(s => s.session);
  const { mounted, on } = usePresence(!!req, 260);
  const last = useRef(req);
  if (req) last.current = req;
  if (!mounted || !last.current) return null;
  return createPortal(<SessionShell req={last.current} on={on && !!req} />, document.body);
}

function SessionShell({ req, on }: { req: SessionReq; on: boolean }) {
  const close = () => openSession(null);
  const drawer = useRef<HTMLElement>(null);
  const [sheetY, setSheetY] = useState<number | null>(null);
  // Esc 先关会话（它在卡片详情、机器详情上面）
  useEffect(() => {
    if (!on) return;
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      const ui = useUI.getState();
      if (ui.confirm || ui.ctx) return;
      e.preventDefault();
      e.stopPropagation();
      close();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [on]);
  // 手机：按住顶部下拉关闭（和卡片详情一样）
  const grab = useRef<{ y0: number; t0: number; dy: number } | null>(null);
  const down = (e: React.PointerEvent) => {
    if (!isMobile()) return;
    const t = e.target as HTMLElement;
    if (!t.closest('.drawer-grab, .d-head') || t.closest('button')) return;
    grab.current = { y0: e.clientY, t0: performance.now(), dy: 0 };
    drawer.current?.setPointerCapture(e.pointerId);
    drawer.current?.classList.add('dragging');
  };
  const move = (e: React.PointerEvent) => {
    if (!grab.current) return;
    grab.current.dy = Math.max(0, e.clientY - grab.current.y0);
    setSheetY(grab.current.dy);
  };
  const up = () => {
    const g = grab.current;
    if (!g) return;
    grab.current = null;
    drawer.current?.classList.remove('dragging');
    const v = g.dy / Math.max(1, performance.now() - g.t0);
    if (g.dy > 140 || v > 0.6) close();
    else setSheetY(0);
  };
  return (
    <>
      <div className={cx('scrim ss-scrim', on && 'on')} onClick={close} />
      <aside
        ref={drawer}
        className={cx('drawer ss-modal', on && 'on')}
        role="dialog"
        aria-modal="true"
        aria-label="会话"
        aria-hidden={!on}
        style={sheetY != null ? { ['--sheet-y' as string]: `${sheetY}px` } : undefined}
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerCancel={up}
      >
        <div className="drawer-grab">
          <i />
        </div>
        <SessionBody key={req.id} id={req.id} card={req.card} known={req.known} on={on} close={close} />
      </aside>
    </>
  );
}

function SessionBody({ id, card, known, on, close }: { id: number; card?: number; known?: Dispatch[]; on: boolean; close: () => void }) {
  const [view, setView] = useState<SessionView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<'events' | 'raw'>('events');
  const rev = useData(s => s.ws?.rev);
  const ws = useData(s => s.ws);
  const now = useNow(1000);
  const running = !!view?.parts.some(p => p.state === 'running');
  // 事件的版本（上次取到的 runs）：之后带 since 只取新增、改过的
  const runs = useRef(0);
  const viewRef = useRef<SessionView | null>(null);
  viewRef.current = view;
  // 取一次；工作区有推送变化时、推送里有这个会话的新事件时马上再取，在跑时另外每 5 秒取一次。同一时刻只有一个请求
  useEffect(() => {
    if (!on) return;
    let alive = true;
    let busy = false;
    let again = false;
    const load = async () => {
      if (busy) {
        again = true;
        return;
      }
      busy = true;
      try {
        do {
          again = false;
          const since = viewRef.current ? runs.current : 0;
          try {
            const v = await api.session(id, since);
            if (!alive) return;
            const next = mergeSession(viewRef.current, v, since > 0);
            viewRef.current = next;
            runs.current = v.runs ?? 0;
            setView(next);
            setError(null);
          } catch (e) {
            if (alive) setError(e instanceof Error ? e.message : String(e));
          }
        } while (again && alive);
      } finally {
        busy = false;
      }
    };
    void load();
    const t = running ? window.setInterval(load, 5000) : 0;
    const off = onRuns(ids => {
      if (viewRef.current?.parts.some(p => ids.includes(p.id))) void load();
    });
    return () => {
      alive = false;
      clearInterval(t);
      off();
    };
  }, [id, on, rev, running]);

  // 第一次还没取回来（出错时不算，直接显示错误）
  const loading = !view && !error;
  // 取回来之前已经知道的几张，按第几张排好；不知道的是 null。一张都不知道时为空，整块画骨架
  const slots = useMemo(() => (loading ? sessionSlots(id, known, dispatchesOf(ws)) : []), [loading, id, known, ws]);

  // 输出：在底部时跟着新内容；往上翻了就不动（新内容进来、展开收起折叠组都不把位置拉走），右下角出现「到最底下」
  const main = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const placed = useRef(false);
  const [away, setAway] = useState(false);
  // 点了「到最底下」、平滑滚动还没到底时：滚动事件不算「往上翻」
  const chasing = useRef(0);
  const follow = (on: boolean) => {
    stick.current = on;
    setAway(!on);
  };
  const onScroll = () => {
    const el = scrollerOf(main.current);
    if (!el) return;
    const bottom = atBottom(el);
    if (chasing.current) {
      if (!bottom) return;
      chasing.current = 0;
    }
    follow(bottom);
  };
  const toBottom = () => {
    const el = scrollerOf(main.current);
    if (!el) return;
    chasing.current = Date.now();
    follow(true);
    el.scrollTo({ top: el.scrollHeight, behavior: reduceMotion() ? 'auto' : 'smooth' });
    // 平滑滚动被打断（比如人又往上翻）也别一直不认滚动
    window.setTimeout(() => {
      if (chasing.current && Date.now() - chasing.current >= 1200) chasing.current = 0;
    }, 1300);
  };
  // 手机上整块一起滚（.ss-grid），桌面上输出区自己滚
  useEffect(() => {
    const el = scrollerOf(main.current);
    if (!el || el === main.current) return;
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => el.removeEventListener('scroll', onScroll);
  });
  const total = view?.parts.reduce((n, p) => n + p.log.length + (p.events?.length ?? 0) * 7 + (p.events?.filter(e => e.ok != null).length ?? 0), 0) ?? 0;
  useLayoutEffect(() => {
    const el = main.current;
    if (!el) return;
    // 定位到要看的那张卡（没有、或者是最后一张时到底部）。先按知道的几张画出来时也定位一次，取回来后再按真实内容定位
    const sc = scrollerOf(el) ?? el;
    const place = () => {
      const seg = card != null ? el.querySelector<HTMLElement>(`[data-card="${card}"]`) : null;
      if (seg && seg !== el.querySelector('section.ss-seg:last-of-type')) {
        sc.scrollTop = seg.offsetTop - el.offsetTop - 8;
        follow(false);
        return;
      }
      sc.scrollTop = sc.scrollHeight;
    };
    if (!view) {
      if (slots.length) place();
      return;
    }
    if (!placed.current) {
      placed.current = true;
      follow(true);
      place();
      return;
    }
    if (stick.current) sc.scrollTop = sc.scrollHeight;
    // 不跟随时内容变长了：还在底部的话（例如本来就差一点）照常认作到底
    else if (!chasing.current) setAway(!atBottom(sc));
  }, [view, total, card, mode, slots.length]); // eslint-disable-line react-hooks/exhaustive-deps
  const jump = (p: Dispatch) => {
    const el = main.current;
    const seg = el?.querySelector<HTMLElement>(`[data-part="${p.id}"]`);
    if (!el || !seg) return;
    el.scrollTo({ top: seg.offsetTop - el.offsetTop - 8, behavior: 'smooth' });
  };

  const parts = view?.parts ?? [];
  // 头部、元信息按取回来的画；还没取回来时按已经知道的几张
  const src: Dispatch[] = view ? parts : slots.filter((p): p is Dispatch => !!p);
  const first = src[0];
  // 第 1 张：来源以它为准；开始时间、用时、状态要每一张都知道（第一张什么时候开始、最后一张的结果）才画，不全时画骨架
  const lead = view ? parts[0] : slots[0];
  const exact = !!view || (slots.length > 0 && slots.every(Boolean));
  const live = src.some(p => p.state === 'running');
  const cur = src.find(p => p.state === 'running');
  const lastDone = [...src].reverse().find(p => p.state === 'done');
  const started = src.find(p => p.startedAt != null)?.startedAt ?? null;
  const ended = live ? null : (lastDone?.endedAt ?? null);
  const removed = first && !hostsOf(ws).some(h => h.id === first.hostId);
  const state = live ? { label: '在跑', color: 'var(--c-doing)' } : lastDone ? resultOf(lastDone) : { label: '还没开始', color: 'var(--text-3)' };
  const usage = totals(parts);
  const rows: (Dispatch | null)[] = view ? parts : slots;
  return (
    <div className="drawer-body">
      <div className="d-head ss-head">
        <span className="ss-title">
          <Icon name="log" />
          会话
        </span>
        {loading && !first && (
          <span className="ss-who sk-who" aria-hidden="true">
            <Skeleton w={20} h={18} r={5} />
            <Skeleton w={76} h={11} />
            <Skeleton w={52} h={11} />
          </span>
        )}
        {first && (
          <span className="ss-who sk-swap">
            <Mark id={first.agent} title={first.agentLabel} />
            <b>{first.agentLabel}</b>
            <button
              type="button"
              className="hp-link"
              disabled={removed}
              onClick={() => {
                close();
                openHost(first.host);
              }}
              title={removed ? '机器已移除' : `打开机器 ${first.host}`}
            >
              {first.host}
            </button>
          </span>
        )}
        <span className="spacer" />
        {cur && !cur.stopBy && (
          <button className="btn sm danger" type="button" onClick={() => void cancel(cur)}>
            <Icon name="stop" />
            停止
          </button>
        )}
        <Tip text="关闭  esc">
          <button aria-label="关闭" className="icon-btn" type="button" data-tip-align="end" onClick={close}>
            <Icon name="close" />
          </button>
        </Tip>
      </div>
      {loading && !first && <MetaSkeleton />}
      {first && (
        <div className="ss-meta sk-swap">
          {first.name && <span className="hp-mono">{first.name}</span>}
          {first.envName && <span>{first.envName}</span>}
          {(first.model || first.effort) && <span>{[first.model, first.effort && `强度 ${first.effort}`].filter(Boolean).join(' · ')}</span>}
          {lead ? <span>{sourceOf(lead)}</span> : <Skeleton w={28} h={10} />}
          {exact ? (
            <>
              {started != null && <span>开始于 {hhmm(started)}</span>}
              {started != null && <span>用时 {durWords((ended ?? now) - started)}</span>}
            </>
          ) : (
            <>
              <Skeleton w={76} h={10} />
              <Skeleton w={84} h={10} />
            </>
          )}
          {usage && <span className="ss-usage">{usageWords(usage)}</span>}
          {exact ? (
            <span className="ss-state" style={{ ['--c' as string]: state.color }}>
              {state.label}
              {cur?.stopBy && ' · 正在停止'}
            </span>
          ) : (
            <Skeleton w={28} h={10} />
          )}
          <span className="ss-modes">
            <Segmented
              value={mode}
              options={[
                ['events', '过程'],
                ['raw', '原始输出'],
              ]}
              onChange={setMode}
            />
          </span>
        </div>
      )}
      <div className="ss-grid" aria-busy={loading || undefined}>
        <nav className="ss-cards" aria-label="这个会话做过的卡片">
          <div className="d-label">
            卡片 {loading && !rows.length ? <Skeleton className="count" w={10} h={10} /> : <span className="count">{rows.length || ''}</span>}
          </div>
          {loading && <span className="sr-only">正在加载…</span>}
          {loading && !rows.length && <CardsSkeleton />}
          {rows.map((p, i) =>
            p ? (
              <CardRow
                key={p.id}
                p={p}
                n={i + 1}
                on={p.card === card}
                now={now}
                jump={() => jump(p)}
                open={() => {
                  close();
                  goCard(p);
                }}
              />
            ) : (
              <CardRowSkeleton key={`sk${i}`} i={i} />
            ),
          )}
        </nav>
        <div
          className="ss-out"
          ref={main}
          onScroll={onScroll}
          onClickCapture={e => {
            // 展开、收起折叠组或一条工具调用：内容变了但人没滚，按现在是不是在底部重新认（不把位置拉走）
            if (!(e.target as HTMLElement).closest?.('[aria-expanded]')) return;
            requestAnimationFrame(() => {
              const el = scrollerOf(main.current);
              if (el && !chasing.current) follow(atBottom(el));
            });
          }}
        >
          {loading && !rows.length && <OutSkeleton />}
          {error && <p className="hp-note ss-err">{error}</p>}
          {rows.map((p, i) => (p ? <Segment key={p.id} p={p} mode={mode} pending={!view} showRaw={() => setMode('raw')} /> : <SegSkeleton key={`sk${i}`} i={i} />))}
          <div className={cx('ss-bottom', away && rows.length > 0 && 'on')} aria-hidden={!away || undefined}>
            <Tip text="到最底下，接着跟新内容">
              <button type="button" className="btn sm ss-tobottom" tabIndex={away ? 0 : -1} onClick={toBottom} data-tip-pos="top" data-tip-align="end">
                <Icon name="chevDown" />
                到最底下
              </button>
            </Tip>
          </div>
        </div>
      </div>
    </div>
  );
}

/** 离底部不到 12px 算在底部 */
const atBottom = (el: HTMLElement) => el.scrollTop + el.clientHeight >= el.scrollHeight - 12;

/** 输出区真正滚动的那一层：桌面上是输出区自己，手机上是整块（.ss-grid） */
function scrollerOf(el: HTMLElement | null): HTMLElement | null {
  if (!el) return null;
  return getComputedStyle(el).overflowY === 'visible' ? el.parentElement : el;
}

/** 左栏的一行：序号、#ID（点开卡片）、标题、状态 · 用时；点这一行跳到它那一段输出 */
function CardRow({ p, n, on, now, jump, open }: { p: Dispatch; n: number; on: boolean; now: number; jump: () => void; open: () => void }) {
  const st = partState(p);
  const took = partTook(p, now);
  return (
    <div className={cx('ss-card sk-swap', on && 'on')} role="button" tabIndex={0} onClick={jump} onKeyDown={e => e.key === 'Enter' && jump()}>
      <span className="ss-n">{n}</span>
      <div>
        <b>
          <button
            type="button"
            className="hp-cid link"
            onClick={e => {
              e.stopPropagation();
              open();
            }}
            title="打开卡片"
          >
            #{p.card}
          </button>{' '}
          {p.title}
        </b>
        <span>
          <em style={{ ['--c' as string]: st.color }}>{st.label}</em>
          {took != null && <> · {durWords(took)}</>}
        </span>
      </div>
    </div>
  );
}

/** 一张卡那一段：标题（#ID 标题 … 状态），下面是过程或原始输出。
 *  pending：还没取回来，标题按已经知道的画，内容画骨架（排着的没有内容，直接写排着）；取回来后内容淡入，标题不动 */
function Segment({ p, mode, pending, showRaw }: { p: Part | Dispatch; mode: 'events' | 'raw'; pending: boolean; showRaw: () => void }) {
  const st = partState(p);
  // 先按已经知道的画出来的段：内容到了要淡入（标题不动）
  const fill = useRef(pending).current;
  const part: Part = 'log' in p ? p : { ...p, log: [], events: [], eventsFrom: 0, eventsDropped: 0 };
  const body = pending && p.state !== 'queued';
  return (
    <section className={cx('ss-seg sk-swap', fill && 'ss-seg-fill')} data-part={p.id} data-card={p.card}>
      <h4>
        #{p.card} {p.title}
        <em style={{ ['--c' as string]: st.color }}>{st.label}</em>
      </h4>
      {body ? (
        <SegBodySkeleton key="sk" i={(p.seq ?? 1) - 1} />
      ) : mode === 'events' ? (
        <Process key="events" p={part} showRaw={showRaw} />
      ) : (
        <pre key="raw" className="hp-log ss-log">
          {part.log.length
            ? part.log.join('\n')
            : part.state === 'queued'
              ? '排着：等这个会话做完前面的卡再开始'
              : part.state === 'running'
                ? '还没有输出（host 每 5 秒上传一次新增的行）'
                : '没有输出'}
          {part.state === 'running' && (
            <>
              {'\n'}
              <span className="hp-caret">▍</span>
            </>
          )}
        </pre>
      )}
    </section>
  );
}

/* ------------------------------ 加载中的骨架 ------------------------------ */

/** 头部下面那一行：名字、启动环境、模型、来源、开始时间、用时、状态，右边「过程 / 原始输出」 */
function MetaSkeleton() {
  return (
    <div className="ss-meta sk-meta" aria-hidden="true">
      {[124, 22, 104, 28, 76, 84, 28].map((w, i) => (
        <Skeleton key={i} w={w} h={10} />
      ))}
      <span className="ss-modes">
        <Skeleton w={131} h={32} r="var(--r-sm)" />
      </span>
    </div>
  );
}

/** 左栏的卡片行：序号圈、#ID + 标题（有的折成两行）、状态 · 用时；行高和真实的行一样 */
const CARD_ROWS: { title: string[]; meta: number }[] = [
  { title: ['74%', '42%'], meta: 72 },
  { title: ['58%'], meta: 64 },
  { title: ['70%', '30%'], meta: 76 },
  { title: ['50%'], meta: 58 },
];

function CardRowSkeleton({ i }: { i: number }) {
  const r = CARD_ROWS[i % CARD_ROWS.length];
  return (
    <div className="ss-card sk-row" aria-hidden="true">
      <Skeleton w={18} h={18} r={9} />
      <div>
        <b>
          {/* 和真实的 #ID 按钮同样的盒子（等宽字、24px 高、上下 -3px），第一行才一样高 */}
          <span className="hp-cid sk-cid">
            {'​'}
            <Skeleton w={30} h={10} />
          </span>{' '}
          {r.title.map((w, j) => (
            <Fragment key={j}>
              {j > 0 && <br />}
              <Skeleton w={w} h={10} />
            </Fragment>
          ))}
        </b>
        <span>
          <Skeleton w={r.meta} h={9} />
        </span>
      </div>
    </div>
  );
}

/** 一张都不知道时：左栏 4 行 */
function CardsSkeleton() {
  return (
    <>
      {CARD_ROWS.map((_, i) => (
        <CardRowSkeleton key={i} i={i} />
      ))}
    </>
  );
}

/** 一段的内容：agent 说的话、工具调用行（第几段错开长短） */
function SegBodySkeleton({ i }: { i: number }) {
  const tool = (w: string, k: number) => (
    <div key={k} className="ev-tool">
      <span className="ev-tool-row">
        <Skeleton w={14} h={14} r={4} />
        <Skeleton w={[36, 44, 30][k % 3]} h={10} />
        <Skeleton w={w} h={9} />
        <Skeleton className="sk-mark" w={13} h={13} r={7} />
      </span>
    </div>
  );
  const msg = (lines: number, last: string, k: number) => (
    <div key={k} className="ev-msg">
      <SkeletonText lines={lines} h={10} gap={11} last={last} />
    </div>
  );
  const rows = [
    [msg(2, '58%', 0), tool('52%', 1), tool('38%', 2), tool('60%', 3), msg(3, '44%', 4)],
    [msg(2, '66%', 0), tool('44%', 1), tool('56%', 2), tool('36%', 3), msg(1, '40%', 4)],
  ][i % 2];
  return (
    <div className="ev-list sk-body" aria-hidden="true">
      {rows}
    </div>
  );
}

/** 不知道的一段：标题（#ID 标题 … 状态）和内容都是骨架 */
function SegSkeleton({ i }: { i: number }) {
  return (
    <section className="ss-seg" aria-hidden="true">
      <h4 className="sk-h4">
        <Skeleton w={['46%', '34%'][i % 2]} h={11} />
        <Skeleton className="sk-state" w={36} h={10} />
      </h4>
      <SegBodySkeleton i={i} />
    </section>
  );
}

/** 一张都不知道时：主区两段 */
function OutSkeleton() {
  return (
    <>
      <SegSkeleton i={0} />
      <SegSkeleton i={1} />
    </>
  );
}

/* ------------------------------ 过程（结构化事件） ------------------------------ */

const TOOL_ICON: Record<ToolKind, IconName> = {
  read: 'file',
  search: 'search',
  edit: 'edit',
  shell: 'terminal',
  web: 'download',
  task: 'sparkle',
  todo: 'check',
  other: 'bolt',
};

/** 一张卡这一次的过程：没有事件时写明为什么（还没开始、只有原始输出、旧版 board host） */
function Process({ p, showRaw }: { p: Part; showRaw: () => void }) {
  const events = p.events ?? [];
  const running = p.state === 'running';
  if (!events.length) {
    const raw = (why: string) => (
      <p className="hp-note ss-why">
        {why}
        <button type="button" className="hp-link" onClick={showRaw}>
          看原始输出
        </button>
      </p>
    );
    if (p.state === 'queued') return <p className="hp-note ss-why">排着：等这个会话做完前面的卡再开始</p>;
    if (p.fmt === '') return raw(`${p.agentLabel || p.agent} 没有结构化输出（配置文件里的命令没带事件流参数，或者是配置文件里加的 agent），只能看原始输出。`);
    if (running) return <p className="hp-note ss-why">还没有事件（board host 每 2 秒上传一次）</p>;
    if (p.fmt == null) return raw('这次派发没有记录过程（board host 版本较旧，或者没开始）。');
    return raw('没有事件。');
  }
  const rows = groupEvents(events);
  // 在跑的那一条：折叠组折着时也写出来
  const now = runningTool(events, running);
  return (
    <div className="ev-list">
      {p.eventsDropped > 0 && (
        <p className="hp-note ss-why">
          前面 {p.eventsDropped} 条事件已经删掉：每次派发只留最新的 2000 条，结束 30 天后只留结果和最后 50 条。
        </p>
      )}
      {rows.map(r => (
        <EventRow key={rowKey(r)} r={r} events={events} running={running} now={now} />
      ))}
      {running && <span className="hp-caret ev-caret">▍</span>}
    </div>
  );
}

function rowKey(r: Row): string {
  return r.type === 'group' || r.type === 'text' ? `${r.type}-${r.events[0].id}` : `${r.type}-${r.e.id}`;
}

function EventRow({ r, events, running, now }: { r: Row; events: RunEvent[]; running: boolean; now: RunEvent | null }) {
  const sub = r.type === 'group' || r.type === 'text' ? r.events[0].sub : r.e.sub;
  const body = (() => {
    switch (r.type) {
      case 'message':
        return (
          <div className="ev-msg">
            <Markdown src={r.e.text || ''} />
            {r.e.cut && <span className="ev-cut">（超过 4 KB，已截断）</span>}
          </div>
        );
      case 'tool':
        return <ToolRow e={r.e} running={running} />;
      case 'group':
        return <GroupRow events={r.events} running={running} now={now && r.events.includes(now) ? now : null} />;
      case 'error':
        return (
          <div className="ev-err">
            <Icon name="alert" />
            <span>{r.e.text}</span>
          </div>
        );
      case 'text':
        return <pre className="ev-text">{r.events.map(e => e.text || '').join('\n')}</pre>;
      case 'result':
        return <ResultRow e={r.e} text={resultText(r.e, events)} />;
    }
  })();
  return sub ? <div className="ev-sub">{body}</div> : body;
}

/** 一次工具调用：图标、工具名、参数摘要、成功或失败；有结果时点开看结果摘要 */
function ToolRow({ e, running }: { e: RunEvent; running: boolean }) {
  const [open, setOpen] = useState(false);
  const kind = toolKind(e.tool);
  const more = !!e.out || e.code != null || e.ms != null;
  const state = e.ok === true ? 'ok' : e.ok === false ? 'fail' : running ? 'busy' : 'none';
  return (
    <div className={cx('ev-tool', state, open && 'open')}>
      <button type="button" className="ev-tool-row" aria-expanded={more ? open : undefined} disabled={!more} onClick={() => setOpen(v => !v)}>
        <Icon name={TOOL_ICON[kind]} />
        <b>{e.tool || '工具'}</b>
        {e.args && <code title={e.args}>{e.args}</code>}
        <span className="ev-mark" title={state === 'ok' ? '成功' : state === 'fail' ? '失败' : state === 'busy' ? '在跑' : '没有结果'}>
          {state === 'ok' ? <Icon name="check" /> : state === 'fail' ? <Icon name="close" /> : state === 'busy' ? <i className="ev-spin" /> : '—'}
        </span>
      </button>
      {open && more && (
        <div className="ev-tool-out">
          {(e.code != null || e.ms != null) && (
            <span className="ev-tool-meta">
              {e.code != null && `退出码 ${e.code}`}
              {e.code != null && e.ms != null && ' · '}
              {e.ms != null && `用时 ${durWords(e.ms)}`}
            </span>
          )}
          {e.out && <pre>{e.out}</pre>}
          {e.cut && <span className="ev-cut">（超过 4 KB，已截断）</span>}
        </div>
      )}
    </div>
  );
}

/** 连续的工具调用（命令、读写文件、搜索、Skill……）：折成一行，写明各类几次；点开列出每一次。
    在跑的会话里这一组有正在跑的那一条（now）时，折着也写出来 */
function GroupRow({ events, running, now }: { events: RunEvent[]; running: boolean; now: RunEvent | null }) {
  const [open, setOpen] = useState(false);
  const busy = !!now;
  const failed = events.some(e => e.ok === false);
  return (
    <div className={cx('ev-group', open && 'open', busy && 'busy')}>
      <button type="button" className="ev-tool-row" aria-expanded={open} onClick={() => setOpen(v => !v)}>
        <Icon name={TOOL_ICON[groupKind(events)]} />
        <span className="ev-group-words">
          <span>{groupWords(events)}</span>
          {now && !open && (
            <span className="ev-group-now">
              <span>正在</span>
              <b>{now.tool || '工具'}</b>
              {now.args && <code title={now.args}>{now.args}</code>}
            </span>
          )}
        </span>
        <span className={cx('ev-mark', failed ? 'fail' : busy ? 'busy' : 'ok')}>{busy ? <i className="ev-spin" /> : <Icon name={open ? 'chevUp' : 'chevDown'} />}</span>
      </button>
      {open && (
        <div className="ev-group-list">
          {events.map(e => (
            <ToolRow key={e.id} e={e} running={running} />
          ))}
        </div>
      )}
    </div>
  );
}

/** 每张卡那一段末尾的结束：成功或出错、用时、轮数、用量；结果文字和最后一句话不同时才写 */
function ResultRow({ e, text }: { e: RunEvent; text: string }) {
  const bits: string[] = [];
  if (e.ms != null) bits.push(`用时 ${durWords(e.ms)}`);
  if (e.turns != null) bits.push(`${e.turns} 轮`);
  const usage = usageWords(e);
  if (usage) bits.push(usage);
  return (
    <div className={cx('ev-result', e.ok === false ? 'fail' : 'ok')}>
      <div className="ev-result-head">
        <Icon name={e.ok === false ? 'alert' : 'checkCircle'} />
        <b>{e.ok === false ? '结束 · 出错' : '结束'}</b>
        {bits.length > 0 && <span>{bits.join(' · ')}</span>}
      </div>
      {text && <Markdown src={text} className="ev-result-text" />}
    </div>
  );
}
