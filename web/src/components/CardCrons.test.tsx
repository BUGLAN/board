/* 定时建卡拿出白板设置（default/main#402）：白板页自己的弹窗（Esc 先关标签选择器）、Agent tab 里的只读总览 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { snapshot, workspace } from '../../test/fixtures';
import type { CardCron, HostInfo, Status, Workspace } from '../lib/types';
import { useData } from '../store/data';
import { closePicker, openCardCrons, openPicker, useUI } from '../store/ui';
import { CardCronDialog, CronOverview } from './CardCrons';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  // 切白板时会取工作区、白板：给同一份
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string) => {
      const path = String(url).split('?')[0];
      const q = new URLSearchParams(String(url).split('?')[1] ?? '');
      const body = path === '/api/workspace' ? data() : path === '/api/board' ? snapshot(q.get('path') ?? 'p/a', { todo: [1] }) : null;
      return new Response(JSON.stringify(body), { status: 200 });
    }),
  );
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  act(() => {
    openCardCrons(null);
    closePicker();
  });
  useData.setState({ ws: null });
  useUI.setState({ tab: 'board' });
});

const status = (key: string, name: string, claim = false) => ({ key, name, color: 'todo', claim, lock: false, final: false }) as Status;
const cron = (id: number, x: Partial<CardCron> = {}): CardCron =>
  ({ id, name: `定时 ${id}`, title: `定时 ${id}`, body: '', labels: [], place: 'home', cron: '0 9 * * 1', tz: 'Asia/Shanghai', on: true, nextAt: null, createdAt: 0, updatedAt: 0, ...x }) as CardCron;

/** 项目 p 的两块白板 a、b 各有定时建卡；a 的首个可领取状态叫「排队中」（排在「需求池」之前的那个不可领取） */
function data(): Workspace {
  const ws = workspace({ hosts: [{ name: 'mbp', dirs: [], agents: [{ id: 'claude', name: 'Claude Code', found: true }] } as unknown as HostInfo] });
  const [a, b] = ws.projects[0].boards;
  a.statuses = [status('pool', '需求池'), status('queue', '排队中', true), status('done', '已完成')];
  b.statuses = [status('todo', '待办', true)];
  a.cardCrons = [
    cron(1, { name: '清理死代码', pick: 'random', ranAt: Date.UTC(2026, 10, 16, 1), ranCard: 13, ranErr: '没有在线的机器' }),
    cron(2, { name: '依赖升级检查', host: 'mbp', agent: 'claude', place: 'inbox' }),
  ];
  b.cardCrons = [cron(3, { name: '找文档缺口', on: false, host: 'gone' })];
  return ws;
}

const rows = () => [...host.querySelectorAll('.co-row')];

describe('Agent tab 的定时建卡总览', () => {
  it('按白板分组，汇总「N 条 · 开着 M 条 · K 块白板」', async () => {
    act(() => useData.setState({ ws: data() }));
    await act(async () => root.render(<CronOverview />));
    expect([...host.querySelectorAll('.co-head code')].map(c => c.textContent)).toEqual(['p/a', 'p/b']);
    expect(host.querySelector('.co-sum')?.textContent).toBe('3 条 · 开着 2 条 · 2 块白板');
    expect([...host.querySelectorAll('.co-n')].map(n => n.textContent)).toEqual(['2 条', '1 条']);
  });

  it('每条：放到哪写具体的状态名或「未整理」，派给谁、下次、上次结果', async () => {
    act(() => useData.setState({ ws: data() }));
    await act(async () => root.render(<CronOverview />));
    const [r1, r2, r3] = rows();
    expect(r1.querySelector('.co-place')?.textContent).toBe('→ 排队中');
    expect(r1.querySelector('.co-to')?.textContent).toBe('随机一台');
    expect(r1.querySelector('.co-last')?.textContent).toContain('建了 #13 · 没派发出去');
    expect(r1.querySelector('.co-last')?.classList.contains('skip')).toBe(true);
    expect(r2.querySelector('.co-place')?.textContent).toBe('→ 未整理');
    expect(r2.querySelector('.co-to')?.textContent).toBe('mbp · Claude Code');
    expect(r2.querySelector('.co-last')?.textContent).toBe('还没到过点');
    expect(r3.querySelector('.co-place')?.textContent).toBe('→ 待办');
    expect(r3.querySelector('.co-to')?.textContent).toBe('gone（已移除）');
    expect(r3.querySelector('.co-to')?.classList.contains('gone')).toBe(true);
    expect(r3.querySelector('.co-next')?.textContent).toBe('关着');
  });

  it('只读：没有开关、编辑、删除；点一条切到那块白板的白板页，打开它的定时建卡', async () => {
    act(() => useData.setState({ ws: data() }));
    act(() => useUI.setState({ tab: 'agent' }));
    await act(async () => root.render(<CronOverview />));
    expect(host.querySelector('[role="switch"], .icon-btn')).toBeNull();
    await act(async () => (rows()[2] as HTMLButtonElement).click());
    expect(useUI.getState().tab).toBe('board');
    expect(useData.getState().path).toBe('p/b');
    expect(useUI.getState().cardCrons).toBe('p/b');
  });

  it('一条都没有时写怎么加', async () => {
    act(() => useData.setState({ ws: workspace() }));
    await act(async () => root.render(<CronOverview />));
    expect(host.querySelector('.co-sum')?.textContent).toBe('0 条 · 开着 0 条 · 0 块白板');
    expect(host.textContent).toContain('还没有定时建卡。在白板页工作流条右边点「定时建卡」加一条');
  });
});

describe('白板页的定时建卡弹窗', () => {
  const dialog = () => document.querySelector<HTMLElement>('.cc-modal')!;
  const esc = () => act(() => document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));

  it('打开这块白板的弹窗：每条一行、「加一条定时建卡」；Esc 先关标签选择器，再关弹窗', async () => {
    act(() => useData.setState({ ws: data(), path: 'p/a' }));
    await act(async () => root.render(<CardCronDialog path="p/a" />));
    expect(dialog().classList.contains('on')).toBe(false);
    act(() => openCardCrons('p/a'));
    expect(dialog().classList.contains('on')).toBe(true);
    expect(dialog().querySelectorAll('.cc-row')).toHaveLength(2);
    expect([...dialog().querySelectorAll('button')].some(b => b.textContent?.includes('加一条定时建卡'))).toBe(true);
    // 弹窗底部不放说明文字
    expect(dialog().querySelector('.bs-foot')).toBeNull();
    act(() => openPicker({ rect: new DOMRect(), project: 'p', selected: [], onToggle: () => {} }));
    esc();
    expect(useUI.getState().picker).toBe(null);
    expect(useUI.getState().cardCrons).toBe('p/a');
    esc();
    expect(useUI.getState().cardCrons).toBe(null);
  });

  it('别的白板的弹窗不开', async () => {
    act(() => useData.setState({ ws: data(), path: 'p/a' }));
    await act(async () => root.render(<CardCronDialog path="p/a" />));
    act(() => openCardCrons('p/b'));
    expect(dialog().classList.contains('on')).toBe(false);
  });
});
