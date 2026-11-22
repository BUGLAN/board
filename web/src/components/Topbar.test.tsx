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
