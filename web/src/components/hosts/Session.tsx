/* ==========================================================================
   「会话」弹窗（PRD §10.8、§10.12）：同一个 agent 进程用 board next 接着做的几张卡算一个会话
   - 沿用卡片详情的居中弹窗（.scrim / .drawer / .d-*）；手机上从底部弹出，按住顶部下拉关闭
   - 头部：机器、agent、agent 名字、启动环境、来源、开始时间、用时、状态（在跑 / 已结束和结果），在跑时「停止」
   - 左栏：这个会话做过的卡片（按先后，#ID、标题、结果、用时）；点一行跳到它那一段输出，点 #ID 打开卡片
   - 主区：整个会话的输出，按卡片分段。在跑时每 2 秒取一次（有推送变化时马上取）；滚在底部时跟着新内容，
     往上翻时位置不动
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { hostsOf, resultOf, SOURCE } from '../../lib/hosts';
import type { Dispatch, SessionView } from '../../lib/types';
import { cx, durWords, hhmm, isMobile } from '../../lib/util';
import { useData } from '../../store/data';
import { openHost, openSession, useUI } from '../../store/ui';
import { useNow } from '../bits';
import { Icon } from '../Icon';
import { usePresence } from '../overlays/presence';
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
  const rev = useData(s => s.ws?.rev);
  const ws = useData(s => s.ws);
  const now = useNow(1000);
  const running = !!view?.parts.some(p => p.state === 'running');
  // 取一次；有推送变化时马上再取，在跑时每 2 秒取一次
  useEffect(() => {
    if (!on) return;
    let alive = true;
    const load = () =>
      api
        .session(id)
        .then(v => {
          if (!alive) return;
          setView(v);
          setError(null);
        })
        .catch(e => alive && setError(e instanceof Error ? e.message : String(e)));
    void load();
    const t = running ? window.setInterval(load, 2000) : 0;
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [id, on, rev, running]);

  // 输出：在底部时跟着新内容；往上翻了就不动
  const main = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const placed = useRef(false);
  const total = view?.parts.reduce((n, p) => n + p.log.length, 0) ?? 0;
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
  }, [view, total, card]);
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
  return (
    <div className="drawer-body">
      <div className="d-head ss-head">
        <span className="ss-title">
          <Icon name="log" />
          会话
        </span>
        {first && (
          <span className="ss-who">
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
      {first && (
        <div className="ss-meta">
          {first.name && <span className="hp-mono">{first.name}</span>}
          {first.envName && <span>{first.envName}</span>}
          {(first.model || first.effort) && <span>{[first.model, first.effort && `强度 ${first.effort}`].filter(Boolean).join(' · ')}</span>}
          <span>{SOURCE[first.source]}</span>
          {started != null && <span>开始于 {hhmm(started)}</span>}
          {started != null && <span>用时 {durWords((ended ?? now) - started)}</span>}
          <span className="ss-state" style={{ ['--c' as string]: state.color }}>
            {state.label}
            {cur?.stopBy && ' · 正在停止'}
          </span>
        </div>
      )}
      <div className="ss-grid">
        <nav className="ss-cards" aria-label="这个会话做过的卡片">
          <div className="d-label">
            卡片 <span className="count">{parts.length || ''}</span>
          </div>
          {parts.map((p, i) => {
            const st = partState(p);
            const took = partTook(p, now);
            return (
              <div key={p.id} className={cx('ss-card', p.card === card && 'on')} role="button" tabIndex={0} onClick={() => jump(p)} onKeyDown={e => e.key === 'Enter' && jump(p)}>
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
          {!view && !error && <p className="hp-note">正在加载…</p>}
        </nav>
        <div className="ss-out" ref={main} onScroll={e => {
          const el = e.currentTarget;
          stick.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 12;
        }}>
          {error && <p className="hp-note ss-err">{error}</p>}
          {parts.map(p => (
            <section key={p.id} className="ss-seg" data-part={p.id} data-card={p.card}>
              <h4>
                #{p.card} {p.title}
                <em style={{ ['--c' as string]: partState(p).color }}>{partState(p).label}</em>
              </h4>
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
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}
