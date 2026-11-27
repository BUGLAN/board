import { beforeEach, describe, expect, it, vi } from 'vitest';
import { card, snapshot, workspace } from '../../test/fixtures';
import type * as Data from './data';

let D: typeof Data;
beforeEach(async () => {
  vi.resetModules();
  D = await import('./data');
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

describe('「未整理」的新条目', () => {
  const agent = { kind: 'agent' as const, name: 'claude/x' };
  /** p/a 的「未整理」：1 是 agent 记下、没看过的；2 是 agent 记下、人看过的；3 是人自己记的；4 在工作流里 */
  const inbox = () =>
    snapshot('p/a', { inbox: [1, 2, 3], todo: [4] }, [
      { id: 1, createdBy: agent, seenAt: null },
      { id: 2, createdBy: agent, seenAt: 1_000 },
      { id: 3, createdBy: { kind: 'human', name: '' }, seenAt: 500 },
      { id: 4, createdBy: agent, seenAt: null },
    ]);

  it('entryIsNew / newEntryCount：只数 agent 记下、还在「未整理」、还没看过的；sent 之后记下的照常算新', () => {
    const b = inbox();
    expect([1, 2, 3, 4].map(id => D.entryIsNew('p/a', b.cards[id]))).toEqual([true, false, false, false]);
    expect(D.newEntryCount('p/a', b, null)).toBe(1);
    expect(D.newEntryCount('p/a', null, null)).toBe(0);
    // 本窗口刚标完「全部已读」、服务端快照还没回来：标之前记下的不算新了，之后 agent 新记的照常算
    const sent = 1_000;
    const before = { ...card(1), createdAt: 0, column: 'inbox', createdBy: agent, seenAt: null };
    const after = { ...card(5), createdAt: sent + 1, column: 'inbox', createdBy: agent, seenAt: null };
    const later = { ...snapshot('p/a', {}), cols: { inbox: [1, 5] }, cards: { 1: before, 5: after } };
    expect(D.newEntryCount('p/a', later, sent)).toBe(1);
    expect(D.newEntryCount('p/a', later, null)).toBe(2);
  });
});

describe('快捷提示词保存', () => {
  const list = () => [
    { id: 'p-2', title: '重启', text: '重启一下' },
    { id: 'p-1', title: '等发版', text: '等发版' },
  ];

  it('叠在 store 上：当帧生效；还在保存时同一列表再改被拒绝（BUSY），失败只撤自己这一层', async () => {
    const { api } = await import('../lib/api');
    let fail: ((e: unknown) => void) | undefined;
    vi.spyOn(api, 'savePrompts').mockImplementation(() => new Promise((_, no) => (fail = no)));
    D.useData.setState({ prompts: list() });
    // base 由 loadPrompts 写入
    vi.spyOn(api, 'prompts').mockResolvedValue(list());
    await D.loadPrompts();
    const req = D.savePrompts(D.removePromptPatch('p-2'));
    expect(D.useData.getState().prompts!.map(p => p.id)).toEqual(['p-1']);
    expect(D.busy(D.PROMPTS_BUSY)).toBe(true);
    expect(() => D.savePrompts(D.removePromptPatch('p-1'))).toThrow(/保存/);
    await vi.waitFor(() => expect(fail).toBeDefined());
    fail!(new Error('失败'));
    await expect(req).rejects.toThrow('失败');
    expect(D.useData.getState().prompts!.map(p => p.id)).toEqual(['p-2', 'p-1']);
    expect(D.busy(D.PROMPTS_BUSY)).toBe(false);
    vi.restoreAllMocks();
  });
});
