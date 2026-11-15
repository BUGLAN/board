/* ==========================================================================
   共用配置各节共用的小部件（PRD §10.12「共用配置」）
   - 机器胶囊：一台机器一个（状态标记、机器名、状态），悬停看原因，点了打开那台机器
   - 「怎么生效」：把长说明收进悬停；版本太旧、不同步的机器一句话说完
   ========================================================================== */
import type { HostSync } from '../../lib/syncPills';
import type { HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { openHost } from '../../store/ui';
import { Icon } from '../Icon';
import { Tip } from '../bits';

export function HostSyncPill({ s }: { s: HostSync }) {
  return (
    <Tip text={s.tip}>
      <button type="button" className={cx('pv-cell', `k-${s.kind === 'off' ? 'local' : s.kind}`)} onClick={() => openHost(s.host.name)}>
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

/** 版本太旧、不按请求同步的机器一句话：「mbp（0.1.21）的 board 版本太旧，不同步 skills：升级后自动同步」 */
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
