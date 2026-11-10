/* 写队列（store/data.ts 的 exec / optimistic）：补丁当帧生效；请求按发起顺序一个接一个发；
   成功后刷新再撤补丁，失败只撤自己那一层；保存完之前同一个对象不能再改（BUSY）。
   服务端是下面的假 api：卡片命令挂起，由用例决定什么时候成功、失败 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardSnapshot, CardDetail, Cmd, Workspace } from '../lib/types';
import { card, detail, snapshot, workspace } from '../../test/fixtures';
import type * as Data from './data';

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
  hosts: null as null | ((r: { hosts: []; dispatches: [] }) => void),
}));

vi.mock('../lib/api', async orig => {
  const real = await orig<typeof import('../lib/api')>();
  const clone = <T,>(x: T): T => structuredClone(x);
  return {
    ...real,
    connect: () => () => {},
    api: {
      workspace: async () => clone(server.ws),
      board: async (path: string) => clone(server.boards[path]),
      card: async (path: string, id: number) => detail(clone(server.boards[path].cards[id])),
      hosts: () => new Promise(res => (server.hosts = res)),
      cmd: (board: string, cmd: Cmd) =>
        new Promise<CardDetail>((res, rej) => {
          server.calls.push({
            board,
            cmd,
            ok: r => res(r ?? detail(card('id' in cmd ? cmd.id : 0))),
            fail: (code = 'INTERNAL') => rej(new real.ApiError(code as never, '失败了')),
          });
        }),
      reorderLabels: () =>
        new Promise<void>((res, rej) => {
          server.other.push({ name: 'reorderLabels', ok: () => res(), fail: () => rej(new real.ApiError('INTERNAL', '失败了')) });
        }),
    },
  };
});

let D: typeof Data;
/** 等挂起的 Promise 链走完（请求排队、刷新） */
const settle = () => new Promise(r => setTimeout(r, 0));
const title = (id: number) => D.useData.getState().board!.cards[id].title;
/** 服务端把卡片改成这样（模拟命令成功后的数据） */
const serverEdit = (path: string, id: number, change: object) => {
  const b = server.boards[path];
  b.cards[id] = { ...b.cards[id], ...change };
};

beforeEach(async () => {
  vi.resetModules(); // 队列、补丁层、刷新状态都是模块级的
  server.ws = workspace();
  server.boards = {
    'p/a': snapshot('p/a', { todo: [1, 2, 3] }),
    'p/b': snapshot('p/b', { todo: [1] }, [{ id: 1, title: 'B 白板的卡片' }]),
  };
  server.calls = [];
  server.other = [];
  server.hosts = null;
  D = await import('./data');
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
    await expect(a).rejects.toBeTruthy();
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
    await expect(a).rejects.toBeTruthy();
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
});
