import { act, Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Card } from '../lib/types';
import { useData } from '../store/data';
import { CardDetail } from './CardDetail';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/* 卡片详情的计时（default/main#698）：关掉之后（留着上一张卡给关闭动画用）不再每秒重渲染；
   没人持有的卡只有「N 分钟前」，最多 30 秒更新一次；有人在做时耗时照常每秒走 */

const NOW = new Date(2026, 10, 14, 12).getTime();

const card = (holder: string | null): Card => ({
  id: 2,
  title: '修一下横轴',
  body: '',
  column: holder ? 'doing' : 'todo',
  holder,
  claimedAt: holder ? NOW - 60_000 : null,
  beatAt: holder ? NOW : null,
  runStartedAt: holder ? NOW - 50_000 : null,
  workMs: 0,
  runs: holder ? 1 : 0,
  runsLost: 0,
  claimedFrom: null,
  labels: [],
  createdAt: NOW - 3600_000,
  updatedAt: NOW,
  createdBy: { kind: 'human', name: '人' },
  comments: 0,
});

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let commits = 0;

const open = (c: Card) =>
  useData.setState({
    ws: { rev: 1, now: NOW, projects: [{ key: 'oss', name: '开源仓库', boards: [{ key: 'main', path: 'oss/main', name: '主线', statuses: [] }] }] },
    path: 'oss/main',
    board: { cards: { 2: c }, cols: { [c.column]: [2] } },
    detailId: 2,
    detail: { ...c, activity: [{ id: 1, kind: 'comment', at: NOW - 120_000, actor: { kind: 'human', name: '人' }, text: '先看看' }] },
    detailError: null,
  } as never);

/** 往前走 ms 毫秒，一秒一步，返回其中有几秒详情重渲染过（commit 过）。按秒数而不是 commit 次数：同一次计时触发的更新
 *  在 jsdom 里可能拆成前后两次 commit，用户看到的还是这一秒更新了一次 */
const during = (ms: number) => {
  let busy = 0;
  for (let t = 0; t < ms; t += 1000) {
    const before = commits;
    act(() => vi.advanceTimersByTime(Math.min(1000, ms - t)));
    if (commits > before) busy++;
  }
  return busy;
};

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  vi.stubGlobal('matchMedia', (q: string) => ({ matches: false, media: q, onchange: null, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false }));
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  commits = 0;
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const render = () =>
  act(() =>
    root.render(
      <Profiler id="detail" onRender={() => commits++}>
        <CardDetail />
      </Profiler>,
    ),
  );

describe('卡片详情的计时', () => {
  it('没人持有的卡：最多每 30 秒重渲染一次；关掉后不再重渲染', () => {
    open(card(null));
    render();
    expect(host.querySelector('.drawer.on')).not.toBeNull();
    // 打开后的那一下（effect 里补的状态）先落定，再数
    during(1000);
    expect(during(60_000)).toBeLessThanOrEqual(2);
    expect(during(120_000)).toBeLessThanOrEqual(4);
    act(() => useData.setState({ detailId: null } as never));
    expect(host.querySelector('.drawer.on')).toBeNull();
    expect(host.querySelector('.drawer [data-part="activity"]'), '关闭动画期间还留着上一张卡').not.toBeNull();
    expect(during(10_000)).toBe(0);
    expect(during(60_000)).toBe(0);
  });

  it('有人在做的卡：耗时每秒走；关掉后停', () => {
    open(card('mbp/opus-5.5'));
    render();
    expect(during(5_000)).toBeGreaterThanOrEqual(4);
    act(() => useData.setState({ detailId: null } as never));
    expect(during(10_000)).toBe(0);
  });
});
