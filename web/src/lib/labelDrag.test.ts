import { describe, expect, it } from 'vitest';
import { moveLabel } from './labelDrag';

describe('标签拖动顺序', () => {
  it('隐藏标签留在原来的位置：拖到某个看得见的标签之前时，按全部标签的 ID 换算', () => {
    // [a, v1(隐藏), b, c] 里把 c 拖到 a 前
    expect(moveLabel([1, 2, 3, 4], [1, 3, 4], 4, 0)).toEqual([4, 1, 2, 3]);
  });

  it('拖到末尾时落在最后一个看得见的标签之后，不会把隐藏标签挤到后面', () => {
    // [a, v1(隐藏), b] 里把 a 拖到 b 后
    expect(moveLabel([1, 2, 3], [1, 3], 1, 2)).toEqual([2, 3, 1]);
  });

  it('位置没变、目标不在了或原数组不含这个标签时不改', () => {
    expect(moveLabel([1, 2, 3], [1, 3], 1, 1)).toBeNull();
    expect(moveLabel([1, 2, 3], [1, 3, 9], 1, 2)).toBeNull();
    expect(moveLabel([1, 2, 3], [2, 3, 9], 9, 0)).toBeNull();
  });
});
