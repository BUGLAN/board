/* ==========================================================================
   Board · 标签
   - 标签属于项目，项目下所有白板共用；卡片可以有多个标签
   - picker(anchor, opts)：选择 / 搜索 / 直接新建标签的浮层（详情、新建弹窗共用）
   - manage(pk)：标签管理弹窗（改名、改色、看使用数、删除、新建）
   - chips(ids)：渲染卡片上的标签
   ========================================================================== */
(function (B) {
  'use strict';
  const { $, $$, el, esc, icons, toast, toastError } = B.ui;

  let store;

  /* ------------------------------ 渲染 ------------------------------ */
  const byId = (pk = store.projectKey) => Object.fromEntries(store.labelsOf(pk).map(l => [l.id, l]));
  function chip(l, { removable = false } = {}) {
    return `<span class="lb" style="--lc:${l.color}" data-label="${esc(l.id)}"><i></i>${esc(l.name)}${removable ? `<button type="button" class="lb-x" data-remove="${esc(l.id)}" aria-label="移除标签 ${esc(l.name)}">×</button>` : ''}</span>`;
  }
  /** 卡片上的标签：最多显示 max 个，其余折叠成 +n */
  function chips(ids, { max = 3, pk } = {}) {
    const map = byId(pk);
    const ls = (ids || []).map(id => map[id]).filter(Boolean);
    if (!ls.length) return '';
    const shown = ls.slice(0, max).map(l => chip(l)).join('');
    const more = ls.length > max ? `<span class="lb more" title="${esc(ls.slice(max).map(l => l.name).join('、'))}">+${ls.length - max}</span>` : '';
    return shown + more;
  }

  /* ------------------------------ 选择器 ------------------------------ */
  let picker = null; // { el, opts, active }
  function ensurePicker() {
    let p = $('#labelPicker');
    if (p) return p;
    p = el(`<div class="lb-picker" id="labelPicker" role="dialog" aria-label="选择标签">
      <div class="lbp-search">${icons.tag}<input placeholder="搜索或新建标签…" spellcheck="false" autocomplete="off"></div>
      <div class="lbp-list scroll" role="listbox"></div>
      <div class="lbp-foot"><button type="button" data-manage>${icons.edit}管理标签</button></div>
    </div>`);
    document.body.appendChild(p);
    const input = p.querySelector('input');
    input.addEventListener('input', () => { picker.active = 0; renderPicker(); });
    input.addEventListener('keydown', e => {
      const rows = $$('#labelPicker .lbp-row');
      if (e.key === 'ArrowDown') { e.preventDefault(); picker.active = Math.min(rows.length - 1, picker.active + 1); renderPicker(); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); picker.active = Math.max(0, picker.active - 1); renderPicker(); }
      else if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); rows[picker.active]?.click(); }
      else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closePicker(); }
    });
    p.querySelector('.lbp-list').addEventListener('click', e => {
      const r = e.target.closest('.lbp-row'); if (!r) return;
      if (r.dataset.create != null) {
        try {
          const l = store.createLabel(picker.opts.pk, { name: r.dataset.create });
          picker.opts.onToggle(l.id, true);
          p.querySelector('input').value = '';
        } catch (err) { toastError(err); }
      } else {
        const id = r.dataset.id;
        picker.opts.onToggle(id, !picker.opts.selected().includes(id));
      }
      renderPicker();
      p.querySelector('input').focus();
    });
    p.querySelector('[data-manage]').onclick = () => { const pk = picker.opts.pk; closePicker(); manage(pk); };
    document.addEventListener('pointerdown', e => { if (picker && !e.target.closest('#labelPicker, [data-label-picker]')) closePicker(); });
    return p;
  }
  /**
   * @param anchor 触发元素
   * @param opts { pk, selected: () => ids, onToggle(id, on) }
   */
  function openPicker(anchor, opts) {
    const p = ensurePicker();
    picker = { el: p, opts: { pk: store.projectKey, ...opts }, active: 0 };
    p.querySelector('input').value = '';
    renderPicker();
    p.classList.add('on');
    const r = anchor.getBoundingClientRect();
    const w = 260, h = p.offsetHeight || 300;
    p.style.left = `${Math.max(8, Math.min(r.left, innerWidth - w - 8))}px`;
    p.style.top = `${r.bottom + h + 8 > innerHeight ? Math.max(8, r.top - h - 6) : r.bottom + 6}px`;
    setTimeout(() => p.querySelector('input').focus(), 20);
  }
  function closePicker() { if (!picker) return; picker.el.classList.remove('on'); picker = null; }
  function renderPicker() {
    if (!picker) return;
    const q = picker.el.querySelector('input').value.trim();
    const sel = new Set(picker.opts.selected());
    const all = store.labelsOf(picker.opts.pk);
    const list = all.filter(l => !q || l.name.toLowerCase().includes(q.toLowerCase()));
    const exact = all.some(l => l.name.toLowerCase() === q.toLowerCase());
    let html = list.map((l, i) => `
      <button type="button" class="lbp-row ${i === picker.active ? 'active' : ''}" data-id="${esc(l.id)}" role="option" aria-selected="${sel.has(l.id)}">
        <span class="lbp-check ${sel.has(l.id) ? 'on' : ''}">${icons.check}</span>
        <span class="lb" style="--lc:${l.color}"><i></i>${esc(l.name)}</span>
      </button>`).join('');
    if (q && !exact) html += `<button type="button" class="lbp-row create ${list.length === picker.active ? 'active' : ''}" data-create="${esc(q)}">${icons.plus}新建标签「${esc(q)}」</button>`;
    picker.el.querySelector('.lbp-list').innerHTML = html || '<div class="lbp-empty">还没有标签，输入名称即可新建</div>';
  }

  /* ------------------------------ 管理弹窗 ------------------------------ */
  let managing = null;
  function ensureManage() {
    let w = $('#labelsModal');
    if (w) return w;
    w = el(`<div class="dialog-wrap" id="labelsModal" aria-hidden="true">
      <div class="dialog lbm" role="dialog" aria-modal="true" aria-labelledby="lbmTitle">
        <div class="lbm-head"><h3 id="lbmTitle">管理标签</h3><span class="lbm-sub"></span><button type="button" class="icon-btn" data-close aria-label="关闭">${icons.close}</button></div>
        <p class="lbm-p">标签属于项目，项目下的所有白板共用。改名或改色会立即生效；删除会从所有卡片上移除。agent 用 <code>board label &lt;ID&gt; +名称</code> 打标签。</p>
        <div class="lbm-list scroll"></div>
        <form class="lbm-new" novalidate>
          <button type="button" class="lbm-color" data-new-color aria-label="选择颜色"></button>
          <input class="input" placeholder="新标签名称" maxlength="24" autocomplete="off" spellcheck="false">
          <button type="submit" class="btn sm primary">${icons.plus}新建</button>
        </form>
      </div>
    </div>`);
    document.body.appendChild(w);
    w.addEventListener('click', async e => {
      if (e.target === w || e.target.closest('[data-close]')) return closeManage();
      const sw = e.target.closest('.lbm-swatches .swatch');
      if (sw) {
        const id = sw.closest('[data-row]')?.dataset.row;
        if (id) store.updateLabel(managing, id, { color: sw.dataset.color });
        else { w.dataset.newColor = sw.dataset.color; }
        closeSwatches(); renderManage(); return;
      }
      const c = e.target.closest('.lbm-color');
      if (c) { e.stopPropagation(); openSwatches(c); return; }
      const del = e.target.closest('[data-del]');
      if (del) {
        const l = store.labelsOf(managing).find(x => x.id === del.dataset.del);
        const n = store.labelUsage(managing)[l.id] || 0;
        if (n && !await B.ui.confirm({ title: `删除标签「${l.name}」？`, text: `${n} 张卡片上的这个标签会一起移除。`, ok: '删除标签', icon: icons.tag })) return;
        store.deleteLabel(managing, l.id);
        renderManage();
        return;
      }
      if (!e.target.closest('.lbm-swatches')) closeSwatches();
    });
    w.addEventListener('change', e => {
      const inp = e.target.closest('[data-rename]'); if (!inp) return;
      try { store.updateLabel(managing, inp.dataset.rename, { name: inp.value }); }
      catch (err) { toastError(err); renderManage(); }
    });
    w.addEventListener('keydown', e => {
      if (e.key === 'Enter' && e.target.closest('[data-rename]')) { e.preventDefault(); e.target.blur(); }
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); if ($('#labelsModal .lbm-swatches')) closeSwatches(); else closeManage(); }
    });
    w.querySelector('.lbm-new').addEventListener('submit', e => {
      e.preventDefault();
      const inp = w.querySelector('.lbm-new input');
      try {
        store.createLabel(managing, { name: inp.value, color: w.dataset.newColor });
        inp.value = ''; delete w.dataset.newColor; renderManage();
        const list = w.querySelector('.lbm-list'); list.scrollTop = list.scrollHeight;
        list.lastElementChild?.classList.add('new');
      } catch (err) {
        inp.classList.remove('invalid'); void inp.offsetWidth; inp.classList.add('invalid');
        if (err.code !== 'EMPTY_TITLE') toastError(err);
      }
      inp.focus();
    });
    return w;
  }
  function openSwatches(btn) {
    closeSwatches();
    const pop = el(`<div class="lbm-swatches">${B.Store.LABEL_COLORS.map(c => `<button type="button" class="swatch" data-color="${c}" style="--sw:${c}" aria-label="${c}"></button>`).join('')}</div>`);
    btn.parentElement.appendChild(pop);
  }
  function closeSwatches() { $$('#labelsModal .lbm-swatches').forEach(n => n.remove()); }

  function manage(pk = store.projectKey) {
    managing = pk;
    const w = ensureManage();
    renderManage();
    w.classList.add('on'); w.setAttribute('aria-hidden', 'false');
    setTimeout(() => w.querySelector('.lbm-new input').focus(), 40);
  }
  function closeManage() { const w = $('#labelsModal'); if (!w) return; w.classList.remove('on'); w.setAttribute('aria-hidden', 'true'); managing = null; }
  function renderManage() {
    const w = $('#labelsModal'); if (!w || !managing) return;
    const p = store.projectMeta(managing); if (!p) return closeManage();
    const used = store.labelUsage(managing);
    const ls = store.labelsOf(managing);
    w.querySelector('.lbm-sub').innerHTML = `${esc(p.name)} · ${ls.length} 个标签`;
    w.querySelector('.lbm-list').innerHTML = ls.length ? ls.map(l => `
      <div class="lbm-row" data-row="${esc(l.id)}" style="--lc:${l.color}">
        <button type="button" class="lbm-color" aria-label="改颜色"></button>
        <input class="lbm-name" data-rename="${esc(l.id)}" value="${esc(l.name)}" maxlength="24" spellcheck="false" aria-label="标签名称">
        <span class="lbm-preview">${chip(l)}</span>
        <span class="lbm-count">${used[l.id] || 0} 张卡片</span>
        <button type="button" class="icon-btn sm lbm-del" data-del="${esc(l.id)}" aria-label="删除">${icons.trash}</button>
      </div>`).join('') : '<div class="lbp-empty">这个项目还没有标签</div>';
    const nc = w.dataset.newColor || B.Store.LABEL_COLORS[ls.length % B.Store.LABEL_COLORS.length];
    w.querySelector('[data-new-color]').style.setProperty('--lc', nc);
    w.dataset.newColor = nc;
  }

  B.labels = {
    init(s) {
      store = s;
      store.subscribe((_, meta) => { if (meta.structure || meta.labels) { renderManage(); renderPicker(); } });
    },
    chip, chips, byId, openPicker, closePicker, manage,
    get pickerOpen() { return !!picker; },
  };
})(window.Board = window.Board || {});
