/* 小部件：头像、环形倒计时、标签、项目头像、Markdown、卡片标题、倒计时 hook */
import { useEffect, useMemo, useState } from 'react';
import { inlineCode, markdown } from '../lib/markdown';
import type { Actor, Label } from '../lib/types';
import { agentColor, agentGlyph, cx, initial } from '../lib/util';
import { serverNow } from '../store/data';

/** 当前时间（与服务端对齐），每 interval 毫秒刷新 */
export function useNow(interval = 1000) {
  const [now, setNow] = useState(serverNow);
  useEffect(() => {
    const t = window.setInterval(() => setNow(serverNow()), interval);
    return () => clearInterval(t);
  }, [interval]);
  return now;
}

export function Avatar({ actor, className }: { actor: Actor; className?: string }) {
  if (actor.kind === 'human')
    return (
      <span className={cx('avatar human', className)} title="你（界面）">
        你
      </span>
    );
  if (actor.kind === 'system')
    return (
      <span className={cx('avatar', className)} style={{ ['--c' as string]: 'var(--text-3)' }} title="系统">
        ⏱
      </span>
    );
  return (
    <span className={cx('avatar', className)} style={{ ['--c' as string]: agentColor(actor.name) }} title={actor.name}>
      {agentGlyph(actor.name)}
    </span>
  );
}

export const agent = (name: string): Actor => ({ kind: 'agent', name });

/** 环形进度：p ∈ [0, 1] */
export function Ring({ r, p, className }: { r: number; p: number; className?: string }) {
  const c = 2 * Math.PI * r;
  const size = (r + 3) * 2;
  const off = c * (1 - Math.max(0, Math.min(1, p)));
  return (
    <svg viewBox={`0 0 ${size} ${size}`} className={className}>
      <circle className="track" cx={size / 2} cy={size / 2} r={r} />
      <circle className="bar" cx={size / 2} cy={size / 2} r={r} strokeDasharray={c.toFixed(2)} strokeDashoffset={off.toFixed(2)} />
    </svg>
  );
}

export function LabelChip({ label, onRemove }: { label: Pick<Label, 'id' | 'name' | 'color'>; onRemove?: () => void }) {
  return (
    <span className="lb" style={{ ['--lc' as string]: label.color }} data-label={label.id}>
      <i />
      {label.name}
      {onRemove && (
        <button
          type="button"
          className="lb-x"
          aria-label={`移除标签 ${label.name}`}
          onClick={e => {
            e.stopPropagation();
            onRemove();
          }}
        >
          ×
        </button>
      )}
    </span>
  );
}

/** 卡片上的标签：最多显示 max 个，其余折叠成 +n */
export function LabelChips({ ids, labels, max = 3 }: { ids: number[]; labels: Label[]; max?: number }) {
  const ls = ids.map(id => labels.find(l => l.id === id)).filter(Boolean) as Label[];
  if (!ls.length) return null;
  return (
    <>
      {ls.slice(0, max).map(l => (
        <LabelChip key={l.id} label={l} />
      ))}
      {ls.length > max && (
        <span className="lb more" title={ls.slice(max).map(l => l.name).join('、')}>
          +{ls.length - max}
        </span>
      )}
    </>
  );
}

export function ProjectAvatar({ name, color, className }: { name: string; color: string; className?: string }) {
  return (
    <span className={cx('pj-av', className)} style={{ ['--pc' as string]: color }}>
      {initial(name)}
    </span>
  );
}

export function Markdown({ src, className }: { src: string; className?: string }) {
  const html = useMemo(() => markdown(src), [src]);
  return <div className={cx('md', className)} dangerouslySetInnerHTML={{ __html: html }} />;
}

/** 标题里的 `code` */
export function Title({ text }: { text: string }) {
  return (
    <>
      {inlineCode(text).map((p, i) => (typeof p === 'string' ? <span key={i}>{p}</span> : <code key={i}>{p.code}</code>))}
    </>
  );
}

export function Kbd({ children }: { children: string }) {
  return <kbd>{children}</kbd>;
}
