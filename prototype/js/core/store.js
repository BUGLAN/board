/* ==========================================================================
   Board · Store / Transport / Clock
   - 工作区（workspace）= 多个项目；一个项目有多块白板（1:N），每块白板有名字和标识。
       { v, rev, order: [pkey],
         projects: { pkey: { key, name, desc, color, createdAt, pos,
                             boardOrder: [bkey], boards: { bkey: { key, name, desc, createdAt, pos, board } } } } }
     白板用路径定位：「项目/白板」，例如 oss/release。CLI 用 --board 或环境变量 BOARD。
     engine 只认识单块白板；Store 负责把命令路由到指定的白板。
   - Store：store.state 是「当前白板」（每个窗口可以看不同的白板），所有写操作经 engine.apply，并广播变更
   - Transport：持久化与同步的适配层。原型用 LocalTransport（localStorage +
     storage 事件，多窗口实时同步）；接入真实后端时实现同样的接口即可：
         read()           → state | null
         write(state)     → void
         subscribe(fn)    → unsubscribe     fn(state) 在远端写入时调用
     例如 HttpTransport：read/write 走 REST，subscribe 用 SSE / WebSocket，
     或每 1s 轮询一次 GET /board?since=rev（满足 PRD「2 秒内显示」）。
   - Clock：演示用的可加速时钟（×1 / ×10 / ×60），让 30 分钟超时能在几十秒内看到
   ========================================================================== */
(function (B) {
  'use strict';
  const { engine, schema } = B;

  /* ------------------------------- Clock ------------------------------- */
  const CLOCK_KEY = 'board.prototype.clock';
  class Clock {
    constructor() {
      this.listeners = new Set();
      this.cfg = this._load() || { speed: 1, vBase: Date.now(), rBase: Date.now() };
      window.addEventListener('storage', e => {
        if (e.key === CLOCK_KEY && e.newValue) { this.cfg = JSON.parse(e.newValue); this._emit(); }
      });
    }
    _load() { try { return JSON.parse(localStorage.getItem(CLOCK_KEY)); } catch { return null; } }
    _save() { try { localStorage.setItem(CLOCK_KEY, JSON.stringify(this.cfg)); } catch {} }
    _emit() { this.listeners.forEach(fn => fn(this)); }
    now() { return Math.round(this.cfg.vBase + (Date.now() - this.cfg.rBase) * this.cfg.speed); }
    get speed() { return this.cfg.speed; }
    setSpeed(speed) {
      const v = this.now();
      this.cfg = { speed, vBase: v, rBase: Date.now() };
      this._save(); this._emit();
    }
    /** 把虚拟时间换算为真实毫秒（agent 模拟等使用） */
    toReal(ms) { return ms / this.cfg.speed; }
    onChange(fn) { this.listeners.add(fn); return () => this.listeners.delete(fn); }
  }

  /* ----------------------------- Transport ----------------------------- */
  const STATE_KEY = 'board.prototype.workspace.v5';
  class LocalTransport {
    read() { try { return JSON.parse(localStorage.getItem(STATE_KEY)); } catch { return null; } }
    write(state) { try { localStorage.setItem(STATE_KEY, JSON.stringify(state)); } catch (e) { console.warn('[board] 持久化失败', e); } }
    subscribe(fn) {
      const h = e => { if (e.key === STATE_KEY && e.newValue) fn(JSON.parse(e.newValue)); };
      window.addEventListener('storage', h);
      return () => window.removeEventListener('storage', h);
    }
    reset() { try { localStorage.removeItem(STATE_KEY); } catch {} }
  }

  /* ------------------------------- Store ------------------------------- */
  const CURRENT_KEY = 'board.current';
  const PROJECT_COLORS = ['#FF6A33', '#4DB5FF', '#3DDC97', '#B98CFF', '#FFB020', '#FF5C8A', '#2DD4BF', '#94A3B8'];
  const LABEL_COLORS = ['#F43F5E', '#F97316', '#EAB308', '#22C55E', '#14B8A6', '#3B82F6', '#8B5CF6', '#EC4899', '#64748B'];
  const KEY_RE = /^[a-z0-9][a-z0-9-]{0,31}$/;
  const ss = { get: k => { try { return sessionStorage.getItem(k) || localStorage.getItem(k); } catch { return null; } },
               set: (k, v) => { try { sessionStorage.setItem(k, v); localStorage.setItem(k, v); } catch {} } };
  const { BoardError } = schema;
  const split = path => { const i = String(path || '').indexOf('/'); return i < 0 ? [path, null] : [path.slice(0, i), path.slice(i + 1)]; };

  /** 新节点放在画布上的位置：项目在左列，白板在项目右侧依次往下排 */
  const LAYOUT = { projectX: 0, boardX: 420, projectGap: 320, boardGap: 200 };

  class Store {
    constructor({ transport, clock }) {
      this.transport = transport;
      this.clock = clock;
      this.listeners = new Set();
      let ws = transport.read();
      if (!ws || ws.v !== 5) { ws = seedWorkspace(clock.now()); transport.write(ws); }
      this.ws = ws;
      const want = ss.get(CURRENT_KEY);
      this.current = this.boardOf(want) ? want : this.paths()[0];
      this.lastSeq = Object.fromEntries(this.paths().map(p => [p, this.boardOf(p).logSeq]));
      transport.subscribe(next => this._receive(next));
    }

    /* ---------- 读 ---------- */
    paths(ws = this.ws) { return ws.order.flatMap(k => ws.projects[k].boardOrder.map(b => `${k}/${b}`)); }
    boardOf(path, ws = this.ws) { const [p, b] = split(path); return ws.projects[p]?.boards?.[b]?.board || null; }
    get state() { return this.boardOf(this.current); }
    get projectKey() { return split(this.current)[0]; }
    get project() { return this.projectMeta(this.projectKey); }
    get boardMeta() { return this.boardMetaOf(this.current); }

    boardMetaOf(path) {
      const [pk, bk] = split(path);
      const p = this.ws.projects[pk], b = p?.boards?.[bk];
      if (!b) return null;
      const st = b.board, n = c => st.cols[c].length;
      return {
        path: `${pk}/${bk}`, key: bk, name: b.name, desc: b.desc || '', createdAt: b.createdAt, pos: b.pos,
        project: pk, projectName: p.name, color: p.color,
        counts: { inbox: n('inbox'), todo: n('todo'), doing: n('doing'), review: n('review'), done: n('done') },
        holders: st.cols.doing.map(id => st.cards[id]?.holder).filter(Boolean),
        updatedAt: st.log.length ? st.log[st.log.length - 1].at : b.createdAt,
      };
    }
    projectMeta(key) {
      const p = this.ws.projects[key]; if (!p) return null;
      const boards = p.boardOrder.map(b => this.boardMetaOf(`${key}/${b}`));
      const sum = c => boards.reduce((a, b) => a + b.counts[c], 0);
      return {
        key: p.key, name: p.name, desc: p.desc || '', color: p.color, createdAt: p.createdAt, pos: p.pos, boards,
        labels: (p.labels || []).map(l => ({ ...l })),
        counts: { inbox: sum('inbox'), todo: sum('todo'), doing: sum('doing'), review: sum('review'), done: sum('done') },
        holders: boards.flatMap(b => b.holders),
      };
    }
    get projects() { return this.ws.order.map(k => this.projectMeta(k)); }
    get boards() { return this.paths().map(p => this.boardMetaOf(p)); }

    /**
     * CLI 用：把 --board / BOARD 的值解析成白板路径。
     * 写成「项目/白板」；项目只有一块白板时也可以只写项目标识。
     */
    resolve(raw) {
      raw = String(raw || '').trim();
      if (!raw) throw new BoardError('NO_BOARD');
      const [pk, bk] = split(raw);
      const p = this.ws.projects[pk];
      if (!p) throw new BoardError('BOARD_NOT_FOUND', `项目「${pk}」不存在，用 board boards 查看可用的白板`);
      if (bk == null) {
        if (p.boardOrder.length === 1) return `${pk}/${p.boardOrder[0]}`;
        throw new BoardError('BOARD_NOT_FOUND', `项目「${pk}」有 ${p.boardOrder.length} 块白板，请写成 ${pk}/<白板>：${p.boardOrder.join('、')}`);
      }
      if (!p.boards[bk]) throw new BoardError('BOARD_NOT_FOUND', `项目「${pk}」里没有白板「${bk}」，可选：${p.boardOrder.join('、') || '（空）'}`);
      return `${pk}/${bk}`;
    }

    subscribe(fn) { this.listeners.add(fn); return () => this.listeners.delete(fn); }
    _emit(meta) { this.listeners.forEach(fn => fn(this.state, meta)); }

    /** 把一组事件按「当前白板 / 其他白板」拆开 */
    _split(perBoard) {
      const events = perBoard[this.current] || [];
      const others = [];
      for (const [k, evs] of Object.entries(perBoard)) if (k !== this.current) evs.forEach(e => others.push({ ...e, board: k }));
      return { events, others };
    }

    _receive(next) {
      if (!next || next.v !== 5 || next.rev === this.ws.rev) return;
      const per = {};
      for (const path of this.paths(next)) {
        const b = this.boardOf(path, next);
        per[path] = (b.log || []).filter(e => e.id > (this.lastSeq[path] ?? Infinity));
        this.lastSeq[path] = b.logSeq;
      }
      const prev = this.state;
      this.ws = next;
      if (!this.boardOf(this.current)) { this.current = this.paths()[0]; this._emit({ source: 'switch', events: [], others: [], prev, structure: true }); return; }
      const { events, others } = this._split(per);
      this._emit({ source: 'remote', events, others, prev, structure: true });
    }

    _base() { return this.transport.read() || this.ws; }
    _write(ws) { ws.rev = (ws.rev || 0) + 1; this.transport.write(ws); this.ws = ws; }
    _clone(base) { return JSON.parse(JSON.stringify(base)); }

    /**
     * 执行一个命令。失败抛 BoardError，state 不变。
     * @param {object} cmd  { op, actor, ...args }
     * @param {object} [opt] { via: 'ui' | 'cli' | 'system', board: 路径（默认当前白板） }
     */
    dispatch(cmd, opt = {}) {
      // 先取最新（另一个窗口可能刚写过），再在其上应用 —— 与服务端串行处理同构
      const base = this._base();
      const path = opt.board || this.current;
      const cur = this.boardOf(path, base);
      if (!cur) throw new BoardError('BOARD_NOT_FOUND', `白板「${path}」不存在`);
      const { state, result, events } = engine.apply(cur, cmd, this.clock.now());
      if (cmd.op === 'sweep' && !events.length) return result;
      events.forEach(e => (e.via = opt.via || 'ui'));
      const prev = this.state;
      const [pk, bk] = split(path);
      const ws = { ...base, projects: { ...base.projects, [pk]: { ...base.projects[pk], boards: { ...base.projects[pk].boards, [bk]: { ...base.projects[pk].boards[bk], board: state } } } } };
      this._write(ws);
      this.lastSeq[path] = state.logSeq;
      const { events: mine, others } = this._split({ [path]: events });
      this._emit({ source: 'local', via: opt.via || 'ui', board: path, foreign: path !== this.current, events: mine, others, prev, op: cmd.op, result });
      return result;
    }

    /** 尝试执行，返回 { ok, result, error } 而不抛异常 */
    try(cmd, opt) {
      try { return { ok: true, result: this.dispatch(cmd, opt) }; }
      catch (error) { if (!(error instanceof BoardError)) console.error(error); return { ok: false, error }; }
    }

    /** 规则 7：所有白板一起扫描超时 */
    sweep() { for (const p of this.paths()) this.try({ op: 'sweep', actor: schema.SYSTEM }, { via: 'system', board: p }); }

    /* ---------- 切换 ---------- */
    setBoard(path) {
      if (!this.boardOf(path) || path === this.current) return;
      const prev = this.state;
      this.current = path;
      ss.set(CURRENT_KEY, path);
      this._emit({ source: 'switch', events: [], others: [], prev });
    }
    setProject(key) {
      const p = this.ws.projects[key];
      if (p && p.boardOrder.length) this.setBoard(`${key}/${p.boardOrder[0]}`);
    }

    /* ---------- 项目 / 白板的增删改 ---------- */
    static slug(name) { return String(name || '').toLowerCase().trim().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 32); }
    validateProjectKey(key) {
      if (!KEY_RE.test(key)) return '标识只能用小写字母、数字和 -，以字母或数字开头';
      if (this.ws.projects[key]) return `项目标识「${key}」已被占用`;
      return null;
    }
    validateBoardKey(pk, key) {
      if (!KEY_RE.test(key)) return '标识只能用小写字母、数字和 -，以字母或数字开头';
      if (this.ws.projects[pk]?.boards[key]) return `「${pk}」里已经有标识为「${key}」的白板`;
      return null;
    }
    _structure(meta = {}) { this._emit({ source: 'structure', events: [], others: [], structure: true, ...meta }); }
    _need(name, what) { name = String(name || '').trim(); if (!name) throw new BoardError('EMPTY_TITLE', `${what}名称不能为空`); return name; }

    createProject({ name, key, desc = '', color, pos, board }) {
      name = this._need(name, '项目');
      const err = this.validateProjectKey(key); if (err) throw new BoardError('USAGE', err);
      const ws = this._clone(this._base());
      const ys = ws.order.map(k => ws.projects[k].pos.y);
      const p = {
        key, name, desc: String(desc).trim(), createdAt: this.clock.now(),
        color: color || PROJECT_COLORS[ws.order.length % PROJECT_COLORS.length],
        pos: pos || { x: LAYOUT.projectX, y: (ys.length ? Math.max(...ys) : -LAYOUT.projectGap) + LAYOUT.projectGap },
        boardOrder: [], boards: {},
      };
      ws.order.push(key); ws.projects[key] = p;
      if (board) {
        const bk = board.key || 'main';
        p.boardOrder.push(bk);
        p.boards[bk] = { key: bk, name: this._need(board.name, '白板'), desc: '', createdAt: p.createdAt, pos: { x: p.pos.x + LAYOUT.boardX, y: p.pos.y - 10 }, board: engine.createEmpty() };
        this.lastSeq[`${key}/${bk}`] = 0;
      }
      this._write(ws);
      this._structure({ created: { project: key } });
      return this.projectMeta(key);
    }

    createBoard(pk, { name, key, desc = '', pos }) {
      name = this._need(name, '白板');
      const err = this.validateBoardKey(pk, key); if (err) throw new BoardError('USAGE', err);
      const ws = this._clone(this._base());
      const p = ws.projects[pk]; if (!p) throw new BoardError('BOARD_NOT_FOUND', `项目「${pk}」不存在`);
      const ys = p.boardOrder.map(b => p.boards[b].pos.y);
      p.boardOrder.push(key);
      p.boards[key] = {
        key, name, desc: String(desc).trim(), createdAt: this.clock.now(),
        pos: pos || { x: p.pos.x + LAYOUT.boardX, y: ys.length ? Math.max(...ys) + LAYOUT.boardGap : p.pos.y - 10 },
        board: engine.createEmpty(),
      };
      this.lastSeq[`${pk}/${key}`] = 0;
      this._write(ws);
      this._structure({ created: { board: `${pk}/${key}` } });
      return this.boardMetaOf(`${pk}/${key}`);
    }

    updateProject(pk, patch) {
      const ws = this._clone(this._base()); const p = ws.projects[pk]; if (!p) return;
      if (patch.name != null) p.name = this._need(patch.name, '项目');
      if (patch.desc != null) p.desc = String(patch.desc).trim();
      if (patch.color) p.color = patch.color;
      this._write(ws); this._structure();
    }
    updateBoard(path, patch) {
      const [pk, bk] = split(path);
      const ws = this._clone(this._base()); const b = ws.projects[pk]?.boards[bk]; if (!b) return;
      if (patch.name != null) b.name = this._need(patch.name, '白板');
      if (patch.desc != null) b.desc = String(patch.desc).trim();
      this._write(ws); this._structure();
    }

    /* ---------- 标签（属于项目，项目下所有白板共用） ---------- */
    labelsOf(pk = this.projectKey) { return (this.ws.projects[pk]?.labels || []).map(l => ({ ...l })); }
    /** 每个标签在项目里被多少张卡片使用 */
    labelUsage(pk = this.projectKey) {
      const n = {};
      const p = this.ws.projects[pk]; if (!p) return n;
      for (const bk of p.boardOrder) for (const c of Object.values(p.boards[bk].board.cards)) (c.labels || []).forEach(l => (n[l] = (n[l] || 0) + 1));
      return n;
    }
    findLabel(name, pk = this.projectKey) {
      const q = String(name || '').trim().toLowerCase();
      return this.labelsOf(pk).find(l => l.name.toLowerCase() === q || l.id === q) || null;
    }
    createLabel(pk, { name, color }) {
      name = String(name || '').trim();
      if (!name) throw new BoardError('EMPTY_TITLE', '标签名称不能为空');
      if (this.findLabel(name, pk)) throw new BoardError('USAGE', `标签「${name}」已存在`);
      const ws = this._clone(this._base()); const p = ws.projects[pk];
      p.labels = p.labels || [];
      let id = Store.slug(name) || 'label', n = 1;
      while (p.labels.some(l => l.id === id)) id = `${Store.slug(name) || 'label'}-${++n}`;
      const label = { id, name, color: color || LABEL_COLORS[p.labels.length % LABEL_COLORS.length] };
      p.labels.push(label);
      this._write(ws); this._structure({ labels: true });
      return label;
    }
    updateLabel(pk, id, patch) {
      const ws = this._clone(this._base()); const l = (ws.projects[pk]?.labels || []).find(x => x.id === id); if (!l) return;
      if (patch.name != null) {
        const n = String(patch.name).trim();
        if (!n) throw new BoardError('EMPTY_TITLE', '标签名称不能为空');
        if (ws.projects[pk].labels.some(x => x.id !== id && x.name.toLowerCase() === n.toLowerCase())) throw new BoardError('USAGE', `标签「${n}」已存在`);
        l.name = n;
      }
      if (patch.color) l.color = patch.color;
      this._write(ws); this._structure({ labels: true });
    }
    /** 删除标签：同时从项目内所有卡片上移除 */
    deleteLabel(pk, id) {
      const ws = this._clone(this._base()); const p = ws.projects[pk]; if (!p) return;
      p.labels = (p.labels || []).filter(l => l.id !== id);
      for (const bk of p.boardOrder) for (const c of Object.values(p.boards[bk].board.cards)) if (c.labels) c.labels = c.labels.filter(l => l !== id);
      this._write(ws); this._structure({ labels: true });
    }
    /** 给卡片加 / 去标签（走 engine，会记入动态） */
    setLabels(cardId, { add = [], remove = [] }, opt = {}) {
      const path = opt.board || this.current;
      const names = Object.fromEntries(this.labelsOf(Store.split(path)[0]).map(l => [l.id, l.name]));
      return this.dispatch({ op: 'label', actor: opt.actor || schema.HUMAN, id: cardId, add, remove, names }, opt);
    }

    /** 画布上拖动节点：只保存位置 */
    setPos(kind, id, pos) {
      const ws = this._clone(this._base());
      const x = Math.round(pos.x), y = Math.round(pos.y);
      if (kind === 'project') { if (!ws.projects[id]) return; ws.projects[id].pos = { x, y }; }
      else { const [pk, bk] = split(id); const b = ws.projects[pk]?.boards[bk]; if (!b) return; b.pos = { x, y }; }
      this._write(ws);
      this._emit({ source: 'layout', events: [], others: [] });
    }

    deleteBoard(path) {
      const [pk, bk] = split(path);
      if (this.paths().length <= 1) throw new BoardError('USAGE', '至少要保留一块白板');
      const ws = this._clone(this._base());
      const p = ws.projects[pk]; if (!p?.boards[bk]) return;
      delete p.boards[bk]; p.boardOrder = p.boardOrder.filter(k => k !== bk);
      this._write(ws);
      delete this.lastSeq[path];
      this._afterRemove();
    }
    deleteProject(pk) {
      const ws = this._clone(this._base());
      if (!ws.projects[pk]) return;
      const remaining = this.paths(ws).filter(p => !p.startsWith(pk + '/'));
      if (!remaining.length) throw new BoardError('USAGE', '至少要保留一块白板');
      delete ws.projects[pk]; ws.order = ws.order.filter(k => k !== pk);
      this._write(ws);
      Object.keys(this.lastSeq).forEach(k => { if (k.startsWith(pk + '/')) delete this.lastSeq[k]; });
      this._afterRemove();
    }
    _afterRemove() {
      if (!this.boardOf(this.current)) {
        const prev = null; this.current = this.paths()[0]; ss.set(CURRENT_KEY, this.current);
        this._emit({ source: 'switch', events: [], others: [], prev, structure: true });
      } else this._structure();
    }

    reset() {
      this.transport.reset();
      const ws = seedWorkspace(this.clock.now());
      this.transport.write(ws); this.ws = ws;
      if (!this.boardOf(this.current)) this.current = this.paths()[0];
      this.lastSeq = Object.fromEntries(this.paths().map(p => [p, this.boardOf(p).logSeq]));
      this._emit({ source: 'switch', events: [], others: [], structure: true });
    }
  }
  Store.COLORS = PROJECT_COLORS;
  Store.LABEL_COLORS = LABEL_COLORS;
  Store.split = split;

  /* -------------------------------- Seed -------------------------------- */
  // 示例工作区：三个项目，共五块白板
  function seedWorkspace(now) {
    const H = 60 * 60 * 1000, D = 24 * H;
    const bd = (key, name, desc, pos, board, age) => ({ key, name, desc, pos, createdAt: now - age, board });
    const pj = (key, name, desc, color, pos, age, boards) => ({
      key, name, desc, color, pos, createdAt: now - age,
      boardOrder: boards.map(b => b.key), boards: Object.fromEntries(boards.map(b => [b.key, b])),
    });
    const projects = {
      oss: pj('oss', 'Board 开源仓库', '自动维护 board 的 GitHub 仓库', '#FF6A33', { x: 0, y: 40 }, 30 * D, [
        bd('main', '维护主线', 'issue、社区反馈与定期清理', { x: 420, y: -60 }, seedOss(now), 30 * D),
        bd('release', 'v1.0 发布', '发布前必须完成的事项', { x: 420, y: 160 }, seedRelease(now), 6 * D),
      ]),
      docs: pj('docs', '文档站', 'board 的使用文档与示例', '#4DB5FF', { x: 0, y: 470 }, 12 * D, [
        bd('content', '内容', '命令参考、快速开始、错误码', { x: 420, y: 380 }, seedDocs(now), 12 * D),
        bd('i18n', '英文翻译', '', { x: 420, y: 590 }, seedI18n(now), 4 * D),
      ]),
      site: pj('site', '官网改版', '新版首页与定价页，下周上线', '#3DDC97', { x: 0, y: 820 }, 3 * D, [
        bd('launch', '上线清单', '', { x: 420, y: 800 }, seedSite(now), 3 * D),
      ]),
    };
    // 示例标签
    const L = (id, name, color) => ({ id, name, color });
    projects.oss.labels = [L('bug', 'bug', '#F43F5E'), L('docs', '文档', '#3B82F6'), L('refactor', '重构', '#8B5CF6'), L('community', '社区', '#22C55E'), L('release', '发布', '#EAB308'), L('windows', 'Windows', '#14B8A6')];
    projects.docs.labels = [L('writing', '写作', '#3B82F6'), L('screenshot', '截图', '#F97316'), L('i18n', '翻译', '#14B8A6')];
    projects.site.labels = [L('design', '设计', '#EC4899'), L('copy', '文案', '#EAB308')];
    const tag = (board, map) => Object.entries(map).forEach(([id, ls]) => { if (board.cards[id]) board.cards[id].labels = ls; });
    tag(projects.oss.boards.main.board, { 1: ['community'], 2: ['bug', 'windows'], 3: ['bug'], 4: ['refactor'], 5: ['docs'], 6: ['community'], 7: ['refactor'], 8: ['bug', 'docs'] });
    tag(projects.oss.boards.release.board, { 1: ['release'], 2: ['release', 'windows'], 3: ['release', 'docs'], 4: ['release'] });
    tag(projects.docs.boards.content.board, { 1: ['writing'], 2: ['writing'], 4: ['screenshot'], 5: ['writing'] });
    tag(projects.docs.boards.i18n.board, { 1: ['i18n'], 2: ['i18n'] });
    tag(projects.site.boards.launch.board, { 1: ['design', 'copy'], 2: ['copy'], 3: ['design'] });
    return { v: 5, rev: 1, order: ['oss', 'docs', 'site'], projects };
  }

  function mini(now, steps) {
    let s = engine.createEmpty();
    for (const [cmd, at] of steps) s = engine.apply(s, cmd, now - at).state;
    return s;
  }
  function seedDocs(now) {
    const M = 60 * 1000, H = schema.HUMAN, A = schema.agent;
    return mini(now, [
      [{ op: 'new', actor: H, title: '命令参考：补全 `board move` 的 --top 说明', body: '' }, 300 * M],
      [{ op: 'new', actor: H, title: '快速开始：五分钟接入一个 agent', body: '**验收标准**\n- [ ] 从安装到第一次 `board claim` 不超过 5 步\n- [ ] 附一个 shell 循环示例' }, 280 * M],
      [{ op: 'new', actor: A('issue-watcher'), title: '错误码页面缺少 `EMPTY_TODO` 的说明', body: 'gh#77' }, 200 * M],
      [{ op: 'new', actor: H, title: '示例仓库截图过期，需要重截', body: '' }, 150 * M],
      [{ op: 'new', actor: H, title: '中文文档术语统一（领取 / 认领）', body: '' }, 120 * M],
      [{ op: 'claim', actor: A('alpha'), id: 1 }, 260 * M],
      [{ op: 'move', actor: A('alpha'), id: 1, column: 'review' }, 230 * M],
      [{ op: 'move', actor: H, id: 1, column: 'done' }, 200 * M],
      [{ op: 'claim', actor: A('gamma'), id: 2 }, 12 * M],
      [{ op: 'comment', actor: A('gamma'), id: 2, text: '初稿写好了，正在补 shell 循环示例。' }, 6 * M],
      [{ op: 'claim', actor: A('delta'), id: 3 }, 90 * M],
      [{ op: 'move', actor: A('delta'), id: 3, column: 'review' }, 70 * M],
      [{ op: 'new', actor: H, column: 'inbox', title: '要不要做一个交互式的命令演示？', body: '' }, 40 * M],
    ]);
  }
  function seedRelease(now) {
    const M = 60 * 1000, H = schema.HUMAN, A = schema.agent;
    return mini(now, [
      [{ op: 'new', actor: H, title: '冻结 CLI 参数，写进 CHANGELOG', body: '' }, 900 * M],
      [{ op: 'new', actor: H, title: '各平台安装包签名（macOS / Windows）', body: '' }, 800 * M],
      [{ op: 'new', actor: H, title: '发布公告草稿', body: '' }, 700 * M],
      [{ op: 'new', actor: H, title: '跑一遍完整的验收标准（PRD §10）', body: '**验收标准**\n- [ ] 11 条全部通过' }, 600 * M],
      [{ op: 'claim', actor: A('epsilon'), id: 2 }, 9 * M],
      [{ op: 'comment', actor: A('epsilon'), id: 2, text: 'macOS 公证已提交，等待 Apple 返回。' }, 3 * M],
      [{ op: 'claim', actor: A('delta'), id: 1 }, 500 * M],
      [{ op: 'move', actor: A('delta'), id: 1, column: 'review' }, 480 * M],
    ]);
  }
  function seedI18n(now) {
    const M = 60 * 1000, H = schema.HUMAN;
    return mini(now, [
      [{ op: 'new', actor: H, title: '翻译「快速开始」', body: '' }, 300 * M],
      [{ op: 'new', actor: H, title: '翻译错误码页面', body: '' }, 280 * M],
      [{ op: 'new', actor: H, column: 'inbox', title: '考虑接入机器翻译 + 人工校对', body: '' }, 60 * M],
    ]);
  }
  function seedSite(now) {
    const M = 60 * 1000, H = schema.HUMAN, A = schema.agent;
    return mini(now, [
      [{ op: 'new', actor: H, title: '首页首屏：一句话讲清「人和 agent 共用一块白板」', body: '' }, 600 * M],
      [{ op: 'new', actor: H, title: '定价页：开源版 / 团队版对比表', body: '' }, 500 * M],
      [{ op: 'new', actor: H, title: 'OG 图与 favicon', body: '' }, 400 * M],
      [{ op: 'claim', actor: A('beta'), id: 3 }, 380 * M],
      [{ op: 'move', actor: A('beta'), id: 3, column: 'review' }, 360 * M],
      [{ op: 'move', actor: H, id: 3, column: 'done' }, 300 * M],
      [{ op: 'new', actor: H, column: 'inbox', title: '客户案例页先放三个', body: '' }, 100 * M],
      [{ op: 'new', actor: H, column: 'inbox', title: '博客要不要迁到同一个域名', body: '' }, 50 * M],
    ]);
  }

  // 示例数据：开源项目自动维护场景（PRD §11）
  function seedOss(now) {
    const M = 60 * 1000;
    const H = schema.HUMAN, A = schema.agent;
    const issue = A('issue-watcher'), cleaner = A('cleaner');
    let s = engine.createEmpty();
    const run = (cmd, at) => { s = engine.apply(s, cmd, at).state; return s; };
    const t0 = now - 6 * 60 * M;

    run({ op: 'new', actor: issue, title: '`board ls --search` 支持匹配描述里的 gh 编号', body: 'gh#118\n\n**验收标准**\n- `board ls --search gh#118` 能找到描述里写了 gh#118 的卡片\n- 大小写不敏感' }, t0);
    run({ op: 'new', actor: issue, title: 'Windows 下路径分隔符导致快照测试失败', body: 'gh#140 · 来自 issue 监听\n\n在 Windows runner 上 `tests/snapshot.rs` 有 3 个用例失败，路径里出现了 `\\\\`。\n\n**验收标准**\n- [ ] 快照里统一使用 `/`\n- [ ] CI 增加 windows-latest' }, t0 + 5 * M);
    run({ op: 'new', actor: issue, title: '无网络时 `sync` 命令直接 panic', body: 'gh#152\n\n```\nthread \'main\' panicked at src/net.rs:88\n```\n\n应当给出可读的错误信息并以退出码 1 结束。' }, t0 + 9 * M);
    run({ op: 'new', actor: cleaner, title: '清理已废弃的 v1 配置解析代码', body: '`src/config/v1.rs` 自 0.9 起已无调用方。删除前确认 `cargo test` 全绿。' }, t0 + 20 * M);
    run({ op: 'new', actor: H, title: '文档：为每个命令补充 `--json` 输出示例', body: '参考 README 的「CLI」一节，每个命令给一段输出样例。' }, t0 + 30 * M);
    run({ op: 'new', actor: A('community-watcher'), title: '论坛反馈：`claim` 超时时间希望可配置', body: '来源：https://forum.example.org/t/claim-timeout/311\n\n先收集更多反馈，暂不处理。' }, t0 + 44 * M);
    run({ op: 'new', actor: cleaner, title: '升级 tokio 到 1.40，移除 `rt-multi-thread` 之外的 feature', body: '' }, t0 + 60 * M);
    run({ op: 'new', actor: issue, title: 'README 里的安装命令在 zsh 下报错', body: 'gh#161\n\n`curl ... | sh -s -- --to ~/.local/bin` 在 zsh 中 `~` 未展开。' }, t0 + 70 * M);

    // #1：已完成
    run({ op: 'claim', actor: A('alpha'), id: 1 }, t0 + 80 * M);
    run({ op: 'comment', actor: A('alpha'), id: 1, text: '已实现：搜索范围扩展到描述，补了 4 个测试。' }, t0 + 96 * M);
    run({ op: 'move', actor: A('alpha'), id: 1, column: 'review' }, t0 + 98 * M);
    run({ op: 'comment', actor: H, id: 1, text: '看过了，没问题 👍' }, t0 + 150 * M);
    run({ op: 'move', actor: H, id: 1, column: 'done' }, t0 + 151 * M);

    // #4：待审核
    run({ op: 'claim', actor: A('beta'), id: 4 }, t0 + 160 * M);
    run({ op: 'comment', actor: A('beta'), id: 4, text: '确认 `v1.rs` 无引用，已删除 412 行；cargo test 全绿。' }, t0 + 175 * M);
    run({ op: 'move', actor: A('beta'), id: 4, column: 'review' }, t0 + 176 * M);

    // #8：待审核
    run({ op: 'claim', actor: A('gamma'), id: 8 }, t0 + 200 * M);
    run({ op: 'comment', actor: A('gamma'), id: 8, text: '改为使用 `$HOME`，并在 CI 里加了 zsh / bash 两个 shell 的冒烟测试。' }, t0 + 214 * M);
    run({ op: 'move', actor: A('gamma'), id: 8, column: 'review' }, t0 + 215 * M);

    // 人调整优先级：#3 提到最上面
    run({ op: 'move', actor: H, id: 3, column: 'todo', index: 0 }, t0 + 230 * M);

    // #3：alpha 正在做（剩约 19 分钟）
    run({ op: 'claim', actor: A('alpha'), id: 3 }, now - 15 * M);
    run({ op: 'comment', actor: A('alpha'), id: 3, text: '复现了：`reqwest` 的错误被 `unwrap()` 了。正在改成返回 `Result`。' }, now - 11 * M);

    // 未整理：人随手记下的计划和想法
    run({ op: 'new', actor: H, column: 'inbox', title: '调研：一个仓库是否需要多块白板', body: '- 按子项目拆？\n- 还是用标签区分？\n\n先看看社区里其他项目怎么做。' }, now - 50 * M);
    run({ op: 'new', actor: H, column: 'inbox', title: '「待审核」里加一个打回的快捷操作', body: '现在打回要拖回待办，再写一条评论说明原因，有点绕。' }, now - 34 * M);
    run({ op: 'new', actor: H, column: 'inbox', title: 'CI 缓存总是失效，查一下 key 的计算方式', body: '' }, now - 20 * M);
    run({ op: 'new', actor: H, column: 'inbox', title: '整理 11 月社区反馈，挑出值得做的', body: '论坛 + Discord 两个来源。' }, now - 6 * M);

    // #2：beta 正在做（只剩约 4 分钟）
    run({ op: 'claim', actor: A('beta'), id: 2 }, now - 26 * M);
    run({ op: 'comment', actor: A('beta'), id: 2, text: '已定位到 `snapshot::normalize`，准备统一替换分隔符。' }, now - 26 * M + 30 * 1000);

    return s;
  }

  B.Clock = Clock;
  B.LocalTransport = LocalTransport;
  B.Store = Store;
})(window.Board = window.Board || {});
