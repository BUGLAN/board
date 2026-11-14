/* ==========================================================================
   隐藏的标签（default/main#324）：项目里存一组正则（hiddenLabels），名称匹配任一条的标签
   不出现在日常的标签列表里——侧栏「标签」、提示词菜单「做某状态里带指定标签的任务」、标签选择器。
   卡片上已经带着的标签、卡片详情、里程碑、「管理标签」照常显示。
   默认 `^v\d`：版本标签（v0.1.24 这类，发版时自动建，一次累积几十个）。
   规则按浏览器的 RegExp 解释、不区分大小写；写错的规则不生效（管理标签里会标出来）
   ========================================================================== */
import type { Label, ProjectInfo } from './types';

/** 旧服务端（项目信息里没有 hiddenLabels）、新项目的默认规则 */
export const DEFAULT_HIDDEN = ['^v\\d'];
/** 规则最多几条、每条最长几个字符（和服务端 HIDDEN_LABELS_MAX / HIDDEN_LABEL_LEN 一致） */
export const HIDDEN_MAX = 20;
export const HIDDEN_LEN = 100;

/** 项目的隐藏规则；旧服务端没有这个字段时按默认 */
export const hiddenRules = (p: Pick<ProjectInfo, 'hiddenLabels'> | null | undefined): string[] => p?.hiddenLabels ?? DEFAULT_HIDDEN;

/** 一条规则有什么问题；没问题为 null。others：已有的规则（查重） */
export function ruleError(rule: string, others: string[] = []): string | null {
  const r = rule.trim();
  if (!r) return '规则不能为空';
  if (r.length > HIDDEN_LEN) return `每条最多 ${HIDDEN_LEN} 个字符`;
  if (others.includes(r)) return '已经有这条规则了';
  if (others.length >= HIDDEN_MAX) return `最多 ${HIDDEN_MAX} 条规则`;
  try {
    new RegExp(r, 'i');
  } catch (e) {
    return `不是有效的正则：${e instanceof Error ? e.message : String(e)}`;
  }
  return null;
}

const cache = new Map<string, RegExp[]>();
function compile(rules: string[]): RegExp[] {
  const k = rules.join('\n');
  let out = cache.get(k);
  if (!out) {
    out = [];
    for (const r of rules) {
      if (!r.trim()) continue;
      try {
        out.push(new RegExp(r.trim(), 'i'));
      } catch {
        // 写错的规则不生效
      }
    }
    if (cache.size > 50) cache.clear();
    cache.set(k, out);
  }
  return out;
}

/** 这个名称被哪条规则隐藏；没被隐藏为 null */
export function hiddenBy(name: string, rules: string[]): string | null {
  const n = name.trim();
  const res = compile(rules);
  const i = res.findIndex(re => re.test(n));
  return i < 0 ? null : res[i].source;
}

/** 日常列表里显示的标签：去掉被隐藏的。keep 里的例外（正选中筛选的、卡片上已经带着的）——列出来才点得到它取消。
 *  按原顺序；一个都没去掉时返回原数组 */
export function visibleLabels(labels: Label[], rules: string[], keep: readonly number[] = []): Label[] {
  const res = compile(rules);
  if (!res.length) return labels;
  const out = labels.filter(l => keep.includes(l.id) || !res.some(re => re.test(l.name.trim())));
  return out.length === labels.length ? labels : out;
}

/** 侧栏「标签」里列出的标签：隐藏的不列，正选中筛选的例外（不然人被看不见的筛选困住）。
 *  labels 每次刷新都是新数组，sel 只在标签变动时换（见 store/ui 的 toggleLabelSel），可以直接拿去当 useMemo 的依赖 */
export function sidebarLabels(labels: Label[], sel: number[] = [], rules: string[] = DEFAULT_HIDDEN): Label[] {
  return visibleLabels(labels, rules, sel);
}
