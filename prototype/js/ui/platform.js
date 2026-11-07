/* ==========================================================================
   Board · 平台适配
   - 识别运行环境：浏览器 / Electron / Tauri；?shell=desktop 在浏览器里预览桌面外壳
   - 主题偏好：auto / light / dark（持久化在 localStorage）
   扩展：桌面端需要原生能力（菜单、通知、托盘）时，在 bridge 上挂实现即可，
         界面代码只调用 platform.bridge.notify(...) 之类的抽象方法。
   ========================================================================== */
(function (B) {
  'use strict';
  const root = document.documentElement;
  const params = new URLSearchParams(location.search);

  const native =
    (window.__TAURI__ && 'tauri') ||
    (/Electron/i.test(navigator.userAgent) && 'electron') ||
    null;

  let shell = native ? 'desktop' : (params.get('shell') || safeGet('board.shell') || 'web');

  function safeGet(k) { try { return localStorage.getItem(k); } catch { return null; } }
  function safeSet(k, v) { try { localStorage.setItem(k, v); } catch {} }

  function applyShell(s) {
    shell = s;
    root.dataset.shell = s;
    // 在真实桌面容器里交通灯由系统绘制；浏览器预览时画一个假的
    if (s === 'desktop' && !native) root.dataset.mockChrome = ''; else delete root.dataset.mockChrome;
    const label = document.getElementById('shellLabel');
    if (label) label.textContent = s === 'desktop' ? '桌面' : 'Web';
  }

  /* ---------- 主题 ---------- */
  const THEMES = ['auto', 'light', 'dark'];
  let themePref = safeGet('board.theme') || 'auto';
  function applyTheme(pref) {
    themePref = pref;
    root.dataset.themePref = pref;
    if (pref === 'auto') delete root.dataset.theme; else root.dataset.theme = pref;
    safeSet('board.theme', pref);
  }

  /* ---------- 原生桥（占位，便于后续接入） ---------- */
  const bridge = {
    notify(title, body) {
      if (native === 'tauri' && window.__TAURI__?.notification) return window.__TAURI__.notification.sendNotification({ title, body });
      // Web：暂不打扰用户；需要时可在这里接入 Notification API
    },
  };

  applyShell(shell);
  applyTheme(themePref);
  document.querySelectorAll('.mod-key').forEach(k => (k.textContent = B.ui.isMac ? '⌘' : 'Ctrl'));

  B.platform = {
    native,
    get shell() { return shell; },
    setShell(s) { if (native) return; applyShell(s); safeSet('board.shell', s); },
    toggleShell() { this.setShell(shell === 'desktop' ? 'web' : 'desktop'); },
    get theme() { return themePref; },
    setTheme: applyTheme,
    cycleTheme() { applyTheme(THEMES[(THEMES.indexOf(themePref) + 1) % THEMES.length]); return themePref; },
    bridge,
  };
})(window.Board = window.Board || {});
