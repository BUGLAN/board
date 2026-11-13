/* 地址栏（lib/route.ts + store）：打开卡片、关闭、切换白板时写地址；
   后退、前进（popstate 后调用 applyRoute）按地址改界面，而且不再往地址栏推新的记录。
   服务端是下面的假 api，和 queue.test.ts 一样 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardEvent, BoardSnapshot, CardDetail, Cmd, Workspace } from '../lib/types';
import { detail, snapshot, workspace } from '../../test/fixtures';
import type * as Data from './data';

const server = vi.hoisted(() => ({
  ws: null as unknown as Workspace,
  boards: {} as Record<string, BoardSnapshot>,
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
      cmd: (_board: string, cmd: Cmd) => Promise.resolve(detail(clone(server.boards['p/a'].cards['id' in cmd ? cmd.id ?? 1 : 1])) as CardDetail),
    },
  };
});

let D: typeof Data;
let U: typeof import('./ui');
const settle = () => new Promise(r => setTimeout(r, 0));
const href = () => `${location.pathname}${location.search}`;

beforeEach(async () => {
  history.replaceState(null, '', '/');
  vi.resetModules(); // 地址、补丁层、刷新状态都是模块级的
  server.ws = workspace();
  server.boards = {
    'p/a': snapshot('p/a', { todo: [1, 2] }),
    'p/b': snapshot('p/b', { todo: [1] }, [{ id: 1, title: 'B 白板的卡片' }]),
  };
  D = await import('./data');
  U = await import('./ui');
  await D.refresh();
  expect(D.useData.getState().path).toBe('p/a');
  // 打开页面时地址里没有白板：刷新后补上
  expect(href()).toBe('/p/a');
});

describe('打开、关闭卡片和切换白板写进地址栏', () => {
  it('打开详情是 push，地址带上卡片编号；关闭也 push 回白板', async () => {
    const push = vi.spyOn(history, 'pushState');
    await D.openDetail(1);
    await settle();
    expect(href()).toBe('/p/a/1');
    expect(push).toHaveBeenCalledTimes(1);
    D.closeDetail();
    expect(href()).toBe('/p/a');
    expect(push).toHaveBeenCalledTimes(2);
    push.mockRestore();
  });

  it('在详情里切上一张 / 下一张是 replace（后退不会一张张回放）', async () => {
    await D.openDetail(1);
    const push = vi.spyOn(history, 'pushState');
    const replace = vi.spyOn(history, 'replaceState');
    await D.openDetail(2);
    await settle();
    expect(href()).toBe('/p/a/2');
    expect(push).not.toHaveBeenCalled();
    expect(replace).toHaveBeenCalled();
    push.mockRestore();
    replace.mockRestore();
  });

  it('切换白板是 push，地址换成新白板、详情关掉', async () => {
    await D.openDetail(1);
    history.replaceState(null, '', '/p/a/1');
    await D.setPath('p/b');
    await settle();
    expect(href()).toBe('/p/b');
    expect(D.useData.getState().detailId).toBeNull();
  });

  it('其他 Tab 用 ?tab=，地址里带着当前白板', () => {
    U.setTab('inbox');
    expect(href()).toBe('/p/a?tab=inbox');
    U.setTab('hosts');
    expect(href()).toBe('/p/a?tab=hosts');
  });
});

describe('后退、前进（applyRoute）', () => {
  it('按地址换白板、打开卡片，不再往地址栏推记录', async () => {
    // 浏览器已经把地址改成了这一条（popstate 之前），界面状态还是 p/a 没有详情
    history.replaceState(null, '', '/p/b/1');
    const push = vi.spyOn(history, 'pushState');
    const replace = vi.spyOn(history, 'replaceState');
    D.applyRoute({ path: 'p/b', card: 1, tab: 'board' });
    await settle();
    await settle();
    expect(D.useData.getState().path).toBe('p/b');
    expect(D.useData.getState().detailId).toBe(1);
    expect(href()).toBe('/p/b/1');
    expect(push).not.toHaveBeenCalled();
    expect(replace).not.toHaveBeenCalled();
    push.mockRestore();
    replace.mockRestore();
  });

  it('后退关掉详情：地址里没有卡片时详情关闭', async () => {
    await D.openDetail(1);
    history.replaceState(null, '', '/p/a');
    D.applyRoute({ path: 'p/a', card: null, tab: 'board' });
    expect(D.useData.getState().detailId).toBeNull();
  });

  it('后退切回别的 Tab：Tab 跟着地址变', () => {
    history.replaceState(null, '', '/p/a?tab=stats');
    D.applyRoute({ path: 'p/a', card: null, tab: 'stats' });
    expect(U.useUI.getState().tab).toBe('stats');
    expect(href()).toBe('/p/a?tab=stats'); // 没有再写一遍
  });
});

describe('详情里的返回 / 前进（#195）', () => {
  it('从一张卡片跳到另一张（描述里的编号、上一张 / 下一张）：能返回，再能前进', async () => {
    await D.openDetail(1);
    await D.jumpDetail(2);
    await settle();
    expect(href()).toBe('/p/a/2');
    expect(D.canCardBack()).toBe(true);
    expect(D.canCardFwd()).toBe(false);

    D.cardBackStep();
    await settle();
    expect(D.useData.getState().detailId).toBe(1);
    expect(href()).toBe('/p/a/1');
    expect(D.canCardBack()).toBe(false);
    expect(D.canCardFwd()).toBe(true);

    D.cardFwdStep();
    await settle();
    expect(D.useData.getState().detailId).toBe(2);
    expect(href()).toBe('/p/a/2');
  });

  it('同一张不算跳转；在两张之间来回跳时一层层退回', async () => {
    await D.openDetail(1);
    await D.jumpDetail(2);
    await settle();
    await D.jumpDetail(2); // 还是这一张：不记
    await D.jumpDetail(1);
    await settle();
    expect(D.useData.getState().detailId).toBe(1);
    D.cardBackStep();
    await settle();
    expect(D.useData.getState().detailId).toBe(2);
    D.cardBackStep();
    await settle();
    expect(D.useData.getState().detailId).toBe(1);
    expect(D.canCardBack()).toBe(false);
  });

  it('跳到另一块白板上的卡片只算一条地址：后退直接回到原来那张', async () => {
    await D.openDetail(1);
    const push = vi.spyOn(history, 'pushState');
    await D.jumpToCard('p/b', 1);
    await settle();
    await settle();
    expect(push).not.toHaveBeenCalled(); // 换掉当前这条，不是用两条记录夹出一页只有白板的地址
    expect(href()).toBe('/p/b/1');
    push.mockRestore();
  });

  it('跳到另一块白板上的卡片：返回时先回到原来那块白板', async () => {
    await D.openDetail(1);
    await D.jumpToCard('p/b', 1);
    await settle();
    await settle();
    expect(D.useData.getState().path).toBe('p/b');
    expect(D.useData.getState().detailId).toBe(1);
    D.cardBackStep();
    await settle();
    await settle();
    expect(D.useData.getState().path).toBe('p/a');
    expect(D.useData.getState().detailId).toBe(1);
  });

  it('关掉详情后不再有可返回的；重新打开的卡片从头开始记', async () => {
    await D.openDetail(1);
    await D.jumpDetail(2);
    await settle();
    D.closeDetail();
    await D.openDetail(1);
    expect(D.canCardBack()).toBe(false);
    expect(D.canCardFwd()).toBe(false);
  });

  it('浏览器后退（applyRoute）不留下可返回的记录', async () => {
    await D.openDetail(1);
    history.replaceState(null, '', '/p/a/3');
    D.applyRoute({ path: 'p/a', card: 3, tab: 'board' });
    await settle();
    expect(D.useData.getState().detailId).toBe(3);
    expect(D.canCardBack()).toBe(false);
  });
});

describe('打开页面时的地址（卡片链接）', () => {
  it('刷新后直接打开地址里的卡片', async () => {
    history.replaceState(null, '', '/p/b/1');
    vi.resetModules();
    D = await import('./data');
    await D.refresh();
    expect(D.useData.getState().path).toBe('p/b');
    expect(D.useData.getState().detailId).toBe(1);
    expect(D.useData.getState().detail?.title).toBe('B 白板的卡片');
  });

  it('地址里的白板不在了：换成记住的白板，地址跟着改，不打开卡片', async () => {
    history.replaceState(null, '', '/gone/board/9');
    vi.resetModules();
    D = await import('./data');
    await D.refresh();
    expect(D.useData.getState().path).toBe('p/a');
    expect(D.useData.getState().detailId).toBeNull();
    expect(href()).toBe('/p/a');
  });
});

// 别让类型检查以为 BoardEvent 没用上（假 api 的 connect 用不到事件）
export type _Unused = BoardEvent;
