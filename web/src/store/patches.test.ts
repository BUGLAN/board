/* 乐观更新的补丁（store/data.ts）：纯函数，每次刷新都会重新叠到新数据上，
   所以要能重复执行（结果不变），目标不在了原样返回 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardSnapshot, Workspace } from '../lib/types';
import { card, detail, snapshot, workspace } from '../../test/fixtures';
import type * as Data from './data';

let D: typeof Data;
beforeEach(async () => {
  // draftCard 有模块级状态（临时 ID、已认领的真卡片）：每个用例一份新的
  vi.resetModules();
  D = await import('./data');
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

describe('项目与标签', () => {
  const labelsOf = (ws: Workspace, key: string) => ws.projects.find(p => p.key === key)!.labels;

  it('patchProject：只改这个项目；项目不在了内容不变', () => {
    const p = D.patchProject('p', x => ({ ...x, name: '新名字' }));
    const ws = twice(p.ws!, workspace());
    expect(ws.projects.map(x => x.name)).toEqual(['新名字', 'Q']);
    const orig = workspace();
    expect(D.patchProject('nope', x => ({ ...x, name: 'x' })).ws!(orig)).toEqual(orig);
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

describe('draftCard（新建）', () => {
  const draft = (title = '新卡片') => D.draftCard({ title, body: '', labels: [], column: 'todo' });
  const start = () => D.useData.setState({ board: board() });

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

describe('选择器', () => {
  it('boardMeta：按路径找项目和白板', () => {
    const ws = workspace();
    expect(D.boardMeta(ws, 'p/b')?.board.key).toBe('b');
    expect(D.boardMeta(ws, 'p/x')).toBeNull();
    expect(D.boardMeta(null, 'p/a')).toBeNull();
    expect(D.allBoards(ws).map(b => b.path)).toEqual(['p/a', 'p/b']);
  });
});

