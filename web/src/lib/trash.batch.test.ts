/* 回收站批量恢复、永久删除（default/main#512）：只按实际成功、失败的 ID 更新提示和选择 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

const m = vi.hoisted(() => ({
  batch: vi.fn<(ids: number[], op: { op: 'delete' | 'restore' | 'purge' }, patch: (id: number) => unknown) => Promise<{ ok: number[]; failed: { id: number; error: unknown }[] }>>(),
  trash: vi.fn<(path: string) => Promise<unknown[]>>(),
  toast: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('./api', async orig => {
  const actual = await orig<typeof import('./api')>();
  return { ...actual, api: { ...actual.api, trash: (path: string) => m.trash(path) } };
});

vi.mock('../store/data', async orig => {
  const actual = await orig<typeof import('../store/data')>();
  return {
    ...actual,
    batchCards: (ids: number[], op: { op: 'delete' | 'restore' | 'purge' }, patch: (id: number) => unknown) => m.batch(ids, op, patch),
    useData: { getState: () => ({ path: 'p/a', board: { rev: 0, statuses: [], cols: {}, cards: {} }, rev: 0 }) },
  };
});

vi.mock('../store/ui', async orig => {
  const actual = await orig<typeof import('../store/ui')>();
  return { ...actual, toast: (...a: unknown[]) => m.toast(...a), toastError: (...a: unknown[]) => m.toastError(...a) };
});

let T: typeof import('./trash');
const error = new Error('INTERNAL');
const toastTitle = () => (m.toast.mock.calls.at(-1)?.[0] as { title?: string } | undefined)?.title;

beforeEach(async () => {
  Object.values(m).forEach(fn => fn.mockReset());
  m.trash.mockResolvedValue([]);
  vi.resetModules();
  T = await import('./trash');
});

describe('restoreMany（批量恢复）', () => {
  it('全部成功：整批一个请求并返回每一条的 ID', async () => {
    m.batch.mockResolvedValue({ ok: [1, 2], failed: [] });
    expect(await T.restoreMany('p/a', [1, 2])).toEqual({ ok: [1, 2], failed: [] });
    expect(m.batch.mock.calls[0][0]).toEqual([1, 2]);
    expect(m.batch.mock.calls[0][1]).toEqual({ op: 'restore' });
    expect(m.batch.mock.calls).toHaveLength(1);
    expect(m.toastError).not.toHaveBeenCalled();
  });

  it('部分失败：返回成功的 #2，错误只提示一次', async () => {
    m.batch.mockResolvedValue({ ok: [2], failed: [{ id: 1, error }] });
    expect(await T.restoreMany('p/a', [1, 2])).toEqual({ ok: [2], failed: [{ id: 1, error }] });
    expect(m.toastError).toHaveBeenCalledTimes(1);
    expect(m.toastError).toHaveBeenCalledWith(error);
  });

  it('全部失败：没有成功项，原内容不动', async () => {
    m.batch.mockRejectedValue(error);
    expect(await T.restoreMany('p/a', [1, 2])).toEqual({
      ok: [],
      failed: [
        { id: 1, error },
        { id: 2, error },
      ],
    });
    expect(m.toastError).toHaveBeenCalledTimes(1);
  });
});

describe('purgeCards（批量永久删除）', () => {
  it('全部成功：整批一个请求，提示实际删除的张数', async () => {
    m.batch.mockResolvedValue({ ok: [1, 2], failed: [] });
    expect(await T.purgeCards([1, 2])).toEqual({ ok: [1, 2], failed: [] });
    expect(m.batch.mock.calls[0][0]).toEqual([1, 2]);
    expect(m.batch.mock.calls[0][1]).toEqual({ op: 'purge' });
    expect(m.batch.mock.calls).toHaveLength(1);
    expect(toastTitle()).toBe('已永久删除 2 张卡片');
    expect(m.toastError).not.toHaveBeenCalled();
  });

  it('部分失败：提示实际删掉的 #2，错误只提示一次', async () => {
    m.batch.mockResolvedValue({ ok: [2], failed: [{ id: 1, error }] });
    expect(await T.purgeCards([1, 2])).toEqual({ ok: [2], failed: [{ id: 1, error }] });
    expect(toastTitle()).toBe('已永久删除 #2');
    expect(m.toastError).toHaveBeenCalledTimes(1);
  });

  it('全部失败：不提示成功', async () => {
    m.batch.mockRejectedValue(error);
    await T.purgeCards([1, 2]);
    expect(m.toast).not.toHaveBeenCalled();
    expect(m.toastError).toHaveBeenCalledTimes(1);
  });
});

describe('deleteCards（批量删除）', () => {
  it('整批一个请求，返回是否全部删掉', async () => {
    m.batch.mockResolvedValue({ ok: [1, 2], failed: [] });
    expect(await T.deleteCards([1, 2], { silent: true })).toBe(true);
    expect(m.batch.mock.calls[0][0]).toEqual([1, 2]);
    expect(m.batch.mock.calls[0][1]).toEqual({ op: 'delete' });
    expect(m.batch.mock.calls).toHaveLength(1);
    expect(m.trash).not.toHaveBeenCalled();
  });
});
