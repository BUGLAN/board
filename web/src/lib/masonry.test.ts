import { describe, expect, it, vi } from 'vitest';
import { explicitTracks, measure, pack, type Entry } from './masonry';
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

describe('pack：块里一行排得下的卡一样高', () => {
  it('同一行的卡按最高的那张占行，上下沿对齐；块的下沿跟着最高的那张，default/main#462', () => {
    const out = pack([head(26), item(150), item(130), foot(12)], 3);
    expect(out[1]).toMatchObject({ row: 26, rows: 150, even: true });
    expect(out[2]).toMatchObject({ row: 26, rows: 150, even: true });
    expect(out[3].row).toBe(176);
  });

  it('排成多行（卡比栏多）、只有一张、本来就一样高时不拉高', () => {
    const many = pack([head(26), item(150), item(130), item(90), foot(12)], 2);
    expect(many.slice(1, 4).map(p => [p.rows, p.even])).toEqual([[150, undefined], [130, undefined], [90, undefined]]);
    const one = pack([head(26), item(150), foot(12)], 2);
    expect(one[1].even).toBeUndefined();
    const same = pack([head(26), item(41), item(41), foot(12)], 3);
    expect([same[1].even, same[2].even]).toEqual([undefined, undefined]);
  });
});

describe('explicitTracks', () => {
  it('只数显式栏：块变窄后旧位置撑出的隐式栏（grid-auto-columns: 0，宽 0）不算，default/main#387', () => {
    // 块从 17 栏拖窄到 11 栏：计算样式列出 11 个显式栏 + 6 个隐式栏
    const template = [...Array(11).fill('38.3984px'), ...Array(6).fill('0px')].join(' ');
    expect(explicitTracks(template)).toHaveLength(11);
    expect(explicitTracks(template)[0]).toBeCloseTo(38.3984);
  });

  it('没有隐式栏时原样返回；none（还没排版）时为空，由调用方按 1 栏处理', () => {
    expect(explicitTracks('220px 280.5px')).toEqual([220, 280.5]);
    expect(explicitTracks('none')).toEqual([]);
  });
});

describe('measure', () => {
  const el = (h: number, attr?: string, marginBottom?: string) => {
    const e = document.createElement('div');
    if (attr) e.setAttribute(attr, '');
    if (marginBottom) e.style.marginBottom = marginBottom;
    vi.spyOn(e, 'offsetHeight', 'get').mockReturnValue(h);
    return e;
  };

  it('卡片、组尾按高度 + 间距占行；量不到高度时占 1 行', () => {
    expect(measure(el(100), 8, 2)).toEqual({ kind: 'item', rows: 108 });
    expect(measure(el(4, 'data-block-end'), 8, 2)).toEqual({ kind: 'foot', rows: 12 });
    expect(measure(el(0), 8, 2)).toEqual({ kind: 'item', rows: 1 });
  });

  it('组头的负 margin-bottom 算进行数：块里第一行的卡一起上移、上沿对齐，default/main#462', () => {
    expect(measure(el(22, 'data-block', '-4px'), 8, 2)).toEqual({ kind: 'head', rows: 26, min: 2 });
    expect(measure(el(22, 'data-block'), 8, 2)).toEqual({ kind: 'head', rows: 30, min: 2 });
    // 第一行两张卡起始行相同
    const out = pack([measure(el(22, 'data-block', '-4px'), 8, 2), item(100), item(80), foot(12)], 2);
    expect(out[1].row).toBe(26);
    expect(out[2].row).toBe(26);
  });
});
