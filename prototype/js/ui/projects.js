/* ==========================================================================
   Board · 项目与白板（项目 1 : N 白板）
   - 顶栏切换器：按项目分组列出白板，搜索 + 键盘选择（P 打开）
   - 侧栏「项目」树：项目 → 白板，点白板打开；⋯ 菜单里新建 / 编辑 / 删除
   - 表单弹窗：新建项目（同时建第一块白板）、新建白板、编辑项目 / 白板
   画布（canvas.js）与这里共用 newProject / newBoard / contextMenu
   ========================================================================== */
(function (B) {
  'use strict';
  const { $, $$, esc, icons, toast, toastError, reduceMotion, isMobile } = B.ui;

  let store;
  let items = [];
  let active = 0;

  const initial = name => [...String(name || '?').trim()][0]?.toUpperCase() || '?';
  const av = (p, cls = '') => `<span class="pj-av ${cls}" style="--pc:${p.color}">${esc(initial(p.name))}</span>`;
  /** 从名称生成标识；中文名称等无法生成时用 prefix-1、prefix-2… */
  const autoKey = (name, taken, prefix) => {
    const slug = B.Store.slug(name);
    if (slug && !taken(slug)) return slug;
    const base = slug || prefix;
    let n = slug ? 2 : 1, k;
    do { k = `${base}-${n++}`; } while (taken(k));
    return k;
  };

  /* ============================ 顶栏切换器 ============================ */
  function renderSwitch() {
    const p = store.project, b = store.boardMeta;
    const btn = $('#projectSwitch');
    btn.querySelector('.pj-av-slot').innerHTML = av(p, 'sm');
    btn.querySelector('.pj-label').textContent = p.name;
    btn.querySelector('.pj-board').textContent = b.name;
    const others = store.boards.filter(x => x.path !== b.path && x.holders.length).length;
    btn.querySelector('.pj-elsewhere').hidden = !others;
    btn.querySelector('.pj-elsewhere').title = `另外 ${others} 块白板上有 agent 正在工作`;
  }

  function openMenu() {
    const m = $('#projectMenu');
    const r = $('#projectSwitch').getBoundingClientRect();
    m.style.left = `${Math.max(8, Math.min(r.left, innerWidth - 388))}px`;
    m.style.top = `${r.bottom + 8}px`;
    m.classList.add('on'); m.setAttribute('aria-hidden', 'false');
    $('#projectSwitch').setAttribute('aria-expanded', 'true');
    $('#projectSearch').value = '';
    active = Math.max(0, store.boards.findIndex(x => x.path === store.current));
    renderMenu();
    setTimeout(() => $('#projectSearch').focus(), 20);
  }
  function closeMenu() {
    const m = $('#projectMenu');
    if (!m.classList.contains('on')) return;
    m.classList.remove('on'); m.setAttribute('aria-hidden', 'true');
    $('#projectSwitch').setAttribute('aria-expanded', 'false');
  }
  const menuOpen = () => $('#projectMenu').classList.contains('on');

  function renderMenu() {
    const q = $('#projectSearch').value.trim().toLowerCase();
    const hit = (p, b) => !q || `${p.name}\n${p.key}\n${b.name}\n${b.key}\n${b.desc}`.toLowerCase().includes(q);
    items = [];
    let html = '';
    for (const p of store.projects) {
      const bs = p.boards.filter(b => hit(p, b));
      if (!bs.length) continue;
      html += `<div class="pj-group">${av(p, 'sm')}<b>${esc(p.name)}</b><span class="mono">${esc(p.key)}</span></div>`;
      for (const b of bs) {
        const i = items.push(b) - 1;
        const cur = b.path === store.current;
        html += `<button type="button" class="pj-row ${cur ? 'current' : ''} ${i === active ? 'active' : ''}" data-board="${esc(b.path)}" data-i="${i}" role="option" aria-selected="${cur}">
          <span class="pj-bic" style="--pc:${p.color}">${icons.board}</span>
          <span class="pj-main"><b>${esc(b.name)}</b><span class="mono">${esc(b.path)}</span></span>
          <span class="pj-stats">
            ${b.counts.doing ? `<span class="pj-live" title="${esc([...new Set(b.holders)].join('、'))} 正在工作"><i></i>${b.counts.doing}</span>` : ''}
            <span title="待办">${b.counts.todo}</span>
          </span>
          <span class="pj-check">${cur ? icons.check : ''}</span>
        </button>`;
      }
    }
    active = Math.min(active, Math.max(0, items.length - 1));
    $('#projectList').innerHTML = html || `<div class="pj-empty">没有匹配的白板</div>`;
  }
  function setActive(i) {
    if (!items.length) return;
    active = (i + items.length) % items.length;
    $$('#projectList .pj-row').forEach(r => r.classList.toggle('active', Number(r.dataset.i) === active));
    $('#projectList .pj-row.active')?.scrollIntoView({ block: 'nearest' });
  }

  /** 打开一块白板（切到「白板」Tab） */
  function openBoard(path) {
    closeMenu();
    const changed = path !== store.current;
    store.setBoard(path);
    if (B.workbench.tab !== 'board' && B.workbench.tab !== 'inbox') B.workbench.setTab('board');
    if (changed && !reduceMotion()) {
      $('.page-board').animate([{ opacity: 0, transform: 'translateY(8px)' }, { opacity: 1, transform: 'none' }], { duration: 380, easing: 'cubic-bezier(.16,1,.3,1)' });
      $('#projectSwitch').animate([{ transform: 'scale(.96)' }, { transform: 'none' }], { duration: 320, easing: 'cubic-bezier(.34,1.56,.64,1)' });
    }
  }

  /* ============================ 侧栏：项目树 ============================ */
  const closed = new Set(JSON.parse(localStorage.getItem('board.projTree.closed') || '[]'));
  function renderTree() {
    const q = $('#projFilter').value.trim().toLowerCase();
    const projects = store.projects;
    $('#projCount').textContent = `${projects.length} 个项目`;
    let html = '';
    for (const p of projects) {
      const pHit = !q || `${p.name}\n${p.key}\n${p.desc}`.toLowerCase().includes(q);
      const bs = p.boards.filter(b => pHit || `${b.name}\n${b.key}\n${b.desc}`.toLowerCase().includes(q));
      if (!pHit && !bs.length) continue;
      const isClosed = closed.has(p.key) && !q;
      const working = new Set(p.holders).size;
      html += `<div class="pt-proj ${isClosed ? 'closed' : ''}" data-project="${esc(p.key)}" role="treeitem" aria-expanded="${!isClosed}">
        <div class="pt-row proj ${p.key === store.projectKey ? 'in-current' : ''}" data-toggle="${esc(p.key)}" tabindex="0">
          <svg class="chev" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m6 8 4 4 4-4"/></svg>
          ${av(p, 'sm')}
          <span class="nm">${esc(p.name)}</span>
          ${working ? `<span class="pt-live" title="${working} 个 agent 在工作"><i></i>${working}</span>` : ''}
          <span class="n">${p.boards.length}</span>
          <button type="button" class="pt-more" data-ctx="project" data-id="${esc(p.key)}" aria-label="更多">${icons.more}</button>
        </div>
        <div class="pt-children"><div>
          ${bs.map(b => `
            <div class="pt-row brd ${b.path === store.current ? 'current' : ''}" data-board="${esc(b.path)}" tabindex="0" role="treeitem" style="--pc:${p.color}">
              <span class="pt-bic">${icons.board}</span>
              <span class="nm">${esc(b.name)}</span>
              <span class="key mono">${esc(b.key)}</span>
              ${b.counts.doing ? `<span class="pt-live" title="进行中"><i></i>${b.counts.doing}</span>` : ''}
              <span class="n" title="待办">${b.counts.todo}</span>
              <button type="button" class="pt-more" data-ctx="board" data-id="${esc(b.path)}" aria-label="更多">${icons.more}</button>
            </div>`).join('')}
          <button type="button" class="pt-row add" data-add="${esc(p.key)}">${icons.plus}新建白板</button>
        </div></div>
      </div>`;
    }
    $('#projTree').innerHTML = html || `<div class="tree-empty" style="padding:14px 20px">没有匹配的项目或白板</div>`;
  }

  function bindTree() {
    const tree = $('#projTree');
    tree.addEventListener('click', e => {
      const m = e.target.closest('[data-ctx]');
      if (m) { e.stopPropagation(); return contextMenu(m.dataset.ctx, m.dataset.id, m); }
      const add = e.target.closest('[data-add]'); if (add) return newBoard(add.dataset.add);
      const t = e.target.closest('[data-toggle]');
      if (t) {
        const k = t.dataset.toggle, wrap = t.parentElement;
        wrap.classList.toggle('closed');
        wrap.classList.contains('closed') ? closed.add(k) : closed.delete(k);
        localStorage.setItem('board.projTree.closed', JSON.stringify([...closed]));
        if (!wrap.classList.contains('closed')) B.canvas.focusProject(k);
        return;
      }
      const b = e.target.closest('[data-board]'); if (b) openBoard(b.dataset.board);
    });
    tree.addEventListener('keydown', e => {
      if (e.key !== 'Enter' && e.key !== ' ') return;
      const r = e.target.closest('.pt-row'); if (!r) return;
      e.preventDefault(); r.click();
    });
    tree.addEventListener('contextmenu', e => {
      const r = e.target.closest('[data-board], [data-toggle]'); if (!r) return;
      e.preventDefault();
      contextMenu(r.dataset.board ? 'board' : 'project', r.dataset.board || r.dataset.toggle, { x: e.clientX, y: e.clientY });
    });
    tree.addEventListener('pointerover', e => { const r = e.target.closest('[data-board]'); if (r) B.canvas.highlight(r.dataset.board, true); });
    tree.addEventListener('pointerleave', () => B.canvas.highlight(null, false));
    const f = $('#projFilter');
    f.addEventListener('input', () => { f.closest('.side-filter').classList.toggle('has-value', !!f.value); renderTree(); });
    f.addEventListener('keydown', e => { if (e.key === 'Escape') { e.stopPropagation(); f.value = ''; f.dispatchEvent(new Event('input')); } });
    $('#projFilterClear').onclick = () => { f.value = ''; f.dispatchEvent(new Event('input')); f.focus(); };
    $('#newProjectSide').onclick = () => newProject();
  }

  /* ============================ ⋯ 菜单 ============================ */
  function contextMenu(kind, id, at) {
    const m = $('#ctxMenu');
    let html = '';
    if (kind === 'project') {
      const p = store.projectMeta(id); if (!p) return;
      html = `
        <div class="ctx-title">${av(p, 'sm')}<b>${esc(p.name)}</b></div>
        <button type="button" data-do="new-board">${icons.plus}新建白板</button>
        <button type="button" data-do="edit-project">${icons.edit}编辑项目</button>
        <button type="button" data-do="labels">${icons.tag}管理标签</button>
        <div class="ctx-swatches">${B.Store.COLORS.map(c => `<button type="button" class="swatch ${c === p.color ? 'on' : ''}" data-color="${c}" style="--sw:${c}" aria-label="颜色 ${c}"></button>`).join('')}</div>
        <div class="ctx-sep"></div>
        <button type="button" data-do="delete-project" class="danger">${icons.trash}删除项目</button>`;
    } else {
      const b = store.boardMetaOf(id); if (!b) return;
      html = `
        <div class="ctx-title"><span class="pj-bic" style="--pc:${b.color}">${icons.board}</span><b>${esc(b.name)}</b></div>
        <button type="button" data-do="open">${icons.move}打开白板</button>
        <button type="button" data-do="edit-board">${icons.edit}编辑白板</button>
        <button type="button" data-do="copy-board-prompt">${icons.prompt}复制为提示词</button>
        <button type="button" data-do="copy-path">${icons.copy}复制 <code>BOARD=${esc(b.path)}</code></button>
        <div class="ctx-sep"></div>
        <button type="button" data-do="delete-board" class="danger" ${store.paths().length <= 1 ? 'disabled' : ''}>${icons.trash}删除白板</button>`;
    }
    m.innerHTML = html;
    m.dataset.kind = kind; m.dataset.id = id;
    m.classList.add('on'); m.setAttribute('aria-hidden', 'false');
    const r = at.getBoundingClientRect ? at.getBoundingClientRect() : { left: at.x, right: at.x, bottom: at.y, top: at.y };
    const w = m.offsetWidth, h = m.offsetHeight;
    m.style.left = `${Math.max(8, Math.min((at.getBoundingClientRect ? r.right - w : r.left), innerWidth - w - 8))}px`;
    m.style.top = `${r.bottom + h + 8 > innerHeight ? Math.max(8, r.top - h - 6) : r.bottom + 6}px`;
    m.querySelector('button:not([disabled])')?.focus({ preventScroll: true });
  }
  function closeCtx() { const m = $('#ctxMenu'); m.classList.remove('on'); m.setAttribute('aria-hidden', 'true'); }

  function bindCtx() {
    const m = $('#ctxMenu');
    m.addEventListener('click', async e => {
      const sw = e.target.closest('.swatch');
      const { kind, id } = m.dataset;
      if (sw) { store.updateProject(id, { color: sw.dataset.color }); m.querySelectorAll('.swatch').forEach(x => x.classList.toggle('on', x === sw)); return; }
      const b = e.target.closest('[data-do]'); if (!b) return;
      closeCtx();
      const act = b.dataset.do;
      if (act === 'new-board') newBoard(id);
      if (act === 'edit-project') editProject(id);
      if (act === 'labels') B.labels?.manage(id);
      if (act === 'open') openBoard(id);
      if (act === 'edit-board') editBoard(id);
      if (act === 'copy-board-prompt') B.prompt?.copyBoard(id);
      if (act === 'copy-path') { try { await navigator.clipboard.writeText(`export BOARD=${id}`); } catch {} toast({ type: 'success', title: '已复制', text: `export BOARD=${id}`, duration: 1800 }); }
      if (act === 'delete-project') deleteProject(id);
      if (act === 'delete-board') deleteBoard(id);
    });
    m.addEventListener('keydown', e => {
      const bs = [...m.querySelectorAll('[data-do]:not([disabled])')];
      const i = bs.indexOf(document.activeElement);
      if (e.key === 'ArrowDown') { e.preventDefault(); bs[(i + 1) % bs.length]?.focus(); }
      if (e.key === 'ArrowUp') { e.preventDefault(); bs[(i - 1 + bs.length) % bs.length]?.focus(); }
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeCtx(); }
    });
    document.addEventListener('pointerdown', e => { if (!e.target.closest('#ctxMenu, [data-ctx]')) closeCtx(); });
    window.addEventListener('resize', closeCtx);
  }

  /* ============================ 表单弹窗 ============================ */
  /**
   * fields: [{ name, label, hint, type: 'text'|'key'|'color'|'select', value, placeholder, from, auto, validate, options, readonly }]
   */
  let formState = null;
  function openForm(cfg) {
    formState = { ...cfg, touched: {}, color: (cfg.fields.find(f => f.type === 'color') || {}).value };
    const f = $('#formModalForm');
    f.innerHTML = `
      <h3>${esc(cfg.title)}</h3>
      ${cfg.subtitle ? `<p>${cfg.subtitle}</p>` : ''}
      ${cfg.preview ? `<div class="pj-preview" id="fmPreview"></div>` : ''}
      ${cfg.fields.map(fd => fieldHtml(fd)).join('')}
      <div class="field-err" id="fmErr" hidden></div>
      <div class="dialog-actions">
        <button class="btn" type="button" data-act="cancel">取消</button>
        <button class="btn brand" type="submit">${esc(cfg.submit || '保存')}</button>
      </div>`;
    sync();
    const w = $('#formModal');
    w.classList.add('on'); w.setAttribute('aria-hidden', 'false');
    setTimeout(() => f.querySelector('input:not([readonly]), select')?.focus(), 40);
  }
  function fieldHtml(fd) {
    const hint = fd.hint ? ` <em>${fd.hint}</em>` : '';
    if (fd.type === 'color') return `<div class="field"><span>${fd.label}${hint}</span><div class="swatches" data-field="${fd.name}">${B.Store.COLORS.map(c => `<button type="button" class="swatch ${c === fd.value ? 'on' : ''}" data-color="${c}" style="--sw:${c}" aria-label="${c}"></button>`).join('')}</div></div>`;
    if (fd.type === 'select') return `<label class="field"><span>${fd.label}${hint}</span><select class="input" name="${fd.name}">${fd.options.map(o => `<option value="${esc(o.value)}" ${o.value === fd.value ? 'selected' : ''}>${esc(o.label)}</option>`).join('')}</select></label>`;
    return `<label class="field"><span>${fd.label}${hint}</span><input class="input ${fd.type === 'key' ? 'mono' : ''}" name="${fd.name}" value="${esc(fd.value || '')}" placeholder="${esc(fd.placeholder || '')}" maxlength="${fd.type === 'key' ? 32 : 60}" autocomplete="off" spellcheck="false" ${fd.readonly ? 'readonly' : ''}></label>`;
  }
  function values() {
    const f = $('#formModalForm'), v = {};
    formState.fields.forEach(fd => { v[fd.name] = fd.type === 'color' ? formState.color : f.elements[fd.name]?.value ?? ''; });
    return v;
  }
  function sync() {
    const f = $('#formModalForm');
    // 标识自动跟随名称，直到用户手动改过
    formState.fields.filter(fd => fd.type === 'key' && fd.from && !fd.readonly && !formState.touched[fd.name]).forEach(fd => {
      const v = values();
      f.elements[fd.name].value = v[fd.from].trim() ? fd.auto(v[fd.from], v) : '';
    });
    const v = values();
    if (formState.preview) $('#fmPreview').innerHTML = formState.preview(v);
    const err = formState.fields.map(fd => fd.validate && v[fd.name] ? fd.validate(v[fd.name], v) : null).find(Boolean);
    $('#fmErr').hidden = !err; $('#fmErr').textContent = err || '';
  }
  function closeForm() { const w = $('#formModal'); w.classList.remove('on'); w.setAttribute('aria-hidden', 'true'); formState = null; }
  function bindForm() {
    const f = $('#formModalForm');
    f.addEventListener('input', e => {
      const fd = formState?.fields.find(x => x.name === e.target.name);
      if (fd?.type === 'key') { formState.touched[fd.name] = true; e.target.value = e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ''); }
      sync();
    });
    f.addEventListener('change', sync);
    f.addEventListener('click', e => {
      if (e.target.closest('[data-act="cancel"]')) return closeForm();
      const sw = e.target.closest('.swatch');
      if (sw) { formState.color = sw.dataset.color; sw.parentElement.querySelectorAll('.swatch').forEach(x => x.classList.toggle('on', x === sw)); sync(); }
    });
    f.addEventListener('submit', e => {
      e.preventDefault();
      const v = values();
      const first = formState.fields.find(fd => fd.required && !String(v[fd.name]).trim());
      if (first) { const el = f.elements[first.name]; el.classList.remove('invalid'); void el.offsetWidth; el.classList.add('invalid'); el.focus(); return; }
      try { const cfg = formState; closeForm(); cfg.onSubmit(v); }
      catch (err) { $('#fmErr').hidden = false; $('#fmErr').textContent = err.message; }
    });
    $('#formModal').addEventListener('click', e => { if (e.target.id === 'formModal') closeForm(); });
    $('#formModal').addEventListener('keydown', e => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeForm(); } });
  }
  /** 出错时重新打开表单并保留输入 */
  function retry(cfg, v, err) {
    cfg.fields.forEach(fd => { if (fd.type === 'color') fd.value = v[fd.name]; else fd.value = v[fd.name]; });
    openForm(cfg);
    Object.keys(v).forEach(k => { if (cfg.fields.find(fd => fd.name === k && fd.type === 'key')) formState.touched[k] = true; });
    $('#fmErr').hidden = false; $('#fmErr').textContent = err.message;
  }

  /* ============================ 动作 ============================ */
  const pkTaken = k => !!store.projectMeta(k);
  function newProject({ pos } = {}) {
    closeMenu();
    const cfg = {
      title: '新建项目', subtitle: '一个项目下可以有多块白板。先建第一块，之后随时在项目里添加。', submit: '创建项目',
      preview: v => `${av({ name: v.name || '新', color: formState.color })}<div><b>${esc(v.name || '新项目')}</b><span class="mono">export BOARD=${esc(v.key || '…')}/${esc(v.boardKey || 'main')}</span></div>`,
      fields: [
        { name: 'name', label: '项目名称', placeholder: '例如：官网改版', required: true },
        { name: 'key', label: '项目标识', hint: '小写字母、数字和 -；创建后不能修改', type: 'key', from: 'name', placeholder: 'site', auto: n => autoKey(n, pkTaken, 'project'), validate: k => store.validateProjectKey(k) },
        { name: 'desc', label: '描述', hint: '选填', placeholder: '一句话说明这个项目' },
        { name: 'color', label: '颜色', type: 'color', value: B.Store.COLORS[store.projects.length % B.Store.COLORS.length] },
        { name: 'boardName', label: '第一块白板', value: '主白板', required: true },
        { name: 'boardKey', label: '白板标识', type: 'key', from: 'boardName', value: 'main', auto: n => B.Store.slug(n) || 'main' },
      ],
      onSubmit: v => {
        try {
          const p = store.createProject({ name: v.name, key: v.key, desc: v.desc, color: v.color, pos, board: { name: v.boardName, key: v.boardKey || 'main' } });
          openBoardAfterCreate(`${p.key}/${p.boards[0].key}`);
          toast({ type: 'success', title: `已创建项目「${p.name}」`, text: `agent 用 BOARD=${p.key}/${p.boards[0].key} 接入第一块白板。`, duration: 4200 });
        } catch (err) { retry(cfg, v, err); }
      },
    };
    openForm(cfg);
  }

  function newBoard(pk, { pos } = {}) {
    closeMenu();
    pk = pk || store.projectKey;
    const p = store.projectMeta(pk);
    const taken = k => !!store.boardMetaOf(`${pk}/${k}`);
    const cfg = {
      title: '新建白板', subtitle: `白板属于一个项目，用「项目/白板」定位。`, submit: '创建白板',
      preview: v => { const pp = store.projectMeta(v.project) || p; return `<span class="pj-bic lg" style="--pc:${pp.color}">${icons.board}</span><div><b>${esc(pp.name)} / ${esc(v.name || '新白板')}</b><span class="mono">export BOARD=${esc(v.project)}/${esc(v.key || '…')}</span></div>`; },
      fields: [
        { name: 'project', label: '所属项目', type: 'select', value: pk, options: store.projects.map(x => ({ value: x.key, label: `${x.name}（${x.key}）` })) },
        { name: 'name', label: '白板名称', placeholder: '例如：v1.0 发布', required: true },
        { name: 'key', label: '白板标识', hint: '小写字母、数字和 -；创建后不能修改', type: 'key', from: 'name', placeholder: 'release',
          auto: (n, v) => autoKey(n, k => !!store.boardMetaOf(`${v.project}/${k}`), 'board'), validate: (k, v) => store.validateBoardKey(v.project, k) },
        { name: 'desc', label: '描述', hint: '选填', placeholder: '这块白板用来做什么' },
      ],
      onSubmit: v => {
        try {
          const b = store.createBoard(v.project, { name: v.name, key: v.key, desc: v.desc, pos: v.project === pk ? pos : undefined });
          if (B.workbench.tab === 'projects') { B.canvas.render(); setTimeout(() => B.canvas.focusBoard(b.path), 60); toast({ type: 'success', title: `已创建白板「${b.name}」`, text: `BOARD=${b.path}`, action: { label: '打开', fn: () => openBoard(b.path) } }); }
          else { openBoardAfterCreate(b.path); toast({ type: 'success', title: `已创建白板「${b.name}」`, text: `agent 用 BOARD=${b.path} 接入。` }); }
        } catch (err) { retry(cfg, v, err); }
      },
    };
    openForm(cfg);
  }
  function openBoardAfterCreate(path) {
    if (B.workbench.tab === 'projects') { B.canvas.render(); store.setBoard(path); setTimeout(() => B.canvas.focusBoard(path), 60); }
    else openBoard(path);
  }

  function editProject(pk) {
    const p = store.projectMeta(pk); if (!p) return;
    const cfg = {
      title: '编辑项目', submit: '保存',
      preview: v => `${av({ name: v.name || p.name, color: formState.color })}<div><b>${esc(v.name || p.name)}</b><span class="mono">${esc(p.key)} · ${p.boards.length} 块白板</span></div>`,
      fields: [
        { name: 'name', label: '项目名称', value: p.name, required: true },
        { name: 'key', label: '项目标识', hint: '创建后不能修改', type: 'key', value: p.key, readonly: true },
        { name: 'desc', label: '描述', hint: '选填', value: p.desc },
        { name: 'color', label: '颜色', type: 'color', value: p.color },
      ],
      onSubmit: v => { try { store.updateProject(pk, { name: v.name, desc: v.desc, color: v.color }); toast({ type: 'success', title: '已保存', duration: 1400 }); } catch (err) { retry(cfg, v, err); } },
    };
    openForm(cfg);
  }
  function editBoard(path) {
    const b = store.boardMetaOf(path); if (!b) return;
    const cfg = {
      title: '编辑白板', submit: '保存',
      preview: v => `<span class="pj-bic lg" style="--pc:${b.color}">${icons.board}</span><div><b>${esc(b.projectName)} / ${esc(v.name || b.name)}</b><span class="mono">BOARD=${esc(b.path)}</span></div>`,
      fields: [
        { name: 'name', label: '白板名称', value: b.name, required: true },
        { name: 'key', label: '白板标识', hint: 'agent 用它定位，创建后不能修改', type: 'key', value: b.key, readonly: true },
        { name: 'desc', label: '描述', hint: '选填', value: b.desc },
      ],
      onSubmit: v => { try { store.updateBoard(path, { name: v.name, desc: v.desc }); toast({ type: 'success', title: '已保存', duration: 1400 }); } catch (err) { retry(cfg, v, err); } },
    };
    openForm(cfg);
  }
  async function deleteBoard(path) {
    const b = store.boardMetaOf(path); if (!b) return;
    const total = Object.values(b.counts).reduce((a, x) => a + x, 0);
    if (!await B.ui.confirm({ title: `删除白板「${b.name}」？`, text: `${total} 张卡片和全部动态会被永久删除，无法恢复。使用 BOARD=${b.path} 的 agent 会收到 BOARD_NOT_FOUND。`, ok: '删除白板' })) return;
    try { store.deleteBoard(path); toast({ type: 'success', title: `已删除白板「${b.name}」` }); } catch (err) { toastError(err); }
  }
  async function deleteProject(pk) {
    const p = store.projectMeta(pk); if (!p) return;
    const total = Object.values(p.counts).reduce((a, x) => a + x, 0);
    if (!await B.ui.confirm({ title: `删除项目「${p.name}」？`, text: `项目下的 ${p.boards.length} 块白板、${total} 张卡片会被永久删除，无法恢复。`, ok: '删除项目' })) return;
    try { store.deleteProject(pk); toast({ type: 'success', title: `已删除项目「${p.name}」` }); } catch (err) { toastError(err); }
  }

  /* ============================ 绑定 ============================ */
  function bindSwitch() {
    $('#projectSwitch').addEventListener('click', () => (menuOpen() ? closeMenu() : openMenu()));
    $('#projectSearch').addEventListener('input', () => { active = 0; renderMenu(); });
    $('#projectSearch').addEventListener('keydown', e => {
      if (e.key === 'ArrowDown') { e.preventDefault(); setActive(active + 1); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); setActive(active - 1); }
      else if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); if (items[active]) openBoard(items[active].path); }
      else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeMenu(); $('#projectSwitch').focus(); }
    });
    $('#projectList').addEventListener('click', e => { const r = e.target.closest('.pj-row'); if (r) openBoard(r.dataset.board); });
    $('#projectList').addEventListener('pointermove', e => { const r = e.target.closest('.pj-row'); if (r && Number(r.dataset.i) !== active) setActive(Number(r.dataset.i)); });
    $('#projectNew').onclick = () => newBoard(store.projectKey);
    $('#projectManage').onclick = () => { closeMenu(); B.workbench.setTab('projects'); };
    document.addEventListener('pointerdown', e => { if (menuOpen() && !e.target.closest('#projectMenu, #projectSwitch')) closeMenu(); });
    window.addEventListener('resize', closeMenu);
    $$('.crumb-link[data-goto]').forEach(b => (b.onclick = () => B.workbench.setTab(b.dataset.goto)));
  }

  B.projects = {
    init(s) {
      store = s;
      bindSwitch(); bindTree(); bindCtx(); bindForm();
      renderSwitch(); renderTree();
      store.subscribe((_, meta) => {
        if (meta.source === 'layout') return;
        renderSwitch(); renderTree();
        if (menuOpen()) renderMenu();
      });
    },
    openMenu, closeMenu, openBoard, newProject, newBoard, editProject, editBoard, contextMenu, openForm,
    get menuOpen() { return menuOpen(); },
  };
})(window.Board = window.Board || {});
