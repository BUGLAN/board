/* 白板勾选卡片、批量打标签（PRD §10.3「批量打标签」）：连选、整块勾选、批量写入 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

const addLabel = vi.fn<(id: number, label: number) => Promise<boolean>>();
const removeLabel = vi.fn<(id: number, label: number) => Promise<boolean>>();
const toastError = vi.fn();
vi.mock('../store/data', () => ({
  addLabel: (id: number, l: number) => addLabel(id, l),
  removeLabel: (id: number, l: number) => removeLabel(id, l),
  useData: { getState: () => ({ detailId: null }) },
}));
vi.mock('../store/ui', () => ({ overlayOpen: () => false, toastError: (e: unknown) => toastError(e) }));
vi.mock('./drag', () => ({ justDropped: () => false }));

const { blockState, bulkLabel, clearSelect, rangeIds, selectIds, selectRange, startSelect, toggleBlock, toggleCard, useBoardSel } = await import('./boardSelect');

beforeEach(() => {
  clearSelect();
  addLabel.mockReset();
  removeLabel.mockReset();
  toastError.mockReset();
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
  it('每张都走 addLabel；本来就有的不算进改了几张', async () => {
    addLabel.mockImplementation(async id => id !== 2);
    expect(await bulkLabel([1, 2, 3], 7, true)).toBe(2);
    expect(addLabel.mock.calls).toEqual([[1, 7], [2, 7], [3, 7]]);
    expect(removeLabel).not.toHaveBeenCalled();
  });

  it('去标签走 removeLabel', async () => {
    removeLabel.mockResolvedValue(true);
    expect(await bulkLabel([4, 5], 7, false)).toBe(2);
    expect(removeLabel.mock.calls).toEqual([[4, 7], [5, 7]]);
  });

  it('有一张失败：其余照样写，提示一次错误', async () => {
    const err = new Error('BUSY');
    addLabel.mockImplementation(async id => {
      if (id === 2) throw err;
      return true;
    });
    expect(await bulkLabel([1, 2, 3], 7, true)).toBe(2);
    expect(toastError).toHaveBeenCalledTimes(1);
    expect(toastError).toHaveBeenCalledWith(err);
  });
});
