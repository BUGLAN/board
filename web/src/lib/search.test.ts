/* 命令面板的跨白板搜索结果：高亮什么、哪些放到「其他白板」一组 */
import { describe, expect, it } from 'vitest';
import { card } from '../../test/fixtures';
import { highlightQuery, otherHits, searchBoardCards } from './search';
import type { SearchHit } from './types';

const hit = (board: string, id: number, title = '标题'): SearchHit => ({
  board,
  id,
  title,
  column: 'todo',
  status: '待办',
  labels: [],
  updatedAt: 0,
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

describe('searchBoardCards（当前白板卡片）', () => {
  it('按最近更新倒序，同一时间按编号倒序', () => {
    const cards = [
      card(1, { title: '界面', updatedAt: 10 }),
      card(2, { title: '界面', updatedAt: 30 }),
      card(3, { title: '界面', updatedAt: 20 }),
      card(4, { title: '界面', updatedAt: 30 }),
    ];
    expect(searchBoardCards(cards, '界面').map(c => c.id)).toEqual([4, 2, 3, 1]);
  });

  it('标题、描述、编号都能搜到，未整理和已完成一视同仁，最多 12 条', () => {
    const cards = [
      card(20, { title: '界面', column: 'inbox', updatedAt: 200 }),
      card(19, { title: '界面', column: 'done', updatedAt: 190 }),
      ...Array.from({ length: 11 }, (_, i) => card(30 + i, { title: '界面', updatedAt: 180 - i })),
      card(99, { title: '别的', body: '界面', updatedAt: 1 }),
      card(100, { title: '别的', updatedAt: 999 }),
    ];
    const out = searchBoardCards(cards, '界面');
    expect(out).toHaveLength(12);
    expect(out.map(c => c.id)).toEqual([20, 19, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39]);
    expect(searchBoardCards(cards, '#20').map(c => c.id)).toEqual([20]);
    expect(searchBoardCards(cards, '别的').map(c => c.id)).toEqual([100, 99]);
  });
});
