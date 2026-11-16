/* ==========================================================================
   快捷提示词（default/main#413）：常写的描述片段，存在本机偏好里
   - 纯函数：读进来（list）、新增、修改、删除都能单独测
   - 插入只在当前描述文本的光标处做，不在这里碰 DOM
   ========================================================================== */
import { pref } from './util';

export interface Prompt {
  /** 本机唯一，列表里按它找编辑、删除的目标 */
  id: string;
  title: string;
  /** 原样插入描述的 Markdown 文本 */
  text: string;
}

/** 存在 pref 里（实际键是 board.creator.prompts） */
export const PROMPTS_KEY = 'creator.prompts';

const titleOf = (v: unknown) => (typeof v === 'string' ? v.trim() : '');
const textOf = (v: unknown) => (typeof v === 'string' ? v.replace(/\r\n?/g, '\n') : '');
const usable = (title: string, text: string) => !!title && !!text.trim();

/** 读本机偏好里的一列提示词：坏数据、缺字段、重复 id 的条目丢掉，不让它们把整个浮层带坏 */
export function readPrompts(raw: unknown): Prompt[] {
  if (!Array.isArray(raw)) return [];
  const out: Prompt[] = [];
  const ids = new Set<string>();
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue;
    const r = item as Record<string, unknown>;
    const id = typeof r.id === 'string' ? r.id.trim() : '';
    const title = titleOf(r.title);
    const text = textOf(r.text);
    if (!id || !usable(title, text) || ids.has(id)) continue;
    ids.add(id);
    out.push({ id, title, text });
  }
  return out;
}

/** 读本机的提示词列表（lib/util 的 pref，localStorage） */
export const listPrompts = (): Prompt[] => readPrompts(pref.get<unknown>(PROMPTS_KEY, []));

/** 把整列写回本机；上一笔已经存了就覆盖 */
export const savePrompts = (list: Prompt[]) => pref.set(PROMPTS_KEY, list);

function nextId(list: readonly Prompt[]) {
  let n = 0;
  for (const p of list) {
    const m = /^p-(\d+)$/.exec(p.id);
    if (m) n = Math.max(n, Number(m[1]));
  }
  return `p-${n + 1}`;
}

/** 新建一条，放在最前面；标题或内容为空时原样返回，不生成空条目 */
export function addPrompt(list: readonly Prompt[], title: string, text: string): Prompt[] {
  const t = titleOf(title);
  const x = textOf(text);
  if (!usable(t, x)) return [...list];
  return [{ id: nextId(list), title: t, text: x }, ...list];
}

/** 改标题、内容；找不到 id 或内容为空时原样返回 */
export function updatePrompt(list: readonly Prompt[], id: string, title: string, text: string): Prompt[] {
  const t = titleOf(title);
  const x = textOf(text);
  if (!usable(t, x) || !list.some(p => p.id === id)) return [...list];
  return list.map(p => (p.id === id ? { ...p, title: t, text: x } : p));
}

/** 删除；id 已经不在了就原样返回 */
export function removePrompt(list: readonly Prompt[], id: string): Prompt[] {
  const next = list.filter(p => p.id !== id);
  return next.length === list.length ? [...list] : next;
}

export interface Inserted {
  value: string;
  /** 插入后光标的位置（在插入内容之后） */
  caret: number;
}

/** 把 text 插进 value 的 [start, end)；传 null / 越界时夹到文本末尾，光标停在插入内容之后 */
export function insertPromptText(value: string, start: number | null | undefined, end: number | null | undefined, text: string): Inserted {
  const len = value.length;
  const at = (n: number | null | undefined, fallback: number) => (typeof n === 'number' && Number.isFinite(n) ? Math.trunc(n) : fallback);
  const a = Math.max(0, Math.min(at(start, len), len));
  const b = Math.max(a, Math.min(at(end, a), len));
  const before = value.slice(0, a);
  return { value: before + text + value.slice(b), caret: before.length + text.length };
}
