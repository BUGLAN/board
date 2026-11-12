/* ==========================================================================
   白板设置（工作流条末尾「白板设置」，或点状态块标题上的排序标记）：每块白板自己的设置，存在服务端、所有窗口和机器同步
   沿用卡片详情的居中弹窗（.scrim / .drawer / .d-*）：左边是设置，右栏是「预览」（手机上是底部 sheet，预览在上）
   - 卡片顺序：「保留顺序」开关 + 每个状态的排序依据（选择框 + 方向）。预览用示例卡片演示拖动的结果（lib/orderDemo.ts）
   - 状态块布局：现在的 / 保存的 / 默认三张缩略图，按钮放在各自的图下面；预览里块在「现在」和「换过去以后」之间来回移动
   保存：开关和排序依据是设置项，各存各的（saveSetting）：当帧生效、不锁别的控件，保存状态画在控件旁边（Sync）；
   布局的三个操作沿用工作流的乐观更新（flowKey），保存完之前再点不发请求
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../lib/api';
import { playDemo, scenesOf, stepsOf, type DemoInput, type Scene } from '../lib/orderDemo';
import { statusColor, viewOf } from '../lib/schema';
import { ORDERS, orderDir, orderName, orderOf, orderOption } from '../lib/sort';
import type { BoardSnapshot, Label, Order, Status, Tile } from '../lib/types';
import { cx, reduceMotion } from '../lib/util';
import { colCount, flowKey, optimistic, saveSetting, useBusy, useCurrent, useData } from '../store/data';
import { openBoardSettings, toast, useUI } from '../store/ui';
import { Select, Sync, useSync } from './bits';
import { ReadyCheckSection } from './BoardReady';
import { pack, withDefaults } from './BoardView';
import { Icon } from './Icon';

type Which = 'current' | 'saved' | 'def';
type Pv = { mode: 'order'; key: string } | { mode: 'layout'; which: Which };

const sameLayout = (a: Record<string, Tile>, b: Record<string, Tile>) =>
  Object.keys(a).length === Object.keys(b).length && Object.entries(a).every(([k, t]) => b[k] && b[k].x === t.x && b[k].w === t.w && b[k].y === t.y);

const FLAG = (s: Status) => (s.claim ? '可领取' : s.lock ? '锁定' : s.final ? '终点' : s.error ? '错误节点' : '');

export function BoardSettings({ path }: { path: string }) {
  const req = useUI(s => s.boardSettings);
  const on = req?.path === path;
  const board = useData(s => (s.board?.path === path ? s.board : null));
  // 关的时候弹窗淡出，内容留到动画结束
  const [shown, setShown] = useState(on);
  useEffect(() => {
    if (on) return setShown(true);
    const t = setTimeout(() => setShown(false), 420);
    return () => clearTimeout(t);
  }, [on]);
  const close = () => openBoardSettings(null);
  useEffect(() => {
    if (!on) return;
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      // 选择框的列表、确认框先关自己
      const ui = useUI.getState();
      if (ui.ctx || ui.confirm || ui.palette.open) return;
      e.preventDefault();
      e.stopPropagation();
      close();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [on]);
  return createPortal(
    <>
      <div className={cx('scrim', on && 'on')} onClick={close} />
      <aside className={cx('drawer bs-modal', on && 'on')} role="dialog" aria-modal="true" aria-hidden={!on} aria-label="白板设置">
        {board && shown && <Body board={board} path={path} focus={on ? req?.focus : undefined} on={on} close={close} />}
      </aside>
    </>,
    document.body,
  );
}

function Body({ board, path, focus, on, close }: { board: BoardSnapshot; path: string; focus?: string; on: boolean; close: () => void }) {
  const meta = useCurrent();
  const labels = meta?.project.labels ?? [];
  const statuses = board.statuses;
  const keep = !!board.keepSorted;
  const { sync, track } = useSync();
  const firstKey = (statuses.find(s => s.final) ?? statuses[0])?.key ?? '';
  const [pv, setPv] = useState<Pv>({ mode: 'order', key: focus ?? firstKey });
  const rows = useRef<Record<string, HTMLDivElement | null>>({});
  const hoverT = useRef(0);
  useEffect(() => () => clearTimeout(hoverT.current), []);

  // 从状态块标题上的排序标记打开：预览这一块，滚到这一行，焦点放到它的排序依据上
  useEffect(() => {
    if (!on || !focus) return;
    setPv({ mode: 'order', key: focus });
    const t = setTimeout(() => {
      const row = rows.current[focus];
      row?.scrollIntoView({ block: 'nearest', behavior: reduceMotion() ? 'auto' : 'smooth' });
      row?.querySelector<HTMLElement>('.sel')?.focus({ preventScroll: true });
    }, 260);
    return () => clearTimeout(t);
  }, [on, focus]);

  // 悬停换预览：稍等一下再换（鼠标划过几行时不来回重放）；选择框的列表开着时不换
  const preview = (next: Pv) => {
    clearTimeout(hoverT.current);
    hoverT.current = window.setTimeout(() => {
      if (useUI.getState().ctx) return;
      setPv(p => (p.mode === next.mode && (p.mode === 'order' ? p.key === (next as typeof p).key : p.which === (next as typeof p).which) ? p : next));
    }, 140);
  };

  const setKeep = (v: boolean) => {
    setPv(p => (p.mode === 'order' ? p : { mode: 'order', key: firstKey }));
    track('keep', saveSetting(`${flowKey(path)}:keep`, { board: b => ({ ...b, keepSorted: v }) }, () => api.keepSorted(path, v)));
  };
  const setOrder = (key: string, o: Order) => {
    setPv({ mode: 'order', key });
    track(
      `order:${key}`,
      saveSetting(
        `${flowKey(path)}:order:${key}`,
        { board: b => ({ ...b, statuses: b.statuses.map(s => (s.key === key ? { ...s, order: o } : s)) }) },
        () => api.updateStatus(path, key, { order: o }),
      ),
    );
  };

  /* ---------- 状态块布局 ---------- */
  const busy = useBusy(flowKey(path));
  const keys = statuses.map(s => s.key);
  const current = Object.fromEntries(statuses.filter(s => s.layout).map(s => [s.key, s.layout!]));
  const now = withDefaults(keys, current);
  const saved = board.savedLayout ?? null;
  const savedAll = saved ? withDefaults(keys, saved) : null;
  const def = withDefaults(keys, {});
  const savedNow = !!savedAll && sameLayout(savedAll, now);
  const defNow = Object.keys(current).length === 0;
  const layouts: Record<Which, Record<string, Tile> | null> = { current: now, saved: savedAll, def };
  const counts = Object.fromEntries(keys.map(k => [k, colCount(board, k)]));
  /** 布局的操作：当帧生效，保存完之前再点不发请求（保存状态画在那张图的标题旁边） */
  const layoutOp = (which: Which, patch: (b: BoardSnapshot) => BoardSnapshot, req: () => Promise<unknown>, done?: string) => {
    if (busy) return;
    setPv({ mode: 'layout', which });
    track(
      `lay:${which}`,
      optimistic({ board: patch, keys: [flowKey(path)] }, req).then(() => {
        if (done) toast({ type: 'success', title: done });
      }),
    );
  };
  const doSave = () => layoutOp('current', b => ({ ...b, savedLayout: now }), () => api.saveLayout(path, now), '已保存布局');
  const doApply = () =>
    saved &&
    layoutOp(
      'saved',
      // 已删除的状态不在 saved 里；之后新增的状态没有位置，按默认规则补位
      b => ({ ...b, statuses: b.statuses.map(s => ({ ...s, layout: saved[s.key] ?? null })) }),
      () => api.applyLayout(path),
    );
  const doReset = () =>
    layoutOp('def', b => ({ ...b, statuses: b.statuses.map(s => (s.layout ? { ...s, layout: null } : s)) }), () => api.flowLayout(path, {}), '已复原为默认布局，保存的布局还在');

  const lay = (which: Which, title: string, badge: React.ReactNode, sub: string, btn: string, can: boolean, act: () => void) => (
    <div className={cx('bs-lay', pv.mode === 'layout' && pv.which === which && 'focus')} onPointerOver={() => preview({ mode: 'layout', which })} onFocus={() => preview({ mode: 'layout', which })}>
      <div className="bs-lay-h">
        <b>{title}</b>
        <Sync s={sync[`lay:${which}`]} inline />
        {badge}
      </div>
      {layouts[which] ? (
        <div className="bs-thumb">
          <LayoutMap statuses={statuses} counts={counts} tiles={layouts[which]!} />
        </div>
      ) : (
        <div className="bs-thumb empty">还没有保存过</div>
      )}
      <div className="bs-lay-s">{sub}</div>
      <button type="button" className="btn sm" disabled={!can} onClick={act}>
        {btn}
      </button>
    </div>
  );

  const pvStatus = pv.mode === 'order' ? statuses.find(s => s.key === pv.key) ?? statuses[0] : null;
  return (
    <>
      <header className="d-head">
        <span className="bs-ttl">
          <Icon name="settings" />
          白板设置
        </span>
        <span className="d-col bs-where">
          <i />
          {meta ? `${meta.project.name} / ${meta.board.name}` : path}
        </span>
        <span className="spacer" />
        <button type="button" className="icon-btn sm" aria-label="关闭" title="关闭（Esc）" onClick={close}>
          <Icon name="close" />
        </button>
      </header>
      <div className="drawer-body">
        <div className="d-grid">
          <div className="d-main">
            <section className="d-section" style={{ ['--i' as string]: 0 }}>
              <div className="d-label">卡片顺序</div>
              <div className={cx('bs-keep', keep && 'on')} onClick={() => setKeep(!keep)}>
                <button
                  type="button"
                  className={cx('switch', keep && 'on')}
                  role="switch"
                  aria-checked={keep}
                  aria-label="保留顺序"
                  onClick={e => {
                    e.stopPropagation();
                    setKeep(!keep);
                  }}
                >
                  <i />
                </button>
                <div className="bs-keep-t">
                  <b>
                    保留顺序
                    <Sync s={sync.keep} inline />
                  </b>
                  <span>
                    {keep
                      ? '每个状态里的卡片按下面的排序依据排列，拖动改不了顺序：拖到别的状态、在同一状态里拖，松手后卡片都会从落点滑到它该在的位置。'
                      : '拖到哪里就停在哪里，顺序按拖动的结果保存在服务端。'}
                  </span>
                </div>
              </div>
              <div className={cx('bs-rules', !keep && 'off')}>
                <div className="bs-rules-h">
                  <span>状态</span>
                  <span>排序依据</span>
                </div>
                {!keep && (
                  <div className="bs-rules-off">
                    <Icon name="info" />
                    保留顺序关着，这些依据不生效；打开后立即按它们重排。
                  </div>
                )}
                {statuses.map(s => {
                  const o = orderOf(s);
                  const x = orderOption(o);
                  return (
                    <div
                      key={s.key}
                      ref={el => {
                        rows.current[s.key] = el;
                      }}
                      className={cx('bs-rule', pv.mode === 'order' && pv.key === s.key && 'cur')}
                      style={{ ['--c' as string]: statusColor(s.color) }}
                      onPointerOver={() => keep && preview({ mode: 'order', key: s.key })}
                      onFocus={() => preview({ mode: 'order', key: s.key })}
                    >
                      <span className="col-dot" />
                      <span className="bs-rule-n">{s.name}</span>
                      <span className="bs-rule-k">{counts[s.key]}</span>
                      {FLAG(s) && <span className="bs-rule-f">{FLAG(s)}</span>}
                      <span className="sp" />
                      <Sync s={sync[`order:${s.key}`]} inline />
                      <span className="bs-rule-ctl">
                        <Select
                          value={o.by}
                          options={ORDERS.map(v => ({ value: v.by, label: v.name }))}
                          onChange={v => {
                            const opt = ORDERS.find(q => q.by === v)!;
                            setOrder(s.key, { by: opt.by, desc: opt.desc });
                          }}
                          disabled={!keep}
                          aria-label={`「${s.name}」的排序依据`}
                        />
                        <button
                          type="button"
                          className="btn sm bs-dir"
                          disabled={!keep || !x.dirs}
                          aria-label={`「${s.name}」的排序方向：${orderDir(o)}，点一下换成相反的方向`}
                          onClick={() => setOrder(s.key, { by: o.by, desc: !o.desc })}
                        >
                          <span>{orderDir(o)}</span>
                          {x.dirs && <Icon name="reorder" />}
                        </button>
                      </span>
                    </div>
                  );
                })}
              </div>
            </section>
            <section className="d-section" style={{ ['--i' as string]: 1 }}>
              <div className="d-label">
                状态块布局<span className="act">在白板上拖块标题换位置、拖右边缘改宽度</span>
              </div>
              <div className="bs-lays">
                {lay('current', '现在的布局', <span className="bs-badge live">白板上</span>, '白板上现在的样子', savedNow ? '已保存' : '保存为白板布局', !savedNow, doSave)}
                {lay(
                  'saved',
                  '保存的布局',
                  saved ? savedNow && <span className="bs-badge eq">和现在一样</span> : <span className="bs-badge">没有</span>,
                  '上次「保存为白板布局」时的样子',
                  savedNow ? '正在使用' : '应用',
                  !!saved && !savedNow,
                  () => void doApply(),
                )}
                {lay('def', '默认布局', defNow && <span className="bs-badge eq">和现在一样</span>, '每块 3 栏、一行 4 块，按工作流顺序', defNow ? '正在使用' : '复原', !defNow, doReset)}
              </div>
              <p className="bs-note">复原以后保存的布局还在，可以再应用。保存之后删掉的状态应用时跳过，新加的状态按默认规则补位。</p>
            </section>
            <ReadyCheckSection path={path} i={2} />
            <div className="bs-foot">改动立即生效，同步到所有窗口和机器。</div>
          </div>
          <aside className="d-side bs-side" aria-label="预览">
            {pv.mode === 'layout' ? (
              <LayoutPreview statuses={statuses} counts={counts} which={pv.which} layouts={layouts} />
            ) : (
              pvStatus && <OrderPreview key={pvStatus.key} status={pvStatus} statuses={statuses} keep={keep} labels={labels} />
            )}
          </aside>
        </div>
      </div>
    </>
  );
}

/* ------------------------------ 布局图 ------------------------------ */

/** 缩略图按一块假想的白板缩放：12 栏，每栏连间距 111px（和 1440 宽的窗口差不多） */
const VW = 1332;
const COLW = VW / 12;
const GAPPX = 12;

/** 估一块的高度（像素，按假想的白板宽度）：标题 + 卡片按形态和宽度排成几行；终点状态只画最近的 50 张和「更早的」一格 */
function estHeight(s: Status, n: number, w: number) {
  const inner = w * COLW - GAPPX - 20;
  const view = viewOf(s);
  const [minW, h, gap] = view === 'tile' ? [40, 24, 4] : view === 'line' ? [260, 34, 4] : [220, 88, 8];
  const per = Math.max(1, Math.floor((inner + gap) / (minW + gap)));
  const m = Math.min(n, 51);
  return 44 + (m ? Math.ceil(m / per) * (h + gap) - gap : 92) + 10;
}

function place(statuses: Status[], counts: Record<string, number>, tiles: Record<string, Tile>) {
  const keys = statuses.map(s => s.key);
  const h = Object.fromEntries(statuses.map(s => [s.key, estHeight(s, counts[s.key] ?? 0, tiles[s.key].w)]));
  const p = pack(keys, tiles, h);
  const bottom = Math.max(0, ...keys.map(k => p[k].top + h[k]));
  return { p, h, bottom };
}

/** 量容器宽度：缩略图按它缩放 */
function useWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [w, setW] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current!;
    const ro = new ResizeObserver(() => el.clientWidth && setW(el.clientWidth));
    ro.observe(el);
    if (el.clientWidth) setW(el.clientWidth);
    return () => ro.disconnect();
  }, []);
  return [ref, w] as const;
}

/** 布局图：每块按位置和宽度画，里面按卡片形态画出卡片的格子。from 给了时画成虚线框（现在的位置），块的位置变化有补间 */
function LayoutMap({ statuses, counts, tiles, from, names }: { statuses: Status[]; counts: Record<string, number>; tiles: Record<string, Tile>; from?: Record<string, Tile>; names?: boolean }) {
  const [ref, width] = useWidth();
  const s = width / VW;
  const a = place(statuses, counts, tiles);
  const b = from ? place(statuses, counts, from) : null;
  const box = (k: string, r: { p: Record<string, { x: number; w: number; top: number }>; h: Record<string, number> }): CSSProperties => ({
    left: r.p[k].x * COLW * s,
    top: r.p[k].top * s,
    width: (r.p[k].w * COLW - GAPPX) * s,
    height: r.h[k] * s,
  });
  return (
    <div className={cx('bs-map', names && 'big')} ref={ref} style={{ height: Math.max(a.bottom, b?.bottom ?? 0) * s, ['--s' as string]: s }} aria-hidden="true">
      {width > 0 && (
        <>
          {b &&
            statuses.map(st => <div key={`g${st.key}`} className="bs-map-g" style={{ ...box(st.key, b), ['--c' as string]: statusColor(st.color) }} />)}
          {statuses.map(st => (
            <div key={st.key} className="bs-map-b" style={{ ...box(st.key, a), ['--c' as string]: statusColor(st.color) }}>
              <div className="bs-map-h">
                <i />
                {names && st.name}
              </div>
              <div className={cx('bs-map-l', `v-${viewOf(st)}`)}>
                {Array.from({ length: Math.min(counts[st.key] ?? 0, 51) }, (_, i) => (
                  <b key={i} />
                ))}
              </div>
            </div>
          ))}
        </>
      )}
    </div>
  );
}

const WHICH_NAME: Record<Which, string> = { current: '现在的布局', saved: '保存的布局', def: '默认布局' };

/** 右栏：布局预览。保存的 / 默认的和现在不一样时，块在「现在」和「换过去以后」之间来回移动，虚线框是现在的位置 */
function LayoutPreview({ statuses, counts, which, layouts }: { statuses: Status[]; counts: Record<string, number>; which: Which; layouts: Record<Which, Record<string, Tile> | null> }) {
  const lay = layouts[which];
  const cur = layouts.current!;
  const moving = which !== 'current' && !!lay && !sameLayout(lay, cur);
  const [to, setTo] = useState(false);
  useEffect(() => {
    setTo(false);
    if (!moving || reduceMotion()) return;
    let t = 0;
    let on = false;
    const flipIt = () => {
      on = !on;
      setTo(on);
      t = window.setTimeout(flipIt, on ? 2600 : 1200);
    };
    t = window.setTimeout(flipIt, 1000);
    return () => clearTimeout(t);
  }, [which, moving]);
  const verb = which === 'saved' ? '应用' : '复原';
  let txt: React.ReactNode;
  if (!lay) txt = '还没有保存过布局。点「现在的布局」下面的「保存为白板布局」，把白板上现在的样子存下来。';
  else if (which === 'current') txt = '白板上现在的样子。在白板上拖块标题换位置、拖右边缘改宽度。';
  else if (!moving) txt = '和白板上现在的布局一样。';
  else {
    const n = Object.keys(lay).filter(k => !cur[k] || lay[k].x !== cur[k].x || lay[k].w !== cur[k].w || lay[k].y !== cur[k].y).length;
    txt = (
      <>
        {verb}以后，<b>{n}</b> 个状态块会换位置或宽度。
      </>
    );
  }
  return (
    <>
      <div className="bs-pv-h">
        <b>预览</b>
        <span className="d-col" style={{ ['--c' as string]: 'var(--c-review)' }}>
          <i />
          {WHICH_NAME[which]}
        </span>
      </div>
      <div className="bs-pv-b">
        {moving && (
          <div className="bs-phase">
            <span className={cx(!to && 'on')}>现在</span>→<span className={cx(to && 'on')}>{verb}以后</span>
          </div>
        )}
        {lay && (
          <div className="bs-pv-map">
            <LayoutMap statuses={statuses} counts={counts} tiles={moving && to ? lay : moving ? cur : lay} from={moving ? cur : undefined} names />
          </div>
        )}
        {moving && (
          <div className="bs-legend">
            <i />
            虚线是现在的位置
          </div>
        )}
        <div className="bs-pv-txt">{txt}</div>
      </div>
    </>
  );
}

/* ------------------------------ 拖动演示 ------------------------------ */

/** 右栏：一个状态的拖动演示，两种场景轮流放；点分段切换从那一种开始放，点 ↻ 重放 */
function OrderPreview({ status, statuses, keep, labels }: { status: Status; statuses: Status[]; keep: boolean; labels: Label[] }) {
  const o = orderOf(status);
  // 拖出来的状态：优先找连到这一块的（工作流里的上一步），没有就用别的状态；锁定状态里的卡片人拖不动，不用
  const source =
    statuses.find(s => s.key !== status.key && !s.lock && s.to.includes(status.key)) ?? statuses.find(s => s.key !== status.key && !s.lock) ?? null;
  const input: DemoInput = { target: status, source, order: o, keep, labels };
  const scenes = scenesOf(input);
  const [start, setStart] = useState<Scene>('cross');
  const [run, setRun] = useState(0);
  const [scene, setScene] = useState<Scene>(scenes.includes('cross') ? 'cross' : 'same');
  const [step, setStep] = useState(-1);
  const stage = useRef<HTMLDivElement>(null);
  const live = useRef(input);
  live.current = input;
  const labelKey = labels.map(l => l.id).join();
  useEffect(() => {
    const el = stage.current;
    if (!el || !scenes.length) return;
    let alive = true;
    void (async () => {
      let sc: Scene = scenes.includes(start) ? start : scenes[0];
      while (alive) {
        setScene(sc);
        setStep(-1);
        await playDemo(el, sc, live.current, () => alive, setStep, !reduceMotion());
        if (!alive || reduceMotion()) return;
        await new Promise(r => setTimeout(r, 400));
        sc = scenes[(scenes.indexOf(sc) + 1) % scenes.length];
      }
    })();
    return () => {
      alive = false;
      document.querySelectorAll('.bsp-ghost').forEach(g => g.remove());
    };
  }, [status.key, o.by, o.desc, keep, start, run, source?.key, labelKey]); // eslint-disable-line react-hooks/exhaustive-deps

  const steps = scenes.length ? stepsOf(scene, input, { order: orderName(o), dir: orderDir(o) }) : [];
  return (
    <>
      <div className="bs-pv-h">
        <b>预览</b>
        <span className="d-col" style={{ ['--c' as string]: statusColor(status.color) }}>
          <i />
          {status.name}
        </span>
        {scenes.length > 0 && (
          <button type="button" className="icon-btn sm" aria-label="再放一次" title="再放一次" onClick={() => setRun(n => n + 1)}>
            <Icon name="replay" />
          </button>
        )}
      </div>
      <div className="bs-pv-b">
        {scenes.length > 0 ? (
          <>
            {/* 两格一样宽：滑块按场景左右移（CSS），不用量按钮的位置 */}
            <div className="segmented bs-seg" role="tablist" aria-label="演示" data-scene={scene}>
              <span className="seg-pill" />
              {(['cross', 'same'] as Scene[]).map(sc => (
                <button
                  key={sc}
                  type="button"
                  role="tab"
                  aria-selected={scene === sc}
                  className={cx(scene === sc && 'on')}
                  disabled={!scenes.includes(sc)}
                  onClick={() => {
                    setStart(sc);
                    setRun(n => n + 1);
                  }}
                >
                  {sc === 'cross' ? '拖到别的状态' : '同一状态里拖动'}
                </button>
              ))}
            </div>
            <div className="bs-stage" ref={stage} />
            <ol className="bs-steps">
              {steps.map((t, i) => (
                <li key={i} className={cx(i === step && 'on', i < step && 'done')}>
                  <i>{i + 1}</i>
                  <span>{t}</span>
                </li>
              ))}
            </ol>
          </>
        ) : (
          <div className="bs-pv-txt">「{status.name}」是锁定状态：卡片只能由 agent 领取进入，被持有期间只有持有者能移动，界面上拖不动。</div>
        )}
        <div className="bs-pv-note">
          {keep ? `「${status.name}」按${orderName(o)}，${orderDir(o)}。` : '保留顺序关着：拖到哪就在哪。'}
          示例卡片，白板上的卡片不会动。
        </div>
      </div>
    </>
  );
}
