/* ==========================================================================
   左侧操作台：活动栏 + 侧栏（PRD §10.1）
   - 活动栏：白板 / 未整理 / 大盘 / 项目 / 机器 / 设置。再次点击当前 Tab 收起或展开侧栏
   - 侧栏宽度可以拖动调整；手机上活动栏变为底部 Tab 栏
   ========================================================================== */
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type UIEvent } from 'react';
import { newBoard, newProject, openBoard, openCard } from '../lib/actions';
import { api } from '../lib/api';
import { canvasHighlight } from '../lib/canvasHighlight';
import { labelDragJustEnded, labelPointerDown } from '../lib/labelDrag';
import type { BoardEvent, Card, Label } from '../lib/types';
import { boardFlow, flowOf, timing } from '../lib/schema';
import { clock, cx, ago, agentInk, pref, replay } from '../lib/util';
import { deleteLabelPatch, newEntryCount, optimistic, useCurrent, useData, useOnline } from '../store/data';
import { confirm, openCreator, openCtx, setBoardSheet, setFilter, setSide, setTab, TAB_ORDER, toastError, toggleLabelSel, useUI, type Tab } from '../store/ui';
import { Avatar, LabelLock, LabelPriority, ProjectAvatar, SYSTEM_NAME, Title, agent, useMeName, useNow } from './bits';
import { useMobile } from './BoardView';
import { HostsSide } from './Hosts';
import { InboxSideParts } from './Inbox';
import { BoardIcon24, HostsIcon24, Icon, InboxIcon24, ProjectsIcon24, SettingsIcon24, StatsIcon24 } from './Icon';
import { SettingsNav } from './SettingsNav';
import { StatsSide } from './StatsTab';
import { Skeleton } from './Skeleton';

/** 每个标签的名称、提示和图标；顺序由 store/ui 的 TAB_ORDER 决定（和数字快捷键一致），设置固定在底部 */
const TAB_META: Record<Tab, { label: string; tip: string; icon: ReactNode }> = {
  board: { label: '白板', tip: '白板', icon: <BoardIcon24 /> },
  inbox: { label: '未整理', tip: '未整理', icon: <InboxIcon24 /> },
  stats: { label: '大盘', tip: '大盘 · 白板和项目的统计', icon: <StatsIcon24 /> },
  projects: { label: '项目', tip: '项目 · 白板总览', icon: <ProjectsIcon24 /> },
  hosts: { label: '机器', tip: '机器 · board host', icon: <HostsIcon24 /> },
  settings: { label: '设置', tip: '设置', icon: <SettingsIcon24 /> },
};

export function ActivityBar() {
  const tab = useUI(s => s.tab);
  const ref = useRef<HTMLElement>(null);
  const [y, setY] = useState(18);
  useLayoutEffect(() => {
    const place = () => {
      const on = ref.current?.querySelector<HTMLElement>('.ab-item.on');
      if (on) setY(on.offsetTop + (on.offsetHeight - 24) / 2);
    };
    place();
    window.addEventListener('resize', place);
    document.fonts?.ready.then(place);
    return () => window.removeEventListener('resize', place);
  }, [tab]);
  const item = (t: Tab) => {
    const m = TAB_META[t];
    return (
      <button
        key={t}
        className={cx('ab-item', tab === t && 'on')}
        type="button"
        data-tab={t}
        data-tip={m.tip}
        data-tip-pos="right"
        aria-label={m.label}
        aria-current={tab === t ? 'page' : undefined}
        onClick={() => setTab(t, { toggle: true })}
      >
        {m.icon}
        <span className="ab-label">{m.label}</span>
      </button>
    );
  };
  return (
    <nav className="activitybar" aria-label="操作台" ref={ref}>
      <i className="ab-indicator" aria-hidden="true" style={{ ['--ab-y' as string]: `${y}px` }} />
      {TAB_ORDER.filter(t => t !== 'settings').map(item)}
      <span className="ab-spacer" />
      {item('settings')}
    </nav>
  );
}

/* ============================ 侧栏 ============================ */

const SIDE_W = { min: 220, max: 460, def: 292 };

export function SidePanel() {
  const ref = useRef<HTMLElement>(null);
  const [resizing, setResizing] = useState(false);
  useLayoutEffect(() => {
    document.documentElement.style.setProperty('--side-w', `${pref.get('sideW', SIDE_W.def)}px`);
  }, []);
  const onDown = (e: React.PointerEvent) => {
    const x0 = e.clientX;
    const w0 = ref.current!.offsetWidth;
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    setResizing(true);
    const apply = (w: number) => document.documentElement.style.setProperty('--side-w', `${w}px`);
    const move = (ev: PointerEvent) => {
      const w = w0 + (ev.clientX - x0);
      if (w < SIDE_W.min - 70) {
        // 拖得够窄 → 收起
        end();
        apply(w0);
        setSide(false);
        return;
      }
      apply(Math.max(SIDE_W.min, Math.min(SIDE_W.max, w)));
    };
    const end = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', end);
      handle.removeEventListener('pointercancel', end);
      setResizing(false);
      pref.set('sideW', ref.current?.offsetWidth ?? SIDE_W.def);
      window.dispatchEvent(new Event('resize'));
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
  };
  return (
    <aside className={cx('sidepanel', resizing && 'resizing')} aria-label="侧栏" ref={ref}>
      <div className="side-inner">
        <ProjectsSide />
        <BoardSide />
        <InboxSide />
        <StatsSide />
        <HostsSide />
        <section className="side-view" data-view="settings">
          <header className="side-head">
            <span className="side-title">设置</span>
          </header>
          <SettingsNav />
        </section>
      </div>
      <div
        className="side-resize"
        title="拖动调整宽度，双击还原"
        onPointerDown={onDown}
        onDoubleClick={() => {
          document.documentElement.style.setProperty('--side-w', `${SIDE_W.def}px`);
          pref.set('sideW', SIDE_W.def);
        }}
      />
    </aside>
  );
}

function SideFilter({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) {
  const ref = useRef<HTMLInputElement>(null);
  return (
    <label className={cx('side-filter', value && 'has-value')}>
      <Icon name="filter" />
      <input
        ref={ref}
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        autoComplete="off"
        onChange={e => onChange(e.target.value)}
        onKeyDown={e => {
          if (e.key === 'Escape') {
            e.stopPropagation();
            if (value) onChange('');
            else ref.current?.blur();
          }
        }}
      />
      <button
        type="button"
        className="filter-clear"
        aria-label="清除筛选"
        onClick={() => {
          onChange('');
          ref.current?.focus();
        }}
      >
        ×
      </button>
    </label>
  );
}

/* ---------------- 白板 Tab：筛选 / 标签 / 正在工作 / 最近动态 ---------------- */

const OP: Record<string, string> = {
  create: '新建了', edit: '编辑了', move: '移动了', reorder: '调整了顺序', claim: '领取了', renew: '续期了',
  comment: '评论了', edit_comment: '改了评论', delete_comment: '删除了评论', timeout: '超时释放了', release: '强制释放了', delete: '删除了', label: '改了标签', merge: '合并了',
  beat: '心跳', lost: '心跳中断，释放了', dispatch: '派发',
};

/** n 为 'loading'：数量还不知道，计数位置放骨架，标题不跳 */
function Section({ id, title, n, closed, onToggle, children }: { id: string; title: string; n?: number | 'loading'; closed: boolean; onToggle: (id: string) => void; children: ReactNode }) {
  return (
    <section className={cx('tree-sec', closed && 'closed')} data-sec={id}>
      <button className="tree-head" type="button" onClick={() => onToggle(id)}>
        <svg className="chev" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="m6 8 4 4 4-4" />
        </svg>
        {title}
        {n === 'loading' ? <Skeleton className="n sk-n" w={14} h={10} /> : n != null && <span className="n">{n}</span>}
      </button>
      <div className="tree-body">
        <div>{children}</div>
      </div>
    </section>
  );
}

/** 标签：整块就是输入框。平时看着只是一排胶囊；双击胶囊以外的空白处进入编辑态，光标跟在最后一个胶囊后面，
    输入名称回车新建；Esc 或点到区块外退出。单击空白不做任何事；单击胶囊筛选、点 × 删除、拖胶囊到卡片都不进入编辑态。
    胶囊可以拖到卡片上，给卡片加上这个标签（lib/labelDrag.ts） */
function LabelBlock({ project, labels, used }: { project: string; labels: Label[]; used: Record<number, number> }) {
  const sel = useUI(s => s.labelSel);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const edit = () => {
    setEditing(true);
    const el = input.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  };
  const create = async () => {
    const n = name.trim();
    if (!n || busy) return;
    // 新标签要等服务端分配 ID：输入框进行中（只读、转圈），不会重复新建
    setBusy(true);
    try {
      await optimistic(null, () => api.createLabel(project, n));
      setName('');
    } catch (e) {
      toastError(e);
      replay(input.current, 'invalid');
    } finally {
      setBusy(false);
    }
  };
  const remove = async (l: Label) => {
    const text = l.uses ? `${l.uses} 张卡片在用这个标签，删除后会从这些卡片上一起去掉。` : '还没有卡片在用这个标签。';
    if (!(await confirm({ title: `删除标签「${l.name}」？`, text, ok: '删除标签', icon: 'tag' }))) return;
    // 当帧从侧栏和卡片上拿掉，被拒绝时回来并提示
    if (useUI.getState().labelSel.includes(l.id)) toggleLabelSel(l.id);
    try {
      await optimistic(deleteLabelPatch(project, l.id), () => api.deleteLabel(project, l.id));
    } catch (e) {
      toastError(e);
    }
  };
  return (
    <div className="lb-block">
      <div
        className={cx('lb-field', editing && 'editing')}
        onMouseDown={e => {
          // 胶囊、× 照常；空白处单击什么也不做：不聚焦、不选中文字，编辑中也不让输入框失焦
          if ((e.target as HTMLElement).closest('button, input')) return;
          e.preventDefault();
        }}
        onDoubleClick={e => {
          // 只有双击胶囊以外的空白处才进入编辑态；刚拖完胶囊补发的点击不算
          if ((e.target as HTMLElement).closest('button') || labelDragJustEnded()) return;
          edit();
        }}
      >
        {labels.map(l => (
          <span key={l.id} className="lb-item" data-label={l.id}>
            <button
              type="button"
              className={cx('lb lb-pick', sel.includes(l.id) && 'on')}
              style={{ ['--lc' as string]: l.color }}
              aria-pressed={sel.includes(l.id)}
              title={`${sel.includes(l.id) ? '取消选中' : `筛选「${l.name}」（可多选，带任一个就显示）`} · 拖到卡片上加这个标签，在这里拖动调整顺序`}
              onPointerDown={e => labelPointerDown(e, l, project)}
              onClick={() => {
                if (!labelDragJustEnded()) toggleLabelSel(l.id);
              }}
            >
              <LabelPriority label={l} />
              <LabelLock label={l} />
              {l.name}
              <em>{used[l.id] || 0}</em>
            </button>
            {/* 悬停时盖在计数上，不占位，胶囊排列不变 */}
            <button
              type="button"
              className={cx('lb-del', sel.includes(l.id) && 'on')}
              style={{ ['--lc' as string]: l.color }}
              aria-label={`删除标签「${l.name}」`}
              title="删除标签"
              onClick={() => remove(l)}
            >
              ×
            </button>
          </span>
        ))}
        <input
          ref={input}
          className={cx('lb-input', busy && 'busy')}
          value={name}
          placeholder={labels.length ? '' : editing ? '输入标签名，回车新建' : '双击新建标签'}
          maxLength={24}
          spellCheck={false}
          autoComplete="off"
          aria-label="新标签"
          readOnly={busy}
          aria-busy={busy || undefined}
          // 键盘 Tab 进来也算进入编辑态；鼠标单击不到它（不在编辑态时 pointer-events: none）
          onFocus={() => setEditing(true)}
          onBlur={() => setEditing(false)}
          onChange={e => setName(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
              e.preventDefault();
              create();
            }
            if (e.key === 'Escape') {
              e.stopPropagation();
              setName('');
              input.current?.blur();
            }
          }}
        />
      </div>
    </div>
  );
}

/* ---------------- 侧栏骨架：快照还没到时（首次打开、切换白板） ---------------- */

/** 标签胶囊行：数量已知（工作区里有）时按数量，宽度错开 */
function SkeletonLabels({ n }: { n: number }) {
  return (
    <div className="lb-block sk-labels" aria-hidden="true">
      {Array.from({ length: Math.min(n, 12) }, (_, i) => (
        <Skeleton key={i} w={[52, 44, 60, 48, 66, 40][i % 6]} h={22} r={6} />
      ))}
    </div>
  );
}

/** 持有者行（头像 + 一行）/ 动态行（头像 + 两行） */
function SkeletonRows({ n, kind }: { n: number; kind: 'working' | 'recent' }) {
  return (
    <div aria-hidden="true">
      {Array.from({ length: n }, (_, i) =>
        kind === 'working' ? (
          <div key={i} className="tree-row card-row sk-row" style={{ paddingLeft: 20 }}>
            <Skeleton w={16} h={16} r={8} />
            <Skeleton w={['68%', '54%', '62%'][i % 3]} h={10} />
            <Skeleton className="sk-tm" w={34} h={10} />
          </div>
        ) : (
          <div key={i} className="recent-row sk-row">
            <Skeleton w={18} h={18} r={9} />
            <div className="sk-lines">
              <Skeleton w={['72%', '58%', '66%'][i % 3]} h={10} />
              <Skeleton w={52} h={8} />
            </div>
          </div>
        ),
      )}
    </div>
  );
}

/** 最近动态每页条数（服务端上限 100）：正好比一块列表高，滚到底再取下一页 */
const RECENT_PAGE = 20;

/** 把新到的动态并到顶部：按 id 去重、新的在前；已加载到哪（游标）不受影响 */
function mergeNewer(prev: BoardEvent[], fresh: BoardEvent[]): BoardEvent[] {
  if (!prev.length || !fresh.length) return prev;
  const newest = prev[0].id;
  const add = fresh.filter(e => e.id > newest);
  if (!add.length) return prev;
  add.sort((a, b) => b.id - a.id);
  return [...add, ...prev];
}

/** 最近动态：首次取一页，滚到底用游标往前翻更早的。
    实时推送和自己刚做的操作（重新取快照时带回）插到顶部，不移动游标、不改已加载的页 */
function useRecentEvents(path: string | undefined, live: BoardEvent[] | undefined) {
  const [events, setEvents] = useState<BoardEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 请求状态放 ref：滚动回调不用跟着重建，连续滚动也不会并发请求
  const st = useRef({ path: '', cursor: null as number | null, busy: false, ended: false, seq: 0 });
  const liveRef = useRef(live);
  liveRef.current = live;

  const loadMore = useCallback(async () => {
    const s = st.current;
    if (!s.path || s.busy || s.ended) return;
    s.busy = true;
    const seq = s.seq;
    setLoading(true);
    setError(null);
    try {
      const page = await api.events(s.path, s.cursor, RECENT_PAGE);
      if (s.seq !== seq) return; // 已经换了白板：这一页丢掉
      if (page.length < RECENT_PAGE) {
        s.ended = true;
        setDone(true);
      }
      if (page.length) s.cursor = page[page.length - 1].id;
      setEvents(prev => {
        const seen = new Set(prev.map(e => e.id));
        const older = page.filter(e => !seen.has(e.id));
        return mergeNewer(older.length ? [...prev, ...older] : prev, liveRef.current ?? []);
      });
    } catch (e) {
      if (s.seq === seq) setError((e as Error).message || '动态没有加载出来');
    } finally {
      if (s.seq === seq) {
        s.busy = false;
        setLoading(false);
      }
    }
  }, []);

  // 换白板：列表和游标一起重置，重新取第一页
  useEffect(() => {
    const s = st.current;
    s.seq++;
    s.path = path ?? '';
    s.cursor = null;
    s.busy = false;
    s.ended = !path;
    setEvents([]);
    setDone(false);
    setError(null);
    setLoading(false);
    if (path) loadMore();
  }, [path, loadMore]);

  // 快照里带回来的新动态（实时推送后重新取的快照、自己刚做的操作）：插到顶部
  useEffect(() => {
    if (!live?.length) return;
    setEvents(prev => mergeNewer(prev, live));
  }, [live]);

  return { events, loading, done, error, loadMore };
}

/** 侧栏「正在工作」的计时：自己每秒走表，侧栏其余部分（要遍历全部卡片）不跟着每秒重渲染 */
function RunTime({ card: c }: { card: Card }) {
  const now = useNow(1000);
  const online = useOnline();
  const t = timing(c, now, online);
  return (
    <span className={cx('tm run-tm', t.beat)} title="这一次的耗时">
      <i className="beat" />
      {t.cur != null ? clock(t.cur) : t.beat === 'wait' ? '等待心跳' : '已结束'}
    </span>
  );
}

/** 「几分钟前」：半分钟更新一次 */
function Ago({ at }: { at: number }) {
  const now = useNow(30000);
  return <>{ago(at, now)}</>;
}

/** 手机白板页的底部抽屉：桌面侧栏里的白板部分（筛选框、标签、正在工作、最近动态）。下拉、点遮罩、Esc 关闭；打开卡片时让位给详情 */
export function BoardSheet() {
  const mobile = useMobile();
  const open = useUI(s => s.boardSheet && s.tab === 'board') && mobile;
  const detailId = useData(s => s.detailId);
  const ref = useRef<HTMLElement>(null);
  const [y, setY] = useState<number | null>(null);
  const grab = useRef<{ y0: number; t0: number; dy: number } | null>(null);
  const close = () => setBoardSheet(false);
  useEffect(() => {
    if (!open) {
      setY(null);
      return;
    }
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      close();
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, [open]);
  useEffect(() => {
    if (detailId != null) close();
  }, [detailId]);
  const onDown = (e: React.PointerEvent) => {
    const t = e.target as HTMLElement;
    if (!t.closest('.drawer-grab, .side-head') || t.closest('button')) return;
    grab.current = { y0: e.clientY, t0: performance.now(), dy: 0 };
    ref.current?.setPointerCapture(e.pointerId);
    ref.current?.classList.add('dragging');
  };
  const onMove = (e: React.PointerEvent) => {
    if (!grab.current) return;
    grab.current.dy = Math.max(0, e.clientY - grab.current.y0);
    setY(grab.current.dy);
  };
  const onUp = () => {
    const g = grab.current;
    if (!g) return;
    grab.current = null;
    ref.current?.classList.remove('dragging');
    if (g.dy > 120 || g.dy / Math.max(1, performance.now() - g.t0) > 0.6) close();
    else setY(0);
  };
  if (!mobile) return null;
  return (
    <>
      <div className={cx('scrim', open && 'on')} onClick={close} />
      <aside
        ref={ref}
        className={cx('drawer board-sheet', open && 'on')}
        role="dialog"
        aria-modal="true"
        aria-label="筛选、标签、正在工作、最近动态"
        aria-hidden={!open}
        style={y != null ? { ['--sheet-y' as string]: `${y}px` } : undefined}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
      >
        <div className="drawer-grab">
          <i />
        </div>
        {open && <BoardSide />}
      </aside>
    </>
  );
}

function BoardSide() {
  const board = useData(s => s.board);
  const detailId = useData(s => s.detailId);
  const cur = useCurrent();
  const filter = useUI(s => s.filter);
  const meName = useMeName();
  const [closed, setClosed] = useState<Set<string>>(() => new Set(pref.get<string[]>('treeClosed', [])));
  const toggle = (id: string) =>
    setClosed(prev => {
      const n = new Set(prev);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      pref.set('treeClosed', [...n]);
      return n;
    });
  const lastSeen = useRef(0);
  const recent = useRecentEvents(board?.path, board?.log);
  const newest = recent.events[0]?.id ?? 0;
  const recentList = useRef<HTMLDivElement>(null);
  useEffect(() => {
    lastSeen.current = newest;
  }, [newest]);
  useEffect(() => {
    lastSeen.current = 0;
    if (recentList.current) recentList.current.scrollTop = 0;
  }, [cur?.board.path]);
  // 滚到底（离底 120px 以内）自动取更早的一页；失败后等点「重试」，不在滚动里反复请求
  const onRecentScroll = (e: UIEvent<HTMLDivElement>) => {
    if (recent.loading || recent.done || recent.error) return;
    const el = e.currentTarget;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 120) recent.loadMore();
  };

  const labels = cur?.project.labels ?? [];
  const used = useMemo(() => {
    const m: Record<number, number> = {};
    if (board) for (const c of Object.values(board.cards)) for (const l of c.labels) m[l] = (m[l] || 0) + 1;
    return m;
  }, [board]);
  const holders = board ? boardFlow(board).list.flatMap(s => (board.cols[s.key] ?? []).map(id => board.cards[id])).filter(c => c?.holder) : [];

  return (
    <section className="side-view" data-view="board">
      <header className="side-head">
        <span className="side-title">白板</span>
        <span className="side-sub">{cur?.board.path}</span>
        <button
          className="icon-btn sm"
          type="button"
          aria-label="折叠 / 展开分组"
          data-tip="折叠 / 展开分组"
          data-tip-align="end"
          onClick={() => {
            const all = ['labels', 'working', 'recent'];
            const close = all.some(x => !closed.has(x));
            const n = new Set(close ? all : []);
            pref.set('treeClosed', [...n]);
            setClosed(n);
          }}
        >
          <Icon name="collapse" />
        </button>
      </header>
      <SideFilter value={filter} onChange={setFilter} placeholder="筛选卡片：标题、描述、#ID、agent" />
      <div className="side-scroll">
        <Section id="labels" title="标签" n={cur ? labels.length : 'loading'} closed={closed.has('labels')} onToggle={toggle}>
          {cur && (board || !labels.length) ? (
            <LabelBlock project={cur.project.key} labels={labels} used={used} />
          ) : (
            <SkeletonLabels n={cur ? labels.length : 5} />
          )}
        </Section>
        <Section id="working" title="正在工作" n={board ? holders.length : cur ? cur.board.holders.length : 'loading'} closed={closed.has('working')} onToggle={toggle}>
          {!board && (cur?.board.holders.length ?? 1) > 0 ? (
            <SkeletonRows n={cur?.board.holders.length ?? 1} kind="working" />
          ) : holders.length ? (
            holders.map(c => (
              <button
                key={c.id}
                className={cx('tree-row card-row', detailId === c.id && 'active')}
                type="button"
                style={{ paddingLeft: 20 }}
                onClick={() => openCard(c.id)}
              >
                <Avatar actor={agent(c.holder!)} />
                <span className="tt">
                  <span className="mono" style={{ color: agentInk(c.holder!) }}>
                    {c.holder}
                  </span>{' '}
                  · #{c.id} <Title text={c.title} />
                </span>
                <RunTime card={c} />
              </button>
            ))
          ) : (
            <div className="tree-empty" style={{ paddingLeft: 20 }}>
              没有 agent 在工作
            </div>
          )}
        </Section>
        <Section id="recent" title="最近动态" closed={closed.has('recent')} onToggle={toggle}>
          {!board ? (
            <SkeletonRows n={3} kind="recent" />
          ) : (
            <div className="recent-list" ref={recentList} onScroll={onRecentScroll}>
              {recent.events.length ? (
                <>
                  {recent.events.map(ev => (
                    <div key={ev.id} className={cx('recent-row', lastSeen.current > 0 && ev.id > lastSeen.current && 'new')}>
                      <Avatar actor={ev.actor} />
                      <div>
                        <b style={ev.actor.kind === 'agent' ? { color: agentInk(ev.actor.name) } : undefined}>
                          {ev.actor.kind === 'human' ? meName || '你' : ev.actor.kind === 'system' ? SYSTEM_NAME : ev.actor.name}
                        </b>{' '}
                        {OP[ev.op] || ev.op}{' '}
                        <button type="button" className="mono recent-id" onClick={() => openCard(ev.cardId)}>
                          #{ev.cardId}
                        </button>
                        <span className="when">
                          <Ago at={ev.at} />
                          {ev.via === 'cli' ? ' · CLI' : ''}
                        </span>
                      </div>
                    </div>
                  ))}
                  {(recent.loading || recent.error || recent.done) && (
                    <div className={cx('recent-foot', recent.error && 'error')}>
                      {recent.loading ? (
                        '正在加载更早的动态…'
                      ) : recent.error ? (
                        <>
                          <span>加载失败</span>
                          <button type="button" onClick={recent.loadMore}>
                            重试
                          </button>
                        </>
                      ) : (
                        '没有更多了'
                      )}
                    </div>
                  )}
                </>
              ) : recent.loading ? (
                <SkeletonRows n={4} kind="recent" />
              ) : recent.error ? (
                <div className="recent-foot error" title={recent.error}>
                  <span>动态没有加载出来</span>
                  <button type="button" onClick={recent.loadMore}>
                    重试
                  </button>
                </div>
              ) : (
                <div className="tree-empty" style={{ paddingLeft: 20 }}>
                  暂无
                </div>
              )}
            </div>
          )}
        </Section>
      </div>
    </section>
  );
}

/* ---------------- 未整理 Tab ----------------
   主区是「未整理」页（components/Inbox）；侧栏放「记一条未整理」、筛选、来源和时间段 */

function InboxSide() {
  const board = useData(s => s.board);
  const cur = useCurrent();
  const filter = useUI(s => s.filter);
  // 快照还没到：数量用工作区里的计数
  const count = board ? board.cols.inbox?.length ?? 0 : cur?.board.counts.inbox ?? null;
  // agent 新记下、人还没看过几条（PRD §10.5）
  const fresh = useData(s => newEntryCount(s.path, s.board, null));
  return (
    <section className="side-view" data-view="inbox" aria-label="未整理">
      <header className="side-head">
        <span className="side-title">未整理</span>
        {count == null ? (
          <Skeleton className="sk-side-count" w={22} h={20} r={999} />
        ) : (
          <span className="side-count mono">
            {count}
            {fresh > 0 && <span className="side-count-new" title={`${fresh} 条 agent 新记下、还没看的`}> · {fresh} 新</span>}
          </span>
        )}
      </header>
      <button className="new-entry inbox-entry" type="button" onClick={() => openCreator('inbox')}>
        <span className="ne-plus">
          <Icon name="plus" sw={2} />
        </span>
        <span className="ne-text">
          <b>记一条未整理</b>
          <span>计划、想法，先记下来</span>
        </span>
        <kbd>I</kbd>
      </button>
      <div className="side-scroll ib-side">
        <SideFilter value={filter} onChange={setFilter} placeholder="筛选标题、描述、#编号" />
        <InboxSideParts />
      </div>
    </section>
  );
}

/* ---------------- 项目 Tab：项目 → 白板的树 ---------------- */

/** 工作区还没到：两个项目、每个两三块白板的骨架 */
function SkeletonTree() {
  return (
    <div aria-hidden="true" aria-busy="true">
      {[3, 2].map((boards, p) => (
        <div key={p}>
          <div className="tree-row sk-row">
            <Skeleton w={22} h={22} r={6} />
            <Skeleton w={p ? 72 : 96} h={11} />
          </div>
          {Array.from({ length: boards }, (_, i) => (
            <div key={i} className="tree-row sk-row" style={{ paddingLeft: 44 }}>
              <Skeleton w={16} h={16} r={5} />
              <Skeleton w={['58%', '44%', '50%'][i]} h={10} />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

function ProjectsSide() {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const [q, setQ] = useState('');
  const [closed, setClosed] = useState<Set<string>>(() => new Set(pref.get<string[]>('projTreeClosed', [])));
  const projects = ws?.projects ?? [];
  const pk = path.split('/')[0];
  const query = q.trim().toLowerCase();

  const toggle = (key: string) =>
    setClosed(prev => {
      const n = new Set(prev);
      if (n.has(key)) n.delete(key);
      else {
        n.add(key);
      }
      pref.set('projTreeClosed', [...n]);
      if (!n.has(key)) window.dispatchEvent(new CustomEvent('project:focus', { detail: key }));
      return n;
    });
  const ctx = (kind: 'project' | 'board', id: string) => (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const btn = (e.target as HTMLElement).closest('button');
    openCtx(btn ? { kind, id, x: 0, y: 0, anchor: btn.getBoundingClientRect() } : { kind, id, x: e.clientX, y: e.clientY });
  };
  const key = (fn: () => void) => (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      fn();
    }
  };

  const rows = projects
    .map(p => {
      const pHit = !query || `${p.name}\n${p.key}\n${p.desc}`.toLowerCase().includes(query);
      const bs = p.boards.filter(b => pHit || `${b.name}\n${b.key}\n${b.desc}`.toLowerCase().includes(query));
      return { p, bs, show: pHit || bs.length > 0 };
    })
    .filter(r => r.show);

  return (
    <section className="side-view" data-view="projects">
      <header className="side-head">
        <span className="side-title">项目</span>
        {ws ? <span className="side-sub">{projects.length} 个项目</span> : <Skeleton w={52} h={10} />}
        <button aria-label="新建项目" className="icon-btn sm" type="button" data-tip="新建项目" data-tip-align="end" onClick={() => newProject()}>
          <Icon name="plus" sw={1.9} />
        </button>
      </header>
      <SideFilter value={q} onChange={setQ} placeholder="筛选项目和白板" />
      <div className="side-scroll" role="tree" onPointerLeave={() => canvasHighlight(null)}>
        {!ws ? (
          <SkeletonTree />
        ) : rows.length ? (
          rows.map(({ p, bs }) => {
            const isClosed = closed.has(p.key) && !query;
            const working = new Set(p.boards.flatMap(b => b.holders)).size;
            return (
              <div key={p.key} className={cx('pt-proj', isClosed && 'closed')} data-project={p.key} role="treeitem" aria-expanded={!isClosed}>
                <div
                  className={cx('pt-row proj', p.key === pk && 'in-current')}
                  tabIndex={0}
                  onClick={() => toggle(p.key)}
                  onKeyDown={key(() => toggle(p.key))}
                  onContextMenu={ctx('project', p.key)}
                  onPointerEnter={() => canvasHighlight(p.key)}
                >
                  <svg className="chev" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                    <path d="m6 8 4 4 4-4" />
                  </svg>
                  <ProjectAvatar name={p.name} color={p.color} className="sm" />
                  <span className="nm">{p.name}</span>
                  {working > 0 && (
                    <span className="pt-live" title={`${working} 个 agent 在工作`}>
                      <i />
                      {working}
                    </span>
                  )}
                  <span className="n">{p.boards.length}</span>
                  <button type="button" className="pt-more" aria-label="更多" onClick={ctx('project', p.key)}>
                    <Icon name="more" />
                  </button>
                </div>
                <div className="pt-children">
                  <div>
                    {bs.map(b => (
                      <div
                        key={b.path}
                        className={cx('pt-row brd', b.path === path && 'current')}
                        tabIndex={0}
                        role="treeitem"
                        style={{ ['--pc' as string]: p.color }}
                        onClick={() => openBoard(b.path)}
                        onKeyDown={key(() => openBoard(b.path))}
                        onContextMenu={ctx('board', b.path)}
                        onPointerEnter={() => canvasHighlight(b.path)}
                      >
                        <span className="pt-bic">
                          <Icon name="board" />
                        </span>
                        <span className="nm">{b.name}</span>
                        <span className="key mono">{b.key}</span>
                        {b.holders.length > 0 && (
                          <span className="pt-live" title={`${[...new Set(b.holders)].join('、')} 正在工作`}>
                            <i />
                            {b.holders.length}
                          </span>
                        )}
                        <span className="n" title="可领取状态里的卡片数">
                          {flowOf(b.statuses).ready(b.counts)}
                        </span>
                        <button type="button" className="pt-more" aria-label="更多" onClick={ctx('board', b.path)}>
                          <Icon name="more" />
                        </button>
                      </div>
                    ))}
                    <button type="button" className="pt-row add" onClick={() => newBoard(p.key)}>
                      <Icon name="plus" />
                      新建白板
                    </button>
                  </div>
                </div>
              </div>
            );
          })
        ) : (
          <div className="tree-empty" style={{ padding: '14px 20px' }}>
            没有匹配的项目或白板
          </div>
        )}
      </div>
    </section>
  );
}
