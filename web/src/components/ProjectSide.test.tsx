/* 左侧项目面板（default/main#610）：隐藏的项目不显示；有搜索词或它是当前项目时仍然显示 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { FLOW, workspace } from '../../test/fixtures';
import { useData } from '../store/data';
import { ProjectsSide } from './Workbench';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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
