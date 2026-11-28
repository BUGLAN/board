/* 卡片详情的按键（default/main#684），Windows / Linux（jsdom 不是 macOS）：
   - 输入法回车上屏、Esc 取消组字（keyCode 229）不提交标题、不退出描述和评论的编辑，内容不丢；
   - Alt+← / Alt+→ 是返回 / 前进，但在输入框里不换卡（写到一半的内容会丢），只挡住浏览器的后退 */
import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cardBack, cardFwd } from '../lib/actions';
import { exec, useData } from '../store/data';
import { mount, press, type, unmount } from '../../test/detailKeys';

vi.mock('../lib/actions', async importOriginal => ({ ...(await importOriginal<typeof import('../lib/actions')>()), cardBack: vi.fn(), cardFwd: vi.fn() }));
vi.mock('../store/data', async importOriginal => ({ ...(await importOriginal<typeof import('../store/data')>()), exec: vi.fn(() => Promise.resolve()) }));

let host: HTMLElement;
beforeEach(() => {
  vi.mocked(cardBack).mockReset();
  vi.mocked(cardFwd).mockReset();
  vi.mocked(exec).mockClear();
  host = mount();
});
afterEach(() => unmount());

const title = () => host.querySelector<HTMLTextAreaElement>('textarea.d-title')!;
const comment = () => host.querySelector<HTMLTextAreaElement>('textarea[aria-label="评论"]')!;

describe('输入法组字时的回车、Esc', () => {
  it('标题：回车上屏不提交、不失焦，Esc 取消组字不还原；平常的 Esc 才还原', () => {
    act(() => title().focus());
    type(title(), '修复 api');
    press(title(), { key: 'Enter', keyCode: 229 });
    expect(document.activeElement).toBe(title());
    expect(exec).not.toHaveBeenCalled();
    press(title(), { key: 'Escape', keyCode: 229 });
    press(title(), { key: 'Escape', isComposing: true });
    expect(title().value).toBe('修复 api');
    expect(useData.getState().detailId).toBe(2);
    press(title(), { key: 'Escape' });
    expect(title().value).toBe('修一下横轴');
  });

  it('描述：Esc 取消组字不退出编辑，没保存的内容还在', () => {
    const editBtn = [...host.querySelectorAll<HTMLButtonElement>('[data-part="desc"] button')].find(b => b.textContent?.includes('编辑'))!;
    act(() => editBtn.click());
    const ta = () => host.querySelector<HTMLTextAreaElement>('[data-part="desc"] textarea')!;
    expect(ta()).not.toBeNull();
    type(ta(), '写了一大段 pinyin');
    press(ta(), { key: 'Escape', keyCode: 229 });
    expect(ta()?.value).toBe('写了一大段 pinyin');
    press(ta(), { key: 'Escape' });
    expect(host.querySelector('[data-part="desc"] textarea')).toBeNull();
  });

  it('改评论：Esc 取消组字不退出编辑', () => {
    act(() => host.querySelector<HTMLButtonElement>('.tl-edit')!.click());
    const ta = () => host.querySelector<HTMLTextAreaElement>('textarea[aria-label="修改评论"]');
    expect(ta()).not.toBeNull();
    press(ta()!, { key: 'Escape', keyCode: 229 });
    expect(ta()).not.toBeNull();
    press(ta()!, { key: 'Escape' });
    expect(ta()).toBeNull();
  });
});

describe('返回 / 前进（Windows、Linux：Alt+← / Alt+→）', () => {
  it('焦点不在输入框里：换卡', () => {
    press(document.body, { key: 'ArrowLeft', altKey: true });
    expect(cardBack).toHaveBeenCalledTimes(1);
    press(document.body, { key: 'ArrowRight', altKey: true });
    expect(cardFwd).toHaveBeenCalledTimes(1);
  });

  it('在评论框里：挡住浏览器后退，但不换卡', () => {
    const e = press(comment(), { key: 'ArrowLeft', altKey: true });
    expect(e.defaultPrevented).toBe(true);
    expect(cardBack).not.toHaveBeenCalled();
    expect(useData.getState().detailId).toBe(2);
  });

  it('Shift+Alt+← 不是返回', () => {
    press(document.body, { key: 'ArrowLeft', altKey: true, shiftKey: true });
    expect(cardBack).not.toHaveBeenCalled();
  });
});
