import { describe, expect, it } from 'vitest';
import { approveTargets, deskQueue, homeOf, latestAgentComment, rejectTargets, splitFacts, statusRoles, taskCount, toggleTask } from './review';
import type { Activity, BoardSnapshot, Status } from './types';

const st = (key: string, name: string, more: Partial<Status> = {}): Status => ({ key, name, color: 'todo', claim: false, lock: false, final: false, layout: null, to: [], ...more });
// 默认工作流加一个错误节点
const flow: Status[] = [
  st('todo', '待办', { claim: true, to: ['doing'] }),
  st('doing', '进行中', { lock: true, to: ['review', 'check', 's2'] }),
  st('review', '待审核', { to: ['done', 'rework'] }),
  st('done', '已完成', { final: true }),
  st('rework', '返工', { claim: true, to: ['doing'] }),
  st('check', '需人工校验', { to: ['todo'] }),
  st('s2', '错误', { error: true }),
];

describe('状态的角色', () => {
  it('交审、需要人处理（含错误节点）和服务端一样认', () => {
    const r = statusRoles(flow);
    expect([...r.review]).toEqual(['review']);
    expect([...r.human]).toEqual(['check', 's2']);
    // 不是从锁定状态连过去的不算
    expect(statusRoles([st('a', '待审核')]).review.size).toBe(0);
    expect(statusRoles(null).review.size).toBe(0);
  });
  it('去向：通过去连向的终点，打回去连向的可领取状态；没有连线时退回', () => {
    expect(approveTargets(flow, 'review').map(s => s.key)).toEqual(['done']);
    expect(rejectTargets(flow, 'review').map(s => s.key)).toEqual(['rework']);
    // 没有连向终点：全部终点；没有连向可领取状态：首个可领取状态
    expect(approveTargets(flow, 'check').map(s => s.key)).toEqual(['done']);
    expect(rejectTargets(flow, 's2').map(s => s.key)).toEqual(['todo']);
    expect(homeOf(flow)?.key).toBe('todo');
  });
});

describe('队列', () => {
  it('这几个状态里的卡片按进入当前状态的时间从早到晚', () => {
    const card = (id: number, column: string, movedAt: number) => ({ id, column, movedAt, createdAt: 0 });
    const b = { cols: { review: [3, 1], check: [2] }, cards: { 1: card(1, 'review', 300), 2: card(2, 'check', 100), 3: card(3, 'review', 200) } } as unknown as BoardSnapshot;
    expect(deskQueue(b, ['review'])).toEqual([3, 1]);
    expect(deskQueue(b, ['review', 'check'])).toEqual([2, 3, 1]);
    expect(deskQueue(null, ['review'])).toEqual([]);
  });
});

describe('交审评论', () => {
  const a = (id: number, kind: 'agent' | 'human', text: string, type: 'comment' | 'system' = 'comment'): Activity => ({ id, at: id, actor: { kind, name: 'x' }, kind: type, action: type, text });
  it('最后一条 agent 发的评论', () => {
    expect(latestAgentComment([a(1, 'agent', '一'), a(2, 'agent', '二'), a(3, 'human', '人'), a(4, 'agent', '移动', 'system')])?.id).toBe(2);
    expect(latestAgentComment([a(1, 'human', '人')])).toBeNull();
  });
  it('拆出提交、集成分支、检查几行；代码块里的不拆', () => {
    const { body, facts } = splitFacts('改好了。\n\n```\n提交：不算\n```\n\n提交：a5ea001\n集成分支：main\n检查：npm test 通过');
    expect(facts).toEqual([
      { key: '提交', value: 'a5ea001' },
      { key: '集成分支', value: 'main' },
      { key: '检查', value: 'npm test 通过' },
    ]);
    expect(body).toBe('改好了。\n\n```\n提交：不算\n```');
  });
});

describe('勾选验收标准', () => {
  const body = '## 验收标准\n\n- [ ] 一\n- [x] 二\n```\n- [ ] 代码里的\n```\n  * [ ] 三\n1. [X] 四';
  it('翻转第 n 项，代码块里的不算', () => {
    expect(toggleTask(body, 0)).toContain('- [x] 一');
    expect(toggleTask(body, 1)).toContain('- [ ] 二');
    expect(toggleTask(body, 2)).toContain('  * [x] 三');
    expect(toggleTask(body, 3)).toContain('1. [ ] 四');
    expect(toggleTask(body, 2)).toContain('- [ ] 代码里的');
    expect(toggleTask(body, 9)).toBe(body);
    expect(toggleTask(toggleTask(body, 0), 0)).toBe(body);
  });
  it('数一共几项、勾了几项', () => {
    expect(taskCount(body)).toEqual({ done: 2, total: 4 });
  });
});
