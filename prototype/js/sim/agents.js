/* ==========================================================================
   Board · 模拟 agent（原型专用，正式产品里没有）
   按 PRD §11 的「执行 agent」循环：
     board claim → 干活期间 board comment 记进度（同时续期）→ board move <ID> review
     做不下去：先 board comment 写原因，再 board move <ID> todo
   「掉线」：agent 停止一切操作但仍持有卡片 → 到期后被系统超时释放
   ========================================================================== */
(function (B) {
  'use strict';
  const { $, el, esc, avatar, agentColor, bus } = B.ui;
  const KEY = 'board.sim.agents';
  const SPARE = ['epsilon', 'zeta', 'eta', 'theta', 'kappa', 'lambda', 'sigma', 'omega'];

  const PROGRESS = [
    '读完了描述和相关代码，开始动手。',
    '复现了问题，定位到根因。',
    '修复写好了，正在跑 `cargo test`。',
    '测试全绿，补了一个回归用例。',
    '整理提交说明，准备提交审核。',
    '对照验收标准逐条自查中。',
  ];
  const FAIL = [
    '缺少复现环境（需要 Windows runner），先放回待办。',
    '描述里的验收标准有歧义，需要人确认后再做。',
    '依赖的上游 PR 还没合并，暂时做不下去。',
  ];

  let store;
  let agents = load();

  function load() {
    try { const a = JSON.parse(localStorage.getItem(KEY)); if (Array.isArray(a) && a.length) return a.map(x => ({ name: x.name, board: x.board || 'oss/main', running: false, offline: false })); } catch {}
    return [['alpha', 'oss/main'], ['beta', 'oss/main'], ['gamma', 'docs/content']].map(([name, board]) => ({ name, board, running: false, offline: false }));
  }
  function save() { try { localStorage.setItem(KEY, JSON.stringify(agents.map(a => ({ name: a.name, board: a.board })))); } catch {} }

  const rand = (a, b) => a + Math.random() * (b - a);
  const pick = arr => arr[Math.floor(Math.random() * arr.length)];

  function schedule(a, firstDelay) {
    clearTimeout(a.timer);
    if (!a.running || a.offline) return;
    // 每一步代表 3–7 分钟的虚拟工作量；按演示时钟换算成真实时间，并限制在 1.6s–9s
    const real = firstDelay ?? Math.max(1600, Math.min(9000, store.clock.toReal(rand(3, 7) * 60e3)));
    a.timer = setTimeout(() => { step(a); schedule(a); }, real);
  }

  function step(a) {
    if (!a.running || a.offline) return;
    if (!store.boardOf(a.board)) a.board = store.paths()[0];
    const board = store.boardOf(a.board);
    const run = args => B.console.execAs(a.name, args, a.board);
    const held = B.engine.query.heldBy(board, a.name);
    if (!held) {
      a.work = null;
      const r = run('claim');
      const got = r.exit === 0 && B.engine.query.heldBy(store.boardOf(a.board), a.name);
      if (got) { a.work = { id: got.id, done: 0, total: 2 + Math.floor(Math.random() * 3), fail: Math.random() < 0.15, failed: false }; a.note = `领取了 #${got.id}`; }
      else a.note = r.error?.code === 'EMPTY_TODO' ? '空闲：待办为空，稍后再试' : `claim 被拒绝：${r.error?.code}`;
      return render();
    }
    if (!a.work || a.work.id !== held.id) a.work = { id: held.id, done: 0, total: 2 + Math.floor(Math.random() * 2), fail: Math.random() < 0.12, failed: false };
    const w = a.work;
    if (w.fail && w.done >= 1) {
      if (!w.failed) { run(`comment ${w.id} "${pick(FAIL)}"`); w.failed = true; a.note = `#${w.id} 做不下去，写明原因`; }
      else { run(`move ${w.id} todo`); a.note = `把 #${w.id} 放回待办`; a.work = null; }
    } else if (w.done < w.total) {
      run(`comment ${w.id} "${PROGRESS[Math.min(w.done + (w.total > 3 ? 0 : 1), PROGRESS.length - 1)]}"`);
      w.done++;
      a.note = `正在做 #${w.id} · 进度 ${w.done}/${w.total}`;
    } else {
      run(`move ${w.id} review`);
      a.note = `#${w.id} 已提交审核`;
      a.work = null;
    }
    render();
  }

  /* ------------------------------ 列表 UI ------------------------------ */
  function stateText(a) {
    const held = B.engine.query.heldBy(store.boardOf(a.board) || store.state, a.name);
    if (a.offline) return held ? `<span class="a-state off">掉线中 · 不再续期 #${held.id}，到期后会被超时释放</span>` : `<span class="a-state off">掉线中</span>`;
    if (!a.running) return `<span class="a-state">${held ? `已暂停 · 持有 <b>#${held.id}</b>` : '已暂停'}</span>`;
    return `<span class="a-state">${esc(a.note || '启动中…').replace(/#(\d+)/g, '<b>#$1</b>')}</span>`;
  }
  function render() {
    const list = $('#agentsList');
    const keep = new Set();
    agents.forEach(a => {
      keep.add(a.name);
      let row = list.querySelector(`[data-name="${CSS.escape(a.name)}"]`);
      if (!row) {
        row = el(`<div class="agent-row" data-name="${esc(a.name)}" style="--ac:${agentColor(a.name)}">
          ${avatar({ kind: 'agent', name: a.name })}
          <span class="a-name">${esc(a.name)}<button type="button" class="a-proj" data-act="project" title="点击切换这个 agent 工作的白板"></button></span>
          <div class="a-ctrls">
            <button type="button" class="a-btn" data-act="run"></button>
            <button type="button" class="a-btn warn" data-act="offline">掉线</button>
            <button type="button" class="a-btn x" data-act="remove" aria-label="移除">×</button>
          </div>
          <span class="a-state"></span>
        </div>`);
        list.appendChild(row);
      }
      row.classList.toggle('running', a.running && !a.offline);
      row.classList.toggle('offline', a.offline);
      const run = row.querySelector('[data-act="run"]');
      run.textContent = a.running ? '暂停' : '运行';
      run.classList.toggle('on', a.running);
      row.querySelector('[data-act="offline"]').classList.toggle('on', a.offline);
      row.querySelector('[data-act="offline"]').textContent = a.offline ? '恢复' : '掉线';
      row.querySelector('.a-state').outerHTML = stateText(a);
      const pm = store.boardMetaOf(a.board);
      const pb = row.querySelector('.a-proj');
      pb.textContent = `@${a.board}`;
      pb.style.setProperty('--pc', pm ? pm.color : '#888');
    });
    [...list.children].forEach(r => { if (!keep.has(r.dataset.name)) r.remove(); });
    bus.emit('sim:changed', agents);
  }

  function bind() {
    $('#agentsList').addEventListener('click', e => {
      const b = e.target.closest('[data-act]'); if (!b) return;
      const a = agents.find(x => x.name === b.closest('.agent-row').dataset.name); if (!a) return;
      if (b.dataset.act === 'run') { a.running = !a.running; if (a.running) { a.offline = false; a.note = '启动中…'; schedule(a, 500); } else clearTimeout(a.timer); }
      if (b.dataset.act === 'offline') {
        a.offline = !a.offline;
        if (a.offline) { clearTimeout(a.timer); B.ui.toast({ type: 'info', title: `${a.name} 掉线了`, text: '它持有的卡片不会再续期，到期后系统会自动释放回「待办」最上面。试试把演示时钟调到 ×60。', duration: 5200 }); }
        else if (a.running) schedule(a, 600);
      }
      if (b.dataset.act === 'project') {
        if (B.engine.query.heldBy(store.boardOf(a.board), a.name)) { B.ui.toast({ type: 'info', title: `${a.name} 还持有 @${a.board} 的卡片`, text: '先让它做完或放回待办，再换白板。', duration: 2600 }); return; }
        const keys = store.paths();
        a.board = keys[(keys.indexOf(a.board) + 1) % keys.length];
        a.work = null; save();
      }
      if (b.dataset.act === 'remove') { clearTimeout(a.timer); agents = agents.filter(x => x !== a); save(); }
      render();
    });
    $('#addAgent').addEventListener('click', () => {
      const name = SPARE.find(n => !agents.some(a => a.name === n)) || `agent-${agents.length + 1}`;
      agents.push({ name, board: store.current, running: false, offline: false });
      save(); render();
    });
  }

  B.sim = {
    init(s) {
      store = s;
      bind(); render();
      store.subscribe(() => render());
      store.clock.onChange(() => agents.forEach(a => a.running && schedule(a, 400)));
    },
    anyRunning: () => agents.some(a => a.running && !a.offline),
    toggleAll() {
      const on = !this.anyRunning();
      agents.forEach((a, i) => { a.running = on; a.offline = false; if (on) { a.note = '启动中…'; schedule(a, 300 + i * 700); } else clearTimeout(a.timer); });
      render();
    },
    get agents() { return agents; },
  };
})(window.Board = window.Board || {});
