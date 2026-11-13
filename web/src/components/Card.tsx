/* 卡片：白板状态块里的卡片（「未整理」的条目在「未整理」页，components/Inbox） */
import { memo, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { BEAT_MISSES, BEAT_MS, timing } from '../lib/schema';
import type { Card as CardT, Label, StatusView } from '../lib/types';
import { agentColor, clock, cx, dateTime, durWords, replay, since } from '../lib/util';
import { useUI, type Flash } from '../store/ui';
import { renderKey, useCurrent, useData, useOnline } from '../store/data';
import { Avatar, LabelChips, Title, agent, useNow } from './bits';
import { Icon } from './Icon';
import { CardDispatchLine } from './hosts/Dispatch';
import { agentMark, cardDispatch } from '../lib/hosts';

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
  /** 所在状态是终点：单行时标题划掉，磁贴的编号用状态色 */
  final?: boolean;
  /** 所在状态的卡片形态；不给时终点单行、其他卡片 */
  view?: StatusView;
  /** 不带 ID 执行 board claim 时会被领走的那张 */
  next?: boolean;
}

export const CardView = memo(function CardView({
  card: c, labels, filter, open, placeholder, flash, entering, delay, color, final, view, next,
}: Props) {
  const held = !!c.holder;
  // 只有被持有的卡片要每秒走表；做过的卡片显示「完成于几分钟前」，半分钟一次；其他卡片不起定时器
  const now = useNow(held ? 1000 : c.finishedAt ? 30000 : null);
  const ref = useRef<HTMLElement>(null);
  const sel = useUI(s => s.labelSel);
  // 正在写入的标签（拖上来还没确认）
  const pending = useData(s => s.pending[`${s.path}#${c.id}`]);
  // 有还没确认的修改（移动、改标题、新建……）：画成「保存中」；负数 ID 是还在新建的临时卡片
  const saving = useData(s => !!s.saving[`${s.path}#${c.id}`]);
  const draft = c.id < 0;
  const no = draft ? '…' : c.id;
  const active = filterActive(filter, sel);
  const hit = active ? cardHit(c, filter, sel, labels) : true;
  const online = useOnline();
  const t = timing(c, now, online);
  const doing = held;
  // 派发排队：卡片在「进行中」里等机器开始，还没有人持有，画成虚线框（和正在做的区分开）
  const queue = useData(s => {
    if (held) return undefined;
    const d = cardDispatch(s.ws, s.path, c.id);
    return d?.state === 'queued' ? d.agent : undefined;
  });
  const gh = (c.body.match(/gh#\d+/) || [])[0];
  // 入场动画放完才去掉 entering：白板在动画中途重画（推送刷新、新建的卡片换成真卡片）时 entering 已经变回 false，
  // 立刻去掉会把动画掐断；一直留着又会在「保存中」结束、animation 换回来时再弹一次
  // 本窗口新建的临时卡片用轻的入场（card-add：只淡入，不缩放、不模糊、不回弹），别处来的新卡片照旧 card-in
  const [enter, setEnter] = useState<'' | 'in' | 'add'>(entering ? (draft ? 'add' : 'in') : '');
  const shape: StatusView = view ?? (final && !doing ? 'line' : 'card');

  useLayoutEffect(() => {
    if (flash) replay(ref.current, 'flash');
  }, [flash?.key]); // eslint-disable-line react-hooks/exhaustive-deps

  const style: Record<string, string> = {};
  if (color) style['--c'] = color;
  if (doing) style['--ac'] = agentColor(c.holder!);
  else if (queue) style['--ac'] = agentMark(queue).color;
  if (flash) style['--flash'] = flash.color;
  if (delay != null) style.animation = `card-in var(--dur-4) var(--spring-soft) both ${delay}ms`;

  return (
    <article
      ref={ref}
      className={cx(
        'card',
        doing && 'doing',
        queue && 'queued',
        final && !doing && shape !== 'card' && 'done',
        shape === 'line' && 'v-line',
        shape === 'tile' && 'v-tile',
        next && 'next',
        open && 'is-open',
        active && !hit && 'filtered-out',
        active && hit && 'matched',
        t.beat === 'late' && 'late',
        enter === 'in' && 'entering',
        enter === 'add' && 'adding',
        placeholder && 'is-placeholder',
        saving && 'saving',
        draft && 'draft',
      )}
      aria-busy={saving || undefined}
      tabIndex={0}
      data-id={c.id}
      data-flip={`c${renderKey(c.id)}`}
      aria-label={draft ? `正在新建：${c.title}` : `#${c.id} ${c.title}`}
      style={style}
      onAnimationEnd={
        enter ? e => e.target === e.currentTarget && (e.animationName === 'card-in' || e.animationName === 'card-add') && setEnter('') : undefined
      }
    >
      {shape === 'tile' ? (
        <>
          <span className="ct-id">{no}</span>
          {doing && <i className={cx('ct-beat', t.beat)} />}
          {/* 标题、标签在整块白板共用的悬浮卡里（TilePeek） */}
        </>
      ) : shape === 'line' ? (
        <div className="card-line">
          {final && !doing ? (
            <span className="done-check">
              <Icon name="check" />
            </span>
          ) : (
            <span className="line-dot" />
          )}
          <span className="card-id">#{no}</span>
          <span className="card-line-title">
            <Title text={c.title} />
          </span>
          {doing && (
            <span className={cx('line-holder', t.beat)} title={c.holder!}>
              <Avatar actor={agent(c.holder!)} />
              <i className="beat" />
            </span>
          )}
        </div>
      ) : (
        <>
          <div className="card-row">
            <span className="card-id">#{no}</span>
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
              <LabelChips ids={c.labels} labels={labels} pending={pending} />
            </div>
          )}
          {!doing && <CardDispatchLine card={c.id} />}
          {doing ? (
            <div className="card-claim">
              <Avatar actor={agent(c.holder!)} />
              <MidName className="claim-name" name={c.holder!} />
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
                {/* 累计耗时、领取次数、完成于排在同一行；窄的块里先省掉「累计」「完成于」两个词，还放不下时次要的文字省略 */}
                <span className="run-pill" title={`累计 ${durWords(t.total)}，做过 ${t.runs} 次${t.lost ? `，其中 ${t.lost} 次异常退出` : ''}`}>
                  <Icon name="timer" />
                  <span className="rp-text">
                    <span className="rp-k">累计 </span>
                    {durWords(t.total)}
                  </span>
                </span>
                <span className="claim-sub" title={c.finishedAt ? `${t.runs} 次 · 完成于 ${dateTime(c.finishedAt)}` : `${t.runs} 次`}>
                  {t.runs} 次
                  {c.finishedAt && (
                    <>
                      {' · '}
                      <span className="rp-k">完成于 </span>
                      {since(c.finishedAt, now)}
                    </>
                  )}
                </span>
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

/** 进行中的计时胶囊：跳动的心跳点 + 这一次的耗时。卡片、详情共用 */
export function RunPill({ card: c, now, big }: { card: CardT; now: number; big?: boolean }) {
  const online = useOnline();
  const t = timing(c, now, online);
  const tip =
    t.beat === 'live'
      ? `这一次的耗时（从第一次心跳算起）· ${c.holder} 在线，心跳正常`
      : t.beat === 'late'
        ? `${c.holder} 已离线：连续 ${BEAT_MISSES} 次没收到心跳，马上释放`
        : t.beat === 'wait'
          ? `领取了，还没收到第一次心跳；${BEAT_MS * BEAT_MISSES / 1000} 秒内没收到就释放`
          : `已发 final，${BEAT_MS * BEAT_MISSES / 1000} 秒内没移走就释放`;
  return (
    <span className={cx('run-pill live', t.beat, big && 'big')} data-tip={tip} data-tip-align="end">
      <i className="beat" />
      {t.cur != null ? clock(t.cur) : t.beat === 'wait' ? '等待心跳' : '已结束'}
    </span>
  );
}

/* ---------------- 磁贴的悬浮卡 ---------------- */

interface Peek {
  id: number;
  el: HTMLElement;
}

/** 磁贴的悬浮卡：整个白板只有一个，挂在 body 上。
    - 悬停时出现在这张磁贴上方（上面放不下就放下面），横向跟着指针走，不挡住磁贴本身
    - 指针在磁贴之间移动时直接换内容、平滑挪过去，不在上一张旁边停留；离开磁贴稍等一下再收起，扫过磁贴之间的缝隙不闪
    - 键盘聚焦（:focus-visible）时也出现；拖动卡片、拖动状态块时收起；没有悬停的设备（手机、平板）不出现 */
export function TilePeek() {
  const [peek, setPeek] = useState<Peek | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const at = useRef<{ x: number; shown: boolean }>({ x: 0, shown: false });

  useEffect(() => {
    if (!window.matchMedia('(hover: hover)').matches) return;
    let hideT = 0;
    const busy = () => document.body.classList.contains('is-dragging') || document.body.classList.contains('tile-moving');
    const tileOf = (t: EventTarget | null) => {
      const el = (t as HTMLElement | null)?.closest?.<HTMLElement>('.card.v-tile');
      return el && !el.classList.contains('is-placeholder') && !el.classList.contains('drag-ghost') ? el : null;
    };
    const show = (el: HTMLElement, x: number) => {
      clearTimeout(hideT);
      at.current.x = x;
      setPeek(p => (p && p.el === el ? p : { id: Number(el.dataset.id), el }));
      place();
    };
    const hide = (now = false) => {
      clearTimeout(hideT);
      if (now) setPeek(null);
      else hideT = window.setTimeout(() => setPeek(null), 90);
    };
    const move = (e: PointerEvent) => {
      if (e.pointerType === 'touch') return;
      if (e.buttons || busy()) return hide(true);
      const el = tileOf(e.target);
      if (el) show(el, e.clientX);
      else hide();
    };
    const focusIn = (e: FocusEvent) => {
      const el = tileOf(e.target);
      if (el && el === e.target && el.matches(':focus-visible')) {
        const r = el.getBoundingClientRect();
        show(el, r.left + r.width / 2);
      }
    };
    const focusOut = (e: FocusEvent) => {
      if (tileOf(e.target)) hide();
    };
    const away = () => hide(true);
    document.addEventListener('pointermove', move, { passive: true });
    document.addEventListener('pointerdown', away, true);
    document.addEventListener('focusin', focusIn);
    document.addEventListener('focusout', focusOut);
    document.addEventListener('scroll', away, true);
    window.addEventListener('blur', away);
    return () => {
      clearTimeout(hideT);
      document.removeEventListener('pointermove', move);
      document.removeEventListener('pointerdown', away, true);
      document.removeEventListener('focusin', focusIn);
      document.removeEventListener('focusout', focusOut);
      document.removeEventListener('scroll', away, true);
      window.removeEventListener('blur', away);
    };
  }, []);

  const peekRef = useRef(peek);
  peekRef.current = peek;
  /** 放到磁贴上方、横向以指针为中心，夹在窗口里 */
  const place = () => {
    const p = peekRef.current;
    const el = box.current;
    if (!p || !el || !p.el.isConnected) return;
    const r = p.el.getBoundingClientRect();
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const gap = 8;
    const x = Math.round(Math.max(gap, Math.min(window.innerWidth - w - gap, at.current.x - w / 2)));
    const above = r.top - gap - h >= gap;
    const y = Math.round(above ? r.top - gap - h : r.bottom + gap);
    el.dataset.side = above ? 'top' : 'bottom';
    // 第一次出现时直接放到位置上（淡入），之后在磁贴之间换时平滑挪过去
    el.style.transition = at.current.shown ? '' : 'none';
    el.style.transform = `translate3d(${x}px, ${y}px, 0)`;
    at.current.shown = true;
  };

  const card = useData(s => (peek ? s.board?.cards[peek.id] : undefined));
  const pending = useData(s => (peek ? s.pending[`${s.path}#${peek.id}`] : undefined));
  const labels = useCurrent()?.project.labels ?? [];
  const now = useNow(card?.holder ? 1000 : null);

  // 内容换了（换了一张磁贴、标题变长）之后量尺寸再放
  useLayoutEffect(() => {
    if (!peek) at.current.shown = false;
    else place();
  });

  if (!peek || !card || !peek.el.isConnected) return null;
  const doing = !!card.holder;
  const style: Record<string, string> = {};
  const c = peek.el.style.getPropertyValue('--c');
  if (c) style['--c'] = c;
  return createPortal(
    <div ref={box} className={cx('ct-pop', peek.el.classList.contains('done') && 'done')} role="tooltip" style={style}>
      <span className="ct-pop-head">
        <b>#{card.id}</b>
        {doing && (
          <>
            <span className="ct-pop-holder">
              <Avatar actor={agent(card.holder!)} />
              <MidName name={card.holder!} />
            </span>
            <RunPill card={card} now={now} />
          </>
        )}
      </span>
      <span className="ct-pop-title">
        <Title text={card.title} />
      </span>
      {card.labels.length > 0 && (
        <span className="ct-pop-labels">
          <LabelChips ids={card.labels} labels={labels} pending={pending} />
        </span>
      )}
    </div>,
    document.body,
  );
}

/** 名字单行显示，放不下时省略中间：开头可以缩，结尾几个字符（多半是区分会话的后缀）一直留着；悬停看全名 */
export function MidName({ name, className, tail = 8 }: { name: string; className?: string; tail?: number }) {
  const cut = name.length > tail + 4 ? name.length - tail : name.length;
  return (
    <span className={cx('mid-name', className)} title={name}>
      <span className="mid-head">{name.slice(0, cut)}</span>
      {cut < name.length && (
        <span className="mid-tail">
          <span>{name.slice(cut)}</span>
        </span>
      )}
    </span>
  );
}

/* ---------------- 卡片、弹窗里的悬停提示 ---------------- */

/** 卡片和弹窗（卡片详情、新建等 .drawer）里带 data-tip 的提示不用 ::after 画：那样会被相邻卡片、状态块、
    弹窗底部的操作栏盖住，被列表、详情的滚动区域裁掉。改成挂在 body 上的一个浮层，放在所有卡片、弹窗之上；
    默认放下面，下面放不下就放上面（data-tip-pos="top" 时反过来），左右夹在窗口里。只在能悬停的设备上启用（App 里挂一份） */
const FLOAT_TIP = '.card [data-tip], .drawer [data-tip]';
export function FloatTips() {
  const [tip, setTip] = useState<{ el: HTMLElement; text: string } | null>(null);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!window.matchMedia('(hover: hover)').matches) return;
    let showT = 0;
    const tipOf = (t: EventTarget | null) => (t as HTMLElement | null)?.closest?.<HTMLElement>(FLOAT_TIP) ?? null;
    const hide = () => {
      clearTimeout(showT);
      setTip(null);
    };
    const over = (e: PointerEvent) => {
      const el = tipOf(e.target);
      if (!el || e.buttons || document.body.classList.contains('is-dragging')) return;
      clearTimeout(showT);
      // 和原来的 ::after 一样停 350ms 再出现，指针扫过不闪
      showT = window.setTimeout(() => el.isConnected && el.dataset.tip && setTip({ el, text: el.dataset.tip }), 350);
    };
    const out = (e: PointerEvent) => {
      const el = tipOf(e.target);
      if (el && !el.contains(e.relatedTarget as Node | null)) hide();
    };
    document.addEventListener('pointerover', over);
    document.addEventListener('pointerout', out);
    document.addEventListener('pointerdown', hide, true);
    document.addEventListener('scroll', hide, true);
    window.addEventListener('blur', hide);
    return () => {
      clearTimeout(showT);
      document.removeEventListener('pointerover', over);
      document.removeEventListener('pointerout', out);
      document.removeEventListener('pointerdown', hide, true);
      document.removeEventListener('scroll', hide, true);
      window.removeEventListener('blur', hide);
    };
  }, []);

  useLayoutEffect(() => {
    const el = box.current;
    if (!tip || !el) return;
    const r = tip.el.getBoundingClientRect();
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const gap = 8;
    const clampX = (x: number) => Math.round(Math.max(gap, Math.min(window.innerWidth - w - gap, x)));
    const clampY = (y: number) => Math.round(Math.max(gap, Math.min(window.innerHeight - h - gap, y)));
    if (tip.el.dataset.tipPos === 'right') {
      el.style.left = `${clampX(r.right + 10)}px`;
      el.style.top = `${clampY(r.top + r.height / 2 - h / 2)}px`;
      el.dataset.side = 'right';
      return;
    }
    const end = tip.el.dataset.tipAlign === 'end';
    const x = clampX(end ? r.right - w : r.left + r.width / 2 - w / 2);
    const fitsBelow = r.bottom + gap + h <= window.innerHeight - gap;
    const fitsAbove = r.top - gap - h >= gap;
    const below = tip.el.dataset.tipPos === 'top' ? !fitsAbove && fitsBelow : fitsBelow || !fitsAbove;
    const y = Math.round(below ? r.bottom + gap : r.top - gap - h);
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    el.dataset.side = below ? 'bottom' : 'top';
  }, [tip]);

  if (!tip || !tip.el.isConnected) return null;
  return createPortal(
    <div ref={box} className="float-tip" role="tooltip">
      {tip.text}
    </div>,
    document.body,
  );
}
