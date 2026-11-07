/* 卡片：白板状态块与侧栏「未整理」共用 */
import { memo, useLayoutEffect, useRef } from 'react';
import { BEAT_MISSES, BEAT_MS, timing } from '../lib/schema';
import type { Card as CardT, Label } from '../lib/types';
import { agentColor, ago, clock, cx, durWords, replay } from '../lib/util';
import { useUI, type Flash } from '../store/ui';
import { Avatar, LabelChips, Title, agent, useNow } from './bits';
import { Icon } from './Icon';

/** 筛选框：空格分隔的词都要命中；label:名称 只看标签 */
export function matches(c: CardT, q: string, labels: Label[]) {
  if (!q) return true;
  const names = c.labels.map(id => (labels.find(l => l.id === id)?.name || '').toLowerCase());
  const hay = `#${c.id}\n${c.title}\n${c.body}\n${c.holder || ''}\n${names.join('\n')}`.toLowerCase();
  return q.split(/\s+/).every(w => {
    const m = w.match(/^(?:label|标签)[:：](.+)$/);
    return m ? names.includes(m[1]) : hay.includes(w);
  });
}

/** 筛选框 + 侧栏选中的标签（带其中任一个） */
export const filterActive = (q: string, sel: number[]) => !!q || sel.length > 0;
export function cardHit(c: CardT, q: string, sel: number[], labels: Label[]) {
  return matches(c, q, labels) && (sel.length === 0 || c.labels.some(l => sel.includes(l)));
}

interface Props {
  card: CardT;
  labels: Label[];
  filter: string;
  open: boolean;
  placeholder: boolean;
  flash?: Flash;
  entering: boolean;
  delay?: number;
  /** 所在状态的颜色（CSS 颜色） */
  color?: string;
  /** 所在状态是终点：只占一行，标题划掉 */
  final?: boolean;
  /** 领取起点的第一张 */
  next?: boolean;
  /** 「整理到…」按钮上的领取起点名称 */
  claimName?: string;
  onTriage?: (id: number) => void;
}

export const CardView = memo(function CardView({
  card: c, labels, filter, open, placeholder, flash, entering, delay, color, final, next, claimName = '待办', onTriage,
}: Props) {
  const held = !!c.holder;
  const now = useNow(held ? 1000 : 30000);
  const ref = useRef<HTMLElement>(null);
  const sel = useUI(s => s.labelSel);
  const active = filterActive(filter, sel);
  const hit = active ? cardHit(c, filter, sel, labels) : true;
  const t = timing(c, now);
  const doing = held;
  const gh = (c.body.match(/gh#\d+/) || [])[0];

  useLayoutEffect(() => {
    if (flash) replay(ref.current, 'flash');
  }, [flash?.key]); // eslint-disable-line react-hooks/exhaustive-deps

  const style: Record<string, string> = {};
  if (color) style['--c'] = color;
  if (doing) style['--ac'] = agentColor(c.holder!);
  if (flash) style['--flash'] = flash.color;
  if (delay != null) style.animation = `card-in var(--dur-4) var(--spring-soft) both ${delay}ms`;

  return (
    <article
      ref={ref}
      className={cx(
        'card',
        c.column === 'inbox' && 'inbox',
        doing && 'doing',
        final && !doing && 'done',
        next && 'next',
        open && 'is-open',
        active && !hit && 'filtered-out',
        active && hit && 'matched',
        t.beat === 'late' && 'late',
        entering && 'entering',
        placeholder && 'is-placeholder',
      )}
      tabIndex={0}
      data-id={c.id}
      data-flip={`c${c.id}`}
      aria-label={`#${c.id} ${c.title}`}
      style={style}
    >
      {c.column === 'inbox' ? (
        <>
          <div className="card-row">
            <span className="card-id">#{c.id}</span>
            <span className="card-flags">
              <span className="card-age">{ago(c.createdAt, now)}</span>
            </span>
          </div>
          <div className="card-title">
            <Title text={c.title} />
          </div>
          {snippet(c.body) && <div className="card-snippet">{snippet(c.body)}</div>}
          {c.labels.length > 0 && (
            <div className="card-labels">
              <LabelChips ids={c.labels} labels={labels} />
            </div>
          )}
          <button
            className="card-act"
            type="button"
            title={`整理到「${claimName}」最下面`}
            onClick={e => {
              e.stopPropagation();
              onTriage?.(c.id);
            }}
          >
            整理到{claimName}
            <Icon name="move" />
          </button>
        </>
      ) : final && !doing ? (
        <div className="card-line">
          <span className="done-check">
            <Icon name="check" />
          </span>
          <span className="card-id">#{c.id}</span>
          <span className="card-line-title">{c.title}</span>
        </div>
      ) : (
        <>
          <div className="card-row">
            <span className="card-id">#{c.id}</span>
            {next && <span className="card-next">下一张</span>}
            <span className="card-flags">
              {gh && <span className="card-tag">{gh}</span>}
              {c.comments > 0 && (
                <span className="f" title={`${c.comments} 条评论`}>
                  <Icon name="comment" />
                  {c.comments}
                </span>
              )}
            </span>
          </div>
          <div className="card-title">
            <Title text={c.title} />
          </div>
          {c.labels.length > 0 && (
            <div className="card-labels">
              <LabelChips ids={c.labels} labels={labels} />
            </div>
          )}
          {doing ? (
            <div className="card-claim">
              <Avatar actor={agent(c.holder!)} />
              <span className="claim-name">{c.holder}</span>
              <RunPill card={c} now={now} />
              {t.runs > 1 && (
                <span className="claim-sub">
                  累计 {clock(t.total)} · 第 {t.runs} 次
                </span>
              )}
            </div>
          ) : (
            t.runs > 0 && (
              <div className="card-claim worked">
                <span className="run-pill" title={`做过 ${t.runs} 次${t.lost ? `，其中 ${t.lost} 次异常退出` : ''}`}>
                  <Icon name="timer" />
                  累计 {durWords(t.total)}
                </span>
                <span className="claim-sub">{t.runs} 次</span>
              </div>
            )
          )}
        </>
      )}
      {flash && (
        <span className="card-badge" key={flash.key}>
          {flash.label}
        </span>
      )}
    </article>
  );
});

function snippet(body: string) {
  return (
    body
      .split('\n')
      // 图片、链接只留文字（附件显示为文件名）
      .map(l => l.replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1').replace(/^[-*#>\s]+|\[[ xX]\]\s*/g, '').trim())
      .find(Boolean) || ''
  );
}

/** 进行中的计时胶囊：跳动的心跳点 + 这一次的耗时。卡片、详情共用 */
export function RunPill({ card: c, now, big }: { card: CardT; now: number; big?: boolean }) {
  const t = timing(c, now);
  const tip =
    t.beat === 'live'
      ? `这一次的耗时（从第一次心跳算起）· 心跳正常，${t.ageSec} 秒前`
      : t.beat === 'late'
        ? `已 ${t.missed} 次没收到心跳，连续 ${BEAT_MISSES} 次就释放`
        : t.beat === 'wait'
          ? `领取了，还没收到第一次心跳；${BEAT_MS * BEAT_MISSES / 1000} 秒内没收到就释放`
          : '已发 final，等待交审';
  return (
    <span className={cx('run-pill live', t.beat, big && 'big')} data-tip={tip} data-tip-align="end">
      <i className="beat" />
      {t.cur != null ? clock(t.cur) : t.beat === 'wait' ? '等待心跳' : '已结束'}
    </span>
  );
}
