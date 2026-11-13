/* ==========================================================================
   本机接任务（设置 → 本机）：打开开关、修改工作空间之前，用应用内的确认框
   把机器名、工作空间、免审批运行的风险说清楚（正文在这里拼，界面只负责显示）
   ========================================================================== */

/** 机器名：配置里没写（空）时用本机主机名，和 board host 启动时一致 */
export function shownHostName(name: string, defaultName: string): string {
  const n = name.trim();
  if (n) return n;
  return defaultName.trim() ? `${defaultName.trim()}（本机主机名）` : '（本机主机名）';
}

/** 工作空间里的白板目录：`<工作空间>/<项目>/<白板>`（分隔符跟着机器走）；工作空间为空时为空 */
export function workspaceDir(workspace: string, board: string, os = ''): string {
  const base = workspace.trim().replace(/[\\/]+$/, '');
  if (!base || !board.trim()) return '';
  const sep = /windows/i.test(os) ? '\\' : '/';
  return base + sep + board.trim().split('/').join(sep);
}

/** 「切换服务端」确认框的正文：从哪个地址换到哪个、写进哪里、本机命令只对新地址开放；
    本机在跑 board host 时说明会按新地址重启它（段落之间空一行，确认框按段落显示） */
export function serverConfirmText(from: string, to: string, version: string, target: string, hostRunning: boolean): string {
  const parts = [
    `从 ${from} 换到 ${to}${version ? `（Board ${version}）` : ''}。`,
    `地址写进 ${target} 的 url（和 board login 同一项），桌面端关掉这个窗口、按新地址重新打开；之后本机命令只对新地址的页面开放，旧地址的页面调不到。`,
  ];
  if (hostRunning) parts.push('这台电脑正在接任务（board host）：切换时先停掉它，按新地址重新打开后再启动，正在跑的 agent 会结束。');
  return parts.join('\n\n');
}

/** 确认框正文：机器名、工作空间、免审批运行的风险（段落之间空一行，确认框按段落显示） */
export function hostConfirmText(name: string, defaultName: string, workspace: string): string {
  const ws = workspace.trim();
  const line = ws ? `${ws}（每块白板的目录是 ${ws.replace(/[\\/]+$/, '')}/<项目>/<白板>）` : '（还没填：这台电脑不接任何白板的卡）';
  return [
    `机器名：${shownHostName(name, defaultName)}`,
    `工作空间：${line}`,
    '派发到这台电脑的卡片，agent 会在对应目录里以免审批模式运行：它可以直接改代码、推送到集成分支。只在信任这些白板时确认。',
  ].join('\n\n');
}
