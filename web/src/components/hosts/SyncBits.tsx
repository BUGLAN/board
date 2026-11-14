/* ==========================================================================
   共用配置各节共用的小部件（PRD §10.12「共用配置」）
   - 机器胶囊：一台机器一个（状态标记、机器名、合成的状态），悬停列出每个 agent，点了打开那台机器
   - 矩阵：展开一项时看「机器 × agent」，一格一个状态，悬停看原因
   - 「怎么生效」：把长说明收进悬停；版本太旧、不同步的机器一句话说完
   ========================================================================== */
import type { HostSync } from '../../lib/syncPills';
import { syncAgentName } from '../../lib/syncPills';
import type { HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { openHost } from '../../store/ui';
import { Icon } from '../Icon';
import { Tip } from '../bits';
import { AgentIcon } from './bits';

export function HostSyncPill({ s }: { s: HostSync }) {
  return (
    <Tip text={s.tip}>
      <button type="button" className={cx('pv-cell', `k-${s.kind}`)} onClick={() => openHost(s.host.name)}>
        <i className="pv-cell-mark" aria-hidden="true" />
        <b>{s.host.name}</b>
        <span>{s.word}</span>
      </button>
    </Tip>
  );
}

export function HostSyncPills({ list }: { list: HostSync[] }) {
  if (!list.length) return <em className="sy-none">还没有机器</em>;
  return (
    <span className="sy-pills">
      {list.map(s => (
        <HostSyncPill key={s.host.id} s={s} />
      ))}
    </span>
  );
}

/** 一行一台机器、一列一个 agent */
export function SyncMatrix({ list }: { list: HostSync[] }) {
  const agents = [...new Set(list.flatMap(s => s.cells.map(c => c.agent)))];
  if (!list.length) return <p className="hp-note">还没有机器：加了机器之后它会自动同步</p>;
  if (!agents.length) return <p className="hp-note">这几台机器上都没有要同步的 agent</p>;
  return (
    <table className="sy-mx">
      <thead>
        <tr>
          <th />
          {agents.map(a => (
            <th key={a}>
              <AgentIcon id={a} />
              {syncAgentName(list.find(s => s.host.agents.some(x => x.id === a))?.host ?? null, a)}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {list.map(s => (
          <tr key={s.host.id} className={cx(!s.host.online && 'off')}>
            <th>
              <button type="button" className="sy-mx-host" onClick={() => openHost(s.host.name)}>
                <i className={cx('sy-dot', s.host.online && 'on')} />
                {s.host.name}
                {!s.host.online && <em>离线</em>}
              </button>
            </th>
            {agents.map(a => {
              const c = s.cells.find(x => x.agent === a);
              return (
                <Tip key={a} text={c?.why ?? ''}>
                  <td className={cx(c && `s-${c.state}`)}>{c ? c.word : '—'}</td>
                </Tip>
              );
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function HowTip({ tip }: { tip: string }) {
  return (
    <Tip text={tip}>
      <span className="sc-how" tabIndex={0}>
        <Icon name="info" />
        怎么生效
      </span>
    </Tip>
  );
}

/** 版本太旧、不同步的机器一句话：「mbp（0.1.21）的 board 版本太旧，不同步 skills：升级后自动同步」 */
export function OldHostsNote({ hosts, what, verb = '同步' }: { hosts: HostInfo[]; what: string; verb?: string }) {
  if (!hosts.length) return null;
  return (
    <p className="pv-old">
      <Icon name="alert" />
      <span>
        {hosts.map((h, i) => (
          <span key={h.id}>
            {i > 0 && '、'}
            <button type="button" className="pv-link" onClick={() => openHost(h.name)}>
              {h.name}
            </button>
            <em>（{h.version || '?'}）</em>
          </span>
        ))}
        的 board 版本太旧，不{verb}{what}：升级后自动{verb}
      </span>
    </p>
  );
}
