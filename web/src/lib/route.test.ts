/* 地址栏的解析和拼装（纯函数，单元测试见 route.test.ts）：
   /<项目>/<白板> 打开这块白板；/<项目>/<白板>/<ID> 再打开这张卡片的详情；其他 Tab 沿用 ?tab= */
import { describe, expect, it } from 'vitest';
import { parseRoute, routeUrl } from './route';

describe('parseRoute', () => {
  it('白板和卡片', () => {
    expect(parseRoute('/default/main')).toEqual({ path: 'default/main', card: null, tab: null });
    expect(parseRoute('/default/main/12')).toEqual({ path: 'default/main', card: 12, tab: null });
  });

  it('其他 Tab 带 ?tab=，也带着白板', () => {
    expect(parseRoute('/default/main', '?tab=inbox')).toEqual({ path: 'default/main', card: null, tab: 'inbox' });
    expect(parseRoute('/default/main/3', '?tab=stats')).toEqual({ path: 'default/main', card: 3, tab: 'stats' });
  });

  it('根路径、认不出的 Tab：没有白板', () => {
    expect(parseRoute('/')).toEqual({ path: null, card: null, tab: null });
    expect(parseRoute('/', '?tab=nope')).toEqual({ path: null, card: null, tab: null });
  });

  it('不像白板路径的地址不当成白板（服务端任何路径都返回 index.html）', () => {
    for (const p of ['/login', '/a/b/c/d', '/Default/main', '/default/main/abc', '/default/main/0', '/default/main/-1', '/default/main/12/']) {
      if (p === '/default/main/12/') {
        // 末尾多一个斜杠还是同一张卡片
        expect(parseRoute(p)).toEqual({ path: 'default/main', card: 12, tab: null });
        continue;
      }
      expect(parseRoute(p).path).toBeNull();
    }
  });
});

describe('routeUrl', () => {
  it('白板 Tab 不带 ?tab=；其他 Tab 带上白板', () => {
    expect(routeUrl('default/main', null)).toBe('/default/main');
    expect(routeUrl('default/main', 12)).toBe('/default/main/12');
    expect(routeUrl('default/main', 12, 'inbox')).toBe('/default/main/12?tab=inbox');
    expect(routeUrl('default/main', null, 'settings')).toBe('/default/main?tab=settings');
  });

  it('还没有白板时是根路径', () => {
    expect(routeUrl(null, null)).toBe('/');
    expect(routeUrl(null, null, 'hosts')).toBe('/?tab=hosts');
  });

  it('拼出来的地址能解析回同样的东西', () => {
    for (const [path, card, tab] of [
      ['oss/main', null, 'board'],
      ['oss/main', 7, 'board'],
      ['oss/release', 7, 'inbox'],
      ['default/main', null, 'hosts'],
    ] as const) {
      const url = routeUrl(path, card, tab);
      const u = new URL(url, 'https://board.example.com');
      const r = parseRoute(u.pathname, u.search);
      expect(r).toEqual({ path, card, tab: tab === 'board' ? null : tab });
    }
  });
});
