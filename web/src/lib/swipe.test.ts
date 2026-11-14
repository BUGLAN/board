import { describe, expect, it } from 'vitest';
import { swipeCommits, swipeOffset, swipeThreshold } from './swipe';

describe('行的横滑（default/main#303）', () => {
  it('阈值是行宽的 35%，夹在 72–160px', () => {
    expect(swipeThreshold(100)).toBe(72);
    expect(swipeThreshold(300)).toBe(105);
    expect(swipeThreshold(1000)).toBe(160);
  });
  it('只往指定方向动，反方向不动；过了阈值只跟一半多', () => {
    expect(swipeOffset(-50, 1, 300)).toBe(0);
    expect(swipeOffset(50, -1, 300)).toBe(0);
    expect(swipeOffset(80, 1, 300)).toBe(80);
    expect(swipeOffset(-80, -1, 300)).toBe(-80);
    expect(swipeOffset(205, 1, 300)).toBeCloseTo(105 + 100 * 0.55);
    expect(swipeOffset(-205, -1, 300)).toBeCloseTo(-(105 + 100 * 0.55));
  });
  it('松手：过了阈值算数；没到阈值但甩得快、距离不太短也算；反方向不算', () => {
    expect(swipeCommits(110, 0, 1, 300)).toBe(true);
    expect(swipeCommits(90, 0.1, 1, 300)).toBe(false);
    expect(swipeCommits(50, 0.9, 1, 300)).toBe(true);
    expect(swipeCommits(30, 2, 1, 300)).toBe(false);
    expect(swipeCommits(-110, 0, -1, 300)).toBe(true);
    expect(swipeCommits(-50, -0.9, -1, 300)).toBe(true);
    expect(swipeCommits(110, 0, -1, 300)).toBe(false);
  });
});
