/* 小部件：头像、环形倒计时、标签、项目头像、Markdown、卡片标题、倒计时 hook */
import { cloneElement, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactElement, type ReactNode, type Ref } from 'react';
import { createPortal } from 'react-dom';
import { openLink } from '../lib/actions';
import { LOOP_STAGGER_MS, loopAgentMotion, playAgentMotion, subscribeMotion } from '../lib/agent-motion';
import { agentIdFor, agentMark, type AgentMark } from '../lib/hosts';
import { inlineCode, markdown } from '../lib/markdown';
import { shell } from '../lib/platform';
import { refsFor, refsVersion, subscribeRefs } from '../lib/xref';
import { PROJECT_COLORS } from '../lib/schema';
import { placeTip } from '../lib/tooltip';
import { staleText } from '../lib/stale';
import type { Actor, Label } from '../lib/types';
import { agentColor, agentGlyph, cx, initial, reduceMotion } from '../lib/util';
import { useAuth } from '../store/auth';
import { serverNow, useData } from '../store/data';
import { closeCtx, openCtx, openImage, openTextFile, toastError, useUI } from '../store/ui';
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
      <span className={cx('avatar brand-mark', className)} title={actor.name || SYSTEM_NAME}>
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

/**
 * agent 的 logo 图本身（透明底）：单色的（`mono`）用遮罩画，颜色是 --agent-ink，深浅主题都看得清；其余直接画图。
 * 头像（AgentAvatar）和机器页里的 agent 图标（hosts/bits.tsx）共用，没有 logo 时什么都不画
 */
export function LogoArt({ mark, className, title }: { mark: AgentMark; className?: string; title?: string }) {
  if (!mark.icon) return null;
  if (mark.mono)
    return (
      <i
        className={cx(className, 'logo-mono')}
        role={title ? 'img' : undefined}
        aria-label={title}
        title={title}
        style={{ maskImage: `url(${mark.icon})`, WebkitMaskImage: `url(${mark.icon})` }}
      />
    );
  return <img className={className} src={mark.icon} alt={title ?? ''} title={title} draggable={false} />;
}

/**
 * 按 agent 标识画头像（服务端带的 `agentId`；旧数据没有时按名字的旧格式 `<标识>-<机器名>/<模型名>` 解析）：
 * 认得出标识就在位子上画 agent 自己的 logo，认不出退回现在的字母头像（`Avatar`），名字为空时什么都不画。
 *
 * logo 的规范（default/main#357，样式在 base.css 的 `.avatar.agent-logo`）：
 * - 画 logo 本身、透明底，不套圆底、不描边、不投影；位子大小沿用各处 `.avatar` 的尺寸（顶栏 26、卡片 20、单行 18、磁贴 16、详情 38），
 *   logo 按 `MARKS.scale`（--logo-k）缩放；单色 logo（`mono`）用遮罩画，颜色跟主题走
 * - 动作是各自的签名动作（lib/agent-motion.ts）：hover 时做一次；`enter` 的位子出现、换人时用进场版；
 *   `loop` 时（在线干活）每 5 秒做一次，`loopIndex` 是并排时的序号，用来错开。减弱动效时都不做
 */
export function AgentAvatar({
  name,
  agentId,
  className,
  enter,
  loop,
  loopIndex = 0,
}: {
  name: string | null | undefined;
  /** 服务端带的 agent 标识；不传时按名字的旧格式解析 */
  agentId?: string | null;
  className?: string;
  enter?: boolean;
  loop?: boolean;
  loopIndex?: number;
}) {
  const id = agentIdFor(name, agentId);
  const tip = name ?? '';
  const m = id ? agentMark(id) : null;
  const ref = useRef<HTMLSpanElement>(null);
  const art = () => ref.current?.querySelector('.logo-art');
  // 减弱动效的开关变了，在循环的要停下或重新开始
  const still = useSyncExternalStore(subscribeMotion, reduceMotion, reduceMotion);
  // 出现、换人（标识变了，位子按标识重新挂上）：进场版
  useLayoutEffect(() => {
    if (enter) playAgentMotion(art(), m?.motion, 'enter');
  }, [id]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!loop || still) return;
    const a = loopAgentMotion(art(), m?.motion, loopIndex * LOOP_STAGGER_MS);
    return () => a?.cancel();
  }, [id, loop, still, loopIndex]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!id || !m) return name ? <Avatar actor={agent(name)} className={className} /> : null;
  return m.icon ? (
    <span
      key={id}
      ref={ref}
      className={cx('avatar', 'agent-logo', className)}
      style={{ ['--c' as string]: m.color, ['--logo-k' as string]: m.scale ?? 1 }}
      title={tip}
      onMouseEnter={() => playAgentMotion(art(), m.motion, 'act')}
    >
      <LogoArt mark={m} className="logo-art" />
    </span>
  ) : (
    // 认识的 agent 还没有自己的图标（旧版 `MARKS`）：两个字母
    <span className={cx('avatar', className)} style={{ ['--c' as string]: m.color }} title={tip}>
      {m.short}
    </span>
  );
}

/**
 * 按身份画头像：agent 先按名字认标识（`AgentAvatar`，认不出才退回字母头像），人、系统身份仍用 `Avatar`。
 * 动态、评论、列表里「谁记下的」这类人、agent、board 都可能出现的位置用它，agent 在哪儿都显示成自己的图标。
 * 名字为空（理论上不会有）时用 `fallback` 兜底，和 `Avatar` 一样总会画一个位子
 */
export function ActorAvatar({ actor, className, fallback = '?' }: { actor: Actor; className?: string; fallback?: string }) {
  return actor.kind === 'agent' ? (
    <AgentAvatar name={actor.name || fallback} agentId={actor.agent} className={className} />
  ) : (
    <Avatar actor={actor} className={className} />
  );
}

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
  [label.priority && '优先：带这个标签的卡片先被领取', label.humanOnly && '仅人可打：agent 不能给已有卡片加、去这个标签，只能在建卡时按人的要求带上'].filter(Boolean).join('\n') || undefined;

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

/** repo：这块白板的仓库地址（PRD §4）；给了时评论里的提交号、gh#123 变成链接。
    links 为 false 时不把卡片编号变成链接（新建弹窗的预览、更新说明：点开会打断正在做的事） */
export function Markdown({ src, className, repo, links = true }: { src: string; className?: string; repo?: string | null; links?: boolean }) {
  const path = useData(s => s.path);
  const board = useData(s => s.board);
  // 其他白板的卡片引用查到了重画一次
  const refsV = useSyncExternalStore(subscribeRefs, refsVersion, refsVersion);
  const html = useMemo(() => markdown(src, { refs: links ? refsFor(path) : undefined, repo }), [src, path, links, refsV, board, repo]);
  return (
    <div
      className={cx('md', className)}
      dangerouslySetInnerHTML={{ __html: html }}
      onClick={e => {
        const t = e.target as HTMLElement;
        // 卡片编号：在当前窗口打开那张卡片（不开新标签；桌面端也不交给系统浏览器）
        const a = t.closest?.('a.xref') as HTMLAnchorElement | null;
        if (a) {
          // 网页端带着修饰键时让浏览器自己开新标签；桌面端一律在这个窗口打开（不交给系统浏览器，也开不了新窗口）
          if (e.button !== 0 || (shell === 'web' && (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey))) return;
          e.preventDefault();
          e.stopPropagation();
          const [board, id] = (a.dataset.card || '').split('#');
          if (!board || !id) return;
          openLink(board, Number(id));
          return;
        }
        // 文本类附件（.txt、.log……）：在界面里查看，不下载；带修饰键时照旧下载
        const f = t.closest?.('a.file-link[data-text]') as HTMLAnchorElement | null;
        if (f) {
          if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
          e.preventDefault();
          e.stopPropagation();
          openTextFile(f.getAttribute('href') || '', f.getAttribute('download') || f.textContent || '');
          return;
        }
        if (t.tagName !== 'IMG') return;
        // 图片（附件）点击放大；同一段里的其他图片可以左右切换
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
  value, options, onChange, className, disabled, ...rest
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  className?: string;
  disabled?: boolean;
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
      <span className="sel-v">{options.find(o => o.value === value)?.label ?? ''}</span>
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

/** 设置页、Agent tab 里的一行设置：左边名称和说明，右边控件（.set-row） */
export const SetRow = ({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) => (
  <div className="set-row">
    <div className="set-label">
      <b>{label}</b>
      {hint && <span>{hint}</span>}
    </div>
    <div className="set-control">{children}</div>
  </div>
);

/** 分段控件：选中的一段下面垫一块滑动的底（设置页、机器页的视图切换） */
/** 分段选择：一项是 [值, 文字]，第三个是 true 时那一项不能选 */
export function Segmented<T extends string>({ value, options, onChange }: { value: T; options: ([T, string] | [T, string, boolean])[]; onChange: (v: T) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pill, setPill] = useState({ w: 0, x: 0 });
  useLayoutEffect(() => {
    const place = () => {
      const on = ref.current?.querySelector<HTMLElement>('button.on');
      if (on) setPill({ w: on.offsetWidth, x: on.offsetLeft - 3 });
    };
    place();
    document.fonts?.ready.then(place);
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  }, [value]);
  return (
    <div className="segmented" role="radiogroup" ref={ref}>
      {options.map(([v, label, off]) => (
        <button key={v} type="button" role="radio" aria-checked={v === value} disabled={!!off && v !== value} className={cx(v === value && 'on')} onClick={() => onChange(v)}>
          {label}
        </button>
      ))}
      <i className="seg-pill" style={{ width: pill.w, transform: `translateX(${pill.x}px)` }} />
    </div>
  );
}

/** 说明收成一个小图标，悬停（或聚焦）时看全文：代替常驻的说明段落（default/main#289） */
export function InfoTip({ text, className }: { text: string; className?: string }) {
  return (
    <span className={cx('info-tip', className)} tabIndex={0} role="img" aria-label={text} data-tip={text}>
      <Icon name="info" />
    </span>
  );
}

/* ------------------------------ 悬停提示的可读名称 ------------------------------ */

/** 提示浮层（FloatTips 挂到 body 上的）的宿主 id：挂一份在 App 里，每个提示在里面是一个 span，
    带 data-tip 的元素用 aria-describedby 指过去。没有宿主时（单元测试单独渲染一个组件）不补可读名称，不报错 */
export const TIP_HOST = 'board-tips';

/** 按提示文字算的 id：同一段提示共用一个 span；用 `~=` 匹配，不担心前缀撞上（t1 和 t12） */
export const tipKey = (text: string) => `t${Math.abs([...text].reduce((h, c) => (Math.imul(h, 31) + c.codePointAt(0)!) | 0, 7)).toString(36)}`;

/** 提示浮层本身看不见也点不到（FloatTips 的那份是 pointer-events: none），不占可访问树 */
const mask = (el: HTMLElement) => {
  el.setAttribute('aria-hidden', 'true');
  el.removeAttribute('role');
  return el;
};

/** 给带 data-tip 的元素补一个可读的名称（default/main#314）：提示只画在悬停时的伪元素上，
    读屏看不到，键盘用户也没有地方聚焦它。这里按提示文字生成一个浮层，用 aria-describedby 指过去，
    读屏在焦点落上去时读出「<元素自己的名称>，<提示>」；元素本身名称是空的时候（只有图标），提示就是它的名称。
    提示内容变了（例如按钮什么时候变成「取消勾选」）到下一次渲染就对上；元素不在的时候浮层一起摘掉 */
export function TipNote({ el, text }: { el: HTMLElement; text: string }) {
  useLayoutEffect(() => {
    const host = document.getElementById(TIP_HOST);
    // 没有提示文字（例如按钮可以点之后就不用说了）就不补
    if (!host || !text.trim()) return;
    const id = `tip-${tipKey(text)}`;
    let node = document.getElementById(id);
    if (!node || node.parentElement !== host) {
      node = mask(document.createElement('span'));
      node.id = id;
      host.append(node);
    }
    node.textContent = text;
    const named = el.hasAttribute('aria-label') || el.hasAttribute('aria-labelledby') || !!el.textContent?.trim() || el.matches('input, textarea, select, img, svg');
    if (!named) el.setAttribute('aria-label', text);
    el.setAttribute('aria-describedby', id);
    const span = node;
    return () => {
      el.removeAttribute('aria-describedby');
      el.removeAttribute('aria-label');
      // 摘之前再确认一次：还有别的元素用着同一段提示就留着（按顺序对上的，不查页面里的老属性）
      setTimeout(() => {
        if (span.isConnected && !document.querySelector(`[aria-describedby~="${id}"]`)) span.remove();
      }, 0);
    };
  }, [el, text]);
  return null;
}

/** 提示的宿主：一个 `hidden` 的空 div，App 里挂一份；每个提示在其中是一个 span（见 TipNote） */
export function TipHost() {
  return <div id={TIP_HOST} hidden />;
}

/** 服务端升级之后开着旧页面：顶栏门面上留一条（提示条关掉之后还看得见，点一下刷新。default/main#368） */
export function StaleBanner() {
  const stale = useData(s => s.stale);
  const server = useData(s => s.serverVersion) ?? '';
  if (!stale) return null;
  return (
    <button className="stale-banner" type="button" onClick={() => location.reload()} data-tip={staleText(server)}>
      <Icon name="update" />
      <span>{staleText(server)}</span>
    </button>
  );
}

/** 一个元素 + 它的提示：提示由全站的 data-tip 浮层画，同时按上面说的补上可读的名称 */
export function Tip({ text, children }: { text: string; children: ReactElement }) {
  const ref = useRef<HTMLElement>(null);
  // 第一次渲染时子元素还没挂上、ref 是空的：挂上之后补一遍，浮层在这一遍里画出来
  const [, redraw] = useState(0);
  useEffect(() => {
    if (ref.current) redraw(n => n + 1);
  }, []);
  // 元素自己带着 ref（例如「更早的 M 张」要量高度）时两边都收着，不互相顶掉
  const own = (children as ReactElement<{ ref?: Ref<HTMLElement> }>).props.ref;
  const set = useCallback(
    (el: HTMLElement | null) => {
      ref.current = el;
      if (typeof own === 'function') own(el);
      else if (own) (own as { current: HTMLElement | null }).current = el;
    },
    [own],
  );
  return (
    <>
      {cloneElement(children as ReactElement<{ ref?: Ref<HTMLElement>; 'data-tip'?: string }>, { ref: set, 'data-tip': text })}
      {ref.current && createPortal(<TipNote el={ref.current} text={text} />, document.body)}
    </>
  );
}

/** 全站的悬停提示：所有 `[data-tip]` 共用 body 上的一个浮层（App 里挂一份）。
    - 用 Firefox / Chromium / WebKit 的 top layer（popover）画，不参与页面里的 z-index、overflow、transform 叠层；
      不支持 popover 时退回固定的 z-index，仍然挂在 body 上
    - 只在能悬停的设备上启用；触屏沿用原来的点击操作，不额外弹一层提示
    - 默认放下方，`data-tip-pos` 选方向，放不下就翻到对面；`data-tip-align` 控制左右对齐，最后夹在视口里
    - 滚动、缩放、按下、按 Esc、焦点移走时收起，不在抽屉 / 弹窗打开后残留 */
export function FloatTips() {
  const [tip, setTip] = useState<{ el: HTMLElement; text: string } | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const shown = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!window.matchMedia('(hover: hover)').matches) return;
    let showT = 0;
    const tipOf = (t: EventTarget | null) => (t as HTMLElement | null)?.closest?.<HTMLElement>('[data-tip]') ?? null;
    const hide = () => {
      clearTimeout(showT);
      setTip(null);
    };
    const over = (e: PointerEvent) => {
      const el = tipOf(e.target);
      if (!el || e.buttons || document.body.classList.contains('is-dragging') || document.body.classList.contains('tile-moving')) return;
      clearTimeout(showT);
      if (shown.current && shown.current !== el) setTip(null);
      // 和原来的 ::after 一样停 350ms 再出现，指针扫过不闪
      showT = window.setTimeout(() => {
        if (el.isConnected && el.dataset.tip) setTip({ el, text: el.dataset.tip });
      }, 350);
    };
    const out = (e: PointerEvent) => {
      const el = tipOf(e.target);
      if (el && !el.contains(e.relatedTarget as Node | null)) hide();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') hide();
    };
    const focus = (e: FocusEvent) => {
      if (shown.current && !shown.current.contains(e.target as Node)) hide();
    };
    document.addEventListener('pointerover', over);
    document.addEventListener('pointerout', out);
    document.addEventListener('pointerdown', hide, true);
    document.addEventListener('scroll', hide, true);
    document.addEventListener('keydown', key, true);
    document.addEventListener('focusin', focus, true);
    window.addEventListener('blur', hide);
    window.addEventListener('resize', hide);
    return () => {
      clearTimeout(showT);
      document.removeEventListener('pointerover', over);
      document.removeEventListener('pointerout', out);
      document.removeEventListener('pointerdown', hide, true);
      document.removeEventListener('scroll', hide, true);
      document.removeEventListener('keydown', key, true);
      document.removeEventListener('focusin', focus, true);
      window.removeEventListener('blur', hide);
      window.removeEventListener('resize', hide);
    };
  }, []);

  useLayoutEffect(() => {
    const el = box.current;
    if (!tip || !el) return;
    shown.current = tip.el;
    // popover 先把浮层放进浏览器的 top layer，再量尺寸；不支持时这里什么也不做
    el.setAttribute('popover', 'manual');
    if (typeof el.showPopover === 'function') {
      try {
        el.showPopover();
      } catch {
        /* 已经打开（换了提示内容）时 showPopover 会抛，忽略 */
      }
    }
    const r = tip.el.getBoundingClientRect();
    const at = placeTip(
      r,
      { width: el.offsetWidth, height: el.offsetHeight },
      { width: window.innerWidth, height: window.innerHeight },
      { pos: tip.el.dataset.tipPos, align: tip.el.dataset.tipAlign },
    );
    el.style.left = `${at.left}px`;
    el.style.top = `${at.top}px`;
    el.dataset.side = at.side;
    el.dataset.placed = 'true';
    return () => {
      shown.current = null;
    };
  }, [tip]);

  // 触发器被拿掉（切页、弹窗内容换掉、卡片刷新）或提示文字变了：别留一层过时的浮层
  useEffect(() => {
    if (!tip) return;
    const check = () => {
      if (!tip.el.isConnected || !tip.el.dataset.tip) setTip(null);
      else if (tip.el.dataset.tip !== tip.text) setTip({ el: tip.el, text: tip.el.dataset.tip });
    };
    const observer = new MutationObserver(check);
    observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-tip'] });
    return () => observer.disconnect();
  }, [tip]);

  if (!tip || !tip.el.isConnected) return null;
  // 不写 role="tooltip"：它得配 aria-describedby 才读得到，而这里挂的时候不一定有（default/main#314），
  // 可读名称由上面的 Tip / TipNote 补，挂的这份浮层对读屏隐藏
  return createPortal(
    <div ref={box} className="float-tip" data-placed="false" aria-hidden="true">
      {tip.text}
    </div>,
    document.body,
  );
}
