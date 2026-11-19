/* 终点状态「更早的 N 张」的全量加载（default/main#511）：full=1 失败后不留下「已取全量」的标记，
   展开状态收回、按钮可以再点；重试成功后才记住全量，之后的刷新继续带 full、再展开不重复请求。
   服务端是下面的假 api：full=1 的结果由用例决定成功、失败还是仍旧省略 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardSnapshot, Workspace } from '../lib/types';
import { snapshot, workspace } from '../../test/fixtures';
import type * as Data from './data';

const server = vi.hoisted(() => ({
  ws: null as unknown as Workspace,
  /** 普通快照：终点状态还省略着更早的卡片 */
  lite: null as unknown as BoardSnapshot,
  /** full=1 的快照：所有卡片都在 */
  full: null as unknown as BoardSnapshot,
  /** full=1 这次请求的结果 */
  fullMode: 'ok' as 'ok' | 'fail' | 'omit',
  calls: [] as { path: string; full: boolean }[],
}));

vi.mock('../lib/api', async orig => {
  const real = await orig<typeof import('../lib/api')>();
  return {
    ...real,
    api: {
      workspace: async () => structuredClone(server.ws),
      board: async (path: string, full = false) => {
        server.calls.push({ path, full });
        if (!full) return structuredClone(server.lite);
        if (server.fullMode === 'fail') throw new real.ApiError('NETWORK', '连不上 Board 服务');
        return structuredClone(server.fullMode === 'omit' ? server.lite : server.full);
      },
    },
  };
});

let D: typeof Data;

beforeEach(async () => {
  vi.resetModules(); // fullPaths / fullWanted / 展开状态都是模块级的
  server.ws = workspace();
  const lite = snapshot('p/a', { done: [1, 2], todo: [3] });
  lite.omitted = { done: 60 };
  server.lite = lite;
  server.full = snapshot('p/a', { done: [1, 2, 4, 5], todo: [3] });
  server.fullMode = 'ok';
  server.calls = [];
  D = await import('./data');
  await D.refresh();
  server.calls = []; // 开局那一轮不算
});

describe('全量快照失败后的重试', () => {
  it('full=1 失败：不留标记、展开状态收回，重试成功后才展开', async () => {
    server.fullMode = 'fail';
    expect(await D.setEarlier('p/a', 'done', true)).toBe(false);
    expect(D.useEarlier.getState().open).toEqual({});
    expect(D.useData.getState().board!.omitted).toEqual({ done: 60 });
    expect(server.calls.map(c => c.full)).toEqual([true]);

    // 失败的尝试没有把这块白板变成长久全量：普通刷新还是省略快照
    await D.refresh();
    expect(server.calls.map(c => c.full)).toEqual([true, false]);

    // 再点一次：重新请求 full=1，成功后展开、拿到全部卡片
    server.fullMode = 'ok';
    expect(await D.setEarlier('p/a', 'done', true)).toBe(true);
    expect(server.calls.map(c => c.full)).toEqual([true, false, true]);
    expect(D.useEarlier.getState().open).toEqual({ 'p/a\ndone': true });
    expect(D.useData.getState().board!.omitted).toBeUndefined();
    expect(D.useData.getState().board!.cols.done).toEqual([1, 2, 4, 5]);
  });

  it('full=1 返回的快照还省略着：同样算失败，不留下标记', async () => {
    server.fullMode = 'omit';
    expect(await D.loadFull()).toBe(false);
    expect(D.useData.getState().board!.omitted).toEqual({ done: 60 });

    server.fullMode = 'ok';
    expect(await D.loadFull()).toBe(true);
    expect(server.calls.map(c => c.full)).toEqual([true, true]);
    expect(D.useData.getState().board!.omitted).toBeUndefined();
  });

  it('成功后不再重取：再展开、收起都不发请求；之后的刷新继续带 full', async () => {
    expect(await D.loadFull()).toBe(true);
    expect(server.calls.map(c => c.full)).toEqual([true]);

    expect(await D.loadFull()).toBe(true);
    await D.setEarlier('p/a', 'done', false);
    expect(server.calls).toHaveLength(1);

    await D.refresh();
    expect(server.calls.map(c => c.full)).toEqual([true, true]);
  });
});
