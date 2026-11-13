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
   - 第一次取回来之前：头部、左栏、主区按真实形状画骨架（Skeleton），容器 aria-busy；取到后淡入（sk-swap）
   ========================================================================== */
import { Fragment, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { hostsOf, resultOf, SOURCE } from '../../lib/hosts';
import { groupEvents, groupWords, mergeSession, resultText, toolKind, totals, usageWords, type Row, type ToolKind } from '../../lib/runEvents';
import type { Dispatch, RunEvent, SessionView } from '../../lib/types';
import { cx, durWords, hhmm, isMobile } from '../../lib/util';
import { onRuns, useData } from '../../store/data';
import { openHost, openSession, useUI } from '../../store/ui';
import { Markdown, Segmented, useNow } from '../bits';
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

function SessionShell({ req, on }: { req: { id: number; card?: number }; on: boolean }) {
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
        <SessionBody key={req.id} id={req.id} card={req.card} on={on} close={close} />
      </aside>
    </>
  );
}

function SessionBody({ id, card, on, close }: { id: number; card?: number; on: boolean; close: () => void }) {
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

  // 输出：在底部时跟着新内容；往上翻了就不动
  const main = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const placed = useRef(false);
  const total = view?.parts.reduce((n, p) => n + p.log.length + (p.events?.length ?? 0) * 7 + (p.events?.filter(e => e.ok != null).length ?? 0), 0) ?? 0;
  useLayoutEffect(() => {
    const el = main.current;
    if (!el || !view) return;
    // 第一次：定位到要看的那张卡（没有时到底部）
    if (!placed.current) {
      placed.current = true;
      const seg = card != null ? el.querySelector<HTMLElement>(`[data-card="${card}"]`) : null;
      if (seg && seg !== el.lastElementChild) {
        el.scrollTop = seg.offsetTop - el.offsetTop - 8;
        stick.current = false;
        return;
      }
      el.scrollTop = el.scrollHeight;
      return;
    }
    if (stick.current) el.scrollTop = el.scrollHeight;
  }, [view, total, card, mode]);
  const jump = (p: Part) => {
    const el = main.current;
    const seg = el?.querySelector<HTMLElement>(`[data-part="${p.id}"]`);
    if (!el || !seg) return;
    el.scrollTo({ top: seg.offsetTop - el.offsetTop - 8, behavior: 'smooth' });
  };

  const parts = view?.parts ?? [];
  const first = parts[0];
  const cur = parts.find(p => p.state === 'running');
  const lastDone = [...parts].reverse().find(p => p.state === 'done');
  const started = parts.find(p => p.startedAt != null)?.startedAt ?? null;
  const ended = running ? null : (lastDone?.endedAt ?? null);
  const removed = first && !hostsOf(ws).some(h => h.id === first.hostId);
  const state = running ? { label: '在跑', color: 'var(--c-doing)' } : lastDone ? resultOf(lastDone) : { label: '还没开始', color: 'var(--text-3)' };
  const usage = totals(parts);
  // 第一次还没取回来：画骨架（出错时不画，直接显示错误）
  const loading = !view && !error;
  return (
    <div className="drawer-body">
      <div className="d-head ss-head">
        <span className="ss-title">
          <Icon name="log" />
          会话
        </span>
        {loading && (
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
        <button aria-label="关闭" className="icon-btn" type="button" data-tip="关闭  esc" data-tip-align="end" onClick={close}>
          <Icon name="close" />
        </button>
      </div>
      {loading && <MetaSkeleton />}
      {first && (
        <div className="ss-meta sk-swap">
          {first.name && <span className="hp-mono">{first.name}</span>}
          {first.envName && <span>{first.envName}</span>}
          {(first.model || first.effort) && <span>{[first.model, first.effort && `强度 ${first.effort}`].filter(Boolean).join(' · ')}</span>}
          <span>{SOURCE[first.source]}</span>
          {started != null && <span>开始于 {hhmm(started)}</span>}
          {started != null && <span>用时 {durWords((ended ?? now) - started)}</span>}
          {usage && <span className="ss-usage">{usageWords(usage)}</span>}
          <span className="ss-state" style={{ ['--c' as string]: state.color }}>
            {state.label}
            {cur?.stopBy && ' · 正在停止'}
          </span>
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
            卡片 {loading ? <Skeleton className="count" w={10} h={10} /> : <span className="count">{parts.length || ''}</span>}
          </div>
          {loading && <CardsSkeleton />}
          {parts.map((p, i) => {
            const st = partState(p);
            const took = partTook(p, now);
            return (
              <div key={p.id} className={cx('ss-card sk-swap', p.card === card && 'on')} role="button" tabIndex={0} onClick={() => jump(p)} onKeyDown={e => e.key === 'Enter' && jump(p)}>
                <span className="ss-n">{i + 1}</span>
                <div>
                  <b>
                    <button
                      type="button"
                      className="hp-cid link"
                      onClick={e => {
                        e.stopPropagation();
                        close();
                        goCard(p);
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
          })}
        </nav>
        <div className="ss-out" ref={main} onScroll={e => {
          const el = e.currentTarget;
          stick.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 12;
        }}>
          {loading && <OutSkeleton />}
          {error && <p className="hp-note ss-err">{error}</p>}
          {parts.map(p => (
            <section key={p.id} className="ss-seg sk-swap" data-part={p.id} data-card={p.card}>
              <h4>
                #{p.card} {p.title}
                <em style={{ ['--c' as string]: partState(p).color }}>{partState(p).label}</em>
              </h4>
              {mode === 'events' ? (
                <Process p={p} showRaw={() => setMode('raw')} />
              ) : (
                <pre className="hp-log ss-log">
                  {p.log.length
                    ? p.log.join('\n')
                    : p.state === 'queued'
                      ? '排着：等这个会话做完前面的卡再开始'
                      : p.state === 'running'
                        ? '还没有输出（host 每 5 秒上传一次新增的行）'
                        : '没有输出'}
                  {p.state === 'running' && (
                    <>
                      {'\n'}
                      <span className="hp-caret">▍</span>
                    </>
                  )}
                </pre>
              )}
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}

/* ------------------------------ 加载中的骨架 ------------------------------ */

/** 头部下面那一行：名字、启动环境、模型、来源、开始时间、用时、状态，右边「过程 / 原始输出」 */
function MetaSkeleton() {
  return (
    <div className="ss-meta sk-meta" aria-hidden="true">
      {[92, 64, 118, 56, 72, 60, 36].map((w, i) => (
        <Skeleton key={i} w={w} h={10} />
      ))}
      <span className="ss-modes">
        <Skeleton w={128} h={32} r="var(--r-sm)" />
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

function CardsSkeleton() {
  return (
    <>
      <span className="sr-only">正在加载…</span>
      {CARD_ROWS.map((r, i) => (
        <div key={i} className="ss-card sk-row" aria-hidden="true">
          <Skeleton w={18} h={18} r={9} />
          <div>
            <b>
              {/* 和真实的 #ID 按钮同样的盒子（等宽字、24px 高、上下 -3px），第一行才一样高 */}
              <span className="hp-cid sk-cid">
                {'\u200b'}
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
      ))}
    </>
  );
}

/** 主区：两段，每段一个标题（#ID 标题 … 状态），下面是 agent 说的话、工具调用行 */
function OutSkeleton() {
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
  const segs = [
    { title: '46%', rows: [msg(2, '58%', 0), tool('52%', 1), tool('38%', 2), tool('60%', 3), msg(3, '44%', 4)] },
    { title: '34%', rows: [msg(2, '66%', 0), tool('44%', 1), tool('56%', 2), tool('36%', 3), msg(1, '40%', 4)] },
  ];
  return (
    <div className="sk-out" aria-hidden="true">
      {segs.map((s, i) => (
        <section key={i} className="ss-seg">
          <h4>
            <Skeleton w={s.title} h={11} />
            <Skeleton className="sk-state" w={36} h={10} />
          </h4>
          <div className="ev-list">{s.rows}</div>
        </section>
      ))}
    </div>
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
  return (
    <div className="ev-list">
      {p.eventsDropped > 0 && (
        <p className="hp-note ss-why">
          前面 {p.eventsDropped} 条事件已经删掉：每次派发只留最新的 2000 条，结束 30 天后只留结果和最后 50 条。
        </p>
      )}
      {rows.map(r => (
        <EventRow key={rowKey(r)} r={r} events={events} running={running} />
      ))}
      {running && <span className="hp-caret ev-caret">▍</span>}
    </div>
  );
}

function rowKey(r: Row): string {
  return r.type === 'group' || r.type === 'text' ? `${r.type}-${r.events[0].id}` : `${r.type}-${r.e.id}`;
}

function EventRow({ r, events, running }: { r: Row; events: RunEvent[]; running: boolean }) {
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
        return <GroupRow events={r.events} running={running} />;
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

/** 连续的读文件、搜索：折成一行，点开列出每一次 */
function GroupRow({ events, running }: { events: RunEvent[]; running: boolean }) {
  const [open, setOpen] = useState(false);
  const busy = running && events.some(e => e.ok == null);
  const failed = events.some(e => e.ok === false);
  return (
    <div className={cx('ev-group', open && 'open')}>
      <button type="button" className="ev-tool-row" aria-expanded={open} onClick={() => setOpen(v => !v)}>
        <Icon name={toolKind(events[0].tool) === 'search' ? 'search' : 'file'} />
        <span className="ev-group-words">{groupWords(events)}</span>
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
