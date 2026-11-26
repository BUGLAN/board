/* ==========================================================================
   「Agent → 模型供应商」页的「官方订阅」：按账号汇总机器上报的身份和额度
   - 账号 = agent + 身份里的邮箱（不区分大小写，#669）；只算订阅（kind = account）、报了邮箱的
   - 额度取这个账号所有机器里最近看到的一次：5 小时、本周两个窗口。来源是跑卡时 agent 推的，或者点「刷新」查的
   - 暂停（额度用完）同一账号的机器一起停、一起恢复（服务端做），这里取最晚的
   ========================================================================== */
import type { AgentQuota, HostInfo } from './types';

export type WinKey = 'five_hour' | 'seven_day';

export interface QuotaWin {
  key: WinKey;
  /** 用了百分之几；没看到为 null */
  used: number | null;
  /** 什么时候重置（毫秒）；没看到、或已经重置了为 null */
  resetsAt: number | null;
  /** 看到之后已经过了重置时间：按 0% 画，写「已重置」 */
  reset: boolean;
  /** Claude 报的这个窗口用完了 */
  rejected: boolean;
}

export interface SubAccount {
  /** agent:邮箱（小写） */
  id: string;
  agent: string;
  /** 「Claude Max」「ChatGPT Plus」 */
  label: string;
  email: string;
  /** 登着这个账号的机器 */
  hosts: { name: string; online: boolean }[];
  /** 最近一次看到额度的时间（服务端收到的） */
  seenAt: number | null;
  /** 5 小时、本周（总是这两个，顺序固定） */
  wins: QuotaWin[];
  /** 额度用完，暂停到什么时候（没暂停为 null） */
  pausedUntil: number | null;
  /** 暂停着的一台机器：点「恢复」发给它（服务端把同一账号的一起恢复） */
  pausedHost: string | null;
  /** 点「刷新」时让哪台机器去查：在线、会查的第一台 */
  probeHost: string | null;
  /** 刷新不了的原因 */
  probeWhy: string | null;
  /** 正在查：点了刷新、还没查到（最多等 2 分钟） */
  asking: boolean;
  /** 上一次刷新没查到的原因（比最近看到的额度新时才有） */
  fail: string | null;
}

/** 窗口的长度（毫秒）：算时间过去了多少 */
export const WIN_SPAN: Record<WinKey, number> = { five_hour: 5 * 3600_000, seven_day: 7 * 24 * 3600_000 };
export const WIN_LABEL: Record<WinKey, string> = { five_hour: '5 小时', seven_day: '本周' };
const KEYS: WinKey[] = ['five_hour', 'seven_day'];
/** 点了「刷新」最多等多久还算「正在查」 */
export const ASK_WAIT = 2 * 60_000;

/** 一次额度里的两个窗口：有 windows 用它，旧的只有一个窗口（five_hour / seven_day）的也认 */
function winsOf(q: AgentQuota | null | undefined, now: number): QuotaWin[] {
  return KEYS.map(key => {
    const w = q?.windows?.find(x => x.key === key) ?? (q && q.window === key ? { key, used: q.used, resetsAt: q.resetsAt, status: q.status } : null);
    const used = w?.used ?? null;
    const resetsAt = w?.resetsAt ?? null;
    const reset = used != null && resetsAt != null && resetsAt <= now;
    return { key, used: reset ? 0 : used, resetsAt: reset ? null : resetsAt, reset, rejected: !reset && w?.status === 'rejected' };
  });
}

export function accountsOf(hosts: HostInfo[], now: number): SubAccount[] {
  const out: SubAccount[] = [];
  const byId = new Map<string, { acct: SubAccount; quota: AgentQuota | null; fail: { at: number; why: string } | null }>();
  for (const h of hosts) {
    for (const a of h.agents) {
      const email = a.auth?.email?.trim();
      if (a.auth?.kind !== 'account' || !email || !a.found) continue;
      const id = `${a.id}:${email.toLowerCase()}`;
      let e = byId.get(id);
      if (!e) {
        const acct: SubAccount = {
          id,
          agent: a.id,
          label: a.auth.label?.trim() || '订阅',
          email,
          hosts: [],
          seenAt: null,
          wins: [],
          pausedUntil: null,
          pausedHost: null,
          probeHost: null,
          probeWhy: null,
          asking: false,
          fail: null,
        };
        e = { acct, quota: null, fail: null };
        byId.set(id, e);
        out.push(acct);
      }
      const acct = e.acct;
      acct.hosts.push({ name: h.name, online: h.online });
      // 最近看到的一次额度
      if (a.quota && (!e.quota || a.quota.at > e.quota.at)) e.quota = a.quota;
      // 暂停：取最晚的
      if (a.quota_until && a.quota_until > now && (!acct.pausedUntil || a.quota_until > acct.pausedUntil)) {
        acct.pausedUntil = a.quota_until;
        acct.pausedHost = h.name;
      }
      // 刷新：在线、会查的第一台
      if (!acct.probeHost && h.online && a.quota_probe) acct.probeHost = h.name;
      // 正在查：点的刷新比这台机器最近查到（或没查到）的都新
      const seen = Math.max(a.quota?.at ?? 0, a.quota_fail?.at ?? 0);
      if (a.quota_ask && a.quota_ask > seen && now - a.quota_ask < ASK_WAIT) acct.asking = true;
      if (a.quota_fail && (!e.fail || a.quota_fail.at > e.fail.at)) e.fail = a.quota_fail;
    }
  }
  for (const { acct, quota, fail } of byId.values()) {
    acct.seenAt = quota?.at ?? null;
    acct.wins = winsOf(quota, now);
    // 没查到：比最近看到的额度新才提
    acct.fail = fail && fail.at > (quota?.at ?? 0) ? fail.why : null;
    if (!acct.probeHost) {
      const online = acct.hosts.some(h => h.online);
      acct.probeWhy = online ? 'board 版本太旧，升级后才能查额度' : '登着这个账号的机器都不在线';
    }
  }
  return out;
}

/** 时间过去了多少（0–1）；没有重置时间时为 null */
export const elapsedOf = (w: QuotaWin, now: number) => (w.resetsAt == null ? null : Math.min(1, Math.max(0, 1 - (w.resetsAt - now) / WIN_SPAN[w.key])));

/** ok 照常；fast 用得比时间快；high 快用完（85% 以上）；out 用完；none 没看到 */
export function toneOf(w: QuotaWin, now: number): 'none' | 'ok' | 'fast' | 'high' | 'out' {
  if (w.used == null) return 'none';
  if (w.rejected || w.used >= 100) return 'out';
  if (w.used >= 85) return 'high';
  const e = elapsedOf(w, now);
  if (e != null && w.used >= 40 && w.used / 100 > e + 0.2) return 'fast';
  return 'ok';
}
