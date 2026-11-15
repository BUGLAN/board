/* ==========================================================================
   机器详情「设置」页最上面的「资源」：四块磁贴 CPU、内存、GPU、存储（PRD §10.12「资源」）
   - 大数字 + 最近 5 分钟的趋势线（服务端记最近 30 次采样）+ 一句细节
   - 第二块起的 GPU、工作空间以外的盘在磁贴下面各一行小条
   - 取不到的写「不支持」/「未上报」，不画 0；盘、内存到 90% 起警告色
   - 离线：画离线前最后一次（变淡）；旧版 board host：只写一行
   数据跟着机器列表刷新（机器详情开着时每 10 秒取一次）
   ========================================================================== */
import { resLine, sparkPath, tiles, type Tile } from '../../lib/hostRes';
import { since } from '../../lib/hosts';
import type { HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { Icon } from '../Icon';

export function Resources({ h, now, server, behind }: { h: HostInfo; now: number; server: string | null; behind: boolean }) {
  const r = h.res;
  const line = resLine(h, now, server, behind, since);
  return (
    <section className={cx('hd-res', r && !h.online && 'stale')}>
      <h3 className="hd-res-h">
        资源
        {r && <span>{line}</span>}
      </h3>
      {r ? (
        <div className="hd-tiles">
          {tiles(r).map(t => (
            <ResTile key={t.key} t={t} />
          ))}
        </div>
      ) : (
        <p className="hd-line hd-res-none">
          <Icon name="info" />
          {line}
        </p>
      )}
    </section>
  );
}

function ResTile({ t }: { t: Tile }) {
  return (
    <div className={cx('hd-tile', t.warn && 'warn', t.pct == null && 'none')}>
      <span className="hd-tile-k">{t.title}</span>
      <b className="hd-tile-v">
        {t.pct != null ? (
          <>
            {t.pct}
            <small>%</small>
          </>
        ) : (
          t.none
        )}
      </b>
      {t.key === 'disk' ? <Bar pct={t.pct} className="hd-tile-bar" /> : <Spark data={t.hist} warn={t.warn} />}
      <span className="hd-tile-sub">
        {t.disk && (
          <>
            <code>{t.disk.mount}</code>
            {t.disk.ws && <em>工作空间</em>}
          </>
        )}
        <span>{t.sub}</span>
        {t.warn && <span className="hd-tile-warn">快满了</span>}
      </span>
      {t.more.length > 0 && (
        <ul className="hd-tile-more">
          {t.more.map(m => (
            <li key={m.key} className={cx(m.warn && 'warn')} title={m.sub}>
              <code>{m.label}</code>
              <Bar pct={m.pct} />
              <b>{m.pct != null ? `${m.pct}%` : '—'}</b>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Bar({ pct, className }: { pct: number | null; className?: string }) {
  return (
    <span className={cx('hd-bar', className)} aria-hidden>
      {pct != null && <i style={{ width: `${Math.max(1.5, pct)}%` }} />}
    </span>
  );
}

function Spark({ data, warn }: { data: number[]; warn: boolean }) {
  const p = sparkPath(data);
  if (!p) return <span className="hd-spark" aria-hidden />;
  return (
    <svg className={cx('hd-spark', warn && 'warn')} viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden>
      <path className="area" d={p.area} />
      <path className="line" d={p.line} vectorEffect="non-scaling-stroke" />
    </svg>
  );
}
