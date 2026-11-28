/* 手机上持续耗电（default/main#698）：常驻的模糊层只在要用时才建，常驻的循环动画在触屏、减弱动效下停下 */
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

const read = (f: string) => readFileSync(new URL(`./${f}`, pathToFileURL(__filename)), 'utf8');
const board = read('board.css');
const settings = read('settings.css');
const platform = read('platform.css');
const inbox = read('inbox.css');
const labels = read('labels.css');

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

/** `head` 第一次出现处开始的那个块 */
const rule = (src: string, head: string) => {
  const at = src.indexOf(head);
  expect(at, `找不到 ${head}`).toBeGreaterThanOrEqual(0);
  return block(src, at);
};

/** 所有 `head` 开头的块（同一个媒体查询可能写了几处），拼在一起 */
const all = (src: string, head: string) => {
  const out: string[] = [];
  for (let at = src.indexOf(head); at >= 0; at = src.indexOf(head, at + 1)) out.push(block(src, at));
  return out.join('\n');
};

/** 块里停掉动画的选择器（`animation: none` 那几条规则的选择器，逗号拆开） */
const stopped = (css: string) =>
  [...css.matchAll(/([^{}]+)\{[^{}]*animation:\s*none[^{}]*\}/g)].flatMap(m => m[1].split(',').map(s => s.trim()).filter(Boolean));

describe('状态块的模糊层', () => {
  it('禁止放入的提示层平时隐藏、没有 backdrop-filter，拖到不能放的块上才建模糊层', () => {
    const off = rule(board, '.col-veto {');
    expect(off).toMatch(/visibility: hidden/);
    expect(off).toMatch(/backdrop-filter: none/);
    expect(off).toMatch(/visibility 0s linear var\(--dur-2\)/);

    const on = rule(board, '.col.drop-bad .col-veto {');
    expect(on).toMatch(/visibility: visible/);
    expect(on).toMatch(/backdrop-filter: blur\(3px\)/);
    expect(on).toMatch(/visibility 0s[;}]/);
  });

  it('手机上状态块不常驻毛玻璃', () => {
    const phone = rule(platform, '@media (max-width: 760px) {');
    expect(block(phone, phone.indexOf('  .col {'))).toMatch(/backdrop-filter: none/);
  });
});

describe('平时隐藏的浮层（#732）', () => {
  for (const [name, src, off, on] of [
    ['批量操作条（白板、未整理共用）', inbox, '.ib-bulk {', '.ib-bulk.on {'],
    ['标签选择器', labels, '.lb-picker {', '.lb-picker.on {'],
  ] as const) {
    it(`${name}关着时没有 backdrop-filter，打开才建模糊层`, () => {
      const closed = rule(src, off);
      expect(closed).toMatch(/visibility: hidden/);
      expect(closed).toMatch(/backdrop-filter: none; -webkit-backdrop-filter: none;/);
      expect(closed).not.toMatch(/blur\(/);

      const open = rule(src, on);
      expect(open).toMatch(/visibility: visible/);
      expect(open).toMatch(/backdrop-filter: blur\(24px\) saturate\(1\.5\); -webkit-backdrop-filter: blur\(24px\) saturate\(1\.5\);/);
    });
  }
});

describe('常驻的循环动画', () => {
  it('触屏上流光边框换成静态描边，呼吸的圆点停下', () => {
    const touch = all(board, '@media (hover: none) {');
    expect(touch).toMatch(/\.card\.doing::before \{ animation: none; background:/);
    const still = stopped(touch);
    for (const s of ['.col:has(.card.doing) .col-dot', '.pj-elsewhere', '.pj-live i', '.pt-live i']) expect(still).toContain(s);
  });

  it('系统「减少动态效果」停掉的和界面里的「减弱动效」偏好一样多', () => {
    const pref = stopped(settings.slice(settings.indexOf('/* ---------------- 偏好：减弱动效'), settings.indexOf('@media (prefers-reduced-motion: reduce)')))
      .filter(s => s.startsWith(':root[data-motion="reduced"] '))
      .map(s => s.replace(':root[data-motion="reduced"] ', ''));
    const system = [
      ...stopped(all(settings, '@media (prefers-reduced-motion: reduce)')),
      ...stopped(all(read('layout.css'), '@media (prefers-reduced-motion: reduce)')),
    ];
    expect(pref).toEqual(expect.arrayContaining(['.logo i', '.card.doing::before', '.col-dot', '.pj-elsewhere', '.pj-live i', '.pt-live i']));
    for (const s of pref) expect(system, `系统减少动态效果时 ${s} 没停`).toContain(s);
  });
});
