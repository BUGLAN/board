/* 机器详情：居中弹窗（就绪、正在跑、排队、派发规则、本机 agent、启动环境、白板与目录、最近）；添加机器 */
import { useEffect, useMemo, useState, useRef } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { agentMark, boardOf, dirReadiness, envName, hostLoad, hostNameError, hostsOf, installCommand, isLocalHost, queueReason, resultOf, sessionGroups, sessionQueue, since, skillGaps, skillMiss, SOURCE } from '../../lib/hosts';
import { installSkill, isWindows, localHost, shell, type LocalHost } from '../../lib/platform';
import type { ClaimRule, Dispatch, HostAgent, HostDir, HostInfo, Label } from '../../lib/types';
import { readiness, type ReadyGo } from '../../lib/ready';
import { COPY_ITEMS } from '../../lib/templates';
import { copyText, cx, durWords, pref, reduceMotion } from '../../lib/util';
import { buildCron, describeCron, normalizeCron, type Schedule } from '../../lib/cron';
import { allBoards, hostKey, optimistic, saveSetting, useData } from '../../store/data';
import { confirm, confirmCheck, openHost, openPicker, openSession, setTab, toast, toastError, updatePicker, useUI } from '../../store/ui';
import { LabelChip, Segmented, Select, Sync, Title, useNow, useSync, type SyncState } from '../bits';
import { Icon } from '../Icon';
import { cancel, goCard } from './Dispatch';
import { AgentIcon, Beat, lastSeen, Mark, OsGlyph, patchHost, StatePill, usePollHosts } from './bits';
import { ManageSection, versionLt } from './Manage';
import { ReadySection, useServerVersion } from './Ready';
import { SelfTestAgentButton, SelfTestSection } from './SelfTest';
import { STATE_WORDS } from '../../lib/skills';

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
      if (ui.confirm || ui.palette.open || ui.ctx || ui.session) return;
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

/** 改的是哪个设置项：保存状态、连点合并都按它分。领取规则和兜底算同一项，启动环境按 agent 分，模型、思考强度按 agent 各算一项 */
const fieldOf = (p: HostPatch) =>
  p.env
    ? `env:${p.env.agent}`
    : p.run
      ? `run:${p.run.agent}:${p.run.model !== undefined ? 'model' : 'effort'}`
      : 'rules' in p || 'fallback' in p
        ? 'rules'
        : (Object.keys(p)[0] ?? 'host');

/** 改一个 agent 的启动环境、模型、思考强度（当帧画出来） */
const agentPatch = (a: HostAgent, p: HostPatch): HostAgent => {
  if (p.env?.agent === a.id) return { ...a, env: p.env.env };
  if (p.run?.agent === a.id) {
    const r = p.run;
    return {
      ...a,
      ...(r.model !== undefined ? { run_model: r.model.trim() || null } : {}),
      ...(r.effort !== undefined ? { run_effort: r.effort || null } : {}),
    };
  }
  return a;
};
/** 步进器连点：停手这么久才发请求，只发最后的值 */
const STEPPED = new Set(['max', 'perSession']);
const STEP_MS = 450;

/** 单次最长运行可以选的档位（分钟，0 = 不限） */
const MAX_RUN_CHOICES = [0, 30, 60, 90, 120, 180, 240, 300, 360, 480, 600, 660, 720];
/** 没输出多久算卡住可以选的档位（分钟，0 = 不限） */
const IDLE_RUN_CHOICES = [0, 5, 10, 15, 20, 30, 45, 60, 75, 90, 105, 120];

/** 分钟数写成人看的话：3 小时、1 小时 30 分、20 分钟、不限 */
function minsWords(n: number): string {
  if (n <= 0) return '不限';
  const h = Math.floor(n / 60);
  const m = n % 60;
  if (h > 0 && m > 0) return `${h} 小时 ${m} 分`;
  return h > 0 ? `${h} 小时` : `${n} 分钟`;
}
/** 机器设置的保存：每一项各存各的（store 的 saveSetting）。改动当帧生效，不禁用任何控件，没确认时也能接着改；
    sync 是每一项的保存状态，画在那个控件旁边：保存中（转圈，晚一点才出现）→ 已保存（打勾后淡出）；
    被拒绝时值回到原处、那一行回弹，并提示 */
function useSaver(name: string) {
  const { sync, track } = useSync();
  const save: Save = p => {
    const f = fieldOf(p);
    const { env, run, ...fields } = p;
    const done = saveSetting(
      `${hostKey(name)}:${f}`,
      {
        ws: ws => ({
          ...ws,
          hosts: ws.hosts?.map(x => (x.name === name ? { ...x, ...fields, agents: env || run ? x.agents.map(a => agentPatch(a, p)) : x.agents } : x)),
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
  const server = useServerVersion();
  const behind = !!server && !!h.version && versionLt(h.version, server);
  // 就绪清单里没通过的项点按钮：展开对应的那一块（收起的「白板与目录」先展开），滚过去闪一下
  const [reveal, setReveal] = useState<{ to: ReadyGo | null; n: number }>({ to: null, n: 0 });
  /** 「复制设置到…」展开着 */
  const [copying, setCopying] = useState(false);
  const go = (to: ReadyGo) => setReveal(r => ({ to, n: r.n + 1 }));
  useEffect(() => {
    if (!reveal.to) return;
    const raf = requestAnimationFrame(() => {
      const el = document.getElementById(`hp-sec-${reveal.to}`);
      if (!el) return;
      el.scrollIntoView?.({ behavior: reduceMotion() ? 'auto' : 'smooth', block: 'start' });
      el.classList.remove('hp-flash');
      void el.offsetWidth;
      el.classList.add('hp-flash');
    });
    return () => cancelAnimationFrame(raf);
  }, [reveal]);
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
    const t = h.token;
    const { ok: yes, checked: revoke } = await confirmCheck({
      title: `移除 ${h.name}？`,
      text: '取消它所有排队中的派发，停止正在跑的派发，删除机器设置；派发记录保留。那台机器的 board host 再连上来时按新机器处理。',
      ok: '移除',
      check: {
        label: t ? `同时吊销它用的令牌「${t.name}」（${t.hint}）` : '同时吊销它用的令牌',
        why: !t ? '看不到这台机器用的令牌（还没上报过，或已经吊销）' : t.shared?.length ? `机器 ${t.shared.join('、')} 也在用这个令牌，不能一起吊销` : undefined,
      },
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
        () => api.removeHost(h.name, revoke),
      );
      toast({ type: 'success', title: `已移除 ${h.name}`, text: revoke && t ? `令牌「${t.name}」已吊销` : undefined });
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
                <span className={cx(behind && 'hp-behind')}>
                  board {h.version || '?'}
                  {behind && ` · 落后于服务端 ${server}`}
                </span>
                <span>{lastSeen(h, now)}</span>
              </div>
              {behind && (
                <p className="hp-behind-tip">
                  升级：在下面「维护」里点「升级到 {server}」，或在那台机器上执行 <code>board update</code>；桌面端运行的在桌面端「检查更新」
                </p>
              )}
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

          <ReadySection checks={readiness(h, now, server)} go={go} i={1} />
          <ManageSection h={h} server={server} i={1} />
          <SelfTestSection h={h} server={server} i={1} />

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
            <div className="d-label">
              接单
              {hostsOf(ws).length > 1 && (
                <button type="button" className="act hp-link" aria-expanded={copying} onClick={() => setCopying(v => !v)}>
                  {copying ? '收起' : '复制设置到…'}
                </button>
              )}
            </div>
            {copying && <CopySettings h={h} hosts={hostsOf(ws).filter(x => x.name !== h.name)} close={() => setCopying(false)} />}
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
                    <span data-tip="定时领取开会话时一次排好这个会话要做的卡片（先挑带优先标签的，再挑和第一张标签相近的），agent 做完一张用 board next 接着做排着的下一张，做完就退出；设为 1 时有几个空位就同时开几个会话。改了只影响之后新开的会话；手动派发只做指定的那一张">
                      定时领取先把一个会话排满再开下一个
                    </span>
                  </div>
                  <span className="hp-ctl">
                    <Sync s={sync.perSession} />
                    <Stepper value={h.perSession} label="一个会话最多做几张" onChange={v => save({ perSession: v })} />
                  </span>
                </div>
              )}
              {h.maxRun != null && (
                <div className={cx('hp-knob', failed('maxRun'))}>
                  <div className="hp-knob-text">
                    <b>单次最长运行</b>
                    <span data-tip="到时间后 board host 按「停止」的流程结束整个进程树，派发记「超时」，卡片按异常退出的规则移到错误节点（没有时移到需要人处理的状态），评论写明原因和日志最后 20 行">
                      到时间就停止，防止卡住的 agent 一直占着并发位
                    </span>
                  </div>
                  <span className="hp-ctl">
                    <Sync s={sync.maxRun} />
                    <Select
                      className="hp-env-select"
                      value={String(h.maxRun)}
                      options={MAX_RUN_CHOICES.map(n => ({ value: String(n), label: minsWords(n) }))}
                      onChange={v => save({ maxRun: +v })}
                      aria-label="单次最长运行"
                    />
                  </span>
                </div>
              )}
              {h.idleRun != null && (
                <div className={cx('hp-knob', failed('idleRun'))}>
                  <div className="hp-knob-text">
                    <b>没输出多久算卡住</b>
                    <span data-tip="只对已经输出过内容的派发生效。内置模板都用结构化输出，运行中每一步都有输出；配置文件里改成只在结束时打印的 agent 一行都还没输出时，这条不适用">
                      长时间没有输出就停止，不等它自己退出
                    </span>
                  </div>
                  <span className="hp-ctl">
                    <Sync s={sync.idleRun} />
                    <Select
                      className="hp-env-select"
                      value={String(h.idleRun)}
                      options={IDLE_RUN_CHOICES.map(n => ({ value: String(n), label: minsWords(n) }))}
                      onChange={v => save({ idleRun: +v })}
                      aria-label="没输出多久算卡住"
                    />
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

          <section className="d-section" id="hp-sec-agents" style={{ ['--i' as string]: 3 }}>
            <div className="d-label">
              本机 agent
              {h.recheckedAt && <span className="act">{new Date(h.recheckedAt).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })} 重新检查过</span>}
            </div>
            <SkillBanner h={h} fixed={fixed} setFixed={setFixed} />
            {h.agentSet && (
              <p className="hp-note hp-warn-line">
                <Icon name="alert" />
                设置的默认 agent「{h.agentSet}」这台机器上找不到，不生效{h.agent ? `：用的是 ${h.agents.find(a => a.id === h.agent)?.name ?? h.agent}` : ''}
              </p>
            )}
            {h.agents.map(a => (
              <AgentRow
                key={a.id}
                h={h}
                a={a}
                save={save}
                sync={sync[`env:${a.id}`] ?? (a.id === h.agent ? sync.agent : undefined)}
                runSync={{ model: sync[`run:${a.id}:model`], effort: sync[`run:${a.id}:effort`] }}
                server={server}
                fixed={fixed?.includes(a.id)}
              />
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

          <Fold id="dirs" title="白板与目录" n={h.dirs.length} i={5} reveal={reveal.to === 'dirs' ? reveal.n : 0}>
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
              最近 <span className="count">{recent ? recent.length + (h.ops?.length ?? 0) : ''}</span>
            </button>
            <span className="act">
              今天交审 {today.review} · 异常退出 {today.crashed}
            </span>
          </div>
          <div className="d-activity" hidden={!recentOpen}>
            {recent === null ? (
              <p className="hp-note hp-pad">正在加载…</p>
            ) : recent.length === 0 && !h.ops?.length ? (
              <p className="hp-note hp-pad">还没有结束的派发</p>
            ) : (
              // 派发（同一个会话的合成一行）和操作记录（谁、什么时间、做了什么）按时间混在一起，新的在前
              [...sessionGroups(recent).map(g => ({ t: g.t, g, o: null })), ...(h.ops ?? []).map(o => ({ t: o.at, g: null, o }))]
                .sort((a, b) => b.t - a.t)
                .map(({ g, o }) => {
                if (o)
                  return (
                    <div className="hp-hist hp-op" key={`op${o.at}${o.text}`}>
                      <Icon name="host" />
                      <div>
                        <b>{o.text}</b>
                        <span>
                          {o.by} · {since(o.at, now)}
                        </span>
                      </div>
                    </div>
                  );
                if (!g) return null;
                const p = g.list[0];
                if (g.list.length > 1) {
                  // 同一个会话：一行，写做了几张、各自的结果
                  const first = g.list[g.list.length - 1];
                  return (
                    <button type="button" className="hp-hist" key={`s${g.session}`} onClick={() => openSession({ id: g.session })} title="打开这个会话">
                      <Mark id={p.agent} title={p.agentLabel} />
                      <div>
                        <b>
                          会话 · {g.list.length} 张 <span className="hp-cid">{[...g.list].reverse().map(x => `#${x.card}`).join(' ')}</span>
                        </b>
                        <span>
                          {[...g.list].reverse().map((x, k) => {
                            const r = resultOf(x);
                            return (
                              <em key={x.id} style={{ ['--c' as string]: r.color }}>
                                {k > 0 && ' · '}
                                {r.label}
                              </em>
                            );
                          })}
                          {first.startedAt != null && p.endedAt != null && <> · 用时 {durWords(p.endedAt - first.startedAt)}</>}
                          {p.endedAt != null && <> · {since(p.endedAt, now)}</>}
                        </span>
                      </div>
                    </button>
                  );
                }
                const r = resultOf(p);
                return (
                  <button type="button" className="hp-hist" key={p.id} onClick={() => openSession({ id: p.session || p.id, card: p.card })} title="打开这个会话">
                    <Mark id={p.agent} title={p.agentLabel} />
                    <div>
                      <b>
                        <span className="hp-cid">#{p.card}</span> <Title text={p.title} />
                      </b>
                      <span>
                        <em style={{ ['--c' as string]: r.color }}>{r.label}</em>
                        {p.envName && <> · {p.envName}</>}
                        {p.model && <> · {p.model}</>}
                        {p.effort && <> · {p.effort}</>}
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

/** 一块白板的环境检查结果（白板写了检查命令时才有这一行） */
function DirReadyLine({ h, d, now }: { h: HostInfo; d: HostDir; now: number }) {
  const rc = useData(s => boardOf(s.ws, d.board)?.readyCheck);
  const r = dirReadiness(rc, h, d);
  if (r.key === 'none') return null;
  return (
    <div className={cx('hp-dir-ready', r.key)} title={d.ready?.tail.join('\n') || undefined}>
      <span className="hp-dir-ready-k">环境检查</span>
      <span className="hp-dir-ready-v">
        {r.key === 'ok' ? '通过' : r.key === 'failed' ? `没通过：${r.last}` : '还没检查（等 board host 跑完；旧版不会检查）'}
      </span>
      {'at' in r && <span className="hp-dir-ready-at">{since(r.at, now)}</span>}
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
  // 仓库地址的建议：白板设置里设了就用它（PRD §4），没设时看别的机器上报的这块白板目录的 origin
  const originOf = (board: string) =>
    boardOf(ws, board)?.repo?.trim() || hosts.find(x => x.name !== h.name)?.dirs.find(d => d.board === board && d.origin)?.origin || '';
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
              <DirReadyLine h={h} d={d} now={now} />
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
function AgentRow({
  h,
  a,
  save,
  sync,
  runSync,
  server,
  fixed,
}: {
  h: HostInfo;
  a: HostAgent;
  save: Save;
  sync?: SyncState;
  runSync: { model?: SyncState; effort?: SyncState };
  server: string | null;
  fixed?: boolean;
}) {
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
        {a.local_override && (
          <span className="hp-skill local" tabIndex={0} data-tip={`共用配置里有 ${a.id} 的 agent 模板，但这台机器的 ~/.board/config.toml 里有 [agents.${a.id}]：用的是本机的那份`}>
            本机配置覆盖
          </span>
        )}
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
        <SelfTestAgentButton h={h} a={a} />
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
      {a.run_env && a.env && a.run_env !== a.env && (
        <p className="hp-note hp-warn-line hp-arow-note">
          <Icon name="alert" />
          设置的启动环境「{name(a.run_env)}」这台机器上没有，不生效：用的是 {name(a.env)}
        </p>
      )}
      <AgentRunRow h={h} a={a} save={save} sync={runSync} server={server} />
    </>
  );
}

/** 复制设置到其他机器（PRD §10.12）：选目标机器和要复制的项，目标机器的这几项换成这台的，没选的不变。
 *  照原样复制：目标机器上找不到的 agent、没有的启动环境照样带过去，在那台机器的详情里标「不生效」。要等服务端：按钮转圈 */
function CopySettings({ h, hosts, close }: { h: HostInfo; hosts: HostInfo[]; close: () => void }) {
  const [to, setTo] = useState<string[]>([]);
  const [items, setItems] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const flip = (list: string[], v: string, on: boolean) => (on ? [...list, v] : list.filter(x => x !== v));
  const go = async () => {
    setBusy(true);
    try {
      await optimistic(null, () => api.copyHost(h.name, to, items));
      toast({ type: 'success', title: `已复制到 ${to.join('、')}`, text: COPY_ITEMS.filter(([k]) => items.includes(k)).map(([, w]) => w).join('、'), duration: 2400 });
      close();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="hp-copy">
      <div className="hp-copy-row">
        <span className="hp-add-k">复制到</span>
        <div className="hp-copy-opts">
          {hosts.map(x => (
            <label key={x.name} className="hp-copy-opt">
              <input type="checkbox" checked={to.includes(x.name)} onChange={e => setTo(flip(to, x.name, e.currentTarget.checked))} />
              {x.name}
              {!x.online && <em>离线</em>}
            </label>
          ))}
        </div>
      </div>
      <div className="hp-copy-row">
        <span className="hp-add-k">复制哪些</span>
        <div className="hp-copy-opts col">
          {COPY_ITEMS.map(([k, w]) => (
            <label key={k} className="hp-copy-opt">
              <input type="checkbox" checked={items.includes(k)} onChange={e => setItems(flip(items, k, e.currentTarget.checked))} />
              {w}
            </label>
          ))}
        </div>
      </div>
      <p className="hp-note">目标机器的这几项换成 {h.name} 的，没选的不变。那台机器上找不到的 agent、没有的启动环境照样带过去，在它的详情里标「不生效」。</p>
      <div className="lh-actions">
        <button className="btn sm ghost" type="button" disabled={busy} onClick={close}>
          取消
        </button>
        <button className={cx('btn sm brand', busy && 'is-busy')} type="button" disabled={busy || !to.length || !items.length} onClick={() => void go()}>
          复制
        </button>
      </div>
    </div>
  );
}

/** 一个 agent 的模型、思考强度（PRD §9.1）：模型是可以手填的下拉，强度只列它支持的档位；空是 agent 自己的默认（不传参数）。
 *  各存各的、当帧生效，保存状态画在控件旁边。旧版 board host 不上报能选什么：置灰，写要升级到哪个版本；
 *  模板没有这两项的 agent（dsh）不显示 */
function AgentRunRow({ h, a, save, sync, server }: { h: HostInfo; a: HostAgent; save: Save; sync: { model?: SyncState; effort?: SyncState }; server: string | null }) {
  const opts = a.opts;
  const [model, setModel] = useState(a.run_model ?? '');
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) setModel(a.run_model ?? '');
  }, [a.run_model, editing]);
  if (!a.found) return null;
  if (opts && !opts.model && !opts.efforts.length) return null;
  const old = !opts;
  const tip = old ? `这台机器的 board 要升级到 ${server ?? '服务端的版本'} 才能设置` : undefined;
  const commit = () => {
    setEditing(false);
    const v = model.trim();
    if (v !== (a.run_model ?? '')) save({ run: { agent: a.id, model: v } });
  };
  const listId = `hp-models-${h.id}-${a.id}`;
  return (
    <div className={cx('hp-arun', old && 'off')} title={tip}>
      {(old || opts?.model) && (
        <label className="hp-arun-f">
          <span>模型</span>
          <input
            className="input hp-arun-model"
            list={listId}
            value={model}
            disabled={old}
            placeholder={a.model ? `默认（${a.model}）` : '默认'}
            spellCheck={false}
            autoComplete="off"
            aria-label={`${a.name} 的模型`}
            onFocus={() => setEditing(true)}
            onChange={e => setModel(e.target.value)}
            onBlur={commit}
            onKeyDown={e => {
              if (e.key === 'Enter') e.currentTarget.blur();
            }}
          />
          <datalist id={listId}>
            {(opts?.models ?? []).map(m => (
              <option key={m} value={m} />
            ))}
          </datalist>
          <Sync s={sync.model} />
        </label>
      )}
      {(old || !!opts?.efforts.length) && (
        <span className="hp-arun-f">
          <span>强度</span>
          <Select
            className="hp-arun-effort"
            disabled={old}
            value={a.run_effort ?? ''}
            options={[{ value: '', label: '默认' }, ...(opts?.efforts ?? []).map(e => ({ value: e, label: e }))]}
            onChange={v => save({ run: { agent: a.id, effort: v } })}
            aria-label={`${a.name} 的思考强度`}
          />
          <Sync s={sync.effort} />
        </span>
      )}
      {old && <em className="hp-arun-old">{tip}</em>}
    </div>
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
  // 新版 board host 按「共用配置」同步 board-skills：同步没成的写出来
  const syncing = h.skills != null;
  const syncBad = (h.skills ?? []).filter(s => s.skill === 'board-skills' && s.state !== 'synced');
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
  if (!gaps.missing.length && !syncBad.length) return null;
  const can = shell === 'desktop' && !noSupport && isLocalHost(h, local) && gaps.auto.length > 0;
  const ids = gaps.auto.map(a => a.id);
  const why = syncBad.map(s => `${s.dir}：${s.why ?? STATE_WORDS[s.state]}`).join('；');

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
        {gaps.missing.length ? `${gaps.missing.length} 个 agent 看不到 board-skills，派给它们的卡片会做不成` : 'board-skills 没有同步好'}
        <small>
          {syncing ? (
            <>board host 按「机器 → 共用配置」把 board-skills（随服务端版本）同步到各个 agent 的目录{why ? `：${why}` : '，正在同步…'}</>
          ) : can ? (
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
          {installing ? (syncing ? '正在同步…' : '正在安装…') : syncing ? '立即同步' : '一键安装'}
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

/** 次要区块：标题行点一下收起 / 展开，状态记在本机（默认收起），重开弹窗保持；reveal 变了（就绪清单里点了按钮）就展开 */
function Fold({ id, title, n, i, reveal = 0, children }: { id: string; title: string; n: number; i: number; reveal?: number; children: React.ReactNode }) {
  const [open, setOpen] = useState(() => pref.get(`hostFold.${id}`, false));
  useEffect(() => {
    if (!reveal) return;
    pref.set(`hostFold.${id}`, true);
    setOpen(true);
  }, [reveal, id]);
  return (
    <section id={`hp-sec-${id}`} className={cx('d-section hp-fold', !open && 'folded')} style={{ ['--i' as string]: i }}>
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
  // 这个会话开会话时排好、还没开始的卡片：按顺序挂在这一行下面，每张可以单独取消
  const ws = useData(s => s.ws);
  const rest = sessionQueue(ws, r.session);
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
        {(r.model || r.effort) && <span>{[r.model, r.effort].filter(Boolean).join(' · ')}</span>}
        <span>{r.startedAt != null ? durWords(now - r.startedAt) : '刚开始'}</span>
        <Beat at={r.beatAt} now={now} />
        <span>{SOURCE[r.source]}</span>
        {r.stopBy && <span className="hp-stopping">正在停止</span>}
        <span className="hp-sp" />
        <button className="hp-link" type="button" onClick={() => openSession({ id: r.session || r.id, card: r.card })}>
          <Icon name="log" />
          会话
        </button>
        {!r.stopBy && (
          <button className="hp-link danger" type="button" onClick={() => cancel(r)}>
            <Icon name="stop" />
            停止
          </button>
        )}
      </div>
      {rest.length > 0 && (
        <ol className="hp-run-rest" aria-label="这个会话排着的卡片">
          {rest.map(q => (
            <li key={q.id}>
              <button type="button" className="hp-cid link" onClick={() => goCard(q)}>
                #{q.card}
              </button>
              <span className="hp-run-title">
                <Title text={q.title} />
              </span>
              <span className="hp-run-seq">
                第 {q.seq ?? '?'}/{q.total ?? '?'} 张
              </span>
              <button className="hp-link danger" type="button" onClick={() => cancel(q)}>
                <Icon name="close" />
                取消派发
              </button>
            </li>
          ))}
        </ol>
      )}
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
          <CliInstall />
        </div>
      </div>
    </div>
  );
}

/**
 * 只有命令行的机器：填机器名、选白板和路径，「生成命令」时以机器名新建一个访问令牌，给出一行命令——
 * 在新机器上执行就依次安装 board、board login、board host --install（装成服务，开机自动接任务）。
 * 令牌原文只在这里显示这一次；生成之后再改机器名、目录，命令跟着变，令牌不变
 */
function CliInstall() {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const boards = allBoards(ws).map(b => b.path);
  // 就是这个页面的服务端地址（网页端、桌面端都一样，不写死地址）
  const origin = typeof location !== 'undefined' ? location.origin : '<服务端地址>';
  const [os, setOs] = useState<'unix' | 'windows'>(isWindows ? 'windows' : 'unix');
  const [name, setName] = useState('');
  const [dirs, setDirs] = useState<{ board: string; path: string }[]>(() => [{ board: path || boards[0] || '', path: '' }]);
  const [token, setToken] = useState<{ raw: string; name: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const nameErr = hostNameError(name);
  const picked = dirs.filter(d => d.board);
  const dirsErr = !picked.some(d => d.path.trim())
    ? '至少给一块白板填上目录'
    : new Set(picked.map(d => d.board)).size !== picked.length
      ? '同一块白板写了两次'
      : picked.some(d => !d.path.trim())
        ? '选了白板的行都要填目录'
        : null;
  const err = nameErr ?? dirsErr;
  const cmd = token && !err ? installCommand({ origin, token: token.raw, name, dirs, os }) : null;
  const edit = (f: (v: typeof dirs) => typeof dirs) => setDirs(f);
  const copy = async (t: string) => {
    if (await copyText(t)) toast({ type: 'success', title: '已复制', text: '在新机器的终端里粘贴执行', duration: 1800 });
  };
  const generate = async () => {
    if (err || busy) return;
    setBusy(true);
    try {
      const r = await optimistic(null, () => api.createToken(name));
      setToken({ raw: r.token, name: r.info.name });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="d-section">
      <div className="d-label">只有命令行的机器（服务器、devbox）</div>
      <p className="hp-note">填好机器名和要接的白板、目录，生成一行命令，在那台机器上执行：安装 board、登录、装成服务（开机自动接任务）。</p>
      <div className="hp-add">
        <div className="hp-add-row">
          <span className="hp-add-k">系统</span>
          <Segmented
            value={os}
            options={[
              ['unix', 'macOS / Linux'],
              ['windows', 'Windows'],
            ]}
            onChange={setOs}
          />
        </div>
        <label className="hp-add-row">
          <span className="hp-add-k">机器名</span>
          <input
            className="input mono"
            value={name}
            placeholder="devbox"
            spellCheck={false}
            autoComplete="off"
            onChange={e => setName(e.target.value.toLowerCase().trim())}
          />
        </label>
        <div className="hp-add-row top">
          <span className="hp-add-k">白板与目录</span>
          <div className="lh-dirs hp-add-dirs">
            {dirs.map((d, i) => (
              <div className="lh-dir" key={i}>
                <Select
                  value={d.board}
                  options={[{ value: '', label: '选白板…' }, ...[...new Set([...boards, ...(d.board ? [d.board] : [])])].map(b => ({ value: b, label: b }))]}
                  onChange={val => edit(v => v.map((x, j) => (j === i ? { ...x, board: val } : x)))}
                  aria-label="白板"
                />
                <Icon name="move" />
                <input
                  className="input"
                  value={d.path}
                  placeholder={os === 'windows' ? 'C:\\repo\\board' : '~/repo/board'}
                  spellCheck={false}
                  onChange={e => edit(v => v.map((x, j) => (j === i ? { ...x, path: e.target.value } : x)))}
                  aria-label="目录"
                />
                {dirs.length > 1 && (
                  <button className="icon-btn sm" type="button" aria-label="去掉这一行" onClick={() => edit(v => v.filter((_, j) => j !== i))}>
                    <Icon name="close" />
                  </button>
                )}
              </div>
            ))}
            <div className="lh-actions">
              <button className="btn sm" type="button" onClick={() => edit(v => [...v, { board: boards.find(b => !v.some(x => x.board === b)) ?? '', path: '' }])}>
                <Icon name="plus" />
                加一块白板
              </button>
            </div>
          </div>
        </div>
      </div>
      {!token ? (
        <div className="hp-add-go">
          <button className={cx('btn sm brand', busy && 'is-busy')} type="button" disabled={!!err || busy} onClick={() => void generate()}>
            <Icon name="key" />
            生成命令
          </button>
          <span className="hp-note">{err ?? `会新建一个名为「${name}」的访问令牌，命令里带着它`}</span>
        </div>
      ) : (
        <>
          {cmd ? (
            <button type="button" className="hp-cmd" onClick={() => copy(cmd)} data-tip="复制">
              <code>{cmd}</code>
              <Icon name="copy" />
            </button>
          ) : (
            <p className="hp-note lh-warn">{err}</p>
          )}
          <p className="hp-note">
            令牌「{token.name}」的原文只在这里显示这一次，关掉弹窗后看不到了；不再用这台机器时在「设置 → Agent 接入」吊销它。
            {os === 'windows' ? '在 PowerShell 里执行。' : '在终端里执行（装到 ~/.local/bin）。'}执行完几分钟内，这台机器会出现在列表里并在线。
          </p>
          <p className="hp-note">
            最后一步 <code>board host --install</code> 把它装成开机自启的服务（{os === 'windows' ? '登录时运行的计划任务' : 'macOS 是 launchd，Linux 是 systemd 用户服务'}）：重启后自动上线，board host 异常退出 10 秒后自动重启，日志在 <code>~/.board/host/host.log</code>，卸载用 <code>board host --uninstall</code>。
            {os === 'unix' && ' Linux 上要在没人登录时也运行，再执行一次 loginctl enable-linger。'}
          </p>
        </>
      )}
    </section>
  );
}
