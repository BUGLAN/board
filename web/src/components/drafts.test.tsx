/* 人输入的内容不能丢（#682）：卡片详情、未整理展开行换卡时没保存的描述和评论；标签改名失败；切回「未整理」Tab 不重置 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { card, snapshot, workspace } from '../../test/fixtures';
import type { Card } from '../lib/types';
import { refresh, useData } from '../store/data';
import { useUI } from '../store/ui';
import { CardDetail } from './CardDetail';
import { InboxPage, Row, useInbox } from './Inbox';
import { LabelManager } from './overlays/Labels';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

/** 服务端：写请求按 fail 决定成败（hold 放行前不返回）；读工作区、白板时返回当前这份 */
let fail = false;
let hold: Promise<void> | null = null;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  fail = false;
  hold = null;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string, init?: RequestInit) => {
      const write = init?.method && init.method !== 'GET';
      if (write && hold) await hold;
      if (write && fail) return new Response(JSON.stringify({ error: { code: 'USAGE', message: '被拒绝了' } }), { status: 400 });
      const path = String(url).split('?')[0];
      const s = useData.getState();
      const body = path === '/api/workspace' ? workspace() : path === '/api/board' ? s.board : null;
      return new Response(JSON.stringify(body), { status: 200 });
    }),
  );
  // 发评论时纸飞机的动画：jsdom 没有 animate
  Element.prototype.animate ??= vi.fn(() => ({ finished: Promise.resolve(), cancel() {} })) as unknown as Element['animate'];
  useUI.setState({ toasts: [] });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

const settle = () => act(async () => {
  for (let i = 0; i < 5; i++) await new Promise(r => setTimeout(r, 0));
});

const type = (el: HTMLTextAreaElement | HTMLInputElement, value: string) => {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  act(() => {
    Object.getOwnPropertyDescriptor(proto, 'value')!.set!.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
};

describe('卡片详情换卡', () => {
  const desc = () => host.querySelector<HTMLTextAreaElement>('.desc-edit textarea');
  const comment = () => host.querySelector<HTMLTextAreaElement>('textarea[aria-label="评论"]')!;
  const open = (id: number) => act(() => useData.setState({ detailId: id }));

  beforeEach(() => {
    useData.setState({
      ws: workspace(),
      path: 'p/a',
      board: snapshot('p/a', { todo: [1, 2, 3] }),
      detailId: 1,
      detail: null,
      detailError: null,
    } as never);
  });

  it('A 卡输入描述和评论，切到 B 再切回 A，内容都在', () => {
    act(() => root.render(<CardDetail />));
    act(() => host.querySelector<HTMLElement>('.desc-view.empty')!.click());
    type(desc()!, '还没保存的描述');
    type(comment(), '还没发的评论');
    open(2);
    expect(desc()).toBeNull();
    expect(comment().value).toBe('');
    open(1);
    expect(desc()?.value).toBe('还没保存的描述');
    expect(comment().value).toBe('还没发的评论');
    // 取消编辑后再换卡：不再带回来
    act(() => [...host.querySelectorAll<HTMLButtonElement>('.desc-edit button')].find(b => b.textContent === '取消')!.click());
    open(2);
    open(1);
    expect(desc()).toBeNull();
  });

  it('评论发送失败时已经换了卡：回来后评论框里是原文', async () => {
    fail = true;
    let release = () => {};
    hold = new Promise(r => (release = r));
    act(() => root.render(<CardDetail />));
    open(3);
    type(comment(), '要发的评论');
    act(() => host.querySelector<HTMLButtonElement>('.comment-send')!.click());
    expect(comment().value).toBe('');
    open(2);
    release();
    await settle();
    open(3);
    expect(comment().value).toBe('要发的评论');
  });
});

describe('未整理展开行', () => {
  const cards: Card[] = [card(40, { column: 'inbox', title: '甲' }), card(41, { column: 'inbox', title: '乙' })];
  function Rows() {
    const open = useInbox(s => s.open);
    return (
      <>
        {cards.map(c => (
          <Row key={c.id} card={c} now={0} labels={[]} all={[]} home="待办" checked={false} open={open === c.id} onCheck={() => {}} />
        ))}
      </>
    );
  }

  it('展开 A 编辑描述后展开 B，再回 A，内容在', () => {
    useData.setState({ ws: workspace(), path: 'p/b', board: snapshot('p/b', { inbox: [40, 41] }) } as never);
    useInbox.setState({ open: 40 });
    act(() => root.render(<Rows />));
    const row = (id: number) => host.querySelector<HTMLElement>(`[data-ib="${id}"]`) ?? [...host.querySelectorAll<HTMLElement>('.ib-row')][id - 40];
    act(() => row(40).querySelector<HTMLElement>('.desc-view.empty')!.click());
    type(row(40).querySelector('textarea.textarea')!, '甲的新描述');
    act(() => useInbox.setState({ open: 41 }));
    expect(row(40).querySelector('textarea.textarea')).toBeNull();
    act(() => useInbox.setState({ open: 40 }));
    expect(row(40).querySelector<HTMLTextAreaElement>('textarea.textarea')?.value).toBe('甲的新描述');
  });
});

describe('标签管理改名', () => {
  it('保存中输入框只读；改名失败后输入框里仍是刚输入的名字', async () => {
    fail = true;
    let release = () => {};
    hold = new Promise(r => (release = r));
    // 补丁叠在 store 取回的数据上：走一次刷新，不直接 setState
    useData.setState({ path: 'p/a', board: snapshot('p/a', { todo: [1] }) } as never);
    await act(async () => {
      await refresh();
    });
    useUI.setState({ labelsOf: 'p' });
    act(() => root.render(<LabelManager />));
    await settle();
    const input = () => document.querySelector<HTMLInputElement>('.lbm-name')!;
    act(() => input().focus());
    type(input(), 'bug-改');
    act(() => input().blur());
    expect(input().readOnly).toBe(true);
    release();
    await settle();
    expect(input().readOnly).toBe(false);
    expect(input().value).toBe('bug-改');
    act(() => useUI.setState({ labelsOf: null }));
  });
});

describe('切回「未整理」Tab', () => {
  it('同一块白板重新挂载时子页、合并预览的改动都留着；换了白板才回到「未整理」', async () => {
    useData.setState({ ws: workspace(), path: 'p/a', board: snapshot('p/a', { inbox: [40, 41] }) } as never);
    act(() => root.render(<InboxPage />));
    const merge = { ids: [40, 41], keep: 40, draft: { title: '改过的标题', body: '改过的描述' } };
    act(() => useInbox.setState({ page: 'later', src: 'agent', merge }));
    // 切到白板（这一页卸载）再切回来
    act(() => root.render(<></>));
    act(() => root.render(<InboxPage />));
    expect(useInbox.getState()).toMatchObject({ page: 'later', src: 'agent', merge });
    // 换白板：清空
    act(() => useData.setState({ path: 'p/b', board: snapshot('p/b', { inbox: [] }) } as never));
    expect(useInbox.getState()).toMatchObject({ page: 'inbox', src: null, merge: null });
  });
});
