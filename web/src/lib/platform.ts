/* ==========================================================================
   平台：网页 / 桌面端（Tauri）；主题、卡片密度、动效偏好
   桌面端：顶栏即标题栏，按住空白处拖动窗口，双击最大化
   - Windows 外壳没有系统标题栏（__BOARD_TITLEBAR__ = 'custom'）：窗口按钮由界面画，
     新建窗口（Ctrl+Shift+N）也由界面响应
   - 桌面端在系统默认浏览器里用 GitHub 登录（desktopLogin）
   ========================================================================== */
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { pref } from './util';

const isTauri = typeof window !== 'undefined' && ('__TAURI_INTERNALS__' in window || window.__BOARD_SHELL__ === 'desktop');
export const shell: 'web' | 'desktop' = isTauri ? 'desktop' : 'web';
/** 能调用外壳的命令（外壳给这个页面开放了 IPC） */
const ipc = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
/** 外壳没有系统标题栏，窗口按钮由界面画（Windows） */
export const customTitlebar = ipc && window.__BOARD_TITLEBAR__ === 'custom';
/** 桌面外壳的版本（外壳加载页面前注入；网页端没有） */
export const shellVersion: string | null = (typeof window !== 'undefined' && window.__BOARD_SHELL_VERSION__) || null;
/** macOS / iOS（界面上的 ⌘、桌面端的路径都按它区分） */
export const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
/** Windows：连接说明除了 bash 的 export，还要给 PowerShell 的 $env: 写法 */
export const isWindows = typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent);
/** 桌面端自带的 board 命令行路径（外壳注入；网页端、旧版桌面端没有） */
export const boardCli: string | null = (typeof window !== 'undefined' && window.__BOARD_CLI__) || null;

export type ThemePref = 'auto' | 'light' | 'dark';
export type Density = 'comfy' | 'compact';
export type Motion = 'full' | 'reduced';

export function applyPrefs() {
  const root = document.documentElement;
  root.dataset.shell = shell;
  root.dataset.os = isMac ? 'mac' : 'other';
  if (customTitlebar) {
    root.dataset.titlebar = 'custom';
    // 告诉外壳这个页面会画窗口按钮（否则外壳过几秒恢复系统标题栏）
    invoke('window_ready').catch(() => {});
    document.addEventListener('keydown', e => {
      if (e.ctrlKey && e.shiftKey && !e.altKey && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        newWindow();
      }
    });
  }
  setTheme(pref.get<ThemePref>('theme', 'auto'));
  root.dataset.density = pref.get<Density>('density', 'comfy');
  root.dataset.motion = pref.get<Motion>('motion', 'full');
}

export function setTheme(p: ThemePref) {
  const root = document.documentElement;
  root.dataset.themePref = p;
  if (p === 'auto') delete root.dataset.theme;
  else root.dataset.theme = p;
  pref.set('theme', p);
  // 桌面端：告诉外壳，下次的启动页和窗口底色按这个主题来（旧版外壳没有这个命令，忽略）
  if (ipc) invoke('remember_theme', { theme: p }).catch(() => {});
}
export const getTheme = () => pref.get<ThemePref>('theme', 'auto');
export function cycleTheme(): ThemePref {
  const order: ThemePref[] = ['auto', 'light', 'dark'];
  const next = order[(order.indexOf(getTheme()) + 1) % order.length];
  setTheme(next);
  return next;
}
export function setDensity(d: Density) {
  document.documentElement.dataset.density = d;
  pref.set('density', d);
}
export function setMotion(m: Motion) {
  document.documentElement.dataset.motion = m;
  pref.set('motion', m);
}

// 静态导入：按下鼠标时要立刻发起拖动（macOS 需要在按住期间调用）
const win = () => (isTauri ? getCurrentWindow() : null);
export function startWindowDrag() {
  win()?.startDragging().catch(() => {});
}
export function toggleMaximize() {
  win()?.toggleMaximize().catch(() => {});
}
export function minimizeWindow() {
  win()?.minimize().catch(() => {});
}
export function closeWindow() {
  win()?.close().catch(() => {});
}
/** 窗口是否最大化；变化时回调。返回取消监听的函数 */
export function watchMaximized(fn: (max: boolean) => void): () => void {
  const w = win();
  if (!w) return () => {};
  let stop: (() => void) | null = null;
  let done = false;
  const sync = () => w.isMaximized().then(m => !done && fn(m), () => {});
  sync();
  w.onResized(sync).then(
    un => (done ? un() : (stop = un)),
    () => {},
  );
  return () => {
    done = true;
    stop?.();
  };
}

/** 新建窗口（桌面端） */
export const canNewWindow = ipc;
export function newWindow() {
  if (ipc) invoke('new_window').catch(() => {});
}

/* ---------------- 桌面端：一键把服务端地址和令牌写进 ~/.board/config.toml ---------------- */

/** 网页不能改电脑上的配置，只有桌面端能 */
export const canSetEnv = ipc;
/** 写进 board 命令行的配置文件；返回写到了哪里（~/.board/config.toml）。外壳命令沿用 set_board_env 这个名字。
    外壳不支持（旧版桌面端）时抛出 'unsupported'，其他失败抛出原因文字 */
export async function setBoardEnv(token: string): Promise<string> {
  try {
    const r = await invoke<{ target: string }>('set_board_env', { token });
    return r.target;
  } catch (e) {
    const msg = String(e);
    if (/not found|not allowed|unknown/i.test(msg)) throw 'unsupported';
    throw msg;
  }
}

/* ---------------- 桌面端登录：在系统默认浏览器里用 GitHub 登录 ---------------- */

/** 外壳返回的「登录没有完成」 */
export interface DesktopLoginError {
  /** 服务端给的：cancelled / denied / state / github；外壳的：superseded / timeout / browser / io */
  reason: string;
  login?: string;
  message?: string;
}
export const canDesktopLogin = ipc;
/** 在默认浏览器里登录，返回一次性登录码；用 `/auth/desktop?code=…` 换取这个窗口的会话。
    外壳不支持（旧版桌面端）时抛出 'unsupported' */
export async function desktopLogin(): Promise<string> {
  try {
    return await invoke<string>('desktop_login');
  } catch (e) {
    if (e && typeof e === 'object' && 'reason' in e) throw e as DesktopLoginError;
    throw 'unsupported';
  }
}

/* ---------------- 桌面端：让这台电脑接任务（后台运行 board host） ---------------- */

/** 只有桌面端能在本机运行 board host */
export const canHost = ipc;

export interface LocalHost {
  /** 「让这台电脑接任务」打开了 */
  enabled: boolean;
  /** board host 正在运行 */
  running: boolean;
  /** 停了的话：退出码 */
  exit: number | null;
  /** ~/.board/config.toml 里的机器名（空表示用本机主机名） */
  name: string;
  /** 不填时用的机器名 */
  defaultName: string;
  dirs: { board: string; path: string }[];
  /** board host 最近的输出 */
  log: string[];
  /** 配置文件里有服务端地址和令牌 */
  loggedIn: boolean;
  /** 桌面端自带的 board 命令行找到了 */
  cli: boolean;
}

/** 本机 board host 的现状；外壳不支持（旧版桌面端）时返回 null */
export async function localHost(): Promise<LocalHost | null> {
  try {
    return await invoke<LocalHost>('host_status');
  } catch {
    return null;
  }
}

/** 保存机器名、白板与目录（写进 ~/.board/config.toml 的 [host]）；失败时抛出原因文字 */
export async function saveLocalHost(name: string, dirs: { board: string; path: string }[]): Promise<void> {
  try {
    await invoke('host_save', { name, dirs });
  } catch (e) {
    throw String(e);
  }
}

/** 打开 / 关掉「让这台电脑接任务」；失败时抛出原因文字 */
export async function enableLocalHost(on: boolean): Promise<void> {
  try {
    await invoke('host_enable', { on });
  } catch (e) {
    throw String(e);
  }
}
