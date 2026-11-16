/* ==========================================================================
   机器详情（机器详情与配置同步重构，原型变体 B「分页签」）：看状态和改设置分开，没有右侧栏
   - 头部：系统图标、状态、上一台 / 下一台、关闭
   - 标题区：机器名，下面一行系统、board 版本（落后时写「落后于服务端 x.y.z」）、心跳；右上角页签 概况 / agent · N / 设置 · N
     （N 是这一页要处理的项数，0 时不写）。接单三态只在机器 Tab 的「接单」里改
   - 概况：左「正在跑 x/最多」（会话里排着的卡挂在那一行下面）、「排队」；右「最近」（派发和操作记录按时间混排）
   - agent（HostAgents.tsx）、设置（HostSettings.tsx）
   ========================================================================== */
import { useEffect, useState } from 'react';
import { api } from '../../lib/api';
import { agentHealth, pushRows, todayCounts, workspaceExceptions } from '../../lib/hostDetail';
import { fullDisks } from '../../lib/hostRes';
import { hostLoad, resultOf, sessionGroups, since } from '../../lib/hosts';
import type { Dispatch, HostInfo, ProviderView } from '../../lib/types';
import { cx, durWords } from '../../lib/util';
import { useData } from '../../store/data';
import { openSession } from '../../store/ui';
import { Segmented, Tip, Title, useNow } from '../bits';
import { Icon } from '../Icon';
import { lastSeen, Mark, OsGlyph, StatePill } from './bits';
import { HostAgents } from './HostAgents';
import { HostSettings } from './HostSettings';
import { versionLt } from './Manage';
import { useServerVersion } from './Ready';
import { QueueRow, RunRow } from './RunRows';
import { useSaver } from './Saver';

export type DetailTab = 'overview' | 'agents' | 'settings';

export function HostDetail({
  h,
  i,
  n,
  step,
  close,
  on,
  tab,
  setTab,
}: {
  h: HostInfo;
  i: number;
  n: number;
  step: (d: number) => void;
  close: () => void;
  on: boolean;
  tab: DetailTab;
  setTab: (t: DetailTab) => void;
}) {
  const ws = useData(s => s.ws);
  const now = useNow(1000);
  const { running, queued } = hostLoad(ws, h);
  const server = useServerVersion();
  const behind = !!server && !!h.version && versionLt(h.version, server);
  const [recent, setRecent] = useState<Dispatch[] | null>(null);
  // 服务器上的供应商：agent 页「模型」下拉里列的、写进配置文件的地址
  const [view, setView] = useState<ProviderView | null>(null);
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
  useEffect(() => {
    if (!on) return;
    let alive = true;
    api
      .providers()
      .then(v => alive && setView(v))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [on, rev]);
  const { save, sync } = useSaver(h.name);

  // 页签上的数字：这一页要处理的项
  const agentBad = h.agents.filter(a => a.found && !agentHealth(h, a).ok).length;
  const last = h.sync?.last;
  const setBad =
    workspaceExceptions(h).filter(e => e.tone === 'bad').length +
    pushRows(h).filter(r => !r.ok).length +
    (behind ? 1 : 0) +
    fullDisks(h) +
    (h.syncSkills && last?.skills.state === 'failed' && !h.sync?.ask ? 1 : 0) +
    (h.syncMcp && last?.mcp.state === 'failed' && !h.sync?.ask ? 1 : 0) +
    (h.syncGit && last?.git.state === 'failed' && !h.sync?.ask ? 1 : 0);
  const today = todayCounts(recent, now);

  return (
    <div className="drawer-body">
      <div className="d-head">
        <OsGlyph os={h.os} small />
        <StatePill h={h} running={running.length} />
        <span className="d-nav">
          <Tip text="上一台  ↑ / K">
            <button aria-label="上一台" className="icon-btn sm" type="button" disabled={i <= 0} onClick={() => step(-1)}>
              <Icon name="chevUp" />
            </button>
          </Tip>
          <span className="d-pos mono">
            {i + 1} / {n}
          </span>
          <Tip text="下一台  ↓ / J">
            <button aria-label="下一台" className="icon-btn sm" type="button" disabled={i >= n - 1} onClick={() => step(1)}>
              <Icon name="chevDown" />
            </button>
          </Tip>
        </span>
        <span className="spacer" />
        <Tip text="关闭  esc">
          <button aria-label="关闭" className="icon-btn" type="button" data-tip-align="end" onClick={close}>
            <Icon name="close" />
          </button>
        </Tip>
      </div>
      <div className="hd-top">
        <div className="hd-id">
          <h2 className="hp-m-title">{h.name}</h2>
          <div className="hp-m-meta">
            <span>{h.os || '未知系统'}</span>
            <span className={cx(behind && 'hp-behind')}>
              board {h.version || '?'}
              {behind && ` · 落后于服务端 ${server}`}
            </span>
            <span>{lastSeen(h, now)}</span>
          </div>
        </div>
        <div className="hd-tabs">
          <Segmented<DetailTab>
            value={tab}
            options={[
              ['overview', '概况'],
              ['agents', agentBad ? `agent · ${agentBad}` : 'agent'],
              ['settings', setBad ? `设置 · ${setBad}` : '设置'],
            ]}
            onChange={setTab}
          />
        </div>
      </div>

      <div className="hd-body" key={tab}>
        {tab === 'overview' && (
          <div className="hd-over">
            <div>
              <div className="d-label">
                正在跑{' '}
                <span className="count">
                  {running.length}/{h.max}
                </span>
              </div>
              {running.length ? (
                running.map(r => <RunRow key={r.id} r={r} now={now} />)
              ) : (
                <p className="hp-empty">{h.online ? (h.paused ? '暂停：不启动新的派发' : '没有在跑的任务') : '机器离线'}</p>
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
            </div>
            <div className="hd-right">
              <div className="d-label">
                最近
                <span className="act">
                  今天交审 {today.review} · 异常退出 {today.crashed}
                </span>
              </div>
              <Recent h={h} recent={recent} now={now} />
            </div>
          </div>
        )}
        {tab === 'agents' && <HostAgents h={h} view={view} save={save} sync={sync} now={now} />}
        {tab === 'settings' && <HostSettings h={h} save={save} sync={sync} now={now} server={server} behind={behind} running={running.length} close={close} />}
      </div>
    </div>
  );
}

/** 最近：派发（同一个会话的合成一行）和操作记录（谁、什么时间、做了什么）按时间混在一起，新的在前 */
function Recent({ h, recent, now }: { h: HostInfo; recent: Dispatch[] | null; now: number }) {
  if (recent === null) return <p className="hp-note hp-pad">正在加载…</p>;
  const items = [...sessionGroups(recent).map(g => ({ t: g.t, g, o: null })), ...(h.ops ?? []).map(o => ({ t: o.at, g: null, o }))].sort((a, b) => b.t - a.t);
  if (!items.length) return <p className="hp-note hp-pad">还没有结束的派发</p>;
  return (
    <div className="d-activity">
      {items.map(({ g, o }) => {
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
            <button type="button" className="hp-hist" key={`s${g.session}`} onClick={() => openSession({ id: g.session, known: g.list })} title="打开这个会话">
              <Mark id={p.agent} title={p.agentLabel} />
              <div>
                <b>
                  会话 · {g.list.length} 张{' '}
                  <span className="hp-cid">
                    {[...g.list]
                      .reverse()
                      .map(x => `#${x.card}`)
                      .join(' ')}
                  </span>
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
                  {p.model && <> · {[p.model, p.effort].filter(Boolean).join(' · ')}</>}
                  {first.startedAt != null && p.endedAt != null && <> · 用时 {durWords(p.endedAt - first.startedAt)}</>}
                  {p.endedAt != null && <> · {since(p.endedAt, now)}</>}
                </span>
              </div>
            </button>
          );
        }
        const r = resultOf(p);
        return (
          <button type="button" className="hp-hist" key={p.id} onClick={() => openSession({ id: p.session || p.id, card: p.card, known: [p] })} title="打开这个会话">
            <Mark id={p.agent} title={p.agentLabel} />
            <div>
              <b>
                <span className="hp-cid">#{p.card}</span> <Title text={p.title} />
              </b>
              <span>
                <em style={{ ['--c' as string]: r.color }}>{r.label}</em>
                {p.model && <> · {[p.model, p.effort].filter(Boolean).join(' · ')}</>}
                {p.startedAt != null && p.endedAt != null && <> · 用时 {durWords(p.endedAt - p.startedAt)}</>}
                {p.endedAt != null && <> · {since(p.endedAt, now)}</>}
              </span>
            </div>
          </button>
        );
      })}
    </div>
  );
}
