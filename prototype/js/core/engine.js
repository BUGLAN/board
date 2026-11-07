/* ==========================================================================
   Board · Engine（纯逻辑，不碰 DOM）
   apply(state, cmd, now) → { state, result, events }
   - 输入 state 不会被修改；返回新 state
   - 违反规则抛出 BoardError(code)
   这一层可以原样搬进正式实现（Node / Deno / 浏览器均可）。
   ========================================================================== */
(function (B) {
  'use strict';
  const { COLUMN_KEYS, ALL_KEYS, COLUMN_NAME, CLAIM_MS, BoardError, SYSTEM } = B.schema;

  function createEmpty() {
    return {
      v: 1,
      rev: 0,       // 每次写入 +1，界面用来判断是否需要刷新
      seq: 0,       // 卡片自增 ID
      aseq: 0,      // 动态条目自增 ID
      logSeq: 0,    // 全局事件流 ID
      cards: {},
      cols: { inbox: [], todo: [], doing: [], review: [], done: [] },
      log: [],      // 最近的全局事件（界面的「实时动态」与跨窗口高亮用）
    };
  }

  const clone = s => (typeof structuredClone === 'function' ? structuredClone(s) : JSON.parse(JSON.stringify(s)));
  const isAgent = a => a && a.kind === 'agent';
  const isHuman = a => a && a.kind === 'human';
  const pickActor = a => ({ kind: a.kind, name: a.name });

  function apply(prev, cmd, now) {
    const s = clone(prev);
    const events = [];
    const actor = cmd.actor;

    /* ---------- helpers ---------- */
    const get = id => {
      const c = s.cards[Number(id)];
      if (!c) throw new BoardError('NOT_FOUND', `卡片 #${id} 不存在`);
      return c;
    };
    const detach = c => {
      const list = s.cols[c.column];
      const i = list.indexOf(c.id);
      if (i >= 0) list.splice(i, 1);
      return i;
    };
    const insert = (c, col, pos) => {
      const list = s.cols[col];
      let i = pos === 'top' ? 0 : pos === 'bottom' || pos == null ? list.length : Math.max(0, Math.min(list.length, pos));
      list.splice(i, 0, c.id);
      c.column = col;
    };
    const heldBy = name => s.cols.doing.map(id => s.cards[id]).find(c => c && c.holder === name);
    const track = (c, who, action, text, extra) => {
      const entry = { id: ++s.aseq, at: now, actor: pickActor(who), kind: action === 'comment' ? 'comment' : 'system', action, text, ...(extra || {}) };
      c.activity.push(entry);
      c.updatedAt = now;
      const ev = { id: ++s.logSeq, at: now, actor: pickActor(who), op: action, cardId: c.id, title: c.title, text };
      s.log.push(ev);
      if (s.log.length > 80) s.log.splice(0, s.log.length - 80);
      events.push(ev);
      return entry;
    };
    const renew = c => { c.expiresAt = now + CLAIM_MS; };
    const clearClaim = c => { c.holder = null; c.expiresAt = null; c.claimedAt = null; };
    const isHolder = c => isAgent(actor) && c.holder === actor.name;
    const hhmm = t => new Date(t).toTimeString().slice(0, 5);

    let result = null;

    switch (cmd.op) {
      /* ---------------- 新建 ---------------- */
      case 'new': {
        const title = String(cmd.title || '').trim();
        if (!title) throw new BoardError('EMPTY_TITLE');
        // 默认进「待办」；column: 'inbox' 时作为未整理条目
        const col = cmd.column === 'inbox' ? 'inbox' : 'todo';
        const c = {
          id: ++s.seq, title, body: String(cmd.body || '').trim(),
          column: col, holder: null, expiresAt: null, claimedAt: null,
          labels: [...new Set(cmd.labels || [])],
          createdAt: now, updatedAt: now, activity: [],
        };
        s.cards[c.id] = c;
        insert(c, col, cmd.top ? 'top' : 'bottom');
        track(c, actor, 'create', col === 'inbox' ? '记下了一条未整理的条目' : cmd.top ? '新建了卡片，放在「待办」最上面' : '新建了卡片');
        result = c;
        break;
      }

      /* ---------------- 编辑 ---------------- */
      case 'edit': {
        const c = get(cmd.id);
        if (c.column === 'doing' && !isHolder(c)) {
          throw new BoardError('NOT_HOLDER', `#${c.id} 在「进行中」，只有持有者 ${c.holder} 能编辑`, { holder: c.holder });
        }
        const changes = [];
        if (cmd.title != null) {
          const t = String(cmd.title).trim();
          if (!t) throw new BoardError('EMPTY_TITLE');
          if (t !== c.title) { changes.push(['title', c.title, t]); c.title = t; }
        }
        if (cmd.body != null) {
          const b = String(cmd.body).trim();
          if (b !== c.body) { changes.push(['body']); c.body = b; }
        }
        if (isHolder(c)) renew(c);
        for (const ch of changes) {
          if (ch[0] === 'title') track(c, actor, 'edit', `把标题从「${ch[1]}」改为「${ch[2]}」`);
          else track(c, actor, 'edit', '修改了描述');
        }
        result = c;
        break;
      }

      /* ---------------- 移动 / 排序 ---------------- */
      case 'move': {
        const c = get(cmd.id);
        const to = cmd.column;
        const from = c.column;
        if (!ALL_KEYS.includes(to)) throw new BoardError('USAGE', `未知的列「${to}」，可选：${ALL_KEYS.join(' / ')}`);

        // 规则 5：进行中的卡片只有持有者能移动
        if (from === 'doing' && !isHolder(c)) {
          throw new BoardError('NOT_HOLDER',
            isHuman(actor)
              ? `#${c.id} 由 ${c.holder} 持有，只有持有者能移动。需要的话，打开卡片「强制释放」。`
              : `#${c.id} 由 ${c.holder} 持有，你不是持有者`,
            { holder: c.holder });
        }
        // 规则 4：进入「进行中」只能靠领取
        if (to === 'doing' && from !== 'doing') {
          throw new BoardError('FORBIDDEN_COLUMN', '卡片只能由 agent 通过 board claim 领取进入「进行中」');
        }
        // 规则 9：「已完成」只归人管
        if (isAgent(actor) && (to === 'done' || from === 'done')) {
          throw new BoardError('FORBIDDEN_COLUMN', to === 'done' ? '只有人能把卡片移入「已完成」' : '只有人能把卡片移出「已完成」');
        }

        const oldIndex = s.cols[from].indexOf(c.id);
        detach(c);
        const pos = cmd.index != null ? cmd.index : cmd.top ? 'top' : 'bottom';
        insert(c, to, pos);
        const newIndex = s.cols[to].indexOf(c.id);

        if (from !== to) {
          // 规则 8：离开「进行中」即释放
          const releasedFrom = from === 'doing' ? c.holder : null;
          if (from === 'doing') clearClaim(c);
          const text = from === 'inbox' ? `整理好了，从「未整理」移到「${COLUMN_NAME[to]}」`
            : `从「${COLUMN_NAME[from]}」移到「${COLUMN_NAME[to]}」${releasedFrom ? '，领取已释放' : ''}`;
          track(c, actor, 'move', text, { from, to });
        } else if (oldIndex !== newIndex) {
          if (isHolder(c)) renew(c);
          track(c, actor, 'reorder', `调整了在「${COLUMN_NAME[to]}」中的顺序（第 ${oldIndex + 1} → ${newIndex + 1} 位）`, { from, to });
        }
        result = c;
        break;
      }

      /* ---------------- 领取 ---------------- */
      case 'claim': {
        if (!isAgent(actor)) throw new BoardError('USAGE', '只有 agent 能通过 CLI 领取卡片');
        const mine = heldBy(actor.name);
        let c;
        if (cmd.id == null) {
          if (mine) throw new BoardError('ALREADY_HOLDING', `你已经持有 #${mine.id}，一个 agent 同时只能持有一张卡片`, { holding: mine.id });
          if (!s.cols.todo.length) throw new BoardError('EMPTY_TODO');
          c = s.cards[s.cols.todo[0]];
        } else {
          c = get(cmd.id);
          // 再次 claim 自己持有的卡片 = 续期（规则 6）
          if (c.column === 'doing' && c.holder === actor.name) {
            renew(c);
            track(c, actor, 'renew', `重新领取，到期时间续至 ${hhmm(c.expiresAt)}`);
            result = c;
            break;
          }
          if (c.column !== 'todo') throw new BoardError('NOT_IN_TODO', `#${c.id} 在「${COLUMN_NAME[c.column]}」，不在「待办」`);
          if (mine) throw new BoardError('ALREADY_HOLDING', `你已经持有 #${mine.id}，一个 agent 同时只能持有一张卡片`, { holding: mine.id });
        }
        detach(c);
        insert(c, 'doing', 'bottom');
        c.holder = actor.name;
        c.claimedAt = now;
        renew(c);
        track(c, actor, 'claim', `领取了卡片，到期 ${hhmm(c.expiresAt)}`);
        result = c;
        break;
      }

      /* ---------------- 标签 ----------------
         标签是整理用的元数据，人和 agent 都能改，「进行中」的卡片也可以（不受持有者独占限制）。
         cmd: { id, add: [labelId], remove: [labelId], names: { labelId: 名称 } }  */
      case 'label': {
        const c = get(cmd.id);
        const cur = new Set(c.labels || []);
        const added = (cmd.add || []).filter(l => !cur.has(l));
        const removed = (cmd.remove || []).filter(l => cur.has(l));
        added.forEach(l => cur.add(l));
        removed.forEach(l => cur.delete(l));
        c.labels = [...cur];
        const nm = l => `「${(cmd.names && cmd.names[l]) || l}」`;
        if (added.length) track(c, actor, 'label', `添加了标签 ${added.map(nm).join('')}`);
        if (removed.length) track(c, actor, 'label', `移除了标签 ${removed.map(nm).join('')}`);
        result = c;
        break;
      }

      /* ---------------- 评论 ---------------- */
      case 'comment': {
        const c = get(cmd.id);
        const text = String(cmd.text || '').trim();
        if (!text) throw new BoardError('USAGE', '评论内容不能为空');
        if (isHolder(c)) renew(c);   // 规则 6：持有者评论即续期
        track(c, actor, 'comment', text);
        result = c;
        break;
      }

      /* ---------------- 强制释放（仅人） ---------------- */
      case 'release': {
        if (!isHuman(actor)) throw new BoardError('USAGE', '只有人能在界面上强制释放');
        const c = get(cmd.id);
        if (c.column !== 'doing') throw new BoardError('USAGE', `#${c.id} 不在「进行中」`);
        const who = c.holder;
        detach(c);
        clearClaim(c);
        insert(c, 'todo', 'top');
        track(c, actor, 'release', `强制释放了 ${who} 的领取，卡片回到「待办」最上面`, { holder: who });
        result = c;
        break;
      }

      /* ---------------- 删除（仅人） ---------------- */
      case 'delete': {
        if (!isHuman(actor)) throw new BoardError('USAGE', '删除只能在界面上做');
        const c = get(cmd.id);
        detach(c);
        delete s.cards[c.id];
        const ev = { id: ++s.logSeq, at: now, actor: pickActor(actor), op: 'delete', cardId: c.id, title: c.title, text: '删除了卡片' };
        s.log.push(ev);
        events.push(ev);
        result = c;
        break;
      }

      /* ---------------- 超时扫描（规则 7） ---------------- */
      case 'sweep': {
        const expired = s.cols.doing
          .map(id => s.cards[id])
          .filter(c => c.expiresAt != null && c.expiresAt <= now)
          .sort((a, b) => b.expiresAt - a.expiresAt); // 最早到期的最后插入 → 位于最上面
        for (const c of expired) {
          const who = c.holder;
          detach(c);
          clearClaim(c);
          insert(c, 'todo', 'top');
          track(c, SYSTEM, 'timeout', `领取超时，自动释放（原持有者 ${who}）`, { holder: who });
        }
        result = expired;
        break;
      }

      default:
        throw new BoardError('USAGE', `未知操作 ${cmd.op}`);
    }

    if (events.length || cmd.op !== 'sweep') s.rev++;
    return { state: s, result, events };
  }

  /** 只读查询 */
  const query = {
    list(s, { column, search, label } = {}) {
      const cols = column ? [column] : ALL_KEYS;
      const q = (search || '').trim().toLowerCase();
      const out = [];
      for (const col of cols) {
        for (const id of s.cols[col] || []) {
          const c = s.cards[id];
          if (!c) continue;
          if (label && !(c.labels || []).includes(label)) continue;
          if (q && !(`${c.title}\n${c.body}`.toLowerCase().includes(q))) continue;
          out.push(c);
        }
      }
      return out;
    },
    heldBy(s, name) {
      return s.cols.doing.map(id => s.cards[id]).find(c => c && c.holder === name) || null;
    },
    holders(s) {
      return s.cols.doing.map(id => s.cards[id]).filter(Boolean).map(c => ({ name: c.holder, card: c }));
    },
  };

  B.engine = { createEmpty, apply, query };
})(window.Board = window.Board || {});
