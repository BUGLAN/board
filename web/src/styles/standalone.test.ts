/* iOS 主屏安装版顶部的系统级模糊（default/main#627）：实色层只在 iOS standalone 下出现，顶栏让出安全区 */
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

const css = (name: string) => readFileSync(new URL(`./${name}`, pathToFileURL(__filename)), 'utf8');

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
  const platform = css('platform.css');

  it('body::before 只写在 iOS 的 standalone 里', () => {
    const at = platform.indexOf('@media (display-mode: standalone)');
    expect(at).toBeGreaterThan(-1);
    const media = block(platform, at);
    expect(media).toContain('@supports (-webkit-touch-callout: none)');
    expect(media).toContain('body::before');
    // 别处不再出现 body::before：Safari 里打开、Android、桌面都不受影响
    expect(platform.split('body::before').length).toBe(2);
    for (const name of ['base.css', 'layout.css', 'app.css', 'overlays.css']) expect(css(name)).not.toContain('body::before');
  });

  it('是贴顶、实色、不挡点击的 fixed 层，至少 6px 高、盖住状态栏', () => {
    const media = block(platform, platform.indexOf('@media (display-mode: standalone)'));
    const rule = block(media, media.indexOf('body::before'));
    expect(rule).toMatch(/position:\s*fixed/);
    expect(rule).toMatch(/inset:\s*0 0 auto 0/);
    expect(rule).toMatch(/height:\s*max\(6px,\s*env\(safe-area-inset-top/);
    expect(rule).toMatch(/background:\s*var\(--bg\)/);
    expect(rule).not.toMatch(/gradient|rgba|transparent|opacity/);
    expect(rule).toMatch(/pointer-events:\s*none/);
  });

  it('顶栏这一行算上顶部安全区，内容排在状态栏下面', () => {
    const layout = css('layout.css');
    expect(layout).toMatch(/grid-template-rows:\s*calc\(var\(--topbar-h\) \+ env\(safe-area-inset-top, 0px\)\) minmax\(0, 1fr\);/);
    expect(layout).toMatch(/\.topbar \{[^}]*padding:\s*env\(safe-area-inset-top, 0px\) 18px 0 20px;/);
    expect(platform).toMatch(/grid-template-rows:\s*calc\(var\(--topbar-h\) \+ env\(safe-area-inset-top, 0px\)\) minmax\(0, 1fr\) calc\(60px/);
    expect(platform).toMatch(/\.topbar \{[^}]*padding-top:\s*env\(safe-area-inset-top\)/);
  });
});
