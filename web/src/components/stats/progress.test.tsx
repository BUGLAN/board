import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, describe, expect, it } from 'vitest';
import type { StatsProgress } from '../../lib/types';
import { bigVersion, ProgressChart, ProgressLegend, yTicks, type ProgressVersion } from './progress';

/* jsdom 没有 ResizeObserver：图表用它量宽度，这里给一个什么都不做的 */
class RO {
  observe() {}
  disconnect() {}
}
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = RO;

const HOUR = 3600_000;
const t0 = new Date(2026, 10, 6, 9).getTime(); // 本地时间 11 月 6 日 9 点
const t = Array.from({ length: 12 }, (_, i) => t0 + i * 2 * HOUR);

/** 未整理、待办、进行中（一直是 0）、已完成，每 2 小时一个点 */
const p: StatsProgress = {
  since: t0,
  t,
  statuses: [
    { key: 'inbox', name: '未整理', color: 'inbox', final: false },
    { key: 'todo', name: '待办', color: 'todo', final: false },
    { key: 'doing', name: '进行中', color: 'doing', final: false },
    { key: 'done', name: '已完成', color: 'done', final: true },
  ],
  n: [
    [1, 1, 2, 2, 1, 1, 0, 0, 1, 1, 1, 2],
    [0, 1, 1, 2, 2, 1, 1, 1, 0, 0, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 2, 3, 3, 4, 5, 5, 6],
  ],
  versions: [],
};

const versions: ProgressVersion[] = [
  { name: 'v0.1.0', at: t[4] + 10 * 60_000, cards: 1 },
  { name: 'v0.1.1', at: t[7], cards: 2 },
  { name: 'v0.2.0', at: t[9] + 30 * 60_000, cards: 3 },
];

function mount(node: React.ReactElement) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  act(() => createRoot(host).render(node));
  return host;
}

/** jsdom 里 svg 的位置是 (0, 0)、宽度取默认 600：横坐标和图里一样算 */
const W = 600, L = 44, R = 14, HEIGHT = 276, B = 42;
const xOf = (v: number) => L + ((v - t[0]) / (t[t.length - 1] - t[0])) * (W - L - R);
const hover = (host: HTMLElement, clientX: number, clientY: number) =>
  act(() => {
    host.querySelector('svg')!.dispatchEvent(new MouseEvent('pointermove', { bubbles: true, clientX, clientY }));
  });
const tipLines = (host: HTMLElement) => (host.querySelector('.pg-tip') as HTMLElement | null)?.textContent ?? '';

afterEach(() => {
  document.body.innerHTML = '';
});

describe('进步曲线', () => {
  it('竖轴大约 4 格，刻度都是整张', () => {
    expect(yTicks(544)).toEqual([0, 200, 400, 600]);
    expect(yTicks(7)).toEqual([0, 2, 4, 6, 8]);
    expect(yTicks(9).every(Number.isInteger)).toBe(true);
    expect(yTicks(90)).toEqual([0, 25, 50, 75, 100]);
    expect(yTicks(0)).toEqual([0, 1]);
  });

  it('x.y.0 是大版本，补丁版本不是', () => {
    expect(['v0.1.0', 'V2.0.0', 'v1.0', 'v0.2.0-rc'].map(bigVersion)).toEqual([true, true, true, true]);
    expect(['v0.1.61', 'v1.2.3', '发布'].map(bigVersion)).toEqual([false, false, false]);
  });

  it('终点状态垫在最下面，一直是 0 的状态不画、图例里也没有；只描完成线和总数线', () => {
    const host = mount(
      <>
        <ProgressChart p={p} />
        <ProgressLegend p={p} />
      </>,
    );
    expect(host.querySelectorAll('path.pg-band')).toHaveLength(3);
    // 第一层（最下面）是已完成：它的上沿只到 6 张，比最上面的总数低
    const edges = [...host.querySelectorAll('path.pg-edge')];
    expect(edges).toHaveLength(2);
    expect(edges[0].classList.contains('final')).toBe(true);
    expect(host.querySelector('.pg-legend')!.textContent).toBe('未整理待办已完成');
  });

  it('悬停某个时刻：从上往下列出各状态的张数和总数，右上角是那时已经发了的版本', () => {
    const host = mount(<ProgressChart p={p} versions={versions} />);
    hover(host, xOf(t[11]), 60);
    expect(tipLines(host)).toMatch(/现在/);
    expect(tipLines(host)).toContain('v0.2.0');
    expect(tipLines(host)).toContain('未整理2待办1已完成6共9');
    // 第一个版本发之前没有版本标签
    hover(host, xOf(t[2]), 60);
    expect(tipLines(host)).not.toMatch(/v0\./);
    expect(tipLines(host)).toContain('未整理2待办1已完成0共3');
  });

  it('横轴下面一行是每次发版：大版本刻度更长、画一条竖线写版本号；悬停刻度看这一版', () => {
    const host = mount(
      <>
        <ProgressChart p={p} versions={versions} />
        <ProgressLegend p={p} versions={versions.length} />
      </>,
    );
    expect(host.querySelectorAll('.pg-ver')).toHaveLength(3);
    expect(host.querySelectorAll('.pg-ver.big')).toHaveLength(2);
    expect([...host.querySelectorAll('.pg-major text')].map(x => x.textContent)).toEqual(['v0.1.0', 'v0.2.0']);
    expect(host.querySelector('.pg-legend')!.textContent).toContain('发版 3 次');

    hover(host, xOf(versions[1].at) + 3, HEIGHT - B + 10);
    expect(host.querySelector('.pg-ver.on')).not.toBeNull();
    expect(tipLines(host)).toContain('v0.1.1');
    expect(tipLines(host)).toContain('这一版2 张卡');
    expect(tipLines(host)).toContain('发版时已完成3');
    // 离哪个刻度都远：什么都不显示
    hover(host, xOf(t[1]), HEIGHT - B + 10);
    expect(host.querySelector('.pg-tip')).toBeNull();
  });

  it('悬停全部项目的发版时写出是哪个项目的', () => {
    const host = mount(<ProgressChart p={p} versions={[{ ...versions[0], project: 'Board 开源仓库' }]} />);
    hover(host, xOf(versions[0].at), HEIGHT - B + 10);
    expect(tipLines(host)).toContain('Board 开源仓库');
  });
});
