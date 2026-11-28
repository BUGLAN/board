/* src/components/detail-keys*.test.tsx 共用：渲染一张打开着的卡片详情（有一条人写的评论），派发按键 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import type { Card } from '../src/lib/types';
import { useData } from '../src/store/data';
import { CardDetail } from '../src/components/CardDetail';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = new Date(2026, 10, 14, 12).getTime();

export const card: Card = {
  id: 2,
  title: '修一下横轴',
  body: '原来的描述',
  column: 'todo',
  holder: null,
  claimedAt: null,
  beatAt: null,
  runStartedAt: null,
  workMs: 0,
  runs: 0,
  runsLost: 0,
  claimedFrom: null,
  labels: [],
  createdAt: NOW - 3600_000,
  updatedAt: NOW,
  createdBy: { kind: 'human', name: '人' },
  comments: 1,
};

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

export function mount() {
  if (!Element.prototype.animate) {
    Object.defineProperty(Element.prototype, 'animate', { configurable: true, value: () => ({ cancel() {}, finished: Promise.resolve() }) });
  }
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({
    ws: { rev: 1, now: NOW, projects: [{ key: 'oss', name: '开源仓库', boards: [{ key: 'main', path: 'oss/main', name: '主线', statuses: [] }] }] },
    path: 'oss/main',
    board: { cards: { 2: card }, cols: { todo: [2] } },
    detailId: 2,
    detail: { ...card, activity: [{ id: 9, kind: 'comment', at: NOW - 60_000, actor: { kind: 'human', name: '人' }, text: '先看看' }] },
    detailError: null,
    saving: {},
  } as never);
  act(() => root.render(<CardDetail />));
  return host;
}

export function unmount() {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
}

/** 在 el 上派发一次 keydown（会冒泡到 document），返回事件；keyCode 229 是 WebKit 结束组字时的那个 */
export function press(el: Element, init: KeyboardEventInit & { keyCode?: number }) {
  const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  if (init.keyCode != null) Object.defineProperty(e, 'keyCode', { value: init.keyCode });
  act(() => {
    el.dispatchEvent(e);
  });
  return e;
}

/** 像人一样改输入框的值（React 认得出来） */
export function type(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')!.set!;
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
