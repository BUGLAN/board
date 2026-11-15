/* ==========================================================================
   服务端升级之后，浏览器里还开着的旧页面（纯函数，便于测试；调用方在 lib/stale-runner.ts 和 store/data.ts）
   - 为什么：服务端升级后接口的含义可能变了（字段改名、换了写法，甚至整个去掉），旧页面接着用会画错、存错
   - 网页端：页面打包时的版本 ≠ 服务端 GET /api/info 的版本时提示刷新；桌面端自带的界面走自己的更新流程，不在这里提示
   - 检查的时机：启动时、推送重连成功、切回标签页；同一个服务端版本只提示一次
   ========================================================================== */
import { cmpVersion } from './ready';

/** 两份版本号是不是同一版：三个数字比，`0.1.30` 和 `0.1.30-build.5` 算同一版；解析不了的按原文比 */
export function sameVersion(a: string, b: string): boolean {
  const c = cmpVersion(a, b);
  return c != null ? c === 0 : a.trim() === b.trim();
}

/** 页面打包时的版本和服务端不一样时返回服务端版本（提示里要用），否则 null */
export function staleServer(build: string, server: string | null | undefined): string | null {
  const s = (server ?? '').trim();
  // 不知道打包版本（开发服务器没注入）、服务端没给版本：不提示，免得每打开一页都弹一句
  if (!build.trim() || !s) return null;
  return sameVersion(build, s) ? null : s;
}

/** 提示条上的文字 */
export function staleText(server: string): string {
  return `服务端已经升级到 v${server.trim().replace(/^v/, '')}，刷新页面用新版`;
}

/** 记下已经为哪个服务端版本提示过（每个标签页一份：刷新之后自然没了） */
const KEY = 'board.staleNotice';
export const staleNoticed = (server: string): boolean => {
  try {
    return sessionStorage.getItem(KEY) === server;
  } catch {
    return false;
  }
};
export const markStaleNoticed = (server: string) => {
  try {
    sessionStorage.setItem(KEY, server);
  } catch {
    /* 隐私模式等 */
  }
};
