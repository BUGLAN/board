/* ==========================================================================
   Board · 卡片详情（桌面：居中固定尺寸弹窗，左内容 / 右动态；手机：可下拉关闭的底部 sheet）
   编辑标题和描述、查看动态、写评论；进行中 → 强制释放；所有卡片 → 删除（二次确认）
   ========================================================================== */
(function (B) {
  'use strict';
  const { schema } = B;
  const { HUMAN, COLUMN_NAME, CLAIM_MS } = schema;
  const { $, el, esc, icons, avatar, agentColor, hhmm, mmss, ago, markdown, ring, setRing, autosize, toast, toastError, isMobile, bus } = B.ui;

  let store;
  let openId = null;
  let descMode = 'view';
  let renderedActivity = 0;
  let lastSig = '';

  /* ---------------------------- 确认对话框 ---------------------------- */
  function confirmDialog({ title, text, ok = '确定', icon = icons.trash }) {
    return new Promise(resolve => {
      const wrap = $('#dialog');
      $('#dialogTitle').textContent = title;
      $('#dialogText').textContent = text;
      $('#dialogIcon').innerHTML = icon;
      $('#dialogOk').textContent = ok;
      wrap.classList.add('on'); wrap.setAttribute('aria-hidden', 'false');
      const prev = document.activeElement;
      setTimeout(() => $('#dialogCancel').focus(), 40);
      const done = v => {
        wrap.classList.remove('on'); wrap.setAttribute('aria-hidden', 'true');
        $('#dialogOk').onclick = $('#dialogCancel').onclick = wrap.onclick = null;
        document.removeEventListener('keydown', key, true);
        prev?.focus?.();
        resolve(v);
      };
      const key = e => {
        if (e.key === 'Escape') { e.stopPropagation(); e.preventDefault(); done(false); }
        if (e.key === 'Tab') { e.preventDefault(); (document.activeElement === $('#dialogOk') ? $('#dialogCancel') : $('#dialogOk')).focus(); }
      };
      document.addEventListener('keydown', key, true);
      $('#dialogOk').onclick = () => done(true);
      $('#dialogCancel').onclick = () => done(false);
      wrap.onclick = e => { if (e.target === wrap) done(false); };
    });
  }

  /* ------------------------------ 打开 / 关闭 ------------------------------ */
  function open(id) {
    const c = store.state.cards[id];
    if (!c) return;
    const switching = openId != null && openId !== id;
    openId = id;
    descMode = 'view';
    renderedActivity = 0;
    lastSig = '';
    const body = $('#drawerBody');
    body.innerHTML = skeleton(c);
    body.scrollTop = 0;
    bindBody(body);
    update(true);
    const act = body.querySelector('.d-activity');
    if (act) act.scrollTop = act.scrollHeight;   // 动态默认停在最新
    const drawer = $('#drawer');
    drawer.style.removeProperty('--sheet-y');
    drawer.classList.add('on'); drawer.setAttribute('aria-hidden', 'false');
    $('#scrim').classList.add('on');
    B.boardView.setOpen(id);
    if (switching) body.animate([{ transform: `translateY(${navDir * 10}px)`, opacity: .3 }, { transform: 'none', opacity: 1 }], { duration: 280, easing: 'cubic-bezier(.16,1,.3,1)' });
    navDir = 0;
    B.boardView.cardEl(id)?.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'smooth' });
    setTimeout(() => body.querySelector('.d-head [data-act="close"]')?.focus({ preventScroll: true }), 60);
  }

  function close() {
    if (openId == null) return;
    const drawer = $('#drawer');
    drawer.classList.remove('on'); drawer.setAttribute('aria-hidden', 'true');
    drawer.style.removeProperty('--sheet-y');
    $('#scrim').classList.remove('on');
    const prev = openId;
    openId = null;
    B.boardView.setOpen(null);
    B.boardView.cardEl(prev)?.focus({ preventScroll: true });
  }

  /* ------------------------------ 结构 ------------------------------ */
  function skeleton(c) {
    const mod = B.ui.isMac ? '⌘' : 'Ctrl';
    return `
      <div class="d-head">
        <span class="d-id">#${c.id}</span>
        <span class="d-col"><i></i><span></span></span>
        <span class="d-nav">
          <button class="icon-btn sm" type="button" data-act="prev" data-tip="上一张  ↑ / K">${icons.chevUp}</button>
          <span class="d-pos mono"></span>
          <button class="icon-btn sm" type="button" data-act="next" data-tip="下一张  ↓ / J">${icons.chevDown}</button>
        </span>
        <span class="spacer"></span>
        <button class="btn sm ghost d-prompt" type="button" data-act="prompt" data-tip="复制为给 agent 的提示词  C" data-tip-align="end">${icons.prompt}复制为提示词</button>
        <button class="icon-btn" type="button" data-act="copy" data-tip="复制 board show ${c.id}" data-tip-align="end">${icons.copy}</button>
        <button class="icon-btn" type="button" data-act="close" data-tip="关闭  esc" data-tip-align="end">${icons.close}</button>
      </div>
      <div class="d-grid">
        <div class="d-main scroll">
          <section class="d-section" style="--i:0">
            <textarea class="d-title" id="drawerTitle" rows="1" maxlength="200" spellcheck="false"></textarea>
            <div class="d-meta"></div>
            <div class="d-labels" id="dLabels"></div>
          </section>
          <section class="d-section" data-part="claim" style="--i:1"></section>
          <section class="d-section" data-part="desc" style="--i:2">
            <div class="d-label">描述
              <div class="act desc-tabs"><span class="thumb"></span><button type="button" data-mode="view">预览</button><button type="button" data-mode="edit">编辑</button></div>
            </div>
            <div class="desc-holder"></div>
          </section>
          <div class="d-danger">
            <span>删除后无法恢复</span>
            <button class="btn sm danger" type="button" data-act="delete">${icons.trash}删除卡片</button>
          </div>
        </div>
        <div class="d-side" data-part="activity">
          <div class="d-label d-side-head">动态 <span class="count"></span><span class="act">只能追加</span></div>
          <div class="d-activity scroll"><div class="timeline"></div></div>
          <div class="d-comment">
            <div class="comment-box">
              <textarea rows="1" placeholder="写评论…  ${mod}↵ 发送" aria-label="评论"></textarea>
              <button class="comment-send" type="button" aria-label="发送评论">${icons.send}</button>
            </div>
          </div>
        </div>
      </div>`;
  }

  function bindBody(body) {
    body.querySelector('[data-act="close"]').onclick = close;
    body.querySelector('[data-act="prompt"]').onclick = () => B.prompt.copyCard(openId);
    body.querySelector('#dLabels').addEventListener('click', e => {
      const x = e.target.closest('[data-remove]');
      if (x) { const r = store.try({ op: 'label', actor: HUMAN, id: openId, remove: [x.dataset.remove], names: Object.fromEntries(store.labelsOf().map(l => [l.id, l.name])) }); if (!r.ok) toastError(r.error); return; }
      const add = e.target.closest('[data-label-picker]');
      if (add) B.labels.openPicker(add, {
        selected: () => store.state.cards[openId]?.labels || [],
        onToggle: (id, on) => { try { store.setLabels(openId, on ? { add: [id] } : { remove: [id] }); } catch (err) { toastError(err); } },
      });
    });
    body.querySelector('[data-act="prev"]').onclick = () => step(-1);
    body.querySelector('[data-act="next"]').onclick = () => step(1);
    body.querySelector('[data-act="copy"]').onclick = async () => {
      try { await navigator.clipboard.writeText(`board show ${openId}`); toast({ type: 'success', title: '已复制', text: `board show ${openId}`, duration: 1800 }); }
      catch { toast({ type: 'info', title: `board show ${openId}`, duration: 2400 }); }
    };
    body.querySelector('[data-act="delete"]').onclick = async () => {
      const c = store.state.cards[openId]; if (!c) return;
      const yes = await confirmDialog({ title: `删除 #${c.id}？`, text: `「${c.title}」及其全部动态会被永久删除，无法恢复。`, ok: '删除' });
      if (!yes) return;
      const id = c.id;
      close();
      const r = store.try({ op: 'delete', actor: HUMAN, id });
      if (r.ok) toast({ type: 'success', title: `已删除 #${id}`, text: c.title });
      else toastError(r.error);
    };

    // 标题
    const title = body.querySelector('.d-title');
    title.addEventListener('input', () => autosize(title));
    title.addEventListener('keydown', e => {
      if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); title.blur(); }
      if (e.key === 'Escape') { e.stopPropagation(); title.value = store.state.cards[openId]?.title || ''; title.blur(); }
    });
    title.addEventListener('blur', () => {
      const c = store.state.cards[openId]; if (!c || title.readOnly) return;
      if (title.value.trim() === c.title) return;
      const r = store.try({ op: 'edit', actor: HUMAN, id: c.id, title: title.value });
      if (!r.ok) {
        toastError(r.error);
        title.classList.remove('invalid'); void title.offsetWidth; title.classList.add('invalid');
        title.value = c.title; autosize(title);
      }
    });

    // 描述 tabs
    body.querySelectorAll('.desc-tabs button').forEach(b => (b.onclick = () => setDescMode(b.dataset.mode)));

    // 评论
    const box = body.querySelector('.comment-box');
    const ta = box.querySelector('textarea');
    const send = () => {
      const text = ta.value.trim();
      if (!text) { ta.focus(); return; }
      const r = store.try({ op: 'comment', actor: HUMAN, id: openId, text });
      if (!r.ok) return toastError(r.error);
      ta.value = ''; autosize(ta); box.classList.remove('has-text');
      const btn = box.querySelector('.comment-send');
      btn.animate([{ transform: 'rotate(-12deg)' }, { transform: 'translate(18px,-18px) rotate(-12deg)', opacity: 0 }, { transform: 'translate(-10px, 10px)', opacity: 0 }, { transform: 'none', opacity: 1 }], { duration: 520, easing: 'cubic-bezier(.16,1,.3,1)' });
      const act = body.querySelector('.d-activity');
      requestAnimationFrame(() => { act.scrollTo({ top: act.scrollHeight, behavior: 'smooth' }); if (B.ui.isMobile()) body.scrollTo({ top: body.scrollHeight, behavior: 'smooth' }); });
    };
    ta.addEventListener('input', () => { autosize(ta); box.classList.toggle('has-text', !!ta.value.trim()); });
    ta.addEventListener('keydown', e => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); send(); } if (e.key === 'Escape') { e.stopPropagation(); ta.blur(); } });
    box.querySelector('.comment-send').onclick = send;
  }

  function setDescMode(mode) {
    const c = store.state.cards[openId]; if (!c) return;
    if (mode === 'edit' && locked(c)) return;
    descMode = mode;
    renderDesc(c, true);
    if (mode === 'edit') {
      const ta = $('#drawerBody .desc-edit textarea');
      if (ta) { ta.focus(); ta.setSelectionRange(ta.value.length, ta.value.length); }
    }
  }

  const locked = c => c.column === 'doing';

  /* 同一列内上一张 / 下一张 */
  let navDir = 0;
  function siblings() {
    const c = store.state.cards[openId]; if (!c) return { list: [], i: -1 };
    const list = store.state.cols[c.column] || [];
    return { list, i: list.indexOf(c.id) };
  }
  function step(d) {
    const { list, i } = siblings();
    const next = list[i + d];
    if (next == null) {
      $('#drawer .d-nav')?.animate([{ transform: 'none' }, { transform: `translateY(${d * 3}px)` }, { transform: 'none' }], { duration: 220 });
      return;
    }
    navDir = d;
    document.activeElement?.blur?.();   // 先提交正在编辑的标题
    open(next);
  }
  function renderNav() {
    const { list, i } = siblings();
    const body = $('#drawerBody');
    body.querySelector('.d-pos').textContent = list.length ? `${i + 1} / ${list.length}` : '';
    body.querySelector('[data-act="prev"]').disabled = i <= 0;
    body.querySelector('[data-act="next"]').disabled = i < 0 || i >= list.length - 1;
  }

  /* ------------------------------ 更新 ------------------------------ */
  function update(first) {
    if (openId == null) return;
    const c = store.state.cards[openId];
    if (!c) { close(); return; }
    const body = $('#drawerBody');

    // 头部列标签
    const pill = body.querySelector('.d-col');
    const prevCol = pill.dataset.col;
    pill.dataset.col = c.column;
    pill.style.setProperty('--c', `var(--c-${c.column})`);
    pill.querySelector('span').textContent = COLUMN_NAME[c.column];
    if (prevCol && prevCol !== c.column) { pill.classList.remove('changed'); void pill.offsetWidth; pill.classList.add('changed'); }

    // 标题
    const title = body.querySelector('.d-title');
    title.readOnly = locked(c);
    if (document.activeElement !== title && title.value !== c.title) { title.value = c.title; }
    requestAnimationFrame(() => autosize(title));
    const creator = c.activity[0]?.actor;
    body.querySelector('.d-meta').innerHTML = `
      <span>由 <b>${esc(creator ? (creator.kind === 'human' ? '你' : creator.name) : '—')}</b> 创建于 <b data-at="${c.createdAt}">${ago(c.createdAt, store.clock.now())}</b></span>
      <span>最近更新 <b data-at="${c.updatedAt}">${ago(c.updatedAt, store.clock.now())}</b></span>`;

    // 标签
    const lmap = B.labels.byId();
    const ls = (c.labels || []).map(id => lmap[id]).filter(Boolean);
    body.querySelector('#dLabels').innerHTML = ls.map(l => B.labels.chip(l, { removable: true })).join('') +
      `<button type="button" class="lb-add" data-label-picker>${icons.tag}${ls.length ? '' : '添加标签'}</button>`;

    // 领取区
    const sig = [c.column, c.holder, c.claimedAt].join('|');
    if (sig !== lastSig) { renderClaim(c); lastSig = sig; }

    // 描述
    body.querySelector('.desc-tabs').style.display = locked(c) ? 'none' : '';
    if (locked(c) && descMode === 'edit') descMode = 'view';
    renderDesc(c, first);

    // 动态
    renderActivity(c, first);
    renderNav();
    tick(store.clock.now());
  }

  function renderClaim(c) {
    const part = $('#drawerBody [data-part="claim"]');
    if (c.column === 'inbox') {
      part.style.display = '';
      part.innerHTML = `
        <div class="inbox-note">
          <div class="in-ic">${icons.inbox}</div>
          <div><b>未整理的条目</b><span>还没放上白板，agent 不会领取。补充好标题和描述后整理到「待办」。</span></div>
          <button class="btn sm primary" type="button" data-act="to-todo">整理到待办${icons.move}</button>
        </div>`;
      part.animate([{ opacity: 0, transform: 'translateY(8px)' }, { opacity: 1, transform: 'none' }], { duration: 360, easing: 'cubic-bezier(.16,1,.3,1)' });
      part.querySelector('[data-act="to-todo"]').onclick = () => {
        B.boardView.triage(c.id);
        toast({ type: 'success', title: `#${c.id} 已整理到「待办」`, text: '放在「待办」最下面，可以拖动调整优先级。', duration: 2400 });
      };
      return;
    }
    if (c.column !== 'doing' || !c.holder) {
      part.innerHTML = '';
      part.style.display = 'none';
      return;
    }
    part.style.display = '';
    part.innerHTML = `
      <div class="lock-note" style="margin-bottom:10px">${icons.lock}<div>「进行中」的卡片只有持有者 <b class="mono">${esc(c.holder)}</b> 能编辑和移动。你仍可以评论，或强制释放让它回到待办。</div></div>
      <div class="claim-card" style="--ac:${agentColor(c.holder)}">
        ${avatar({ kind: 'agent', name: c.holder }, 'lg')}
        <div class="claim-who"><b>${esc(c.holder)}</b><span>领取于 ${hhmm(c.claimedAt || c.updatedAt)} · 到期 <span class="exp">${hhmm(c.expiresAt)}</span></span></div>
        <div class="big-ring">${ring(27)}<span class="t">--:--</span></div>
        <div class="release">
          <span>持有者评论、编辑或再次 claim 会续期 30 分钟</span>
          <button class="btn sm danger" type="button" data-act="release">${icons.unlock}强制释放</button>
        </div>
      </div>`;
    part.animate([{ opacity: 0, transform: 'translateY(8px)' }, { opacity: 1, transform: 'none' }], { duration: 360, easing: 'cubic-bezier(.16,1,.3,1)' });
    part.querySelector('[data-act="release"]').onclick = () => {
      const r = store.try({ op: 'release', actor: HUMAN, id: c.id });
      if (!r.ok) return toastError(r.error);
      toast({ type: 'success', title: `已强制释放 #${c.id}`, text: `${c.holder} 的领取已清空，卡片回到「待办」最上面` });
    };
  }

  function renderDesc(c, force) {
    const holder = $('#drawerBody .desc-holder');
    const tabs = $('#drawerBody .desc-tabs');
    tabs.querySelectorAll('button').forEach(b => b.classList.toggle('on', b.dataset.mode === descMode));
    const on = tabs.querySelector('button.on'), thumb = tabs.querySelector('.thumb');
    if (on && thumb) requestAnimationFrame(() => { thumb.style.width = `${on.offsetWidth}px`; thumb.style.transform = `translateX(${on.offsetLeft - 2}px)`; });

    if (descMode === 'edit') {
      if (!force && holder.querySelector('.desc-edit')) return;
      holder.innerHTML = `
        <div class="desc-edit">
          <textarea class="textarea scroll" spellcheck="false" placeholder="支持 Markdown：验收标准、相关链接…"></textarea>
          <div class="row"><span class="hint">Markdown · ${B.ui.isMac ? '⌘' : 'Ctrl'}↵ 保存 · esc 取消</span><button class="btn sm ghost" type="button" data-act="cancel">取消</button><button class="btn sm primary" type="button" data-act="save">保存</button></div>
        </div>`;
      const ta = holder.querySelector('textarea');
      ta.value = c.body;
      requestAnimationFrame(() => autosize(ta));
      ta.addEventListener('input', () => autosize(ta));
      const save = () => {
        const r = store.try({ op: 'edit', actor: HUMAN, id: c.id, body: ta.value });
        if (!r.ok) return toastError(r.error);
        descMode = 'view'; renderDesc(store.state.cards[c.id], true);
      };
      ta.addEventListener('keydown', e => {
        if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); save(); }
        if (e.key === 'Escape') { e.stopPropagation(); descMode = 'view'; renderDesc(store.state.cards[c.id], true); }
      });
      holder.querySelector('[data-act="save"]').onclick = save;
      holder.querySelector('[data-act="cancel"]').onclick = () => { descMode = 'view'; renderDesc(store.state.cards[c.id], true); };
      return;
    }

    const key = `${c.body}|${locked(c)}`;
    if (!force && holder.dataset.key === key) return;
    holder.dataset.key = key;
    holder.innerHTML = c.body
      ? `<div class="desc-view md ${locked(c) ? 'locked' : ''}">${markdown(c.body)}</div>`
      : `<div class="desc-view empty ${locked(c) ? 'locked' : ''}">${icons.edit}${locked(c) ? '没有描述' : '添加描述…（支持 Markdown）'}</div>`;
    const view = holder.firstElementChild;
    view.addEventListener('dblclick', () => setDescMode('edit'));
    if (!c.body) view.addEventListener('click', () => setDescMode('edit'));
  }

  const ACT_ICON = { create: icons.sparkle, edit: icons.edit, move: icons.move, reorder: icons.reorder, claim: icons.claim, renew: icons.renew, timeout: icons.timer, release: icons.unlock };
  function actorName(a) {
    if (a.kind === 'human') return '<b>你</b>';
    if (a.kind === 'system') return '<b>系统</b>';
    return `<b class="agent-name" style="color:${agentColor(a.name)}">${esc(a.name)}</b>`;
  }

  function renderActivity(c, first) {
    const tl = $('#drawerBody .timeline');
    $('#drawerBody [data-part="activity"] .count').textContent = c.activity.length;
    if (first) { tl.innerHTML = ''; renderedActivity = 0; }
    const now = store.clock.now();
    const frag = document.createDocumentFragment();
    for (let i = renderedActivity; i < c.activity.length; i++) {
      const a = c.activity[i];
      const time = `<span class="tl-time" data-at="${a.at}" title="${new Date(a.at).toLocaleString()}">${ago(a.at, now)}</span>`;
      const node = a.kind === 'comment'
        ? el(`<div class="tl-item ${a.actor.kind === 'human' ? 'mine' : ''}">
                <div>${avatar(a.actor)}</div>
                <div class="tl-bubble"><div class="who">${actorName(a.actor)}${a.actor.kind === 'agent' ? '<span class="chip" style="height:18px;font-size:10.5px">CLI</span>' : ''}${time}</div><div class="md">${markdown(a.text)}</div></div>
              </div>`)
        : el(`<div class="tl-item">
                <div class="tl-icon k-${a.action}">${ACT_ICON[a.action] || icons.info}</div>
                <div class="tl-sys">${actorName(a.actor)} ${esc(a.text)} ${time}</div>
              </div>`);
      if (!first) node.classList.add('new');
      frag.appendChild(node);
    }
    const sc = tl.closest('.d-activity');
    const atBottom = sc && sc.scrollHeight - sc.scrollTop - sc.clientHeight < 60;
    tl.appendChild(frag);
    if (sc && !first && atBottom) requestAnimationFrame(() => sc.scrollTo({ top: sc.scrollHeight, behavior: 'smooth' }));
    renderedActivity = c.activity.length;
  }

  /* ------------------------------ 倒计时 ------------------------------ */
  let agoTick = 0;
  function tick(now) {
    if (openId == null) return;
    const c = store.state.cards[openId]; if (!c) return;
    const body = $('#drawerBody');
    if (c.column === 'doing' && c.expiresAt) {
      const left = c.expiresAt - now;
      const card = body.querySelector('.claim-card');
      if (card) {
        card.querySelector('.big-ring .t').textContent = mmss(left);
        setRing(card.querySelector('.big-ring svg'), left / CLAIM_MS);
        card.classList.toggle('urgent', left < 5 * 60 * 1000);
        card.querySelector('.exp').textContent = hhmm(c.expiresAt);
      }
    }
    if (++agoTick % 8 === 0) body.querySelectorAll('[data-at]').forEach(n => (n.textContent = ago(Number(n.dataset.at), now)));
  }

  /* --------------------------- 手机：下拉关闭 --------------------------- */
  function bindSheetGesture() {
    const drawer = $('#drawer');
    let y0 = null, dy = 0, t0 = 0;
    const startOn = e => {
      if (!isMobile()) return;
      const grab = e.target.closest('.drawer-grab, .d-head');
      if (!grab || e.target.closest('button')) return;
      y0 = e.clientY; dy = 0; t0 = performance.now();
      drawer.classList.add('dragging');
      drawer.setPointerCapture(e.pointerId);
    };
    drawer.addEventListener('pointerdown', startOn);
    drawer.addEventListener('pointermove', e => {
      if (y0 == null) return;
      dy = Math.max(0, e.clientY - y0);
      const eased = dy < 0 ? dy * 0.3 : dy;
      drawer.style.setProperty('--sheet-y', `${eased}px`);
    });
    const end = () => {
      if (y0 == null) return;
      drawer.classList.remove('dragging');
      const v = dy / Math.max(1, performance.now() - t0);
      y0 = null;
      if (dy > 140 || v > 0.6) close();
      else drawer.style.setProperty('--sheet-y', '0px');
    };
    drawer.addEventListener('pointerup', end);
    drawer.addEventListener('pointercancel', end);
  }

  B.drawer = {
    init(s) {
      store = s;
      $('#scrim').addEventListener('click', close);
      bindSheetGesture();
      bus.on('card:open', id => open(id));
      document.addEventListener('keydown', e => {
        if (openId == null || e.metaKey || e.ctrlKey || e.altKey) return;
        if (e.target.closest?.('input, textarea, [contenteditable]') || $('#dialog').classList.contains('on') || B.palette?.isOpen() || B.creator?.isOpen()) return;
        if (B.labels.pickerOpen) return;
        if (e.key === 'c' || e.key === 'C') { e.preventDefault(); B.prompt.copyCard(openId); return; }
        if (e.key === 'l' || e.key === 'L') { e.preventDefault(); $('#dLabels [data-label-picker]')?.click(); return; }
        if (e.key === 'ArrowUp' || e.key === 'k' || e.key === 'K') { e.preventDefault(); step(-1); }
        if (e.key === 'ArrowDown' || e.key === 'j' || e.key === 'J') { e.preventDefault(); step(1); }
      });
      store.subscribe((_, meta) => { if (meta.source === 'switch') close(); else if (!meta.foreign) update(false); });
    },
    open, close, tick,
    get openId() { return openId; },
  };
  B.ui.confirm = confirmDialog;
})(window.Board = window.Board || {});
