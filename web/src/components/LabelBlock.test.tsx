/* 侧栏新建标签（default/main#524）：请求期间输入框保持可编辑，保存完成不覆盖后输入的名称；
   失败时失败的名称回到输入框并提示；保存中回车不会重复新建 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { snapshot, workspace } from '../../test/fixtures';
import type { Label, Workspace } from '../lib/types';
import { useData } from '../store/data';
import { useUI } from '../store/ui';
import { LabelBlock } from './Workbench';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let labels: Label[];
let posts: { name: string; resolve: (res: Response) => void }[];

class NoResize {
  observe() {}
  unobserve() {}
  disconnect() {}
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

function wsWith(labels: Label[]): Workspace {
  const ws = workspace();
  ws.projects[0].labels = labels;
  return ws;
}

function Harness() {
  const ws = useData(s => s.ws);
  const all = ws?.projects.find(p => p.key === 'p')?.labels ?? [];
  return <LabelBlock project="p" all={all} rules={[]} used={{}} />;
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', NoResize);
  Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve() })) as unknown as Element['animate'];
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  labels = [];
  posts = [];
  useUI.setState({ labelSel: [], toasts: [] });
  useData.setState({ ws: wsWith(labels), board: null, path: 'p/a', saving: {} });
  vi.stubGlobal(
    'fetch',
    vi.fn((url: string, init?: RequestInit) => {
      const path = String(url);
      if (path === '/api/workspace') return Promise.resolve(json(wsWith(labels)));
      if (path.startsWith('/api/board')) return Promise.resolve(json(snapshot('p/a', {})));
      if (path === '/api/labels' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body)) as { name: string };
        return new Promise<Response>(resolve => posts.push({ name: body.name, resolve }));
      }
      return Promise.resolve(json(null));
    }),
  );
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  delete (Element.prototype as Partial<Element>).animate;
});

const input = () => host.querySelector<HTMLInputElement>('.lb-input')!;
const pending = () => host.querySelector('.lb-pick.pending');
const type = (el: HTMLInputElement, v: string) =>
  act(() => {
    el.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
const key = (el: Element, k: string) => act(() => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true })));
const flush = () =>
  act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });

describe('侧栏新建标签时输入框保持可编辑', () => {
  it('回车新建后仍聚焦，请求期间可以接着输入下一个名称，保存完成不清掉', async () => {
    act(() => root.render(<Harness />));
    type(input(), '重构');
    expect(input().value).toBe('重构');
    key(input(), 'Enter');
    await flush();

    expect(posts.map(p => p.name)).toEqual(['重构']);
    expect(input().value).toBe('');
    expect(input().readOnly).toBe(false);
    expect(document.activeElement).toBe(input());
    expect(pending()?.textContent).toContain('重构');

    type(input(), '文档');
    expect(input().value).toBe('文档');

    labels.push({ id: 10, name: '重构', color: '#0f0', uses: 0 });
    await act(async () => posts.shift()!.resolve(json({ id: 10, name: '重构', color: '#0f0', uses: 0 })));
    await flush();

    expect(input().value).toBe('文档');
    expect(document.activeElement).toBe(input());
    expect(pending()).toBe(null);
    expect(host.textContent).toContain('重构');
  });

  it('保存中按 Enter 不会重复新建；前一保存完成后可以提交后输入的名称', async () => {
    act(() => root.render(<Harness />));
    type(input(), '重构');
    key(input(), 'Enter');
    await flush();
    type(input(), '文档');
    key(input(), 'Enter');
    expect(posts.map(p => p.name)).toEqual(['重构']);

    labels.push({ id: 10, name: '重构', color: '#0f0', uses: 0 });
    await act(async () => posts.shift()!.resolve(json({ id: 10, name: '重构', color: '#0f0', uses: 0 })));
    await flush();

    expect(input().value).toBe('文档');
    key(input(), 'Enter');
    await flush();
    expect(posts.map(p => p.name)).toEqual(['文档']);

    labels.push({ id: 11, name: '文档', color: '#00f', uses: 0 });
    await act(async () => posts.shift()!.resolve(json({ id: 11, name: '文档', color: '#00f', uses: 0 })));
    await flush();
  });

  it('创建失败时失败的名称回到输入框，焦点保留并提示', async () => {
    act(() => root.render(<Harness />));
    type(input(), '坏');
    key(input(), 'Enter');
    await flush();
    await act(async () =>
      posts.shift()!.resolve(json({ error: { code: 'USAGE', message: '标签「坏」已存在' } }, 400)),
    );
    await flush();

    expect(input().value).toBe('坏');
    expect(document.activeElement).toBe(input());
    expect(useUI.getState().toasts.at(-1)).toMatchObject({ type: 'error', text: '标签「坏」已存在' });
  });
});
