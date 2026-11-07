/* ==========================================================================
   Board · CLI 控制台（浏览器内终端）
   - 支持 board 全部命令、export/unset BOARD_AS、clear、历史（↑↓）、Tab 补全
   - 模拟 agent 执行的命令也会显示在这里（可关闭）
   ========================================================================== */
(function (B) {
  'use strict';
  const { $, el, esc, agentColor, bus } = B.ui;
  const ENV_KEY = 'board.cli.env', HIST_KEY = 'board.cli.history', H_KEY = 'board.console.h';

  let store;
  let env = load(ENV_KEY, { BOARD_AS: 'delta', BOARD: 'oss/main' });
  if (!('BOARD' in env)) env.BOARD = 'oss/main';
  delete env.BOARD_PROJECT;
  let history = load(HIST_KEY, []);
  let hIndex = -1;
  let showAgents = true;

  function load(k, d) { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } }
  function save(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch {} }

  /* ------------------------------ 渲染 ------------------------------ */
  function segHtml(seg) {
    if (seg.c === 'json') return `<span class="json">${colorJSON(seg.t)}</span>`;
    return seg.c ? `<span class="${seg.c}">${esc(seg.t)}</span>` : esc(seg.t);
  }
  function colorJSON(s) {
    return esc(s).replace(/(&quot;(?:\\.|[^&]|&(?!quot;))*?&quot;)(\s*:)?|\b(true|false|null)\b|(-?\d+(?:\.\d+)?)/g, (m, str, colon, lit, num) => {
      if (str) return colon ? `<span class="k">${str}</span>${colon}` : `<span class="s">${str}</span>`;
      if (lit) return `<span class="b">${lit}</span>`;
      return `<span class="n">${num}</span>`;
    });
  }
  function promptHtml(name, agentRun, project) {
    const who = name ? esc(name) : '<span class="none">(no BOARD_AS)</span>';
    const where = project ? `<span class="ps-proj">@${esc(project)}</span>` : (agentRun ? '' : '<span class="none">@(no BOARD)</span>');
    return `<span class="ps ${agentRun ? 'agent' : ''}">${who}${where}</span><span class="ps-sym">~ $</span>`;
  }

  function print({ cmd, name, res, agentRun, project = env.BOARD }) {
    const out = $('#termOut');
    const block = el(`<div class="t-block ${agentRun ? 'agent-run' : ''} ${agentRun && !showAgents ? 'hidden-agent' : ''}"></div>`);
    if (agentRun) block.style.setProperty('--ac', agentColor(name));
    const exit = res && res.exit ? `<span class="exit">exit ${res.exit}</span>` : '';
    block.innerHTML = `<div class="t-line t-cmd">${promptHtml(name, agentRun, project)}${esc(cmd)}${exit}</div>` +
      (res?.lines || []).map(line => `<div class="t-line">${line.map(segHtml).join('')}</div>`).join('');
    out.appendChild(block);
    while (out.children.length > 300) out.firstElementChild.remove();
    scrollBottom();
  }
  function printRaw(html, cls = '') {
    const b = el(`<div class="t-block ${cls}"></div>`); b.innerHTML = html;
    $('#termOut').appendChild(b); scrollBottom();
  }
  function scrollBottom() { const t = $('#term'); requestAnimationFrame(() => (t.scrollTop = t.scrollHeight)); }

  function updatePrompt() {
    $('#termPrompt').innerHTML = promptHtml(env.BOARD_AS, false, env.BOARD);
    const v = k => env[k] ? `${k}=<b>${esc(env[k])}</b>` : `<span class="none">${k} 未设置</span>`;
    $('#consoleEnv').innerHTML = `${v('BOARD_AS')} · ${v('BOARD')}`;
    renderSnippets();
  }

  /* ------------------------------ 执行 ------------------------------ */
  function exec(line) {
    line = line.trim();
    if (!line) return;
    if (history[history.length - 1] !== line) { history.push(line); if (history.length > 100) history.shift(); save(HIST_KEY, history); }
    hIndex = -1;
    const name = env.BOARD_AS;

    if (line === 'clear') { $('#termOut').innerHTML = ''; return; }
    if (line === 'help') return print({ cmd: line, name, res: B.cli.run('board help', { env, store }) });
    let m;
    const before = env.BOARD;
    if ((m = line.match(/^export\s+(BOARD_AS|BOARD)=(?:"([^"]*)"|'([^']*)'|(\S*))$/))) {
      env[m[1]] = (m[2] ?? m[3] ?? m[4] ?? '').trim();
      save(ENV_KEY, env); updatePrompt();
      return print({ cmd: line, name, project: before, res: { exit: 0, lines: [] } });
    }
    if ((m = line.match(/^unset\s+(BOARD_AS|BOARD)$/))) {
      env[m[1]] = ''; save(ENV_KEY, env); updatePrompt();
      return print({ cmd: line, name, project: before, res: { exit: 0, lines: [] } });
    }
    if ((m = line.match(/^echo\s+\$(BOARD_AS|BOARD)$/))) return print({ cmd: line, name, res: { exit: 0, lines: [[{ t: env[m[1]] || '' }]] } });

    const res = B.cli.run(line, { env, store });
    // --as 时提示符显示实际身份
    print({ cmd: line, name, res });
    renderSnippets();
    return res;
  }

  /** 模拟 agent 调用：以 --as 身份执行并输出到终端 */
  function execAs(agentName, args, project) {
    const line = `board ${args} --as ${agentName} --board ${project}`;
    const res = B.cli.run(line, { env: {}, store });
    print({ cmd: `board ${args}`, name: agentName, project, res: compact(res), agentRun: true });
    return res;
  }
  function compact(res) {
    if (!res.lines || res.lines.length <= 3) return res;
    return { ...res, lines: [...res.lines.slice(0, 2), [{ t: `  … 共 ${res.lines.length} 行`, c: 'dim' }]] };
  }

  /* ------------------------------ 补全 ------------------------------ */
  const CANDIDATES = [
    'board help', 'board ls', 'board ls --column todo', 'board ls --column doing', 'board ls --column review', 'board ls --search ',
    'board show ', 'board new "', 'board claim', 'board claim --json', 'board move ', 'board comment ', 'board edit ',
    'board boards', 'export BOARD_AS=', 'export BOARD=', 'unset BOARD_AS', 'unset BOARD', 'echo $BOARD_AS', 'clear',
  ];
  function suggestion(v) {
    if (!v) return '';
    const h = [...history].reverse().find(x => x.startsWith(v) && x !== v);
    if (h) return h;
    return CANDIDATES.find(x => x.startsWith(v) && x !== v) || '';
  }
  function updateGhost() {
    const input = $('#termInput'), ghost = $('#termGhost');
    const s = suggestion(input.value);
    ghost.textContent = s;
    ghost.style.visibility = s ? 'visible' : 'hidden';
  }

  /* ------------------------------ 快捷片段 ------------------------------ */
  function renderSnippets() {
    const viewing = store.current;
    let target = null; try { target = store.resolve(env.BOARD); } catch {}
    const s = (target && store.boardOf(target)) || store.state;
    const name = env.BOARD_AS;
    const mine = name ? B.engine.query.heldBy(s, name) : null;
    const todo2 = s.cols.todo[1] || s.cols.todo[0];
    const snips = [
      target !== viewing ? `export BOARD=${viewing}` : null,
      'board ls',
      mine ? `board comment ${mine.id} "进展：已完成一半"` : 'board claim',
      mine ? `board move ${mine.id} review` : null,
      mine ? `board move ${mine.id} done` : null,
      todo2 ? `board move ${todo2} doing` : null,
      `board new "从 CLI 新建的卡片" --body "gh#${200 + s.seq}" --top`,
      'board ls --search gh#140',
      'board claim --json',
      'board boards',
      name ? 'unset BOARD_AS' : 'export BOARD_AS=delta',
      env.BOARD ? 'unset BOARD' : null,
    ].filter(Boolean);
    $('#snippets').innerHTML = snips.map(x => `<button type="button" class="snippet" data-cmd="${esc(x)}">${esc(x).replace(/^board/, '<b>board</b>')}</button>`).join('');
  }

  /* ------------------------------ 开合 / 尺寸 ------------------------------ */
  function setOpen(on) {
    const c = $('#console');
    c.classList.toggle('on', on);
    c.setAttribute('aria-hidden', on ? 'false' : 'true');
    $('#toggleConsole').classList.toggle('on', on);
    document.documentElement.style.setProperty('--console-offset', on ? `${c.querySelector('.console-inner').offsetHeight}px` : '0px');
    try { localStorage.setItem('board.console.open', on ? '1' : '0'); } catch {}
    if (on) {
      if (!$('#termOut').children.length) welcome();
      setTimeout(() => $('#termInput').focus({ preventScroll: true }), 200);
    }
    bus.emit('console:toggle', on);
  }
  function welcome() {
    printRaw(`<div class="t-welcome"><b>模拟 CLI</b> · 仅用于演示，正式产品里没有这个终端（agent 在自己的环境里直接运行 board 命令）。<br>它和网页读写同一块白板，改动会实时出现在上方。输入 <b>board help</b> 查看命令；↑↓ 历史，Tab / → 补全，点击下方片段快速执行。</div>`);
  }

  function bindResize() {
    const handle = $('#consoleResize'), c = $('#console');
    const saved = Number(localStorage.getItem(H_KEY));
    if (saved) c.style.setProperty('--console-h', `${saved}px`);
    let y0, h0;
    handle.addEventListener('pointerdown', e => {
      y0 = e.clientY; h0 = c.querySelector('.console-inner').offsetHeight;
      c.classList.add('resizing'); handle.setPointerCapture(e.pointerId);
    });
    handle.addEventListener('pointermove', e => {
      if (y0 == null) return;
      const h = Math.max(180, Math.min(window.innerHeight * 0.75, h0 + (y0 - e.clientY)));
      c.style.setProperty('--console-h', `${h}px`);
      document.documentElement.style.setProperty('--console-offset', `${h}px`);
    });
    const end = () => {
      if (y0 == null) return;
      y0 = null; c.classList.remove('resizing');
      try { localStorage.setItem(H_KEY, String(c.querySelector('.console-inner').offsetHeight)); } catch {}
    };
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
  }

  B.console = {
    init(s) {
      store = s;
      const input = $('#termInput');
      $('#termForm').addEventListener('submit', e => { e.preventDefault(); const v = input.value; input.value = ''; updateGhost(); exec(v); });
      input.addEventListener('input', updateGhost);
      input.addEventListener('keydown', e => {
        if (e.key === 'ArrowUp' && history.length) {
          e.preventDefault(); hIndex = hIndex < 0 ? history.length - 1 : Math.max(0, hIndex - 1); input.value = history[hIndex]; updateGhost();
        } else if (e.key === 'ArrowDown' && hIndex >= 0) {
          e.preventDefault(); hIndex++; if (hIndex >= history.length) { hIndex = -1; input.value = ''; } else input.value = history[hIndex]; updateGhost();
        } else if ((e.key === 'Tab' || (e.key === 'ArrowRight' && input.selectionStart === input.value.length)) && suggestion(input.value)) {
          e.preventDefault(); input.value = suggestion(input.value); updateGhost();
        } else if (e.key === 'Escape') { e.stopPropagation(); input.blur(); }
        else if (e.key === 'l' && e.ctrlKey) { e.preventDefault(); $('#termOut').innerHTML = ''; }
      });
      $('#term').addEventListener('click', e => { if (!window.getSelection().toString() && !e.target.closest('button')) input.focus(); });
      $('#snippets').addEventListener('click', e => { const b = e.target.closest('.snippet'); if (!b) return; exec(b.dataset.cmd); input.focus({ preventScroll: true }); });
      $('#toggleConsole').onclick = () => B.console.toggle();
      $('#closeConsole').onclick = () => setOpen(false);
      $('#clearConsole').onclick = () => ($('#termOut').innerHTML = '');
      $('#showAgentLog').addEventListener('change', e => {
        showAgents = e.target.checked;
        document.querySelectorAll('.t-block.agent-run').forEach(b => b.classList.toggle('hidden-agent', !showAgents));
      });
      bindResize();
      updatePrompt();
      store.subscribe(() => renderSnippets());
      if (localStorage.getItem('board.console.open') === '1') setOpen(true);
    },
    open: () => setOpen(true),
    close: () => setOpen(false),
    toggle: () => setOpen(!$('#console').classList.contains('on')),
    isOpen: () => $('#console').classList.contains('on'),
    exec, execAs,
  };
})(window.Board = window.Board || {});
