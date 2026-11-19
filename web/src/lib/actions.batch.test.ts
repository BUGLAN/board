/* 批量整理的结果（default/main#512）：提示只按实际成功、失败的 ID 更新 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardSnapshot } from './types';
import { snapshot } from '../../test/fixtures';

const m = vi.hoisted(() => ({
  exec: vi.fn<(cmd: { op: string; id: number; column: string }, patch: unknown) => Promise<unknown>>(),
  board: null as BoardSnapshot | null,
}));

vi.mock('../store/data', async orig => {
  const actual = await orig<typeof import('../store/data')>();
  return {
    ...actual,
    exec: (cmd: { op: string; id: number; column: string }, patch: unknown) => m.exec(cmd, patch),
    movePatch: (id: number, column: string) => ({ move: [id, column] }),
    useData: { getState: () => ({ board: m.board }) },
  };
});

const { triageMany } = await import('./actions');

beforeEach(() => {
  m.exec.mockReset();
  m.board = snapshot('p/a', { inbox: [1, 2] });
});

describe('triageMany（批量整理）', () => {
  it('全部成功：返回每一条的 ID', async () => {
    m.exec.mockResolvedValue({});
    expect(await triageMany([1, 2])).toEqual({ ok: [1, 2], failed: [] });
    expect(m.exec.mock.calls.map(([cmd]) => cmd.id)).toEqual([1, 2]);
  });

  it('部分失败：失败的 ID 和原因单独返回，成功的照常移动', async () => {
    const error = new Error('NOT_HOLDER');
    m.exec.mockImplementation(async cmd => {
      if (cmd.id === 1) throw error;
      return {};
    });
    expect(await triageMany([1, 2])).toEqual({ ok: [2], failed: [{ id: 1, error }] });
  });

  it('全部失败：没有成功项', async () => {
    const error = new Error('BUSY');
    m.exec.mockRejectedValue(error);
    expect(await triageMany([1, 2])).toEqual({
      ok: [],
      failed: [
        { id: 1, error },
        { id: 2, error },
      ],
    });
  });
});
