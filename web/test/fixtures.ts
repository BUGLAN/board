/* 测试用的数据：最小的工作区、白板快照、卡片 */
import type { BoardSnapshot, Card, CardDetail, Status, Workspace } from '../src/lib/types';

export function card(id: number, over: Partial<Card> = {}): Card {
  return {
    id,
    title: `卡片 ${id}`,
    body: '',
    column: 'todo',
    holder: null,
    claimedAt: null,
    beatAt: null,
    runStartedAt: null,
    workMs: 0,
    runs: 0,
    runsLost: 0,
    claimedFrom: null,
    labels: [],
    createdAt: 0,
    updatedAt: 0,
    createdBy: { kind: 'human', name: '' },
    comments: 0,
    ...over,
  };
}

export function status(key: string, over: Partial<Status> = {}): Status {
  return { key, name: key, color: 'todo', claim: false, lock: false, final: false, layout: null, to: [], ...over };
}

/** 默认工作流：todo → doing → review → done / rework → doing */
export const FLOW: Status[] = [
  status('todo', { claim: true, to: ['doing'] }),
  status('doing', { lock: true, to: ['review'] }),
  status('review', { to: ['done', 'rework'] }),
  status('done', { final: true }),
  status('rework', { claim: true, to: ['doing'] }),
];

/** cols：状态 → 卡片 ID；卡片按所在状态生成，cards 里的字段覆盖 */
export function snapshot(path: string, cols: Record<string, number[]>, cards: Partial<Card>[] = []): BoardSnapshot {
  const all: Record<string, Card> = {};
  for (const [col, ids] of Object.entries(cols)) for (const id of ids) all[id] = card(id, { column: col });
  for (const c of cards) all[c.id!] = { ...all[c.id!], ...c } as Card;
  return { rev: 1, now: 0, path, statuses: FLOW, cols, cards: all, log: [] };
}

export function detail(c: Card): CardDetail {
  return { ...c, activity: [], attachments: [] } as unknown as CardDetail;
}

export function workspace(over: Partial<Workspace> = {}): Workspace {
  const board = (key: string) => ({ key, path: `p/${key}`, name: key, desc: '', pos: { x: 0, y: 0 }, createdAt: 0 });
  return {
    rev: 1,
    now: 0,
    projects: [
      {
        key: 'p',
        name: 'P',
        desc: '',
        color: '#000000',
        pos: { x: 0, y: 0 },
        createdAt: 0,
        labels: [
          { id: 1, name: 'bug', color: '#f00', uses: 1 },
          { id: 2, name: 'ui', color: '#0f0', uses: 1 },
        ],
        boards: [board('a'), board('b')],
      },
      { key: 'q', name: 'Q', desc: '', color: '#000000', pos: { x: 0, y: 0 }, createdAt: 0, labels: [{ id: 1, name: 'bug', color: '#f00', uses: 0 }], boards: [] },
    ],
    agents: [],
    hosts: [],
    dispatches: [],
    ...over,
  } as Workspace;
}
