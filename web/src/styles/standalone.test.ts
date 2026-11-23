/* iOS 主屏安装版顶部的系统级模糊（default/main#627）：实色层是 index.html 里的真实元素、首帧就在，只在 iOS standalone 下出现，顶栏让出安全区 */
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

const read = (rel: string) => readFileSync(new URL(rel, pathToFileURL(__filename)), 'utf8');
const css = (name: string) => read(`./${name}`);

/** 从 start 起取一个花括号块（含外层的花括号） */
const block = (src: string, start: number) => {
  const open = src.indexOf('{', start);
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === '{') depth++;
    else if (src[i] === '}' && --depth === 0) return src.slice(open, i + 1);
  }
  throw new Error('花括号不配对');
};

describe('iOS 主屏安装版顶部实色层', () => {
  const html = read('../../index.html');
  const style = html.slice(html.indexOf('<style>'), html.indexOf('</style>'));

  it('是 body 里第一个真实元素，样式内联在 index.html：首帧就在，不等样式包', () => {
    expect(html).toMatch(/<body>\s*<div class="ios-top" aria-hidden="true"><\/div>\s*<div id="root"><\/div>/);
    // 不再用伪元素（首次打开认不到，滚一下才好）
    for (const name of ['platform.css', 'base.css', 'layout.css', 'app.css', 'overlays.css']) expect(css(name)).not.toContain('body::before');
  });

  it('默认不显示，只在 iOS 的 standalone 里出现', () => {
    expect(style).toMatch(/\.ios-top \{ display: none; \}/);
    const media = block(style, style.indexOf('@media (display-mode: standalone)'));
    const supports = block(media, media.indexOf('@supports (-webkit-touch-callout: none)'));
    expect(supports).toMatch(/\.ios-top \{[^}]*display: block/);
    // display: block 只出现在 standalone 里
    expect(style.split('display: block').length).toBe(2);
  });

  it('是贴顶、实色、不挡点击的 fixed 层，至少 6px 高、盖住状态栏，跟着主题', () => {
    const media = block(style, style.indexOf('@media (display-mode: standalone)'));
    const rule = block(media, media.indexOf('.ios-top {'));
    expect(rule).toMatch(/position: fixed/);
    expect(rule).toMatch(/inset: 0 0 auto 0/);
    expect(rule).toMatch(/height: max\(6px, env\(safe-area-inset-top/);
    expect(rule).toMatch(/background: var\(--bg, #F2F0EB\)/);
    expect(rule).not.toMatch(/gradient|rgba|transparent|opacity/);
    expect(rule).toMatch(/pointer-events: none/);
    expect(media).toMatch(/html\[data-theme="dark"\] \.ios-top \{ background: var\(--bg, #0C0D0F\); \}/);
    expect(media).toMatch(/prefers-color-scheme: dark\) \{ html:not\(\[data-theme="light"\]\) \.ios-top \{ background: var\(--bg, #0C0D0F\); \} \}/);
    // 动一下时高 1px
    expect(media).toMatch(/\.ios-top\.is-nudge \{ height: calc\(max\(6px, env\(safe-area-inset-top, 0px\)\) \+ 1px\); \}/);
  });

  it('顶栏这一行算上顶部安全区，内容排在状态栏下面', () => {
    const layout = css('layout.css');
    const platform = css('platform.css');
    expect(layout).toMatch(/grid-template-rows:\s*calc\(var\(--topbar-h\) \+ env\(safe-area-inset-top, 0px\)\) minmax\(0, 1fr\);/);
    expect(layout).toMatch(/\.topbar \{[^}]*padding:\s*env\(safe-area-inset-top, 0px\) 18px 0 20px;/);
    expect(platform).toMatch(/grid-template-rows:\s*calc\(var\(--topbar-h\) \+ env\(safe-area-inset-top, 0px\)\) minmax\(0, 1fr\) calc\(60px/);
    expect(platform).toMatch(/\.topbar \{[^}]*padding-top:\s*env\(safe-area-inset-top\)/);
  });
});
