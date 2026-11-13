import { describe, expect, it } from 'vitest';
import { cardPrompt, detailPrompt, splitPrompt } from './prompt';

describe('复制为提示词（PRD §10.9）', () => {
  it('卡片：开始执行这一张', () => {
    expect(cardPrompt('mini/main', 2)).toBe('/board-skills 开始执行 mini项目之中的main白板中的#2任务');
  });
  it('「未整理」里的条目：拆解这一条（SKILL.md 的句子表按这个写法识别）', () => {
    expect(splitPrompt('default/main', 5)).toBe('/board-skills 拆解 default项目之中的main白板中的#5需求');
  });
  it('卡片详情按卡片在哪选句子', () => {
    expect(detailPrompt('default/main', { id: 5, column: 'inbox' })).toBe(splitPrompt('default/main', 5));
    expect(detailPrompt('default/main', { id: 5, column: 'todo' })).toBe(cardPrompt('default/main', 5));
  });
});
