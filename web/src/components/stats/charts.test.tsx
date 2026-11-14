import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, describe, expect, it } from 'vitest';
import { axisLayout, axisNote, axisPlacement, fmtShortDay, PairBars, TrendBars } from './charts';

const HOUR = 3600_000;
const DAY = 24 * HOUR;
const t0 = new Date(2026, 10, 6).getTime(); // 本地时间 11 月 6 日 0 点
const arrayOf = (n: number, f: (i: number) => number) => Array.from({ length: n }, (_, i) => f(i));
const days = (n: number) => arrayOf(n, i => t0 + i * DAY);
const hoursOf = (start: number, n: number) => arrayOf(n, i => start + i * HOUR);
const atMidnight = (ts: number[], i: number) => new Date(ts[i]).getHours() === 0;
const shownIdx = (n: number, W: number, L: number, hour: boolean, ts: number[], edge?: (i: number) => boolean) =>
  axisLayout(n, W, L, hour, ts, edge)
    .show.map((s, i) => (s ? i : -1))
    .filter(i => i >= 0);

/** 按估算的字宽算相邻标签有没有叠上（和 charts.tsx 同一个口径） */
const overlaps = (ts: number[], W: number, L: number, hour: boolean, edge?: (i: number) => boolean) => {
  const fs = W <= 420 ? 9 : 10.5;
  const px = (s: string) => s.replace(/[^\x00-\xff]/g, 'xx').length * 0.6 * fs;
  const ps = axisPlacement(ts, W, L, hour, edge);
  let bad = 0;
  for (let i = 1; i < ps.length; i++) if (ps[i].x < ps[i - 1].x + px(ps[i - 1].text) - 0.5) bad++;
  return bad;
};

describe('图表横轴：按宽度挑要标日期的桶', () => {
  it('390px 手机、7 天：7 个「M/D」都放得下', () => {
    expect(shownIdx(7, 322, 30, false, days(7))).toEqual([0, 1, 2, 3, 4, 5, 6]);
  });

  it('首尾两个桶一定显示，并且贴左边、贴右边', () => {
    for (const n of [7, 30]) {
      const ax = axisLayout(n, 322, 30, false, days(n));
      expect(ax.show[0]).toBe(true);
      expect(ax.label[0]!.anchor).toBe('start');
      expect(ax.show[n - 1]).toBe(true);
      expect(ax.label[n - 1]!.anchor).toBe('end');
    }
  });

  it('容器越窄，中间的标签越少（只减不增）', () => {
    const wide = shownIdx(30, 960, 30, false, days(30)).length;
    const mid = shownIdx(30, 640, 30, false, days(30)).length;
    const narrow = shownIdx(30, 322, 30, false, days(30)).length;
    expect(wide).toBeGreaterThanOrEqual(mid);
    expect(mid).toBeGreaterThanOrEqual(narrow);
    expect(narrow).toBeGreaterThanOrEqual(2);
  });

  it('7 天按天在 390px 下不叠；30 天在 640px 下也不叠', () => {
    expect(overlaps(days(7), 322, 30, false)).toBe(0);
    expect(overlaps(days(30), 640, 30, false)).toBe(0);
  });

  it('按小时：整天那一格写长日期，它左右两个整点让开，48 小时在 390px 下不叠', () => {
    const twoDays = hoursOf(t0 + 12 * HOUR, 48); // 中午到再下一天中午
    // 第 12 格是第二天 0 点：整天那一格一定标
    expect(shownIdx(48, 322, 30, true, twoDays, i => atMidnight(twoDays, i))).toContain(12);
    expect(overlaps(twoDays, 322, 30, true, i => atMidnight(twoDays, i))).toBe(0);
    const labels = axisPlacement(twoDays, 322, 30, true, i => atMidnight(twoDays, i));
    const dayLabel = labels.find(p => /月/.test(p.text))!;
    expect(dayLabel).toBeDefined();
    expect(labels.filter(p => p !== dayLabel && p.i === dayLabel.i - 1)).toHaveLength(0);
    expect(labels.filter(p => p !== dayLabel && p.i === dayLabel.i + 1)).toHaveLength(0);
  });
});

describe('横轴说明和短日期', () => {
  it('只标了首尾两天时写出是哪两天；标得下时什么都不写', () => {
    // 30 天挤在 90px 里，中间一个日子都放不下（只有首尾两个）：首尾是哪两天要写出来
    const note = axisNote(days(30), 90, 30);

    expect(note).toContain('11/6');
    expect(note).toContain('12/5');
    // 7 天标得下：不写；只有两天：不写
    expect(axisNote(days(7), 322, 30)).toBe('');
    expect(axisNote(days(2), 300, 30)).toBe('');
    // 30 天在 300px 下隔几天标一个、900px 下隔一天标一个，都不用这句
    expect(axisNote(days(30), 300, 30)).toBe('');
    expect(axisNote(days(30), 900, 30)).toBe('');
  });

  it('短格式是 M/D', () => {
    expect(fmtShortDay(t0)).toBe('11/6');
    expect(fmtShortDay(new Date(2026, 11, 31, 12).getTime())).toBe('12/31');
  });
});

/* ---------- 画出来的图：7 天每根柱子下面都写「M/D」，首尾贴边 ---------- */

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
class RO {
  observe() {}
  disconnect() {}
}
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = RO;

const dayTs = (n: number) => arrayOf(n, i => t0 + i * DAY);
const hours = hoursOf(t0 + 12 * HOUR, 48).map((t, i) => ({ t, v: i % 6, created: i % 4, done: i % 3 }));

function mount(node: React.ReactElement) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  act(() => createRoot(host).render(node));
  return host;
}

afterEach(() => {
  document.body.innerHTML = '';
});

const axisTexts = (host: HTMLElement) => [...host.querySelectorAll('text.sp-axis')].filter(t => !/^\d+$/.test(t.textContent!.trim()));

describe('积压净流量 / 趋势的横轴', () => {
  it('7 天：每根柱子下面都写「M/D」，首尾贴边、中间居中', () => {
    const labels = axisTexts(mount(<PairBars data={dayTs(7).map(t => ({ t, created: 1, done: 0 }))} bucket="day" />));
    expect(labels.map(t => t.textContent)).toEqual(['11/6', '11/7', '11/8', '11/9', '11/10', '11/11', '11/12']);
    expect(labels[0].getAttribute('text-anchor')).toBe('start');
    expect(labels[labels.length - 1].getAttribute('text-anchor')).toBe('end');
    expect(labels[3].getAttribute('text-anchor')).toBe('middle');
  });

  it('趋势图按天也写短日期；按小时写「11月7日」和整点，且互不重叠', () => {
    const day = mount(<TrendBars data={dayTs(7).map(t => ({ t, v: 1 }))} bucket="day" label="完成" fmt={String} />);
    const dayText = axisTexts(day).map(t => t.textContent);
    expect(dayText).toContain('11/6');
    expect(dayText.some(t => /月/.test(t!))).toBe(false);

    const hour = mount(<TrendBars data={hours} bucket="hour" label="完成" fmt={String} />);
    const hourText = axisTexts(hour).map(t => t.textContent);
    expect(hourText).toContain('11月7日');
    expect(hourText.length).toBeLessThan(12);
    const boxes = axisTexts(hour).map(t => t.getBoundingClientRect());
    for (let i = 1; i < boxes.length; i++) expect(boxes[i].left).toBeGreaterThanOrEqual(boxes[i - 1].right - 0.5);
  });

  it('标得下整排时不另加横轴说明', () => {
    const notes: string[] = [];
    mount(<PairBars data={dayTs(7).map(t => ({ t, created: 1, done: 0 }))} bucket="day" onAxisNote={n => notes.push(n)} />);
    expect(notes.every(n => n === '')).toBe(true);
  });
});
