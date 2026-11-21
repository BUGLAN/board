import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../lib/api';
import type { Prompt } from '../lib/prompts';
import { exec, useData } from '../store/data';
import { useUI } from '../store/ui';
import { Creator, creatorCli } from './Creator';

vi.mock('../store/data', async importOriginal => {
  const actual = await importOriginal<typeof import('../store/data')>();
  return { ...actual, exec: vi.fn() };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = new Date(2026, 10, 16, 10).getTime();
let host: HTMLDivElement;
/** 假的服务端：快捷提示词整列（#572） */
let server: Prompt[] = [];
let failSave: Error | null = null;
const puts: Prompt[][] = [];
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
  server = [];
  failSave = null;
  puts.length = 0;
  // 保存后的刷新轮不连真的服务端
  vi.spyOn(globalThis, 'fetch').mockRejectedValue(new TypeError('offline'));
  vi.spyOn(api, 'prompts').mockImplementation(async () => server.map(p => ({ ...p })));
  vi.spyOn(api, 'savePrompts').mockImplementation(async list => {
    puts.push(list);
    if (failSave) throw failSave;
    server = list.map(p => ({ ...p }));
    return server.map(p => ({ ...p }));
  });
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
      prompts: null,
    } as never,
  );
  useUI.setState({ creator: { open: true, column: 'todo', seq: 1 }, picker: null, labelsOf: null, tab: 'board', toasts: [] });
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
});

/** 等 promise 链（取列表、保存、保存后的刷新轮）走完 */
const settle = () => act(async () => {
  for (let i = 0; i < 10; i++) await new Promise<void>(resolve => setTimeout(resolve, 0));
});

const promptButton = () => [...host.querySelectorAll<HTMLButtonElement>('.cr-label .act button')].find(b => b.textContent?.includes('快捷提示词'))!;

describe('新建卡片里的快捷提示词（default/main#413）', () => {
  it('按钮在附件左边；点一条插到描述光标处、替换选中文字，浮层不关', async () => {
    server = [
      { id: 'p-2', title: '重启', text: '重启一下' },
      { id: 'p-1', title: '等发版', text: '等发版' },
    ];
    act(() => root.render(<Creator />));

    const btn = promptButton();
    expect(btn.getAttribute('aria-haspopup')).toBe('dialog');
    expect(btn.getAttribute('aria-expanded')).toBe('false');
    expect(btn.getAttribute('aria-controls')).toBe('cr-prompt-pop');
    expect(host.querySelector('.cr-prompts')!.nextElementSibling?.textContent).toContain('附件');

    act(() => btn.click());
    expect(btn.getAttribute('aria-expanded')).toBe('true');
    expect(host.querySelector('.cr-prompt-loading')?.textContent).toContain('正在取');
    await settle();
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

  it('空列表给新建入口；新建、编辑、删除当帧生效，整列存到服务端', async () => {
    act(() => root.render(<Creator />));
    act(() => promptButton().click());
    await settle();
    expect(host.querySelector('.cr-prompt-empty')?.textContent).toContain('还没有快捷提示词');

    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-empty button')!.click());
    type(host.querySelector<HTMLInputElement>('input[aria-label="提示词标题"]')!, '等发版');
    type(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="提示词内容"]')!, '等发版\n发完通知我');
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-form .btn.brand')!.click());
    // 当帧：列表里已经有了，还在保存时不能再改
    expect(host.querySelector('.cr-prompt-insert')?.textContent).toContain('等发版');
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="编辑提示词「等发版」"]')!.disabled).toBe(true);
    await settle();
    expect(server).toEqual([{ id: 'p-1', title: '等发版', text: '等发版\n发完通知我' }]);
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="编辑提示词「等发版」"]')!.disabled).toBe(false);

    act(() => host.querySelector<HTMLButtonElement>('button[aria-label="编辑提示词「等发版」"]')!.click());
    type(host.querySelector<HTMLInputElement>('input[aria-label="提示词标题"]')!, '等发版 ✅');
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-form .btn.brand')!.click());
    expect(host.querySelector('.cr-prompt-insert')?.textContent).toContain('等发版 ✅');
    await settle();
    expect(server[0].title).toBe('等发版 ✅');

    act(() => host.querySelector<HTMLButtonElement>('button[aria-label="删除提示词「等发版 ✅」"]')!.click());
    expect(host.querySelector('.cr-prompt-del')?.textContent).toContain('删除？');
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-del button.danger')!.click());
    expect(host.querySelector('.cr-prompt-empty')).not.toBeNull();
    await settle();
    expect(server).toEqual([]);
    expect(localStorage.getItem('board.creator.prompts')).toBeNull();
  });

  it('另一端改过：每次打开浮层都重新从服务端取', async () => {
    act(() => root.render(<Creator />));
    act(() => promptButton().click());
    await settle();
    expect(host.querySelector('.cr-prompt-empty')).not.toBeNull();
    act(() => promptButton().click());
    server = [{ id: 'p-1', title: '别处加的', text: '别处加的' }];
    act(() => promptButton().click());
    await settle();
    expect(host.querySelector('.cr-prompt-insert')?.textContent).toContain('别处加的');
  });

  it('保存失败：撤回、提示原因，带着刚才的内容重新打开编辑框', async () => {
    server = [{ id: 'p-1', title: '等发版', text: '等发版' }];
    act(() => root.render(<Creator />));
    act(() => promptButton().click());
    await settle();
    failSave = new Error('服务端挂了');

    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-head .btn')!.click());
    type(host.querySelector<HTMLInputElement>('input[aria-label="提示词标题"]')!, '新的');
    type(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="提示词内容"]')!, '新的内容');
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-form .btn.brand')!.click());
    expect(host.querySelectorAll('.cr-prompt-insert')).toHaveLength(2);
    await settle();
    expect(useUI.getState().toasts.map(t => t.text).join()).toContain('服务端挂了');
    expect(host.querySelector<HTMLInputElement>('input[aria-label="提示词标题"]')!.value).toBe('新的');
    expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="提示词内容"]')!.value).toBe('新的内容');
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-form .btn:not(.brand)')!.click());
    expect(host.querySelectorAll('.cr-prompt-insert')).toHaveLength(1);

    // 删除失败：那一条回来
    act(() => host.querySelector<HTMLButtonElement>('button[aria-label="删除提示词「等发版」"]')!.click());
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-del button.danger')!.click());
    expect(host.querySelector('.cr-prompt-empty')).not.toBeNull();
    await settle();
    expect(host.querySelector('.cr-prompt-insert')?.textContent).toContain('等发版');
    expect(server).toEqual([{ id: 'p-1', title: '等发版', text: '等发版' }]);
  });

  it('本机旧存的：服务端为空时传上去一次并删掉本机的键', async () => {
    localStorage.setItem('board.creator.prompts', JSON.stringify([{ id: 'p-3', title: '本机的', text: '本机的内容' }]));
    act(() => root.render(<Creator />));
    act(() => promptButton().click());
    await settle();
    expect(puts).toEqual([[{ id: 'p-3', title: '本机的', text: '本机的内容' }]]);
    expect(server).toEqual([{ id: 'p-3', title: '本机的', text: '本机的内容' }]);
    expect(localStorage.getItem('board.creator.prompts')).toBeNull();
    expect(host.querySelector('.cr-prompt-insert')?.textContent).toContain('本机的');

    // 再打开不会再传
    act(() => promptButton().click());
    act(() => promptButton().click());
    await settle();
    expect(puts).toHaveLength(1);
  });

  it('本机旧存的：服务端已经有内容时不覆盖，本机的键也删掉', async () => {
    server = [{ id: 'p-1', title: '服务端的', text: '服务端的' }];
    localStorage.setItem('board.creator.prompts', JSON.stringify([{ id: 'p-1', title: '本机的', text: '本机的' }]));
    act(() => root.render(<Creator />));
    act(() => promptButton().click());
    await settle();
    expect(puts).toEqual([]);
    expect(server).toEqual([{ id: 'p-1', title: '服务端的', text: '服务端的' }]);
    expect(localStorage.getItem('board.creator.prompts')).toBeNull();
    expect(host.querySelector('.cr-prompt-insert')?.textContent).toContain('服务端的');
  });

  it('本机旧存的：上传失败时留着本机的，下次打开再试', async () => {
    failSave = new Error('连不上');
    localStorage.setItem('board.creator.prompts', JSON.stringify([{ id: 'p-1', title: '本机的', text: '本机的' }]));
    act(() => root.render(<Creator />));
    act(() => promptButton().click());
    await settle();
    expect(localStorage.getItem('board.creator.prompts')).not.toBeNull();
    failSave = null;
    act(() => promptButton().click());
    act(() => promptButton().click());
    await settle();
    expect(server).toEqual([{ id: 'p-1', title: '本机的', text: '本机的' }]);
    expect(localStorage.getItem('board.creator.prompts')).toBeNull();
  });

  it('取不到列表：写明原因，可以重试', async () => {
    vi.mocked(api.prompts).mockRejectedValueOnce(new Error('连不上 Board 服务'));
    server = [{ id: 'p-1', title: '等发版', text: '等发版' }];
    act(() => root.render(<Creator />));
    act(() => promptButton().click());
    await settle();
    expect(host.querySelector('.cr-prompt-loading')?.textContent).toContain('连不上 Board 服务');
    act(() => host.querySelector<HTMLButtonElement>('.cr-prompt-loading button')!.click());
    await settle();
    expect(host.querySelector('.cr-prompt-insert')?.textContent).toContain('等发版');
  });

  it('Esc 只关提示词浮层，不关新建卡片弹窗', () => {
    server = [{ id: 'p-1', title: '等发版', text: '等发版' }];
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

describe('「等价的 CLI」复制按钮（#522）', () => {
  it('一键复制整条命令，随标题、描述、目标实时更新；复制后图标变勾、弹提示', async () => {
    const writes: string[] = [];
    vi.stubGlobal('navigator', { ...navigator, userAgent: 'Mac', clipboard: { writeText: async (t: string) => void writes.push(t) } });
    act(() => root.render(<Creator />));
    const btn = () => host.querySelector<HTMLButtonElement>('.cr-cli .cr-cli-copy')!;
    expect(btn().getAttribute('aria-label')).toBe('复制等价的 CLI 命令');
    await act(async () => btn().click());
    expect(writes.at(-1)).toBe("board new '<标题>' --board default/main");
    type(host.querySelector<HTMLInputElement>('.cr-title, input[placeholder^="标题"], textarea[placeholder^="标题"]')!, "修 bug：别把 $HOME 展开");
    type(host.querySelector<HTMLTextAreaElement>('textarea[placeholder^="验收标准"]')!, '第一行\n第二行，描述超过四十个字也要完整复制过去，不能只复制界面上显示的那一段开头');
    act(() => host.querySelector<HTMLElement>('[data-col="inbox"]')!.click());
    await act(async () => btn().click());
    expect(writes.at(-1)).toBe(
      "board new '修 bug：别把 $HOME 展开' --body '第一行\n第二行，描述超过四十个字也要完整复制过去，不能只复制界面上显示的那一段开头' --inbox --board default/main",
    );
    expect(btn().classList.contains('done')).toBe(true);
    expect(useUI.getState().toasts.at(-1)?.title).toBe('已复制命令');
    vi.unstubAllGlobals();
  });

  it('引号：macOS / Linux 用单引号（$ 和反引号不展开），PowerShell 用两个单引号转义', () => {
    const o = { title: "it's `x`", body: '', inbox: false, labels: ['bug', "a'b"], path: 'p/b' };
    expect(creatorCli({ ...o, ps: false })).toBe("board new 'it'\\''s `x`' --label 'bug' --label 'a'\\''b' --board p/b");
    expect(creatorCli({ ...o, ps: true })).toBe("board new 'it''s `x`' --label 'bug' --label 'a''b' --board p/b");
  });
});
