import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HostInfo } from '../../lib/types';
import { useRemoteList } from './useRemoteList';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const machine = (name: string) => ({ name }) as HostInfo;

/** 把 hook 的结果画出来：`取得|原因|列表`，并留一个按钮调 refresh() */
type Hook = ReturnType<typeof useRemoteList<string[]>>;
function Probe({ load, hosts }: { load: () => Promise<string[]>; hosts: HostInfo[] }) {
  const h: Hook = useRemoteList(load, hosts);
  return (
    <button type="button" onClick={() => void h.refresh()}>
      {`${h.list === null ? '读取中' : h.list.join(',')}|${h.err ?? ''}`}
    </button>
  );
}
const text = () => host.querySelector('button')!.textContent;

describe('共用配置的远程列表', () => {
  it('取到之前是 null，取到之后是列表；没有错误', async () => {
    const load = vi.fn(async () => ['a', 'b']);
    await act(async () => root.render(<Probe load={load} hosts={[]} />));
    expect(load).toHaveBeenCalledTimes(1);
    expect(text()).toBe('a,b|');
  });

  it('取不到时留下原因，再取到一次就清掉', async () => {
    // 第一次失败，之后成功
    let fail = true;
    const load = vi.fn(async () => {
      if (fail) throw new Error('连不上 Board 服务');
      return ['ok'];
    });
    await act(async () => root.render(<Probe load={load} hosts={[]} />));
    expect(text()).toBe('读取中|连不上 Board 服务');
    // 点一下重新取：这次成功，错误清掉、列表出来
    fail = false;
    await act(async () => (host.querySelector('button') as HTMLButtonElement).click());
    expect(text()).toBe('ok|');
  });

  it('机器有变化时跟着重新取一次，refresh() 也能手动重取', async () => {
    const load = vi.fn(async () => ['a']);
    const none: HostInfo[] = [];
    const one = [machine('win2')];
    await act(async () => root.render(<Probe load={load} hosts={none} />));
    expect(load).toHaveBeenCalledTimes(1);
    await act(async () => root.render(<Probe load={load} hosts={one} />));
    expect(load).toHaveBeenCalledTimes(2);
    await act(async () => (host.querySelector('button') as HTMLButtonElement).click());
    expect(load).toHaveBeenCalledTimes(3);
    // 机器没变（同一个数组）时重新渲染不再发请求
    await act(async () => root.render(<Probe load={load} hosts={one} />));
    expect(load).toHaveBeenCalledTimes(3);
  });
});
