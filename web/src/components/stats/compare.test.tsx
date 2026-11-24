import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { ProjectInfo, Stats, StatsBrief, Workspace } from '../../lib/types';
import { useData } from '../../store/data';
import { AllOverview } from './Stats';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class RO {
  observe() {}
  disconnect() {}
}
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = RO;

const now = new Date('2026-05-20T12:00:00').getTime();

const project = (key: string, name: string, color: string): ProjectInfo => ({
  key,
  name,
  desc: `${name}说明`,
  color,
  pos: { x: 0, y: 0 },
  createdAt: now - 30 * 24 * 3600_000,
  labels: [],
  boards: [
    {
      key: 'main',
      path: `${key}/main`,
      name: '主白板',
      desc: '',
      readme: '',
      pos: { x: 0, y: 0 },
      createdAt: now - 10 * 24 * 3600_000,
      updatedAt: now,
      counts: {},
      statuses: [],
      holders: [],
    },
  ],
});

const ws: Workspace = {
  rev: 1,
  now,
  projects: [project('default', '默认项目', '#FF6A33'), project('docs', '文档站', '#3B82F6')],
};

const brief = (key: string, done: number): StatsBrief => ({
  key,
  review: 1,
  todo: 2,
  busy: false,
  done,
  series: [0, 1, done],
  leadP50: 90 * 60_000,
  firstPass: 1,
  rework: 0,
});

const stats: Stats = {
  scope: 'all',
  path: null,
  from: now - 7 * 24 * 3600_000,
  to: now,
  now,
  tz: 'Asia/Shanghai',
  bucket: 'day',
  activity: 8,
  cur: {} as Stats['cur'],
  prev: {} as Stats['prev'],
  series: [],
  idle: [],
  agents: [],
  dwell: [],
  hourHeat: [],
  dayHeat: [],
  runs: [],
  lives: [],
  present: {
    statuses: [],
    review: { n: 0, oldest: null },
    todo: { n: 0, oldest: null },
    doing: { n: 0, holders: [] },
    stuck: [],
  },
  life: null,
  boards: [],
  projects: [brief('default', 12), brief('docs', 3)],
  all: brief('', 15),
};

const statsCss = readFileSync(resolve(__dirname, 'stats.css'), 'utf8');

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({ ws, path: 'default/main' });
  act(() => root.render(<AllOverview s={stats} />));
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('大盘全部项目的对比表', () => {
  it('列顺序不变，数字列表头都走居中样式', () => {
    const heads = [...host.querySelectorAll<HTMLTableCellElement>('thead th')];
    expect(heads.map(th => th.textContent?.trim())).toEqual([
      '项目',
      '待办',
      '待复盘',
      '这段时间完成',
      '交付周期 · 中位',
      '一次通过率',
      '返工',
    ]);
    expect(heads.slice(1).every(th => th.classList.contains('num'))).toBe(true);
  });

  it('头像和项目文字分开，完成列的计数与趋势图仍在同一格', () => {
    const row = host.querySelector<HTMLTableRowElement>('tbody tr')!;
    const name = row.querySelector('.sp-cmp-name')!;
    const avatar = name.querySelector('.sp-cmp-av')!;
    const meta = name.querySelector('.sp-cmp-meta')!;
    expect([...name.children]).toEqual([avatar, meta]);
    expect(avatar.textContent).toBe('默');
    expect(meta.querySelector('b')!.textContent).toBe('默认项目');
    expect(meta.querySelector('small')!.textContent).toBe('1 块白板 · 默认项目说明');

    const done = row.querySelectorAll<HTMLTableCellElement>('td')[3];
    expect(done.classList.contains('num')).toBe(true);
    expect(done.querySelector('.sp-cmp-done b')!.textContent).toBe('12');
    expect(done.querySelector('.sp-cmp-done .sp-spark')).not.toBeNull();
  });

  it('完成列用行内 flex，让单元格的居中样式同时居中数字和趋势图', () => {
    const rule = /\.sp-cmp-done \{[^}]*\}/.exec(statsCss)?.[0] ?? '';
    expect(rule).toMatch(/display:\s*inline-flex/);
    expect(rule).toMatch(/width:\s*124px/);
    expect(statsCss).toMatch(/\.sp-cmp \.num \{\s*text-align:\s*center;/);
  });
});
