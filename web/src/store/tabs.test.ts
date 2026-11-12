/* 活动栏顺序：界面（Workbench 的 TAB_META / TAB_ORDER）、数字快捷键（App）、命令面板（Palette 的 TAB_ITEM）
   都从 store/ui 的 TAB_ORDER 来，设置固定在底部。这里守住顺序本身，免得改一处漏一处、按 2 却打开第 4 个标签 */
import { describe, expect, it } from 'vitest';
import { TAB_ORDER } from './ui';

describe('活动栏顺序', () => {
  it('白板 → 未整理 → 大盘 → 项目 → 机器 → 设置', () => {
    expect(TAB_ORDER).toEqual(['board', 'inbox', 'stats', 'projects', 'hosts', 'settings']);
  });

  it('数字快捷键 1~6 按这个顺序切换（App 用 TAB_ORDER[n-1]，命令面板的提示也按同一个下标）', () => {
    // 独立写一遍期望的顺序：不能拿 TAB_ORDER 和自己比，那样恒真
    const expected = ['board', 'inbox', 'stats', 'projects', 'hosts', 'settings'];
    expect(['1', '2', '3', '4', '5', '6'].map(k => TAB_ORDER[Number(k) - 1])).toEqual(expected);
  });

  it('每个 Tab 正好出现一次', () => {
    expect(new Set(TAB_ORDER).size).toBe(TAB_ORDER.length);
  });
});
