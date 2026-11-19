/* 未整理的批量动作（default/main#512）：提示、选择、错误都只按实际结果更新 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { snapshot } from '../../test/fixtures';

const m = vi.hoisted(() => ({
  setLater: vi.fn(),
  triage: vi.fn(),
  triageMany: vi.fn(),
  untriage: vi.fn(),
  toast: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../store/data', async orig => {
  const actual = await orig<typeof import('../store/data')>();
  return { ...actual, setLater: (...a: unknown[]) => m.setLater(...a) };
});

vi.mock('../lib/actions', async orig => {
  const actual = await orig<typeof import('../lib/actions')>();
  return {
    ...actual,
    triage: (...a: unknown[]) => m.triage(...a),
    triageMany: (...a: unknown[]) => m.triageMany(...a),
    untriage: (...a: unknown[]) => m.untriage(...a),
  };
});

vi.mock('../store/ui', async orig => {
  const actual = await orig<typeof import('../store/ui')>();
  return { ...actual, toast: (...a: unknown[]) => m.toast(...a), toastError: (...a: unknown[]) => m.toastError(...a) };
});

vi.mock('./triageMotion', async orig => {
  const actual = await orig<typeof import('./triageMotion')>();
  return { ...actual, triageToastDelay: () => 0, leaveOne: vi.fn() };
});

const { useData } = await import('../store/data');
const { doLater, doTriage, doUnlater, keepBatchSelection, useInbox } = await import('./Inbox');

const error = new Error('NOT_HOLDER');
const toastTitle = () => (m.toast.mock.calls.at(-1)?.[0] as { title?: string } | undefined)?.title;

beforeEach(() => {
  Object.values(m).forEach(fn => fn.mockReset());
  useInbox.setState({ checked: [1, 2], open: null, view: 'list', page: 'inbox' });
  useData.setState({ board: snapshot('p/a', { inbox: [1, 2] }) });
});

describe('keepBatchSelection（批量后的勾选）', () => {
  it('全部成功：都去掉', () => {
    expect(keepBatchSelection([1, 2], [1, 2, 3], { ok: [1, 2], failed: [] })).toEqual([3]);
  });

  it('部分失败：失败的留下，成功的去掉', () => {
    expect(keepBatchSelection([1, 2], [1, 2, 3], { ok: [2], failed: [{ id: 1, error }] })).toEqual([1, 3]);
  });

  it('全部失败：发起时选中的都留下', () => {
    expect(keepBatchSelection([1, 2], [], { ok: [], failed: [{ id: 1, error }, { id: 2, error }] })).toEqual([1, 2]);
  });
});

describe('doTriage（批量整理）', () => {
  it('全部成功：提示实际移走的张数', async () => {
    m.triageMany.mockResolvedValue({ ok: [1, 2], failed: [] });
    expect(await doTriage([1, 2])).toEqual({ ok: [1, 2], failed: [] });
    expect(toastTitle()).toBe('2 条已整理到「todo」');
    expect(useInbox.getState().checked).toEqual([]);
    expect(m.toastError).not.toHaveBeenCalled();
  });

  it('部分失败：只提示成功的 #2，失败项保留选择并提示一次错误', async () => {
    m.triageMany.mockResolvedValue({ ok: [2], failed: [{ id: 1, error }] });
    await doTriage([1, 2]);
    expect(toastTitle()).toBe('#2 已整理到「todo」');
    expect(useInbox.getState().checked).toEqual([1]);
    expect(m.toastError).toHaveBeenCalledTimes(1);
    expect(m.toastError).toHaveBeenCalledWith(error);
  });

  it('全部失败：不提示成功，原选择保留', async () => {
    m.triageMany.mockResolvedValue({ ok: [], failed: [{ id: 1, error }, { id: 2, error }] });
    await doTriage([1, 2]);
    expect(m.toast).not.toHaveBeenCalled();
    expect(useInbox.getState().checked).toEqual([1, 2]);
    expect(m.toastError).toHaveBeenCalledTimes(1);
  });
});

describe('doLater（稍后再做）', () => {
  it('全部成功：提示实际收起的张数，成功项从选择里去掉', async () => {
    useInbox.setState({ open: 2 });
    m.setLater.mockResolvedValue({ ok: [1, 2], failed: [] });
    await doLater([1, 2]);
    expect(toastTitle()).toBe('2 条已移到「稍后再做」');
    expect(useInbox.getState()).toMatchObject({ checked: [], open: null });
    expect(m.toastError).not.toHaveBeenCalled();
  });

  it('部分失败：提示 #2，打开的失败项不收起，选择保留失败项', async () => {
    useInbox.setState({ open: 1 });
    m.setLater.mockResolvedValue({ ok: [2], failed: [{ id: 1, error }] });
    await doLater([1, 2]);
    expect(toastTitle()).toBe('#2 已移到「稍后再做」');
    expect(useInbox.getState()).toMatchObject({ checked: [1], open: 1 });
    expect(m.toastError).toHaveBeenCalledTimes(1);
  });

  it('全部失败：不提示成功，选择和打开的行都不动', async () => {
    useInbox.setState({ open: 1 });
    m.setLater.mockResolvedValue({ ok: [], failed: [{ id: 1, error }, { id: 2, error }] });
    await doLater([1, 2]);
    expect(m.toast).not.toHaveBeenCalled();
    expect(useInbox.getState()).toMatchObject({ checked: [1, 2], open: 1 });
    expect(m.toastError).toHaveBeenCalledTimes(1);
  });
});

describe('doUnlater（放回未整理）', () => {
  it('全部成功：提示实际放回的张数', async () => {
    m.setLater.mockResolvedValue({ ok: [1, 2], failed: [] });
    expect(await doUnlater([1, 2])).toEqual({ ok: [1, 2], failed: [] });
    expect(toastTitle()).toBe('2 条已放回「未整理」');
    expect(m.toastError).not.toHaveBeenCalled();
  });

  it('部分失败：只提示成功的 #2，错误只提示一次', async () => {
    m.setLater.mockResolvedValue({ ok: [2], failed: [{ id: 1, error }] });
    await doUnlater([1, 2]);
    expect(toastTitle()).toBe('#2 已放回「未整理」');
    expect(m.toastError).toHaveBeenCalledTimes(1);
  });

  it('全部失败：不提示成功', async () => {
    m.setLater.mockResolvedValue({ ok: [], failed: [{ id: 1, error }, { id: 2, error }] });
    await doUnlater([1, 2]);
    expect(m.toast).not.toHaveBeenCalled();
    expect(m.toastError).toHaveBeenCalledTimes(1);
  });
});
