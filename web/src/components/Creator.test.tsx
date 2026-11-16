import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { listPrompts, savePrompts } from '../lib/prompts';
import { useData } from '../store/data';
import { useUI } from '../store/ui';
import { Creator } from './Creator';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = new Date(2026, 10, 16, 10).getTime();
let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

const type = (el: HTMLInputElement | HTMLTextAreaElement, value: string) => {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')!.set!;
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
};

beforeEach(() => {
  localStorage.clear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState(
    {
      ws: {
        rev: 1,
        now: NOW,
        projects: [
          {
            key: 'default',
            name: '默认项目',
            desc: '',
            color: '#FF6A33',
            pos: 0,
            createdAt: NOW,
            labels: [],
            boards: [
              {
                key: 'main',
                path: 'default/main',
                name: '主白板',
                desc: '',
                pos: 0,
                createdAt: NOW,
                updatedAt: NOW,
                counts: {},
                statuses: [],
                holders: [],
              },
            ],
          },
        ],
      },
      path: 'default/main',
      board: null,
      cards: {},
      saving: {},
      pending: {},
    } as never,
  );
  useUI.setState({ creator: { open: true, column: 'todo', seq: 1 }, picker: null, labelsOf: null, tab: 'board' });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
});

const promptButton = () => [...host.querySelectorAll<HTMLButtonElement>('.cr-label .act button')].find(b => b.textContent?.includes('快捷提示词'))!;

describe('新建卡片里的快捷提示词（default/main#413）', () => {
  it('按钮在附件左边；点一条插到描述光标处、替换选中文字，浮层不关', async () => {
    savePrompts([
      { id: 'p-2', title: '重启', text: '重启一下' },
      { id: 'p-1', title: '等发版', text: '等发版' },
    ]);
    act(() => root.render(<Creator />));

    const btn = promptButton();
    expect(btn.getAttribute('aria-haspopup')).toBe('dialog');
    expect(btn.getAttribute('aria-expanded')).toBe('false');
    expect(btn.getAttribute('aria-controls')).toBe('cr-prompt-pop');
    expect(host.querySelector('.cr-prompts')!.nextElementSibling?.textContent).toContain('附件');

    act(() => btn.click());
    expect(btn.getAttribute('aria-expanded')).toBe('true');
    expect(host.querySelectorAll('.cr-prompt-insert')).toHaveLength(2);

    const ta = host.querySelector<HTMLTextAreaElement>('.cr-desc textarea')!;
    type(ta, '开头留空结尾');
    act(() => ta.focus());
    ta.setSelectionRange(2, 4);

    const row = [...host.querySelectorAll<HTMLButtonElement>('.cr-prompt-insert')].find(b => b.textContent?.includes('等发版'))!;
    await act(async () => {
      row.click();
      await new Promise<void>(resolve => setTimeout(resolve, 30));
    });

    expect(ta.value).toBe('开头等发版结尾');
    expect(ta.selectionStart).toBe(5);
    expect(ta.selectionEnd).toBe(5);
    expect(host.querySelector('.cr-prompt-pop')).not.toBeNull();
  });

  it('空列表给新建入口；新建、编辑、删除都写回本机，刷新后还在', async () => {
    act(() => root.render(<Creator />));
    act(() => promptButton().click());
    expect(host.querySelector('.cr-prompt-empty')?.textContent).toContain('还没有快捷提示词');

    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-empty button')!.click());
    type(host.querySelector<HTMLInputElement>('input[aria-label="提示词标题"]')!, '等发版');
    type(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="提示词内容"]')!, '等发版\n发完通知我');
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-form .btn.brand')!.click());

    expect(host.querySelector('.cr-prompt-insert')?.textContent).toContain('等发版');
    expect(listPrompts()).toEqual([{ id: 'p-1', title: '等发版', text: '等发版\n发完通知我' }]);

    act(() => host.querySelector<HTMLButtonElement>('button[aria-label="编辑提示词「等发版」"]')!.click());
    type(host.querySelector<HTMLInputElement>('input[aria-label="提示词标题"]')!, '等发版 ✅');
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-form .btn.brand')!.click());
    expect(listPrompts()[0].title).toBe('等发版 ✅');

    act(() => host.querySelector<HTMLButtonElement>('button[aria-label="删除提示词「等发版 ✅」"]')!.click());
    expect(host.querySelector('.cr-prompt-del')?.textContent).toContain('删除？');
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-del button.danger')!.click());
    expect(listPrompts()).toEqual([]);
    expect(host.querySelector('.cr-prompt-empty')).not.toBeNull();
  });

  it('Esc 只关提示词浮层，不关新建卡片弹窗', () => {
    savePrompts([{ id: 'p-1', title: '等发版', text: '等发版' }]);
    act(() => root.render(<Creator />));
    act(() => promptButton().click());
    expect(host.querySelector('.cr-prompt-pop')).not.toBeNull();

    act(() => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
    expect(host.querySelector('.cr-prompt-pop')).toBeNull();
    expect(host.querySelector('.creator')!.classList.contains('on')).toBe(true);
  });
});
