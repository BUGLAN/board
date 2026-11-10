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
import { flowOf, INBOX, statusColor, viewOf, type FlowQ } from '../lib/schema';
import { shownCols, sortKey, sortName, type SortSpec } from '../lib/sort';
import type { BoardSnapshot, Label, Status, Tile } from '../lib/types';
import { cx, isMobile, reduceMotion, replay } from '../lib/util';
import { colCount, loadFull, omittedIn, refresh, serverNow, useCurrent, useData } from '../store/data';
import { clearLabelSel, closeCtx, openCreator, openCtx, setBoardSheet, setFilter, setSort, toastError, useUI, type DragUI } from '../store/ui';
import { cardHit, CardView } from './Card';
import { FlowBar } from './FlowBar';
import { Icon } from './Icon';
import { Skeleton, SkeletonCard } from './Skeleton';

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


/** 白板只纵向滚动（overflow-x: hidden），人横向滚不回来。右侧磁贴的悬停卡片之类会让它有一点横向溢出，
    这时浏览器拖动时的自动滚动、scrollIntoView、聚焦都可能把它横向滚开，整块内容左移、只能切换 Tab 恢复。被滚开就立刻拉回 */
function keepLeft(e: React.UIEvent<HTMLElement>) {
  if (e.currentTarget.scrollLeft) e.currentTarget.scrollLeft = 0;
}

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
  const sorts = useUI(s => s.sorts);
  // 手机：上面已有状态切换条，工作流图默认收起成一行入口
  const mobile = useMobile();
  const [flowOpen, setFlowOpen] = useState(false);
  // 排序中的块按排序依据显示（只改显示顺序）；拖拽预览的位置是「看到的顺序」里的位置
  const shown = useMemo(() => (board ? shownCols(board, sorts, labels, serverNow()) : null), [board, sorts, labels]);
  const cols = useMemo(() => (shown ? previewCols(shown, drag) : null), [shown, drag]);
  // 快照还没到（首次打开、切换白板）：工作区里已经有这块白板的工作流和计数，先按它摆好状态块，卡片位置放骨架
  const f = flowOf(board?.statuses ?? cur?.board.statuses);
  const counts = useMemo(
    () => (board ? Object.fromEntries(Object.keys(board.cols).map(k => [k, colCount(board, k)])) : cur?.board.counts ?? {}),
    [board, cur?.board.counts],
  );
  const path = cur?.board.path ?? '';
  // 「下一张」按优先级顺序（不受排序影响），去掉正在拖的卡片再算
  const prio = board?.cols ?? null;
  // 有排队派发的卡片不会被不带 ID 的 board claim 领走
  const queued = useData(s => s.ws?.dispatches);
  const urgentLabels = (cur?.project.labels ?? []).filter(l => l.priority).map(l => l.id);
  const nextId = f.nextCard(
    prio && drag ? Object.fromEntries(Object.entries(prio).map(([k, v]) => [k, v.filter(id => id !== drag.id)])) : prio,
    id => !!queued?.some(d => d.state === 'queued' && d.board === path && d.card === id),
    id => urgentLabels.length > 0 && !!board?.cards[id]?.labels.some(l => urgentLabels.includes(l)),
  );

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
  // 终点状态默认只带最近的几张：筛选、排序时取全量，免得找不到、排不到更早的卡片
  const truncated = !!board && Object.values(board.omitted ?? {}).some(Boolean);
  const sortsTruncated = !!board && Object.keys(board.omitted ?? {}).some(k => omittedIn(board, k) > 0 && sorts[sortKey(path, k)]);
  useEffect(() => {
    if (truncated && (filter || labelSel.length > 0 || sortsTruncated)) void loadFull();
  }, [truncated, filter, labelSel, sortsTruncated]);
  // 桌面页头只提示筛选框里的文字（侧栏选中的标签在胶囊上已经高亮）；手机上侧栏在抽屉里，选中的标签也写进页头
  const pickedLabels = mobile ? labels.filter(l => labelSel.includes(l.id)).map(l => l.name) : [];
  const filtering = !!filter || pickedLabels.length > 0;
  const n = filtering && board ? Object.values(board.cards).filter(c => c.column !== INBOX && cardHit(c, filter, labelSel, labels)).length : 0;

  return (
    <div className="page page-board" data-page="board">
      <div className="page-head">
        {/* 项目只在顶栏出现一次，页头只写白板名 */}
        <nav className="crumbs" aria-label="位置">
          <b>{cur?.board.name}</b>
        </nav>
        {filtering && (
          <span className="filter-pill">
            {`筛选${filter ? `「${filter}」` : ''}${pickedLabels.map(l => `「${l}」`).join('')}· ${n} 张`}
            <button
              type="button"
              aria-label="清除筛选"
              onClick={() => {
                setFilter('');
                if (mobile) clearLabelSel();
              }}
            >
              ×
            </button>
          </span>
        )}
        <div className="page-actions">
          {mobile && (
            <button
              className={cx('btn filter-btn', filtering && 'on')}
              type="button"
              aria-label="筛选、标签、正在工作、最近动态"
              onClick={() => setBoardSheet(true)}
            >
              <Icon name="filter" />
            </button>
          )}
          <button
            className="btn prompts-btn"
            type="button"
            aria-haspopup="menu"
            aria-label="复制给 agent 的提示词"
            onClick={e => {
              const open = useUI.getState().ctx;
              if (open?.kind === 'prompts') return closeCtx();
              openCtx({ kind: 'prompts', id: path, x: 0, y: 0, anchor: e.currentTarget.getBoundingClientRect() });
            }}
          >
            <Icon name="prompt" />
            提示词
          </button>
          <button className="btn brand new-card-btn" type="button" onClick={() => openCreator('todo')}>
            <Icon name="plus" sw={2} />
            新建卡片<kbd>N</kbd>
          </button>
        </div>
      </div>
      <ColTabs board={board} f={f} boardRef={boardRef} />
      <div
        className={cx('board flow-scroll', drag && 'dragging')}
        ref={boardRef}
        onScroll={keepLeft}
        aria-label="白板"
        aria-busy={!board || undefined}
        key={generation}
      >
        {/* 工作流条放在滚动区域里：随白板内容一起滚走，不常驻顶部、不压在卡片上 */}
        {cur && mobile && (
          <button type="button" className="wf-toggle" aria-expanded={flowOpen} onClick={() => setFlowOpen(!flowOpen)}>
            <Icon name="board" />
            工作流 · {f.list.length} 个状态
            <Icon name={flowOpen ? 'chevUp' : 'chevDown'} />
          </button>
        )}
        {cur && (!mobile || flowOpen) && <FlowBar key={`f${generation}`} counts={counts} path={path} f={f} />}
        {!board && f.list.length > 0 && (
          <Magnet path={path} statuses={f.list} render={s => <SkeletonColumn col={s} index={f.list.indexOf(s)} f={f} count={counts[s.key] ?? 0} />} />
        )}
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
                  next={nextId}
                  sort={sorts[sortKey(path, s.key)]}
                  sortId={sortKey(path, s.key)}
                />
              );
            }}
          />
        )}
      </div>
    </div>
  );
}

/* ------------------------------ 骨架 ------------------------------ */

/** 状态块骨架：头部和真实状态块一样（名称、颜色、规则、数量），卡片位置按形态和数量放骨架（卡片最多 3 张、单行 4 行、磁贴 12 个）。
    fake：首屏还不知道真实数量，数量也用骨架 */
function SkeletonColumn({ col, index, f, count, fake }: { col: Status; index: number; f: FlowQ; count: number; fake?: boolean }) {
  const view = viewOf(col);
  const n = Math.min(count, view === 'tile' ? 12 : view === 'line' ? 4 : 3);
  return (
    <section
      className={cx('col sk-col', count === 0 && 'is-empty', col.final && 'is-final')}
      style={{ ['--c' as string]: statusColor(col.color), ['--i' as string]: index + 1 }}
      aria-label={col.name}
    >
      <header className="col-head">
        <span className="col-i">{index + 1}</span>
        <span className="col-dot" />
        <h2 className="col-name">{col.name}</h2>
        {col.claim && (
          <span className="col-tag claim">
            <Icon name="claim" />
            <span className="t">agent 从这里领</span>
          </span>
        )}
        {col.lock && (
          <span className="col-tag">
            <Icon name="lock" />
            <span className="t">仅领取进入</span>
          </span>
        )}
        {fake ? (
          <Skeleton className="sk-count" w={26} h={22} r={11} />
        ) : (
          <span className="col-count">
            <span>{count}</span>
          </span>
        )}
      </header>
      <div className="col-body">
        <div className={cx('col-list', `view-${view}`)} style={{ ['--tile-ch' as string]: 2 }}>
          {Array.from({ length: n }, (_, i) =>
            view === 'tile' ? (
              <Skeleton key={i} className="sk-tile" h={24} r={6} />
            ) : view === 'line' ? (
              <div key={i} className="sk-done" aria-hidden="true">
                <Skeleton w={14} h={14} r={4} />
                <Skeleton w={['70%', '54%', '62%', '48%'][i]} h={10} />
              </div>
            ) : (
              <SkeletonCard key={i} seed={index + i} />
            ),
          )}
        </div>
        <div className="col-empty">
          <Icon name={col.claim ? 'inbox' : col.lock ? 'claim' : col.final ? 'sparkle' : 'move'} />
          <div>{emptyText(col, f)}</div>
        </div>
      </div>
    </section>
  );
}

/** 默认工作流（还没有工作区时的首屏骨架用） */
const DEFAULT_FLOW: Status[] = [
  { key: 'todo', name: '待办', color: 'todo', claim: true, lock: false, final: false, layout: null, to: ['doing'] },
  { key: 'doing', name: '进行中', color: 'doing', claim: false, lock: true, final: false, layout: null, to: ['review'] },
  { key: 'review', name: '待审核', color: 'review', claim: false, lock: false, final: false, layout: null, to: ['done'] },
  { key: 'done', name: '已完成', color: 'done', claim: false, lock: false, final: true, layout: null, to: [] },
];
const DEFAULT_COUNTS: Record<string, number> = { todo: 3, doing: 1, review: 2, done: 4 };

/** 首屏骨架：工作区还没到时代替整个白板页（页头、工作流条、4 个默认状态块） */
export function BoardSkeleton() {
  const f = flowOf(DEFAULT_FLOW);
  return (
    <div className="page page-board" data-page="board" aria-busy="true" aria-label="正在打开白板">
      <div className="page-head">
        <div className="sk-crumbs">
          <Skeleton w={64} h={12} />
          <Skeleton w={88} h={16} />
        </div>
        <div className="page-actions">
          <Skeleton w={78} h={34} r={10} />
          <Skeleton w={112} h={34} r={10} />
        </div>
      </div>
      <div className="board flow-scroll">
        <div className="wf-bar sk-flow" aria-hidden="true">
          <span className="wf-label">工作流</span>
          <div className="sk-pills">
            {[64, 72, 72, 68].map((w, i) => (
              <Skeleton key={i} w={w} h={26} r={8} />
            ))}
          </div>
        </div>
        <Magnet path="" statuses={DEFAULT_FLOW} render={s => <SkeletonColumn col={s} index={f.list.indexOf(s)} f={f} count={DEFAULT_COUNTS[s.key]} fake />} />
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
  /** 不带 ID 执行 board claim 时会被领走的卡片 */
  next: number | undefined;
  /** 这块的一次性排序（没有时按优先级） */
  sort: SortSpec | undefined;
  sortId: string;
}

function emptyText(s: Status, f: FlowQ) {
  if (s.claim && s.key === f.home?.key) return `${s.name}是空的。新建一张卡片，或等 agent 用 board new 创建`;
  if (s.claim) return `${s.name}是空的。agent 也会从这里领取`;
  if (s.lock && f.claimables.some(c => f.claimTarget(c.key)?.key === s.key)) return 'agent 执行 board claim 后出现在这里';
  if (s.lock) return '持有者把卡片移过来后出现在这里';
  if (s.final) return '把确认完成的卡片拖到这里';
  return '把卡片拖到这里';
}

function Column({ col, index, f, ids, board, labels, filter, detailId, drag, flashes, entering, fresh, onHead, resizer, next, sort, sortId }: ColumnProps) {
  const count = colCount(board, col.key);
  const earlier = omittedIn(board, col.key);
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
  const firstFree = next;
  const view = viewOf(col);
  // 磁贴按这一块里编号最多几位定宽，同一块里的磁贴一样宽
  const listStyle = view === 'tile' ? { ['--tile-ch' as string]: String(Math.max(1, ...ids)).length } : undefined;
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
          <span className="col-tag claim" title="可领取：agent 从这里领取">
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
        <SortButton col={col} sort={sort} sortId={sortId} />
        <span className="col-count" aria-label="卡片数量" ref={badge}>
          <span>{count}</span>
        </span>
      </header>
      <div className="col-body">
        {earlier > 0 && <EarlierButton n={earlier} />}
        <div className={cx('col-list', `view-${view}`)} data-list={col.key} ref={list} style={listStyle}>
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
                view={view}
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

/** 终点状态只带了最近的几张：上面一行「更早的 M 张」，点开取全量（这个窗口里这块白板之后都带全量） */
function EarlierButton({ n }: { n: number }) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className={cx('btn sm col-earlier', busy && 'is-busy')}
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        try {
          await loadFull();
        } catch (e) {
          toastError(e);
        } finally {
          setBusy(false);
        }
      }}
    >
      <Icon name="chevUp" />
      更早的 {n} 张
    </button>
  );
}

/** 标题栏的排序入口：没排序时是一个图标；排序中显示按什么排、方向，旁边可以取消 */
function SortButton({ col, sort, sortId }: { col: Status; sort: SortSpec | undefined; sortId: string }) {
  const dir = sort && sort.by !== 'labels' ? (sort.desc ? '↓' : '↑') : '';
  const open = useUI(s => s.ctx?.kind === 'sort' && s.ctx.id === col.key);
  return (
    <span className={cx('col-sort', sort && 'on')}>
      <button
        type="button"
        className="col-sort-btn"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={sort ? `「${col.name}」按${sortName(sort)}${dir === '↓' ? '降序' : dir ? '升序' : ''}显示，点开更改` : `排序「${col.name}」的卡片（只改显示顺序）`}
        data-tip={sort ? '只改显示顺序；在这块里拖动会按看到的顺序保存为优先级' : '排序（只改显示顺序）'}
        onClick={e => {
          const open = useUI.getState().ctx;
          if (open?.kind === 'sort' && open.id === col.key) return closeCtx();
          openCtx({ kind: 'sort', id: col.key, x: 0, y: 0, anchor: e.currentTarget.getBoundingClientRect() });
        }}
      >
        <Icon name="sort" />
        {sort && (
          <span className="t">
            {sortName(sort)}
            {dir && <i>{dir}</i>}
          </span>
        )}
      </button>
      {sort && (
        <button type="button" className="col-sort-x" aria-label="取消排序，回到优先级顺序" data-tip="回到优先级顺序" onClick={() => setSort(sortId, null)}>
          <Icon name="close" sw={2} />
        </button>
      )}
    </span>
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
export function pack(keys: string[], tiles: Record<string, Tile>, h: Record<string, number>): Record<string, Pos> {
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

/**
 * 没有保存布局的块：位置只由它在工作流里的顺序决定，不看块的高度（卡片数一变就换栏，人记不住位置）。
 * 第 i 个状态放在第 i mod 4 栏（每栏 3 格）；一个都没保存时前 4 个并排在最上面，其余按顺序接在下面，上下位置由 pack 贴紧。
 */
export function withDefaults(keys: string[], saved: Record<string, Tile>) {
  const tiles: Record<string, Tile> = {};
  const none = !keys.some(q => saved[q]);
  keys.forEach((k, i) => {
    tiles[k] = saved[k] ?? { x: (i % 4) * 3, w: 3, y: none && i < 4 ? 0 : 1e6 + i };
  });
  return tiles;
}

export function useMobile() {
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

  const tiles = withDefaults(keys, live ?? saved);
  const col = (width + GAP) / COLS;
  const pos = pack(keys, tiles, heights);
  const bottom = Math.max(0, ...keys.map(k => (pos[k]?.top ?? 0) + (heights[k] ?? 0)));

  // 切到别的 Tab 时白板页 display: none，量出来是 0：不记，切回来时布局还是原样，不会先挤成一团再展开（抖动）
  useLayoutEffect(() => {
    const el = ref.current!;
    const measure = () => {
      if (el.clientWidth) setWidth(el.clientWidth);
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);
  useLayoutEffect(() => {
    const ro = new ResizeObserver(es => {
      setHeights(h => {
        let n = h;
        for (const e of es) {
          const el = e.target as HTMLElement;
          const k = el.dataset.tile!;
          const v = el.offsetHeight;
          // 隐藏时（offsetParent 为空）量出来是 0：不记
          if (!v && !el.offsetParent) continue;
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

/* ------------------------------ 手机：状态切换条 ------------------------------ */

function ColTabs({ board, f, boardRef }: { board: BoardSnapshot | null; f: FlowQ; boardRef: React.RefObject<HTMLDivElement | null> }) {
  const go = (key: string) => {
    const el = boardRef.current?.querySelector<HTMLElement>(`[data-tile="${key}"]`);
    el?.scrollIntoView({ block: 'start', behavior: reduceMotion() ? 'auto' : 'smooth' });
  };
  return (
    <nav className="col-tabs" aria-label="状态" data-scrollbar="none">
      {f.list.map(s => (
        <button key={s.key} className="ct" type="button" data-col={s.key} style={{ ['--c' as string]: statusColor(s.color) }} onClick={() => go(s.key)}>
          <i />
          {s.name}
          <span className="n">{board ? colCount(board, s.key) : 0}</span>
        </button>
      ))}
    </nav>
  );
}
