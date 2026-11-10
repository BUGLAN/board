/* 小部件：头像、环形倒计时、标签、项目头像、Markdown、卡片标题、倒计时 hook */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { inlineCode, markdown } from '../lib/markdown';
import { PROJECT_COLORS } from '../lib/schema';
import type { Actor, Label } from '../lib/types';
import { agentColor, agentGlyph, cx, initial } from '../lib/util';
import { useAuth } from '../store/auth';
import { serverNow } from '../store/data';
import { openImage } from '../store/ui';
import { Icon } from './Icon';

/** 同一间隔的 useNow 共用一个定时器 */
const tickers = new Map<number, { subs: Set<() => void>; timer: number }>();
function tick(interval: number, fn: () => void) {
  let t = tickers.get(interval);
  if (!t) {
    const subs = new Set<() => void>();
    t = { subs, timer: window.setInterval(() => subs.forEach(f => f()), interval) };
    tickers.set(interval, t);
  }
  t.subs.add(fn);
  return () => {
    t.subs.delete(fn);
    if (!t.subs.size) {
      clearInterval(t.timer);
      tickers.delete(interval);
    }
  };
}
/** 现在有几个定时器、几个订阅（调试、测试用） */
export const tickerStats = () => ({ timers: tickers.size, subs: [...tickers.values()].reduce((n, t) => n + t.subs.size, 0) });

/** 服务端对齐的当前时间，每 interval 毫秒更新一次；interval 为 null 时不更新（用不到时间的组件不起定时器） */
export function useNow(interval: number | null = 1000) {
  const [now, setNow] = useState(serverNow);
  useEffect(() => {
    if (!interval) return;
    // 从不更新变成更新（例如卡片刚被领取）：先对一次表，不等第一次跳动
    setNow(prev => {
      const n = serverNow();
      return n - prev >= interval ? n : prev;
    });
    return tick(interval, () => setNow(serverNow()));
  }, [interval]);
  return now;
}

/** 界面上怎么称呼「你」：线上用 GitHub 昵称（没有昵称时用登录名）；本机模式没有名字 */
export function useMeName(): string | null {
  const me = useAuth(s => s.me);
  if (me?.mode !== 'github') return null;
  return (me.name || '').trim() || me.login;
}

/** 头像：登录后「人」用 GitHub 头像；取不到地址或加载失败时退回到灰圈首字母（本机模式仍是「你」），不出现破图 */
export function Avatar({ actor, className }: { actor: Actor; className?: string }) {
  const me = useAuth(s => s.me);
  // 加载失败的头像地址：换了账号（地址变了）会自动重试
  const [badSrc, setBadSrc] = useState<string | null>(null);
  const who = actor.kind === 'human' && me?.mode === 'github' ? me : null;
  const name = who ? (who.name || '').trim() || who.login : '';
  const src = who?.avatar && who.avatar !== badSrc ? who.avatar : '';
  if (actor.kind === 'human')
    return (
      <span className={cx('avatar human', src && 'photo', className)} title={name ? `${name}（你）` : '你（界面）'}>
        {src ? <img src={src} alt="" draggable={false} onError={() => setBadSrc(src)} /> : name ? initial(name) : '你'}
      </span>
    );
  if (actor.kind === 'system')
    return (
      <span className={cx('avatar', className)} style={{ ['--c' as string]: 'var(--text-3)', ['--avatar-fg' as string]: 'var(--surface)' }} title="系统">
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

/** 仅人可打的标签在名称前带一把小锁 */
export const LabelLock = ({ label }: { label: Pick<Label, 'humanOnly'> }) =>
  label.humanOnly ? (
    <span className="lb-lock" aria-label="仅人可打">
      <Icon name="lock" sw={2} />
    </span>
  ) : null;

export function LabelChip({
  label,
  onRemove,
  pending,
}: {
  label: Pick<Label, 'id' | 'name' | 'color' | 'humanOnly'>;
  onRemove?: () => void;
  pending?: boolean;
}) {
  return (
    <span
      className={pending ? 'lb pending' : 'lb'}
      aria-busy={pending || undefined}
      title={pending ? '正在加上…' : label.humanOnly ? '仅人可打：agent 不能加、去这个标签' : undefined}
      style={{ ['--lc' as string]: label.color }}
      data-label={label.id}
    >
      <LabelLock label={label} />
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
        // 图片（附件）点击放大；同一段里的其他图片可以左右切换
        const t = e.target as HTMLElement;
        if (t.tagName !== 'IMG') return;
        e.preventDefault();
        e.stopPropagation();
        const list = [...e.currentTarget.querySelectorAll('img')].map(i => ({ src: i.getAttribute('src') || '', alt: i.getAttribute('alt') || '' }));
        openImage(t.getAttribute('src') || '', t.getAttribute('alt') || '', list);
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

/** 描述的「预览 / 编辑」切换（卡片详情、新建卡片共用） */
export function DescTabs({ mode, setMode, modes }: { mode: string; setMode: (m: string) => void; modes: [string, string][] }) {
  const ref = useRef<HTMLDivElement>(null);
  const [thumb, setThumb] = useState({ w: 0, x: 0 });
  useLayoutEffect(() => {
    const on = ref.current?.querySelector<HTMLElement>('button.on');
    if (on) setThumb({ w: on.offsetWidth, x: on.offsetLeft - 2 });
  }, [mode]);
  return (
    <div className="act desc-tabs" ref={ref}>
      <span className="thumb" style={{ width: thumb.w, transform: `translateX(${thumb.x}px)` }} />
      {modes.map(([k, label]) => (
        <button key={k} type="button" className={cx(mode === k && 'on')} onClick={() => setMode(k)}>
          {label}
        </button>
      ))}
    </div>
  );
}
