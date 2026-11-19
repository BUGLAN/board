import { describe, expect, it } from 'vitest';
import { trashHit } from './Trash';

describe('回收站页的筛选（#486）', () => {
  const t = { id: 42, title: '登录页按钮圆角', body: '设置页也一样\n第二行' };
  it('空的筛选全都显示', () => expect(trashHit(t, '')).toBe(true));
  it('#编号按前缀', () => {
    expect(trashHit(t, '#4')).toBe(true);
    expect(trashHit(t, '#5')).toBe(false);
  });
  it('标题、描述里有就算；编号整个相等也算', () => {
    expect(trashHit(t, '圆角')).toBe(true);
    expect(trashHit(t, '第二行')).toBe(true);
    expect(trashHit(t, '42')).toBe(true);
    expect(trashHit(t, '注册')).toBe(false);
  });
});
