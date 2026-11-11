/* 小部件：头像、环形倒计时、标签、项目头像、Markdown、卡片标题、倒计时 hook */
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { inlineCode, markdown } from '../lib/markdown';
import { PROJECT_COLORS } from '../lib/schema';
import type { Actor, Label } from '../lib/types';
import { agentColor, agentGlyph, cx, initial, reduceMotion } from '../lib/util';
import { useAuth } from '../store/auth';
import { serverNow } from '../store/data';
import { closeCtx, openCtx, openImage, toastError, useUI } from '../store/ui';
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

/** 系统身份在界面上的称呼：动态、浮层、toast 里都显示成 board 本身（服务端 Actor::display() 用同一个词） */
export const SYSTEM_NAME = 'board';

/** 四根柱子的高度比例：顶栏、启动页、登录页和系统身份的头像都用它，换个尺寸还是同一组柱子 */
const BRAND_BARS = ['100%', '70%', '46%', '82%'];

/** Board 的品牌图标：四根柱子一块牌子。`.logo` 是带底色的那块（顶栏、启动页、登录页）；
    `.avatar.brand-mark` 沿用头像的圆形底，只放里面的柱子 */
export function BrandMark({ className }: { className?: string }) {
  return (
    <span className={cx(className)} aria-hidden="true">
      {BRAND_BARS.map(h => (
        <i key={h} style={{ ['--h' as string]: h }} />
      ))}
    </span>
  );
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
  // 系统身份就是 board 本身：头像换成品牌图标，底色用面板色，深色、浅色主题下柱子都看得清
  if (actor.kind === 'system')
    return (
      <span className={cx('avatar brand-mark', className)} title={SYSTEM_NAME}>
        <BrandMark className="brand-bars" />
      </span>
    );
  return (
    <span className={cx('avatar', className)} style={{ ['--c' as string]: agentColor(actor.name) }} title={actor.name}>
      {agentGlyph(actor.name)}
    </span>
  );
}

export const agent = (name: string): Actor => ({ kind: 'agent', name });

/** 仅人可打的标签在名称前带一把小锁 */
export const LabelLock = ({ label }: { label: Pick<Label, 'humanOnly'> }) =>
  label.humanOnly ? (
    <span className="lb-lock" aria-label="仅人可打">
      <Icon name="lock" sw={2} />
    </span>
  ) : null;

/** 优先标签在名称前带一个闪电：带这个标签的卡片先被领取 */
export const LabelPriority = ({ label }: { label: Pick<Label, 'priority'> }) =>
  label.priority ? (
    <span className="lb-prio" aria-label="优先">
      <Icon name="bolt" sw={2} />
    </span>
  ) : null;

/** 标签胶囊的悬停提示：优先、仅人可打 */
const labelTip = (label: Pick<Label, 'humanOnly' | 'priority'>) =>
  [label.priority && '优先：带这个标签的卡片先被领取', label.humanOnly && '仅人可打：agent 不能加、去这个标签'].filter(Boolean).join('\n') || undefined;

export function LabelChip({
  label,
  onRemove,
  removeDisabled,
  pending,
}: {
  label: Pick<Label, 'id' | 'name' | 'color' | 'humanOnly' | 'priority'>;
  onRemove?: () => void;
  /** × 还显示，但不能点（例如卡片正在保存） */
  removeDisabled?: boolean;
  /** 正在加上或去掉：半透明 + 呼吸 + 扫光；× 不能点 */
  pending?: boolean;
}) {
  const ref = useRef<HTMLSpanElement>(null);
  const was = useRef(pending);
  // pending 结束（服务端确认）：从半透明平滑过渡到正常，不是一下跳亮（动画撤掉时 transition 不会接着当前透明度走）
  useLayoutEffect(() => {
    if (was.current && !pending && !reduceMotion()) ref.current?.animate([{ opacity: 0.55 }, { opacity: 1 }], { duration: 260, easing: 'ease-out' });
    was.current = pending;
  }, [pending]);
  return (
    <span
      ref={ref}
      className={pending ? 'lb pending' : 'lb'}
      aria-busy={pending || undefined}
      title={pending ? '正在加上…' : labelTip(label)}
      style={{ ['--lc' as string]: label.color }}
      data-label={label.id}
    >
      <LabelPriority label={label} />
      <LabelLock label={label} />
      <span className="lb-name">{label.name}</span>
      {onRemove && (
        <button
          type="button"
          className="lb-x"
          aria-label={`移除标签 ${label.name}`}
          disabled={pending || removeDisabled}
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

/** 卡片上的标签：按项目里标签的顺序排（加上新标签、服务端确认时位置都不跳），默认全部显示（给了 max 时多出的折叠成 +n）。
    pending：正在写入的标签，画成半透明 + 扫光 */
export function LabelChips({ ids, labels, max = Infinity, pending }: { ids: number[]; labels: Label[]; max?: number; pending?: number[] }) {
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
  const v = value.toUpperCase();
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
      <ColorPick value={v} custom={!PROJECT_COLORS.includes(v)} onPick={onPick} label={label} />
    </div>
  );
}

/** 色板最后的取色器：打开系统取色器选任意颜色（#RRGGBB，大写）。custom：当前颜色不在色板里，取色器显示成这个颜色并标为选中。
    确定时（change）才回调，拖动过程中不回调。
    input 不受控：受控的话，拖动时每个 input 事件 React 都会把值还原成 value，确定时读到的还是旧颜色。value 变了由 effect 写回去 */
export function ColorPick({ value, custom, onPick, label = '颜色' }: { value: string; custom: boolean; onPick: (c: string) => void; label?: string }) {
  const input = useRef<HTMLInputElement>(null);
  const pick = useRef(onPick);
  pick.current = onPick;
  const v = value.toUpperCase();
  const hex = /^#[0-9A-F]{6}$/.test(v) ? v.toLowerCase() : '#888888';
  useEffect(() => {
    const el = input.current;
    if (!el) return;
    const done = () => pick.current(el.value.toUpperCase());
    el.addEventListener('change', done);
    return () => el.removeEventListener('change', done);
  }, []);
  useEffect(() => {
    if (input.current) input.current.value = hex;
  }, [hex]);
  return (
    <label
      className={cx('swatch swatch-pick', custom && 'on')}
      style={custom ? { ['--sw' as string]: v } : undefined}
      title={custom ? `自定义颜色 ${v}` : '自定义颜色'}
    >
      <input ref={input} type="color" defaultValue={hex} aria-label={`${label}：自定义`} />
    </label>
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

/* ---------------- 选择框 ---------------- */

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

/** 选择框：代替系统的 <select>。系统下拉的箭头和弹出的列表由各平台自己画（macOS、Windows、Linux 都不一样）；
    这里按钮是 .input 的样子加项目自己的箭头图标，点开的列表是项目的弹出菜单（overlays/CtxMenu 的 select，和「移动到」「排序」同一套）。
    键盘：回车 / 空格 / 上下键打开；列表里上下键、Home / End、按字符跳转，回车选中，Esc 关闭 */
export function Select({
  value, options, onChange, className, disabled, placeholder, ...rest
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  className?: string;
  disabled?: boolean;
  /** 当前值不在选项里时显示 */
  placeholder?: string;
  /** 没有可见的标签文字时必须给（外面包着 <label> 时可以不给） */
  'aria-label'?: string;
  'data-tip'?: string;
  name?: string;
}) {
  const id = useId();
  const ref = useRef<HTMLButtonElement>(null);
  const open = useUI(s => s.ctx?.kind === 'select' && s.ctx.id === id);
  // 选择框不在了（弹窗关闭、这一行被删）：列表跟着关
  useEffect(
    () => () => {
      const c = useUI.getState().ctx;
      if (c?.kind === 'select' && c.id === id) closeCtx();
    },
    [id],
  );
  const show = () => {
    const el = ref.current;
    if (!el) return;
    openCtx({
      kind: 'select', id, x: 0, y: 0, anchor: el.getBoundingClientRect(), items: options, value,
      pick: v => {
        if (v !== value) onChange(v);
        el.focus({ preventScroll: true });
      },
    });
  };
  return (
    <button
      {...rest}
      ref={ref}
      type="button"
      className={cx('input sel', open && 'open', className)}
      role="combobox"
      aria-haspopup="listbox"
      aria-expanded={open}
      disabled={disabled}
      onClick={() => (open ? closeCtx() : show())}
      onKeyDown={e => {
        if (!open && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
          e.preventDefault();
          e.stopPropagation();
          show();
        }
      }}
    >
      <span className="sel-v">{options.find(o => o.value === value)?.label ?? placeholder ?? ''}</span>
      <Icon name="chevDown" />
    </button>
  );
}

/* ------------------------------ 设置项的保存状态 ------------------------------ */

export type SyncState = 'saving' | 'saved' | 'failed';
/** 「已保存」「没保存上」的标记停留多久 */
const SAVED_MS = 1600;
const FAILED_MS = 1900;

/** 设置项各存各的（store 的 saveSetting）时，每一项的保存状态：track(这一项, 保存的 Promise)。
    保存中（转圈，晚一点才出现）→ 已保存（打勾后淡出）；失败时提示并画「没保存上」。
    连点合并成的同一批（同一个 Promise）只处理一次；这一项后面又改过时，只认最后那次的结果 */
export function useSync() {
  const [sync, setSync] = useState<Record<string, SyncState | undefined>>({});
  const last = useRef(new Map<string, Promise<unknown>>());
  const timers = useRef(new Map<string, number>());
  useEffect(() => () => timers.current.forEach(t => clearTimeout(t)), []);
  const mark = (f: string, st?: SyncState) => setSync(x => (x[f] === st ? x : { ...x, [f]: st }));
  const track = (f: string, done: Promise<unknown>) => {
    clearTimeout(timers.current.get(f));
    mark(f, 'saving');
    if (last.current.get(f) === done) return;
    last.current.set(f, done);
    const end = (st: SyncState, ms: number) => {
      if (last.current.get(f) !== done) return;
      mark(f, st);
      timers.current.set(f, window.setTimeout(() => mark(f), ms));
    };
    done.then(
      () => end('saved', SAVED_MS),
      e => {
        toastError(e);
        end('failed', FAILED_MS);
      },
    );
  };
  return { sync, track };
}

const SYNC_TEXT: Record<SyncState, string> = { saving: '正在保存', saved: '已保存', failed: '没保存上' };

/** 一个设置项的保存状态（.hp-sync）：默认贴在控件左边（不占位，状态变化时布局不动）；inline 时跟在文字后面 */
export function Sync({ s, inline }: { s?: SyncState; inline?: boolean }) {
  return (
    <span className={cx('hp-sync', inline && 'in')} data-s={s ?? ''} role="status">
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <circle className="ring" cx="8" cy="8" r="6" />
        <path className="tick" d="M4.8 8.3l2.2 2.2 4.3-4.8" />
        <path className="bang" d="M8 4.6v4.1M8 11.2v.3" />
      </svg>
      {s && <span className="sr-only">{SYNC_TEXT[s]}</span>}
    </span>
  );
}
