/* ==========================================================================
   Board · 复制为提示词
   把卡片 / 白板整理成一段可以直接粘贴给 agent 的提示词（Markdown）：
   - 卡片：任务、标签、描述、最近动态，以及处理这张卡片该执行的 board 命令
   - 未整理的条目：改为「帮我把它整理成可执行的卡片」
   - 白板：执行 agent 的工作循环 + 当前待办列表
   复制后 toast 里可以「预览」完整内容。
   ========================================================================== */
(function (B) {
  'use strict';
  const { $, el, esc, icons, toast, hhmm } = B.ui;
  const { COLUMN_NAME } = B.schema;

  let store;
  const labelNames = (ids, pk) => { const m = B.labels.byId(pk); return (ids || []).map(id => m[id]?.name).filter(Boolean); };
  const who = a => (a.kind === 'human' ? '人' : a.kind === 'system' ? '系统' : a.name);

  /* ------------------------------ 卡片 ------------------------------ */
  function cardPrompt(id, path = store.current) {
    const board = store.boardOf(path), meta = store.boardMetaOf(path);
    const c = board?.cards[id]; if (!c) return null;
    const pk = meta.project;
    const labels = labelNames(c.labels, pk);
    const recent = c.activity.slice(-6).map(a => `- ${hhmm(a.at)} ${who(a.actor)}${a.kind === 'comment' ? '：' : ' '}${a.text}`).join('\n');
    const env = `BOARD=${path}`;

    if (c.column === 'inbox') {
      return [
        `# 帮我整理一个想法：#${c.id} ${c.title}`,
        '',
        `来自 Board「${meta.projectName} / ${meta.name}」的「未整理」。请把它整理成一张可以交给 agent 执行的卡片：`,
        '',
        '1. 给出一个清楚、可执行的标题（一句话说清楚要做什么）',
        '2. 写出描述：背景、范围，以及可以逐条检查的**验收标准**（用 `- [ ]` 列表）',
        '3. 如果它其实应该拆成几张卡片，直接给出拆分后的每一张',
        '4. 如果信息不足以判断，列出需要先确认的问题',
        '',
        '## 原始内容',
        c.body || '（只有标题，没有描述）',
        labels.length ? `\n标签：${labels.join('、')}` : '',
        '',
        '## 整理好之后',
        `在界面上把它移到「待办」，或用 CLI 新建：\`${env} board new "<标题>" --body "<描述>"\``,
      ].join('\n');
    }

    const lines = [
      `# 任务：#${c.id} ${c.title}`,
      '',
      `- 白板：${meta.projectName} / ${meta.name}（\`${env}\`）`,
      `- 当前状态：${COLUMN_NAME[c.column]}${c.holder ? `，由 ${c.holder} 持有，领取到期 ${hhmm(c.expiresAt)}` : ''}`,
      labels.length ? `- 标签：${labels.join('、')}` : null,
      '',
      '## 描述',
      c.body || '（没有描述，按标题理解任务；不清楚的地方先在卡片上评论提问）',
    ];
    if (recent) lines.push('', '## 最近动态', recent);
    lines.push(
      '',
      '## 怎么做',
      '你是通过 Board CLI 协作的 agent。所有命令都加 `--as <你的名字>`（或设置 `BOARD_AS`），并在同一个白板上执行：',
      '',
      '```sh',
      `export ${env}`,
    );
    if (c.column === 'todo') lines.push(`board claim ${c.id} --json          # 先领取；只有领取后才算「进行中」`);
    else if (c.column === 'doing') lines.push(`board claim ${c.id} --json          # 如果你就是持有者，这会续期；否则会被拒绝`);
    lines.push(
      `board comment ${c.id} "<进展>"       # 干活期间记录进度，同时续期 30 分钟`,
      `board move ${c.id} review           # 做完，交给人审核`,
      '```',
      '',
      `做不下去时：先 \`board comment ${c.id} "<原因>"\`，再 \`board move ${c.id} todo\`。`,
      '规则：不要把卡片移进「已完成」，那由人确认；一个 agent 同时只能持有一张卡片；30 分钟不续期会被自动释放。',
    );
    if (c.column === 'review' || c.column === 'done') lines.push('', `注意：这张卡片已经在「${COLUMN_NAME[c.column]}」，通常不需要再处理，除非有人在动态里要求返工。`);
    return lines.filter(l => l != null).join('\n');
  }

  /* ------------------------------ 白板 ------------------------------ */
  function boardPrompt(path = store.current) {
    const board = store.boardOf(path), meta = store.boardMetaOf(path);
    if (!board) return null;
    const ln = c => { const l = labelNames(c.labels, meta.project); return l.length ? ` [${l.join(', ')}]` : ''; };
    const todo = board.cols.todo.slice(0, 12).map((id, i) => `${i + 1}. #${id} ${board.cards[id].title}${ln(board.cards[id])}`).join('\n');
    const doing = board.cols.doing.map(id => `- #${id} ${board.cards[id].title}（${board.cards[id].holder}）`).join('\n');
    return [
      `# 你是「${meta.projectName} / ${meta.name}」的执行 agent`,
      '',
      meta.desc ? `这块白板：${meta.desc}\n` : null,
      '用 Board CLI 领取并完成卡片。先设置环境变量：',
      '',
      '```sh',
      `export BOARD=${path}`,
      'export BOARD_AS=<你的名字>',
      '```',
      '',
      '## 工作循环',
      '1. `board claim --json` 领取「待办」最上面的卡片（越靠上越优先）；返回 `EMPTY_TODO` 说明暂时没活',
      '2. 按卡片描述和验收标准干活，期间用 `board comment <ID> "<进展>"` 记录进度（同时续期 30 分钟）',
      '3. 做完执行 `board move <ID> review`，交给人审核',
      '4. 做不下去：先 `board comment <ID> "<原因>"`，再 `board move <ID> todo`',
      '5. 回到第 1 步',
      '',
      '规则：进入「进行中」只能靠 claim；不要移入「已完成」；一个 agent 同时只持有一张卡片；30 分钟不续期会被自动释放。',
      '',
      `## 当前待办（${board.cols.todo.length} 张，按优先级）`,
      todo || '（空）',
      doing ? `\n## 正在进行\n${doing}` : null,
    ].filter(l => l != null).join('\n');
  }

  /* ------------------------------ 复制 / 预览 ------------------------------ */
  async function copy(text, title) {
    if (!text) return;
    let ok = true;
    try { await navigator.clipboard.writeText(text); } catch { ok = false; }
    toast({
      type: ok ? 'success' : 'info',
      title: ok ? `已复制为提示词` : '浏览器不允许直接写剪贴板',
      text: ok ? `${title} · ${text.length} 字，可以直接粘贴给 agent` : '在预览里手动复制',
      action: { label: '预览', fn: () => preview(text, title) },
      duration: ok ? 4200 : 8000,
    });
    if (!ok) preview(text, title);
  }

  function preview(text, title) {
    let w = $('#promptModal');
    if (!w) {
      w = el(`<div class="dialog-wrap" id="promptModal" aria-hidden="true">
        <div class="dialog pm" role="dialog" aria-modal="true" aria-labelledby="pmTitle">
          <div class="lbm-head"><h3 id="pmTitle">提示词</h3><span class="lbm-sub"></span><button type="button" class="icon-btn" data-close aria-label="关闭">${icons.close}</button></div>
          <textarea class="pm-text scroll" spellcheck="false" aria-label="提示词内容"></textarea>
          <div class="dialog-actions"><button class="btn" type="button" data-close>关闭</button><button class="btn brand" type="button" data-copy>${icons.copy}复制</button></div>
        </div>
      </div>`);
      document.body.appendChild(w);
      w.addEventListener('click', async e => {
        if (e.target === w || e.target.closest('[data-close]')) { w.classList.remove('on'); w.setAttribute('aria-hidden', 'true'); }
        if (e.target.closest('[data-copy]')) {
          const ta = w.querySelector('textarea');
          try { await navigator.clipboard.writeText(ta.value); } catch { ta.select(); document.execCommand('copy'); }
          toast({ type: 'success', title: '已复制', duration: 1400 });
        }
      });
      w.addEventListener('keydown', e => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); w.classList.remove('on'); w.setAttribute('aria-hidden', 'true'); } });
    }
    w.querySelector('.lbm-sub').textContent = `${title} · 可以先改再复制`;
    w.querySelector('textarea').value = text;
    w.classList.add('on'); w.setAttribute('aria-hidden', 'false');
    setTimeout(() => w.querySelector('textarea').focus(), 40);
  }

  B.prompt = {
    init(s) { store = s; },
    cardPrompt, boardPrompt, preview,
    copyCard(id, path = store.current) { copy(cardPrompt(id, path), `#${id}`); },
    copyBoard(path = store.current) { const m = store.boardMetaOf(path); copy(boardPrompt(path), `${m.projectName} / ${m.name}`); },
  };
})(window.Board = window.Board || {});
