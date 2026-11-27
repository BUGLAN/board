/* Agent tab（default/main#394）：侧栏三组八节、一节一页；没有机器时照常能进，分工里能先建角色、再加规则；
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
  const profiles: Record<string, unknown>[] = [];
  const fetch = vi.fn(async (url: string, init?: RequestInit) => {
    const path = String(url).split('?')[0];
    const method = init?.method ?? 'GET';
    if (method !== 'GET') sent.push({ method, path, body: init?.body ? JSON.parse(String(init.body)) : null });
    if (path === '/api/sched' && method === 'PATCH') rules = (JSON.parse(String(init!.body)) as { rules: unknown[] }).rules;
    let made: unknown = null;
    if (path === '/api/profiles' && method === 'POST') {
      made = { ...(JSON.parse(String(init!.body)) as object), id: `p${profiles.length + 1}` };
      profiles.push(made as Record<string, unknown>);
    }
    const sched = { rules, minute: 600, autoUpgrade: false, maxRun: 0, idleRun: 0, profiles };
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
                : path === '/api/profiles'
                  ? made
                  : path === '/api/stats'
                    ? { profiles: [{ profile: 'p1', name: '主力', gone: false, claims: 4, review: 3, firstPass: 0.5, rework: 1, lost: 1, gaveup: 0, spent: 80 * 60_000, avg: 20 * 60_000 }] }
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
    expect(navItems()).toEqual(['模型供应商', 'MCP 服务器', 'skills', 'Git 凭据', '分工', '定时建卡', '跑多久停', '接入 agent', '访问令牌']);
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

  it('分工：没有角色时先新建角色（内置的 agent 能选），再加规则交给它，规则能存', async () => {
    const { fetch, sent } = localServer();
    vi.stubGlobal('fetch', fetch);
    const ws = await api.workspace();
    act(() => useData.setState({ ws }));
    act(() => useAgentSec.setState({ sec: 'rules' }));
    await show();
    expect(button('加一条规则')!.disabled, '还没有角色：规则没地方交').toBe(true);
    expect(host.querySelector('.sd-empty')?.textContent).toContain('还没有角色');
    await click(button('新建角色'));
    const form = document.querySelector('.pf-form')!;
    // 谁来做：内置的 agent（服务端的说明），默认第一个
    expect([...form.querySelectorAll('.pf-agent')].map(b => b.textContent)).toEqual(['Claude Code', 'Codex']);
    expect(form.querySelector('.pf-agent.on')?.textContent).toBe('Claude Code');
    const name = form.querySelector<HTMLInputElement>('input.input')!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(name, '主力');
      name.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await click([...form.querySelectorAll('button')].find(b => b.textContent === '新建'));
    await act(async () => {});
    expect(sent).toContainEqual({ method: 'POST', path: '/api/profiles', body: { name: '主力', agent: 'claude', provider: '', model: '', effort: '', note: '', only: [] } });
    // 存好之后服务端发回的工作区里有这个角色（这里直接换上新的工作区，不走 store 的刷新）
    act(() => useData.setState({ ws: null }));
    const next = await api.workspace();
    act(() => useData.setState({ ws: next }));
    await act(async () => {});
    expect(host.querySelector('.pfa-prof b')?.textContent).toBe('主力');
    // 选中角色的「近 30 天」（#573）：全部项目、最近 30 天的大盘里按角色那一行
    const dl = [...host.querySelectorAll('.pfq-dl dt')].map(d => d.textContent);
    expect(dl).toEqual(['领卡', '机器', '现在', '近 30 天']);
    expect(host.querySelector('.pfq-score')?.textContent).toBe('交审 3 张一次通过 50%返工 1异常 1平均 20 分钟');
    expect(host.querySelector('.pfq-score .bad')?.textContent, '一次通过率低于 75% 标红').toContain('一次通过');
    const q = new URLSearchParams(String(fetch.mock.calls.find(([u]) => String(u).startsWith('/api/stats'))![0]).split('?')[1]);
    expect(q.get('scope')).toBe('all');
    expect(Number(q.get('to')) - Number(q.get('from'))).toBe(30 * 24 * 3600_000);
    // 加一条规则：条件选「没有标签」（不用挑标签），交给主力
    await click(button('加一条规则'));
    const dlg = document.querySelector('.pf-rd')!;
    await click([...dlg.querySelectorAll('.segmented button')].find(b => b.textContent === '没有标签'));
    await click([...dlg.querySelectorAll('button')].find(b => b.textContent === '保存'));
    await act(async () => {});
    expect(sent).toContainEqual({ method: 'PATCH', path: '/api/sched', body: { rules: [{ op: 'none', labels: [], profile: 'p1' }] } });
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

describe('安装 skill 命令（#518）', () => {
  it('Windows：用 PowerShell 创建目录联接，目标已存在时不覆盖，路径用 %USERPROFILE%', async () => {
    const writes: string[] = [];
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: async (t: string) => void writes.push(t) } });
    const { SkillInstallation } = await import('./Access');
    await act(async () => root.render(<SkillInstallation windows />));
    const cmd = host.querySelector('.code-line code')?.textContent ?? '';
    const hint = host.querySelector('.set-row .set-label span')?.textContent ?? '';
    expect(cmd).toContain('New-Item -ItemType Junction');
    expect(cmd).toContain('Test-Path -LiteralPath $dst');
    expect(cmd).toContain('throw "目标已存在，未覆盖：$dst"');
    expect(cmd).not.toContain('ln -s');
    expect(hint).toContain('%USERPROFILE%\\.agents\\skills\\board-skills');
    expect(hint).toContain('%USERPROFILE%\\.claude\\skills\\board-skills');
    expect(hint).not.toContain('~/.agents');
    expect(host.querySelector('.code-line')?.classList.contains('ps')).toBe(true);
    await act(async () => host.querySelector<HTMLButtonElement>('.code-line button.copy')!.click());
    expect(writes).toEqual([cmd]);
  });

  it('macOS / Linux：保留原来的软链接命令和 ~ 路径', async () => {
    const { SkillInstallation } = await import('./Access');
    await act(async () => root.render(<SkillInstallation windows={false} />));
    const cmd = host.querySelector('.code-line code')?.textContent ?? '';
    const hint = host.querySelector('.set-row .set-label span')?.textContent ?? '';
    expect(cmd).toBe('ln -s "$PWD/.agents/skills/board-skills" ~/.agents/skills/board-skills');
    expect(hint).toContain('~/.agents/skills/board-skills/');
    expect(hint).toContain('~/.claude/skills/board-skills/');
    expect(hint).not.toContain('%USERPROFILE%');
    expect(host.querySelector('.code-line')?.classList.contains('ps')).toBe(false);
  });
});

describe('安装 board 命令行（#711）', () => {
  it('Windows：目录带撇号时正确转义，读写用户 PATH 保留 REG_EXPAND_SZ', async () => {
    const { pathCommand } = await import('./Access');
    const cmd = pathCommand('powershell', '', String.raw`C:\Users\O'Brien\Board\bin`);
    expect(cmd).toContain(String.raw`$d = 'C:\Users\O''Brien\Board\bin'`);
    expect(cmd).toContain("[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)");
    expect(cmd).toContain('DoNotExpandEnvironmentNames');
    expect(cmd).toContain("$key.SetValue('Path', (($parts + $d) -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)");
    expect(cmd).not.toContain('[Environment]::GetEnvironmentVariable');
    expect(cmd).not.toContain('[Environment]::SetEnvironmentVariable');
  });
});

describe('复制按钮（#519）', () => {
  it('接入 agent 页每个复制按钮的名称都不一样，写明复制的是哪条；复制成功有 toast', async () => {
    const { fetch } = localServer();
    vi.stubGlobal('fetch', fetch);
    const writes: string[] = [];
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: async (t: string) => void writes.push(t) } });
    const ws = await api.workspace();
    act(() => useData.setState({ ws }));
    act(() => useAgentSec.setState({ sec: 'access' }));
    await show();
    const btns = [...host.querySelectorAll<HTMLButtonElement>('#ag-access button.copy')];
    const names = btns.map(b => b.getAttribute('aria-label') ?? '');
    expect(btns.length).toBeGreaterThan(5);
    expect(names.every(n => n.startsWith('复制：') && n.length > 3)).toBe(true);
    expect(new Set(names).size).toBe(names.length);
    expect(names).toContain('复制：处理一张卡片');
    // 点一个：复制的是那一行的命令，toast 告诉屏幕阅读器
    const line = btns[0].closest('.code-line')!.querySelector('code')!.textContent;
    await act(async () => btns[0].click());
    expect(writes).toEqual([line]);
    expect(useUI.getState().toasts.at(-1)?.title).toBe('已复制');
  });

  it('访问令牌页：命令里的令牌原文不出现在按钮名称里', async () => {
    const { copyLabel } = await import('./Access');
    expect(copyLabel('board login --url https://b.test --token bd_0123456789abcdef0123')).toBe('复制：board login --url https://b.test --token <令牌>');
    expect(copyLabel('x', '处理一张卡片')).toBe('复制：处理一张卡片');
    expect([...copyLabel('a'.repeat(100))].length).toBe('复制：'.length + 48);
  });
});
