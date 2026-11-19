/* 添加机器 → 只有命令行的机器（#519）：安装命令的复制按钮有自己的名称（不把带令牌的整条命令当名称），复制后有提示 */
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Workspace } from '../../lib/types';
import { useData } from '../../store/data';
import { openHost, useUI } from '../../store/ui';
import { HostModal } from './HostModal';

const RAW = 'bd_0123456789abcdef0123456789abcdef';
vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return {
    ...real,
    api: {
      ...real.api,
      createToken: async (name: string, opts: { host?: string } = {}) => ({ token: RAW, info: { id: 3, name, hint: 'bd_012345…', createdAt: 0, lastUsedAt: null, host: opts.host ?? null } }),
    },
  };
});
// 写操作的队列和刷新不在这里测：直接执行请求
vi.mock('../../store/data', async orig => {
  const real = await orig<typeof import('../../store/data')>();
  return { ...real, optimistic: (_patch: unknown, f: () => Promise<unknown>) => f() };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => openHost(null));
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  useData.setState({ ws: null });
  useUI.setState({ host: null });
});

const type = (el: HTMLInputElement, v: string) =>
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });

describe('添加机器的安装命令', () => {
  it('复制按钮名称是「复制安装命令」，不带令牌原文；复制的是整条命令，并提示已复制', async () => {
    // 弹窗里别的请求（机器列表轮询之类）都挂着不回
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(() => {})));
    const writes: string[] = [];
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: async (t: string) => void writes.push(t) } });
    act(() => useData.setState({ ws: { hosts: [], dispatches: [] } as unknown as Workspace }));
    await act(async () => root.render(<HostModal />));
    await act(async () => openHost('new'));
    const inputs = [...document.querySelectorAll<HTMLInputElement>('.hp-add input.input')];
    type(inputs.find(i => i.placeholder === 'devbox')!, 'devbox');
    type(inputs.find(i => i.placeholder.includes('repo'))!, '~/repo');
    const gen = [...document.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent?.includes('生成命令'))!;
    await act(async () => gen.click());
    const btn = document.querySelector<HTMLButtonElement>('button.hp-cmd')!;
    expect(btn.getAttribute('aria-label')).toBe('复制安装命令');
    await act(async () => btn.click());
    expect(writes.length).toBe(1);
    expect(writes[0]).toContain(RAW);
    expect(useUI.getState().toasts.at(-1)?.title).toBe('已复制');
  });
});

const css = (name: string) => readFileSync(new URL(`../../styles/${name}`, pathToFileURL(__filename)), 'utf8');

describe('长命令不在中间断行（#521）', () => {
  it('安装命令、接入 agent 页的 shell 命令都不断行、放不下时横向滚动；提示词句子照常换行', () => {
    const cmd = css('hosts.css').match(/\.hp-cmd code \{[^}]*\}/)![0];
    expect(cmd).toMatch(/white-space: pre;/);
    expect(cmd).toMatch(/overflow-x: auto/);
    expect(cmd).not.toMatch(/break-all|anywhere/);
    const line = css('settings.css').match(/\.code-line:not\(\.plain\) code \{[^}]*\}/)![0];
    expect(line).toMatch(/white-space: pre;/);
    expect(line).toMatch(/overflow-x: auto/);
    expect(line).toMatch(/overflow-wrap: normal/);
    expect(css('settings.css')).toMatch(/\.code-line\.plain code \{ overflow-wrap: break-word; \}/);
    expect(css('app.css')).not.toMatch(/\.token-fresh \.code-line code \{[^}]*anywhere/);
  });
});
