import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { RangeBar } from './Stats';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root.render(<RangeBar loading={false} />));
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const btn = (name: string) => [...host.querySelectorAll<HTMLButtonElement>('.sp-seg button')].find(b => b.textContent === name)!;
const pop = () => host.querySelector('.sp-pop') as HTMLElement | null;
const input = (name: string) => host.querySelector<HTMLInputElement>(`.sp-pop input[aria-label="${name}"]`);
const note = () => host.querySelector('.sp-range-note')!.textContent!;
const click = (el: Element) => act(() => el.dispatchEvent(new MouseEvent('click', { bubbles: true })));
const outside = () => act(() => document.body.dispatchEvent(new Event('pointerdown', { bubbles: true })));
const esc = () => act(() => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
/** 用户改日期：走原型上的 setter，React 才认得出值变了（同 bits.test.tsx） */
const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
const typeDate = (el: HTMLInputElement, value: string) => {
  setValue.call(el, value);
  act(() => {
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
};

describe('大盘时间范围的自定义日期', () => {
  it('默认是预设范围：不在按钮下面单开一行日期输入', () => {
    expect(pop()).toBeNull();
    expect(host.querySelector('.sp-range input[type="date"]')).toBeNull();
    expect(note()).toContain('对比');
  });

  it('点「自定义」：日期输入在悬浮面板里，页面里没有多出来的行', () => {
    click(btn('自定义'));
    const p = pop()!;
    expect(p).not.toBeNull();
    expect(p.getAttribute('role')).toBe('dialog');
    // 日期输入只在浮层里，浮层挂在 .sp-range 末尾（fixed 定位，不占布局）
    expect(input('开始日期')).not.toBeNull();
    expect(input('结束日期')).not.toBeNull();
    expect(host.querySelectorAll('.sp-range input[type="date"]')).toHaveLength(2);
    expect(btn('自定义').getAttribute('aria-expanded')).toBe('true');
    expect(btn('自定义').getAttribute('aria-selected')).toBe('true');
  });

  it('再点「自定义」收起，日期保留', () => {
    click(btn('自定义'));
    click(btn('自定义'));
    expect(pop()).toBeNull();
    expect(btn('自定义').getAttribute('aria-expanded')).toBe('false');
    click(btn('自定义'));
    expect(input('开始日期')!.value).toBeTruthy();
  });

  it('点浮层外面、按 Esc 都收起', () => {
    click(btn('自定义'));
    outside();
    expect(pop()).toBeNull();
    click(btn('自定义'));
    esc();
    expect(pop()).toBeNull();
  });

  it('点浮层里的日期输入不会把它关掉', () => {
    click(btn('自定义'));
    act(() => input('开始日期')!.dispatchEvent(new Event('pointerdown', { bubbles: true })));
    expect(pop()).not.toBeNull();
  });

  it('选预设关掉浮层', () => {
    click(btn('自定义'));
    click(btn('3 天'));
    expect(pop()).toBeNull();
    expect(btn('3 天').getAttribute('aria-selected')).toBe('true');
  });

  it('改日期即时生效：当前区间和对比区间跟着变', () => {
    click(btn('自定义'));
    const before = note();
    typeDate(input('开始日期')!, '2026-01-05');
    expect(input('开始日期')!.value).toBe('2026-01-05');
    expect(note()).not.toBe(before);
    typeDate(input('结束日期')!, '2026-01-11');
    expect(note()).toContain('1月5日');
    expect(note()).toContain('1月11日');
    expect(note()).toContain('对比');
  });

  it('「正在更新」提示还在', () => {
    act(() => root.render(<RangeBar loading />));
    expect(host.querySelector('.sp-loading')!.textContent).toBe('正在更新');
  });
});
