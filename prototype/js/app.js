/* ==========================================================================
   Board · 启动
   组装 Store / Clock / Transport 与各个视图；全局快捷键；状态栏与在线 agent。
   ========================================================================== */
(function (B) {
  'use strict';
  const { $, $$, el, esc, icons, avatar, agentColor, agentGlyph, hhmm, mmss, ring, setRing, bus, toast } = B.ui;
  const { CLAIM_MS, COLUMN_NAME } = B.schema;

  const clock = new B.Clock();
  const store = new B.Store({ transport: new B.LocalTransport(), clock });

  /* ------------------------------ 视图 ------------------------------ */
  // 标签与提示词先初始化：白板渲染卡片时就要用到
  B.labels.init(store);
  B.prompt.init(store);
  B.boardView.init(store);
  B.drag.init(store);
  B.drawer.init(store);
  B.creator.init(store);
  B.projects.init(store);
  B.workbench.init(store);
  B.canvas.init(store);
  B.palette.init(store);
  // 演示专用模块：正式产品不加载 js/sim/*，这里自动跳过
  B.console?.init(store);
  B.sim?.init(store);
  if (!B.console) document.querySelectorAll('.demo-group, .console').forEach(n => n.remove());

  store.subscribe((state, meta) => {
    B.boardView.render(meta);
    onChange(state, meta);
  });

  /* ------------------------------ 实时指示 / 状态栏 ------------------------------ */
  function onChange(state, meta) {
    if (meta.foreign) return;
    const live = $('#live');
    $('#liveRev').textContent = `rev ${state.rev}`;
    live.classList.remove('bump'); void live.offsetWidth; live.classList.add('bump');

    const sync = $('#sbSync');
    sync.classList.add('syncing');
    sync.lastElementChild.textContent = meta.source === 'remote' ? '收到其他窗口的变更' : '同步中…';
    clearTimeout(sync._t);
    sync._t = setTimeout(() => { sync.classList.remove('syncing'); sync.lastElementChild.textContent = '已同步'; }, 600);

    updateCounts(state);
    renderPresence(state);
    (meta.events || []).forEach(ticker);

    // 系统超时释放：额外弹出提示，便于观察规则 7（其他项目的也提示，并可一键切过去）
    (meta.events || []).filter(e => e.op === 'timeout').forEach(e => {
      toast({ type: 'info', title: `#${e.cardId} 领取超时，已自动释放`, text: `${e.text.replace(/^.*（原持有者 (.+)）$/, '$1')} 没有在 30 分钟内续期，卡片回到「待办」最上面。` });
    });
    (meta.others || []).filter(e => e.op === 'timeout').forEach(e => {
      const b = store.boardMetaOf(e.board); if (!b) return;
      toast({ type: 'info', title: `「${b.projectName} / ${b.name}」#${e.cardId} 领取超时`, text: '卡片已回到那块白板「待办」最上面。', action: { label: '打开', fn: () => B.projects.openBoard(e.board) } });
    });
  }

  function updateCounts(state) {
    const n = k => (state.cols[k] || []).length;
    $('#sbCounts').textContent = `未整理 ${n('inbox')} · 待办 ${n('todo')} · 进行中 ${n('doing')} · 待审核 ${n('review')} · 已完成 ${n('done')}`;
  }

  const OP = { create: '新建了', edit: '编辑了', move: '移动了', reorder: '调整了顺序', claim: '领取了', renew: '续期了', comment: '评论了', timeout: '超时释放了', release: '强制释放了', delete: '删除了' };
  let tickerQueue = [], tickerBusy = false;
  function ticker(ev) {
    tickerQueue.push(ev);
    if (tickerQueue.length > 4) tickerQueue.shift();
    if (!tickerBusy) nextTick();
  }
  function nextTick() {
    const ev = tickerQueue.shift();
    if (!ev) { tickerBusy = false; return; }
    tickerBusy = true;
    const wrap = $('#ticker');
    const old = wrap.querySelector('.tick:not(.out)');
    if (old) { old.classList.add('out'); old.addEventListener('animationend', () => old.remove(), { once: true }); }
    const who = ev.actor.kind === 'human' ? '你' : ev.actor.kind === 'system' ? '系统' : ev.actor.name;
    const node = el(`<span class="tick">
      <span class="mono">${hhmm(ev.at)}</span>
      <b style="${ev.actor.kind === 'agent' ? `color:${agentColor(ev.actor.name)}` : ''}">${esc(who)}</b>
      ${OP[ev.op] || ev.op} <span class="mono">#${ev.cardId}</span> ${esc(ev.title || '')}
      ${ev.via === 'cli' ? '<span class="via">CLI</span>' : ''}
    </span>`);
    wrap.appendChild(node);
    setTimeout(nextTick, 900);
  }

  /* ------------------------------ 在线 agent（持有卡片的） ------------------------------ */
  function renderPresence(state) {
    const wrap = $('#presence');
    const holders = B.engine.query.holders(state);
    const want = new Set(holders.map(h => h.name));
    [...wrap.children].forEach(n => { if (!want.has(n.dataset.name)) { n.animate([{ transform: 'none', opacity: 1 }, { transform: 'scale(.3)', opacity: 0 }], { duration: 220, easing: 'ease-in' }).onfinish = () => n.remove(); n.dataset.name = ''; } });
    holders.forEach(({ name, card }) => {
      let n = wrap.querySelector(`[data-name="${CSS.escape(name)}"]`);
      if (!n) {
        n = el(`<span class="p-item" data-name="${esc(name)}" style="--c:${agentColor(name)}">${avatar({ kind: 'agent', name })}${ring(15, 'ring')}</span>`);
        n.addEventListener('click', () => { const c = B.engine.query.heldBy(store.state, name); if (c) bus.emit('card:open', c.id); });
        wrap.appendChild(n);
      }
      n.dataset.tip = `${name} 正在做 #${card.id}`;
      n.dataset.card = card.id;
    });
  }
  function tickPresence(now) {
    $$('#presence .p-item').forEach(n => {
      const c = store.state.cards[n.dataset.card]; if (!c || !c.expiresAt) return;
      const left = c.expiresAt - now;
      setRing(n.querySelector('.ring'), left / CLAIM_MS);
      n.classList.toggle('urgent', left < 5 * 60e3);
      n.dataset.tip = `${c.holder} 正在做 #${c.id} · 剩余 ${mmss(left)}`;
    });
  }

  /* ------------------------------ 演示时钟 ------------------------------ */
  function renderSpeed() {
    const seg = $('#speed .seg');
    if (!seg) return;
    seg.querySelectorAll('button').forEach(b => b.classList.toggle('on', Number(b.dataset.speed) === clock.speed));
    const on = seg.querySelector('button.on'), thumb = seg.querySelector('.seg-thumb');
    if (on) { thumb.style.width = `${on.offsetWidth}px`; thumb.style.transform = `translateX(${on.offsetLeft}px)`; }
    $('#speed').classList.toggle('fast', clock.speed > 1);
  }
  $('#speed .seg')?.addEventListener('click', e => {
    const b = e.target.closest('button[data-speed]'); if (!b) return;
    clock.setSpeed(Number(b.dataset.speed));
  });
  clock.onChange(() => {
    renderSpeed();
    if (clock.speed > 1) toast({ type: 'info', title: `演示时钟 ×${clock.speed}`, text: clock.speed === 60 ? '1 秒 ≈ 1 分钟：30 分钟的领取超时大约 30 秒后发生。' : '时间流速加快 10 倍。', duration: 2600 });
  });

  /* ------------------------------ 主循环 ------------------------------ */
  let lastSweep = 0;
  function loop() {
    const now = clock.now();
    B.boardView.tick(now);
    B.drawer.tick(now);
    B.workbench.tick(now);
    tickPresence(now);
    const ct = $('#clockTime'); if (ct) ct.textContent = hhmm(now);
    // 规则 7：超时释放。正式产品由服务端执行；原型里由打开的页面执行
    if (performance.now() - lastSweep > 500) { lastSweep = performance.now(); store.sweep(); }
  }
  setInterval(loop, 250);

  /* ------------------------------ 主题 / 外壳 ------------------------------ */
  function cycleTheme() {
    const p = B.platform.cycleTheme();
    toast({ type: 'info', title: { auto: '主题：跟随系统', light: '主题：浅色', dark: '主题：深色' }[p], duration: 1400 });
  }
  $('#shellToggle')?.addEventListener('click', () => { B.platform.toggleShell(); setTimeout(renderSpeed, 50); });

  /* ------------------------------ 快捷键 ------------------------------ */
  document.addEventListener('keydown', e => {
    const typing = e.target.closest('input, textarea, [contenteditable]');
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === 'k') { e.preventDefault(); B.palette.isOpen() ? B.palette.close() : B.palette.open(); return; }
    if (mod && e.key.toLowerCase() === 'b') { e.preventDefault(); B.workbench.toggleSide(); return; }
    if (mod && ['1', '2', '3', '4'].includes(e.key)) { e.preventDefault(); B.workbench.setTab(['board', 'projects', 'inbox', 'settings'][Number(e.key) - 1]); return; }
    if (e.key === '`' && !typing && B.console) { e.preventDefault(); B.console.toggle(); return; }
    if (e.key === 'Escape') {
      if (B.palette.isOpen()) return B.palette.close();
      if (B.drawer.openId != null) return B.drawer.close();
      if (B.console?.isOpen() && !typing) return B.console.close();
    }
    if (typing || mod || e.altKey) return;
    if (B.creator.isOpen()) return;
    if (e.key === 'n' || e.key === 'N') { e.preventDefault(); B.creator.open({ column: 'todo' }); }
    if (e.key === 'i' || e.key === 'I') { e.preventDefault(); B.creator.open({ column: 'inbox' }); }
    if (e.key === 'p' || e.key === 'P') { e.preventDefault(); B.projects.openMenu(); }
    if (e.key === '/') { e.preventDefault(); B.palette.open(); }
    if (e.key === '>') { e.preventDefault(); B.palette.open('> '); }
  });

  /* ------------------------------ 启动 ------------------------------ */
  onChange(store.state, { events: [] });
  renderSpeed();
  window.addEventListener('resize', renderSpeed);
  document.fonts?.ready.then(renderSpeed);

  B.app = { store, clock, cycleTheme };
  window.board = B; // 方便在 DevTools 里调试：board.app.store.state
})(window.Board = window.Board || {});
