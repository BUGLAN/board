/* ==========================================================================
   本机接任务（设置 → 本机）：打开开关、修改「白板与目录」之前，用应用内的确认框
   把机器名、白板与目录、免审批运行的风险说清楚（正文在这里拼，界面只负责显示）
   ========================================================================== */

export interface HostDir {
  board: string;
  path: string;
}

/** 目录列表最多列几行：再多就在弹窗里滚动，行里也拉长会撑破弹窗 */
const MAX_LINES = 40;

/** 机器名：配置里没写（空）时用本机主机名，和 board host 启动时一致 */
export function shownHostName(name: string, defaultName: string): string {
  const n = name.trim();
  if (n) return n;
  return defaultName.trim() ? `${defaultName.trim()}（本机主机名）` : '（本机主机名）';
}

/** 「白板与目录」的逐行列表；一行一条，没填全的行跳过 */
export function hostDirLines(dirs: HostDir[]): string[] {
  const lines = dirs
    .filter(d => d.board.trim() || d.path.trim())
    .map(d => `· ${d.board.trim() || '（没选白板）'} → ${d.path.trim() || '（没有目录）'}`);
  if (lines.length <= MAX_LINES) return lines;
  return [...lines.slice(0, MAX_LINES), `…还有 ${lines.length - MAX_LINES} 条`];
}

/** 确认框正文：机器名、白板与目录、免审批运行的风险（段落之间空一行，确认框按段落显示） */
export function hostConfirmText(name: string, defaultName: string, dirs: HostDir[]): string {
  const lines = hostDirLines(dirs);
  const list = lines.length ? lines.join('\n') : '（还没有白板目录：派到这台电脑的卡片要先有目录才会运行）';
  return [
    `机器名：${shownHostName(name, defaultName)}`,
    `白板与目录：\n${list}`,
    '派发到这台电脑的卡片，agent 会在对应目录里以免审批模式运行：它可以直接改代码、推送到集成分支。只在信任这些白板时确认。',
  ].join('\n\n');
}
