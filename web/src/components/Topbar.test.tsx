/* 左上角项目总览（default/main#615）：隐藏项目不显示、搜索也搜不到；当前项目例外 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FLOW, workspace } from '../../test/fixtures';
import { useData } from '../store/data';
import { closeSwitcher } from '../store/ui';
import { Topbar } from './Topbar';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  act(() => closeSwitcher());
  useData.setState({ ws: null, path: '' });
});

function setup(hidden: { p?: boolean; q?: boolean }) {
  const ws = workspace();
  const board = { ...ws.projects[0].boards[0], key: 'x', path: 'q/x', name: 'x', statuses: FLOW, counts: {}, holders: [] };
  ws.projects[1].boards = [board];
  for (const p of ws.projects) {
    p.hidden = hidden[p.key as 'p' | 'q'] ?? false;
    for (const b of p.boards) {
      b.statuses = FLOW;
      b.counts = {};
      b.holders = [];
    }
  }
  act(() => useData.setState({ ws, path: 'p/a' }));
}

async function openMenu() {
  await act(async () => root.render(<Topbar />));
  const button = host.querySelector<HTMLButtonElement>('.project-switch')!;
  await act(async () => button.click());
  return document.querySelector<HTMLElement>('.pj-menu')!;
}

const groups = () => [...document.querySelectorAll('.pj-group b')].map(x => x.textContent);

describe('左上角项目总览', () => {
  it('隐藏的项目不显示，搜索也搜不到', async () => {
    setup({ q: true });
    const menu = await openMenu();
    expect(groups()).toEqual(['P']);

    const search = menu.querySelector<HTMLInputElement>('.pj-search input')!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(search, 'Q');
      search.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(groups()).toEqual([]);
    expect(menu.querySelector('.pj-empty')?.textContent).toBe('没有匹配的白板');
  });

  it('当前项目即使隐藏也保留；其它隐藏项目仍不显示', async () => {
    setup({ p: true, q: true });
    const menu = await openMenu();
    expect(groups()).toEqual(['P']);

    const search = menu.querySelector<HTMLInputElement>('.pj-search input')!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(search, 'P');
      search.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(groups()).toEqual(['P']);
  });
});

describe('打开切换下拉框时聚焦搜索框（default/main#651）', () => {
  const viewport = (touch: boolean) =>
    vi.stubGlobal('matchMedia', (q: string) => ({
      matches: touch && (q.includes('max-width') || q.includes('coarse')),
      media: q,
      onchange: null,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      dispatchEvent: () => false,
    }));
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it('桌面：打开就聚焦，能直接打字搜索', async () => {
    viewport(false);
    setup({});
    vi.useFakeTimers();
    const menu = await openMenu();
    act(() => vi.advanceTimersByTime(50));
    expect(document.activeElement).toBe(menu.querySelector('.pj-search input'));
  });

  it('触屏：不自动聚焦（iOS 主屏 PWA 里程序化聚焦后下拉框列表滑不动）', async () => {
    viewport(true);
    setup({});
    vi.useFakeTimers();
    const menu = await openMenu();
    act(() => vi.advanceTimersByTime(50));
    expect(document.activeElement).not.toBe(menu.querySelector('.pj-search input'));
  });
});
