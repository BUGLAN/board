/* 乐观更新的补丁（store/patches.ts）：纯函数，每次刷新都会重新叠到新数据上，
   所以要能重复执行（结果不变），目标不在了原样返回 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardSnapshot, Dispatch, Workspace } from '../lib/types';
import { card, detail, snapshot, workspace } from '../../test/fixtures';
import type * as Patches from './patches';
import type * as State from './state';

let D: typeof Patches;
let S: typeof State;
beforeEach(async () => {
  // draftCard 有模块级状态（临时 ID、已认领的真卡片）：每个用例一份新的
  vi.resetModules();
  D = await import('./patches');
  S = await import('./state');
});

const board = () => snapshot('p/a', { todo: [1, 2, 3], doing: [4], review: [] });

/** 补丁叠两次和叠一次结果相同 */
function twice<T>(fn: (x: T) => T, x: T) {
  const once = fn(x);
  expect(fn(once)).toEqual(once);
  return once;
}

/** 每张卡片在所有状态里一共只出现一次 */
function eachOnce(b: BoardSnapshot) {
  const ids = Object.values(b.cols).flat();
  expect(ids.length).toBe(new Set(ids).size);
  expect(new Set(ids)).toEqual(new Set(Object.keys(b.cards).map(Number)));
}

describe('movePatch', () => {
  it('换状态：放到指定位置，原状态里去掉，卡片的 column 跟着改', () => {
    const b = twice(D.movePatch(2, 'doing', 0).board!, board());
    expect(b.cols).toEqual({ todo: [1, 3], doing: [2, 4], review: [] });
    expect(b.cards[2].column).toBe('doing');
    eachOnce(b);
  });

  it('同一状态内调整顺序；index 超出时放最后；省略时放最后', () => {
    expect(twice(D.movePatch(3, 'todo', 0).board!, board()).cols.todo).toEqual([3, 1, 2]);
    expect(twice(D.movePatch(1, 'todo', 99).board!, board()).cols.todo).toEqual([2, 3, 1]);
    expect(twice(D.movePatch(4, 'review').board!, board()).cols).toEqual({ todo: [1, 2, 3], doing: [], review: [4] });
  });

  it('目标状态还不在 cols 里时新建', () => {
    const b = twice(D.movePatch(1, 'rework').board!, board());
    expect(b.cols.rework).toEqual([1]);
    eachOnce(b);
  });

  it('带 order：按 order 排，没列出的卡片补在后面', () => {
    const b = twice(D.movePatch(4, 'todo', 0, [3, 4]).board!, board());
    expect(b.cols.todo).toEqual([3, 4, 1, 2]);
    eachOnce(b);
  });

  it('order 里有不在这个状态的卡片：忽略', () => {
    const b = twice(D.movePatch(1, 'todo', 0, [9, 2, 1]).board!, board());
    expect(b.cols.todo).toEqual([2, 1, 3]);
  });

  it('extra 一起改；详情只改这一张', () => {
    const p = D.movePatch(4, 'todo', 0, undefined, { holder: null });
    const b = twice(p.board!, snapshot('p/a', { todo: [1], doing: [4] }, [{ id: 4, holder: 'claude/x' }]));
    expect(b.cards[4]).toMatchObject({ holder: null, column: 'todo' });
    const d4 = detail(card(4, { column: 'doing', holder: 'claude/x' }));
    expect(twice(p.detail!, d4)).toMatchObject({ holder: null, column: 'todo' });
    const d1 = detail(card(1));
    expect(p.detail!(d1)).toBe(d1);
    expect(p.cards).toEqual([4]);
  });

  it('卡片不在了：原样返回', () => {
    const b = board();
    expect(D.movePatch(9, 'doing', 0).board!(b)).toBe(b);
  });

  it('换状态时移入时间记成发起的时刻（建补丁时取定，之后重复执行不变）；同一状态里调整顺序不改', () => {
    const now = vi.spyOn(Date, 'now').mockReturnValue(5_000);
    const p = D.movePatch(2, 'doing', 0);
    now.mockReturnValue(9_000);
    const b = twice(p.board!, board());
    expect(b.cards[2].movedAt).toBe(5_000);
    // 服务端确认后（卡片已经在目标状态）以服务端的为准
    const confirmed = snapshot('p/a', { todo: [1, 3], doing: [2, 4] }, [{ id: 2, movedAt: 7_000 }]);
    expect(p.board!(confirmed).cards[2].movedAt).toBe(7_000);
    const same = twice(D.movePatch(3, 'todo', 0).board!, snapshot('p/a', { todo: [1, 2, 3] }, [{ id: 3, movedAt: 100 }]));
    expect(same.cards[3].movedAt).toBe(100);
    now.mockRestore();
  });

  it('换状态时移入、更新时间至少比快照里最新的还新：连着拖两张，前一张确认后后一张仍是最新的（慢网下确认时不换位）', () => {
    const now = vi.spyOn(Date, 'now').mockReturnValue(5_000);
    const first = D.movePatch(1, 'review');
    now.mockReturnValue(6_000);
    const second = D.movePatch(2, 'review');
    now.mockRestore();
    const base = snapshot('p/a', { todo: [1, 2, 3], doing: [4], review: [] });
    const both = second.board!(first.board!(base));
    expect(both.cards[2].movedAt!).toBeGreaterThan(both.cards[1].movedAt!);
    // 第一张确认了：服务端处理它的时刻（7000）比第二张的发起时刻（6000）还晚，第二张仍排在它后面
    const confirmed = snapshot('p/a', { todo: [2, 3], doing: [4], review: [1] }, [{ id: 1, movedAt: 7_000, updatedAt: 7_000 }]);
    const b = twice(second.board!, first.board!(confirmed));
    expect(b.cards[1]).toMatchObject({ movedAt: 7_000, updatedAt: 7_000 });
    expect(b.cards[2]).toMatchObject({ movedAt: 7_001, updatedAt: 7_001 });
  });

  it('批量移动：同一批的移入时间一样（服务端一个事务写的也一样，按优先级顺序排），仍比快照里已有的新', () => {
    const now = vi.spyOn(Date, 'now').mockReturnValue(5_000);
    const of = D.batchMovePatch([1, 2, 3], 'review');
    now.mockReturnValue(6_000);
    const ps = [1, 2, 3].map(of);
    now.mockRestore();
    const base = snapshot('p/a', { todo: [1, 2, 3], doing: [4], review: [5] }, [{ id: 5, column: 'review', movedAt: 8_000 }]);
    const b = twice(D.joinPatch(...ps).board!, base);
    expect(b.cols.review).toEqual([5, 1, 2, 3]);
    expect([1, 2, 3].map(id => b.cards[id].movedAt)).toEqual([8_001, 8_001, 8_001]);
  });

  it('拖进只带了最近几张的终点状态：最旧的那张当帧落进「更早的」；没有省略时不动', () => {
    const done = Array.from({ length: 50 }, (_, i) => 100 + i);
    const cards = done.map(id => ({ id, movedAt: id }));
    const part = { ...snapshot('p/a', { todo: [1], done }, cards), omitted: { done: 7 } };
    const b = twice(D.movePatch(1, 'done').board!, part);
    expect(b.cols.done).toHaveLength(50);
    expect(b.cols.done).toContain(1);
    expect(b.cols.done).not.toContain(100);
    expect(b.cards[100]).toBeUndefined();
    expect(b.omitted).toEqual({ done: 8 });
    eachOnce(b);
    // 取过全量：快照没有省略，界面自己截断
    const full = snapshot('p/a', { todo: [1], done }, cards);
    expect(D.movePatch(1, 'done').board!(full).cols.done).toHaveLength(51);
  });
});

/** 派发：只填用到的字段 */
const disp = (over: Partial<Dispatch>): Dispatch =>
  ({ id: 1, board: 'p/a', card: 4, title: '', hostId: 1, host: 'mbp', agent: 'claude', agentLabel: 'Claude Code', name: null, source: 'manual',
     by: { kind: 'human', name: '' }, state: 'queued', result: null, createdAt: 0, startedAt: null, endedAt: null, stopBy: null, beatAt: null,
     ...over }) as Dispatch;

describe('派发排队的卡片（在「进行中」里等，没有持有者）', () => {
  // 4 号卡在 doing 里排队（从 todo 来），5 号卡被持有
  const queuedBoard = () => snapshot('p/a', { todo: [1, 2], doing: [5, 4], review: [] }, [{ id: 4, claimedFrom: 'todo' }, { id: 5, holder: 'claude/x' }]);
  const ws = () => workspace({ dispatches: [disp({ id: 7, card: 4 }), disp({ id: 8, card: 4, board: 'p/b' }), disp({ id: 9, card: 5, state: 'running' })] });
  const start = () => S.useData.setState({ path: 'p/a', board: queuedBoard(), ws: ws() });

  it('unqueuePatch：只去掉这块白板这张卡片排队的那条；重复执行结果相同；没有时原样返回', () => {
    const w = twice(D.unqueuePatch('p/a', 4).ws!, ws());
    expect(w.dispatches!.map(d => d.id)).toEqual([8, 9]);
    const w2 = ws();
    expect(D.unqueuePatch('p/a', 5).ws!(w2)).toBe(w2);
  });

  it('movePatch：拖出进行中时排队的派发一起去掉、claimedFrom 清空；在进行中里调顺序不动派发', () => {
    start();
    const p = D.movePatch(4, 'todo', 1);
    const b = twice(p.board!, queuedBoard());
    expect(b.cols).toEqual({ todo: [1, 4, 2], doing: [5], review: [] });
    expect(b.cards[4]).toMatchObject({ column: 'todo', claimedFrom: null });
    expect(twice(p.ws!, ws()).dispatches!.map(d => d.id)).toEqual([8, 9]);
    expect(D.movePatch(4, 'doing', 0).ws).toBeUndefined();
    // 被持有的卡片、没有排队派发的卡片：不碰派发
    expect(D.movePatch(1, 'review').ws).toBeUndefined();
  });

  it('unqueueCard：当前白板上排队的卡片回到原来的可领取状态最上面；别的白板只去掉派发', () => {
    start();
    const p = D.unqueueCard('p/a', 4);
    const b = twice(p.board!, queuedBoard());
    expect(b.cols).toEqual({ todo: [4, 1, 2], doing: [5], review: [] });
    expect(b.cards[4]).toMatchObject({ column: 'todo', claimedFrom: null });
    expect(twice(p.ws!, ws()).dispatches!.map(d => d.id)).toEqual([8, 9]);
    expect(p.cards).toEqual([4]);
    const other = D.unqueueCard('p/b', 4);
    expect(other.board).toBeUndefined();
    expect(other.ws!(ws()).dispatches!.map(d => d.id)).toEqual([7, 9]);
  });

  it('unqueueCard：原来的状态不再可领取时回首个可领取状态；卡片已经回去了原样返回', () => {
    S.useData.setState({ path: 'p/a', board: snapshot('p/a', { todo: [1], doing: [4] }, [{ id: 4, claimedFrom: 'review' }]), ws: ws() });
    expect(D.unqueueCard('p/a', 4).board!(queuedBoard()).cols.todo).toEqual([4, 1, 2]);
    const back = snapshot('p/a', { todo: [4, 1, 2], doing: [5], review: [] });
    S.useData.setState({ board: back });
    expect(D.unqueueCard('p/a', 4).board).toBeUndefined();
  });

  it('joinPatch：按顺序叠，保存中的卡片和对象合在一起', () => {
    const p = D.joinPatch({ ws: w => ({ ...w, rev: w.rev + 1 }), keys: ['a'] }, { ws: w => ({ ...w, rev: w.rev * 10 }), cards: [3], keys: ['b'] });
    expect(p.ws!(workspace()).rev).toBe(20);
    expect([p.cards, p.keys, p.acts, p.board]).toEqual([[3], ['a', 'b'], undefined, undefined]);
  });
});

describe('removeCard', () => {
  it('从卡片和所有状态里拿掉；重复执行结果不变', () => {
    const b = twice(D.removeCard(2).board!, board());
    expect(b.cols.todo).toEqual([1, 3]);
    expect(b.cards[2]).toBeUndefined();
    eachOnce(b);
  });

  it('卡片不在了：原样返回', () => {
    const b = board();
    expect(D.removeCard(9).board!(b)).toBe(b);
  });

  it('回收站里多一张；重复执行只加一次（卡片已经不在时原样返回，不再计数）', () => {
    const b = twice(D.removeCard(2).board!, board());
    expect(b.trash).toBe(1);
    const again = D.removeCard(2).board!(b);
    expect(again).toBe(b);
    expect(again.trash).toBe(1);
  });
});

describe('restoreCard', () => {
  /** 删掉 #2 之后的快照：回收站里有一张 */
  const after = () => ({ ...snapshot('p/a', { todo: [1, 3], doing: [4] }), trash: 1 });
  const plan = { id: 2, column: 'todo', pos: 1, movedAt: 0, card: card(2) };

  it('放回原来那一列的第 pos 位，回收站里少一张；重复执行结果不变', () => {
    const b = twice(D.restoreCard(plan).board!, after());
    expect(b.cols.todo).toEqual([1, 2, 3]);
    expect(b.cards[2]).toMatchObject({ id: 2, column: 'todo' });
    expect(b.trash).toBe(0);
    eachOnce(b);
  });

  it('原位置超出了现在的张数：放最后；pos 为负按最前面', () => {
    expect(twice(D.restoreCard({ ...plan, pos: 9 }).board!, after()).cols.todo).toEqual([1, 3, 2]);
    expect(twice(D.restoreCard({ ...plan, pos: -3 }).board!, after()).cols.todo).toEqual([2, 1, 3]);
  });

  it('原来那一列不在了：新建，卡片放在那一列', () => {
    const b = twice(D.restoreCard({ ...plan, column: 'rework' }).board!, after());
    expect(b.cols.rework).toEqual([2]);
    expect(b.cards[2].column).toBe('rework');
    eachOnce(b);
  });

  it('已经在白板上：原样返回（重复执行不变）', () => {
    const b = board();
    expect(D.restoreCard(plan).board!(b)).toBe(b);
  });

  it('打开着的这张卡片的详情跟着回到那一列；别的卡片不动', () => {
    const p = D.restoreCard({ ...plan, column: 'doing' });
    expect(twice(p.detail!, detail(card(2)))).toMatchObject({ column: 'doing' });
    const d4 = detail(card(4));
    expect(p.detail!(d4)).toBe(d4);
  });

  it('恢复拆出的卡：原条目在白板上、没记拆解过时当帧记上；已经记着、旧服务端（undefined）、不在白板上都不动（default/main#735）', () => {
    const inbox = (splitAt: number | null | undefined) => ({ ...snapshot('p/a', { inbox: [1] }, [{ id: 1, splitAt }]), trash: 1 });
    const kid = { id: 2, column: 'inbox', pos: 1, movedAt: 50, card: card(2, { splitFrom: 1 }) };
    expect(twice(D.restoreCard(kid).board!, inbox(null)).cards[1].splitAt).toBe(50);
    expect(twice(D.restoreCard(kid).board!, inbox(20)).cards[1].splitAt).toBe(20);
    expect(twice(D.restoreCard(kid).board!, inbox(undefined)).cards[1].splitAt).toBeUndefined();
    const gone = { ...snapshot('p/a', { inbox: [] }), trash: 1 };
    expect(twice(D.restoreCard(kid).board!, gone).cards[1]).toBeUndefined();
  });
});

describe('unsplitPatch（撤销拆解，default/main#735）', () => {
  const split = () =>
    snapshot('p/a', { inbox: [1, 2, 3, 5], todo: [4] }, [
      { id: 1, splitAt: 10 },
      { id: 2, splitFrom: 1 },
      { id: 3, splitFrom: 1 },
    ]);

  it('拆出的卡从白板上拿掉（回收站多几张），原条目清掉拆解过；重复执行结果不变', () => {
    const b = twice(D.unsplitPatch(1, [2, 3]).board!, split());
    expect(b.cols.inbox).toEqual([1, 5]);
    expect(b.cards[2]).toBeUndefined();
    expect(b.cards[1].splitAt).toBeNull();
    expect(b.trash).toBe(2);
    eachOnce(b);
  });

  it('原条目不在了：原样返回（拆出的卡也不动）', () => {
    const b = split();
    expect(D.unsplitPatch(9, [2, 3]).board!(b)).toBe(b);
  });

  it('有的拆出卡已经不在：只拿掉还在的；详情只改原条目；保存中画在原条目和拆出的卡上', () => {
    const p = D.unsplitPatch(1, [2, 3, 8]);
    const b = twice(p.board!, D.removeCard(3).board!(split()));
    expect(b.cols.inbox).toEqual([1, 5]);
    expect(b.trash).toBe(2);
    expect(p.cards).toEqual([1, 2, 3, 8]);
    expect(twice(p.detail!, detail(card(1, { splitAt: 10 })))).toMatchObject({ splitAt: null });
    const other = detail(card(5));
    expect(p.detail!(other)).toBe(other);
  });
});

describe('mergePatch', () => {
  const inbox = () => snapshot('p/a', { inbox: [5, 6, 7], todo: [1] }, [{ id: 5, labels: [1] }, { id: 6, labels: [2, 1] }, { id: 7, labels: [3] }]);
  const change = { title: '合并后', body: '合并后的描述' };

  it('留下 keep，换标题描述、标签取并集；来源从卡片和状态里拿掉；重复执行结果不变', () => {
    const b = twice(D.mergePatch(5, [6, 7], change).board!, inbox());
    expect(b.cols).toEqual({ inbox: [5], todo: [1] });
    expect(b.cards[5]).toMatchObject({ title: '合并后', body: '合并后的描述', labels: [1, 2, 3] });
    expect(b.cards[6]).toBeUndefined();
    eachOnce(b);
  });

  it('留下的那条已经不在：原样返回，来源也不动', () => {
    const b = inbox();
    expect(D.mergePatch(9, [6], change).board!(b)).toBe(b);
  });

  it('有的来源已经不在：只拿掉还在的；详情只改留下的那张；保存中画在所有涉及的卡片上', () => {
    const p = D.mergePatch(5, [6, 9, 5], change);
    const b = twice(p.board!, inbox());
    expect(b.cols.inbox).toEqual([5, 7]);
    expect(p.cards).toEqual([5, 6, 9]);
    const d = detail(card(5));
    expect(twice(p.detail!, d)).toMatchObject(change);
    const other = detail(card(7));
    expect(p.detail!(other)).toBe(other);
  });
});

describe('removeComment', () => {
  const act = (id: number) => ({ id, at: 0, actor: { kind: 'human' as const, name: '' }, kind: 'comment' as const, action: 'comment', text: `评论 ${id}` });

  it('从详情动态里拿掉这条、评论数减一；重复执行结果不变', () => {
    const d0 = detail(card(1, { comments: 2 }));
    d0.activity = [act(10), act(11)];
    const p = D.removeComment(1, 10);
    const once = twice(p.detail!, d0);
    expect(once.activity.map(a => a.id)).toEqual([11]);
    expect(once.comments).toBe(1);
  });

  it('评论已经不在了、详情是别的卡片：原样返回', () => {
    const d = detail(card(1, { comments: 1 }));
    expect(D.removeComment(1, 10).detail!(d)).toBe(d);
    const other = detail(card(2));
    expect(D.removeComment(1, 10).detail!(other)).toBe(other);
  });
});

describe('editCard', () => {
  it('白板摘要和打开着的详情一起改；默认画成保存中', () => {
    const p = D.editCard(1, { title: '新标题' });
    expect(twice(p.board!, board()).cards[1].title).toBe('新标题');
    expect(twice(p.detail!, detail(card(1))).title).toBe('新标题');
    expect(p.cards).toEqual([1]);
  });

  it('change 是函数时按当前卡片算；重复执行结果不变', () => {
    const p = D.editCard(1, c => (c.labels.includes(2) ? {} : { labels: [...c.labels, 2] }), false);
    const b = twice(p.board!, snapshot('p/a', { todo: [1] }, [{ id: 1, labels: [1] }]));
    expect(b.cards[1].labels).toEqual([1, 2]);
    expect(p.cards).toBeUndefined();
  });

  it('卡片不在了、详情是别的卡片：原样返回', () => {
    const b = board();
    const d = detail(card(2));
    const p = D.editCard(9, { title: 'x' });
    expect(p.board!(b)).toBe(b);
    expect(p.detail!(d)).toBe(d);
  });
});

describe('稍后再做（default/main#303）', () => {
  const inboxBoard = () => {
    const b = snapshot('p/a', { inbox: [1, 2], todo: [3] });
    return b;
  };
  it('laterPatch：收起、放回只改「未整理」里的；重复执行结果相同；目标不在了原样返回', () => {
    const on = D.laterPatch([1, 3, 99], true);
    const b = twice(on.board!, inboxBoard());
    expect(b.cards[1].laterAt).toEqual(expect.any(Number));
    expect(b.cards[2].laterAt ?? null).toBeNull();
    expect(b.cards[3].laterAt ?? null).toBeNull();
    const off = twice(D.laterPatch([1], false).board!, b);
    expect(off.cards[1].laterAt).toBeNull();
    const orig = inboxBoard();
    expect(D.laterPatch([99], true).board!(orig)).toBe(orig);
    expect(D.laterPatch([2], false).board!(orig)).toBe(orig);
  });
  it('laterPatch：详情跟着改', () => {
    const d = detail(card(1, { column: 'inbox' }));
    expect(D.laterPatch([1], true).detail!(d).laterAt).toEqual(expect.any(Number));
    expect(D.laterPatch([2], true).detail!(d)).toBe(d);
  });
});

describe('项目与标签', () => {
  const labelsOf = (ws: Workspace, key: string) => ws.projects.find(p => p.key === key)!.labels;

  it('patchProject：只改这个项目；项目不在了内容不变', () => {
    const p = D.patchProject('p', x => ({ ...x, name: '新名字' }));
    const ws = twice(p.ws!, workspace());
    expect(ws.projects.map(x => x.name)).toEqual(['新名字', 'Q']);
    const orig = workspace();
    expect(D.patchProject('nope', x => ({ ...x, name: 'x' })).ws!(orig)).toEqual(orig);
  });

  it('hiddenLabelsPatch：整组替换规则；带 hiddenLabelsKey；项目不在了内容不变', () => {
    const p = D.hiddenLabelsPatch('p', ['^v\\d', '^tmp-']);
    const ws = twice(p.ws!, workspace());
    expect(ws.projects.find(x => x.key === 'p')!.hiddenLabels).toEqual(['^v\\d', '^tmp-']);
    expect(ws.projects.find(x => x.key === 'q')!.hiddenLabels).toBeUndefined();
    expect(p.keys).toEqual([D.hiddenLabelsKey('p')]);
    const orig = workspace();
    expect(D.hiddenLabelsPatch('nope', []).ws!(orig)).toEqual(orig);
  });

  it('editLabelPatch：改名、改颜色；带 labelKey；标签不在了内容不变', () => {
    const p = D.editLabelPatch('p', 2, { name: '界面', color: '#00f' });
    const ws = twice(p.ws!, workspace());
    expect(labelsOf(ws, 'p')[1]).toMatchObject({ id: 2, name: '界面', color: '#00f' });
    expect(labelsOf(ws, 'q')[0].name).toBe('bug');
    expect(p.keys).toEqual([D.labelKey('p', 2)]);
    const orig = workspace();
    expect(D.editLabelPatch('p', 9, { name: 'x' }).ws!(orig)).toEqual(orig);
  });

  it('deleteLabelPatch：从项目和这个项目白板的卡片上拿掉；别的项目的同号标签不动', () => {
    const p = D.deleteLabelPatch('p', 1);
    const ws = twice(p.ws!, workspace());
    expect(labelsOf(ws, 'p').map(l => l.id)).toEqual([2]);
    expect(labelsOf(ws, 'q').map(l => l.id)).toEqual([1]);
    expect(p.keys).toEqual([D.labelKey('p', 1)]);

    const b = twice(p.board!, snapshot('p/a', { todo: [1, 2] }, [{ id: 1, labels: [1, 2] }, { id: 2, labels: [2] }]));
    expect(b.cards[1].labels).toEqual([2]);
    expect(b.cards[2].labels).toEqual([2]);

    const other = snapshot('q/a', { todo: [1] }, [{ id: 1, labels: [1] }]);
    expect(p.board!(other)).toBe(other);
    const none = snapshot('p/a', { todo: [1] });
    expect(p.board!(none)).toBe(none);
  });
});

describe('cardCronPatch（白板的定时建卡）', () => {
  const cron = { id: 7, name: '清理死代码', title: '清理死代码', body: '', labels: [], place: 'home' as const, cron: '0 9 * * 1', tz: 'Asia/Shanghai', on: true, nextAt: 123, createdAt: 0, updatedAt: 0 };
  const withCron = () => {
    const ws = workspace();
    return { ...ws, projects: ws.projects.map((p, i) => (i ? p : { ...p, boards: p.boards.map((b, j) => (j ? b : { ...b, cardCrons: [cron] })) })) };
  };
  it('只改这一条，重复执行结果相同；改名称时 title 跟着改；改时间、关掉时先清掉下次时间', () => {
    const ws = twice(D.cardCronPatch('p/a', 7, { name: '清理废代码' }).ws!, withCron());
    expect(ws.projects[0].boards[0].cardCrons![0]).toMatchObject({ name: '清理废代码', title: '清理废代码', nextAt: 123, on: true });
    const again = twice(D.cardCronPatch('p/a', 7, { name: '清理废代码' }).ws!, ws);
    expect(again.projects[0].boards[0].cardCrons![0]).toMatchObject({ name: '清理废代码', title: '清理废代码' });
    const off = twice(D.cardCronPatch('p/a', 7, { on: false }).ws!, withCron());
    expect(off.projects[0].boards[0].cardCrons![0]).toMatchObject({ on: false, nextAt: null });
    const moved = twice(D.cardCronPatch('p/a', 7, { cron: '0 10 * * 1' }).ws!, withCron());
    expect(moved.projects[0].boards[0].cardCrons![0].nextAt).toBeNull();
  });
  it('派给机器和角色：改机器、角色不动下次时间；空字符串画成不派发、角色一起清掉', () => {
    const to = twice(D.cardCronPatch('p/a', 7, { host: 'mbp', profile: 'p1' }).ws!, withCron());
    expect(to.projects[0].boards[0].cardCrons![0]).toMatchObject({ host: 'mbp', profile: 'p1', agent: null, nextAt: 123 });
    const off = twice(D.cardCronPatch('p/a', 7, { host: '', profile: 'p1' }).ws!, to);
    expect(off.projects[0].boards[0].cardCrons![0]).toMatchObject({ host: null, profile: null, nextAt: 123 });
    const rule = twice(D.cardCronPatch('p/a', 7, { host: 'mbp', profile: '' }).ws!, to);
    expect(rule.projects[0].boards[0].cardCrons![0]).toMatchObject({ host: 'mbp', profile: null }); // 空字符串是按规则
  });
  it('随机派发：换成随机时清掉指定的机器、角色留着（不动下次时间）；换回指定机器时照样画出来', () => {
    const to = twice(D.cardCronPatch('p/a', 7, { host: 'mbp', profile: 'p1' }).ws!, withCron());
    const any = twice(D.cardCronPatch('p/a', 7, { pick: 'random' }).ws!, to);
    expect(any.projects[0].boards[0].cardCrons![0]).toMatchObject({ pick: 'random', host: null, profile: 'p1', nextAt: 123 });
    const back = twice(D.cardCronPatch('p/a', 7, { pick: 'host', host: 'mini', profile: '' }).ws!, any);
    expect(back.projects[0].boards[0].cardCrons![0]).toMatchObject({ pick: 'host', host: 'mini', profile: null, nextAt: 123 });
  });
  it('白板或这一条不在了原样返回', () => {
    const orig = withCron();
    expect(D.cardCronPatch('p/nope', 7, { on: false }).ws!(orig)).toBe(orig);
    expect(D.cardCronPatch('p/a', 8, { on: false }).ws!(orig)).toBe(orig);
    const plain = workspace();
    expect(D.cardCronPatch('p/a', 7, { on: false }).ws!(plain)).toBe(plain);
  });
});

describe('repoPatch（项目的仓库地址、集成分支）', () => {
  it('改的是项目：项目、它下面每块白板、当前白板的快照一起改；重复执行结果相同；项目不在了原样返回', () => {
    const p = D.repoPatch('p', { repo: 'git@github.com:o/r.git', branch: 'dev' });
    const ws = twice(p.ws!, workspace());
    expect(ws.projects[0]).toMatchObject({ repo: 'git@github.com:o/r.git', branch: 'dev' });
    expect(ws.projects[0].boards.every(b => b.repo === 'git@github.com:o/r.git' && b.branch === 'dev')).toBe(true);
    const b = twice(p.board!, snapshot('p/a', { todo: [1] }));
    expect(b.repo).toBe('git@github.com:o/r.git');
    expect(b.branch).toBe('dev');
    const orig = workspace();
    expect(D.repoPatch('nope', { repo: 'x' }).ws!(orig)).toBe(orig);
    // 别的项目的白板快照不动
    const other = snapshot('q/a', { todo: [1] });
    expect(p.board!(other)).toBe(other);
  });

  it('只改一项时另一项保持原值（没设过时补成空字符串）', () => {
    const ws = twice(D.repoPatch('p', { branch: 'dev' }).ws!, workspace());
    expect(ws.projects[0]).toMatchObject({ repo: '', branch: 'dev' });
    expect(ws.projects[0].boards[0]).toMatchObject({ repo: '', branch: 'dev' });
  });
});

describe('boardAutoPatch（白板参不参与自动领取）', () => {
  it('只改这块白板；重复执行结果相同；白板不在了原样返回', () => {
    const ws = twice(D.boardAutoPatch('p/a', false).ws!, workspace());
    const boards = ws.projects.flatMap(p => p.boards);
    expect(boards.find(b => b.path === 'p/a')?.auto).toBe(false);
    expect(boards.filter(b => b.path !== 'p/a').every(b => b.auto === undefined)).toBe(true);
    expect(twice(D.boardAutoPatch('p/a', true).ws!, ws).projects.flatMap(p => p.boards).find(b => b.path === 'p/a')?.auto).toBe(true);
    const orig = workspace();
    expect(D.boardAutoPatch('p/nope', false).ws!(orig)).toBe(orig);
  });
});

describe('draftCard（新建）', () => {
  const draft = (title = '新卡片') => D.draftCard({ title, body: '', labels: [], column: 'todo' });
  const start = () => S.useData.setState({ board: board() });

  it('临时卡片画在目标状态最下面，负数 ID，保存中；重复执行结果相同', () => {
    start();
    const d = draft();
    expect(d.id).toBeLessThan(0);
    // render 每次都把补丁叠到新的 base 上（不会叠到自己的结果上）：对同一份 base 执行两次结果相同
    const b = d.patch.board!(board());
    expect(d.patch.board!(board())).toEqual(b);
    expect(b.cols.todo).toEqual([1, 2, 3, d.id]);
    expect(b.cards[d.id].title).toBe('新卡片');
    expect(d.patch.cards).toEqual([d.id]);
    expect(D.isDraft(d.id)).toBe(true);
  });

  it('真卡片出现在快照里之后临时卡片不再画（新建时出现两张卡片的回归用例）', () => {
    start();
    const d = draft();
    const real = snapshot('p/a', { todo: [1, 2, 3, 5] }, [{ id: 5, title: '新卡片' }]);
    const b = twice(d.patch.board!, real);
    expect(b).toBe(real);
    expect(Object.keys(b.cards)).not.toContain(String(d.id));
  });

  it('不认：新建前就有的、agent 建的、状态不同的同名卡片', () => {
    start();
    const d = draft('卡片 1'); // 白板上已经有「卡片 1」（ID 1）
    const s = snapshot('p/a', { todo: [1, 2, 3, 5], doing: [6] }, [
      { id: 5, title: '卡片 1', createdBy: { kind: 'agent', name: 'claude/x' } },
      { id: 6, title: '卡片 1' },
    ]);
    expect(d.patch.board!(s).cols.todo).toContain(d.id);
  });

  it('两张同名临时卡各认各的真卡片', () => {
    start();
    const a = draft();
    const b = draft();
    const one = snapshot('p/a', { todo: [1, 2, 3, 5] }, [{ id: 5, title: '新卡片' }]);
    // 只来了一张：a 认走它，b 还画着
    expect(a.patch.board!(one)).toBe(one);
    expect(b.patch.board!(one).cols.todo).toEqual([1, 2, 3, 5, b.id]);
    // 两张都来了：都不再画
    const two = snapshot('p/a', { todo: [1, 2, 3, 5, 6] }, [{ id: 5, title: '新卡片' }, { id: 6, title: '新卡片' }]);
    expect(b.patch.board!(a.patch.board!(two))).toBe(two);
  });

  it('settle：按服务端返回的 ID 认，之前猜错的让出来', () => {
    start();
    const a = draft();
    const b = draft();
    const two = snapshot('p/a', { todo: [1, 2, 3, 5, 6] }, [{ id: 5, title: '新卡片' }, { id: 6, title: '新卡片' }]);
    a.patch.board!(two); // a 猜成 5
    a.settle(6); // 服务端说 a 是 6：5 让给 b
    expect(b.patch.board!(two)).toBe(two);
    const only6 = snapshot('p/a', { todo: [1, 2, 3, 6] }, [{ id: 6, title: '新卡片' }]);
    expect(a.patch.board!(only6)).toBe(only6);
  });

  it('renderKey：顶替临时卡片的真卡片沿用临时卡片的身份（同一个元素，不重播入场动画）；猜错的让出来', () => {
    start();
    const a = draft();
    const b = draft();
    expect(D.renderKey(5)).toBe(5);
    expect(D.renderKey(a.id)).toBe(a.id);
    const two = snapshot('p/a', { todo: [1, 2, 3, 5, 6] }, [{ id: 5, title: '新卡片' }, { id: 6, title: '新卡片' }]);
    a.patch.board!(two); // a 猜成 5
    expect(D.renderKey(5)).toBe(a.id);
    a.settle(6);
    expect(D.renderKey(6)).toBe(a.id);
    expect(D.renderKey(5)).toBe(5);
    b.patch.board!(two); // 5 让给 b
    expect(D.renderKey(5)).toBe(b.id);
    expect(D.renderKey(1)).toBe(1);
  });
});

describe('seenPatch / 新条目', () => {
  const agent = { kind: 'agent' as const, name: 'claude/x' };
  /** p/a 的「未整理」：1 是 agent 记下、没看过的；2 是 agent 记下、人看过的；3 是人自己记的；4 在工作流里 */
  const inbox = () =>
    snapshot('p/a', { inbox: [1, 2, 3], todo: [4] }, [
      { id: 1, createdBy: agent, seenAt: null },
      { id: 2, createdBy: agent, seenAt: 1_000 },
      { id: 3, createdBy: { kind: 'human', name: '' }, seenAt: 500 },
      { id: 4, createdBy: agent, seenAt: null },
    ]);

  it('seenPatch：记下看过的时刻，只改「未整理」里没看过的；重复执行结果不变', () => {
    const now = vi.spyOn(Date, 'now').mockReturnValue(5_000);
    const p = D.seenPatch([1, 2, 4]);
    now.mockRestore();
    const b = twice(p.board!, inbox());
    expect(b.cards[1].seenAt).toBe(5_000);
    expect(b.cards[2].seenAt).toBe(1_000); // 早就看过的时间不刷新
    expect(b.cards[4].seenAt).toBeNull(); // 不在「未整理」里
    expect(b.cards[3].seenAt).toBe(500);
    expect(p.cards).toBeUndefined(); // 不画成「保存中」：点本来就是一闪没
  });

  it('seenPatch：详情里的这一张一起改；别的卡片、目标不在了原样返回', () => {
    const p = D.seenPatch([1]);
    const d = detail(card(1, { column: 'inbox', createdBy: agent, seenAt: null }));
    expect(twice(p.detail!, d).seenAt).toBeTruthy();
    const other = detail(card(9));
    expect(p.detail!(other)).toBe(other);
    const gone = snapshot('p/a', { todo: [9] });
    expect(p.board!(gone)).toBe(gone);
  });

});

describe('labelAddPatch（批量加标签，default/main#531）', () => {
  it('这几张都画上标签，已经有的不重复加；重复执行结果不变；不画「保存中」', () => {
    const b0 = board();
    b0.cards[2] = { ...b0.cards[2], labels: [7] };
    const p = D.labelAddPatch([1, 2, 4], 7);
    const b = twice(p.board!, b0);
    expect([1, 2, 3, 4].map(id => b.cards[id].labels)).toEqual([[7], [7], [], [7]]);
    expect(b.cards[2]).toBe(b0.cards[2]);
    expect(p.cards).toBeUndefined();
  });

  it('卡片不在了：原样返回；详情只改选中的那张', () => {
    const b0 = board();
    expect(D.labelAddPatch([99], 7).board!(b0)).toEqual(b0);
    const p = D.labelAddPatch([1], 7);
    expect(twice(p.detail!, detail(card(1))).labels).toEqual([7]);
    const other = detail(card(5));
    expect(p.detail!(other)).toBe(other);
  });
});

describe('快捷提示词的补丁（#572）', () => {
  const list = () => [
    { id: 'p-2', title: '重启', text: '重启一下' },
    { id: 'p-1', title: '等发版', text: '等发版' },
  ];

  it('新建：放在最前面；已经有这个 id 时原样返回（重复执行结果相同）', () => {
    const p = { id: 'p-3', title: '新', text: '新的' };
    const out = twice(D.addPromptPatch(p).prompts!, list());
    expect(out.map(x => x.id)).toEqual(['p-3', 'p-2', 'p-1']);
    expect(D.addPromptPatch(p).keys).toEqual([D.PROMPTS_BUSY]);
  });

  it('修改只动那一条；不在了原样返回', () => {
    const out = twice(D.editPromptPatch('p-1', ' 等发版 ✅ ', '等发版\r\n通知我').prompts!, list());
    expect(out[1]).toEqual({ id: 'p-1', title: '等发版 ✅', text: '等发版\n通知我' });
    expect(out[0]).toEqual(list()[0]);
    const before = list();
    expect(D.editPromptPatch('p-9', '标题', '内容').prompts!(before)).toBe(before);
  });

  it('删除；不在了原样返回', () => {
    expect(twice(D.removePromptPatch('p-2').prompts!, list())).toEqual([list()[1]]);
    const before = list();
    expect(D.removePromptPatch('p-9').prompts!(before)).toBe(before);
  });

  it('整列换掉（搬本机旧存的）：一样时原样返回', () => {
    const next = [{ id: 'p-5', title: '本机的', text: '本机的' }];
    expect(twice(D.replacePromptsPatch(next).prompts!, list())).toEqual(next);
    const same = list();
    expect(D.replacePromptsPatch(list()).prompts!(same)).toBe(same);
  });

});

describe('角色的补丁（分工）', () => {
  const role = (id: string, name: string) => ({ id, name, agent: 'claude', provider: '', model: '', effort: '', note: '', only: [] });
  const withRoles = (): Workspace => ({
    ...workspace(),
    sched: { minute: 0, rules: [{ op: 'in', labels: ['bug'], profile: 'p1' }, { op: 'none', labels: [], profile: 'p2' }], profiles: [role('p1', '修复'), role('p2', '批量')] },
  });

  it('改一个角色：按标识换掉，别的不动；带上保存中的 key', () => {
    const p = D.editProfilePatch({ ...role('p1', '修 bug'), model: 'model-a' });
    const w = twice(p.ws!, withRoles());
    expect(w.sched!.profiles!.map(x => [x.name, x.model])).toEqual([['修 bug', 'model-a'], ['批量', '']]);
    expect(p.keys).toEqual([D.profileKey('p1')]);
  });

  it('删掉一个角色：交给它的规则一起拿掉', () => {
    const w = twice(D.deleteProfilePatch('p1').ws!, withRoles());
    expect(w.sched!.profiles!.map(x => x.id)).toEqual(['p2']);
    expect(w.sched!.rules.map(r => r.profile)).toEqual(['p2']);
  });

  it('角色不在了（别处删掉了）、旧服务端没有角色：原样返回', () => {
    const w = withRoles();
    expect(D.editProfilePatch(role('p9', 'x')).ws!(w)).toBe(w);
    expect(D.deleteProfilePatch('p9').ws!(w)).toBe(w);
    const bare = workspace();
    expect(D.deleteProfilePatch('p1').ws!(bare)).toBe(bare);
  });
});

describe('chatPromptPatch', () => {
  const chat = (id: number, x: Partial<NonNullable<Workspace['chats']>[number]> = {}) =>
    ({ id, hostId: 1, host: 'mbp', agent: 'claude', agentLabel: 'Claude Code', title: '看看主目录', by: 'lan', createdAt: 1, lastAt: 10, turn: 'idle', ...x }) as NonNullable<Workspace['chats']>[number];
  it('发一条：这个对话当帧变成回答中，最近动静挪到现在；别的对话不动；重复执行结果相同', () => {
    const ws = workspace({ chats: [chat(1), chat(2)] });
    const out = twice(D.chatPromptPatch(1, 50).ws!, ws);
    expect(out.chats!.map(c => [c.id, c.turn, c.lastAt])).toEqual([
      [1, 'busy', 50],
      [2, 'idle', 10],
    ]);
  });
  it('对话不在了：原样返回', () => {
    const ws = workspace({ chats: [chat(2)] });
    expect(D.chatPromptPatch(1, 50).ws!(ws)).toBe(ws);
    const none = workspace();
    expect(D.chatPromptPatch(1, 50).ws!(none)).toBe(none);
  });
});
