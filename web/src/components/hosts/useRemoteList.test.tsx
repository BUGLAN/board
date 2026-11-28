import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HostInfo, Workspace } from '../../lib/types';
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

const machine = (id: number, name: string, online = true) => ({ id, name, online }) as HostInfo;
const state = (rev: number, hosts: HostInfo[]): Pick<Workspace, 'rev' | 'hosts'> => ({ rev, hosts });

/** 把 hook 的结果画出来：`取得|原因|列表`，并留一个按钮调 refresh() */
type Hook = ReturnType<typeof useRemoteList<string[]>>;
function Probe({ load, ws }: { load: () => Promise<string[]>; ws: Pick<Workspace, 'rev' | 'hosts'> }) {
  const h: Hook = useRemoteList(load, ws);
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
    await act(async () => root.render(<Probe load={load} ws={state(1, [])} />));
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
    await act(async () => root.render(<Probe load={load} ws={state(1, [])} />));
    expect(text()).toBe('读取中|连不上 Board 服务');
    // 点一下重新取：这次成功，错误清掉、列表出来
    fail = false;
    await act(async () => (host.querySelector('button') as HTMLButtonElement).click());
    expect(text()).toBe('ok|');
  });

  it('只有 ws.rev 或机器在线状态变化时重新取，refresh() 也能手动重取', async () => {
    const load = vi.fn(async () => ['a']);
    const none = state(1, []);
    await act(async () => root.render(<Probe load={load} ws={none} />));
    expect(load).toHaveBeenCalledTimes(1);
    const hosts = [machine(1, 'win2', true), machine(2, 'mbp', false)];
    await act(async () => root.render(<Probe load={load} ws={state(1, hosts)} />));
    expect(load).toHaveBeenCalledTimes(2);
    // 机器对象换了、心跳时间变了，但 id 和在线状态没变：不再取
    await act(async () =>
      root.render(
        <Probe
          load={load}
          ws={state(1, [
            { ...hosts[0], beatAt: 100 },
            { ...hosts[1], beatAt: 200 },
          ])}
        />,
      ),
    );
    expect(load).toHaveBeenCalledTimes(2);
    // 一台机器的在线状态变了：取一次
    await act(async () => root.render(<Probe load={load} ws={state(1, [hosts[0], { ...hosts[1], online: true }])} />));
    expect(load).toHaveBeenCalledTimes(3);
    // 工作区有新的 rev：取一次
    await act(async () => root.render(<Probe load={load} ws={state(2, [hosts[0], { ...hosts[1], online: true }])} />));
    expect(load).toHaveBeenCalledTimes(4);
    // 手动 refresh 照常
    await act(async () => (host.querySelector('button') as HTMLButtonElement).click());
    expect(load).toHaveBeenCalledTimes(5);
  });
});
