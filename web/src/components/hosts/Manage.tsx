/* ==========================================================================
   机器详情「维护」（PRD §9.1「远程管理」、§10.12）用到的纯函数：版本比较、远程管理请求的进展。
   界面在 HostSettings.tsx（重新检查、升级、重启 board host、令牌、移除）；「排空」并进了「暂停」（机器 Tab「接单」里的接单三态）
   ========================================================================== */
import type { HostInfo } from '../../lib/types';

/** 版本号 a 比 b 旧（和服务端 version_lt 一致） */
export function versionLt(a: string, b: string) {
  const parts = (v: string) => {
    const p = v.trim().replace(/^v/, '').split(/[.+-]/).slice(0, 3).map(x => Number.parseInt(x, 10) || 0);
    while (p.length < 3) p.push(0);
    return p;
  };
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] < y[i];
  return false;
}

/** 请求的进展（一行字）：正在重新检查、等几个派发结束后升级、正在重启……；没有请求时为空 */
export function askState(h: HostInfo, running: number): { text: string; tone: 'busy' | 'ok' | 'warn' } | null {
  const a = h.ask;
  if (a?.kind === 'recheck') return { text: '正在重新检查：等 board host 重新找启动环境、agent、board-skills', tone: 'busy' };
  if (a?.kind === 'restart') return { text: running ? `等 ${running} 个在跑的派发停下后重启 board host` : '正在重启 board host', tone: 'busy' };
  if (a?.kind === 'upgrade') {
    const to = a.to ?? '新版本';
    return { text: running ? `等 ${running} 个派发结束后升级到 ${to}` : `正在升级到 ${to}`, tone: 'busy' };
  }
  return null;
}
