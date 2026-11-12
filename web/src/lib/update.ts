/* ==========================================================================
   桌面端更新（外壳命令见 src-tauri/src/update.rs）
   - 设置 → 本机：显示当前版本、手动「检查更新」，有新版本时展开说明、下载安装、装完提示重启
   - 启动后自动检查一次（默认开，可在设置里关掉）：有新版本时用应用内的确认框问，不阻塞启动、不弹系统框
   - 这里只放不碰外壳的逻辑：版本比较、状态文字、检查的状态机；调用外壳的薄封装在 platform.ts
   ========================================================================== */
import { pref } from './util';

/** 外壳 update_check 的返回 */
export interface UpdateCheck {
  /** 当前版本 */
  current: string;
  /** 比当前新的版本；已是最新时为 null */
  version: string | null;
  /** 更新说明（那一版 CHANGELOG） */
  notes: string | null;
  /** 发布时间（ISO 8601） */
  date: string | null;
}

/** 外壳经 update://progress 事件推来的下载进展 */
export interface UpdateProgress {
  downloaded: number;
  /** 总字节数；不知道时为 null */
  total: number | null;
}

/** 外壳 update_install 的返回 */
export interface UpdateInstalled {
  version: string;
  /** 安装后要重启才能用上新版本（macOS / Linux；Windows 由安装器拉起） */
  restart: boolean;
}

/* ---------------- 启动后自动检查 ---------------- */

const AUTO = 'update.auto';
/** 启动后自动检查更新（默认开） */
export const autoCheckUpdate = () => pref.get<boolean>(AUTO, true);
export function setAutoCheckUpdate(on: boolean) {
  pref.set(AUTO, on);
}

/** 自动检查的间隔：30 分钟 */
export const AUTO_CHECK_INTERVAL = 30 * 60 * 1000;
/** 打开后等这么久再查：先让页面和登录就绪，不和首屏抢网络 */
export const AUTO_CHECK_START_DELAY = 1500;
const LAST_AUTO = 'update.lastAutoCheck';

/** 上次自动检查的时间（毫秒时间戳）；从没查过时是 0 */
export const lastAutoCheck = () => pref.get<number>(LAST_AUTO, 0);
/** 记下这次自动检查的时间：页面刷新、重复挂载都按它节流 */
export function markAutoCheck(at = Date.now()) {
  pref.set(LAST_AUTO, at);
}
/** 距离下一次自动检查还要等多久：没查过时用启动延迟，查过就等到满 30 分钟（刷新页面不会重复查） */
export function autoCheckDelay(last: number, now: number): number {
  return Math.max(AUTO_CHECK_START_DELAY, last > 0 ? last + AUTO_CHECK_INTERVAL - now : 0);
}

/* ---------------- 版本 ---------------- */

/** 按点分成数字：`1.2.3` → `[1,2,3]`；`v` 前缀、预发布 / 构建后缀（`-beta.1`、`+sha`）不看；有不是数字的段时返回 null */
export function versionParts(v: string): number[] | null {
  const core = v.trim().replace(/^v/, '').split(/[-+]/, 1)[0];
  if (!core) return null;
  const parts: number[] = [];
  for (const p of core.split('.')) {
    if (!/^\d+$/.test(p)) return null;
    parts.push(Number(p));
    if (parts.length > 3) return null;
  }
  return parts;
}

/** `a` 比 `b` 新：`0.2.0` 比 `0.10.0` 旧（按数字比，不按字符串）；解析不了时不算更新 */
export function isNewer(a: string, b: string): boolean {
  const x = versionParts(a);
  const y = versionParts(b);
  if (!x || !y) return false;
  const n = Math.max(x.length, y.length);
  for (let i = 0; i < n; i++) {
    // 两段、三段都认：`1.2` 当成 `1.2.0`
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d > 0;
  }
  return false;
}

/** 检查结果里有比当前新的版本时返回它的版本号（以服务端的说法为准，再用版本号核一遍），否则 null */
export function newerVersion(r: UpdateCheck | null | undefined): string | null {
  if (!r?.version) return null;
  return isNewer(r.version, r.current) ? r.version : null;
}

/* ---------------- 显示用 ---------------- */

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** 下载进度 0–100；不知道总大小时返回 null（界面只显示已下载多少） */
export function downloadPercent(p: UpdateProgress | null): number | null {
  if (!p || !p.total || p.total <= 0) return null;
  return Math.min(100, Math.max(0, Math.round((p.downloaded / p.total) * 100)));
}

/** 下载进度的文字：`12.3 MB / 45.6 MB（27%）`；不知道总大小时是 `已下载 12.3 MB` */
export function progressText(p: UpdateProgress | null): string {
  if (!p) return '';
  const size = formatBytes(p.downloaded);
  const pct = downloadPercent(p);
  if (pct == null) return `已下载 ${size}`;
  return `${size} / ${formatBytes(p.total ?? 0)}（${pct}%）`;
}

/** 检查失败的提示：保持原样，加上可以重试的说法（旧版外壳不支持时原样是 'unsupported'，自动检查不看它） */
export function failureText(e: unknown): string {
  return `${String(e)}。当前版本不受影响，可以重新检查`;
}

/* ---------------- 一次检查 ---------------- */

export type CheckState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'up-to-date'; result: UpdateCheck }
  | { kind: 'available'; result: UpdateCheck; version: string }
  | { kind: 'failed'; error: string };

/** 一次检查完了：版本号更大才算发现新版本 */
export function finishCheck(r: UpdateCheck): CheckState {
  const v = newerVersion(r);
  return v ? { kind: 'available', result: r, version: v } : { kind: 'up-to-date', result: r };
}

export const failCheck = (e: unknown): CheckState => ({ kind: 'failed', error: failureText(e) });

/** 下载安装的状态：下载中带进度，装完等重启；失败可以重试 */
export type InstallState =
  | { kind: 'idle' }
  | { kind: 'downloading'; progress: UpdateProgress | null }
  | { kind: 'ready'; version: string; restart: boolean }
  | { kind: 'failed'; error: string };
