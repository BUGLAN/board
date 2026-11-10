/* 机器 Tab：侧栏机器列表、主区机位总览 */
import { agentMark, cronSummary, envName, hostLoad, hostsOf, mins, since, skillMiss, SOURCE } from '../../lib/hosts';
import type { Dispatch, HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { useData } from '../../store/data';
import { openHost, useUI } from '../../store/ui';
import { useNow } from '../bits';
import { Icon } from '../Icon';
import { AgentChip, Beat, Dot, Mark, OsGlyph, StatePill, lastSeen, usePollHosts } from './bits';

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
                        <AgentChip key={a.id} a={a} def={a.id === def} env={envName(h, a.env)} skill={skillMiss(h, a)} small />
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
