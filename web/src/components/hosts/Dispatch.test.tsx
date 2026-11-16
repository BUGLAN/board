import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { agentMark } from '../../lib/hosts';
import type { Dispatch } from '../../lib/types';
import { useData } from '../../store/data';
import { CardDispatchLine } from './Dispatch';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = new Date(2026, 10, 16, 9).getTime();

const dispatch = (agent: string, agentLabel: string): Dispatch => ({
  id: 7,
  board: 'default/main',
  card: 404,
  title: '修一下派发排队行',
  hostId: 2,
  host: 'win2',
  agent,
  agentLabel,
  name: null,
  source: 'manual',
  by: { kind: 'human', name: '人' },
  state: 'queued',
  result: null,
  createdAt: NOW,
  startedAt: null,
  endedAt: null,
  exitCode: null,
  stopBy: null,
  beatAt: null,
});

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

const render = (d: Dispatch) => {
  useData.setState({
    ws: { rev: 1, now: NOW, projects: [], dispatches: [d] },
    path: 'default/main',
    cards: {},
    saving: {},
    pending: {},
  } as never);
  act(() => root.render(<CardDispatchLine card={d.card} />));
};

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
});

describe('卡片上的派发排队行', () => {
  it('认识的 agent 画它自己的 logo，不再是通用派发图标', () => {
    render(dispatch('codex', 'Codex'));
    const line = host.querySelector('.card-dispatch') as HTMLElement;
    expect(line.textContent).toContain('排队 → win2 · Codex');
    expect(line.querySelector('img.hp-logo')?.getAttribute('src')).toBe(agentMark('codex').icon);
    // 通用派发图标是 <svg>；换了 logo 之后这一行没有它了
    expect(line.querySelector('svg')).toBeNull();
  });

  it('单色 logo（dsh 的鲸鱼）用遮罩画，深浅主题都跟 --agent-ink', () => {
    render(dispatch('dsh', 'DeepSeek Harness'));
    const line = host.querySelector('.card-dispatch') as HTMLElement;
    const art = line.querySelector('.hp-logo.logo-mono') as HTMLElement;
    expect(art.style.maskImage).toContain(agentMark('dsh').icon!);
    expect(line.querySelector('img')).toBeNull();
  });

  it('认不出的 agent 退回两个字母，不出现空图标', () => {
    render(dispatch('alpha', 'Alpha'));
    const line = host.querySelector('.card-dispatch') as HTMLElement;
    const mark = line.querySelector('b.hp-mini') as HTMLElement;
    expect(mark.textContent).toBe('AL');
    expect(line.querySelector('img, .logo-mono')).toBeNull();
    expect(line.textContent).toContain('排队 → win2 · Alpha');
  });
});
