/* 机器详情：居中弹窗（正在跑、排队、派发规则、本机 agent、启动环境、白板与目录、最近）；添加机器 */
import { useEffect, useMemo, useState, useRef } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { agentMark, envName, hostLoad, hostsOf, isLocalHost, queueReason, resultOf, since, skillGaps, skillMiss, SOURCE } from '../../lib/hosts';
import { installSkill, localHost, shell, type LocalHost } from '../../lib/platform';
import type { ClaimRule, Dispatch, HostAgent, HostDir, HostInfo, Label } from '../../lib/types';
import { copyText, cx, durWords, pref } from '../../lib/util';
import { buildCron, describeCron, normalizeCron, type Schedule } from '../../lib/cron';
import { allBoards, hostKey, optimistic, saveSetting, useData } from '../../store/data';
import { confirm, openHost, openPicker, setTab, toast, toastError, updatePicker, useUI } from '../../store/ui';
import { LabelChip, Select, Sync, Title, useNow, useSync, type SyncState } from '../bits';
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
      if (ui.confirm || ui.palette.open || ui.ctx) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      // 焦点在选择框上：上下键是打开它的选项，不是换机器
      if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && (e.target as HTMLElement).closest?.('.sel')) return;
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

type HostPatch = Parameters<typeof api.updateHost>[1];
type Save = (p: HostPatch) => void;

/** 改的是哪个设置项：保存状态、连点合并都按它分。领取规则和兜底算同一项，启动环境按 agent 分 */
const fieldOf = (p: HostPatch) => (p.env ? `env:${p.env.agent}` : 'rules' in p || 'fallback' in p ? 'rules' : Object.keys(p)[0] ?? 'host');
/** 步进器连点：停手这么久才发请求，只发最后的值 */
const STEPPED = new Set(['max', 'perSession']);
const STEP_MS = 450;
/** 机器设置的保存：每一项各存各的（store 的 saveSetting）。改动当帧生效，不禁用任何控件，没确认时也能接着改；
    sync 是每一项的保存状态，画在那个控件旁边：保存中（转圈，晚一点才出现）→ 已保存（打勾后淡出）；
    被拒绝时值回到原处、那一行回弹，并提示 */
function useSaver(name: string) {
  const { sync, track } = useSync();
  const save: Save = p => {
    const f = fieldOf(p);
    const { env, ...fields } = p;
    const done = saveSetting(
      `${hostKey(name)}:${f}`,
      {
        ws: ws => ({
          ...ws,
          hosts: ws.hosts?.map(x =>
            x.name === name ? { ...x, ...fields, agents: env ? x.agents.map(a => (a.id === env.agent ? { ...a, env: env.env } : a)) : x.agents } : x,
          ),
        }),
      },
      () => api.updateHost(name, p),
      STEPPED.has(f) ? STEP_MS : 0,
    );
    track(f, done);
  };
  return { save, sync };
}

/** 步进器：数字当帧变，往上加从下面滚进来、往下减从上面滚进来 */
function Stepper({ value, min = 1, max = 8, label, onChange }: { value: number; min?: number; max?: number; label: string; onChange: (v: number) => void }) {
  const prev = useRef(value);
  const up = value >= prev.current;
  useEffect(() => {
    prev.current = value;
  }, [value]);
  return (
    <span className="hp-stepper" role="group" aria-label={label}>
      <button type="button" aria-label="减少" disabled={value <= min} onClick={() => onChange(value - 1)}>
        −
      </button>
      <b aria-live="polite">
        <span key={value} className="hp-num" style={{ ['--roll' as string]: up ? '80%' : '-80%' }}>
          {value}
        </span>
      </b>
      <button type="button" aria-label="增加" disabled={value >= max} onClick={() => onChange(value + 1)}>
        +
      </button>
    </span>
  );
}

function HostBody({ h, i, n, step, close, on }: { h: HostInfo; i: number; n: number; step: (d: number) => void; close: () => void; on: boolean }) {
  const ws = useData(s => s.ws);
  const now = useNow(1000);
  const { running, queued } = hostLoad(ws, h);
  // 没有一条领取规则时定时领取不会领卡，「现在领一轮」也不能开始
  const hasRules = (h.rules?.length ?? 0) > 0;
  const [recent, setRecent] = useState<Dispatch[] | null>(null);
  const rev = ws?.rev;
  /** 刚一键装好 board-skills 的 agent：它们那一行的胶囊变绿，横幅消失时清掉 */
  const [fixed, setFixed] = useState<string[] | null>(null);

  const [recentOpen, setRecentOpen] = useState(() => pref.get('hostFold.recent', true));
  const toggleRecent = () => {
    pref.set('hostFold.recent', !recentOpen);
    setRecentOpen(!recentOpen);
  };
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

  const { save, sync } = useSaver(h.name);
  const failed = (...fs: string[]) => fs.some(f => sync[f] === 'failed') && 'hp-rollback';
  const canRound = h.online && !h.paused && !h.round && hasRules;
  const roundTip = h.round
    ? '正在领一轮：有空位就领下一张，所有白板都没有符合规则的卡片时结束'
    : !hasRules
      ? '先添加一条领取规则'
      : !h.online
        ? '机器离线，不能领'
        : h.paused
          ? '暂停接单时不能领'
          : '现在按领取规则领一轮，效果与定时到点相同';
  const round = async () => {
    // 当帧变成「正在领」，被拒绝时改回来并提示
    try {
      await optimistic(patchHost(h.name, x => ({ ...x, round: true })), () => api.hostRound(h.name));
      toast({ type: 'success', title: `${h.name} 开始领一轮` });
    } catch (e) {
      toastError(e);
    }
  };
  const remove = async () => {
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
          <button aria-label="上一台" className="icon-btn sm" type="button" data-tip="上一台  ↑ / K" disabled={i <= 0} onClick={() => step(-1)}>
            <Icon name="chevUp" />
          </button>
          <span className="d-pos mono">
            {i + 1} / {n}
          </span>
          <button aria-label="下一台" className="icon-btn sm" type="button" data-tip="下一台  ↓ / J" disabled={i >= n - 1} onClick={() => step(1)}>
            <Icon name="chevDown" />
          </button>
        </span>
        <span className="spacer" />
        <button aria-label="关闭" className="icon-btn" type="button" data-tip="关闭  esc" data-tip-align="end" onClick={close}>
          <Icon name="close" />
        </button>
      </div>
      <div className="d-grid">
        <div className="d-main">
          {/* 顶部：机器名和最常用的两个操作（领一轮、暂停接单） */}
          <section className="d-section hp-hero" style={{ ['--i' as string]: 0 }}>
            <div className="hp-hero-id">
              <h2 className="hp-m-title">{h.name}</h2>
              <div className="hp-m-meta">
                <span>{h.os || '未知系统'}</span>
                <span>board {h.version || '?'}</span>
                <span>{lastSeen(h, now)}</span>
              </div>
            </div>
            <div className="hp-hero-acts">
              <span data-tip={roundTip}>
                <button className={cx('btn sm', h.round && 'is-busy')} type="button" disabled={!canRound} onClick={round}>
                  <Icon name="play" />
                  {h.round ? '正在领一轮' : '领一轮'}
                </button>
              </span>
              <span className="hp-ctl">
                <Sync s={sync.paused} />
                <button
                  className={cx('btn sm hp-pause', h.paused && 'on', failed('paused'))}
                  type="button"
                  aria-pressed={h.paused}
                  data-tip={h.paused ? '恢复后接着做排队的派发' : '暂停后不启动新的派发，在跑的不受影响'}
                  data-tip-align="end"
                  onClick={() => save({ paused: !h.paused })}
                >
                  <Icon name={h.paused ? 'play' : 'pause'} />
                  {h.paused ? '恢复接单' : '暂停接单'}
                </button>
              </span>
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
            <div className="d-label">接单</div>
            <div className="hp-knobs">
              <div className={cx('hp-knob', failed('max'))}>
                <div className="hp-knob-text">
                  <b>同时跑几个</b>
                  <span>跑满之后，新的派发在服务端排队</span>
                </div>
                <span className="hp-ctl">
                  <Sync s={sync.max} />
                  <Stepper value={h.max} label="同时跑几个" onChange={v => save({ max: v })} />
                </span>
              </div>
              {h.perSession != null && (
                <div className={cx('hp-knob', failed('perSession'))}>
                  <div className="hp-knob-text">
                    <b>一个会话最多做几张</b>
                    <span data-tip="定时领取启动的 agent 做完一张后用 board next 接着做，优先挑和第一张相近的，做满就退出、下一批换新会话；手动派发只做指定的那一张">
                      定时领取的会话做满就换新会话
                    </span>
                  </div>
                  <span className="hp-ctl">
                    <Sync s={sync.perSession} />
                    <Stepper value={h.perSession} label="一个会话最多做几张" onChange={v => save({ perSession: v })} />
                  </span>
                </div>
              )}
            </div>
            <div className={cx('hp-block', failed('cronOn', 'cron'))}>
              <div className="hp-block-head">
                <div className="hp-knob-text">
                  <b>定时领取</b>
                  <span>到了下面这些时间，按领取规则领卡、把空位填满，直到没有符合规则的卡片</span>
                </div>
                <span className="hp-ctl">
                  <Sync s={sync.cronOn} />
                  <button className={cx('toggle', h.cronOn && 'on')} type="button" role="switch" aria-checked={h.cronOn} aria-label="定时领取" onClick={() => save({ cronOn: !h.cronOn })}>
                    <i />
                  </button>
                </span>
              </div>
              <CronTimes h={h} save={save} sync={sync.cron} />
            </div>
            {h.rules != null && <ClaimRules h={h} save={save} sync={sync.rules} />}
          </section>

          <section className="d-section" style={{ ['--i' as string]: 3 }}>
            <div className="d-label">本机 agent</div>
            <SkillBanner h={h} fixed={fixed} setFixed={setFixed} />
            {h.agents.map(a => (
              <AgentRow key={a.id} h={h} a={a} save={save} sync={sync[`env:${a.id}`] ?? (a.id === h.agent ? sync.agent : undefined)} fixed={fixed?.includes(a.id)} />
            ))}
            {h.agents.length === 0 && <p className="hp-note">board host 没有上报 agent</p>}
          </section>

          {!!h.envs?.length && (
            <Fold id="envs" title="启动环境" n={h.envs.length} i={4}>
              {h.envs.map(e => (
                <div className="hp-dir hp-env" key={e.id}>
                  <span className="hp-board" title={e.name}>
                    {e.name}
                  </span>
                  {e.path && <MidPath path={e.path} />}
                  {e.id === 'direct' && <span className="hp-env-hint">不经过 shell，直接执行命令</span>}
                </div>
              ))}
            </Fold>
          )}

          <Fold id="dirs" title="白板与目录" n={h.dirs.length} i={5}>
            <DirsEditor h={h} save={save} sync={sync.dirs} hosts={hostsOf(ws)} now={now} />
          </Fold>

          <section className="d-section hp-danger" style={{ ['--i' as string]: 6 }}>
            <div>
              <div className="hp-knob-text">
                <b>移除这台机器</b>
                <span>排队的取消，在跑的停止，机器设置删除；派发记录保留</span>
              </div>
              <button className="btn sm danger" type="button" onClick={remove}>
                <Icon name="trash" />
                移除机器
              </button>
            </div>
          </section>
        </div>

        <div className={cx('d-side', !recentOpen && 'folded')}>
          <div className="d-label d-side-head">
            <button type="button" className="hp-fold-btn" aria-expanded={recentOpen} onClick={toggleRecent}>
              <Icon name="chevDown" />
              最近 <span className="count">{recent?.length ?? ''}</span>
            </button>
            <span className="act">
              今天交审 {today.review} · 异常退出 {today.crashed}
            </span>
          </div>
          <div className="d-activity" hidden={!recentOpen}>
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
                        <span className="hp-cid">#{p.card}</span> <Title text={p.title} />
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

/** 白板与目录：逐行编辑、检查状态、缺失时克隆、复制到其他机器 */
function DirsEditor({ h, save, sync, hosts, now }: { h: HostInfo; save: Save; sync?: SyncState; hosts: HostInfo[]; now: number }) {
  const ws = useData(s => s.ws);
  const boards = allBoards(ws).map(b => b.path);
  const key = h.dirs.map(d => `${d.board}\u0000${d.path}`).join('\u0001');
  const [paths, setPaths] = useState<Record<string, string>>({});
  const [draft, setDraft] = useState({ board: '', path: '' });
  const [target, setTarget] = useState('');
  const [cloneUrls, setCloneUrls] = useState<Record<string, string>>({});
  const [cloning, setCloning] = useState('');
  const [copying, setCopying] = useState(false);
  const [active, setActive] = useState<string | null>(null);
  const [local, setLocal] = useState(false);
  useEffect(() => {
    setPaths(Object.fromEntries(h.dirs.map(d => [d.board, d.path])));
    setCloneUrls({});
    setDraft(v => ({ ...v, board: v.board || boards.find(b => !h.dirs.some(d => d.board === b)) || '' }));
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (shell !== 'desktop') return;
    let alive = true;
    void localHost().then(x => alive && setLocal(isLocalHost(h, x)));
    return () => {
      alive = false;
    };
  }, [h]); // eslint-disable-line react-hooks/exhaustive-deps

  const others = hosts.filter(x => x.name !== h.name);
  const fixed = h.dirs.some(d => d.fixed);
  const apply = async (next: HostDir[]) => {
    if (
      local &&
      !(await confirm({
        title: '修改这台电脑接任务的目录？',
        text: '保存后 board host 会换上新的白板与目录，正在跑的 agent 不受影响。',
        ok: '保存',
        icon: 'alert',
        tone: 'warn',
      }))
    ) {
      return;
    }
    save({ dirs: next.map(d => ({ board: d.board, path: d.path })) });
  };
  const originOf = (board: string) => hosts.find(x => x.name !== h.name)?.dirs.find(d => d.board === board && d.origin)?.origin ?? '';
  const when = h.dirsChangedAt ? new Date(h.dirsChangedAt).toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '';

  const commitPath = (d: HostDir) => {
    const path = (paths[d.board] ?? d.path).trim();
    if (path && path !== d.path) void apply(h.dirs.map(x => (x.board === d.board ? { ...x, path } : x)));
  };

  const add = () => {
    const board = draft.board.trim();
    const path = draft.path.trim();
    if (!board || !path || h.dirs.some(d => d.board === board)) return;
    void apply([...h.dirs, { board, path }]);
    setDraft({ board: '', path: '' });
  };

  const clone = async (d: HostDir) => {
    const url = (cloneUrls[d.board] ?? originOf(d.board)).trim();
    if (!url) return;
    setCloning(d.board);
    try {
      await optimistic(
        patchHost(h.name, x => ({
          ...x,
          dirs: x.dirs.map(y =>
            y.board === d.board ? { ...y, clone: { state: 'queued', url, message: '等待 board host 开始克隆', updatedAt: Date.now(), by: '' } } : y,
          ),
        })),
        () => api.cloneHostDir(h.name, d.board, url),
      );
    } catch (e) {
      toastError(e);
    } finally {
      setCloning('');
    }
  };

  const copy = async () => {
    const to = others.find(x => x.name === target);
    if (!to) return;
    const carried: HostDir[] = [];
    for (const d of h.dirs) {
      if (!d.path.startsWith('~')) {
        const yes = await confirm({
          title: `把 ${d.board} 的绝对路径带过去？`,
          text: `目标机器会使用 ${d.path}。如果那台机器上的目录不同，带过去后还要再改。`,
          ok: '带过去',
          icon: 'alert',
          tone: 'warn',
        });
        if (!yes) return;
      }
      carried.push({ board: d.board, path: d.path });
    }
    const next = [...to.dirs.filter(x => !carried.some(d => d.board === x.board)), ...carried];
    setCopying(true);
    try {
      await saveSetting(
        `${hostKey(to.name)}:dirs`,
        patchHost(to.name, x => ({ ...x, dirs: next })),
        () => api.updateHost(to.name, { dirs: next.map(d => ({ board: d.board, path: d.path })) }),
      );
      toast({ type: 'success', title: `已复制到 ${to.name}`, text: `${carried.length} 块白板；以服务端保存的设置为准` });
    } catch (e) {
      toastError(e);
    } finally {
      setCopying(false);
    }
  };

  return (
    <div className="hp-dirs">
      {fixed && <p className="hp-note">这台机器用 --dir 启动，带「固定」的行要在那台机器上改启动参数。</p>}
      {when && h.dirsChangedBy && h.dirsChangedBy !== '本机' && (
        <p className="hp-note hp-remote">{when} 由 {h.dirsChangedBy} 在别的设备上修改</p>
      )}
      {h.dirs.length ? (
        h.dirs.map(d => {
          const state = d.status ?? '';
          const cloneState = d.clone?.state;
          return (
            <div className={cx('hp-dir hp-dir-edit', state === 'missing' && 'warn', state === 'not_git' && 'warn')} key={d.board}>
              <div className="hp-dir-main">
                <span className="hp-board">{d.board}</span>
                <Icon name="move" />
                <input
                  className="input hp-dir-path"
                  value={paths[d.board] ?? d.path}
                  readOnly={d.fixed}
                  spellCheck={false}
                  aria-label={`${d.board} 的目录`}
                  onFocus={() => setActive(d.board)}
                  onChange={e => setPaths(v => ({ ...v, [d.board]: e.target.value }))}
                  onBlur={() => commitPath(d)}
                  onKeyDown={e => {
                    if (e.key === 'Enter') e.currentTarget.blur();
                  }}
                />
                <span className={cx('hp-dir-state', state)} data-tip={d.checkedAt ? `检查于 ${since(d.checkedAt, now)}` : undefined}>
                  {state === 'ok' ? `git${d.branch ? ` · ${d.branch}` : ''}` : state === 'missing' ? '不存在' : state === 'not_git' ? '不是 git 仓库' : '等待检查'}
                </span>
                <Sync s={active === d.board ? sync : undefined} />
                {d.fixed ? (
                  <span className="hp-dir-fixed">--dir 固定</span>
                ) : (
                  <button className="icon-btn sm" type="button" aria-label="去掉这一行" onClick={() => void apply(h.dirs.filter(x => x.board !== d.board))}>
                    <Icon name="close" />
                  </button>
                )}
              </div>
              {state !== 'missing' && d.clone && (
                <div className="hp-clone">
                  <span className={cx('hp-clone-msg', cloneState === 'failed' && 'bad', cloneState === 'done' && 'ok')} title={d.clone.message}>
                    {d.clone.message}
                  </span>
                </div>
              )}
              {state === 'missing' && !d.fixed && (
                <div className="hp-clone">
                  <input
                    className="input"
                    value={cloneUrls[d.board] ?? originOf(d.board)}
                    placeholder="仓库地址，例如 git@github.com:org/repo.git"
                    spellCheck={false}
                    aria-label={`${d.board} 的仓库地址`}
                    onChange={e => setCloneUrls(v => ({ ...v, [d.board]: e.target.value }))}
                  />
                  <button
                    className={cx('btn sm', cloning === d.board && 'is-busy')}
                    type="button"
                    disabled={cloning === d.board || !(cloneUrls[d.board] ?? originOf(d.board)).trim()}
                    onClick={() => void clone(d)}
                  >
                    <Icon name="folder" />
                    克隆到这里
                  </button>
                  {d.clone && (
                    <span className={cx('hp-clone-msg', cloneState === 'failed' && 'bad', cloneState === 'done' && 'ok')} title={d.clone.message}>
                      {d.clone.message || (cloneState === 'queued' ? '等待 board host 开始克隆' : '正在克隆…')}
                    </span>
                  )}
                </div>
              )}
            </div>
          );
        })
      ) : (
        <p className="hp-note">没有白板：在下面加一块，或在那台机器上用 --dir 启动。</p>
      )}
      <div className="hp-dir-add">
        <Select
          value={draft.board}
          options={[
            { value: '', label: '选白板…' },
            ...boards.filter(b => !h.dirs.some(d => d.board === b)).map(b => ({ value: b, label: b })),
          ]}
          onChange={board => setDraft(v => ({ ...v, board }))}
          aria-label="要加的白板"
        />
        <input
          className="input"
          value={draft.path}
          placeholder="~/repo/board"
          spellCheck={false}
          aria-label="目录"
          onChange={e => setDraft(v => ({ ...v, path: e.target.value }))}
        />
        <button className="btn sm" type="button" disabled={!draft.board || !draft.path.trim()} onClick={add}>
          <Icon name="plus" />
          加一块白板
        </button>
      </div>
      {others.length > 0 && h.dirs.length > 0 && (
        <div className="hp-dir-copy">
          <Select
            value={target}
            options={[{ value: '', label: '复制到其他机器…' }, ...others.map(x => ({ value: x.name, label: x.name }))]}
            onChange={setTarget}
            aria-label="复制到其他机器"
          />
          <button className={cx('btn sm', copying && 'is-busy')} type="button" disabled={!target} onClick={() => void copy()}>
            复制过去
          </button>
        </div>
      )}
    </div>
  );
}

/** 本机 agent 的一行：图标、名称（看不到 board-skills 时后面一个「缺 skill」小胶囊，悬停写装到哪里；刚一键装好的变绿「skill 已装」）、
 *  启动环境（下拉，每项带版本号，找不到的置灰写「没找到」）、「默认」（点一下设为默认）。
 *  旧版 board host 不上报启动环境：不显示下拉，写版本号 */
function AgentRow({ h, a, save, sync, fixed }: { h: HostInfo; a: HostAgent; save: Save; sync?: SyncState; fixed?: boolean }) {
  const def = a.id === h.agent;
  const envs = h.envs ?? [];
  const checks = a.envs ?? [];
  const name = (id: string) => envName(h, id) ?? id;
  const miss = skillMiss(h, a);
  return (
    <>
      <div className={cx('hp-arow', !a.found && 'missing', sync === 'failed' && 'hp-rollback')} style={{ ['--ac' as string]: agentMark(a.id).color }}>
        <AgentIcon id={a.id} />
        <b className="hp-arow-name">{a.name}</b>
        {fixed ? (
          <span className="hp-skill ok">
            <Icon name="check" />
            skill 已装
          </span>
        ) : (
          miss && (
            <span className="hp-skill" data-tip={miss} tabIndex={0}>
              <Icon name="alert" />
              缺 skill
            </span>
          )
        )}
        {envs.length === 0 && <em className="hp-arow-ver">{a.found ? a.version || '找到了' : '没找到'}</em>}
        <span className="hp-sp" />
        <span className="hp-ctl">
          <Sync s={sync} />
        {envs.length > 0 &&
          (checks.length ? (
            <Select
              className="hp-env-select"
              disabled={!checks.some(c => c.found)}
              value={a.env ?? ''}
              placeholder="没找到"
              options={checks.map(c => ({ value: c.env, label: `${name(c.env)} · ${c.found ? c.version || '找到了' : '没找到'}`, disabled: !c.found && c.env !== a.env }))}
              onChange={v => save({ env: { agent: a.id, env: v } })}
              aria-label={`${a.name} 的启动环境`}
              data-tip="启动环境：在哪种 shell 里启动它；改了下一次派发就用新的"
            />
          ) : (
            <span className="hp-arow-none">本机没有它支持的启动环境</span>
          ))}
        <button
          type="button"
          className={cx('hp-def', def && 'on')}
          disabled={!a.found}
          onClick={() => !def && save({ agent: a.id })}
          data-tip={!a.found ? '这台机器上没找到命令' : def ? '定时领取用它；手动派发时预先选中它' : '设为默认'}
          data-tip-align="end"
        >
          {def ? '默认' : '设为默认'}
        </button>
        </span>
      </div>
    </>
  );
}

/** 刚装好的提示停留的时间 */
const SKILL_DONE_MS = 5000;

/** 「本机 agent」上方的横幅：有 agent 看不到 board-skills 时出现。
 *  在桌面端里看的就是这台电脑：右侧「一键安装」，桌面端写进 agent 读 skill 的目录（src-tauri/src/skill.rs），装好变绿 5 秒后消失；
 *  网页端、别的机器、WSL 里的 agent：没有按钮，写怎么手动装 */
function SkillBanner({ h, fixed, setFixed }: { h: HostInfo; fixed: string[] | null; setFixed: (v: string[] | null) => void }) {
  const [local, setLocal] = useState<LocalHost | null>(null);
  const [installing, setInstalling] = useState(false);
  const [noSupport, setNoSupport] = useState(false);
  const [done, setDone] = useState<string[] | null>(null);
  useEffect(() => {
    if (shell !== 'desktop') return;
    let live = true;
    void localHost().then(s => live && setLocal(s));
    return () => void (live = false);
  }, [h.name]);
  useEffect(() => {
    if (!done) return;
    const t = window.setTimeout(() => {
      setDone(null);
      setFixed(null);
    }, SKILL_DONE_MS);
    return () => window.clearTimeout(t);
  }, [done, setFixed]);

  const gaps = skillGaps(h);
  if (done) {
    return (
      <div className="hp-skill-ban ok" role="status">
        <Icon name="checkCircle" />
        <p>
          board-skills 已装好，{fixed?.length ?? 0} 个 agent 都能接卡片了
          <small>装在 {done.join('、')}</small>
        </p>
      </div>
    );
  }
  if (!gaps.missing.length) return null;
  const can = shell === 'desktop' && !noSupport && isLocalHost(h, local) && gaps.auto.length > 0;
  const ids = gaps.auto.map(a => a.id);

  const install = async () => {
    setInstalling(true);
    try {
      const paths = await installSkill(ids);
      setFixed(ids);
      setDone(paths);
    } catch (e) {
      if (e === 'unsupported') {
        setNoSupport(true);
        toast({ type: 'info', title: '这个版本的桌面端不能一键安装', text: '先更新桌面端，或按 docs/agents.md 手动安装' });
      } else toastError(e);
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div className="hp-skill-ban" role="status">
      <Icon name="alert" />
      <p>
        {gaps.missing.length} 个 agent 看不到 board-skills，派给它们的卡片会做不成
        <small>
          {can ? (
            <>
              装到 {gaps.dirs.join('、')}（这台电脑）
              {gaps.manualPlaces.length > 0 && <>；{gaps.manualPlaces.join('、')} 要手动装（见 docs/agents.md）</>}
            </>
          ) : (
            <>在那台电脑的 Board 桌面端里一键安装，或手动装到 {gaps.places.join('、')}（见 docs/agents.md）</>
          )}
        </small>
      </p>
      {can && (
        <button type="button" className={cx('btn sm', installing && 'is-busy')} disabled={installing} onClick={() => void install()}>
          <Icon name="download" />
          {installing ? '正在安装…' : '一键安装'}
        </button>
      )}
    </div>
  );
}

const OPS: [ClaimRule['op'], string][] = [
  ['in', '带其中任一个'],
  ['all', '全都带'],
  ['not_in', '一个都不带'],
  ['equal', '标签正好是'],
  ['only', '只在这几个里'],
  ['none', '没有标签'],
];
const OP_OPTIONS = OPS.map(([value, label]) => ({ value, label }));
/** 「没有标签」不带标签列表：这一行不选标签，只看卡片有没有标签 */
const NEEDS_LABELS = (op: ClaimRule['op']) => op !== 'none';

/** 一条规则里的标签：和卡片详情的标签一行一样——标签胶囊（项目里的颜色，× 去掉）加标签按钮，点开同一个标签选择器，
    勾一个就应用一个。机器只对应一个项目时就是那个项目的选择器（能新建、管理标签）；对应几个项目时只列这几个项目里的标签
    （同名的按第一个），不能新建。项目里已经没有的标签灰色划掉、原样保留 */
function LabelSet({ value, onChange, labels, project, missing, disabled }: {
  value: string[];
  onChange: (v: string[]) => void | Promise<unknown>;
  labels: Label[];
  /** 机器只对应一个项目时是它的标识：选择器用这个项目的标签（id 是真的），可以新建、管理 */
  project?: string;
  missing?: string[];
  disabled?: boolean;
}) {
  const pool = () => (project ? (useData.getState().ws?.projects.find(p => p.key === project)?.labels ?? labels) : labels);
  const byName = (n: string, ls = labels) => ls.find(l => l.name.toLowerCase() === n.toLowerCase());
  const ids = (names: string[]) => names.map(n => byName(n)).filter((l): l is Label => !!l).map(l => l.id);
  const open = (anchor: HTMLElement) => {
    // 不在列表里的（项目里已经没有的）标签选择器里没有，原样留在规则里
    const kept = value.filter(n => !byName(n));
    let applied = value;
    let latest = ids(value);
    // 勾一个应用一个。机器设置保存完之前不能再改（BUSY）：连点时排队，轮到时按选择器最新的勾选保存，中间几次合并成一次
    let chain: Promise<unknown> = Promise.resolve();
    openPicker({
      rect: anchor.getBoundingClientRect(),
      project: project ?? '',
      labels: project ? undefined : labels,
      selected: latest,
      onToggle: () => {
        latest = useUI.getState().picker?.selected ?? latest;
        chain = chain.then(async () => {
          const ls = pool();
          const next = [...ls.filter(l => latest.includes(l.id)).map(l => l.name), ...kept];
          if (!next.length) {
            // 规则至少要有一个标签：一个都不勾时不改，勾回原样
            updatePicker(applied.map(n => byName(n, ls)).filter((l): l is Label => !!l).map(l => l.id));
            return;
          }
          if (next.length === applied.length && next.every(n => applied.includes(n))) return;
          applied = next;
          await onChange(next);
        });
      },
    });
  };
  return (
    <span className="hp-lset">
      {value.map(n => {
        const l = byName(n);
        const off = !l || missing?.some(m => m.toLowerCase() === n.toLowerCase());
        return (
          <span key={n} className={cx('hp-lset-chip', off && 'off')} title={off ? '项目里没有这个标签' : undefined}>
            <LabelChip label={l ?? { id: -1, name: n, color: 'var(--text-4)' }} onRemove={() => onChange(value.filter(x => x !== n))} removeDisabled={disabled} />
          </span>
        );
      })}
      <button type="button" className="lb-add" aria-label="添加标签" data-label-picker disabled={disabled} onClick={e => open(e.currentTarget)}>
        <Icon name="tag" />
        {value.length ? '' : '选标签'}
      </button>
    </span>
  );
}

/** 领取规则：条件（带其中任一个 / 全都带 / 一个都不带 / 标签正好是 / 只在这几个里 / 没有标签）→ agent，
 *  有顺序（第一条命中的生效），最后可选兜底。只影响定时领取和会话里的 board next，手动派发、board claim 不受影响 */
function ClaimRules({ h, save: put, sync }: { h: HostInfo; save: Save; sync?: SyncState }) {
  const ws = useData(s => s.ws);
  const rules = h.rules ?? [];
  const found = h.agents.filter(a => a.found);
  const agentName = (id: string) => h.agents.find(a => a.id === id)?.name ?? id;
  // 这台机器有目录的项目里的标签，同名（不区分大小写）的只留第一个（颜色取它的）；id 换成序号，选择器按它勾选
  const labels = useMemo(() => {
    const seen = new Set<string>();
    return (ws?.projects ?? [])
      .filter(p => h.dirs.some(d => d.board.startsWith(`${p.key}/`)))
      .flatMap(p => p.labels)
      .filter(l => !seen.has(l.name.toLowerCase()) && !!seen.add(l.name.toLowerCase()))
      .map((l, i) => ({ ...l, id: i + 1 }));
  }, [ws, h.dirs]);
  // 只对应一个项目时，用这个项目的标签和选择器（和卡片详情一样，能新建、管理标签）
  const keys = [...new Set(h.dirs.map(d => d.board.split('/')[0]))];
  const one = keys.length === 1 ? ws?.projects.find(p => p.key === keys[0]) : undefined;
  const [op, setOp] = useState<ClaimRule['op']>('in');
  const [picked, setPicked] = useState<string[]>([]);
  // 从「没有标签」换成要标签的条件：服务端不允许没有标签的规则，先只改界面，选了标签再一起保存
  const [draft, setDraft] = useState<{ i: number; op: ClaimRule['op'] } | null>(null);
  const [agent, setAgent] = useState('');
  const pick = agent || found[0]?.id || '';
  const has = (a: string) => found.some(x => x.id === a);
  const agentOptions = found.map(a => ({ value: a.id, label: a.name }));
  // 标签选择器是勾一个存一个：连着勾时回调可能来自上一帧，改的要是最新一份规则，不能把中间存下的改动覆盖回去
  const latest = useRef(rules);
  latest.current = rules;
  const save = (next: ClaimRule[]) => {
    latest.current = next;
    put({
      rules: next.map(r => ({
        op: r.op,
        labels: NEEDS_LABELS(r.op) ? r.labels : [],
        agent: r.agent,
        active: has(r.agent),
        missing: NEEDS_LABELS(r.op) ? r.missing : [],
      })),
    });
  };
  const edit = (i: number, change: Partial<ClaimRule>) => {
    // 改这一行的任何一项都从「还没选好标签」的状态里出来，下拉和存下的规则保持一致
    setDraft(null);
    save(latest.current.map((x, j) => (j === i ? { ...x, ...change, missing: change.labels ? [] : x.missing } : x)));
  };
  const changeOp = (i: number, next: ClaimRule['op']) => {
    if (NEEDS_LABELS(next) && !rules[i]?.labels.length) {
      setDraft({ i, op: next });
      return;
    }
    setDraft(null);
    edit(i, { op: next });
  };
  const move = (i: number, d: number) => {
    const next = [...rules];
    const [r] = next.splice(i, 1);
    next.splice(i + d, 0, r);
    setDraft(null);
    void save(next);
  };
  const add = () => {
    if ((NEEDS_LABELS(op) && !picked.length) || !pick) return;
    void save([...rules, { op, labels: NEEDS_LABELS(op) ? picked : [], agent: pick }]);
    setPicked([]);
  };
  const fallback = h.fallback || '';
  return (
    <div className={cx('hp-block hp-crules', sync === 'failed' && 'hp-rollback')}>
      <div className="hp-rule-text">
        <b>
          领取规则
          <Sync s={sync} inline />
        </b>
        <span>定时领取只领满足条件的卡片，交给对应的 agent；从上到下，第一条命中的生效。没有领取规则时不会自动领取，手动派发不受影响</span>
      </div>
      {rules.map((r, i) => (
        <div className={cx('hp-crule', !r.active && 'off')} key={`${i}-${r.op}-${r.labels.join(',')}`}>
          <Select
            className="hp-crule-op"
            value={draft?.i === i ? draft.op : r.op}
            options={OP_OPTIONS}
            aria-label="条件"
            onChange={v => changeOp(i, v as ClaimRule['op'])}
          />
          {NEEDS_LABELS(draft?.i === i ? draft.op : r.op) ? (
            <LabelSet
              value={r.labels}
              labels={one?.labels ?? labels}
              project={one?.key}
              missing={r.missing}
              onChange={v => {
                if (!v.length) return;
                if (draft?.i !== i) return void edit(i, { labels: v });
                // 选好标签：新条件和标签一起保存
                const next = draft.op;
                setDraft(null);
                edit(i, { op: next, labels: v });
              }}
            />
          ) : (
            <span className="hp-lset hp-lset-none">卡片没有任何标签</span>
          )}
          <Icon name="chevRight" />
          <Select
            className="hp-crule-agent"
            value={r.agent}
            options={[...(has(r.agent) ? [] : [{ value: r.agent, label: `${agentName(r.agent)}（找不到）` }]), ...agentOptions]}
            aria-label="由哪个 agent 做"
            onChange={v => edit(i, { agent: v })}
          />
          <span className="hp-sp" />
          <button type="button" className="icon-btn sm" aria-label="上移" disabled={i === 0} onClick={() => move(i, -1)}>
            <Icon name="chevUp" />
          </button>
          <button type="button" className="icon-btn sm" aria-label="下移" disabled={i === rules.length - 1} onClick={() => move(i, 1)}>
            <Icon name="chevDown" />
          </button>
          <button
            type="button"
            className="icon-btn sm"
            aria-label="删除这条规则"
            onClick={() => {
              setDraft(null);
              void save(rules.filter((_, j) => j !== i));
            }}
          >
            <Icon name="trash" />
          </button>
          {draft?.i === i && <em className="hp-crule-warn">选好标签后才保存这个条件</em>}
          {!r.active && <em className="hp-crule-warn">这台机器上找不到 {agentName(r.agent)}，这条规则不生效</em>}
          {!!r.missing?.length && <em className="hp-crule-warn">项目里没有标签 {r.missing.map(l => `「${l}」`).join('')}</em>}
        </div>
      ))}
      <div className="hp-crule add">
        <Select className="hp-crule-op" value={op} options={OP_OPTIONS} aria-label="新规则的条件" onChange={v => setOp(v as ClaimRule['op'])} />
        {NEEDS_LABELS(op) ? (
          <LabelSet value={picked} labels={one?.labels ?? labels} project={one?.key} onChange={setPicked} />
        ) : (
          <span className="hp-lset hp-lset-none">卡片没有任何标签</span>
        )}
        <Icon name="chevRight" />
        <Select className="hp-crule-agent" value={pick} options={agentOptions} aria-label="新规则的 agent" onChange={setAgent} />
        <button className="btn sm" type="button" disabled={(NEEDS_LABELS(op) && !picked.length) || !pick} onClick={add}>
          <Icon name="plus" />
          加规则
        </button>
      </div>
      {(rules.length > 0 || fallback) && (
        <div className={cx('hp-crule fallback', fallback && h.fallbackActive === false && 'off')}>
          <span className="hp-crule-label">其余卡片</span>
          <Icon name="chevRight" />
          <Select
            className="hp-crule-agent"
            value={fallback}
            options={[{ value: '', label: '不领' }, ...(fallback && !has(fallback) ? [{ value: fallback, label: `${agentName(fallback)}（找不到）` }] : []), ...agentOptions]}
            aria-label="一条规则都没命中的卡片由哪个 agent 做"
            onChange={v => put({ fallback: v })}
          />
          {fallback && h.fallbackActive === false && <em className="hp-crule-warn">这台机器上找不到 {agentName(fallback)}，兜底不生效</em>}
        </div>
      )}
      {rules.length === 0 && (
        <p className="hp-unmatched">
          <Icon name="info" />还没有领取规则：定时领取不会自动领卡，「现在领一轮」也不能开始
        </p>
      )}
      {!!h.unmatched && !fallback && (
        <p className="hp-unmatched">
          <Icon name="info" />有 {h.unmatched} 张可领取的卡片没有匹配任何规则
        </p>
      )}
    </div>
  );
}

/** 次要区块：标题行点一下收起 / 展开，状态记在本机（默认收起），重开弹窗保持 */
function Fold({ id, title, n, i, children }: { id: string; title: string; n: number; i: number; children: React.ReactNode }) {
  const [open, setOpen] = useState(() => pref.get(`hostFold.${id}`, false));
  return (
    <section className={cx('d-section hp-fold', !open && 'folded')} style={{ ['--i' as string]: i }}>
      <button
        type="button"
        className="d-label hp-fold-btn"
        aria-expanded={open}
        onClick={() => {
          pref.set(`hostFold.${id}`, !open);
          setOpen(!open);
        }}
      >
        <Icon name="chevDown" />
        {title} <span className="count">{n}</span>
      </button>
      {open && <div className="hp-fold-body">{children}</div>}
    </section>
  );
}

/** 长路径中间省略：开头和最后一级目录始终可见，悬停看全路径 */
function MidPath({ path }: { path: string }) {
  const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  const head = cut > 0 ? path.slice(0, cut + 1) : '';
  const tail = cut > 0 ? path.slice(cut + 1) : path;
  return (
    <code className="hp-path" title={path}>
      <span className="hp-path-head">{head}</span>
      <span className="hp-path-tail">{tail}</span>
    </code>
  );
}

const KINDS: [Schedule['kind'], string][] = [
  ['daily', '每天'],
  ['weekdays', '工作日'],
  ['weekly', '每周'],
  ['monthly', '每月'],
  ['minutes', '每隔几分钟'],
  ['hours', '每隔几小时'],
];
const WEEKDAYS: [number, string][] = [[1, '一'], [2, '二'], [3, '三'], [4, '四'], [5, '五'], [6, '六'], [0, '日']];
const EVERY = { minutes: [5, 10, 15, 20, 30], hours: [1, 2, 3, 4, 6, 8, 12] };
const two = (n: number) => String(n).padStart(2, '0');
const KIND_OPTIONS = KINDS.map(([value, label]) => ({ value, label }));
const DAY_OPTIONS = Array.from({ length: 31 }, (_, i) => ({ value: String(i + 1), label: `${i + 1} 号` }));
const HOUR_OPTIONS = Array.from({ length: 24 }, (_, i) => ({ value: String(i), label: two(i) }));
const MIN_OPTIONS = Array.from({ length: 12 }, (_, i) => ({ value: String(i * 5), label: two(i * 5) }));

/** 定时领取：胶囊列表（写说明，悬停看 cron 表达式），可去掉；加的时候选「每天 / 工作日 / 每周几 / 每月几号」和时间，
    或「每隔几分钟 / 几小时」，由 buildCron 生成表达式，不让人手写（手写容易错）。旧的手写表达式说不出来时原样显示 */
function CronTimes({ h, save, sync }: { h: HostInfo; save: Save; sync?: SyncState }) {
  // 加时间的那一排选择器平时收着，点「加时间」才展开；一个时间点都没有时直接展开
  const [adding, setAdding] = useState(h.cron.length === 0);
  const [kind, setKind] = useState<Schedule['kind']>('daily');
  const [hour, setHour] = useState(9);
  const [min, setMin] = useState(0);
  const [days, setDays] = useState<number[]>([1]);
  const [day, setDay] = useState(1);
  const [every, setEvery] = useState({ minutes: 15, hours: 1 });
  const spec: Schedule =
    kind === 'minutes' || kind === 'hours'
      ? { kind, every: every[kind] }
      : kind === 'weekly'
        ? { kind, days, h: hour, m: min }
        : kind === 'monthly'
          ? { kind, day, h: hour, m: min }
          : { kind, h: hour, m: min };
  const cron = buildCron(spec);
  const dup = !!cron && h.cron.map(normalizeCron).includes(cron);
  const add = () => {
    if (!cron || dup) return;
    save({ cron: [...h.cron, cron] });
  };
  const timed = kind !== 'minutes' && kind !== 'hours';
  return (
    <div className={cx('hp-cron', !h.cronOn && 'off')}>
      <div className="hp-times">
      {h.cron.map(x => (
        <span className="hp-time" key={x} title={normalizeCron(x)}>
          {/* 说不出来的（旧的手写表达式）原样用等宽字显示 */}
          {describeCron(x) === normalizeCron(x) ? <code>{normalizeCron(x)}</code> : describeCron(x)}
          <button type="button" aria-label={`去掉 ${describeCron(x)}`} onClick={() => save({ cron: h.cron.filter(y => y !== x) })}>
            <Icon name="close" />
          </button>
        </span>
      ))}
        {!adding && (
          <button type="button" className="hp-time-add" onClick={() => setAdding(true)}>
            <Icon name="plus" />
            加时间
          </button>
        )}
        <Sync s={sync} inline />
      </div>
      {adding && (
      <span className="hp-sched" role="group" aria-label="加一个定时">
        <Select value={kind} options={KIND_OPTIONS} aria-label="多久一次" onChange={v => setKind(v as Schedule['kind'])} />
        {kind === 'weekly' && (
          <span className="hp-wdays">
            {WEEKDAYS.map(([d, n]) => {
              const on = days.includes(d);
              return (
                <button
                  key={d}
                  type="button"
                  className={cx('hp-wday', on && 'on')}
                  aria-pressed={on}
                  aria-label={`周${n}`}
                  onClick={() => setDays(on ? days.filter(x => x !== d) : [...days, d])}
                >
                  {n}
                </button>
              );
            })}
          </span>
        )}
        {kind === 'monthly' && (
          <Select value={String(day)} options={DAY_OPTIONS} aria-label="几号" onChange={v => setDay(+v)} data-tip="没有这一天的月份（例如 2 月 30 号）不领" />
        )}
        {timed ? (
          <span className="hp-hm">
            <Select value={String(hour)} options={HOUR_OPTIONS} aria-label="时" onChange={v => setHour(+v)} />
            :
            <Select value={String(min)} options={MIN_OPTIONS} aria-label="分" onChange={v => setMin(+v)} />
          </span>
        ) : (
          <Select
            value={String(every[kind])}
            options={EVERY[kind].map(n => ({ value: String(n), label: `${n} ${kind === 'minutes' ? '分钟' : '小时'}` }))}
            aria-label={kind === 'minutes' ? '每隔几分钟' : '每隔几小时'}
            onChange={v => setEvery({ ...every, [kind]: +v })}
          />
        )}
        <span data-tip={dup ? '已经有这一条了' : !cron ? '至少选一天' : undefined}>
          <button className="btn sm" type="button" disabled={!cron || dup} onClick={add}>
            <Icon name="plus" />
            加上
          </button>
        </span>
        {h.cron.length > 0 && (
          <button className="hp-link" type="button" onClick={() => setAdding(false)}>
            收起
          </button>
        )}
      </span>
      )}
      <span className="hp-cron-hint">按这台机器的本地时间；同一分钟有几条到点只领一轮</span>
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
        <span className="hp-run-title">
          <Title text={r.title} />
        </span>
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
        <span className="hp-run-title">
          <Title text={r.title} />
        </span>
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
        <button aria-label="关闭" className="icon-btn" type="button" data-tip="关闭  esc" data-tip-align="end" onClick={close}>
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
