/* 状态块里同一会话的卡框在一起（default/main#289） */
import { describe, expect, it } from 'vitest';
import type { Dispatch } from './types';
import { groupRuns, sessionOrder } from './sessionGroups';

const d = (id: number, card: number, more: Partial<Dispatch> = {}): Dispatch =>
  ({ id, card, board: 'p/a', hostId: 1, host: 'win2', agentLabel: 'Claude Code', state: 'running', ...more }) as Dispatch;

describe('sessionOrder', () => {
  it('截图里交错排着的两个会话：各自挨着放，正在做的在前', () => {
    // #272 会话 A 在做、#271 排在 A 后面；#270 会话 B 在做、#269 排在 B 后面
    const ds = [
      d(10, 272),
      d(11, 270, { hostId: 2, host: 'mbp' }),
      d(12, 271, { state: 'queued', session: 10, seq: 2, total: 2 }),
      d(13, 269, { state: 'queued', session: 11, seq: 2, total: 2, hostId: 2, host: 'mbp' }),
    ];
    const r = sessionOrder([272, 270, 271, 269], ds, 'p/a');
    expect(r.ids).toEqual([272, 271, 270, 269]);
    expect(r.groups[271].ids).toEqual([272, 271]);
    expect(r.groups[269]).toMatchObject({ kind: 'session', host: 'mbp', ids: [270, 269] });
  });

  it('会话里排着的按第几张排，不按状态块里的顺序', () => {
    const ds = [d(1, 5), d(2, 7, { state: 'queued', session: 1, seq: 3 }), d(3, 6, { state: 'queued', session: 1, seq: 2 })];
    expect(sessionOrder([7, 6, 5], ds, 'p/a').ids).toEqual([5, 6, 7]);
  });

  it('等机器空位的排队卡按机器放在一起；没派发的卡不动', () => {
    const ds = [d(1, 3, { state: 'queued' }), d(2, 9, { state: 'queued', session: 2 }), d(3, 4)];
    const r = sessionOrder([3, 8, 4, 9], ds, 'p/a');
    expect(r.ids).toEqual([3, 9, 8, 4]);
    expect(r.groups[3]).toMatchObject({ kind: 'wait', ids: [3, 9] });
    expect(r.groups[4]).toBeUndefined();
    expect(r.groups[8]).toBeUndefined();
  });

  it('一张不成组；别的白板、结束了的派发不算', () => {
    const ds = [d(1, 3), d(2, 4, { state: 'queued', session: 1, board: 'p/b' }), d(3, 5, { state: 'done', session: 1 })];
    const r = sessionOrder([3, 4, 5], ds, 'p/a');
    expect(r).toEqual({ ids: [3, 4, 5], groups: {} });
  });

  it('排队的开始做、会话结束：组跟着变，不留空组', () => {
    // 排着的那张开始做了（会话还是同一个），前面那张交审离开了这一块
    const ds = [d(2, 271, { session: 10, seq: 2 })];
    expect(sessionOrder([271, 270], ds, 'p/a')).toEqual({ ids: [271, 270], groups: {} });
  });
});

describe('groupRuns', () => {
  it('连着的同一组算一段，两张以上才框', () => {
    const ds = [d(1, 1), d(2, 2, { state: 'queued', session: 1, seq: 2 })];
    const { ids, groups } = sessionOrder([1, 2, 3], ds, 'p/a');
    expect(groupRuns(ids, groups).map(r => [r.start, r.n])).toEqual([[0, 2]]);
    // 拖动预览把一张卡插进组中间：拆成两段，各一张，都不框
    expect(groupRuns([1, 3, 2], groups)).toEqual([]);
  });
});
