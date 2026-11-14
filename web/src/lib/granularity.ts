/* ==========================================================================
   拆解前的粒度预检（default/main#306）
   任务本身已经足够细（一句话需求、单点操作）时不该再拆：界面上直接提示、按钮改成「强制拆解」；
   拦不住的情况（人点了「强制拆解」、从别处建的条目）在拆解提示词里告诉 agent 先判一遍。
   纯函数：输入条目的标题、描述，输出「偏大」「偏小」和依据，不发请求
   ========================================================================== */

export type Size = 'small' | 'large';

export interface Verdict {
  size: Size;
  /** 说给人听的一句（界面上提示、评论里写） */
  why: string;
}

/** 短到这样（去掉空白后的字数）又只做一件事：按偏小处理 */
const SHORT = 40;
/** 长到这样：就算没有拆分标志，也按偏大处理 */
const LONG = 200;
/** 小标题、列表项各自到两个，句子到四句，就算几件事 */
const PARTS = 2;
const CLAUSES = 4;
/** 并列连接词到两个才算「一句里有几个目标」 */
const GOALS = 2;

const chars = (s: string) => [...s.replace(/\s/g, '')].length;

/** 拆开的句子数：句号、问号、叹号、分号，以及非空行开头的列表项、小标题 */
export function clauses(text: string): string[] {
  return text
    .split(/[。．.！!？?；;\n]+/)
    .map(s => s.replace(/^[\s#>*\-+\d.、)）]+/, '').trim())
    .filter(Boolean);
}

export interface Marks {
  /** 以 `-`、`*`、`1.` 这类开头的行 */
  bullets: number;
  /** `##` 开头的小标题 */
  headings: number;
  /** 「以及」「并且」「另外」这类并列连接词（「和」不算：「检查项和提示语」是一件事） */
  goals: number;
}

/** 多个独立可执行步骤的标志 */
export function markers(text: string): Marks {
  const lines = text.split('\n').filter(l => l.trim());
  return {
    bullets: lines.filter(l => /^\s*([-*+]|\d+[.、)])\s+/.test(l)).length,
    headings: lines.filter(l => /^\s*#{2,}\s+/.test(l)).length,
    goals: (text.match(/以及|并且|同时|另外|还要|然后|顺便/g) ?? []).length,
  };
}

/**
 * 把一条需求归到「偏大」「偏小」。按「拆卡只看能不能分开复盘」：
 * 小标题或列表分成几块、写了四句以上、并列的几个目标，就是偏大；否则一个小任务，不用拆。
 * 长短本身只做兜底：很长的一段也按偏大，很短的一段按偏小。
 */
export function judge(title: string, body: string): Verdict {
  const src = body.trim() || title.trim();
  const n = chars(src);
  const { bullets, headings, goals } = markers(src);
  const c = clauses(src).length;
  if (headings >= PARTS) return { size: 'large', why: `分成 ${headings} 个小标题` };
  if (bullets >= PARTS) return { size: 'large', why: `列了 ${bullets} 项` };
  if (c >= CLAUSES) return { size: 'large', why: `写了 ${c} 句，像几件事` };
  if (goals >= GOALS) return { size: 'large', why: '一句里有几个并列的目标' };
  if (n >= LONG) return { size: 'large', why: `写了 ${n} 个字，不止一件事` };
  if (n <= SHORT) return { size: 'small', why: '只有一句话，是一个独立的小任务' };
  return { size: 'small', why: '看着只是一件事，可以整个做' };
}
