/* 机器详情「概况」里的一行：正在跑的（会话里排着的卡挂在它下面）、排队的 */
import { queueReason, sessionQueue, sourceOf } from '../../lib/hosts';
import type { Dispatch, HostInfo } from '../../lib/types';
import { durWords } from '../../lib/util';
import { useData } from '../../store/data';
import { openSession } from '../../store/ui';
import { Title } from '../bits';
import { Icon } from '../Icon';
import { cancel, goCard } from './Dispatch';
import { Beat, Mark } from './bits';

export function RunRow({ r, now }: { r: Dispatch; now: number }) {
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
        <span>{sourceOf(r)}</span>
        {r.stopBy && <span className="hp-stopping">正在停止</span>}
        <span className="hp-sp" />
        <button className="hp-link" type="button" onClick={() => openSession({ id: r.session || r.id, card: r.card, known: [r] })}>
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

export function QueueRow({ r, h, now }: { r: Dispatch; h: HostInfo; now: number }) {
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
        <span>{sourceOf(r)}</span>
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

