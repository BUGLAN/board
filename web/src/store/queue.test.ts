/* 写队列（store/data.ts 的 exec / optimistic）：补丁当帧生效；请求按发起顺序一个接一个发；
   成功后刷新再撤补丁，失败只撤自己那一层；保存完之前同一个对象不能再改（BUSY）。
   服务端是下面的假 api：卡片命令挂起，由用例决定什么时候成功、失败 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CardBatchOp } from '../lib/api';
import type { BoardEvent, BoardSnapshot, CardDetail, Cmd, Workspace } from '../lib/types';
import { card, detail, snapshot, workspace } from '../../test/fixtures';
import type * as Data from './data';
import type * as Trash from '../lib/trash';

interface Call {
  board: string;
  cmd: Cmd;
  ok: (r?: CardDetail) => void;
  fail: (code?: string) => void;
}

const server = vi.hoisted(() => ({
  ws: null as unknown as Workspace,
  boards: {} as Record<string, BoardSnapshot>,
  calls: [] as Call[],
  other: [] as { name: string; ok: () => void; fail: () => void }[],
  /** 批量打标签（POST /api/labels/batch） */
  batches: [] as {
    board: string;
    ids: number[];
    add: number[];
    remove: number[];
    ok: (r: { changed: number[]; failed: { id: number; code: string; message: string }[]; rev: number }) => void;
    fail: () => void;
  }[],
  /** 批量卡片命令（POST /api/cards/batch） */
  cardBatches: [] as {
    board: string;
    ids: number[];
    op: CardBatchOp;
    ok: (r: { ok: number[]; failed: { id: number; code: string; message: string }[]; rev: number }) => void;
    fail: (code?: string) => void;
  }[],
  hosts: null as null | ((r: { hosts: []; dispatches: [] }) => void),
  /** 推送：boot() 之后由用例调用 */
  push: null as null | ((m: { rev: number; events: BoardEvent[]; ran?: number[]; chats?: number[] }) => void),
  /** 推送的连接状态：boot() 之后由用例调用 */
  status: null as null | ((s: 'connecting' | 'live' | 'offline') => void),
  /** 各接口被调用的次数 */
  n: { workspace: 0, board: 0 },
  /** 回收站（GET /api/trash）：白板路径 → 删掉的卡片 */
  trash: {} as Record<string, unknown[]>,
  /** 设了就让 api.board 等它（让刷新停在半路） */
  gate: null as null | Promise<void>,
  /** 设了就让 api.card 等它（让加载轮里的详情停在半路） */
  cardGate: null as null | Promise<void>,
  /** 设了就让 api.workspace 等它 */
  wsGate: null as null | Promise<void>,
  /** 接下来几次取白板失败（模拟网关 502 / 超时） */
  boardFail: 0,
  /** 覆盖 api.card 返回的详情（按「白板路径#卡片」），没设时按白板快照生成 */
  details: {} as Record<string, CardDetail>,
  /** 派发请求：单卡和批量都挂起，由用例决定什么时候返回 */
  dispatches: [] as { ok: (r: unknown) => void; fail: () => void }[],
}));

vi.mock('../lib/api', async orig => {
  const real = await orig<typeof import('../lib/api')>();
  const clone = <T,>(x: T): T => structuredClone(x);
  return {
    ...real,
    connect: (onMessage: (m: { rev: number; events: BoardEvent[] }) => void, onStatus: (s: 'connecting' | 'live' | 'offline') => void) => {
      server.push = onMessage;
      server.status = onStatus;
      return () => {};
    },
    api: {
      workspace: async () => {
        server.n.workspace++;
        const ws = clone(server.ws);
        if (server.wsGate) await server.wsGate;
        return ws;
      },
      board: async (path: string) => {
        server.n.board++;
        if (!server.boards[path]) throw new real.ApiError('BOARD_NOT_FOUND' as never, '白板不存在');
        const snap = clone(server.boards[path]);
        if (server.boardFail > 0) {
          server.boardFail--;
          throw new real.ApiError('INTERNAL' as never, '加载失败了');
        }
        if (server.gate) await server.gate;
        return snap;
      },
      card: async (path: string, id: number) => {
        const d = clone(server.details[`${path}#${id}`] ?? detail(server.boards[path].cards[id]));
        if (server.cardGate) await server.cardGate;
        return d;
      },
      // 服务端版本（GET /api/info）：版本号空，页面不会被判成旧页面（default/main#368）
      info: async () => ({ name: 'board', version: '', commit: '' }),
      trash: async (path: string) => {
        return clone(server.trash[path] ?? []);
      },
      hosts: () => new Promise(res => (server.hosts = res)),
      cmd: (board: string, cmd: Cmd) =>
        new Promise<CardDetail>((res, rej) => {
          server.calls.push({
            board,
            cmd,
            ok: r => res(r ?? detail(card('id' in cmd ? cmd.id ?? 0 : 0))),
            fail: (code = 'INTERNAL') => rej(new real.ApiError(code as never, '失败了')),
          });
        }),
      labelBatch: (board: string, ids: number[], add: number[], remove: number[]) =>
        new Promise((res, rej) => {
          server.batches.push({ board, ids, add, remove, ok: res, fail: () => rej(new real.ApiError('NETWORK', '断网了')) });
        }),
      cardBatch: (board: string, ids: number[], op: CardBatchOp) =>
        new Promise((res, rej) => {
          server.cardBatches.push({
            board,
            ids,
            op,
            ok: res,
            fail: (code = 'INTERNAL') => rej(new real.ApiError(code as never, '失败了')),
          });
        }),
      reorderLabels: () =>
        new Promise<void>((res, rej) => {
          server.other.push({ name: 'reorderLabels', ok: () => res(), fail: () => rej(new real.ApiError('INTERNAL', '失败了')) });
        }),
      dispatch: () => new Promise<unknown>((res, rej) => server.dispatches.push({ ok: res, fail: () => rej(new real.ApiError('INTERNAL', '失败了')) })),
    },
  };
});

let D: typeof Data;
let T: typeof Trash;
/** 等挂起的 Promise 链走完（请求排队、刷新） */
const settle = () => new Promise(r => setTimeout(r, 0));
const title = (id: number) => D.useData.getState().board!.cards[id].title;
/** 服务端把卡片改成这样（模拟命令成功后的数据）；全局 rev 加一，返回新的 rev */
const serverEdit = (path: string, id: number, change: object) => {
  const b = server.boards[path];
  b.cards[id] = { ...b.cards[id], ...change };
  const rev = server.ws.rev + 1;
  server.ws.rev = rev;
  for (const x of Object.values(server.boards)) x.rev = rev;
  return rev;
};
const event = (board: string, cardId = 1): BoardEvent => ({
  id: Math.random(), board, cardId, title: '', at: 0, actor: { kind: 'agent', name: 'claude/x' }, op: 'edit', text: '', via: 'cli',
});

beforeEach(async () => {
  vi.resetModules(); // 队列、补丁层、刷新状态都是模块级的
  server.ws = workspace();
  server.boards = {
    'p/a': snapshot('p/a', { todo: [1, 2, 3] }),
    'p/b': snapshot('p/b', { todo: [1] }, [{ id: 1, title: 'B 白板的卡片' }]),
  };
  server.calls = [];
  server.other = [];
  server.batches = [];
  server.cardBatches = [];
  server.hosts = null;
  server.push = null;
  server.dispatches = [];
  server.n = { workspace: 0, board: 0 };
  server.trash = {};
  server.gate = null;
  server.cardGate = null;
  server.wsGate = null;
  server.boardFail = 0;
  server.details = {};
  D = await import('./data');
  T = await import('../lib/trash');
  await D.refresh();
  expect(D.useData.getState().path).toBe('p/a');
});

// 刷新出错只会记进 store 的 error，不会抛出：每个用例结束时确认没有吞掉的错误
afterEach(() => {
  expect(D.useData.getState().error).toBeNull();
});

const rename = (id: number, t: string) => D.exec({ op: 'edit', id, title: t }, D.editCard(id, { title: t }));

describe('写队列', () => {
  it('补丁当帧生效；请求按发起顺序发出，前一个返回后才发下一个', async () => {
    const a = rename(1, 'A');
    const b = rename(2, 'B');
    expect(title(1)).toBe('A');
    expect(title(2)).toBe('B');
    await settle();
    expect(server.calls.map(c => 'id' in c.cmd && c.cmd.id)).toEqual([1]);

    serverEdit('p/a', 1, { title: 'A' });
    server.calls[0].ok();
    await a;
    await settle();
    expect(server.calls.map(c => 'id' in c.cmd && c.cmd.id)).toEqual([1, 2]);

    serverEdit('p/a', 2, { title: 'B' });
    server.calls[1].ok();
    await b;
    expect([title(1), title(2)]).toEqual(['A', 'B']);
    expect(D.useData.getState().saving).toEqual({});
  });

  it('中间一个失败：只撤它自己那一层，后面的照常', async () => {
    const a = rename(1, 'A');
    const b = rename(2, 'B');
    const c = rename(3, 'C');
    await settle();

    serverEdit('p/a', 1, { title: 'A' });
    server.calls[0].ok();
    await a;
    await settle();

    server.calls[1].fail();
    await expect(b).rejects.toMatchObject({ code: 'INTERNAL' });
    // 失败的撤回原样，还没返回的那一层仍然叠着
    expect([title(1), title(2), title(3)]).toEqual(['A', '卡片 2', 'C']);
    expect(D.cardBusy(2)).toBe(false);
    expect(D.cardBusy(3)).toBe(true);

    await settle();
    expect(server.calls.length).toBe(3);
    serverEdit('p/a', 3, { title: 'C' });
    server.calls[2].ok();
    await c;
    expect([title(1), title(2), title(3)]).toEqual(['A', '卡片 2', 'C']);
  });

  it('写请求断线、超时：撤掉补丁，再按服务端的数据刷新一次（那次写可能已经生效）', async () => {
    const a = rename(1, 'A');
    await settle();
    const n = server.n.board;
    serverEdit('p/a', 1, { title: 'A' }); // 其实已经写进去了，只是响应没回来
    server.calls[0].fail('NETWORK');
    await expect(a).rejects.toMatchObject({ code: 'NETWORK' });
    await settle();
    expect(server.n.board).toBe(n + 1);
    expect(title(1)).toBe('A');
  });

  it('optimistic：失败撤掉补丁并抛出，之后的操作不受影响', async () => {
    const a = D.optimistic(D.patchProject('p', p => ({ ...p, name: '改名' })), () => Promise.reject(new Error('坏了')));
    expect(D.useData.getState().ws!.projects[0].name).toBe('改名');
    await expect(a).rejects.toThrow('坏了');
    expect(D.useData.getState().ws!.projects[0].name).toBe('P');
  });

  it('刷新进行中（服务端还是旧数据）时补丁不被冲掉', async () => {
    const a = rename(1, 'A');
    await D.refresh(); // 例如别处的修改推送过来
    expect(title(1)).toBe('A');
    expect(D.cardBusy(1)).toBe(true);
    await settle();
    server.calls[0].fail();
    await expect(a).rejects.toMatchObject({ code: 'INTERNAL', message: '失败了' });
    expect(title(1)).toBe('卡片 1');
  });

  it('切换白板后，旧白板的补丁不落到新白板（同号卡片）', async () => {
    const a = rename(1, 'A');
    await D.setPath('p/b');
    expect(D.useData.getState().board!.path).toBe('p/b');
    expect(title(1)).toBe('B 白板的卡片');
    // 命令发给发起时所在的白板
    await settle();
    expect(server.calls[0].board).toBe('p/a');
    server.calls[0].fail();
    await expect(a).rejects.toMatchObject({ code: 'INTERNAL', message: '失败了' });
    expect(title(1)).toBe('B 白板的卡片');
  });

  it('同一张卡保存中再改：BUSY 拒绝，不叠补丁、不发请求', async () => {
    const a = rename(1, 'A');
    await expect(rename(1, 'AA')).rejects.toMatchObject({ code: 'BUSY' });
    // 命令里的卡片也算，即使补丁没写 cards
    await expect(D.exec({ op: 'comment', id: 1, text: 'x' })).rejects.toMatchObject({ code: 'BUSY' });
    expect(title(1)).toBe('A');
    await settle();
    expect(server.calls.length).toBe(1);
    serverEdit('p/a', 1, { title: 'A' });
    server.calls[0].ok();
    await a;
    // 保存完就能再改
    void rename(1, 'AA');
    expect(title(1)).toBe('AA');
  });

  it('其他对象（keys）保存中再改：BUSY', async () => {
    const a = D.optimistic(D.editLabelPatch('p', 1, { name: 'x' }), () => new Promise(() => {}));
    void a;
    await expect(D.optimistic(D.deleteLabelPatch('p', 1), async () => {})).rejects.toMatchObject({ code: 'BUSY' });
    expect(D.busy(D.labelKey('p', 1))).toBe(true);
    expect(D.busy(D.labelKey('p', 2))).toBe(false);
  });

  it('请求发出后又有写操作：丢掉不走写队列的读（refreshHosts）', async () => {
    const h = D.refreshHosts();
    void rename(1, 'A');
    server.hosts!({ hosts: [{ name: 'stale' }] as never, dispatches: [] });
    await h;
    expect(D.useData.getState().ws!.hosts).toEqual([]);
    // 没有写操作插进来时照常用
    const h2 = D.refreshHosts();
    server.hosts!({ hosts: [{ name: 'mbp' }] as never, dispatches: [] });
    await h2;
    expect(D.useData.getState().ws!.hosts).toEqual([{ name: 'mbp' }]);
  });

  it('派发期间发出的 refreshHosts：结果作废，不会把旧的派发列表写回', async () => {
    const { api } = await import('../lib/api');
    const h = D.refreshHosts();
    const p = D.optimistic(null, () => api.dispatch('p/a', 1, 'mbp'));
    server.hosts!({ hosts: [], dispatches: [{ id: 99, state: 'queued' }] as never });
    await h;
    expect(D.useData.getState().ws!.dispatches).toEqual([]);
    server.dispatches[0].ok({ id: 100 });
    await p;
    expect(D.useData.getState().error).toBeNull();
  });
});

describe('撤补丁的时机、一轮加载的往返（#174）', () => {
  it('前一笔的补丁在它返回之后开始的第一轮加载完成时撤掉，不等后一笔的请求和刷新', async () => {
    let open1!: () => void;
    server.gate = new Promise(r => (open1 = r));
    const a = rename(1, 'A');
    const b = rename(2, 'B');
    let aDone = false;
    void a.then(() => (aDone = true));
    await settle();
    serverEdit('p/a', 1, { title: 'A' });
    server.calls[0].ok(); // a 返回：开始一轮加载，停在取白板
    await settle();
    await settle();
    expect(server.calls.length).toBe(2);
    serverEdit('p/a', 2, { title: 'B' });
    server.calls[1].ok(); // b 也返回了：要再来一轮
    await settle();
    // 第二轮也先停住：a 只等第一轮
    let open2!: () => void;
    server.gate = new Promise(r => (open2 = r));
    open1();
    await settle();
    await settle();
    expect(aDone).toBe(true);
    expect(D.cardBusy(1)).toBe(false);
    expect(title(1)).toBe('A');
    expect(D.cardBusy(2)).toBe(true);
    open2();
    await b;
    expect(D.cardBusy(2)).toBe(false);
    expect([title(1), title(2)]).toEqual(['A', 'B']);
  });

  it('推送已经带来这次修改：由推送引起、正在进行的那一轮完成就撤补丁，不再多等一轮', async () => {
    D.boot();
    await settle();
    let open!: () => void;
    server.gate = new Promise(r => (open = r));
    const a = rename(1, 'A');
    await settle();
    const rev = serverEdit('p/a', 1, { title: 'A' });
    const r = detail(card(1, { title: 'A' }));
    r.activity = [{ id: 9, at: 500, actor: { kind: 'human', name: '人' }, kind: 'system', action: 'edit', text: '改了标题' }];
    // 推送比响应先到：开始一轮（停在取白板）
    server.push!({ rev, events: [{ ...event('p/a', 1), at: 500 }] });
    await settle();
    const n = server.n.board;
    server.calls[0].ok(r);
    await settle();
    open();
    await a;
    expect(server.n.board).toBe(n);
    expect(title(1)).toBe('A');
    expect(D.cardBusy(1)).toBe(false);
  });

  it('当前白板在上一次的工作区里：工作区和白板一起取，不等工作区返回', async () => {
    let open!: () => void;
    server.wsGate = new Promise(r => (open = r));
    const n = { ws: server.n.workspace, board: server.n.board };
    const p = D.refresh();
    await settle();
    expect(server.n.workspace).toBe(n.ws + 1);
    expect(server.n.board).toBe(n.board + 1);
    open();
    await p;
  });

  it('一起取时当前白板刚被删掉：照样换到别的白板，不报错', async () => {
    server.ws.projects[0].boards = server.ws.projects[0].boards.filter(b => b.key !== 'a');
    delete server.boards['p/a'];
    await D.refresh();
    expect(D.useData.getState().path).toBe('p/b');
    expect(D.useData.getState().board!.path).toBe('p/b');
  });

  it('写成功后紧接着的一轮加载失败：补丁留着，退避到点自动重取，卡片落到新位置', async () => {
    vi.useFakeTimers();
    try {
      const a = D.exec({ op: 'move', id: 1, column: 'review' }, D.movePatch(1, 'review'));
      await vi.advanceTimersByTimeAsync(0);
      expect(D.useData.getState().board!.cols.review).toEqual([1]);

      const b = server.boards['p/a'];
      b.cols.todo = b.cols.todo.filter(id => id !== 1);
      b.cols.review = [1];
      serverEdit('p/a', 1, { column: 'review' });
      server.boardFail = 1;
      server.calls[0].ok();
      await vi.advanceTimersByTimeAsync(0);
      expect(D.useData.getState().board!.cols.review).toEqual([1]);
      expect(D.useData.getState().saving).toEqual({ 'p/a#1': 1 });

      const n = server.n.board;
      await vi.advanceTimersByTimeAsync(2_000);
      await vi.advanceTimersByTimeAsync(0);
      await a;
      expect(server.n.board).toBe(n + 1);
      expect(D.useData.getState().board!.cols.review).toEqual([1]);
      expect(D.useData.getState().saving).toEqual({});
      expect(D.useData.getState().error).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('设置项（saveSetting）：不上锁的乐观保存', () => {
  const name = () => D.useData.getState().ws!.projects[0].name;
  const key = () => D.useData.getState().ws!.projects[0].key;
  const setName = (n: string) => D.patchProject(key(), p => ({ ...p, name: n }));
  /** 一个挂起的请求：成功时把服务端的项目名改成 n */
  const pending = (n: string) => {
    const call = { sent: false, ok: () => {}, fail: () => {} };
    const req = () =>
      new Promise<void>((res, rej) => {
        call.sent = true;
        call.ok = () => {
          server.ws.projects[0].name = n;
          server.ws.rev++;
          for (const x of Object.values(server.boards)) x.rev = server.ws.rev;
          res();
        };
        call.fail = () => rej(new Error('失败了'));
      });
    return { call, req };
  };

  it('前一笔还没确认也能接着改：不 BUSY，补丁当帧生效，同一个 key 的请求一个接一个发', async () => {
    const a = pending('A');
    const b = pending('B');
    const pa = D.saveSetting('k', setName('A'), a.req);
    const pb = D.saveSetting('k', setName('B'), b.req);
    expect(name()).toBe('B');
    expect(D.useData.getState().saving).toEqual({ k: 2 });
    await settle();
    expect([a.call.sent, b.call.sent]).toEqual([true, false]);
    a.call.ok();
    await pa;
    await settle();
    expect(b.call.sent).toBe(true);
    b.call.ok();
    await pb;
    await settle();
    expect(name()).toBe('B');
    expect(D.useData.getState().saving).toEqual({});
  });

  it('不同 key 的请求同时发，互相不等', async () => {
    const a = pending('A');
    const b = pending('B');
    void D.saveSetting('k1', setName('A'), a.req);
    void D.saveSetting('k2', {}, b.req);
    await settle();
    expect([a.call.sent, b.call.sent]).toEqual([true, true]);
    a.call.ok();
    b.call.ok();
    await settle();
  });

  it('delay 内连着改：合并成一个请求，发最后一次的值，拿到的是同一个 Promise', async () => {
    vi.useFakeTimers();
    try {
      const a = pending('A');
      const b = pending('B');
      const pa = D.saveSetting('k', setName('A'), a.req, 400);
      await vi.advanceTimersByTimeAsync(200);
      const pb = D.saveSetting('k', setName('B'), b.req, 400);
      expect(pb).toBe(pa);
      expect(name()).toBe('B');
      await vi.advanceTimersByTimeAsync(399);
      expect([a.call.sent, b.call.sent]).toEqual([false, false]);
      await vi.advanceTimersByTimeAsync(2);
      expect([a.call.sent, b.call.sent]).toEqual([false, true]);
      b.call.ok();
      await pb;
      await vi.advanceTimersByTimeAsync(0);
      expect(name()).toBe('B');
      expect(D.useData.getState().saving).toEqual({});
    } finally {
      vi.useRealTimers();
    }
  });

  it('请求一返回 Promise 就结束；补丁留到刷新完成再撤，值不闪回旧的', async () => {
    const a = pending('A');
    const pa = D.saveSetting('k', setName('A'), a.req);
    await settle();
    let open!: () => void;
    server.gate = new Promise<void>(r => (open = r));
    a.call.ok();
    await pa;
    expect(name()).toBe('A');
    expect(D.useData.getState().saving).toEqual({ k: 1 });
    open();
    server.gate = null;
    await settle();
    await settle();
    expect(name()).toBe('A');
    expect(D.useData.getState().saving).toEqual({});
  });

  it('失败：这一批的补丁一起撤掉，值回到之前，Promise 抛出错误', async () => {
    const before = name();
    const a = pending('A');
    const pa = D.saveSetting('k', setName('A'), a.req);
    await settle();
    a.call.fail();
    await expect(pa).rejects.toThrow('失败了');
    expect(name()).toBe(before);
    expect(D.useData.getState().saving).toEqual({});
  });
});

describe('新建', () => {
  it('服务端返回后、刷新之前就认下真卡片：临时卡片和真卡片不会同时出现，真卡片沿用临时卡片的身份', async () => {
    const temp = D.draftCard({ title: '新卡片', body: '', labels: [], column: 'todo' });
    const done = D.exec({ op: 'new', title: '新卡片', body: '', labels: [] }, temp.patch, c => temp.settle(c.id));
    expect(D.useData.getState().board!.cols.todo).toEqual([1, 2, 3, temp.id]);
    // 服务端存下的标题和提交的不一样（例如去掉了多余空白）：按标题猜不出来，只能靠返回的 ID
    const b = server.boards['p/a'];
    b.cards[5] = card(5, { title: '新 卡片' });
    b.cols.todo = [1, 2, 3, 5];
    server.ws.rev = b.rev = 2;
    const seen: number[][] = [];
    const off = D.useData.subscribe(s => seen.push(s.board!.cols.todo));
    await settle();
    server.calls[0].ok(detail(b.cards[5]));
    await done;
    off();
    expect(D.useData.getState().board!.cols.todo).toEqual([1, 2, 3, 5]);
    for (const ids of seen) expect(ids.includes(5) && ids.includes(temp.id)).toBe(false);
    expect(D.renderKey(5)).toBe(temp.id);
  });
});

describe('删除、恢复（回收站）', () => {
  it('删掉：当帧从白板上拿掉，回收站里多一张；被拒绝时回来并提示', async () => {
    const r = T.deleteCards([2], { silent: true });
    expect(D.useData.getState().board!.cols.todo).toEqual([1, 3]);
    expect(D.useData.getState().board!.cards[2]).toBeUndefined();
    expect(D.useData.getState().board!.trash).toBe(1);
    await settle();
    server.cardBatches[0].fail('USAGE');
    expect(await r).toBe(false);
    expect(D.useData.getState().board!.cols.todo).toEqual([1, 2, 3]);
    // 撤掉补丁后回到服务端的快照（旧的快照没有这个字段）
    expect(D.useData.getState().board!.trash).toBeUndefined();
  });

  it('恢复：当帧回到原来那一列，回收站里少一张', async () => {
    const b = server.boards['p/a'];
    delete b.cards[2];
    b.cols.todo = [1, 3];
    b.trash = 1;
    serverEdit('p/a', 1, {});
    await D.refresh();
    server.trash['p/a'] = [{ ...card(2), deletedAt: 0, deletedBy: { kind: 'human', name: '人' }, col: 'todo', pos: 1 }];
    const r = T.restoreMany('p/a', [2]);
    await settle();
    expect(D.useData.getState().board!.cols.todo).toEqual([1, 2, 3]);
    expect(D.useData.getState().board!.trash).toBe(0);
    b.cards[2] = card(2);
    b.cols.todo = [1, 2, 3];
    b.trash = 0;
    const rev = serverEdit('p/a', 2, {});
    server.cardBatches[0].ok({ ok: [2], failed: [], rev });
    expect(await r).toEqual({ ok: [2], failed: [] });
    expect(D.useData.getState().board!.cols.todo).toEqual([1, 2, 3]);
    expect(D.useData.getState().board!.trash).toBe(0);
  });
});

describe('稍后再做的批量结果（default/main#512）', () => {
  it('全部成功：一个请求并返回每一条的 ID', async () => {
    const r = D.setLater('p/a', [1, 2], true);
    await settle();
    expect(server.cardBatches[0].ids).toEqual([1, 2]);
    expect(server.cardBatches[0].op).toEqual({ op: 'later', on: true });
    server.boards['p/a'].cards[1].laterAt = 100;
    const rev = serverEdit('p/a', 2, { laterAt: 100 });
    server.cardBatches[0].ok({ ok: [1, 2], failed: [], rev });
    expect(await r).toEqual({ ok: [1, 2], failed: [] });
  });

  it('部分失败：返回成功的 #2 和失败的 #1', async () => {
    const r = D.setLater('p/a', [1, 2], true);
    await settle();
    const rev = serverEdit('p/a', 2, { laterAt: 100 });
    server.cardBatches[0].ok({ ok: [2], failed: [{ id: 1, code: 'NOT_IN_TODO', message: '不在未整理里' }], rev });
    expect(await r).toEqual({ ok: [2], failed: [{ id: 1, error: expect.objectContaining({ code: 'NOT_IN_TODO' }) }] });
  });

  it('全部失败：没有成功项', async () => {
    const r = D.setLater('p/a', [1, 2], true);
    await settle();
    const rev = serverEdit('p/a', 1, {});
    server.cardBatches[0].ok({
      ok: [],
      failed: [
        { id: 1, code: 'INTERNAL', message: '失败' },
        { id: 2, code: 'INTERNAL', message: '失败' },
      ],
      rev,
    });
    expect(await r).toEqual({
      ok: [],
      failed: [
        { id: 1, error: expect.objectContaining({ code: 'INTERNAL' }) },
        { id: 2, error: expect.objectContaining({ code: 'INTERNAL' }) },
      ],
    });
  });
});

describe('标签', () => {
  const labels = (id: number) => D.useData.getState().board!.cards[id].labels;
  const pending = (id: number) => D.useData.getState().pending[`p/a#${id}`] ?? [];

  it('加标签：当帧出现并画成 pending，确认后只去掉 pending，不先消失再出现', async () => {
    const a = D.addLabel(1, 7);
    expect(labels(1)).toEqual([7]);
    expect(pending(1)).toEqual([7]);
    await settle();
    serverEdit('p/a', 1, { labels: [7] });
    server.calls[0].ok();
    await a;
    expect(labels(1)).toEqual([7]);
    expect(pending(1)).toEqual([]);
  });

  it('去掉标签：保存期间标签还在、画成 pending，卡片不画成「保存中」；确认后消失', async () => {
    serverEdit('p/a', 1, { labels: [7, 8] });
    await D.refresh();
    const r = D.removeLabel(1, 7);
    expect(labels(1)).toEqual([7, 8]);
    expect(pending(1)).toEqual([7]);
    expect(D.cardBusy(1)).toBe(false);
    // 正在去掉时再点：不发第二个请求
    expect(await D.removeLabel(1, 7)).toBe(false);
    await settle();
    expect(server.calls.length).toBe(1);
    serverEdit('p/a', 1, { labels: [8] });
    server.calls[0].ok();
    expect(await r).toBe(true);
    expect(labels(1)).toEqual([8]);
    expect(pending(1)).toEqual([]);
  });

  it('去掉标签失败：标签留着、去掉 pending，并抛出错误', async () => {
    serverEdit('p/a', 1, { labels: [7] });
    await D.refresh();
    const r = D.removeLabel(1, 7);
    await settle();
    server.calls[0].fail();
    await expect(r).rejects.toMatchObject({ code: 'INTERNAL' });
    expect(labels(1)).toEqual([7]);
    expect(pending(1)).toEqual([]);
  });
});

describe('批量打标签（default/main#531）', () => {
  const labels = (id: number) => D.useData.getState().board!.cards[id].labels;
  const pending = (id: number) => D.useData.getState().pending[`p/a#${id}`] ?? [];

  it('一次请求打完：当帧画上、本来就有的跳过；成功后只刷新一次', async () => {
    serverEdit('p/a', 2, { labels: [7] });
    await D.refresh();
    const r = D.labelCards([1, 2, 3], 7, true);
    expect([labels(1), labels(2), labels(3)]).toEqual([[7], [7], [7]]);
    expect([pending(1), pending(2), pending(3)]).toEqual([[7], [], [7]]);
    await settle();
    expect(server.calls).toEqual([]);
    expect(server.batches.map(b => [b.board, b.ids, b.add, b.remove])).toEqual([['p/a', [1, 3], [7], []]]);
    const n = server.n.board;
    serverEdit('p/a', 1, { labels: [7] });
    const rev = serverEdit('p/a', 3, { labels: [7] });
    server.batches[0].ok({ changed: [1, 3], failed: [], rev });
    expect(await r).toEqual({ changed: [1, 3], skipped: [2], failed: [] });
    expect(server.n.board - n).toBe(1);
    expect([labels(1), labels(3)]).toEqual([[7], [7]]);
    expect([pending(1), pending(3)]).toEqual([[], []]);
  });

  it('推送比响应先到、已经取到写完后的 rev：不再多取一轮', async () => {
    D.boot();
    await settle();
    const r = D.labelCards([1], 7, true);
    await settle();
    const rev = serverEdit('p/a', 1, { labels: [7] });
    server.push!({ rev, events: [event('p/a', 1)] });
    await settle();
    const n = server.n.board;
    server.batches[0].ok({ changed: [1], failed: [], rev });
    await r;
    expect(server.n.board).toBe(n);
    expect(labels(1)).toEqual([7]);
  });

  it('部分失败：一返回就只撤失败的那张，其余的等刷新；错误带着服务端的错误码', async () => {
    const r = D.labelCards([1, 2, 3], 7, true);
    await settle();
    let open!: () => void;
    server.gate = new Promise(res => (open = res));
    serverEdit('p/a', 1, { labels: [7] });
    const rev = serverEdit('p/a', 3, { labels: [7] });
    server.batches[0].ok({ changed: [1, 3], failed: [{ id: 2, code: 'NOT_FOUND', message: '卡片 #2 不存在' }], rev });
    await settle();
    // 刷新还停在半路：失败的已经恢复原样，成功的还画着
    expect([labels(1), labels(2), labels(3)]).toEqual([[7], [], [7]]);
    expect([pending(1), pending(2), pending(3)]).toEqual([[7], [], [7]]);
    server.gate = null;
    open();
    const out = await r;
    expect(out.changed).toEqual([1, 3]);
    expect(out.failed).toEqual([{ id: 2, error: expect.objectContaining({ code: 'NOT_FOUND' }) }]);
    expect([labels(1), labels(2), labels(3)]).toEqual([[7], [], [7]]);
  });

  it('去标签：保存期间标签留着画成 pending；整个请求失败时全部恢复原样并抛出', async () => {
    serverEdit('p/a', 1, { labels: [7] });
    serverEdit('p/a', 2, { labels: [7] });
    await D.refresh();
    const r = D.labelCards([1, 2, 3], 7, false);
    expect([labels(1), labels(2)]).toEqual([[7], [7]]);
    expect([pending(1), pending(2)]).toEqual([[7], [7]]);
    await settle();
    expect(server.batches[0].ids).toEqual([1, 2]);
    expect(server.batches[0].remove).toEqual([7]);
    server.batches[0].fail();
    await expect(r).rejects.toMatchObject({ code: 'NETWORK' });
    expect([labels(1), labels(2)]).toEqual([[7], [7]]);
    expect([pending(1), pending(2)]).toEqual([[], []]);
  });

  it('全部本来就有：不发请求', async () => {
    serverEdit('p/a', 1, { labels: [7] });
    await D.refresh();
    expect(await D.labelCards([1], 7, true)).toEqual({ changed: [], skipped: [1], failed: [] });
    await settle();
    expect(server.batches).toEqual([]);
  });
});

describe('批量卡片命令', () => {
  const moveOnServer = (ids: number[], to: string) => {
    const b = server.boards['p/a'];
    for (const id of ids) {
      for (const [key, list] of Object.entries(b.cols)) b.cols[key] = list.filter(x => x !== id);
      b.cols[to] = [...(b.cols[to] ?? []), id];
      b.cards[id] = { ...b.cards[id], column: to };
    }
    const rev = server.ws.rev + 1;
    server.ws.rev = rev;
    for (const x of Object.values(server.boards)) x.rev = rev;
    return rev;
  };

  it('一个请求、当帧移动全部卡片；部分失败只撤失败的卡片并只刷新一次', async () => {
    const r = D.batchCards('p/a', [1, 2, 3], { op: 'move', column: 'review' }, id => D.movePatch(id, 'review'));
    expect(D.useData.getState().board!.cols.review).toEqual([1, 2, 3]);
    expect([D.cardBusy(1), D.cardBusy(2), D.cardBusy(3)]).toEqual([true, true, true]);
    await settle();
    expect(server.calls).toEqual([]);
    expect(server.cardBatches.map(b => [b.board, b.ids, b.op])).toEqual([['p/a', [1, 2, 3], { op: 'move', column: 'review' }]]);

    let open!: () => void;
    server.gate = new Promise(res => (open = res));
    const rev = moveOnServer([1, 3], 'review');
    const n = server.n.board;
    server.cardBatches[0].ok({
      ok: [1, 3],
      failed: [{ id: 2, code: 'NOT_IN_TODO', message: '#2 不能移' }],
      rev,
    });
    await settle();

    // 刷新还停在半路：失败的那张已经回原处，成功的两张仍显示目标状态
    expect(D.useData.getState().board!.cols.todo).toEqual([2]);
    expect(D.useData.getState().board!.cols.review).toEqual([1, 3]);
    expect([D.cardBusy(1), D.cardBusy(2), D.cardBusy(3)]).toEqual([true, false, true]);

    server.gate = null;
    open();
    expect(await r).toEqual({
      ok: [1, 3],
      failed: [{ id: 2, error: expect.objectContaining({ code: 'NOT_IN_TODO' }) }],
    });
    expect(server.n.board - n).toBe(1);
    expect(D.useData.getState().saving).toEqual({});
  });

  it('整批请求失败：所有卡片恢复原样并抛出', async () => {
    const r = D.batchCards('p/a', [1, 2, 3], { op: 'move', column: 'review' }, id => D.movePatch(id, 'review'));
    expect(D.useData.getState().board!.cols.review).toEqual([1, 2, 3]);
    await settle();
    server.cardBatches[0].fail();
    await expect(r).rejects.toMatchObject({ code: 'INTERNAL' });
    expect(D.useData.getState().board!.cols.todo).toEqual([1, 2, 3]);
    expect(D.useData.getState().board!.cols.review).toBeUndefined();
    expect(D.useData.getState().saving).toEqual({});
  });

  it('正在保存的卡片记 BUSY，不发进这一批；其余卡片照常发', async () => {
    const pending = rename(2, '正在保存');
    const r = D.batchCards('p/a', [1, 2], { op: 'move', column: 'review' }, id => D.movePatch(id, 'review'));
    await settle();
    serverEdit('p/a', 2, { title: '正在保存' });
    server.calls[0].ok();
    await pending;
    await settle();

    expect(server.cardBatches[0].ids).toEqual([1]);
    const rev = moveOnServer([1], 'review');
    server.cardBatches[0].ok({ ok: [1], failed: [], rev });
    expect(await r).toEqual({
      ok: [1],
      failed: [{ id: 2, error: expect.objectContaining({ code: 'BUSY' }) }],
    });
  });

  it('已经换了白板：整批失败，不叠补丁、不发请求、不动新白板', async () => {
    await D.setPath('p/b');
    const before = D.useData.getState().board!;
    const r = await D.batchCards('p/a', [1], { op: 'delete' }, D.removeCard);
    expect(r).toEqual({
      ok: [],
      failed: [{ id: 1, error: expect.objectContaining({ code: 'USAGE', message: '已经换了白板，回到原来的白板再撤销' }) }],
    });
    expect(server.cardBatches).toEqual([]);
    expect(D.useData.getState().board).toBe(before);
    expect(D.useData.getState().saving).toEqual({});
  });

  it('删除后换白板再撤销：不发批量请求，新白板的同号卡片不变', async () => {
    await D.setPath('p/b');
    const before = D.useData.getState().board!;
    const r = await T.restoreMany('p/a', [1]);
    expect(r.failed[0].error).toMatchObject({ code: 'USAGE', message: '已经换了白板，回到原来的白板再撤销' });
    expect(server.cardBatches).toEqual([]);
    expect(D.useData.getState().board).toBe(before);
    expect(D.useData.getState().board!.cards[1].title).toBe('B 白板的卡片');
  });
});

describe('评论（sendComment）', () => {
  const said = (id: number, text: string): CardDetail['activity'][number] => ({
    id,
    at: 1_000 + id,
    actor: { kind: 'human', name: '人' },
    kind: 'comment',
    action: 'comment',
    text,
  });
  const texts = () => D.useData.getState().detail!.activity.filter(a => a.kind === 'comment').map(a => a.text);

  it('正常发送：当帧出现发送中气泡，确认后换成正式评论', async () => {
    await D.openDetail(1);
    const p = D.sendComment(1, '第一条评论');
    expect(D.useData.getState().sending['p/a#1']?.map(x => x.text)).toEqual(['第一条评论']);
    await settle();

    const r = { ...detail(card(1)), comments: 1, activity: [said(10, '第一条评论')] };
    server.details['p/a#1'] = r;
    server.calls[0].ok(r);
    await p;
    await settle();
    expect(texts()).toEqual(['第一条评论']);
    expect(D.useData.getState().sending['p/a#1']).toBeUndefined();
    expect(D.isSettled(10)).toBe(true);
  });

  it('失败：撤掉发送中气泡并抛出，后面的评论照常发', async () => {
    await D.openDetail(1);
    const p = D.sendComment(1, '没发出去');
    await settle();
    server.calls[0].fail();
    await expect(p).rejects.toMatchObject({ code: 'INTERNAL' });
    expect(D.useData.getState().sending['p/a#1']).toBeUndefined();
    expect(texts()).toEqual([]);

    const q = D.sendComment(1, '再发一次');
    await settle();
    const r = { ...detail(card(1)), comments: 1, activity: [said(11, '再发一次')] };
    server.details['p/a#1'] = r;
    server.calls[1].ok(r);
    await q;
    expect(texts()).toEqual(['再发一次']);
  });

  it('发送途中换卡：详情留在新卡片，旧卡片的气泡照样撤掉', async () => {
    await D.openDetail(1);
    const p = D.sendComment(1, '发完再切');
    await D.openDetail(2);
    await settle();

    const r = { ...detail(card(1)), comments: 1, activity: [said(12, '发完再切')] };
    server.details['p/a#1'] = r;
    server.calls[0].ok(r);
    await p;
    expect(D.useData.getState().detailId).toBe(2);
    expect(D.useData.getState().detail!.id).toBe(2);
    expect(D.useData.getState().sending['p/a#1']).toBeUndefined();
  });

  it('一轮加载取回的旧详情不会盖掉刚写进去的评论', async () => {
    await D.openDetail(1);
    let release!: () => void;
    server.cardGate = new Promise(r => (release = r));
    const load = D.refresh();
    await settle();

    const p = D.sendComment(1, '刚发的评论');
    await settle();
    const r = { ...detail(card(1)), comments: 1, activity: [said(13, '刚发的评论')] };
    server.details['p/a#1'] = r;
    server.calls[0].ok(r);
    await p;
    expect(texts()).toEqual(['刚发的评论']);

    release();
    server.cardGate = null;
    await load;
    await settle();
    await settle();
    expect(texts()).toEqual(['刚发的评论']);
  });
});

describe('推送与刷新', () => {
  beforeEach(async () => {
    D.boot();
    await settle();
    server.n = { workspace: 0, board: 0 };
  });

  it('第一次连上推送不再全量取一遍；第一条推送的 rev 不比已经取到的新就不取（default/main#469）', async () => {
    server.status!('connecting');
    server.status!('live');
    server.push!({ rev: server.ws.rev, events: [] });
    await settle();
    expect(server.n).toEqual({ workspace: 0, board: 0 });
  });

  it('一个推送监听抛错：控制台记下错误，其余监听照常收到', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    const runs: number[][] = [];
    const chats: number[][] = [];
    const stop = [
      D.onRuns(() => {
        throw new Error('boom');
      }),
      D.onRuns(v => runs.push(v)),
      D.onChatRuns(v => chats.push(v)),
    ];
    try {
      server.push!({ rev: server.ws.rev, events: [], ran: [7], chats: [8] });
      await settle();
      expect(runs).toEqual([[7]]);
      expect(chats).toEqual([[8]]);
      expect(error).toHaveBeenCalledWith('推送监听出错', expect.any(Error));
    } finally {
      stop.forEach(fn => fn());
      error.mockRestore();
    }
  });

  it('断开后重新连上：取一遍，补上断开期间的变化', async () => {
    server.status!('live');
    await settle();
    server.status!('offline');
    await settle();
    const n = server.n.board;
    serverEdit('p/a', 2, { title: '断开时改的' });
    server.status!('live');
    await settle();
    expect(server.n.board).toBe(n + 1);
    expect(title(2)).toBe('断开时改的');
  });

  it('写操作返回后的刷新进行中，推送带来的新 rev 不再多加载一轮', async () => {
    const a = rename(1, 'A');
    await settle();
    const rev = serverEdit('p/a', 1, { title: 'A' });
    let open!: () => void;
    server.gate = new Promise(r => (open = r));
    server.calls[0].ok();
    await settle(); // exec 发起的刷新停在取白板上
    expect(server.n.board).toBe(1);
    server.push!({ rev, events: [{ ...event('p/a'), via: 'ui' }] });
    open();
    server.gate = null;
    await a;
    await settle();
    expect(server.n.board).toBe(1);
    expect(title(1)).toBe('A');
  });

  it('推送比写操作的响应先到：只加载一轮（按动态时间认出推送里就是这次修改）', async () => {
    const a = rename(1, 'A');
    await settle();
    const rev = serverEdit('p/a', 1, { title: 'A' });
    server.push!({ rev, events: [{ ...event('p/a'), at: 500, client: 'me', via: 'ui' }] });
    await settle();
    expect(server.n.board).toBe(1);
    server.calls[0].ok({ ...detail(card(1, { title: 'A' })), activity: [{ id: 9, at: 500 }] as never });
    await a;
    await settle();
    expect(server.n.board).toBe(1);
    expect(title(1)).toBe('A');
  });

  it('响应里的动态比见过的推送新（推送还没到）：照常刷新', async () => {
    const a = rename(1, 'A');
    await settle();
    server.push!({ rev: server.ws.rev, events: [{ ...event('p/a'), at: 100 }] });
    await settle();
    serverEdit('p/a', 1, { title: 'A' });
    server.calls[0].ok({ ...detail(card(1, { title: 'A' })), activity: [{ id: 9, at: 500 }] as never });
    await a;
    expect(server.n.board).toBe(1);
    expect(title(1)).toBe('A');
  });

  it('别处的修改：推送到了就重取当前白板', async () => {
    const rev = serverEdit('p/a', 2, { title: '别处改的' });
    server.push!({ rev, events: [event('p/a', 2)] });
    await settle();
    expect(server.n.board).toBe(1);
    expect(title(2)).toBe('别处改的');
    // 同一个 rev 再推一次（例如重连后的第一条）：不再取
    server.push!({ rev, events: [] });
    await settle();
    expect(server.n.board).toBe(1);
  });

  it('变化都属于别的白板：只取工作区，不取当前白板', async () => {
    server.ws.rev += 1;
    server.ws.projects[0].name = '改名了';
    server.push!({ rev: server.ws.rev, events: [event('p/b'), event('p/b', 2)] });
    await settle();
    expect(server.n).toEqual({ workspace: 1, board: 0 });
    expect(D.useData.getState().ws!.projects[0].name).toBe('改名了');
    // 之后当前白板的变化照样重取（只取工作区不会让当前白板的 rev 往前走）
    const rev = serverEdit('p/a', 1, { title: 'A2' });
    server.push!({ rev, events: [event('p/a')] });
    await settle();
    expect(server.n.board).toBe(1);
    expect(title(1)).toBe('A2');
  });

  it('没有事件的变化、落后太多（rev < 0）：全量刷新', async () => {
    server.ws.rev += 1;
    server.push!({ rev: server.ws.rev, events: [] });
    await settle();
    expect(server.n.board).toBe(1);
    server.push!({ rev: -1, events: [] });
    await settle();
    expect(server.n.board).toBe(2);
  });

  it('推送带了 boards：当前白板不在其中只取工作区，在其中才重取白板（default/main#739）', async () => {
    const push = server.push as (m: { rev: number; events: BoardEvent[]; boards?: string[] | null }) => void;
    // 机器心跳、名字上下线：没有动态，boards 为空
    server.ws.rev += 1;
    push({ rev: server.ws.rev, events: [], boards: [] });
    await settle();
    expect(server.n).toEqual({ workspace: 1, board: 0 });
    // 只有别的白板变了
    server.ws.rev += 1;
    push({ rev: server.ws.rev, events: [], boards: ['p/b'] });
    await settle();
    expect(server.n).toEqual({ workspace: 2, board: 0 });
    // 当前白板变了（标记已读、拖动状态块这类不记动态的写）
    const rev = serverEdit('p/a', 1, { title: '没有动态的修改' });
    push({ rev, events: [], boards: ['p/b', 'p/a'] });
    await settle();
    expect(server.n.board).toBe(1);
    expect(title(1)).toBe('没有动态的修改');
  });

  it('推送带了 boards 但动态属于当前白板（一个事务的动态分几批推）：照样重取白板', async () => {
    const push = server.push as (m: { rev: number; events: BoardEvent[]; boards?: string[] | null }) => void;
    const rev = serverEdit('p/a', 2, { title: '后一批' });
    push({ rev, events: [event('p/a', 2)], boards: [] });
    await settle();
    expect(server.n.board).toBe(1);
    expect(title(2)).toBe('后一批');
  });

  it('boards 为 null（期间删掉过白板）：按原来的规则，没有动态时重取当前白板', async () => {
    const push = server.push as (m: { rev: number; events: BoardEvent[]; boards?: string[] | null }) => void;
    server.ws.rev += 1;
    push({ rev: server.ws.rev, events: [], boards: null });
    await settle();
    expect(server.n.board).toBe(1);
    // 有动态、都属于别的白板：只取工作区
    server.ws.rev += 1;
    push({ rev: server.ws.rev, events: [event('p/b')], boards: null });
    await settle();
    expect(server.n.board).toBe(1);
  });

  it('刷新后没变的卡片、标签、在线名单沿用原来的对象', async () => {
    const before = D.useData.getState();
    const online = D.useData.getState().ws!.agents;
    const rev = serverEdit('p/a', 2, { title: '只改了这张' });
    server.ws.agents = [{ name: 'claude/x' }] as never;
    server.push!({ rev, events: [event('p/a', 2)] });
    await settle();
    const after = D.useData.getState();
    expect(after.board).not.toBe(before.board);
    expect(after.board!.cards[1]).toBe(before.board!.cards[1]);
    expect(after.board!.cards[3]).toBe(before.board!.cards[3]);
    expect(after.board!.cards[2]).not.toBe(before.board!.cards[2]);
    expect(after.board!.statuses).toBe(before.board!.statuses);
    expect(after.ws!.projects[0].labels).toBe(before.ws!.projects[0].labels);
    expect(after.ws!.agents).not.toBe(online);
  });
});

describe('share（结构共享）', () => {
  it('相同的部分沿用原来的对象，变了的换新', () => {
    const a = { x: { y: [1, 2] }, z: { w: 1 }, arr: [{ id: 1 }, { id: 2 }] };
    const b = { x: { y: [1, 2] }, z: { w: 2 }, arr: [{ id: 1 }, { id: 3 }] };
    const r = D.share(a, b);
    expect(r).toEqual(b);
    expect(r.x).toBe(a.x);
    expect(r.z).not.toBe(a.z);
    expect(r.arr[0]).toBe(a.arr[0]);
    expect(r.arr[1]).not.toBe(a.arr[1]);
    expect(D.share(a, structuredClone(a))).toBe(a);
    // 多了、少了键或元素：换新
    expect(D.share({ a: 1 }, { a: 1, b: 2 })).toEqual({ a: 1, b: 2 });
    expect(D.share({ a: 1, b: 2 }, { a: 1 })).toEqual({ a: 1 });
    expect(D.share([1, 2], [1])).toEqual([1]);
    expect(D.share(null, { a: 1 })).toEqual({ a: 1 });
  });
});

describe('切换白板只取白板（default/main#504）', () => {
  beforeEach(async () => {
    D.boot();
    await settle();
  });

  it('推送连着：切到工作区里已有的白板只取白板快照，不再取工作区；切回来同样', async () => {
    server.status!('live');
    await settle();
    server.n = { workspace: 0, board: 0 };
    await D.setPath('p/b');
    expect(server.n).toEqual({ workspace: 0, board: 1 });
    expect(D.useData.getState().board!.path).toBe('p/b');
    expect(D.useData.getState().board!.cards[1].title).toBe('B 白板的卡片');
    await D.setPath('p/a');
    expect(server.n).toEqual({ workspace: 0, board: 2 });
    expect(D.useData.getState().board!.path).toBe('p/a');
  });

  it('推送断着：工作区可能漏了变化，照常一起取', async () => {
    server.status!('offline');
    await settle();
    server.n = { workspace: 0, board: 0 };
    await D.setPath('p/b');
    expect(server.n).toEqual({ workspace: 1, board: 1 });
  });

  it('切换后别的原因要的刷新（写操作返回、手动刷新）照常取工作区', async () => {
    server.status!('live');
    await settle();
    await D.setPath('p/b');
    server.n = { workspace: 0, board: 0 };
    await D.refresh();
    expect(server.n).toEqual({ workspace: 1, board: 1 });
  });

  it('切换那一轮之前推送说工作区变了：只取白板之后再补取一次工作区', async () => {
    server.status!('live');
    await settle();
    let open!: () => void;
    server.gate = new Promise(r => (open = r));
    server.n = { workspace: 0, board: 0 };
    const p = D.setPath('p/b');
    await settle();
    // 切换那一轮还在取白板时，推送说别的白板变了
    const rev = serverEdit('p/a', 1, { title: '别处改的' });
    server.push!({ rev, events: [event('p/a')] });
    open();
    server.gate = null;
    await p;
    await settle();
    expect(server.n.workspace).toBe(1);
    expect(D.useData.getState().board!.path).toBe('p/b');
  });
});
