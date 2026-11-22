import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { workspace } from '../../test/fixtures';
import { useData } from '../store/data';
import { useUI } from '../store/ui';
import { SidePanel } from './Workbench';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  const ws = workspace();
  for (const project of ws.projects) {
    for (const board of project.boards) {
      Object.assign(board, { counts: {}, statuses: [], holders: [] });
    }
  }
  useData.setState({ ws, board: null, path: 'p/a', saving: {} } as never);
  useUI.setState({ tab: 'projects', side: true });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  useData.setState({ ws: null });
});

describe('侧栏项目树', () => {
  it('项目下不再重复显示「新建白板」，只列出白板', () => {
    act(() => root.render(<SidePanel />));
    const tree = host.querySelector<HTMLElement>('[data-view="projects"] .side-scroll')!;
    expect(tree.querySelectorAll('.pt-row.brd')).toHaveLength(2);
    expect(tree.querySelector('.pt-row.add')).toBeNull();
    expect(tree.textContent).not.toContain('新建白板');
  });
});
