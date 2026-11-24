import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HostInfo, LocalSkill } from '../../lib/types';
import { SkillImportModal } from './SkillImport';

const mocks = vi.hoisted(() => ({ importSkills: vi.fn(), rescanHostSkills: vi.fn() }));

vi.mock('../../lib/api', async orig => {
  const real = await orig<typeof import('../../lib/api')>();
  return { ...real, api: { ...real.api, importSkills: mocks.importSkills, rescanHostSkills: mocks.rescanHostSkills } };
});

// 写队列和刷新不在这里测：直接执行请求
vi.mock('../../store/data', async orig => {
  const real = await orig<typeof import('../../store/data')>();
  return { ...real, optimistic: (_patch: unknown, req: () => Promise<unknown>) => req() };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  mocks.importSkills.mockReset();
  mocks.importSkills.mockResolvedValue({ skillImport: { able: true, local: [], last: null } });
  mocks.rescanHostSkills.mockReset();
  mocks.rescanHostSkills.mockResolvedValue({ skillImport: { able: true, rescan: true, local: [], last: null } });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.replaceChildren();
});

const local = (name: string, x: Partial<LocalSkill> = {}): LocalSkill => ({
  name,
  version: '123456789abc',
  files: 1,
  size: 1024,
  agents: ['codex'],
  dirs: [`~/.agents/skills/${name.toLowerCase().replaceAll(' ', '-')}`],
  ...x,
});

const machine = (rescan = false, online = true): HostInfo =>
  ({
    id: 1,
    name: 'win',
    os: 'windows',
    version: '0.1.43',
    agents: [],
    dirs: [],
    max: 1,
    paused: false,
    agent: null,
    round: false,
    online,
    beatAt: 0,
    offAt: null,
    skillImport: {
      able: true,
      rescan,
      local: [local('Agent Browser'), local('Beta Tool'), local('Gamma', { dirs: ['~/.agents/skills/tool-gamma'] }), local('Tool Legacy', { why: '超过 5 MB' })],
      last: null,
    },
  }) as HostInfo;

const render = (h = machine()) => act(() => root.render(<SkillImportModal hosts={[h]} lib={[]} onClose={() => {}} />));
const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]')!;
const search = () => dialog().querySelector<HTMLInputElement>('input[aria-label="搜索 skill"]')!;
const rows = () => [...dialog().querySelectorAll<HTMLButtonElement>('.si-row')];
const row = (name: string) => rows().find(r => r.querySelector('.si-name b')?.textContent === name)!;
const button = (text: string) => [...dialog().querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent?.trim() === text)!;
const importButton = () => [...dialog().querySelectorAll<HTMLButtonElement>('button')].find(b => /^导入 \d+ 个$/.test(b.textContent?.trim() ?? ''))!;
const type = (el: HTMLInputElement, value: string) =>
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
const click = (el: Element) => act(() => el.dispatchEvent(new MouseEvent('click', { bubbles: true })));

describe('从机器导入 skills 的搜索', () => {
  it('按名称不区分大小写筛选，只匹配名称；清空恢复全部，没有匹配时显示专门的空状态', () => {
    render();
    expect(rows().map(r => r.querySelector('.si-name b')?.textContent)).toEqual(['Agent Browser', 'Beta Tool', 'Gamma', 'Tool Legacy']);

    type(search(), 'TOOL');
    expect(rows().map(r => r.querySelector('.si-name b')?.textContent)).toEqual(['Beta Tool', 'Tool Legacy']);

    type(search(), 'zzz');
    expect(rows()).toHaveLength(0);
    expect(dialog().textContent).toContain('没有匹配的 skill');
    expect(dialog().textContent).not.toContain('这台机器的各 agent 目录里没有自己放的 skill');

    type(search(), '');
    expect(rows()).toHaveLength(4);
  });

  it('全选只作用于当前结果里能导入的行；切换搜索保留已选；导入仍带上隐藏的已选', async () => {
    render();
    type(search(), 'tool');
    click(button('全选能导入的'));
    expect(row('Beta Tool').getAttribute('aria-checked')).toBe('true');
    expect(row('Tool Legacy').getAttribute('aria-checked')).toBe('false');
    expect(importButton().textContent?.trim()).toBe('导入 1 个');

    type(search(), '');
    click(row('Agent Browser'));
    expect(importButton().textContent?.trim()).toBe('导入 2 个');

    type(search(), 'browser');
    expect(rows().map(r => r.querySelector('.si-name b')?.textContent)).toEqual(['Agent Browser']);
    expect(row('Agent Browser').getAttribute('aria-checked')).toBe('true');
    expect(importButton().textContent?.trim()).toBe('导入 2 个');

    await act(async () => importButton().click());
    expect(mocks.importSkills).toHaveBeenCalledWith('win', [
      { name: 'Beta Tool', dir: '~/.agents/skills/beta-tool' },
      { name: 'Agent Browser', dir: '~/.agents/skills/agent-browser' },
    ]);
  });
});

describe('从机器导入 skills 的按需重扫', () => {
  it('打开时让在线的新版 host 重扫一次；重复渲染不重复，点刷新再发请求', async () => {
    const win = machine(true);
    await act(async () => {
      root.render(<SkillImportModal hosts={[win]} lib={[]} onClose={() => {}} />);
    });
    expect(mocks.rescanHostSkills).toHaveBeenCalledTimes(1);
    expect(mocks.rescanHostSkills).toHaveBeenCalledWith('win');
    await act(async () => {
      root.render(<SkillImportModal hosts={[win]} lib={[]} onClose={() => {}} />);
    });
    expect(mocks.rescanHostSkills).toHaveBeenCalledTimes(1);
    await act(async () => button('刷新').click());
    expect(mocks.rescanHostSkills).toHaveBeenCalledTimes(2);
  });

  it('旧版 host 只显示缓存并提示升级，不发重扫请求', async () => {
    await act(async () => {
      root.render(<SkillImportModal hosts={[machine(false)]} lib={[]} onClose={() => {}} />);
    });
    expect(mocks.rescanHostSkills).not.toHaveBeenCalled();
    expect(dialog().textContent).toContain('太旧，要升级');
    expect(rows()).toHaveLength(4);
  });

  it('离线 host 仍能看上次上报的清单，不重扫也不能导入', async () => {
    await act(async () => {
      root.render(<SkillImportModal hosts={[machine(true, false)]} lib={[]} onClose={() => {}} />);
    });
    expect(mocks.rescanHostSkills).not.toHaveBeenCalled();
    expect(dialog().textContent).toContain('机器离线：下面是上次上报的 skill');
    expect(rows()).toHaveLength(4);
    expect(row('Agent Browser').hasAttribute('disabled')).toBe(true);
    expect(button('导入').hasAttribute('disabled')).toBe(true);
  });
});
