/* 「未整理」里的拆解组（default/main#285）：原条目拆出卡之后隐藏，拆出的卡成组，可以撤销拆解 */
import { describe, expect, it, vi } from 'vitest';
import { card, snapshot } from '../../test/fixtures';
import { canUndoSplit, hiddenEntries, kidsOf, laterEntries, omittedKids, laterGroups, timeGroups, unitsOf, visibleInbox } from './split';
import type { Card } from './types';

// 1 是原条目，拆成 3、4；2 是别的一条；5 是从待办里的 9 拆出来的
const board = () =>
  snapshot('p/a', { inbox: [1, 2, 3, 4, 5], todo: [9] }, [
    { id: 3, splitFrom: 1, createdAt: 30 },
    { id: 4, splitFrom: 1, createdAt: 40 },
    { id: 5, splitFrom: 9, createdAt: 50 },
    { id: 2, createdAt: 20 },
    { id: 1, createdAt: 10 },
  ]);

// 同一块白板，服务端记着「拆解过」（default/main#735）：1、9 拆过，其余没拆过
const marked = () => {
  const b = board();
  for (const c of Object.values(b.cards)) b.cards[c.id] = { ...c, splitAt: c.id === 1 || c.id === 9 ? 30 : null };
  return b;
};

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

  it('旧服务端（没有 splitAt）：拆出的卡都删了，原条目回来', () => {
    const b = board();
    delete b.cards[3];
    delete b.cards[4];
    b.cols.inbox = [1, 2, 5];
    expect(visibleInbox(b).map(c => c.id)).toEqual([1, 2, 5]);
  });

  it('看 splitAt：拆出的卡都删了（不在白板上），原条目照样隐藏（default/main#735）', () => {
    const b = marked();
    expect(visibleInbox(b).map(c => c.id)).toEqual([2, 3, 4, 5]);
    delete b.cards[3];
    delete b.cards[4];
    b.cols.inbox = [1, 2, 5];
    expect([...hiddenEntries(b.cards, b.cols.inbox)]).toEqual([1]);
    expect(visibleInbox(b).map(c => c.id)).toEqual([2, 5]);
  });

  it('看 splitAt：撤销拆解清掉之后原条目回来；快照省略的拆出卡不再决定隐藏', () => {
    const b = marked();
    b.cards[1] = { ...b.cards[1], splitAt: null };
    expect(visibleInbox({ ...b, splitOmitted: { 1: 2 } }).map(c => c.id)).toEqual([1, 2, 3, 4, 5]);
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

  it('拆出的卡都被快照省略了（整理到终点状态、挤出最近几张）：原条目照样隐藏、计数不算它（default/main#534）', () => {
    // 2 拆出的卡都在已完成里、被快照省略：快照里认不出，只有 splitOmitted 记着
    const b = { ...board(), splitOmitted: { 2: 1 } };
    expect([...hiddenEntries(b.cards, b.cols.inbox, b.splitOmitted)]).toEqual([1, 2]);
    expect(visibleInbox(b).map(c => c.id)).toEqual([3, 4, 5]);
    expect(omittedKids(b.splitOmitted, 2)).toBe(1);
    expect(omittedKids(undefined, 2), '旧服务端没有这个字段').toBe(0);
    // 旧服务端：按原来的办法，只看快照里的卡
    expect(visibleInbox(board()).map(c => c.id)).toEqual([2, 3, 4, 5]);
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

  it('还有拆出的卡被快照省略（在终点状态里）：不能撤销，免得只删掉看得见的几张', () => {
    expect(canUndoSplit(board().cards, 1, { 1: 1 })).toBe(false);
    expect(canUndoSplit(board().cards, 1, { 9: 1 }), '别的原条目的不相干').toBe(true);
  });

  it('没拆过、拆出的卡都不在了：不能撤销', () => {
    expect(canUndoSplit(board().cards, 2)).toBe(false);
    expect(canUndoSplit(undefined, 1)).toBe(false);
  });
});

describe('撤销拆解的补丁：拆出的卡当帧拿掉，原条目当帧回来', () => {
  it('unsplitPatch 叠上去之后原条目可见；补丁重复执行结果相同（default/main#735）', async () => {
    vi.resetModules();
    const D = await import('../store/data');
    const p = D.unsplitPatch(1, [3, 4]);
    const once = p.board!(marked());
    expect(visibleInbox(once).map(c => c.id)).toEqual([1, 2, 5]);
    expect(once.cards[1].splitAt).toBeNull();
    expect(once.trash).toBe(2);
    const again = p.board!(once);
    expect(again).toEqual(once);
  });

  it('只删拆出的卡不让原条目回来；恢复拆出的卡后原条目又记上拆解过、并进那一组', async () => {
    vi.resetModules();
    const D = await import('../store/data');
    const gone = D.removeCard(4).board!(D.removeCard(3).board!(marked()));
    expect(visibleInbox(gone).map(c => c.id)).toEqual([2, 5]);
    const undone = D.unsplitPatch(1, [3, 4]).board!(D.removeCard(4).board!(D.removeCard(3).board!(marked())));
    expect(visibleInbox(undone).map(c => c.id)).toEqual([1, 2, 5]);
    const back = D.restoreCard({ id: 3, column: 'inbox', pos: 2, movedAt: 99, card: marked().cards[3] }).board!(undone);
    expect(back.cards[1].splitAt).toBe(99);
    expect(visibleInbox(back).map(c => c.id)).toEqual([2, 3, 5]);
  });
});

describe('稍后再做（default/main#303）', () => {
  const b = () => {
    const cards: Record<number, Card> = {};
    for (const [id, laterAt] of [[1, null], [2, 100], [3, null], [4, 200]] as const) cards[id] = { ...card(id, { column: 'inbox' }), laterAt } as Card;
    return { cards, cols: { inbox: [1, 2, 3, 4] } };
  };
  it('visibleInbox 不含收起的；laterEntries 只列收起的，最近收起的在前', () => {
    expect(visibleInbox(b()).map(c => c.id)).toEqual([1, 3]);
    expect(laterEntries(b()).map(c => c.id)).toEqual([4, 2]);
    expect(laterEntries(null)).toEqual([]);
  });
  it('laterGroups 按收起时间分今天、这一周、更早，空组不出现', () => {
    const now = new Date(2026, 10, 14, 15, 0).getTime();
    const at = (d: number, h = 12) => new Date(2026, 10, 14 - d, h, 0).getTime();
    const cs = [
      { ...card(1), laterAt: at(0, 9) },
      { ...card(2), laterAt: at(1, 23) },
      { ...card(3), laterAt: at(10) },
      { ...card(4), laterAt: at(0, 1) },
    ] as Card[];
    expect(laterGroups(cs, now).map(g => [g.key, g.rows.map(c => c.id)])).toEqual([
      ['today', [1, 4]],
      ['week', [2]],
      ['older', [3]],
    ]);
    expect(laterGroups([cs[2]], now).map(g => g.label)).toEqual(['更早收起']);
    expect(laterGroups([], now)).toEqual([]);
  });

  it('timeGroups：回收站按删除时间分组，组名跟着动作，组内保持传入的顺序（#486）', () => {
    const now = new Date(2026, 10, 18, 15, 0).getTime();
    const at = (d: number, h = 12) => new Date(2026, 10, 18 - d, h, 0).getTime();
    const ts = [
      { id: 7, deletedAt: at(0, 14) },
      { id: 5, deletedAt: at(0, 8) },
      { id: 3, deletedAt: at(3) },
      { id: 2, deletedAt: at(30) },
    ];
    expect(timeGroups(ts, t => t.deletedAt, now, '删除').map(g => [g.label, g.rows.map(t => t.id)])).toEqual([
      ['今天删除', [7, 5]],
      ['这一周删除', [3]],
      ['更早删除', [2]],
    ]);
  });
});
