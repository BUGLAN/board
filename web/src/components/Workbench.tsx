/* ==========================================================================
   左侧操作台：活动栏 + 侧栏（PRD §10.1）
   - 活动栏：白板 / 项目 / 未整理 / 设置。再次点击当前 Tab 收起或展开侧栏
   - 侧栏宽度可以拖动调整；手机上活动栏变为底部 Tab 栏
   ========================================================================== */
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { newBoard, newProject, openBoard, openCard, triage } from '../lib/actions';
import { api } from '../lib/api';
import { labelDragJustEnded, labelPointerDown } from '../lib/labelDrag';
import type { Label } from '../lib/types';
import { boardFlow, flowOf } from '../lib/schema';
import { cx, mmss, ago, agentColor, pref, replay } from '../lib/util';
import { refresh, useCurrent, useData } from '../store/data';
import { openCreator, openCtx, setFilter, setSide, setTab, toastError, useUI, type Tab } from '../store/ui';
import { Avatar, ProjectAvatar, Title, agent, useNow } from './bits';
import { previewCols, useEntering, Veto } from './BoardView';
import { CardView } from './Card';
import { BoardIcon24, Icon, InboxIcon24, ProjectsIcon24, SettingsIcon24 } from './Icon';
import { SettingsNav } from './Settings';
import { canvasHighlight } from './Canvas';

const TABS: { tab: Tab; label: string; tip: string; icon: ReactNode }[] = [
  { tab: 'board', label: '白板', tip: '白板', icon: <BoardIcon24 /> },
  { tab: 'projects', label: '项目', tip: '项目 · 白板总览', icon: <ProjectsIcon24 /> },
  { tab: 'inbox', label: '未整理', tip: '未整理', icon: <InboxIcon24 /> },
];

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
  const item = (t: (typeof TABS)[number] | { tab: Tab; label: string; tip: string; icon: ReactNode }) => (
    <button
      key={t.tab}
      className={cx('ab-item', tab === t.tab && 'on')}
      type="button"
      data-tab={t.tab}
      data-tip={t.tip}
      data-tip-pos="right"
      aria-label={t.label}
      aria-current={tab === t.tab ? 'page' : undefined}
      onClick={() => setTab(t.tab, { toggle: true })}
    >
      {t.icon}
      <span className="ab-label">{t.label}</span>
    </button>
  );
  return (
    <nav className="activitybar" aria-label="操作台" ref={ref}>
      <i className="ab-indicator" aria-hidden="true" style={{ ['--ab-y' as string]: `${y}px` }} />
      {TABS.map(item)}
      <span className="ab-spacer" />
      {item({ tab: 'settings', label: '设置', tip: '设置', icon: <SettingsIcon24 /> })}
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
  comment: '评论了', timeout: '超时释放了', release: '强制释放了', delete: '删除了', label: '改了标签',
};

function Section({ id, title, n, closed, onToggle, children }: { id: string; title: string; n?: number; closed: boolean; onToggle: (id: string) => void; children: ReactNode }) {
  return (
    <section className={cx('tree-sec', closed && 'closed')} data-sec={id}>
      <button className="tree-head" type="button" onClick={() => onToggle(id)}>
        <svg className="chev" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="m6 8 4 4 4-4" />
        </svg>
        {title}
        {n != null && <span className="n">{n}</span>}
      </button>
      <div className="tree-body">
        <div>{children}</div>
      </div>
    </section>
  );
}

/** 标签：整块就是输入框。平时看着只是一排胶囊，点空白处光标跟在最后一个胶囊后面，输入名称回车新建。
    胶囊可以拖到卡片上，给卡片加上这个标签（lib/labelDrag.ts） */
function LabelBlock({
  project, labels, used, active, onPick,
}: {
  project: string;
  labels: Label[];
  used: Record<number, number>;
  active: string | undefined;
  onPick: (name: string) => void;
}) {
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const create = async () => {
    const n = name.trim();
    if (!n || busy) return;
    setBusy(true);
    try {
      await api.createLabel(project, n);
      await refresh();
      setName('');
    } catch (e) {
      toastError(e);
      replay(input.current, 'invalid');
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="lb-block">
      <div
        className="lb-field"
        onMouseDown={e => {
          // 点胶囊是筛选；点其余空白处把光标放进输入框（阻止默认，避免先失焦再聚焦）
          if ((e.target as HTMLElement).closest('button, input')) return;
          e.preventDefault();
          input.current?.focus();
        }}
      >
        {labels.map(l => (
          <button
            key={l.id}
            type="button"
            className={cx('lb lb-pick', active === l.name.toLowerCase() && 'on')}
            style={{ ['--lc' as string]: l.color }}
            title={`${active === l.name.toLowerCase() ? '取消筛选' : `只看「${l.name}」`} · 拖到卡片上加这个标签`}
            onPointerDown={e => labelPointerDown(e, l)}
            onClick={() => {
              if (!labelDragJustEnded()) onPick(l.name);
            }}
          >
            <i />
            {l.name}
            <em>{used[l.id] || 0}</em>
          </button>
        ))}
        <input
          ref={input}
          className="lb-input"
          value={name}
          placeholder={labels.length ? '' : '输入标签名，回车新建'}
          maxLength={24}
          spellCheck={false}
          autoComplete="off"
          aria-label="新标签"
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

function BoardSide() {
  const board = useData(s => s.board);
  const detailId = useData(s => s.detailId);
  const cur = useCurrent();
  const filter = useUI(s => s.filter);
  const now = useNow(1000);
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
  const logs = board ? [...board.log].slice(-10).reverse() : [];
  const newest = logs[0]?.id ?? 0;
  useEffect(() => {
    lastSeen.current = newest;
  }, [newest]);
  useEffect(() => {
    lastSeen.current = 0;
  }, [cur?.board.path]);

  const labels = cur?.project.labels ?? [];
  const used = useMemo(() => {
    const m: Record<number, number> = {};
    if (board) for (const c of Object.values(board.cards)) for (const l of c.labels) m[l] = (m[l] || 0) + 1;
    return m;
  }, [board]);
  const active = (filter.trim().toLowerCase().match(/^(?:label|标签)[:：](.+)$/) || [])[1];
  const holders = board ? boardFlow(board).list.flatMap(s => (board.cols[s.key] ?? []).map(id => board.cards[id])).filter(c => c?.holder) : [];

  return (
    <section className="side-view" data-view="board">
      <header className="side-head">
        <span className="side-title">白板</span>
        <span className="side-sub">{cur?.board.path}</span>
        <button
          className="icon-btn sm"
          type="button"
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
      <SideFilter value={filter} onChange={setFilter} placeholder="筛选卡片：标题、描述、#ID、agent、label:标签" />
      <div className="side-scroll scroll">
        <Section id="labels" title="标签" n={labels.length} closed={closed.has('labels')} onToggle={toggle}>
          {cur && (
            <LabelBlock
              project={cur.project.key}
              labels={labels}
              used={used}
              active={active}
              onPick={name => {
                const q = `label:${name}`;
                setFilter(filter.trim().toLowerCase() === q.toLowerCase() ? '' : q);
              }}
            />
          )}
        </Section>
        <Section id="working" title="正在工作" n={holders.length} closed={closed.has('working')} onToggle={toggle}>
          {holders.length ? (
            holders.map(c => {
              const left = (c.expiresAt ?? 0) - now;
              return (
                <button
                  key={c.id}
                  className={cx('tree-row card-row', detailId === c.id && 'active')}
                  type="button"
                  style={{ paddingLeft: 20 }}
                  onClick={() => openCard(c.id)}
                >
                  <Avatar actor={agent(c.holder!)} />
                  <span className="tt">
                    <span className="mono" style={{ color: agentColor(c.holder!) }}>
                      {c.holder}
                    </span>{' '}
                    · #{c.id} <Title text={c.title} />
                  </span>
                  <span className={cx('tm', left < 5 * 60e3 && 'urgent')}>{mmss(left)}</span>
                </button>
              );
            })
          ) : (
            <div className="tree-empty" style={{ paddingLeft: 20 }}>
              没有 agent 在工作
            </div>
          )}
        </Section>
        <Section id="recent" title="最近动态" closed={closed.has('recent')} onToggle={toggle}>
          {logs.length ? (
            logs.map(ev => (
              <div key={ev.id} className={cx('recent-row', lastSeen.current > 0 && ev.id > lastSeen.current && 'new')}>
                <Avatar actor={ev.actor} />
                <div>
                  <b style={ev.actor.kind === 'agent' ? { color: agentColor(ev.actor.name) } : undefined}>
                    {ev.actor.kind === 'human' ? '你' : ev.actor.kind === 'system' ? '系统' : ev.actor.name}
                  </b>{' '}
                  {OP[ev.op] || ev.op} <span className="mono">#{ev.cardId}</span>
                  <span className="when">
                    {ago(ev.at, now)}
                    {ev.via === 'cli' ? ' · CLI' : ''}
                  </span>
                </div>
              </div>
            ))
          ) : (
            <div className="tree-empty" style={{ paddingLeft: 20 }}>
              暂无
            </div>
          )}
        </Section>
      </div>
    </section>
  );
}

/* ---------------- 未整理 Tab ---------------- */

function InboxSide() {
  const board = useData(s => s.board);
  const generation = useData(s => s.generation);
  const detailId = useData(s => s.detailId);
  const cur = useCurrent();
  const drag = useUI(s => s.drag);
  const flashes = useUI(s => s.flashes);
  const filter = useUI(s => s.filter.trim().toLowerCase());
  const { entering } = useEntering(board, generation);
  const ids = board ? previewCols(board.cols, drag).inbox ?? [] : [];
  const start = boardFlow(board).claim?.name ?? '待办';
  const count = board?.cols.inbox?.length ?? 0;
  const over = drag?.over === 'inbox';
  const empty = ids.length === 0;
  return (
    <section
      className={cx(
        'side-view inbox-zone',
        empty && 'is-empty',
        drag && over && drag.valid && !(drag.fromCol === 'inbox' && drag.col === 'inbox') && 'drop-ok',
        drag && over && !drag.valid && 'drop-bad',
      )}
      data-view="inbox"
      data-drop="inbox"
      aria-label="未整理"
    >
      <header className="side-head">
        <span className="side-title">未整理</span>
        <span className="side-count mono">{count}</span>
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
      <div className="side-scroll scroll inbox-scroll">
        <div className="col-list inbox-list" data-list="inbox">
          {board &&
            ids.map(id => {
              const c = board.cards[id];
              if (!c) return null;
              return (
                <CardView
                  key={id}
                  card={drag?.id === id ? { ...c, column: 'inbox' } : c}
                  labels={cur?.project.labels ?? []}
                  filter={filter}
                  open={detailId === id}
                  placeholder={drag?.id === id}
                  flash={flashes[id]}
                  entering={entering.has(id)}
                  claimName={start}
                  onTriage={triage}
                />
              );
            })}
        </div>
        <div className="inbox-empty">
          <div className="ie-art" aria-hidden="true">
            <i />
            <i />
            <i />
          </div>
          <b>全部整理完了</b>
          <span>
            想到什么先记在这里，
            <br />
            整理清楚后再放进「{start}」。
          </span>
        </div>
      </div>
      <Veto drag={over ? drag : null} />
    </section>
  );
}

/* ---------------- 项目 Tab：项目 → 白板的树 ---------------- */

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
        <span className="side-sub">{projects.length} 个项目</span>
        <button className="icon-btn sm" type="button" data-tip="新建项目" data-tip-align="end" onClick={() => newProject()}>
          <Icon name="plus" sw={1.9} />
        </button>
      </header>
      <SideFilter value={q} onChange={setQ} placeholder="筛选项目和白板" />
      <div className="side-scroll scroll" role="tree" onPointerLeave={() => canvasHighlight(null)}>
        {rows.length ? (
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
                        <span className="n" title={flowOf(b.statuses).claim?.name}>
                          {b.counts[flowOf(b.statuses).claim?.key ?? 'todo'] ?? 0}
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
