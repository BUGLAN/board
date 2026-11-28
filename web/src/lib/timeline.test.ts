/* 项目 Tab 提交时间轴的纯函数（PRD §10.4）：分段、版本颜色、按行数截断、日期分隔、间隔文字、总数 */
import { describe, expect, it } from 'vitest';
import type { MilestoneCard, Milestones, ReleaseCommit } from './milestones';
import { dayLabel, gapText, PALETTE, rowsThrough, segmentsOf, summary, takeRows, verColor, withDays } from './timeline';
import type { Status } from './types';

const st = (key: string, name: string, p: Partial<Status> = {}): Status => ({ key, name, color: 'todo', claim: false, lock: false, final: false, layout: null, to: [], ...p });
const FLOW = [st('todo', '待办', { claim: true }), st('doing', '进行中', { lock: true }), st('review', '待审核'), st('done', '已完成', { final: true })];
const statusOf = (c: MilestoneCard) => FLOW.find(s => s.key === c.column);

const card = (id: number, column: string, p: Partial<MilestoneCard> = {}): MilestoneCard => ({
  board: 'p/main',
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
const commit = (h: string, at: number, cards: string[] = []): ReleaseCommit => ({ h, at, s: `feat: ${h}`, cards });

const DAY = 86400e3;
// 版本从新到旧：v3 是手动建的标签（没有发版记录），v2、v1、v0 发了版
const m: Milestones = {
  versions: [
    { label: 30, name: 'v3', color: '#8B5CF6', releasedAt: null, commits: [] },
    { label: 20, name: 'v2', color: '#8B5CF6', releasedAt: 10 * DAY, commits: [commit('c2b', 9 * DAY, ['main#2']), commit('c2a', 8 * DAY, ['ops#7'])] },
    { label: 10, name: 'v1', color: '#8B5CF6', releasedAt: 5 * DAY, commits: [commit('c1', 4 * DAY, ['ops#8'])] },
    { label: 1, name: 'v0', color: '#8B5CF6', releasedAt: DAY, commits: [commit('c0b', DAY - 1000), commit('c0a', DAY - 2000, ['main#1'])] },
  ],
  lastRelease: 10 * DAY,
  cards: [
    card(1, 'done'),
    card(2, 'done'),
    card(7, 'done', { board: 'p/ops' }),
    card(5, 'todo', { group: 'next' }),
    card(6, 'review', { group: 'next' }),
    card(9, 'doing', { group: 'next', board: 'p/ops' }),
  ],
};

describe('segmentsOf', () => {
  it('最上面是未发版（按里程碑页的顺序），下面是发了版的版本、从新到旧；手动建的版本标签不进时间轴', () => {
    const segs = segmentsOf(m, null, statusOf);
    expect(segs.map(s => s.name)).toEqual(['未发版', 'v2', 'v1', 'v0']);
    expect(segs[0]).toMatchObject({ kind: 'next', key: 'next', commits: [], prevAt: 10 * DAY });
    // 待审核在待办前面
    expect(segs[0].cards.map(c => c.id)).toEqual([6, 9, 5]);
    expect(segs[1]).toMatchObject({ kind: 'version', key: '20', releasedAt: 10 * DAY, prevAt: 5 * DAY, cards: [] });
    expect(segs[3].prevAt).toBeNull();
  });

  it('版本颜色按发版先后从最早的数起：新发一版不改老版本的颜色，相邻两版不同色', () => {
    const segs = segmentsOf(m, null, statusOf).filter(s => s.kind === 'version');
    expect(segs.map(s => s.color)).toEqual([PALETTE[2], PALETTE[1], PALETTE[0]]);
    const newer: Milestones = { ...m, versions: [{ label: 40, name: 'v4', color: '', releasedAt: 20 * DAY, commits: [commit('c4', 19 * DAY)] }, ...m.versions] };
    const again = segmentsOf(newer, null, statusOf).filter(s => s.kind === 'version');
    expect(again.slice(1).map(s => s.color)).toEqual(segs.map(s => s.color));
    for (let i = 1; i < again.length; i++) expect(again[i].color).not.toBe(again[i - 1].color);
  });

  it('只看一块白板：只留连到这块白板卡片的提交，一个都不剩的版本整段不显示；没连卡片的提交不算', () => {
    const segs = segmentsOf(m, 'p/ops', statusOf);
    expect(segs.map(s => s.name)).toEqual(['未发版', 'v2', 'v1']);
    expect(segs[0].cards.map(c => c.id)).toEqual([9]);
    expect(segs[1].commits.map(c => c.h)).toEqual(['c2a']);
    // 颜色还是整个项目里的那一个
    expect(segs[1].color).toBe(segmentsOf(m, null, statusOf)[1].color);
  });

  it('没有发版记录、也没有未发版的卡片：空', () => {
    expect(segmentsOf({ versions: [], lastRelease: null, cards: [] }, null, statusOf)).toEqual([]);
  });
});

describe('按行数截断', () => {
  const segs = segmentsOf(m, null, statusOf);
  it('前面的段先画满，后面的段不画', () => {
    expect(takeRows(segs, 4).map(r => r.take)).toEqual([3, 1, 0, 0]);
    expect(takeRows(segs, 100).map(r => r.take)).toEqual([3, 2, 1, 2]);
  });
  it('跳到一段要画到它的最后一行', () => {
    expect(rowsThrough(segs, 'next')).toBe(3);
    expect(rowsThrough(segs, '10')).toBe(6);
    expect(rowsThrough(segs, '1')).toBe(8);
  });
});

describe('日期和间隔', () => {
  it('同一天的提交只在第一条前面写日期', () => {
    const t = new Date(2026, 10, 28, 18, 0).getTime();
    const days = withDays([commit('a', t), commit('b', t - 3600e3), commit('c', t - 20 * 3600e3)]).map(d => d.day);
    expect(days).toEqual(['11月28日 周六', null, '11月27日 周五']);
    expect(dayLabel(t)).toBe('11月28日');
  });

  it('两次发版之间：不到一小时写分钟，不到一天写小时，再往上写天', () => {
    expect(gapText(20e3)).toBe('1 分钟');
    expect(gapText(25 * 60e3)).toBe('25 分钟');
    expect(gapText(8 * 3600e3)).toBe('8 小时');
    expect(gapText(2 * DAY + 3600e3)).toBe('2 天');
  });

  it('版本颜色轮流取色板', () => {
    expect(verColor(0)).toBe(PALETTE[0]);
    expect(verColor(PALETTE.length)).toBe(PALETTE[0]);
    expect(PALETTE).not.toContain('#F97316');
  });
});

describe('summary', () => {
  it('只算版本，不算未发版；没有版本时为空', () => {
    expect(summary(segmentsOf(m, null, statusOf))).toBe('3 个版本 · 5 个提交');
    expect(summary(segmentsOf({ ...m, versions: [] }, null, statusOf))).toBeNull();
  });
});
