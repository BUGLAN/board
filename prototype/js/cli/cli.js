/* ==========================================================================
   Board · CLI（浏览器内模拟）
   与 PRD §7 一致的命令集；身份来自 --as 或 BOARD_AS。
   run(line, env) → { exit, json, lines: Seg[][], actor }
   Seg = { t: string, c?: string }   c 为终端着色用的 class 名
   正式产品里这一层就是真正的 `board` 可执行文件，调用同一个 engine。
   ========================================================================== */
(function (B) {
  'use strict';
  const { schema, engine } = B;
  const { ALL_KEYS: COLUMN_KEYS, COLUMN_NAME, BoardError, ERRORS } = schema;

  /* ---------- 词法：支持 "双引号" '单引号' 与 \ 转义 ---------- */
  function tokenize(line) {
    const out = [];
    let cur = '', q = null, has = false;
    for (let i = 0; i < line.length; i++) {
      const ch = line[i];
      if (q) {
        if (ch === '\\' && q === '"' && i + 1 < line.length) { cur += line[++i]; continue; }
        if (ch === q) { q = null; continue; }
        cur += ch;
      } else if (ch === '"' || ch === "'") { q = ch; has = true; }
      else if (/\s/.test(ch)) { if (cur || has) out.push(cur); cur = ''; has = false; }
      else if (ch === '\\' && i + 1 < line.length) { cur += line[++i]; has = true; }
      else { cur += ch; has = true; }
    }
    if (q) throw new BoardError('USAGE', '引号没有闭合');
    if (cur || has) out.push(cur);
    return out;
  }

  const VALUE_FLAGS = new Set(['--as', '--board', '--column', '--search', '--label', '--body', '--title']);
  const BOOL_FLAGS = new Set(['--json', '--top', '--help', '-h']);

  function parse(tokens) {
    const flags = {}, pos = [];
    for (let i = 0; i < tokens.length; i++) {
      const t = tokens[i];
      if (t.startsWith('--') && t.includes('=')) { const [k, ...v] = t.split('='); flags[k] = v.join('='); continue; }
      if (VALUE_FLAGS.has(t)) {
        if (i + 1 >= tokens.length) throw new BoardError('USAGE', `${t} 需要一个值`);
        flags[t] = tokens[++i]; continue;
      }
      if (BOOL_FLAGS.has(t)) { flags[t] = true; continue; }
      if (t.startsWith('--')) throw new BoardError('USAGE', `未知参数 ${t}`);
      pos.push(t);
    }
    return { flags, pos };
  }

  /* ---------- 输出工具 ---------- */
  const seg = (t, c) => ({ t: String(t), c });
  const width = s => [...String(s)].reduce((w, ch) => w + (/[ᄀ-ᅟ⺀-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦]/.test(ch) ? 2 : 1), 0);
  const pad = (s, w) => s + ' '.repeat(Math.max(0, w - width(s)));
  const hhmm = t => new Date(t).toTimeString().slice(0, 5);
  const mmss = ms => { ms = Math.max(0, ms); const m = Math.floor(ms / 60000), s = Math.floor(ms / 1000) % 60; return `${m}:${String(s).padStart(2, '0')}`; };
  const iso = t => (t == null ? null : new Date(t).toISOString());

  let labelName = id => id;
  const cardJSON = (c, full) => {
    const base = { id: c.id, column: c.column, title: c.title, holder: c.holder || null, labels: (c.labels || []).map(l => labelName(l)) };
    if (!full) return base;
    return {
      ...base, body: c.body, expires_at: iso(c.expiresAt), created_at: iso(c.createdAt), updated_at: iso(c.updatedAt),
      activity: c.activity.map(a => ({ at: iso(a.at), actor: a.actor.name, actor_kind: a.actor.kind, type: a.kind, action: a.action, text: a.text })),
    };
  };

  function showLines(c, now) {
    const L = [];
    L.push([seg(`#${c.id}`, 'id'), seg('  '), seg(c.title, 'strong')]);
    L.push([seg(pad('列', 10), 'dim'), seg(`${COLUMN_NAME[c.column]} `), seg(`(${c.column})`, `col-${c.column}`)]);
    if ((c.labels || []).length) L.push([seg(pad('标签', 10), 'dim'), ...c.labels.flatMap(l => [seg(`#${labelName(l)}`, 'label'), seg(' ')])]);
    if (c.holder) {
      L.push([seg(pad('持有者', 10), 'dim'), seg(c.holder, 'agent')]);
      L.push([seg(pad('到期', 10), 'dim'), seg(hhmm(c.expiresAt)), seg(`  剩余 ${mmss(c.expiresAt - now)}`, c.expiresAt - now < 5 * 60000 ? 'warn' : 'dim')]);
    }
    L.push([seg(pad('描述', 10), 'dim'), seg(c.body ? '' : '（空）', 'dim')]);
    if (c.body) c.body.split('\n').forEach(l => L.push([seg('  ' + l)]));
    L.push([seg('动态', 'dim')]);
    c.activity.forEach(a => {
      L.push([
        seg(`  ${hhmm(a.at)}  `, 'dim'),
        seg(pad(a.actor.name, 16), a.actor.kind === 'agent' ? 'agent' : a.actor.kind === 'human' ? 'human' : 'warn'),
        seg(a.kind === 'comment' ? '💬 ' + a.text : a.text, a.kind === 'comment' ? '' : 'dim'),
      ]);
    });
    return L;
  }

  const HELP = [
    [seg('board', 'strong'), seg(' — 看板式白板的命令行（原型模拟）', 'dim')],
    [seg('')],
    [seg('  ls ', 'cmd'), seg('[--column <列>] [--search <文字>] [--label <标签>]', 'dim'), seg('  列出卡片')],
    [seg('  show ', 'cmd'), seg('<ID>', 'dim'), seg('                                显示全部字段和动态')],
    [seg('  new ', 'cmd'), seg('"<标题>" [--body "<描述>"] [--top]', 'dim'), seg('   在「待办」新建')],
    [seg('  edit ', 'cmd'), seg('<ID> [--title ".."] [--body ".."]', 'dim'), seg('    修改标题或描述')],
    [seg('  move ', 'cmd'), seg('<ID> <列> [--top]', 'dim'), seg('                   移到指定列')],
    [seg('  claim ', 'cmd'), seg('[<ID>]', 'dim'), seg('                             领取（默认取「待办」最上面）')],
    [seg('  comment ', 'cmd'), seg('<ID> "<内容>"', 'dim'), seg('                  追加评论')],
    [seg('  label ', 'cmd'), seg('<ID> +<标签> -<标签>', 'dim'), seg('              加 / 去标签')],
    [seg('  labels', 'cmd'), seg('                                    列出白板所属项目的标签')],
    [seg('  boards', 'cmd'), seg('                                    列出所有白板（项目/白板）')],
    [seg('')],
    [seg('  列：', 'dim'), seg('todo  doing  review  done'), seg('    未整理：inbox', 'dim')],
    [seg('  身份：', 'dim'), seg('--as <名字>'), seg(' 或 ', 'dim'), seg('export BOARD_AS=<名字>')],
    [seg('  白板：', 'dim'), seg('--board <项目>/<白板>'), seg(' 或 ', 'dim'), seg('export BOARD=<项目>/<白板>')],
    [seg('  所有命令支持 ', 'dim'), seg('--json')],
  ];

  /* ---------- 执行 ---------- */
  function run(line, { env = {}, store }) {
    let json = false;
    try {
      const tokens = tokenize(line.trim());
      if (tokens[0] !== 'board') throw new BoardError('USAGE', `command not found: ${tokens[0] || ''}`);
      const { flags, pos } = parse(tokens.slice(1));
      json = !!flags['--json'];
      const sub = pos[0];
      if (!sub || flags['--help'] || flags['-h'] || sub === 'help') return { exit: 0, json: false, lines: HELP };

      const name = (flags['--as'] || env.BOARD_AS || '').trim();
      if (!name) throw new BoardError('NO_IDENTITY');
      const actor = schema.agent(name);

      // 白板：项目下可以有多块白板，用「项目/白板」定位
      if (sub === 'boards') {
        const list = store.boards;
        return {
          exit: 0, json, actor,
          lines: json
            ? [[seg(JSON.stringify(list.map(b => ({ board: b.path, name: b.name, project: b.projectName, counts: b.counts })), null, 2), 'json')]]
            : list.map(b => [seg(pad(b.path, 16), 'id'), seg(pad(`${b.projectName} / ${b.name}`, 28)), seg(`待办 ${b.counts.todo} · 进行中 ${b.counts.doing} · 待审核 ${b.counts.review}`, 'dim')]),
        };
      }
      const path = store.resolve(flags['--board'] || env.BOARD);
      const board = store.boardOf(path);
      const pk = path.split('/')[0];
      const labels = store.labelsOf(pk);
      labelName = id => (labels.find(l => l.id === id) || {}).name || id;
      const findLabel = name => {
        const l = store.findLabel(name, pk);
        if (!l) throw new BoardError('USAGE', `项目里没有标签「${name}」，可选：${labels.map(x => x.name).join('、') || '（还没有标签）'}`);
        return l;
      };
      const now = () => store.clock.now();
      const needId = () => {
        const raw = pos[1];
        if (raw == null) throw new BoardError('USAGE', `board ${sub} 需要 <ID>`);
        const id = Number(String(raw).replace(/^#/, ''));
        if (!Number.isInteger(id) || id < 1) throw new BoardError('USAGE', `无效的 ID「${raw}」`);
        return id;
      };
      const exec = cmd => store.dispatch({ ...cmd, actor }, { via: 'cli', board: path });
      const ok = (data, lines) => ({ exit: 0, json, actor, lines: json ? [[seg(JSON.stringify(data, null, 2), 'json')]] : lines });

      switch (sub) {
        case 'ls': {
          const column = flags['--column'];
          if (column && !COLUMN_KEYS.includes(column)) throw new BoardError('USAGE', `未知的列「${column}」，可选：${COLUMN_KEYS.join(' / ')}`);
          const label = flags['--label'] ? findLabel(flags['--label']).id : null;
          const cards = engine.query.list(board, { column, search: flags['--search'], label });
          const lines = cards.length
            ? cards.map(c => [seg(pad(`#${c.id}`, 6), 'id'), seg(pad(c.column, 8), `col-${c.column}`), seg(pad(c.title, 48)), seg(pad((c.labels || []).map(l => '#' + labelName(l)).join(' '), 18), 'label'), seg(c.holder || '', 'agent')])
            : [[seg('（没有匹配的卡片）', 'dim')]];
          return ok(cards.map(c => cardJSON(c)), lines);
        }
        case 'show': {
          const id = needId();
          const c = board.cards[id];
          if (!c) throw new BoardError('NOT_FOUND', `卡片 #${id} 不存在`);
          return ok(cardJSON(c, true), showLines(c, now()));
        }
        case 'new': {
          const title = pos.slice(1).join(' ');
          const c = exec({ op: 'new', title, body: flags['--body'], top: !!flags['--top'] });
          return ok(cardJSON(c, true), [[seg('✓ ', 'ok'), seg('已新建 '), seg(`#${c.id}`, 'id'), seg(`  ${c.title}`), seg(flags['--top'] ? '  → 待办最上面' : '  → 待办最下面', 'dim')]]);
        }
        case 'edit': {
          const id = needId();
          if (flags['--title'] == null && flags['--body'] == null) throw new BoardError('USAGE', 'board edit 需要 --title 或 --body');
          const c = exec({ op: 'edit', id, title: flags['--title'], body: flags['--body'] });
          return ok(cardJSON(c, true), [[seg('✓ ', 'ok'), seg('已更新 '), seg(`#${c.id}`, 'id'), ...(c.holder === name ? [seg(`  领取续期至 ${hhmm(c.expiresAt)}`, 'dim')] : [])]]);
        }
        case 'move': {
          const id = needId();
          const col = pos[2];
          if (!col) throw new BoardError('USAGE', 'board move 需要 <列>');
          const c = exec({ op: 'move', id, column: col, top: !!flags['--top'] });
          return ok(cardJSON(c, true), [[seg('✓ ', 'ok'), seg(`#${c.id}`, 'id'), seg(' → '), seg(`${COLUMN_NAME[c.column]} (${c.column})`, `col-${c.column}`), seg(flags['--top'] ? '  最上面' : '', 'dim')]]);
        }
        case 'claim': {
          const id = pos[1] != null ? needId() : null;
          const c = exec({ op: 'claim', id });
          return ok(cardJSON(c, true), [[seg('✓ ', 'ok'), seg('已领取', 'strong'), seg(`  到期 ${hhmm(c.expiresAt)}（30 分钟）`, 'dim')], [seg('')], ...showLines(c, now())]);
        }
        case 'comment': {
          const id = needId();
          const text = pos.slice(2).join(' ');
          const c = exec({ op: 'comment', id, text });
          return ok(cardJSON(c, true), [[seg('✓ ', 'ok'), seg('已评论 '), seg(`#${c.id}`, 'id'), ...(c.holder === name ? [seg(`  领取续期至 ${hhmm(c.expiresAt)}`, 'dim')] : [])]]);
        }
        case 'labels': {
          const used = store.labelUsage(pk);
          return ok(labels.map(l => ({ name: l.name, color: l.color, cards: used[l.id] || 0 })),
            labels.length ? labels.map(l => [seg(pad('#' + l.name, 16), 'label'), seg(`${used[l.id] || 0} 张卡片`, 'dim')]) : [[seg('（这个项目还没有标签）', 'dim')]]);
        }
        case 'label': {
          const id = needId();
          const ops = pos.slice(2);
          if (!ops.length) throw new BoardError('USAGE', 'board label <ID> +<标签> -<标签>');
          const add = [], remove = [];
          for (const o of ops) {
            if (o.startsWith('-')) remove.push(findLabel(o.slice(1)).id);
            else add.push(findLabel(o.replace(/^\+/, '')).id);
          }
          if (!board.cards[id]) throw new BoardError('NOT_FOUND', `卡片 #${id} 不存在`);
          const c = store.setLabels(id, { add, remove }, { board: path, actor, via: 'cli' });
          return ok(cardJSON(c, true), [[seg('✓ ', 'ok'), seg(`#${c.id}`, 'id'), seg('  标签：'), ...((c.labels || []).length ? c.labels.flatMap(l => [seg('#' + labelName(l), 'label'), seg(' ')]) : [seg('（无）', 'dim')])]]);
        }
        default:
          throw new BoardError('USAGE', `未知命令「${sub}」，运行 board help 查看用法`);
      }
    } catch (err) {
      if (!(err instanceof BoardError)) { console.error(err); err = new BoardError('USAGE', String(err.message || err)); }
      const data = { error: { code: err.code, message: err.message } };
      return {
        exit: 1, json, error: err,
        lines: json
          ? [[seg(JSON.stringify(data, null, 2), 'json')]]
          : [[seg('✗ ', 'err'), seg(err.code, 'err-code'), seg('  ' + err.message, 'err')]],
      };
    }
  }

  B.cli = { run, tokenize, ERRORS };
})(window.Board = window.Board || {});
