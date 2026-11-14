import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Card } from '../lib/types';
import { useData } from '../store/data';
import { CardDetail } from './CardDetail';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/* 详情顶栏在两档窗口下的样子：
   - 手机：上一张 / 下一张整组 inert + aria-hidden（CSS 藏了它，不能还留在 Tab 序列里），
     「复制链接」「复制为提示词」收进「更多」菜单，返回 / 前进 / 更多 / 关闭 都是 44 的触屏目标；
   - 桌面：上一张 / 下一张在、没有「更多」、复制链接和复制为提示词是带字的按钮。 */

const NOW = new Date(2026, 10, 14, 12).getTime();

const card: Card = {
  id: 2,
  title: '修一下横轴',
  body: '',
  column: 'todo',
  holder: null,
  claimedAt: null,
  beatAt: null,
  runStartedAt: null,
  workMs: 0,
  runs: 0,
  runsLost: 0,
  claimedFrom: null,
  labels: [],
  createdAt: NOW - 3600_000,
  updatedAt: NOW,
  createdBy: { kind: 'human', name: '人' },
  comments: 2,
};

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({
    ws: { rev: 1, now: NOW, projects: [{ key: 'oss', name: '开源仓库', boards: [{ key: 'main', path: 'oss/main', name: '主线', statuses: [] }] }] },
    path: 'oss/main',
    board: { cards: { 2: card }, cols: { todo: [2] } },
    detailId: 2,
    detail: null,
  } as never);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

const setViewport = (mobile: boolean) => {
  vi.stubGlobal('matchMedia', (q: string) => ({
    matches: q.includes('max-width') ? mobile : !mobile,
    media: q,
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent: () => false,
  }));
};

const render = () => act(() => root.render(<CardDetail />));
const head = () => host.querySelector<HTMLElement>('.d-head')!;
const nav = () => host.querySelector<HTMLElement>('.d-nav')!;

describe('卡片详情顶栏 · 手机', () => {
  beforeEach(() => {
    setViewport(true);
    render();
  });

  it('上一张 / 下一张不在焦点序列里：整组 inert 且对读屏隐藏', () => {
    expect(nav()).not.toBeNull();
    expect(nav().hasAttribute('inert')).toBe(true);
    expect(nav().getAttribute('aria-hidden')).toBe('true');
    for (const b of nav().querySelectorAll('button')) expect(b.closest('[inert]')).toBe(nav());
  });

  it('「复制链接」「复制为提示词」收进「更多」菜单，菜单项带文字', () => {
    expect(head().querySelector('.d-prompt')).toBeNull();
    const more = head().querySelector<HTMLButtonElement>('.d-more > button')!;
    expect(more.getAttribute('aria-label')).toBe('更多');
    expect(more.getAttribute('aria-haspopup')).toBe('menu');
    expect(more.getAttribute('aria-expanded')).toBe('false');
    act(() => more.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    const items = [...host.querySelectorAll('.d-more-menu [role="menuitem"]')].map(x => x.textContent!.trim());
    expect(items).toEqual(['复制链接', '复制为提示词']);
    expect(more.getAttribute('aria-expanded')).toBe('true');
  });

  it('返回 / 前进 / 更多 / 关闭 都在，「更多」打开后点菜单外关掉', () => {
    for (const label of ['返回', '前进', '更多', '关闭']) expect(head().querySelector(`[aria-label="${label}"]`)).not.toBeNull();
    const more = head().querySelector<HTMLButtonElement>('.d-more > button')!;
    act(() => more.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(host.querySelector('.d-more-menu')).not.toBeNull();
    act(() => host.querySelector('.d-more-back')!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(host.querySelector('.d-more-menu')).toBeNull();
  });
});

describe('卡片详情顶栏 · 桌面', () => {
  beforeEach(() => {
    setViewport(false);
    render();
  });

  it('上一张 / 下一张还在（没有 inert），复制按钮带文字，没有「更多」菜单', () => {
    expect(nav().hasAttribute('inert')).toBe(false);
    expect(nav().getAttribute('aria-hidden')).toBeNull();
    expect(head().querySelector('.d-more')).toBeNull();
    const texts = [...head().querySelectorAll('.d-prompt')].map(b => b.textContent!.trim());
    expect(texts).toEqual(['复制链接', '复制为提示词']);
  });
});
