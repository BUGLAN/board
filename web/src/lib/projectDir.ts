/* ==========================================================================
   项目目录（PRD §4，default/main#488）：每台机器上这个项目的目录是 <工作空间>/<项目目录>，留空用项目标识。
   规则和服务端 check_project_dir / set_project_dir 一致：只能是一个目录名，不同项目不能用同一个目录。
   界面先查一遍，免得等服务端返回才知道填错了；最终以服务端为准
   ========================================================================== */
import type { ProjectInfo } from './types';

/** 和服务端 PROJECT_DIR_MAX 一致 */
export const PROJECT_DIR_MAX = 64;

/** 这个项目实际用的目录名：填了项目目录用它，没填用项目标识 */
export const dirOf = (p: Pick<ProjectInfo, 'key' | 'dir'>) => p.dir?.trim() || p.key;

/** 项目目录写得不对时的原因；对（含留空）时为 null。others：别的项目（查重名） */
export function projectDirError(raw: string, key: string, others: Pick<ProjectInfo, 'key' | 'dir'>[] = []): string | null {
  const d = raw.trim();
  if (!d) return taken(key, key, others, true);
  if ([...d].length > PROJECT_DIR_MAX) return `最长 ${PROJECT_DIR_MAX} 个字符`;
  if (d.includes('/') || d.includes('\\')) return '只能写一个目录名，不能写路径（不能有 / 或 \\），目录在每台机器的工作空间里';
  if (d === '.' || d === '..') return '不能是 . 或 ..';
  if (d.includes(':')) return '不能有 :（也不能带盘符）';
  if (d.startsWith('~')) return '不能以 ~ 开头';
  // eslint-disable-next-line no-control-regex
  const bad = d.match(/[\u0000-\u001f\u007f<>"|?*]/);
  if (bad) return `不能有 ${/[<>"|?*]/.test(bad[0]) ? bad[0] : '控制字符'}`;
  if (d.endsWith('.')) return '不能以 . 结尾（Windows 上用不了）';
  return taken(d, key, others, d === key);
}

/** 实际用的目录 used 和别的项目实际用的重名（不分大小写）时的原因 */
function taken(used: string, key: string, others: Pick<ProjectInfo, 'key' | 'dir'>[], own: boolean): string | null {
  const other = others.find(p => p.key !== key && dirOf(p).toLowerCase() === used.toLowerCase());
  if (!other) return null;
  return own ? `项目「${other.key}」的目录就叫「${used}」：这个项目要填一个别的项目目录` : `「${used}」已经是项目「${other.key}」的目录：每个项目一个目录`;
}
