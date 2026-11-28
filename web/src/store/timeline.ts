/* ==========================================================================
   项目 Tab 时间轴的范围（PRD §10.4）：侧栏（Workbench 的 ProjectsSide）写，时间轴页（components/Timeline）读。
   单放一个文件：侧栏在入口包里，时间轴页按需加载
   - 项目跟着当前白板走：侧栏点项目、顶栏切换器换白板都一样
   - 侧栏点白板时只看这块白板（再点一次回到整个项目）；换到别的项目后自动回到整个项目
   ========================================================================== */
import { create } from 'zustand';
import { useCurrent } from './data';

const useTlBoard = create<{ board: string | null }>(() => ({ board: null }));

/** 只看哪块白板（路径）；null = 整个项目 */
export const setTlBoard = (board: string | null) => useTlBoard.setState({ board });

/** 实际生效的范围：当前白板所在的项目；只看的那块白板不在这个项目里就看整个项目 */
export function useTlScope(): { project: string | null; board: string | null } {
  const cur = useCurrent();
  const b = useTlBoard(s => s.board);
  const project = cur?.project.key ?? null;
  return { project, board: b && project && b.startsWith(`${project}/`) ? b : null };
}
