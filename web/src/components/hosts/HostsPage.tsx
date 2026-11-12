/* 机器 Tab：侧栏机器列表、主区机架总览（每台机器一行：左边是机器，右边是它的并发位，一条车道一个任务）；主区可以切到多机对比 */
import { useState } from 'react';
import { api } from '../../lib/api';
import { agentMark, cronSummary, hostLoad, hostState, hostsOf, inSession, queueReason, sessionQueue, since, skillMiss, SOURCE } from '../../lib/hosts';
import type { Dispatch, HostInfo, Workspace } from '../../lib/types';
import { readiness } from '../../lib/ready';
import { cx, pref } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, openHost, toast, toastError, useUI } from '../../store/ui';
import { Segmented, Title, useNow } from '../bits';
import { Icon } from '../Icon';
import { goCard } from './Dispatch';
import { Beat, Dot, Mark, OsGlyph, StatePill, lastSeen, usePollHosts } from './bits';
import { Compare, ReadyBadge, useServerVersion } from './Ready';

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
        {/* 没有机器时只留主区空状态里的一个「添加机器」 */}
        {hosts.length > 0 && (
          <button aria-label="添加机器" className="icon-btn sm" type="button" data-tip="添加机器" data-tip-align="end" onClick={() => openHost('new')}>
            <Icon name="plus" />
          </button>
        )}
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

/** 车道上的时间只写到分钟（页面 5 秒刷新一次，秒数会跳着走） */
function span(ms: number) {
  const m = Math.floor(Math.max(0, ms) / 60000);
  return m < 1 ? '不到 1 分' : m < 60 ? `${m} 分` : `${Math.floor(m / 60)} 小时 ${m % 60} 分`;
}

/** 机器的顺序：接单的在前，暂停的其次，离线的最后；同一组里保持服务端的顺序（状态不变时行不跳动） */
const rank = (h: HostInfo) => (!h.online ? 2 : h.paused ? 1 : 0);
const ordered = (hosts: HostInfo[]) => hosts.map((h, i) => ({ h, i })).sort((a, b) => rank(a.h) - rank(b.h) || a.i - b.i).map(x => x.h);

/** 一台机器的并发位：在跑的排前面（颜色是 agent 的），剩下的是空位 */
const slotsOf = (h: HostInfo, running: Dispatch[]) => Array.from({ length: Math.max(h.max, running.length) }, (_, i) => running[i] ?? null);

/** 并发位的小格子：机器行里一台机器一组，顶部总览里所有机器连成一条 */
function Pips({ h, running }: { h: HostInfo; running: Dispatch[] }) {
  return (
    <span className={cx('hp-pips', !h.online && 'off', h.online && h.paused && 'paused')} aria-hidden="true">
      {slotsOf(h, running).map((r, i) => (
        <i key={r ? r.id : `e${i}`} className={r ? 'on' : undefined} style={r ? { ['--ac' as string]: agentMark(r.agent).color } : undefined} />
      ))}
    </span>
  );
}

/** 顶部总览：在线、并发位、排队三个数，加一条所有机器的并发位（点一组打开那台机器） */
function Fleet({ ws, hosts }: { ws: Workspace | null; hosts: HostInfo[] }) {
  const on = hosts.filter(h => h.online);
  const open = on.filter(h => !h.paused);
  const slots = open.reduce((n, h) => n + h.max, 0);
  const used = open.reduce((n, h) => n + hostLoad(ws, h).running.length, 0);
  // 会话里排着的不算排队：它们不等空位，由会话接着做
  const queue = (ws?.dispatches ?? []).filter(d => d.state === 'queued' && !inSession(d)).length;
  return (
    <div className="hp-fleet enter" style={{ ['--i' as string]: 0 }}>
      <dl className="hp-stats">
        <div>
          <dt>在线</dt>
          <dd>
            {on.length}
            <small>/{hosts.length}</small>
          </dd>
        </div>
        <div>
          <dt>并发位在用</dt>
          <dd>
            {used}
            <small>/{slots}</small>
          </dd>
        </div>
        <div className={cx(queue > 0 && 'hot')}>
          <dt>排队</dt>
          <dd>{queue}</dd>
        </div>
      </dl>
      <div className="hp-strip" role="list" aria-label="所有机器的并发位">
        {hosts.map(h => {
          const { running } = hostLoad(ws, h);
          const s = hostState(h, running.length);
          return (
            <button
              key={h.id}
              type="button"
              role="listitem"
              className="hp-strip-g"
              style={{ ['--n' as string]: Math.max(h.max, running.length) }}
              data-tip={`${h.name} · ${s.label}${h.online && !h.paused ? ` ${running.length}/${h.max}` : ''}`}
              onClick={() => openHost(h.name)}
            >
              <Pips h={h} running={running} />
              <span className="hp-strip-name">{h.name}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** 一条车道：在跑的任务（agent 色的轨、标题、已用时间、心跳），点开是那张卡片；会话里还排着卡片时写「后面还有 N 张」 */
function RunLane({ r, now, more }: { r: Dispatch; now: number; more: number }) {
  return (
    <button type="button" className="hp-lane on" style={{ ['--ac' as string]: agentMark(r.agent).color }} onClick={() => goCard(r)} title={`打开 ${r.board}#${r.card}`}>
      <Mark id={r.agent} title={r.agentLabel} />
      <span className="hp-cid">#{r.card}</span>
      <span className="hp-lane-title">
        <Title text={r.title} />
      </span>
      {r.stopBy && <span className="hp-stopping">正在停止</span>}
      {more > 0 && <span className="hp-lane-more">后面还有 {more} 张</span>}
      <span className="hp-lane-board">{r.board}</span>
      <span className="hp-lane-src">{SOURCE[r.source]}</span>
      <span className="hp-lane-t">{r.startedAt != null ? span(now - r.startedAt) : '刚开始'}</span>
      <Beat at={r.beatAt} now={now} />
    </button>
  );
}

/** 排队的任务：虚线车道，写明在等什么 */
function QueueLane({ r, h, now }: { r: Dispatch; h: HostInfo; now: number }) {
  return (
    <button type="button" className="hp-lane queued" onClick={() => goCard(r)} title={`打开 ${r.board}#${r.card}`}>
      <Mark id={r.agent} title={r.agentLabel} />
      <span className="hp-cid">#{r.card}</span>
      <span className="hp-lane-title">
        <Title text={r.title} />
      </span>
      <span className="hp-lane-board">{r.board}</span>
      <span className="hp-lane-wait">
        排队 · {queueReason(h)}
        <i> · 已排 {span(now - r.createdAt)}</i>
      </span>
    </button>
  );
}

/** 一台机器的一行（磁贴 + 车道）；单独导出给单元测试画。server 是服务端版本，没取到时不检查 board 版本 */
export function Rack({ h, ws, now, index, server }: { h: HostInfo; ws: Workspace | null; now: number; index: number; server?: string | null }) {
  const { running, queued } = hostLoad(ws, h);
  const free = Math.max(0, h.max - running.length);
  const cron = cronSummary(h);
  // 磁贴上不放 agent 胶囊（默认 agent、领取规则、兜底里的 agent 在机器详情里看）；只在有 agent 看不到 board-skills 时提一行
  const noskill = h.agents.filter(a => skillMiss(h, a)).length;
  return (
    <article
      className={cx('hp-rack enter', !h.online && 'off', h.online && h.paused && 'paused')}
      style={{ ['--i' as string]: index + 1 }}
      onClick={e => !(e.target as HTMLElement).closest('button') && openHost(h.name)}
    >
      <button type="button" className="hp-rack-id" onClick={() => openHost(h.name)} aria-label={`${h.name}：看详情、改规则`}>
        <span className="hp-rack-top">
          <OsGlyph os={h.os} />
          <span className="hp-rack-name">
            <b>{h.name}</b>
            <span>
              {h.os} · {lastSeen(h, now)}
            </span>
          </span>
        </span>
        <span className="hp-rack-state">
          <StatePill h={h} running={running.length} />
          <Pips h={h} running={running} />
          {h.online && (
            <span className="hp-rack-load">
              {running.length}/{h.max}
            </span>
          )}
          {/* 磁贴上只加这一个小标记，哪几项没通过在机器详情的「就绪」里看 */}
          <ReadyBadge checks={readiness(h, now, server)} />
        </span>
        {(cron || h.round || (!!h.unmatched && !h.fallback) || noskill > 0) && (
          <span className="hp-rack-notes">
            {h.round && (
              <span className="live">
                <Icon name="timer" />
                正在领一轮
              </span>
            )}
            {cron && (
              <span>
                <Icon name="timer" />
                {cron}
              </span>
            )}
            {!!h.unmatched && !h.fallback && (
              <span className="warn">
                <Icon name="info" />
                {h.unmatched} 张卡片没有匹配规则
              </span>
            )}
            {/* 原来靠胶囊外框提示的「看不到 board-skills」，改成磁贴底部一行警告；鼠标放上去写装到哪里 */}
            {noskill > 0 && (
              <span className="warn" title={`在用的启动环境里看不到 board-skills，点开看是哪个 agent、装到哪里（见 docs/agents.md）`}>
                <Icon name="alert" />
                {noskill} 个 agent 看不到 board-skills
              </span>
            )}
          </span>
        )}
      </button>
      <div className="hp-lanes">
        {running.map(r => (
          <RunLane key={r.id} r={r} now={now} more={sessionQueue(ws, r.session).length} />
        ))}
        {!h.online ? (
          <div className="hp-lane idle dead">离线{queued.length > 0 ? ` · 上线后接着做排队的 ${queued.length} 张` : ' · 上线后才能接单'}</div>
        ) : h.paused ? (
          <div className="hp-lane idle paused">
            <Icon name="pause" />
            暂停接单{free > 0 ? ` · ${free} 个并发位停用` : ''}
          </div>
        ) : (
          free > 0 && (
            <div className="hp-lane idle">
              {free} 个空位{running.length === 0 ? ' · 等卡片派过来' : ''}
            </div>
          )
        )}
        {queued.map(r => (
          <QueueLane key={r.id} r={r} h={h} now={now} />
        ))}
      </div>
    </article>
  );
}

/** 全部暂停 / 全部恢复、全部停止：机器的状态要等服务端返回，按钮先变忙、禁用，直到返回 */
export function FleetActions({ hosts }: { hosts: HostInfo[] }) {
  const all = hosts.filter(h => h.online);
  const paused = all.length > 0 && all.every(h => h.paused);
  const [busy, setBusy] = useState<'pause' | 'stop' | null>(null);
  const pause = async () => {
    setBusy('pause');
    try {
      await optimistic(null, () => api.pauseAllHosts(!paused));
      toast({ type: 'success', title: paused ? '所有机器恢复接单' : '所有机器暂停接单' });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const stop = async () => {
    const running = (useData.getState().ws?.dispatches ?? []).filter(d => d.state === 'running').length;
    const yes = await confirm({
      title: '停止所有机器上正在跑的派发？',
      text: `会结束 ${running} 个正在跑的 agent 进程（连同它们启动的子进程），卡片按异常退出处理；所有机器同时暂停接单，排队的派发继续排队。确认后要点「全部恢复」才会重新接单。`,
      ok: '全部停止',
      icon: 'alert',
      tone: 'danger',
    });
    if (!yes) return;
    setBusy('stop');
    try {
      await optimistic(null, () => api.stopAllHosts());
      toast({ type: 'success', title: '已停止所有派发，所有机器暂停接单' });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  return (
    <>
      <button className={cx('btn sm', busy === 'pause' && 'is-busy')} type="button" disabled={busy !== null || all.length === 0} onClick={pause}>
        <Icon name={paused ? 'play' : 'pause'} />
        {paused ? '全部恢复' : '全部暂停'}
      </button>
      <button className={cx('btn sm danger', busy === 'stop' && 'is-busy')} type="button" disabled={busy !== null} onClick={stop}>
        <Icon name="stop" />
        全部停止
      </button>
    </>
  );
}

type View = 'racks' | 'compare';
const VIEWS: [View, string][] = [
  ['racks', '总览'],
  ['compare', '对比'],
];

export function HostsPage() {
  const ws = useData(s => s.ws);
  const tab = useUI(s => s.tab);
  const now = useNow(5000);
  const hosts = ordered(hostsOf(ws));
  const server = useServerVersion();
  // 机位总览 / 多机对比，选过的记在本机
  const [view, setView] = useState<View>(() => (pref.get<string>('hosts.view', 'racks') === 'compare' ? 'compare' : 'racks'));
  const pick = (v: View) => {
    pref.set('hosts.view', v);
    setView(v);
  };
  usePollHosts(tab === 'hosts');
  return (
    <div className="page page-hosts" data-page="hosts">
      <div className="hp-b">
        <div className="hp-b-head">
          <div>
            <h1>机器</h1>
            <p>
              {view === 'compare'
                ? '每台机器一行，对比就绪的每一项和 agent 的版本、模型、启动环境；和其他机器不一致的格子标出来。'
                : '每台机器常驻一个 board host。一行一台机器，一条车道一个任务：点任务看卡片，点机器改规则。'}
            </p>
          </div>
          {hosts.length > 0 && (
            <div className="hp-b-acts">
              <Segmented value={view} options={VIEWS} onChange={pick} />
              <FleetActions hosts={hosts} />
              <button className="btn sm" type="button" onClick={() => openHost('new')}>
                <Icon name="plus" />
                添加机器
              </button>
            </div>
          )}
        </div>
        {hosts.length === 0 ? (
          <div className="hp-empty-page">
            <Icon name="host" />
            <b>还没有机器</b>
            <span>在要干活的机器上运行 board host，它会出现在这里；之后可以把卡片派给它，或让它按时间点自己领卡。</span>
            <button className="btn sm brand" type="button" onClick={() => openHost('new')}>
              添加机器
            </button>
          </div>
        ) : view === 'compare' ? (
          <Compare hosts={hosts} ws={ws} now={now} server={server} />
        ) : (
          <>
            <Fleet ws={ws} hosts={hosts} />
            <div className="hp-racks">
              {hosts.map((h, i) => (
                <Rack key={h.id} h={h} ws={ws} now={now} index={i} server={server} />
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
