/* ==========================================================================
   这份界面打包时的版本号（vite 在打包时写进 import.meta.env.VITE_BOARD_VERSION）
   和 GET /api/info 的服务端版本比：不一样就说明服务端已经升级、浏览器里开着的是旧页面（default/main#368）
   - import.meta.env 只在 vite 打包时才有：别的环境（node 里跑单元测试、以后进了 SSR）退回空串，不提示
   ========================================================================== */

/** 界面打包时的版本；开发服务器、没注入时是空串 */
export function buildVersion(): string {
  try {
    return (import.meta.env?.VITE_BOARD_VERSION ?? '').toString().trim();
  } catch {
    return '';
  }
}
