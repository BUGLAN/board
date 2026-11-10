/* 机器详情：居中弹窗（正在跑、排队、派发规则、本机 agent、启动环境、白板与目录、最近）；添加机器 */
import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { agentMark, envName, hostLoad, hostsOf, queueReason, resultOf, since, SOURCE } from '../../lib/hosts';
import { shell } from '../../lib/platform';
import type { Dispatch, HostAgent, HostInfo } from '../../lib/types';
import { copyText, cx, durWords } from '../../lib/util';
import { hostKey, optimistic, useBusy, useData } from '../../store/data';
import { confirm, openHost, setTab, toast, toastError, useUI } from '../../store/ui';
import { useNow } from '../bits';
import { Icon } from '../Icon';
import { cancel, goCard } from './Dispatch';
import { AgentIcon, Beat, lastSeen, LogView, Mark, OsGlyph, patchHost, StatePill, usePollHosts } from './bits';

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

  /** 机器设置：当帧生效，被拒绝时改回来并提示。保存完之前这台机器的设置都禁用（不会在没确认的值上再改一次） */
  const busy = useBusy(hostKey(h.name));
  const patch = async (p: Parameters<typeof api.updateHost>[1]) => {
    const { env, ...fields } = p;
    const local = patchHost(h.name, x => ({
      ...x,
      ...fields,
      agents: env ? x.agents.map(a => (a.id === env.agent ? { ...a, env: env.env } : a)) : x.agents,
    }));
    try {
      await optimistic(local, () => api.updateHost(h.name, p));
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
                <button type="button" aria-label="减少" disabled={busy || h.max <= 1} onClick={() => patch({ max: h.max - 1 })}>
                  −
                </button>
                <b>{h.max}</b>
                <button type="button" aria-label="增加" disabled={busy || h.max >= 8} onClick={() => patch({ max: h.max + 1 })}>
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
                disabled={busy}
                onClick={() => patch({ cronOn: !h.cronOn })}
              >
                <i />
              </button>
            </div>
            <CronTimes h={h} patch={patch} busy={busy} />
            <div className="hp-rule">
              <div>
                <b>现在领一轮</b>
                <span>{h.round ? '正在领一轮：有空位就领下一张，所有白板都没有可领取的卡片时结束' : '效果与到点相同；机器离线或暂停接单时不能领'}</span>
              </div>
              <button
                className="btn sm"
                type="button"
                disabled={busy || !h.online || h.paused || h.round}
                onClick={async () => {
                  // 当帧变成「正在领」（按钮禁用），被拒绝时改回来并提示
                  try {
                    await optimistic(patchHost(h.name, x => ({ ...x, round: true })), () => api.hostRound(h.name));
                    toast({ type: 'success', title: `${h.name} 开始领一轮` });
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
              <AgentRow key={a.id} h={h} a={a} patch={patch} busy={busy} />
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
              disabled={busy}
              onClick={async () => {
                const yes = await confirm({
                  title: `移除 ${h.name}？`,
                  text: '取消它所有排队中的派发，停止正在跑的派发，删除机器设置；派发记录保留。那台机器的 board host 再连上来时按新机器处理。',
                  ok: '移除',
                });
                if (!yes) return;
                // 当帧关闭面板、从列表里拿掉（连同它的派发），被拒绝时回来并提示
                close();
                try {
                  await optimistic(
                    {
                      ws: ws => ({ ...ws, hosts: ws.hosts?.filter(x => x.name !== h.name), dispatches: ws.dispatches?.filter(x => x.host !== h.name) }),
                      keys: [hostKey(h.name)],
                    },
                    () => api.removeHost(h.name),
                  );
                  toast({ type: 'success', title: `已移除 ${h.name}` });
                } catch (e) {
                  toastError(e);
                }
              }}
            >
              <Icon name="trash" />
              移除机器
            </button>
            <span className="d-actions-hint">排队的取消，在跑的停止</span>
            <button className="btn sm d-move" type="button" disabled={busy} onClick={() => patch({ paused: !h.paused })}>
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
function AgentRow({ h, a, patch, busy }: { h: HostInfo; a: HostAgent; patch: (p: Parameters<typeof api.updateHost>[1]) => Promise<void>; busy: boolean }) {
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
            disabled={busy}
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
        disabled={busy || !a.found}
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
function CronTimes({ h, patch, busy }: { h: HostInfo; patch: (p: { cron: string[] }) => Promise<void>; busy: boolean }) {
  const [t, setT] = useState('');
  const add = () => {
    if (busy || !/^\d{1,2}:\d{2}$/.test(t)) return;
    void patch({ cron: [...h.cron, t] });
    setT('');
  };
  return (
    <div className={cx('hp-cron', !h.cronOn && 'off')}>
      {h.cron.map(x => (
        <span className="hp-time" key={x}>
          {x}
          <button type="button" aria-label={`去掉 ${x}`} disabled={busy} onClick={() => patch({ cron: h.cron.filter(y => y !== x) })}>
            <Icon name="close" />
          </button>
        </span>
      ))}
      <span className="hp-time-add">
        <input type="time" value={t} onChange={e => setT(e.target.value)} onKeyDown={e => e.key === 'Enter' && add()} aria-label="新的时间点" />
        <button className="btn sm" type="button" disabled={busy || !t} onClick={add}>
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
