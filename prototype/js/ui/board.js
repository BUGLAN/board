/* ==========================================================================
   Board · 白板视图
   - 按 ID 复用卡片 DOM，增量更新；位置变化用 FLIP 动画补间
   - 「进行中」卡片的倒计时由 tick() 独立刷新，不触发整体渲染
   - 来自 CLI / 其他窗口 / 系统 的变更：卡片闪光 + 角标说明是谁做的
   ========================================================================== */
(function (B) {
  'use strict';
  const { schema } = B;
  const { COLUMNS, COLUMN_KEYS, ALL_KEYS, CLAIM_MS, HUMAN } = schema;
  const { $, $$, el, esc, icons, agentColor, agentGlyph, avatar, mmss, ago, ring, setRing, reduceMotion, isMobile, autosize, bus, toastError } = B.ui;

  const EMPTY = {
    todo: '待办是空的<br>新建一张卡片，或等 issue 监听自动创建',
    doing: '没有 agent 在工作<br>agent 执行 <code class="mono">board claim</code> 后会出现在这里',
    review: '没有等待审核的卡片',
    done: '还没有完成的卡片<br>把审核通过的卡片拖到这里',
  };
  const EMPTY_ICON = { todo: icons.inbox, doing: icons.claim, review: icons.checkCircle, done: icons.sparkle };
  const OP_LABEL = { create: '新建', edit: '编辑', move: '移动', reorder: '排序', claim: '领取', renew: '续期', comment: '评论', timeout: '超时释放', release: '强制释放' };
  const SPRING = 'cubic-bezier(.22, 1.18, .36, 1)';

  let store;
  const els = new Map();          // id → card element
  let counts = {};
  let locked = false, pending = false;
  let openId = null;
  let filter = '';

  /* ------------------------------ 结构 ------------------------------ */
  function build() {
    const board = $('#board');
    board.innerHTML = COLUMNS.map((c, i) => `
      <section class="col enter" data-col="${c.key}" data-drop="${c.key}" style="--c:var(--c-${c.key});--i:${i + 1}" aria-label="${c.name}">
        <header class="col-head">
          <span class="col-dot"></span>
          <h2 class="col-name">${c.name}</h2>
          <span class="col-key mono">${c.key}</span>
          <span class="col-count" aria-label="卡片数量"><span>0</span></span>
          <span class="col-info" tabindex="0" data-tip="${esc(c.desc)}">${icons.info}</span>
        </header>
        <div class="col-body scroll">
          <div class="col-list" data-list="${c.key}"></div>
          <div class="col-empty">${EMPTY_ICON[c.key]}<div>${EMPTY[c.key]}</div></div>
        </div>
        ${c.key === 'todo' ? composerMarkup() : ''}
        <div class="col-veto"><div class="veto-icon">${icons.lock}</div><b></b><span></span></div>
      </section>`).join('');

    // 移动端列切换条
    const tabs = $('#colTabs');
    tabs.innerHTML = COLUMNS.map(c => `<button class="ct" type="button" data-col="${c.key}" style="--c:var(--c-${c.key})"><i></i>${c.name}<span class="n">0</span></button>`).join('') + '<span class="ct-ind"></span>';
    tabs.addEventListener('click', e => {
      const b = e.target.closest('.ct'); if (!b) return;
      scrollToColumn(b.dataset.col);
    });
    board.addEventListener('scroll', syncTabs, { passive: true });
    window.addEventListener('resize', syncTabs);

    bindComposer();

    // 卡片：点击打开、键盘、指针高光（白板与侧栏「未整理」共用，挂在 document 上）
    const cardOf = e => e.target.closest('[data-drop] .card');
    document.addEventListener('click', e => {
      const act = e.target.closest('[data-act="triage"]');
      if (act) { e.stopPropagation(); triage(Number(act.closest('.card').dataset.id)); return; }
      const card = cardOf(e);
      if (!card || B.drag?.justDropped()) return;
      bus.emit('card:open', Number(card.dataset.id));
    });
    document.addEventListener('keydown', e => {
      const card = cardOf(e);
      if (!card) return;
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); bus.emit('card:open', Number(card.dataset.id)); }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const sib = e.key === 'ArrowDown' ? card.nextElementSibling : card.previousElementSibling;
        sib?.focus();
      }
      if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
        e.preventDefault();
        const ci = COLUMN_KEYS.indexOf(card.closest('[data-drop]').dataset.drop);
        for (let k = ci + (e.key === 'ArrowRight' ? 1 : -1); k >= 0 && k < 4; k += e.key === 'ArrowRight' ? 1 : -1) {
          const first = listEl(COLUMN_KEYS[k]).firstElementChild;
          if (first) { first.focus(); break; }
        }
      }
    });
    document.addEventListener('pointermove', e => {
      const card = cardOf(e);
      if (!card || e.pointerType !== 'mouse') return;
      const r = card.getBoundingClientRect();
      card.style.setProperty('--mx', `${e.clientX - r.left}px`);
      card.style.setProperty('--my', `${e.clientY - r.top}px`);
    });
  }

  function composerMarkup() {
    return `
      <div class="composer" id="composer">
        <button class="composer-open new-entry" type="button"><span class="ne-plus">${icons.plus}</span><span class="ne-text"><b>新建卡片</b></span><kbd>N</kbd></button>
      </div>`;
  }

  function bindComposer() {
    // 新建统一在居中弹窗里完成（js/ui/creator.js）
    $('#composer .composer-open').onclick = () => B.creator.open({ column: 'todo' });
  }

  /* ------------------------------ 卡片 ------------------------------ */
  const titleHtml = t => esc(t).replace(/`([^`]+)`/g, '<code>$1</code>');

  function signature(c) {
    const comments = c.activity.filter(a => a.kind === 'comment').length;
    return [c.title, c.column, c.holder, comments, c.claimedAt, c.column === 'inbox' ? c.body : '', B.labels.chips(c.labels)].join('\u0001');
  }

  function cardInner(c) {
    const comments = c.activity.filter(a => a.kind === 'comment').length;
    const gh = (c.body.match(/gh#\d+/) || [])[0];
    const flags = [
      gh ? `<span class="card-tag">${gh}</span>` : '',
      comments ? `<span class="f" title="${comments} 条评论">${icons.comment}${comments}</span>` : '',
    ].join('');
    let claim = '';
    if (c.column === 'doing' && c.holder) {
      claim = `
        <div class="card-claim">
          ${avatar({ kind: 'agent', name: c.holder })}
          <span class="claim-name">${esc(c.holder)}</span>
          <span class="claim-timer" title="剩余领取时间">${ring(5)}<span class="t">--:--</span></span>
        </div>
        <div class="card-bar"><i></i></div>`;
    }
    if (c.column === 'inbox') {
      const snippet = c.body.split('\n').map(l => l.replace(/^[-*#>\s]+|\[[ xX]\]\s*/g, '').trim()).find(Boolean) || '';
      return `
      <div class="card-row">
        <span class="card-id">#${c.id}</span>
        <span class="card-flags"><span class="card-age" data-at="${c.createdAt}">${ago(c.createdAt, store.clock.now())}</span></span>
      </div>
      <div class="card-title">${titleHtml(c.title)}</div>
      ${snippet ? `<div class="card-snippet">${esc(snippet)}</div>` : ''}
      ${labelsRow(c)}
      <button class="card-act" type="button" data-act="triage" title="整理到「待办」最下面">整理到待办${icons.move}</button>`;
    }
    return `
      <div class="card-row">
        <span class="card-id">#${c.id}</span>
        ${c.column === 'done' ? `<span class="done-check">${icons.check}</span>` : ''}
        <span class="card-flags">${flags}</span>
      </div>
      <div class="card-title">${titleHtml(c.title)}</div>
      ${labelsRow(c)}
      ${claim}`;
  }

  function labelsRow(c) {
    const h = B.labels.chips(c.labels);
    return h ? `<div class="card-labels">${h}</div>` : '';
  }

  function createCard(c) {
    const node = el(`<article class="card" tabindex="0" data-id="${c.id}"></article>`);
    els.set(c.id, node);
    return node;
  }

  function updateCard(node, c) {
    const sig = signature(c);
    if (node.dataset.sig !== sig) {
      node.innerHTML = cardInner(c);
      node.dataset.sig = sig;
    }
    node.className = node.className.replace(/\b(inbox|todo|doing|review|done)\b/g, '').trim();
    node.classList.add(c.column);
    node.classList.toggle('is-open', c.id === openId);
    node.setAttribute('aria-label', `#${c.id} ${c.title}`);
    applyFilter(node, c);
    if (c.column === 'doing' && c.holder) node.style.setProperty('--ac', agentColor(c.holder));
    else node.style.removeProperty('--ac');
  }

  /* ------------------------------ 渲染 ------------------------------ */
  function render(meta = {}) {
    if (locked) { pending = true; return; }
    if (meta.foreign) return;                       // 其他项目的变更，与当前白板无关
    const s = store.state;
    // 切换项目：卡片 ID 在各项目里各自从 1 开始，必须整块重建
    const fresh = meta.source === 'init' || meta.source === 'switch';
    if (meta.source === 'switch') { els.forEach(n => n.remove()); els.clear(); counts = {}; openId = null; }
    const animate = !reduceMotion() && !fresh;

    // First
    const first = new Map();
    if (animate) els.forEach((node, id) => first.set(id, { rect: node.getBoundingClientRect(), col: node.parentElement?.dataset.list }));

    // 更新 / 插入 / 排序
    const seen = new Set();
    for (const col of ALL_KEYS) {
      const list = listEl(col);
      if (!list) continue;
      (s.cols[col] || []).forEach((id, idx) => {
        const c = s.cards[id]; if (!c) return;
        seen.add(id);
        let node = els.get(id);
        const isNew = !node;
        if (!node) node = createCard(c);
        updateCard(node, c);
        if (list.children[idx] !== node) list.insertBefore(node, list.children[idx] || null);
        if (isNew && !fresh) {
          node.classList.add('entering');
          node.addEventListener('animationend', () => node.classList.remove('entering'), { once: true });
        } else if (isNew) {
          node.style.animation = `card-in var(--dur-4) var(--spring-soft) both ${180 + ALL_KEYS.indexOf(col) * 70 + idx * 45}ms`;
          node.addEventListener('animationend', () => (node.style.animation = ''), { once: true });
        }
      });
      // 清理多余节点（理论上不会有）
      [...list.children].forEach(n => { if (!seen.has(Number(n.dataset.id))) n.remove(); });
    }

    // 删除：在原位留一个淡出的影子
    els.forEach((node, id) => {
      if (seen.has(id)) return;
      els.delete(id);
      const r = first.get(id)?.rect || node.getBoundingClientRect();
      node.remove();
      if (animate && r.width) exitGhost(node, r);
    });

    // Last + Invert + Play
    if (animate) {
      els.forEach((node, id) => {
        const f = first.get(id); if (!f || !f.rect.width) return;
        const r = node.getBoundingClientRect();
        if (!r.width) return;
        const dx = f.rect.left - r.left, dy = f.rect.top - r.top;
        if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
        const cross = f.col !== col0(node);
        node.animate(
          cross
            ? [{ transform: `translate(${dx}px, ${dy}px) scale(1.03) rotate(${dx > 0 ? 1.5 : -1.5}deg)`, zIndex: 5 }, { transform: 'none', zIndex: 5 }]
            : [{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }],
          { duration: cross ? 640 : 420, easing: SPRING }
        );
      });
    }

    updateCounts(s, !fresh);
    flashForEvents(meta);
    tick(store.clock.now());
    bus.emit('board:rendered', s);
  }
  const col0 = node => node.parentElement?.dataset.list;

  function exitGhost(node, r) {
    const g = node.cloneNode(true);
    Object.assign(g.style, { position: 'fixed', left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`, margin: 0, zIndex: 60, pointerEvents: 'none' });
    document.body.appendChild(g);
    g.animate(
      [{ opacity: 1, transform: 'none', filter: 'blur(0)' }, { opacity: 0, transform: 'scale(.86) translateY(8px)', filter: 'blur(6px)' }],
      { duration: 420, easing: 'cubic-bezier(.4,0,.2,1)', fill: 'forwards' }
    ).onfinish = () => g.remove();
  }

  function updateCounts(s, animate) {
    for (const col of ALL_KEYS) {
      const n = (s.cols[col] || []).length;
      const colEl = columnEl(col);
      if (!colEl) continue;
      const badge = colEl.querySelector('.col-count');
      if (counts[col] !== n) {
        if (badge && animate && counts[col] != null) {
          badge.style.setProperty('--roll', n > counts[col] ? '100%' : '-100%');
          badge.classList.remove('bump'); void badge.offsetWidth; badge.classList.add('bump');
        }
        if (badge) badge.firstElementChild.textContent = n;
        if (counts[col] != null && animate) bus.emit('count:changed', { col, n, prev: counts[col] });
        counts[col] = n;
      }
      colEl.classList.toggle('is-empty', n === 0);
      const tab = $(`.ct[data-col="${col}"] .n`); if (tab) tab.textContent = n;
    }
    syncTabs();
  }

  function flashForEvents(meta) {
    if (!meta.events || !meta.events.length) return;
    const remote = meta.source === 'remote';
    for (const ev of meta.events) {
      if (!remote && ev.via === 'ui') continue;          // 自己在界面上的操作不需要提示
      const node = els.get(ev.cardId); if (!node) continue;
      const color = ev.actor.kind === 'agent' ? agentColor(ev.actor.name) : ev.actor.kind === 'system' ? 'var(--danger)' : 'var(--brand)';
      node.style.setProperty('--flash', color);
      node.classList.remove('flash'); void node.offsetWidth; node.classList.add('flash');
      node.querySelector('.card-badge')?.remove();
      const who = ev.actor.kind === 'agent' ? `${agentGlyph(ev.actor.name)} ${ev.actor.name}` : ev.actor.kind === 'system' ? '⏱ 系统' : remote ? '另一个窗口' : '你';
      const via = ev.via === 'cli' ? ' · CLI' : '';
      const badge = el(`<span class="card-badge">${esc(who)}${via} · ${OP_LABEL[ev.op] || ev.op}</span>`);
      node.appendChild(badge);
      badge.addEventListener('animationend', () => badge.remove(), { once: true });
    }
  }

  /* ------------------------------ 倒计时 ------------------------------ */
  function tick(now) {
    const s = store.state;
    for (const id of s.cols.doing) {
      const c = s.cards[id]; const node = els.get(id);
      if (!c || !node || !c.expiresAt) continue;
      const left = c.expiresAt - now;
      const p = left / CLAIM_MS;
      const t = node.querySelector('.claim-timer .t');
      if (t) t.textContent = mmss(left);
      setRing(node.querySelector('.claim-timer svg'), p);
      const bar = node.querySelector('.card-bar i'); if (bar) bar.style.setProperty('--p', Math.max(0, Math.min(1, p)).toFixed(4));
      node.classList.toggle('urgent', left < 5 * 60 * 1000);
    }
  }

  /* ------------------------------ 移动端列切换 ------------------------------ */
  function scrollToColumn(col) {
    const c = columnEl(col); const board = $('#board');
    board.scrollTo({ left: c.offsetLeft - (board.clientWidth - c.clientWidth) / 2, behavior: reduceMotion() ? 'auto' : 'smooth' });
  }
  let tabRaf = 0;
  function syncTabs() {
    cancelAnimationFrame(tabRaf);
    tabRaf = requestAnimationFrame(() => {
      if (!isMobile()) return;
      const board = $('#board');
      const center = board.scrollLeft + board.clientWidth / 2;
      let best = 'todo', bd = Infinity;
      for (const col of COLUMN_KEYS) {
        const c = columnEl(col); const mid = c.offsetLeft + c.clientWidth / 2;
        if (Math.abs(mid - center) < bd) { bd = Math.abs(mid - center); best = col; }
      }
      $$('.ct').forEach(t => t.classList.toggle('on', t.dataset.col === best));
      const on = $(`.ct[data-col="${best}"]`), ind = $('.ct-ind');
      if (on && ind) { ind.style.width = `${on.offsetWidth}px`; ind.style.transform = `translateX(${on.offsetLeft}px)`; }
    });
  }

  /* ------------------------------ 对外 ------------------------------ */
  const listEl = col => $(`[data-list="${col}"]`);
  const columnEl = col => $(`[data-drop="${col}"]`);

  /** 未整理 → 待办最下面 */
  function triage(id) {
    const r = store.try({ op: 'move', actor: HUMAN, id, column: 'todo' });
    if (!r.ok) return toastError(r.error);
    if (isMobile()) B.ui.toast({ type: 'success', title: `#${id} 已整理到「待办」`, duration: 1800 });
    else requestAnimationFrame(() => { const n = els.get(id); n?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); });
  }

  /* ------------------------------ 筛选 ------------------------------ */
  /** 筛选：空格分隔的词都要命中；label:名称 只看标签 */
  function matches(c, q) {
    if (!q) return true;
    const map = B.labels.byId();
    const names = (c.labels || []).map(id => (map[id]?.name || '').toLowerCase());
    const hay = `#${c.id}\n${c.title}\n${c.body}\n${c.holder || ''}\n${names.join('\n')}`.toLowerCase();
    return q.split(/\s+/).every(w => {
      const m = w.match(/^(?:label|标签)[:：](.+)$/);
      return m ? names.includes(m[1]) : hay.includes(w);
    });
  }
  function applyFilter(node, c) {
    const hit = matches(c, filter);
    node.classList.toggle('filtered-out', !!filter && !hit);
    node.classList.toggle('matched', !!filter && hit);
  }
  function setFilter(q) {
    filter = String(q || '').trim().toLowerCase();
    let n = 0;
    els.forEach((node, id) => { const c = store.state.cards[id]; if (!c) return; applyFilter(node, c); if (filter && matches(c, filter) && c.column !== 'inbox') n++; });
    return n;
  }

  B.boardView = {
    init(s) { store = s; build(); render({ source: 'init' }); },
    render,
    tick,
    listEl, columnEl, scrollToColumn, triage, setFilter, matches: c => matches(c, filter),
    get filter() { return filter; },
    reveal(id) {
      const n = els.get(Number(id)); if (!n) return;
      const col = store.state.cards[id]?.column;
      if (isMobile() && col && col !== 'inbox') scrollToColumn(col);
      n.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: reduceMotion() ? 'auto' : 'smooth' });
      n.classList.remove('pulse'); void n.offsetWidth; n.classList.add('pulse');
    },
    cardEl: id => els.get(Number(id)),
    lock() { locked = true; },
    unlock() { locked = false; if (pending) { pending = false; render({}); } },
    setOpen(id) {
      openId = id;
      els.forEach((n, k) => n.classList.toggle('is-open', k === id));
    },
    rejectColumn(col) {
      const c = columnEl(col); if (!c) return;
      c.classList.remove('reject'); void c.offsetWidth; c.classList.add('reject');
      setTimeout(() => c.classList.remove('reject'), 500);
    },
  };
})(window.Board = window.Board || {});
