import { describe, expect, it } from 'vitest';
import { agentTake, claimable, hhmm, hostTake, inWins, nextStart, ruleAgents, serverMinute, tidyWins, winWords, type Win } from './sched';
import type { BoardInfo, ClaimRule, HostAgent, HostInfo } from './types';

const agent = (id: string, x: Partial<HostAgent> = {}): HostAgent => ({ id, name: id, version: '1', found: true, ...x });
const host = (x: Partial<HostInfo> = {}): HostInfo =>
  ({ id: 1, name: 'mac', os: 'macOS', version: '1', agents: [agent('claude')], envs: [], dirs: [], max: 2, paused: false, agent: null, cronOn: false, cron: [], round: false, online: true, firstAt: 0, beatAt: 0, offAt: null, auto: true, windows: [[0, 1440]], ...x }) as HostInfo;

describe('接单时段', () => {
  it('写成一句：全天、不接单、跨零点的两段并成一句', () => {
    expect(winWords([[0, 1440]])).toBe('全天');
    expect(winWords([])).toBe('不接单');
    expect(winWords([[1320, 1440], [0, 480]])).toBe('22:00–08:00');
    expect(winWords([[540, 1080]])).toBe('09:00–18:00');
    expect(winWords([[0, 480], [720, 840], [1320, 1440]])).toBe('12:00–14:00、22:00–08:00');
    expect(hhmm(570)).toBe('09:30');
  });

  it('整理：去掉空的、排好、挨着和重叠的并成一段（重复执行结果相同）', () => {
    const messy: Win[] = [[720, 1080], [540, 720], [600, 660], [300, 300]];
    expect(tidyWins(messy)).toEqual([[540, 1080]]);
    expect(tidyWins(tidyWins(messy))).toEqual([[540, 1080]]);
    expect(tidyWins([])).toEqual([]);
  });

  it('在不在时段里（止是开区间），下一次什么时候开始', () => {
    const night: Win[] = [[0, 480], [1320, 1440]];
    expect([0, 479, 480, 860, 1320].map(m => inWins(night, m))).toEqual([true, true, false, false, true]);
    expect(nextStart(night, 860)).toBe('22:00');
    expect(nextStart([[540, 1080]], 1200)).toBe('09:00');
    expect(nextStart([], 100)).toBeNull();
  });

  it('服务端现在是第几分钟：取数据时的值加上之后过去的时间，过了零点绕回去；钟差得离谱时用取数据时的值', () => {
    const ws = { now: 1_000_000, sched: { rules: [], minute: 1430 } };
    expect(serverMinute(ws, 1_000_000)).toBe(1430);
    expect(serverMinute(ws, 1_000_000 + 20 * 60_000)).toBe(10);
    expect(serverMinute(ws, 1_000_000 + 20 * 3600_000)).toBe(1430);
    expect(serverMinute(null, 5)).toBe(0);
  });
});

describe('机器现在接不接单', () => {
  it('按顺序给出原因：离线、没开、暂停、没有时段、时段外、满了；都没有就是空位', () => {
    expect(hostTake(host({ online: false }), 0, 600)).toMatchObject({ ok: false, why: '离线', tone: 'off' });
    expect(hostTake(host({ auto: false }), 0, 600)).toMatchObject({ why: '只做派给它的', tone: 'off' });
    expect(hostTake(host({ auto: false, paused: true }), 0, 600).why).toBe('暂停接单');
    expect(hostTake(host({ paused: true }), 0, 600)).toMatchObject({ why: '暂停接单', tone: 'wait' });
    expect(hostTake(host({ windows: [] }), 0, 600)).toMatchObject({ why: '没有接单时段', tone: 'off' });
    expect(hostTake(host({ windows: [[1320, 1440], [0, 480]] }), 0, 860)).toMatchObject({ why: '时段外，22:00 开始', tone: 'wait' });
    expect(hostTake(host(), 2, 600)).toMatchObject({ why: '2 个会话在跑，已满', tone: 'wait' });
    expect(hostTake(host(), 1, 600)).toEqual({ ok: true, why: '空位 1（1/2 在跑）', tone: 'ok' });
    // 旧版服务端没有时段：当全天
    expect(hostTake(host({ windows: undefined }), 0, 600).ok).toBe(true);
  });

  it('分 agent：没有它、没找到命令、连续异常退出暂停了都接不了', () => {
    const h = host({ agents: [agent('claude'), agent('pi', { found: false }), agent('dsh', { tripped: true })] });
    expect(agentTake(h, 'codex', 0, 600)).toMatchObject({ ok: false, why: '没有这个 agent' });
    expect(agentTake(h, 'pi', 0, 600)).toMatchObject({ ok: false, why: '没找到命令' });
    expect(agentTake(h, 'dsh', 0, 600)).toMatchObject({ ok: false, why: '连续异常退出，已暂停' });
    expect(agentTake(h, 'claude', 0, 600)).toEqual({ ok: true, why: '空位 2', tone: 'ok' });
    expect(agentTake(host({ auto: false }), 'claude', 0, 600).why).toBe('只做派给它的');
  });
});

describe('规则和白板', () => {
  const rule = (agent: string): ClaimRule => ({ op: 'in', labels: ['x'], agent });
  it('规则里的 agent 按第一次出现的先后，兜底的排在最后', () => {
    expect(ruleAgents([rule('dsh'), rule('claude'), rule('dsh')], 'pi')).toEqual(['dsh', 'claude', 'pi']);
    expect(ruleAgents([rule('dsh')], 'dsh')).toEqual(['dsh']);
    expect(ruleAgents([], null)).toEqual([]);
  });

  it('白板上现在能领的卡片数：只数可领取状态', () => {
    const b = { statuses: [{ key: 'todo', claim: true }, { key: 'rework', claim: true }, { key: 'doing', claim: false }], counts: { todo: 3, rework: 1, doing: 5, inbox: 9 } } as unknown as BoardInfo;
    expect(claimable(b)).toBe(4);
  });
});
