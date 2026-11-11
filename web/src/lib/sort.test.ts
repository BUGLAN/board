/* 白板设置「保留顺序」的显示顺序（lib/sort.ts）：只改显示，不动优先级 */
import { describe, expect, it } from 'vitest';
import { FLOW, snapshot } from '../../test/fixtures';
import { orderDir, orderOf, shownCols, sortIds } from './sort';
import type { Label } from './types';

const labels: Label[] = [
  { id: 1, name: 'bug', color: '#f00', uses: 1 },
  { id: 2, name: 'ui', color: '#0f0', uses: 1 },
];

// 优先级顺序 [3, 1, 2]；移入时间 1 最新、3 最早；编号和移入时间的顺序故意不同
const board = (keepSorted: boolean, order?: { by: 'moved' | 'id' | 'labels' | 'updated'; desc: boolean }) => ({
  ...snapshot('p/a', { todo: [3, 1, 2], doing: [] }, [
    { id: 1, movedAt: 300, updatedAt: 10, labels: [2] },
    { id: 2, movedAt: 200, updatedAt: 30, labels: [] },
    { id: 3, movedAt: 100, updatedAt: 20, labels: [1] },
  ]),
  statuses: FLOW.map(s => (s.key === 'todo' && order ? { ...s, order } : s)),
  keepSorted,
});

describe('orderOf', () => {
  it('没有排序依据（旧服务端）时按移入时间，新移入的在最后；按标签不分方向', () => {
    expect(orderOf(FLOW[0])).toEqual({ by: 'moved', desc: false });
    expect(orderDir(orderOf(FLOW[0]))).toBe('新移入的在最后');
    expect(orderOf({ order: { by: 'labels', desc: true } })).toEqual({ by: 'labels', desc: false });
    expect(orderDir({ by: 'id', desc: true })).toBe('大号在前');
  });
});

describe('shownCols', () => {
  it('保留顺序关着：按优先级显示（拖到哪就在哪）', () => {
    const b = board(false, { by: 'id', desc: false });
    expect(shownCols(b, labels, 0)).toBe(b.cols);
  });

  it('保留顺序开着：每块按自己的排序依据，优先级不变', () => {
    expect(shownCols(board(true), labels, 0).todo).toEqual([3, 2, 1]);
    expect(shownCols(board(true, { by: 'moved', desc: true }), labels, 0).todo).toEqual([1, 2, 3]);
    expect(shownCols(board(true, { by: 'id', desc: false }), labels, 0).todo).toEqual([1, 2, 3]);
    expect(shownCols(board(true, { by: 'updated', desc: true }), labels, 0).todo).toEqual([2, 3, 1]);
    // 按项目里的标签顺序，没有标签的排最后
    expect(shownCols(board(true, { by: 'labels', desc: false }), labels, 0).todo).toEqual([3, 1, 2]);
    const b = board(true, { by: 'id', desc: true });
    shownCols(b, labels, 0);
    expect(b.cols.todo).toEqual([3, 1, 2]);
  });

  it('没有移入时间（旧服务端）时按新建时间', () => {
    const b = snapshot('p/a', { todo: [1, 2] }, [{ id: 1, createdAt: 50 }, { id: 2, createdAt: 10 }]);
    expect(sortIds([1, 2], b.cards, { by: 'moved', desc: false }, labels, 0)).toEqual([2, 1]);
  });
});
