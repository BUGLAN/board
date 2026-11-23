/* 拖动中占位换位置时，和这次拖动无关的状态块不重画（default/main#631）。
   卡片换成不带 memo 的计数组件：所在的状态块一重画，它就跟着多渲染一次 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardSnapshot, Card, Status, Workspace } from '../lib/types';

const renders: Record<number, number> = {};
vi.mock('./Card', () => ({
  CardView: ({ card }: { card: { id: number } }) => {
    renders[card.id] = (renders[card.id] ?? 0) + 1;
    return <article className="card" data-id={card.id} />;
  },
  TilePeek: () => null,
  cardHit: () => true,
}));
vi.mock('./FlowBar', () => ({ FlowBar: () => null }));
vi.mock('./BoardSettings', () => ({ BoardSettings: () => null }));
vi.mock('./CardCrons', () => ({ CardCronDialog: () => null }));
vi.mock('./FirstRun', () => ({ FirstRunHint: () => null }));
vi.mock('./BoardSelect', () => ({ BoardBulkBar: () => null, ColSelect: () => null }));

import { useData } from '../store/data';
import { setDrag, useUI, type DragUI } from '../store/ui';
import { BoardView } from './BoardView';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords(): ResizeObserverEntry[] {
    return [];
  }
}

const status = (key: string, over: Partial<Status> = {}): Status => ({ key, name: key, color: 'todo', claim: false, lock: false, final: false, layout: null, to: [], ...over });
const statuses = [status('todo', { claim: true }), status('review'), status('done', { final: true })];
const card = (id: number, column: string) =>
  ({ id, title: `卡片 ${id}`, body: '', column, holder: null, labels: [], comments: 0, createdAt: 0, updatedAt: 0, movedAt: id }) as unknown as Card;
const cols = { todo: [1], review: [2, 3], done: [4, 5, 6] };
const board = {
  rev: 1, now: 0, path: 'p/b', statuses, cols,
  cards: Object.fromEntries(Object.entries(cols).flatMap(([k, ids]) => ids.map(id => [id, card(id, k)]))),
  omitted: {}, log: [], keepSorted: false, trash: [],
} as unknown as BoardSnapshot;
const ws = {
  rev: 1, now: 0, agents: [], hosts: [], dispatches: [],
  projects: [{ key: 'p', name: 'P', labels: [], boards: [{ path: 'p/b', name: 'B', statuses, counts: {} }] }],
} as unknown as Workspace;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
const original = globalThis.ResizeObserver;

beforeEach(() => {
  for (const k of Object.keys(renders)) delete renders[Number(k)];
  globalThis.ResizeObserver = FakeResizeObserver as unknown as typeof ResizeObserver;
  useData.setState({ ws, board, path: 'p/b', detailId: null });
  useUI.setState({ tab: 'board', drag: null, filter: '', labelSel: [] });
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  act(() => useUI.setState({ drag: null }));
  globalThis.ResizeObserver = original;
});

const drag = (over: Partial<DragUI>): DragUI => ({ id: 2, fromCol: 'review', col: 'review', index: 0, over: 'review', valid: true, veto: null, ...over });
const ids = (col: string) => [...host.querySelectorAll<HTMLElement>(`[data-list="${col}"] .card`)].map(el => Number(el.dataset.id));

describe('拖动卡片时只重画受影响的状态块（default/main#631）', () => {
  it('占位在「待审核」里换位置：「待办」「已完成」不重画', () => {
    act(() => root.render(<BoardView />));
    // 第一次渲染带着入场动画的延时（fresh），之后的第一次重画所有块都要去掉它：先过掉这一次再数
    act(() => root.render(<BoardView />));
    expect(ids('review')).toEqual([2, 3]);
    expect(ids('done')).toEqual([4, 5, 6]);
    const before = { ...renders };

    act(() => setDrag(drag({ index: 0 })));
    act(() => setDrag(drag({ index: 1 })));
    expect(ids('review')).toEqual([3, 2]);
    act(() => setDrag(drag({ index: 0 })));
    expect(ids('review')).toEqual([2, 3]);

    // 待审核跟着重画了，另外两块一次都没有
    expect(renders[3]).toBeGreaterThan(before[3]);
    for (const id of [1, 4, 5, 6]) expect(renders[id]).toBe(before[id]);
  });

  it('占位进了「已完成」这一块才重画；指针只是经过「待办」时，「已完成」不再跟着画', () => {
    act(() => root.render(<BoardView />));
    act(() => setDrag(drag({ index: 0 })));
    const before = { ...renders };

    act(() => setDrag(drag({ col: 'done', index: 1, over: 'done' })));
    expect(ids('done')).toEqual([4, 2, 5, 6]);
    expect(ids('review')).toEqual([3]);
    expect(renders[4]).toBeGreaterThan(before[4]);
    expect(renders[1]).toBe(before[1]);

    // 占位回到「待审核」之后，在「待审核」里再换位置：「已完成」停在刚才那次
    act(() => setDrag(drag({ index: 1 })));
    const mid = { ...renders };
    act(() => setDrag(drag({ index: 0 })));
    act(() => setDrag(drag({ index: 1 })));
    for (const id of [1, 4, 5, 6]) expect(renders[id]).toBe(mid[id]);
  });
});
