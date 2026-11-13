import { describe, expect, it } from 'vitest';
import { sidebarLabels } from './sidebarLabels';
import type { Label } from './types';

const label = (id: number, name: string): Label => ({ id, name, color: '#888', uses: 0 });
const names = (list: Label[]) => list.map(l => l.name);

describe('侧栏标签：不列版本标签', () => {
  it('版本标签剔除，其余按原顺序、原数量返回', () => {
    const all = ['claude', 'dsh', 'v0.1.24', '定时', 'v0.1.0', '优先'].map((n, i) => label(i + 1, n));
    expect(names(sidebarLabels(all))).toEqual(['claude', 'dsh', '定时', '优先']);
  });

  it('没有版本标签时原样返回（同一批对象）', () => {
    const all = ['claude', 'dsh', '定时'].map((n, i) => label(i + 1, n));
    expect(sidebarLabels(all)).toEqual(all);
    expect(sidebarLabels([])).toEqual([]);
  });

  it('大小写、v 后面是不是数字', () => {
    const all = ['V0.2.0', 'v1', 'vip', '版本', 'v', '0.1', 'visual'].map((n, i) => label(i + 1, n));
    expect(names(sidebarLabels(all))).toEqual(['vip', '版本', 'v', '0.1', 'visual']);
  });

  it('正选中筛选的版本标签仍然列出（点得到取消），未选中的照旧隐藏', () => {
    const all = [label(1, 'claude'), label(2, 'v0.1.23'), label(3, 'v0.1.22')];
    expect(names(sidebarLabels(all, [2]))).toEqual(['claude', 'v0.1.23']);
    expect(names(sidebarLabels(all, [1]))).toEqual(['claude']);
    expect(names(sidebarLabels(all, [2, 3]))).toEqual(['claude', 'v0.1.23', 'v0.1.22']);
  });

  it('重复执行结果相同', () => {
    const all = [label(1, 'claude'), label(2, 'v0.1.23')];
    expect(names(sidebarLabels(sidebarLabels(all)))).toEqual(names(sidebarLabels(all)));
  });
});
