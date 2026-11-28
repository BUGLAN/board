import { beforeEach, describe, expect, it, vi } from 'vitest';
import { card, detail, snapshot } from '../../test/fixtures';
import { openDetail, useData } from './data';

const server = vi.hoisted(() => ({ card: vi.fn() }));
vi.mock('../lib/api', async orig => {
  const real = await orig<typeof import('../lib/api')>();
  return { ...real, api: { ...real.api, card: server.card } };
});

const ids = Array.from({ length: 60 }, (_, i) => i + 1);

beforeEach(() => {
  server.card.mockReset();
  server.card.mockImplementation(async (_path: string, id: number) => detail(card(id)));
  useData.setState({ ws: null, board: snapshot('p/a', { todo: ids }), path: 'p/a', detailId: null, detail: null });
});

describe('卡片详情缓存', () => {
  it('打开 60 张详情后只留最近 50 张：11 还能直接打开，10 已经丢掉', async () => {
    for (const id of ids) await openDetail(id);
    expect(server.card).toHaveBeenCalledTimes(60);
    // 11 还在缓存里：请求还没回来时就能先显示它
    const hit = openDetail(11);
    expect(useData.getState().detail?.id).toBe(11);
    await hit;
    expect(server.card).toHaveBeenCalledTimes(61);
    // 10 已经因为超过 50 条被丢掉了：请求回来之前没有详情可显示
    const miss = openDetail(10);
    expect(useData.getState().detail).toBeNull();
    await miss;
    expect(server.card).toHaveBeenCalledTimes(62);
  });
});
