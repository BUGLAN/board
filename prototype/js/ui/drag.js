/* ==========================================================================
   Board · 拖拽（Pointer Events，鼠标 / 触控笔 / 触摸统一）
   - 鼠标：按下移动 5px 开始；触摸：长按 220ms 开始
   - 拖动中用 engine 做「预演」判断规则：不合法的列显示禁止遮罩
   - 松手：合法 → 落位；不合法 → 弹回原位并提示原因（PRD §8）
   ========================================================================== */
(function (B) {
  'use strict';
  const { engine, schema } = B;
  const { HUMAN, COLUMN_NAME } = schema;
  const { $, $$, el, esc, icons, reduceMotion, toastError } = B.ui;
  const view = () => B.boardView;

  let store;
  let st = null;
  let justDroppedAt = 0;

  function init(s) {
    store = s;
    document.addEventListener('pointerdown', onDown);
    document.addEventListener('touchmove', e => { if (st && st.started) e.preventDefault(); }, { passive: false });
    document.addEventListener('contextmenu', e => { if (st) e.preventDefault(); });
  }

  function onDown(e) {
    const card = e.target.closest('[data-drop] .card');
    if (!card || e.button !== 0 || st) return;
    if (e.target.closest('button, a, input, textarea')) return;
    const id = Number(card.dataset.id);
    const list = card.parentElement;
    st = {
      id, card, pointerId: e.pointerId, type: e.pointerType,
      x0: e.clientX, y0: e.clientY, x: e.clientX, y: e.clientY,
      fromCol: list.dataset.list, fromIndex: [...list.children].indexOf(card),
      started: false, rot: 0, vx: 0, lastX: e.clientX, lastT: performance.now(),
      target: null, valid: true, error: null,
    };
    if (e.pointerType !== 'mouse') {
      st.timer = setTimeout(() => { if (st && !st.started) start(); }, 220);
    }
    window.addEventListener('pointermove', onMove, { passive: false });
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onCancel);
    window.addEventListener('keydown', onKey);
  }

  function onMove(e) {
    if (!st || e.pointerId !== st.pointerId) return;
    st.x = e.clientX; st.y = e.clientY;
    const dist = Math.hypot(st.x - st.x0, st.y - st.y0);
    if (!st.started) {
      if (st.type === 'mouse' && dist > 5) start();
      else if (st.type !== 'mouse' && dist > 10) return cleanup(); // 手指在滑动页面
      if (!st || !st.started) return;
    }
    e.preventDefault();
    const now = performance.now();
    const dt = Math.max(1, now - st.lastT);
    st.vx = st.vx * 0.7 + ((st.x - st.lastX) / dt) * 0.3;
    st.lastX = st.x; st.lastT = now;
    if (!st.raf) st.raf = requestAnimationFrame(frame);
  }

  function start() {
    const { card } = st;
    clearTimeout(st.timer);
    st.started = true;
    view().lock();
    const r = card.getBoundingClientRect();
    st.offX = st.x0 - r.left; st.offY = st.y0 - r.top;
    st.w = r.width;
    const ghost = card.cloneNode(true);
    ghost.classList.remove('flash', 'entering', 'is-open');
    ghost.querySelector('.card-badge')?.remove();
    ghost.classList.add('drag-ghost');
    ghost.style.width = `${r.width}px`;
    ghost.style.transform = `translate(${r.left}px, ${r.top}px)`;
    document.body.appendChild(ghost);
    st.ghost = ghost;
    st.home = { col: st.fromCol, index: st.fromIndex };
    card.classList.add('is-placeholder');
    $('#board').classList.add('dragging');
    document.body.classList.add('is-dragging');
    view().columnEl(st.fromCol).classList.add('drop-home');
    if (st.type !== 'mouse' && navigator.vibrate) navigator.vibrate(10);
    // 抬起的瞬间
    ghost.animate([{ scale: '1' }, { scale: '1.045' }], { duration: 220, easing: 'cubic-bezier(.34,1.56,.64,1)', fill: 'forwards' });
    frame();
  }

  function frame() {
    if (!st || !st.started) return;
    st.raf = 0;
    const tx = st.x - st.offX, ty = st.y - st.offY;
    const targetRot = Math.max(-7, Math.min(7, st.vx * 9));
    st.rot += (targetRot - st.rot) * 0.25;
    st.vx *= 0.86; // 指针停下后，倾斜慢慢回正
    st.ghost.style.transform = `translate(${tx}px, ${ty}px) rotate(${st.rot.toFixed(2)}deg)`;
    locate();
    autoScroll();
    if (Math.abs(st.rot) > 0.05 || st.scrolling) st.raf = requestAnimationFrame(frame);
  }

  /* 找到指针下的列与插入位置 */
  /* 可放置区域：白板四列 + 侧栏「未整理」（只算当前可见的） */
  const zones = () => $$('[data-drop]').filter(z => z.offsetParent !== null && z.getBoundingClientRect().width > 40);

  function locate() {
    const cols = zones();
    let col = null;
    for (const c of cols) {
      const r = c.getBoundingClientRect();
      if (st.x >= r.left - 4 && st.x <= r.right + 4 && st.y >= r.top - 40 && st.y <= r.bottom + 40) { col = c; break; }
    }
    if (!col) col = view().columnEl(st.target ? st.target.col : st.fromCol);
    const key = col.dataset.drop;

    // 计算插入下标（忽略占位卡片本身）
    const list = view().listEl(key);
    const items = [...list.children].filter(n => n !== st.card);
    let index = items.length;
    for (let i = 0; i < items.length; i++) {
      const r = items[i].getBoundingClientRect();
      if (st.y < r.top + r.height / 2) { index = i; break; }
    }

    // 预演规则
    let valid = true, error = null;
    const same = key === st.fromCol && index === st.fromIndex;
    if (!same) {
      try { engine.apply(store.state, { op: 'move', actor: HUMAN, id: st.id, column: key, index }, store.clock.now()); }
      catch (err) { valid = false; error = err; }
    }

    const changed = !st.target || st.target.col !== key || st.target.index !== index || st.valid !== valid;
    st.target = { col: key, index };
    st.valid = valid; st.error = error;
    if (!changed) return;

    // 列的状态
    cols.forEach(c => {
      const k = c.dataset.drop;
      c.classList.toggle('drop-ok', valid && k === key && !(same && k === st.fromCol));
      c.classList.toggle('drop-bad', !valid && k === key);
      if (!valid && k === key) {
        c.querySelector('.col-veto b').textContent = vetoTitle(error, key);
        c.querySelector('.col-veto span').innerHTML = `${esc(error.message)}<br><code>${error.code}</code>`;
      }
    });
    st.ghost.classList.toggle('bad', !valid);
    st.ghost.querySelector('.ghost-hint')?.remove();
    if (!valid) st.ghost.appendChild(el(`<span class="ghost-hint">${icons.lock}<span>${esc(vetoTitle(error, key))}</span></span>`));

    // 移动占位：合法时跟随，不合法时回到原位
    const dest = valid ? st.target : st.home;
    placePlaceholder(dest.col, dest.index);
  }

  function vetoTitle(err, col) {
    if (err.code === 'FORBIDDEN_COLUMN' && col === 'doing') return '只能由 agent 领取进入';
    if (err.code === 'NOT_HOLDER') return `${err.detail?.holder || 'agent'} 正持有这张卡片`;
    if (col === 'inbox') return '不能放回「未整理」';
    return `不能放进「${COLUMN_NAME[col]}」`;
  }

  /* 把占位卡片放到 (col, index)，并给同列卡片做 FLIP 让位动画 */
  function placePlaceholder(col, index) {
    const list = view().listEl(col);
    const current = st.card.parentElement;
    const others = [...list.children].filter(n => n !== st.card);
    const ref = others[index] || null;
    if (current === list && st.card.nextElementSibling === ref) return;
    const affected = new Set([...list.children, ...(current ? current.children : [])]);
    affected.delete(st.card);
    const before = new Map([...affected].map(n => [n, n.getBoundingClientRect()]));
    list.insertBefore(st.card, ref);
    if (reduceMotion()) return;
    before.forEach((r, n) => {
      const a = n.getBoundingClientRect();
      const dy = r.top - a.top, dx = r.left - a.left;
      if (Math.abs(dy) > 0.5 || Math.abs(dx) > 0.5) {
        n.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], { duration: 300, easing: 'cubic-bezier(.22,1.1,.36,1)' });
      }
    });
    // 占位出现
    st.card.animate([{ opacity: 0, transform: 'scaleY(.6)' }, { opacity: 1, transform: 'none' }], { duration: 260, easing: 'cubic-bezier(.16,1,.3,1)' });
  }

  /* 靠近边缘时自动滚动（列内纵向 / 白板横向） */
  function autoScroll() {
    st.scrolling = false;
    const edge = 56, speed = 14;
    const body = view().columnEl(st.target?.col || st.fromCol)?.querySelector('.col-body, .side-scroll');
    if (body) {
      const r = body.getBoundingClientRect();
      if (st.y < r.top + edge && body.scrollTop > 0) { body.scrollTop -= speed * (1 - (st.y - r.top) / edge); st.scrolling = true; }
      else if (st.y > r.bottom - edge && body.scrollTop < body.scrollHeight - body.clientHeight) { body.scrollTop += speed * (1 - (r.bottom - st.y) / edge); st.scrolling = true; }
    }
    const board = $('#board');
    const br = board.getBoundingClientRect();
    if (board.scrollWidth > board.clientWidth) {
      if (st.x < br.left + edge) { board.scrollLeft -= speed; st.scrolling = true; }
      else if (st.x > br.right - edge) { board.scrollLeft += speed; st.scrolling = true; }
    }
  }

  function onUp(e) {
    if (!st || e.pointerId !== st.pointerId) return;
    if (!st.started) return cleanup();
    finish(false);
  }
  function onCancel() { if (st && st.started) finish(true); else cleanup(); }
  function onKey(e) { if (e.key === 'Escape' && st && st.started) { e.preventDefault(); finish(true); } }

  function finish(cancelled) {
    const s = st;
    locate();
    const { target, valid, error } = s;
    const same = target.col === s.fromCol && target.index === s.fromIndex;
    let ok = false;

    if (cancelled || !valid) {
      placePlaceholder(s.home.col, s.home.index);
      if (!cancelled && error) { toastError(error); view().rejectColumn(target.col); }
    } else if (!same) {
      const r = store.try({ op: 'move', actor: HUMAN, id: s.id, column: target.col, index: target.index });
      ok = r.ok;
      if (!r.ok) { placePlaceholder(s.home.col, s.home.index); toastError(r.error); }
    }

    // 幽灵飞回占位处
    const dest = s.card.getBoundingClientRect();
    const g = s.ghost;
    const anim = g.animate(
      [{ transform: g.style.transform, scale: '1.045' }, { transform: `translate(${dest.left}px, ${dest.top}px) rotate(0deg)`, scale: '1' }],
      { duration: reduceMotion() ? 1 : (valid && !cancelled ? 280 : 460), easing: valid && !cancelled ? 'cubic-bezier(.2,1.2,.4,1)' : 'cubic-bezier(.34,1.3,.64,1)', fill: 'forwards' }
    );
    const card = s.card;
    anim.onfinish = () => {
      g.remove();
      card.classList.remove('is-placeholder');
      if (ok) card.animate([{ transform: 'scale(1.03)' }, { transform: 'none' }], { duration: 300, easing: 'cubic-bezier(.34,1.56,.64,1)' });
      view().unlock();
    };
    justDroppedAt = performance.now();
    cleanup();
  }

  function cleanup() {
    if (!st) return;
    clearTimeout(st.timer);
    cancelAnimationFrame(st.raf);
    $('#board').classList.remove('dragging');
    document.body.classList.remove('is-dragging');
    $$('[data-drop]').forEach(c => c.classList.remove('drop-ok', 'drop-bad', 'drop-home'));
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', onCancel);
    window.removeEventListener('keydown', onKey);
    st = null;
  }

  B.drag = {
    init,
    justDropped: () => performance.now() - justDroppedAt < 120,
    get active() { return !!(st && st.started); },
  };
})(window.Board = window.Board || {});
