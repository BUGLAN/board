import { isVersionName } from './milestones';
import type { Label } from './types';

/** 侧栏「标签」里列出的标签：版本标签（v0.1.24 这类，发布时自动建，一次累积几十个）平时不列，
 *  正选中筛选的例外——列出来才点得到它取消，不然人被看不见的筛选困住。
 *  只影响侧栏：卡片上的标签胶囊、标签菜单、里程碑页照旧。
 *  labels 每次刷新都是新数组，sel 只在标签变动时换（见 store/ui 的 toggleLabelSel），可以直接拿去当 useMemo 的依赖 */
export function sidebarLabels(labels: Label[], sel: number[] = []): Label[] {
  return labels.filter(l => sel.includes(l.id) || !isVersionName(l.name));
}
