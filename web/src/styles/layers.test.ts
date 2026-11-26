/* 卡片拖拽 / 落位的幽灵（.drag-ghost）在弹窗层之下（default/main#671）：
   在卡片详情的「移动到」里换状态时，落位动画在遮罩下面播，不能盖住弹窗 */
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

const read = (name: string) => readFileSync(new URL(`./${name}`, pathToFileURL(__filename)), 'utf8');

/** 行首的 `<selector> {` 规则块（含花括号） */
const rule = (src: string, selector: string) => {
  const m = new RegExp(`^${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')} \\{`, 'm').exec(src);
  if (!m) throw new Error(`找不到 ${selector}`);
  const open = m.index + m[0].length - 1;
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === '{') depth++;
    else if (src[i] === '}' && --depth === 0) return src.slice(open, i + 1);
  }
  throw new Error('花括号不配对');
};

const z = (file: string, selector: string) => {
  const m = /z-index:\s*(-?\d+)/.exec(rule(read(file), selector));
  if (!m) throw new Error(`${selector} 没写 z-index`);
  return Number(m[1]);
};

/** 挂在 body 上、和幽灵在同一个层叠上下文里的遮罩、弹窗 */
const overlays: [string, string][] = [
  ['overlays.css', '.scrim'],
  ['overlays.css', '.drawer'],
  ['overlays.css', '.palette-wrap'],
  ['overlays.css', '.dialog-wrap'],
  ['review.css', '.rv-scrim'],
  ['review.css', '.drawer.rv-modal'],
  ['hosts.css', '.ss-scrim'],
  ['hosts.css', '.drawer.ss-modal'],
  ['hosts.css', '.fm-scrim'],
  ['hosts.css', '.fm-wrap'],
  ['hosts.css', '.hc-full'],
  ['hostdetail.css', '.hd-sheet-scrim'],
  ['hostdetail.css', '.hd-sheet'],
];

describe('拖拽、落位的幽灵不盖住弹窗', () => {
  it('幽灵挂在 body 上（fixed），层级低于所有遮罩和弹窗', () => {
    expect(rule(read('board.css'), '.drag-ghost')).toMatch(/position: fixed/);
    const ghost = z('board.css', '.drag-ghost');
    for (const [file, sel] of overlays) expect(ghost, `${sel}（${file}）`).toBeLessThan(z(file, sel));
  });

  it('仍在页面内容之上：拖动时不被状态块、侧栏、顶栏挡住，和未整理页的飞行小卡片同一层', () => {
    const ghost = z('board.css', '.drag-ghost');
    expect(ghost).toBeGreaterThan(z('layout.css', '.app'));
    expect(ghost).toBe(z('inbox.css', '.ib-fly'));
  });
});
