/* 项目设置（default/main#395）：三节；每一项失焦保存、Esc 只取消这一次输入；仓库没设时的警告和「用机器的 origin」；
   管理标签叠在上面时 Esc 先关它。白板设置：没有代码仓库一节，最上面「参与自动领取」，末尾一行去项目设置 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { snapshot, workspace } from '../../test/fixtures';
import type { HostInfo, Workspace } from '../lib/types';
import { useData } from '../store/data';
import { closeLabels, openBoardSettings, openProjectSettings, useUI } from '../store/ui';
import { BoardSettings } from './BoardSettings';
import { ProjectSettings } from './ProjectSettings';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let sent: { method: string; path: string; body: unknown }[];

// 白板设置的布局缩略图量宽度用 ResizeObserver：jsdom 没有，给一个什么都不做的
class NoResize {
  observe() {}
  unobserve() {}
  disconnect() {}
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', NoResize);
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  sent = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string, init?: RequestInit) => {
      const method = init?.method ?? 'GET';
      if (method !== 'GET') sent.push({ method, path: String(url).split('?')[0], body: init?.body ? JSON.parse(String(init.body)) : null });
      // 写操作之后的刷新拿到的是同一份数据；各节自己的读取给空的
      const path = String(url).split('?')[0];
      const body = path === '/api/workspace' ? workspace() : path === '/api/board' ? snapshot('p/a', { todo: [1] }) : path.includes('/api/card-crons') ? [] : null;
      return new Response(JSON.stringify(body), { status: 200 });
    }),
  );
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  act(() => {
    openProjectSettings(null);
    openBoardSettings(null);
    closeLabels();
  });
  useData.setState({ ws: null, board: null });
});

const dialog = () => document.querySelector<HTMLElement>('.ps-modal')!;
const labels = () => [...dialog().querySelectorAll('.d-label')].map(l => l.firstChild?.textContent?.trim());
const input = (name: string) => dialog().querySelector<HTMLInputElement>(`input[aria-label="${name}"]`)!;
const key = (el: Element, k: string) => act(() => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true })));
/** 在输入框里改值：React 的受控输入要走原生 setter 再发 input 事件 */
const type = (el: HTMLInputElement, v: string) =>
  act(() => {
    el.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
const blur = (el: HTMLInputElement) => act(() => el.blur());

async function open(ws: Workspace = workspace()) {
  act(() => useData.setState({ ws }));
  await act(async () => root.render(<ProjectSettings />));
  act(() => openProjectSettings('p'));
}

describe('项目设置', () => {
  it('四节：基本信息、代码仓库、环境检查、标签；头部写项目名和几块白板', async () => {
    await open();
    expect(dialog().classList.contains('on')).toBe(true);
    expect(labels()).toEqual(['基本信息', '代码仓库', '环境检查', '能接这个项目的机器', '标签']);
    expect(dialog().querySelector('.d-head')?.textContent).toContain('2 块白板：a、b');
    expect(input('标识').readOnly).toBe(true);
    // 标签一节：列出项目的标签
    expect([...dialog().querySelectorAll('.ps-label')].map(l => l.textContent)).toEqual(['bug', 'ui']);
  });

  it('改名称：失焦保存；清空时回到原值、不发请求', async () => {
    await open();
    type(input('名称'), '');
    blur(input('名称'));
    expect(input('名称').value).toBe('P');
    expect(sent).toEqual([]);
    type(input('名称'), '新名字');
    blur(input('名称'));
    await act(async () => {});
    expect(sent).toContainEqual({ method: 'PATCH', path: '/api/projects/p', body: { name: '新名字' } });
  });

  it('Esc 只取消这一次输入、不保存；再按一次才关弹窗', async () => {
    await open();
    const el = input('描述');
    type(el, '写了一半');
    key(el, 'Escape');
    expect(el.value).toBe('');
    expect(sent).toEqual([]);
    expect(useUI.getState().projectSettings).toBe('p');
    key(document.body, 'Escape');
    expect(useUI.getState().projectSettings).toBe(null);
  });

  it('仓库地址没设：一行警告；有机器报过 origin 时「用机器的 origin」直接存', async () => {
    const h = { name: 'mbp', dirs: [{ board: 'p/a', path: '~/ws/p', origin: 'git@github.com:o/p.git' }] } as unknown as HostInfo;
    await open(workspace({ hosts: [h] }));
    expect(dialog().querySelector('.ps-note.warn')?.textContent).toContain('还没设仓库地址');
    const btn = [...dialog().querySelectorAll('button')].find(b => b.textContent === '用机器的 origin')!;
    await act(async () => btn.click());
    expect(sent).toContainEqual({ method: 'PATCH', path: '/api/projects/p', body: { repo: 'git@github.com:o/p.git' } });
  });

  it('设了仓库地址：没有警告，也没有「用机器的 origin」', async () => {
    const ws = workspace();
    ws.projects[0].repo = 'git@github.com:o/p.git';
    await open(ws);
    expect(dialog().querySelector('.ps-note.warn')).toBeNull();
    expect(input('仓库地址').value).toBe('git@github.com:o/p.git');
  });

  it('环境检查是项目的：写「N 块白板共用」，回车保存到项目；Esc 只取消这一次输入', async () => {
    const ws = workspace();
    ws.projects[0].readyCheck = { cmd: 'cargo --version', win: '' };
    await open(ws);
    expect(dialog().querySelector('.bs-rp-scope')?.textContent).toBe('2 块白板共用');
    const cmd = input('环境检查：所有系统');
    expect(cmd.value).toBe('cargo --version');
    type(cmd, '写了一半');
    key(cmd, 'Escape');
    expect(cmd.value).toBe('cargo --version');
    expect(sent).toEqual([]);
    type(cmd, 'cargo --version && node --version');
    key(cmd, 'Enter');
    blur(cmd);
    await act(async () => {});
    expect(sent).toContainEqual({ method: 'PATCH', path: '/api/projects/p', body: { readyCheck: 'cargo --version && node --version' } });
  });

  it('没有机器能接这个项目：写提醒和项目的目录', async () => {
    await open();
    expect(dialog().querySelector('.bs-rc-empty')?.textContent).toContain('<工作空间>/p');
  });

  it('「管理标签」叠在上面：Esc 不关项目设置', async () => {
    await open();
    const btn = [...dialog().querySelectorAll('button')].find(b => b.textContent?.includes('管理标签'))!;
    act(() => btn.click());
    expect(useUI.getState().labelsOf).toBe('p');
    key(document.body, 'Escape');
    expect(useUI.getState().projectSettings).toBe('p');
  });
});

describe('白板设置', () => {
  async function openBoard(auto?: boolean) {
    const ws = workspace();
    if (auto !== undefined) ws.projects[0].boards[0].auto = auto;
    act(() => useData.setState({ ws, path: 'p/a', board: snapshot('p/a', { todo: [1] }) }));
    await act(async () => root.render(<BoardSettings path="p/a" />));
    act(() => openBoardSettings('p/a'));
    await act(async () => {});
    return document.querySelector<HTMLElement>('.bs-modal:not(.ps-modal)')!;
  }

  it('没有「代码仓库」「环境检查」两节；最上面是「自动领取」', async () => {
    const m = await openBoard();
    const secs = [...m.querySelectorAll('.d-label')].map(l => l.firstChild?.textContent?.trim());
    expect(secs[0]).toBe('自动领取');
    expect(secs).not.toContain('代码仓库');
    expect(secs).not.toContain('环境检查');
  });

  it('「参与自动领取」开关改的是白板的 auto；关着时写手动派发照常', async () => {
    const m = await openBoard(false);
    const sw = m.querySelector<HTMLButtonElement>('button[aria-label="参与自动领取"]')!;
    expect(sw.getAttribute('aria-checked')).toBe('false');
    expect(m.querySelector('.bs-keep')?.textContent).toContain('手动派发、board claim 照常');
    await act(async () => sw.click());
    expect(sent).toContainEqual({ method: 'PATCH', path: '/api/boards/p/a', body: { auto: true } });
  });

  it('末尾一行「打开项目设置」：关掉白板设置，打开这个项目的设置', async () => {
    const m = await openBoard();
    expect(m.querySelector('.bs-proj')?.textContent).toContain('代码仓库、环境检查、标签是项目「P」的，2 块白板共用');
    const btn = [...m.querySelectorAll('.bs-proj button')][0] as HTMLButtonElement;
    act(() => btn.click());
    expect(useUI.getState().boardSettings).toBe(null);
    expect(useUI.getState().projectSettings).toBe('p');
  });
});
