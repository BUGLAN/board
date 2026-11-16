/* ==========================================================================
   自动领取（PRD §9.1「自动领取」）：哪些卡自动交给哪个 agent 定一次，所有机器共用
   - ClaimRules（Agent tab「领取规则」，PRD §10.16）：
     - 自动领取的白板：一块白板一个胶囊，点一下参与 / 不参与（不参与的只是不被自动领取，手动派发照常）
     - 领取规则：按 agent 分卡——这个 agent 领哪些卡（规则前面的数字是全局的先后，第一条命中的生效）、由哪些机器做
       （每台一个胶囊：现在能接、要等、接不了和原因）；下面一行是兜底「其余卡片 →」
     - 交给哪个 agent：机器上找到了的 agent；没有机器（或哪台都没找到）时列内置的 agent（GET /api/providers 的 meta），规则照样能加
   - HostAccept（机器 Tab「接单」，PRD §10.12）：一台机器一条 24 小时的时间带，直接在上面拖出接单时段（空白处拖是新加一段，拖两头改起止，
     拖中间平移，双击删掉，半小时一格；全天就是从 00 拖到 24）；右边是会话上限、一个会话几张、接单（自动领 / 只做派给它的 / 暂停）。
     时间按服务器时间。最下面是所有机器的「自动升级 board」
   - RunLimits（Agent tab「跑多久停」）：单次最长运行、没输出多久算卡住，所有机器一个值
   写操作都是设置项（saveSetting）：当帧生效，保存状态画在那一项旁边，失败回到原处并提示
   ========================================================================== */
import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { ACCEPT_WORDS, acceptOf, acceptPatch, hostLoad, minsWords, type Accept } from '../../lib/hosts';
import { metasOf } from '../../lib/providers';
import { ALL_DAY, DAY, SNAP, agentTake, claimable, hhmm, hostTake, pickState, ruleAgents, serverMinute, tidyWins, togglePick, winWords, type Win } from '../../lib/sched';
import type { AgentMeta, ClaimRule, HostInfo, Label, Workspace } from '../../lib/types';
import { cx } from '../../lib/util';
import { boardAutoPatch, saveSetting, schedPickPatch, useData } from '../../store/data';
import { openHost } from '../../store/ui';
import { Select, Sync, Tip, useNow, useSync } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon } from './bits';
import { Stepper, useSaver } from './Saver';
import { LabelSet } from './LabelSet';

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
const needsLabels = (op: ClaimRule['op']) => op !== 'none';
const plain = (r: ClaimRule) => ({ op: r.op, labels: needsLabels(r.op) ? r.labels : [], agent: r.agent });

/** 没有机器上报 agent 时拿内置 agent 的说明（GET /api/providers 的 meta）：领取规则照样能选 agent。
    取不到（旧服务端、断网）时是空的，「加一条规则」仍然不能点 */
function useBuiltinAgents(need: boolean): AgentMeta[] {
  const [meta, setMeta] = useState<AgentMeta[]>([]);
  useEffect(() => {
    if (!need) return;
    let alive = true;
    api
      .providers()
      .then(v => alive && setMeta(metasOf(v)))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [need]);
  return need ? meta : [];
}

/** 领取规则（Agent tab）：自动领取的白板、按 agent 分卡的规则、兜底 */
export function ClaimRules({ hosts }: { hosts: HostInfo[] }) {
  const ws = useData(s => s.ws);
  const now = useNow(30_000);
  const { sync, track } = useSync();
  const sched = ws?.sched;
  const [adding, setAdding] = useState(false);
  // 所有项目里的标签，同名（不区分大小写）的只留第一个；id 换成序号，选择器按它勾选
  const labels = useMemo(() => {
    const seen = new Set<string>();
    return (ws?.projects ?? [])
      .flatMap(p => p.labels)
      .filter(l => !seen.has(l.name.toLowerCase()) && !!seen.add(l.name.toLowerCase()))
      .map((l, i) => ({ ...l, id: i + 1 }));
  }, [ws?.projects]);
  // 至少在一台机器上找得到命令的 agent；一个都没有时用内置的 agent
  const found = hosts.flatMap(h => h.agents.filter(a => a.found)).filter((a, i, all) => all.findIndex(b => b.id === a.id) === i);
  const builtin = useBuiltinAgents(found.length === 0);
  // 标签选择器勾一个存一个：连着勾时回调可能来自上一帧，改的要是最新一份规则
  const latest = useRef<ClaimRule[]>([]);
  // 旧版服务端没有这一项：不显示这一节
  if (!ws || !sched) return null;
  const rules = sched.rules;
  latest.current = rules;
  const minute = serverMinute(ws, now);
  const fallback = sched.fallback ?? '';
  const pickable = found.length ? found : builtin;
  const agentName = (id: string) => hosts.flatMap(h => h.agents).find(a => a.id === id)?.name ?? builtin.find(a => a.id === id)?.name ?? id;
  const agentOptions = pickable.map(a => ({ value: a.id, label: a.name }));
  const boards = ws.projects.flatMap(p => p.boards);

  const saveRules = (next: ClaimRule[]) => {
    latest.current = next;
    track(
      'rules',
      saveSetting('sched:rules', { ws: w => (w.sched ? { ...w, sched: { ...w.sched, rules: next } } : w) }, () => api.updateSched({ rules: next.map(plain) })),
    );
  };
  const saveFallback = (v: string) =>
    track(
      'fallback',
      saveSetting('sched:fallback', { ws: w => (w.sched ? { ...w, sched: { ...w.sched, fallback: v || null, fallbackActive: !!v } } : w) }, () => api.updateSched({ fallback: v })),
    );
  const toggleBoard = (path: string, on: boolean) =>
    track(
      `board:${path}`,
      // 和白板设置里「参与自动领取」是同一个值、同一个 key：一处改了另一处跟着变，请求一个接一个发
      saveSetting(`sched:board:${path}`, boardAutoPatch(path, on), () => api.updateBoard(path, { auto: on })),
    );
  const edit = (i: number, change: Partial<ClaimRule>) => saveRules(latest.current.map((x, j) => (j === i ? { ...x, ...change, missing: change.labels ? [] : x.missing } : x)));
  // 「由这些机器做」：勾上 / 取消一台机器（按 agent 一组存），或者回到「所有装了它的机器都参与」。
  // 设置项：当帧生效，保存状态画在胶囊旁边，失败回到原处
  const savePick = (agent: string, on: number[], req: { add?: number[]; remove?: number[]; all?: boolean }) =>
    track(
      `pick:${agent}`,
      saveSetting(`sched:pick:${agent}`, schedPickPatch(agent, on, req.all), () => api.updateSched({ agent, ...req })),
    );
  const move = (i: number, d: number) => {
    const next = [...rules];
    const [r] = next.splice(i, 1);
    next.splice(i + d, 0, r);
    saveRules(next);
  };

  const cards = ruleAgents(rules, fallback);
  return (
    <section className="sc sd">
      <div className="sc-head">
        <div>
          <h2>领取规则</h2>
          <p>
            哪些卡自动交给哪个 agent，所有机器共用；机器在自己的接单时段里有符合规则的卡就领，几台都能接时按空位分。时间都按服务器时间
            {sched.tz ? `（${sched.tz}）` : ''}。每台机器什么时候接单、开几个会话在「机器 → 接单」里。
          </p>
        </div>
        <div className="sc-acts">
          {/* 保存状态贴在按钮左边：和其他设置项一样包在 .hp-ctl 里，不然定位落到页面上、画到可视区外面（default/main#391） */}
          <span className="hp-ctl">
            <Sync s={sync.rules} />
            <button className="btn sm" type="button" disabled={adding || agentOptions.length === 0} onClick={() => setAdding(true)}>
              <Icon name="plus" />
              加一条规则
            </button>
          </span>
        </div>
      </div>

      <div className="sd-boards">
        <span className="sd-k">自动领取的白板</span>
        {boards.map(b => {
          const on = b.auto !== false;
          return (
            <Tip key={b.path} text={on ? `${b.name}：点一下不再自动领取（手动派发照常）` : `${b.name}：点一下参与自动领取`}>
              <button type="button" className={cx('sd-bchip', on && 'on', sync[`board:${b.path}`] === 'failed' && 'hp-rollback')} aria-pressed={on} onClick={() => toggleBoard(b.path, !on)}>
                <i />
                {b.path}
                <small>{claimable(b)}</small>
              </button>
            </Tip>
          );
        })}
      </div>

      {adding && (
        <RuleForm
          labels={labels}
          agents={agentOptions}
          onCancel={() => setAdding(false)}
          onAdd={r => {
            saveRules([...latest.current, r]);
            setAdding(false);
          }}
        />
      )}

      {cards.length === 0 && !adding ? (
        <p className="hp-empty sd-empty">
          {agentOptions.length === 0 ? '还没有机器上报 agent：先在要干活的机器上运行 board host。' : '还没有领取规则：机器不会自动领卡，只做派给它的。点「加一条规则」开始。'}
        </p>
      ) : (
        <div className={cx('sd-cards', sync.rules === 'failed' && 'hp-rollback')}>
          {cards.map(a => {
            const mine = rules.map((r, i) => ({ r, i })).filter(x => x.r.agent === a);
            const has = hosts.filter(h => h.agents.some(x => x.id === a));
            const pick = pickState(ws, a, has.map(h => h.id));
            const able = has.filter(h => pick.on.includes(h.id) && agentTake(h, a, hostLoad(ws, h).running.length, minute).ok).length;
            return (
              <article key={a} className="sd-card">
                <header>
                  <AgentIcon id={a} />
                  <b>{agentName(a)}</b>
                  <span className={cx('sd-sum', able === 0 && 'bad')}>
                    {has.length === 0 ? '没有机器上有它' : pick.none ? '一台机器都没选：不自动领卡' : able ? `${able} 台现在能接` : '现在没有机器能接'}
                  </span>
                </header>
                <div className="sd-row">
                  <span className="sd-k">领这些卡</span>
                  <div className="sd-conds">
                    {mine.map(({ r, i }) => (
                      <div key={`${i}-${r.op}-${r.labels.join(',')}`} className="sd-cond">
                        <Tip text="先后顺序：一张卡用第一条命中的规则">
                          <span className="sd-no">{i + 1}</span>
                        </Tip>
                        <Select className="sd-op" value={r.op} options={OP_OPTIONS} aria-label="条件" onChange={v => edit(i, needsLabels(v as ClaimRule['op']) && !r.labels.length ? {} : { op: v as ClaimRule['op'] })} />
                        {needsLabels(r.op) ? (
                          <LabelSet value={r.labels} labels={labels} missing={r.missing} onChange={v => (v.length ? edit(i, { labels: v }) : undefined)} />
                        ) : (
                          <span className="hp-lset hp-lset-none">卡片没有任何标签</span>
                        )}
                        <span className="sd-cond-acts">
                        <button type="button" className="icon-btn sm" aria-label="提前" disabled={i === 0} onClick={() => move(i, -1)}>
                          <Icon name="chevUp" />
                        </button>
                        <button type="button" className="icon-btn sm" aria-label="推后" disabled={i === rules.length - 1} onClick={() => move(i, 1)}>
                          <Icon name="chevDown" />
                        </button>
                        <button type="button" className="icon-btn sm" aria-label="删掉这条规则" onClick={() => saveRules(rules.filter((_, j) => j !== i))}>
                          <Icon name="trash" />
                        </button>
                        </span>
                        {!!r.missing?.length && <em className="hp-crule-warn">项目里没有标签 {r.missing.map(l => `「${l}」`).join('')}</em>}
                      </div>
                    ))}
                    {fallback === a && (
                      <div className="sd-cond">
                        <span className="sd-no">·</span>
                        <span className="hp-lset hp-lset-none">其余卡片（规则都没命中的）</span>
                      </div>
                    )}
                  </div>
                </div>
                <div className="sd-row">
                  <span className="sd-k">由这些机器做</span>
                  <div className="sd-pills">
                    {has.length === 0 && <em className="sd-none">没有机器上有 {agentName(a)}：这些规则不生效</em>}
                    {has.map(h => {
                      const t = agentTake(h, a, hostLoad(ws, h).running.length, minute);
                      const on = pick.on.includes(h.id);
                      const pickTip = on ? `${h.name}：点一下不再由这台机器做${agentName(a)}的卡（手动派发照常）` : `${h.name}：点一下由这台机器做${agentName(a)}的卡`;
                      return (
                        <Tip key={h.id} text={`${h.name}：${t.why}${'　'}${pickTip}`}>
                          <span className={cx('sd-mp-wrap', on && 'on')}>
                            <button
                              type="button"
                              className={cx('sd-mp', 'pick', on ? t.tone : 'unpicked', sync[`pick:${a}`] === 'failed' && 'hp-rollback')}
                              aria-pressed={on}
                              onClick={() => {
                                const next = togglePick(pick, h.id);
                                savePick(a, next, next.includes(h.id) ? { add: [h.id] } : { remove: [h.id] });
                              }}
                            >
                              <i />
                              {on && <Icon name="check" />}
                              {h.name}
                              <small>{t.ok ? t.why : t.why.split('，')[0]}</small>
                            </button>
                            <button type="button" className="sd-mp-open" aria-label={`打开机器 ${h.name}`} onClick={() => openHost(h.name)}>
                              <Icon name="chevRight" />
                            </button>
                          </span>
                        </Tip>
                      );
                    })}
                    {has.length > 0 && pick.none && <em className="sd-none sd-pick-none">一台都没选：{agentName(a)} 不自动领卡</em>}
                    {has.length > 0 && !pick.none && !pick.unset && pick.on.length < has.length && (
                      <button type="button" className="sd-pick-all" onClick={() => savePick(a, has.map(h => h.id), { all: true })}>
                        全部参与
                      </button>
                    )}
                    <Sync s={sync[`pick:${a}`]} inline />
                  </div>
                </div>
              </article>
            );
          })}
        </div>
      )}
      {(rules.length > 0 || fallback) && (
        <p className={cx('hp-note sd-after', sync.fallback === 'failed' && 'hp-rollback')}>
          其余卡片（规则都没命中的）
          <Icon name="chevRight" />
          <Select
            className="sd-fallback"
            value={fallback}
            options={[{ value: '', label: '不领' }, ...(fallback && !pickable.some(a => a.id === fallback) ? [{ value: fallback, label: `${agentName(fallback)}（找不到）` }] : []), ...agentOptions]}
            aria-label="规则都没命中的卡片由哪个 agent 做"
            onChange={saveFallback}
          />
          <Sync s={sync.fallback} inline />
        </p>
      )}
    </section>
  );
}

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

/** 新加一条规则：条件、标签、交给哪个 agent */
function RuleForm({ labels, agents, onAdd, onCancel }: { labels: Label[]; agents: { value: string; label: string }[]; onAdd: (r: ClaimRule) => void; onCancel: () => void }) {
  const [op, setOp] = useState<ClaimRule['op']>('in');
  const [picked, setPicked] = useState<string[]>([]);
  const [agent, setAgent] = useState(agents[0]?.value ?? '');
  const ok = !!agent && (!needsLabels(op) || picked.length > 0);
  return (
    <div className="sd-form">
      <Select className="sd-op" value={op} options={OP_OPTIONS} aria-label="新规则的条件" onChange={v => setOp(v as ClaimRule['op'])} />
      {needsLabels(op) ? <LabelSet value={picked} labels={labels} onChange={setPicked} /> : <span className="hp-lset hp-lset-none">卡片没有任何标签</span>}
      <Icon name="chevRight" />
      <Select className="sd-agent-pick" value={agent} options={agents} aria-label="交给哪个 agent" onChange={setAgent} />
      <span className="hp-sp" />
      <button className="btn sm" type="button" onClick={onCancel}>
        取消
      </button>
      <button className="btn sm brand" type="button" disabled={!ok} onClick={() => onAdd({ op, labels: needsLabels(op) ? picked : [], agent, active: true })}>
        <Icon name="plus" />
        加上
      </button>
    </div>
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
