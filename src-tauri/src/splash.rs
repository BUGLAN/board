//! 启动页：窗口先打开本地的启动页，确认服务端连得上再跳到线上页面，慢网、断网时不白屏。
//!
//! - 启动页由自定义协议 `boardsplash` 直接返回内联 HTML，不依赖网络，开发模式和生产模式一样
//!   （macOS / Linux：`boardsplash://localhost/`；Windows：`http://boardsplash.localhost/`）
//! - 页面上和网页端加载中的样子一致（四个色条 logo +「正在打开白板…」），跳到线上页面后由网页端的加载态接上
//! - 先用 no-cors 请求探测服务端：通了就跳转；断网或 8 秒没响应显示「连不上 Board 服务」和重试，恢复网络时自动重试
//! - 最短展示 300ms，免得一闪而过；跳转时启动页内容 200ms 淡出
//! - 主题：启动页是本地页面，读不到线上页面的 localStorage。网页端每次应用主题时调用 `remember_theme`，
//!   外壳写到 app config 目录的 theme.txt（auto / light / dark），启动页和窗口底色按它来；
//!   跟随系统时启动页用 prefers-color-scheme，系统切换深浅色时同步变化
//! - Windows 没有系统标题栏：启动页自己画拖动区域和窗口按钮（与网页端的 TitleStrip 一致），画好后调用 `window_ready`

use std::fs;
use std::path::PathBuf;
use tauri::http::{header, Response};
use tauri::window::Color;
use tauri::{AppHandle, Manager, Runtime, Url, WebviewUrl};

pub const SCHEME: &str = "boardsplash";
const THEME_FILE: &str = "theme.txt";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Auto,
    Light,
    Dark,
}

impl Theme {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "auto" => Some(Self::Auto),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

fn theme_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(THEME_FILE))
}

/// 上次网页端用的主题；没有记录时跟随系统
pub fn load_theme<R: Runtime>(app: &AppHandle<R>) -> Theme {
    theme_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| Theme::parse(&s))
        .unwrap_or(Theme::Auto)
}

pub fn save_theme<R: Runtime>(app: &AppHandle<R>, theme: Theme) -> Result<(), String> {
    let path = theme_path(app).ok_or("找不到配置目录")?;
    if fs::read_to_string(&path).ok().and_then(|s| Theme::parse(&s)) == Some(theme) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&path, theme.as_str()).map_err(|e| e.to_string())
}

/// 窗口底色（启动页画出来之前、跳到线上页面途中露出来的颜色），与 tokens.css 的 --bg 一致
pub fn background(dark: bool) -> Color {
    if dark {
        Color(0x0C, 0x0D, 0x0F, 0xFF)
    } else {
        Color(0xF2, 0xF0, 0xEB, 0xFF)
    }
}

/// 启动页地址
pub fn url() -> WebviewUrl {
    if cfg!(any(windows, target_os = "android")) {
        WebviewUrl::External(format!("http://{SCHEME}.localhost/").parse().unwrap())
    } else {
        WebviewUrl::CustomProtocol(format!("{SCHEME}://localhost/").parse().unwrap())
    }
}

/// 自定义协议的响应：任何路径都返回启动页
pub fn respond<R: Runtime>(app: &AppHandle<R>, target: &Url) -> Response<Vec<u8>> {
    let html = page(target, load_theme(app), cfg!(target_os = "windows"));
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(html.into_bytes())
        .unwrap()
}

/// 放进 JS 字符串字面量（双引号）里的内容
fn js_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\u003c"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn html_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn page(target: &Url, theme: Theme, custom_titlebar: bool) -> String {
    let host = target.host_str().map(|h| match target.port() {
        Some(p) => format!("{h}:{p}"),
        None => h.to_string(),
    });
    let theme_attr = match theme {
        Theme::Auto => String::new(),
        t => format!(" data-theme=\"{}\"", t.as_str()),
    };
    TEMPLATE
        .replace("__THEME_ATTR__", &theme_attr)
        .replace("__TITLEBAR__", if custom_titlebar { " data-titlebar=\"custom\"" } else { "" })
        .replace("__HOST_JS__", &js_str(host.as_deref().unwrap_or(target.as_str())))
        .replace("__HOST__", &html_text(host.as_deref().unwrap_or(target.as_str())))
        .replace("__TARGET__", &js_str(target.as_str()))
}

const TEMPLATE: &str = r##"<!doctype html>
<html lang="zh-CN"__THEME_ATTR____TITLEBAR__>
<head>
<meta charset="utf-8">
<meta name="color-scheme" content="light dark">
<title>Board</title>
<style>
/* 颜色与网页端 tokens.css 一致；logo 用到的 --surface / --line-2 / --shadow-2 / 圆角也跟着主题走 */
:root {
  --bg: #F2F0EB; --surface: #FFFFFF;
  --line-2: rgba(26, 24, 20, .14);
  --shadow-2: 0 1px 0 rgba(26, 24, 20, .03), 0 6px 18px -6px rgba(26, 24, 20, .16);
  --text: #18181A; --text-2: #55555C; --text-3: #666670; --brand: #F2541B;
  --c1: #6E7891; --c2: #F07A12; --c3: #1D8FE0; --c4: #12A06A;
  --r-hair: 2px; --r-sm: 9px; --r-md: 13px;
  --f-sans: "Geist Variable", "Geist", "PingFang SC", "Noto Sans SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
  --f-display: "Bricolage Grotesque Variable", "Bricolage Grotesque", "PingFang SC", "Noto Sans SC", system-ui, sans-serif;
  --f-mono: "Geist Mono Variable", "Geist Mono", "SF Mono", ui-monospace, Menlo, Consolas, monospace;
  color-scheme: light;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #0C0D0F; --surface: #18191C;
    --line-2: rgba(255, 255, 255, .11);
    --shadow-2: 0 1px 0 rgba(255, 255, 255, .04) inset, 0 8px 22px -8px rgba(0, 0, 0, .7);
    --text: #EDECE8; --text-2: #A6A5AB; --text-3: #93929A; --brand: #FF6A33;
    --c1: #8F9AB4; --c2: #FF9A3C; --c3: #4DB5FF; --c4: #3DDC97;
    color-scheme: dark;
  }
}
:root[data-theme="dark"] {
  --bg: #0C0D0F; --surface: #18191C;
  --line-2: rgba(255, 255, 255, .11);
  --shadow-2: 0 1px 0 rgba(255, 255, 255, .04) inset, 0 8px 22px -8px rgba(0, 0, 0, .7);
  --text: #EDECE8; --text-2: #A6A5AB; --text-3: #93929A; --brand: #FF6A33;
  --c1: #8F9AB4; --c2: #FF9A3C; --c3: #4DB5FF; --c4: #3DDC97;
  color-scheme: dark;
}
* { box-sizing: border-box; margin: 0; }
html, body { height: 100%; }
body {
  display: flex; background: var(--bg); color: var(--text); font: 14px/1.5 var(--f-sans);
  -webkit-font-smoothing: antialiased; user-select: none; -webkit-user-select: none; cursor: default; overflow: hidden;
}
/* 与网页端 TitleStrip 一致：顶部 48px 拖动区域，Windows 右上角是窗口按钮 */
.strip { position: fixed; z-index: 2; inset: 0 0 auto 0; height: 48px; display: flex; justify-content: flex-end; }
.win { display: none; }
html[data-titlebar="custom"] .win { display: flex; }
.win button {
  width: 46px; height: 100%; display: grid; place-items: center; border: 0; background: none; color: var(--text-2);
  transition: background-color 90ms linear, color 90ms linear;
}
.win svg { width: 10px; height: 10px; fill: none; stroke: currentColor; stroke-width: 1px; overflow: visible; }
.win svg * { vector-effect: non-scaling-stroke; }
.win button:hover { color: var(--text); background: color-mix(in srgb, var(--text) 7%, transparent); }
.win button.close:hover { color: #fff; background: #C42B1C; }
/* 与网页端 .boot 一致 */
.boot {
  flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px;
  color: var(--text-3); font-size: 13px; text-align: center; padding: 24px;
  transition: opacity 200ms ease-out;
}
.boot.leave { opacity: 0; }
/* 与网页端 .logo（layout.css）一致：底色跟着主题走——浅色主题浅色面、深色主题深色面，
   不用反色；柱子掺 8% 当前主题的文字色，深浅两种底色上四根柱子都看得清 */
.logo {
  display: flex; align-items: flex-start; gap: 2.5px; width: 44px; height: 44px; padding: 10px 9px;
  border-radius: var(--r-md); background: var(--surface);
  box-shadow: inset 0 0 0 1px var(--line-2), var(--shadow-2);
}
.logo i { flex: 1; height: var(--h); border-radius: var(--r-hair); transform-origin: top; animation: bar 3.6s cubic-bezier(.65, 0, .35, 1) infinite; }
.logo i:nth-child(1) { background: color-mix(in srgb, var(--c1) 92%, var(--text)); }
.logo i:nth-child(2) { background: color-mix(in srgb, var(--c2) 92%, var(--text)); animation-delay: -.9s; }
.logo i:nth-child(3) { background: color-mix(in srgb, var(--c3) 92%, var(--text)); animation-delay: -1.8s; }
.logo i:nth-child(4) { background: color-mix(in srgb, var(--c4) 92%, var(--text)); animation-delay: -2.7s; }
@keyframes bar { 0%, 100% { transform: scaleY(1); } 50% { transform: scaleY(.62); } }
@media (prefers-reduced-motion: reduce) { .logo i { animation: none; } }
.err { display: none; flex-direction: column; align-items: center; gap: 12px; }
.boot.failed .err { display: flex; }
.boot.failed .wait { display: none; }
.err b { font-family: var(--f-display); font-size: 16px; color: var(--text); }
.err code { font-family: var(--f-mono); font-size: 12px; color: var(--text-2); }
.retry {
  height: 34px; padding: 0 16px; border: 0; border-radius: 10px; font: 600 13px var(--f-sans); color: #fff; background: var(--brand);
  box-shadow: 0 6px 18px -6px color-mix(in srgb, var(--brand) 70%, transparent); cursor: pointer;
}
.retry:active { transform: scale(.96); }
</style>
</head>
<body>
<div class="strip" data-tauri-drag-region>
  <div class="win">
    <button type="button" data-act="minimize" aria-label="最小化" title="最小化"><svg viewBox="0 0 10 10"><path d="M0 5.5h10"/></svg></button>
    <button type="button" data-act="toggle_maximize" aria-label="最大化" title="最大化"><svg viewBox="0 0 10 10"><rect x=".5" y=".5" width="9" height="9" rx="1"/></svg></button>
    <button type="button" data-act="close" class="close" aria-label="关闭" title="关闭"><svg viewBox="0 0 10 10"><path d="m.5.5 9 9m0-9-9 9"/></svg></button>
  </div>
</div>
<div class="boot" id="boot">
  <div class="logo" aria-hidden="true"><i style="--h:100%"></i><i style="--h:70%"></i><i style="--h:46%"></i><i style="--h:82%"></i></div>
  <span class="wait">正在打开白板…</span>
  <div class="err" role="alert">
    <b>连不上 Board 服务</b>
    <span id="why">__HOST__ 没有响应，检查网络后重试</span>
    <button type="button" class="retry" id="retry">重试</button>
  </div>
</div>
<script>
(() => {
  const target = __TARGET__;
  const MIN = 300, TIMEOUT = 8000;
  const boot = document.getElementById('boot');
  const why = document.getElementById('why');
  const ipc = window.__TAURI_INTERNALS__;
  const label = ipc && ipc.metadata && ipc.metadata.currentWindow && ipc.metadata.currentWindow.label;

  // Windows：窗口按钮能用了才告诉外壳（否则外壳 3 秒后恢复系统标题栏，窗口不会关不掉）
  if (ipc && label && document.documentElement.dataset.titlebar === 'custom') {
    document.querySelectorAll('.win [data-act]').forEach(b =>
      b.addEventListener('click', () => ipc.invoke('plugin:window|' + b.dataset.act, { label }).catch(() => {})));
    ipc.invoke('window_ready').catch(() => {});
  }

  let run = 0;
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  async function go() {
    const id = ++run;
    const t0 = Date.now();
    boot.classList.remove('failed', 'leave');
    let ok = false;
    try {
      const ctl = new AbortController();
      const timer = setTimeout(() => ctl.abort(), TIMEOUT);
      // no-cors：只关心连不连得上，不读内容
      await fetch(target, { mode: 'no-cors', cache: 'no-store', signal: ctl.signal });
      clearTimeout(timer);
      ok = true;
    } catch (e) {
      if (navigator.onLine === false) why.textContent = '网络已断开，恢复后会自动重试';
      else if (e && e.name === 'AbortError') why.textContent = __HOST_JS__ + ' 超过 8 秒没有响应，检查网络后重试';
      else why.textContent = __HOST_JS__ + ' 没有响应，检查网络后重试';
    }
    await sleep(Math.max(0, MIN - (Date.now() - t0)));
    if (id !== run) return;
    if (!ok) return boot.classList.add('failed');
    location.replace(target);
    boot.classList.add('leave');
  }
  document.getElementById('retry').addEventListener('click', go);
  window.addEventListener('online', () => boot.classList.contains('failed') && go());
  go();
})();
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_roundtrip() {
        for t in [Theme::Auto, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::parse(t.as_str()), Some(t));
        }
        assert_eq!(Theme::parse(" dark\n"), Some(Theme::Dark));
        assert_eq!(Theme::parse("blue"), None);
    }

    #[test]
    fn page_fills_placeholders() {
        let url: Url = "https://board.buglan.com".parse().unwrap();
        let html = page(&url, Theme::Dark, true);
        for p in ["__THEME_ATTR__", "__TITLEBAR__", "__HOST__", "__HOST_JS__", "__TARGET__"] {
            assert!(!html.contains(p), "占位符 {p} 没有替换");
        }
        assert!(html.contains(r#"data-theme="dark""#));
        assert!(html.contains(r#"data-titlebar="custom""#));
        assert!(html.contains(r#"const target = "https://board.buglan.com/";"#));
        assert!(html.contains("board.buglan.com 没有响应"));

        let dev: Url = "http://localhost:1420".parse().unwrap();
        let html = page(&dev, Theme::Auto, false);
        assert!(html.contains("<html lang=\"zh-CN\">"));
        assert!(html.contains("localhost:1420 没有响应"));
    }

    /// 取 CSS 里某个选择器之后的第一条声明块
    fn rule(html: &str, selector: &str) -> String {
        let start = html.find(selector).unwrap_or_else(|| panic!("没有 {selector}"));
        let end = start + html[start..].find('}').expect("样式块没有结束");
        html[start..end].to_string()
    }

    #[test]
    fn logo_uses_surface_not_invert() {
        let url: Url = "https://board.buglan.com".parse().unwrap();
        for theme in [Theme::Auto, Theme::Light, Theme::Dark] {
            let html = page(&url, theme, true);
            let logo = rule(&html, ".logo {");
            assert!(logo.contains("background: var(--surface);"), "{theme:?}：logo 底色应跟随主题面");
            assert!(!logo.contains("--invert"), "{theme:?}：logo 不再用反色");
            assert!(logo.contains("inset 0 0 0 1px var(--line-2)"), "{theme:?}：边框用 --line-2");
            assert!(logo.contains("var(--shadow-2)"), "{theme:?}：阴影用 --shadow-2");
            assert!(logo.contains("border-radius: var(--r-md)"), "{theme:?}：圆角 13px");
            assert!(logo.contains("width: 44px; height: 44px; padding: 10px 9px"), "{theme:?}：与网页端加载态同尺寸");
            for bar in 1..=4 {
                assert!(html.contains(&format!(".logo i:nth-child({bar}) {{ background: color-mix(in srgb, var(--c{bar}) 92%, var(--text));")), "{theme:?}：柱子 {bar} 掺文字色");
            }
        }
    }

    #[test]
    fn theme_tokens_match_web() {
        let url: Url = "https://board.buglan.com".parse().unwrap();
        let html = page(&url, Theme::Auto, true);
        // 网页端 tokens.css 的深色面、边框、文字色和列标识色
        let dark = rule(&html, r#":root[data-theme="dark"] {"#);
        for decl in [
            "--surface: #18191C",
            "--line-2: rgba(255, 255, 255, .11)",
            "--bg: #0C0D0F",
            "--text: #EDECE8",
            "--text-3: #93929A",
            "--c1: #8F9AB4",
            "--c4: #3DDC97",
        ] {
            assert!(dark.contains(decl), "深色主题缺 {decl}");
        }
        // 跟随系统深色：同样的值
        let auto_dark = rule(&html, r#":root:not([data-theme="light"]) {"#);
        assert!(auto_dark.contains("--surface: #18191C"));
        assert!(auto_dark.contains("--text-3: #93929A"));
        // 浅色主题是浅色面
        let light = rule(&html, ":root {");
        assert!(light.contains("--surface: #FFFFFF"));
        assert!(light.contains("--line-2: rgba(26, 24, 20, .14)"));
        assert!(light.contains("--text-3: #666670"));
        // 启动页不再引用网页端已经删掉的 --invert
        assert!(!html.contains("--invert"));
    }

    #[test]
    fn js_str_escapes() {
        assert_eq!(js_str(r#"a"b\c</script>"#), r#""a\"b\\c\u003c/script>""#);
    }
}
