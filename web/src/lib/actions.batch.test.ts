/* 批量整理的结果（default/main#512）：提示只按实际成功、失败的 ID 更新 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardSnapshot } from './types';
import { snapshot } from '../../test/fixtures';

const m = vi.hoisted(() => ({
  batch: vi.fn<(ids: number[], op: { op: 'move'; column: string }, patch: (id: number) => unknown) => Promise<{ ok: number[]; failed: { id: number; error: unknown }[] }>>(),
  toastError: vi.fn(),
  board: null as BoardSnapshot | null,
}));

vi.mock('../store/data', async orig => {
  const actual = await orig<typeof import('../store/data')>();
  return {
    ...actual,
    batchCards: (ids: number[], op: { op: 'move'; column: string }, patch: (id: number) => unknown) => m.batch(ids, op, patch),
    movePatch: (id: number, column: string) => ({ move: [id, column] }),
    useData: { getState: () => ({ board: m.board }) },
  };
});

vi.mock('../store/ui', async orig => {
  const actual = await orig<typeof import('../store/ui')>();
  return { ...actual, toastError: (e: unknown) => m.toastError(e) };
});

const { triageMany, untriage } = await import('./actions');

beforeEach(() => {
  m.batch.mockReset();
  m.toastError.mockReset();
  m.board = snapshot('p/a', { inbox: [1, 2] });
});

describe('triageMany（批量整理）', () => {
  it('全部成功：返回每一条的 ID', async () => {
    m.batch.mockResolvedValue({ ok: [1, 2], failed: [] });
    expect(await triageMany([1, 2])).toEqual({ ok: [1, 2], failed: [] });
    expect(m.batch.mock.calls[0][0]).toEqual([1, 2]);
    expect(m.batch.mock.calls[0][1]).toEqual({ op: 'move', column: 'todo' });
  });

  it('部分失败：失败的 ID 和原因单独返回，成功的照常移动', async () => {
    const error = new Error('NOT_HOLDER');
    m.batch.mockResolvedValue({ ok: [2], failed: [{ id: 1, error }] });
    expect(await triageMany([1, 2])).toEqual({ ok: [2], failed: [{ id: 1, error }] });
  });

  it('全部失败：没有成功项', async () => {
    const error = new Error('BUSY');
    m.batch.mockRejectedValue(error);
    expect(await triageMany([1, 2])).toEqual({
      ok: [],
      failed: [
        { id: 1, error },
        { id: 2, error },
      ],
    });
  });
});

describe('untriage（撤销整理）', () => {
  it('整批放回未整理，只提示第一个失败原因', async () => {
    const error = new Error('NOT_HOLDER');
    m.batch.mockResolvedValue({ ok: [2], failed: [{ id: 1, error }] });
    await untriage([1, 2]);
    expect(m.batch.mock.calls[0][0]).toEqual([1, 2]);
    expect(m.batch.mock.calls[0][1]).toEqual({ op: 'move', column: 'inbox' });
    expect(m.toastError).toHaveBeenCalledTimes(1);
    expect(m.toastError).toHaveBeenCalledWith(error);
  });
});
