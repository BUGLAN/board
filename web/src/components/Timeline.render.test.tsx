/* 项目 Tab 的提交时间轴（PRD §10.4）：一个提交一栏，连到卡片的挂卡片、点开详情，没连卡片的只有提交说明、不能点；
   按版本分段、右栏版本索引；侧栏点白板只看这块白板；没有发版记录时的空状态 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FLOW, workspace } from '../../test/fixtures';
import type { Milestones } from '../lib/milestones';
import { useData } from '../store/data';
import { setTlBoard } from '../store/timeline';
import { TimelinePage } from './Timeline';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const server = vi.hoisted(() => ({ milestones: vi.fn(), openCardAt: vi.fn() }));
vi.mock('../lib/api', async orig => {
  const real = await orig<typeof import('../lib/api')>();
  return { ...real, api: { ...real.api, milestones: server.milestones } };
});
vi.mock('../lib/actions', async orig => ({ ...(await orig<typeof import('../lib/actions')>()), openCardAt: server.openCardAt }));

const DAY = 86400e3;
const data: Milestones = {
  versions: [
    {
      label: 20,
      name: 'v0.2.0',
      color: '#8B5CF6',
      releasedAt: 10 * DAY,
      commits: [
        { h: 'c2b', at: 9 * DAY, s: 'fix: 修二', cards: ['a#2'] },
        { h: 'c2a', at: 8 * DAY, s: 'chore: 升级依赖', cards: [] },
      ],
    },
    { label: 10, name: 'v0.1.0', color: '#8B5CF6', releasedAt: 5 * DAY, commits: [{ h: 'c1', at: 4 * DAY, s: 'feat: 做一', cards: ['b#1', 'a#99'] }] },
  ],
  lastRelease: 10 * DAY,
  cards: [
    { board: 'p/a', id: 2, title: '修二', column: 'done', labels: [1], holder: null, createdAt: 0, movedAt: 9 * DAY, group: 'version' },
    { board: 'p/b', id: 1, title: '做一', column: 'review', labels: [], holder: null, createdAt: 0, movedAt: 4 * DAY, group: 'version' },
    { board: 'p/a', id: 5, title: '在做的', column: 'doing', labels: [], holder: 'claude', createdAt: 0, movedAt: 11 * DAY, group: 'next' },
  ],
};

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

function ws() {
  const w = workspace();
  for (const b of w.projects[0].boards) Object.assign(b, { statuses: FLOW, counts: {}, holders: [], updatedAt: 0 });
  return w;
}

/** 等取数据（第一次是 setTimeout 0）和画出来 */
const settle = () => act(async () => await new Promise(r => setTimeout(r, 10)));
const $$ = (sel: string) => [...host.querySelectorAll<HTMLElement>(sel)];

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  server.milestones.mockReset();
  server.milestones.mockResolvedValue(data);
  server.openCardAt.mockReset();
  useData.setState({ ws: ws(), path: 'p/a' });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  setTlBoard(null);
  useData.setState({ ws: null, path: 'p/a' });
});

describe('提交时间轴', () => {
  it('取当前白板所在项目的数据：未发版在最上面，版本从新到旧，右栏是版本索引', async () => {
    await act(async () => root.render(<TimelinePage />));
    await settle();
    expect(server.milestones).toHaveBeenCalledWith('p');
    expect($$('.gt-seg').map(s => s.querySelector('.gt-vname')!.textContent)).toEqual(['未发版', 'v0.2.0', 'v0.1.0']);
    expect($$('.gt-ix').map(b => b.textContent)).toEqual(['未发版', 'v0.2.0', 'v0.1.0']);
    expect(host.querySelector('.gt-sum')!.textContent).toBe('2 个版本 · 3 个提交');
    expect(host.querySelector('.gt-crumb')!.textContent).toContain('全部白板');
  });

  it('连到卡片的提交挂着卡片、点了打开那张卡片；没连卡片的只有提交说明、不能点；找不到的卡片写编号', async () => {
    await act(async () => root.render(<TimelinePage />));
    await settle();
    const items = $$('.gt-seg[data-seg="20"] .gt-item');
    expect(items.map(i => i.classList.contains('bare'))).toEqual([false, true]);
    expect(items[1].querySelector('button')).toBeNull();
    expect(items[1].querySelector('.gt-msg')!.textContent).toBe('chore: 升级依赖');
    const tile = items[0].querySelector<HTMLButtonElement>('.gt-card')!;
    expect(tile.textContent).toContain('修二');
    act(() => tile.click());
    expect(server.openCardAt).toHaveBeenCalledWith('p/a', 2);
    // v0.1.0 的提交连了两张：一张在 b 白板上（写白板名），一张已经不在了
    const v1 = host.querySelector('.gt-seg[data-seg="10"]')!;
    expect(v1.querySelector('.gt-board')!.textContent).toBe('b');
    expect(v1.querySelector('.gt-gone')!.textContent).toBe('a#99');
    // 发了版、连到的卡片还没到终点
    expect(v1.querySelector('.gt-vwarn')!.textContent).toBe('1 张没验收');
  });

  it('未发版的提交：一个提交一栏、挂着卡片；还没有提交的卡片画在提交上面；段头写提交数、卡片数', async () => {
    server.milestones.mockResolvedValue({
      ...data,
      unreleased: [
        { h: 'n2', at: 12.5 * DAY, s: 'feat: 新的', cards: ['a#6'] },
        { h: 'n1', at: 12.5 * DAY - 15 * 60e3, s: 'docs: 说明', cards: [] },
      ],
      cards: [...data.cards, { board: 'p/a', id: 6, title: '推上去了', column: 'review', labels: [], holder: null, createdAt: 0, movedAt: 12 * DAY, group: 'next' }],
    } satisfies Milestones);
    await act(async () => root.render(<TimelinePage />));
    await settle();
    const next = host.querySelector('.gt-seg[data-seg="next"]')!;
    expect(next.querySelector('.gt-vmeta')!.textContent).toMatch(/^2 个提交 · 2 张卡片 · 距上一版/);
    // 先是还没有提交的 #5，再是两个提交（#6 挂在 n2 下面，不再单独列）
    expect([...next.querySelectorAll('.gt-list > li')].map(li => li.querySelector('.gt-sha')?.textContent ?? li.querySelector('.gt-id')?.textContent ?? li.textContent)).toEqual([
      '还没有提交',
      '#5',
      expect.stringContaining('日'),
      'n2',
      'n1',
    ]);
    expect(next.querySelector('.gt-item.solo .gt-title')!.textContent).toBe('在做的');
    expect([...next.querySelectorAll('.gt-item.bare .gt-msg')].map(e => e.textContent)).toEqual(['docs: 说明']);
    expect(host.querySelector('.gt-sum')!.textContent).toBe('2 个版本 · 5 个提交');
  });

  it('只看一块白板：只留连到它的提交，没有提交的版本整段不显示', async () => {
    act(() => setTlBoard('p/b'));
    await act(async () => root.render(<TimelinePage />));
    await settle();
    expect($$('.gt-seg').map(s => s.dataset.seg)).toEqual(['10']);
    expect(host.querySelector('.gt-crumb')!.textContent).toContain('b');
    expect(host.querySelector('.gt-index')).toBeNull();
  });

  it('没有发版记录、也没有在做的卡片：空状态', async () => {
    server.milestones.mockResolvedValue({ versions: [], lastRelease: null, cards: [] });
    await act(async () => root.render(<TimelinePage />));
    await settle();
    expect(host.querySelector('.gt-empty')!.textContent).toContain('还没有发版记录');
    expect(host.querySelector('.gt-sum')).toBeNull();
  });
});
