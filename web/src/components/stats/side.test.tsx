import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { workspace } from '../../../test/fixtures';
import type { Stats } from '../../lib/types';
import { useData } from '../../store/data';
import { StatsSide, useSD, useSP } from './Stats';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const stats = {
  all: { review: 3, done: 8 },
  projects: [{ key: 'p', review: 2, done: 5 }],
  boards: [{ path: 'p/a', review: 1, done: 4 }],
} as unknown as Stats;
const ws = workspace();
ws.projects[0].boards = ws.projects[0].boards.map(b => ({ ...b, holders: [] }));

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({ ws, path: 'p/a' });
  useSP.setState({ scope: 'all' });
  useSD.setState({ data: stats, key: 'all', loading: false, error: null });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('大盘侧栏的计数', () => {
  it('工作区推送后沿用同一棵 Nums 子树，不卸载重挂', () => {
    act(() => root.render(<StatsSide />));
    const num = host.querySelector('.sp-nav-n b');
    expect(num?.textContent).toBe('8');
    act(() => useData.setState({ ws: { ...ws, rev: 2 } }));
    expect(host.querySelector('.sp-nav-n b')).toBe(num);
  });
});
