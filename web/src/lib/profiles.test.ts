/* 角色（lib/profiles.ts）：规则命中、机器现在能不能接、挑机器、推算一块白板上的卡会交给谁 */
import { describe, expect, it } from 'vitest';
import { forecast, groupPick, hostsFor, localChoices, pickHost, recipe, routeOf, ruleGroups, ruleMatches, short, slotOf, soonest, SUBSCRIPTION, subscribable } from './profiles';
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

describe('批量派送：按规则分组、预选角色', () => {
  const ws = {
    rev: 1,
    now: 0,
    projects: [{ key: 'p', labels: [{ id: 1, name: 'bug' }, { id: 2, name: 'ui' }, { id: 3, name: 'docs' }], boards: [] }],
    hosts: [],
    dispatches: [],
    sched: {
      minute: 0,
      rules: [
        { op: 'in', labels: ['bug'], profile: 'fix' },
        { op: 'in', labels: ['ui'], profile: 'ui' },
        { op: 'in', labels: ['docs'], profile: 'gone' },
        { op: 'in', labels: ['ui', 'docs'], profile: 'fix' },
      ],
      profiles: [{ id: 'fix' }, { id: 'ui' }],
    },
  } as unknown as Workspace;
  const c = (id: number, labels: number[]) => ({ id, labels });

  it('按第一条命中的规则分组：规则靠前的在前，没命中的一组放最后；规则交给的角色已经删了时角色为空', () => {
    expect(ruleGroups(ws, 'p/a', [c(5, []), c(1, [2]), c(2, [1, 2]), c(3, [2]), c(4, [3])])).toEqual([
      { rule: 0, profile: 'fix', ids: [2] },
      { rule: 1, profile: 'ui', ids: [1, 3] },
      { rule: 2, profile: null, ids: [4] },
      { rule: null, profile: null, ids: [5] },
    ]);
  });

  it('预选按规则交给它的卡最多的角色；一样多取规则靠前的；同一个角色的几组合起来算；都没命中时为空', () => {
    expect(groupPick([{ rule: 0, profile: 'fix', ids: [1] }, { rule: 1, profile: 'ui', ids: [2, 3] }])).toBe('ui');
    expect(groupPick([{ rule: 0, profile: 'fix', ids: [1] }, { rule: 1, profile: 'ui', ids: [2] }])).toBe('fix');
    expect(groupPick([{ rule: 0, profile: 'fix', ids: [1] }, { rule: 1, profile: 'ui', ids: [2, 3] }, { rule: 3, profile: 'fix', ids: [4, 5] }])).toBe('fix');
    expect(groupPick([{ rule: 2, profile: null, ids: [1] }, { rule: null, profile: null, ids: [2] }])).toBeNull();
  });
});

describe('模型来源的叫法（#667）', () => {
  const name = (id: string) => ({ claude: 'Claude Code', codex: 'Codex' })[id] ?? id;
  it('本机配置、官方订阅、供应商', () => {
    expect(recipe({ agent: 'claude', provider: '', model: '', effort: 'high' }, name)).toBe('Claude Code · 本机配置 · high');
    expect(recipe({ agent: 'codex', provider: SUBSCRIPTION, model: 'gpt-5', effort: '' }, name)).toBe('Codex · 官方订阅 · gpt-5');
    expect(recipe({ agent: 'claude', provider: 'relay', model: 'm1', effort: '' }, name)).toBe('Claude Code · relay · m1');
    expect(short({ provider: SUBSCRIPTION, model: '', effort: '' })).toBe('默认模型');
    expect(short({ provider: 'relay', model: '', effort: 'high' })).toBe('high');
  });
  it('只有 Claude Code、Codex 能选官方订阅', () => {
    expect([subscribable('claude'), subscribable('codex'), subscribable('dsh')]).toEqual([true, true, false]);
  });
});

describe('本机账号的模型表（#666）', () => {
  const h = (id: number, models?: { id: string; efforts: string[] }[], def = '') =>
    ({ id, name: `h${id}`, agents: [{ id: 'claude', name: 'Claude Code', version: '1', found: true, local_models: models ? { models, model: def } : null }] }) as unknown as HostInfo;
  const mbp = h(1, [{ id: 'opus', efforts: ['low', 'high', 'max'] }, { id: 'haiku', efforts: [] }], 'opus');
  const desk = h(2, [{ id: 'opus', efforts: ['low', 'high'] }, { id: 'sonnet', efforts: ['low', 'medium'] }], 'opus');
  const old = h(3);
  it('选中的机器报的模型并起来；强度按选的模型（没选时按各台的默认模型）', () => {
    const c = localChoices([mbp, desk, old], 'claude', [], '', '');
    expect(c.known).toBe(true);
    expect(c.models.map(m => m.id)).toEqual(['opus', 'haiku', 'sonnet']);
    expect(c.efforts).toEqual(['low', 'high', 'max']);
    expect(localChoices([mbp, desk], 'claude', [2], 'sonnet', '').efforts).toEqual(['low', 'medium']);
    expect(localChoices([old], 'claude', [], '', '').known).toBe(false);
  });
  it('账号里没有这个模型、模型不支持这个强度的机器写明原因；没报过的不管', () => {
    const c = localChoices([mbp, desk, old], 'claude', [], 'sonnet', '');
    expect([...c.why]).toEqual([[1, '账号里没有 sonnet']]);
    expect([...localChoices([mbp, desk], 'claude', [], 'opus', 'max').why]).toEqual([[2, 'opus 不支持强度 max']]);
    expect([...localChoices([mbp], 'claude', [], 'haiku', 'high').why]).toEqual([[1, 'haiku 不能设强度']]);
  });
});
