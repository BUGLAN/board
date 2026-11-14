/* 图片放大预览、未整理展开行的回归测试（default/main#320）。jsdom 不排版，尺寸相关的约束按样式表里的规则核对 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { closeImage, openImage, stepImage } from '../../store/ui';
import { Lightbox } from './Lightbox';

// vitest 里 CSS 的 ?raw 是空的，直接读文件（jsdom 环境里 import.meta.url 不是 file: 地址，用 __dirname）
const css = (name: string) => readFileSync(resolve(__dirname, '../../styles', name), 'utf8');
const appCss = css('app.css');
const inboxCss = css('inbox.css');

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root.render(<Lightbox />));
});

afterEach(() => {
  act(() => closeImage());
  act(() => root.unmount());
  host.remove();
});

/** 样式表里某个选择器那一条规则的声明（选择器要和样式表里写的一样） */
function rule(css: string, selector: string): string {
  const flat = css.replace(/\/\*[\s\S]*?\*\//g, '');
  // 只认独立的一条（行首），不认 `.x .ib-edit {` 这种带前缀的
  const esc = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const m = new RegExp(`(^|\\n)${esc} \\{`).exec(flat);
  expect(m, `样式表里没有 ${selector}`).not.toBeNull();
  const open = flat.indexOf('{', m!.index + m![0].length - 1);
  return flat.slice(open + 1, flat.indexOf('}', open)).replace(/\s+/g, ' ');
}

describe('图片放大预览', () => {
  it('底部条只有文件名和切换，没有下载按钮', () => {
    const list = [
      { src: '/api/files/1', alt: 'a.png' },
      { src: '/api/files/2', alt: 'b.png' },
    ];
    act(() => openImage(list[0].src, list[0].alt, list));
    const bar = host.querySelector('.lb-bar')!;
    expect(bar.querySelector('.lb-name')?.textContent).toBe('a.png');
    expect(host.querySelector('a[download]')).toBeNull();
    expect([...bar.querySelectorAll('a, button')].map(b => b.textContent?.trim() || b.getAttribute('aria-label'))).toEqual(['上一张', '下一张']);
    act(() => stepImage(1));
    expect(host.querySelector('.lb-pos')?.textContent).toBe('2 / 2');
    expect(host.querySelector('img')?.getAttribute('src')).toBe('/api/files/2');
  });

  it('按视口宽高一起缩放：网格轨道不按图片原尺寸撑开，图片宽高都不超出', () => {
    const box = rule(appCss, '.lightbox');
    expect(box).toContain('grid-template: minmax(0, 1fr) / minmax(0, 1fr)');
    const img = rule(appCss, '.lightbox img');
    expect(img).toContain('max-width: 100%');
    expect(img).toContain('max-height: 100%');
    expect(img).toContain('object-fit: contain');
  });
});

describe('未整理展开行', () => {
  it('编辑区的列宽不按内容撑开（长代码块不会把按钮挤出右边）', () => {
    expect(rule(inboxCss, '.ib-exp')).toContain('grid-template-columns: minmax(0, 1fr)');
    expect(rule(inboxCss, '.ib-exp > div')).toContain('min-width: 0');
    expect(rule(inboxCss, '.ib-edit')).toContain('grid-template-columns: minmax(0, 1fr)');
    expect(rule(inboxCss, '.ib-editacts')).toContain('flex-wrap: wrap');
  });
});
