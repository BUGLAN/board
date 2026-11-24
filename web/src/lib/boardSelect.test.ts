/* 白板勾选卡片、批量打标签（PRD §10.3「批量打标签」）：连选、整块勾选、批量写入 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Dispatch } from './types';

type LabelBatch = { changed: number[]; skipped: number[]; failed: { id: number; error: unknown }[] };
const labelCards = vi.fn<(ids: number[], label: number, add: boolean) => Promise<LabelBatch>>();
const batchCards = vi.fn<(ids: number[], op: { op: 'move'; column: string }, patch: (id: number) => unknown) => Promise<{ ok: number[]; failed: { id: number; error: unknown }[] }>>();
const toastError = vi.fn();
const toast = vi.fn();
vi.mock('../store/data', () => ({
  labelCards: (ids: number[], l: number, add: boolean) => labelCards(ids, l, add),
  batchCards: (ids: number[], op: { op: 'move'; column: string }, patch: (id: number) => unknown) => batchCards(ids, op, patch),
  movePatch: (id: number, to: string) => ({ move: [id, to] }),
  useData: { getState: () => ({ detailId: null }) },
}));
vi.mock('../store/ui', () => ({ overlayOpen: () => false, toastError: (e: unknown) => toastError(e), toast: (t: unknown) => toast(t) }));

const { FLOW, snapshot } = await import('../../test/fixtures');
const { blockState, bulkDispatchToast, bulkLabel, bulkMove, clearSelect, dispatchPlan, moveTargets, rangeIds, runBulkDispatch, selectIds, selectRange, startSelect, toggleBlock, toggleCard, useBoardSel } =
  await import('./boardSelect');

beforeEach(() => {
  clearSelect();
  labelCards.mockReset();
  batchCards.mockReset();
  toastError.mockReset();
  toast.mockReset();
});

describe('rangeIds（Shift 连选）', () => {
  const order = [5, 6, 7, 2, 3];
  it('从起点到这张，按看到的顺序，前后都行', () => {
    expect(rangeIds(order, 6, 2)).toEqual([6, 7, 2]);
    expect(rangeIds(order, 3, 7)).toEqual([7, 2, 3]);
  });
  it('点的就是起点：只有它', () => {
    expect(rangeIds(order, 7, 7)).toEqual([7]);
  });
  it('没有起点、起点在别的块：返回 null（只切换这一张）', () => {
    expect(rangeIds(order, null, 2)).toBeNull();
    expect(rangeIds(order, 99, 2)).toBeNull();
  });
});

describe('选择', () => {
  it('点一张进入选择并选上，再点取消；取消后还在选择里', () => {
    toggleCard(4);
    expect(useBoardSel.getState()).toMatchObject({ on: true, ids: [4], anchor: 4 });
    toggleCard(4);
    expect(useBoardSel.getState()).toMatchObject({ on: true, ids: [] });
  });

  it('「批量」进来一张没选也在选择里；退出清掉选中和起点', () => {
    startSelect();
    expect(useBoardSel.getState()).toMatchObject({ on: true, ids: [] });
    toggleCard(2);
    clearSelect();
    expect(useBoardSel.getState()).toEqual({ on: false, ids: [], anchor: null });
  });

  it('连选并进已选的，不重复；起点换成这一张', () => {
    toggleCard(9);
    toggleCard(6);
    selectRange([5, 6, 7, 2], 2);
    expect(useBoardSel.getState().ids.sort()).toEqual([2, 6, 7, 9]);
    expect(useBoardSel.getState().anchor).toBe(2);
  });

  it('起点不在这一块：连选退化成点一张', () => {
    toggleCard(9);
    selectRange([5, 6], 6);
    expect(useBoardSel.getState().ids).toEqual([9, 6]);
  });

  it('全选：并进去、不重复', () => {
    toggleCard(1);
    selectIds([1, 2, 3]);
    expect(useBoardSel.getState().ids).toEqual([1, 2, 3]);
  });
});

describe('块标题的勾选框', () => {
  it('一张没选 / 一部分 / 全选上', () => {
    expect(blockState([1, 2], [])).toBe('none');
    expect(blockState([1, 2], [2, 9])).toBe('some');
    expect(blockState([1, 2], [1, 2, 9])).toBe('all');
  });

  it('没全选上时选上这一块，全选上时只取消这一块（别的块的不动）', () => {
    toggleCard(9);
    toggleBlock([1, 2]);
    expect(useBoardSel.getState().ids).toEqual([9, 1, 2]);
    toggleBlock([1, 2]);
    expect(useBoardSel.getState().ids).toEqual([9]);
  });

  it('这一块没有能选的卡片：什么都不做', () => {
    toggleBlock([]);
    expect(useBoardSel.getState().on).toBe(false);
  });
});

describe('bulkLabel（批量加 / 去一个标签）', () => {
  it('整批交给 labelCards 一次写完：加、去都是同一个调用', async () => {
    labelCards.mockResolvedValue({ changed: [1, 3], skipped: [2], failed: [] });
    expect(await bulkLabel([1, 2, 3], 7, true)).toEqual({ changed: [1, 3], skipped: [2], failed: [] });
    expect(labelCards.mock.calls).toEqual([[[1, 2, 3], 7, true]]);
    labelCards.mockResolvedValue({ changed: [4, 5], skipped: [], failed: [] });
    await bulkLabel([4, 5], 7, false);
    expect(labelCards.mock.calls[1]).toEqual([[4, 5], 7, false]);
    expect(toastError).not.toHaveBeenCalled();
  });

  it('有几张失败：只提示一次（第一个错误）', async () => {
    const a = new Error('NOT_FOUND');
    labelCards.mockResolvedValue({ changed: [1], skipped: [], failed: [{ id: 2, error: a }, { id: 3, error: new Error('IN_TRASH') }] });
    const r = await bulkLabel([1, 2, 3], 7, true);
    expect(r.failed.map(f => f.id)).toEqual([2, 3]);
    expect(toastError).toHaveBeenCalledTimes(1);
    expect(toastError).toHaveBeenCalledWith(a);
  });

  it('整个请求失败：提示一次，每张都算失败、不算改了或跳过', async () => {
    const err = new Error('NETWORK');
    labelCards.mockRejectedValue(err);
    const r = await bulkLabel([1, 2], 7, true);
    expect(r).toEqual({ changed: [], skipped: [], failed: [{ id: 1, error: err }, { id: 2, error: err }] });
    expect(toastError).toHaveBeenCalledTimes(1);
  });
});

describe('moveTargets（「移动到」菜单的每一行）', () => {
  // todo: 1 2 · doing: 3（beta 持有）· review: 4 · done: 5 · 未整理: 9
  const b = () =>
    snapshot('p/b', { todo: [1, 2], doing: [3], review: [4], done: [5], rework: [], inbox: [9] }, [{ id: 3, holder: 'beta' }]);
  const row = (key: string, ids: number[]) => moveTargets(b(), ids).find(t => t.key === key)!;

  it('列出工作流的每个状态，最后是「未整理」', () => {
    expect(moveTargets(b(), [1]).map(t => t.key)).toEqual([...FLOW.map(s => s.key), 'inbox']);
    expect(row('inbox', [1]).name).toBe('未整理');
  });

  it('已经在目标状态里的跳过、记进 here；能移的按白板上的顺序', () => {
    expect(row('review', [4, 2, 1])).toMatchObject({ ids: [1, 2], here: 1, held: 0, locked: null });
  });

  it('被 agent 持有的跳过、记进 held', () => {
    expect(row('done', [3, 1])).toMatchObject({ ids: [1], here: 0, held: 1 });
  });

  it('锁定状态（只能领取进入）整行不能选，写明原因', () => {
    const t = row('doing', [1, 2]);
    expect(t.ids).toEqual([]);
    expect(t.locked).toBe('只能由 agent 领取进入');
  });

  it('「未整理」不算锁定：人可以放回去', () => {
    expect(row('inbox', [1, 4])).toMatchObject({ ids: [1, 4], locked: null });
  });
});

describe('bulkMove（批量移动）', () => {
  const t = { key: 'review', name: '待审核', ids: [1, 2], here: 1, held: 1, locked: null };

  it('整批走一次 cardBatch，提示移了几张、跳过的写明', async () => {
    batchCards.mockResolvedValue({ ok: [1, 2], failed: [] });
    expect(await bulkMove(t)).toBe(2);
    expect(batchCards.mock.calls[0][0]).toEqual([1, 2]);
    expect(batchCards.mock.calls[0][1]).toEqual({ op: 'move', column: 'review' });
    expect(batchCards.mock.calls).toHaveLength(1);
    expect(toast).toHaveBeenCalledWith(expect.objectContaining({ title: '2 张移到了「待审核」', text: '跳过：1 张本来就在这里，1 张被 agent 持有' }));
  });

  it('有一张被拒：其余照样移，提示一次错误', async () => {
    const err = new Error('NOT_HOLDER');
    batchCards.mockResolvedValue({ ok: [1], failed: [{ id: 2, error: err }] });
    expect(await bulkMove({ ...t, here: 0, held: 0 })).toBe(1);
    expect(toastError).toHaveBeenCalledTimes(1);
    expect(toast).toHaveBeenCalledWith(expect.objectContaining({ title: '1 张移到了「待审核」', text: undefined }));
  });

  it('没有能移的：不发请求', async () => {
    expect(await bulkMove({ ...t, ids: [] })).toBe(0);
    expect(batchCards).not.toHaveBeenCalled();
  });
});

describe('dispatchPlan（批量派送跳过规则）', () => {
  // todo: 1（已有排队）2（beta 持有）· doing: 3（锁定）· review: 4（能派）5（alpha 持有）· 未整理: 9
  const b = () =>
    snapshot('p/b', { todo: [1, 2], doing: [3], review: [4, 5], inbox: [9] }, [
      { id: 2, holder: 'beta' },
      { id: 5, holder: 'alpha' },
    ]);

  it('能派的按选中顺序留下；锁定、被持有、已有派发、未整理和重复的跳过', () => {
    expect(dispatchPlan(b().statuses, b().cards, [5, 2, 3, 4, 9, 1, 4, 99], new Set([1]))).toEqual({
      ids: [4],
      skipped: [
        { id: 5, why: '被 alpha 持有' },
        { id: 2, why: '被 beta 持有' },
        { id: 3, why: '在锁定状态「doing」' },
        { id: 9, why: '未整理不在批量派送范围' },
        { id: 1, why: '已有排队或在跑的派发' },
        { id: 99, why: '卡片不存在' },
      ],
    });
  });

  it('待审核等非锁定状态也能派发', () => {
    expect(dispatchPlan(b().statuses, b().cards, [4], new Set())).toEqual({ ids: [4], skipped: [] });
  });
});

describe('runBulkDispatch（逐张派发）', () => {
  const made = (id: number) =>
    ({
      id,
      board: 'default/main',
      card: id,
      title: `卡片 ${id}`,
      hostId: 1,
      host: 'win',
      agent: 'codex',
      agentLabel: 'Codex',
      name: null,
      source: 'manual',
      by: { kind: 'human', name: '人' },
      state: 'queued',
      result: null,
      createdAt: 0,
      startedAt: null,
      endedAt: null,
      stopBy: null,
      beatAt: null,
    }) as Dispatch;

  it('一张接一张发；中间一张失败不影响后面的', async () => {
    const err = new Error('BOARD_NOT_ON_HOST');
    const send = vi.fn(async (id: number) => {
      if (id === 2) throw err;
      return made(id);
    });
    const r = await runBulkDispatch([1, 2, 3], send);
    expect(send.mock.calls).toEqual([[1], [2], [3]]);
    expect(r.ok.map(d => d.card)).toEqual([1, 3]);
    expect(r.failed).toEqual([{ id: 2, error: err }]);
  });

  it('没有能派的：一张请求也不发', async () => {
    const send = vi.fn(async (id: number) => made(id));
    expect(await runBulkDispatch([], send)).toEqual({ ok: [], failed: [] });
    expect(send).not.toHaveBeenCalled();
  });

  it('收尾汇总：张数写在标题；去向、跳过、失败的原因写在正文；有失败时是错误提示', () => {
    const t = bulkDispatchToast({ ok: [made(3)], failed: [{ id: 4, error: new Error('机器没有这块白板的目录') }] }, [{ id: 1, why: '已有排队或在跑的派发' }], '派给「Codex」· 随机');
    expect(t).toEqual({
      type: 'error',
      title: '成功 1 张 · 跳过 1 张 · 失败 1 张',
      text: '派给「Codex」· 随机\n跳过：#1 已有排队或在跑的派发\n失败：#4 机器没有这块白板的目录',
    });
    expect(bulkDispatchToast({ ok: [made(1)], failed: [] }, [], '')).toEqual({ type: 'success', title: '成功 1 张 · 跳过 0 张 · 失败 0 张', text: undefined });
  });
});
