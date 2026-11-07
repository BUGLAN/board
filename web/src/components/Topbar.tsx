/* ==========================================================================
   顶栏：品牌、项目 / 白板切换器、搜索（命令面板入口）、正在工作的 agent
   桌面端：顶栏同时是窗口标题栏，可以拖动窗口；Windows 上右侧是窗口按钮
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { newBoard, openBoard, openCard } from '../lib/actions';
import { startWindowDrag, toggleMaximize } from '../lib/platform';
import { boardFlow, flowOf, timing } from '../lib/schema';
import { agentColor, clock, cx, MOD, reduceMotion } from '../lib/util';
import { allBoards, useCurrent, useData } from '../store/data';
import { closeSwitcher, openPalette, openSwitcher, setTab, useUI } from '../store/ui';
import { Avatar, ProjectAvatar, agent, useNow } from './bits';
import { Icon } from './Icon';
import { WindowControls } from './WindowControls';

export function Topbar() {
  const onDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest('button, input, a, .presence, .pj-menu')) return;
    if (e.detail === 2) toggleMaximize();
    else startWindowDrag();
  };
  return (
    <header className="topbar enter-down" onMouseDown={onDown}>
      <div className="topbar-left">
        <div className="brand">
        <div className="logo" aria-hidden="true">
            <i style={{ ['--h' as string]: '100%' }} />
            <i style={{ ['--h' as string]: '70%' }} />
            <i style={{ ['--h' as string]: '46%' }} />
            <i style={{ ['--h' as string]: '82%' }} />
          </div>
          <span className="brand-name">Board</span>
        </div>
        <span className="brand-slash" aria-hidden="true">
          /
        </span>
        <ProjectSwitch />
      </div>
      <button className="search-trigger" type="button" onClick={() => openPalette()}>
        <Icon name="search" />
        <span className="search-label">搜索卡片、跳转或执行命令</span>
        <span className="search-keys">
          <kbd>{MOD}</kbd>
          <kbd>K</kbd>
        </span>
      </button>
      <div className="topbar-right">
        <Presence />
        <WindowControls />
      </div>
    </header>
  );
}

function Presence() {
  const board = useData(s => s.board);
  const now = useNow(1000);
  const holders = board ? boardFlow(board).list.flatMap(s => (board.cols[s.key] ?? []).map(id => board.cards[id])).filter(c => c?.holder) : [];
  return (
    <div className="presence" aria-label="正在工作的 agent">
      {holders.map((c, i) => {
        const t = timing(c, now);
        return (
          // 悬停时展开：头像右边滑出这一次的耗时
          <span
            key={c.holder}
            className={cx('pres-item', t.beat)}
            style={{ ['--c' as string]: agentColor(c.holder!), ['--k' as string]: holders.length - 1 - i }}
            data-tip={`${c.holder} 正在做 #${c.id}${t.runs > 1 ? ` · 累计 ${clock(t.total)}（${t.runs} 次）` : ''}`}
            data-tip-align="end"
            onClick={() => openCard(c.id)}
          >
            <Avatar actor={agent(c.holder!)} />
            <i className="pres-pulse" />
            <span className="pres-time">{t.cur != null ? clock(t.cur) : t.beat === 'wait' ? '等待心跳' : '已结束'}</span>
          </span>
        );
      })}
    </div>
  );
}

/* ---------------- 项目 / 白板切换器 ---------------- */

function ProjectSwitch() {
  const cur = useCurrent();
  const ws = useData(s => s.ws);
  const open = useUI(s => s.switcher);
  const ref = useRef<HTMLButtonElement>(null);
  const elsewhere = allBoards(ws).filter(b => b.path !== cur?.board.path && b.holders.length).length;
  const seen = useRef('');
  useEffect(() => {
    const p = cur?.board.path ?? '';
    if (seen.current && p && seen.current !== p && !reduceMotion())
      ref.current?.animate([{ transform: 'scale(.96)' }, { transform: 'none' }], { duration: 320, easing: 'cubic-bezier(.34,1.56,.64,1)' });
    seen.current = p;
  }, [cur?.board.path]);
  return (
    <>
      <button
        ref={ref}
        className="project-switch"
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label="切换白板（P）"
        onClick={() => (open ? closeSwitcher() : openSwitcher())}
      >
        <span className="pj-av-slot">{cur && <ProjectAvatar name={cur.project.name} color={cur.project.color} className="sm" />}</span>
        <span className="pj-label">{cur?.project.name ?? '…'}</span>
        <span className="pj-sep">/</span>
        <span className="pj-board">{cur?.board.name ?? ''}</span>
        {elsewhere > 0 && <span className="pj-elsewhere" title={`另外 ${elsewhere} 块白板上有 agent 正在工作`} />}
        <svg className="pj-chev" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="m6.5 8 3.5-3.5L13.5 8M6.5 12l3.5 3.5 3.5-3.5" />
        </svg>
      </button>
      {open && <ProjectMenu anchor={ref.current} />}
    </>
  );
}

function ProjectMenu({ anchor }: { anchor: HTMLElement | null }) {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const [q, setQ] = useState('');
  const [active, setActive] = useState(() => Math.max(0, allBoards(ws).findIndex(b => b.path === path)));
  const [pos, setPos] = useState({ left: 8, top: 60 });
  const menu = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [on, setOn] = useState(false);

  useLayoutEffect(() => {
    const r = anchor?.getBoundingClientRect();
    if (r) setPos({ left: Math.max(8, Math.min(r.left, innerWidth - 388)), top: r.bottom + 8 });
    requestAnimationFrame(() => setOn(true));
    setTimeout(() => input.current?.focus(), 20);
  }, [anchor]);

  useEffect(() => {
    const down = (e: PointerEvent) => {
      const t = e.target as HTMLElement;
      if (!t.closest('.pj-menu, .project-switch')) closeSwitcher();
    };
    document.addEventListener('pointerdown', down);
    window.addEventListener('resize', closeSwitcher);
    return () => {
      document.removeEventListener('pointerdown', down);
      window.removeEventListener('resize', closeSwitcher);
    };
  }, []);

  const query = q.trim().toLowerCase();
  const groups = (ws?.projects ?? [])
    .map(p => ({ p, bs: p.boards.filter(b => !query || `${p.name}\n${p.key}\n${b.name}\n${b.key}\n${b.desc}`.toLowerCase().includes(query)) }))
    .filter(g => g.bs.length);
  const items = groups.flatMap(g => g.bs);
  const act = Math.min(active, Math.max(0, items.length - 1));

  useEffect(() => {
    menu.current?.querySelector('.pj-row.active')?.scrollIntoView({ block: 'nearest' });
  }, [act]);

  let i = 0;
  return (
    <div className={cx('pj-menu', on && 'on')} ref={menu} role="dialog" aria-label="切换白板" style={{ left: pos.left, top: pos.top }}>
      <div className="pj-search">
        <Icon name="search" />
        <input
          ref={input}
          value={q}
          placeholder="搜索项目或白板…"
          spellCheck={false}
          autoComplete="off"
          onChange={e => {
            setQ(e.target.value);
            setActive(0);
          }}
          onKeyDown={e => {
            if (e.key === 'ArrowDown') {
              e.preventDefault();
              setActive((act + 1) % Math.max(1, items.length));
            } else if (e.key === 'ArrowUp') {
              e.preventDefault();
              setActive((act - 1 + items.length) % Math.max(1, items.length));
            } else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
              e.preventDefault();
              if (items[act]) openBoard(items[act].path);
            } else if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              closeSwitcher();
            }
          }}
        />
      </div>
      <div className="pj-head">
        <span>项目 / 白板</span>
        <span>进行中 · 待办</span>
      </div>
      <div className="pj-list scroll" role="listbox">
        {groups.length === 0 && <div className="pj-empty">没有匹配的白板</div>}
        {groups.map(({ p, bs }) => (
          <div key={p.key}>
            <div className="pj-group">
              <ProjectAvatar name={p.name} color={p.color} className="sm" />
              <b>{p.name}</b>
              <span className="mono">{p.key}</span>
            </div>
            {bs.map(b => {
              const idx = i++;
              const current = b.path === path;
              return (
                <button
                  key={b.path}
                  type="button"
                  className={cx('pj-row', current && 'current', idx === act && 'active')}
                  role="option"
                  aria-selected={current}
                  onClick={() => openBoard(b.path)}
                  onPointerMove={() => idx !== act && setActive(idx)}
                >
                  <span className="pj-bic" style={{ ['--pc' as string]: p.color }}>
                    <Icon name="board" />
                  </span>
                  <span className="pj-main">
                    <b>{b.name}</b>
                    <span className="mono">{b.path}</span>
                  </span>
                  <span className="pj-stats">
                    {b.holders.length > 0 && (
                      <span className="pj-live" title={`${[...new Set(b.holders)].join('、')} 正在工作`}>
                        <i />
                        {b.holders.length}
                      </span>
                    )}
                    <span title="可领取状态里的卡片数">{flowOf(b.statuses).ready(b.counts)}</span>
                  </span>
                  <span className="pj-check">{current && <Icon name="check" />}</span>
                </button>
              );
            })}
          </div>
        ))}
      </div>
      <div className="pj-foot">
        <button type="button" onClick={() => newBoard()}>
          <Icon name="plus" sw={1.9} />
          新建白板
        </button>
        <button
          type="button"
          onClick={() => {
            closeSwitcher();
            setTab('projects');
          }}
        >
          <Icon name="folder" />
          项目总览
        </button>
      </div>
    </div>
  );
}

