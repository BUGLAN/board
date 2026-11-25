import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { snapshot, workspace } from '../../../test/fixtures';
import { visibleInbox } from '../../lib/split';
import { useData } from '../../store/data';
import { closeCtx, openCtx } from '../../store/ui';
import { CtxMenu } from './CtxMenu';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  const board = snapshot('p/a', { todo: [1], inbox: [10, 11, 12, 13] }, [
    { id: 11, laterAt: 100 },
    { id: 12, splitFrom: 10 },
    { id: 13, splitFrom: 10 },
  ]);
  useData.setState({ ws: workspace(), path: 'p/a', board } as never);
  openCtx({ kind: 'move', id: '1', x: 0, y: 0 });
});

afterEach(() => {
  act(() => root.unmount());
  closeCtx();
  host.remove();
  document.body.innerHTML = '';
});

describe('「移动到」的计数', () => {
  it('「未整理」只数 visibleInbox：不含稍后再做和已拆出的原条目', () => {
    act(() => root.render(<CtxMenu />));
    const count = host.querySelector('.mv-item[data-do="move-inbox"] .mv-count');
    expect(visibleInbox(useData.getState().board).length).toBe(2);
    expect(count?.textContent).toBe('2');
  });
});
