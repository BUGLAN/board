/* ==========================================================================
   复盘台（PRD §10.14，卡片 #194）用到的纯函数
   - 状态的角色：交审状态（锁定状态连出去、等人审核的）、需要人处理的状态（错误节点，和名字里带「人工」这类关键词的），
     规则和服务端统计（board-core stats.rs Roles）、board-skills「认出角色」一致
   - 队列：这几个状态里的卡片按进入当前状态的时间从早到晚排
   - 去向：通过去这个状态连向的终点状态，打回去连向的可领取状态（都没有连线时退回全部终点 / 首个可领取状态），
     回复并放回去首个可领取状态
   - agent 最近一次交审评论：拆出「提交：」「集成分支：」「检查：」几行单独高亮
   - 描述里的任务列表：点一项就翻转第 n 个 `- [ ]` / `- [x]`（代码块里的不算）
   ========================================================================== */
import type { Activity, BoardSnapshot, Status } from './types';

/** 交审状态名称里的关键词 */
const REVIEW_WORDS = ['审核', '审查', '评审', 'review'];
/** 需要人处理的状态名称里的关键词 */
const HUMAN_WORDS = ['人工', '人处理', '人确认', '需人', 'check', 'manual', 'human'];

const has = (s: Status, words: string[]) => {
  const name = s.name.toLowerCase();
  const key = s.key.toLowerCase();
  return words.some(w => name.includes(w) || key.includes(w));
};

/** 每个状态的角色：review 交审（复盘），human 需要人处理 */
export function statusRoles(statuses: Status[] | null | undefined): { review: Set<string>; human: Set<string> } {
  const review = new Set<string>();
  const human = new Set<string>();
  const list = statuses ?? [];
  for (const s of list) {
    const fromLock = list.some(l => l.lock && l.to.includes(s.key));
    const plain = !s.claim && !s.lock && !s.final && !s.error;
    if (s.error || (plain && fromLock && has(s, HUMAN_WORDS))) human.add(s.key);
    else if (plain && fromLock) review.add(s.key);
  }
  return { review, human };
}

/** 交审状态里名字像「审核」的排前面（只是为了从大盘进来时先看最像交审的那个） */
export const looksLikeReview = (s: Status) => has(s, REVIEW_WORDS);

/** 复盘台的队列：这几个状态里的卡片，按进入当前状态的时间从早到晚 */
export function deskQueue(board: BoardSnapshot | null | undefined, cols: string[]): number[] {
  if (!board) return [];
  const ids = cols.flatMap(k => board.cols[k] ?? []).filter(id => board.cards[id]);
  const at = (id: number) => board.cards[id].movedAt ?? board.cards[id].createdAt ?? 0;
  return [...ids].sort((a, b) => at(a) - at(b) || a - b);
}

/** 通过：这个状态连向的终点状态；没有连线时是全部终点状态 */
export function approveTargets(statuses: Status[], col: string): Status[] {
  const from = statuses.find(s => s.key === col);
  const linked = statuses.filter(s => s.final && from?.to.includes(s.key));
  return linked.length ? linked : statuses.filter(s => s.final);
}

/** 打回：这个状态连向的可领取状态（默认工作流里是「返工」）；没有连线时是首个可领取状态 */
export function rejectTargets(statuses: Status[], col: string): Status[] {
  const from = statuses.find(s => s.key === col);
  const linked = statuses.filter(s => s.claim && from?.to.includes(s.key));
  if (linked.length) return linked;
  const home = statuses.find(s => s.claim);
  return home ? [home] : [];
}

/** 回复并放回：首个可领取状态 */
export const homeOf = (statuses: Status[]) => statuses.find(s => s.claim) ?? null;

/** agent 最近一次评论（交审评论、做不下去时留的问题）：动态里最后一条 agent 发的评论 */
export function latestAgentComment(activity: Activity[] | null | undefined): Activity | null {
  const list = activity ?? [];
  for (let i = list.length - 1; i >= 0; i--) {
    const a = list[i];
    if (a.kind === 'comment' && a.actor.kind === 'agent') return a;
  }
  return null;
}

/** 交审评论末尾固定的几行（提交、集成分支、检查）：拆出来单独高亮，其余的照 Markdown 画 */
const FACT = /^\s*(提交|集成分支|检查)\s*[：:]\s*(.*)$/;
export function splitFacts(text: string): { body: string; facts: { key: string; value: string }[] } {
  const facts: { key: string; value: string }[] = [];
  const rest: string[] = [];
  let fence = false;
  for (const line of text.split('\n')) {
    if (/^\s*(```|~~~)/.test(line)) fence = !fence;
    const m = !fence && line.match(FACT);
    if (m) facts.push({ key: m[1], value: m[2].trim() });
    else rest.push(line);
  }
  return { body: rest.join('\n').trim(), facts };
}

/** 任务列表的一项：`- [ ] …`、`* [x] …`、`1. [ ] …`（行首可以缩进） */
const TASK = /^(\s*(?:>\s*)*(?:[-*+]|\d+[.)])\s+\[)([ xX])(\].*)$/;

/** 翻转描述里第 n 个（从 0 数）任务列表项的勾选；代码块里的不算；没有第 n 个时原样返回 */
export function toggleTask(body: string, n: number): string {
  const lines = body.split('\n');
  let fence = false;
  let i = 0;
  for (let k = 0; k < lines.length; k++) {
    if (/^\s*(```|~~~)/.test(lines[k])) {
      fence = !fence;
      continue;
    }
    if (fence) continue;
    const m = lines[k].match(TASK);
    if (!m) continue;
    if (i === n) {
      lines[k] = `${m[1]}${m[2] === ' ' ? 'x' : ' '}${m[3]}`;
      return lines.join('\n');
    }
    i++;
  }
  return body;
}

/** 描述里勾了几项、一共几项 */
export function taskCount(body: string): { done: number; total: number } {
  let fence = false;
  let done = 0;
  let total = 0;
  for (const line of body.split('\n')) {
    if (/^\s*(```|~~~)/.test(line)) fence = !fence;
    const m = !fence && line.match(TASK);
    if (m) {
      total++;
      if (m[2] !== ' ') done++;
    }
  }
  return { done, total };
}
