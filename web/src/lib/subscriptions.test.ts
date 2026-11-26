import { describe, expect, it } from 'vitest';
import { accountsOf, ASK_WAIT, toneOf, WIN_SPAN, type QuotaWin } from './subscriptions';
import type { HostAgent, HostInfo } from './types';

const now = Date.UTC(2026, 10, 25, 20, 0);
const MIN = 60_000;
const HOUR = 60 * MIN;

const claude = (email: string | null, x: Partial<HostAgent> = {}): HostAgent => ({
  id: 'claude',
  name: 'Claude Code',
  version: '2.1',
  found: true,
  auth: email == null ? { kind: 'api_key', label: 'Anthropic API key' } : { kind: 'account', label: 'Claude Max', plan: 'max', email },
  quota_probe: true,
  ...x,
});
const host = (name: string, online: boolean, agents: HostAgent[]): HostInfo => ({ name, online, agents }) as unknown as HostInfo;
const wins = (five: number, week: number) => [
  { key: 'five_hour', used: five, resetsAt: now + 2 * HOUR },
  { key: 'seven_day', used: week, resetsAt: now + 3 * 24 * HOUR },
];

describe('官方订阅：按账号汇总', () => {
  it('同一个 agent、同一个邮箱（不区分大小写）的机器是一个账号；API key、没报邮箱的不算', () => {
    const list = accountsOf(
      [
        host('mbp', true, [claude('a@example.com')]),
        host('desk', false, [claude('A@Example.com')]),
        host('lap', true, [claude(null)]),
        host('mini', true, [claude('b@example.com')]),
      ],
      now,
    );
    expect(list.map(a => [a.id, a.hosts.map(h => h.name)])).toEqual([
      ['claude:a@example.com', ['mbp', 'desk']],
      ['claude:b@example.com', ['mini']],
    ]);
    expect(list[0].label).toBe('Claude Max');
  });

  it('额度取所有机器里最近看到的一次；两个窗口顺序固定，没看到的是空', () => {
    const [a] = accountsOf(
      [
        host('mbp', true, [claude('a@example.com', { quota: { at: now - 30 * MIN, windows: wins(10, 20) } })]),
        host('desk', true, [claude('a@example.com', { quota: { at: now - 5 * MIN, windows: wins(43, 58) } })]),
      ],
      now,
    );
    expect(a.seenAt).toBe(now - 5 * MIN);
    expect(a.wins.map(w => [w.key, w.used])).toEqual([
      ['five_hour', 43],
      ['seven_day', 58],
    ]);
    const [b] = accountsOf([host('mbp', true, [claude('a@example.com')])], now);
    expect(b.seenAt).toBeNull();
    expect(b.wins.map(w => w.used)).toEqual([null, null]);
  });

  it('旧的一个窗口的额度也认；过了重置时间的按 0% 画、写已重置', () => {
    const [a] = accountsOf([host('mbp', true, [claude('a@example.com', { quota: { at: now - 6 * HOUR, window: 'five_hour', used: 91, resetsAt: now - HOUR } })])], now);
    expect(a.wins[0]).toMatchObject({ key: 'five_hour', used: 0, resetsAt: null, reset: true });
    expect(a.wins[1].used).toBeNull();
  });

  it('暂停：取最晚的，恢复发给暂停着的那台', () => {
    const [a] = accountsOf(
      [host('mbp', true, [claude('a@example.com', { quota_until: now + HOUR })]), host('desk', true, [claude('a@example.com', { quota_until: now + 2 * HOUR })])],
      now,
    );
    expect([a.pausedUntil, a.pausedHost]).toEqual([now + 2 * HOUR, 'desk']);
    // 已经过了的不算
    const [b] = accountsOf([host('mbp', true, [claude('a@example.com', { quota_until: now - MIN })])], now);
    expect(b.pausedUntil).toBeNull();
  });

  it('刷新：找在线、会查的机器；都不在线、版本太旧时写原因', () => {
    const [a] = accountsOf([host('mbp', false, [claude('a@example.com')]), host('desk', true, [claude('a@example.com')])], now);
    expect([a.probeHost, a.probeWhy]).toEqual(['desk', null]);
    const [b] = accountsOf([host('mbp', false, [claude('a@example.com')])], now);
    expect([b.probeHost, b.probeWhy]).toEqual([null, '登着这个账号的机器都不在线']);
    const [c] = accountsOf([host('mbp', true, [claude('a@example.com', { quota_probe: false })])], now);
    expect(c.probeWhy).toBe('board 版本太旧，升级后才能查额度');
  });

  it('正在查：点的刷新比查到、没查到的都新，最多等 2 分钟；没查到的原因比额度新才提', () => {
    const q = { at: now - 10 * MIN, windows: wins(10, 20) };
    const ask = (quota_ask: number, x: Partial<HostAgent> = {}) => accountsOf([host('mbp', true, [claude('a@example.com', { quota: q, quota_ask, ...x })])], now)[0];
    expect(ask(now - MIN).asking).toBe(true);
    expect(ask(now - ASK_WAIT - MIN).asking).toBe(false);
    expect(ask(now - 20 * MIN).asking).toBe(false);
    const failed = ask(now - 2 * MIN, { quota_fail: { at: now - MIN, why: '60 秒内没查完' } });
    expect([failed.asking, failed.fail]).toEqual([false, '60 秒内没查完']);
    expect(ask(now - 30 * MIN, { quota_fail: { at: now - 20 * MIN, why: '旧的' } }).fail).toBeNull();
  });

  it('色调：用完、快用完、用得比时间快、照常', () => {
    const w = (used: number, left: number, rejected = false): QuotaWin => ({ key: 'five_hour', used, resetsAt: now + left, reset: false, rejected });
    const span = WIN_SPAN.five_hour;
    expect(toneOf(w(100, HOUR), now)).toBe('out');
    expect(toneOf(w(60, HOUR, true), now)).toBe('out');
    expect(toneOf(w(90, HOUR), now)).toBe('high');
    // 过了 20% 的时间用了 62%
    expect(toneOf(w(62, span * 0.8), now)).toBe('fast');
    // 过了 80% 的时间用了 62%
    expect(toneOf(w(62, span * 0.2), now)).toBe('ok');
  });
});
