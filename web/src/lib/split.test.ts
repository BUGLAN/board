/* 「未整理」里的拆解组（default/main#285）：原条目拆出卡之后隐藏，拆出的卡成组，可以撤销拆解 */
import { describe, expect, it, vi } from 'vitest';
import { snapshot } from '../../test/fixtures';
import { canUndoSplit, hiddenEntries, kidsOf, unitsOf, visibleInbox } from './split';

// 1 是原条目，拆成 3、4；2 是别的一条；5 是从待办里的 9 拆出来的
const board = () =>
  snapshot('p/a', { inbox: [1, 2, 3, 4, 5], todo: [9] }, [
    { id: 3, splitFrom: 1, createdAt: 30 },
    { id: 4, splitFrom: 1, createdAt: 40 },
    { id: 5, splitFrom: 9, createdAt: 50 },
    { id: 2, createdAt: 20 },
    { id: 1, createdAt: 10 },
  ]);

describe('隐藏已经拆出卡的原条目', () => {
  it('kidsOf：每张卡拆出了哪些，按编号', () => {
    const k = kidsOf(board().cards);
    expect(k.get(1)).toEqual([3, 4]);
    expect(k.get(9)).toEqual([5]);
    expect(k.has(2)).toBe(false);
  });

  it('只隐藏在「未整理」里、拆出过卡的原条目；计数和列表一致', () => {
    const b = board();
    expect([...hiddenEntries(b.cards, b.cols.inbox)]).toEqual([1]);
    expect(visibleInbox(b).map(c => c.id)).toEqual([2, 3, 4, 5]);
  });

  it('拆出的卡整理走了，原条目照样隐藏', () => {
    const b = snapshot('p/a', { inbox: [1, 2], todo: [3, 4] }, [
      { id: 3, splitFrom: 1 },
      { id: 4, splitFrom: 1 },
    ]);
    expect(visibleInbox(b).map(c => c.id)).toEqual([2]);
  });

  it('拆出的卡都删了（撤销拆解，进了回收站、不在白板上）：原条目回来', () => {
    const b = board();
    delete b.cards[3];
    delete b.cards[4];
    b.cols.inbox = [1, 2, 5];
    expect(visibleInbox(b).map(c => c.id)).toEqual([1, 2, 5]);
  });

  it('快照还没到：空', () => {
    expect(visibleInbox(null)).toEqual([]);
  });
});

describe('unitsOf（成组）', () => {
  it('拆自同一条的卡合成一组，放在组里第一张的位置；父条目不在未整理里的照旧一条一行', () => {
    const b = board();
    const list = visibleInbox(b).sort((x, y) => y.createdAt - x.createdAt);
    const units = unitsOf(list, { 1: b.cards[1] });
    expect(units.map(u => (u.kind === 'one' ? u.card.id : `${u.parent.id}:${u.cards.map(c => c.id).join(',')}`))).toEqual([5, '1:4,3', 2]);
  });

  it('筛选后组里只剩一张也成组', () => {
    const b = board();
    const units = unitsOf([b.cards[3]], { 1: b.cards[1] });
    expect(units).toEqual([{ kind: 'split', parent: b.cards[1], cards: [b.cards[3]] }]);
  });
});

describe('canUndoSplit（能不能撤销拆解）', () => {
  it('拆出的卡都还在未整理里才能撤销', () => {
    expect(canUndoSplit(board().cards, 1)).toBe(true);
    const b = board();
    b.cards[4] = { ...b.cards[4], column: 'todo' };
    expect(canUndoSplit(b.cards, 1), '有一张整理走了').toBe(false);
  });

  it('没拆过、拆出的卡都不在了：不能撤销', () => {
    expect(canUndoSplit(board().cards, 2)).toBe(false);
    expect(canUndoSplit(undefined, 1)).toBe(false);
  });
});

describe('撤销拆解的补丁：拆出的卡当帧拿掉，原条目当帧回来', () => {
  it('removeCard 补丁叠上去之后原条目可见；补丁重复执行结果相同', async () => {
    vi.resetModules();
    const D = await import('../store/data');
    const b = board();
    const once = D.removeCard(4).board!(D.removeCard(3).board!(b));
    expect(visibleInbox(once).map(c => c.id)).toEqual([1, 2, 5]);
    const again = D.removeCard(4).board!(D.removeCard(3).board!(once));
    expect(visibleInbox(again).map(c => c.id)).toEqual([1, 2, 5]);
  });
});
