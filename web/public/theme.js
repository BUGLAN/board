/* 首帧就按主题上底色（index.html 同步加载）：主题偏好存在 localStorage，样式和脚本包还没加载时页面也不是白的。
   与 lib/platform.ts 的 setTheme、styles/tokens.css 的 --bg 一致。单独成文件是因为页面的 CSP 不允许内联脚本 */
try {
  var t = JSON.parse(localStorage.getItem('board.theme') || '"auto"');
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
} catch (e) {}
