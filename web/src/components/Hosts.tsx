/* ==========================================================================
   机器（board host）与派发（任务下发需求 §八）
   - 活动栏「机器」Tab：侧栏列出每台机器；主区是机位总览（每台一张卡片，并发位一格一格画出来）
   - 机器详情：和卡片详情同一种居中弹窗（左：正在跑、排队、派发规则、本机 agent（每个 agent 的启动环境）、启动环境、
     白板与目录；右：最近）
   - 添加机器：同样的弹窗，写两种方法
   - 卡片详情的「派发」对话框，卡片和卡片详情上的派发状态
   ========================================================================== */
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { openBoard, openCard } from '../lib/actions';
import { api } from '../lib/api';
import {
  agentMark, cardDispatch, cronSummary, envName, hostLoad, hostState, hostsOf, mins, osKind, queueReason, resultOf, since, SOURCE,
} from '../lib/hosts';
import { shell } from '../lib/platform';
import type { Dispatch, HostAgent, HostInfo } from '../lib/types';
import { copyText, cx, durWords, hhmm } from '../lib/util';
import { refresh, refreshHosts, useData } from '../store/data';
import { confirm, openDispatch, openHost, setTab, toast, toastError, useUI } from '../store/ui';
import { useNow } from './bits';
import { Icon } from './Icon';
import { usePresence } from './Overlays';

/* ------------------------------ 小部件 ------------------------------ */

const OS_GLYPH = { mac: '⌘', linux: '$_', win: '⊞' } as const;

function OsGlyph({ os, small }: { os: string; small?: boolean }) {
  const k = osKind(os);
  return (
    <span className={cx('hp-os', `os-${k}`, small && 'sm')} title={os}>
      {OS_GLYPH[k]}
    </span>
  );
}

function StatePill({ h, running }: { h: HostInfo; running: number }) {
  const s = hostState(h, running);
  return (
    <span className="hp-pill" style={{ ['--c' as string]: s.color }}>
      <i className={cx('hp-dot', `st-${s.key}`)} />
      {s.label}
    </span>
  );
}

function Dot({ h, running }: { h: HostInfo; running: number }) {
  const s = hostState(h, running);
  return <i className={cx('hp-dot', `st-${s.key}`)} style={{ ['--c' as string]: s.color }} />;
}

/** agent 标记：认识的 agent 用它自己的图标，其他用两个字母 */
export function Mark({ id, title }: { id: string; title?: string }) {
  const m = agentMark(id);
  if (m.icon) return <img className={cx('hp-mini hp-logo', m.light && 'ring')} src={m.icon} alt={title ?? id} title={title ?? id} draggable={false} />;
  return (
    <b className="hp-mini" style={{ ['--ac' as string]: m.color }} title={title ?? id}>
      {m.short}
    </b>
  );
}

/** agent 自己的图标；不认识的 agent 用两个字母 */
function AgentIcon({ id }: { id: string }) {
  const m = agentMark(id);
  return m.icon ? <img className={cx('hp-logo', m.light && 'ring')} src={m.icon} alt="" draggable={false} /> : <b style={{ ['--ac' as string]: m.color }}>{m.short}</b>;
}

/** agent 胶囊：图标、名称、启动环境（「pi · Git Bash」）、版本号、「默认」 */
function AgentChip({ a, def, small, env }: { a: HostAgent; def?: boolean; small?: boolean; env?: string | null }) {
  const m = agentMark(a.id);
  return (
    <span className={cx('hp-agent', !a.found && 'missing', def && 'def', small && 'sm')} style={{ ['--ac' as string]: m.color }}>
      <AgentIcon id={a.id} />
      {a.name}
      {env && <i className="hp-agent-env">· {env}</i>}
      {!small && <em>{a.found ? a.version || '找到了' : '没找到'}</em>}
      {def && <small>默认</small>}
    </span>
  );
}

function Beat({ at, now }: { at: number | null; now: number }) {
  if (at == null) return <span className="hp-beat off">还没有心跳</span>;
  return (
    <span className="hp-beat" title={`最近一次心跳 ${hhmm(at)}`}>
      <i />
      {since(at, now)}
    </span>
  );
}

const lastSeen = (h: HostInfo, now: number) => (h.online ? `心跳 ${since(h.beatAt, now)}` : `${since(h.offAt ?? h.beatAt, now)}在线`);

/** 机器页打开（或机器详情打开）时每 10 秒刷新一次机器和派发：机器心跳不推送，靠它更新「最近心跳」 */
function usePollHosts(on: boolean) {
  useEffect(() => {
    if (!on) return;
    const t = window.setInterval(() => void refreshHosts().catch(() => {}), 10000);
    return () => clearInterval(t);
  }, [on]);
}

/** 打开卡片：切到它的白板，打开详情 */
function goCard(d: Dispatch) {
  openHost(null);
  openBoard(d.board);
  setTimeout(() => openCard(d.card), 120);
}

async function cancel(d: Dispatch) {
  const running = d.state === 'running';
  if (running) {
    const yes = await confirm({
      title: `停止 #${d.card}？`,
      text: `${d.host} 会结束 agent 进程（先 SIGTERM，10 秒后强制结束），卡片移到需要人处理的状态，评论写明是你停止的。`,
      ok: '停止',
      icon: 'trash',
    });
    if (!yes) return;
  }
  try {
    await api.cancelDispatch(d.id);
    toast({ type: 'success', title: running ? `已通知 ${d.host} 停止 #${d.card}` : `已取消 #${d.card} 的派发` });
    await refresh();
  } catch (e) {
    toastError(e);
  }
}

/* ------------------------------ 侧栏 ------------------------------ */

export function HostsSide() {
  const ws = useData(s => s.ws);
  const sel = useUI(s => s.host);
  const now = useNow(10000);
  const hosts = hostsOf(ws);
  const online = hosts.filter(h => h.online).length;
  return (
    <section className="side-view" data-view="hosts">
      <header className="side-head">
        <span className="side-title">机器</span>
        <span className="side-sub">
          {online}/{hosts.length}
        </span>
        <button className="icon-btn sm" type="button" data-tip="添加机器" data-tip-align="end" onClick={() => openHost('new')}>
          <Icon name="plus" />
        </button>
      </header>
      <div className="side-scroll hp-side">
        {hosts.map(h => {
          const { running } = hostLoad(ws, h);
          return (
            <button key={h.id} type="button" className={cx('hp-side-row', sel === h.name && 'active', !h.online && 'off')} onClick={() => openHost(h.name)}>
              <Dot h={h} running={running.length} />
              <span className="hp-side-name">{h.name}</span>
              <span className="hp-side-sub">{!h.online ? `${since(h.offAt ?? h.beatAt, now)}在线` : h.paused ? '暂停' : `${running.length}/${h.max}`}</span>
              {running.length > 0 && (
                <span className="hp-side-runs">
                  {running.map(r => (
                    <span key={r.id}>
                      <Mark id={r.agent} />#{r.card}
                    </span>
                  ))}
                </span>
              )}
            </button>
          );
        })}
        <p className="hp-side-hint">在机器上运行 board host，或在桌面端「设置 → 本机」打开「让这台电脑接任务」。</p>
      </div>
    </section>
  );
}

/* ------------------------------ 主区：机位总览 ------------------------------ */

function Slots({ h, running, now }: { h: HostInfo; running: Dispatch[]; now: number }) {
  const cells = Array.from({ length: Math.max(h.max, running.length) }, (_, i) => running[i] ?? null);
  return (
    <div className="hp-slots">
      {cells.map((r, i) =>
        r ? (
          <div className="hp-slot on" key={r.id} style={{ ['--ac' as string]: agentMark(r.agent).color }}>
            <div className="hp-slot-top">
              <Mark id={r.agent} title={r.agentLabel} />
              <span className="hp-cid">#{r.card}</span>
              <span className="hp-sp" />
              <span className="hp-slot-t">{mins(r.startedAt, now)} 分</span>
            </div>
            <div className="hp-slot-title">{r.title}</div>
            <div className="hp-slot-foot">
              <Beat at={r.beatAt} now={now} />
              <span>{SOURCE[r.source]}</span>
            </div>
          </div>
        ) : (
          <div className={cx('hp-slot', !h.online && 'dead', h.online && h.paused && 'paused')} key={`e${i}`}>
            {!h.online ? '离线' : h.paused ? '暂停' : '空位'}
          </div>
        ),
      )}
    </div>
  );
}

function Summary() {
  const ws = useData(s => s.ws);
  const hosts = hostsOf(ws);
  const on = hosts.filter(h => h.online);
  const slots = on.filter(h => !h.paused).reduce((n, h) => n + h.max, 0);
  const used = on.filter(h => !h.paused).reduce((n, h) => n + hostLoad(ws, h).running.length, 0);
  const queue = (ws?.dispatches ?? []).filter(d => d.state === 'queued').length;
  return (
    <div className="hp-summary">
      <div>
        <b>
          {on.length}
          <small>/{hosts.length}</small>
        </b>
        <span>台在线</span>
      </div>
      <div>
        <b>
          {used}
          <small>/{slots}</small>
        </b>
        <span>个并发位在用</span>
      </div>
      <div>
        <b>{queue}</b>
        <span>张排队</span>
      </div>
      <div className="hp-meter" title={slots ? `${Math.round((used / slots) * 100)}%` : '没有可用的并发位'}>
        <i style={{ width: `${slots ? Math.min(100, (used / slots) * 100) : 0}%` }} />
      </div>
    </div>
  );
}

export function HostsPage() {
  const ws = useData(s => s.ws);
  const tab = useUI(s => s.tab);
  const now = useNow(5000);
  const hosts = hostsOf(ws);
  usePollHosts(tab === 'hosts');
  return (
    <div className="page page-hosts" data-page="hosts">
      <div className="hp-b">
        <div className="hp-b-head">
          <div>
            <h1>机器</h1>
            <p>每台机器上常驻一个 board host；每一格是一个并发位。点机器看详情、改规则。</p>
          </div>
          <button className="btn sm" type="button" onClick={() => openHost('new')}>
            <Icon name="plus" />
            添加机器
          </button>
        </div>
        <Summary />
        {hosts.length === 0 ? (
          <div className="hp-empty-page">
            <Icon name="host" />
            <b>还没有机器</b>
            <span>在要干活的机器上运行 board host，它会出现在这里；之后可以把卡片派给它，或让它按时间点自己领卡。</span>
            <button className="btn sm brand" type="button" onClick={() => openHost('new')}>
              添加机器
            </button>
          </div>
        ) : (
          <div className="hp-tiles">
            {hosts.map(h => {
              const { running, queued } = hostLoad(ws, h);
              const def = h.agent;
              return (
                <button key={h.id} type="button" className={cx('hp-tile', !h.online && 'off', h.paused && 'paused')} onClick={() => openHost(h.name)}>
                  <div className="hp-tile-head">
                    <OsGlyph os={h.os} />
                    <div className="hp-tile-name">
                      <b>{h.name}</b>
                      <span>
                        {h.os} · {lastSeen(h, now)}
                      </span>
                    </div>
                    <StatePill h={h} running={running.length} />
                  </div>
                  <Slots h={h} running={running} now={now} />
                  {queued.length > 0 && (
                    <div className="hp-tile-queue">
                      排队
                      {queued.map(r => (
                        <span key={r.id}>
                          <Mark id={r.agent} />#{r.card} {r.title}
                        </span>
                      ))}
                    </div>
                  )}
                  <div className="hp-tile-foot">
                    {h.agents
                      .filter(a => a.found)
                      .map(a => (
                        <AgentChip key={a.id} a={a} def={a.id === def} env={envName(h, a.env)} small />
                      ))}
                    {!h.agents.some(a => a.found) && <span className="hp-tile-auto">一个 agent 也没找到</span>}
                    <span className="hp-sp" />
                    <span className="hp-tile-auto">{cronSummary(h)}</span>
                  </div>
                </button>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

/* ------------------------------ 机器详情：居中弹窗 ------------------------------ */

export function HostModal() {
  const sel = useUI(s => s.host);
  const ws = useData(s => s.ws);
  const hosts = hostsOf(ws);
  const [last, setLast] = useState<string | null>(sel);
  useEffect(() => {
    if (sel) setLast(sel);
  }, [sel]);
  // 机器被移除（或改名后不在了）：关掉
  useEffect(() => {
    if (sel && sel !== 'new' && ws && !hosts.some(h => h.name === sel)) openHost(null);
  }, [sel, ws, hosts]);
  const on = !!sel;
  const id = sel ?? last;
  const i = hosts.findIndex(x => x.name === id);
  const close = () => openHost(null);
  const step = (d: number) => {
    const next = hosts[i + d];
    if (next) openHost(next.name);
  };
  usePollHosts(on);
  useEffect(() => {
    if (!on) return;
    const key = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest?.('input, textarea, [contenteditable]')) return;
      const ui = useUI.getState();
      if (ui.confirm || ui.palette.open) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === 'Escape') close();
      else if (id !== 'new' && (e.key === 'ArrowUp' || e.key === 'k')) step(-1);
      else if (id !== 'new' && (e.key === 'ArrowDown' || e.key === 'j')) step(1);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  });
  const h = i >= 0 ? hosts[i] : null;
  return createPortal(
    <>
      <div className={cx('scrim', on && 'on')} onClick={close} />
      <aside className={cx('drawer hp-modal', on && 'on')} role="dialog" aria-modal="true" aria-hidden={!on} aria-label={id === 'new' ? '添加机器' : h?.name ?? '机器'}>
        {id === 'new' ? <AddHostBody close={close} /> : h && <HostBody key={h.id} h={h} i={i} n={hosts.length} step={step} close={close} on={on} />}
      </aside>
    </>,
    document.body,
  );
}

function HostBody({ h, i, n, step, close, on }: { h: HostInfo; i: number; n: number; step: (d: number) => void; close: () => void; on: boolean }) {
  const ws = useData(s => s.ws);
  const now = useNow(1000);
  const { running, queued } = hostLoad(ws, h);
  const [recent, setRecent] = useState<Dispatch[] | null>(null);
  const rev = ws?.rev;

  // 最近：打开时和每次有变化时重新取
  useEffect(() => {
    if (!on) return;
    let alive = true;
    api
      .hostRecent(h.name)
      .then(r => alive && setRecent(r))
      .catch(() => alive && setRecent(r => r ?? []));
    return () => {
      alive = false;
    };
  }, [on, h.name, rev]);

  const patch = async (p: Parameters<typeof api.updateHost>[1]) => {
    try {
      await api.updateHost(h.name, p);
      await refresh();
    } catch (e) {
      toastError(e);
    }
  };

  const today = useMemo(() => {
    const d = new Date(now);
    d.setHours(0, 0, 0, 0);
    const t0 = d.getTime();
    const list = (recent ?? []).filter(x => (x.endedAt ?? 0) >= t0);
    return { review: list.filter(x => x.result === 'review').length, crashed: list.filter(x => x.result === 'crashed').length };
  }, [recent, now]);

  return (
    <div className="drawer-body">
      <div className="d-head">
        <OsGlyph os={h.os} small />
        <StatePill h={h} running={running.length} />
        <span className="d-nav">
          <button className="icon-btn sm" type="button" data-tip="上一台  ↑ / K" disabled={i <= 0} onClick={() => step(-1)}>
            <Icon name="chevUp" />
          </button>
          <span className="d-pos mono">
            {i + 1} / {n}
          </span>
          <button className="icon-btn sm" type="button" data-tip="下一台  ↓ / J" disabled={i >= n - 1} onClick={() => step(1)}>
            <Icon name="chevDown" />
          </button>
        </span>
        <span className="spacer" />
        <button className="icon-btn" type="button" data-tip="关闭  esc" data-tip-align="end" onClick={close}>
          <Icon name="close" />
        </button>
      </div>
      <div className="d-grid">
        <div className="d-main">
          <section className="d-section" style={{ ['--i' as string]: 0 }}>
            <h2 className="hp-m-title">{h.name}</h2>
            <div className="hp-m-meta">
              <span>{h.os || '未知系统'}</span>
              <span>board {h.version || '?'}</span>
              <span>{lastSeen(h, now)}</span>
            </div>
          </section>

          <section className="d-section" style={{ ['--i' as string]: 1 }}>
            <div className="d-label">
              正在跑{' '}
              <span className="count">
                {running.length}/{h.max}
              </span>
            </div>
            {running.length ? (
              running.map(r => <RunRow key={r.id} r={r} now={now} />)
            ) : (
              <p className="hp-empty">{h.online ? (h.paused ? '暂停接单：不启动新的派发' : '没有在跑的任务') : '机器离线'}</p>
            )}
            {queued.length > 0 && (
              <>
                <div className="d-label hp-m-sub">
                  排队 <span className="count">{queued.length}</span>
                </div>
                {queued.map(r => (
                  <QueueRow key={r.id} r={r} h={h} now={now} />
                ))}
              </>
            )}
          </section>

          <section className="d-section" style={{ ['--i' as string]: 2 }}>
            <div className="d-label">派发规则</div>
            <div className="hp-rule">
              <div>
                <b>同时跑几个</b>
                <span>正在跑的达到上限后，新的派发在服务端排队</span>
              </div>
              <span className="hp-stepper">
                <button type="button" aria-label="减少" disabled={h.max <= 1} onClick={() => patch({ max: h.max - 1 })}>
                  −
                </button>
                <b>{h.max}</b>
                <button type="button" aria-label="增加" disabled={h.max >= 8} onClick={() => patch({ max: h.max + 1 })}>
                  +
                </button>
              </span>
            </div>
            <div className="hp-rule">
              <div>
                <b>定时领取</b>
                <span>每天到这些时间点（机器本地时区），在这台机器的白板上领卡，把空位填满，做完一张领下一张，直到没有可领取的卡片</span>
              </div>
              <button
                className={cx('toggle', h.cronOn && 'on')}
                type="button"
                role="switch"
                aria-checked={h.cronOn}
                aria-label="定时领取"
                onClick={() => patch({ cronOn: !h.cronOn })}
              >
                <i />
              </button>
            </div>
            <CronTimes h={h} patch={patch} />
            <div className="hp-rule">
              <div>
                <b>现在领一轮</b>
                <span>{h.round ? '正在领一轮：有空位就领下一张，所有白板都没有可领取的卡片时结束' : '效果与到点相同；机器离线或暂停接单时不能领'}</span>
              </div>
              <button
                className="btn sm"
                type="button"
                disabled={!h.online || h.paused || h.round}
                onClick={async () => {
                  try {
                    await api.hostRound(h.name);
                    toast({ type: 'success', title: `${h.name} 开始领一轮` });
                    await refresh();
                  } catch (e) {
                    toastError(e);
                  }
                }}
              >
                <Icon name="play" />
                {h.round ? '正在领' : '领一轮'}
              </button>
            </div>
          </section>

          <section className="d-section" style={{ ['--i' as string]: 3 }}>
            <div className="d-label">本机 agent</div>
            {h.agents.map(a => (
              <AgentRow key={a.id} h={h} a={a} patch={patch} />
            ))}
            {h.agents.length === 0 && <p className="hp-note">board host 没有上报 agent</p>}
          </section>

          {!!h.envs?.length && (
            <section className="d-section" style={{ ['--i' as string]: 4 }}>
              <div className="d-label">启动环境</div>
              {h.envs.map(e => (
                <div className="hp-dir" key={e.id}>
                  <span className="hp-board">{e.name}</span>
                  {e.path && <code title={e.path}>{e.path}</code>}
                  {e.id === 'direct' && <span className="hp-env-hint">不经过 shell，直接执行命令</span>}
                </div>
              ))}
            </section>
          )}

          <section className="d-section" style={{ ['--i' as string]: 5 }}>
            <div className="d-label">白板与目录</div>
            {h.dirs.length ? (
              h.dirs.map(d => (
                <div className="hp-dir" key={d.board}>
                  <span className="hp-board">{d.board}</span>
                  <Icon name="move" />
                  <code title={d.path}>{d.path}</code>
                </div>
              ))
            ) : (
              <p className="hp-note">没有白板：board host 启动时用 --dir 项目/白板=路径 加上，否则接不到卡片</p>
            )}
          </section>

          <div className="d-actions">
            <button
              className="btn sm danger"
              type="button"
              onClick={async () => {
                const yes = await confirm({
                  title: `移除 ${h.name}？`,
                  text: '取消它所有排队中的派发，停止正在跑的派发，删除机器设置；派发记录保留。那台机器的 board host 再连上来时按新机器处理。',
                  ok: '移除',
                });
                if (!yes) return;
                try {
                  await api.removeHost(h.name);
                  close();
                  toast({ type: 'success', title: `已移除 ${h.name}` });
                  await refresh();
                } catch (e) {
                  toastError(e);
                }
              }}
            >
              <Icon name="trash" />
              移除机器
            </button>
            <span className="d-actions-hint">排队的取消，在跑的停止</span>
            <button className="btn sm d-move" type="button" onClick={() => patch({ paused: !h.paused })}>
              <Icon name={h.paused ? 'play' : 'pause'} />
              {h.paused ? '恢复接单' : '暂停接单'}
            </button>
          </div>
        </div>

        <div className="d-side">
          <div className="d-label d-side-head">
            最近 <span className="count">{recent?.length ?? ''}</span>
            <span className="act">
              今天交审 {today.review} · 异常退出 {today.crashed}
            </span>
          </div>
          <div className="d-activity">
            {recent === null ? (
              <p className="hp-note hp-pad">正在加载…</p>
            ) : recent.length === 0 ? (
              <p className="hp-note hp-pad">还没有结束的派发</p>
            ) : (
              recent.map(p => {
                const r = resultOf(p);
                return (
                  <button type="button" className="hp-hist" key={p.id} onClick={() => goCard(p)}>
                    <Mark id={p.agent} title={p.agentLabel} />
                    <div>
                      <b>
                        <span className="hp-cid">#{p.card}</span> {p.title}
                      </b>
                      <span>
                        <em style={{ ['--c' as string]: r.color }}>{r.label}</em>
                        {p.envName && <> · {p.envName}</>}
                        {p.startedAt != null && p.endedAt != null && <> · 用时 {durWords(p.endedAt - p.startedAt)}</>}
                        {p.endedAt != null && <> · {since(p.endedAt, now)}</>}
                      </span>
                    </div>
                  </button>
                );
              })
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/** 本机 agent 的一行：图标、名称、启动环境（下拉，每项带版本号，找不到的置灰写「没找到」）、「默认」（点一下设为默认）。
 *  旧版 board host 不上报启动环境：不显示下拉，写版本号 */
function AgentRow({ h, a, patch }: { h: HostInfo; a: HostAgent; patch: (p: Parameters<typeof api.updateHost>[1]) => Promise<void> }) {
  const def = a.id === h.agent;
  const envs = h.envs ?? [];
  const checks = a.envs ?? [];
  const name = (id: string) => envName(h, id) ?? id;
  return (
    <div className={cx('hp-arow', !a.found && 'missing')} style={{ ['--ac' as string]: agentMark(a.id).color }}>
      <AgentIcon id={a.id} />
      <b className="hp-arow-name">{a.name}</b>
      {envs.length === 0 && <em className="hp-arow-ver">{a.found ? a.version || '找到了' : '没找到'}</em>}
      <span className="hp-sp" />
      {envs.length > 0 &&
        (checks.length ? (
          <select
            className="input hp-env-select"
            value={a.env ?? ''}
            onChange={e => e.target.value && e.target.value !== a.env && patch({ env: { agent: a.id, env: e.target.value } })}
            aria-label={`${a.name} 的启动环境`}
            data-tip="启动环境：在哪种 shell 里启动它；改了下一次派发就用新的"
          >
            {!a.env && <option value="">没找到</option>}
            {checks.map(c => (
              <option key={c.env} value={c.env} disabled={!c.found && c.env !== a.env}>
                {name(c.env)} · {c.found ? c.version || '找到了' : '没找到'}
              </option>
            ))}
          </select>
        ) : (
          <span className="hp-arow-none">本机没有它支持的启动环境</span>
        ))}
      <button
        type="button"
        className={cx('hp-def', def && 'on')}
        disabled={!a.found}
        onClick={() => !def && patch({ agent: a.id })}
        data-tip={!a.found ? '这台机器上没找到命令' : def ? '定时领取用它；手动派发时预先选中它' : '设为默认'}
        data-tip-align="end"
      >
        {def ? '默认' : '设为默认'}
      </button>
    </div>
  );
}

/** 定时领取的时间点：胶囊列表，可增删 */
function CronTimes({ h, patch }: { h: HostInfo; patch: (p: { cron: string[] }) => Promise<void> }) {
  const [t, setT] = useState('');
  const add = () => {
    if (!/^\d{1,2}:\d{2}$/.test(t)) return;
    void patch({ cron: [...h.cron, t] });
    setT('');
  };
  return (
    <div className={cx('hp-cron', !h.cronOn && 'off')}>
      {h.cron.map(x => (
        <span className="hp-time" key={x}>
          {x}
          <button type="button" aria-label={`去掉 ${x}`} onClick={() => patch({ cron: h.cron.filter(y => y !== x) })}>
            <Icon name="close" />
          </button>
        </span>
      ))}
      <span className="hp-time-add">
        <input type="time" value={t} onChange={e => setT(e.target.value)} onKeyDown={e => e.key === 'Enter' && add()} aria-label="新的时间点" />
        <button className="btn sm" type="button" disabled={!t} onClick={add}>
          <Icon name="plus" />
          加时间点
        </button>
      </span>
    </div>
  );
}

function RunRow({ r, now }: { r: Dispatch; now: number }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="hp-run">
      <div className="hp-run-main">
        <Mark id={r.agent} title={r.agentLabel} />
        <button type="button" className="hp-cid link" onClick={() => goCard(r)}>
          #{r.card}
        </button>
        <span className="hp-run-title">{r.title}</span>
        <span className="hp-run-board">{r.board}</span>
      </div>
      <div className="hp-run-meta">
        <span className="hp-mono">{r.name}</span>
        {r.envName && <span>{r.envName}</span>}
        <span>{r.startedAt != null ? durWords(now - r.startedAt) : '刚开始'}</span>
        <Beat at={r.beatAt} now={now} />
        <span>{SOURCE[r.source]}</span>
        {r.stopBy && <span className="hp-stopping">正在停止</span>}
        <span className="hp-sp" />
        <button className="hp-link" type="button" onClick={() => setOpen(!open)}>
          <Icon name="log" />
          {open ? '收起日志' : '日志'}
        </button>
        {!r.stopBy && (
          <button className="hp-link danger" type="button" onClick={() => cancel(r)}>
            <Icon name="stop" />
            停止
          </button>
        )}
      </div>
      {open && <LogView id={r.id} live />}
    </div>
  );
}

function QueueRow({ r, h, now }: { r: Dispatch; h: HostInfo; now: number }) {
  return (
    <div className="hp-run waiting">
      <div className="hp-run-main">
        <Mark id={r.agent} title={r.agentLabel} />
        <button type="button" className="hp-cid link" onClick={() => goCard(r)}>
          #{r.card}
        </button>
        <span className="hp-run-title">{r.title}</span>
        <span className="hp-run-board">{r.board}</span>
      </div>
      <div className="hp-run-meta">
        <span>{queueReason(h)}</span>
        <span>{SOURCE[r.source]}</span>
        <span>排了 {durWords(now - r.createdAt)}</span>
        <span className="hp-sp" />
        <button className="hp-link danger" type="button" onClick={() => cancel(r)}>
          <Icon name="close" />
          取消派发
        </button>
      </div>
    </div>
  );
}

/** 派发日志：最后 500 行；在跑时每 3 秒取一次 */
function LogView({ id, live }: { id: number; live?: boolean }) {
  const [lines, setLines] = useState<string[] | null>(null);
  const ref = useRef<HTMLPreElement>(null);
  const stick = useRef(true);
  useEffect(() => {
    let alive = true;
    const load = () =>
      api
        .dispatchLog(id)
        .then(r => alive && setLines(r.lines))
        .catch(() => alive && setLines(l => l ?? []));
    void load();
    const t = live ? window.setInterval(load, 3000) : 0;
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [id, live]);
  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);
  return (
    <pre
      className="hp-log"
      ref={ref}
      onScroll={e => {
        const el = e.currentTarget;
        stick.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 8;
      }}
    >
      {lines === null ? '正在加载…' : lines.length ? lines.join('\n') : '还没有输出（host 每 5 秒上传一次新增的行）'}
      {live && lines !== null && (
        <>
          {'\n'}
          <span className="hp-caret">▍</span>
        </>
      )}
    </pre>
  );
}

function AddHostBody({ close }: { close: () => void }) {
  const path = useData(s => s.path);
  const origin = typeof location !== 'undefined' ? location.origin : 'https://board.buglan.com';
  const login = `board login --url ${origin} --token bd_…`;
  const run = `board host --name devbox --dir ${path || 'default/main'}=~/repo/board`;
  const copy = async (t: string) => {
    if (await copyText(t)) toast({ type: 'success', title: '已复制', text: t, duration: 1800 });
  };
  return (
    <div className="drawer-body">
      <div className="d-head">
        <span className="hp-m-head">添加机器</span>
        <span className="spacer" />
        <button className="icon-btn" type="button" data-tip="关闭  esc" data-tip-align="end" onClick={close}>
          <Icon name="close" />
        </button>
      </div>
      <div className="d-grid hp-single">
        <div className="d-main">
          <section className="d-section">
            <h2 className="hp-m-title">在要干活的机器上运行 board host</h2>
            <div className="hp-m-meta">
              <span>运行起来后它会出现在机器列表里；派给它的卡片在对应目录里启动本机的 agent</span>
            </div>
          </section>
          <section className="d-section">
            <div className="d-label">装了桌面端的电脑</div>
            <p className="hp-note">在「设置 → 本机」打开「让这台电脑接任务」，填好机器名和白板与目录。桌面端在后台运行 board host，关掉窗口也继续接任务，退出桌面端时结束。</p>
            {shell === 'desktop' && (
              <button
                className="btn sm"
                type="button"
                onClick={() => {
                  close();
                  setTab('settings');
                  setTimeout(() => document.getElementById('set-host')?.scrollIntoView({ behavior: 'smooth', block: 'start' }), 120);
                }}
              >
                <Icon name="monitor" />
                打开「设置 → 本机」
              </button>
            )}
          </section>
          <section className="d-section">
            <div className="d-label">只有命令行的机器（服务器、devbox）</div>
            <p className="hp-note">先在「设置 → Agent 接入」新建一个访问令牌，登录一次：</p>
            <button type="button" className="hp-cmd" onClick={() => copy(login)} data-tip="复制">
              <code>{login}</code>
              <Icon name="copy" />
            </button>
            <p className="hp-note">再在这台机器上常驻运行（--dir 可以写多个；也可以写进 ~/.board/config.toml 的 [host]）：</p>
            <button type="button" className="hp-cmd" onClick={() => copy(run)} data-tip="复制">
              <code>{run}</code>
              <Icon name="copy" />
            </button>
            <p className="hp-note">Ctrl+C 退出：先结束它启动的 agent（卡片移到需要人处理的状态），再离线。</p>
          </section>
        </div>
      </div>
    </div>
  );
}

/* ------------------------------ 卡片：派发 ------------------------------ */

/** 卡片上：排队时写「排队 → <机器> · <agent>（原因）」 */
export function CardDispatchLine({ card }: { card: number }) {
  const d = useData(s => cardDispatch(s.ws, s.path, card));
  const h = useData(s => (d ? hostsOf(s.ws).find(x => x.id === d.hostId) : undefined));
  if (!d || d.state !== 'queued') return null;
  return (
    <div className="card-dispatch" title={`派发于 ${hhmm(d.createdAt)}（${SOURCE[d.source]}）`}>
      <Icon name="dispatch" />
      <span>
        排队 → {d.host} · {d.agentLabel}
        <em>（{queueReason(h)}）</em>
      </span>
    </div>
  );
}

/** 卡片详情里：排队或正在跑的派发，以及「取消派发」/「停止」 */
export function DispatchNote({ board, card }: { board: string; card: number }) {
  const d = useData(s => cardDispatch(s.ws, board, card));
  const h = useData(s => (d ? hostsOf(s.ws).find(x => x.id === d.hostId) : undefined));
  const now = useNow(1000);
  const [log, setLog] = useState(false);
  if (!d) return null;
  const queued = d.state === 'queued';
  return (
    <section className="d-section" data-part="dispatch" style={{ ['--i' as string]: 1 }}>
      <div className={cx('dispatch-note', queued && 'queued')} style={{ ['--ac' as string]: agentMark(d.agent).color }}>
        <Mark id={d.agent} title={d.agentLabel} />
        <div className="dn-text">
          <b>
            {queued ? '排队' : '在跑'} → {d.host} · {d.agentLabel}
          </b>
          <span>
            {queued ? (
              <>
                {queueReason(h)} · {SOURCE[d.source]}派发 · 排了 {durWords(now - d.createdAt)}
              </>
            ) : (
              <>
                {d.name}
                {d.envName && ` · ${d.envName}`} · {SOURCE[d.source]}派发 · 已跑 {d.startedAt != null ? durWords(now - d.startedAt) : '0 秒'}
                {d.stopBy && ' · 正在停止'}
              </>
            )}
          </span>
        </div>
        {!queued && (
          <button className="btn sm ghost" type="button" onClick={() => setLog(!log)}>
            <Icon name="log" />
            {log ? '收起日志' : '日志'}
          </button>
        )}
        {!d.stopBy && (
          <button className="btn sm danger" type="button" onClick={() => cancel(d)}>
            <Icon name={queued ? 'close' : 'stop'} />
            {queued ? '取消派发' : '停止'}
          </button>
        )}
      </div>
      {log && !queued && <LogView id={d.id} live />}
    </section>
  );
}

/** 卡片详情底部「派发」对话框：选机器、选 agent */
export function DispatchDialog() {
  const card = useUI(s => s.dispatchOf);
  const { mounted, on } = usePresence(card != null, 220);
  const last = useRef(card);
  if (card != null) last.current = card;
  if (!mounted || last.current == null) return null;
  return <DispatchBody key={last.current} card={last.current} on={on && card != null} />;
}

function DispatchBody({ card, on }: { card: number; on: boolean }) {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const title = useData(s => s.board?.cards[card]?.title ?? '');
  const hosts = hostsOf(ws);
  const usable = (h: HostInfo) => h.dirs.some(d => d.board === path);
  const busy = (h: HostInfo) => !h.online || h.paused || hostLoad(ws, h).running.length >= h.max;
  // 默认选中：有这块白板目录的机器里，在线、没满、没暂停的优先
  const first = hosts.filter(usable).sort((a, b) => Number(busy(a)) - Number(busy(b)))[0];
  const [host, setHost] = useState<string | null>(first?.name ?? null);
  const h = hosts.find(x => x.name === host) ?? null;
  const [agent, setAgent] = useState<string | null>(h?.agent ?? null);
  const [sending, setSending] = useState(false);
  const close = () => openDispatch(null);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        e.preventDefault();
        close();
      }
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, []);

  const pick = (x: HostInfo) => {
    setHost(x.name);
    setAgent(x.agent);
  };

  const submit = async () => {
    if (!h || !agent) return;
    setSending(true);
    try {
      const d = await api.dispatch(path, card, h.name, agent);
      close();
      toast({ type: 'success', title: `#${card} 已派发到 ${d.host} · ${d.agentLabel}`, text: busy(h) ? `排队：${queueReason(h)}` : '机器有空位，马上开始' });
      await refresh();
    } catch (e) {
      toastError(e);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className="dialog dp-dialog" role="dialog" aria-modal="true" aria-label={`派发 #${card}`}>
        <h3>派发 #{card}</h3>
        <p className="dp-sub">{title}</p>
        {hosts.length === 0 ? (
          <div className="dp-empty">
            <p>还没有机器。在要干活的机器上运行 board host，它会出现在这里。</p>
            <button
              className="btn sm"
              type="button"
              onClick={() => {
                close();
                openHost('new');
              }}
            >
              <Icon name="plus" />
              添加机器
            </button>
          </div>
        ) : (
          <>
            <div className="dp-label">机器</div>
            <div className="dp-hosts" role="radiogroup" aria-label="机器">
              {hosts.map(x => {
                const ok = usable(x);
                const run = hostLoad(ws, x).running.length;
                const why = !x.online ? '离线' : x.paused ? '暂停接单' : run >= x.max ? '已满' : '';
                return (
                  <button
                    key={x.id}
                    type="button"
                    role="radio"
                    aria-checked={host === x.name}
                    className={cx('dp-host', host === x.name && 'on')}
                    disabled={!ok}
                    onClick={() => pick(x)}
                  >
                    <Dot h={x} running={run} />
                    <b>{x.name}</b>
                    <span className="dp-load">
                      {run}/{x.max}
                    </span>
                    {x.paused && <span className="dp-tag">暂停接单</span>}
                    {!x.online && <span className="dp-tag">离线</span>}
                    <span className="hp-sp" />
                    {!ok ? <em>没有这块白板的目录</em> : why ? <em>会排队</em> : <em className="ok">有空位</em>}
                  </button>
                );
              })}
            </div>
            {h && (
              <>
                <div className="dp-label">agent</div>
                <div className="hp-agents">
                  {h.agents.map(a => (
                    <button
                      key={a.id}
                      type="button"
                      className={cx('hp-agent-btn', agent === a.id && 'on')}
                      disabled={!a.found}
                      onClick={() => setAgent(a.id)}
                      data-tip={a.found ? undefined : a.env ? `在${envName(h, a.env)}里没找到命令` : '这台机器上没找到命令'}
                    >
                      <AgentChip a={a} def={a.id === h.agent} env={envName(h, a.env)} />
                    </button>
                  ))}
                </div>
                {busy(h) && <p className="dp-note">{h.name} {!h.online ? '离线' : h.paused ? '暂停接单' : '没有空位'}：派发会排队，{queueReason(h)}后开始。</p>}
              </>
            )}
          </>
        )}
        <div className="dialog-actions">
          <button className="btn" type="button" onClick={close}>
            取消
          </button>
          <button className="btn brand" type="button" disabled={!h || !agent || sending || !usable(h)} onClick={submit}>
            <Icon name="dispatch" />
            派发
          </button>
        </div>
      </div>
    </div>
  );
}
