import { describe, expect, it } from 'vitest';
import { cardRef, cardsOf, commitsByCard, defaultPage, gap, isVersionName, looseCommits, matchCard, pagesOf, roleOf, sortCards, startOf, summarize, type MilestoneCard, type Milestones } from './milestones';
import type { Status } from './types';

const st = (key: string, name: string, p: Partial<Status> = {}): Status => ({ key, name, color: 'todo', claim: false, lock: false, final: false, layout: null, to: [], ...p });
const FLOW = [st('todo', '待办', { claim: true }), st('doing', '进行中', { lock: true }), st('review', '待审核'), st('human', '需要人处理', { error: true }), st('done', '已完成', { final: true })];
const statusOf = (c: MilestoneCard) => FLOW.find(s => s.key === c.column);

const card = (id: number, column: string, p: Partial<MilestoneCard> = {}): MilestoneCard => ({
  board: 'default/main',
  id,
  title: `卡片 ${id}`,
  column,
  labels: [],
  holder: null,
  createdAt: id,
  movedAt: null,
  group: 'version',
  ...p,
});

const m: Milestones = {
  versions: [
    { label: 3, name: 'v0.3.0', color: '#8B5CF6', releasedAt: null, commits: [] },
    {
      label: 2,
      name: 'v0.2.0',
      color: '#8B5CF6',
      releasedAt: 200,
      commits: [
        { h: 'b2', at: 190, s: 'fix: 二', cards: ['main#2'] },
        { h: 'b1', at: 150, s: 'feat: 二', cards: ['main#2', 'site#9'] },
        { h: 'b0', at: 140, s: 'docs: 没写卡片行', cards: [] },
      ],
    },
    { label: 1, name: 'v0.1.0', color: '#8B5CF6', releasedAt: 100, commits: [{ h: 'a1', at: 90, s: 'feat: 一', cards: ['main#1'] }] },
  ],
  lastRelease: 200,
  cards: [
    card(1, 'done', { labels: [1], movedAt: 95 }),
    card(2, 'done', { labels: [2, 7], movedAt: 195 }),
    card(4, 'review', { labels: [2] }),
    card(5, 'doing', { group: 'next', holder: 'claude' }),
    card(6, 'done', { group: 'none', movedAt: 50 }),
  ],
};

describe('里程碑', () => {
  it('版本 = v 加数字开头的标签', () => {
    expect(['v0.1.21', 'V2', 'v1-beta'].every(isVersionName)).toBe(true);
    expect(['visual', 'v', '0.1', 'bug'].some(isVersionName)).toBe(false);
  });

  it('页的顺序和默认页', () => {
    expect(pagesOf(m)).toEqual(['next', 3, 2, 1, 'none']);
    expect(defaultPage(m)).toBe('next');
    // 未发版没有卡片：默认打开最新的版本，但「未发版」这一页还在（有发版记录）
    const quiet = { ...m, cards: m.cards.filter(c => c.group !== 'next') };
    expect(pagesOf(quiet)[0]).toBe('next');
    expect(defaultPage(quiet)).toBe(3);
    // 什么都没有
    expect(pagesOf({ versions: [], lastRelease: null, cards: [] })).toEqual([]);
    expect(defaultPage({ versions: [], lastRelease: null, cards: [] })).toBeNull();
  });

  it('每一页的卡片和进度图的起点', () => {
    expect(cardsOf(m, 2).map(c => c.id)).toEqual([2, 4]);
    expect(cardsOf(m, 'next').map(c => c.id)).toEqual([5]);
    expect(cardsOf(m, 'none').map(c => c.id)).toEqual([6]);
    expect(startOf(m, 2)).toBe(100);
    // 没发版的 v0.3.0：上一个发了版的是 v0.2.0
    expect(startOf(m, 3)).toBe(200);
    expect(startOf(m, 1)).toBeNull();
    expect(startOf(m, 'next')).toBe(200);
  });

  it('提交号按卡片对上，没写卡片行的单独列出', () => {
    const v = m.versions[1];
    const by = commitsByCard(v);
    expect(by.get(cardRef(m.cards[1]))?.map(c => c.h)).toEqual(['b2', 'b1']);
    expect(by.get('site#9')?.map(c => c.h)).toEqual(['b1']);
    expect(looseCommits(v).map(c => c.h)).toEqual(['b0']);
    expect(looseCommits(null)).toEqual([]);
  });

  it('状态归类、进度和排序', () => {
    expect(FLOW.map(s => roleOf(s))).toEqual(['todo', 'doing', 'review', 'error', 'done']);
    expect(roleOf(undefined)).toBe('todo');
    const s = summarize(cardsOf(m, 2), statusOf);
    expect([s.total, s.done, s.pct]).toEqual([2, 1, 50]);
    expect(s.byStatus.map(x => `${x.name}${x.n}`)).toEqual(['已完成1', '待审核1']);
    expect(summarize([], statusOf).pct).toBe(0);
    const cards = [card(10, 'done', { movedAt: 5 }), card(11, 'todo'), card(12, 'review'), card(13, 'done', { movedAt: 9 })];
    expect(sortCards(cards, statusOf).map(c => c.id)).toEqual([13, 10, 12, 11]);
  });

  it('筛选和间隔', () => {
    expect(m.cards.filter(matchCard('#5')).map(c => c.id)).toEqual([5]);
    expect(m.cards.filter(matchCard('卡片 2')).map(c => c.id)).toEqual([2]);
    expect(m.cards.filter(matchCard('  ')).length).toBe(5);
    expect(gap(30 * 60e3)).toBe('1 小时');
    expect(gap(6 * 3600e3)).toBe('6 小时');
    expect(gap(50 * 3600e3)).toBe('2 天');
  });
});
