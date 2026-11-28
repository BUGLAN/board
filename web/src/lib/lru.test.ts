import { describe, expect, it } from 'vitest';
import { Lru } from './lru';

describe('详情缓存的小 LRU', () => {
  it('写入 60 张卡的详情后只剩 50 条，最早写的那 10 张被丢掉', () => {
    const details = new Lru<string, { id: number; activity: unknown[] }>(50);
    for (let id = 1; id <= 60; id++) details.set(`default/main#${id}`, { id, activity: [] });
    expect(details.size).toBe(50);
    expect(details.get('default/main#1')).toBeUndefined();
    expect(details.get('default/main#10')).toBeUndefined();
    expect(details.get('default/main#11')).toEqual({ id: 11, activity: [] });
    expect(details.get('default/main#60')).toEqual({ id: 60, activity: [] });
  });

  it('取过一张会挪到最近使用，下一次超出时先丢更久没碰的', () => {
    const details = new Lru<string, number>(2);
    details.set('a', 1).set('b', 2);
    expect(details.get('a')).toBe(1);
    details.set('c', 3);
    expect(details.get('a')).toBe(1);
    expect(details.get('b')).toBeUndefined();
    expect(details.get('c')).toBe(3);
  });
});
