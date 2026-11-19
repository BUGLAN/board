import { describe, expect, it } from 'vitest';
import { collapseCols, earlierKey, FINAL_RECENT, recentIds } from './earlier';
import type { BoardSnapshot, Card, Status } from './types';

const statuses = [
  { key: 'todo', name: '待办', claim: true },
  { key: 'done', name: '已完成', final: true },
] as unknown as Status[];

/** 已完成里 n 张：编号 1..n，movedAt 打乱（编号越大不一定越新），cols 里按 pos 排（编号顺序） */
function board(n: number, omitted?: Record<string, number>): Pick<BoardSnapshot, 'cards' | 'cols' | 'omitted' | 'statuses' | 'path'> {
  const cards: Record<number, Card> = {};
  const ids = Array.from({ length: n }, (_, i) => i + 1);
  for (const id of ids) cards[id] = { id, column: 'done', movedAt: (id * 37) % 101, createdAt: 0 } as Card;
  cards[999] = { id: 999, column: 'todo', movedAt: 0, createdAt: 0 } as Card;
  return { path: 'p/b', statuses, cards, cols: { todo: [999], done: ids }, omitted };
}

describe('终点状态的「更早的 N 张」（#485）', () => {
  it('快照省略了卡片：原样，末尾是更早的张数；点了展开、全量还没回来时 open', () => {
    const b = board(10, { done: 7 });
    const r = collapseCols(b, b.cols, {}, false);
    expect(r.cols.done).toEqual(b.cols.done);
    expect(r.more).toEqual({ done: { n: 7, open: false } });
    expect(collapseCols(b, b.cols, { [earlierKey('p/b', 'done')]: true }, false).more.done).toEqual({ n: 7, open: true });
  });

  it('全量快照、没展开：只留最近移入的 FINAL_RECENT 张，顺序不变；展开：全部，末尾是「收起」', () => {
    const b = board(FINAL_RECENT + 12);
    const r = collapseCols(b, b.cols, {}, false);
    expect(r.cols.done).toHaveLength(FINAL_RECENT);
    expect(r.more.done).toEqual({ n: 12, open: false });
    const keep = recentIds(b.cards, b.cols.done);
    expect(r.cols.done).toEqual(b.cols.done.filter(id => keep.has(id)));
    // 留下的都比去掉的新
    const at = (id: number) => b.cards[id].movedAt ?? 0;
    const gone = b.cols.done.filter(id => !keep.has(id));
    expect(Math.min(...r.cols.done.map(at))).toBeGreaterThanOrEqual(Math.max(...gone.map(at)));
    expect(r.cols.todo).toEqual([999]);
    expect(r.more.todo).toBeUndefined();

    const open = collapseCols(b, b.cols, { [earlierKey('p/b', 'done')]: true }, false);
    expect(open.cols.done).toEqual(b.cols.done);
    expect(open.more.done).toEqual({ n: 0, open: true });
  });

  it('筛选时不截断、没有那一格；不超过 FINAL_RECENT 张时也没有', () => {
    const b = board(FINAL_RECENT + 3);
    const r = collapseCols(b, b.cols, {}, true);
    expect(r.cols.done).toEqual(b.cols.done);
    expect(r.more).toEqual({});
    const small = board(FINAL_RECENT);
    expect(collapseCols(small, small.cols, { [earlierKey('p/b', 'done')]: true }, false).more).toEqual({});
  });

  it('只按这块白板的展开状态；不改输入', () => {
    const b = board(FINAL_RECENT + 1);
    const before = JSON.stringify(b);
    expect(collapseCols(b, b.cols, { [earlierKey('other/b', 'done')]: true }, false).more.done).toEqual({ n: 1, open: false });
    expect(JSON.stringify(b)).toBe(before);
  });
});
