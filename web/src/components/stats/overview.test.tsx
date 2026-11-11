import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { ProjectInfo, Stats, Workspace } from '../../lib/types';
import { useData } from '../../store/data';
import { BoardOverview } from './Stats';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/* jsdom 没有 ResizeObserver：图表用它量宽度，这里给一个什么都不做的 */
class RO {
  observe() {}
  disconnect() {}
}
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = RO;

const DAY = 24 * 3600_000;
const now = new Date('2026-05-20T12:00:00').getTime();

const project: ProjectInfo = {
  key: 'default',
  name: '默认项目',
  desc: '一块给人看、也给 agent 干活的看板',
  color: '#FF6A33',
  pos: { x: 0, y: 0 },
  createdAt: now - 30 * DAY,
  labels: [{ id: 1, name: 'UI', color: '#4DB5FF', uses: 9 }],
  boards: [
    {
      key: 'main',
      path: 'default/main',
      name: '主白板',
      desc: '一块给人看、也给 agent 干活的看板',
      readme: '# 说明\n\n这段说明文字不该出现在大盘里。',
      pos: { x: 0, y: 0 },
      createdAt: now - 10 * DAY,
      updatedAt: now,
      counts: { todo: 2 },
      statuses: [],
      holders: [],
    },
  ],
};

const ws: Workspace = { rev: 1, now, projects: [project] };

const days = Array.from({ length: 26 * 7 }, (_, i) => ({ t: now - (26 * 7 - 1 - i) * DAY, v: i % 5 === 0 ? 3 : 0, future: false }));

const stats: Stats = {
  scope: 'board',
  path: 'default/main',
  from: now - 7 * DAY,
  to: now,
  now,
  tz: 'UTC',
  bucket: 'day',
  boardsInScope: ['default/main'],
  activity: 12,
  cur: {} as Stats['cur'],
  prev: {} as Stats['prev'],
  series: [],
  idle: [],
  agents: [],
  dwell: [],
  hourHeat: Array.from({ length: 7 }, () => Array.from({ length: 24 }, () => 0)),
  dayHeat: days,
  runs: [],
  lives: [],
  present: {
    statuses: [
      { key: 'todo', name: '待办', color: 'gray', n: 2 },
      { key: 'done', name: '已完成', color: 'green', n: 5 },
    ],
    review: { n: 0, oldest: null } as unknown as Stats['present']['review'],
    todo: { n: 2, oldest: null } as unknown as Stats['present']['todo'],
    doing: { n: 0, holders: [] },
    stuck: [],
  },
  life: { since: now - 10 * DAY, days: 10, cards: 7, finals: 5, reworks: 1, agents: 2, spent: 3 * 3600_000, perDay: 0.5 },
  boards: [],
  projects: [],
  all: {} as Stats['all'],
};

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({ ws, path: 'default/main' });
  act(() => root.render(<BoardOverview s={stats} />));
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('大盘的白板概览', () => {
  it('不再显示「白板说明」整块（标题、编辑按钮、正文都没有）', () => {
    expect(host.querySelector('.sp-ov')!.textContent).not.toContain('白板说明');
    expect(host.querySelector('.sp-ov')!.textContent).not.toContain('这段说明文字不该出现在大盘里');
    expect(host.querySelector('.sp-ov-readme')).toBeNull();
    expect(host.querySelector('.sp-ov-blank')).toBeNull();
    // 头部信息还在
    expect(host.querySelector('.sp-ov-title h2')!.textContent).toBe('主白板');
    expect(host.querySelector('.sp-ov-meta')!.textContent).toContain('default/main');
  });

  it('日历热力图在概览里，占住原「白板说明」的位置', () => {
    const cal = host.querySelector('.sp-ov-cal .sp-cal');
    expect(cal).not.toBeNull();
    expect(host.querySelector('#sp-cal h2')!.textContent).toBe('日历热力图');
    // 概览里只有一张热力图（不会和「这段时间」里的重复）
    expect(host.querySelectorAll('.sp-cal')).toHaveLength(1);
  });

  it('日历热力图没有标当前时间范围的粗边框', () => {
    expect(host.querySelectorAll('.sp-cal .sp-cell.in')).toHaveLength(0);
    expect(host.querySelectorAll('.sp-cal .sp-cell').length).toBeGreaterThan(0);
  });

  it('「创建以来」「现在卡片在哪」「标签」「同项目的其他白板」都还在', () => {
    const text = host.querySelector('.sp-ov-facts')!.textContent!;
    expect(text).toContain('创建以来');
    expect(text).toContain('现在卡片在哪');
    expect(text).toContain('待办');
    expect(text).toContain('已完成');
    expect(text).toContain('标签');
    // 日历热力图在「创建以来」上面：文档顺序上先出现
    const html = host.querySelector('.sp-ov')!.innerHTML;
    expect(html.indexOf('sp-ov-cal')).toBeLessThan(html.indexOf('sp-ov-facts'));
  });
});
