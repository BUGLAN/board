import { describe, expect, it } from 'vitest';
import { placeTip } from './tooltip';

const anchor = (left: number, top: number, width = 40, height = 20) => ({
  left,
  top,
  right: left + width,
  bottom: top + height,
  width,
  height,
});

const viewport = { width: 1024, height: 768 };

describe('提示浮层的位置', () => {
  it('默认放在触发器下方、水平居中；靠近底部时翻到上方', () => {
    expect(placeTip(anchor(100, 100), { width: 80, height: 28 }, viewport)).toEqual({
      left: 80,
      top: 128,
      side: 'bottom',
    });
    expect(placeTip(anchor(100, 740), { width: 80, height: 28 }, viewport)).toEqual({
      left: 80,
      top: 704,
      side: 'top',
    });
  });

  it('data-tip-pos="top" 优先上方，上方放不下就翻到下方', () => {
    expect(placeTip(anchor(100, 100), { width: 80, height: 28 }, viewport, { pos: 'top' })).toEqual({
      left: 80,
      top: 64,
      side: 'top',
    });
    expect(placeTip(anchor(100, 10), { width: 80, height: 28 }, viewport, { pos: 'top' })).toEqual({
      left: 80,
      top: 38,
      side: 'bottom',
    });
  });

  it('end 对齐右边缘，靠近两侧时夹在视口里', () => {
    expect(placeTip(anchor(1000, 100), { width: 80, height: 28 }, viewport, { align: 'end' })).toEqual({
      left: 936,
      top: 128,
      side: 'bottom',
    });
    expect(placeTip(anchor(0, 100), { width: 80, height: 28 }, viewport, { align: 'end' })).toEqual({
      left: 8,
      top: 128,
      side: 'bottom',
    });
  });

  it('右侧放不下时翻到左侧，垂直方向也夹在视口里', () => {
    expect(placeTip(anchor(900, 100), { width: 100, height: 28 }, viewport, { pos: 'right' })).toEqual({
      left: 790,
      top: 96,
      side: 'left',
    });
    expect(placeTip(anchor(20, 2), { width: 100, height: 28 }, viewport, { pos: 'right', align: 'end' })).toEqual({
      left: 70,
      top: 8,
      side: 'right',
    });
  });
});
