import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionView } from '../../lib/types';
import { useData } from '../../store/data';
import { Process, SessionBody } from './Session';

type Part = SessionView['parts'][number];

const calls = vi.hoisted(() => ({ view: null as unknown }));
vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return { ...real, api: { ...real.api, session: async () => calls.view } };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const part = (x: Partial<Part> = {}): Part =>
  ({
    id: 1,
    board: 'default/main',
    card: 635,
    title: '会话',
    hostId: 1,
    host: 'mbp',
    agent: 'codex',
    agentLabel: 'Codex',
    name: null,
    source: 'manual',
    by: { kind: 'human', name: '人' },
    state: 'done',
    result: 'review',
    createdAt: 1,
    startedAt: 1,
    endedAt: 2,
    stopBy: null,
    beatAt: null,
    log: [],
    fmt: 'codex',
    events: [],
    eventsFrom: 0,
    eventsDropped: 0,
    ...x,
  }) as Part;

let el: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  el = document.createElement('div');
  document.body.appendChild(el);
  root = createRoot(el);
  calls.view = null;
});
afterEach(() => {
  act(() => root.unmount());
  el.remove();
});

describe('会话弹窗', () => {
  it('顶部不再有「过程 / 原始输出」切换，打开时直接画过程', async () => {
    calls.view = { session: 1, parts: [part({ events: [{ id: 1, kind: 'message', text: '我先看看' }] })], runs: 1 };
    useData.setState({ ws: { rev: 1, hosts: [], dispatches: [], chats: [], projects: [] } as never });
    await act(async () => root.render(<SessionBody id={1} on close={() => {}} />));
    await act(async () => {});

    expect(el.querySelector('.ss-modes')).toBeNull();
    expect(el.querySelector('.ss-meta')?.textContent).not.toContain('原始输出');
    expect(el.querySelector('.ev-list')?.textContent).toContain('我先看看');
  });
});

describe('过程兜底', () => {
  it('有结构化事件：画过程，不出现切换或原始输出链接', () => {
    act(() => root.render(<Process p={part({ events: [{ id: 1, kind: 'error', text: '出错了' }] })} />));
    expect(el.querySelector('.ev-err')?.textContent).toBe('出错了');
    expect(el.querySelector('button')).toBeNull();
    expect(el.textContent).not.toContain('原始输出');
  });

  it('没有事件流参数：说明之后直接在段内画原始日志', () => {
    act(() => root.render(<Process p={part({ fmt: '', log: ['第一行', '第二行'] })} />));
    expect(el.querySelector('.ss-why')?.textContent).toContain('没有结构化输出');
    expect(el.querySelector('pre.hp-log.ss-log')?.textContent).toBe('第一行\n第二行');
    expect(el.querySelector('button')).toBeNull();
  });

  it('旧版 board host 且没有输出：说明之后写「没有输出」', () => {
    act(() => root.render(<Process p={part({ fmt: null })} />));
    expect(el.querySelector('.ss-why')?.textContent).toContain('board host 版本较旧');
    expect(el.querySelector('pre.hp-log.ss-log')?.textContent).toBe('没有输出');
    expect(el.textContent).not.toContain('看原始输出');
  });

  it('排着：只写排着，不画原始日志', () => {
    act(() => root.render(<Process p={part({ state: 'queued', fmt: '', log: ['不该出现'] })} />));
    expect(el.textContent).toContain('排着：等这个会话做完前面的卡再开始');
    expect(el.querySelector('pre')).toBeNull();
  });

  it('在跑、还没有事件：写还没有事件，等 board host 上传', () => {
    act(() => root.render(<Process p={part({ state: 'running' })} />));
    expect(el.textContent).toContain('还没有事件（board host 每 2 秒上传一次）');
    expect(el.querySelector('button')).toBeNull();
  });
});
