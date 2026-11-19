import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { listPrompts, savePrompts } from '../lib/prompts';
import { exec, useData } from '../store/data';
import { useUI } from '../store/ui';
import { Creator } from './Creator';

vi.mock('../store/data', async importOriginal => {
  const actual = await importOriginal<typeof import('../store/data')>();
  return { ...actual, exec: vi.fn() };
});

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
  vi.mocked(exec).mockReset();
  if (!Element.prototype.animate) {
    Object.defineProperty(Element.prototype, 'animate', {
      configurable: true,
      value: () => ({ cancel() {}, finished: Promise.resolve() }),
    });
  }
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
  useUI.setState({ creator: { open: true, column: 'todo', seq: 1 }, picker: null, labelsOf: null, tab: 'board', toasts: [] });
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

describe('连续新建时保存失败草稿（default/main#513）', () => {
  it('当前表单为空时，失败内容仍写入草稿，并可从提示恢复', async () => {
    localStorage.setItem('board.creator.keep', 'true');
    let reject!: (err: unknown) => void;
    vi.mocked(exec).mockReturnValue(
      new Promise<never>((_, rej) => {
        reject = rej;
      }),
    );
    act(() => root.render(<Creator />));

    const title = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="标题"]')!;
    const body = host.querySelector<HTMLTextAreaElement>('.cr-desc textarea')!;
    type(title, '空表单 A');
    type(body, '失败内容');
    act(() => host.querySelector<HTMLButtonElement>('.cr-submit')!.click());

    await act(async () => {
      reject(new Error('服务端拒绝'));
      await Promise.resolve();
    });

    expect(JSON.parse(localStorage.getItem('board.creator.draft')!)).toMatchObject({ title: '空表单 A', body: '失败内容' });
    const toast = useUI.getState().toasts.find(t => t.type === 'error')!;
    act(() => toast.action!.fn());
    await act(async () => {
      await new Promise<void>(resolve => setTimeout(resolve, 80));
    });
    expect(title.value).toBe('空表单 A');
    expect(body.value).toBe('失败内容');
  });

  it('A 还在保存时输入 B，A 失败后 B 不变；恢复 A 后仍能切回 B', async () => {
    localStorage.setItem('board.creator.keep', 'true');
    let reject!: (err: unknown) => void;
    const request = new Promise<never>((_, rej) => {
      reject = rej;
    });
    vi.mocked(exec).mockReturnValue(request);
    act(() => root.render(<Creator />));

    const title = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="标题"]')!;
    const body = host.querySelector<HTMLTextAreaElement>('.cr-desc textarea')!;
    act(() => host.querySelector<HTMLElement>('[data-col="inbox"]')!.click());
    type(title, 'A 标题');
    type(body, 'A 描述');
    act(() => host.querySelector<HTMLButtonElement>('.cr-submit')!.click());

    expect(title.value).toBe('');
    act(() => host.querySelector<HTMLElement>('[data-col="todo"]')!.click());
    type(title, 'B 标题');
    type(body, 'B 描述');

    await act(async () => {
      reject(Object.assign(new Error('断网了'), { code: 'NETWORK' }));
      await Promise.resolve();
    });

    expect(title.value).toBe('B 标题');
    expect(body.value).toBe('B 描述');
    expect(host.querySelector('.dest.on')!.getAttribute('data-col')).toBe('todo');
    expect(localStorage.getItem('board.creator.draft')).toBeNull();
    const toast = useUI.getState().toasts.find(t => t.type === 'error')!;
    expect(toast.text).toContain('当前输入没有改动');
    expect(toast.action?.label).toBe('恢复失败草稿');

    act(() => toast.action!.fn());
    expect(title.value).toBe('A 标题');
    expect(body.value).toBe('A 描述');
    expect(host.querySelector('.dest.on')!.getAttribute('data-col')).toBe('inbox');
    expect(JSON.parse(localStorage.getItem('board.creator.draft')!)).toMatchObject({ title: 'B 标题', body: 'B 描述' });
    expect(host.querySelector('.cr-failed')!.textContent).toContain('B 标题');

    act(() => host.querySelector<HTMLButtonElement>('button[aria-label="恢复另存的草稿"]')!.click());
    expect(title.value).toBe('B 标题');
    expect(body.value).toBe('B 描述');
    expect(host.querySelector('.dest.on')!.getAttribute('data-col')).toBe('todo');
  });
});
