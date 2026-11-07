/* ==========================================================================
   Board · 项目总览画布（无限画布）
   - 项目节点 ──连线── 白板节点（1:N）；有 agent 在工作的连线会流动
   - 空白处拖动平移；⌘/Ctrl + 滚轮（或触控板捏合）缩放；双指捏合（触摸）
   - 拖动节点调整位置（保存在工作区数据里，所有窗口同步）
   - 双击白板节点或点「打开」进入白板；项目节点右侧 + 新建白板；双击空白处新建项目
   ========================================================================== */
(function (B) {
  'use strict';
  const { $, $$, esc, icons, avatar, agentColor, ago, reduceMotion, bus } = B.ui;
  const VIEW_KEY = 'board.canvas.view';
  const K_MIN = 0.25, K_MAX = 2;

  let store;
  let view = load() || { x: 140, y: 120, k: 0.85 };
  let drag = null;           // 平移 / 节点拖动
  const pointers = new Map(); // 触摸捏合
  let pinch = null;
  let pending = false;
  let tween = 0;

  function load() { try { return JSON.parse(localStorage.getItem(VIEW_KEY)); } catch { return null; } }
  function save() { try { localStorage.setItem(VIEW_KEY, JSON.stringify(view)); } catch {} }

  const canvas = () => $('#canvas');
  const world = () => $('#cvWorld');
  const visible = () => !!canvas() && canvas().offsetParent !== null;

  /* ------------------------------ 视图变换 ------------------------------ */
  function apply() {
    world().style.transform = `translate(${view.x}px, ${view.y}px) scale(${view.k})`;
    const c = canvas();
    c.style.setProperty('--cv-x', `${view.x}px`);
    c.style.setProperty('--cv-y', `${view.y}px`);
    c.style.setProperty('--cv-s', `${24 * view.k}px`);
    $('#cvZoomLabel').textContent = `${Math.round(view.k * 100)}%`;
  }
  const clampK = k => Math.max(K_MIN, Math.min(K_MAX, k));
  function zoomAt(cx, cy, k) {
    k = clampK(k);
    view.x = cx - (cx - view.x) * (k / view.k);
    view.y = cy - (cy - view.y) * (k / view.k);
    view.k = k;
    apply(); save();
  }
  /** 平滑过渡到目标视图 */
  function animateTo(target, ms = 420) {
    cancelAnimationFrame(tween);
    if (reduceMotion()) { Object.assign(view, target); apply(); save(); return; }
    const from = { ...view }, t0 = performance.now();
    const ease = t => 1 - Math.pow(1 - t, 3);
    const step = now => {
      const t = Math.min(1, (now - t0) / ms), e = ease(t);
      view.x = from.x + (target.x - from.x) * e;
      view.y = from.y + (target.y - from.y) * e;
      view.k = from.k + (target.k - from.k) * e;
      apply();
      if (t < 1) tween = requestAnimationFrame(step); else save();
    };
    tween = requestAnimationFrame(step);
  }
  const toWorld = (cx, cy) => ({ x: (cx - view.x) / view.k, y: (cy - view.y) / view.k });
  const local = e => { const r = canvas().getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };

  function bounds(filter) {
    const nodes = $$('#cvNodes .cv-node').filter(filter || (() => true));
    if (!nodes.length) return null;
    let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity;
    nodes.forEach(n => {
      const x = parseFloat(n.style.left), y = parseFloat(n.style.top);
      x1 = Math.min(x1, x); y1 = Math.min(y1, y);
      x2 = Math.max(x2, x + n.offsetWidth); y2 = Math.max(y2, y + n.offsetHeight);
    });
    return { x1, y1, x2, y2 };
  }
  function fit(filter, maxK = 1.1) {
    const b = bounds(filter); if (!b) return;
    const c = canvas(), pad = 90, top = 60;
    const w = c.clientWidth - pad * 2, h = c.clientHeight - pad * 2 - top;
    const k = clampK(Math.min(w / (b.x2 - b.x1), h / (b.y2 - b.y1), maxK));
    animateTo({ k, x: pad + (w - (b.x2 - b.x1) * k) / 2 - b.x1 * k, y: pad + top + (h - (b.y2 - b.y1) * k) / 2 - b.y1 * k });
  }
  function focusNode(sel, k = Math.max(view.k, 0.9)) {
    const n = $(sel); if (!n) return;
    const c = canvas();
    const cx = parseFloat(n.style.left) + n.offsetWidth / 2, cy = parseFloat(n.style.top) + n.offsetHeight / 2;
    animateTo({ k, x: c.clientWidth / 2 - cx * k, y: (c.clientHeight + 40) / 2 - cy * k });
    n.classList.remove('ping'); void n.offsetWidth; n.classList.add('ping');
  }

  /* ------------------------------ 渲染 ------------------------------ */
  const BOARD_ICON = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><rect x="3.5" y="4" width="17" height="16" rx="3"/><path d="M9.2 4v16M14.8 4v16"/></svg>';
  const initial = name => [...String(name || '?').trim()][0]?.toUpperCase() || '?';

  function boardNode(b, i) {
    const c = b.counts, total = c.todo + c.doing + c.review + c.done;
    const seg = k => c[k] ? `<i style="flex:${c[k]};background:var(--c-${k})" title="${B.schema.COLUMN_NAME[k]} ${c[k]}"></i>` : '';
    const agents = [...new Set(b.holders)];
    return `
      <div class="cv-node cv-board ${b.path === store.current ? 'current' : ''} ${agents.length ? 'active' : ''}" data-board="${esc(b.path)}" style="left:${b.pos.x}px;top:${b.pos.y}px;--pc:${b.color};--i:${i}">
        <span class="cv-port in"></span>
        <div class="cvb-head">
          <span class="cvb-ic">${BOARD_ICON}</span>
          <div class="cvb-title"><b>${esc(b.name)}</b><span class="mono">${esc(b.path)}</span></div>
          <button type="button" class="cv-more" data-ctx="board" data-id="${esc(b.path)}" aria-label="更多">${icons.more}</button>
        </div>
        ${b.desc ? `<div class="cvb-desc">${esc(b.desc)}</div>` : ''}
        <div class="cvb-bar ${total ? '' : 'empty'}">${total ? seg('todo') + seg('doing') + seg('review') + seg('done') : ''}</div>
        <div class="cvb-counts">
          ${['todo', 'doing', 'review', 'done'].map(k => `<span style="--c:var(--c-${k})"><i></i>${c[k]}<em>${B.schema.COLUMN_NAME[k]}</em></span>`).join('')}
        </div>
        <div class="cvb-foot">
          <span class="cvb-agents">${agents.length ? agents.map(n => avatar({ kind: 'agent', name: n })).join('') + `<em>${agents.length} 个 agent 在工作</em>` : '<em>没有 agent 在工作</em>'}</span>
          ${c.inbox ? `<span class="cvb-inbox" title="未整理">${icons.inbox}${c.inbox}</span>` : ''}
          <button type="button" class="cvb-open" data-open="${esc(b.path)}">打开${icons.move}</button>
        </div>
      </div>`;
  }
  function projectNode(p, i) {
    const working = new Set(p.holders).size;
    return `
      <div class="cv-node cv-project ${p.key === store.projectKey ? 'current' : ''}" data-project="${esc(p.key)}" style="left:${p.pos.x}px;top:${p.pos.y}px;--pc:${p.color};--i:${i}">
        <span class="pj-av lg" style="--pc:${p.color}">${esc(initial(p.name))}</span>
        <div class="cvp-text">
          <b>${esc(p.name)}</b>
          <span class="mono">${esc(p.key)}</span>
          ${p.desc ? `<span class="cvp-desc">${esc(p.desc)}</span>` : ''}
          <span class="cvp-stats">${p.boards.length} 块白板${working ? ` · <em>${working} 个 agent 在工作</em>` : ''}</span>
        </div>
        <button type="button" class="cv-more" data-ctx="project" data-id="${esc(p.key)}" aria-label="更多">${icons.more}</button>
        <span class="cv-port out"></span>
        <button type="button" class="cv-add" data-add="${esc(p.key)}" data-tip="在「${esc(p.name)}」里新建白板" data-tip-pos="right">${icons.plus}</button>
      </div>`;
  }

  function render() {
    if (!canvas()) return;
    if (drag && drag.kind === 'node') { pending = true; return; }
    const projects = store.projects;
    let i = 0;
    $('#cvNodes').innerHTML = projects.map(p => projectNode(p, i++) + p.boards.map(b => boardNode(b, i++)).join('')).join('');
    const nb = projects.reduce((a, p) => a + p.boards.length, 0);
    $('#cvStat').textContent = `${projects.length} 个项目 · ${nb} 块白板`;
    requestAnimationFrame(drawLinks);
  }

  function anchor(n, side) {
    const x = parseFloat(n.style.left), y = parseFloat(n.style.top);
    return side === 'out' ? { x: x + n.offsetWidth, y: y + n.offsetHeight / 2 } : { x, y: y + n.offsetHeight / 2 };
  }
  function drawLinks() {
    const svg = $('#cvLinks'); if (!svg) return;
    let html = '';
    for (const p of store.projects) {
      const pn = $(`#cvNodes [data-project="${CSS.escape(p.key)}"]`); if (!pn) continue;
      const a = anchor(pn, 'out');
      for (const b of p.boards) {
        const bn = $(`#cvNodes [data-board="${CSS.escape(b.path)}"]`); if (!bn) continue;
        const z = anchor(bn, 'in');
        const dx = Math.max(60, Math.abs(z.x - a.x) * 0.5);
        const d = `M${a.x},${a.y} C${a.x + dx},${a.y} ${z.x - dx},${z.y} ${z.x},${z.y}`;
        const active = b.holders.length > 0;
        html += `<g class="cv-link ${active ? 'active' : ''} ${b.path === store.current ? 'current' : ''}" data-p="${esc(p.key)}" data-b="${esc(b.path)}" style="--pc:${p.color}">
          <path class="halo" d="${d}"/><path class="line" d="${d}" pathLength="1"/>${active ? `<path class="flow" d="${d}"/>` : ''}
          <circle class="dot" cx="${a.x}" cy="${a.y}" r="4"/><circle class="dot" cx="${z.x}" cy="${z.y}" r="4"/>
        </g>`;
      }
    }
    svg.innerHTML = html;
  }

  function playIntro() {
    if (reduceMotion()) return;
    const c = canvas();
    c.classList.remove('intro'); void c.offsetWidth; c.classList.add('intro');
    setTimeout(() => c.classList.remove('intro'), 1600);
  }

  /** 某块白板有新动作：节点闪一下 */
  function ping(path) {
    const n = $(`#cvNodes [data-board="${CSS.escape(path)}"]`);
    if (!n) return;
    n.classList.remove('ping'); void n.offsetWidth; n.classList.add('ping');
  }

  function highlight(path, on) {
    $$('#cvLinks .cv-link').forEach(l => l.classList.toggle('hl', on && (l.dataset.b === path || l.dataset.p === path)));
    $$('#cvNodes .cv-node').forEach(n => n.classList.toggle('hl', on && (n.dataset.board === path || n.dataset.project === path)));
  }

  /* ------------------------------ 交互 ------------------------------ */
  function bind() {
    const c = canvas();

    c.addEventListener('pointerdown', e => {
      if (e.button !== 0 && e.pointerType === 'mouse') return;
      pointers.set(e.pointerId, local(e));
      if (pointers.size === 2) { // 双指捏合
        const [p1, p2] = [...pointers.values()];
        pinch = { d: Math.hypot(p1.x - p2.x, p1.y - p2.y), k: view.k, cx: (p1.x + p2.x) / 2, cy: (p1.y + p2.y) / 2, x: view.x, y: view.y };
        drag = null; return;
      }
      if (e.target.closest('button, .cv-head, .cv-zoom, .cv-hint')) return;
      const node = e.target.closest('.cv-node');
      const l = local(e);
      if (node) {
        drag = { kind: 'node', node, id: node.dataset.board || node.dataset.project, type: node.dataset.board ? 'board' : 'project',
          sx: l.x, sy: l.y, x0: parseFloat(node.style.left), y0: parseFloat(node.style.top), moved: false };
      } else {
        drag = { kind: 'pan', sx: l.x, sy: l.y, x0: view.x, y0: view.y, moved: false };
      }
      drag.pid = e.pointerId;   // 真正开始拖动时才捕获指针，否则 click / dblclick 会被重定向到画布
    });

    c.addEventListener('pointermove', e => {
      if (pointers.has(e.pointerId)) pointers.set(e.pointerId, local(e));
      if (pinch && pointers.size === 2) {
        const [p1, p2] = [...pointers.values()];
        const k = clampK(pinch.k * Math.hypot(p1.x - p2.x, p1.y - p2.y) / pinch.d);
        view.x = pinch.cx - (pinch.cx - pinch.x) * (k / pinch.k);
        view.y = pinch.cy - (pinch.cy - pinch.y) * (k / pinch.k);
        view.k = k; apply(); return;
      }
      if (!drag) return;
      const l = local(e);
      const dx = l.x - drag.sx, dy = l.y - drag.sy;
      if (!drag.moved && Math.hypot(dx, dy) < 4) return;
      if (!drag.moved) { try { c.setPointerCapture(drag.pid); } catch {} if (drag.kind === 'pan') c.classList.add('panning'); }
      drag.moved = true;
      if (drag.kind === 'pan') { view.x = drag.x0 + dx; view.y = drag.y0 + dy; apply(); }
      else {
        drag.node.classList.add('dragging');
        drag.node.style.left = `${drag.x0 + dx / view.k}px`;
        drag.node.style.top = `${drag.y0 + dy / view.k}px`;
        drawLinks();
      }
    });

    const end = e => {
      pointers.delete(e.pointerId);
      if (pointers.size < 2) pinch = null;
      if (!drag) { save(); return; }
      const d = drag; drag = null;
      c.classList.remove('panning');
      if (d.kind === 'pan') { save(); return; }
      d.node.classList.remove('dragging');
      if (d.moved) store.setPos(d.type, d.id, { x: parseFloat(d.node.style.left), y: parseFloat(d.node.style.top) });
      if (pending) { pending = false; render(); }
    };
    c.addEventListener('pointerup', end);
    c.addEventListener('pointercancel', end);

    c.addEventListener('wheel', e => {
      e.preventDefault();
      const l = local(e);
      if (e.ctrlKey || e.metaKey) zoomAt(l.x, l.y, view.k * Math.exp(-e.deltaY * 0.0022));
      else { view.x -= e.deltaX; view.y -= e.deltaY; apply(); clearTimeout(c._t); c._t = setTimeout(save, 200); }
    }, { passive: false });

    c.addEventListener('dblclick', e => {
      if (e.target.closest('button, .cv-head, .cv-zoom')) return;
      const bnode = e.target.closest('.cv-board');
      if (bnode) return open(bnode.dataset.board);
      if (e.target.closest('.cv-node')) return;
      const l = local(e), w = toWorld(l.x, l.y);
      B.projects.newProject({ pos: { x: Math.round(w.x - 130), y: Math.round(w.y - 40) } });
    });

    c.addEventListener('click', e => {
      const o = e.target.closest('[data-open]'); if (o) return open(o.dataset.open);
      const a = e.target.closest('[data-add]');
      if (a) {
        const pn = a.closest('.cv-project');
        const p = store.projectMeta(a.dataset.add);
        const ys = p.boards.map(b => b.pos.y);
        const pos = { x: parseFloat(pn.style.left) + 420, y: ys.length ? Math.max(...ys) + 200 : parseFloat(pn.style.top) - 10 };
        return B.projects.newBoard(a.dataset.add, { pos });
      }
      const m = e.target.closest('[data-ctx]');
      if (m) return B.projects.contextMenu(m.dataset.ctx, m.dataset.id, m);
    });
    c.addEventListener('contextmenu', e => {
      const n = e.target.closest('.cv-node'); if (!n) return;
      e.preventDefault();
      B.projects.contextMenu(n.dataset.board ? 'board' : 'project', n.dataset.board || n.dataset.project, { x: e.clientX, y: e.clientY });
    });
    c.addEventListener('pointerover', e => { const n = e.target.closest('.cv-node'); if (n && !drag) highlight(n.dataset.board || n.dataset.project, true); });
    c.addEventListener('pointerout', e => { const n = e.target.closest('.cv-node'); if (n && !n.contains(e.relatedTarget)) highlight(null, false); });

    $('.cv-zoom').addEventListener('click', e => {
      const b = e.target.closest('[data-z]'); if (!b) return;
      const cx = c.clientWidth / 2, cy = c.clientHeight / 2;
      const z = b.dataset.z;
      if (z === 'in') animateTo(zoomTarget(cx, cy, view.k * 1.25), 220);
      if (z === 'out') animateTo(zoomTarget(cx, cy, view.k / 1.25), 220);
      if (z === 'reset') animateTo(zoomTarget(cx, cy, 1), 260);
      if (z === 'fit') fit();
      if (z === 'current') focusNode(`#cvNodes [data-board="${CSS.escape(store.current)}"]`, 1);
    });
    c.addEventListener('keydown', e => {
      if (e.target !== c) return;
      const step = 60;
      if (e.key === 'ArrowLeft') view.x += step; else if (e.key === 'ArrowRight') view.x -= step;
      else if (e.key === 'ArrowUp') view.y += step; else if (e.key === 'ArrowDown') view.y -= step;
      else if (e.key === '=' || e.key === '+') return animateTo(zoomTarget(c.clientWidth / 2, c.clientHeight / 2, view.k * 1.25), 200);
      else if (e.key === '-') return animateTo(zoomTarget(c.clientWidth / 2, c.clientHeight / 2, view.k / 1.25), 200);
      else if (e.key === '0') return fit();
      else return;
      e.preventDefault(); apply(); save();
    });

    $('#cvNewProject').onclick = () => B.projects.newProject();
    $('#cvNewBoard').onclick = () => B.projects.newBoard(store.projectKey);
  }
  function zoomTarget(cx, cy, k) {
    k = clampK(k);
    return { k, x: cx - (cx - view.x) * (k / view.k), y: cy - (cy - view.y) * (k / view.k) };
  }

  function open(path) {
    store.setBoard(path);
    B.workbench.setTab('board');
  }

  B.canvas = {
    init(s) {
      store = s;
      bind();
      apply();
      render();
      store.subscribe((_, meta) => {
        if (meta.source === 'layout' && drag) return;
        render();
        (meta.events || []).forEach(() => ping(store.current));
        (meta.others || []).forEach(e => ping(e.board));
      });
      bus.on('tab:changed', t => {
        if (t !== 'projects') return;
        render();
        requestAnimationFrame(() => { drawLinks(); playIntro(); if (!load()) fit(); });
      });
      window.addEventListener('resize', () => visible() && drawLinks());
    },
    render, fit, highlight,
    focusBoard(path) { focusNode(`#cvNodes [data-board="${CSS.escape(path)}"]`); },
    focusProject(key) { focusNode(`#cvNodes [data-project="${CSS.escape(key)}"]`); },
  };
})(window.Board = window.Board || {});
