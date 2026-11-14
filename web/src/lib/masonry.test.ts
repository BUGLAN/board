import { describe, expect, it } from 'vitest';
import { pack, type Entry } from './masonry';
import { placeAt } from './drag';

const item = (rows: number): Entry => ({ kind: 'item', rows });
const head = (rows = 30, min = 1): Entry => ({ kind: 'head', rows, min });
const foot = (rows = 10): Entry => ({ kind: 'foot', rows });

describe('pack', () => {
  it('没有块时和网格自动摆放（drag.placeAt）一样', () => {
    const spans = [100, 60, 80, 40, 120, 30, 70];
    const out = pack(spans.map(item), 3);
    spans.forEach((_, k) => {
      const p = placeAt(spans, 3, k);
      expect([out[k].row, out[k].col]).toEqual([p.row, p.col]);
    });
  });

  it('块只占组里卡片需要的几栏，旁边接着放别的卡片', () => {
    // 4 栏：一组 2 张，后面 1 张单卡、再一组 2 张
    const out = pack([head(), item(100), item(80), foot(), item(90), head(), item(50), item(50), foot()], 4);
    expect(out[0]).toEqual({ row: 0, col: 0, cols: 2, rows: 30 });
    expect(out[1]).toMatchObject({ row: 30, col: 0 });
    expect(out[2]).toMatchObject({ row: 30, col: 1 });
    // 块的下沿取最低的那栏：30 + 100，组尾在它下面
    expect(out[3]).toEqual({ row: 130, col: 0, cols: 2, rows: 10 });
    // 单卡并排在块右边，不另起一行
    expect(out[4]).toMatchObject({ row: 0, col: 2 });
    // 第二组 2 栏宽，第 3 栏下面空着的地方放不下（只剩 1 栏），排到最先空出 2 栏的位置
    expect(out[5]).toMatchObject({ col: 2, cols: 2 });
    expect(out[5].row).toBe(90);
  });

  it('组里的卡比栏数多时在块里换行，块占满所有栏', () => {
    const out = pack([head(), item(50), item(60), item(70), foot()], 2);
    expect(out[0]).toMatchObject({ col: 0, cols: 2 });
    expect(out[3]).toMatchObject({ row: 80, col: 0 });
    expect(out[4]).toMatchObject({ row: 150, cols: 2 });
  });

  it('只有一栏（手机）时块占满宽度，组里的卡上下排', () => {
    const out = pack([item(40), head(), item(50), item(60), foot(), item(40)], 1);
    expect(out.map(p => [p.row, p.col, p.cols])).toEqual([
      [0, 0, 1],
      [40, 0, 1],
      [70, 0, 1],
      [120, 0, 1],
      [180, 0, 1],
      [190, 0, 1],
    ]);
  });

  it('块至少占 min 栏（磁贴一栏太窄放不下组头），不超过总栏数', () => {
    expect(pack([head(22, 5), item(28), item(28), foot()], 12)[0].cols).toBe(5);
    expect(pack([head(22, 5), item(28), item(28), foot()], 3)[0].cols).toBe(3);
  });

  it('起始行单调不减（阅读顺序和优先级一致）', () => {
    const out = pack([item(80), head(), item(40), item(90), foot(), item(30), item(30), head(), item(20), item(20), foot(), item(50)], 3);
    const starts = [0, 1, 5, 6, 7, 11].map(i => out[i].row);
    expect([...starts].sort((a, b) => a - b)).toEqual(starts);
  });
});
