/* 不碰 DOM 的纯函数：拖拽落点（placeAt）、拖拽预览（previewCols）、工作流条排版（mainPath / layoutFlow） */
import { describe, expect, it } from 'vitest';
import { pack, previewCols, withDefaults } from '../components/BoardView';
import type { DragUI } from '../store/ui';
import { FLOW, status } from '../../test/fixtures';
import { placeAt } from './drag';
import { CH, edgeId, layoutFlow, mainPath } from './flowGraph';
import { flowOf } from './schema';

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

describe('withDefaults（没保存过布局的状态块）', () => {
  const keys = ['todo', 'doing', 'review', 'done', 'rework', 'check'];
  const cols = (h: Record<string, number>) => {
    const tiles = withDefaults(keys, {});
    const pos = pack(keys, tiles, h);
    return Object.fromEntries(keys.map(k => [k, pos[k].x]));
  };

  it('第 i 个放第 i mod 4 栏，前 4 个并排在最上面', () => {
    const tiles = withDefaults(keys, {});
    expect(keys.map(k => tiles[k].x)).toEqual([0, 3, 6, 9, 0, 3]);
    const pos = pack(keys, tiles, {});
    expect(keys.slice(0, 4).map(k => pos[k].top)).toEqual([0, 0, 0, 0]);
  });

  it('块的高度不同（卡片数不同），各块所在的栏不变', () => {
    const a = cols({ todo: 300, doing: 80, review: 120, done: 400, rework: 80, check: 80 });
    const b = cols({ todo: 300, doing: 160, review: 120, done: 400, rework: 80, check: 80 });
    const c = cols({ todo: 80, doing: 600, review: 80, done: 80, rework: 300, check: 80 });
    expect(b).toEqual(a);
    expect(c).toEqual(a);
  });

  it('上下位置照旧贴紧上面的块', () => {
    const pos = pack(keys, withDefaults(keys, {}), { todo: 300, doing: 80 });
    expect(pos.rework.top).toBe(300 + 12);
    expect(pos.check.top).toBe(80 + 12);
  });

  it('已保存的位置原样保留，新增的状态按顺序取栏，与高度无关', () => {
    const saved = { todo: { x: 0, w: 6, y: 0 }, doing: { x: 6, w: 6, y: 0 } };
    const k3 = ['todo', 'doing', 'review'];
    const t = withDefaults(k3, saved);
    expect(t.todo).toEqual(saved.todo);
    expect(t.doing).toEqual(saved.doing);
    expect(t.review.x).toBe(6);
    const p1 = pack(k3, t, { todo: 100, doing: 400 });
    const p2 = pack(k3, t, { todo: 400, doing: 100 });
    expect(p1.review.x).toBe(p2.review.x);
  });
});

describe('nextCard（不带 ID 领取会领走的那张，PRD §8 规则 1）', () => {
  const f = flowOf(FLOW);
  const cols = { todo: [1, 2, 3], rework: [4, 5] };
  it('没有优先标签时按工作流顺序取最上面一张', () => {
    expect(f.nextCard(cols)).toBe(1);
    expect(f.nextCard(cols, id => id === 1)).toBe(2);
  });
  it('带优先标签的先领；同一状态里两张时先领上面那张；待办没有时去返工找', () => {
    expect(f.nextCard(cols, undefined, id => id === 3)).toBe(3);
    expect(f.nextCard(cols, undefined, id => id === 5 || id === 4)).toBe(4);
    expect(f.nextCard(cols, id => id === 4, id => id === 5 || id === 4)).toBe(5); // 排队中的跳过
  });
});

describe('since（完成于多久前）', () => {
  it('分钟 → 小时 → 天，随时间推移变化', async () => {
    const { since } = await import('./util');
    const t = 1_700_000_000_000;
    expect(since(t, t + 10e3)).toBe('刚刚');
    expect(since(t, t + 5 * 60e3)).toBe('5 分钟前');
    expect(since(t, t + 3 * 3600e3)).toBe('3 小时前');
    expect(since(t, t + 2 * 86400e3)).toBe('2 天前');
  });
});
