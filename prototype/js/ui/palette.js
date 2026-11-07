/* ==========================================================================
   Board · 命令面板（⌘K / Ctrl K）
   搜索卡片、执行界面操作；以 > 或 board 开头时作为 CLI 命令发送到控制台。
   扩展：B.palette.register({ id, title, hint, icon, run, group }) 追加命令。
   ========================================================================== */
(function (B) {
  'use strict';
  const { ALL_KEYS, COLUMN_NAME } = B.schema;
  const { $, esc, icons, bus } = B.ui;

  let store;
  let items = [];
  let active = 0;
  const extra = [];

  function actions() {
    const P = B.platform;
    const W = B.workbench;
    const list = [
      { id: 'new', group: '操作', title: '新建卡片', hint: 'N', icon: icons.plus, run: () => B.creator.open({ column: 'todo' }) },
      { id: 'capture', group: '操作', title: '记一条未整理', hint: 'I', icon: icons.inbox, run: () => B.creator.open({ column: 'inbox' }) },
      { id: 'tab-board', group: '切换到', title: '白板', hint: '⌘1', icon: icons.move, run: () => W.setTab('board') },
      { id: 'tab-inbox', group: '切换到', title: '未整理', hint: '⌘3', icon: icons.inbox, run: () => W.setTab('inbox') },
      { id: 'tab-settings', group: '切换到', title: '设置', hint: '⌘4', icon: icons.sun, run: () => W.setTab('settings') },
      { id: 'side', group: '切换到', title: '收起 / 展开侧栏', hint: '⌘B', icon: icons.monitor, run: () => W.toggleSide() },
      { id: 'board-prompt', group: '操作', title: '把当前白板复制为提示词', icon: icons.prompt, run: () => B.prompt.copyBoard() },
      { id: 'labels', group: '操作', title: '管理标签', icon: icons.tag, run: () => B.labels.manage() },
      { id: 'theme', group: '外观', title: `切换主题（当前：${{ auto: '跟随系统', light: '浅色', dark: '深色' }[P.theme]}）`, icon: icons.sun, run: () => B.app.cycleTheme() },
    ];
    if (B.console) list.push(
      { id: 'console', group: '演示（仅原型）', title: B.console.isOpen() ? '收起模拟终端' : '打开模拟终端', hint: '`', icon: icons.terminal, run: () => B.console.toggle() },
      { id: 'agents', group: '演示（仅原型）', title: B.sim.anyRunning() ? '暂停所有模拟 agent' : '启动所有模拟 agent', icon: icons.sparkle, run: () => { B.sim.toggleAll(); B.console.open(); } },
      { id: 'speed1', group: '演示（仅原型）', title: '演示时钟 ×1（真实时间）', icon: icons.timer, run: () => store.clock.setSpeed(1) },
      { id: 'speed60', group: '演示（仅原型）', title: '演示时钟 ×60（30 分钟超时 ≈ 30 秒）', icon: icons.timer, run: () => store.clock.setSpeed(60) },
      { id: 'shell', group: '演示（仅原型）', title: P.shell === 'desktop' ? '预览 Web 外壳' : '预览桌面端外壳', icon: icons.monitor, run: () => P.toggleShell() },
      { id: 'reset', group: '演示（仅原型）', title: '重置示例数据', icon: icons.reset, run: async () => { if (await B.ui.confirm({ title: '重置示例数据？', text: '当前白板上的所有卡片会被示例数据替换（所有打开的窗口同步生效）。', ok: '重置', icon: icons.reset })) store.reset(); } },
    );
    store.boards.filter(b => b.path !== store.current).forEach(b => list.push(
      { id: 'board-' + b.path, group: '切换白板', title: `${b.projectName} / ${b.name}`, sub: b.path, icon: icons.board, run: () => B.projects.openBoard(b.path) }));
    list.push({ id: 'board-new', group: '切换白板', title: '新建白板…', icon: icons.plus, run: () => B.projects.newBoard(store.projectKey) });
    list.push({ id: 'proj-new', group: '切换白板', title: '新建项目…', icon: icons.folder, run: () => B.projects.newProject() });
    list.push({ id: 'proj-overview', group: '切换白板', title: '项目总览（画布）', hint: '⌘2', icon: icons.folder, run: () => W.setTab('projects') });
    return [...list, ...extra];
  }

  function highlight(text, q) {
    const code = s => s.replace(/`([^`]+)`/g, '<code>$1</code>');
    if (!q) return code(esc(text));
    const i = text.toLowerCase().indexOf(q.toLowerCase());
    if (i < 0) return code(esc(text));
    return code(esc(text.slice(0, i)) + '<mark>' + esc(text.slice(i, i + q.length)) + '</mark>' + esc(text.slice(i + q.length)));
  }

  function compute(q) {
    q = q.trim();
    const out = [];
    if (B.console && /^(>|board\b)/.test(q)) {
      const cmd = q.replace(/^>\s*/, '');
      out.push({ group: 'CLI', title: cmd.startsWith('board') ? cmd : `board ${cmd}`, sub: '在控制台执行', icon: icons.terminal, run: () => { B.console.open(); B.console.exec(cmd.startsWith('board') ? cmd : `board ${cmd}`); } });
      return out;
    }
    const s = store.state;
    const all = ALL_KEYS.flatMap(col => (s.cols[col] || []).map(id => s.cards[id]));
    let cards;
    if (!q) cards = [...all].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 6);
    else {
      const ql = q.toLowerCase().replace(/^#/, '');
      cards = all.filter(c => String(c.id) === ql || c.title.toLowerCase().includes(ql) || c.body.toLowerCase().includes(ql)).slice(0, 12);
    }
    cards.forEach(c => out.push({
      group: q ? '卡片' : '最近更新', card: c, html: highlight(c.title, q.replace(/^#/, '')),
      sub: c.holder ? `${COLUMN_NAME[c.column]} · ${c.holder}` : COLUMN_NAME[c.column],
      run: () => {
        if (c.column === 'inbox') B.workbench.setTab('inbox');
        else if (B.workbench.tab === 'settings') B.workbench.setTab('board');
        setTimeout(() => { B.boardView.reveal(c.id); bus.emit('card:open', c.id); }, 60);
      },
    }));
    const acts = actions().filter(a => !q || a.title.toLowerCase().includes(q.toLowerCase()));
    out.push(...acts);
    return out;
  }

  function render() {
    const q = $('#paletteInput').value;
    $('.palette-search').classList.toggle('cli', /^(>|board\b)/.test(q.trim()));
    items = compute(q);
    active = Math.min(active, Math.max(0, items.length - 1));
    if (!items.length) { $('#paletteList').innerHTML = `<div class="p-empty">没有匹配的结果。以 <kbd>&gt;</kbd> 开头可以执行 CLI 命令</div>`; return; }
    let g = null, i = 0, html = '';
    for (const it of items) {
      if (it.group !== g) { html += `<div class="p-group">${esc(it.group)}</div>`; g = it.group; }
      const ic = it.card
        ? `<span class="p-ic col" style="--c:var(--c-${it.card.column})">${it.card.column.slice(0, 2).toUpperCase()}</span>`
        : `<span class="p-ic">${it.icon || icons.sparkle}</span>`;
      html += `<button type="button" class="p-item ${i === active ? 'active' : ''}" role="option" data-i="${i}">
        ${ic}
        ${it.card ? `<span class="p-id">#${it.card.id}</span>` : ''}
        <span class="p-main">${it.html || esc(it.title)}</span>
        ${it.sub ? `<span class="p-sub">${esc(it.sub)}</span>` : ''}
        ${it.hint ? `<kbd>${esc(it.hint)}</kbd>` : ''}
      </button>`;
      i++;
    }
    $('#paletteList').innerHTML = html;
  }

  function setActive(i) {
    active = (i + items.length) % items.length;
    document.querySelectorAll('.p-item').forEach(n => n.classList.toggle('active', Number(n.dataset.i) === active));
    document.querySelector('.p-item.active')?.scrollIntoView({ block: 'nearest' });
  }

  function run(i) {
    const it = items[i]; if (!it) return;
    close();
    setTimeout(() => it.run(), 60);
  }

  let prevFocus = null;
  function open(prefill = '') {
    const w = $('#palette');
    prevFocus = document.activeElement;
    w.classList.add('on'); w.setAttribute('aria-hidden', 'false');
    const input = $('#paletteInput');
    input.value = prefill; active = 0; render();
    setTimeout(() => input.focus(), 30);
  }
  function close() {
    const w = $('#palette');
    if (!w.classList.contains('on')) return;
    w.classList.remove('on'); w.setAttribute('aria-hidden', 'true');
    prevFocus?.focus?.({ preventScroll: true });
  }

  B.palette = {
    init(s) {
      store = s;
      const input = $('#paletteInput');
      input.addEventListener('input', () => { active = 0; render(); });
      input.addEventListener('keydown', e => {
        if (e.key === 'ArrowDown') { e.preventDefault(); setActive(active + 1); }
        else if (e.key === 'ArrowUp') { e.preventDefault(); setActive(active - 1); }
        else if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); run(active); }
        else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
      });
      $('#paletteList').addEventListener('click', e => { const b = e.target.closest('.p-item'); if (b) run(Number(b.dataset.i)); });
      $('#paletteList').addEventListener('pointermove', e => { const b = e.target.closest('.p-item'); if (b && Number(b.dataset.i) !== active) setActive(Number(b.dataset.i)); });
      $('#palette').addEventListener('click', e => { if (e.target.id === 'palette') close(); });
      $('#openPalette').addEventListener('click', () => open());
    },
    open, close,
    isOpen: () => $('#palette').classList.contains('on'),
    register(cmd) { extra.push({ group: '扩展', ...cmd }); },
  };
})(window.Board = window.Board || {});
