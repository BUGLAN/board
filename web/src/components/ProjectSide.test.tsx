/* 左侧项目面板（default/main#610）：隐藏的项目不显示；有搜索词或它是当前项目时仍然显示。
   点项目 / 白板选时间轴的范围（PRD §10.4）：点项目看整个项目、点白板只看这块白板，都不离开项目 Tab；箭头和 ← → 收起展开 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FLOW, workspace } from '../../test/fixtures';
import { useData } from '../store/data';
import { setTlBoard } from '../store/timeline';
import { ProjectsSide } from './Workbench';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const acts = vi.hoisted(() => ({ openBoard: vi.fn() }));
vi.mock('../lib/actions', async orig => ({ ...(await orig<typeof import('../lib/actions')>()), openBoard: acts.openBoard }));

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  useData.setState({ ws: null, path: 'p/a' });
  setTlBoard(null);
  acts.openBoard.mockReset();
  localStorage.clear();
});

function ws() {
  const w = workspace();
  for (const p of w.projects) {
    for (const b of p.boards) {
      b.statuses = FLOW;
      b.counts = {};
      b.holders = [];
      b.updatedAt = 0;
    }
  }
  return w;
}

const shown = () => [...host.querySelectorAll<HTMLElement>('[data-project]')].map(x => x.dataset.project);
const search = () => host.querySelector<HTMLInputElement>('.side-filter input')!;
const type = (v: string) =>
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(search(), v);
    search().dispatchEvent(new Event('input', { bubbles: true }));
  });

describe('左侧项目面板', () => {
  it('隐藏的项目默认不显示；有搜索词时显示', async () => {
    const w = ws();
    w.projects[1].hidden = true;
    act(() => useData.setState({ ws: w, path: 'p/a' }));
    await act(async () => root.render(<ProjectsSide />));
    expect(shown()).toEqual(['p']);
    type('q');
    expect(shown()).toEqual(['q']);
  });

  it('隐藏的项目是当前项目时仍然显示', async () => {
    const w = ws();
    w.projects[0].hidden = true;
    w.projects[1].hidden = true;
    act(() => useData.setState({ ws: w, path: 'p/a' }));
    await act(async () => root.render(<ProjectsSide />));
    expect(shown()).toEqual(['p']);
  });
});

describe('点项目、白板选时间轴的范围', () => {
  const withQ = () => {
    const w = ws();
    w.projects[1].boards = [{ ...w.projects[0].boards[0], key: 'x', path: 'q/x', name: 'x', statuses: FLOW, counts: {}, holders: [] }];
    return w;
  };
  const row = (sel: string) => host.querySelector<HTMLElement>(sel)!;
  const proj = (k: string) => row(`[data-project="${k}"] .pt-row.proj`);
  const brd = (name: string) => [...host.querySelectorAll<HTMLElement>('.pt-row.brd')].find(r => r.querySelector('.nm')?.textContent === name)!;
  const click = (el: HTMLElement) => act(() => el.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  const press = (el: HTMLElement, key: string) => act(() => el.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true })));

  beforeEach(async () => {
    act(() => useData.setState({ ws: withQ(), path: 'p/a' }));
    await act(async () => root.render(<ProjectsSide />));
  });

  it('默认看当前白板所在的整个项目', () => {
    expect(proj('p').classList.contains('current')).toBe(true);
    expect(row('[data-project="p"]').getAttribute('aria-selected')).toBe('true');
    expect(host.querySelectorAll('.pt-row.brd.current')).toHaveLength(0);
  });

  it('点白板：只看这块白板、当前白板换成它，不离开项目 Tab；再点一次回到整个项目', () => {
    click(brd('b'));
    expect(acts.openBoard).toHaveBeenLastCalledWith('p/b', { stay: true });
    expect(brd('b').classList.contains('current')).toBe(true);
    expect(brd('b').getAttribute('aria-selected')).toBe('true');
    expect(proj('p').classList.contains('current')).toBe(false);
    click(brd('b'));
    expect(brd('b').classList.contains('current')).toBe(false);
    expect(proj('p').classList.contains('current')).toBe(true);
  });

  it('点项目：回到整个项目；换项目时当前白板换成它的第一块，不离开项目 Tab', () => {
    click(brd('a'));
    click(proj('p'));
    expect(brd('a').classList.contains('current')).toBe(false);
    expect(acts.openBoard).toHaveBeenCalledTimes(1);
    click(proj('q'));
    expect(acts.openBoard).toHaveBeenLastCalledWith('q/x', { stay: true });
  });

  it('收起、展开只在箭头和 ← → 上；点整行不收起', () => {
    click(proj('p'));
    expect(row('[data-project="p"]').getAttribute('aria-expanded')).toBe('true');
    click(row('[data-project="p"] .pt-chev'));
    expect(row('[data-project="p"]').getAttribute('aria-expanded')).toBe('false');
    expect(acts.openBoard).not.toHaveBeenCalled();
    press(proj('p'), 'ArrowRight');
    expect(row('[data-project="p"]').getAttribute('aria-expanded')).toBe('true');
    press(proj('p'), 'ArrowLeft');
    expect(row('[data-project="p"]').getAttribute('aria-expanded')).toBe('false');
    // 点收起的项目会展开它
    click(proj('p'));
    expect(row('[data-project="p"]').getAttribute('aria-expanded')).toBe('true');
  });
});
