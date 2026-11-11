/* 机器详情：居中弹窗（正在跑、排队、派发规则、本机 agent、启动环境、白板与目录、最近）；添加机器 */
import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { agentMark, envName, hostLoad, hostsOf, isLocalHost, queueReason, resultOf, since, skillGaps, skillMiss, SOURCE } from '../../lib/hosts';
import { installSkill, localHost, shell, type LocalHost } from '../../lib/platform';
import type { ClaimRule, Dispatch, HostAgent, HostInfo, Label } from '../../lib/types';
import { copyText, cx, durWords, pref, replay } from '../../lib/util';
import { buildCron, describeCron, normalizeCron, type Schedule } from '../../lib/cron';
import { hostKey, optimistic, useBusy, useData } from '../../store/data';
import { confirm, openHost, openPicker, setTab, toast, toastError, updatePicker, useUI } from '../../store/ui';
import { LabelChip, Title, useNow } from '../bits';
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

  /** 机器设置：当帧生效，被拒绝时改回来并提示。保存完之前这台机器的设置都禁用（不会在没确认的值上再改一次） */
  const busy = useBusy(hostKey(h.name));
  const patch = async (p: Parameters<typeof api.updateHost>[1]) => {
    const { env, ...fields } = p;
    const local = patchHost(h.name, x => ({
      ...x,
      ...fields,
      agents: env ? x.agents.map(a => (a.id === env.agent ? { ...a, env: env.env } : a)) : x.agents,
    }));
    // 改的是哪一行：保存成功后闪一下，失败时值回到原处并回弹，不静默跳变
    const row = (document.activeElement as HTMLElement | null)?.closest('.hp-rule, .hp-arow, .hp-crules, .hp-cron, .d-actions');
    try {
      await optimistic(local, () => api.updateHost(h.name, p));
      row?.classList.remove('hp-rollback');
      replay(row, 'hp-saved');
    } catch (e) {
      row?.classList.remove('hp-saved');
      replay(row, 'hp-rollback');
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
                <b key={h.max} className="hp-num">
                  {h.max}
                </b>
                <button type="button" aria-label="增加" disabled={busy || h.max >= 8} onClick={() => patch({ max: h.max + 1 })}>
                  +
                </button>
              </span>
            </div>
            {h.perSession != null && (
              <div className="hp-rule">
                <div>
                  <b>一个会话最多做几张</b>
                  <span>定时领取启动的 agent 做完一张后用 board next 接着做，优先挑和第一张相近的，做满就退出、下一批换新会话；手动派发只做指定的那一张</span>
                </div>
                <span className="hp-stepper">
                  <button type="button" aria-label="减少" disabled={busy || h.perSession <= 1} onClick={() => patch({ perSession: h.perSession! - 1 })}>
                    −
                  </button>
                  <b key={h.perSession} className="hp-num">
                    {h.perSession}
                  </b>
                  <button type="button" aria-label="增加" disabled={busy || h.perSession >= 8} onClick={() => patch({ perSession: h.perSession! + 1 })}>
                    +
                  </button>
                </span>
              </div>
            )}
            <div className="hp-rule">
              <div>
                <b>定时领取</b>
                <span>到了这些时间（机器本地时区），按下面的领取规则在这台机器的白板上领卡，把空位填满，一个会话做完再开下一个，直到没有符合规则的卡片；没有领取规则时不会自动领取</span>
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
                <span>
                  {h.round
                    ? '正在领一轮：有空位就领下一张，所有白板都没有符合规则的卡片时结束'
                    : hasRules
                      ? '效果与到点相同；机器离线或暂停接单时不能领'
                      : '先添加一条领取规则；配好之后，效果与到点相同'}
                </span>
              </div>
              <button
                className="btn sm"
                type="button"
                disabled={busy || !h.online || h.paused || h.round || !hasRules}
                title={hasRules ? undefined : '先添加一条领取规则'}
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
            {h.rules != null && <ClaimRules h={h} patch={patch} busy={busy} />}
          </section>

          <section className="d-section" style={{ ['--i' as string]: 3 }}>
            <div className="d-label">本机 agent</div>
            <SkillBanner h={h} fixed={fixed} setFixed={setFixed} />
            {h.agents.map(a => (
              <AgentRow key={a.id} h={h} a={a} patch={patch} busy={busy} fixed={fixed?.includes(a.id)} />
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
            {h.dirs.length ? (
              h.dirs.map(d => (
                <div className="hp-dir" key={d.board}>
                  <span className="hp-board">{d.board}</span>
                  <Icon name="move" />
                  <MidPath path={d.path} />
                </div>
              ))
            ) : (
              <p className="hp-note">没有白板：board host 启动时用 --dir 项目/白板=路径 加上，否则接不到卡片</p>
            )}
          </Fold>

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

/** 本机 agent 的一行：图标、名称（看不到 board-skills 时后面一个「缺 skill」小胶囊，悬停写装到哪里；刚一键装好的变绿「skill 已装」）、
 *  启动环境（下拉，每项带版本号，找不到的置灰写「没找到」）、「默认」（点一下设为默认）。
 *  旧版 board host 不上报启动环境：不显示下拉，写版本号 */
function AgentRow({ h, a, patch, busy, fixed }: { h: HostInfo; a: HostAgent; patch: (p: Parameters<typeof api.updateHost>[1]) => Promise<void>; busy: boolean; fixed?: boolean }) {
  const def = a.id === h.agent;
  const envs = h.envs ?? [];
  const checks = a.envs ?? [];
  const name = (id: string) => envName(h, id) ?? id;
  const miss = skillMiss(h, a);
  return (
    <>
      <div className={cx('hp-arow', !a.found && 'missing')} style={{ ['--ac' as string]: agentMark(a.id).color }}>
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
        {envs.length > 0 &&
          (checks.length ? (
            <select
              className="input hp-env-select"
              disabled={busy || !checks.some(c => c.found)}
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
  ['not_in', '一个都不带'],
  ['equal', '标签正好是'],
];

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

/** 领取规则：条件（带其中任一个 / 一个都不带 / 标签正好是）→ agent，有顺序（第一条命中的生效），最后可选兜底。
 *  只影响定时领取和会话里的 board next，手动派发、board claim 不受影响 */
function ClaimRules({ h, patch, busy }: { h: HostInfo; patch: (p: Parameters<typeof api.updateHost>[1]) => Promise<void>; busy: boolean }) {
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
  const [agent, setAgent] = useState('');
  const pick = agent || found[0]?.id || '';
  const has = (a: string) => found.some(x => x.id === a);
  const save = (next: ClaimRule[]) => patch({ rules: next.map(r => ({ op: r.op, labels: r.labels, agent: r.agent, active: has(r.agent), missing: r.missing })) });
  const edit = (i: number, change: Partial<ClaimRule>) => save(rules.map((x, j) => (j === i ? { ...x, ...change, missing: change.labels ? [] : x.missing } : x)));
  const move = (i: number, d: number) => {
    const next = [...rules];
    const [r] = next.splice(i, 1);
    next.splice(i + d, 0, r);
    void save(next);
  };
  const add = () => {
    if (!picked.length || !pick || busy) return;
    void save([...rules, { op, labels: picked, agent: pick }]);
    setPicked([]);
  };
  const fallback = h.fallback || '';
  return (
    <div className="hp-crules">
      <div className="hp-rule-text">
        <b>领取规则</b>
        <span>定时领取只领满足条件的卡片，交给对应的 agent；从上到下，第一条命中的生效。没有领取规则时不会自动领取，手动派发不受影响</span>
      </div>
      {rules.map((r, i) => (
        <div className={cx('hp-crule', !r.active && 'off')} key={`${i}-${r.op}-${r.labels.join(',')}`}>
          <select className="input hp-crule-op" value={r.op} disabled={busy} aria-label="条件" onChange={e => edit(i, { op: e.target.value as ClaimRule['op'] })}>
            {OPS.map(([k, t]) => (
              <option key={k} value={k}>
                {t}
              </option>
            ))}
          </select>
          <LabelSet value={r.labels} labels={one?.labels ?? labels} project={one?.key} missing={r.missing} disabled={busy} onChange={v => (v.length ? edit(i, { labels: v }) : undefined)} />
          <Icon name="chevRight" />
          <select className="input hp-crule-agent" value={r.agent} disabled={busy} aria-label="由哪个 agent 做" onChange={e => edit(i, { agent: e.target.value })}>
            {!has(r.agent) && <option value={r.agent}>{agentName(r.agent)}（找不到）</option>}
            {found.map(a => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
          <span className="hp-sp" />
          <button type="button" className="icon-btn sm" aria-label="上移" disabled={busy || i === 0} onClick={() => move(i, -1)}>
            <Icon name="chevUp" />
          </button>
          <button type="button" className="icon-btn sm" aria-label="下移" disabled={busy || i === rules.length - 1} onClick={() => move(i, 1)}>
            <Icon name="chevDown" />
          </button>
          <button type="button" className="icon-btn sm" aria-label="删除这条规则" disabled={busy} onClick={() => save(rules.filter((_, j) => j !== i))}>
            <Icon name="trash" />
          </button>
          {!r.active && <em className="hp-crule-warn">这台机器上找不到 {agentName(r.agent)}，这条规则不生效</em>}
          {!!r.missing?.length && <em className="hp-crule-warn">项目里没有标签 {r.missing.map(l => `「${l}」`).join('')}</em>}
        </div>
      ))}
      <div className="hp-crule add">
        <select className="input hp-crule-op" value={op} aria-label="新规则的条件" onChange={e => setOp(e.target.value as ClaimRule['op'])}>
          {OPS.map(([k, t]) => (
            <option key={k} value={k}>
              {t}
            </option>
          ))}
        </select>
        <LabelSet value={picked} labels={one?.labels ?? labels} project={one?.key} onChange={setPicked} />
        <Icon name="chevRight" />
        <select className="input hp-crule-agent" value={pick} aria-label="新规则的 agent" onChange={e => setAgent(e.target.value)}>
          {found.map(a => (
            <option key={a.id} value={a.id}>
              {a.name}
            </option>
          ))}
        </select>
        <button className="btn sm" type="button" disabled={busy || !picked.length || !pick} onClick={add}>
          <Icon name="plus" />
          加规则
        </button>
      </div>
      {(rules.length > 0 || fallback) && (
        <div className={cx('hp-crule fallback', fallback && h.fallbackActive === false && 'off')}>
          <span className="hp-crule-label">其余卡片</span>
          <Icon name="chevRight" />
          <select
            className="input hp-crule-agent"
            value={fallback}
            disabled={busy}
            aria-label="一条规则都没命中的卡片由哪个 agent 做"
            onChange={e => patch({ fallback: e.target.value })}
          >
            <option value="">不领</option>
            {fallback && !has(fallback) && <option value={fallback}>{agentName(fallback)}（找不到）</option>}
            {found.map(a => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
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

/** 定时领取：胶囊列表（写说明，悬停看 cron 表达式），可去掉；加的时候选「每天 / 工作日 / 每周几 / 每月几号」和时间，
    或「每隔几分钟 / 几小时」，由 buildCron 生成表达式，不让人手写（手写容易错）。旧的手写表达式说不出来时原样显示 */
function CronTimes({ h, patch, busy }: { h: HostInfo; patch: (p: { cron: string[] }) => Promise<void>; busy: boolean }) {
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
    if (busy || !cron || dup) return;
    void patch({ cron: [...h.cron, cron] });
  };
  const timed = kind !== 'minutes' && kind !== 'hours';
  return (
    <div className={cx('hp-cron', !h.cronOn && 'off')}>
      {h.cron.map(x => (
        <span className="hp-time" key={x} title={normalizeCron(x)}>
          {/* 说不出来的（旧的手写表达式）原样用等宽字显示 */}
          {describeCron(x) === normalizeCron(x) ? <code>{normalizeCron(x)}</code> : describeCron(x)}
          <button type="button" aria-label={`去掉 ${describeCron(x)}`} disabled={busy} onClick={() => patch({ cron: h.cron.filter(y => y !== x) })}>
            <Icon name="close" />
          </button>
        </span>
      ))}
      <span className="hp-sched" role="group" aria-label="加一个定时">
        <select className="input" value={kind} aria-label="多久一次" onChange={e => setKind(e.target.value as Schedule['kind'])}>
          {KINDS.map(([k, n]) => (
            <option key={k} value={k}>
              {n}
            </option>
          ))}
        </select>
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
          <select className="input" value={day} aria-label="几号" onChange={e => setDay(+e.target.value)} data-tip="没有这一天的月份（例如 2 月 30 号）不领">
            {Array.from({ length: 31 }, (_, i) => i + 1).map(d => (
              <option key={d} value={d}>
                {d} 号
              </option>
            ))}
          </select>
        )}
        {timed ? (
          <span className="hp-hm">
            <select className="input" value={hour} aria-label="时" onChange={e => setHour(+e.target.value)}>
              {Array.from({ length: 24 }, (_, i) => (
                <option key={i} value={i}>
                  {two(i)}
                </option>
              ))}
            </select>
            :
            <select className="input" value={min} aria-label="分" onChange={e => setMin(+e.target.value)}>
              {Array.from({ length: 12 }, (_, i) => i * 5).map(m => (
                <option key={m} value={m}>
                  {two(m)}
                </option>
              ))}
            </select>
          </span>
        ) : (
          <select
            className="input"
            value={every[kind]}
            aria-label={kind === 'minutes' ? '每隔几分钟' : '每隔几小时'}
            onChange={e => setEvery({ ...every, [kind]: +e.target.value })}
          >
            {EVERY[kind].map(n => (
              <option key={n} value={n}>
                {n} {kind === 'minutes' ? '分钟' : '小时'}
              </option>
            ))}
          </select>
        )}
        <button className="btn sm" type="button" disabled={busy || !cron || dup} onClick={add} data-tip={dup ? '已经有这一条了' : !cron ? '至少选一天' : undefined}>
          <Icon name="plus" />
          加一条
        </button>
      </span>
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
