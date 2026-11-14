/* ==========================================================================
   「未整理」里的拆解组（default/main#285）
   - 原条目拆出卡之后（白板上有「拆自」是它的卡，不含回收站里的），它不再单独占一行：
     拆出的、还在未整理里的卡成组显示，组的开头写「拆自 #ID 标题」
   - 拆出的卡都还在未整理里时可以「撤销拆解」：删掉拆出的卡，原条目回到列表里
   - 拆出的卡整理走了，原条目照样隐藏（计数也不算它）
   都是纯函数：输入白板上的卡片，输出隐藏哪些、怎么成组
   ========================================================================== */
import type { Card } from './types';

const INBOX = 'inbox';

/** 白板上每张卡片拆出了哪些卡（按编号）。只算白板上的（回收站里的不在 cards 里） */
export function kidsOf(cards: Record<number, Card> | undefined): Map<number, number[]> {
  const m = new Map<number, number[]>();
  for (const c of Object.values(cards ?? {})) {
    if (c.splitFrom == null || c.splitFrom === c.id) continue;
    m.set(c.splitFrom, [...(m.get(c.splitFrom) ?? []), c.id]);
  }
  for (const v of m.values()) v.sort((a, b) => a - b);
  return m;
}

/** 「未整理」里隐藏的原条目：已经拆出了卡的 */
export function hiddenEntries(cards: Record<number, Card> | undefined, inbox: number[]): Set<number> {
  const kids = kidsOf(cards);
  return new Set(inbox.filter(id => kids.has(id)));
}

/** 「未整理」里显示的条目（去掉已经拆出卡的原条目、收到「稍后再做」里的），顺序同 inbox */
export function visibleInbox(b: { cards: Record<number, Card>; cols: Record<string, number[]> } | null | undefined): Card[] {
  if (!b) return [];
  const inbox = b.cols[INBOX] ?? [];
  const hidden = hiddenEntries(b.cards, inbox);
  return inbox.filter(id => !hidden.has(id)).map(id => b.cards[id]).filter(c => c && !c.laterAt);
}

/** 收到「稍后再做」里的条目（default/main#303）：最近收起的在前 */
export function laterEntries(b: { cards: Record<number, Card>; cols: Record<string, number[]> } | null | undefined): Card[] {
  if (!b) return [];
  return (b.cols[INBOX] ?? [])
    .map(id => b.cards[id])
    .filter((c): c is Card => !!c && !!c.laterAt)
    .sort((a, b) => (b.laterAt ?? 0) - (a.laterAt ?? 0) || b.id - a.id);
}

/** 列表里的一项：单独的一条，或者拆自同一条原条目的一组 */
export type Unit = { kind: 'one'; card: Card } | { kind: 'split'; parent: Card; cards: Card[] };

/** 这一项拿来分组、排序的那张卡：一组按组里最新的那张 */
export const unitRep = (u: Unit): Card => (u.kind === 'one' ? u.card : u.cards[0]);
export const unitCards = (u: Unit): Card[] => (u.kind === 'one' ? [u.card] : u.cards);

/**
 * 把（筛选过、已排好序的）条目合成列表项：父条目在未整理里、被隐藏了的，拆出的卡合成一组，
 * 组放在组里第一张出现的位置；父条目不在未整理里（例如从待办里的卡拆出来的）照旧一条一行。
 * parents：被隐藏的原条目（按编号）
 */
export function unitsOf(list: Card[], parents: Record<number, Card>): Unit[] {
  const out: Unit[] = [];
  const at = new Map<number, Extract<Unit, { kind: 'split' }>>();
  for (const c of list) {
    const p = c.splitFrom != null ? parents[c.splitFrom] : undefined;
    if (!p) {
      out.push({ kind: 'one', card: c });
      continue;
    }
    const u = at.get(p.id);
    if (u) u.cards.push(c);
    else {
      const n: Unit = { kind: 'split', parent: p, cards: [c] };
      at.set(p.id, n);
      out.push(n);
    }
  }
  return out;
}

/** 能不能撤销拆解：拆出的卡（白板上的）都还在未整理里，而且至少有一张 */
export function canUndoSplit(cards: Record<number, Card> | undefined, parent: number): boolean {
  const kids = kidsOf(cards).get(parent) ?? [];
  return kids.length > 0 && kids.every(id => cards?.[id]?.column === INBOX);
}
