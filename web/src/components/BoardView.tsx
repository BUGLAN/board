/* ==========================================================================
   白板 Tab 的主内容区：页头 + 工作流条 + 状态块（PRD §10.3）
   - 工作流条：状态的顺序、卡片数和规则标记；拖动排序，点开设置（名称、颜色、规则、删除），末尾新增
   - 状态块：磁吸布局。拖标题摆放，吸附到 12 栏网格并向上贴紧；拖右边缘调宽。布局存进数据库、所有窗口同步
   - 卡片位置变化用 FLIP 补间；新卡片弹入；通过 CLI 或其他窗口做的修改：卡片闪光 + 角标说明是谁做的
   - 手机：状态块上下排列，顶部有状态切换条
   ========================================================================== */
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactElement, type ReactNode } from 'react';
import { flushMasonry, useMasonry } from '../lib/masonry';
import { groupRuns, sessionOrder, type SessGroup } from '../lib/sessionGroups';
import { setShownCols } from '../lib/drag';
import { api } from '../lib/api';
import * as flip from '../lib/flip';
import { flowOf, INBOX, statusColor, viewOf, type FlowQ } from '../lib/schema';
import { statusRoles } from '../lib/review';
import { orderDir, orderName, orderOf, orderOption, shownCols } from '../lib/sort';
import type { BoardSnapshot, Label, Status, StatusView, Tile } from '../lib/types';
import { cx, isMobile, reduceMotion, replay } from '../lib/util';
import { colCount, flowKey, loadFull, omittedIn, optimistic, renderKey, serverNow, setEarlier, useBusy, useCurrent, useData, useEarlier } from '../store/data';
import { collapseCols, FINAL_RECENT, type Earlier } from '../lib/earlier';
import { clearLabelSel, closeCtx, openBoardSettings, openCreator, openCtx, openReview, setBoardSheet, setFilter, toastError, useUI, type DragUI } from '../store/ui';
import { Tip } from './bits';
import { BoardSettings } from './BoardSettings';
import { CardCronDialog } from './CardCrons';
import { cardHit, CardView, TilePeek } from './Card';
import { FirstRunHint } from './FirstRun';
import { FlowBar } from './FlowBar';
import { Icon } from './Icon';
import { Skeleton, SkeletonCard } from './Skeleton';
import { BoardBulkBar, ColSelect } from './BoardSelect';
import { clearSelect } from '../lib/boardSelect';

/** 拖拽预览：被拖的卡片放到预览位置。landing：已经松手、还在落位的卡片（被下一次拖动顶出 drag 的，store/ui 的 startDrag），
    先按它们各自的落点放好，再放正在拖的那张；不在这块白板上的（落位时换了白板）跳过。
    没受影响的状态（被拖的卡片原来不在、占位现在也不在）沿用原来的数组：状态块（memo）按引用比较，不用跟着重画（default/main#631） */
export function previewCols(cols: BoardSnapshot['cols'], drag: DragUI | null, landing: DragUI[] = NO_LANDING) {
  if (!drag && !landing.length) return cols;
  const out = { ...cols } as BoardSnapshot['cols'];
  const place = (m: DragUI) => {
    for (const [k, v] of Object.entries(out)) if (v.includes(m.id)) out[k] = v.filter(id => id !== m.id);
    const to = [...(out[m.col] ?? [])];
    to.splice(Math.min(m.index, to.length), 0, m.id);
    out[m.col] = to;
  };
  for (const m of landing) if (m.id !== drag?.id && Object.values(out).some(v => v.includes(m.id))) place(m);
  if (drag) place(drag);
  return out;
}
const NO_LANDING: DragUI[] = [];

/** 落位中的卡片按落点所在的状态分组（状态块据此把它们画成占位）；没有的状态不在里面，状态块拿到的是 undefined、不用重画 */
export function landingByCol(landing: DragUI[]): Record<string, number[]> {
  const out: Record<string, number[]> = {};
  for (const m of landing) (out[m.col] ??= []).push(m.id);
  return out;
}

/** 这次拖动和这个状态块有没有关系：指针在它上面、占位在它里面、卡片是从它拖出来的。
    没关系的块拿到的 drag 是 null（它画出来的样子和没在拖动时一样），占位每换一次位置不用重画 */
export const dragFor = (drag: DragUI | null, col: string) => (drag && (drag.over === col || drag.col === col || drag.fromCol === col) ? drag : null);

/** 批量选择只属于当前白板 Tab：换白板、离开白板页、组件卸载时退出 */
export function useBoardSelectionScope(path: string) {
  const tab = useUI(s => s.tab);
  useEffect(() => clearSelect, [path]);
  useEffect(() => {
    if (tab !== 'board') clearSelect();
  }, [tab]);
}

/** 记录每块白板已经出现过的卡片：首次出现的弹入 */
export function useEntering(board: BoardSnapshot | null, generation: number) {
  const seen = useRef<{ gen: number; ids: Set<number>; fresh: boolean }>({ gen: -1, ids: new Set(), fresh: true });
  if (seen.current.gen !== generation) seen.current = { gen: generation, ids: new Set(), fresh: true };
  const s = seen.current;
  // 没有新卡片时（绝大多数渲染）用同一个空集合：状态块（memo）的 props 不因为它每次都是新对象而变
  let entering = NO_ENTERING;
  // 按 renderKey 记：顶替临时卡片的真卡片算已经出现过，不再弹入一次
  if (board && !s.fresh)
    for (const id of Object.keys(board.cards).map(Number))
      if (!s.ids.has(renderKey(id))) {
        if (entering === NO_ENTERING) entering = new Set<number>();
        entering.add(id);
      }
  useEffect(() => {
    if (!board) return;
    for (const id of Object.keys(board.cards).map(Number)) s.ids.add(renderKey(id));
    s.fresh = false;
  });
  return { entering, fresh: !!board && s.fresh };
}
const NO_ENTERING: Set<number> = new Set();


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
  const landing = useUI(s => s.landing);
  const reveal = useUI(s => s.reveal);
  const reject = useUI(s => s.reject);
  const boardRef = useRef<HTMLDivElement>(null);
  const labels = cur?.project.labels ?? NO_LABELS;
  // 交审状态的块标题上「复盘」、需要人处理的状态上「处理」（复盘台）
  const roles = useMemo(() => statusRoles(board?.statuses), [board?.statuses]);
  const { entering, fresh } = useEntering(board, generation);
  // 手机：上面已有状态切换条，工作流图默认收起成一行入口
  const mobile = useMobile();
  const [flowOpen, setFlowOpen] = useState(false);
  // 保留顺序开着时每块按自己的排序依据显示（只改显示顺序）；拖拽预览的位置是「看到的顺序」里的位置
  const shown1 = useMemo(() => (board ? shownCols(board, labels, serverNow()) : null), [board, labels]);
  const labelSel = useUI(s => s.labelSel);
  // 终点状态的「更早的 N 张」：没展开的只显示最近移入的几张（取过全量也一样，lib/earlier）；筛选时更早的只显示命中的
  const earlierOpen = useEarlier(s => s.open);
  const filteringAny = !!filter || labelSel.length > 0;
  const hit = useMemo(() => {
    if (!filteringAny || !board) return null;
    const cards = board.cards;
    return (id: number) => !!cards[id] && cardHit(cards[id], filter, labelSel, labels);
  }, [filteringAny, board, filter, labelSel, labels]);
  const cut = useMemo(() => (board && shown1 ? collapseCols(board, shown1, earlierOpen, hit) : null), [board, shown1, earlierOpen, hit]);
  const shown0 = cut?.cols ?? null;
  // 同一会话的卡挨着放、框在一起（lib/sessionGroups）：拖拽预览、落点都按这个看到的顺序算
  const dispatches = useData(s => s.ws?.dispatches);
  const boardPath = board?.path ?? '';
  const grouped = useMemo(() => {
    if (!shown0) return null;
    const cols: BoardSnapshot['cols'] = {};
    const groups: Record<number, SessGroup> = {};
    for (const [k, ids] of Object.entries(shown0)) {
      const r = sessionOrder(ids, dispatches, boardPath);
      cols[k] = r.ids;
      Object.assign(groups, r.groups);
    }
    return { cols, groups };
  }, [shown0, dispatches, boardPath]);
  const shown = grouped?.cols ?? null;
  useEffect(() => setShownCols(shown), [shown]);
  const cols = useMemo(() => (shown ? previewCols(shown, drag, landing) : null), [shown, drag, landing]);
  const landed = useMemo(() => landingByCol(landing), [landing]);
  // 快照还没到（首次打开、切换白板）：工作区里已经有这块白板的工作流和计数，先按它摆好状态块，卡片位置放骨架
  const flowStatuses = board?.statuses ?? cur?.board.statuses;
  const f = useMemo(() => flowOf(flowStatuses), [flowStatuses]);
  const counts = useMemo(
    () => (board ? Object.fromEntries(Object.keys(board.cols).map(k => [k, colCount(board, k)])) : cur?.board.counts ?? {}),
    [board, cur?.board.counts],
  );
  const path = cur?.board.path ?? '';
  useBoardSelectionScope(path);
  // 「下一张」按优先级顺序（不受排序影响），去掉正在拖的卡片再算
  const prio = board?.cols ?? null;
  // 有排队派发的卡片不会被不带 ID 的 board claim 领走
  const queued = useData(s => s.ws?.dispatches);
  const urgentLabels = (cur?.project.labels ?? []).filter(l => l.priority).map(l => l.id);
  const moving = new Set([...(drag ? [drag.id] : []), ...landing.map(l => l.id)]);
  const nextId = f.nextCard(
    prio && moving.size ? Object.fromEntries(Object.entries(prio).map(([k, v]) => [k, v.filter(id => !moving.has(id))])) : prio,
    id => !!queued?.some(d => d.state === 'queued' && d.board === path && d.card === id),
    id => urgentLabels.length > 0 && !!board?.cards[id]?.labels.some(l => urgentLabels.includes(l)),
  );

  // 各状态块的瀑布流攒到这里一起排（先全部量、再全部写），排完再播 FLIP（play 读的是排好之后的位置）
  useLayoutEffect(() => {
    flushMasonry();
  });
  useLayoutEffect(() => {
    flip.play();
  }, [board, drag, landing]);

  // 定位到卡片
  useEffect(() => {
    if (!reveal) return;
    const n = document.querySelector<HTMLElement>(`[data-flip="c${renderKey(reveal.id)}"]`);
    if (!n) return;
    n.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: reduceMotion() ? 'auto' : 'smooth' });
    if (!reveal.pulse) return;
    // 放完就去掉：一直留着的话，卡片别的动画（入场、保存中）结束时 .card.pulse 会重新生效，光环再放一次
    replay(n, 'pulse');
    const done = (e: AnimationEvent) => {
      if (e.target !== n || e.animationName !== 'card-pulse') return;
      n.classList.remove('pulse');
      n.removeEventListener('animationend', done);
    };
    n.addEventListener('animationend', done);
  }, [reveal]);

  useEffect(() => {
    if (!reject) return;
    const c = boardRef.current?.querySelector(`[data-drop="${reject.col}"]`);
    replay(c, 'reject');
    const t = setTimeout(() => c?.classList.remove('reject'), 500);
    return () => clearTimeout(t);
  }, [reject]);

  // 终点状态默认只带最近移入的几张：筛选时、保留顺序开着且这块不按移入时间排时取全量，免得找不到、排不到更早的卡片
  //（按移入时间排时，最近移入的几张本来就在一头，不用取）
  const truncated = !!board && Object.values(board.omitted ?? {}).some(Boolean);
  const sortsTruncated =
    !!board?.keepSorted && Object.keys(board.omitted ?? {}).some(k => omittedIn(board, k) > 0 && orderOf(board.statuses.find(s => s.key === k)).by !== 'moved');
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
      {cur && <FirstRunHint key={path} path={path} />}
      <ColTabs board={board} f={f} boardRef={boardRef} />
      {!mobile && <TilePeek />}
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
        {cur && <BoardSettings path={path} />}
        {cur && <CardCronDialog path={path} />}
        {!board && f.list.length > 0 && (
          <Magnet path={path} statuses={f.list} render={s => <SkeletonColumn col={s} index={f.list.indexOf(s)} f={f} count={counts[s.key] ?? 0} />} />
        )}
        {board && (
          <Magnet
            path={path}
            statuses={f.list}
            frozen={!!drag && !drag.dropping}
            render={(s, h) => {
              const i = f.list.indexOf(s);
              return (
                <Column
                  col={s}
                  index={i}
                  f={f}
                  ids={cols?.[s.key] ?? NO_IDS}
                  board={board}
                  labels={labels}
                  filter={filter}
                  detailId={detailId}
                  drag={dragFor(drag, s.key)}
                  landing={landed[s.key]}
                  flashes={flashes}
                  entering={entering}
                  fresh={fresh}
                  onHead={h.onMove}
                  resizer={h.resizer}
                  next={nextId}
                  sorted={!!board.keepSorted}
                  groups={grouped?.groups ?? NO_GROUPS}
                  path={path}
                  more={cut?.more[s.key]}
                  role={roles.review.has(s.key) ? 'review' : roles.human.has(s.key) ? 'human' : undefined}
                />
              );
            }}
          />
        )}
      </div>
      <BoardBulkBar />
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
        <ColTags col={col} icons />
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
  /** 交审状态（块标题上「复盘」）、需要人处理的状态（「处理」） */
  role?: 'review' | 'human';
  col: Status;
  index: number;
  f: FlowQ;
  ids: number[];
  board: BoardSnapshot;
  labels: Label[];
  filter: string;
  detailId: number | null;
  drag: DragUI | null;
  /** 落点在这一块、还在落位的卡片（被下一次拖动顶出 drag 的）：和正在拖的卡片一样画成占位 */
  landing?: number[];
  flashes: ReturnType<typeof useUI.getState>['flashes'];
  entering: Set<number>;
  fresh: boolean;
  onHead: (e: React.PointerEvent) => void;
  resizer: ReactNode;
  /** 不带 ID 执行 board claim 时会被领走的卡片 */
  next: number | undefined;
  /** 白板设置开着「保留顺序」：标题上显示这块的排序依据 */
  sorted: boolean;
  /** 同一会话的卡成组（lib/sessionGroups）：按卡片编号查 */
  groups: Record<number, SessGroup>;
  path: string;
  /** 终点状态末尾那一格：「更早的 N 张」或展开后的「收起」（lib/earlier） */
  more?: Earlier;
}

/** 状态的规则：可领取 / 锁定 / 错误节点 / 终点（互斥：错误节点和另外三个不同时打开）。只有图标时（标题旁）按顺序取这几个 */
const COL_RULES = [
  { key: 'claim', icon: 'claim', name: '可领取', tip: '可领取：agent 从这里领' },
  { key: 'lock', icon: 'lock', name: '锁定', tip: '锁定：只能领取进入；派发排队的卡片也在这里等机器开始' },
  { key: 'error', icon: 'alert', name: '错误节点', tip: '错误节点：agent 异常退出、做不下去时移到这里，只有人能移出' },
  { key: 'final', icon: 'checkCircle', name: '终点', tip: '终点：只有人能移入移出' },
] as const;

function rulesOf(col: Status) {
  return COL_RULES.filter(r => col[r.key]);
}

/** 标题旁的规则标记；`icons` 用于骨架，和真实状态块一样只取图标，悬停时说明这条规则 */
function ColTags({ col, icons }: { col: Status; icons?: boolean }) {
  return (
    <>
      {rulesOf(col).map(r => (
        <Tip key={r.key} text={r.tip}>
          <span className="col-tag" data-rule={r.key} aria-label={r.name}>
            <Icon name={r.icon} />
            {icons ? null : <span className="t">{r.name}</span>}
          </span>
        </Tip>
      ))}
    </>
  );
}

function emptyText(s: Status, f: FlowQ) {
  if (s.claim && s.key === f.home?.key) return `${s.name}是空的。新建一张卡片，或等 agent 用 board new 创建`;
  if (s.claim) return `${s.name}是空的。agent 也会从这里领取`;
  if (s.lock && f.claimables.some(c => f.claimTarget(c.key)?.key === s.key)) return 'agent 执行 board claim、或派发排队后出现在这里';
  if (s.lock) return '持有者把卡片移过来后出现在这里';
  if (s.final) return '把确认完成的卡片拖到这里';
  return '把卡片拖到这里';
}

/** memo：磁吸布局自己重排（量到块高度变化、拖动 / 调宽状态块）时 props 都没变，不重画块里的卡片。
    卡片拖动松手时块要补测尺寸再排一次，以前这一遍把所有卡片又画了一次，落位那一帧明显卡（default/main#419） */
const Column = memo(function Column({ col, index, f, ids, board, labels, filter, detailId, drag, landing, flashes, entering, fresh, onHead, resizer, next, sorted, groups, path, more, role }: ColumnProps) {
  const count = colCount(board, col.key);
  const badge = useRef<HTMLSpanElement>(null);
  const list = useRef<HTMLDivElement>(null);
  useMasonry(list, sessionFrames);
  const runs = groupRuns(ids, groups);
  const runAt = new Map(runs.map(r => [r.start, r]));
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
        count === 0 && !(drag && drag.col === col.key && drag.valid) && !landing?.length && 'is-empty',
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
        <ColTags col={col} />
        {sorted && <OrderChip col={col} path={path} />}
        {role && count > 0 && (
          <Tip text={role === 'review' ? '复盘台：逐张看，通过或打回' : '逐张看 agent 留的问题，回复后放回'}>
            <button
              type="button"
              className={cx('col-desk', role)}
              onPointerDown={e => e.stopPropagation()}
              onClick={() => openReview({ cols: [col.key], mode: role })}
            >
              <Icon name={role === 'review' ? 'checkCircle' : 'comment'} />
              {role === 'review' ? '复盘' : '处理'}
            </button>
          </Tip>
        )}
        <ColSelect ids={ids} board={board} labels={labels} />
        <span className="col-count" aria-label="卡片数量" ref={badge}>
          <span>{count}</span>
        </span>
      </header>
      <div className="col-body">
        <div className={cx('col-list', `view-${view}`)} data-list={col.key} ref={list} style={listStyle}>
          {ids.flatMap((id, idx) => {
            const c = board.cards[id];
            if (!c) return [];
            const run = runAt.get(idx);
            const end = runs.find(r => r.start + r.n - 1 === idx);
            const grouped = runs.some(r => idx >= r.start && idx < r.start + r.n);
            const ph = drag?.id === id || !!landing?.includes(id);
            const card = (
              <CardView
                key={renderKey(id)}
                card={ph ? { ...c, column: col.key } : c}
                labels={labels}
                filter={filter}
                open={detailId === id}
                placeholder={ph}
                flash={flashes[id]}
                entering={entering.has(id)}
                delay={fresh ? 180 + index * 70 + idx * 45 : undefined}
                color={color}
                final={col.final}
                view={view}
                next={firstFree === id}
                grouped={grouped}
              />
            );
            // 组头、组尾和卡片是同一层的兄弟（不包一层）：成组、散开时卡片不重新挂载，拖拽按 data-deco 跳过它们
            return [
              run && <SessionHead key={`sh:${run.g.key}`} g={run.g} n={run.n} gk={`${run.g.key}@${run.start}`} />,
              card,
              end && <i key={`sf:${end.g.key}`} className="sess-foot" data-deco="" data-block-end="" data-g={`${end.g.key}@${end.start}`} aria-hidden="true" />,
            ].filter(Boolean);
          })}
          {more && <EarlierButton more={more} view={view} path={path} col={col.key} />}
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
});

/** 提示可能为空（不是磁贴时不加提示）：为空时原样返回元素，不套 <Tip> */
function TipOr({ text, children }: { text?: string; children: ReactElement }) {
  return text ? <Tip text={text}>{children}</Tip> : children;
}

/** 终点状态只显示最近移入的几张：列表最后一格「更早的 M 张」，和这一块的卡片形态一样大（磁贴里写成 +M）；
    点开取全量（这个窗口里这块白板之后都带全量）、显示全部，同一格变成「收起」；收起只在界面上截断，不重新请求（#485）。
    拖拽算落点时不算它（data-earlier） */
function EarlierButton({ more, view, path, col }: { more: Earlier; view: StatusView; path: string; col: string }) {
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLButtonElement>(null);
  // 卡片、单行的高度随内容和密度变：和前面一张一样高（在父组件的瀑布流排版之前量）；磁贴是定高
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const prev = el.previousElementSibling as HTMLElement | null;
    const h = view !== 'tile' && prev ? getComputedStyle(prev).height : '';
    if (el.style.height !== h) el.style.height = h;
  });
  // 点了展开、全量还没回来：还按「更早的 N 张」画，转圈
  const loading = !more.miss && (busy || (more.open && more.n > 0));
  const fold = more.open && more.n === 0;
  // 筛选中：更早的卡片里没命中的不画，这一格只写张数，不能点（default/main#630）
  const miss = !!more.miss;
  const label = miss ? `更早的 ${more.n} 张没有命中筛选，没有显示` : fold ? `收起，只看最近的 ${FINAL_RECENT} 张` : `显示更早的 ${more.n} 张`;
  return (
    <TipOr text={view === 'tile' ? (miss ? `更早的 ${more.n} 张未命中` : fold ? '收起' : `更早的 ${more.n} 张`) : undefined}>
      <button
        ref={ref}
        type="button"
        data-earlier=""
        className={cx('col-earlier', `v-${view}`, fold && 'is-open', loading && 'is-busy', miss && 'is-miss')}
        disabled={loading || miss}
        aria-label={label}
        aria-expanded={miss ? undefined : fold}
        onClick={async () => {
          if (fold) return void setEarlier(path, col, false);
          setBusy(true);
          try {
            await setEarlier(path, col, true);
          } catch (e) {
            // loadFull 自己接住加载错误并提示；这里只兜底意外错误，别让展开状态卡住
            void setEarlier(path, col, false);
            toastError(e);
          } finally {
            setBusy(false);
          }
        }}
      >
        {view === 'tile' ? (
          fold ? <Icon name="chevUp" /> : `+${more.n}`
        ) : miss ? (
          <>更早的 {more.n} 张未命中</>
        ) : fold ? (
          <>
            <Icon name="chevUp" />
            收起
          </>
        ) : (
          <>
            <Icon name="more" />
            更早的 {more.n} 张
          </>
        )}
      </button>
    </TipOr>
  );
}

/** 保留顺序开着时标题上的排序标记：写这块按什么排、方向；点开白板设置，定位到这一块的排序依据 */
function OrderChip({ col, path }: { col: Status; path: string }) {
  const o = orderOf(col);
  const x = orderOption(o);
  return (
    <Tip text={`保留顺序：按${orderName(o)}，${orderDir(o)}`}>
      <button
        type="button"
        className="col-order"
        aria-label={`「${col.name}」按${orderName(o)}排，${orderDir(o)}。点开白板设置更改`}
        onClick={() => openBoardSettings(path, col.key)}
      >
        <Icon name="sort" />
        <span className="t">{x.name}</span>
        <span className="ts">{x.short}</span>
        {x.dirs && <i aria-hidden="true">{o.desc ? '↓' : '↑'}</i>}
      </button>
    </Tip>
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

export function Magnet({
  path, statuses, frozen = false, render,
}: {
  path: string;
  statuses: Status[];
  /** 卡片拖动期间（指针还按着）冻结白板宽度的测量（Windows 上滚动条随块变高出现 / 消失会改宽，整块重排）。
      块的高度照常跟着占位变：目标块随占位撑开、下面的块跟着让开（default/main#502）。
      落点判定读开始拖动时拍下的块位置（drag.ts 的 snapZones），块挪动不会让落点来回变（#419 的反馈环）。
      白板内容的总高度在拖动中只增不减（见下面的 floor）：变矮会让滚在底部附近的白板 scrollTop 被动回收，落点跟着跳（default/main#628）。
      松手当帧就解冻（DragUI.dropping），宽度补测一次、高度恢复按内容算 */
  frozen?: boolean;
  render: (s: Status, h: { onMove: (e: React.PointerEvent) => void; resizer: ReactNode }) => ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const mobile = useMobile();
  const [width, setWidth] = useState(1000);
  const [heights, setHeights] = useState<Record<string, number>>({});
  const frozenRef = useRef(frozen);
  frozenRef.current = frozen;
  // 拖动中的预览 / 刚保存、等服务端确认的布局
  const [live, setLive] = useState<Record<string, Tile> | null>(null);
  const [drag, setDrag] = useState<{ key: string; left: number; top: number; mode: 'move' | 'size' } | null>(null);
  const els = useRef<Record<string, HTMLElement | null>>({});
  const keys = statuses.map(s => s.key);
  const layoutBusy = useBusy(flowKey(path));
  const saved = useMemo(() => Object.fromEntries(statuses.filter(s => s.layout).map(s => [s.key, s.layout!])), [statuses]);

  const tiles = withDefaults(keys, live ?? saved);
  const col = (width + GAP) / COLS;
  const pos = pack(keys, tiles, heights);
  const bottom = Math.max(0, ...keys.map(k => (pos[k]?.top ?? 0) + (heights[k] ?? 0)));
  // 卡片拖动中白板内容的高度只增不减，松手当帧恢复（default/main#628）。
  // 最高的那块（多半是拖出卡片的那块）一变矮，白板就变短；白板正滚在底部附近时浏览器把 scrollTop 往回收，
  // 所有块在指针底下整体下移，落点（按快照 + 滚动偏移算）跳到上面那块，占位回去、白板又变高，来回跳。
  // 块的高度、位置照常跟着占位变，只是拖动中白板底部多留出空白
  const floor = useRef(0);
  floor.current = frozen ? Math.max(floor.current, bottom + 4) : 0;
  const height = Math.max(bottom + 4, floor.current);

  // 切到别的 Tab 时白板页 display: none，量出来是 0：不记，切回来时布局还是原样，不会先挤成一团再展开（抖动）
  useLayoutEffect(() => {
    const el = ref.current!;
    const measure = () => {
      if (el.clientWidth && !frozenRef.current) setWidth(el.clientWidth);
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
  // 拖动中宽度变化被跳过，ResizeObserver 之后不会再把它送进来：解冻时（松手当帧）统一补测一次；高度顺带对一遍。
  useLayoutEffect(() => {
    if (frozen) return;
    const el = ref.current;
    if (!el) return;
    if (el.clientWidth) setWidth(el.clientWidth);
    setHeights(h => {
      let n = h;
      for (const k of keys) {
        const e = els.current[k];
        if (!e) continue;
        const v = e.offsetHeight;
        if (!v && !e.offsetParent) continue;
        if (n[k] !== v) n = { ...n, [k]: v };
      }
      return n;
    });
  }, [frozen, keys.join()]); // eslint-disable-line react-hooks/exhaustive-deps

  // 手机上块上下排列（.magnet.stacked），高度由内容撑开、不写 height：拖动中用 min-height 撑住出现过的最大高度，道理同上面的 floor
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (!mobile || !frozen) {
      if (el.style.minHeight) el.style.minHeight = '';
      return;
    }
    const h = el.offsetHeight;
    if (h > (parseFloat(el.style.minHeight) || 0)) el.style.minHeight = `${h}px`;
  });

  // 拖动、调宽的入口交给状态块时是固定的函数（见下面的 handles），按下时从这里读最新的布局
  const latest = useRef({ pos, tiles, heights, col, mobile, keys, path, layoutBusy });
  latest.current = { pos, tiles, heights, col, mobile, keys, path, layoutBusy };

  /** 松手：把每块的 y 对齐到实际位置后保存，下次拖动从这里排先后 */
  const commit = (all: Record<string, Tile>) => {
    const { keys, heights, path } = latest.current;
    const p = pack(keys, all, heights);
    const next = Object.fromEntries(keys.map(k => [k, { ...all[k], y: p[k].top }]));
    setLive(next);
    void Promise.resolve()
      .then(() =>
        optimistic(
          { board: b => ({ ...b, statuses: b.statuses.map(s => ({ ...s, layout: next[s.key] ?? null })) }), keys: [flowKey(path)] },
          () => api.flowLayout(path, next),
        ),
      )
      .catch(toastError)
      .finally(() => setLive(null));
  };

  const onMove = (k: string, e: React.PointerEvent) => {
    const { pos, tiles, heights, col, mobile, keys, layoutBusy } = latest.current;
    if (layoutBusy || mobile || e.button !== 0 || (e.target as HTMLElement).closest('button, input, .card')) return;
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

  const onSize = (k: string, e: React.PointerEvent) => {
    const { pos, tiles, col, keys, layoutBusy } = latest.current;
    if (layoutBusy || e.button !== 0) return;
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

  // 每块一份、只在状态或手机 / 桌面切换时换：状态块（memo）的 props 不因磁吸布局重排而变
  const handlers = useRef({ onMove, onSize });
  handlers.current = { onMove, onSize };
  const handles = useMemo(
    () =>
      Object.fromEntries(
        keys.map(k => [
          k,
          {
            onMove: (e: React.PointerEvent) => handlers.current.onMove(k, e),
            resizer: mobile ? null : <span className="tile-resize" title="拖动调整宽度" onPointerDown={e => handlers.current.onSize(k, e)} />,
          },
        ]),
      ),
    [keys.join(), mobile], // eslint-disable-line react-hooks/exhaustive-deps
  );

  const moving = drag?.mode === 'move' ? drag : null;
  return (
    <div className={cx('magnet', moving && 'moving', mobile && 'stacked')} ref={ref} style={mobile ? undefined : { height }}>
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
            {render(s, handles[s.key])}
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

const NO_GROUPS: Record<number, SessGroup> = {};
const NO_LABELS: Label[] = [];
const NO_IDS: number[] = [];

/** 一组的开头：和整块一样宽（lib/masonry 按块摆），写机器 · agent（会话）或「排队等空位 · 机器」；框由 sessionFrames 按组尾量出高度 */
function SessionHead({ g, n, gk }: { g: SessGroup; n: number; gk: string }) {
  return (
    <div className={cx('sess-head', g.kind)} data-deco="" data-block="" data-g={gk}>
      <Icon name={g.kind === 'session' ? 'terminal' : 'timer'} />
      {g.kind === 'session' ? (
        <span>
          <b>{g.host}</b> · {g.agentLabel}
        </span>
      ) : (
        <span>
          排队等空位 · <b>{g.host}</b>
        </span>
      )}
      <em className="mono">{g.kind === 'session' ? `会话 ${n} 张` : `${n} 张`}</em>
    </div>
  );
}

/** 瀑布流排好之后：每组的框从组头一直画到组尾（--fh） */
function sessionFrames(list: HTMLElement) {
  for (const h of list.querySelectorAll<HTMLElement>(':scope > .sess-head')) {
    const foot = list.querySelector<HTMLElement>(`:scope > .sess-foot[data-g="${CSS.escape(h.dataset.g || '')}"]`);
    if (!foot) continue;
    const fh = `${Math.max(0, Math.round(foot.getBoundingClientRect().top - h.getBoundingClientRect().top))}px`;
    if (h.style.getPropertyValue('--fh') !== fh) h.style.setProperty('--fh', fh);
  }
}
