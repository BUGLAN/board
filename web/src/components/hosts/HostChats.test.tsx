/* 机器详情「对话」页签（PRD §9.1「机器对话」）：列表、在右栏里聊、发消息（原样、乐观更新、失败退回）、这一轮在跑时不能发、
   停止这一轮、机器离线、新对话（跑不了的角色禁用写原因，开始要等服务端返回）；
   手机上列表 → 整屏聊天、右上角放大 / 还原（default/main#623） */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { normHost } from '../../lib/normalize';
import type { Chat, ChatView, HostInfo, Profile, Workspace } from '../../lib/types';
import { useData } from '../../store/data';
import { useUI } from '../../store/ui';
import { chatsOf, HostChats, type ChatSel } from './HostChats';

const calls = vi.hoisted(() => ({
  prompt: [] as [number, string][],
  cancel: [] as number[],
  start: [] as [string, { profile?: string; agent?: string }, string][],
  fail: null as null | { code: string; message: string },
  views: new Map<number, unknown>(),
}));
vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return {
    ...real,
    api: {
      ...real.api,
      chat: async (id: number) => calls.views.get(id),
      chatPrompt: async (id: number, text: string) => {
        calls.prompt.push([id, text]);
        if (calls.fail) throw calls.fail;
        return {};
      },
      chatCancel: async (id: number) => void calls.cancel.push(id),
      startChat: async (host: string, by: { profile?: string; agent?: string }, text: string) => {
        calls.start.push([host, by, text]);
        return { id: 9 };
      },
    },
  };
});
// 写队列和刷新不在这里测（patches.test.ts、queue.test.ts）：直接执行请求
vi.mock('../../store/data', async orig => {
  const real = await orig<typeof import('../../store/data')>();
  return { ...real, optimistic: (_patch: unknown, f: () => Promise<unknown>) => f() };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let el: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  el = document.createElement('div');
  document.body.appendChild(el);
  root = createRoot(el);
  Object.assign(calls, { prompt: [], cancel: [], start: [], fail: null });
  calls.views.clear();
  useUI.setState({ toasts: [] });
  localStorage.clear();
});
afterEach(() => {
  act(() => root.unmount());
  el.remove();
});

const host = (x: Partial<HostInfo> = {}): HostInfo =>
  normHost({
    id: 1,
    name: 'mbp',
    os: 'macOS 15.4',
    version: '0.1.50',
    agents: [{ id: 'claude', name: 'Claude Code', version: '2.4.1', found: true }],
    envs: [],
    online: true,
    fits: [
      { profile: 'code', ok: true },
      { profile: 'doc', ok: false, why: '没装 dsh' },
    ],
    ...x,
  } as unknown as HostInfo);
const chat = (id: number, x: Partial<Chat> = {}): Chat => ({ id, hostId: 1, host: 'mbp', agent: 'claude', agentLabel: 'Claude Code', profileName: '写代码', title: `第 ${id} 个`, by: 'lan', createdAt: 1, lastAt: id, turn: 'idle', ...x });
const view = (c: Chat, events: ChatView['events']): ChatView => ({ chat: c, log: [], events, eventsFrom: 1, eventsDropped: 0, runs: 1 });
const profiles: Profile[] = [
  { id: 'code', name: '写代码', agent: 'claude', provider: '', model: 'model-a', effort: 'high', note: '', only: [] },
  { id: 'doc', name: '写文档', agent: 'dsh', provider: '', model: '', effort: '', note: '', only: [] },
];

const render = async (h: HostInfo, chats: Chat[], sel: ChatSel = null, setSel: (s: ChatSel) => void = () => {}) => {
  useData.setState({ ws: { hosts: [h], dispatches: [], chats, sched: { rules: [], minute: 0, profiles } } as unknown as Workspace });
  await act(async () => root.render(<HostChats h={h} chats={chatsOf(useData.getState().ws, h.id)} sel={sel} setSel={setSel} now={Date.now()} />));
  await act(async () => {});
};
const button = (text: string) => [...el.querySelectorAll('button')].find(b => b.textContent?.trim() === text)!;
const box = () => el.querySelector<HTMLTextAreaElement>('.ss-input')!;
const type = async (text: string) => {
  const t = box();
  await act(async () => {
    const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    set.call(t, text);
    t.dispatchEvent(new Event('input', { bubbles: true }));
  });
};
const enter = async () => {
  await act(async () => {
    box().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  });
};

describe('对话页签', () => {
  it('列表按最近动静排，回答中的写「回答中」；默认打开最近的那个，画出人发的消息和 agent 的回答', async () => {
    const a = chat(1, { lastAt: 100 });
    const b = chat(2, { lastAt: 200, turn: 'busy', title: '看看主目录下哪些文件夹最大' });
    calls.views.set(2, view(b, [{ id: 1, kind: 'prompt', text: '看看主目录下哪些文件夹最大', at: 1 }, { id: 2, kind: 'message', text: '我先看看' }]));
    await render(host(), [a, b]);
    const rows = [...el.querySelectorAll('.hc-item b')].map(x => x.textContent);
    expect(rows).toEqual(['看看主目录下哪些文件夹最大', '第 1 个']);
    expect(el.querySelector('.hc-item.on small')?.textContent).toContain('回答中');
    expect(el.querySelector('.ev-bubble')?.textContent).toBe('看看主目录下哪些文件夹最大');
    expect(el.querySelector('.ev-msg')?.textContent).toContain('我先看看');
  });

  it('这一轮在跑：发送禁用，旁边「停止这一轮」，点了调接口', async () => {
    const c = chat(1, { turn: 'busy' });
    calls.views.set(1, view(c, []));
    await render(host(), [c]);
    await type('再看看');
    expect(button('发送').disabled).toBe(true);
    await act(async () => button('停止这一轮').click());
    expect(calls.cancel).toEqual([1]);
  });

  it('空闲：Enter 把消息原样发出去，当帧画上「发送中」；失败时退回输入框并提示', async () => {
    const c = chat(1);
    calls.views.set(1, view(c, []));
    await render(host(), [c]);
    await type('  清一下 ~/.cache  ');
    await enter();
    expect(calls.prompt).toEqual([[1, '清一下 ~/.cache']]);
    expect(el.querySelector('.ev-prompt.sending .ev-bubble')?.textContent).toBe('清一下 ~/.cache');

    calls.fail = { code: 'USAGE', message: '机器「mbp」离线：上线后才能对话' };
    await render(host(), [chat(2)], 2);
    calls.views.set(2, view(chat(2), []));
    await type('再清一下');
    await enter();
    expect(box().value).toBe('再清一下');
    expect(useUI.getState().toasts.at(-1)?.text).toBe('机器「mbp」离线：上线后才能对话');
    expect(el.querySelector('.ev-prompt.sending')).toBeNull();
  });

  it('机器离线：「新对话」禁用，输入框写离线、不能发', async () => {
    const c = chat(1);
    calls.views.set(1, view(c, []));
    await render(host({ online: false }), [c]);
    expect(button('新对话').disabled).toBe(true);
    expect(box().disabled).toBe(true);
    expect(box().placeholder).toBe('mbp 离线，上线后才能接着发');
  });
});

describe('新对话', () => {
  it('没有对话时右栏就是新对话：跑不了的角色禁用写原因；开始带上角色和原样的第一条消息，返回后换成这个对话', async () => {
    const sel: ChatSel[] = [];
    await render(host(), [], null, s => sel.push(s));
    const tiles = [...el.querySelectorAll<HTMLButtonElement>('.nc-tile')];
    expect(tiles.map(t => [t.querySelector('b')?.textContent, t.disabled])).toEqual([
      ['写代码', false],
      ['写文档', true],
    ]);
    expect(tiles[1].textContent).toContain('没装 dsh');
    const t = el.querySelector<HTMLTextAreaElement>('.nc-text')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(t, '看看主目录下哪些文件夹最大');
      t.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => button('开始').click());
    expect(calls.start).toEqual([['mbp', { profile: 'code' }, '看看主目录下哪些文件夹最大']]);
    expect(sel).toEqual([9]);
  });
});

describe('手机上：列表 → 整屏聊天，放大 / 还原', () => {
  const mm = window.matchMedia;
  // 窄屏：对话这块的媒体查询匹配
  const narrow = () => {
    window.matchMedia = ((q: string) => ({ ...mm(q), matches: q.includes('max-width: 760px') })) as typeof window.matchMedia;
  };
  afterEach(() => {
    window.matchMedia = mm;
  });
  const zoom = () => document.querySelector<HTMLButtonElement>('.hc-zoom')!;

  it('窄屏进页签是列表，不自动打开最近的；点一行进入这个对话', async () => {
    narrow();
    const sel: ChatSel[] = [];
    await render(host(), [chat(1), chat(2)], null, s => sel.push(s));
    expect(el.querySelector('.hc')?.getAttribute('data-view')).toBe('list');
    expect(document.querySelector('.ss-input')).toBeNull();
    await act(async () => el.querySelectorAll<HTMLButtonElement>('.hc-item')[1].click());
    expect(sel).toEqual([1]);
  });

  it('窄屏打开对话默认整屏（挂在 body 上）；← 回列表；还原后回到页签里，选择记住', async () => {
    narrow();
    const c = chat(1);
    calls.views.set(1, view(c, []));
    const sel: ChatSel[] = [];
    await render(host(), [c], 1, s => sel.push(s));
    const full = document.querySelector('.hc-full.on-screen')!;
    expect(full.parentElement).toBe(document.body);
    expect(full.getAttribute('role')).toBe('dialog');
    expect(full.querySelector('.ss-input')).not.toBeNull();
    expect(zoom().getAttribute('aria-label')).toBe('还原');
    expect(zoom().getAttribute('aria-pressed')).toBe('true');
    await act(async () => document.querySelector<HTMLButtonElement>('.hc-back')!.click());
    expect(sel).toEqual([null]);

    await act(async () => zoom().click());
    expect(document.querySelector('.hc-full')).toBeNull();
    expect(el.querySelector('.hc-main .ss-input')).not.toBeNull();
    expect(zoom().getAttribute('aria-label')).toBe('放大');
    expect(zoom().getAttribute('aria-pressed')).toBe('false');
    // 下次打开还是还原
    act(() => root.unmount());
    root = createRoot(el);
    await render(host(), [c], 1);
    expect(document.querySelector('.hc-full')).toBeNull();
  });

  it('宽屏默认左右两栏；放大盖住机器详情，Esc 先还原', async () => {
    const c = chat(1);
    calls.views.set(1, view(c, []));
    await render(host(), [c]);
    expect(document.querySelector('.hc-full')).toBeNull();
    expect(el.querySelector('.hc-back')).toBeNull();
    expect(zoom().getAttribute('aria-label')).toBe('放大');
    await act(async () => zoom().click());
    expect(document.querySelector('.hc-full.in-drawer')).not.toBeNull();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(document.querySelector('.hc-full')).toBeNull();
    expect(localStorage.getItem('board.chatFull.d')).toBe('0');
  });
});
