import { describe, expect, it } from 'vitest';
import { costWords, groupEvents, groupKind, groupWords, mergeEvents, runningTool, mergeSession, resultText, tokenWords, toolKind, totals, usageWords } from './runEvents';
import type { RunEvent, SessionView } from './types';

const ev = (id: number, kind: RunEvent['kind'], more: Partial<RunEvent> = {}): RunEvent => ({ id, kind, ...more });
const tool = (id: number, name: string, more: Partial<RunEvent> = {}) => ev(id, 'tool', { tool: name, args: `a${id}`, ...more });

describe('增量取的合并', () => {
  it('按 id 更新、追加，排好序', () => {
    const prev = [ev(1, 'message', { text: '先领卡' }), tool(2, 'Bash')];
    const next = mergeEvents(prev, [tool(2, 'Bash', { ok: true, out: 'ok' }), ev(3, 'result')], 1);
    expect(next.map(e => e.id)).toEqual([1, 2, 3]);
    expect(next[1].ok).toBe(true);
    expect(next[0]).toBe(prev[0]);
  });
  it('被裁掉的（比 eventsFrom 早的）丢掉；没变化时原样返回', () => {
    const prev = [ev(1, 'text'), ev(2, 'text'), ev(3, 'text')];
    expect(mergeEvents(prev, [], 1)).toBe(prev);
    expect(mergeEvents(prev, [ev(4, 'text')], 3).map(e => e.id)).toEqual([3, 4]);
    expect(mergeEvents(prev, [], 0)).toEqual([]);
  });
  it('会话：日志、派发记录用新的，事件合并；第一次取（不是增量）整份换掉', () => {
    const part = (events: RunEvent[], log: string[], eventsFrom = 1) =>
      ({ id: 7, state: 'running', log, events, eventsFrom, eventsDropped: 0 }) as unknown as SessionView['parts'][number];
    const old: SessionView = { session: 7, parts: [part([ev(1, 'message')], ['a'])], runs: 3 };
    const inc: SessionView = { session: 7, parts: [part([ev(2, 'text')], ['a', 'b'])], runs: 4 };
    const m = mergeSession(old, inc, true);
    expect(m.parts[0].log).toEqual(['a', 'b']);
    expect(m.parts[0].events.map(e => e.id)).toEqual([1, 2]);
    expect(m.runs).toBe(4);
    expect(mergeSession(old, inc, false)).toBe(inc);
  });
});

describe('画成行', () => {
  it('连续的工具调用（不只读文件、搜索）折叠成一行；说的话、原始输出、出错、结束把工具组隔开', () => {
    const rows = groupEvents([
      ev(1, 'message', { text: '看看代码' }),
      tool(2, 'Read'),
      tool(3, 'Grep'),
      tool(4, 'Read', { ok: false }),
      tool(5, 'Bash'),
      tool(6, 'Skill'),
      ev(7, 'text', { text: 'warning: a' }),
      ev(8, 'text', { text: 'warning: b' }),
      tool(9, 'PowerShell'),
      ev(10, 'message', { text: '改好了' }),
      tool(11, 'Edit'),
      ev(12, 'error', { text: 'rate_limit' }),
      ev(13, 'result', { ok: true }),
    ]);
    expect(rows.map(r => r.type)).toEqual(['message', 'group', 'text', 'tool', 'message', 'tool', 'error', 'result']);
    const g = rows[1];
    expect(g.type === 'group' && g.events.map(e => e.id)).toEqual([2, 3, 4, 5, 6]);
    expect(g.type === 'group' && groupWords(g.events)).toBe('运行了 1 条命令、读了 2 个文件、搜索 1 次、调用 Skill 1 次（1 次出错）');
    const t = rows[2];
    expect(t.type === 'text' && t.events.length).toBe(2);
  });
  it('折叠行的文字：各类几次；认不出种类的写工具名', () => {
    const evs = [tool(1, 'Bash'), tool(2, 'Bash'), tool(3, 'Read'), tool(4, 'WebFetch'), tool(5, 'Write'), tool(6, 'TodoWrite'), tool(7, 'Task')];
    expect(groupWords(evs)).toBe('运行了 2 条命令、读了 1 个文件、改了 1 个文件、查了 1 次网页、派了 1 个子 agent、更新了 1 次待办');
    expect(groupWords([tool(1, 'Skill'), tool(2, 'ToolSearch'), tool(3, 'Skill')])).toBe('调用 Skill、ToolSearch 3 次');
    expect(groupWords([tool(1, 'Skill'), tool(2, 'ToolSearch'), tool(3, 'mcp__x__y'), tool(4, 'Skill')])).toBe('调用 Skill、ToolSearch 等 3 种工具 4 次');
  });
  it('折叠组的图标按最多的那种；在跑时找出正在跑的那一条', () => {
    expect(groupKind([tool(1, 'Read'), tool(2, 'Bash'), tool(3, 'Bash')])).toBe('shell');
    expect(groupKind([tool(1, 'Read'), tool(2, 'Bash')])).toBe('read');
    const evs = [tool(1, 'Read', { ok: true }), tool(2, 'Bash', { ok: true }), tool(3, 'Bash')];
    expect(runningTool(evs, true)?.id).toBe(3);
    expect(runningTool(evs, false)).toBeNull();
    expect(runningTool([tool(1, 'Read', { ok: true })], true)).toBeNull();
  });
  it('子 agent 里的不和主线折在一起', () => {
    const rows = groupEvents([tool(1, 'Read'), tool(2, 'Read', { sub: true }), tool(3, 'Read', { sub: true })]);
    expect(rows.map(r => r.type)).toEqual(['tool', 'group']);
  });
  it('工具种类：各个 agent 的工具名', () => {
    expect([toolKind('Read'), toolKind('read'), toolKind('read_file'), toolKind('Grep'), toolKind('shell'), toolKind('apply_patch'), toolKind('docs/search'), toolKind(undefined)]).toEqual([
      'read',
      'read',
      'read',
      'search',
      'shell',
      'edit',
      'other',
      'other',
    ]);
  });
});

describe('用量', () => {
  const part = (events: RunEvent[]) => ({ events }) as unknown as SessionView['parts'][number];
  it('各张卡的结束事件加起来；都没有时为 null', () => {
    const t = totals([part([ev(1, 'result', { tokensIn: 1000, tokensOut: 20, cost: 0.5 })]), part([ev(2, 'result', { tokensIn: 500, cost: 1.25 }), ev(3, 'message')])]);
    expect(t).toEqual({ tokensIn: 1500, tokensOut: 20, cost: 1.75 });
    expect(totals([part([ev(1, 'result', { ok: true })])])).toBeNull();
    expect(usageWords(t!)).toBe('输入 1.5k · 输出 20 tokens · $1.75');
    expect(usageWords({ tokensIn: 24763, tokensOut: 122 })).toBe('输入 25k · 输出 122 tokens');
    expect(usageWords({ cost: 0.004 })).toBe('$0.004');
  });
  it('数字的写法', () => {
    expect([tokenWords(980), tokenWords(1000), tokenWords(12_345), tokenWords(1_234_567), tokenWords(12_000_000)]).toEqual(['980', '1k', '12k', '1.2M', '12M']);
    expect([costWords(3.1234), costWords(0)]).toEqual(['$3.12', '$0.00']);
  });
  it('结束的文字和最后一句话一样时不重复', () => {
    const evs = [ev(1, 'message', { text: '改好了' }), ev(2, 'result', { text: '改好了' }), ev(3, 'message', { text: 'x' }), ev(4, 'result', { text: '别的' })];
    expect(resultText(evs[1], evs)).toBe('');
    expect(resultText(evs[3], evs)).toBe('别的');
  });
});
