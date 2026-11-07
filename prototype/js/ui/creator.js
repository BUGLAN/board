/* ==========================================================================
   Board · 新建（统一入口）
   白板「新建卡片」与侧栏「未整理」的新建都打开这个居中弹窗：
   - 尺寸与卡片详情一致（固定宽高），左边写标题和描述，右边选「放到哪里」
   - ↵ 在标题里直接创建；⌘/Ctrl ↵ 在任何位置创建；Tab 去写描述
   - 「连续新建」打开时，创建后清空表单继续录入
   - 关闭时未提交的内容保留为草稿，下次打开自动恢复
   ========================================================================== */
(function (B) {
  'use strict';
  const { HUMAN, COLUMN_NAME } = B.schema;
  const { $, $$, el, esc, icons, markdown, autosize, toast, toastError, isMobile, reduceMotion, bus } = B.ui;
  const DRAFT_KEY = 'board.creator.draft', KEEP_KEY = 'board.creator.keep';

  let store;
  let column = 'todo';
  let mode = 'edit';
  let created = [];
  let labels = [];

  const get = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } };
  const set = (k, v) => { try { v == null ? localStorage.removeItem(k) : localStorage.setItem(k, JSON.stringify(v)); } catch {} };

  const root = () => $('#creator');
  const titleEl = () => $('#creatorTitle');
  const bodyEl = () => $('#creatorBody');

  /* ------------------------------ 打开 / 关闭 ------------------------------ */
  function open(opts = {}) {
    if (B.drawer.openId != null) B.drawer.close();
    const draft = get(DRAFT_KEY, null);
    const hasDraft = draft && (draft.title || draft.body);
    column = opts.column || (hasDraft && draft.column) || (B.workbench?.tab === 'inbox' ? 'inbox' : 'todo');
    titleEl().value = hasDraft ? draft.title : '';
    bodyEl().value = hasDraft ? draft.body : '';
    $('#crDraft').hidden = !hasDraft;
    created = [];
    labels = hasDraft && draft.labels ? draft.labels.filter(id => B.labels.byId()[id]) : [];
    renderLabels();
    renderRecent();
    setMode('edit');
    setColumn(column, false);
    syncProject();
    clearError();
    const r = root();
    r.classList.add('on'); r.setAttribute('aria-hidden', 'false');
    $('#scrim').classList.add('on');
    requestAnimationFrame(() => { autosize(titleEl()); syncCli(); titleEl().focus(); titleEl().setSelectionRange(titleEl().value.length, titleEl().value.length); });
  }

  function close() {
    const r = root();
    if (!r.classList.contains('on')) return;
    saveDraft();
    r.classList.remove('on'); r.setAttribute('aria-hidden', 'true');
    if (B.drawer.openId == null) $('#scrim').classList.remove('on');
  }
  const isOpen = () => root().classList.contains('on');

  function saveDraft() {
    const t = titleEl().value.trim(), b = bodyEl().value.trim();
    set(DRAFT_KEY, t || b ? { title: titleEl().value, body: bodyEl().value, column, labels } : null);
  }

  /* ------------------------------ 表单状态 ------------------------------ */
  function setColumn(col, animate = true) {
    column = col;
    $$('#creator .dest').forEach(d => {
      const on = d.dataset.col === col;
      d.classList.toggle('on', on);
      d.querySelector('input').checked = on;
    });
    const r = root();
    r.dataset.dest = col;
    $('#crSubmitLabel').textContent = col === 'inbox' ? '记到未整理' : '创建到待办';
    $('#crHeadLabel').textContent = col === 'inbox' ? '记一条未整理' : '新建卡片';
    if (animate && !reduceMotion()) $('#crHeadBadge').animate([{ transform: 'scale(.9)' }, { transform: 'none' }], { duration: 300, easing: 'cubic-bezier(.34,1.56,.64,1)' });
    syncCli();
  }

  function setMode(m) {
    mode = m;
    $$('#creator .desc-tabs button').forEach(b => b.classList.toggle('on', b.dataset.mode === m));
    const on = $('#creator .desc-tabs button.on'), thumb = $('#creator .desc-tabs .thumb');
    if (on && thumb) requestAnimationFrame(() => { thumb.style.width = `${on.offsetWidth}px`; thumb.style.transform = `translateX(${on.offsetLeft - 2}px)`; });
    const prev = $('#crPreview');
    bodyEl().hidden = m !== 'edit';
    prev.hidden = m !== 'preview';
    if (m === 'preview') prev.innerHTML = bodyEl().value.trim() ? markdown(bodyEl().value) : '<p class="cr-empty">还没有描述</p>';
    else bodyEl().focus();
  }

  function syncProject() {
    const b = store.boardMeta;
    const n = $('#crProject');
    n.innerHTML = `<i style="background:${b.color}"></i>${esc(b.projectName)} / ${esc(b.name)}`;
  }

  function syncCli() {
    const t = titleEl().value.trim() || '<标题>';
    const b = bodyEl().value.trim();
    const q = s => `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n')}"`;
    const short = b.length > 40 ? b.slice(0, 40) + '…' : b;
    $('#crCli').innerHTML = column === 'inbox'
      ? `<span class="dim">未整理只在界面上记录，整理后再放进待办</span>`
      : `<span class="cmd">board</span> new ${esc(q(t))}${b ? ` --body ${esc(q(short))}` : ''} <span class="dim">--board</span> ${esc(store.current)}`;
    root().classList.toggle('has-title', !!titleEl().value.trim());
  }

  function showError(msg) {
    const e = $('#crError');
    e.textContent = msg; e.hidden = false;
    const t = titleEl();
    t.classList.remove('invalid'); void t.offsetWidth; t.classList.add('invalid');
    t.focus();
  }
  function clearError() { $('#crError').hidden = true; titleEl().classList.remove('invalid'); }

  /* ------------------------------ 提交 ------------------------------ */
  function submit() {
    const r = store.try({ op: 'new', actor: HUMAN, column, title: titleEl().value, body: bodyEl().value, labels });
    if (!r.ok) {
      if (r.error.code === 'EMPTY_TITLE') return showError('标题是必填的');
      return toastError(r.error);
    }
    const c = r.result;
    set(DRAFT_KEY, null);
    $('#crDraft').hidden = true;
    const keep = $('#crKeep').classList.contains('on');

    if (keep) {
      created.unshift({ id: c.id, title: c.title, column });  // 标签保留，方便连续录入同一类卡片
      renderRecent(true);
      titleEl().value = ''; bodyEl().value = '';
      autosize(titleEl()); setMode('edit'); syncCli(); clearError();
      titleEl().focus();
      const btn = $('#crSubmit');
      btn.animate([{ transform: 'scale(.94)' }, { transform: 'scale(1.03)' }, { transform: 'none' }], { duration: 360, easing: 'cubic-bezier(.34,1.56,.64,1)' });
      return;
    }

    close();
    if (column === 'inbox') {
      if (B.workbench.tab !== 'inbox') {
        toast({ type: 'success', title: `已记到「未整理」 #${c.id}`, text: c.title, action: { label: '查看', fn: () => { B.workbench.setTab('inbox'); setTimeout(() => B.boardView.reveal(c.id), 120); } } });
      } else setTimeout(() => B.boardView.reveal(c.id), 80);
    } else {
      if (B.workbench.tab === 'settings') B.workbench.setTab('board');
      setTimeout(() => B.boardView.reveal(c.id), 80);
    }
  }

  function renderLabels() {
    const map = B.labels.byId();
    const ls = labels.map(id => map[id]).filter(Boolean);
    $('#crLabels').innerHTML = ls.map(l => B.labels.chip(l, { removable: true })).join('') +
      `<button type="button" class="lb-add" data-label-picker>${icons.tag}${ls.length ? '' : '添加标签'}</button>`;
  }

  function renderRecent(animate) {
    const wrap = $('#crRecent');
    wrap.hidden = !created.length;
    $('#crRecentCount').textContent = created.length;
    wrap.querySelector('.cr-recent-list').innerHTML = created.map((c, i) => `
      <button type="button" class="cr-recent-row ${animate && i === 0 ? 'new' : ''}" data-id="${c.id}" style="--c:var(--c-${c.column})">
        <i></i><span class="mono">#${c.id}</span><span class="tt">${esc(c.title)}</span><em>${COLUMN_NAME[c.column]}</em>
      </button>`).join('');
  }

  /* ------------------------------ 绑定 ------------------------------ */
  function bind() {
    const r = root(), t = titleEl(), b = bodyEl();
    const mod = B.ui.isMac ? '⌘' : 'Ctrl';
    $$('#creator .mod').forEach(n => (n.textContent = mod));

    t.addEventListener('input', () => { autosize(t); syncCli(); if (t.value.trim()) clearError(); });
    b.addEventListener('input', syncCli);
    t.addEventListener('keydown', e => {
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); submit(); }
      if (e.key === 'Enter' && e.shiftKey) { e.preventDefault(); b.focus(); }
    });
    // 挂在 document（捕获阶段）：焦点即使不在弹窗里，快捷键也有效
    document.addEventListener('keydown', e => {
      if (!isOpen()) return;
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); e.stopPropagation(); submit(); }
      if (e.key === 'Escape') { if (B.labels.pickerOpen) return; e.preventDefault(); e.stopPropagation(); close(); }
    }, true);
    r.addEventListener('keydown', e => {
      // 焦点留在弹窗内
      if (e.key === 'Tab') {
        const f = $$('#creator button:not([disabled]), #creator textarea:not([hidden]), #creator input:not([type="radio"])').filter(n => n.offsetParent !== null);
        if (!f.length) return;
        const first = f[0], last = f[f.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    });
    $('#creatorForm').addEventListener('submit', e => { e.preventDefault(); submit(); });
    r.querySelector('[data-act="close"]').onclick = close;
    r.querySelector('[data-act="cancel"]').onclick = close;
    r.querySelectorAll('.desc-tabs button').forEach(x => (x.onclick = () => setMode(x.dataset.mode)));
    r.querySelectorAll('.dest').forEach(d => d.addEventListener('click', e => { e.preventDefault(); setColumn(d.dataset.col); titleEl().focus(); }));
    r.querySelector('[data-act="template"]').onclick = () => {
      setMode('edit');
      const tpl = '**验收标准**\n- [ ] \n\n**相关链接**\n- ';
      const v = b.value;
      b.value = v + (v && !v.endsWith('\n') ? '\n\n' : '') + tpl;
      const pos = b.value.indexOf('- [ ] ', v.length) + 6;
      b.focus(); b.setSelectionRange(pos, pos); syncCli();
    };
    $('#crDraftClear').onclick = () => { t.value = ''; b.value = ''; set(DRAFT_KEY, null); $('#crDraft').hidden = true; autosize(t); syncCli(); t.focus(); };
    const keep = $('#crKeep');
    const applyKeep = on => { keep.classList.toggle('on', on); keep.setAttribute('aria-checked', on); };
    applyKeep(get(KEEP_KEY, false));
    keep.onclick = () => { const on = !keep.classList.contains('on'); applyKeep(on); set(KEEP_KEY, on); };
    $('#crRecent').addEventListener('click', e => {
      const row = e.target.closest('[data-id]'); if (!row) return;
      const id = Number(row.dataset.id);
      close();
      const c = store.state.cards[id]; if (!c) return;
      if (c.column === 'inbox') B.workbench.setTab('inbox');
      setTimeout(() => { B.boardView.reveal(id); bus.emit('card:open', id); }, 120);
    });
    $('#scrim').addEventListener('click', close);
    $('#crLabels').addEventListener('click', e => {
      const x = e.target.closest('[data-remove]');
      if (x) { labels = labels.filter(id => id !== x.dataset.remove); return renderLabels(); }
      const add = e.target.closest('[data-label-picker]');
      if (add) B.labels.openPicker(add, { selected: () => labels, onToggle: (id, on) => { labels = on ? [...new Set([...labels, id])] : labels.filter(x => x !== id); renderLabels(); } });
    });
    window.addEventListener('beforeunload', () => { if (isOpen()) saveDraft(); });
  }

  B.creator = {
    init(s) {
      store = s; bind();
      store.subscribe((_, meta) => { if (meta.source === 'switch') close(); if (isOpen()) syncProject(); });
    },
    open, close, isOpen,
  };
  B.ui.openComposer = opts => open(opts);
})(window.Board = window.Board || {});
