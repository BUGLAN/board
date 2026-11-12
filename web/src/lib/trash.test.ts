/* 回收站恢复的位置（PRD §8 规则 10）：和服务端 engine.rs 的 Cmd::Restore 一致 */
import { describe, expect, it } from 'vitest';
import { FLOW, status } from '../../test/fixtures';
import { restoreColumn, restorePos } from './trash';

describe('restoreColumn（恢复后放回哪一列）', () => {
  it('原来那一列还在就用它', () => {
    expect(restoreColumn({ col: 'review', heldFrom: null }, FLOW)).toBe('review');
  });

  it('「未整理」不在工作流里，但恢复后还是回「未整理」，不扔进待办', () => {
    expect(restoreColumn({ col: 'inbox', heldFrom: null }, FLOW)).toBe('inbox');
  });

  it('原来在锁定状态（被删时还在做）：回到当初领取它的可领取状态', () => {
    expect(restoreColumn({ col: 'doing', heldFrom: 'rework' }, FLOW)).toBe('rework');
  });

  it('领取它的那个可领取状态已经删了，或者不再是可领取：回首个可领取状态', () => {
    expect(restoreColumn({ col: 'doing', heldFrom: 'gone' }, FLOW)).toBe('todo');
    // 旧服务端没有 heldFrom
    expect(restoreColumn({ col: 'doing' }, FLOW)).toBe('todo');
    expect(restoreColumn({ col: 'doing', heldFrom: null }, FLOW)).toBe('todo');
    // 那个状态还在、但已经不可领取了
    const noClaim = FLOW.map(s => (s.key === 'rework' ? status('rework', { to: ['doing'] }) : s));
    expect(restoreColumn({ col: 'doing', heldFrom: 'rework' }, noClaim)).toBe('todo');
  });

  it('原来那一列已经删了：首个可领取状态', () => {
    expect(restoreColumn({ col: 'gone', heldFrom: null }, FLOW)).toBe('todo');
    expect(restoreColumn(null, FLOW)).toBe('todo');
    expect(restoreColumn({ col: 'review' }, undefined)).toBe('todo');
  });
});

describe('restorePos（恢复后排第几位）', () => {
  const at = (column: string, pos: number) => ({ column, pos }) as never;
  it('还是原来那一列就放回原来的位置，否则放最后', () => {
    expect(restorePos(at('todo', 1), 'todo', [4, 5, 6])).toBe(1);
    expect(restorePos(at('todo', 9), 'todo', [4, 5, 6])).toBe(3);
    // 被删时还在做：恢复回当初领取它的返工，和删除前不在同一列，放最下面
    expect(restorePos(at('doing', 1), 'rework', [4, 5, 6])).toBe(3);
    expect(restorePos(at('inbox', 1), 'inbox', [4, 5, 6])).toBe(1);
    expect(restorePos(null, 'todo', [4, 5])).toBe(2);
  });
});
