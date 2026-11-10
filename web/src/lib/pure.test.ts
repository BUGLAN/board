/* 不碰 DOM 的纯函数：拖拽落点（placeAt）、拖拽预览（previewCols）、工作流条排版（mainPath / layoutFlow） */
import { describe, expect, it } from 'vitest';
import { previewCols } from '../components/BoardView';
import type { DragUI } from '../store/ui';
import { FLOW, status } from '../../test/fixtures';
import { placeAt } from './drag';
import { CH, edgeId, layoutFlow, mainPath } from './flowGraph';

describe('placeAt（CSS 网格自动摆放）', () => {
  it('一样高的卡片按行从左到右排', () => {
    const spans = [10, 10, 10, 10, 10];
    expect([0, 1, 2, 3, 4].map(k => placeAt(spans, 3, k))).toEqual([
      { row: 0, col: 0 },
      { row: 0, col: 1 },
      { row: 0, col: 2 },
      { row: 10, col: 0 },
      { row: 10, col: 1 },
    ]);
  });

  it('换行时落到最先空出来的那一竖列', () => {
    // 第 0 列很高，第 1 列先空出来
    const spans = [30, 10, 10];
    expect(placeAt(spans, 2, 2)).toEqual({ row: 10, col: 1 });
  });

  it('游标只往后走：同一行里不回头填左边的空位', () => {
    // 第 0 列矮、第 1 列高：第 2 张换到下一行，第 3 张在同一行接着往右找（第 1 列还没空出来，再换行）
    const spans = [5, 20, 5, 5];
    expect(placeAt(spans, 2, 2)).toEqual({ row: 5, col: 0 });
    expect(placeAt(spans, 2, 3)).toEqual({ row: 10, col: 0 });
  });

  it('k 超出时返回最后的游标位置', () => {
    expect(placeAt([], 3, 0)).toEqual({ row: 0, col: 0 });
  });
});

describe('previewCols（拖拽预览）', () => {
  const cols = { todo: [1, 2, 3], doing: [4] };
  const drag = (over: Partial<DragUI>): DragUI => ({
    id: 1, fromCol: 'todo', col: 'todo', index: 0, over: null, valid: true, veto: null, ...over,
  });

  it('不在拖动时原样返回', () => {
    expect(previewCols(cols, null)).toBe(cols);
  });

  it('被拖的卡片放到预览位置，原位置去掉；不改原数据', () => {
    expect(previewCols(cols, drag({ col: 'doing', index: 0 }))).toEqual({ todo: [2, 3], doing: [1, 4] });
    expect(previewCols(cols, drag({ col: 'todo', index: 2 }))).toEqual({ todo: [2, 3, 1], doing: [4] });
    expect(cols).toEqual({ todo: [1, 2, 3], doing: [4] });
  });

  it('index 超出放最后；目标状态还没有卡片时新建', () => {
    expect(previewCols(cols, drag({ col: 'doing', index: 9 })).doing).toEqual([4, 1]);
    expect(previewCols(cols, drag({ col: 'review', index: 0 })).review).toEqual([1]);
  });
});

describe('工作流条排版', () => {
  it('mainPath：从首个可领取状态走到终点，优先不可领取的', () => {
    expect(mainPath(FLOW)).toEqual(['todo', 'doing', 'review', 'done']);
    expect(mainPath([])).toEqual([]);
  });

  it('mainPath：走不动就停；不重复走', () => {
    const list = [status('a', { claim: true, to: ['b'] }), status('b', { to: ['a'] }), status('c')];
    expect(mainPath(list)).toEqual(['a', 'b']);
  });

  it('layoutFlow：主线在第一行依次排开，其余在第二行；连线都有坐标', () => {
    const widths = { todo: 60, doing: 70, review: 80, done: 60, rework: 60 };
    const g = layoutFlow(FLOW, widths, 40);
    const main = ['todo', 'doing', 'review', 'done'];
    for (const k of main) expect(g.boxes[k].row).toBe(0);
    expect(g.boxes.rework.row).toBe(1);
    // 主线从左到右不重叠
    for (let i = 1; i < main.length; i++) {
      const a = g.boxes[main[i - 1]], b = g.boxes[main[i]];
      expect(b.x).toBeGreaterThanOrEqual(a.x + a.w);
    }
    expect(g.boxes.rework.y).toBeGreaterThan(g.boxes.todo.y + CH);
    // 每条连线都画出来
    const ids = FLOW.flatMap(s => s.to.map(t => edgeId(s.key, t)));
    expect(g.edges.map(e => e.id).sort()).toEqual(ids.sort());
    for (const e of g.edges) for (const [x, y] of e.pts) expect(Number.isFinite(x) && Number.isFinite(y)).toBe(true);
    // 「添加状态」在主线最后
    expect(g.add.x).toBeGreaterThan(g.boxes.done.x + g.boxes.done.w);
    expect(g.width).toBeGreaterThanOrEqual(g.add.x + 40);
  });

  it('layoutFlow：只有主线时没有第二行', () => {
    const list = FLOW.slice(0, 4).map(s => ({ ...s, to: s.to.filter(t => t !== 'rework') }));
    const g = layoutFlow(list, {}, 40);
    expect(g.height).toBe(CH);
    expect(Object.values(g.boxes).every(b => b.row === 0)).toBe(true);
  });
});
