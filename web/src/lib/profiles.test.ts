/* 角色（lib/profiles.ts）：规则命中、机器现在能不能接、挑机器、推算一块白板上的卡会交给谁 */
import { describe, expect, it } from 'vitest';
import { forecast, hostsFor, pickHost, routeOf, ruleMatches, slotOf, soonest } from './profiles';
import type { BoardSnapshot, ClaimRule, HostInfo, Workspace } from './types';

const host = (name: string, id: number, x: Partial<HostInfo> = {}): HostInfo =>
  ({ id, name, os: 'macOS', version: '0.1.47', agents: [], dirs: [], max: 2, paused: false, online: true, auto: true, windows: [[0, 1440]], fits: [], ...x }) as unknown as HostInfo;
const fit = (profile: string, ok = true, why = '') => ({ profile, ok, why });

describe('规则命中', () => {
  it('六种条件，标签不分大小写', () => {
    expect(ruleMatches('in', ['Bug', 'UI'], ['bug'])).toBe(true);
    expect(ruleMatches('all', ['bug', 'ui'], ['BUG'])).toBe(false);
    expect(ruleMatches('not_in', ['bug'], ['ui'])).toBe(true);
    expect(ruleMatches('equal', ['ui', 'bug'], ['bug', 'UI'])).toBe(true);
    expect(ruleMatches('only', ['bug', 'ui'], ['bug'])).toBe(true);
    expect(ruleMatches('only', ['bug'], [])).toBe(false);
    expect(ruleMatches('none', [], [])).toBe(true);
  });

  it('第一条命中的生效；都没命中时为空（不自动领）', () => {
    const rules = [
      { op: 'all', labels: ['bug', 'Windows'], profile: 'fix' },
      { op: 'in', labels: ['bug'], profile: 'bulk' },
    ] as ClaimRule[];
    expect(routeOf(['bug', 'windows'], rules)).toEqual({ rule: 0, profile: 'fix' });
    expect(routeOf(['bug'], rules)).toEqual({ rule: 1, profile: 'bulk' });
    expect(routeOf(['文档'], rules)).toBeNull();
  });
});

describe('机器现在能不能接', () => {
  const ws = (hosts: HostInfo[], extra: Partial<Workspace> = {}) => ({ rev: 1, now: 0, projects: [], hosts, dispatches: [], ...extra }) as unknown as Workspace;

  it('跑不了（服务端算的原因）、离线、暂停、时段外（只有自动领取看）、满了、有空位', () => {
    const w = ws([
      host('a', 1, { fits: [fit('p', false, '没装 Codex')] }),
      host('b', 2, { fits: [fit('p')], online: false }),
      host('c', 3, { fits: [fit('p')], windows: [[1200, 1300]] }),
      host('d', 4, { fits: [fit('p')], max: 1 }),
    ]);
    w.dispatches = [{ id: 9, state: 'running', host: 'd', hostId: 4 } as never];
    const got = hostsFor(w, 'p', 600, true).map(x => [x.host.name, x.can, x.ok, x.why]);
    expect(got).toEqual([
      ['a', false, false, '没装 Codex'],
      ['b', true, false, '离线'],
      ['c', true, false, '时段外，20:00 开始接单'],
      ['d', true, false, '满 1/1'],
    ]);
    // 手动派发不看接单时段
    expect(slotOf(w, w.hosts![2], 600, false)).toMatchObject({ ok: true, why: '空 2 位' });
    expect(soonest(hostsFor(w, 'p', 600, true))).toBe('d 空出来就开始；c 20:00 开始接单');
  });

  it('挑机器：空位多的优先', () => {
    const w = ws([host('a', 1, { fits: [fit('p')], max: 1 }), host('b', 2, { fits: [fit('p')], max: 3 })]);
    expect(pickHost(w, hostsFor(w, 'p', 600, true))?.host.name).toBe('b');
  });
});

describe('推算一块白板', () => {
  const board = (): BoardSnapshot =>
    ({
      path: 'oss/main',
      statuses: [{ key: 'todo', name: '待办', claim: true }, { key: 'doing', name: '进行中', claim: false }],
      cols: { todo: [1, 2, 3], doing: [4] },
      cards: {
        1: { id: 1, title: 'A', column: 'todo', labels: [1] },
        2: { id: 2, title: 'B', column: 'todo', labels: [1] },
        3: { id: 3, title: 'C', column: 'todo', labels: [] },
        4: { id: 4, title: 'D', column: 'doing', labels: [1], holder: 'x' },
      },
    }) as unknown as BoardSnapshot;
  const ws = (boardAuto = true): Workspace =>
    ({
      rev: 1,
      now: 0,
      projects: [{ key: 'oss', labels: [{ id: 1, name: 'bug' }], boards: [{ path: 'oss/main', auto: boardAuto }] }],
      hosts: [host('mac', 1, { fits: [fit('p1')], max: 1 })],
      dispatches: [],
      sched: { minute: 600, rules: [{ op: 'in', labels: ['bug'], profile: 'p1' }], profiles: [{ id: 'p1', name: '修复', agent: 'claude', provider: '', model: '', effort: '', note: '', only: [] }] },
    }) as unknown as Workspace;

  it('命中规则的第一张马上开始、第二张排队第 1；没命中的不自动领；持有中的不算', () => {
    const fc = forecast(ws(), board(), 0);
    expect([...fc.values()].map(f => [f.card, f.kind, f.host ?? null, f.pos ?? null])).toEqual([
      [1, 'start', 'mac', null],
      [2, 'wait', null, 1],
      [3, 'none', null, null],
    ]);
  });

  it('白板不参与自动领取：命中的也写「不参与」；没有角色时不推算', () => {
    expect(forecast(ws(false), board(), 0).get(1)?.kind).toBe('off');
    const none = ws();
    none.sched!.profiles = [];
    expect(forecast(none, board(), 0).size).toBe(0);
  });
});
