import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { Lightbox } from './Lightbox';
import { closeImage, openImage, stepImage } from '../../store/ui';

// vitest 里 CSS 的 ?raw 是空的，直接读文件（jsdom 环境里 import.meta.url 不是 file: 地址，用 __dirname）
const css = (name: string) => readFileSync(resolve(__dirname, '../../styles', name), 'utf8');

/** 样式表里某个选择器那一条规则的声明（选择器要和样式表里写的一样，只认行首独立的一条） */
function rule(sheet: string, selector: string): string {
  const flat = sheet.replace(/\/\*[\s\S]*?\*\//g, '');
  const esc = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const m = new RegExp(`(^|\\n)${esc} \\{`).exec(flat);
  expect(m, `样式表里没有 ${selector}`).not.toBeNull();
  const open = flat.indexOf('{', m!.index + m![0].length - 1);
  return flat.slice(open + 1, flat.indexOf('}', open)).replace(/\s+/g, ' ');
}

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

const list = [
  { src: '/api/files/1', alt: 'before314-tip.png' },
  { src: '/api/files/2', alt: 'after314-tip.png' },
];

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => openImage(list[0].src, list[0].alt, list));
});

afterEach(() => {
  act(() => closeImage());
  act(() => root.unmount());
  host.remove();
});

const stageImages = () => [...host.querySelectorAll('.lb-stage .lb-img')] as HTMLImageElement[];
const back = () => host.querySelector('.lb-img.lb-prev') as HTMLImageElement | null;
const front = () => stageImages().find(i => !i.classList.contains('lb-prev'))!;
const box = () => host.querySelector('.lightbox') as HTMLElement;

// jsdom 不画图、也不发 load / animationend：直接调 React 挂在 img 上的处理函数。
// 这里不能手动派发原生事件：React 把冒泡的 animationend 委托在根容器上，本文件把组件挂在临时 div 里渲染，
// 容器所在的 fiber 没有 alternate，React 认不出这个事件，处理函数根本不跑
const fire = (img: HTMLImageElement, prop: 'onLoad' | 'onAnimationEnd') => {
  const key = Object.keys(img).find(k => k.startsWith('__reactProps$'))!;
  const props = (img as unknown as Record<string, Record<string, (e: unknown) => void>>)[key];
  act(() => props[prop]?.({ currentTarget: img, target: img }));
};
const loaded = (img: HTMLImageElement) => fire(img, 'onLoad');
const fadeDone = (img: HTMLImageElement) => fire(img, 'onAnimationEnd');

/** 没有上一层要垫、新图已经显示出来了（不会停在内联的透明度 0） */
const clear = () => {
  expect(front().style.opacity).toBe('');
  expect(back()).toBeNull();
};

describe('图片预览（default/main#328 切换图片时整屏闪烁）', () => {
  it('切换图片时预览层和图片那一层都不重建，只换里面的图', () => {
    act(() => root.render(<Lightbox />));
    loaded(front());
    fadeDone(front());
    const shell = box();
    const stage = host.querySelector('.lb-stage');
    expect(shell).toBeTruthy();
    expect(stage).toBeTruthy();

    act(() => stepImage(1));
    expect(host.querySelector('.lightbox')).toBe(shell);
    expect(host.querySelector('.lb-stage')).toBe(stage);
    expect(front().getAttribute('src')).toBe(list[1].src);
    expect(host.querySelector('.lb-pos')?.textContent).toContain('2 / 2');
    expect(host.querySelector('.lb-name')?.textContent).toBe(list[1].alt);
  });

  it('新图解码好之前，上一张还稳稳地显示着，不会出现只有遮罩、没有图的一帧', () => {
    act(() => root.render(<Lightbox />));
    loaded(front());
    fadeDone(front());

    act(() => stepImage(1));
    // 新图还没解码，先只显示上一张（完整的图）
    expect(back()?.getAttribute('src')).toBe(list[0].src);
    expect(front().style.opacity).toBe('0');

    // 解码完：新图淡入，上一张还垫着；淡入跑完才撤掉
    loaded(front());
    expect(front().style.opacity).toBe('');
    expect(back()?.getAttribute('src')).toBe(list[0].src);
    fadeDone(front());
    expect(back()).toBeNull();
  });

  it('慢网：上一张还没加载完就切下一张，第一张一直留着当底，不会露出底层页面', () => {
    act(() => root.render(<Lightbox />));
    loaded(front());
    fadeDone(front());
    act(() => stepImage(1));
    // 第二张还在加载就点「上一张」：第一张（已经画出来的那张）接着垫
    act(() => stepImage(-1));
    loaded(front());
    expect(front().getAttribute('src')).toBe(list[0].src);
    fadeDone(front());
    expect(back()).toBeNull();
  });

  it('图片慢、一直不加载：新图自己藏着（透明度 0），不会露出底层页面', () => {
    act(() => root.render(<Lightbox />));
    loaded(front());
    fadeDone(front());
    act(() => stepImage(1));
    // 加载完之前：新图一直是透明度 0
    expect(front().getAttribute('src')).toBe(list[1].src);
    expect(front().style.opacity).toBe('0');
  });

  it('加载失败也照样收尾：破图占位显示出来，上一张再垫着', () => {
    act(() => root.render(<Lightbox />));
    loaded(front());
    fadeDone(front());
    act(() => stepImage(1));
    act(() => void front().dispatchEvent(new Event('error')));
    expect(front().style.opacity).toBe('');
    expect(back()?.getAttribute('src')).toBe(list[0].src);
    act(() => stepImage(-1));
    expect(back()?.getAttribute('src')).toBe(list[1].src);
  });

  it('已经看过一遍的图，切回来也重新淡入（新节点、新的动画锚点），跑完照样撤上一张', () => {
    act(() => root.render(<Lightbox />));
    loaded(front());
    fadeDone(front());
    act(() => stepImage(1));
    loaded(front());
    fadeDone(front());

    act(() => stepImage(-1));
    expect(front().getAttribute('src')).toBe(list[0].src);
    loaded(front());
    expect(front().style.opacity).toBe('');
    fadeDone(front());
    expect(back()).toBeNull();
  });

  it('键盘 ← → 切换同样走这一层，不重建预览层', () => {
    act(() => root.render(<Lightbox />));
    loaded(front());
    const shell = box();
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight' }));
    });
    expect(host.querySelector('.lightbox')).toBe(shell);
    expect(front().getAttribute('src')).toBe(list[1].src);
    loaded(front());
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft' }));
    });
    expect(host.querySelector('.lightbox')).toBe(shell);
    expect(front().getAttribute('src')).toBe(list[0].src);
  });

  it('先打开 A，关掉，再打开 B：淡入重新跑，图能显示出来（不会被内联的透明度 0 卡住）', () => {
    act(() => root.render(<Lightbox />));
    loaded(front());
    fadeDone(front());
    act(() => closeImage());
    expect(host.querySelector('.lightbox')).toBeNull();

    act(() => openImage(list[1].src, list[1].alt, list));
    expect(front().getAttribute('src')).toBe(list[1].src);
    loaded(front());
    fadeDone(front());
    expect(clear());
  });

  it('关掉时图片还没加载完，再打开同一张：从头来，不残留上一轮的底图', () => {
    act(() => root.render(<Lightbox />));
    loaded(front());
    fadeDone(front());
    act(() => stepImage(1));
    expect(back()).not.toBeNull();

    act(() => closeImage());
    expect(host.querySelector('.lightbox')).toBeNull();
    act(() => openImage(list[1].src, list[1].alt, list));
    expect(back()).toBeNull();
    expect(front().getAttribute('src')).toBe(list[1].src);
  });

  it('切换、关闭时现有交互不变：首尾按钮禁用，Esc 关闭，点遮罩关闭', () => {
    act(() => root.render(<Lightbox />));
    const [prevBtn, nextBtn] = [...host.querySelectorAll('.lb-nav button')] as HTMLButtonElement[];
    expect(prevBtn.disabled).toBe(true);
    expect(nextBtn.disabled).toBe(false);
    act(() => stepImage(1));
    const [prevBtn2, nextBtn2] = [...host.querySelectorAll('.lb-nav button')] as HTMLButtonElement[];
    expect(prevBtn2.disabled).toBe(false);
    expect(nextBtn2.disabled).toBe(true);

    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    expect(host.querySelector('.lightbox')).toBeNull();

    act(() => openImage(list[0].src, list[0].alt, list));
    act(() => void box().click());
    expect(host.querySelector('.lightbox')).toBeNull();
  });

  it('单图：没有切换按钮，也不垫上一张', () => {
    act(() => closeImage());
    act(() => openImage('/api/files/9', 'only.png'));
    act(() => root.render(<Lightbox />));
    expect(host.querySelector('.lb-nav')).toBeNull();
    expect(back()).toBeNull();
    expect(front().getAttribute('src')).toBe('/api/files/9');
    loaded(front());
    fadeDone(front());
    expect(clear());
  });
});




// 下面几条是 default/main#320 的回归测试：曾在 #328 解 rebase 冲突时被整份测试文件覆盖掉，下载按钮跟着回来了
describe('图片预览底部条（default/main#320）', () => {
  it('只有文件名和切换，没有下载按钮', () => {
    act(() => root.render(<Lightbox />));
    const bar = host.querySelector('.lb-bar')!;
    expect(bar.querySelector('.lb-name')?.textContent).toBe(list[0].alt);
    expect(host.querySelector('a[download]')).toBeNull();
    expect([...bar.querySelectorAll('a, button')].map(b => b.textContent?.trim() || b.getAttribute('aria-label'))).toEqual(['上一张', '下一张']);
  });
});

describe('预览和未整理展开行的网格约束（default/main#320，jsdom 不排版，按样式表核对）', () => {
  it('图片按视口宽高一起缩放：网格轨道不按图片原尺寸撑开，图片宽高都不超出', () => {
    const appCss = css('app.css');
    expect(rule(appCss, '.lightbox')).toContain('grid-template: minmax(0, 1fr) / minmax(0, 1fr)');
    expect(rule(appCss, '.lb-stage')).toContain('grid-template: minmax(0, 1fr) / minmax(0, 1fr)');
    const img = rule(appCss, '.lightbox .lb-img');
    expect(img).toContain('max-width: 100%');
    expect(img).toContain('max-height: 100%');
    expect(img).toContain('object-fit: contain');
  });

  it('未整理编辑区的列宽不按内容撑开（长代码块不会把按钮挤出右边）', () => {
    const inboxCss = css('inbox.css');
    expect(rule(inboxCss, '.ib-exp')).toContain('grid-template-columns: minmax(0, 1fr)');
    expect(rule(inboxCss, '.ib-exp > div')).toContain('min-width: 0');
    expect(rule(inboxCss, '.ib-edit')).toContain('grid-template-columns: minmax(0, 1fr)');
    expect(rule(inboxCss, '.ib-editacts')).toContain('flex-wrap: wrap');
  });
});
