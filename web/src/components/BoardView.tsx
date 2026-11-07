/* ==========================================================================
   白板 Tab 的主内容区：页头 + 工作流条 + 状态块（PRD §10.3）
   - 工作流条：状态的顺序、卡片数和规则标记；拖动排序，点开设置（名称、颜色、规则、删除），末尾新增
   - 状态块：磁吸布局。拖标题摆放，吸附到 12 栏网格并向上贴紧；拖右边缘调宽。布局存进数据库、所有窗口同步
   - 卡片位置变化用 FLIP 补间；新卡片弹入；通过 CLI 或其他窗口做的修改：卡片闪光 + 角标说明是谁做的
   - 手机：状态块上下排列，顶部有状态切换条
   ========================================================================== */
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useMasonry } from '../lib/masonry';
import { api } from '../lib/api';
import * as flip from '../lib/flip';
import { boardFlow, INBOX, statusColor, STATUS_COLORS, type FlowQ } from '../lib/schema';
import type { BoardSnapshot, Label, Status, Tile } from '../lib/types';
import { cx, isMobile, reduceMotion, replay } from '../lib/util';
import { refresh, useCurrent, useData } from '../store/data';
import { openCreator, setFilter, setTab, toastError, useUI, type DragUI } from '../store/ui';
import { cardHit, CardView } from './Card';
import { Icon } from './Icon';

/** 拖拽预览：被拖的卡片放到预览位置 */
export function previewCols(cols: BoardSnapshot['cols'], drag: DragUI | null) {
  if (!drag) return cols;
  const out = Object.fromEntries(Object.entries(cols).map(([k, v]) => [k, v.filter(id => id !== drag.id)])) as BoardSnapshot['cols'];
  out[drag.col] = out[drag.col] ?? [];
  out[drag.col].splice(Math.min(drag.index, out[drag.col].length), 0, drag.id);
  return out;
}

/** 记录每块白板已经出现过的卡片：首次出现的弹入 */
export function useEntering(board: BoardSnapshot | null, generation: number) {
  const seen = useRef<{ gen: number; ids: Set<number>; fresh: boolean }>({ gen: -1, ids: new Set(), fresh: true });
  if (seen.current.gen !== generation) seen.current = { gen: generation, ids: new Set(), fresh: true };
  const s = seen.current;
  const entering = new Set<number>();
  if (board && !s.fresh) for (const id of Object.keys(board.cards).map(Number)) if (!s.ids.has(id)) entering.add(id);
  useEffect(() => {
    if (!board) return;
    for (const id of Object.keys(board.cards).map(Number)) s.ids.add(id);
    s.fresh = false;
  });
  return { entering, fresh: !!board && s.fresh };
}

const run = (p: Promise<unknown>) =>
  p.then(() => refresh()).catch(e => {
    toastError(e);
    refresh();
  });

export function BoardView() {
  const board = useData(s => s.board);
  const generation = useData(s => s.generation);
  const detailId = useData(s => s.detailId);
  const cur = useCurrent();
  const filter = useUI(s => s.filter.trim().toLowerCase());
  const flashes = useUI(s => s.flashes);
  const drag = useUI(s => s.drag);
  const reveal = useUI(s => s.reveal);
  const reject = useUI(s => s.reject);
  const boardRef = useRef<HTMLDivElement>(null);
  const labels = cur?.project.labels ?? [];
  const { entering, fresh } = useEntering(board, generation);
  const cols = useMemo(() => (board ? previewCols(board.cols, drag) : null), [board, drag]);
  const f = boardFlow(board);
  const path = cur?.board.path ?? '';

  useLayoutEffect(() => {
    flip.play();
  }, [board, drag]);

  // 定位到卡片
  useEffect(() => {
    if (!reveal) return;
    const n = document.querySelector<HTMLElement>(`[data-flip="c${reveal.id}"]`);
    if (!n) return;
    n.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: reduceMotion() ? 'auto' : 'smooth' });
    replay(n, 'pulse');
  }, [reveal]);

  useEffect(() => {
    if (!reject) return;
    const c = boardRef.current?.querySelector(`[data-drop="${reject.col}"]`);
    replay(c, 'reject');
    const t = setTimeout(() => c?.classList.remove('reject'), 500);
    return () => clearTimeout(t);
  }, [reject]);

  const labelSel = useUI(s => s.labelSel);
  // 页头只提示筛选框里的文字；侧栏选中的标签在胶囊上已经高亮，不再重复提示
  const n = filter && board ? Object.values(board.cards).filter(c => c.column !== INBOX && cardHit(c, filter, labelSel, labels)).length : 0;

  return (
    <div className="page page-board" data-page="board">
      <div className="page-head">
        <nav className="crumbs" aria-label="位置">
          <button type="button" className="crumb-link" onClick={() => setTab('projects')}>
            {cur?.project.name}
          </button>
          <Icon name="chevRight" />
          <b>{cur?.board.name}</b>
        </nav>
        {filter && (
          <span className="filter-pill">
            筛选「{filter}」· {n} 张
            <button type="button" aria-label="清除筛选" onClick={() => setFilter('')}>
              ×
            </button>
          </span>
        )}
        <div className="page-actions">
          <button className="btn brand new-card-btn" type="button" onClick={() => openCreator('todo')}>
            <Icon name="plus" sw={2} />
            新建卡片<kbd>N</kbd>
          </button>
        </div>
      </div>
      {board && <FlowBar key={`f${generation}`} board={board} path={path} f={f} />}
      <ColTabs board={board} f={f} boardRef={boardRef} />
      <div className={cx('board flow-scroll scroll', drag && 'dragging')} ref={boardRef} aria-label="白板" key={generation}>
        {board && (
          <Magnet
            path={path}
            statuses={f.list}
            render={(s, h) => {
              const i = f.list.indexOf(s);
              return (
                <Column
                  col={s}
                  index={i}
                  f={f}
                  ids={cols?.[s.key] ?? []}
                  board={board}
                  labels={labels}
                  filter={filter}
                  detailId={detailId}
                  drag={drag}
                  flashes={flashes}
                  entering={entering}
                  fresh={fresh}
                  onHead={h.onMove}
                  resizer={h.resizer}
                />
              );
            }}
          />
        )}
      </div>
    </div>
  );
}

/* ------------------------------ 状态块 ------------------------------ */

interface ColumnProps {
  col: Status;
  index: number;
  f: FlowQ;
  ids: number[];
  board: BoardSnapshot;
  labels: Label[];
  filter: string;
  detailId: number | null;
  drag: DragUI | null;
  flashes: ReturnType<typeof useUI.getState>['flashes'];
  entering: Set<number>;
  fresh: boolean;
  onHead: (e: React.PointerEvent) => void;
  resizer: ReactNode;
}

function emptyText(s: Status, f: FlowQ) {
  if (s.claim) return `${s.name}是空的。新建一张卡片，或等 agent 用 board new 创建`;
  const start = f.list.indexOf(f.claim);
  const target = f.list.slice(start + 1).find(x => x.lock) ?? f.list.find(x => x.lock);
  if (s.lock && target?.key === s.key) return 'agent 执行 board claim 后出现在这里';
  if (s.lock) return '持有者把卡片移过来后出现在这里';
  if (s.final) return '把确认完成的卡片拖到这里';
  return '把卡片拖到这里';
}

function Column({ col, index, f, ids, board, labels, filter, detailId, drag, flashes, entering, fresh, onHead, resizer }: ColumnProps) {
  const count = board.cols[col.key]?.length ?? 0;
  const badge = useRef<HTMLSpanElement>(null);
  const list = useRef<HTMLDivElement>(null);
  useMasonry(list);
  const prev = useRef<number | null>(null);
  useEffect(() => {
    if (prev.current != null && prev.current !== count) {
      badge.current?.style.setProperty('--roll', count > prev.current ? '100%' : '-100%');
      replay(badge.current, 'bump');
    }
    prev.current = count;
  }, [count]);

  const color = statusColor(col.color);
  const over = drag?.over === col.key;
  const same = drag && drag.col === drag.fromCol && col.key === drag.fromCol;
  const firstFree = col.claim ? ids.find(id => id !== drag?.id) : undefined;
  return (
    <section
      className={cx(
        'col',
        count === 0 && !(drag && drag.col === col.key && drag.valid) && 'is-empty',
        col.final && 'is-final',
        drag && over && drag.valid && !same && 'drop-ok',
        drag && over && !drag.valid && 'drop-bad',
        drag && drag.fromCol === col.key && 'drop-home',
      )}
      data-col={col.key}
      data-drop={col.key}
      style={{ ['--c' as string]: color, ['--i' as string]: index + 1 }}
      aria-label={col.name}
    >
      <header className="col-head" onPointerDown={onHead} title="拖动标题摆放位置">
        <span className="col-i">{index + 1}</span>
        <span className="col-dot" />
        <h2 className="col-name">{col.name}</h2>
        {col.claim && (
          <span className="col-tag claim" title="领取起点：agent 从这里领取">
            <Icon name="claim" />
            <span className="t">agent 从这里领</span>
          </span>
        )}
        {col.lock && (
          <span className="col-tag" title="锁定：只能由 agent 领取进入">
            <Icon name="lock" />
            <span className="t">仅领取进入</span>
          </span>
        )}
        <span className="col-count" aria-label="卡片数量" ref={badge}>
          <span>{count}</span>
        </span>
      </header>
      <div className="col-body">
        <div className="col-list" data-list={col.key} ref={list}>
          {ids.map((id, idx) => {
            const c = board.cards[id];
            if (!c) return null;
            return (
              <CardView
                key={id}
                card={drag?.id === id ? { ...c, column: col.key } : c}
                labels={labels}
                filter={filter}
                open={detailId === id}
                placeholder={drag?.id === id}
                flash={flashes[id]}
                entering={entering.has(id)}
                delay={fresh ? 180 + index * 70 + idx * 45 : undefined}
                color={color}
                final={col.final}
                next={firstFree === id}
              />
            );
          })}
        </div>
        <div className="col-empty">
          <Icon name={col.claim ? 'inbox' : col.lock ? 'claim' : col.final ? 'sparkle' : 'move'} />
          <div>{emptyText(col, f)}</div>
        </div>
      </div>
      <Veto drag={over ? drag : null} />
      {resizer}
    </section>
  );
}

export function Veto({ drag }: { drag: DragUI | null }) {
  return (
    <div className="col-veto">
      <div className="veto-icon">
        <Icon name="lock" />
      </div>
      <b>{drag?.veto?.title}</b>
      <span>
        {drag?.veto?.message}
        {drag?.veto && (
          <>
            <br />
            <code>{drag.veto.code}</code>
          </>
        )}
      </span>
    </div>
  );
}

/* ------------------------------ 磁吸布局 ------------------------------ */

const COLS = 12;
const GAP = 12;
type Pos = { x: number; w: number; top: number };

/** 按期望位置从上到下依次放：每块落在它覆盖的栏里已有块的最下沿，块之间不留空隙 */
function pack(keys: string[], tiles: Record<string, Tile>, h: Record<string, number>): Record<string, Pos> {
  const order = [...keys].sort((a, b) => tiles[a].y - tiles[b].y || tiles[a].x - tiles[b].x);
  const out: Record<string, Pos> = {};
  for (const k of order) {
    const t = tiles[k];
    let top = 0;
    for (const [p, o] of Object.entries(out)) {
      if (o.x < t.x + t.w && t.x < o.x + o.w) top = Math.max(top, o.top + (h[p] ?? 0) + GAP);
    }
    out[k] = { x: t.x, w: t.w, top };
  }
  return out;
}

/** 没有保存布局的块：一个都没保存时前 4 个并排各占 3 栏；其余放在当前最矮的那一栏下面 */
function withDefaults(keys: string[], saved: Record<string, Tile>, h: Record<string, number>) {
  const tiles: Record<string, Tile> = {};
  const none = !keys.some(q => saved[q]);
  keys.forEach((k, i) => {
    if (saved[k]) tiles[k] = saved[k];
    else if (none && i < 4) tiles[k] = { x: i * 3, w: 3, y: 0 };
  });
  for (const [i, k] of keys.entries()) {
    if (tiles[k]) continue;
    const placed = Object.keys(tiles);
    const base = pack(placed, tiles, h);
    let best = 0;
    let bestH = Infinity;
    for (let x = 0; x <= COLS - 3; x += 3) {
      const bottom = Math.max(0, ...placed.filter(q => base[q].x < x + 3 && x < base[q].x + base[q].w).map(q => base[q].top + (h[q] ?? 0)));
      if (bottom < bestH) {
        bestH = bottom;
        best = x;
      }
    }
    tiles[k] = { x: best, w: 3, y: 1e6 + i };
  }
  return tiles;
}

function useMobile() {
  const [m, setM] = useState(isMobile);
  useEffect(() => {
    const mq = window.matchMedia('(max-width: 760px)');
    const on = () => setM(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  return m;
}

function Magnet({
  path, statuses, render,
}: {
  path: string;
  statuses: Status[];
  render: (s: Status, h: { onMove: (e: React.PointerEvent) => void; resizer: ReactNode }) => ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const mobile = useMobile();
  const [width, setWidth] = useState(1000);
  const [heights, setHeights] = useState<Record<string, number>>({});
  // 拖动中的预览 / 刚保存、等服务端确认的布局
  const [live, setLive] = useState<Record<string, Tile> | null>(null);
  const [drag, setDrag] = useState<{ key: string; left: number; top: number; mode: 'move' | 'size' } | null>(null);
  const els = useRef<Record<string, HTMLElement | null>>({});
  const keys = statuses.map(s => s.key);
  const saved = useMemo(() => Object.fromEntries(statuses.filter(s => s.layout).map(s => [s.key, s.layout!])), [statuses]);

  const tiles = withDefaults(keys, live ?? saved, heights);
  const col = (width + GAP) / COLS;
  const pos = pack(keys, tiles, heights);
  const bottom = Math.max(0, ...keys.map(k => (pos[k]?.top ?? 0) + (heights[k] ?? 0)));

  useLayoutEffect(() => {
    const el = ref.current!;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);
  useLayoutEffect(() => {
    const ro = new ResizeObserver(es => {
      setHeights(h => {
        let n = h;
        for (const e of es) {
          const k = (e.target as HTMLElement).dataset.tile!;
          const v = (e.target as HTMLElement).offsetHeight;
          if (n[k] !== v) n = { ...n, [k]: v };
        }
        return n;
      });
    });
    for (const k of keys) if (els.current[k]) ro.observe(els.current[k]!);
    return () => ro.disconnect();
  }, [keys.join()]); // eslint-disable-line react-hooks/exhaustive-deps

  /** 松手：把每块的 y 对齐到实际位置后保存，下次拖动从这里排先后 */
  const commit = (all: Record<string, Tile>) => {
    const p = pack(keys, all, heights);
    const next = Object.fromEntries(keys.map(k => [k, { ...all[k], y: p[k].top }]));
    setLive(next);
    api
      .flowLayout(path, next)
      .then(() => refresh())
      .catch(toastError)
      .finally(() => setLive(null));
  };

  const onMove = (k: string) => (e: React.PointerEvent) => {
    if (mobile || e.button !== 0 || (e.target as HTMLElement).closest('button, input, .card')) return;
    e.preventDefault();
    const p = pos[k];
    const x0 = e.clientX;
    const y0 = e.clientY;
    const left0 = p.x * col;
    const top0 = p.top;
    let started = false;
    let cur = { ...tiles };
    const mv = (ev: PointerEvent) => {
      const dx = ev.clientX - x0;
      const dy = ev.clientY - y0;
      if (!started && Math.hypot(dx, dy) < 4) return;
      started = true;
      const left = left0 + dx;
      const top = top0 + dy;
      const x = Math.max(0, Math.min(COLS - tiles[k].w, Math.round(left / col)));
      // 用块的上沿和别的块比先后：往上拖过别人的一半就排到它前面
      cur = { ...cur, [k]: { ...tiles[k], x, y: top + 1 } };
      for (const q of keys) if (q !== k) cur[q] = { ...cur[q], y: pos[q].top + (heights[q] ?? 0) / 2 };
      setLive(cur);
      setDrag({ key: k, left, top, mode: 'move' });
    };
    const up = () => {
      removeEventListener('pointermove', mv);
      removeEventListener('pointerup', up);
      document.body.classList.remove('tile-moving');
      if (started) commit(cur);
      setDrag(null);
    };
    document.body.classList.add('tile-moving');
    addEventListener('pointermove', mv);
    addEventListener('pointerup', up);
  };

  const onSize = (k: string) => (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const w0 = tiles[k].w * col;
    let cur = { ...tiles };
    let changed = false;
    document.body.classList.add('tile-sizing');
    const mv = (ev: PointerEvent) => {
      const w = Math.max(2, Math.min(COLS - tiles[k].x, Math.round((w0 + ev.clientX - x0) / col)));
      changed = w !== tiles[k].w;
      cur = { ...cur, [k]: { ...tiles[k], w } };
      for (const q of keys) cur[q] = { ...cur[q], y: pos[q].top };
      setLive(cur);
      setDrag({ key: k, left: 0, top: 0, mode: 'size' });
    };
    const up = () => {
      document.body.classList.remove('tile-sizing');
      removeEventListener('pointermove', mv);
      removeEventListener('pointerup', up);
      if (changed) commit(cur);
      else setLive(null);
      setDrag(null);
    };
    addEventListener('pointermove', mv);
    addEventListener('pointerup', up);
  };

  const moving = drag?.mode === 'move' ? drag : null;
  return (
    <div className={cx('magnet', moving && 'moving', mobile && 'stacked')} ref={ref} style={mobile ? undefined : { height: bottom + 4 }}>
      {moving && pos[moving.key] && (
        <div
          className="tile-ghost"
          style={{
            width: pos[moving.key].w * col - GAP,
            height: heights[moving.key],
            transform: `translate(${pos[moving.key].x * col}px, ${pos[moving.key].top}px)`,
          }}
        />
      )}
      {statuses.map(s => {
        const p = pos[s.key];
        const lifted = moving?.key === s.key;
        return (
          <div
            key={s.key}
            data-tile={s.key}
            ref={el => {
              els.current[s.key] = el;
            }}
            className={cx('tile', lifted && 'lifted', drag?.key === s.key && drag.mode === 'size' && 'sizing')}
            style={
              mobile || !p
                ? undefined
                : {
                    width: p.w * col - GAP,
                    transform: lifted ? `translate(${moving!.left}px, ${moving!.top}px) rotate(.6deg) scale(1.015)` : `translate(${p.x * col}px, ${p.top}px)`,
                  }
            }
          >
            {render(s, {
              onMove: onMove(s.key),
              resizer: mobile ? null : <span className="tile-resize" title="拖动调整宽度" onPointerDown={onSize(s.key)} />,
            })}
          </div>
        );
      })}
    </div>
  );
}

/* ------------------------------ 工作流条 ------------------------------ */

function FlowBar({ board, path, f }: { board: BoardSnapshot; path: string; f: FlowQ }) {
  const [menu, setMenu] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [lift, setLift] = useState<string | null>(null);
  const [order, setOrder] = useState<string[] | null>(null);
  const els = useRef<Record<string, HTMLElement | null>>({});
  const dragged = useRef(false);
  const list = order ? order.map(k => f.get(k)).filter((s): s is Status => !!s) : f.list;
  const hasLayout = f.list.some(s => s.layout);

  const startDrag = (k: string) => (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const x0 = e.clientX;
    dragged.current = false;
    let cur = list.map(s => s.key);
    const mv = (ev: PointerEvent) => {
      if (!dragged.current) {
        if (Math.abs(ev.clientX - x0) < 4) return;
        dragged.current = true;
        setLift(k);
        setMenu(null);
      }
      const others = cur.filter(x => x !== k);
      let idx = 0;
      for (const o of others) {
        const n = els.current[o]?.getBoundingClientRect();
        if (n && n.left + n.width / 2 < ev.clientX) idx++;
      }
      const next = [...others.slice(0, idx), k, ...others.slice(idx)];
      if (next.join() !== cur.join()) {
        cur = next;
        setOrder(next);
      }
    };
    const up = () => {
      setLift(null);
      removeEventListener('pointermove', mv);
      removeEventListener('pointerup', up);
      if (dragged.current && cur.join() !== f.list.map(s => s.key).join()) {
        api
          .reorderStatuses(path, cur)
          .then(() => refresh())
          .catch(toastError)
          .finally(() => setOrder(null));
      } else setOrder(null);
    };
    addEventListener('pointermove', mv);
    addEventListener('pointerup', up);
  };

  return (
    <div className="wf-bar">
      <span className="wf-label">工作流</span>
      <div className="wf-line">
        {list.map((s, i) => (
          <span key={s.key} className="wf-seg">
            {i > 0 && <span className="wf-arrow" aria-hidden="true" />}
            <span className="wf-node-wrap">
              <button
                type="button"
                ref={el => {
                  els.current[s.key] = el;
                }}
                className={cx('wf-node', lift === s.key && 'lift', menu === s.key && 'open')}
                style={{ ['--c' as string]: statusColor(s.color) }}
                onPointerDown={startDrag(s.key)}
                onClick={() => {
                  if (dragged.current) return;
                  setMenu(m => (m === s.key ? null : s.key));
                }}
                title="点击设置 · 拖动调整顺序"
              >
                <i className="wf-dot" />
                <span className="wf-name">{s.name}</span>
                <b>{board.cols[s.key]?.length ?? 0}</b>
                {s.claim && (
                  <span className="wf-glyph" title="领取起点：agent 从这里领取">
                    <Icon name="claim" />
                  </span>
                )}
                {s.lock && (
                  <span className="wf-glyph" title="锁定：只能由 agent 领取进入">
                    <Icon name="lock" />
                  </span>
                )}
                {s.final && (
                  <span className="wf-glyph" title="终点：进来算完成，只有人能移入移出">
                    <Icon name="check" />
                  </span>
                )}
              </button>
              {menu === s.key && <StatusMenu s={s} f={f} path={path} count={board.cols[s.key]?.length ?? 0} onClose={() => setMenu(null)} />}
            </span>
          </span>
        ))}
        <span className="wf-seg">
          <span className="wf-arrow dashed" aria-hidden="true" />
          {adding ? (
            <span className="wf-node add editing">
              <input
                autoFocus
                className="wf-input"
                placeholder="状态名，回车添加"
                maxLength={12}
                onBlur={() => setAdding(false)}
                onKeyDown={e => {
                  const t = e.target as HTMLInputElement;
                  if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                    const v = t.value.trim();
                    if (!v) return;
                    api
                      .createStatus(path, v)
                      .then(() => {
                        t.value = '';
                        return refresh();
                      })
                      .catch(err => {
                        toastError(err);
                        replay(t.parentElement, 'invalid');
                      });
                  }
                  if (e.key === 'Escape') {
                    e.stopPropagation();
                    setAdding(false);
                  }
                }}
              />
            </span>
          ) : (
            <button type="button" className="wf-node add" onClick={() => setAdding(true)}>
              + 添加状态
            </button>
          )}
        </span>
      </div>
      {hasLayout && (
        <button type="button" className="wf-reset" title="这块白板的状态块回到默认布局" onClick={() => run(api.flowLayout(path, {}))}>
          复原布局
        </button>
      )}
    </div>
  );
}

function StatusMenu({ s, f, path, count, onClose }: { s: Status; f: FlowQ; path: string; count: number; onClose: () => void }) {
  const [confirming, setConfirming] = useState(false);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const down = (e: PointerEvent) => {
      if (!(e.target as HTMLElement).closest('.wf-menu, .wf-node')) close.current();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        close.current();
      }
    };
    addEventListener('pointerdown', down);
    addEventListener('keydown', key, true);
    return () => {
      removeEventListener('pointerdown', down);
      removeEventListener('keydown', key, true);
    };
  }, []);

  const patch = (p: Parameters<typeof api.updateStatus>[2]) => run(api.updateStatus(path, s.key, p));
  const rename = (v: string) => {
    const n = v.trim();
    if (n && n !== s.name) patch({ name: n });
  };
  const sw = (on: boolean, label: string, hint: string, set: () => void, locked?: string) => (
    <button type="button" className={cx('wf-sw', on && 'on')} onClick={locked ? undefined : set} aria-pressed={on} aria-disabled={!!locked}>
      <span className="wf-sw-track">
        <i />
      </span>
      <span className="wf-sw-text">
        <b>{label}</b>
        <span>{locked || hint}</span>
      </span>
    </button>
  );

  return (
    <div className="wf-menu" style={{ ['--c' as string]: statusColor(s.color) }} onPointerDown={e => e.stopPropagation()}>
      <input
        className="wf-menu-name"
        defaultValue={s.name}
        maxLength={12}
        autoFocus
        onFocus={e => e.target.select()}
        onBlur={e => rename(e.target.value)}
        onKeyDown={e => {
          if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
            rename((e.target as HTMLInputElement).value);
            onClose();
          }
        }}
        aria-label="状态名"
      />
      <div className="wf-key">
        标识 <code>{s.key}</code>
        <span>CLI 里写标识或名称都可以</span>
      </div>
      <div className="wf-swatches">
        {STATUS_COLORS.map(c => (
          <button
            key={c}
            type="button"
            className={cx(c === s.color && 'on')}
            style={{ ['--sc' as string]: statusColor(c) }}
            onClick={() => c !== s.color && patch({ color: c })}
            aria-label="颜色"
          />
        ))}
      </div>
      <div className="wf-rules">
        {sw(
          s.claim,
          '领取起点',
          'board claim 从这里领；新建、超时释放、强制释放都回到这里',
          () => patch({ claim: true }),
          s.claim ? '只能有一个：打开另一个状态的这个开关即可换过去' : undefined,
        )}
        {sw(s.lock, '锁定', '只能由 agent 领取进入；被持有期间只有持有者能移动', () => patch({ lock: !s.lock }))}
        {sw(s.final, '终点', '进来算完成：卡片只占一行；只有人能移入移出', () => patch({ final: !s.final }))}
      </div>
      <div className="wf-menu-foot">
        {s.claim ? (
          <span className="wf-note">领取起点不能删除</span>
        ) : confirming ? (
          <button
            type="button"
            className="wf-del sure"
            onClick={() => {
              onClose();
              run(api.deleteStatus(path, s.key));
            }}
          >
            确认删除{count ? `，${count} 张卡片移到「${f.claim?.name}」` : ''}
          </button>
        ) : (
          <button type="button" className="wf-del" onClick={() => setConfirming(true)}>
            删除状态
          </button>
        )}
      </div>
    </div>
  );
}

/* ------------------------------ 手机：状态切换条 ------------------------------ */

function ColTabs({ board, f, boardRef }: { board: BoardSnapshot | null; f: FlowQ; boardRef: React.RefObject<HTMLDivElement | null> }) {
  const go = (key: string) => {
    const el = boardRef.current?.querySelector<HTMLElement>(`[data-tile="${key}"]`);
    el?.scrollIntoView({ block: 'start', behavior: reduceMotion() ? 'auto' : 'smooth' });
  };
  return (
    <nav className="col-tabs" aria-label="状态">
      {f.list.map(s => (
        <button key={s.key} className="ct" type="button" data-col={s.key} style={{ ['--c' as string]: statusColor(s.color) }} onClick={() => go(s.key)}>
          <i />
          {s.name}
          <span className="n">{board?.cols[s.key]?.length ?? 0}</span>
        </button>
      ))}
    </nav>
  );
}
