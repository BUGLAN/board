import { describe, expect, it } from 'vitest';
import { confirm, settleConfirm, useUI } from './ui';

describe('确认框排队', () => {
  it('连续两次 confirm 各自拿到结果，第一个 settle 前第二个不出现', async () => {
    let first: boolean | undefined;
    let second: boolean | undefined;
    const p1 = confirm({ title: '删除白板？', text: '删了就没了' }).then(v => (first = v));
    const p2 = confirm({ title: '发现新版本', text: '现在更新？' }).then(v => (second = v));

    expect(useUI.getState().confirm?.title).toBe('删除白板？');
    settleConfirm(true);
    expect(useUI.getState().confirm?.title).toBe('发现新版本');
    expect(first).toBeUndefined();

    settleConfirm(false);
    await Promise.all([p1, p2]);
    expect(useUI.getState().confirm).toBeNull();
    expect([first, second]).toEqual([true, false]);
  });
});
