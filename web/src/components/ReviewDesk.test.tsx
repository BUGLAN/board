import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { card, detail, snapshot, workspace } from '../../test/fixtures';
import { api } from '../lib/api';
import type { Attachment, Cmd } from '../lib/types';
import { type Patch, useData } from '../store/data';
import { useUI } from '../store/ui';
import { ReviewDesk } from './ReviewDesk';

const { execMock } = vi.hoisted(() => ({ execMock: vi.fn() }));

vi.mock('../store/data', async importOriginal => {
  const actual = await importOriginal<typeof import('../store/data')>();
  return { ...actual, exec: (cmd: Cmd, patch?: Patch) => execMock(cmd, patch) };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

const type = (el: HTMLTextAreaElement, value: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
};

const shot = () => new File([new Uint8Array([1, 2, 3])], 'shot.png', { type: 'image/png' });

const paste = (el: HTMLTextAreaElement, file: File) => {
  const e = new Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(e, 'clipboardData', { value: { files: [file] } });
  act(() => el.dispatchEvent(e));
};

const key = (k: string) => act(() => window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true })));
const box = () => document.querySelector<HTMLTextAreaElement>('.rv-foot textarea')!;

const deferredUpload = () => {
  let done!: (a: Attachment) => void;
  const promise = new Promise<Attachment>(resolve => {
    done = resolve;
  });
  vi.mocked(api.upload).mockReturnValue(promise);
  return (markdown = '![shot.png](/api/files/77)') =>
    act(async () => {
      done({ id: 77, name: 'shot.png', markdown } as Attachment);
      await promise;
      await new Promise<void>(resolve => setTimeout(resolve, 0));
    });
};

beforeEach(async () => {
  execMock.mockReset();
  vi.spyOn(api, 'card').mockImplementation(async (_path, id) => detail(card(id, { column: 'review' })));
  vi.spyOn(api, 'upload');
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  const cards = [card(1, { column: 'review', title: '第一张' }), card(2, { column: 'review', title: '第二张' })];
  useData.setState({
    ws: workspace(),
    path: 'p/a',
    board: snapshot('p/a', { review: [1, 2] }, cards),
    saving: {},
    pending: {},
  } as never);
  useUI.setState({ review: { cols: ['review'], mode: 'review' }, toasts: [] });
  act(() => root.render(<ReviewDesk />));
  // usePresence 连续两帧后才把 on 打开，快捷键在 on 之后才监听
  await act(async () => {
    for (let i = 0; i < 4; i++) await new Promise<void>(resolve => setTimeout(resolve, 20));
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
  useUI.setState({ review: null });
  useData.setState({ ws: null, board: null } as never);
});

describe('复盘台意见框的附件', () => {
  it('粘贴截图：先在光标处显示占位，上传完换成图片 Markdown', async () => {
    const finish = deferredUpload();
    const ta = box();
    type(ta, '位置：');
    ta.setSelectionRange(3, 3);

    paste(ta, shot());
    expect(box().value).toContain('位置：\n![上传中：shot.png…]');
    expect(api.upload).toHaveBeenCalledWith('p/a', 1, expect.any(File));

    await finish();
    expect(box().value).toBe('位置：\n![shot.png](/api/files/77)');
    expect(box().value).not.toContain('上传中');
    expect(document.querySelector('.rv-attach')).not.toBeNull();
  });

  it('还在上传时按 A 不通过：提示附件还在上传，不发请求', async () => {
    const finish = deferredUpload();
    paste(box(), shot());

    key('a');
    expect(execMock).not.toHaveBeenCalled();
    expect(useUI.getState().toasts.at(-1)).toMatchObject({ type: 'info', title: '附件还在上传' });

    await finish();
    expect(box().value).toBe('![shot.png](/api/files/77)');
  });

  it('处理模式上传中按 ⌘↵ 也不回复', async () => {
    const finish = deferredUpload();
    act(() => {
      useData.setState({ board: snapshot('p/a', { check: [3] }, [card(3, { column: 'check' })]) } as never);
      useUI.setState({ review: { cols: ['check'], mode: 'human' } });
    });
    paste(box(), shot());

    act(() => box().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', metaKey: true, bubbles: true })));
    expect(execMock).not.toHaveBeenCalled();
    expect(useUI.getState().toasts.at(-1)).toMatchObject({ type: 'info', title: '附件还在上传' });

    await finish();
  });

  it('上传途中换到别的卡：结果写回原来那张卡的草稿，当前草稿不变', async () => {
    const finish = deferredUpload();
    paste(box(), shot());

    key('j');
    expect(document.querySelector('.rv-card-title')?.textContent).toBe('第二张');
    type(box(), '第二张的意见');

    await finish();
    expect(box().value).toBe('第二张的意见');

    key('k');
    expect(document.querySelector('.rv-card-title')?.textContent).toBe('第一张');
    expect(box().value).toBe('![shot.png](/api/files/77)');

    key('j');
    expect(box().value).toBe('第二张的意见');
  });
});
