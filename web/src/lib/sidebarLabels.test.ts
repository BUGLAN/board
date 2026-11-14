import { describe, expect, it } from 'vitest';
import { DEFAULT_HIDDEN, HIDDEN_LEN, HIDDEN_MAX, hiddenBy, hiddenRules, ruleError, sidebarLabels, visibleLabels } from './sidebarLabels';
import type { Label } from './types';

const label = (id: number, name: string): Label => ({ id, name, color: '#888', uses: 0 });
const names = (list: Label[]) => list.map(l => l.name);

describe('侧栏标签：默认不列版本标签', () => {
  it('版本标签剔除，其余按原顺序、原数量返回', () => {
    const all = ['claude', 'dsh', 'v0.1.24', '定时', 'v0.1.0', '优先'].map((n, i) => label(i + 1, n));
    expect(names(sidebarLabels(all))).toEqual(['claude', 'dsh', '定时', '优先']);
  });

  it('没有版本标签时原样返回（同一批对象）', () => {
    const all = ['claude', 'dsh', '定时'].map((n, i) => label(i + 1, n));
    expect(sidebarLabels(all)).toBe(all);
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

describe('隐藏规则（default/main#324）', () => {
  const all = ['claude', 'v0.1.25', 'tmp-a', 'TMP-b', '定时'].map((n, i) => label(i + 1, n));

  it('按项目的规则隐藏，不区分大小写；规则为空时全部显示', () => {
    expect(names(visibleLabels(all, ['^tmp-']))).toEqual(['claude', 'v0.1.25', '定时']);
    expect(names(visibleLabels(all, ['^tmp-', ...DEFAULT_HIDDEN]))).toEqual(['claude', '定时']);
    expect(visibleLabels(all, [])).toBe(all);
  });

  it('写错的、空的规则不生效，不影响其他规则', () => {
    expect(names(visibleLabels(all, ['(', '  ', '^tmp-']))).toEqual(['claude', 'v0.1.25', '定时']);
    expect(visibleLabels(all, ['['])).toBe(all);
  });

  it('改名后按新名字重新匹配', () => {
    expect(hiddenBy('v0.1.25', DEFAULT_HIDDEN)).toBe('^v\\d');
    expect(hiddenBy('release-0.1.25', DEFAULT_HIDDEN)).toBeNull();
    expect(hiddenBy('tmp-x', ['^v\\d', '^tmp-'])).toBe('^tmp-');
  });

  it('旧服务端没有 hiddenLabels 时按默认；有（哪怕是空的）就用它', () => {
    expect(hiddenRules(undefined)).toEqual(DEFAULT_HIDDEN);
    expect(hiddenRules({})).toEqual(DEFAULT_HIDDEN);
    expect(hiddenRules({ hiddenLabels: [] })).toEqual([]);
    expect(hiddenRules({ hiddenLabels: ['^x'] })).toEqual(['^x']);
  });

  it('ruleError：空、太长、重复、超过条数、不是正则', () => {
    expect(ruleError('^tmp-')).toBeNull();
    expect(ruleError('  ')).toMatch('不能为空');
    expect(ruleError('a'.repeat(HIDDEN_LEN + 1))).toMatch(`${HIDDEN_LEN}`);
    expect(ruleError(' ^v\\d ', DEFAULT_HIDDEN)).toMatch('已经有');
    expect(ruleError('^z', Array.from({ length: HIDDEN_MAX }, (_, i) => `^x${i}`))).toMatch(`${HIDDEN_MAX}`);
    expect(ruleError('(')).toMatch('不是有效的正则');
  });
});
