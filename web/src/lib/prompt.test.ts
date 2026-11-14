import { describe, expect, it } from 'vitest';
import { cardPrompt, detailPrompt, quickPrompts, splitPrompt } from './prompt';

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
  it('判过够细、人点了「强制拆解」：句子写明强制（default/main#306）', () => {
    expect(splitPrompt('default/main', 5, { force: true })).toBe('/board-skills 强制拆解 default项目之中的main白板中的#5需求');
    expect(detailPrompt('default/main', { id: 5, column: 'inbox' }, true)).toBe(splitPrompt('default/main', 5, { force: true }));
  });
});

describe('快捷提示词里的拆解：先判粒度（default/main#306）', () => {
  it('两条拆解都先判粒度，够小就整条建一张卡', () => {
    const list = quickPrompts('default/main', ['待办', '返工']);
    for (const kind of ['split', 'split-inbox']) {
      const q = list.find(x => x.kind === kind)!;
      expect(q.text).toContain('先判断下面这条需求的粒度');
      expect(q.text).toContain('已经是一个独立的小任务就整条建成一张卡，不拆');
      expect(q.text.endsWith(kind === 'split' ? '记到default项目之中的main白板的待办：' : '记到default项目之中的main白板的未整理：')).toBe(true);
    }
  });
});
