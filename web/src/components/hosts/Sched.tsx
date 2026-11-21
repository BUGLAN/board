/* ==========================================================================
   自动领取（PRD §9.1「自动领取」）：哪些卡自动交给哪个 agent 定一次，所有机器共用
   - 领取规则、自动领取的白板在 Agent tab「分工」里（components/profiles/Division.tsx）
   - HostAccept（机器 Tab「接单」，PRD §10.12）：一台机器一条 24 小时的时间带，直接在上面拖出接单时段（空白处拖是新加一段，拖两头改起止，
     拖中间平移，双击删掉，半小时一格；全天就是从 00 拖到 24）；右边是会话上限、一个会话几张、接单（自动领 / 只做派给它的 / 暂停）。
     时间按服务器时间。最下面是所有机器的「自动升级 board」
   - RunLimits（Agent tab「跑多久停」）：单次最长运行、没输出多久算卡住，所有机器一个值
   写操作都是设置项（saveSetting）：当帧生效，保存状态画在那一项旁边，失败回到原处并提示
   ========================================================================== */
import { useRef, useState } from 'react';
import { api } from '../../lib/api';
import { ACCEPT_WORDS, acceptOf, acceptPatch, hostLoad, minsWords, type Accept } from '../../lib/hosts';
import { ALL_DAY, DAY, SNAP, hhmm, hostTake, serverMinute, tidyWins, winWords, type Win } from '../../lib/sched';
import type { HostInfo, Workspace } from '../../lib/types';
import { cx } from '../../lib/util';
import { saveSetting, useData } from '../../store/data';
import { openHost } from '../../store/ui';
import { Select, Sync, Tip, useNow, useSync } from '../bits';
import { Stepper, useSaver } from './Saver';

/** 接单（机器 Tab）：每台机器的接单时段、会话数、一个会话几张、接单三态，和所有机器的自动升级 */
export function HostAccept({ hosts }: { hosts: HostInfo[] }) {
  const ws = useData(s => s.ws);
  const now = useNow(30_000);
  const { sync, track } = useSync();
  const sched = ws?.sched;
  // 旧版服务端没有这一项：不显示这一节
  if (!ws || !sched) return null;
  const minute = serverMinute(ws, now);
  const setAutoUpgrade = (on: boolean) =>
    track(
      'upgrade',
      saveSetting('sched:upgrade', { ws: w => (w.sched ? { ...w, sched: { ...w.sched, autoUpgrade: on } } : w) }, () => api.updateSched({ autoUpgrade: on })),
    );
  return (
    <section className="sc sd">
      <h3 className="sd-h">
        机器什么时候接单 <small>服务器时间 {hhmm(minute)} · 在时间带上拖出时段：空白处拖是新加一段，拖两头改起止，拖中间平移，双击删掉</small>
      </h3>
      {hosts.length === 0 ? (
        <p className="hp-empty sd-empty">还没有机器。</p>
      ) : (
        <div className="sd-time">
          <div className="sd-axis">
            <span />
            <div className="sd-ticks">
              {[0, 4, 8, 12, 16, 20, 24].map(h => (
                <i key={h} style={{ left: `${(h / 24) * 100}%` }}>
                  {String(h).padStart(2, '0')}
                </i>
              ))}
            </div>
            <span />
          </div>
          {hosts.map(h => (
            <TimeRow key={h.id} h={h} ws={ws} minute={minute} />
          ))}
        </div>
      )}

      <h3 className="sd-h">升级</h3>
      <div className={cx('sd-up', sync.upgrade === 'failed' && 'hp-rollback')}>
        <div>
          <b>自动升级 board</b>
          <span>
            服务端升级之后，每台机器等手上的派发做完，自己升级到同一个版本再接着接单，不用逐台点「升级」。升级没成的恢复接单、在机器的操作记录里写明；发版包还没出来这类临时失败每隔 10 分钟再试，同一个版本最多 5 次。
          </span>
        </div>
        <span className="hp-ctl">
          <Sync s={sync.upgrade} />
          <button
            className={cx('switch', sched.autoUpgrade && 'on')}
            type="button"
            role="switch"
            aria-checked={!!sched.autoUpgrade}
            aria-label="自动升级 board"
            onClick={() => setAutoUpgrade(!sched.autoUpgrade)}
          >
            <i />
          </button>
        </span>
      </div>
    </section>
  );
}

/** 一台机器一行：时间带、时段文字、会话上限、一个会话几张、接单（自动领 / 只做派给它的 / 暂停） */
function TimeRow({ h, ws, minute }: { h: HostInfo; ws: Workspace; minute: number }) {
  const { save, sync } = useSaver(h.name);
  const wins = h.windows ?? ALL_DAY;
  const t = hostTake(h, hostLoad(ws, h).running.length, minute);
  const words = winWords(wins);
  const failed = ['windows', 'auto', 'paused', 'max', 'perSession'].some(k => sync[k] === 'failed');
  const accept = acceptOf(h);
  return (
    <div className={cx('sd-line', (!h.online || accept !== 'auto') && 'off', failed && 'hp-rollback')}>
      <button type="button" className="sd-m-name" onClick={() => openHost(h.name)} title="打开机器详情">
        <i className={cx('sd-dot', t.tone)} />
        <b>{h.name}</b>
      </button>
      <Track wins={wins} minute={minute} off={!h.online || accept !== 'auto'} onChange={v => save({ windows: v })} />
      <span className="sd-ctl">
        <span className="sd-win-words" title={words}>
          {words}
        </span>
        <Sync s={sync.windows ?? sync.max ?? sync.perSession ?? sync.paused} inline />
        <Tip text="最多同时开几个会话">
          <span>
            <Stepper value={h.max} label="最多几个会话" onChange={v => save({ max: v })} />
          </span>
        </Tip>
        <Tip text="一个会话最多做几张">
          <span>
            <Stepper value={h.perSession ?? 4} label="一个会话最多做几张" onChange={v => save({ perSession: v })} />
          </span>
        </Tip>
        {/* 「暂停接单」「自动接单」「排空」合成三态：暂停后在跑的做完就是已排空 */}
        <Select
          className="sd-accept"
          value={accept}
          options={ACCEPT_WORDS.map(([value, label]) => ({ value, label }))}
          onChange={v => v !== accept && save(acceptPatch(v as Accept))}
          aria-label={`${h.name} 接单`}
        />
      </span>
      <small className={cx('sd-now', t.tone)}>{t.why}</small>
    </div>
  );
}

type Mode = 'new' | 'move' | 'l' | 'r';
const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));

/** 时间带：在上面直接拖出接单时段。拖动时先画在本地，松手再存（挨着、重叠的并成一段） */
function Track({ wins, minute, off, onChange }: { wins: Win[]; minute: number; off?: boolean; onChange: (v: Win[]) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [live, setLive] = useState<{ wins: Win[]; at: number } | null>(null);
  const shown = live?.wins ?? wins;
  const minuteAt = (x: number) => {
    const r = ref.current!.getBoundingClientRect();
    return clamp(Math.round((((x - r.left) / r.width) * DAY) / SNAP) * SNAP, 0, DAY);
  };
  const start = (e: React.PointerEvent, mode: Mode, i = -1) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const m0 = minuteAt(e.clientX);
    const base = wins.map(w => [...w] as Win);
    const apply = (m: number): { wins: Win[]; at: number } => {
      const next = base.map(w => [...w] as Win);
      if (mode === 'new') {
        next.push([Math.min(m0, m), Math.max(m0, m)]);
        return { wins: next, at: next.length - 1 };
      }
      if (mode === 'l') next[i][0] = Math.min(m, next[i][1]);
      if (mode === 'r') next[i][1] = Math.max(m, next[i][0]);
      if (mode === 'move') {
        const w = base[i][1] - base[i][0];
        const a = clamp(base[i][0] + (m - m0), 0, DAY - w);
        next[i] = [a, a + w];
      }
      return { wins: next, at: i };
    };
    setLive(apply(m0));
    const move = (ev: PointerEvent) => setLive(apply(minuteAt(ev.clientX)));
    const up = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', move);
      setLive(null);
      const next = tidyWins(apply(minuteAt(ev.clientX)).wins);
      if (JSON.stringify(next) !== JSON.stringify(tidyWins(wins))) onChange(next);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up, { once: true });
  };
  return (
    <div ref={ref} className={cx('sd-track', off && 'off', live && 'dragging')} onPointerDown={e => start(e, 'new')} title="在空白处拖出一段接单时段">
      {shown.map(([a, b], i) => {
        const active = live?.at === i;
        return (
          <div
            key={i}
            className={cx('sd-span', active && 'active')}
            style={{ left: `${(a / DAY) * 100}%`, width: `${((b - a) / DAY) * 100}%` }}
            onPointerDown={e => start(e, 'move', i)}
            onDoubleClick={() => onChange(wins.filter((_, k) => k !== i))}
            title={`${hhmm(a)}–${hhmm(b)}：拖中间平移，拖两头改起止，双击删掉`}
          >
            <i className="sd-grip l" onPointerDown={e => start(e, 'l', i)} />
            {b - a >= 180 && (
              <span>
                {hhmm(a)}–{hhmm(b)}
              </span>
            )}
            <i className="sd-grip r" onPointerDown={e => start(e, 'r', i)} />
            {active && (
              <b className="sd-tip">
                {hhmm(a)}–{hhmm(b)}
              </b>
            )}
          </div>
        );
      })}
      <i className="sd-nowline" style={{ left: `${(minute / DAY) * 100}%` }} />
    </div>
  );
}

/** 单次最长运行可以选的档位（分钟，0 = 不限）。服务端只校验范围（`board-core` 的 `MAX_RUN_LIMIT`，
    就是最高一档），选哪几档由界面决定，所以档位只在这里列一份 */
const MAX_RUN_CHOICES = [0, 30, 60, 90, 120, 180, 240, 300, 360, 480, 600, 660, 720];
/** 没输出多久算卡住可以选的档位（分钟，0 = 不限）；范围是 `IDLE_RUN_LIMIT`，最高一档就是它 */
const IDLE_RUN_CHOICES = [0, 5, 10, 15, 20, 30, 45, 60, 75, 90, 105, 120];

/** 「跑多久停」：单次最长运行、没输出多久算卡住，所有机器一个值（服务端存一份，随机器发给 board host） */
export function RunLimits() {
  const sched = useData(s => s.ws?.sched);
  const { sync, track } = useSync();
  // 旧版服务端没有这两项：不显示这一节
  if (!sched || sched.maxRun == null || sched.idleRun == null) return null;
  const set = (k: 'maxRun' | 'idleRun', v: number) =>
    track(
      k,
      saveSetting(
        `sched:${k}`,
        { ws: w => ({ ...w, sched: w.sched ? { ...w.sched, [k]: v } : w.sched, hosts: w.hosts?.map(h => ({ ...h, [k]: v })) }) },
        () => api.updateSched({ [k]: v }),
      ),
    );
  return (
    <section className="sc sd">
      <div className="sc-head">
        <div>
          <h2>跑多久停</h2>
          <p>所有机器一样。到时间按「停止」结束整个进程，派发记「超时」或「卡住」，卡片按异常退出的规则移走，评论里写原因和日志最后 20 行。</p>
        </div>
      </div>
      <div className="hd-limits">
        <div className={cx('hp-knob', sync.maxRun === 'failed' && 'hp-rollback')}>
          <div className="hp-knob-text">
            <b>单次最长运行</b>
            <span>到时间就停，防止卡住的 agent 一直占着会话位</span>
          </div>
          <span className="hp-ctl">
            <Sync s={sync.maxRun} />
            <Select
              className="hp-env-select"
              value={String(sched.maxRun)}
              options={MAX_RUN_CHOICES.map(n => ({ value: String(n), label: minsWords(n) }))}
              onChange={v => set('maxRun', +v)}
              aria-label="单次最长运行"
            />
          </span>
        </div>
        <div className={cx('hp-knob', sync.idleRun === 'failed' && 'hp-rollback')}>
          <div className="hp-knob-text">
            <b>没输出多久算卡住</b>
            <Tip text="只对已经输出过内容的派发生效。内置的 agent 都用结构化输出，运行中每一步都有输出">
              <span>已经输出过内容、之后长时间没有新输出就停</span>
            </Tip>
          </div>
          <span className="hp-ctl">
            <Sync s={sync.idleRun} />
            <Select
              className="hp-env-select"
              value={String(sched.idleRun)}
              options={IDLE_RUN_CHOICES.map(n => ({ value: String(n), label: minsWords(n) }))}
              onChange={v => set('idleRun', +v)}
              aria-label="没输出多久算卡住"
            />
          </span>
        </div>
      </div>
    </section>
  );
}
