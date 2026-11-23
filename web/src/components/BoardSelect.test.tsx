import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { snapshot, workspace } from '../../test/fixtures';
import { useBoardSel } from '../lib/boardSelect';
import { useData } from '../store/data';
import { useUI } from '../store/ui';
import { BoardBulkBar } from './BoardSelect';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({
    ws: workspace(),
    path: 'p/a',
    board: snapshot('p/a', { todo: [1, 2], review: [3], inbox: [9] }),
  } as never);
  useUI.setState({ filter: '', labelSel: [], dispatchOf: null });
  useBoardSel.setState({ on: true, ids: [], anchor: null });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
  useBoardSel.setState({ on: false, ids: [], anchor: null });
  useUI.setState({ dispatchOf: null });
});

describe('白板批量条的派送', () => {
  it('一张没选时禁用；选上工作流里的卡片后打开批量派送，未整理和已删掉的不带进去', () => {
    act(() => root.render(<BoardBulkBar />));
    const button = host.querySelector<HTMLButtonElement>('.bulk-dispatch-btn')!;
    expect(button.textContent).toBe('派送');
    expect(button.disabled).toBe(true);

    act(() => useBoardSel.setState({ on: true, ids: [2, 9, 88, 1], anchor: 1 }));
    expect(button.disabled).toBe(false);
    act(() => button.click());

    expect(useUI.getState().dispatchOf).toEqual({ cards: [2, 1] });
    expect(useBoardSel.getState().ids).toEqual([2, 9, 88, 1]);
  });
});
