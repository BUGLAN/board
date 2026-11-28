import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { card, snapshot } from '../../test/fixtures';
import type { BoardSnapshot, TrashCard } from '../lib/types';
import { useData } from '../store/data';
import { TrashPage, TrashRow } from './Trash';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const server = vi.hoisted(() => ({ trash: vi.fn() }));
vi.mock('../lib/api', async orig => {
  const real = await orig<typeof import('../lib/api')>();
  return { ...real, api: { ...real.api, trash: server.trash } };
});

const item: TrashCard = {
  ...card(40, { column: 'inbox', title: '删掉的条目' }),
  deletedAt: 1000,
  deletedBy: { kind: 'human', name: '人' },
  col: 'inbox',
  pos: 0,
};

const board = (rev: number, trash: number): BoardSnapshot => ({ ...snapshot('p/a', {}, []), rev, trash });
let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  server.trash.mockReset();
  server.trash.mockResolvedValue([item]);
  useData.setState({ path: 'p/a', board: board(1, 1) });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('回收站只在张数变化时重新取', () => {
  it('同一张数下 agent 改卡不触发 GET /api/trash，张数变了才取', async () => {
    await act(async () => root.render(<TrashPage path="p/a" />));
    expect(server.trash).toHaveBeenCalledTimes(1);
    act(() => useData.setState({ board: board(2, 1) }));
    expect(server.trash).toHaveBeenCalledTimes(1);
    await act(async () => useData.setState({ board: board(3, 2) }));
    expect(server.trash).toHaveBeenCalledTimes(2);
  });
});

describe('回收站的描述首行', () => {
  it('只有小标题时显示小标题', () => {
    const t: TrashCard = {
      id: 42,
      title: '登录页按钮圆角',
      body: '### 设置页也一样',
      column: 'todo',
      deletedAt: 0,
      deletedBy: { kind: 'human', name: '' },
    } as TrashCard;
    act(() =>
      root.render(
        <TrashRow t={t} now={1000} statuses={[]} busy={false} checked={false} onCheck={() => {}} onRestore={() => {}} onPurge={() => {}} />,
      ),
    );
    expect(host.querySelector('.ib-t > span')?.textContent).toBe('设置页也一样');
  });
});
