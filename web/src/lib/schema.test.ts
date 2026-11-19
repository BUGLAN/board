import { describe, expect, it } from 'vitest';
import { cardTotal, flowOf } from './schema';

describe('cardTotal（切换器的卡片总数）', () => {
  it('所有状态 + 未整理相加', () => {
    expect(cardTotal({ todo: 3, doing: 2, review: 2, done: 1, rework: 0, check: 0, inbox: 4 })).toBe(12);
  });
  it('没有 counts 时是 0', () => {
    expect(cardTotal(undefined)).toBe(0);
    expect(cardTotal(null)).toBe(0);
    expect(cardTotal({})).toBe(0);
  });
  it('和可领取数分开算：总数包含可领取和未整理', () => {
    const statuses = [
      { key: 'todo', name: '待办', claim: true, to: ['doing'] },
      { key: 'doing', name: '进行中', lock: true, to: [] },
      { key: 'rework', name: '返工', claim: true, to: ['doing'] },
    ] as unknown as Parameters<typeof flowOf>[0];
    const counts = { todo: 3, doing: 1, rework: 2, inbox: 4 };
    expect(flowOf(statuses).ready(counts)).toBe(5);
    expect(cardTotal(counts)).toBe(10);
  });
});
