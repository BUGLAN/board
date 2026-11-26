import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Card } from '../lib/types';
import { useData } from '../store/data';
import { useUI } from '../store/ui';
import { CardView } from './Card';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = new Date(2026, 10, 14, 12).getTime();

const card: Card = {
  id: 2,
  title: '修一下横轴',
  body: '',
  column: 'todo',
  holder: null,
  claimedAt: null,
  beatAt: null,
  runStartedAt: null,
  workMs: 0,
  runs: 0,
  runsLost: 0,
  claimedFrom: null,
  labels: [],
  createdAt: NOW - 3600_000,
  updatedAt: NOW,
  createdBy: { kind: 'human', name: '人' },
  comments: 2,
};

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

const render = (c: Card = card) =>
  act(() =>
    root.render(
      <CardView card={c} labels={[]} filter="" open={false} placeholder={false} entering={false} />,
    ),
  );

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  // 卡片要读 store（保存中、派发、标签筛选），给一个最小的工作区
  useData.setState({ ws: { rev: 1, now: NOW, projects: [] }, path: 'oss/main', cards: {}, saving: {}, pending: {} } as never);
  useUI.setState({ labelSel: [] });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
});

describe('白板上的卡片', () => {
  it('整张卡是「打开详情」的按钮：role="button"，能 Tab 到，可访问名称是编号和标题', () => {
    render();
    const el = host.querySelector('.card')!;
    expect(el.getAttribute('role')).toBe('button');
    expect(el.getAttribute('tabindex')).toBe('0');
    expect(el.getAttribute('aria-label')).toBe('#2 修一下横轴');
  });

  it('评论数有可访问名称，读屏读「2 条评论」；点了和点卡片一样打开详情，不当成按钮', () => {
    render();
    const flag = host.querySelector('.card-flags .f')!;
    expect(flag.tagName).toBe('SPAN');
    expect(flag.getAttribute('role')).toBeNull();
    expect(flag.getAttribute('tabindex')).toBeNull();
    expect(flag.getAttribute('aria-label')).toBe('2 条评论');
    expect(flag.textContent).toBe('2');
    // 悬停提示写明点卡片看评论
    expect(flag.getAttribute('title')).toContain('2 条评论');
    expect(flag.getAttribute('title')).toContain('点卡片看评论');
  });

  it('没有评论时右上角不显示评论数', () => {
    render({ ...card, comments: 0 });
    expect(host.querySelector('.card-flags .f')).toBeNull();
  });
});

describe('卡片上的累计耗时（#665）', () => {
  afterEach(() => vi.restoreAllMocks());

  it('做过 4 次、不在进行中：显示服务端按每次整秒加好的累计，等于动态里 4 次耗时相加', () => {
    // 48 秒 + 1 分 48 秒 + 10 秒 + 11 分 56 秒 = 14 分 42 秒（原始毫秒相加是 14 分 44.7 秒）
    render({ ...card, column: 'review', runs: 4, runsLost: 1, workMs: 882_000, finishedAt: NOW - 60_000 });
    const pill = host.querySelector('.card-claim.worked .run-pill')!;
    expect(pill.textContent).toBe('累计 14 分 42 秒');
    expect(pill.getAttribute('title')).toBe('累计 14 分 42 秒，做过 4 次，其中 1 次异常退出');
  });

  it('进行中：本次按秒走表，累计是已结束的整秒累计加上本次显示的秒数', () => {
    vi.spyOn(Date, 'now').mockReturnValue(NOW);
    render({
      ...card,
      column: 'doing',
      holder: 'mac/opus-5.5-high',
      claimedAt: NOW - 10_000,
      beatAt: NOW - 1_000,
      runStartedAt: NOW - 5_700,
      runs: 5,
      workMs: 882_000,
    });
    expect(host.querySelector('.card-claim .run-pill')!.textContent).toBe('00:05');
    expect(host.querySelector('.card-claim .claim-sub')!.textContent).toBe('累计 14:47 · 第 5 次');
  });
});
