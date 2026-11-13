/* agent 运行过程的结构化事件（会话「过程」，PRD §9.1「执行」，卡片 #184；服务端定义见 board-core runlog.rs）
   - 增量取：会话第一次全取，之后带上 `since`（上次的 runs）只拿新增、改过的事件，按 id 合进已有的
   - 画的时候：连续的读文件、搜索类工具调用折叠成一行（「读了 3 个文件、搜索 2 次」），连续的 text 合成一段 */
import type { RunEvent, SessionView } from './types';

type Part = SessionView['parts'][number];

/** 增量取到的结果合进已有的：派发记录、日志用新的；事件按 id 更新或追加，比 eventsFrom 早的（被裁掉了）丢掉 */
export function mergeSession(old: SessionView | null, inc: SessionView, incremental: boolean): SessionView {
  if (!old || !incremental || old.session !== inc.session) return inc;
  const parts = inc.parts.map(p => {
    const prev = old.parts.find(x => x.id === p.id);
    if (!prev) return p;
    return { ...p, events: mergeEvents(prev.events, p.events, p.eventsFrom) };
  });
  return { ...inc, parts };
}

export function mergeEvents(prev: RunEvent[], add: RunEvent[], from: number): RunEvent[] {
  // from = 0：服务端上这个派发一条事件都没有了
  const keep = prev.filter(e => from > 0 && e.id >= from);
  if (!add.length) return keep.length === prev.length ? prev : keep;
  const byId = new Map(keep.map(e => [e.id, e]));
  for (const e of add) byId.set(e.id, e);
  return [...byId.values()].sort((a, b) => a.id - b.id);
}

/** 工具的种类：图标、折叠用 */
export type ToolKind = 'read' | 'search' | 'edit' | 'shell' | 'web' | 'task' | 'todo' | 'other';

const KINDS: [ToolKind, RegExp][] = [
  ['read', /^(read|read_file|view|cat|notebookread|ls|list|list_dir|list_files)$/i],
  ['search', /^(grep|glob|search|find|rg|codebase_search|file_search|grep_search)$/i],
  ['edit', /^(edit|write|multiedit|notebookedit|apply_patch|write_file|edit_file|str_replace|create_file)$/i],
  ['shell', /^(bash|shell|exec|exec_command|run|terminal|powershell|cmd)$/i],
  ['web', /^(webfetch|websearch|web_search|web_fetch|fetch)$/i],
  ['task', /^(task|agent|subagent)$/i],
  ['todo', /^(todowrite|todo|update_plan|todo_write)$/i],
];

export function toolKind(name: string | undefined): ToolKind {
  const n = (name || '').trim();
  for (const [k, re] of KINDS) if (re.test(n)) return k;
  return 'other';
}

/** 工具调用（命令、读写文件、搜索、Skill、WebFetch……）：连续几次折叠成一行（default/main#288）。
    agent 说的话、原始输出、出错、结束不折进去，把工具组隔开 */
const quiet = (e: RunEvent) => e.kind === 'tool';

export type Row =
  | { type: 'message'; e: RunEvent }
  | { type: 'tool'; e: RunEvent }
  | { type: 'group'; events: RunEvent[] }
  | { type: 'error'; e: RunEvent }
  | { type: 'text'; events: RunEvent[] }
  | { type: 'result'; e: RunEvent };

/** 事件 → 要画的行。子 agent 里的事件和主线分开折叠 */
export function groupEvents(events: RunEvent[]): Row[] {
  const out: Row[] = [];
  for (const e of events) {
    const last = out[out.length - 1];
    if (e.kind === 'text') {
      if (last?.type === 'text' && !!last.events[0].sub === !!e.sub) last.events.push(e);
      else out.push({ type: 'text', events: [e] });
      continue;
    }
    if (quiet(e)) {
      if (last?.type === 'group' && !!last.events[0].sub === !!e.sub) last.events.push(e);
      else if (last?.type === 'tool' && quiet(last.e) && !!last.e.sub === !!e.sub) out[out.length - 1] = { type: 'group', events: [last.e, e] };
      else out.push({ type: 'tool', e });
      continue;
    }
    if (e.kind === 'message') out.push({ type: 'message', e });
    else if (e.kind === 'tool') out.push({ type: 'tool', e });
    else if (e.kind === 'error') out.push({ type: 'error', e });
    else if (e.kind === 'result') out.push({ type: 'result', e });
    else out.push({ type: 'text', events: [e] });
  }
  return out;
}

const KIND_WORDS: [ToolKind, (n: number) => string][] = [
  ['shell', n => `运行了 ${n} 条命令`],
  ['read', n => `读了 ${n} 个文件`],
  ['search', n => `搜索 ${n} 次`],
  ['edit', n => `改了 ${n} 个文件`],
  ['web', n => `查了 ${n} 次网页`],
  ['task', n => `派了 ${n} 个子 agent`],
  ['todo', n => `更新了 ${n} 次待办`],
];

/** 折叠的一行写什么：「运行了 5 条命令、读了 3 个文件、搜索 2 次（1 次出错）」（读同一个文件几次算几个）；
    认不出种类的写工具名：「调用 Skill、ToolSearch 3 次」，多于两种时「调用 Skill、ToolSearch 等 4 种工具 6 次」 */
export function groupWords(events: RunEvent[]): string {
  const parts: string[] = [];
  for (const [k, words] of KIND_WORDS) {
    const n = events.filter(e => toolKind(e.tool) === k).length;
    if (n) parts.push(words(n));
  }
  const others = events.filter(e => toolKind(e.tool) === 'other');
  if (others.length) {
    const names = [...new Set(others.map(e => (e.tool || '工具').trim() || '工具'))];
    parts.push(names.length <= 2 ? `调用 ${names.join('、')} ${others.length} 次` : `调用 ${names.slice(0, 2).join('、')} 等 ${names.length} 种工具 ${others.length} 次`);
  }
  const failed = events.filter(e => e.ok === false).length;
  return parts.join('、') + (failed ? `（${failed} 次出错）` : '');
}

/** 折叠组的图标：组里最多的那一种（一样多时取先出现的） */
export function groupKind(events: RunEvent[]): ToolKind {
  const n = new Map<ToolKind, number>();
  for (const e of events) n.set(toolKind(e.tool), (n.get(toolKind(e.tool)) ?? 0) + 1);
  let best: ToolKind = 'other';
  let max = 0;
  for (const [k, c] of n) if (c > max) [best, max] = [k, c];
  return best;
}

/** 正在跑的那一条：还没有结果的最后一次调用；会话不在跑时为空 */
export function runningTool(events: RunEvent[], running: boolean): RunEvent | null {
  if (!running) return null;
  for (let i = events.length - 1; i >= 0; i--) if (events[i].ok == null) return events[i];
  return null;
}

/** 一个会话的累计用量：各张卡的结束事件加起来；agent 输出里都没有时为 null */
export function totals(parts: Part[]): { tokensIn?: number; tokensOut?: number; cost?: number } | null {
  let tokensIn: number | undefined;
  let tokensOut: number | undefined;
  let cost: number | undefined;
  for (const p of parts)
    for (const e of p.events || []) {
      if (e.kind !== 'result') continue;
      if (e.tokensIn != null) tokensIn = (tokensIn ?? 0) + e.tokensIn;
      if (e.tokensOut != null) tokensOut = (tokensOut ?? 0) + e.tokensOut;
      if (e.cost != null) cost = (cost ?? 0) + e.cost;
    }
  if (tokensIn == null && tokensOut == null && cost == null) return null;
  return { tokensIn, tokensOut, cost };
}

/** tokens：980、12.3k、1.2M */
export function tokenWords(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0).replace(/\.0$/, '')}k`;
  return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 1 : 0).replace(/\.0$/, '')}M`;
}

/** 费用：$0.42、$3.12、$0.004 */
export function costWords(c: number): string {
  if (c > 0 && c < 0.01) return `$${c.toFixed(3)}`;
  return `$${c.toFixed(2)}`;
}

/** 用量的一句话：「输入 1.2M · 输出 35k tokens · $3.12」 */
export function usageWords(u: { tokensIn?: number; tokensOut?: number; cost?: number }): string {
  const out: string[] = [];
  if (u.tokensIn != null) out.push(`输入 ${tokenWords(u.tokensIn)}`);
  if (u.tokensOut != null) out.push(`输出 ${tokenWords(u.tokensOut)}`);
  let s = out.join(' · ');
  if (s) s += ' tokens';
  if (u.cost != null) s = s ? `${s} · ${costWords(u.cost)}` : costWords(u.cost);
  return s;
}

/** 结束事件的文字和这一段最后一句话一样（Claude Code 的 result 就是最后的回答）时不再重复显示 */
export function resultText(e: RunEvent, events: RunEvent[]): string {
  const t = (e.text || '').trim();
  if (!t) return '';
  const lastMsg = [...events.slice(0, events.indexOf(e))].reverse().find(x => x.kind === 'message');
  return lastMsg && (lastMsg.text || '').trim() === t ? '' : t;
}
