/* 小部件：头像、环形倒计时、标签、项目头像、Markdown、卡片标题、倒计时 hook */
import { useEffect, useMemo, useRef, useState } from 'react';
import { inlineCode, markdown } from '../lib/markdown';
import { PROJECT_COLORS } from '../lib/schema';
import type { Actor, Label } from '../lib/types';
import { agentColor, agentGlyph, cx, initial } from '../lib/util';
import { serverNow } from '../store/data';
import { openImage } from '../store/ui';

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

export function LabelChip({ label, onRemove, pending }: { label: Pick<Label, 'id' | 'name' | 'color'>; onRemove?: () => void; pending?: boolean }) {
  return (
    <span className={pending ? 'lb pending' : 'lb'} aria-busy={pending || undefined} title={pending ? '正在加上…' : undefined} style={{ ['--lc' as string]: label.color }} data-label={label.id}>
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

/** 卡片上的标签：按项目里标签的顺序排（加上新标签、服务端确认时位置都不跳），最多显示 max 个，其余折叠成 +n。
    pending：正在写入的标签，画成半透明 + 扫光 */
export function LabelChips({ ids, labels, max = 3, pending }: { ids: number[]; labels: Label[]; max?: number; pending?: number[] }) {
  const ls = labels.filter(l => ids.includes(l.id));
  if (!ls.length) return null;
  return (
    <>
      {ls.slice(0, max).map(l => (
        <LabelChip key={l.id} label={l} pending={pending?.includes(l.id)} />
      ))}
      {ls.length > max && (
        <span className="lb more" title={ls.slice(max).map(l => l.name).join('、')}>
          +{ls.length - max}
        </span>
      )}
    </>
  );
}

/** 项目颜色：一排色板，最后一个是取色器（可选任意颜色，选中后显示为当前颜色）。
    取色器在确定时（change）才回调，拖动过程中不回调 */
export function ColorRow({ value, onPick, className, label = '颜色' }: { value: string; onPick: (c: string) => void; className?: string; label?: string }) {
  const input = useRef<HTMLInputElement>(null);
  const pick = useRef(onPick);
  pick.current = onPick;
  const v = value.toUpperCase();
  const custom = !PROJECT_COLORS.includes(v);
  useEffect(() => {
    const el = input.current;
    if (!el) return;
    const done = () => pick.current(el.value.toUpperCase());
    el.addEventListener('change', done);
    return () => el.removeEventListener('change', done);
  }, []);
  return (
    <div className={cx('swatches', className)} role="radiogroup" aria-label={label}>
      {PROJECT_COLORS.map(c => (
        <button
          key={c}
          type="button"
          role="radio"
          aria-checked={c === v}
          className={cx('swatch', c === v && 'on')}
          style={{ ['--sw' as string]: c }}
          aria-label={`${label} ${c}`}
          onClick={() => onPick(c)}
        />
      ))}
      <label
        className={cx('swatch swatch-pick', custom && 'on')}
        style={custom ? { ['--sw' as string]: v } : undefined}
        title={custom ? `自定义颜色 ${v}` : '自定义颜色'}
      >
        <input ref={input} type="color" value={/^#[0-9A-F]{6}$/.test(v) ? v.toLowerCase() : '#888888'} onChange={() => {}} aria-label={`${label}：自定义`} />
      </label>
    </div>
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
  return (
    <div
      className={cx('md', className)}
      dangerouslySetInnerHTML={{ __html: html }}
      onClick={e => {
        // 图片（附件）点击放大
        const t = e.target as HTMLElement;
        if (t.tagName !== 'IMG') return;
        e.preventDefault();
        e.stopPropagation();
        openImage(t.getAttribute('src') || '', t.getAttribute('alt') || '');
      }}
    />
  );
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
