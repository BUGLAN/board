/* ==========================================================================
   Board · 操作台（类 VS Code）
   - 活动栏 Tab：board（白板 · 当前页面）/ projects（项目总览）/ inbox（未整理）/ settings（设置）
     · 再次点击当前 Tab：收起 / 展开侧栏
   - 侧栏视图：白板概览（筛选 + 列树 + 正在工作 + 最近动态）/ 未整理列表 / 设置目录
   - 主内容区：白板页 / 设置页
   扩展：新增 Tab 时，在 TABS 里登记，并在 index.html 加对应的 .ab-item 与 .side-view
   ========================================================================== */
(function (B) {
  'use strict';
  const { COLUMNS, COLUMN_NAME, CLAIM_MS, HUMAN } = B.schema;
  const { $, $$, el, esc, icons, avatar, agentColor, ago, mmss, markdown, toast, isMobile, reduceMotion, bus, autosize } = B.ui;

  const TABS = { board: { page: 'board' }, projects: { page: 'canvas' }, inbox: { page: 'board' }, settings: { page: 'settings' } };
  const SIDE_W = { min: 220, max: 460, def: 292 };

  let store;
  const app = () => $('#app');
  const pref = (k, d) => { try { return localStorage.getItem('board.' + k) ?? d; } catch { return d; } };
  const setPref = (k, v) => { try { localStorage.setItem('board.' + k, v); } catch {} };

  /* ============================ Tab 切换 ============================ */
  function setTab(tab, { toggle = false } = {}) {
    if (!TABS[tab]) tab = 'board';
    const a = app();
    const prev = a.dataset.tab;
    if (toggle && prev === tab && !isMobile()) {
      setSide(a.dataset.side === 'closed');
      return;
    }
    a.dataset.tab = tab;
    if (a.dataset.side === 'closed' && toggle) setSide(true);
    $$('.ab-item').forEach(b => { b.classList.toggle('on', b.dataset.tab === tab); b.setAttribute('aria-current', b.dataset.tab === tab ? 'page' : 'false'); });
    moveIndicator();

    // 主内容区
    const page = TABS[tab].page;
    $$('.main .page').forEach(p => {
      const show = p.dataset.page === page;
      if (show && p.hidden) { p.hidden = false; p.classList.remove('page-in'); void p.offsetWidth; p.classList.add('page-in'); }
      else if (!show) p.hidden = true;
    });
    setPref('tab', tab);
    bus.emit('tab:changed', tab);
  }

  function moveIndicator() {
    const on = $('.ab-item.on'); const ind = $('.ab-indicator');
    if (!on || !ind) return;
    ind.style.setProperty('--ab-y', `${on.offsetTop + (on.offsetHeight - 24) / 2}px`);
  }

  function setSide(open) {
    app().dataset.side = open ? 'open' : 'closed';
    setPref('side', open ? 'open' : 'closed');
    setTimeout(() => window.dispatchEvent(new Event('resize')), 380);
  }

  function bindSideResize() {
    const handle = $('#sideResize'), side = $('#side');
    const apply = w => document.documentElement.style.setProperty('--side-w', `${w}px`);
    apply(Number(pref('sideW', SIDE_W.def)));
    let x0, w0;
    handle.addEventListener('pointerdown', e => { x0 = e.clientX; w0 = side.offsetWidth; side.classList.add('resizing'); handle.setPointerCapture(e.pointerId); });
    handle.addEventListener('pointermove', e => {
      if (x0 == null) return;
      const w = w0 + (e.clientX - x0);
      if (w < SIDE_W.min - 70) { side.classList.remove('resizing'); x0 = null; setSide(false); apply(w0); return; } // 拖得够窄 → 收起
      apply(Math.max(SIDE_W.min, Math.min(SIDE_W.max, w)));
    });
    const end = () => { if (x0 == null) return; x0 = null; side.classList.remove('resizing'); setPref('sideW', side.offsetWidth); window.dispatchEvent(new Event('resize')); };
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
    handle.addEventListener('dblclick', () => { apply(SIDE_W.def); setPref('sideW', SIDE_W.def); });
  }

  /* ======================= Tab 1：白板概览（资源管理器） ======================= */
  const closedSecs = new Set(JSON.parse(pref('treeClosed', '[]')));
  let lastLogShown = 0;
  let activeId = null;

  const chev = '<svg class="chev" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m6 8 4 4 4-4"/></svg>';
  const tt = t => esc(t).replace(/`([^`]+)`/g, '<code>$1</code>');

  function section(id, title, n, body) {
    return `<section class="tree-sec ${closedSecs.has(id) ? 'closed' : ''}" data-sec="${id}">
      <button class="tree-head" type="button">${chev}${title}${n != null ? `<span class="n">${n}</span>` : ''}</button>
      <div class="tree-body"><div>${body}</div></div>
    </section>`;
  }

  let exRaf = 0;
  function renderExplorer() {
    cancelAnimationFrame(exRaf);
    exRaf = requestAnimationFrame(() => {
      const s = store.state, now = store.clock.now();
      const filter = B.boardView.filter;
      const holders = B.engine.query.holders(s);
      const working = holders.length
        ? holders.map(({ name, card }) => `<button class="tree-row card-row" type="button" data-card="${card.id}" style="padding-left:20px">
            ${avatar({ kind: 'agent', name })}<span class="tt"><span class="mono" style="color:${agentColor(name)}">${esc(name)}</span> · #${card.id} ${tt(card.title)}</span>
            <span class="tm" data-exp="${card.expiresAt}">${mmss(card.expiresAt - now)}</span>
          </button>`).join('')
        : '<div class="tree-empty" style="padding-left:20px">没有 agent 在工作</div>';

      const logs = s.log.slice(-10).reverse();
      const recent = logs.map(ev => {
        const who = ev.actor.kind === 'human' ? '你' : ev.actor.kind === 'system' ? '系统' : ev.actor.name;
        return `<div class="recent-row ${ev.id > lastLogShown && lastLogShown ? 'new' : ''}">
          ${avatar(ev.actor)}
          <div><b style="${ev.actor.kind === 'agent' ? `color:${agentColor(ev.actor.name)}` : ''}">${esc(who)}</b> ${esc(OP[ev.op] || ev.op)} <span class="mono">#${ev.cardId}</span>
          <span class="when" data-at="${ev.at}">${ago(ev.at, now)}${ev.via === 'cli' ? ' · CLI' : ''}</span></div>
        </div>`;
      }).join('');
      lastLogShown = s.logSeq;

      // 标签：当前白板上的使用数，点击即按标签筛选
      const lbls = store.labelsOf();
      const used = {};
      Object.values(s.cards).forEach(c => (c.labels || []).forEach(l => (used[l] = (used[l] || 0) + 1)));
      const active = (filter.match(/^(?:label|标签)[:：](.+)$/) || [])[1];
      const labelRows = (lbls.length ? lbls.map(l => `<button class="tree-row lb-row ${active === l.name.toLowerCase() ? 'active' : ''}" type="button" data-filter-label="${esc(l.name)}" style="padding-left:20px">
          ${B.labels.chip(l)}<span class="n">${used[l.id] || 0}</span></button>`).join('') : '<div class="tree-empty" style="padding-left:20px">还没有标签</div>') +
        `<button class="tree-row" type="button" data-manage-labels style="padding-left:20px;color:var(--text-3)">${icons.edit}<span class="tt">管理标签</span></button>`;

      const ex = $('#explorer');
      const top = ex.scrollTop;
      ex.innerHTML = section('labels', '标签', lbls.length, labelRows) + section('working', '正在工作', holders.length, working) + section('recent', '最近动态', null, recent || '<div class="tree-empty" style="padding-left:20px">暂无</div>');
      ex.scrollTop = top;
    });
  }
  const OP = { create: '新建了', edit: '编辑了', move: '移动了', reorder: '调整了顺序', claim: '领取了', renew: '续期了', comment: '评论了', timeout: '超时释放了', release: '强制释放了', delete: '删除了' };

  function tickExplorer(now) {
    $$('#explorer [data-exp]').forEach(n => {
      const left = Number(n.dataset.exp) - now;
      n.textContent = mmss(left);
      n.classList.toggle('urgent', left < 5 * 60e3);
    });
  }

  function bindExplorer() {
    const ex = $('#explorer');
    ex.addEventListener('click', e => {
      const head = e.target.closest('.tree-head');
      if (head) {
        const sec = head.parentElement; sec.classList.toggle('closed');
        sec.classList.contains('closed') ? closedSecs.add(sec.dataset.sec) : closedSecs.delete(sec.dataset.sec);
        setPref('treeClosed', JSON.stringify([...closedSecs]));
        return;
      }
      const fl = e.target.closest('[data-filter-label]');
      if (fl) {
        const input = $('#boardFilter'), q = `label:${fl.dataset.filterLabel}`;
        input.value = input.value.trim().toLowerCase() === q.toLowerCase() ? '' : q;
        input.dispatchEvent(new Event('input'));
        return;
      }
      if (e.target.closest('[data-manage-labels]')) return B.labels.manage();
      const row = e.target.closest('[data-card]');
      if (row) {
        const id = Number(row.dataset.card);
        activeId = id;
        $$('#explorer .tree-row.active').forEach(r => r.classList.remove('active'));
        row.classList.add('active');
        B.boardView.reveal(id);
        bus.emit('card:open', id);
      }
    });
    // 折叠 / 展开侧栏分组
    $('#collapseTree').onclick = () => {
      const secs = $$('#explorer .tree-sec');
      const close = secs.some(x => !x.classList.contains('closed'));
      secs.forEach(x => { x.classList.toggle('closed', close); close ? closedSecs.add(x.dataset.sec) : closedSecs.delete(x.dataset.sec); });
      setPref('treeClosed', JSON.stringify([...closedSecs]));
    };

    // 筛选
    const input = $('#boardFilter'), wrap = input.closest('.side-filter');
    const apply = () => {
      const q = input.value;
      const n = B.boardView.setFilter(q);
      wrap.classList.toggle('has-value', !!q);
      const pill = $('#filterPill');
      if (q.trim()) { pill.hidden = false; pill.innerHTML = `筛选「${esc(q.trim())}」· ${n} 张<button type="button" aria-label="清除筛选">×</button>`; pill.querySelector('button').onclick = clear; }
      else pill.hidden = true;
      renderExplorer();
    };
    const clear = () => { input.value = ''; apply(); };
    input.addEventListener('input', apply);
    input.addEventListener('keydown', e => { if (e.key === 'Escape') { e.stopPropagation(); if (input.value) clear(); else input.blur(); } });
    $('#filterClear').onclick = () => { clear(); input.focus(); };
    bus.on('card:open', id => { activeId = id; });
  }

  /* ============================ Tab 2：未整理 ============================ */
  function bindInbox() {
    $('#inboxNew').onclick = () => B.creator.open({ column: 'inbox' });
    // 数量只显示在侧栏标题里；活动栏图标不加徽标——未整理条目常年存在，红点消不掉会一直打扰
    const update = () => { $('#inboxCount').textContent = store.state.cols.inbox.length; };
    update();
    store.subscribe((_, meta) => { if (!meta.foreign) update(); });
  }

  /* ============================ Tab 3：设置 ============================ */
  const SETTINGS = [
    { id: 'appearance', title: '外观', icon: icons.sun },
    { id: 'board', title: '白板规则', icon: icons.lock },
    { id: 'agents', title: 'Agent 接入', icon: icons.terminal },
    { id: 'shortcuts', title: '快捷键', icon: icons.keyboard },
    { id: 'demo', title: '演示工具', icon: icons.sparkle, demo: true },
    { id: 'about', title: '关于', icon: icons.info },
  ];

  const seg = (name, value, options) => `
    <div class="segmented" data-pref="${name}" role="radiogroup">
      ${options.map(([v, label]) => `<button type="button" role="radio" data-v="${v}" class="${String(v) === String(value) ? 'on' : ''}" aria-checked="${String(v) === String(value)}">${label}</button>`).join('')}
      <i class="seg-pill"></i>
    </div>`;
  const row = (label, hint, control) => `<div class="set-row"><div class="set-label"><b>${label}</b>${hint ? `<span>${hint}</span>` : ''}</div><div class="set-control">${control}</div></div>`;
  const code = (cmd, note) => `<div class="code-line"><code>${esc(cmd)}</code>${note ? `<span>${note}</span>` : ''}<button type="button" class="copy" data-copy="${esc(cmd)}" aria-label="复制">${icons.copy}</button></div>`;

  function renderSettings() {
    const P = B.platform;
    const flow = ['inbox', 'todo', 'doing', 'review', 'done'].map((k, i) => `${i ? '<i class="flow-arrow"></i>' : ''}<span class="flow-node" style="--c:var(--c-${k})"><i></i>${COLUMN_NAME[k]}<em>${k === 'inbox' ? '人' : k === 'doing' ? 'agent 领取' : k === 'done' ? '人确认' : k === 'review' ? '人检查' : '排优先级'}</em></span>`).join('');
    $('#settingsPage').innerHTML = `
      <div class="settings">
        <header class="settings-hero">
          <h1>设置</h1>
          <p>这块白板的显示方式，以及 agent 如何通过命令行接入。</p>
        </header>

        <section class="set-card" id="set-appearance" style="--i:1">
          <h2>${icons.sun}外观</h2>
          ${row('主题', '默认跟随系统', seg('theme', P.theme, [['auto', '跟随系统'], ['light', '浅色'], ['dark', '深色']]))}
          ${row('卡片密度', '紧凑模式下一屏能看到更多卡片', seg('density', pref('density', 'comfy'), [['comfy', '舒适'], ['compact', '紧凑']]))}
          ${row('动效', '减弱后关闭大部分过渡动画', seg('motion', pref('motion', 'full'), [['full', '完整'], ['reduced', '减弱']]))}
        </section>

        <section class="set-card" id="set-board" style="--i:1">
          <h2>${icons.lock}白板规则</h2>
          <div class="flow">${flow}</div>
          ${row('领取时长', '持有者评论、编辑或再次 claim 都会重新计时', `<span class="ro mono">30 分钟</span>`)}
          ${row('超时', '到期未续期：回到「待办」最上面，记录「领取超时，自动释放」', `<span class="ro">自动释放</span>`)}
          <ul class="rules">
            <li>进入「进行中」只能靠 agent 领取；人和 agent 都不能直接拖进去</li>
            <li>「进行中」的卡片只有持有者能编辑和移动；人可以强制释放</li>
            <li>一个 agent 同时只持有一张卡片</li>
            <li>「已完成」只归人管；删除只能在界面上做</li>
            <li>「未整理」里的条目不会被 agent 领取，整理后再放进「待办」</li>
          </ul>
        </section>

        <section class="set-card" id="set-agents" style="--i:2">
          <h2>${icons.terminal}Agent 接入</h2>
          <p class="set-p">任何能执行命令行的 agent 都能接入。身份由 <code>--as</code> / <code>BOARD_AS</code> 给出，白板由 <code>--board</code> / <code>BOARD</code> 给出（写成「项目/白板」），缺少任何一个命令都会被拒绝。所有命令都支持 <code>--json</code>。</p>
          <div class="code-block">
            ${code('export BOARD_AS=alpha', '设置身份')}
            ${code(`export BOARD=${store.current}`, '指定白板（当前白板）')}
            ${code('board boards', '列出所有白板')}
            ${code('board label 12 +bug -文档', '加 / 去标签')}
            ${code('board claim --json', '领取「待办」最上面的卡片')}
            ${code('board comment 12 "进展：已复现"', '记进度，同时续期')}
            ${code('board move 12 review', '做完，交给人审核')}
          </div>
          <details class="errcodes">
            <summary>错误码</summary>
            <div class="err-grid">${Object.entries(B.schema.ERRORS).filter(([k]) => k !== 'USAGE').map(([k, v]) => `<code>${k}</code><span>${esc(v)}</span>`).join('')}</div>
          </details>
        </section>

        <section class="set-card" id="set-shortcuts" style="--i:3">
          <h2>${icons.keyboard}快捷键</h2>
          <div class="keys">
            ${[['⌘ K', '搜索 / 命令面板'], ['P', '切换项目'], ['N', '新建卡片'], ['/', '搜索'], ['Esc', '关闭详情或面板'], ['↑ ↓ ← →', '在卡片间移动'], ['Enter', '打开卡片'], ['C', '详情里：复制为提示词'], ['L', '详情里：加标签'], ['↑ ↓ K J', '详情里切换上一张 / 下一张']].map(([k, v]) => `<div><span>${k.split(' ').map(x => `<kbd>${x}</kbd>`).join('')}</span>${v}</div>`).join('')}
          </div>
        </section>

        <section class="set-card demo" id="set-demo" style="--i:4">
          <h2>${icons.sparkle}演示工具 <span class="demo-tag">仅原型 · 正式产品不包含</span></h2>
          <p class="set-p">用来在原型里演示 CLI 与规则，不属于产品功能。</p>
          ${row('模拟 CLI 终端', '在页面底部模拟 agent 执行 board 命令（快捷键 `）', `<button type="button" class="toggle ${B.console?.isOpen() ? 'on' : ''}" data-toggle="console" role="switch" aria-checked="${!!B.console?.isOpen()}"><i></i></button>`)}
          ${row('演示时钟', '加速后能很快看到 30 分钟的超时释放', seg('speed', store.clock.speed, [[1, '×1'], [10, '×10'], [60, '×60']]))}
          ${row('外壳预览', '在浏览器里预览桌面端窗口', seg('shell', P.shell, [['web', 'Web'], ['desktop', '桌面']]))}
          ${row('示例数据', '所有打开的窗口同步生效', `<button type="button" class="btn sm danger" data-action="reset">${icons.reset}重置</button>`)}
        </section>

        <section class="set-card" id="set-about" style="--i:5">
          <h2>${icons.info}关于</h2>
          <div class="about">
            <div class="logo" aria-hidden="true"><i style="--h:100%"></i><i style="--h:70%"></i><i style="--h:46%"></i><i style="--h:82%"></i></div>
            <div><b>Board</b><span>看板式白板 · 人在网页上操作，agent 通过 CLI 操作，两边读写同一块白板</span><span class="mono">PRD v0.2 · 原型</span></div>
          </div>
        </section>
      </div>`;
    requestAnimationFrame(() => $$('#settingsPage .segmented').forEach(placePill));
  }

  function placePill(segEl) {
    const on = segEl.querySelector('button.on'), pill = segEl.querySelector('.seg-pill');
    if (!on || !pill) return;
    pill.style.width = `${on.offsetWidth}px`;
    pill.style.transform = `translateX(${on.offsetLeft - 3}px)`;
  }

  function applyPrefs() {
    const root = document.documentElement;
    root.dataset.density = pref('density', 'comfy');
    root.dataset.motion = pref('motion', 'full');
  }

  function bindSettings() {
    const page = $('#settingsPage');
    page.addEventListener('click', async e => {
      const b = e.target.closest('.segmented button');
      if (b) {
        const segEl = b.parentElement, name = segEl.dataset.pref, v = b.dataset.v;
        segEl.querySelectorAll('button').forEach(x => { x.classList.toggle('on', x === b); x.setAttribute('aria-checked', x === b); });
        placePill(segEl);
        if (name === 'theme') B.platform.setTheme(v);
        if (name === 'density' || name === 'motion') { setPref(name, v); applyPrefs(); }
        if (name === 'speed') store.clock.setSpeed(Number(v));
        if (name === 'shell') { B.platform.setShell(v); setTimeout(() => $$('#settingsPage .segmented').forEach(placePill), 400); }
        return;
      }
      const t = e.target.closest('[data-toggle="console"]');
      if (t) { B.console?.toggle(); t.classList.toggle('on', B.console?.isOpen()); t.setAttribute('aria-checked', !!B.console?.isOpen()); return; }
      const c = e.target.closest('[data-copy]');
      if (c) {
        try { await navigator.clipboard.writeText(c.dataset.copy); } catch {}
        c.classList.add('done'); c.innerHTML = icons.check;
        setTimeout(() => { c.classList.remove('done'); c.innerHTML = icons.copy; }, 1400);
        return;
      }
      if (e.target.closest('[data-action="reset"]')) {
        if (await B.ui.confirm({ title: '重置示例数据？', text: '当前白板上的所有卡片会被示例数据替换（所有打开的窗口同步生效）。', ok: '重置', icon: icons.reset })) store.reset();
      }
    });

    // 侧栏目录 + 滚动联动
    $('#settingsNav').innerHTML = SETTINGS.map(s => `<a href="#set-${s.id}" class="sn-item ${s.demo ? 'demo' : ''}" data-id="${s.id}">${s.icon}<span>${s.title}</span>${s.demo ? '<em class="demo-tag">演示</em>' : ''}</a>`).join('');
    $('#settingsNav').addEventListener('click', e => {
      const a = e.target.closest('.sn-item'); if (!a) return;
      e.preventDefault();
      const target = $(`#set-${a.dataset.id}`);
      spyLock = a.dataset.id;
      clearTimeout(spyTimer); spyTimer = setTimeout(() => (spyLock = null), 900);
      $$('.sn-item').forEach(x => x.classList.toggle('on', x === a));
      page.scrollTo({ top: target.offsetTop - 20, behavior: reduceMotion() ? 'auto' : 'smooth' });
      target.classList.remove('flash'); void target.offsetWidth; target.classList.add('flash');
    });
    page.addEventListener('scroll', spy, { passive: true });
    bus.on('tab:changed', t => { if (t === 'settings') { renderSettings(); spy(); } });
    store.clock.onChange(() => { if (app().dataset.tab === 'settings') renderSettings(); });
    bus.on('console:toggle', on => { const t = $('[data-toggle="console"]'); if (t) { t.classList.toggle('on', on); t.setAttribute('aria-checked', on); } });
  }
  let spyLock = null, spyTimer = 0;
  function spy() {
    if (spyLock) return;
    const page = $('#settingsPage');
    let cur = SETTINGS[0].id;
    for (const s of SETTINGS) { const n = $(`#set-${s.id}`); if (n && n.offsetTop - 80 <= page.scrollTop) cur = s.id; }
    if (page.scrollTop + page.clientHeight >= page.scrollHeight - 4) cur = SETTINGS[SETTINGS.length - 1].id;
    $$('.sn-item').forEach(a => a.classList.toggle('on', a.dataset.id === cur));
  }

  function syncProjectLabels() {
    const p = store.project, b = store.boardMeta;
    $$('.js-project-name').forEach(n => (n.textContent = p.name));
    $$('.js-board-name').forEach(n => (n.textContent = b.name));
    $$('.js-board-path').forEach(n => (n.textContent = b.path));
    document.title = `${b.name} · ${p.name} · Board`;
    document.documentElement.style.setProperty('--project', p.color);
  }

  /* ============================ 启动 ============================ */
  B.workbench = {
    init(s) {
      store = s;
      applyPrefs();
      $('#activitybar').addEventListener('click', e => { const b = e.target.closest('.ab-item'); if (b) setTab(b.dataset.tab, { toggle: true }); });
      bindSideResize();
      bindExplorer();
      bindInbox();
      bindSettings();
      $('#newCardBtn').onclick = () => B.creator.open({ column: 'todo' });
      $('#boardPromptBtn').onclick = () => B.prompt.copyBoard();
      syncProjectLabels();
      store.subscribe((_, meta) => {
        if (meta.source === 'switch') {
          $('#boardFilter').value = ''; B.boardView.setFilter(''); $('#boardFilter').closest('.side-filter').classList.remove('has-value'); $('#filterPill').hidden = true;
          activeId = null; lastLogShown = 0;
          if (app().dataset.tab === 'settings') renderSettings();
        }
        if (meta.source === 'switch' || meta.structure) syncProjectLabels();
      });
      app().dataset.side = pref('side', 'open');
      setTab(new URLSearchParams(location.search).get('tab') || pref('tab', 'board'));
      renderExplorer();
      store.subscribe((_, meta) => { if (!meta.foreign) renderExplorer(); });
      window.addEventListener('resize', () => { moveIndicator(); $$('#settingsPage .segmented').forEach(placePill); });
      document.fonts?.ready.then(moveIndicator);
    },
    setTab,
    get tab() { return app().dataset.tab; },
    toggleSide: () => setSide(app().dataset.side === 'closed'),
    tick(now) { tickExplorer(now); },
  };
})(window.Board = window.Board || {});
