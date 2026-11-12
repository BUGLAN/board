/* 命令面板的跨白板搜索结果：高亮什么、哪些放到「其他白板」一组 */
import { describe, expect, it } from 'vitest';
import { highlightQuery, otherHits } from './search';
import type { SearchHit } from './types';

const hit = (board: string, id: number, title = '标题'): SearchHit => ({
  board,
  id,
  title,
  column: 'todo',
  status: '待办',
  matched: 'title',
  labels: [],
  updatedAt: 0,
  score: 100,
});

describe('highlightQuery（高亮的文字）', () => {
  it('普通文字原样返回，去掉首尾空白', () => {
    expect(highlightQuery(' 登录 按钮 ')).toBe('登录 按钮');
  });

  it('定位写法没有可高亮的正文', () => {
    expect(highlightQuery('#12')).toBe('');
    expect(highlightQuery('oss/main#3')).toBe('');
    expect(highlightQuery('label:bug')).toBe('');
    expect(highlightQuery('LABEL:bug')).toBe('');
  });

  it('文字里带 # 但不是白板路径时照常高亮', () => {
    expect(highlightQuery('gh#140')).toBe('gh#140');
  });
});

describe('otherHits（其他白板）', () => {
  it('去掉当前白板的结果，保持服务端给的顺序', () => {
    const list = [hit('oss/main', 1), hit('default/main', 2), hit('mini/main', 3)];
    expect(otherHits(list, 'default/main').map(h => `${h.board}#${h.id}`)).toEqual(['oss/main#1', 'mini/main#3']);
  });

  it('同一张卡片只留一条', () => {
    const list = [hit('oss/main', 1), hit('oss/main', 1)];
    expect(otherHits(list, 'default/main')).toHaveLength(1);
  });

  it('都在当前白板时为空', () => {
    expect(otherHits([hit('default/main', 2)], 'default/main')).toEqual([]);
  });
});
