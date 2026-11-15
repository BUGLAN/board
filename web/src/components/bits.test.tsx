import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agentMark } from '../lib/hosts';
import { ActorAvatar, AgentAvatar, Avatar, BrandMark, ColorPick, SYSTEM_NAME } from './bits';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

/** 模拟系统取色器：拖动时一串 input，确定时 change（浏览器就是这样发的）。
    值走原型上的 setter（用户改值不经过 React 装在元素上的 value 跟踪），React 才认得出值变了 */
const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
function pickInDialog(el: HTMLInputElement, ...colors: string[]) {
  for (const c of colors) {
    setValue.call(el, c);
    act(() => {
      el.dispatchEvent(new Event('input', { bubbles: true }));
    });
  }
  act(() => {
    el.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('系统身份的头像', () => {
  it('系统 = board：头像里是品牌图标（四根柱子），不再有时钟 emoji，悬停提示是 board', () => {
    act(() => root.render(<Avatar actor={{ kind: 'system', name: '' }} />));
    const el = host.querySelector('.avatar') as HTMLElement;
    expect(el.title).toBe('board');
    expect(SYSTEM_NAME).toBe('board');
    expect(el.textContent).not.toContain('⏱');
    expect(el.textContent).not.toContain('系统');
    expect(el.querySelectorAll('.brand-bars i')).toHaveLength(4);
  });

  it('品牌图标的大小图标用同一组柱子、高度比例不变', () => {
    act(() => root.render(<BrandMark className="logo" />));
    const bars = [...host.querySelectorAll('i')] as HTMLElement[];
    expect(bars.map(b => b.style.getPropertyValue('--h'))).toEqual(['100%', '70%', '46%', '82%']);
  });
});

describe('按名字画 agent 头像（AgentAvatar）', () => {
  it('认得出标识：画 agent 自己的图标', () => {
    act(() => root.render(<AgentAvatar name="dsh-win/deepseek-v4.1-flash" />));
    const el = host.querySelector('.avatar.agent-logo') as HTMLElement;
    expect(el.title).toBe('dsh-win/deepseek-v4.1-flash');
    expect(el.querySelector('img')?.getAttribute('src')).toBeTruthy();
    // 是图标，不是字母
    expect(el.textContent.trim()).toBe('');
  });

  it('codex 也有自己的图标，顶上、卡片上同一套解析都画得出来', () => {
    act(() => root.render(<AgentAvatar name="codex-win/codex" />));
    const el = host.querySelector('.avatar.agent-logo') as HTMLElement;
    // 图标是对应 agent 自己的（测试里 SVG 内联成 data: URL，用 MARKS 里的图标比）
    expect(el.querySelector('img')?.getAttribute('src')).toBe(agentMark('codex').icon);
    // 颜色沿用 MARKS 里的主色（悬停展开的胶囊底色用它）
    expect(el.style.getPropertyValue('--c')).toBe('#10A37F');
  });

  it('认不出：退回字母头像（和原来一样）', () => {
    act(() => root.render(<AgentAvatar name="alpha" />));
    const el = host.querySelector('.avatar') as HTMLElement;
    expect(el.classList.contains('agent-logo')).toBe(false);
    expect(el.textContent).toBe('α');
  });

  it('名字为空时什么都不画', () => {
    act(() => root.render(<AgentAvatar name="" />));
    expect(host.innerHTML).toBe('');
    act(() => root.render(<AgentAvatar name={null} />));
    expect(host.innerHTML).toBe('');
  });
});

describe('按身份画头像（ActorAvatar）：动态、列表里 agent 也用自己的图标', () => {
  it('agent 认得出标识时画图标（卡片详情「领取信息」、动态、评论都是这个样子）', () => {
    act(() => root.render(<ActorAvatar actor={{ kind: 'agent', name: 'claude/claude' }} className="lg" />));
    const el = host.querySelector('.avatar.agent-logo.lg') as HTMLElement;
    expect(el.querySelector('img')?.getAttribute('src')).toBe(agentMark('claude').icon);
  });

  it('agent 认不出时退回字母头像，大小还是原来的位子', () => {
    act(() => root.render(<ActorAvatar actor={{ kind: 'agent', name: 'alpha' }} className="ib-av" />));
    const el = host.querySelector('.avatar.ib-av') as HTMLElement;
    expect(el.classList.contains('agent-logo')).toBe(false);
    expect(el.textContent).toBe('α');
  });

  it('系统身份还是 board 的品牌图标（不受 agent 解析影响）', () => {
    act(() => root.render(<ActorAvatar actor={{ kind: 'system', name: 'board' }} />));
    const el = host.querySelector('.avatar') as HTMLElement;
    expect(el.querySelectorAll('.brand-bars i')).toHaveLength(4);
  });
});

describe('ColorPick', () => {
  it('取色器里拖动后确定，回调拿到选中的颜色（拖动时的 input 不会把值还原成旧颜色）', () => {
    const onPick = vi.fn();
    act(() => root.render(<ColorPick value="#F43F5E" custom={false} onPick={onPick} />));
    const el = host.querySelector('input[type=color]') as HTMLInputElement;
    pickInDialog(el, '#30c010', '#27c412');
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(onPick).toHaveBeenCalledWith('#27C412');
  });

  it('颜色从外面改了，取色器的初始颜色跟着变', () => {
    const onPick = vi.fn();
    act(() => root.render(<ColorPick value="#F43F5E" custom={false} onPick={onPick} />));
    const el = host.querySelector('input[type=color]') as HTMLInputElement;
    pickInDialog(el, '#27c412');
    act(() => root.render(<ColorPick value="#3B82F6" custom={false} onPick={onPick} />));
    expect(el.value).toBe('#3b82f6');
    act(() => root.render(<ColorPick value="#27C412" custom onPick={onPick} />));
    expect(el.value).toBe('#27c412');
    expect(host.querySelector('label')?.classList.contains('on')).toBe(true);
  });
});
