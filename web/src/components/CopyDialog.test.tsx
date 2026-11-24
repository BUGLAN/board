import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { card, detail, snapshot, workspace } from '../../test/fixtures';
import type { Activity, Attachment, Cmd, Workspace } from '../lib/types';
import { type Patch, useData } from '../store/data';
import { useUI } from '../store/ui';
import { CopyDialog } from './CopyDialog';

const { execMock } = vi.hoisted(() => ({ execMock: vi.fn() }));

vi.mock('../store/data', async orig => {
  const real = await orig<typeof import('../store/data')>();
  return { ...real, exec: (cmd: Cmd, patch?: Patch) => execMock(cmd, patch) };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const note: Activity = { id: 3, at: 3, actor: { kind: 'human', name: '' }, kind: 'comment', action: 'comment', text: '换个做法' };
const lost: Activity = { id: 2, at: 2, actor: { kind: 'agent', name: 'mac/codex' }, kind: 'comment', action: 'comment', text: 'agent 异常退出（退出码 1）' };
const shot = { id: 7, cardId: 5, name: 'shot.png', mime: 'image/png', size: 1, image: true, url: '/api/files/7', markdown: '', createdAt: 0, createdBy: { kind: 'human', name: '' } } as Attachment;

function ws(): Workspace {
  const w = workspace();
  w.projects[0].labels = [...w.projects[0].labels, { id: 3, name: 'v0.4.2', color: '#888', uses: 1 }];
  return w;
}

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

function open(over: Partial<ReturnType<typeof card>> = {}) {
  const c = card(5, { column: 'review', runs: 2, runsLost: 1, labels: [1, 3], body: '要做的\n\n![图](/api/files/7)', ...over });
  useData.setState({
    ws: ws(),
    path: 'p/a',
    board: snapshot('p/a', { todo: [1], review: [5], done: [] }, [c]),
    detail: { ...detail(c), activity: [lost, note], attachments: [shot] },
    detailId: 5,
    saving: {},
    pending: {},
  } as never);
  useUI.setState({ copyOf: 5, toasts: [] });
  act(() => root.render(<CopyDialog />));
}
const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector(sel) as T;
const button = (text: string) => [...document.querySelectorAll('.cpy-dialog button')].find(b => b.textContent?.trim() === text) as HTMLButtonElement;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  execMock.mockReset();
  execMock.mockResolvedValue({ ...card(6), activity: [] });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
  useUI.setState({ copyOf: null });
  useData.setState({ ws: null, board: null, detail: null, detailId: null } as never);
});

describe('复制为新卡片面板', () => {
  it('列出原卡的经历；标签去掉版本标签；描述里的附件默认复制', () => {
    open();
    expect(q('.cpy-past').textContent).toContain('领取 2 次');
    expect(q('.cpy-past').textContent).toContain('报错日志 1 条');
    expect([...document.querySelectorAll('.cpy-labels .lb')].map(e => e.textContent?.replace('×', '').trim())).toEqual(['bug']);
    expect(q('.cpy-dialog').textContent).toContain('附件 1 个');
    expect(q('.cpy-dialog').textContent).toContain('人的评论 1 条');
  });

  it('一条命令复制：带人的评论、放进未整理、原卡移到终点；新卡和原卡的去向当帧画出来', () => {
    open();
    act(() => q<HTMLInputElement>('.cpy-row:nth-child(4) input').click());
    act(() => button('移到「done」').click());
    act(() => button('复制').click());
    expect(useUI.getState().copyOf).toBeNull();
    const [cmd, patch] = execMock.mock.calls[0] as [Extract<Cmd, { op: 'copy' }>, Patch];
    expect(cmd).toMatchObject({ op: 'copy', id: 5, title: '卡片 5', column: 'inbox', labels: [1], files: true, move_to: 'done' });
    expect(cmd.trash).toBeUndefined();
    expect(cmd.body).toBe('要做的\n\n![图](/api/files/7)\n\n## 原卡 #5 里人的意见\n\n> 换个做法');
    const b = patch.board!(useData.getState().board!);
    expect(b.cols.done).toEqual([5]);
    expect(b.cols.inbox).toHaveLength(1);
    expect(b.cards[b.cols.inbox[0]].title).toBe('卡片 5');
    expect(patch.cards).toContain(5);
  });

  it('移进回收站：原卡当帧从白板上拿掉；被领取的原卡只能留着', () => {
    open();
    act(() => button('移进回收站').click());
    act(() => button('复制').click());
    const [cmd, patch] = execMock.mock.calls[0] as [Extract<Cmd, { op: 'copy' }>, Patch];
    expect(cmd).toMatchObject({ trash: true, move_to: undefined });
    expect(patch.board!(useData.getState().board!).cards[5]).toBeUndefined();

    act(() => root.unmount());
    root = createRoot(host);
    open({ holder: 'mac/codex', column: 'doing' });
    expect(button('移到「done」').disabled).toBe(true);
    expect(button('移进回收站').disabled).toBe(true);
    expect(button('留着').disabled).toBe(false);
  });
});
