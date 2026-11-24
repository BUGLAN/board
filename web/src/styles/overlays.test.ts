/* 卡片详情开关后的移动端滚动（default/main#637）：关着的遮罩和抽屉不留下全屏模糊合成层 */
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

const css = readFileSync(new URL('./overlays.css', pathToFileURL(__filename)), 'utf8');

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

const rule = (selector: string) => block(css, css.indexOf(selector));

describe('卡片详情关掉后不留模糊层', () => {
  it('遮罩关着时隐藏、没有 backdrop-filter，打开才建模糊层', () => {
    const off = rule('.scrim {');
    expect(off).toMatch(/visibility: hidden/);
    expect(off).toMatch(/backdrop-filter: none/);
    expect(off).toMatch(/visibility 0s linear var\(--dur-3\)/);

    const on = rule('.scrim.on {');
    expect(on).toMatch(/visibility: visible/);
    expect(on).toMatch(/backdrop-filter: blur\(2px\)/);
    expect(on).toMatch(/visibility 0s[;}]/);
  });

  it('抽屉关着时没有 backdrop-filter，打开才建模糊层', () => {
    expect(rule('.drawer {')).toMatch(/backdrop-filter: none/);

    const on = rule('.drawer.on {');
    expect(on).toMatch(/backdrop-filter: blur\(28px\) saturate\(1\.5\)/);
  });
});

describe('关着的弹窗里不留能滚动的区域（default/main#651）', () => {
  it('没有 .on 的抽屉里一律 overflow: hidden，打开时不受影响', () => {
    expect(rule('.drawer:not(.on) * {')).toMatch(/overflow: hidden !important/);
  });
});
