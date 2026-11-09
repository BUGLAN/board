/* ==========================================================================
   项目 Tab 的主内容区：无限画布（PRD §10.4）
   - 项目节点 ──连线── 白板节点；有 agent 在工作的连线显示为流动状态
   - 拖动空白处平移；⌘/Ctrl + 滚轮或双指捏合缩放；放大、缩小、100%、适应画布、定位到当前白板
   - 拖动节点调整位置（保存并在所有窗口同步）
   - 双击白板节点打开；双击空白处新建项目；项目节点右侧「+」新建白板
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { newBoard, newProject, openBoard } from '../lib/actions';
import { api } from '../lib/api';
import { flowOf } from '../lib/schema';
import type { BoardInfo, Pos, ProjectInfo } from '../lib/types';
import { cx, pref, reduceMotion, replay } from '../lib/util';
import { onEvents, optimistic, patchProject, useData, type Patch } from '../store/data';
import { openCtx, toastError, useUI } from '../store/ui';
import { Avatar, agent } from './bits';
import { Icon } from './Icon';

const K_MIN = 0.25;
const K_MAX = 2;
interface View {
  x: number;
  y: number;
  k: number;
}

let setHighlightExternal: ((id: string | null) => void) | null = null;
/** 侧栏项目树悬停时，画布上对应的节点和连线高亮 */
export const canvasHighlight = (id: string | null) => setHighlightExternal?.(id);

export function CanvasPage() {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const tab = useUI(s => s.tab);
  const canvas = useRef<HTMLDivElement>(null);
  const world = useRef<HTMLDivElement>(null);
  const links = useRef<SVGSVGElement>(null);
  const zoomLabel = useRef<HTMLButtonElement>(null);
  const saved = pref.get<View | null>('canvas.view', null);
  const view = useRef<View>(saved ?? { x: 140, y: 120, k: 0.85 });
  const [override, setOverride] = useState<{ type: 'project' | 'board'; id: string; pos: Pos } | null>(null);
  const [hl, setHl] = useState<string | null>(null);
  const [intro, setIntro] = useState(0);
  const tween = useRef(0);
  const pk = path.split('/')[0];
  const projects = ws?.projects ?? [];

  useEffect(() => {
    setHighlightExternal = setHl;
    return () => {
      setHighlightExternal = null;
    };
  }, []);

  /* ---------------- 视图变换 ---------------- */
  const apply = () => {
    const v = view.current;
    if (world.current) world.current.style.transform = `translate(${v.x}px, ${v.y}px) scale(${v.k})`;
    const c = canvas.current;
    if (c) {
      c.style.setProperty('--cv-x', `${v.x}px`);
      c.style.setProperty('--cv-y', `${v.y}px`);
      c.style.setProperty('--cv-s', `${24 * v.k}px`);
    }
    if (zoomLabel.current) zoomLabel.current.textContent = `${Math.round(v.k * 100)}%`;
  };
  const save = () => pref.set('canvas.view', view.current);
  const clampK = (k: number) => Math.max(K_MIN, Math.min(K_MAX, k));
  const zoomTarget = (cx: number, cy: number, k: number): View => {
    const v = view.current;
    k = clampK(k);
    return { k, x: cx - (cx - v.x) * (k / v.k), y: cy - (cy - v.y) * (k / v.k) };
  };
  const animateTo = (target: View, ms = 420) => {
    cancelAnimationFrame(tween.current);
    if (reduceMotion()) {
      view.current = target;
      apply();
      save();
      return;
    }
    const from = { ...view.current };
    const t0 = performance.now();
    const ease = (t: number) => 1 - Math.pow(1 - t, 3);
    const step = (now: number) => {
      const t = Math.min(1, (now - t0) / ms);
      const e = ease(t);
      view.current = { x: from.x + (target.x - from.x) * e, y: from.y + (target.y - from.y) * e, k: from.k + (target.k - from.k) * e };
      apply();
      if (t < 1) tween.current = requestAnimationFrame(step);
      else save();
    };
    tween.current = requestAnimationFrame(step);
  };
  const nodes = () => [...(canvas.current?.querySelectorAll<HTMLElement>('.cv-node') ?? [])];
  const fit = () => {
    const ns = nodes();
    const c = canvas.current;
    if (!ns.length || !c) return;
    let x1 = Infinity;
    let y1 = Infinity;
    let x2 = -Infinity;
    let y2 = -Infinity;
    for (const n of ns) {
      const x = parseFloat(n.style.left);
      const y = parseFloat(n.style.top);
      x1 = Math.min(x1, x);
      y1 = Math.min(y1, y);
      x2 = Math.max(x2, x + n.offsetWidth);
      y2 = Math.max(y2, y + n.offsetHeight);
    }
    const pad = 90;
    const top = 60;
    const w = c.clientWidth - pad * 2;
    const h = c.clientHeight - pad * 2 - top;
    const k = clampK(Math.min(w / (x2 - x1), h / (y2 - y1), 1.1));
    animateTo({ k, x: pad + (w - (x2 - x1) * k) / 2 - x1 * k, y: pad + top + (h - (y2 - y1) * k) / 2 - y1 * k });
  };
  const focusNode = (sel: string, k = Math.max(view.current.k, 0.9)) => {
    const n = canvas.current?.querySelector<HTMLElement>(sel);
    const c = canvas.current;
    if (!n || !c) return;
    const cx = parseFloat(n.style.left) + n.offsetWidth / 2;
    const cy = parseFloat(n.style.top) + n.offsetHeight / 2;
    animateTo({ k, x: c.clientWidth / 2 - cx * k, y: (c.clientHeight + 40) / 2 - cy * k });
    replay(n, 'ping');
  };
  const local = (e: { clientX: number; clientY: number }) => {
    const r = canvas.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  useLayoutEffect(apply, []);

  // 切到项目 Tab：入场动画；没有保存过视图时适应画布
  useEffect(() => {
    if (tab !== 'projects') return;
    setIntro(i => i + 1);
    const t = requestAnimationFrame(() => {
      drawLinks();
      if (!pref.get('canvas.view', null)) fit();
    });
    return () => cancelAnimationFrame(t);
  }, [tab]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!intro || reduceMotion()) return;
    const c = canvas.current;
    replay(c, 'intro');
    const t = setTimeout(() => c?.classList.remove('intro'), 1600);
    return () => clearTimeout(t);
  }, [intro]);

  // 新建白板 / 项目后定位过去
  useEffect(() => {
    const onBoard = (e: Event) => setTimeout(() => focusNode(`[data-board="${CSS.escape((e as CustomEvent).detail)}"]`), 80);
    const onProject = (e: Event) => focusNode(`[data-project="${CSS.escape((e as CustomEvent).detail)}"]`);
    window.addEventListener('board:focus', onBoard);
    window.addEventListener('project:focus', onProject);
    return () => {
      window.removeEventListener('board:focus', onBoard);
      window.removeEventListener('project:focus', onProject);
    };
  });

  // 某块白板有新动作：节点闪一下
  useEffect(
    () =>
      onEvents(events => {
        for (const e of events) replay(canvas.current?.querySelector(`[data-board="${CSS.escape(e.board)}"]`), 'ping');
      }),
    [],
  );

  /* ---------------- 连线 ---------------- */
  const drawLinks = () => {
    const svg = links.current;
    const c = canvas.current;
    if (!svg || !c) return;
    let html = '';
    const anchor = (n: HTMLElement, side: 'in' | 'out') => {
      const x = parseFloat(n.style.left);
      const y = parseFloat(n.style.top);
      return side === 'out' ? { x: x + n.offsetWidth, y: y + n.offsetHeight / 2 } : { x, y: y + n.offsetHeight / 2 };
    };
    for (const p of projects) {
      const pn = c.querySelector<HTMLElement>(`[data-project="${CSS.escape(p.key)}"]`);
      if (!pn) continue;
      const a = anchor(pn, 'out');
      for (const b of p.boards) {
        const bn = c.querySelector<HTMLElement>(`[data-board="${CSS.escape(b.path)}"]`);
        if (!bn) continue;
        const z = anchor(bn, 'in');
        const dx = Math.max(60, Math.abs(z.x - a.x) * 0.5);
        const d = `M${a.x},${a.y} C${a.x + dx},${a.y} ${z.x - dx},${z.y} ${z.x},${z.y}`;
        const active = b.holders.length > 0;
        const lit = hl === b.path || hl === p.key;
        html += `<g class="cv-link${active ? ' active' : ''}${b.path === path ? ' current' : ''}${lit ? ' hl' : ''}" style="--pc:${p.color}">
          <path class="halo" d="${d}"/><path class="line" d="${d}" pathLength="1"/>${active ? `<path class="flow" d="${d}"/>` : ''}
          <circle class="dot" cx="${a.x}" cy="${a.y}" r="4"/><circle class="dot" cx="${z.x}" cy="${z.y}" r="4"/></g>`;
      }
    }
    svg.innerHTML = html;
  };
  useLayoutEffect(drawLinks);
  useEffect(() => {
    const r = () => drawLinks();
    window.addEventListener('resize', r);
    document.fonts?.ready.then(r);
    return () => window.removeEventListener('resize', r);
  });

  /* ---------------- 交互 ---------------- */
  const drag = useRef<
    | { kind: 'pan'; sx: number; sy: number; x0: number; y0: number; moved: boolean; pid: number }
    | { kind: 'node'; type: 'project' | 'board'; id: string; sx: number; sy: number; x0: number; y0: number; moved: boolean; pid: number; pos: Pos }
    | null
  >(null);
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const pinch = useRef<{ d: number; k: number; cx: number; cy: number; x: number; y: number } | null>(null);

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0 && e.pointerType === 'mouse') return;
    pointers.current.set(e.pointerId, local(e));
    if (pointers.current.size === 2) {
      const [p1, p2] = [...pointers.current.values()];
      const v = view.current;
      pinch.current = { d: Math.hypot(p1.x - p2.x, p1.y - p2.y), k: v.k, cx: (p1.x + p2.x) / 2, cy: (p1.y + p2.y) / 2, x: v.x, y: v.y };
      drag.current = null;
      return;
    }
    const t = e.target as HTMLElement;
    if (t.closest('button, .cv-head, .cv-zoom')) return;
    const node = t.closest<HTMLElement>('.cv-node');
    const l = local(e);
    if (node) {
      const type = node.dataset.board ? 'board' : 'project';
      const x0 = parseFloat(node.style.left);
      const y0 = parseFloat(node.style.top);
      drag.current = { kind: 'node', type, id: (node.dataset.board || node.dataset.project)!, sx: l.x, sy: l.y, x0, y0, moved: false, pid: e.pointerId, pos: { x: x0, y: y0 } };
    } else {
      drag.current = { kind: 'pan', sx: l.x, sy: l.y, x0: view.current.x, y0: view.current.y, moved: false, pid: e.pointerId };
    }
  };

  const onPointerMove = (e: React.PointerEvent) => {
    if (pointers.current.has(e.pointerId)) pointers.current.set(e.pointerId, local(e));
    const pc = pinch.current;
    if (pc && pointers.current.size === 2) {
      const [p1, p2] = [...pointers.current.values()];
      const k = clampK((pc.k * Math.hypot(p1.x - p2.x, p1.y - p2.y)) / pc.d);
      view.current = { k, x: pc.cx - (pc.cx - pc.x) * (k / pc.k), y: pc.cy - (pc.cy - pc.y) * (k / pc.k) };
      apply();
      return;
    }
    const d = drag.current;
    if (!d) return;
    const l = local(e);
    const dx = l.x - d.sx;
    const dy = l.y - d.sy;
    if (!d.moved && Math.hypot(dx, dy) < 4) return;
    if (!d.moved) {
      // 真正开始拖动时才捕获指针，否则 click / dblclick 会被重定向到画布
      try {
        canvas.current?.setPointerCapture(d.pid);
      } catch {
        /* 忽略 */
      }
      if (d.kind === 'pan') canvas.current?.classList.add('panning');
    }
    d.moved = true;
    if (d.kind === 'pan') {
      view.current = { ...view.current, x: d.x0 + dx, y: d.y0 + dy };
      apply();
    } else {
      d.pos = { x: d.x0 + dx / view.current.k, y: d.y0 + dy / view.current.k };
      setOverride({ type: d.type, id: d.id, pos: d.pos });
    }
  };

  const onPointerUp = async (e: React.PointerEvent) => {
    pointers.current.delete(e.pointerId);
    if (pointers.current.size < 2) pinch.current = null;
    const d = drag.current;
    drag.current = null;
    canvas.current?.classList.remove('panning');
    if (!d || d.kind === 'pan') {
      save();
      return;
    }
    if (!d.moved) return;
    // 松手：新位置交给工作区补丁（拖动中的 override 立即清掉，下一次拖动不会被这次的保存冲掉）；被拒绝时回到原处并提示
    const pos = d.pos;
    const patch: Patch =
      d.type === 'project'
        ? patchProject(d.id, p => ({ ...p, pos }))
        : patchProject(d.id.split('/')[0], p => ({ ...p, boards: p.boards.map(b => (b.path === d.id ? { ...b, pos } : b)) }));
    const done = optimistic(patch, () => api.layout(d.type, d.id, pos));
    setOverride(null);
    try {
      await done;
    } catch (err) {
      toastError(err);
    }
  };

  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    let t = 0;
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const l = local(e);
      if (e.ctrlKey || e.metaKey) view.current = zoomTarget(l.x, l.y, view.current.k * Math.exp(-e.deltaY * 0.0022));
      else view.current = { ...view.current, x: view.current.x - e.deltaX, y: view.current.y - e.deltaY };
      apply();
      clearTimeout(t);
      t = window.setTimeout(save, 200);
    };
    c.addEventListener('wheel', wheel, { passive: false });
    return () => c.removeEventListener('wheel', wheel);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const onDoubleClick = (e: React.MouseEvent) => {
    const t = e.target as HTMLElement;
    if (t.closest('button, .cv-head, .cv-zoom')) return;
    const b = t.closest<HTMLElement>('.cv-board');
    if (b) return openBoard(b.dataset.board!);
    if (t.closest('.cv-node')) return;
    const l = local(e);
    const v = view.current;
    newProject({ pos: { x: Math.round((l.x - v.x) / v.k - 130), y: Math.round((l.y - v.y) / v.k - 40) } });
  };

  const zoom = (z: 'in' | 'out' | 'reset' | 'fit' | 'current') => {
    const c = canvas.current!;
    const cx = c.clientWidth / 2;
    const cy = c.clientHeight / 2;
    if (z === 'in') animateTo(zoomTarget(cx, cy, view.current.k * 1.25), 220);
    if (z === 'out') animateTo(zoomTarget(cx, cy, view.current.k / 1.25), 220);
    if (z === 'reset') animateTo(zoomTarget(cx, cy, 1), 260);
    if (z === 'fit') fit();
    if (z === 'current') focusNode(`[data-board="${CSS.escape(path)}"]`, 1);
  };

  const ctx = (kind: 'project' | 'board', id: string) => (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const btn = (e.target as HTMLElement).closest('button');
    openCtx(btn ? { kind, id, x: 0, y: 0, anchor: btn.getBoundingClientRect() } : { kind, id, x: e.clientX, y: e.clientY });
  };

  const posOf = (type: 'project' | 'board', id: string, p: Pos) => (override && override.type === type && override.id === id ? override.pos : p);
  const nb = projects.reduce((a, p) => a + p.boards.length, 0);
  let i = 0;

  return (
    <div className="page page-canvas" data-page="canvas">
      <div
        className="canvas"
        ref={canvas}
        tabIndex={0}
        aria-label="项目与白板画布"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onDoubleClick={onDoubleClick}
        onPointerOver={e => {
          const n = (e.target as HTMLElement).closest<HTMLElement>('.cv-node');
          if (n && !drag.current) setHl(n.dataset.board || n.dataset.project || null);
        }}
        onPointerOut={e => {
          const n = (e.target as HTMLElement).closest<HTMLElement>('.cv-node');
          if (n && !n.contains(e.relatedTarget as Node)) setHl(null);
        }}
        onKeyDown={e => {
          if (e.target !== canvas.current) return;
          const step = 60;
          const v = view.current;
          const c = canvas.current!;
          if (e.key === 'ArrowLeft') view.current = { ...v, x: v.x + step };
          else if (e.key === 'ArrowRight') view.current = { ...v, x: v.x - step };
          else if (e.key === 'ArrowUp') view.current = { ...v, y: v.y + step };
          else if (e.key === 'ArrowDown') view.current = { ...v, y: v.y - step };
          else if (e.key === '=' || e.key === '+') return animateTo(zoomTarget(c.clientWidth / 2, c.clientHeight / 2, v.k * 1.25), 200);
          else if (e.key === '-') return animateTo(zoomTarget(c.clientWidth / 2, c.clientHeight / 2, v.k / 1.25), 200);
          else if (e.key === '0') return fit();
          else return;
          e.preventDefault();
          apply();
          save();
        }}
      >
        <div className="cv-world" ref={world}>
          <svg className="cv-links" ref={links} aria-hidden="true" />
          <div className="cv-nodes">
            {projects.map(p => (
              <ProjectGroup
                key={p.key}
                p={p}
                pIndex={i++}
                bIndex={() => i++}
                current={p.key === pk}
                path={path}
                hl={hl}
                override={override}
                posOf={posOf}
                ctx={ctx}
              />
            ))}
          </div>
        </div>
        <div className="cv-head">
          <nav className="crumbs">
            <b>项目总览</b>
          </nav>
          <span className="cv-stat">
            {projects.length} 个项目 · {nb} 块白板
          </span>
          <div className="page-actions">
            <button className="btn sm" type="button" onClick={() => newBoard(pk)}>
              <Icon name="plus" sw={1.9} />
              新建白板
            </button>
            <button className="btn sm primary" type="button" onClick={() => newProject()}>
              <Icon name="plus" sw={1.9} />
              新建项目
            </button>
          </div>
        </div>
        <div className="cv-zoom" role="toolbar" aria-label="缩放">
          <button type="button" aria-label="缩小" onClick={() => zoom('out')}>
            −
          </button>
          <button type="button" className="cv-zoom-label" ref={zoomLabel} aria-label="重置为 100%" onClick={() => zoom('reset')}>
            100%
          </button>
          <button type="button" aria-label="放大" onClick={() => zoom('in')}>
            +
          </button>
          <i />
          <button type="button" data-tip="适应画布" data-tip-pos="top" data-tip-align="end" onClick={() => zoom('fit')}>
            <Icon name="fit" />
          </button>
          <button type="button" data-tip="定位到当前白板" data-tip-pos="top" data-tip-align="end" onClick={() => zoom('current')}>
            <Icon name="target" />
          </button>
        </div>
      </div>
    </div>
  );
}

function ProjectGroup({
  p, pIndex, bIndex, current, path, hl, override, posOf, ctx,
}: {
  p: ProjectInfo;
  pIndex: number;
  bIndex: () => number;
  current: boolean;
  path: string;
  hl: string | null;
  override: { id: string } | null;
  posOf: (type: 'project' | 'board', id: string, p: Pos) => Pos;
  ctx: (kind: 'project' | 'board', id: string) => (e: React.MouseEvent) => void;
}) {
  const working = new Set(p.boards.flatMap(b => b.holders)).size;
  const pos = posOf('project', p.key, p.pos);
  return (
    <>
      <div
        className={cx('cv-node cv-project', current && 'current', hl === p.key && 'hl', override?.id === p.key && 'dragging')}
        data-project={p.key}
        style={{ left: pos.x, top: pos.y, ['--pc' as string]: p.color, ['--i' as string]: pIndex }}
        onContextMenu={ctx('project', p.key)}
      >
        <span className="pj-av lg" style={{ ['--pc' as string]: p.color }}>
          {[...p.name.trim()][0]?.toUpperCase() || '?'}
        </span>
        <div className="cvp-text">
          <b>{p.name}</b>
          <span className="mono">{p.key}</span>
          {p.desc && <span className="cvp-desc">{p.desc}</span>}
          <span className="cvp-stats">
            {p.boards.length} 块白板{working > 0 && <> · <em>{working} 个 agent 在工作</em></>}
          </span>
        </div>
        <button type="button" className="cv-more" aria-label="更多" onClick={ctx('project', p.key)}>
          <Icon name="more" />
        </button>
        <span className="cv-port out" />
        <button
          type="button"
          className="cv-add"
          data-tip={`在「${p.name}」里新建白板`}
          data-tip-pos="right"
          onClick={() => {
            const ys = p.boards.map(b => b.pos.y);
            newBoard(p.key, { pos: { x: pos.x + 420, y: ys.length ? Math.max(...ys) + 200 : pos.y - 10 } });
          }}
        >
          <Icon name="plus" />
        </button>
      </div>
      {p.boards.map(b => (
        <BoardNode key={b.path} b={b} p={p} i={bIndex()} current={b.path === path} hl={hl === b.path} dragging={override?.id === b.path} pos={posOf('board', b.path, b.pos)} ctx={ctx} />
      ))}
    </>
  );
}

const BOARD_ICON = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round">
    <rect x="3.5" y="4" width="17" height="16" rx="3" />
    <path d="M9.2 4v16M14.8 4v16" />
  </svg>
);

function BoardNode({
  b, p, i, current, hl, dragging, pos, ctx,
}: {
  b: BoardInfo;
  p: ProjectInfo;
  i: number;
  current: boolean;
  hl: boolean;
  dragging: boolean;
  pos: Pos;
  ctx: (kind: 'project' | 'board', id: string) => (e: React.MouseEvent) => void;
}) {
  const c = b.counts;
  const f = flowOf(b.statuses);
  const keys = f.list.map(s => s.key);
  const total = keys.reduce((a, k) => a + (c[k] ?? 0), 0);
  const agents = [...new Set(b.holders)];
  return (
    <div
      className={cx('cv-node cv-board', current && 'current', agents.length > 0 && 'active', hl && 'hl', dragging && 'dragging')}
      data-board={b.path}
      style={{ left: pos.x, top: pos.y, ['--pc' as string]: p.color, ['--i' as string]: i }}
      onContextMenu={ctx('board', b.path)}
    >
      <span className="cv-port in" />
      <div className="cvb-head">
        <span className="cvb-ic">{BOARD_ICON}</span>
        <div className="cvb-title">
          <b>{b.name}</b>
          <span className="mono">{b.path}</span>
        </div>
        <button type="button" className="cv-more" aria-label="更多" onClick={ctx('board', b.path)}>
          <Icon name="more" />
        </button>
      </div>
      {b.desc && <div className="cvb-desc">{b.desc}</div>}
      <div className={cx('cvb-bar', !total && 'empty')}>
        {total > 0 &&
          keys.map(k =>
            c[k] ? <i key={k} style={{ flex: c[k], background: f.color(k) }} title={`${f.name(k)} ${c[k]}`} /> : null,
          )}
      </div>
      <div className="cvb-counts">
        {keys.map(k => (
          <span key={k} style={{ ['--c' as string]: f.color(k) }}>
            <i />
            {c[k] ?? 0}
            <em>{f.name(k)}</em>
          </span>
        ))}
      </div>
      <div className="cvb-foot">
        <span className="cvb-agents">
          {agents.length ? (
            <>
              {agents.map(n => (
                <Avatar key={n} actor={agent(n)} />
              ))}
              <em>{agents.length} 个 agent 在工作</em>
            </>
          ) : (
            <em>没有 agent 在工作</em>
          )}
        </span>
        {c.inbox > 0 && (
          <span className="cvb-inbox" title="未整理">
            <Icon name="inbox" />
            {c.inbox}
          </span>
        )}
        <button type="button" className="cvb-open" onClick={() => openBoard(b.path)}>
          打开
          <Icon name="move" />
        </button>
      </div>
    </div>
  );
}
