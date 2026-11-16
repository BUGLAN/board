/* Agent tab（default/main#394）：侧栏三组八节、一节一页；没有机器时照常能进，领取规则用内置的 agent 也能加；
   设置页去掉了搬走的三节 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../../lib/api';
import { useData } from '../../store/data';
import { useUI } from '../../store/ui';
import { SECTIONS } from '../SettingsNav';
import { AgentNav, GROUPS, useAgentSec } from './AgentNav';
import { AgentPage } from './AgentPage';

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
  vi.unstubAllGlobals();
  useData.setState({ ws: null });
  useUI.setState({ ctx: null, tab: 'board' });
  useAgentSec.setState({ sec: 'providers' });
});

const META = [
  { id: 'claude', name: 'Claude Code', protocols: ['anthropic'], efforts: [] },
  { id: 'codex', name: 'Codex', protocols: ['openai'], efforts: [] },
];

/** 本地服务端（--local --demo）那样：一个项目一块白板，没有机器；记下改领取规则的请求 */
function localServer() {
  const sent: { method: string; path: string; body: unknown }[] = [];
  let rules: unknown[] = [];
  const fetch = vi.fn(async (url: string, init?: RequestInit) => {
    const path = String(url).split('?')[0];
    const method = init?.method ?? 'GET';
    if (method !== 'GET') sent.push({ method, path, body: init?.body ? JSON.parse(String(init.body)) : null });
    if (path === '/api/sched' && method === 'PATCH') rules = (JSON.parse(String(init!.body)) as { rules: unknown[] }).rules;
    const sched = { rules, fallback: null, minute: 600, autoUpgrade: false, maxRun: 0, idleRun: 0 };
    const body: unknown =
      path === '/api/workspace'
        ? { rev: 1, now: Date.now(), projects: [{ key: 'oss', name: 'oss', color: '#888', labels: [{ id: 1, name: 'bug', color: '#e5484d' }], boards: [{ key: 'main', name: 'main', path: 'oss/main', statuses: [] }] }], hosts: [], dispatches: [], sched }
        : path === '/api/hosts'
          ? { hosts: [] }
          : path === '/api/providers'
            ? { providers: [], meta: META }
            : path === '/api/sched'
              ? sched
              : path === '/api/tokens'
                ? []
                : null;
    return new Response(body == null ? '' : JSON.stringify(body), { status: 200 });
  });
  return { fetch, sent };
}

async function show() {
  await act(async () =>
    root.render(
      <>
        <AgentNav />
        <AgentPage />
      </>,
    ),
  );
  await act(async () => {});
}

const navItems = () => [...host.querySelectorAll('.ag-nav .sn-item')].map(b => b.textContent);
const click = (el: Element | null | undefined) => act(() => (el as HTMLElement).click());
const button = (text: string) => [...host.querySelectorAll('button')].find(b => b.textContent?.includes(text));

describe('Agent tab 的侧栏目录', () => {
  it('三组九节：共用配置 → 自动领取 → 接入，进 tab 落在模型供应商', async () => {
    const { fetch } = localServer();
    vi.stubGlobal('fetch', fetch);
    act(() => useUI.setState({ tab: 'agent' }));
    const ws = await api.workspace();
    act(() => useData.setState({ ws }));
    await show();
    expect([...host.querySelectorAll('.ag-nav-h')].map(h => h.textContent)).toEqual(['共用配置', '自动领取', '接入']);
    expect(navItems()).toEqual(['模型供应商', 'MCP 服务器', 'skills', 'Git 凭据', '领取规则', '定时建卡', '跑多久停', '接入 agent', '访问令牌']);
    expect(GROUPS.flatMap(g => g.secs).length).toBe(9);
    expect(host.querySelector('.ag-nav .sn-item.on')?.textContent).toBe('模型供应商');
    expect(host.querySelector('.hp-b-head p')?.textContent).toBe('共用配置 · 模型供应商');
    expect(host.querySelectorAll('.ag-sec > *')).toHaveLength(1);
    expect(host.querySelector('.sc-providers')).not.toBeNull();
  });

  it('点一节主区只画那一节', async () => {
    const { fetch } = localServer();
    vi.stubGlobal('fetch', fetch);
    const ws = await api.workspace();
    act(() => useData.setState({ ws }));
    await show();
    await click([...host.querySelectorAll('.ag-nav .sn-item')].find(b => b.textContent === '访问令牌'));
    await act(async () => {});
    expect(host.querySelector('.hp-b-head p')?.textContent).toBe('接入 · 访问令牌');
    expect(host.querySelector('#ag-tokens')).not.toBeNull();
    expect(host.querySelector('.sc-providers')).toBeNull();
    // 令牌列表切过来就取（以前在设置页只在 tab 是 settings 时取）
    expect(fetch.mock.calls.some(([u]) => String(u).startsWith('/api/tokens'))).toBe(true);

    await click([...host.querySelectorAll('.ag-nav .sn-item')].find(b => b.textContent === '接入 agent'));
    expect(host.querySelector('#ag-access')).not.toBeNull();
    expect(host.querySelector('#ag-tokens')).toBeNull();
    // 白板规则里的心跳和规则列表并进来，折叠在错误码前面
    expect([...host.querySelectorAll('#ag-access details summary')].map(s => s.textContent)).toEqual(['规则摘要（心跳、领取、移动）', '错误码']);
  });
});

describe('没有机器时', () => {
  it('顶部一行提示带「添加机器」，每一节都画得出来', async () => {
    const { fetch } = localServer();
    vi.stubGlobal('fetch', fetch);
    const ws = await api.workspace();
    act(() => useData.setState({ ws }));
    for (const s of GROUPS.flatMap(g => g.secs)) {
      act(() => useAgentSec.setState({ sec: s.id }));
      await show();
      expect(host.querySelector('.ag-nohost')?.textContent).toContain('还没有机器。这里的配置现在就能改');
      expect(host.querySelector('.ag-sec')?.children.length).toBe(1);
    }
    await click(button('添加机器'));
    expect(useUI.getState().tab).toBe('hosts');
    expect(useUI.getState().host).toBe('new');
    act(() => useUI.setState({ host: null }));
  });

  it('领取规则：交给哪个 agent 列内置的 agent，规则能加能存，规则卡上写「没有机器上有它」', async () => {
    const { fetch, sent } = localServer();
    vi.stubGlobal('fetch', fetch);
    const ws = await api.workspace();
    act(() => useData.setState({ ws }));
    act(() => useAgentSec.setState({ sec: 'rules' }));
    await show();
    const add = button('加一条规则')!;
    expect(add.disabled).toBe(false);
    await click(add);
    // 下拉里是内置的 agent
    expect(host.querySelector('.sd-form .sd-agent-pick')?.textContent).toBe('Claude Code');
    // 条件选「没有标签」（不用挑标签），然后加上
    await click(host.querySelector('.sd-form .sd-op'));
    const ctx = useUI.getState().ctx;
    expect(ctx?.kind).toBe('select');
    act(() => (ctx as { pick: (v: string) => void }).pick('none'));
    await click(button('加上'));
    await act(async () => {});
    expect(sent).toContainEqual({ method: 'PATCH', path: '/api/sched', body: { rules: [{ op: 'none', labels: [], agent: 'claude' }] } });
    // 存好之后服务端发回的工作区里有这条规则（这里直接换上新的工作区，不走 store 的刷新）
    const next = await api.workspace();
    act(() => useData.setState({ ws: next }));
    await act(async () => {});
    expect(host.querySelector('.sd-card header b')?.textContent).toBe('Claude Code');
    expect(host.querySelector('.sd-card .sd-sum')?.textContent).toBe('没有机器上有它');
  });
});

describe('设置 tab', () => {
  it('没有白板规则、Agent 接入、访问令牌三节', () => {
    const ids = SECTIONS.map(s => s.id);
    for (const gone of ['board', 'agents', 'tokens']) expect(ids).not.toContain(gone);
    expect(ids.slice(0, 2)).toEqual(['account', 'appearance']);
    expect(ids[ids.length - 1]).toBe('shortcuts');
  });
});
